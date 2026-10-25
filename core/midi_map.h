// midi_map.h - what a MIDI message from the tracks rig asks for, and how messages
// that arrive together become one change.
//
//   Note On (velocity > 0) on the channel   key = the note's pitch class
//   CC <scaleCc> on the channel, 0..10      scale = that Tune scale number
//   anything else                           ignored (Note Off, other channels, ...)
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <string>

namespace tr {

// What one change asks for; -1 = leave as it is.
struct Intent {
    int pitchClass = -1;   // 0 = C .. 11 = B
    int scale = -1;        // 0..10, Tune's own scale numbers
    bool empty() const { return pitchClass < 0 && scale < 0; }
};

enum class MsgKind { Ignored, Key, Scale, OutOfRange };

struct Interpreted {
    MsgKind     kind = MsgKind::Ignored;
    int         value = -1;    // pitch class for Key, scale for Scale, the raw value for OutOfRange
    std::string text;          // the message for the log, e.g. "Note On 57 (A) vel 100 ch 16";
                               // "" for system messages (clock, MTC, SysEx), which are never logged
};

// One whole MIDI message (status byte first, running status already expanded).
// channel: 1..16, or 0 = any (Omni). scaleCc: 0..119.
Interpreted interpret(const uint8_t* msg, size_t n, int channel, int scaleCc);

// Messages within kWindowMs of the first one are one change: a clip that sends the
// scale CC and the key note together gives one pass over the Tunes. The last key and
// the last scale in the window win.
class Coalescer {
  public:
    static constexpr uint64_t kWindowMs = 30;

    void add(const Interpreted& m, uint64_t nowMs);
    // Apply-now from the settings window: due at once, merged with anything pending.
    void add_now(const Intent& in, uint64_t nowMs);
    bool pending() const { return !cur_.empty(); }
    // Milliseconds until the pending change is due (0 = now), -1 when nothing is pending.
    int64_t due_in(uint64_t nowMs) const;
    Intent take();

  private:
    Intent   cur_;
    uint64_t dueAt_ = 0;
};

}  // namespace tr
