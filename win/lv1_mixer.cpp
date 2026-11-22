// lv1_mixer.cpp - see lv1_mixer.h.
#ifndef WIN32_LEAN_AND_MEAN
#  define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include "lv1_mixer.h"
#include <math.h>
#include <string.h>
#include "../core/tune.h"

namespace trw {

using namespace wmsd;

bool guarded_call(int (*fn)(void*, void*), void* core, void* arg, int* rc) {
    __try {
        *rc = fn(core, arg);
        return true;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        *rc = (int)GetExceptionCode();
        return false;
    }
}

bool safe_copy(void* dst, const void* src, size_t n) {
    SIZE_T got = 0;
    return src && ReadProcessMemory(GetCurrentProcess(), src, dst, n, &got) && got == n;
}

std::string control_text(const MixerControl& c) {
    size_t n = 0;
    while (n < sizeof c.text && c.text[n]) ++n;
    return std::string(c.text, n);
}

bool Lv1Mixer::call(bool isSet, MixerControl* c, int* rc) {
    if (faulted_ || !core_) return false;
    ++calls_;
    int (*fn)(void*, void*) = isSet ? (int (*)(void*, void*))cb_.SetMixerControl : (int (*)(void*, void*))cb_.GetMixerControl;
    if (!fn) return false;
    if (!guarded_call(fn, core_, c, rc)) {
        faultCode_ = (unsigned)*rc;
        faulted_ = true;
        return false;
    }
    return true;
}

bool Lv1Mixer::read(MixerControl* c) {
    int rc = 0;
    return call(false, c, &rc) && rc == 0;
}

tr::ParamRead Lv1Mixer::get(int trackType, int trackIndex, int slot, int channel, int cid) {
    tr::ParamRead r;
    MixerControl c;
    init_control(&c, kCtlControl, (uint32_t)trackType, (uint32_t)trackIndex, kSecPlugins, (uint32_t)cid);
    c.id.sectionIndex = (uint32_t)slot;
    c.id.channel = (uint32_t)channel;
    c.valueType = 0;
    if (!read(&c)) return r;
    r.ok = true;
    r.value = c.value;
    r.text = control_text(c);
    return r;
}

bool Lv1Mixer::set(int trackType, int trackIndex, int slot, int param, double value) {
    // Touch, Control(value), Release: what a surface knob does, and what moved Tune's key
    // on the workstation. Touch and Release carry the value as text, as Mackie's do.
    auto send = [&](uint32_t ctlType) {
        MixerControl c;
        init_control(&c, ctlType, (uint32_t)trackType, (uint32_t)trackIndex, kSecPlugins, tr::kCidValue);
        c.id.sectionIndex = (uint32_t)slot;
        c.id.channel = (uint32_t)param;
        c.value = value;
        if (ctlType != kCtlControl) _snprintf_s(c.text, sizeof c.text, _TRUNCATE, "%f", value);
        int rc = 0;
        return call(true, &c, &rc) && rc == 0;
    };
    if (!send(kCtlTouch)) return false;
    const bool ok = send(kCtlControl);
    // Wait (bounded) until LV1 shows it; apply() reads it back and judges.
    if (ok) {
        const ULONGLONG until = GetTickCount64() + kSettleMs;
        for (;;) {
            const tr::ParamRead r = get(trackType, trackIndex, slot, param, tr::kCidValue);
            if (!r.ok || fabs(r.value - value) < 1e-6 || GetTickCount64() >= until) break;
            Sleep(2);
        }
    }
    send(kCtlRelease);
    return ok;
}

std::string Lv1Mixer::track_name(int trackType, int trackIndex) {
    MixerControl c;
    init_control(&c, kCtlControl, (uint32_t)trackType, (uint32_t)trackIndex, kSecName, 0xFFFFFFFFu);
    if (!read(&c)) return std::string();
    return control_text(c);
}

}  // namespace trw
