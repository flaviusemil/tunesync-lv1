// lv1_mixer.h - tr::Mixer on LV1's own surface callbacks (GetMixerControl /
// SetMixerControl), the way the fader probe proved Tune RT control on LV1 16.5.
//
// Only the engine's worker thread calls in here, never LV1's own threads. Every call
// into LV1 is SEH-guarded; the first fault makes this mixer refuse every later call
// (Tune Remote then stays passive and says so in the log) rather than poking a core
// that just faulted.
//
// SetMixerControl is asynchronous: it returns at once and a read straight after still
// shows the old value (rig, 2026-10-20). set() therefore sends Touch, Control, waits
// until LV1 reads back the new value (bounded), then Release - so apply()'s read-back
// sees what LV1 really holds.
#pragma once
#include <atomic>
#include <string>
#include "../core/apply.h"
#include "wmsd_abi.h"

namespace trw {

class Lv1Mixer : public tr::Mixer {
  public:
    Lv1Mixer(void* core, const wmsd::CoreCallbacks& cb) : core_(core), cb_(cb) {}

    tr::ParamRead get(int trackType, int trackIndex, int slot, int channel, int cid) override;
    bool set(int trackType, int trackIndex, int slot, int param, double value) override;
    std::string track_name(int trackType, int trackIndex) override;

    bool faulted() const { return faulted_.load(); }
    unsigned fault_code() const { return faultCode_.load(); }
    unsigned calls() const { return calls_.load(); }

    static constexpr int kSettleMs = 400;   // how long set() waits for LV1 to show the value

  private:
    bool call(bool isSet, wmsd::MixerControl* c, int* rc);
    bool read(wmsd::MixerControl* c);

    void*                 core_;
    wmsd::CoreCallbacks   cb_;
    std::atomic<bool>     faulted_{false};
    std::atomic<unsigned> faultCode_{0};
    std::atomic<unsigned> calls_{0};
};

// SEH around one LV1 callback. A plain function on purpose: __try cannot share a frame
// with C++ objects that need unwinding. False when it faulted (*rc = the exception code).
bool guarded_call(int (*fn)(void*, void*), void* core, void* arg, int* rc);

// Copies n bytes LV1 handed us without trusting the pointer: ReadProcessMemory on our
// own process fails cleanly on an unmapped address. (A __try around memcpy does not:
// clang-cl drops the SEH scope around it, which crashed LV1 in the fader spike.)
bool safe_copy(void* dst, const void* src, size_t n);

// UTF-8 text of a control, NUL-terminated within its field.
std::string control_text(const wmsd::MixerControl& c);

}  // namespace trw
