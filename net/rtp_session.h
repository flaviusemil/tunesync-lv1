// rtp_session.h - one AppleMIDI network session that several peers join at once, the
// way a Mac's own network session does: one port pair, each participant told apart by
// its SSRC. A main and a backup playback rig can both be connected; MIDI from any of
// them is delivered with the peer it came from.
//
// Both roles, per peer:
//   the peer invites us   IN on the control port, then on the data port; we answer OK.
//                         It runs clock sync; we answer CK0 with CK1. Dropped after BY or
//                         60 s of silence.
//   we invite the peer    Connect in the window, or a remembered peer. IN to its control
//                         port, then its data port, one a second; then we run clock sync.
//                         Refused, unanswered or gone quiet: invited again later. After a
//                         BY we wait longer (30 s) before inviting it again.
// Receiver feedback (RS) goes to every connected peer's control port once a second while
// its sequence moves.
//
// The interop rules learned with preprobe hold here:
// replies always go to the port a packet came from (rtpmidid answers the data invitation
// from control + 1, a NAT may remap it), RS carries the 32-bit extended sequence, and an
// invited peer may run its own clock sync, which we answer.
//
// Threads: open / close / poll belong to ONE thread (the RTP thread). invite / forget /
// send / peers may be called from any thread; they never wait on the network. Callbacks
// run on the poll thread, outside the session's lock.
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <functional>
#include <mutex>
#include <set>
#include <string>
#include <vector>
#include "rtp_codec.h"

namespace rtpmidi {

struct Udp;

enum class PeerState {
    Accepting,     // it invited our control port; waiting for its data-port invitation
    Inviting,      // we sent IN to its control port
    InvitingData,  // control accepted, IN sent to its data port
    Connected,     // both ports accepted: MIDI flows
    Waiting,       // we invite it, and will again (refused, no answer, BY, silent)
};
const char* peer_state_name(PeerState s);

struct PeerView {
    uint32_t    ssrc = 0;          // 0 until it answered
    std::string name;              // its own session name, "" until it answered
    std::string label;             // what it was invited as (the name it was found under)
    std::string ip;
    uint16_t    target = 0;        // invited: the control port we were asked to invite
    uint16_t    ctlPort = 0, dataPort = 0;
    PeerState   state = PeerState::Accepting;
    bool        invited = false;   // we invite it; else it invited us
    uint64_t    lastHeardMs = 0;   // on the session clock (now_ms), 0 = never
    uint64_t    lastMidiMs = 0;
    unsigned    midi = 0, lost = 0;
    double      rttMs = -1;
    // How the window and the log name it.
    std::string display() const { return !name.empty() ? name : !label.empty() ? label : ip; }
};

struct SessionConfig {
    std::string name = "Tune Remote";   // our session name, what the peers show
    uint16_t    port = 5004;            // control port; data = port + 1
    unsigned    maxPeers = 8;
    unsigned    inviteTries = 12;       // per attempt, one a second (what macOS does)
    unsigned    retryMs = 5000;         // after a refusal or no answer
    unsigned    byeRetryMs = 30000;     // after a peer we invited said goodbye
    unsigned    silentMs = 60000;       // a peer that invited us and went quiet is dropped
    unsigned    acceptMs = 10000;       // ...or never invited our data port
    bool        logPackets = false;     // every session packet, as detail lines
    // The session clock in 100 us units, for tests. Default: the steady clock.
    std::function<uint64_t()> clock10k;
};

class Session {
  public:
    // detail = packet-level noise, for the file log only.
    using LogFn  = std::function<void(const std::string& line, bool detail)>;
    // One whole MIDI message (running status expanded) from a connected peer.
    using MidiFn = std::function<void(const PeerView& from, const uint8_t* msg, size_t n)>;

    Session();
    ~Session();
    Session(const Session&) = delete;
    Session& operator=(const Session&) = delete;

    void on_log(LogFn f) { log_ = std::move(f); }     // before open()
    void on_midi(MidiFn f) { midi_ = std::move(f); }

    // Binds control on cfg.port and data on cfg.port + 1, every interface.
    bool open(const SessionConfig& cfg, std::string* err);
    // BY to every connected peer, then the sockets close. The peer table is cleared.
    void close();
    bool is_open() const;
    // Waits up to timeoutMs for datagrams, handles them, runs the timers.
    void poll(int timeoutMs);

    // We invite ip:port (its control port). A peer already there is invited again now.
    void invite(const std::string& ip, uint16_t port, const std::string& label);
    // Stop inviting ip:port; BY first when it is connected.
    void forget(const std::string& ip, uint16_t port);
    // MIDI to every connected peer (tests, the sender tool). Returns how many got it.
    size_t send(const std::vector<std::vector<uint8_t>>& msgs);

    std::vector<PeerView> peers() const;
    uint16_t port() const;
    uint64_t now_ms() const { return now10k() / 10; }

  private:
    struct Peer {
        PeerView v;
        uint32_t token = 0;            // the initiator's token (ours when we invite)
        unsigned tries = 0;
        uint64_t nextAt = 0;           // next IN / next attempt
        uint64_t acceptBy = 0;         // Accepting: drop when the data IN has not come by then
        uint64_t lastHeard = 0;
        uint64_t nextCk = 0, ckTs1 = 0;
        unsigned ckSent = 0, ckMissed = 0;
        bool     haveSeq = false;
        uint16_t seqIn = 0;
        uint32_t cycles = 0, fed = 0;
        uint64_t nextRs = 0;
        bool     gone = false;
    };
    struct Out {                       // collected under the lock, delivered after it
        std::vector<std::pair<std::string, bool>> logs;
        std::vector<std::pair<PeerView, std::vector<uint8_t>>> midi;
    };

    uint64_t now10k() const;
    void handle(int which, const uint8_t* p, size_t n, const std::string& ip, uint16_t port, Out& out);
    void handle_session(int which, const AmPacket& a, const std::string& ip, uint16_t port, Out& out);
    void handle_midi(const uint8_t* p, size_t n, const std::string& ip, uint16_t port, Out& out);
    void timers(Out& out);
    void start_attempt(Peer& p);
    void set_state(Peer& p, PeerState s, const std::string& why, Out& out);
    void send_to(int which, const std::string& ip, uint16_t port, const std::vector<uint8_t>& pkt, const char* what,
                 Out& out);
    Peer* by_ssrc(uint32_t ssrc, const std::string& ip);
    Peer* invited_at(const std::string& ip, uint16_t target);
    size_t live() const;
    static bool knows_us(const Peer& p);
    void deliver(Out& out);

    SessionConfig cfg_;
    Udp*          ctl_ = nullptr;
    Udp*          data_ = nullptr;
    uint64_t      t0_ = 0;
    uint32_t      ssrc_ = 0;
    uint32_t      seqOut_ = 0;

    mutable std::mutex mu_;             // everything below, and every send
    std::vector<Peer>  peers_;
    std::set<uint32_t> strangers_;      // SSRCs whose MIDI was refused and logged once

    LogFn  log_;
    MidiFn midi_;
};

}  // namespace rtpmidi
