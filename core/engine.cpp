// engine.cpp - see engine.h.
#include "engine.h"

namespace tr {

Engine::Engine(Mixer& m, LogRing& log, std::vector<TrackRange> ranges)
    : mixer_(m), log_(log), ranges_(std::move(ranges)) {}

void Engine::configure(const Settings& s) {
    std::lock_guard<std::mutex> g(mu_);
    settings_ = s;
    if (settings_.spelling == Spelling::Natural) settings_.spelling = Spelling::Sharp;
}

Settings Engine::settings() const {
    std::lock_guard<std::mutex> g(mu_);
    return settings_;
}

void Engine::add_source(const std::string& source) {
    if (!pending_.pending()) pendingSources_.clear();
    for (const std::string& s : pendingSources_)
        if (s == source) return;
    pendingSources_.push_back(source);
}

void Engine::on_midi(const std::string& source, const uint8_t* msg, size_t n, uint64_t nowMs) {
    std::string line;
    Level level = Level::Info;
    bool due = false;
    {
        std::lock_guard<std::mutex> g(mu_);
        const Interpreted m = interpret(msg, n, settings_.channel, settings_.scaleCc);
        if (m.text.empty()) return;   // clock, MTC, SysEx: not ours, not logged
        status_.lastMidiMs = nowMs;
        status_.lastSource = source;
        if (m.kind == MsgKind::Ignored) {
            // A keyboard on the same port must not flood the log: count, and log one
            // line now and then from tick().
            ++ignored_;
            lastIgnored_ = source + " " + m.text;
            due = ignored_in(nowMs) == 0;
        } else {
            if (m.kind != MsgKind::OutOfRange) add_source(source);
            pending_.add(m, nowMs);
            line = source + " " + m.text;
            level = m.kind == MsgKind::OutOfRange ? Level::Warn : Level::Info;
            due = true;
        }
    }
    if (!line.empty()) log_.add(level, line);
    if (due) wake();
}

void Engine::apply_now(const Intent& in, uint64_t nowMs) {
    if (in.empty()) return;
    std::string what;
    if (in.pitchClass >= 0) what = pitch_class_name(in.pitchClass);
    if (const char* s = scale_name(in.scale)) what += (what.empty() ? "" : " ") + std::string(s);
    {
        std::lock_guard<std::mutex> g(mu_);
        add_source("Apply now");
        pending_.add_now(in, nowMs);
    }
    log_.add(Level::Info, "Apply now: " + what);
    wake();
}

void Engine::request_scan() {
    {
        std::lock_guard<std::mutex> g(mu_);
        scanWanted_ = true;
    }
    wake();
}

int64_t Engine::tick(uint64_t nowMs) {
    Intent change;
    bool scan = false;
    Spelling fallback;
    std::string ignoredLine, source;
    {
        std::lock_guard<std::mutex> g(mu_);
        if (pending_.due_in(nowMs) == 0) {
            change = pending_.take();
            for (const std::string& x : pendingSources_) source += (source.empty() ? "" : " + ") + x;
            pendingSources_.clear();
        }
        scan = scanWanted_;
        scanWanted_ = false;
        fallback = settings_.spelling;
        if (ignored_ && ignored_in(nowMs) == 0) {
            ignoredLine = "ignored " + std::to_string(ignored_) + " message" + (ignored_ == 1 ? "" : "s") +
                          " (not a Note On or CC " + std::to_string(settings_.scaleCc) + " on " +
                          (settings_.channel ? "ch " + std::to_string(settings_.channel) : std::string("any channel")) +
                          "), last: " + lastIgnored_;
            ignored_ = 0;
            lastIgnoredLog_ = nowMs;
        }
    }
    if (!ignoredLine.empty()) log_.add(Level::Info, ignoredLine);

    // LV1 work, outside the lock: a slow scan never blocks a MIDI callback.
    if (!change.empty() || scan) {
        std::vector<TuneInstance> found = discover(mixer_, ranges_);
        std::string result;
        Change last;
        if (!change.empty()) {
            const ApplyResult r = apply(mixer_, found, change, fallback);
            result = r.summary();
            log_.add(r.problem() ? Level::Warn : Level::Ok, result);
            last.label = r.label;
            last.stamp = log_.stamp_now();
            last.source = source;
            last.brief = r.brief();
            last.problem = r.problem();
            found = discover(mixer_, ranges_);   // the window shows what LV1 now holds
        }
        size_t locked = 0;
        for (const TuneInstance& t : found) locked += t.layoutOk && !t.receive;
        std::lock_guard<std::mutex> g(mu_);
        tunes_ = std::move(found);
        status_.tunes = tunes_.size();
        status_.locked = locked;
        ++status_.scans;
        if (!result.empty()) {
            status_.lastResult = result;
            status_.last = std::move(last);
            ++status_.changes;
        }
    }

    std::lock_guard<std::mutex> g(mu_);
    const int64_t next = pending_.due_in(nowMs);
    if (!ignored_) return next;
    const int64_t ig = ignored_in(nowMs);
    return next < 0 || ig < next ? ig : next;
}

Engine::Status Engine::status() const {
    std::lock_guard<std::mutex> g(mu_);
    return status_;
}

std::vector<TuneInstance> Engine::tunes() const {
    std::lock_guard<std::mutex> g(mu_);
    return tunes_;
}

}  // namespace tr
