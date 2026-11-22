// settings.cpp - see settings.h.
#include "settings.h"
#include <stdlib.h>

namespace tr {
namespace {

std::string trim(const std::string& s) {
    const size_t a = s.find_first_not_of(" \t\r\n"), b = s.find_last_not_of(" \t\r\n");
    return a == std::string::npos ? std::string() : s.substr(a, b - a + 1);
}

bool to_int(const std::string& v, int lo, int hi, int* out) {
    if (v.empty()) return false;
    char* end = nullptr;
    const long n = strtol(v.c_str(), &end, 10);
    if (*end || n < lo || n > hi) return false;
    *out = (int)n;
    return true;
}

}  // namespace

bool is_ipv4(const std::string& s) {
    int parts = 0;
    size_t i = 0;
    while (i <= s.size()) {
        size_t j = s.find('.', i);
        if (j == std::string::npos) j = s.size();
        int n = 0;
        if (j == i || j - i > 3) return false;
        for (size_t k = i; k < j; ++k)
            if (s[k] < '0' || s[k] > '9') return false;
        if (!to_int(s.substr(i, j - i), 0, 255, &n)) return false;
        ++parts;
        i = j + 1;
        if (j == s.size()) break;
    }
    return parts == 4;
}

std::string to_ini(const Settings& s) {
    std::string o = "[tune-remote]\n";
    o += "midi_port=" + s.midiPort + "\n";
    o += std::string("rtp_enabled=") + (s.rtpEnabled ? "1" : "0") + "\n";
    o += "rtp_name=" + s.rtpName + "\n";
    o += "rtp_port=" + std::to_string(s.rtpPort) + "\n";
    o += "channel=" + (s.channel == 0 ? std::string("omni") : std::to_string(s.channel)) + "\n";
    o += "scale_cc=" + std::to_string(s.scaleCc) + "\n";
    o += std::string("spelling=") + (s.spelling == Spelling::Flat ? "flats" : "sharps") + "\n";
    for (const Settings::Peer& p : s.rtpPeers)
        o += "rtp_peer=" + p.ip + ":" + std::to_string(p.port) + (p.name.empty() ? "" : " " + p.name) + "\n";
    return o;
}

Settings from_ini(const std::string& text, std::vector<std::string>* warnings) {
    Settings s;
    auto warn = [&](const std::string& w) { if (warnings) warnings->push_back(w); };
    size_t pos = 0;
    while (pos <= text.size()) {
        size_t nl = text.find('\n', pos);
        if (nl == std::string::npos) nl = text.size();
        const std::string line = trim(text.substr(pos, nl - pos));
        pos = nl + 1;
        if (line.empty() || line[0] == '#' || line[0] == ';' || line[0] == '[') continue;
        const size_t eq = line.find('=');
        if (eq == std::string::npos) { warn("not key=value: " + line); continue; }
        const std::string k = trim(line.substr(0, eq)), v = trim(line.substr(eq + 1));
        bool ok = true;
        if (k == "midi_port") s.midiPort = v;
        else if (k == "rtp_enabled") { ok = v == "0" || v == "1"; if (ok) s.rtpEnabled = v == "1"; }
        else if (k == "rtp_name") { ok = !v.empty() && v.size() <= 63; if (ok) s.rtpName = v; }
        else if (k == "rtp_port") ok = to_int(v, 1024, 65534, &s.rtpPort);   // data port = +1
        else if (k == "channel") { if (v == "omni") s.channel = 0; else ok = to_int(v, 1, 16, &s.channel); }
        else if (k == "scale_cc") ok = to_int(v, 0, 119, &s.scaleCc);         // 120..127 = channel mode
        else if (k == "spelling") {
            ok = v == "sharps" || v == "flats";
            if (ok) s.spelling = v == "flats" ? Spelling::Flat : Spelling::Sharp;
        } else if (k == "rtp_peer") {
            // ip:port, then the name it was found under
            Settings::Peer p;
            const size_t sp = v.find(' ');
            const std::string addr = v.substr(0, sp);
            const size_t colon = addr.rfind(':');
            ok = colon != std::string::npos && colon > 0 && is_ipv4(addr.substr(0, colon)) &&
                 to_int(addr.substr(colon + 1), 1, 65534, &p.port);
            if (ok) {
                p.ip = addr.substr(0, colon);
                if (sp != std::string::npos) p.name = trim(v.substr(sp + 1));
                bool dup = false;
                for (const Settings::Peer& q : s.rtpPeers) dup |= q == p;
                if (!dup && s.rtpPeers.size() < kMaxRtpPeers) s.rtpPeers.push_back(p);
            }
        } else { warn("unknown setting " + k); continue; }
        if (!ok) warn("bad value for " + k + ": \"" + v + "\", kept the default");
    }
    return s;
}

}  // namespace tr
