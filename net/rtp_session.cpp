// rtp_session.cpp - see rtp_session.h. The UDP layer and the timing rules come from
// Headlink's tools/preprobe/rtpmidi.cpp (demo/pre-probe); the single-peer Session there
// is replaced by a participant table.
#include "rtp_session.h"

#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <algorithm>
#include <chrono>
#include <random>

#ifdef _WIN32
#  ifndef WIN32_LEAN_AND_MEAN
#    define WIN32_LEAN_AND_MEAN
#  endif
#  include <winsock2.h>
#  include <ws2tcpip.h>
typedef SOCKET sock_t;
#  define SOCK_BAD INVALID_SOCKET
#  define sock_close closesocket
static int sock_err() { return WSAGetLastError(); }
#else
#  include <arpa/inet.h>
#  include <errno.h>
#  include <fcntl.h>
#  include <netinet/in.h>
#  include <sys/select.h>
#  include <sys/socket.h>
#  include <unistd.h>
typedef int sock_t;
#  define SOCK_BAD (-1)
#  define sock_close ::close
static int sock_err() { return errno; }
#endif

namespace rtpmidi {

const char* peer_state_name(PeerState s) {
    switch (s) {
    case PeerState::Accepting: return "accepting";
    case PeerState::Inviting: return "inviting";
    case PeerState::InvitingData: return "inviting (data port)";
    case PeerState::Connected: return "connected";
    case PeerState::Waiting: return "waiting";
    }
    return "?";
}

// ================================================================ UDP

struct Udp {
    sock_t   s = SOCK_BAD;
    uint16_t port = 0;
};

namespace {

const uint64_t kSec = 10000;   // the session clock counts 100 us units

bool net_init() {
#ifdef _WIN32
    static bool done = false;
    if (!done) { WSADATA w; if (WSAStartup(MAKEWORD(2, 2), &w) != 0) return false; done = true; }
#endif
    return true;
}

Udp* udp_open(uint16_t port, std::string* err) {
    sock_t s = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (s == SOCK_BAD) { if (err) *err = "socket() failed: " + std::to_string(sock_err()); return nullptr; }
#ifdef _WIN32
    // Nobody else may bind this port while we hold it (no SO_REUSEADDR games).
    BOOL excl = TRUE;
    setsockopt(s, SOL_SOCKET, SO_EXCLUSIVEADDRUSE, (const char*)&excl, sizeof excl);
#endif
    sockaddr_in a{}; a.sin_family = AF_INET; a.sin_port = htons(port); a.sin_addr.s_addr = htonl(INADDR_ANY);
    if (bind(s, (sockaddr*)&a, sizeof a) != 0) {
        const int e = sock_err();
        if (err) *err = "UDP port " + std::to_string(port) + " is in use or not allowed (error " + std::to_string(e) + ")";
        sock_close(s); return nullptr;
    }
#ifdef _WIN32
    u_long nb = 1; ioctlsocket(s, FIONBIO, &nb);
    // An ICMP port-unreachable for an earlier send otherwise makes the next
    // recvfrom fail with WSAECONNRESET on Windows.
    BOOL off = FALSE; DWORD ret = 0;
    WSAIoctl(s, _WSAIOW(IOC_VENDOR, 12) /* SIO_UDP_CONNRESET */, &off, sizeof off, nullptr, 0, &ret, nullptr, nullptr);
#else
    fcntl(s, F_SETFL, fcntl(s, F_GETFL, 0) | O_NONBLOCK);
#endif
    socklen_t al = sizeof a;
    getsockname(s, (sockaddr*)&a, &al);
    Udp* u = new Udp; u->s = s; u->port = ntohs(a.sin_port);
    return u;
}

void udp_close(Udp*& u) { if (u) { sock_close(u->s); delete u; u = nullptr; } }

bool udp_send(Udp* u, const std::string& ip, uint16_t port, const std::vector<uint8_t>& b) {
    if (!u || !port) return false;
    sockaddr_in a{}; a.sin_family = AF_INET; a.sin_port = htons(port);
    if (inet_pton(AF_INET, ip.c_str(), &a.sin_addr) != 1) return false;
    return sendto(u->s, (const char*)b.data(), (int)b.size(), 0, (sockaddr*)&a, sizeof a) == (int)b.size();
}

// -1 = nothing waiting, else the datagram length.
int udp_recv(Udp* u, uint8_t* buf, int cap, std::string* ip, uint16_t* port) {
    sockaddr_in a{}; socklen_t al = sizeof a;
    const int n = (int)recvfrom(u->s, (char*)buf, cap, 0, (sockaddr*)&a, &al);
    if (n < 0) return -1;
    char t[INET_ADDRSTRLEN] = {0};
    inet_ntop(AF_INET, &a.sin_addr, t, sizeof t);
    *ip = t; *port = ntohs(a.sin_port);
    return n;
}

uint64_t steady10k() {
    using namespace std::chrono;
    return (uint64_t)duration_cast<microseconds>(steady_clock::now().time_since_epoch()).count() / 100;
}

uint32_t random32() {
    static std::mt19937 rng{std::random_device{}()};
    static std::mutex m;
    std::lock_guard<std::mutex> g(m);
    uint32_t v = 0;
    while (!v) v = rng();
    return v;
}

std::string fmt(const char* f, ...) {
    char b[1024];
    va_list ap; va_start(ap, f);
    vsnprintf(b, sizeof b, f, ap);
    va_end(ap);
    return b;
}

}  // namespace

// ================================================================ session

Session::Session() {}
Session::~Session() { close(); }

uint64_t Session::now10k() const { return cfg_.clock10k ? cfg_.clock10k() : steady10k() - t0_; }

bool Session::is_open() const { return ctl_ != nullptr; }

uint16_t Session::port() const { return ctl_ ? ctl_->port : 0; }

bool Session::open(const SessionConfig& cfg, std::string* err) {
    close();
    if (!net_init()) { if (err) *err = "WSAStartup failed"; return false; }
    cfg_ = cfg;
    if (cfg_.port == 0 || cfg_.port == 65535) { if (err) *err = "bad UDP port"; return false; }
    t0_ = steady10k() - 1;   // so a timestamp is never 0
    std::string e;
    Udp* c = udp_open(cfg_.port, &e);
    Udp* d = c ? udp_open(uint16_t(cfg_.port + 1), &e) : nullptr;
    if (!c || !d) { udp_close(c); if (err) *err = e; return false; }
    std::lock_guard<std::mutex> g(mu_);
    ctl_ = c; data_ = d;
    ssrc_ = random32();
    peers_.clear();
    strangers_.clear();
    return true;
}

void Session::close() {
    Out out;
    {
        std::lock_guard<std::mutex> g(mu_);
        if (!ctl_) return;
        for (Peer& p : peers_)
            if (!p.gone && knows_us(p)) {
                send_to(0, p.v.ip, p.v.ctlPort, am_bye(p.token, ssrc_), "BY", out);
                out.logs.push_back({"RTP-MIDI: said goodbye to " + p.v.display(), false});
            }
        peers_.clear();
        udp_close(ctl_);
        udp_close(data_);
    }
    deliver(out);
}

void Session::deliver(Out& out) {
    if (log_) for (auto& l : out.logs) log_(l.first, l.second);
    if (midi_) for (auto& m : out.midi) midi_(m.first, m.second.data(), m.second.size());
}

void Session::send_to(int which, const std::string& ip, uint16_t port, const std::vector<uint8_t>& pkt,
                      const char* what, Out& out) {
    const bool ok = udp_send(which ? data_ : ctl_, ip, port, pkt);
    if (cfg_.logPackets || !ok) {
        AmPacket a;
        am_parse(pkt.data(), pkt.size(), &a);
        out.logs.push_back({fmt("RTP-MIDI %s %s -> %s:%u  %s%s", which ? "data" : "ctl ", what, ip.c_str(), port,
                                describe(a).c_str(), ok ? "" : "  SEND FAILED"),
                            ok});
    }
}

// A peer that accepted or sent an invitation holds a session with us: it gets a BY.
bool Session::knows_us(const Peer& p) {
    return p.v.state == PeerState::Connected || p.v.state == PeerState::InvitingData || p.v.state == PeerState::Accepting;
}

Session::Peer* Session::by_ssrc(uint32_t ssrc, const std::string& ip) {
    if (!ssrc) return nullptr;
    for (Peer& p : peers_)
        if (!p.gone && p.v.ssrc == ssrc && p.v.ip == ip) return &p;
    return nullptr;
}

Session::Peer* Session::invited_at(const std::string& ip, uint16_t target) {
    for (Peer& p : peers_)
        if (!p.gone && p.v.invited && p.v.ip == ip && p.v.target == target) return &p;
    return nullptr;
}

size_t Session::live() const {
    size_t n = 0;
    for (const Peer& p : peers_) n += !p.gone;
    return n;
}

void Session::set_state(Peer& p, PeerState s, const std::string& why, Out& out) {
    if (p.v.state == s) return;
    p.v.state = s;
    std::string l = "RTP-MIDI: " + p.v.display() + (p.v.display() == p.v.ip ? " " : " (" + p.v.ip + ") ") + peer_state_name(s);
    if (!why.empty()) l += " - " + why;
    // Inviting / Waiting cycles of a peer that is away are detail; arrivals and departures are not.
    out.logs.push_back({l, s == PeerState::Inviting || s == PeerState::InvitingData || s == PeerState::Accepting});
}

void Session::start_attempt(Peer& p) {
    p.token = random32();
    p.tries = 0;
    p.v.ssrc = 0;
    p.haveSeq = false; p.cycles = 0; p.fed = 0;
    p.ckSent = 0; p.ckTs1 = 0; p.ckMissed = 0;
    p.v.ctlPort = p.v.target;
    p.v.dataPort = uint16_t(p.v.target + 1);
    p.nextAt = now10k();
}

void Session::invite(const std::string& ip, uint16_t port, const std::string& label) {
    Out out;
    {
        std::lock_guard<std::mutex> g(mu_);
        if (!ctl_ || ip.empty() || !port) return;
        Peer* p = invited_at(ip, port);
        if (!p && live() >= cfg_.maxPeers) {
            out.logs.push_back({"RTP-MIDI: cannot invite " + ip + ": the session already has " +
                                    std::to_string(cfg_.maxPeers) + " peers", false});
        } else {
            if (!p) {
                peers_.push_back(Peer());
                p = &peers_.back();
                p->v.invited = true;
                p->v.ip = ip;
                p->v.target = port;
                p->v.state = PeerState::Waiting;
            }
            if (!label.empty()) p->v.label = label;
            if (p->v.state == PeerState::Waiting) {
                start_attempt(*p);
                set_state(*p, PeerState::Inviting, "", out);
            }
        }
    }
    deliver(out);
}

void Session::forget(const std::string& ip, uint16_t port) {
    Out out;
    {
        std::lock_guard<std::mutex> g(mu_);
        for (Peer& p : peers_) {
            if (p.gone || p.v.ip != ip || (p.v.target != port && p.v.ctlPort != port)) continue;
            if (knows_us(p)) send_to(0, p.v.ip, p.v.ctlPort, am_bye(p.token, ssrc_), "BY", out);
            p.gone = true;
            out.logs.push_back({"RTP-MIDI: " + p.v.display() + " (" + ip + ") disconnected", false});
        }
        peers_.erase(std::remove_if(peers_.begin(), peers_.end(), [](const Peer& p) { return p.gone; }), peers_.end());
    }
    deliver(out);
}

size_t Session::send(const std::vector<std::vector<uint8_t>>& msgs) {
    std::lock_guard<std::mutex> g(mu_);
    if (!data_) return 0;
    size_t n = 0;
    std::vector<uint8_t> pkt;
    for (const Peer& p : peers_) {
        if (p.gone || p.v.state != PeerState::Connected) continue;
        if (pkt.empty()) pkt = rtp_build(uint16_t(seqOut_++), uint32_t(now10k()), ssrc_, msgs);
        n += udp_send(data_, p.v.ip, p.v.dataPort, pkt);
    }
    return n;
}

std::vector<PeerView> Session::peers() const {
    std::lock_guard<std::mutex> g(mu_);
    std::vector<PeerView> v;
    for (const Peer& p : peers_)
        if (!p.gone) v.push_back(p.v);
    return v;
}

void Session::poll(int timeoutMs) {
    if (!ctl_) return;
    fd_set rs; FD_ZERO(&rs);
    FD_SET(ctl_->s, &rs); FD_SET(data_->s, &rs);
    timeval tv; tv.tv_sec = timeoutMs / 1000; tv.tv_usec = (timeoutMs % 1000) * 1000;
    const int maxfd = (int)(ctl_->s > data_->s ? ctl_->s : data_->s);
    const int r = select(maxfd + 1, &rs, nullptr, nullptr, &tv);
    Out out;
    {
        std::lock_guard<std::mutex> g(mu_);
        if (r > 0) {
            uint8_t buf[2048]; std::string ip; uint16_t port;
            for (int which = 0; which < 2; ++which) {
                Udp* u = which ? data_ : ctl_;
                if (!FD_ISSET(u->s, &rs)) continue;
                for (int k = 0; k < 64; ++k) {
                    const int n = udp_recv(u, buf, sizeof buf, &ip, &port);
                    if (n < 0) break;
                    handle(which, buf, size_t(n), ip, port, out);
                }
            }
        }
        timers(out);
        peers_.erase(std::remove_if(peers_.begin(), peers_.end(), [](const Peer& p) { return p.gone; }), peers_.end());
    }
    deliver(out);
}

void Session::handle(int which, const uint8_t* p, size_t n, const std::string& ip, uint16_t port, Out& out) {
    if (am_is_session(p, n)) {
        AmPacket a;
        if (!am_parse(p, n, &a)) {
            out.logs.push_back({fmt("RTP-MIDI %s <- %s:%u short session packet [%s]", which ? "data" : "ctl ",
                                    ip.c_str(), port, hex(p, n).c_str()), true});
            return;
        }
        if (cfg_.logPackets)
            out.logs.push_back({fmt("RTP-MIDI %s <- %s:%u  %s", which ? "data" : "ctl ", ip.c_str(), port,
                                    describe(a).c_str()), true});
        handle_session(which, a, ip, port, out);
        return;
    }
    if (which == 1 && rtp_is_midi(p, n)) {
        handle_midi(p, n, ip, port, out);
        return;
    }
    out.logs.push_back({fmt("RTP-MIDI %s <- %s:%u unrecognised datagram (%u bytes)", which ? "data" : "ctl ",
                            ip.c_str(), port, (unsigned)n), true});
}

void Session::handle_session(int which, const AmPacket& a, const std::string& ip, uint16_t port, Out& out) {
    const uint64_t now = now10k();
    switch (a.cmd) {
    case AmCmd::In: {
        if (which == 0) {
            // A peer invites us. The same SSRC again (it restarted its side), or a peer we
            // are inviting at this address that got in first: that entry takes the invitation.
            Peer* p = by_ssrc(a.ssrc, ip);
            if (!p)
                for (Peer& q : peers_)
                    if (!q.gone && q.v.invited && q.v.ip == ip && q.v.state != PeerState::Connected &&
                        (q.v.target == port || q.v.ctlPort == port)) { p = &q; break; }
            if (!p) {
                if (live() >= cfg_.maxPeers) {
                    send_to(0, ip, port, am_invite(AmCmd::No, a.token, ssrc_, cfg_.name), "NO", out);
                    out.logs.push_back({fmt("RTP-MIDI: refused '%s' (%s): the session already has %u peers",
                                            a.name.c_str(), ip.c_str(), cfg_.maxPeers), false});
                    return;
                }
                peers_.push_back(Peer());
                p = &peers_.back();
                p->v.ip = ip;
            }
            p->token = a.token;
            p->v.ssrc = a.ssrc;
            p->v.name = a.name;
            p->v.ctlPort = port;
            p->v.dataPort = uint16_t(port + 1);
            p->haveSeq = false; p->cycles = 0; p->fed = 0;
            p->acceptBy = now + uint64_t(cfg_.acceptMs) * 10;
            p->lastHeard = now; p->v.lastHeardMs = now / 10;
            send_to(0, ip, port, am_invite(AmCmd::Ok, a.token, ssrc_, cfg_.name), "OK", out);
            p->v.state = PeerState::Waiting;   // so the change below is logged even on a re-invite
            set_state(*p, PeerState::Accepting, "invited our control port", out);
        } else {
            Peer* p = nullptr;
            for (Peer& q : peers_)
                if (!q.gone && q.v.state == PeerState::Accepting && q.token == a.token && q.v.ip == ip) { p = &q; break; }
            if (!p) {
                send_to(1, ip, port, am_invite(AmCmd::No, a.token, ssrc_, cfg_.name), "NO", out);
                out.logs.push_back({fmt("RTP-MIDI: data-port invitation from %s:%u without one on the control port - refused",
                                        ip.c_str(), port), true});
                return;
            }
            p->v.dataPort = port;
            send_to(1, ip, port, am_invite(AmCmd::Ok, a.token, ssrc_, cfg_.name), "OK", out);
            p->lastHeard = now; p->v.lastHeardMs = now / 10;
            p->nextRs = now + kSec;
            set_state(*p, PeerState::Connected, "", out);
        }
        return;
    }
    case AmCmd::Ok: {
        Peer* p = nullptr;
        for (Peer& q : peers_)
            if (!q.gone && q.v.invited && q.token == a.token && q.v.ip == ip) { p = &q; break; }
        if (!p) return;
        p->lastHeard = now; p->v.lastHeardMs = now / 10;
        if (which == 0 && p->v.state == PeerState::Inviting) {
            p->v.ssrc = a.ssrc;
            p->v.name = a.name;
            p->v.ctlPort = port;
            p->v.dataPort = uint16_t(port + 1);
            p->tries = 0;
            p->nextAt = now;
            set_state(*p, PeerState::InvitingData, "control port accepted", out);
        } else if (which == 1 && p->v.state == PeerState::InvitingData) {
            p->v.dataPort = port;
            p->nextCk = now; p->ckSent = 0; p->ckMissed = 0; p->ckTs1 = 0;
            p->nextRs = now + kSec;
            set_state(*p, PeerState::Connected, "", out);
        }
        return;
    }
    case AmCmd::No: {
        for (Peer& q : peers_)
            if (!q.gone && q.v.invited && q.token == a.token && q.v.ip == ip &&
                (q.v.state == PeerState::Inviting || q.v.state == PeerState::InvitingData)) {
                q.nextAt = now + uint64_t(cfg_.retryMs) * 10;
                set_state(q, PeerState::Waiting, "it refused the invitation", out);
            }
        return;
    }
    case AmCmd::By: {
        Peer* p = by_ssrc(a.ssrc, ip);
        if (!p)   // a BY before the SSRC was known (it cancelled our invitation)
            for (Peer& q : peers_)
                if (!q.gone && q.token == a.token && q.v.ip == ip) { p = &q; break; }
        if (!p) return;
        if (p->v.invited) {
            p->nextAt = now + uint64_t(cfg_.byeRetryMs) * 10;
            set_state(*p, PeerState::Waiting, "it said goodbye", out);
        } else {
            p->gone = true;
            out.logs.push_back({"RTP-MIDI: " + p->v.display() + " (" + ip + ") left the session", false});
        }
        return;
    }
    case AmCmd::Ck: {
        Peer* p = by_ssrc(a.ssrc, ip);
        if (!p || p->v.state != PeerState::Connected) return;
        p->lastHeard = now; p->v.lastHeardMs = now / 10;
        if (a.count == 0) {
            send_to(1, ip, port, am_clock(ssrc_, 1, a.ts[0], now, 0), "CK1", out);
        } else if (a.count == 1) {
            if (p->ckTs1 && a.ts[0] == p->ckTs1) {
                send_to(1, ip, port, am_clock(ssrc_, 2, a.ts[0], a.ts[1], now), "CK2", out);
                p->v.rttMs = double(now - a.ts[0]) / 10.0;
                p->ckTs1 = 0; p->ckMissed = 0;
            }
        } else if (a.count == 2) {
            if (a.ts[2] >= a.ts[0]) p->v.rttMs = double(a.ts[2] - a.ts[0]) / 10.0;
        }
        return;
    }
    case AmCmd::Rs: case AmCmd::Rl: {
        if (Peer* p = by_ssrc(a.ssrc, ip)) { p->lastHeard = now; p->v.lastHeardMs = now / 10; }
        return;
    }
    default:
        return;
    }
}

void Session::handle_midi(const uint8_t* p, size_t n, const std::string& ip, uint16_t port, Out& out) {
    RtpMidiPacket m;
    std::string err;
    const bool ok = rtp_parse(p, n, &m, &err);
    Peer* peer = by_ssrc(m.ssrc, ip);
    if (!peer || peer->v.state != PeerState::Connected) {
        if (strangers_.size() < 64 && strangers_.insert(m.ssrc).second)
            out.logs.push_back({fmt("RTP-MIDI: MIDI from %s:%u (SSRC 0x%08X), which is not in the session - ignored",
                                    ip.c_str(), port, m.ssrc), false});
        return;
    }
    const uint64_t now = now10k();
    peer->lastHeard = now;
    peer->v.lastHeardMs = now / 10;
    if (peer->haveSeq) {
        const uint16_t gap = uint16_t(m.seq - peer->seqIn - 1);
        if (gap && gap < 1000) peer->v.lost += gap;
        if (m.seq < peer->seqIn && uint16_t(m.seq - peer->seqIn) < 1000) ++peer->cycles;   // wrapped
    }
    peer->haveSeq = true;
    peer->seqIn = m.seq;
    ++peer->v.midi;
    if (!ok)
        out.logs.push_back({fmt("RTP-MIDI: a packet from %s did not decode (%s); %u message(s) before the fault kept",
                                peer->v.display().c_str(), err.c_str(), (unsigned)m.events.size()), false});
    if (m.events.empty()) return;   // a journal-only packet
    peer->v.lastMidiMs = now / 10;
    for (const MidiEvent& e : m.events) out.midi.push_back({peer->v, e.bytes});
}

void Session::timers(Out& out) {
    const uint64_t now = now10k();
    for (Peer& p : peers_) {
        if (p.gone) continue;
        const PeerState s = p.v.state;
        if (p.v.invited && (s == PeerState::Inviting || s == PeerState::InvitingData)) {
            if (now < p.nextAt) continue;
            if (p.tries >= cfg_.inviteTries) {
                p.nextAt = now + uint64_t(cfg_.retryMs) * 10;
                set_state(p, PeerState::Waiting,
                          std::string("no answer from its ") + (s == PeerState::Inviting ? "control" : "data") + " port", out);
                continue;
            }
            ++p.tries;
            const bool ctl = s == PeerState::Inviting;
            send_to(ctl ? 0 : 1, p.v.ip, ctl ? p.v.ctlPort : p.v.dataPort, am_invite(AmCmd::In, p.token, ssrc_, cfg_.name),
                    "IN", out);
            p.nextAt = now + kSec;
            continue;
        }
        if (p.v.invited && s == PeerState::Waiting) {
            if (now >= p.nextAt) {
                start_attempt(p);
                set_state(p, PeerState::Inviting, "", out);
            }
            continue;
        }
        if (s == PeerState::Accepting) {
            if (now > p.acceptBy) {
                if (p.v.invited) {
                    p.nextAt = now + uint64_t(cfg_.retryMs) * 10;
                    set_state(p, PeerState::Waiting, "it never invited our data port", out);
                } else {
                    p.gone = true;
                    out.logs.push_back({"RTP-MIDI: " + p.v.display() + " (" + p.v.ip +
                                            ") never invited our data port - dropped", false});
                }
            }
            continue;
        }
        if (s != PeerState::Connected) continue;

        if (p.v.invited && now >= p.nextCk) {
            // We invited it, so we run clock sync: a burst at the start, then a keep-alive.
            if (p.ckTs1) ++p.ckMissed;
            if (p.ckMissed >= 4 && now - p.lastHeard > 30 * kSec) {
                p.nextAt = now + 2 * kSec;
                set_state(p, PeerState::Waiting, "it stopped answering", out);
                continue;
            }
            ++p.ckSent;
            p.ckTs1 = now;
            send_to(1, p.v.ip, p.v.dataPort, am_clock(ssrc_, 0, now, 0, 0), "CK0", out);
            p.nextCk = now + (p.ckSent < 6 ? 15000 : 10 * kSec);
        }
        if (!p.v.invited && now - p.lastHeard > uint64_t(cfg_.silentMs) * 10) {
            p.gone = true;
            out.logs.push_back({"RTP-MIDI: nothing from " + p.v.display() + " (" + p.v.ip + ") for " +
                                    std::to_string(cfg_.silentMs / 1000) + " s - dropped", false});
            continue;
        }
        if (now >= p.nextRs) {
            p.nextRs = now + kSec;
            const uint32_t ext = (p.cycles << 16) | p.seqIn;
            if (p.haveSeq && ext != p.fed) {
                p.fed = ext;
                send_to(0, p.v.ip, p.v.ctlPort, am_feedback(ssrc_, ext), "RS", out);
            }
        }
    }
}

}  // namespace rtpmidi
