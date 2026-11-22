// core_test.cpp - core/ against a fake LV1: Tune tables and spelling, MIDI
// interpretation, coalescing, discovery, apply (Lock, layout checks, read-back),
// settings and the engine end to end. `make test`.
#include <stdio.h>
#include <map>
#include <set>
#include <string>
#include <tuple>
#include <vector>
#include "../core/apply.h"
#include "../core/engine.h"
#include "../core/log.h"
#include "../core/midi_map.h"
#include "../core/settings.h"
#include "../core/tune.h"

using namespace tr;
static int g_fail = 0, g_checks = 0;
#define CHECK(c, ...) do { ++g_checks; if (!(c)) { ++g_fail; printf("FAIL %s:%d  ", __FILE__, __LINE__); printf(__VA_ARGS__); printf("\n"); } } while (0)
static bool has(const std::string& s, const char* sub) { return s.find(sub) != std::string::npos; }

// ---------------------------------------------------------------- fake LV1
struct FakeSlot {
    std::string name, code;
    std::map<int, double> params;          // param index -> value
    std::map<int, std::string> textOverride;  // a "Tune update" that reshuffled params
};
struct FakeMixer : Mixer {
    std::map<std::tuple<int, int, int>, FakeSlot> slots;   // (trackType, index, slot)
    std::map<std::pair<int, int>, std::string> names;
    std::set<std::tuple<int, int, int, int>> refuse;       // sets that LV1 "ignores"
    int gets = 0, sets = 0;

    FakeSlot& tune(int tt, int ti, int slot, int key, int scale, bool receive, const char* code = "LVLM") {
        FakeSlot& s = slots[{tt, ti, slot}];
        s.name = "Tune RT"; s.code = code;
        s.params = {{kParamKey, key}, {kParamScale, scale}, {kParamReceive, receive ? 1 : 0}, {41, 1}};
        return s;
    }
    static std::string text_of(int param, double v) {
        const int i = (int)v;
        const char* t = param == kParamKey ? key_name(i) : param == kParamScale ? scale_name(i)
                      : param == kParamReceive ? (i ? "Rcv" : "Lock") : "";
        return t ? t : "";
    }
    ParamRead get(int tt, int ti, int slot, int ch, int cid) override {
        ++gets;
        ParamRead r;
        auto it = slots.find({tt, ti, slot});
        r.ok = true;                                     // LV1 answers empty slots with empty text
        if (it == slots.end()) return r;
        const FakeSlot& s = it->second;
        if (ch == kSlotInfo) { r.text = cid == kCidSlotName ? s.name : cid == kCidSlotCode ? s.code : ""; return r; }
        if (cid != kCidValue) return r;
        auto p = s.params.find(ch);
        if (p == s.params.end()) { r.value = -1; return r; }
        r.value = p->second;
        auto o = s.textOverride.find(ch);
        r.text = o != s.textOverride.end() ? o->second : text_of(ch, p->second);
        return r;
    }
    bool set(int tt, int ti, int slot, int param, double v) override {
        ++sets;
        if (refuse.count({tt, ti, slot, param})) return true;   // accepted, silently not applied
        auto it = slots.find({tt, ti, slot});
        if (it == slots.end()) return false;
        it->second.params[param] = v;
        return true;
    }
    std::string track_name(int tt, int ti) override {
        auto it = names.find({tt, ti});
        return it == names.end() ? std::string() : it->second;
    }
    int key(int tt, int ti, int slot) { return (int)slots[{tt, ti, slot}].params[kParamKey]; }
    int scale(int tt, int ti, int slot) { return (int)slots[{tt, ti, slot}].params[kParamScale]; }
};

static std::vector<uint8_t> note(int ch, int n, int vel) { return {(uint8_t)(0x90 | (ch - 1)), (uint8_t)n, (uint8_t)vel}; }
static std::vector<uint8_t> cc(int ch, int c, int v) { return {(uint8_t)(0xB0 | (ch - 1)), (uint8_t)c, (uint8_t)v}; }
static Interpreted in(const std::vector<uint8_t>& m, int ch = 16, int scaleCc = 20) {
    return interpret(m.data(), m.size(), ch, scaleCc);
}

int main() {
    // ---- Tune tables: the values LV1 showed on the workstation
    CHECK(std::string(key_name(0)) == "C" && std::string(key_name(13)) == "A" && std::string(key_name(16)) == "B", "keys");
    CHECK(key_name(17) == nullptr && key_name(-1) == nullptr, "key range");
    CHECK(std::string(scale_name(3)) == "Natural Minor" && std::string(scale_name(10)) == "Mixolydian", "scales");
    CHECK(key_value(1, Spelling::Sharp) == 1 && key_value(1, Spelling::Flat) == 2, "C#/Db");
    CHECK(key_value(3, Spelling::Flat) == 5 && key_value(6, Spelling::Flat) == 9 && key_value(10, Spelling::Flat) == 15, "flats");
    CHECK(key_value(9, Spelling::Flat) == 13 && key_value(9, Spelling::Sharp) == 13, "A is A");
    CHECK(key_value(11, Spelling::Sharp) == 16 && key_value(12, Spelling::Sharp) == -1, "B, range");
    CHECK(key_spelling(5) == Spelling::Flat && key_spelling(4) == Spelling::Sharp && key_spelling(13) == Spelling::Natural, "spelling");
    CHECK(keep_spelling(5, Spelling::Sharp) == Spelling::Flat, "Eb keeps flats");
    CHECK(keep_spelling(13, Spelling::Flat) == Spelling::Flat && keep_spelling(13, Spelling::Sharp) == Spelling::Sharp, "A uses the default");
    for (int v = 0; v < kNumKeys; ++v) CHECK(key_value(key_pitch_class(v), key_spelling(v) == Spelling::Flat ? Spelling::Flat : Spelling::Sharp) == v, "round trip %d", v);
    CHECK(pitch_class_name(1) == "C#/Db" && pitch_class_name(9) == "A", "pc names");
    CHECK(is_tune_code("LVLM") && is_tune_code("LVLS") && !is_tune_code("VXPM") && !is_tune_code(""), "codes");

    // ---- MIDI interpretation
    {
        Interpreted m = in(note(16, 57, 100));
        CHECK(m.kind == MsgKind::Key && m.value == 9 && has(m.text, "Note On 57 (A)"), "note A: %s", m.text.c_str());
        CHECK(in(note(16, 61, 1)).value == 1, "C# any octave");
        CHECK(in(note(16, 57, 0)).kind == MsgKind::Ignored, "vel 0 = note off");
        CHECK(in({0x8F, 57, 64}).kind == MsgKind::Ignored, "note off");
        CHECK(in(note(15, 57, 100)).kind == MsgKind::Ignored && has(in(note(15, 57, 100)).text, "ch 15"), "other channel");
        CHECK(in(note(3, 57, 100), 0).kind == MsgKind::Key, "omni");
        m = in(cc(16, 20, 3));
        CHECK(m.kind == MsgKind::Scale && m.value == 3 && has(m.text, "Natural Minor"), "cc scale: %s", m.text.c_str());
        CHECK(in(cc(16, 20, 14)).kind == MsgKind::OutOfRange, "cc out of range");
        CHECK(in(cc(16, 21, 3)).kind == MsgKind::Ignored, "other cc");
        CHECK(in(cc(16, 7, 3), 16, 7).kind == MsgKind::Scale, "configurable cc");
        CHECK(in({0x9F, 57}).kind == MsgKind::Ignored, "truncated");
        CHECK(in({0xF8}).text.empty() && in({0xF1, 0x20}).text.empty(), "clock and MTC are silent");
        CHECK(in({0xCF, 5}).kind == MsgKind::Ignored && has(in({0xCF, 5}).text, "Program Change 5"), "program change described");
    }

    // ---- coalescing
    {
        Coalescer c;
        CHECK(c.due_in(0) == -1, "idle");
        c.add(in(cc(16, 20, 3)), 1000);
        c.add(in(note(16, 57, 100)), 1005);
        CHECK(c.due_in(1010) == 20 && c.due_in(1030) == 0, "window from the first message");
        Intent i = c.take();
        CHECK(i.pitchClass == 9 && i.scale == 3 && !c.pending(), "both in one change");
        c.add(in(note(16, 60, 100)), 2000);
        c.add(in(note(16, 62, 100)), 2010);
        CHECK(c.take().pitchClass == 2 && true, "last key wins");
        c.add(in(cc(16, 21, 3)), 3000);
        CHECK(!c.pending(), "ignored messages are not a change");
        c.add(in(note(16, 60, 100)), 4000);
        Intent t; t.scale = 2;
        c.add_now(t, 4001);
        CHECK(c.due_in(4001) == 0, "apply now is due at once");
        i = c.take();
        CHECK(i.pitchClass == 0 && i.scale == 2, "apply now merges");
    }

    // ---- discovery and apply
    {
        FakeMixer m;
        m.names[{0, 42}] = "Vox 1";
        m.tune(0, 42, 0, 13, 3, true);           // Ch 43: A Natural Minor
        m.tune(0, 43, 2, 5, 2, true);            // Ch 44 slot 3: Eb Major (flat speller)
        m.tune(0, 44, 0, 8, 2, false);           // Ch 45: F# Major, Lock
        m.tune(1, 0, 1, 1, 2, true, "LVLS");     // Gp 1 slot 2: C# Major, stereo
        m.slots[{0, 10, 0}] = FakeSlot{"PSE", "VXPM", {}, {}};
        auto found = discover(m, default_ranges());
        CHECK(found.size() == 4, "found %zu Tunes", found.size());
        CHECK(found[0].where == "Ch 43 Vox 1" && found[0].key == 13 && found[0].scale == 3 && found[0].receive, "ch 43: %s", found[0].where.c_str());
        CHECK(found[1].where == "Ch 44 (slot 3)", "slot shown: %s", found[1].where.c_str());
        CHECK(!found[2].receive && found[2].layoutOk, "lock read");
        CHECK(found[3].where == "Gp 1 (slot 2)", "group: %s", found[3].where.c_str());
        CHECK(found[0].label == "Ch 43" && found[0].name == "Vox 1" && found[1].label == "Ch 44" && found[1].name.empty(),
              "label and name apart");

        Intent i; i.pitchClass = 1; i.scale = 3;    // the C#/Db note + Natural Minor
        ApplyResult r = apply(m, found, i, Spelling::Sharp);
        CHECK(m.key(0, 42, 0) == 1, "A (natural) -> default sharps: C#, got %d", m.key(0, 42, 0));
        CHECK(m.key(0, 43, 2) == 2, "Eb (flat) -> Db, got %d", m.key(0, 43, 2));
        CHECK(m.key(1, 0, 1) == 1, "C# stays sharp, got %d", m.key(1, 0, 1));
        CHECK(m.key(0, 44, 0) == 8 && m.scale(0, 44, 0) == 2, "locked untouched");
        CHECK(m.scale(0, 42, 0) == 3 && m.scale(0, 43, 2) == 3 && m.scale(1, 0, 1) == 3, "scales set");
        CHECK(r.set.size() == 3 && r.locked.size() == 1 && r.failed.empty() && r.bad.empty(), "counts");
        const std::string s = r.summary();
        CHECK(has(s, "C#/Db Natural Minor: set 3") && has(s, "Ch 44 (slot 3) -> Db") && has(s, "skipped 1 locked (Ch 45)"), "summary: %s", s.c_str());

        // natural key + flats default
        found = discover(m, default_ranges());
        Intent j; j.pitchClass = 10;                // A#/Bb, key only
        m.tune(0, 42, 0, 13, 3, true);
        found = discover(m, default_ranges());
        apply(m, found, j, Spelling::Flat);
        CHECK(m.key(0, 42, 0) == 15 && m.scale(0, 42, 0) == 3, "A -> Bb with flats, scale kept");

        // already there: no write
        found = discover(m, default_ranges());
        const int before = m.sets;
        r = apply(m, found, j, Spelling::Flat);
        CHECK(m.sets == before && r.set.empty() && !r.unchanged.empty() && has(r.summary(), "already there"), "no needless writes: %s", r.summary().c_str());

        // a Tune whose parameters moved: skipped, never written
        m.slots[{0, 42, 0}].textOverride[kParamKey] = "15.0";
        found = discover(m, default_ranges());
        CHECK(!found[0].layoutOk && has(found[0].why, "param 40"), "layout check: %s", found[0].why.c_str());
        const int before2 = m.sets;
        Intent k; k.pitchClass = 0;
        r = apply(m, found, k, Spelling::Sharp);
        CHECK(r.bad.size() == 1 && m.key(0, 42, 0) == 15, "bad layout skipped");
        CHECK(m.sets == before2 + 2, "the others still set (%d sets)", m.sets - before2);
        m.slots[{0, 42, 0}].textOverride.clear();

        // LV1 accepts the set but the value doesn't change: reported as FAILED
        m.refuse.insert({0, 43, 2, kParamKey});
        found = discover(m, default_ranges());
        Intent l; l.pitchClass = 7;
        r = apply(m, found, l, Spelling::Sharp);
        CHECK(r.failed.size() == 1 && has(r.summary(), "FAILED 1 (Ch 44 (slot 3): key reads"), "read-back: %s", r.summary().c_str());

        CHECK(r.brief() == "set 2, 1 locked, 1 FAILED" && r.problem(), "brief: %s", r.brief().c_str());

        // none at all
        FakeMixer empty;
        r = apply(empty, discover(empty, default_ranges()), l, Spelling::Sharp);
        CHECK(has(r.summary(), "no Tune RT found") && r.brief() == "no Tune RT found" && r.problem(), "empty: %s", r.summary().c_str());

        // LV1's own default names are left out
        CHECK(is_default_name("Channel 76", 75) && is_default_name("", 3) && is_default_name("FX Return 2", 1), "default names");
        CHECK(!is_default_name("Channel 76", 74) && !is_default_name("Vox 1", 0) && !is_default_name("76", 75) &&
              !is_default_name("Kick 2", 0) && !is_default_name("Ch-1 1", 0), "real names");
        FakeMixer dn;
        dn.names[{0, 75}] = "Channel 76";
        dn.names[{0, 76}] = "Lead Vox";
        dn.tune(0, 75, 0, 0, 2, true);
        dn.tune(0, 76, 0, 0, 2, true);
        found = discover(dn, default_ranges());
        CHECK(found.size() == 2 && found[0].where == "Ch 76" && found[0].name.empty() && found[1].where == "Ch 77 Lead Vox",
              "default name dropped: %s / %s", found[0].where.c_str(), found[1].where.c_str());
    }

    // ---- settings
    {
        Settings s;
        s.midiPort = "MIDISPORT 2x2 In A"; s.rtpEnabled = true; s.channel = 0; s.scaleCc = 7; s.spelling = Spelling::Flat;
        std::vector<std::string> w;
        Settings t = from_ini(to_ini(s), &w);
        CHECK(w.empty() && t.midiPort == s.midiPort && t.rtpEnabled && t.channel == 0 && t.scaleCc == 7 &&
              t.spelling == Spelling::Flat && t.rtpPort == 5004 && t.rtpName == "Tune Remote", "round trip");
        t = from_ini("channel=17\nscale_cc=120\nrtp_port=80\nfoo=1\nspelling=x\nmidi_port = A B \n", &w);
        CHECK(t.channel == 16 && t.scaleCc == 20 && t.rtpPort == 5004 && t.spelling == Spelling::Sharp && t.midiPort == "A B", "defaults kept");
        CHECK(w.size() == 5, "%zu warnings", w.size());

        // remembered RTP peers
        s.rtpPeers = {{"192.0.2.41", 5004, "Playback Mac"}, {"192.0.2.42", 5008, ""}};
        w.clear();
        t = from_ini(to_ini(s), &w);
        CHECK(w.empty() && t.rtpPeers.size() == 2 && t.rtpPeers[0].ip == "192.0.2.41" && t.rtpPeers[0].port == 5004 &&
              t.rtpPeers[0].name == "Playback Mac" && t.rtpPeers[1].port == 5008 && t.rtpPeers[1].name.empty(), "peers round trip");
        t = from_ini("rtp_peer=192.0.2.41:5004 A\nrtp_peer=192.0.2.41:5004 again\nrtp_peer=host:5004\nrtp_peer=1.2.3:5004\n"
                     "rtp_peer=1.2.3.4:0\nrtp_peer=1.2.3.4\n", &w);
        CHECK(t.rtpPeers.size() == 1 && w.size() == 4, "bad peers skipped, duplicates merged (%zu, %zu warnings)", t.rtpPeers.size(), w.size());
        CHECK(is_ipv4("192.0.2.41") && is_ipv4("0.0.0.0") && !is_ipv4("256.1.1.1") && !is_ipv4("1.2.3") && !is_ipv4("1.2.3.4.5") &&
              !is_ipv4("1..2.3") && !is_ipv4("+1.2.3.4") && !is_ipv4(""), "ipv4");
    }

    // ---- engine end to end
    {
        FakeMixer m;
        m.names[{0, 42}] = "Vox 1";
        m.tune(0, 42, 0, 10, 1, true);   // G Chromatic
        m.tune(0, 43, 0, 5, 2, true);    // Eb Major
        LogRing log;
        int wakes = 0;
        Engine e(m, log);
        e.set_wake([&] { ++wakes; });
        Settings s; e.configure(s);
        CHECK(e.tick(100) == -1, "idle");

        auto send = [&](const std::vector<uint8_t>& msg, uint64_t t) { e.on_midi("RTP Playback-Mac", msg.data(), msg.size(), t); };
        send(cc(16, 20, 3), 1000);
        send(note(16, 57, 100), 1004);
        CHECK(wakes == 2, "woken per message");
        CHECK(e.tick(1010) == 20 && m.key(0, 42, 0) == 10, "not yet");
        CHECK(e.tick(1030) == -1, "applied, idle");
        CHECK(m.key(0, 42, 0) == 13 && m.scale(0, 42, 0) == 3 && m.key(0, 43, 0) == 13, "both set to A Natural Minor");
        auto lines = log.snapshot();
        CHECK(lines.size() == 3 && lines[2].level == Level::Ok && has(lines[2].text, "A Natural Minor: set 2"), "log: %s",
              lines.empty() ? "" : lines.back().text.c_str());
        CHECK(has(lines[0].text, "RTP Playback-Mac CC 20 = 3"), "message logged: %s", lines[0].text.c_str());
        Engine::Status st = e.status();
        CHECK(st.tunes == 2 && st.lastMidiMs == 1004 && has(st.lastResult, "set 2"), "status");
        CHECK(st.changes == 1 && st.last.label == "A Natural Minor" && st.last.source == "RTP Playback-Mac" &&
              st.last.brief == "set 2" && !st.last.problem, "last change: %s / %s / %s", st.last.label.c_str(),
              st.last.source.c_str(), st.last.brief.c_str());

        // main and backup rig send the same cue: one change, both named
        e.on_midi("RTP Main", note(16, 59, 100).data(), 3, 1500);
        e.on_midi("RTP Backup", note(16, 59, 100).data(), 3, 1502);
        e.on_midi("RTP Main", cc(16, 20, 3).data(), 3, 1503);
        e.tick(1540);
        st = e.status();
        CHECK(st.changes == 2 && st.last.source == "RTP Main + RTP Backup" && st.last.label == "B Natural Minor", "two rigs: %s",
              st.last.source.c_str());
        CHECK(m.key(0, 42, 0) == 16, "B");
        send(note(16, 57, 100), 1600);
        e.tick(1640);
        CHECK(e.status().last.source == "RTP Playback-Mac", "a new change has its own source");
        CHECK(e.tunes().size() == 2 && e.tunes()[0].key == 13, "window list shows the new key");

        // a keyboard on another channel: one line, then quiet for 5 s
        const int wakesBefore = wakes;
        send(note(1, 60, 90), 2000);
        e.tick(2000);
        for (int i = 1; i < 50; ++i) send(note(1, 60 + i % 12, 90), 2000 + i);
        CHECK(wakes == wakesBefore + 1, "no wake per ignored message within 5 s (%d)", wakes - wakesBefore);
        e.tick(2100);
        lines = log.snapshot();
        CHECK(lines.size() == 10 && has(lines[9].text, "ignored 1 message"), "first ignored logged at once: %s", lines.back().text.c_str());
        const int64_t wait = e.tick(2200);
        CHECK(wait > 0 && wait <= 5000, "ignored summary scheduled in %lld ms", (long long)wait);
        e.tick(7100);
        lines = log.snapshot();
        CHECK(lines.size() == 11 && has(lines[10].text, "ignored 49 messages"), "summary: %s", lines.back().text.c_str());
        CHECK(m.key(0, 42, 0) == 13, "keyboard changed nothing");

        // Apply now from the window
        Intent t; t.pitchClass = 2; t.scale = 2;
        e.apply_now(t, 9000);
        e.tick(9000);
        CHECK(m.key(0, 42, 0) == 3 && m.scale(0, 42, 0) == 2, "apply now");
        CHECK(e.status().last.source == "Apply now" && e.status().last.label == "D Major", "apply now banner");

        // scale CC out of range: logged as a warning, nothing changes
        send(cc(16, 20, 40), 10000);
        CHECK(e.tick(10100) == -1 && m.scale(0, 42, 0) == 2, "out of range ignored");
        lines = log.snapshot();
        bool warned = false;
        for (const LogLine& l : lines) warned |= l.level == Level::Warn && has(l.text, "CC 20 = 40");
        CHECK(warned, "out-of-range warning logged");

        // scan request
        m.tune(0, 50, 0, 0, 1, true);
        e.request_scan();
        e.tick(11000);
        CHECK(e.tunes().size() == 3 && e.status().tunes == 3, "rescan finds a new Tune");
    }

    printf("%d checks, %d failed\n", g_checks, g_fail);
    return g_fail ? 1 : 0;
}
