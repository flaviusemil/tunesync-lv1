// engine.h - MIDI in, Tunes set: the whole behaviour without Windows or LV1.
//
// Threads: on_midi / apply_now / request_scan / configure come from any thread
// (a WinMM callback, the RTP-MIDI thread, the settings window) and only queue.
// tick() runs on ONE worker thread and is the only caller into the Mixer, so LV1
// is never entered from two threads and never from LV1's own UI thread.
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <functional>
#include <mutex>
#include <string>
#include <vector>
#include "apply.h"
#include "log.h"
#include "midi_map.h"
#include "settings.h"

namespace tr {

class Engine {
  public:
    Engine(Mixer& m, LogRing& log, std::vector<TrackRange> ranges = default_ranges());

    // Called whenever there is new work for tick(): the host's worker waits on it.
    void set_wake(std::function<void()> f) { wake_ = std::move(f); }

    void configure(const Settings& s);
    Settings settings() const;

    // One whole MIDI message from source ("MIDI <port>" / "RTP <peer>").
    void on_midi(const std::string& source, const uint8_t* msg, size_t n, uint64_t nowMs);
    // The settings window's Apply now.
    void apply_now(const Intent& in, uint64_t nowMs);
    // Refresh the Tunes list (the window opened).
    void request_scan();

    // Worker thread: apply a due change, run a requested scan, log ignored
    // messages. Returns ms until it wants to run again, -1 = only when woken.
    int64_t tick(uint64_t nowMs);

    struct Status {
        uint64_t    lastMidiMs = 0;    // nowMs of the last message, 0 = none yet
        std::string lastSource, lastResult;
        size_t      tunes = 0, locked = 0;
    };
    Status status() const;
    std::vector<TuneInstance> tunes() const;   // from the last scan or apply

    static constexpr uint64_t kIgnoredLogMs = 5000;   // at most one "ignored" line per 5 s

  private:
    void wake() { if (wake_) wake_(); }
    // ms until an "ignored" line may be logged (0 = now), with mu_ held.
    int64_t ignored_in(uint64_t nowMs) const {
        if (lastIgnoredLog_ == kNever || nowMs - lastIgnoredLog_ >= kIgnoredLogMs) return 0;
        return (int64_t)(lastIgnoredLog_ + kIgnoredLogMs - nowMs);
    }
    static constexpr uint64_t kNever = ~0ull;

    Mixer&                    mixer_;
    LogRing&                  log_;
    std::vector<TrackRange>   ranges_;
    std::function<void()>     wake_;

    mutable std::mutex        mu_;          // everything below
    Settings                  settings_;
    Coalescer                 pending_;
    bool                      scanWanted_ = false;
    size_t                    ignored_ = 0;
    std::string               lastIgnored_;
    uint64_t                  lastIgnoredLog_ = kNever;
    Status                    status_;
    std::vector<TuneInstance> tunes_;
};

}  // namespace tr
