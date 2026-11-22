// net_test.cpp - net/: the RTP-MIDI codec (golden bytes and a fuzz, carried over from
// Headlink's preprobe_test) and the multi-peer session on loopback UDP. The fake peers
// are written from the wire format with raw bytes, NOT with the codec, so a mistake in
// the codec cannot cancel itself out on both ends. `make test`.
#include <stdio.h>
#include <string.h>
#include <chrono>
#include <functional>
#include <memory>
#include <random>
#include <string>
#include <vector>
#include "../net/rtp_codec.h"
#include "../net/rtp_session.h"

#include <arpa/inet.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>

using namespace rtpmidi;
typedef std::vector<uint8_t> Bytes;

static int g_fail = 0, g_checks = 0;
#define CHECK(c, ...) do { ++g_checks; if (!(c)) { ++g_fail; printf("FAIL %s:%d  %s  ", __FILE__, __LINE__, #c); printf(__VA_ARGS__); printf("\n"); } } while (0)
#define CHECK_BYTES(got, ...) do { const Bytes want_ = __VA_ARGS__; ++g_checks; if ((got) != want_) { ++g_fail; \
    printf("FAIL %s:%d  bytes\n   got  %s\n   want %s\n", __FILE__, __LINE__, hex(got).c_str(), hex(want_).c_str()); } } while (0)

// ================================================================ codec (from preprobe_test)

static void test_session_packets() {
    CHECK_BYTES(am_invite(AmCmd::In, 0x01020304, 0x0A0B0C0D, "abc"),
                {0xFF, 0xFF, 'I', 'N', 0, 0, 0, 2, 1, 2, 3, 4, 0x0A, 0x0B, 0x0C, 0x0D, 'a', 'b', 'c', 0});
    CHECK_BYTES(am_invite(AmCmd::Ok, 0x01020304, 0x0A0B0C0D, ""),
                {0xFF, 0xFF, 'O', 'K', 0, 0, 0, 2, 1, 2, 3, 4, 0x0A, 0x0B, 0x0C, 0x0D, 0});
    CHECK_BYTES(am_bye(0x01020304, 0x0A0B0C0D), {0xFF, 0xFF, 'B', 'Y', 0, 0, 0, 2, 1, 2, 3, 4, 0x0A, 0x0B, 0x0C, 0x0D});
    CHECK_BYTES(am_feedback(0x0A0B0C0D, 0x0001BEEF), {0xFF, 0xFF, 'R', 'S', 0x0A, 0x0B, 0x0C, 0x0D, 0x00, 0x01, 0xBE, 0xEF});
    CHECK_BYTES(am_clock(0x0A0B0C0D, 1, 0x0102030405060708ull, 0x1112131415161718ull, 0),
                {0xFF, 0xFF, 'C', 'K', 0x0A, 0x0B, 0x0C, 0x0D, 1, 0, 0, 0,
                 1, 2, 3, 4, 5, 6, 7, 8, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0, 0, 0, 0, 0, 0, 0, 0});
    AmPacket a;
    const Bytes in = am_invite(AmCmd::In, 0xCAFEBABE, 0x12345678, "Playback");
    CHECK(am_parse(in.data(), in.size(), &a), "IN parses");
    CHECK(a.cmd == AmCmd::In && a.version == 2 && a.token == 0xCAFEBABE && a.ssrc == 0x12345678 && a.name == "Playback", "IN fields");
    Bytes noNul = in; noNul.pop_back();
    CHECK(am_parse(noNul.data(), noNul.size(), &a) && a.name == "Playback", "name without NUL");
    CHECK(am_parse(in.data(), 16, &a) && a.name.empty(), "no name");
    CHECK(!am_parse(in.data(), 15, &a), "short IN");
    const Bytes ck = am_clock(0x01, 2, 100, 200, 300);
    CHECK(am_parse(ck.data(), ck.size(), &a) && a.cmd == AmCmd::Ck && a.count == 2 && a.ts[2] == 300, "CK");
    CHECK(!am_parse(ck.data(), 35, &a), "short CK");
    const Bytes rsRtpmidid = {0xFF, 0xFF, 0x52, 0x53, 0x00, 0x00, 0x5C, 0x35, 0x00, 0x00, 0x00, 0x02};
    CHECK(am_parse(rsRtpmidid.data(), rsRtpmidid.size(), &a) && a.seq == 2, "rtpmidid RS");
}

static Bytes rtp(const Bytes& section) {
    Bytes v = {0x80, 0x61, 0x00, 0x05, 0, 0, 0x10, 0, 0x50, 0x52, 0x45, 0x21};
    v.insert(v.end(), section.begin(), section.end());
    return v;
}

static void test_rtp() {
    CHECK_BYTES(rtp_build(0x1234, 0x01020304, 0xAABBCCDD, {{0x9F, 0x39, 0x64}}),
                {0x80, 0x61, 0x12, 0x34, 1, 2, 3, 4, 0xAA, 0xBB, 0xCC, 0xDD, 0x03, 0x9F, 0x39, 0x64});
    CHECK_BYTES(rtp_build(1, 2, 3, {{0xBF, 20, 3}, {0x9F, 57, 100}}),
                {0x80, 0x61, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0x07, 0xBF, 20, 3, 0x00, 0x9F, 57, 100});
    RtpMidiPacket m; std::string err;
    // What macOS sends: a journal on every packet.
    Bytes b = rtp({0x43, 0xB0, 0x49, 0x0F, 0x20, 0x00, 0x04, 0x00, 0x08, 0x0A, 0x80, 0x49, 0x0F});
    CHECK(rtp_parse(b.data(), b.size(), &m, &err) && m.j && m.events.size() == 1 && m.journalLen == 9, "journal skipped");
    b = rtp({0x06, 0xB0, 0x07, 0x64, 0x00, 0x08, 0x65});
    CHECK(rtp_parse(b.data(), b.size(), &m, &err) && m.events.size() == 2 && m.events[1].running &&
          m.events[1].bytes == (Bytes{0xB0, 0x08, 0x65}), "running status");
    b = rtp({0x25, 0x81, 0x00, 0xB0, 0x07, 0x64});
    CHECK(rtp_parse(b.data(), b.size(), &m, &err) && m.z && m.events[0].delta == 128, "Z delta");
    const Bytes bad[] = {rtp({0x05, 0xB0, 0x07, 0x64}), rtp({0x03, 0x07, 0x64, 0x00}), rtp({0x03, 0xB0, 0x07}),
                         rtp({0x04, 0xF0, 0x01, 0x02, 0x03}), rtp({}), Bytes{0x80, 0x61, 0, 0}};
    for (const Bytes& x : bad) CHECK(!rtp_parse(x.data(), x.size(), &m, &err), "malformed refused");
    // Fuzz: must never crash or read out of bounds (make asan makes that bite).
    std::mt19937 rng(1234);
    const Bytes seed = rtp({0x46, 0xB0, 0x07, 0x64, 0x00, 0x08, 0x65, 0x20, 0x00, 0x04});
    bool allStatus = true;
    for (int it = 0; it < 100000; ++it) {
        Bytes x = seed;
        if (it & 1) {
            x.resize(rng() % 40);
            for (auto& c : x) c = uint8_t(rng());
            if (x.size() > 2) { x[0] = 0x80; x[1] = 0x61; }
        } else {
            for (int k = 0; k < 3; ++k) x[12 + rng() % (x.size() - 12)] = uint8_t(rng());
            if (rng() % 4 == 0) x.resize(rng() % x.size());
        }
        RtpMidiPacket p; AmPacket a;
        rtp_parse(x.data(), x.size(), &p, &err);
        for (const auto& e : p.events) allStatus &= !e.bytes.empty() && (e.bytes[0] & 0x80);
        if (x.size() > 1) { x[0] = 0xFF; x[1] = 0xFF; }
        am_parse(x.data(), x.size(), &a);
    }
    CHECK(allStatus, "every decoded event starts with a status byte");
}

// ================================================================ loopback helpers

namespace {
struct Sock {
    int s = -1;
    uint16_t port = 0;
    explicit Sock(uint16_t p) {
        s = socket(AF_INET, SOCK_DGRAM, 0);
        sockaddr_in a{}; a.sin_family = AF_INET; a.sin_port = htons(p); a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        if (bind(s, (sockaddr*)&a, sizeof a) != 0) { ::close(s); s = -1; return; }
        fcntl(s, F_SETFL, fcntl(s, F_GETFL, 0) | O_NONBLOCK);
        port = p;
    }
    ~Sock() { if (s >= 0) ::close(s); }
    bool ok() const { return s >= 0; }
    void send(uint16_t to, const Bytes& b) {
        sockaddr_in a{}; a.sin_family = AF_INET; a.sin_port = htons(to); a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        sendto(s, (const char*)b.data(), b.size(), 0, (sockaddr*)&a, sizeof a);
    }
    bool recv(Bytes* b, uint16_t* from) {
        uint8_t buf[2048]; sockaddr_in a{}; socklen_t al = sizeof a;
        const ssize_t n = recvfrom(s, buf, sizeof buf, 0, (sockaddr*)&a, &al);
        if (n < 0) return false;
        b->assign(buf, buf + n); *from = ntohs(a.sin_port);
        return true;
    }
};

uint32_t be32(const Bytes& b, size_t o) { return uint32_t(b[o]) << 24 | uint32_t(b[o + 1]) << 16 | uint32_t(b[o + 2]) << 8 | b[o + 3]; }
uint64_t be64(const Bytes& b, size_t o) { return uint64_t(be32(b, o)) << 32 | be32(b, o + 4); }
void put32(Bytes& b, uint32_t x) { for (int i = 3; i >= 0; --i) b.push_back(uint8_t(x >> (8 * i))); }
void put64(Bytes& b, uint64_t x) { put32(b, uint32_t(x >> 32)); put32(b, uint32_t(x)); }
bool is(const Bytes& b, const char* cmd) { return b.size() >= 4 && b[0] == 0xFF && b[1] == 0xFF && b[2] == uint8_t(cmd[0]) && b[3] == uint8_t(cmd[1]); }
Bytes session(char a, char b, uint32_t token, uint32_t ssrc, const std::string& name, bool withName = true) {
    Bytes r = {0xFF, 0xFF, uint8_t(a), uint8_t(b)}; put32(r, 2); put32(r, token); put32(r, ssrc);
    if (withName) { for (char c : name) r.push_back(uint8_t(c)); r.push_back(0); }
    return r;
}

bool open_pair(std::unique_ptr<Sock>* c, std::unique_ptr<Sock>* d, uint16_t from) {
    for (uint16_t p = from; p < from + 400; p += 2) {
        c->reset(new Sock(p)); d->reset(new Sock(uint16_t(p + 1)));
        if ((*c)->ok() && (*d)->ok()) return true;
    }
    return false;
}

// A free even port pair for a Session under test.
uint16_t free_pair(uint16_t from) {
    for (uint16_t p = from; p < from + 400; p += 2) {
        Sock a(p), b(uint16_t(p + 1));
        if (a.ok() && b.ok()) return p;
    }
    return 0;
}

// A playback rig that invites us (what a Mac's network session does), byte by byte.
struct FakeInitiator {
    std::unique_ptr<Sock> ctl, data;
    uint32_t token, ssrc;
    std::string name;
    uint16_t seq = 100;
    bool okCtl = false, okData = false, refused = false;
    std::vector<uint64_t> ck1Ts1;          // ts1 of every CK1 we got back
    std::vector<uint32_t> feedback;        // RS
    FakeInitiator(uint32_t tok, uint32_t s, const char* n, uint16_t portsFrom) : token(tok), ssrc(s), name(n) {
        open_pair(&ctl, &data, portsFrom);
    }
    void step() {
        Bytes b; uint16_t from;
        while (ctl->recv(&b, &from)) {
            if (is(b, "OK") && be32(b, 8) == token) okCtl = true;
            else if (is(b, "NO")) refused = true;
            else if (is(b, "RS") && b.size() >= 12) feedback.push_back(be32(b, 8));
        }
        while (data->recv(&b, &from)) {
            if (is(b, "OK") && be32(b, 8) == token) okData = true;
            else if (is(b, "NO")) refused = true;
            else if (is(b, "CK") && b.size() == 36 && b[8] == 1) ck1Ts1.push_back(be64(b, 12));
        }
    }
    void invite_ctl(uint16_t sp) { ctl->send(sp, session('I', 'N', token, ssrc, name)); }
    void invite_data(uint16_t sp) { data->send(uint16_t(sp + 1), session('I', 'N', token, ssrc, name)); }
    void ck0(uint16_t sp, uint64_t ts1) {
        Bytes r = {0xFF, 0xFF, 'C', 'K'}; put32(r, ssrc); r.insert(r.end(), {0, 0, 0, 0}); put64(r, ts1); put64(r, 0); put64(r, 0);
        data->send(uint16_t(sp + 1), r);
    }
    void midi(uint16_t sp, const Bytes& msg) {
        Bytes r = {0x80, 0x61, uint8_t(seq >> 8), uint8_t(seq)}; put32(r, 4242); put32(r, ssrc);
        r.push_back(uint8_t(msg.size()));
        r.insert(r.end(), msg.begin(), msg.end());
        // a journal, as macOS sends one on every packet
        r[12] |= 0x40; r.insert(r.end(), {0x20, 0x00, 0x04});
        data->send(uint16_t(sp + 1), r);
        ++seq;
    }
    void bye(uint16_t sp) { ctl->send(sp, session('B', 'Y', token, ssrc, "", false)); }
};

// A session that answers our invitations (a Mac we Connect to), byte by byte.
struct FakeResponder {
    std::unique_ptr<Sock> ctl, data;
    uint32_t ssrc = 0x4D41430A;
    bool refuse = false, silent = false;
    uint32_t token = 0; std::string theirName;
    uint16_t theirCtl = 0, theirData = 0;
    bool ctlOk = false, dataOk = false, ck2 = false, bye = false;
    std::vector<Bytes> midiIn;
    explicit FakeResponder(uint16_t portsFrom) { open_pair(&ctl, &data, portsFrom); }
    void step() {
        Bytes b; uint16_t from;
        while (ctl->recv(&b, &from)) {
            if (silent) continue;
            if (is(b, "IN") && b.size() >= 16) {
                token = be32(b, 8); theirCtl = from;
                theirName.assign(b.begin() + 16, b.end()); if (!theirName.empty() && theirName.back() == 0) theirName.pop_back();
                ctl->send(from, session(refuse ? 'N' : 'O', refuse ? 'O' : 'K', token, ssrc, "Studio-Mac"));
                ctlOk = !refuse;
            } else if (is(b, "BY")) {
                bye = true;
            }
        }
        while (data->recv(&b, &from)) {
            if (silent) continue;
            if (is(b, "IN") && b.size() >= 16 && be32(b, 8) == token && ctlOk) {
                theirData = from;
                data->send(from, session('O', 'K', token, ssrc, "Studio-Mac"));
                dataOk = true;
            } else if (is(b, "CK") && b.size() == 36) {
                if (b[8] == 0) {
                    Bytes r = {0xFF, 0xFF, 'C', 'K'}; put32(r, ssrc); r.insert(r.end(), {1, 0, 0, 0});
                    put64(r, be64(b, 12)); put64(r, 777777); put64(r, 0);
                    data->send(from, r);
                } else if (b[8] == 2) {
                    ck2 = true;
                }
            } else if (b.size() >= 13 && b[0] == 0x80 && b[1] == 0x61) {
                midiIn.push_back(Bytes(b.begin() + 12, b.end()));
            }
        }
    }
    void send_bye() { ctl->send(theirCtl, session('B', 'Y', token, ssrc, "", false)); }
};

bool run_until(const std::function<void()>& step, const std::function<bool()>& done, int ms) {
    const auto end = std::chrono::steady_clock::now() + std::chrono::milliseconds(ms);
    while (std::chrono::steady_clock::now() < end) {
        step();
        if (done()) return true;
    }
    return false;
}

struct Got { std::string peer; Bytes msg; };
}  // namespace

// ================================================================ session tests

// Main and backup rig invite us; both connected, MIDI from each named; clock sync and
// feedback answered; a stranger's MIDI refused; BY drops a peer.
static void test_two_rigs_invite_us() {
    uint64_t fake = 1000;
    Session s;
    std::vector<Got> got;
    std::vector<std::string> log;
    s.on_midi([&](const PeerView& p, const uint8_t* m, size_t n) { got.push_back({p.display(), Bytes(m, m + n)}); });
    s.on_log([&](const std::string& l, bool) { log.push_back(l); });
    SessionConfig c; c.port = free_pair(25004); c.clock10k = [&] { return fake; };
    std::string err;
    CHECK(c.port && s.open(c, &err), "open: %s", err.c_str());
    const uint16_t sp = s.port();

    FakeInitiator mainRig(0x11110001, 0xAAAA0001, "Playback-Main", 26004), backup(0x22220002, 0xBBBB0002, "Playback-Backup", 26404);
    auto step = [&] { s.poll(2); mainRig.step(); backup.step(); };
    mainRig.invite_ctl(sp);
    backup.invite_ctl(sp);
    CHECK(run_until(step, [&] { return mainRig.okCtl && backup.okCtl; }, 1000), "both control invitations accepted");
    mainRig.invite_data(sp);
    backup.invite_data(sp);
    CHECK(run_until(step, [&] { return mainRig.okData && backup.okData; }, 1000), "both data invitations accepted");
    auto ps = s.peers();
    CHECK(ps.size() == 2 && ps[0].state == PeerState::Connected && ps[1].state == PeerState::Connected, "two connected peers");
    CHECK(ps.size() == 2 && ps[0].name == "Playback-Main" && ps[1].name == "Playback-Backup" && !ps[0].invited, "names");

    // Clock sync started by the rig: CK1 carries its ts1 back.
    mainRig.ck0(sp, 5555);
    CHECK(run_until(step, [&] { return !mainRig.ck1Ts1.empty(); }, 1000) && mainRig.ck1Ts1[0] == 5555, "CK0 answered");

    // MIDI from each, named by its peer.
    mainRig.midi(sp, {0xBF, 20, 3});
    backup.midi(sp, {0x9F, 57, 100});
    CHECK(run_until(step, [&] { return got.size() >= 2; }, 1000), "MIDI from both");
    CHECK(got.size() == 2 && got[0].peer == "Playback-Main" && got[0].msg == (Bytes{0xBF, 20, 3}) &&
          got[1].peer == "Playback-Backup" && got[1].msg == (Bytes{0x9F, 57, 100}), "who sent what");

    // Receiver feedback names each rig's last sequence, on its control port.
    fake += 15000;
    CHECK(run_until(step, [&] { return !mainRig.feedback.empty() && !backup.feedback.empty(); }, 1000), "RS to both");
    CHECK(!mainRig.feedback.empty() && mainRig.feedback.back() == 100, "RS = last seq");

    // A stranger (right address, an SSRC nobody joined with) is not a peer: ignored, logged once.
    Bytes stranger = {0x80, 0x61, 0, 1}; put32(stranger, 1); put32(stranger, 0xDEADBEEF);
    stranger.insert(stranger.end(), {0x03, 0x9F, 60, 100});
    mainRig.data->send(uint16_t(sp + 1), stranger);
    mainRig.data->send(uint16_t(sp + 1), stranger);
    run_until(step, [&] { return false; }, 50);
    int strangerLines = 0;
    for (const auto& l : log) strangerLines += l.find("0xDEADBEEF") != std::string::npos;
    CHECK(got.size() == 2 && strangerLines == 1, "stranger ignored, logged once (%d)", strangerLines);

    // A data invitation with no control one first is refused.
    FakeInitiator rogue(0x33330003, 0xCCCC0003, "Rogue", 26804);
    rogue.invite_data(sp);
    CHECK(run_until([&] { s.poll(2); rogue.step(); }, [&] { return rogue.refused; }, 1000), "data IN without control refused");

    // The backup says goodbye: gone. The main rig stays.
    backup.bye(sp);
    CHECK(run_until(step, [&] { return s.peers().size() == 1; }, 1000), "BY drops the peer");
    CHECK(s.peers().size() == 1 && s.peers()[0].name == "Playback-Main", "main stays");

    // The main rig goes quiet for 60 s: dropped.
    fake += 61 * 10000;
    run_until(step, [&] { return s.peers().empty(); }, 500);
    CHECK(s.peers().empty(), "silent peer dropped");
    bool saidSilent = false;
    for (const auto& l : log) saidSilent |= l.find("nothing from Playback-Main") != std::string::npos;
    CHECK(saidSilent, "silence logged");
    s.close();
}

// The session is full: the next rig is refused.
static void test_max_peers() {
    Session s;
    SessionConfig c; c.port = free_pair(27004); c.maxPeers = 1;
    std::string err;
    CHECK(s.open(c, &err), "open");
    FakeInitiator a(1, 0xA1, "A", 27404), b(2, 0xB2, "B", 27804);
    a.invite_ctl(s.port());
    b.invite_ctl(s.port());
    CHECK(run_until([&] { s.poll(2); a.step(); b.step(); }, [&] { return a.okCtl && b.refused; }, 1000), "one in, one refused");
}

// We invite a Mac (Connect): both ports, clock sync, MIDI out; refused and silent peers
// are invited again later; BY makes us wait longer; forget says goodbye.
static void test_we_invite() {
    uint64_t fake = 1000;
    Session s;
    std::vector<std::string> log;
    s.on_log([&](const std::string& l, bool) { log.push_back(l); });
    SessionConfig c; c.port = free_pair(28004); c.clock10k = [&] { return fake; };
    std::string err;
    CHECK(s.open(c, &err), "open");
    FakeResponder mac(28404);
    auto step = [&] { s.poll(2); mac.step(); };
    s.invite("127.0.0.1", mac.ctl->port, "Studio-MacBook");
    CHECK(run_until(step, [&] { return mac.ctlOk; }, 1000), "control invited");
    CHECK(mac.theirCtl == s.port() && mac.theirName == "Tune Remote", "from our control port, with our name");
    CHECK(run_until(step, [&] { return mac.dataOk; }, 1000), "data invited");
    CHECK(mac.theirData == s.port() + 1, "from our data port");
    run_until(step, [&] { return !s.peers().empty() && s.peers()[0].state == PeerState::Connected; }, 500);
    auto ps = s.peers();
    CHECK(ps.size() == 1 && ps[0].state == PeerState::Connected && ps[0].invited && ps[0].name == "Studio-Mac" &&
          ps[0].label == "Studio-MacBook", "connected to the Mac");
    CHECK(run_until(step, [&] { return mac.ck2; }, 1000), "we ran clock sync (CK0, CK1, CK2)");
    CHECK(s.send({{0x9F, 57, 100}}) == 1, "send");
    CHECK(run_until(step, [&] { return !mac.midiIn.empty(); }, 1000), "Mac got MIDI");
    if (!mac.midiIn.empty()) CHECK_BYTES(mac.midiIn[0], {0x03, 0x9F, 57, 100});

    // It says goodbye: we wait 30 s, then invite again.
    mac.ctlOk = mac.dataOk = false;
    mac.send_bye();
    CHECK(run_until(step, [&] { return s.peers()[0].state == PeerState::Waiting; }, 1000), "waiting after BY");
    fake += 10 * 10000;
    run_until(step, [&] { return false; }, 30);
    CHECK(!mac.ctlOk, "not re-invited after 10 s");
    fake += 25 * 10000;
    CHECK(run_until(step, [&] { return mac.dataOk; }, 1000), "re-invited after 30 s");

    // Forget: BY, gone, not invited again.
    s.forget("127.0.0.1", mac.ctl->port);
    CHECK(run_until(step, [&] { return mac.bye; }, 1000), "forget sends BY");
    CHECK(s.peers().empty(), "forgotten");
    mac.ctlOk = false;
    fake += 60 * 10000;
    run_until(step, [&] { return false; }, 30);
    CHECK(!mac.ctlOk, "a forgotten peer is not invited again");

    // A Mac that refuses: waiting, invited again after the retry pause.
    FakeResponder no(28804); no.refuse = true;
    s.invite("127.0.0.1", no.ctl->port, "Refuser");
    auto step2 = [&] { s.poll(2); no.step(); };
    CHECK(run_until(step2, [&] { auto p = s.peers(); return !p.empty() && p[0].state == PeerState::Waiting; }, 1000), "refused -> waiting");
    no.refuse = false;
    fake += 6 * 10000;
    CHECK(run_until(step2, [&] { return no.dataOk; }, 1000), "invited again after the pause");

    // A Mac that never answers: 12 tries a second apart, then waiting.
    FakeResponder mute(29204); mute.silent = true;
    s.invite("127.0.0.1", mute.ctl->port, "Mute");
    auto step3 = [&] { s.poll(1); mute.step(); };
    for (int i = 0; i < 14; ++i) { run_until(step3, [&] { return false; }, 5); fake += 10000; }
    bool waiting = false;
    for (const auto& p : s.peers()) waiting |= p.label == "Mute" && p.state == PeerState::Waiting;
    CHECK(waiting, "no answer -> waiting");
    s.close();
    CHECK(run_until([&] { no.step(); }, [&] { return no.bye; }, 500), "close says goodbye to connected peers");
}

// Two Tune Remote sessions (or a sender tool) talk to each other.
static void test_session_to_session() {
    Session a, b;
    std::vector<Got> got;
    b.on_midi([&](const PeerView& p, const uint8_t* m, size_t n) { got.push_back({p.display(), Bytes(m, m + n)}); });
    SessionConfig ca; ca.name = "Sender"; ca.port = free_pair(30004);
    std::string err;
    CHECK(a.open(ca, &err), "open a");
    SessionConfig cb; cb.name = "Tune Remote"; cb.port = free_pair(uint16_t(ca.port + 2));
    CHECK(b.open(cb, &err), "open b: %s", err.c_str());
    a.invite("127.0.0.1", b.port(), "");
    auto step = [&] { a.poll(1); b.poll(1); };
    CHECK(run_until(step, [&] { auto p = a.peers(); return !p.empty() && p[0].state == PeerState::Connected; }, 2000), "a connected");
    CHECK(run_until(step, [&] { auto p = b.peers(); return !p.empty() && p[0].state == PeerState::Connected; }, 2000), "b connected");
    CHECK(a.send({{0xBF, 20, 2}, {0x9F, 62, 90}}) == 1, "sent");
    CHECK(run_until(step, [&] { return got.size() >= 2; }, 1000), "received");
    CHECK(got.size() == 2 && got[0].peer == "Sender" && got[1].msg == (Bytes{0x9F, 62, 90}), "both messages, named");
    // Opening a second session on the same port fails cleanly.
    Session c;
    SessionConfig cc = cb;
    CHECK(!c.open(cc, &err) && err.find("in use") != std::string::npos, "port in use: %s", err.c_str());
    a.close();
    CHECK(run_until(step, [&] { return b.peers().empty(); }, 1000), "a's goodbye reaches b");
}

int main() {
    test_session_packets();
    test_rtp();
    test_two_rigs_invite_us();
    test_max_peers();
    test_we_invite();
    test_session_to_session();
    printf("net: %d checks, %d failed\n", g_checks, g_fail);
    return g_fail ? 1 : 0;
}
