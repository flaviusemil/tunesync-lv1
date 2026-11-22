// dnssd.h - Windows' own DNS-SD (dnsapi DnsServiceRegister / DnsServiceBrowse, Windows
// 10 1809 and later), so no Bonjour install is needed:
//
//   advertise   _apple-midi._udp under the session name and port while RTP-MIDI is on:
//               a Mac lists "Tune Remote" in Audio MIDI Setup -> MIDI Network Setup ->
//               Directory by itself and connects with one click.
//   browse      the Macs' own network sessions, for "Found on the network" in the window
//               (Tune Remote then invites the one you tap).
//
// The functions are looked up at run time; on a Windows without them both report
// "not available" and RTP-MIDI still works by typing this host's address on the Mac.
// Callbacks come on dnsapi's threads and only touch this object's state under its lock.
#pragma once
#include <stdint.h>
#include <functional>
#include <mutex>
#include <string>
#include <vector>

namespace trw {

struct FoundSession {
    std::string instance;   // "Studio-MacBook" (the advertised session name)
    std::string host;       // "Studio-MacBook.local"
    std::string ip;         // "" until resolved
    uint16_t    port = 0;   // its control port
};

class Dnssd {
  public:
    using LogFn = std::function<void(const std::string& line, bool detail)>;

    Dnssd() = default;
    ~Dnssd() { stop(); }

    // Advertise `name` on `port` and browse for others. Returns at once.
    void start(const std::string& name, uint16_t port, LogFn log);
    // Deregister and cancel, waiting (bounded) for dnsapi to let go of our callbacks.
    void stop();

    std::vector<FoundSession> found() const;
    bool available() const;          // the API exists on this Windows
    std::string advert_state() const;   // "advertised", "not available", "failed (error n)", ...

  private:
    struct State;
    State* st_ = nullptr;
};

}  // namespace trw
