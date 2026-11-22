// rtp_codec.cpp - see rtp_codec.h. Copied from Headlink's tools/preprobe/rtpmidi.cpp
// (demo/pre-probe), the codec half, unchanged.
#include "rtp_codec.h"

#include <stdio.h>
#include <string.h>

namespace rtpmidi {

// ================================================================ codec

namespace {
void put16(std::vector<uint8_t>& v, uint16_t x) { v.push_back(uint8_t(x >> 8)); v.push_back(uint8_t(x)); }
void put32(std::vector<uint8_t>& v, uint32_t x) { put16(v, uint16_t(x >> 16)); put16(v, uint16_t(x)); }
void put64(std::vector<uint8_t>& v, uint64_t x) { put32(v, uint32_t(x >> 32)); put32(v, uint32_t(x)); }
uint16_t get16(const uint8_t* p) { return uint16_t((p[0] << 8) | p[1]); }
uint32_t get32(const uint8_t* p) { return (uint32_t(get16(p)) << 16) | get16(p + 2); }
uint64_t get64(const uint8_t* p) { return (uint64_t(get32(p)) << 32) | get32(p + 4); }

void header(std::vector<uint8_t>& v, char a, char b) { v.push_back(0xFF); v.push_back(0xFF); v.push_back(uint8_t(a)); v.push_back(uint8_t(b)); }
}  // namespace

const char* am_name(AmCmd c) {
    switch (c) {
    case AmCmd::In: return "IN"; case AmCmd::Ok: return "OK"; case AmCmd::No: return "NO";
    case AmCmd::By: return "BY"; case AmCmd::Ck: return "CK"; case AmCmd::Rs: return "RS";
    case AmCmd::Rl: return "RL"; case AmCmd::Unknown: return "??"; default: return "-";
    }
}

bool am_is_session(const uint8_t* p, size_t n) { return n >= 4 && p[0] == 0xFF && p[1] == 0xFF; }

bool am_parse(const uint8_t* p, size_t n, AmPacket* out) {
    *out = AmPacket();
    if (!am_is_session(p, n)) return false;
    out->raw[0] = char(p[2]); out->raw[1] = char(p[3]);
    const char a = char(p[2]), b = char(p[3]);
    if ((a == 'I' && b == 'N') || (a == 'O' && b == 'K') || (a == 'N' && b == 'O')) {
        if (n < 16) return false;
        out->cmd = a == 'I' ? AmCmd::In : a == 'O' ? AmCmd::Ok : AmCmd::No;
        out->version = get32(p + 4); out->token = get32(p + 8); out->ssrc = get32(p + 12);
        // The name is NUL-terminated and optional; stop at the end of the
        // datagram if the terminator is missing.
        for (size_t i = 16; i < n && p[i]; ++i) out->name.push_back(char(p[i]));
        return true;
    }
    if (a == 'B' && b == 'Y') {
        if (n < 16) return false;
        out->cmd = AmCmd::By;
        out->version = get32(p + 4); out->token = get32(p + 8); out->ssrc = get32(p + 12);
        return true;
    }
    if (a == 'C' && b == 'K') {
        if (n < 36) return false;
        out->cmd = AmCmd::Ck;
        out->ssrc = get32(p + 4); out->count = p[8];
        for (int i = 0; i < 3; ++i) out->ts[i] = get64(p + 12 + 8 * i);
        return true;
    }
    if (a == 'R' && b == 'S') {
        if (n < 12) return false;
        out->cmd = AmCmd::Rs;
        out->ssrc = get32(p + 4); out->seq = get32(p + 8);
        return true;
    }
    if (a == 'R' && b == 'L') {
        if (n < 12) return false;
        out->cmd = AmCmd::Rl;
        out->ssrc = get32(p + 4); out->limit = get32(p + 8);
        return true;
    }
    out->cmd = AmCmd::Unknown;
    return true;
}

std::vector<uint8_t> am_invite(AmCmd cmd, uint32_t token, uint32_t ssrc, const std::string& name) {
    std::vector<uint8_t> v;
    header(v, cmd == AmCmd::In ? 'I' : cmd == AmCmd::Ok ? 'O' : 'N', cmd == AmCmd::In ? 'N' : cmd == AmCmd::Ok ? 'K' : 'O');
    put32(v, 2); put32(v, token); put32(v, ssrc);
    for (char c : name) v.push_back(uint8_t(c));
    v.push_back(0);
    return v;
}

std::vector<uint8_t> am_bye(uint32_t token, uint32_t ssrc) {
    std::vector<uint8_t> v;
    header(v, 'B', 'Y'); put32(v, 2); put32(v, token); put32(v, ssrc);
    return v;
}

std::vector<uint8_t> am_clock(uint32_t ssrc, uint8_t count, uint64_t ts1, uint64_t ts2, uint64_t ts3) {
    std::vector<uint8_t> v;
    header(v, 'C', 'K'); put32(v, ssrc);
    v.push_back(count); v.push_back(0); v.push_back(0); v.push_back(0);
    put64(v, ts1); put64(v, ts2); put64(v, ts3);
    return v;
}

std::vector<uint8_t> am_feedback(uint32_t ssrc, uint32_t extSeq) {
    std::vector<uint8_t> v;
    header(v, 'R', 'S'); put32(v, ssrc); put32(v, extSeq);
    return v;
}

int midi_data_len(uint8_t s) {
    if (s < 0x80) return 0;
    switch (s & 0xF0) {
    case 0x80: case 0x90: case 0xA0: case 0xB0: case 0xE0: return 2;
    case 0xC0: case 0xD0: return 1;
    default: break;
    }
    switch (s) {
    case 0xF0: return -1;
    case 0xF1: case 0xF3: return 1;
    case 0xF2: return 2;
    default: return 0;   // F4..F7 (F7 = end of SysEx segment) and realtime F8..FF
    }
}

bool rtp_is_midi(const uint8_t* p, size_t n) { return n >= 12 && (p[0] & 0xC0) == 0x80 && (p[1] & 0x7F) == 0x61; }

bool rtp_parse(const uint8_t* p, size_t n, RtpMidiPacket* out, std::string* err) {
    *out = RtpMidiPacket();
    auto fail = [&](const char* why) { if (err) *err = why; return false; };
    if (!rtp_is_midi(p, n)) return fail("not RTP-MIDI (version 2, payload type 0x61)");
    const int cc = p[0] & 0x0F;                  // CSRC count: none expected, skipped if present
    size_t i = 12 + 4 * size_t(cc);
    if (p[0] & 0x10) {                           // RTP header extension: skip it
        if (n < i + 4) return fail("truncated RTP header extension");
        i += 4 + 4 * size_t(get16(p + i + 2));
    }
    out->seq = get16(p + 2); out->ts = get32(p + 4); out->ssrc = get32(p + 8);
    if (n < i + 1) return fail("no MIDI command section");
    const uint8_t h = p[i];
    out->b = h & 0x80; out->j = h & 0x40; out->z = h & 0x20; out->p = h & 0x10;
    size_t len = h & 0x0F;
    ++i;
    if (out->b) {
        if (n < i + 1) return fail("truncated long LEN");
        len = (len << 8) | p[i];
        ++i;
    }
    out->listLen = len;
    if (n < i + len) return fail("MIDI list longer than the packet");
    const size_t end = i + len;
    out->journalLen = n - end;       // with J off this should be 0; kept for the log either way

    uint8_t running = 0;
    bool first = true;
    while (i < end) {
        MidiEvent ev;
        if (!first || out->z) {                  // delta time, 1-4 bytes
            uint32_t d = 0; int k = 0;
            for (;;) {
                if (i >= end) return fail("truncated delta time");
                if (++k > 4) return fail("delta time longer than 4 bytes");
                const uint8_t c = p[i++];
                d = (d << 7) | (c & 0x7F);
                if (!(c & 0x80)) break;
            }
            ev.delta = d;
        }
        if (i >= end) return fail("delta time with no command");
        uint8_t s = p[i];
        if (s & 0x80) {
            ++i;
        } else {
            if (!running) return fail("data byte with no running status");
            s = running;
            ev.running = true;
        }
        ev.bytes.push_back(s);
        const int dl = midi_data_len(s);
        if (dl < 0 || s == 0xF7) {
            // SysEx: F0 (start) or F7 (continuation of a segmented one). Runs to
            // F7 (end), F0 (more segments follow) or F4 (cancelled).
            bool closed = false;
            while (i < end) {
                const uint8_t c = p[i++];
                ev.bytes.push_back(c);
                if (c == 0xF7 || c == 0xF0 || c == 0xF4) { closed = true; break; }
                if (c & 0x80) return fail("status byte inside SysEx");
            }
            if (!closed) return fail("SysEx without an end byte");
            running = 0;
        } else {
            for (int k = 0; k < dl; ++k) {
                if (i >= end) return fail("truncated MIDI command");
                if (p[i] & 0x80) return fail("status byte where a data byte belongs");
                ev.bytes.push_back(p[i++]);
            }
            if (s < 0xF0) running = s;           // channel messages set running status
            else if (s < 0xF8) running = 0;      // system common clears it; realtime leaves it
        }
        out->events.push_back(std::move(ev));
        first = false;
    }
    return true;
}

std::vector<uint8_t> rtp_build(uint16_t seq, uint32_t ts, uint32_t ssrc,
                               const std::vector<std::vector<uint8_t>>& msgs) {
    std::vector<uint8_t> list;
    for (size_t m = 0; m < msgs.size(); ++m) {
        if (m) list.push_back(0x00);             // delta 0 before every command after the first
        list.insert(list.end(), msgs[m].begin(), msgs[m].end());
    }
    std::vector<uint8_t> v;
    v.push_back(0x80); v.push_back(0x61);
    put16(v, seq); put32(v, ts); put32(v, ssrc);
    if (list.size() <= 15) {
        v.push_back(uint8_t(list.size()));
    } else {
        const size_t len = list.size() > 0x0FFF ? 0x0FFF : list.size();
        v.push_back(uint8_t(0x80 | (len >> 8))); v.push_back(uint8_t(len & 0xFF));
        list.resize(len);
    }
    v.insert(v.end(), list.begin(), list.end());
    return v;
}

std::string hex(const uint8_t* p, size_t n) {
    std::string s; char b[4];
    for (size_t i = 0; i < n; ++i) { snprintf(b, sizeof b, i ? " %02X" : "%02X", p[i]); s += b; }
    return s;
}
std::string hex(const std::vector<uint8_t>& v) { return hex(v.data(), v.size()); }

std::string describe(const AmPacket& a) {
    char b[256];
    switch (a.cmd) {
    case AmCmd::In: case AmCmd::Ok: case AmCmd::No:
        snprintf(b, sizeof b, "%s v%u token 0x%08X ssrc 0x%08X name '%s'", am_name(a.cmd), a.version, a.token, a.ssrc, a.name.c_str());
        break;
    case AmCmd::By:
        snprintf(b, sizeof b, "BY v%u token 0x%08X ssrc 0x%08X", a.version, a.token, a.ssrc);
        break;
    case AmCmd::Ck:
        snprintf(b, sizeof b, "CK%u ssrc 0x%08X ts %llu %llu %llu", a.count, a.ssrc,
                 (unsigned long long)a.ts[0], (unsigned long long)a.ts[1], (unsigned long long)a.ts[2]);
        break;
    case AmCmd::Rs: snprintf(b, sizeof b, "RS ssrc 0x%08X seq %u (wraps %u)", a.ssrc, a.seq & 0xFFFF, a.seq >> 16); break;
    case AmCmd::Rl: snprintf(b, sizeof b, "RL ssrc 0x%08X limit %u", a.ssrc, a.limit); break;
    default: snprintf(b, sizeof b, "unknown session command '%c%c'", a.raw[0] ? a.raw[0] : '?', a.raw[1] ? a.raw[1] : '?'); break;
    }
    return b;
}

}  // namespace rtpmidi
