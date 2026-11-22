// midi_in.h - one WinMM MIDI input, chosen by NAME and kept open from its own thread.
//
// Windows renumbers MIDI devices when one comes or goes, so the name is the only stable
// handle: the port is reopened whenever a port of that name reappears (a USB interface
// replugged, a network MIDI driver reconnecting). Every WinMM call happens on the
// worker; the WinMM callback only queues bytes (its rules allow little else), and the
// worker delivers whole messages.
//
// Built from Headlink's tools/preprobe/midiport.cpp (demo/pre-probe): input only here.
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <atomic>
#include <condition_variable>
#include <deque>
#include <functional>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace trw {

// A byte stream as whole messages: running status expanded, realtime bytes inside a
// message come out on their own, an incomplete message at the end is dropped.
std::vector<std::vector<uint8_t>> split_midi(const uint8_t* p, size_t n);

class MidiIn {
  public:
    using MidiFn = std::function<void(const std::string& port, const uint8_t* msg, size_t n)>;
    using LogFn  = std::function<void(const std::string& line)>;

    MidiIn() = default;
    ~MidiIn() { stop(2000); }

    void start(MidiFn midi, LogFn log);
    void stop(unsigned waitMs);                  // closes the port, joins the worker (bounded)

    void want(const std::string& name);          // "" = none. Any thread; never waits on a driver.
    std::string wanted() const;
    std::vector<std::string> ports() const;      // the inputs as of the last rescan (every 2 s)
    std::string open_name() const;               // "" when closed
    uint64_t last_ms() const { return lastMs_.load(); }   // now_ms() of the last message, 0 = none

    void incoming(const uint8_t* p, size_t n);   // from the WinMM callback

  private:
    void run();
    void* impl_ = nullptr;
    std::thread thr_;
    std::atomic<bool> run_{false};
    std::atomic<uint64_t> lastMs_{0};
    mutable std::mutex mu_;
    std::condition_variable cv_;
    std::string want_, open_;
    bool changed_ = false;
    std::vector<std::string> ports_;
    std::deque<std::vector<uint8_t>> inQ_;
    MidiFn midi_;
    LogFn log_;
};

}  // namespace trw
