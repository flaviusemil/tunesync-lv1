//! What a MIDI message from the tracks rig asks for, and how messages that arrive
//! together become one change.
//!
//! ```text
//!   Note On (velocity > 0) on the channel   key = the note's pitch class
//!   CC <scale_cc> on the channel, 0..10     scale = that Tune scale number
//!   CC 21 on the channel, 40..127           LV1's tempo = value BPM
//!   CC 22 on the channel, 0..127            LV1's tempo = 100 + value BPM
//!   MIDI clock (any channel, any source)    LV1's tempo follows it (crate::clock)
//!   anything else                           ignored (Note Off, other channels, ...)
//! ```
use crate::tune::{pitch_class_name, scale_name, NUM_SCALES};

/// The CCs that set LV1's tempo exactly, on the configured channel: one message per song,
/// like the key. The value is the BPM as typed in the playback app, no sums: CC 21 = the BPM
/// (40..127; LV1's tempo starts at 40), CC 22 = the BPM - 100 for faster songs (100..227:
/// 128 BPM is CC 22 = 28). MIDI clock is followed as well ([`crate::clock`]); a CC tempo
/// holds until the clock's tempo changes. A `scale_cc` set to the same number wins.
pub const TEMPO_CC: u8 = 21;
pub const TEMPO_HIGH_CC: u8 = 22;

/// The BPM a tempo CC asks for, `None` when it is not one or the value is below LV1's 40.
pub fn tempo_bpm(cc: u8, value: u8) -> Option<u8> {
    match cc {
        TEMPO_CC => (value >= crate::clock::MIN_BPM as u8).then_some(value),
        TEMPO_HIGH_CC => Some(100 + value),
        _ => None,
    }
}

/// "128", "128.5": a tempo for the log.
pub fn bpm_text(bpm: f64) -> String {
    if (bpm - bpm.round()).abs() < 0.05 {
        format!("{:.0}", bpm)
    } else {
        format!("{:.1}", bpm)
    }
}

/// What one change asks for; `None` = leave as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Intent {
    /// 0 = C .. 11 = B
    pub pitch_class: Option<u8>,
    /// 0..10, Tune's own scale numbers
    pub scale: Option<u8>,
}

impl Intent {
    pub fn is_empty(&self) -> bool {
        self.pitch_class.is_none() && self.scale.is_none()
    }
    /// "A Natural Minor", "C#/Db" (key only), "Major" (scale only).
    pub fn label(&self) -> String {
        let mut s = self.pitch_class.map(pitch_class_name).unwrap_or_default();
        if let Some(n) = self.scale.and_then(scale_name) {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(n);
        }
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgKind {
    Ignored,
    Key,
    Scale,
    /// [`TEMPO_CC`] / [`TEMPO_HIGH_CC`]: `value` is the BPM ([`tempo_bpm`]).
    Tempo,
    OutOfRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interpreted {
    pub kind: MsgKind,
    /// The pitch class for Key, the scale for Scale, the BPM for Tempo, the raw value for
    /// OutOfRange.
    pub value: u8,
    /// The message for the log, e.g. "Note On 57 (A) vel 100 ch 16"; empty for system
    /// messages (clock, MTC, SysEx), which are never logged.
    pub text: String,
}

impl Interpreted {
    fn ignored(text: String) -> Self {
        Interpreted { kind: MsgKind::Ignored, value: 0, text }
    }
}

// A channel message for the log when it is ignored ("Program Change 5 ch 1").
fn describe(msg: &[u8]) -> String {
    const TYPES: [&str; 8] = ["Note Off", "Note On", "Poly Pressure", "CC", "Program Change", "Channel Pressure", "Pitch Bend", ""];
    let mut s = TYPES[((msg[0] >> 4) & 7) as usize].to_string();
    for b in msg.iter().skip(1).take(2) {
        s.push_str(&format!(" {b}"));
    }
    format!("{s} ch {}", (msg[0] & 0x0F) + 1)
}

/// One whole MIDI message (status byte first, running status already expanded).
/// `channel`: 1..16, or 0 = any (Omni). `scale_cc`: 0..119.
pub fn interpret(msg: &[u8], channel: u8, scale_cc: u8) -> Interpreted {
    if msg.is_empty() || msg[0] < 0x80 || msg[0] >= 0xF0 {
        return Interpreted::ignored(String::new()); // system messages: not even logged
    }
    let text = describe(msg);
    let (kind, ch) = (msg[0] & 0xF0, (msg[0] & 0x0F) + 1);
    // The Note Off that ends a cue's note (or Note On velocity 0) on our channel is part of
    // the cue: not a change, and not worth an "ignored" line either (rig, 2026-11-23).
    let on_ours = channel == 0 || ch == channel;
    if on_ours && msg.len() >= 3 && (kind == 0x80 || (kind == 0x90 && msg[2] == 0)) {
        return Interpreted::ignored(String::new());
    }
    if msg.len() < 3 || msg[1] > 0x7F || msg[2] > 0x7F || (kind != 0x90 && kind != 0xB0) || (channel != 0 && ch != channel) {
        return Interpreted::ignored(text);
    }
    if kind == 0x90 {
        if msg[2] == 0 {
            return Interpreted::ignored(text); // Note On vel 0 = Note Off
        }
        let pc = msg[1] % 12;
        return Interpreted {
            kind: MsgKind::Key,
            value: pc,
            text: format!("Note On {} ({}) vel {} ch {ch}", msg[1], pitch_class_name(pc), msg[2]),
        };
    }
    if (msg[1] == TEMPO_CC || msg[1] == TEMPO_HIGH_CC) && msg[1] != scale_cc {
        return match tempo_bpm(msg[1], msg[2]) {
            Some(b) => Interpreted { kind: MsgKind::Tempo, value: b, text: format!("CC {} = {} ({b} BPM) ch {ch}", msg[1], msg[2]) },
            None => Interpreted {
                kind: MsgKind::OutOfRange,
                value: msg[2],
                text: format!("CC {} = {} ch {ch} ignored: LV1's tempo starts at 40 BPM", msg[1], msg[2]),
            },
        };
    }
    if msg[1] != scale_cc {
        return Interpreted::ignored(text);
    }
    match scale_name(msg[2]) {
        Some(n) => Interpreted { kind: MsgKind::Scale, value: msg[2], text: format!("CC {} = {} ({n}) ch {ch}", msg[1], msg[2]) },
        None => Interpreted {
            kind: MsgKind::OutOfRange,
            value: msg[2],
            text: format!("CC {} = {} ch {ch} ignored: Tune's scales are 0-{}", msg[1], msg[2], NUM_SCALES - 1),
        },
    }
}

/// Messages within [`Coalescer::WINDOW_MS`] of the first one are one change: a clip that
/// sends the scale CC and the key note together gives one pass over the Tunes. The last
/// key and the last scale in the window win.
#[derive(Debug, Default)]
pub struct Coalescer {
    cur: Intent,
    due_at: u64,
}

impl Coalescer {
    pub const WINDOW_MS: u64 = 30;

    pub fn add(&mut self, m: &Interpreted, now_ms: u64) {
        if m.kind != MsgKind::Key && m.kind != MsgKind::Scale {
            return;
        }
        if self.cur.is_empty() {
            self.due_at = now_ms + Self::WINDOW_MS;
        }
        if m.kind == MsgKind::Key {
            self.cur.pitch_class = Some(m.value);
        } else {
            self.cur.scale = Some(m.value);
        }
    }

    /// Apply now from the settings window: due at once, merged with anything pending.
    pub fn add_now(&mut self, i: &Intent, now_ms: u64) {
        if i.pitch_class.is_some() {
            self.cur.pitch_class = i.pitch_class;
        }
        if i.scale.is_some() {
            self.cur.scale = i.scale;
        }
        if !self.cur.is_empty() {
            self.due_at = now_ms;
        }
    }

    pub fn pending(&self) -> bool {
        !self.cur.is_empty()
    }

    /// Milliseconds until the pending change is due (0 = now), `None` when nothing is pending.
    pub fn due_in(&self, now_ms: u64) -> Option<u64> {
        if self.cur.is_empty() {
            None
        } else {
            Some(self.due_at.saturating_sub(now_ms))
        }
    }

    pub fn take(&mut self) -> Intent {
        std::mem::take(&mut self.cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(ch: u8, n: u8, vel: u8) -> Vec<u8> {
        vec![0x90 | (ch - 1), n, vel]
    }
    fn cc(ch: u8, c: u8, v: u8) -> Vec<u8> {
        vec![0xB0 | (ch - 1), c, v]
    }
    fn i(m: &[u8]) -> Interpreted {
        interpret(m, 16, 20)
    }

    #[test]
    fn interpretation() {
        let m = i(&note(16, 57, 100));
        assert_eq!(m.kind, MsgKind::Key);
        assert_eq!(m.value, 9);
        assert!(m.text.contains("Note On 57 (A)"), "{}", m.text);
        assert_eq!(i(&note(16, 61, 1)).value, 1, "C# any octave");
        assert_eq!(i(&note(16, 57, 0)).kind, MsgKind::Ignored, "vel 0 = note off");
        assert_eq!(i(&[0x8F, 57, 64]).kind, MsgKind::Ignored, "note off");
        assert!(i(&note(16, 57, 0)).text.is_empty() && i(&[0x8F, 57, 64]).text.is_empty(), "a cue's own Note Off is not logged");
        assert!(!i(&[0x80, 57, 64]).text.is_empty(), "a Note Off on another channel is (a keyboard)");
        let other = i(&note(15, 57, 100));
        assert_eq!(other.kind, MsgKind::Ignored);
        assert!(other.text.contains("ch 15"));
        assert_eq!(interpret(&note(3, 57, 100), 0, 20).kind, MsgKind::Key, "omni");
        let m = i(&cc(16, 20, 3));
        assert_eq!(m.kind, MsgKind::Scale);
        assert_eq!(m.value, 3);
        assert!(m.text.contains("Natural Minor"));
        assert_eq!(i(&cc(16, 20, 14)).kind, MsgKind::OutOfRange);
        assert_eq!(i(&cc(16, 23, 3)).kind, MsgKind::Ignored);
        let t = i(&cc(16, 21, 56));
        assert_eq!((t.kind, t.value, t.text.as_str()), (MsgKind::Tempo, 56, "CC 21 = 56 (56 BPM) ch 16"), "a slow ballad");
        let t = i(&cc(16, 22, 28));
        assert_eq!((t.kind, t.value, t.text.as_str()), (MsgKind::Tempo, 128, "CC 22 = 28 (128 BPM) ch 16"));
        let slow = i(&cc(16, 21, 30));
        assert_eq!(slow.kind, MsgKind::OutOfRange);
        assert!(slow.text.ends_with("ignored: LV1's tempo starts at 40 BPM"), "{}", slow.text);
        assert_eq!(i(&cc(15, 21, 68)).kind, MsgKind::Ignored, "tempo is on our channel only");
        assert_eq!(interpret(&cc(16, 21, 3), 16, 21).kind, MsgKind::Scale, "a scale CC of 21 wins");
        assert_eq!((tempo_bpm(21, 40), tempo_bpm(21, 127), tempo_bpm(21, 39)), (Some(40), Some(127), None));
        assert_eq!((tempo_bpm(22, 0), tempo_bpm(22, 127), tempo_bpm(20, 5)), (Some(100), Some(227), None));
        assert_eq!((bpm_text(128.0), bpm_text(127.96), bpm_text(92.5)), ("128".into(), "128".into(), "92.5".into()));
        assert_eq!(interpret(&cc(16, 7, 3), 16, 7).kind, MsgKind::Scale, "configurable cc");
        assert_eq!(i(&[0x9F, 57]).kind, MsgKind::Ignored, "truncated");
        assert!(i(&[0xF8]).text.is_empty() && i(&[0xF1, 0x20]).text.is_empty(), "clock and MTC are silent");
        let pc = i(&[0xCF, 5]);
        assert_eq!(pc.kind, MsgKind::Ignored);
        assert!(pc.text.contains("Program Change 5"));
    }

    #[test]
    fn coalescing() {
        let mut c = Coalescer::default();
        assert_eq!(c.due_in(0), None);
        c.add(&i(&cc(16, 20, 3)), 1000);
        c.add(&i(&note(16, 57, 100)), 1005);
        assert_eq!(c.due_in(1010), Some(20), "window from the first message");
        assert_eq!(c.due_in(1030), Some(0));
        let t = c.take();
        assert_eq!((t.pitch_class, t.scale), (Some(9), Some(3)));
        assert!(!c.pending());
        c.add(&i(&note(16, 60, 100)), 2000);
        c.add(&i(&note(16, 62, 100)), 2010);
        assert_eq!(c.take().pitch_class, Some(2), "last key wins");
        c.add(&i(&cc(16, 21, 3)), 3000);
        assert!(!c.pending(), "ignored messages are not a change");
        c.add(&i(&note(16, 60, 100)), 4000);
        c.add_now(&Intent { pitch_class: None, scale: Some(2) }, 4001);
        assert_eq!(c.due_in(4001), Some(0), "apply now is due at once");
        let t = c.take();
        assert_eq!((t.pitch_class, t.scale), (Some(0), Some(2)), "apply now merges");
    }

    #[test]
    fn labels() {
        assert_eq!(Intent { pitch_class: Some(9), scale: Some(3) }.label(), "A Natural Minor");
        assert_eq!(Intent { pitch_class: Some(1), scale: None }.label(), "C#/Db");
        assert_eq!(Intent { pitch_class: None, scale: Some(2) }.label(), "Major");
    }
}
