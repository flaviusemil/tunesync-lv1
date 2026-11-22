//! Find every Tune RT in LV1 and set its key and scale, through an abstract [`Mixer`] so
//! the logic runs against a fake LV1 in the tests.
//!
//! Per instance: check the parameters look the way [`crate::tune`] says (else skip it: a
//! Tune update could reorder them), skip it when its Receive button is on Lock, set the
//! key in the spelling it already uses, set the scale, read both back.
use crate::midi_map::Intent;
use crate::tune::*;

/// One read of a plug-in control.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamRead {
    /// LV1 answered.
    pub ok: bool,
    pub value: f64,
    pub text: String,
}

/// The slice of LV1's surface API this needs. The LV1 shell implements it on
/// GetMixerControl / SetMixerControl; every call happens on one worker thread.
pub trait Mixer {
    /// A plug-in control: section 4, sectionIndex = slot, channel = parameter (or
    /// [`SLOT_INFO`]).
    fn get(&mut self, track_type: u32, track_index: u32, slot: u32, channel: i32, cid: i32) -> ParamRead;
    /// Touch, Control(value), Release on a plug-in parameter's value (cid 11). Returns
    /// whether LV1 accepted the calls; the read-back says whether the value landed.
    fn set(&mut self, track_type: u32, track_index: u32, slot: u32, param: i32, value: f64) -> bool;
    /// The track's user name, "" when it has none.
    fn track_name(&mut self, track_type: u32, track_index: u32) -> String;
}

/// Which tracks to look at. Track types: 0 Ch, 1 Gp, 2 Fx (LV1's WSTrackID).
#[derive(Clone, Copy, Debug)]
pub struct TrackRange {
    pub track_type: u32,
    pub count: u32,
    pub label: &'static str,
}

pub fn default_ranges() -> Vec<TrackRange> {
    vec![
        TrackRange { track_type: 0, count: 128, label: "Ch" },
        TrackRange { track_type: 1, count: 16, label: "Gp" },
        TrackRange { track_type: 2, count: 16, label: "Fx" },
    ]
}

pub const INSERT_SLOTS: u32 = 8;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TuneInstance {
    pub track_type: u32,
    pub track_index: u32,
    pub slot: u32,
    /// "Ch 43 Vox 1", for the log ("Ch 44 (slot 3)" past slot 1).
    pub where_: String,
    /// "Ch 43", for the window.
    pub label: String,
    /// "Vox 1": the track's own name, "" for LV1's default one.
    pub name: String,
    pub key: Option<u8>,
    pub scale: Option<u8>,
    /// false = Lock
    pub receive: bool,
    pub layout_ok: bool,
    /// What did not look right, when !layout_ok.
    pub why: String,
}

/// LV1 names an untouched strip after itself ("Channel 76"): that says nothing the label
/// doesn't, so it is left out. LV1's own words only: "Vox 1" on channel 1 is a real name.
pub fn is_default_name(name: &str, track_index: u32) -> bool {
    if name.is_empty() {
        return true;
    }
    const WORDS: [&str; 9] = ["Channel", "Ch", "Input", "Group", "Gp", "FX", "Fx", "FX Return", "Return"];
    let n = track_index + 1;
    WORDS.iter().any(|w| name == format!("{w} {n}"))
}

// A parameter value LV1 reports as a double; Tune's are whole numbers.
fn whole(v: f64) -> Option<i64> {
    let r = (v + 0.5).floor();
    if (v - r).abs() > 1e-6 || !(-1e6..=1e6).contains(&r) {
        return None;
    }
    Some(r as i64)
}

fn receive_name(v: u8) -> Option<&'static str> {
    match v {
        1 => Some("Rcv"),
        0 => Some("Lock"),
        _ => None,
    }
}

// Reads a parameter that must be a whole number whose text is names(value).
fn read_named(m: &mut dyn Mixer, t: &TuneInstance, param: i32, names: fn(u8) -> Option<&'static str>) -> Result<u8, String> {
    let r = m.get(t.track_type, t.track_index, t.slot, param, CID_VALUE);
    if r.ok {
        if let Some(v) = whole(r.value).filter(|v| (0..=255).contains(v)).map(|v| v as u8) {
            if names(v) == Some(r.text.as_str()) {
                return Ok(v);
            }
        }
    }
    Err(format!("param {param} reads {}{:.3} \"{}\"", if r.ok { "" } else { "(no answer) " }, r.value, r.text))
}

fn name_it(m: &mut dyn Mixer, r: &TrackRange, t: &mut TuneInstance) {
    t.label = format!("{} {}", r.label, t.track_index + 1);
    t.name = m.track_name(r.track_type, t.track_index);
    if is_default_name(&t.name, t.track_index) {
        t.name.clear();
    }
    t.where_ = t.label.clone();
    if !t.name.is_empty() {
        t.where_ = format!("{} {}", t.where_, t.name);
    }
    if t.slot > 0 {
        t.where_ = format!("{} (slot {})", t.where_, t.slot + 1);
    }
}

/// Every Tune RT on the given tracks, read fresh (instances added mid-show count).
pub fn discover(m: &mut dyn Mixer, ranges: &[TrackRange]) -> Vec<TuneInstance> {
    let mut out = Vec::new();
    for r in ranges {
        for i in 0..r.count {
            for s in 0..INSERT_SLOTS {
                let code = m.get(r.track_type, i, s, SLOT_INFO, CID_SLOT_CODE);
                if !code.ok || !is_tune_code(&code.text) {
                    continue;
                }
                let mut t = TuneInstance { track_type: r.track_type, track_index: i, slot: s, ..Default::default() };
                name_it(m, r, &mut t);
                let checked = read_named(m, &t, PARAM_KEY, key_name).and_then(|k| {
                    t.key = Some(k);
                    read_named(m, &t, PARAM_SCALE, scale_name).and_then(|sc| {
                        t.scale = Some(sc);
                        read_named(m, &t, PARAM_RECEIVE, receive_name)
                    })
                });
                match checked {
                    Ok(rcv) => {
                        t.layout_ok = true;
                        t.receive = rcv != 0;
                    }
                    Err(why) => {
                        t.layout_ok = false;
                        t.receive = true;
                        t.why = why;
                    }
                }
                out.push(t);
            }
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct ApplyResult {
    /// What was asked for, e.g. "C#/Db Natural Minor".
    pub label: String,
    pub set: Vec<String>,
    pub unchanged: Vec<String>,
    pub locked: Vec<String>,
    pub bad: Vec<String>,
    pub failed: Vec<String>,
}

impl ApplyResult {
    pub fn total(&self) -> usize {
        self.set.len() + self.unchanged.len() + self.locked.len() + self.bad.len() + self.failed.len()
    }

    pub fn problem(&self) -> bool {
        !self.failed.is_empty() || !self.bad.is_empty() || self.total() == 0
    }

    /// One log line.
    pub fn summary(&self) -> String {
        let mut s = format!("{}: ", self.label);
        if self.total() == 0 {
            return s + "no Tune RT found";
        }
        s += &format!("set {}", self.set.len());
        if !self.set.is_empty() {
            s += &format!(" ({})", self.set.join(", "));
        }
        if !self.unchanged.is_empty() {
            s += &format!(", already there {} ({})", self.unchanged.len(), self.unchanged.join(", "));
        }
        if !self.locked.is_empty() {
            s += &format!(", skipped {} locked ({})", self.locked.len(), self.locked.join(", "));
        }
        if !self.bad.is_empty() {
            s += &format!(", skipped {} unexpected Tune layout ({})", self.bad.len(), self.bad.join(", "));
        }
        if !self.failed.is_empty() {
            s += &format!(", FAILED {} ({})", self.failed.len(), self.failed.join(", "));
        }
        s
    }

    /// "set 2, 1 locked": the window's last-change banner.
    pub fn brief(&self) -> String {
        if self.total() == 0 {
            return "no Tune RT found".into();
        }
        let mut s = format!("set {}", self.set.len());
        for (n, what) in [(self.unchanged.len(), "already there"), (self.locked.len(), "locked"), (self.bad.len(), "unexpected"), (self.failed.len(), "FAILED")] {
            if n > 0 {
                s += &format!(", {n} {what}");
            }
        }
        s
    }
}

/// Sets every unlocked, well-formed instance to `intent`, keeping each one's spelling
/// (`fallback` for a Tune on a natural key), and reads each change back.
pub fn apply(m: &mut dyn Mixer, tunes: &[TuneInstance], intent: &Intent, fallback: Spelling) -> ApplyResult {
    let mut res = ApplyResult { label: intent.label(), ..Default::default() };
    for t in tunes {
        if !t.layout_ok {
            res.bad.push(format!("{}: {}", t.where_, t.why));
            continue;
        }
        if !t.receive {
            res.locked.push(t.where_.clone());
            continue;
        }
        let key = intent.pitch_class.and_then(|pc| key_value(pc, keep_spelling(t.key, fallback)));
        let do_key = key.is_some() && key != t.key;
        let do_scale = intent.scale.is_some() && intent.scale != t.scale;
        if !do_key && !do_scale {
            res.unchanged.push(t.where_.clone());
            continue;
        }
        let mut err = Vec::new();
        if do_key && !m.set(t.track_type, t.track_index, t.slot, PARAM_KEY, key.unwrap() as f64) {
            err.push("key not accepted".to_string());
        }
        if do_scale && !m.set(t.track_type, t.track_index, t.slot, PARAM_SCALE, intent.scale.unwrap() as f64) {
            err.push("scale not accepted".to_string());
        }
        // Read back what LV1 now shows: the only proof the change landed.
        if err.is_empty() && do_key {
            match read_named(m, t, PARAM_KEY, key_name) {
                Ok(k) if Some(k) == key => {}
                Ok(k) => err.push(format!("key reads {}", key_name(k).unwrap_or("?"))),
                Err(why) => err.push(why),
            }
        }
        if err.is_empty() && do_scale {
            match read_named(m, t, PARAM_SCALE, scale_name) {
                Ok(s) if Some(s) == intent.scale => {}
                Ok(s) => err.push(format!("scale reads {}", scale_name(s).unwrap_or("?"))),
                Err(why) => err.push(why),
            }
        }
        if !err.is_empty() {
            res.failed.push(format!("{}: {}", t.where_, err.join(", ")));
            continue;
        }
        res.set.push(if do_key { format!("{} -> {}", t.where_, key_name(key.unwrap()).unwrap_or("?")) } else { t.where_.clone() });
    }
    res
}
