//! The runtime end to end on Linux: a fake LV1 behind the Mixer trait, RTP-MIDI from a
//! real session over loopback, settings and log files in a temporary folder.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rtpmidi::session::{PeerState, Session, SessionConfig};
use tunesync::apply::{Mixer, ParamRead};
use tunesync::tune::*;
use tunesync_host::{Runtime, RuntimeConfig};

/// Tune RT on Ch 43 (A Natural Minor, Rcv) and Ch 45 (Eb Major, Lock).
type Params = BTreeMap<(u32, u32, i32), f64>;

#[derive(Clone, Default)]
struct Lv1(Arc<Mutex<Params>>);

impl Lv1 {
    fn new() -> Self {
        let l = Lv1::default();
        {
            let mut m = l.0.lock().unwrap();
            for (ti, key, scale, rcv) in [(42, 13.0, 3.0, 1.0), (44, 5.0, 2.0, 0.0)] {
                m.insert((0, ti, PARAM_KEY), key);
                m.insert((0, ti, PARAM_SCALE), scale);
                m.insert((0, ti, PARAM_RECEIVE), rcv);
            }
        }
        l
    }
    fn key(&self, ti: u32) -> f64 {
        self.0.lock().unwrap()[&(0, ti, PARAM_KEY)]
    }
    fn scale(&self, ti: u32) -> f64 {
        self.0.lock().unwrap()[&(0, ti, PARAM_SCALE)]
    }
}

impl Mixer for Lv1 {
    fn get(&mut self, tt: u32, ti: u32, slot: u32, ch: i32, cid: i32) -> ParamRead {
        let m = self.0.lock().unwrap();
        let mut r = ParamRead { ok: true, ..Default::default() };
        let here = tt == 0 && slot == 0 && m.contains_key(&(0, ti, PARAM_KEY));
        if !here {
            return r;
        }
        if ch == SLOT_INFO {
            r.text = if cid == CID_SLOT_CODE { "LVLM".into() } else { "Tune RT".into() };
            return r;
        }
        if let Some(v) = m.get(&(0, ti, ch)) {
            r.value = *v;
            let i = *v as u8;
            r.text = match ch {
                PARAM_KEY => key_name(i).unwrap_or("").into(),
                PARAM_SCALE => scale_name(i).unwrap_or("").into(),
                _ => (if i == 1 { "Rcv" } else { "Lock" }).into(),
            };
        }
        r
    }
    fn set(&mut self, tt: u32, ti: u32, _slot: u32, param: i32, v: f64) -> bool {
        self.0.lock().unwrap().insert((tt, ti, param), v);
        true
    }
    fn track_name(&mut self, _tt: u32, ti: u32) -> String {
        if ti == 42 { "Vox 1".into() } else { String::new() }
    }
}

fn wait_for(ms: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn rtp_note_sets_every_unlocked_tune() {
    let dir = std::env::temp_dir().join(format!("tunesync-rt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let lv1 = Lv1::new();
    let rt = Runtime::start(RuntimeConfig::new(&dir, "TuneSync test"), Box::new(lv1.clone()));
    assert!(wait_for(2000, || rt.tunes().len() == 2), "first scan finds both Tunes");
    let t = rt.tunes();
    assert_eq!((t[0].label.as_str(), t[0].name.as_str(), t[0].receive), ("Ch 43", "Vox 1", true));
    assert!(!t[1].receive, "Ch 45 is on Lock");

    // RTP-MIDI on a test port, as the window's switch would.
    let port = 47504u16;
    rt.update(|s| {
        s.rtp_enabled = true;
        s.rtp_port = port;
    });
    assert!(wait_for(2000, || rt.rtp().open), "session open: {}", rt.rtp().error);
    assert!(std::fs::read_to_string(dir.join("settings.ini")).unwrap().contains(&format!("rtp_port={port}")), "settings saved");

    // A playback rig joins and sends the cue: CC 20 = 4 (Harmonic Minor) + Note On D.
    let rig = Session::new();
    rig.open(SessionConfig { name: "Playback-Mac".into(), port: 47604, ..Default::default() }).unwrap();
    rig.invite("127.0.0.1", port, "");
    assert!(wait_for(3000, || {
        rig.poll(5);
        rt.rtp().peers.iter().any(|p| p.state == PeerState::Connected && p.name == "Playback-Mac")
    }), "the rig is a connected peer");
    assert!(wait_for(1000, || { rig.poll(5); rig.peers().iter().any(|p| p.state == PeerState::Connected) }));
    assert_eq!(rig.send(&[&[0xBF, 20, 4], &[0x9F, 62, 100]]), 1);
    assert!(wait_for(2000, || { rig.poll(5); lv1.key(42) == 3.0 }), "Ch 43 -> D (key {})", lv1.key(42));
    assert_eq!(lv1.scale(42), 4.0, "Harmonic Minor");
    assert_eq!((lv1.key(44), lv1.scale(44)), (5.0, 2.0), "the locked Tune is untouched");
    let st = rt.status();
    assert!(wait_for(1000, || rt.status().changes >= 1));
    let st2 = rt.status();
    assert_eq!((st2.last.label.as_str(), st2.last.source.as_str()), ("D Harmonic Minor", "RTP Playback-Mac"), "{st:?}");
    assert!(st2.last.brief.contains("set 1") && st2.last.brief.contains("1 locked"), "{}", st2.last.brief);

    // Apply now from the window.
    rt.apply_now(tunesync::midi_map::Intent { pitch_class: Some(9), scale: None });
    assert!(wait_for(2000, || lv1.key(42) == 13.0), "Apply now -> A");

    rig.close();
    rt.stop();
    let log = std::fs::read_to_string(dir.join("tunesync.log")).unwrap();
    assert!(log.contains("D Harmonic Minor: set 1") && log.contains("skipped 1 locked"), "{log}");
    assert!(log.contains("TuneSync stopping"));
    let _ = std::fs::remove_dir_all(&dir);
}
