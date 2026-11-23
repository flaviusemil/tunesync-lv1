//! Discovery, apply (Lock, layout checks, read-back) and the engine end to end, against
//! a fake LV1. The cases are the C++ core_test's (branch feature/windows-shell), ported.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tunesync::apply::*;
use tunesync::engine::Engine;
use tunesync::log::{Level, LogRing};
use tunesync::midi_map::Intent;
use tunesync::settings::Settings;
use tunesync::tune::*;

#[derive(Default, Clone)]
struct FakeSlot {
    name: String,
    code: String,
    params: BTreeMap<i32, f64>,
    text_override: BTreeMap<i32, String>, // a "Tune update" that reshuffled params
}

#[derive(Default)]
struct FakeMixer {
    slots: BTreeMap<(u32, u32, u32), FakeSlot>,
    names: BTreeMap<(u32, u32), String>,
    refuse: BTreeSet<(u32, u32, u32, i32)>, // sets LV1 accepts but silently ignores
    gets: usize,
    sets: usize,
    /// LV1's system tempo; None = can't be read (an LV1 without it).
    tempo: Option<f64>,
    /// set_tempo is taken but nothing changes
    tempo_stuck: bool,
}

impl FakeMixer {
    fn tune(&mut self, tt: u32, ti: u32, slot: u32, key: u8, scale: u8, receive: bool) -> &mut FakeSlot {
        self.tune_code(tt, ti, slot, key, scale, receive, "LVLM")
    }
    #[allow(clippy::too_many_arguments)]
    fn tune_code(&mut self, tt: u32, ti: u32, slot: u32, key: u8, scale: u8, receive: bool, code: &str) -> &mut FakeSlot {
        let s = self.slots.entry((tt, ti, slot)).or_default();
        s.name = "Tune RT".into();
        s.code = code.into();
        s.params = [(PARAM_KEY, key as f64), (PARAM_SCALE, scale as f64), (PARAM_RECEIVE, if receive { 1.0 } else { 0.0 }), (41, 1.0)].into();
        s.text_override.clear();
        s
    }
    fn text_of(param: i32, v: f64) -> String {
        let i = v as u8;
        match param {
            PARAM_KEY => key_name(i).unwrap_or(""),
            PARAM_SCALE => scale_name(i).unwrap_or(""),
            PARAM_RECEIVE => {
                if i != 0 {
                    "Rcv"
                } else {
                    "Lock"
                }
            }
            _ => "",
        }
        .into()
    }
    fn key(&self, tt: u32, ti: u32, slot: u32) -> u8 {
        self.slots[&(tt, ti, slot)].params[&PARAM_KEY] as u8
    }
    fn scale(&self, tt: u32, ti: u32, slot: u32) -> u8 {
        self.slots[&(tt, ti, slot)].params[&PARAM_SCALE] as u8
    }
}

impl Mixer for FakeMixer {
    fn get(&mut self, tt: u32, ti: u32, slot: u32, ch: i32, cid: i32) -> ParamRead {
        self.gets += 1;
        let mut r = ParamRead { ok: true, ..Default::default() }; // LV1 answers empty slots with empty text
        let Some(s) = self.slots.get(&(tt, ti, slot)) else { return r };
        if ch == SLOT_INFO {
            r.text = match cid {
                CID_SLOT_NAME => s.name.clone(),
                CID_SLOT_CODE => s.code.clone(),
                _ => String::new(),
            };
            return r;
        }
        if cid != CID_VALUE {
            return r;
        }
        match s.params.get(&ch) {
            None => r.value = -1.0,
            Some(v) => {
                r.value = *v;
                r.text = s.text_override.get(&ch).cloned().unwrap_or_else(|| Self::text_of(ch, *v));
            }
        }
        r
    }
    fn set(&mut self, tt: u32, ti: u32, slot: u32, param: i32, v: f64) -> bool {
        self.sets += 1;
        if self.refuse.contains(&(tt, ti, slot, param)) {
            return true; // accepted, silently not applied
        }
        match self.slots.get_mut(&(tt, ti, slot)) {
            Some(s) => {
                s.params.insert(param, v);
                true
            }
            None => false,
        }
    }
    fn track_name(&mut self, tt: u32, ti: u32) -> String {
        self.names.get(&(tt, ti)).cloned().unwrap_or_default()
    }
    fn tempo(&mut self) -> Option<f64> {
        self.tempo
    }
    fn set_tempo(&mut self, bpm: f64) -> bool {
        if self.tempo.is_none() {
            return false;
        }
        if !self.tempo_stuck {
            self.tempo = Some(bpm);
        }
        true
    }
}

fn note(ch: u8, n: u8, vel: u8) -> [u8; 3] {
    [0x90 | (ch - 1), n, vel]
}
fn cc(ch: u8, c: u8, v: u8) -> [u8; 3] {
    [0xB0 | (ch - 1), c, v]
}
fn intent(pc: Option<u8>, scale: Option<u8>) -> Intent {
    Intent { pitch_class: pc, scale }
}

#[test]
fn discovery_and_apply() {
    let mut m = FakeMixer::default();
    m.names.insert((0, 42), "Vox 1".into());
    m.tune(0, 42, 0, 13, 3, true); // Ch 43: A Natural Minor
    m.tune(0, 43, 2, 5, 2, true); // Ch 44 slot 3: Eb Major (flat speller)
    m.tune(0, 44, 0, 8, 2, false); // Ch 45: F# Major, Lock
    m.tune_code(1, 0, 1, 1, 2, true, "LVLS"); // Gp 1 slot 2: C# Major, stereo
    m.slots.insert((0, 10, 0), FakeSlot { name: "PSE".into(), code: "VXPM".into(), ..Default::default() });
    let found = discover(&mut m, &default_ranges());
    assert_eq!(found.len(), 4);
    assert_eq!(found[0].where_, "Ch 43 Vox 1");
    assert_eq!((found[0].key, found[0].scale, found[0].receive), (Some(13), Some(3), true));
    assert_eq!(found[1].where_, "Ch 44 (slot 3)");
    assert!(!found[2].receive && found[2].layout_ok, "lock read");
    assert_eq!(found[3].where_, "Gp 1 (slot 2)");
    assert_eq!((found[0].label.as_str(), found[0].name.as_str()), ("Ch 43", "Vox 1"));
    assert_eq!((found[1].label.as_str(), found[1].name.as_str()), ("Ch 44", ""));

    // the C#/Db note + Natural Minor
    let r = apply(&mut m, &found, &intent(Some(1), Some(3)), Spelling::Sharp);
    assert_eq!(m.key(0, 42, 0), 1, "A (natural) -> default sharps: C#");
    assert_eq!(m.key(0, 43, 2), 2, "Eb (flat) -> Db");
    assert_eq!(m.key(1, 0, 1), 1, "C# stays sharp");
    assert_eq!((m.key(0, 44, 0), m.scale(0, 44, 0)), (8, 2), "locked untouched");
    assert_eq!((m.scale(0, 42, 0), m.scale(0, 43, 2), m.scale(1, 0, 1)), (3, 3, 3), "scales set");
    assert_eq!((r.set.len(), r.locked.len(), r.failed.len(), r.bad.len()), (3, 1, 0, 0));
    let s = r.summary();
    assert!(s.contains("C#/Db Natural Minor: set 3") && s.contains("Ch 44 (slot 3) -> Db") && s.contains("skipped 1 locked (Ch 45)"), "{s}");

    // natural key + flats default, key only: the scale stays
    m.tune(0, 42, 0, 13, 3, true);
    let found = discover(&mut m, &default_ranges());
    let bb = intent(Some(10), None);
    apply(&mut m, &found, &bb, Spelling::Flat);
    assert_eq!((m.key(0, 42, 0), m.scale(0, 42, 0)), (15, 3), "A -> Bb with flats, scale kept");

    // already there: no write
    let found = discover(&mut m, &default_ranges());
    let before = m.sets;
    let r = apply(&mut m, &found, &bb, Spelling::Flat);
    assert!(m.sets == before && r.set.is_empty() && !r.unchanged.is_empty() && r.summary().contains("already there"), "{}", r.summary());

    // a Tune whose parameters moved: skipped, never written
    m.slots.get_mut(&(0, 42, 0)).unwrap().text_override.insert(PARAM_KEY, "15.0".into());
    let found = discover(&mut m, &default_ranges());
    assert!(!found[0].layout_ok && found[0].why.contains("param 40"), "{}", found[0].why);
    let before = m.sets;
    let r = apply(&mut m, &found, &intent(Some(0), None), Spelling::Sharp);
    assert_eq!(r.bad.len(), 1);
    assert_eq!(m.key(0, 42, 0), 15, "bad layout skipped");
    assert_eq!(m.sets - before, 2, "the others still set");
    m.slots.get_mut(&(0, 42, 0)).unwrap().text_override.clear();

    // LV1 accepts the set but the value doesn't change: reported as FAILED
    m.refuse.insert((0, 43, 2, PARAM_KEY));
    let found = discover(&mut m, &default_ranges());
    let g = intent(Some(7), None);
    let r = apply(&mut m, &found, &g, Spelling::Sharp);
    assert!(r.failed.len() == 1 && r.summary().contains("FAILED 1 (Ch 44 (slot 3): key reads"), "{}", r.summary());
    assert_eq!(r.brief(), "set 2, 1 locked, 1 FAILED");
    assert!(r.problem());

    // none at all
    let mut empty = FakeMixer::default();
    let found = discover(&mut empty, &default_ranges());
    let r = apply(&mut empty, &found, &g, Spelling::Sharp);
    assert!(r.summary().contains("no Tune RT found") && r.brief() == "no Tune RT found" && r.problem());
    assert!(empty.gets >= 160 * 8, "every slot of every track was asked");
}

#[test]
fn default_names() {
    assert!(is_default_name("Channel 76", 75) && is_default_name("", 3) && is_default_name("FX Return 2", 1));
    for (n, i) in [("Channel 76", 74), ("Vox 1", 0), ("76", 75), ("Kick 2", 0), ("Ch-1 1", 0)] {
        assert!(!is_default_name(n, i), "{n}");
    }
    let mut m = FakeMixer::default();
    m.names.insert((0, 75), "Channel 76".into());
    m.names.insert((0, 76), "Lead Vox".into());
    m.tune(0, 75, 0, 0, 2, true);
    m.tune(0, 76, 0, 0, 2, true);
    let found = discover(&mut m, &default_ranges());
    assert_eq!(found.len(), 2);
    assert_eq!((found[0].where_.as_str(), found[0].name.as_str()), ("Ch 76", ""));
    assert_eq!(found[1].where_, "Ch 77 Lead Vox");
}

#[test]
fn engine_end_to_end() {
    let mut m = FakeMixer::default();
    m.names.insert((0, 42), "Vox 1".into());
    m.tune(0, 42, 0, 10, 1, true); // G Chromatic
    m.tune(0, 43, 0, 5, 2, true); // Eb Major
    let log = Arc::new(LogRing::default());
    let wakes = Arc::new(AtomicUsize::new(0));
    let mut e = Engine::new(log.clone());
    let w2 = wakes.clone();
    e.set_wake(move || {
        w2.fetch_add(1, Ordering::SeqCst);
    });
    e.configure(&Settings::default());
    assert_eq!(e.tick(&mut m, 100), None, "idle");

    let send = |e: &Engine, msg: [u8; 3], t: u64| e.on_midi("RTP Playback-Mac", &msg, t);
    send(&e, cc(16, 20, 3), 1000);
    send(&e, note(16, 57, 100), 1004);
    assert_eq!(wakes.load(Ordering::SeqCst), 2, "woken per message");
    assert_eq!(e.tick(&mut m, 1010), Some(20));
    assert_eq!(m.key(0, 42, 0), 10, "not yet");
    assert_eq!(e.tick(&mut m, 1030), None, "applied, idle");
    assert_eq!((m.key(0, 42, 0), m.scale(0, 42, 0), m.key(0, 43, 0)), (13, 3, 13), "both A Natural Minor");
    let lines = log.snapshot();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2].level, Level::Ok);
    assert!(lines[2].text.contains("A Natural Minor: set 2"), "{}", lines[2].text);
    assert!(lines[0].text.contains("RTP Playback-Mac CC 20 = 3"), "{}", lines[0].text);
    let st = e.status();
    assert!(st.tunes == 2 && st.last_midi_ms == 1004 && st.last_result.contains("set 2"));
    assert_eq!(st.changes, 1);
    assert_eq!((st.last.label.as_str(), st.last.source.as_str(), st.last.brief.as_str(), st.last.problem), ("A Natural Minor", "RTP Playback-Mac", "set 2", false));
    assert_eq!(e.tunes()[0].key, Some(13), "the window list shows the new key");

    // main and backup rig send the same cue: one change, both named
    e.on_midi("RTP Main", &note(16, 59, 100), 1500);
    e.on_midi("RTP Backup", &note(16, 59, 100), 1502);
    e.on_midi("RTP Main", &cc(16, 20, 3), 1503);
    e.tick(&mut m, 1540);
    let st = e.status();
    assert_eq!((st.changes, st.last.source.as_str(), st.last.label.as_str()), (2, "RTP Main + RTP Backup", "B Natural Minor"));
    assert_eq!(m.key(0, 42, 0), 16);
    send(&e, note(16, 57, 100), 1600);
    e.tick(&mut m, 1640);
    assert_eq!(e.status().last.source, "RTP Playback-Mac", "a new change has its own source");

    // a keyboard on another channel: one line, then quiet for 5 s
    let w0 = wakes.load(Ordering::SeqCst);
    send(&e, note(1, 60, 90), 2000);
    e.tick(&mut m, 2000);
    for i in 1..50u8 {
        send(&e, note(1, 60 + i % 12, 90), 2000 + i as u64);
    }
    assert_eq!(wakes.load(Ordering::SeqCst), w0 + 1, "no wake per ignored message within 5 s");
    e.tick(&mut m, 2100);
    let lines = log.snapshot();
    assert!(lines.last().unwrap().text.contains("ignored 1 message"), "{}", lines.last().unwrap().text);
    let n0 = lines.len();
    let wait = e.tick(&mut m, 2200).expect("ignored summary scheduled");
    assert!(wait > 0 && wait <= 5000, "{wait}");
    e.tick(&mut m, 7100);
    let lines = log.snapshot();
    assert_eq!(lines.len(), n0 + 1);
    assert!(lines.last().unwrap().text.contains("ignored 49 messages"), "{}", lines.last().unwrap().text);
    assert_eq!(m.key(0, 42, 0), 13, "keyboard changed nothing");

    // Apply now from the window
    e.apply_now(intent(Some(2), Some(2)), 9000);
    e.tick(&mut m, 9000);
    assert_eq!((m.key(0, 42, 0), m.scale(0, 42, 0)), (3, 2), "apply now");
    assert_eq!((e.status().last.source.as_str(), e.status().last.label.as_str()), ("Apply now", "D Major"));

    // scale CC out of range: logged as a warning, nothing changes
    send(&e, cc(16, 20, 40), 10000);
    assert_eq!(e.tick(&mut m, 10100), None);
    assert_eq!(m.scale(0, 42, 0), 2);
    assert!(log.snapshot().iter().any(|l| l.level == Level::Warn && l.text.contains("CC 20 = 40")));

    // scan request
    m.tune(0, 50, 0, 0, 1, true);
    e.request_scan();
    e.tick(&mut m, 11000);
    assert_eq!((e.tunes().len(), e.status().tunes), (3, 3), "rescan finds a new Tune");
}

/// CC 21 sets LV1's tempo (provisional mapping): no Tune pass, the result read back.
#[test]
fn tempo_by_cc() {
    let mut m = FakeMixer { tempo: Some(120.0), ..Default::default() };
    m.tune(0, 42, 0, 10, 1, true);
    let log = Arc::new(LogRing::default());
    let e = Engine::new(log.clone());
    e.configure(&Settings::default());
    assert_eq!(e.tick(&mut m, 100), None);
    assert!(log.snapshot().is_empty(), "LV1's tempo at load is its default, not the session's: not logged");

    e.on_midi("RTP iPad", &cc(16, 21, 68), 1000);
    assert_eq!(e.tick(&mut m, 1000), None, "set at once: nothing pending");
    assert_eq!(m.tempo, Some(128.0));
    assert_eq!(m.sets, 0, "no Tune touched");
    let l = log.snapshot();
    assert_eq!(l[0].text, "RTP iPad CC 21 = 68 (128 BPM) ch 16");
    assert_eq!((l[1].level, l[1].text.as_str()), (Level::Ok, "Tempo 128 BPM (was 120) (RTP iPad)"));
    assert_eq!(e.status().changes, 0, "the Tunes banner is not a tempo's");

    // a key cue with a tempo: both, one Tune pass
    e.on_midi("RTP iPad", &cc(16, 21, 32), 2000);
    e.on_midi("RTP iPad", &note(16, 57, 100), 2001);
    e.tick(&mut m, 2001);
    e.tick(&mut m, 2031);
    assert_eq!((m.tempo, m.key(0, 42, 0)), (Some(92.0), 13));

    e.on_midi("RTP iPad", &cc(16, 21, 32), 3000);
    e.tick(&mut m, 3000);
    assert!(log.snapshot().last().unwrap().text.contains("Tempo 92 BPM: already there"));

    m.tempo_stuck = true;
    e.on_midi("RTP iPad", &cc(16, 21, 0), 4000);
    e.tick(&mut m, 4000);
    let last = log.snapshot().last().unwrap().clone();
    assert_eq!((last.level, last.text.as_str()), (Level::Warn, "Tempo 60 BPM did not land: LV1 took the call, reads 92 BPM"));
}

#[test]
fn a_refused_tempo_says_so() {
    let mut m = FakeMixer::default();
    let log = Arc::new(LogRing::default());
    let e = Engine::new(log.clone());
    e.on_midi("MIDI A", &cc(16, 21, 60), 200);
    e.tick(&mut m, 200);
    assert!(log.snapshot().last().unwrap().text.contains("refused the call, reads nothing"));
}

/// MIDI clock from the playback rig sets LV1's tempo once it settles (timed on the sender's
/// stamps), quietly: no MIDI line per tick, no Tune pass; a CC holds until the clock moves.
#[test]
fn tempo_follows_midi_clock() {
    let mut m = FakeMixer { tempo: Some(120.0), ..Default::default() };
    m.tune(0, 42, 0, 10, 1, true);
    let log = Arc::new(LogRing::default());
    let e = Engine::new(log.clone());
    e.configure(&Settings::default());
    let clock = |e: &Engine, m: &mut FakeMixer, bpm: f64, beats: usize, t0: u64| -> u64 {
        let tick = 60e6 / (bpm * 24.0);
        let mut t = t0;
        for i in 0..beats * 24 {
            t = t0 + (i as f64 * tick) as u64;
            let late = (i as u64 * 7919) % 25_000; // Wi-Fi
            e.on_midi_at("RTP Mac", &[0xF8], t + late, Some(t));
            e.tick(m, (t + late) / 1000);
        }
        t
    };
    e.on_midi_at("RTP Mac", &[0xFA], 1_000_000, Some(1_000_000));
    let t = clock(&e, &mut m, 128.0, 8, 1_000_000);
    assert_eq!(m.tempo, Some(128.0));
    let l = log.snapshot();
    assert_eq!(l.len(), 1, "one line, not one per tick: {l:?}");
    assert_eq!((l[0].level, l[0].text.as_str()), (Level::Ok, "Tempo 128 BPM (was 120) (RTP Mac clock)"));
    assert_eq!((m.sets, e.status().last_midi_ms), (0, 0), "no Tune touched; clock is not a cue");

    // a CC tempo holds while the clock keeps its tempo...
    e.on_midi("RTP Mac", &cc(16, 21, 64), t / 1000);
    e.tick(&mut m, t / 1000);
    assert_eq!(m.tempo, Some(124.0));
    let t = clock(&e, &mut m, 128.0, 8, t + 19_531);
    assert_eq!(m.tempo, Some(124.0), "the clock did not change: the CC holds");
    // ...until the clock's tempo changes
    clock(&e, &mut m, 140.0, 8, t + 17_857);
    assert_eq!(m.tempo, Some(140.0));
    assert!(log.snapshot().last().unwrap().text.ends_with("(RTP Mac clock)"));
}
