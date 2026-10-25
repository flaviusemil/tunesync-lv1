// tune.cpp - see tune.h.
#include "tune.h"

namespace tr {
namespace {

// LV1's display strings, in value order (read back from Tune RT on the workstation).
const char* const kKeys[kNumKeys] = {"C", "C#", "Db", "D", "D#", "Eb", "E", "F", "F#",
                                     "Gb", "G", "G#", "Ab", "A", "A#", "Bb", "B"};
const int kKeyPc[kNumKeys]        = {0, 1, 1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 8, 9, 10, 10, 11};
const char* const kScales[kNumScales] = {
    "User Scale", "Chromatic", "Major", "Natural Minor", "Harmonic Minor", "Melodic Minor (Asc)",
    "Melodic Minor (Asc+Dec)", "Major Justonic", "Minor Justonic", "Dorian", "Mixolydian"};

// Per pitch class: the sharp-spelled value and the flat-spelled value (equal for white keys).
const int kSharpOf[12] = {0, 1, 3, 4, 6, 7, 8, 10, 11, 13, 14, 16};
const int kFlatOf[12]  = {0, 2, 3, 5, 6, 7, 9, 10, 12, 13, 15, 16};

}  // namespace

const char* key_name(int v) { return v >= 0 && v < kNumKeys ? kKeys[v] : nullptr; }
const char* scale_name(int v) { return v >= 0 && v < kNumScales ? kScales[v] : nullptr; }

int key_pitch_class(int v) { return v >= 0 && v < kNumKeys ? kKeyPc[v] : -1; }

Spelling key_spelling(int v) {
    const int pc = key_pitch_class(v);
    if (pc < 0 || kSharpOf[pc] == kFlatOf[pc]) return Spelling::Natural;
    return v == kFlatOf[pc] ? Spelling::Flat : Spelling::Sharp;
}

int key_value(int pitchClass, Spelling prefer) {
    if (pitchClass < 0 || pitchClass > 11) return -1;
    return prefer == Spelling::Flat ? kFlatOf[pitchClass] : kSharpOf[pitchClass];
}

Spelling keep_spelling(int currentKey, Spelling fallback) {
    const Spelling s = key_spelling(currentKey);
    return s == Spelling::Natural ? fallback : s;
}

std::string pitch_class_name(int pitchClass) {
    if (pitchClass < 0 || pitchClass > 11) return "?";
    const int s = kSharpOf[pitchClass], f = kFlatOf[pitchClass];
    return s == f ? kKeys[s] : std::string(kKeys[s]) + "/" + kKeys[f];
}

bool is_tune_code(const std::string& code) { return code == "LVLM" || code == "LVLS"; }

}  // namespace tr
