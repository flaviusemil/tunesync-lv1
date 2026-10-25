// tune.h - Waves Tune Real-Time as LV1 exposes it to a control surface: which
// parameters hold the key, the scale and Receive, what their values mean, and how
// a MIDI note becomes a key value that keeps the spelling a Tune already uses.
//
// Proven on LV1 16.5.189.293 (2026-10-25). A plug-in parameter is section 4,
// sectionIndex = insert slot, channel = parameter index, controlId 11 = value,
// text = LV1's display string. Slot identity is channel -1: cid 0 = name
// ("Tune RT"), cid 1 = four-letter code ("LVLM" = mono Tune RT).
#pragma once
#include <string>

namespace tr {

constexpr int kSecPlugins     = 4;
constexpr int kCidValue       = 11;
constexpr int kCidSlotName    = 0;
constexpr int kCidSlotCode    = 1;
constexpr int kSlotInfo       = -1;   // the "channel" that answers slot identity

constexpr int kParamScale     = 39;
constexpr int kParamKey       = 40;
constexpr int kParamReceive   = 42;   // 1 = "Rcv", 0 = "Lock"

constexpr int kNumKeys        = 17;   // C C# Db D D# Eb E F F# Gb G G# Ab A A# Bb B
constexpr int kNumScales      = 11;   // 0 User Scale .. 10 Mixolydian

enum class Spelling { Natural, Sharp, Flat };

// nullptr when v is out of range.
const char* key_name(int v);
const char* scale_name(int v);

// Pitch class (0 = C .. 11 = B) of a key value; -1 when out of range.
int key_pitch_class(int v);
// Natural for C D E F G A B, else whether the value is the sharp or the flat spelling.
Spelling key_spelling(int v);
// The key value for a pitch class. prefer decides only for the five black keys
// (Natural counts as Sharp there). -1 for a pitch class outside 0..11.
int key_value(int pitchClass, Spelling prefer);
// The spelling a Tune keeps: its current key's own, or fallback when that key is natural.
Spelling keep_spelling(int currentKey, Spelling fallback);

// "C", "C#/Db", ... for a pitch class, for log lines that are about the MIDI note.
std::string pitch_class_name(int pitchClass);

// Is this slot code Tune Real-Time? LVLM = mono (proven); LVLS = stereo (assumed,
// by the M/S pattern every other Waves code here follows: QDZM/QDZS, VXPM/VXPS).
bool is_tune_code(const std::string& code);

}  // namespace tr
