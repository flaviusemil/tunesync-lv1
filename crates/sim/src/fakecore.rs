//! A fake MixerCoreSG.dll: the seven `*Wrapper` callbacks LV1 hands a surface driver,
//! over three Tune RT instances held in memory, so TuneSync_Control.dll runs (under Wine)
//! without LV1.
//!
//! Like LV1 (rig, 2026-10-20), SetMixerControl is asynchronous: a Control lands ~1 ms
//! later on another thread, so the driver's wait-for-the-value path is exercised.
//!
//! The Tunes (LV1's 0-based track index):
//! - Ch 43 (0, 42) slot 1: A Natural Minor, Rcv, named "Vox 1"
//! - Ch 44 (0, 43) slot 3: Eb Major, Rcv (a flat speller)
//! - Ch 45 (0, 44) slot 1: F# Major, Lock
//!
//! `fakecore_*` exports let lv1sim inspect and count.
#![allow(clippy::missing_safety_doc)]
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use lv1_surface::abi::{ctl, param, section, MixerControl, MixerParamSurfaceStatus, ParamHeader};

const KEYS: [&str; 17] = ["C", "C#", "Db", "D", "D#", "Eb", "E", "F", "F#", "Gb", "G", "G#", "Ab", "A", "A#", "Bb", "B"];
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

type Key = (u32, u32, u32); // track type, track index, slot

static MODEL: Mutex<BTreeMap<Key, BTreeMap<u32, f64>>> = Mutex::new(BTreeMap::new());
static GETS: AtomicU64 = AtomicU64::new(0);
static SETS: AtomicU64 = AtomicU64::new(0);
static TOUCHES: AtomicU64 = AtomicU64::new(0);
static RELEASES: AtomicU64 = AtomicU64::new(0);
/// The last SurfaceStatus a driver sent: (surface type, status, colour, picture).
static STATUS: Mutex<(String, String, u32, String)> = Mutex::new((String::new(), String::new(), 0, String::new()));

fn model() -> std::sync::MutexGuard<'static, BTreeMap<Key, BTreeMap<u32, f64>>> {
    MODEL.lock().unwrap_or_else(|e| e.into_inner())
}

#[no_mangle]
pub extern "C" fn fakecore_reset() {
    let mut m = model();
    m.clear();
    for (k, key, scale, rcv) in [((0, 42, 0), 13.0, 3.0, 1.0), ((0, 43, 2), 5.0, 2.0, 1.0), ((0, 44, 0), 8.0, 2.0, 0.0)] {
        m.insert(k, BTreeMap::from([(40, key), (39, scale), (42, rcv), (41, 1.0)]));
    }
}

#[no_mangle]
pub extern "C" fn fakecore_param(tt: u32, ti: u32, slot: u32, param: u32) -> f64 {
    model().get(&(tt, ti, slot)).and_then(|p| p.get(&param)).copied().unwrap_or(-1.0)
}

/// gets, sets (Controls), touches, releases
#[no_mangle]
pub unsafe extern "C" fn fakecore_counts(out: *mut u64) {
    for (i, c) in [&GETS, &SETS, &TOUCHES, &RELEASES].iter().enumerate() {
        *out.add(i) = c.load(Ordering::SeqCst);
    }
}

fn text_of(param: u32, v: f64) -> String {
    let i = v as usize;
    match param {
        40 => KEYS.get(i).copied().unwrap_or("?").into(),
        39 => SCALES.get(i).copied().unwrap_or("?").into(),
        42 => (if i == 1 { "Rcv" } else { "Lock" }).into(),
        41 => "Snd".into(),
        _ => format!("{v}"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn GetMixerControlWrapper(_core: *mut c_void, arg: *mut c_void) -> i32 {
    GETS.fetch_add(1, Ordering::SeqCst);
    let c = &mut *(arg as *mut MixerControl);
    let id = c.id;
    c.value = 0.0;
    c.set_text("");
    if id.section == section::NAME {
        if (id.track_type, id.track_index) == (0, 42) {
            c.set_text("Vox 1");
        } else {
            c.set_text(&format!("Channel {}", id.track_index + 1)); // LV1's default name
        }
        return 0;
    }
    if id.section != section::PLUGINS {
        return 0;
    }
    let m = model();
    let Some(p) = m.get(&(id.track_type, id.track_index, id.section_index)) else { return 0 }; // empty slot: empty text
    if id.channel == u32::MAX {
        c.set_text(match id.control_id {
            0 => "Tune RT",
            1 => "LVLM",
            _ => "",
        });
        return 0;
    }
    if id.control_id == 11 {
        if let Some(v) = p.get(&{ id.channel }) {
            c.value = *v;
            c.set_text(&text_of(id.channel, *v));
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn SetMixerControlWrapper(_core: *mut c_void, arg: *mut c_void) -> i32 {
    let c = *(arg as *const MixerControl);
    if { c.struct_type } != 0 {
        return -1;
    }
    let id = c.id;
    match c.ctl_type {
        ctl::TOUCH => {
            TOUCHES.fetch_add(1, Ordering::SeqCst);
        }
        ctl::RELEASE => {
            RELEASES.fetch_add(1, Ordering::SeqCst);
        }
        ctl::CONTROL if id.section == section::PLUGINS && id.control_id == 11 => {
            SETS.fetch_add(1, Ordering::SeqCst);
            let (k, param, v) = ((id.track_type, id.track_index, id.section_index), id.channel, c.value);
            // Asynchronous, as LV1 is: the value shows ~1 ms later.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1));
                if let Some(p) = model().get_mut(&k) {
                    p.insert(param, v);
                }
            });
        }
        _ => {}
    }
    0
}

#[no_mangle]
pub extern "C" fn SetMixerAssignmentsWrapper(_: *mut c_void, _: *mut c_void) -> i32 {
    0
}
#[no_mangle]
pub extern "C" fn GetMixerAssignmentsWrapper(_: *mut c_void, _: *mut c_void) -> i32 {
    0
}
/// Takes a SurfaceStatus the way LV1 does: the strings through their pointers, and -1
/// for an empty surface type. Every other param is accepted and ignored.
#[no_mangle]
pub unsafe extern "C" fn SetMixerParamWrapper(_: *mut c_void, p: *mut c_void) -> i32 {
    let Some(head) = (p as *const ParamHeader).as_ref() else { return -1 };
    if { head.type_ } != param::SURFACE_STATUS {
        return 0;
    }
    let s = &*(p as *const MixerParamSurfaceStatus);
    let ty = s.surface_type.text();
    if ty.is_empty() {
        return -1;
    }
    let status: String = s.status.text().chars().take(12).collect(); // LV1 keeps 12
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = (ty, status, s.color as u32, s.extra.text());
    0
}

/// The last SurfaceStatus: the surface type, status text and picture into the buffers
/// (`len` bytes each, NUL-terminated), the colour returned. 0 and "" before any.
#[no_mangle]
pub unsafe extern "C" fn fakecore_status(ty: *mut u8, status: *mut u8, picture: *mut u8, len: usize) -> u32 {
    let st = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    for (dst, src) in [(ty, &st.0), (status, &st.1), (picture, &st.3)] {
        if !dst.is_null() && len > 0 {
            let n = src.len().min(len - 1);
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst, n);
            *dst.add(n) = 0;
        }
    }
    st.2
}
#[no_mangle]
pub extern "C" fn GetMixerParamWrapper(_: *mut c_void, _: *mut c_void) -> i32 {
    0
}
#[no_mangle]
pub extern "C" fn SendMixerCommandWrapper(_: *mut c_void, _: *mut c_void) -> i32 {
    0
}
