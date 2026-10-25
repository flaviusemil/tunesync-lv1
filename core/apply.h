// apply.h - find every Tune RT in LV1 and set its key and scale, through an
// abstract Mixer so the logic runs against a fake LV1 in the tests.
//
// Per instance: check the parameters look the way tune.h says (else skip it:
// a Tune update could reorder them), skip it when its Receive button is on Lock,
// set the key in the spelling it already uses, set the scale, read both back.
#pragma once
#include <string>
#include <vector>
#include "midi_map.h"
#include "tune.h"

namespace tr {

struct ParamRead {
    bool        ok = false;   // LV1 answered
    double      value = 0;
    std::string text;
};

// The slice of LV1's surface API this needs. win/ implements it on
// GetMixerControl / SetMixerControl; every call happens on one worker thread.
class Mixer {
  public:
    virtual ~Mixer() = default;
    // A plug-in control: section 4, sectionIndex = slot, channel = parameter (or kSlotInfo).
    virtual ParamRead get(int trackType, int trackIndex, int slot, int channel, int cid) = 0;
    // Touch, Control(value), Release on a plug-in parameter's value (cid 11).
    virtual bool set(int trackType, int trackIndex, int slot, int param, double value) = 0;
    // The track's user name, "" when it has none.
    virtual std::string track_name(int trackType, int trackIndex) = 0;
};

// Which tracks to look at. Track types: 0 Ch, 1 Gp, 2 Fx (LV1's WSTrackID).
struct TrackRange {
    int         trackType;
    int         count;
    const char* label;
};
std::vector<TrackRange> default_ranges();
constexpr int kInsertSlots = 8;

struct TuneInstance {
    int         trackType = 0, trackIndex = 0, slot = 0;
    std::string where;          // "Ch 43 Vox 1", for the log and the window
    int         key = -1, scale = -1;
    bool        receive = true; // false = Lock
    bool        layoutOk = true;
    std::string why;            // what did not look right, when !layoutOk
};

std::vector<TuneInstance> discover(Mixer& m, const std::vector<TrackRange>& ranges);

struct ApplyResult {
    std::string              label;   // what was asked for, e.g. "C#/Db Natural Minor"
    std::vector<std::string> set, unchanged, locked, bad, failed;
    std::string summary() const;      // one log line
};

ApplyResult apply(Mixer& m, const std::vector<TuneInstance>& tunes, const Intent& in, Spelling fallback);

}  // namespace tr
