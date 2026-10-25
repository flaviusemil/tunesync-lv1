// log.cpp - see log.h.
#include "log.h"

namespace tr {

void LogRing::add(Level level, const std::string& text) {
    LogLine l;
    l.stamp = stamp_ ? stamp_() : std::string();
    l.level = level;
    l.text = text;
    {
        std::lock_guard<std::mutex> g(mu_);
        lines_.push_back(l);
        while (lines_.size() > cap_) lines_.pop_front();
        ++gen_;
    }
    if (sink_) sink_(l);
}

std::vector<LogLine> LogRing::snapshot() const {
    std::lock_guard<std::mutex> g(mu_);
    return std::vector<LogLine>(lines_.begin(), lines_.end());
}

uint64_t LogRing::generation() const {
    std::lock_guard<std::mutex> g(mu_);
    return gen_;
}

void LogRing::clear() {
    std::lock_guard<std::mutex> g(mu_);
    lines_.clear();
    ++gen_;
}

}  // namespace tr
