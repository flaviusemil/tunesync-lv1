// apply.cpp - see apply.h.
#include "apply.h"
#include <math.h>
#include <stdio.h>

namespace tr {
namespace {

// A parameter value LV1 reports as a double; Tune's are whole numbers.
bool whole(double v, int* out) {
    const double r = floor(v + 0.5);
    if (fabs(v - r) > 1e-6 || r < -1e6 || r > 1e6) return false;
    *out = (int)r;
    return true;
}

std::string where(Mixer& m, const TrackRange& r, int index, int slot) {
    std::string s = std::string(r.label) + " " + std::to_string(index + 1);
    const std::string name = m.track_name(r.trackType, index);
    if (!name.empty()) s += " " + name;
    if (slot > 0) s += " (slot " + std::to_string(slot + 1) + ")";
    return s;
}

// Reads a parameter that must be a whole number whose text is names(value).
bool read_named(Mixer& m, const TuneInstance& t, int param, const char* (*names)(int), int* out, std::string* why) {
    const ParamRead r = m.get(t.trackType, t.trackIndex, t.slot, param, kCidValue);
    int v = -1;
    const char* expect = nullptr;
    if (r.ok && whole(r.value, &v)) expect = names(v);
    if (expect && r.text == expect) { *out = v; return true; }
    char buf[128];
    snprintf(buf, sizeof buf, "param %d reads %s %.3f \"%s\"", param, r.ok ? "" : "(no answer)", r.value,
             r.text.c_str());
    *why = buf;
    return false;
}

const char* receive_name(int v) { return v == 1 ? "Rcv" : v == 0 ? "Lock" : nullptr; }

std::string join(const std::vector<std::string>& v) {
    std::string s;
    for (const std::string& x : v) s += (s.empty() ? "" : ", ") + x;
    return s;
}

}  // namespace

std::vector<TrackRange> default_ranges() { return {{0, 128, "Ch"}, {1, 16, "Gp"}, {2, 16, "Fx"}}; }

std::vector<TuneInstance> discover(Mixer& m, const std::vector<TrackRange>& ranges) {
    std::vector<TuneInstance> out;
    for (const TrackRange& r : ranges)
        for (int i = 0; i < r.count; ++i)
            for (int s = 0; s < kInsertSlots; ++s) {
                const ParamRead code = m.get(r.trackType, i, s, kSlotInfo, kCidSlotCode);
                if (!code.ok || !is_tune_code(code.text)) continue;
                TuneInstance t;
                t.trackType = r.trackType; t.trackIndex = i; t.slot = s;
                t.where = where(m, r, i, s);
                int rcv = -1;
                t.layoutOk = read_named(m, t, kParamKey, key_name, &t.key, &t.why) &&
                             read_named(m, t, kParamScale, scale_name, &t.scale, &t.why) &&
                             read_named(m, t, kParamReceive, receive_name, &rcv, &t.why);
                t.receive = rcv != 0;
                out.push_back(t);
            }
    return out;
}

ApplyResult apply(Mixer& m, const std::vector<TuneInstance>& tunes, const Intent& in, Spelling fallback) {
    ApplyResult res;
    if (in.pitchClass >= 0) res.label = pitch_class_name(in.pitchClass);
    if (const char* s = scale_name(in.scale)) res.label += (res.label.empty() ? "" : " ") + std::string(s);
    for (const TuneInstance& t : tunes) {
        if (!t.layoutOk) { res.bad.push_back(t.where + ": " + t.why); continue; }
        if (!t.receive) { res.locked.push_back(t.where); continue; }
        const int key = in.pitchClass >= 0 ? key_value(in.pitchClass, keep_spelling(t.key, fallback)) : -1;
        const bool doKey = key >= 0 && key != t.key, doScale = in.scale >= 0 && in.scale != t.scale;
        if (!doKey && !doScale) { res.unchanged.push_back(t.where); continue; }
        std::string err;
        if (doKey && !m.set(t.trackType, t.trackIndex, t.slot, kParamKey, key)) err = "key not accepted";
        if (doScale && !m.set(t.trackType, t.trackIndex, t.slot, kParamScale, in.scale))
            err += (err.empty() ? "" : ", ") + std::string("scale not accepted");
        // Read back what LV1 now shows: the only proof the change landed.
        int nowKey = -1, nowScale = -1;
        std::string why;
        if (err.empty() && doKey && !(read_named(m, t, kParamKey, key_name, &nowKey, &why) && nowKey == key))
            err = why.empty() ? std::string("key reads ") + (key_name(nowKey) ? key_name(nowKey) : "?") : why;
        if (err.empty() && doScale &&
            !(read_named(m, t, kParamScale, scale_name, &nowScale, &why) && nowScale == in.scale))
            err = why.empty() ? std::string("scale reads ") + (scale_name(nowScale) ? scale_name(nowScale) : "?") : why;
        if (!err.empty()) { res.failed.push_back(t.where + ": " + err); continue; }
        res.set.push_back(doKey ? t.where + " -> " + key_name(key) : t.where);
    }
    return res;
}

std::string ApplyResult::summary() const {
    std::string s = label + ": ";
    const size_t n = set.size() + unchanged.size() + locked.size() + bad.size() + failed.size();
    if (n == 0) return s + "no Tune RT found";
    s += "set " + std::to_string(set.size());
    if (!set.empty()) s += " (" + join(set) + ")";
    if (!unchanged.empty()) s += ", already there " + std::to_string(unchanged.size()) + " (" + join(unchanged) + ")";
    if (!locked.empty()) s += ", skipped " + std::to_string(locked.size()) + " locked (" + join(locked) + ")";
    if (!bad.empty()) s += ", skipped " + std::to_string(bad.size()) + " unexpected Tune layout (" + join(bad) + ")";
    if (!failed.empty()) s += ", FAILED " + std::to_string(failed.size()) + " (" + join(failed) + ")";
    return s;
}

}  // namespace tr
