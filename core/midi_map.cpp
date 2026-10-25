// midi_map.cpp - see midi_map.h.
#include "midi_map.h"
#include <stdio.h>
#include "tune.h"

namespace tr {

namespace {

// A channel message for the log when it is ignored ("Program Change 5 ch 1").
std::string describe(const uint8_t* msg, size_t n) {
    static const char* const kTypes[8] = {"Note Off", "Note On", "Poly Pressure", "CC", "Program Change",
                                          "Channel Pressure", "Pitch Bend", ""};
    std::string s = kTypes[(msg[0] >> 4) & 7];
    for (size_t i = 1; i < n && i < 3; ++i) s += " " + std::to_string(msg[i]);
    return s + " ch " + std::to_string((msg[0] & 0x0F) + 1);
}

}  // namespace

Interpreted interpret(const uint8_t* msg, size_t n, int channel, int scaleCc) {
    Interpreted r;
    if (!msg || n == 0 || msg[0] < 0x80 || msg[0] >= 0xF0) return r;   // system messages: not even logged
    r.text = describe(msg, n);
    const int type = msg[0] & 0xF0, ch = (msg[0] & 0x0F) + 1;
    if (n < 3 || msg[1] > 0x7F || msg[2] > 0x7F) return r;
    if (type != 0x90 && type != 0xB0) return r;
    if (channel != 0 && ch != channel) return r;
    char buf[96];
    if (type == 0x90) {
        if (msg[2] == 0) return r;                                       // Note On vel 0 = Note Off
        r.kind = MsgKind::Key;
        r.value = msg[1] % 12;
        snprintf(buf, sizeof buf, "Note On %d (%s) vel %d ch %d", msg[1], pitch_class_name(r.value).c_str(),
                 msg[2], ch);
    } else {
        if (msg[1] != scaleCc) return r;
        r.value = msg[2];
        if (const char* s = scale_name(msg[2])) {
            r.kind = MsgKind::Scale;
            snprintf(buf, sizeof buf, "CC %d = %d (%s) ch %d", msg[1], msg[2], s, ch);
        } else {
            r.kind = MsgKind::OutOfRange;
            snprintf(buf, sizeof buf, "CC %d = %d ch %d ignored: Tune's scales are 0-%d", msg[1], msg[2], ch,
                     kNumScales - 1);
        }
    }
    r.text = buf;
    return r;
}

void Coalescer::add(const Interpreted& m, uint64_t nowMs) {
    if (m.kind != MsgKind::Key && m.kind != MsgKind::Scale) return;
    if (cur_.empty()) dueAt_ = nowMs + kWindowMs;
    (m.kind == MsgKind::Key ? cur_.pitchClass : cur_.scale) = m.value;
}

void Coalescer::add_now(const Intent& in, uint64_t nowMs) {
    if (in.pitchClass >= 0) cur_.pitchClass = in.pitchClass;
    if (in.scale >= 0) cur_.scale = in.scale;
    if (!cur_.empty()) dueAt_ = nowMs;
}

int64_t Coalescer::due_in(uint64_t nowMs) const {
    if (cur_.empty()) return -1;
    return nowMs >= dueAt_ ? 0 : (int64_t)(dueAt_ - nowMs);
}

Intent Coalescer::take() {
    Intent r = cur_;
    cur_ = Intent{};
    return r;
}

}  // namespace tr
