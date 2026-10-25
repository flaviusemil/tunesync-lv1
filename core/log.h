// log.h - the log the settings window shows (newest last here, the window
// reverses it) and the host mirrors to a file. Thread-safe.
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <deque>
#include <functional>
#include <mutex>
#include <string>
#include <vector>

namespace tr {

enum class Level { Info, Ok, Warn };

struct LogLine {
    std::string stamp;   // from the host's stamper, e.g. "21:04:12.031"
    Level       level = Level::Info;
    std::string text;
};

class LogRing {
  public:
    explicit LogRing(size_t capacity = 500) : cap_(capacity) {}

    // stamp(): the current local time as text. sink(line): called for every line,
    // outside the lock (the file log). Set both before the first add.
    void set_stamper(std::function<std::string()> f) { stamp_ = std::move(f); }
    void set_sink(std::function<void(const LogLine&)> f) { sink_ = std::move(f); }

    void add(Level level, const std::string& text);
    std::vector<LogLine> snapshot() const;
    uint64_t generation() const;   // bumps on every add or clear: the window redraws when it moves
    void clear();

  private:
    size_t                                cap_;
    mutable std::mutex                    mu_;
    std::deque<LogLine>                   lines_;
    uint64_t                              gen_ = 0;
    std::function<std::string()>          stamp_;
    std::function<void(const LogLine&)>   sink_;
};

}  // namespace tr
