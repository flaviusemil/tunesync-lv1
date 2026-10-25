// settings.h - what the settings window edits, stored as a small ini file
// (%ProgramData%\Tune Remote\settings.ini on Windows).
#pragma once
#include <string>
#include <vector>
#include "tune.h"

namespace tr {

struct Settings {
    std::string midiPort;                  // input port name, "" = none
    bool        rtpEnabled = false;
    std::string rtpName = "Tune Remote";   // the session name the Mac sees
    int         rtpPort = 5004;            // control port; data = +1
    int         channel = 16;              // 1..16, 0 = Omni
    int         scaleCc = 20;              // 0..119
    Spelling    spelling = Spelling::Sharp;  // for Tunes on a natural key; never Natural
};

std::string to_ini(const Settings& s);
// Unknown keys and bad values are skipped with a line in *warnings; the rest is kept.
Settings from_ini(const std::string& text, std::vector<std::string>* warnings);

}  // namespace tr
