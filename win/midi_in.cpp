// midi_in.cpp - see midi_in.h.
#ifndef WIN32_LEAN_AND_MEAN
#  define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <mmsystem.h>
#include "midi_in.h"
#include "clock.h"
#include <chrono>

namespace trw {

namespace {

int data_len(uint8_t s) {
    switch (s & 0xF0) {
    case 0x80: case 0x90: case 0xA0: case 0xB0: case 0xE0: return 2;
    case 0xC0: case 0xD0: return 1;
    default: break;
    }
    if (s == 0xF1 || s == 0xF3) return 1;
    if (s == 0xF2) return 2;
    return 0;
}

std::string utf8(const wchar_t* w) {
    char b[256] = {0};
    WideCharToMultiByte(CP_UTF8, 0, w, -1, b, sizeof b, nullptr, nullptr);
    return b;
}

std::vector<std::string> list_inputs() {
    std::vector<std::string> v;
    for (UINT i = 0, n = midiInGetNumDevs(); i < n && i < 64; ++i) {
        MIDIINCAPSW c{};
        v.push_back(midiInGetDevCapsW(i, &c, sizeof c) == MMSYSERR_NOERROR ? utf8(c.szPname) : std::string("?"));
    }
    return v;
}

std::string mm_error(MMRESULT r) {
    wchar_t t[MAXERRORLENGTH] = {0};
    midiInGetErrorTextW(r, t, MAXERRORLENGTH);
    return std::to_string(r) + " " + utf8(t);
}

struct Port {
    MidiIn*           owner = nullptr;
    HMIDIIN           in = nullptr;
    MIDIHDR           hdr[4]{};
    char              buf[4][1024]{};
    std::mutex        doneMu;
    std::vector<MIDIHDR*> done;       // SysEx buffers the driver returned, to add back (not from the callback)
    std::atomic<bool> closing{false};

    static void CALLBACK proc(HMIDIIN, UINT msg, DWORD_PTR inst, DWORD_PTR p1, DWORD_PTR) {
        // The driver's own thread: hand the bytes over, nothing else.
        Port* self = reinterpret_cast<Port*>(inst);
        if (msg == MIM_DATA) {
            const uint8_t b[3] = {uint8_t(p1), uint8_t(p1 >> 8), uint8_t(p1 >> 16)};
            if (b[0] & 0x80) self->owner->incoming(b, size_t(1 + data_len(b[0])));
        } else if (msg == MIM_LONGDATA) {
            MIDIHDR* h = reinterpret_cast<MIDIHDR*>(p1);
            if (h->dwBytesRecorded) self->owner->incoming(reinterpret_cast<uint8_t*>(h->lpData), h->dwBytesRecorded);
            if (!self->closing) { std::lock_guard<std::mutex> g(self->doneMu); self->done.push_back(h); }
        }
    }
    bool open(const std::string& name, std::string* err) {
        const std::vector<std::string> names = list_inputs();
        int idx = -1;
        for (size_t i = 0; i < names.size(); ++i)
            if (names[i] == name) { idx = int(i); break; }
        if (idx < 0) { *err = "not connected"; return false; }
        closing = false;
        MMRESULT r = midiInOpen(&in, UINT(idx), DWORD_PTR(&proc), DWORD_PTR(this), CALLBACK_FUNCTION);
        if (r != MMSYSERR_NOERROR) {
            in = nullptr;
            *err = mm_error(r) + (r == MMSYSERR_ALLOCATED ? " - another program has it open" : "");
            return false;
        }
        for (int k = 0; k < 4; ++k) {
            hdr[k] = MIDIHDR{}; hdr[k].lpData = buf[k]; hdr[k].dwBufferLength = sizeof buf[k];
            midiInPrepareHeader(in, &hdr[k], sizeof hdr[k]);
            midiInAddBuffer(in, &hdr[k], sizeof hdr[k]);
        }
        r = midiInStart(in);
        if (r != MMSYSERR_NOERROR) { *err = mm_error(r); close(); return false; }
        return true;
    }
    void close() {
        if (!in) return;
        closing = true;
        midiInStop(in);
        midiInReset(in);
        for (int k = 0; k < 4; ++k) midiInUnprepareHeader(in, &hdr[k], sizeof hdr[k]);
        midiInClose(in);
        in = nullptr;
        std::lock_guard<std::mutex> g(doneMu);
        done.clear();
    }
    void service() {
        std::vector<MIDIHDR*> d;
        { std::lock_guard<std::mutex> g(doneMu); d.swap(done); }
        for (MIDIHDR* h : d)
            if (in && !closing) { h->dwBytesRecorded = 0; midiInAddBuffer(in, h, sizeof *h); }
    }
};

}  // namespace

std::vector<std::vector<uint8_t>> split_midi(const uint8_t* p, size_t n) {
    std::vector<std::vector<uint8_t>> out;
    std::vector<uint8_t> cur;
    uint8_t running = 0;
    int need = 0;
    bool sysex = false;
    for (size_t i = 0; i < n; ++i) {
        const uint8_t b = p[i];
        if (b >= 0xF8) { out.push_back({b}); continue; }
        if (sysex) {
            if (b == 0xF7) { cur.push_back(b); out.push_back(cur); cur.clear(); sysex = false; continue; }
            if (b & 0x80) { cur.clear(); sysex = false; }
            else { if (cur.size() < 4096) cur.push_back(b); continue; }
        }
        if (b & 0x80) {
            cur.assign(1, b);
            if (b == 0xF0) { sysex = true; running = 0; continue; }
            need = data_len(b);
            running = b < 0xF0 ? b : 0;
            if (!need) { out.push_back(cur); cur.clear(); }
            continue;
        }
        if (cur.empty()) {
            if (!running) continue;
            cur.assign(1, running);
            need = data_len(running);
        }
        cur.push_back(b);
        if (--need == 0) { out.push_back(cur); cur.clear(); }
    }
    return out;
}

void MidiIn::start(MidiFn midi, LogFn log) {
    if (run_) return;
    midi_ = std::move(midi);
    log_ = std::move(log);
    run_ = true;
    thr_ = std::thread([this] { run(); });
}

void MidiIn::stop(unsigned waitMs) {
    if (!run_) return;
    run_ = false;
    cv_.notify_all();
    if (thr_.joinable()) join_for(thr_, waitMs);
}

void MidiIn::want(const std::string& name) {
    {
        std::lock_guard<std::mutex> g(mu_);
        if (name == want_) return;
        want_ = name;
        changed_ = true;
    }
    cv_.notify_all();
}

std::string MidiIn::wanted() const { std::lock_guard<std::mutex> g(mu_); return want_; }
std::vector<std::string> MidiIn::ports() const { std::lock_guard<std::mutex> g(mu_); return ports_; }
std::string MidiIn::open_name() const { std::lock_guard<std::mutex> g(mu_); return open_; }

void MidiIn::incoming(const uint8_t* p, size_t n) {
    if (!n) return;
    {
        std::lock_guard<std::mutex> g(mu_);
        if (inQ_.size() >= 4096) return;
        inQ_.emplace_back(p, p + n);
    }
    cv_.notify_all();
}

void MidiIn::run() {
    Port port;
    port.owner = this;
    using clk = std::chrono::steady_clock;
    auto nextScan = clk::now();
    std::string lastNote;
    auto note = [&](const std::string& s) { if (s != lastNote) { lastNote = s; if (log_) log_(s); } };
    while (run_) {
        bool changed = false;
        std::string want;
        { std::lock_guard<std::mutex> g(mu_); changed = changed_; changed_ = false; want = want_; }
        if (changed && port.in) {
            const std::string was = open_name();
            port.close();
            { std::lock_guard<std::mutex> g(mu_); open_.clear(); }
            note("MIDI in: closed " + was);
        }
        if (changed || clk::now() >= nextScan) {
            nextScan = clk::now() + std::chrono::seconds(2);
            const std::vector<std::string> names = list_inputs();
            { std::lock_guard<std::mutex> g(mu_); ports_ = names; }
            bool there = false;
            for (const std::string& n : names) there |= n == want;
            if (port.in && !there) {
                port.close();
                { std::lock_guard<std::mutex> g(mu_); open_.clear(); }
                note("MIDI in: " + want + " went away; it is reopened when it comes back");
            }
            if (!port.in && !want.empty() && there) {
                std::string err;
                if (port.open(want, &err)) {
                    { std::lock_guard<std::mutex> g(mu_); open_ = want; }
                    note("MIDI in: listening on " + want);
                } else {
                    note("MIDI in: cannot open " + want + ": " + err);
                }
            } else if (!port.in && !want.empty() && changed) {
                note("MIDI in: " + want + " is not connected; it is opened when it appears");
            }
        }
        port.service();
        std::deque<std::vector<uint8_t>> in;
        {
            std::unique_lock<std::mutex> g(mu_);
            if (inQ_.empty() && !changed_)
                cv_.wait_for(g, std::chrono::milliseconds(200), [this] { return !run_ || !inQ_.empty() || changed_; });
            in.swap(inQ_);
        }
        const std::string name = open_name();
        for (const auto& chunk : in)
            for (const auto& m : split_midi(chunk.data(), chunk.size())) {
                lastMs_ = now_ms();
                if (midi_) midi_(name, m.data(), m.size());
            }
    }
    port.close();
    std::lock_guard<std::mutex> g(mu_);
    open_.clear();
}

}  // namespace trw
