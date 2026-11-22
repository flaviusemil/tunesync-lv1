// app.cpp - see app.h.
#ifndef WIN32_LEAN_AND_MEAN
#  define WIN32_LEAN_AND_MEAN
#endif
#include <winsock2.h>
#include <windows.h>
#include <shlobj.h>
#include "app.h"
#include <stdio.h>
#include <atomic>
#include "../core/tune.h"
#include "version.h"
#include "window.h"

namespace trw {

// ================================================================ small things

uint64_t now_ms() {
    static LARGE_INTEGER f = [] { LARGE_INTEGER x; QueryPerformanceFrequency(&x); return x; }();
    LARGE_INTEGER t;
    QueryPerformanceCounter(&t);
    return (uint64_t)(t.QuadPart / (f.QuadPart / 1000)) + 1;
}

bool join_for(std::thread& t, unsigned ms) {
    if (!t.joinable()) return true;
    if (WaitForSingleObject((HANDLE)t.native_handle(), ms) == WAIT_OBJECT_0) { t.join(); return true; }
    t.detach();
    return false;
}

namespace {

std::wstring program_data() {
    PWSTR p = nullptr;
    std::wstring s = L"C:\\ProgramData";
    if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_ProgramData, 0, nullptr, &p)) && p) s = p;
    if (p) CoTaskMemFree(p);
    return s + L"\\Tune Remote";
}

std::string read_file(const std::wstring& path) {
    HANDLE h = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) return std::string();
    std::string s;
    char b[4096];
    DWORD n = 0;
    while (ReadFile(h, b, sizeof b, &n, nullptr) && n && s.size() < (1u << 20)) s.append(b, n);
    CloseHandle(h);
    return s;
}

// Written to a temporary file and moved over the old one, so a crash never leaves half a file.
bool write_file(const std::wstring& path, const std::string& data, DWORD* err) {
    const std::wstring tmp = path + L".tmp";
    HANDLE h = CreateFileW(tmp.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) { *err = GetLastError(); return false; }
    DWORD w = 0;
    const BOOL ok = WriteFile(h, data.data(), (DWORD)data.size(), &w, nullptr) && w == data.size();
    CloseHandle(h);
    if (!ok || !MoveFileExW(tmp.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING)) {
        *err = GetLastError();
        DeleteFileW(tmp.c_str());
        return false;
    }
    return true;
}

std::string clock_text() {
    SYSTEMTIME t;
    GetLocalTime(&t);
    char b[16];
    _snprintf_s(b, sizeof b, _TRUNCATE, "%02d:%02d:%02d", t.wHour, t.wMinute, t.wSecond);
    return b;
}

std::atomic<App*> g_app{nullptr};

}  // namespace

// ================================================================ FileLog

void FileLog::open(const std::wstring& path) {
    std::lock_guard<std::mutex> g(mu_);
    path_ = path;
}

void FileLog::add(const tr::LogLine& l, bool detail) {
    SYSTEMTIME t;
    GetLocalTime(&t);
    char head[64];
    _snprintf_s(head, sizeof head, _TRUNCATE, "%04d-%02d-%02d %02d:%02d:%02d.%03d %s ", t.wYear, t.wMonth, t.wDay, t.wHour,
                t.wMinute, t.wSecond, t.wMilliseconds,
                detail ? "  .  " : l.level == tr::Level::Ok ? "[ok] " : l.level == tr::Level::Warn ? "[!!] " : "     ");
    std::lock_guard<std::mutex> g(mu_);
    if (q_.size() < 5000) q_.push_back(head + l.text + "\r\n");
}

void FileLog::flush() {
    std::deque<std::string> q;
    std::wstring path;
    {
        std::lock_guard<std::mutex> g(mu_);
        q.swap(q_);
        path = path_;
    }
    if (q.empty() || path.empty()) return;
    WIN32_FILE_ATTRIBUTE_DATA fa{};
    if (GetFileAttributesExW(path.c_str(), GetFileExInfoStandard, &fa) && fa.nFileSizeLow > (1u << 20))
        MoveFileExW(path.c_str(), (path + L".1").c_str(), MOVEFILE_REPLACE_EXISTING);   // one rotation
    HANDLE h = CreateFileW(path.c_str(), FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                           OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) return;
    std::string all;
    for (const std::string& l : q) all += l;
    DWORD w = 0;
    WriteFile(h, all.data(), (DWORD)all.size(), &w, nullptr);
    CloseHandle(h);
}

// ================================================================ RtpService

void RtpService::start(LogFn log, MidiFn midi) {
    if (run_) return;
    log_ = std::move(log);
    midi_ = std::move(midi);
    session_.on_log([this](const std::string& l, bool detail) { if (log_) log_(l, detail); });
    session_.on_midi([this](const rtpmidi::PeerView& p, const uint8_t* m, size_t n) {
        lastMidi_ = now_ms();
        if (midi_) midi_("RTP " + p.display(), m, n);
    });
    run_ = true;
    thr_ = std::thread([this] { run(); });
}

void RtpService::stop(unsigned waitMs) {
    if (!run_) return;
    run_ = false;
    cv_.notify_all();
    join_for(thr_, waitMs);
}

void RtpService::configure(bool enabled, const std::string& name, uint16_t port, const std::vector<tr::Settings::Peer>& peers) {
    {
        std::lock_guard<std::mutex> g(mu_);
        if (enabled == want_ && name == name_ && port == port_ && peers == peers_) return;
        const bool reopen = enabled != want_ || name != name_ || port != port_;
        want_ = enabled; name_ = name; port_ = port; peers_ = peers;
        changed_ = changed_ || reopen;
    }
    cv_.notify_all();
}

void RtpService::connect(const std::string& ip, uint16_t port, const std::string& label) {
    { std::lock_guard<std::mutex> g(mu_); cmds_.push_back({true, ip, port, label}); }
    cv_.notify_all();
}

void RtpService::forget(const std::string& ip, uint16_t port) {
    { std::lock_guard<std::mutex> g(mu_); cmds_.push_back({false, ip, port, ""}); }
    cv_.notify_all();
}

RtpService::View RtpService::view() const {
    View v;
    {
        std::lock_guard<std::mutex> g(mu_);
        v.enabled = want_;
        v.open = open_;
        v.error = error_;
        v.port = port_;
    }
    if (v.open) {
        v.peers = session_.peers();
        v.sessionNowMs = session_.now_ms();
        v.found = dnssd_.found();
        v.advert = dnssd_.advert_state();
    }
    v.lastMidiMs = lastMidi_.load();
    return v;
}

void RtpService::run() {
    while (run_) {
        bool changed = false, want = false;
        std::string name;
        uint16_t port = 0;
        std::vector<tr::Settings::Peer> peers;
        std::deque<Cmd> cmds;
        {
            std::lock_guard<std::mutex> g(mu_);
            changed = changed_; changed_ = false;
            want = want_; name = name_; port = port_; peers = peers_;
            cmds.swap(cmds_);
        }
        if (changed) {
            if (session_.is_open()) {
                dnssd_.stop();
                session_.close();
                if (log_) log_("RTP-MIDI: session closed", false);
            }
            std::string err;
            if (want) {
                rtpmidi::SessionConfig c;
                c.name = name;
                c.port = port;
                if (session_.open(c, &err)) {
                    if (log_) log_("RTP-MIDI: session \"" + name + "\" open on UDP " + std::to_string(port) + "/" +
                                       std::to_string(port + 1), false);
                    dnssd_.start(name, port, log_);
                    for (const tr::Settings::Peer& p : peers) session_.invite(p.ip, uint16_t(p.port), p.name);
                } else if (log_) {
                    log_("RTP-MIDI: cannot open the session: " + err, false);
                }
            }
            std::lock_guard<std::mutex> g(mu_);
            open_ = session_.is_open();
            error_ = want && !open_ ? err : std::string();
        }
        for (const Cmd& c : cmds) {
            if (!session_.is_open()) break;
            if (c.connect) session_.invite(c.ip, c.port, c.label);
            else session_.forget(c.ip, c.port);
        }
        if (session_.is_open()) {
            session_.poll(50);
        } else {
            std::unique_lock<std::mutex> g(mu_);
            cv_.wait_for(g, std::chrono::milliseconds(250), [this] { return !run_ || changed_ || !cmds_.empty(); });
        }
    }
    dnssd_.stop();
    session_.close();
    std::lock_guard<std::mutex> g(mu_);
    open_ = false;
}

// ================================================================ App

App* App::create(void* core, const wmsd::CoreCallbacks& cb, std::string* why) {
    App* expected = nullptr;
    App* a = new App(core, cb);
    if (!g_app.compare_exchange_strong(expected, a)) {
        delete a;   // never started: no threads, nothing registered
        if (why) *why = "another Tune Remote surface is already running in this LV1; only the first one acts";
        return nullptr;
    }
    a->start();
    return a;
}

App::App(void* core, const wmsd::CoreCallbacks& cb) : mixer_(core, cb), engine_(mixer_, log_) {}

void App::start() {
    folder_ = program_data();
    CreateDirectoryW(folder_.c_str(), nullptr);
    file_.open(folder_ + L"\\tune-remote.log");
    log_.set_stamper(clock_text);
    log_.set_sink([this](const tr::LogLine& l) { file_.add(l, false); });

    wake_ = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    engine_.set_wake([this] { SetEvent((HANDLE)wake_); });

    std::vector<std::string> warnings;
    const std::string ini = read_file(folder_ + L"\\settings.ini");
    tr::Settings s = tr::from_ini(ini, &warnings);
    info(std::string("Tune Remote ") + TR_VERSION_STR + " started" + (ini.empty() ? " (default settings)" : ""));
    for (const std::string& w : warnings) log_.add(tr::Level::Warn, "settings.ini: " + w);

    midi_.start([this](const std::string& port, const uint8_t* m, size_t n) { engine_.on_midi("MIDI " + port, m, n, now_ms()); },
                [this](const std::string& l) { info(l); });
    rtp_.start(
        [this](const std::string& l, bool detail) {
            if (detail) file_.add(tr::LogLine{log_.stamp_now(), tr::Level::Info, l}, true);
            else info(l);
        },
        [this](const std::string& src, const uint8_t* m, size_t n) { engine_.on_midi(src, m, n, now_ms()); });
    {
        std::lock_guard<std::mutex> g(setMu_);
        settings_ = s;
    }
    apply(s);
    run_ = true;
    worker_ = std::thread([this] { worker(); });
}

void App::apply(const tr::Settings& s) {
    engine_.configure(s);
    midi_.want(s.midiPort);
    rtp_.configure(s.rtpEnabled, s.rtpName, uint16_t(s.rtpPort), s.rtpPeers);
}

void App::save(const tr::Settings& s) {
    DWORD err = 0;
    if (!write_file(folder_ + L"\\settings.ini", tr::to_ini(s), &err))
        log_.add(tr::Level::Warn, "could not save settings.ini (error " + std::to_string(err) +
                                      "): the change holds until LV1 closes");
}

tr::Settings App::settings() const {
    std::lock_guard<std::mutex> g(setMu_);
    return settings_;
}

void App::update(const std::function<void(tr::Settings&)>& change) {
    tr::Settings s;
    {
        std::lock_guard<std::mutex> g(setMu_);
        change(settings_);
        if (settings_.spelling == tr::Spelling::Natural) settings_.spelling = tr::Spelling::Sharp;
        s = settings_;
    }
    save(s);
    apply(s);
}

void App::apply_now(const tr::Intent& in) { engine_.apply_now(in, now_ms()); }
void App::rescan() { engine_.request_scan(); }

void App::rtp_connect(const std::string& ip, uint16_t port, const std::string& label) {
    update([&](tr::Settings& s) {
        tr::Settings::Peer p{ip, port, label};
        for (const auto& q : s.rtpPeers)
            if (q == p) return;
        if (s.rtpPeers.size() < tr::kMaxRtpPeers) s.rtpPeers.push_back(p);
    });
    rtp_.connect(ip, port, label);
}

void App::rtp_forget(const std::string& ip, uint16_t port) {
    update([&](tr::Settings& s) {
        tr::Settings::Peer p{ip, port, ""};
        for (size_t i = 0; i < s.rtpPeers.size(); ++i)
            if (s.rtpPeers[i] == p) { s.rtpPeers.erase(s.rtpPeers.begin() + i); break; }
    });
    rtp_.forget(ip, port);
}

void App::worker() {
    bool saidFault = false;
    engine_.request_scan();   // the Tunes list is ready when the window first opens
    while (run_) {
        const uint64_t t0 = now_ms();
        const tr::Engine::Status before = engine_.status();
        int64_t next = engine_.tick(t0);
        const uint64_t took = now_ms() - t0;
        const tr::Engine::Status after = engine_.status();
        if (after.scans != before.scans && took > 200)
            log_.add(tr::Level::Warn, "a pass over LV1 took " + std::to_string(took) + " ms");
        if (mixer_.faulted() && !saidFault) {
            saidFault = true;
            char b[160];
            _snprintf_s(b, sizeof b, _TRUNCATE,
                        "LV1 refused a control call (exception 0x%08X): Tune Remote stays passive until LV1 restarts",
                        mixer_.fault_code());
            log_.add(tr::Level::Warn, b);
        }
        file_.flush();
        const DWORD wait = next < 0 ? 250 : next > 250 ? 250 : (DWORD)(next < 1 ? 1 : next);
        WaitForSingleObject((HANDLE)wake_, wait);
    }
    file_.flush();
}

void App::show_window() {
    std::lock_guard<std::mutex> g(winMu_);
    if (!run_) return;
    if (!window_) window_ = Window::start(this);
    if (window_) window_->show();
}

void App::stop() {
    if (!run_.exchange(false)) return;
    info("Tune Remote stopping (LV1 closed the surface)");
    {
        std::lock_guard<std::mutex> g(winMu_);
        if (window_) window_->stop(1000);
    }
    SetEvent((HANDLE)wake_);
    if (!join_for(worker_, 1500)) log_.add(tr::Level::Warn, "the worker did not stop in time");
    midi_.stop(1000);
    rtp_.stop(1500);
    file_.flush();
    g_app = nullptr;
}

}  // namespace trw
