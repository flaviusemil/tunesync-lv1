// app.h - Tune Remote inside LV1: owns the engine, the MIDI input, the RTP-MIDI session,
// the settings file, the file log and the settings window, and their threads.
//
// Threads (none of them is LV1's; LV1's threads only ever post to these and return):
//   worker   the engine's tick(): the ONLY thread that calls into LV1 (Lv1Mixer)
//   midi     WinMM input by name (MidiIn)
//   rtp      the RTP-MIDI session and DNS-SD (RtpService)
//   ui       the settings window and its message loop, started on first show
// Settings live in %ProgramData%\Tune Remote\settings.ini, the log in tune-remote.log
// beside it (1 MB, one rotation).
#pragma once
#include <stdint.h>
#include <atomic>
#include <condition_variable>
#include <deque>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>
#include "../core/engine.h"
#include "../core/log.h"
#include "../core/settings.h"
#include "../net/rtp_session.h"
#include "clock.h"
#include "dnssd.h"
#include "lv1_mixer.h"
#include "midi_in.h"

namespace trw {

// The appended log file, written by the worker (never by a MIDI or LV1 thread).
class FileLog {
  public:
    void open(const std::wstring& path);
    void add(const tr::LogLine& l, bool detail);   // any thread: queues
    void flush();                                   // the worker
  private:
    std::mutex mu_;
    std::deque<std::string> q_;
    std::wstring path_;
};

// The RTP-MIDI session, DNS-SD and their thread.
class RtpService {
  public:
    using LogFn  = std::function<void(const std::string& line, bool detail)>;
    using MidiFn = std::function<void(const std::string& source, const uint8_t* msg, size_t n)>;

    void start(LogFn log, MidiFn midi);
    void stop(unsigned waitMs);

    // Applied by the RTP thread: (re)opens the session when something changed.
    void configure(bool enabled, const std::string& name, uint16_t port, const std::vector<tr::Settings::Peer>& peers);
    void connect(const std::string& ip, uint16_t port, const std::string& label);
    void forget(const std::string& ip, uint16_t port);

    struct View {
        bool enabled = false, open = false;
        std::string error;                     // why it is not open
        uint16_t port = 0;
        std::string advert;                    // DNS-SD state
        std::vector<rtpmidi::PeerView> peers;
        std::vector<FoundSession> found;
        uint64_t sessionNowMs = 0;             // the clock PeerView's times are on
        uint64_t lastMidiMs = 0;               // now_ms() of the last message, 0 = none
    };
    View view() const;

  private:
    void run();
    struct Cmd { bool connect; std::string ip; uint16_t port; std::string label; };

    std::thread thr_;
    std::atomic<bool> run_{false};
    mutable std::mutex mu_;
    std::condition_variable cv_;
    bool want_ = false, changed_ = false;
    std::string name_ = "Tune Remote";
    uint16_t port_ = 5004;
    std::vector<tr::Settings::Peer> peers_;
    std::deque<Cmd> cmds_;
    bool open_ = false;
    std::string error_;
    std::atomic<uint64_t> lastMidi_{0};
    rtpmidi::Session session_;
    Dnssd dnssd_;
    LogFn log_;
    MidiFn midi_;
};

class Window;

class App {
  public:
    // The first controller LV1 creates gets the App; nullptr for any later one (a second
    // Tune Remote surface would fight over the MIDI port and UDP 5004).
    static App* create(void* core, const wmsd::CoreCallbacks& cb, std::string* why);
    // Stops every thread (bounded), saves nothing new. The App stays allocated.
    void stop();

    void show_window();      // from LV1's thread: posts and returns

    // ---- for the window
    tr::Settings settings() const;
    void update(const std::function<void(tr::Settings&)>& change);   // saved and applied
    void apply_now(const tr::Intent& in);
    void rescan();
    void rtp_connect(const std::string& ip, uint16_t port, const std::string& label);
    void rtp_forget(const std::string& ip, uint16_t port);
    tr::Engine::Status status() const { return engine_.status(); }
    std::vector<tr::TuneInstance> tunes() const { return engine_.tunes(); }
    tr::LogRing& log() { return log_; }
    std::vector<std::string> midi_ports() const { return midi_.ports(); }
    std::string midi_open() const { return midi_.open_name(); }
    uint64_t midi_last_ms() const { return midi_.last_ms(); }   // now_ms()
    RtpService::View rtp() const { return rtp_.view(); }
    bool lv1_ok() const { return !mixer_.faulted(); }
    std::wstring folder() const { return folder_; }

  private:
    App(void* core, const wmsd::CoreCallbacks& cb);
    void start();
    void apply(const tr::Settings& s);
    void save(const tr::Settings& s);
    void worker();
    void info(const std::string& line) { log_.add(tr::Level::Info, line); }

    std::wstring folder_;
    tr::LogRing log_;
    FileLog file_;
    Lv1Mixer mixer_;
    tr::Engine engine_;
    MidiIn midi_;
    RtpService rtp_;
    mutable std::mutex setMu_;
    tr::Settings settings_;

    std::thread worker_;
    std::atomic<bool> run_{false};
    void* wake_ = nullptr;           // HANDLE: auto-reset event the engine sets
    Window* window_ = nullptr;
    std::mutex winMu_;
};

}  // namespace trw
