//! Waves Tune Real-Time as LV1 exposes it to a control surface: which parameters hold
//! the key, the scale and Receive, what their values mean, and how a MIDI note becomes
//! a key value that keeps the spelling a Tune already uses.
//!
//! Proven on LV1 16.5.189.293 (2026-10-25). A plug-in parameter is section 4,
//! sectionIndex = insert slot, channel = parameter index, controlId 11 = value,
//! text = LV1's display string. Slot identity is channel -1: cid 0 = name ("Tune RT"),
//! cid 1 = four-letter code ("LVLM" = mono Tune RT).

pub const SEC_PLUGINS: u32 = 4;
pub const CID_VALUE: i32 = 11;
pub const CID_SLOT_NAME: i32 = 0;
pub const CID_SLOT_CODE: i32 = 1;
/// The "channel" that answers slot identity.
pub const SLOT_INFO: i32 = -1;

pub const PARAM_SCALE: i32 = 39;
pub const PARAM_KEY: i32 = 40;
/// 1 = "Rcv", 0 = "Lock".
pub const PARAM_RECEIVE: i32 = 42;

/// C C# Db D D# Eb E F F# Gb G G# Ab A A# Bb B
pub const NUM_KEYS: u8 = 17;
/// 0 User Scale .. 10 Mixolydian
pub const NUM_SCALES: u8 = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spelling {
    Natural,
    Sharp,
    Flat,
}

// LV1's display strings, in value order (read back from Tune RT on the workstation).
const KEYS: [&str; 17] = ["C", "C#", "Db", "D", "D#", "Eb", "E", "F", "F#", "Gb", "G", "G#", "Ab", "A", "A#", "Bb", "B"];
const KEY_PC: [u8; 17] = [0, 1, 1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 8, 9, 10, 10, 11];
const SCALES: [&str; 11] = [
    "User Scale",
    "Chromatic",
    "Major",
    "Natural Minor",
    "Harmonic Minor",
    "Melodic Minor (Asc)",
    "Melodic Minor (Asc+Dec)",
    "Major Justonic",
    "Minor Justonic",
    "Dorian",
    "Mixolydian",
];
// Per pitch class: the sharp-spelled value and the flat-spelled value (equal for white keys).
const SHARP_OF: [u8; 12] = [0, 1, 3, 4, 6, 7, 8, 10, 11, 13, 14, 16];
const FLAT_OF: [u8; 12] = [0, 2, 3, 5, 6, 7, 9, 10, 12, 13, 15, 16];

pub fn key_name(v: u8) -> Option<&'static str> {
    KEYS.get(v as usize).copied()
}

pub fn scale_name(v: u8) -> Option<&'static str> {
    SCALES.get(v as usize).copied()
}

/// Pitch class (0 = C .. 11 = B) of a key value.
pub fn key_pitch_class(v: u8) -> Option<u8> {
    KEY_PC.get(v as usize).copied()
}

/// Natural for C D E F G A B, else whether the value is the sharp or the flat spelling.
pub fn key_spelling(v: u8) -> Spelling {
    match key_pitch_class(v) {
        Some(pc) if SHARP_OF[pc as usize] != FLAT_OF[pc as usize] => {
            if v == FLAT_OF[pc as usize] {
                Spelling::Flat
            } else {
                Spelling::Sharp
            }
        }
        _ => Spelling::Natural,
    }
}

/// The key value for a pitch class. `prefer` decides only for the five black keys
/// (Natural counts as Sharp there).
pub fn key_value(pitch_class: u8, prefer: Spelling) -> Option<u8> {
    if pitch_class > 11 {
        return None;
    }
    Some(if prefer == Spelling::Flat { FLAT_OF[pitch_class as usize] } else { SHARP_OF[pitch_class as usize] })
}

/// The spelling a Tune keeps: its current key's own, or `fallback` when that key is natural.
pub fn keep_spelling(current_key: Option<u8>, fallback: Spelling) -> Spelling {
    match current_key.map(key_spelling) {
        Some(Spelling::Natural) | None => fallback,
        Some(s) => s,
    }
}

/// "C", "C#/Db", ... for a pitch class, for log lines that are about the MIDI note.
pub fn pitch_class_name(pc: u8) -> String {
    if pc > 11 {
        return "?".into();
    }
    let (s, f) = (SHARP_OF[pc as usize], FLAT_OF[pc as usize]);
    if s == f {
        KEYS[s as usize].into()
    } else {
        format!("{}/{}", KEYS[s as usize], KEYS[f as usize])
    }
}

/// Is this slot code Tune Real-Time? LVLM = mono (proven); LVLS = stereo (assumed, by the
/// M/S pattern every other Waves code here follows: QDZM/QDZS, VXPM/VXPS).
pub fn is_tune_code(code: &str) -> bool {
    code == "LVLM" || code == "LVLS"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables() {
        assert_eq!(key_name(0), Some("C"));
        assert_eq!(key_name(13), Some("A"));
        assert_eq!(key_name(16), Some("B"));
        assert_eq!(key_name(17), None);
        assert_eq!(scale_name(3), Some("Natural Minor"));
        assert_eq!(scale_name(10), Some("Mixolydian"));
        assert_eq!(scale_name(11), None);
    }

    #[test]
    fn spelling() {
        assert_eq!(key_value(1, Spelling::Sharp), Some(1));
        assert_eq!(key_value(1, Spelling::Flat), Some(2));
        assert_eq!(key_value(3, Spelling::Flat), Some(5));
        assert_eq!(key_value(6, Spelling::Flat), Some(9));
        assert_eq!(key_value(10, Spelling::Flat), Some(15));
        assert_eq!(key_value(9, Spelling::Flat), Some(13));
        assert_eq!(key_value(9, Spelling::Sharp), Some(13));
        assert_eq!(key_value(11, Spelling::Sharp), Some(16));
        assert_eq!(key_value(12, Spelling::Sharp), None);
        assert_eq!(key_spelling(5), Spelling::Flat);
        assert_eq!(key_spelling(4), Spelling::Sharp);
        assert_eq!(key_spelling(13), Spelling::Natural);
        assert_eq!(keep_spelling(Some(5), Spelling::Sharp), Spelling::Flat);
        assert_eq!(keep_spelling(Some(13), Spelling::Flat), Spelling::Flat);
        assert_eq!(keep_spelling(Some(13), Spelling::Sharp), Spelling::Sharp);
        for v in 0..NUM_KEYS {
            let s = if key_spelling(v) == Spelling::Flat { Spelling::Flat } else { Spelling::Sharp };
            assert_eq!(key_value(key_pitch_class(v).unwrap(), s), Some(v), "round trip {v}");
        }
        assert_eq!(pitch_class_name(1), "C#/Db");
        assert_eq!(pitch_class_name(9), "A");
    }

    #[test]
    fn codes() {
        assert!(is_tune_code("LVLM") && is_tune_code("LVLS"));
        assert!(!is_tune_code("VXPM") && !is_tune_code(""));
    }
}
