//! TuneSync as a C library: [`tunesync_host::Runtime`] behind `include/tunesync.h`
//! (generated from this file by cbindgen), with the mixer supplied by the host as a C
//! vtable, [`TsMixer`]. This is how a C++ host such as Headlink embeds TuneSync.
//!
//! - Every exported function catches panics and returns an error value instead: no
//!   unwinding crosses the C boundary.
//! - The vtable is called only from the runtime's worker thread, through a gate that
//!   `ts_stop` closes after the worker ended (or timed out), so the host's mixer is never
//!   entered once `ts_stop` returned, and its `ctx` may then be freed.
//! - By default the whole runtime runs (TuneSync's own MIDI input by name, RTP-MIDI with
//!   DNS-SD, settings.ini and tunesync.log in the data folder); `TS_START_NO_MIDI_IN` /
//!   `TS_START_NO_RTP` leave those inputs to the host, which feeds MIDI with `ts_feed_midi`.
#![allow(clippy::missing_safety_doc)] // the contract is the header's preamble, one place

use std::cell::Cell;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tunesync::apply::{Mixer, ParamRead};
use tunesync::log::Level;
use tunesync::midi_map::Intent;
use tunesync::settings::{from_ini, is_ipv4, to_ini};
use tunesync::tune::NUM_SCALES;
use tunesync_host::sys::data_dir;
use tunesync_host::{Runtime, RuntimeConfig};

pub mod json;

/// Bumped on any incompatible change to this header. A host checks
/// `ts_abi_version() == TS_ABI_VERSION` before `ts_start`.
pub const TS_ABI_VERSION: u32 = 1;

/// `ts_start` flag: do not open the MIDI input named in the settings (`midi_port`); the
/// host feeds MIDI with `ts_feed_midi`.
pub const TS_START_NO_MIDI_IN: u32 = 1;
/// `ts_start` flag: no RTP-MIDI session and no DNS-SD (`rtp_*` settings are kept, unused).
pub const TS_START_NO_RTP: u32 = 2;

/// The size of the text buffers TuneSync passes to `TsMixer.get` and `TsMixer.track_name`.
pub const TS_TEXT_CAP: usize = 256;

/// Success.
pub const TS_OK: c_int = 0;
/// A NULL handle or argument, a value out of range, or text that is not UTF-8.
pub const TS_ERR_ARG: c_int = -1;
/// `ts_stop` was called from inside a `TsMixer` callback: nothing was done.
pub const TS_ERR_IN_CALLBACK: c_int = -2;
/// TuneSync failed inside (a Rust panic, caught); the call had no effect or only part.
pub const TS_ERR_PANIC: c_int = -3;

/// Reads a plug-in control. LV1's terms: section 4 (plug-ins), sectionIndex = `slot` (the
/// insert slot, 0..7), channel = `channel` (the parameter index, or -1 for the slot's
/// identity: `cid` 0 = plug-in name, 1 = four-letter code), controlId = `cid` (11 =
/// value). `track_type`: 0 = Ch, 1 = Gp, 2 = Fx. Write the value to `*value` and LV1's
/// display text, NUL-terminated, into `text[0..text_cap)`. Return 0 when the control
/// answered (LV1's own GetMixerControl result works as is), anything else when not.
pub type TsGetFn =
    Option<unsafe extern "C" fn(ctx: *mut c_void, track_type: u32, track_index: u32, slot: u32, channel: i32, cid: i32, value: *mut f64, text: *mut c_char, text_cap: usize) -> c_int>;

/// Sets a plug-in parameter's value (controlId 11) the way a surface knob does: Touch,
/// Control(value), Release. LV1's SetMixerControl is asynchronous, and TuneSync reads every
/// change back right after: wait (bounded, ~400 ms) until a read shows the new value
/// before the Release, as TuneSync's own LV1 driver does. Return 0 when LV1 accepted it.
pub type TsSetFn = Option<unsafe extern "C" fn(ctx: *mut c_void, track_type: u32, track_index: u32, slot: u32, param: i32, value: f64) -> c_int>;

/// The track's user name, NUL-terminated into `out[0..cap)`; returns its length in bytes
/// without the NUL (0 = no name). TuneSync drops LV1's default names ("Channel 43").
pub type TsTrackNameFn = Option<unsafe extern "C" fn(ctx: *mut c_void, track_type: u32, track_index: u32, out: *mut c_char, cap: usize) -> usize>;

/// The host's mixer. Copied by `ts_start`; every callback is required. Called only from the
/// runtime's worker thread, one call at a time, until `ts_stop` returns.
#[repr(C)]
pub struct TsMixer {
    /// Passed back to every callback.
    pub ctx: *mut c_void,
    pub get: TsGetFn,
    pub set: TsSetFn,
    pub track_name: TsTrackNameFn,
}

/// A running TuneSync, from `ts_start` until `ts_stop`.
pub struct TsRuntime {
    rt: Arc<Runtime>,
    gate: Arc<Mutex<bool>>,
}

thread_local! {
    // Set while a TsMixer callback runs on this thread (the worker).
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

fn in_callback() -> bool {
    IN_CALLBACK.try_with(|c| c.get()).unwrap_or(false)
}

/// The host's vtable as the runtime's [`Mixer`]. Each call holds the gate: `ts_stop`
/// closes it, waiting for a call in flight, and later calls answer "no" without the host.
struct CMixer {
    ctx: *mut c_void,
    // All three non-NULL: ts_start checked.
    get: TsGetFn,
    set: TsSetFn,
    name: TsTrackNameFn,
    gate: Arc<Mutex<bool>>,
}

// The host's ctx is used from the worker thread only, which the header promises the host.
unsafe impl Send for CMixer {}

fn text_of(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

impl CMixer {
    fn call<T>(&self, closed: T, f: impl FnOnce() -> T) -> T {
        let open = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        if !*open {
            return closed;
        }
        IN_CALLBACK.with(|c| c.set(true));
        let r = f();
        IN_CALLBACK.with(|c| c.set(false));
        drop(open);
        r
    }
}

impl Mixer for CMixer {
    fn get(&mut self, tt: u32, ti: u32, slot: u32, channel: i32, cid: i32) -> ParamRead {
        let (Some(f), ctx) = (self.get, self.ctx) else { return ParamRead::default() };
        self.call(ParamRead::default(), || {
            let mut value = 0.0f64;
            let mut buf = [0u8; TS_TEXT_CAP];
            let rc = unsafe { f(ctx, tt, ti, slot, channel, cid, &mut value, buf.as_mut_ptr().cast(), buf.len()) };
            buf[TS_TEXT_CAP - 1] = 0;
            if rc != 0 {
                return ParamRead::default();
            }
            ParamRead { ok: true, value, text: text_of(&buf) }
        })
    }

    fn set(&mut self, tt: u32, ti: u32, slot: u32, param: i32, value: f64) -> bool {
        let (Some(f), ctx) = (self.set, self.ctx) else { return false };
        self.call(false, || unsafe { f(ctx, tt, ti, slot, param, value) } == 0)
    }

    fn track_name(&mut self, tt: u32, ti: u32) -> String {
        let (Some(f), ctx) = (self.name, self.ctx) else { return String::new() };
        self.call(String::new(), || {
            let mut buf = [0u8; TS_TEXT_CAP];
            let n = unsafe { f(ctx, tt, ti, buf.as_mut_ptr().cast(), buf.len()) };
            buf[TS_TEXT_CAP - 1] = 0;
            text_of(&buf[..n.min(TS_TEXT_CAP - 1)])
        })
    }
}

fn guard<T>(failed: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(failed)
}

unsafe fn text_arg(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok().map(str::to_string)
}

/// A string for the host, freed with ts_string_free. NULs inside cannot be represented.
fn give(s: String) -> *mut c_char {
    CString::new(s.replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw)
}

unsafe fn runtime<'a>(rt: *const TsRuntime) -> Option<&'a Runtime> {
    rt.as_ref().map(|h| &*h.rt)
}

/// TS_ABI_VERSION of this library.
#[no_mangle]
pub extern "C" fn ts_abi_version() -> u32 {
    TS_ABI_VERSION
}

/// "0.1.0": TuneSync's version (static, do not free).
#[no_mangle]
pub extern "C" fn ts_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// Starts TuneSync: reads `<data_dir>/settings.ini`, logs to `<data_dir>/tunesync.log`,
/// starts the worker (the first scan of the mixer begins at once) and, unless `flags` say
/// otherwise, the MIDI input and RTP-MIDI.
/// - `data_dir`: NULL = the default (%ProgramData%\TuneSync on Windows, or $TUNESYNC_DIR);
///   created when missing.
/// - `title`: the first log line's name, NULL = "TuneSync <version>".
/// - `mixer`: copied; all three callbacks required.
/// - `flags`: TS_START_NO_MIDI_IN | TS_START_NO_RTP, or 0 for the whole runtime.
///
/// Returns NULL on a bad argument or an unknown flag. One TuneSync per process normally:
/// two would fight over the MIDI port and UDP 5004.
#[no_mangle]
pub unsafe extern "C" fn ts_start(data_dir: *const c_char, title: *const c_char, mixer: *const TsMixer, flags: u32) -> *mut TsRuntime {
    guard(std::ptr::null_mut(), || {
        let Some(m) = mixer.as_ref() else { return std::ptr::null_mut() };
        if m.get.is_none() || m.set.is_none() || m.track_name.is_none() {
            return std::ptr::null_mut();
        }
        if flags & !(TS_START_NO_MIDI_IN | TS_START_NO_RTP) != 0 {
            return std::ptr::null_mut();
        }
        let dir = if data_dir.is_null() {
            self::data_dir()
        } else {
            match text_arg(data_dir) {
                Some(d) if !d.is_empty() => PathBuf::from(d),
                _ => return std::ptr::null_mut(),
            }
        };
        let title = if title.is_null() { format!("TuneSync {}", env!("CARGO_PKG_VERSION")) } else { CStr::from_ptr(title).to_string_lossy().into_owned() };
        let gate = Arc::new(Mutex::new(true));
        let mixer = CMixer { ctx: m.ctx, get: m.get, set: m.set, name: m.track_name, gate: gate.clone() };
        let cfg = RuntimeConfig { local_midi: flags & TS_START_NO_MIDI_IN == 0, rtp: flags & TS_START_NO_RTP == 0, ..RuntimeConfig::new(dir, title) };
        let rt = Runtime::start(cfg, Box::new(mixer));
        Box::into_raw(Box::new(TsRuntime { rt, gate }))
    })
}

/// Stops every thread and frees the handle. The worker finishes the change it is applying
/// (bounded, 1.5 s); once this returns the mixer is never called again. NULL is a no-op.
/// TS_ERR_IN_CALLBACK from inside a callback (the handle stays valid); otherwise the
/// handle is gone, whatever it returns.
#[no_mangle]
pub unsafe extern "C" fn ts_stop(rt: *mut TsRuntime) -> c_int {
    if rt.is_null() {
        return TS_OK;
    }
    if in_callback() {
        return TS_ERR_IN_CALLBACK;
    }
    let h = Box::from_raw(rt);
    let r = guard(TS_ERR_PANIC, || {
        h.rt.stop();
        TS_OK
    });
    // Closed whatever happened above; a call still in flight (the worker timed out)
    // finishes first.
    *h.gate.lock().unwrap_or_else(|e| e.into_inner()) = false;
    r
}

/// MIDI from the host's own inputs: `len` bytes holding one or more whole messages
/// (running status is expanded within one call, not across calls). `source` names the
/// input in the log and the last-change banner ("Headlink MIDI"); NULL = "Host MIDI".
/// Works with or without TS_START_NO_MIDI_IN.
#[no_mangle]
pub unsafe extern "C" fn ts_feed_midi(rt: *mut TsRuntime, source: *const c_char, bytes: *const u8, len: usize) -> c_int {
    guard(TS_ERR_PANIC, || {
        let Some(r) = runtime(rt) else { return TS_ERR_ARG };
        if bytes.is_null() && len > 0 {
            return TS_ERR_ARG;
        }
        if len == 0 {
            return TS_OK;
        }
        let source = if source.is_null() { "Host MIDI".to_string() } else { CStr::from_ptr(source).to_string_lossy().into_owned() };
        r.feed_midi(&source, std::slice::from_raw_parts(bytes, len));
        TS_OK
    })
}

/// Sets every unlocked Tune now (the window's Apply now): `pitch_class` 0 = C .. 11 = B,
/// `scale` 0..10 (Tune's own numbers); -1 leaves that one as it is (not both).
#[no_mangle]
pub unsafe extern "C" fn ts_apply_now(rt: *mut TsRuntime, pitch_class: c_int, scale: c_int) -> c_int {
    guard(TS_ERR_PANIC, || {
        let Some(r) = runtime(rt) else { return TS_ERR_ARG };
        let pc = match pitch_class {
            -1 => None,
            0..=11 => Some(pitch_class as u8),
            _ => return TS_ERR_ARG,
        };
        let sc = match scale {
            -1 => None,
            s if (0..NUM_SCALES as c_int).contains(&s) => Some(s as u8),
            _ => return TS_ERR_ARG,
        };
        let i = Intent { pitch_class: pc, scale: sc };
        if i.is_empty() {
            return TS_ERR_ARG;
        }
        r.apply_now(i);
        TS_OK
    })
}

/// Refreshes the Tunes list (ts_tunes_json) on the worker's next pass.
#[no_mangle]
pub unsafe extern "C" fn ts_rescan(rt: *mut TsRuntime) -> c_int {
    guard(TS_ERR_PANIC, || match runtime(rt) {
        Some(r) => {
            r.rescan();
            TS_OK
        }
        None => TS_ERR_ARG,
    })
}

/// The settings as settings.ini text. NULL on a bad handle. Free with ts_string_free.
#[no_mangle]
pub unsafe extern "C" fn ts_settings_get(rt: *mut TsRuntime) -> *mut c_char {
    guard(std::ptr::null_mut(), || runtime(rt).map_or(std::ptr::null_mut(), |r| give(to_ini(&r.settings()))))
}

/// Replaces the settings with `ini` (settings.ini text: what ts_settings_get gives,
/// edited; a key left out takes its default), saves and applies them at once. A line
/// that is not understood is skipped and logged. Returns how many were skipped (>= 0), or
/// TS_ERR_*.
#[no_mangle]
pub unsafe extern "C" fn ts_settings_set(rt: *mut TsRuntime, ini: *const c_char) -> c_int {
    guard(TS_ERR_PANIC, || {
        let (Some(r), Some(text)) = (runtime(rt), text_arg(ini)) else { return TS_ERR_ARG };
        let mut warnings = Vec::new();
        let s = from_ini(&text, &mut warnings);
        for w in &warnings {
            r.log().add(Level::Warn, format!("settings: {w}"));
        }
        r.update(|g| *g = s);
        warnings.len().min(c_int::MAX as usize) as c_int
    })
}

/// The status as JSON: `{"now_ms","tunes","locked","changes","scans","last_midi_ms",
/// "last_source","last_result","last":{"label","stamp","source","brief","problem"}|null,
/// "midi":{"ports","open","last_ms"}|null, "rtp":{"enabled","open","error","name","port",
/// "open_port","advert","session_now_ms","last_midi_ms","peers","found"}|null}`. `midi`/`rtp`
/// are null when started without them; `changes` counts applied changes (redraw when it
/// moves). `rtp.port` is the setting, `rtp.open_port` the port the session opened (0 when
/// closed): a pair up from `port` when another session held that one.
/// NULL on a bad handle. Free with ts_string_free.
#[no_mangle]
pub unsafe extern "C" fn ts_status_json(rt: *mut TsRuntime) -> *mut c_char {
    guard(std::ptr::null_mut(), || runtime(rt).map_or(std::ptr::null_mut(), |r| give(json::status(r))))
}

/// The Tunes as of the last scan or change, as a JSON array of `{"track_type",
/// "track_index","slot","label","name","where","key","key_name","scale","scale_name",
/// "receive","layout_ok","why"}` (`key`/`scale` null when unreadable; `receive` false =
/// Lock). NULL on a bad handle. Free with ts_string_free.
#[no_mangle]
pub unsafe extern "C" fn ts_tunes_json(rt: *mut TsRuntime) -> *mut c_char {
    guard(std::ptr::null_mut(), || runtime(rt).map_or(std::ptr::null_mut(), |r| give(json::tunes(r))))
}

/// The log lines added after generation `after` (0 = every line still held), as JSON:
/// `{"generation":N,"reset":bool,"lines":[{"stamp","level":"info"|"ok"|"warn","text"}]}`.
/// Pass `generation` back next time. `reset` = the reader's lines are stale (the log was
/// cleared, or `after` is not from this runtime): drop them, then show `lines`.
/// NULL on a bad handle. Free with ts_string_free.
#[no_mangle]
pub unsafe extern "C" fn ts_log_json(rt: *mut TsRuntime, after: u64) -> *mut c_char {
    guard(std::ptr::null_mut(), || runtime(rt).map_or(std::ptr::null_mut(), |r| give(json::log(&r.log().since(after)))))
}

/// Invites an RTP-MIDI session (one from status "rtp.found", or typed in) and remembers it
/// in the settings, so it is invited again whenever it comes back. `ip` a dotted quad,
/// `port` its control port; `label` how to show it until it answers (NULL = "").
#[no_mangle]
pub unsafe extern "C" fn ts_rtp_connect(rt: *mut TsRuntime, ip: *const c_char, port: u16, label: *const c_char) -> c_int {
    guard(TS_ERR_PANIC, || {
        let (Some(r), Some(ip)) = (runtime(rt), text_arg(ip)) else { return TS_ERR_ARG };
        if !is_ipv4(&ip) || port == 0 || port == u16::MAX {
            return TS_ERR_ARG;
        }
        let label = if label.is_null() { String::new() } else { CStr::from_ptr(label).to_string_lossy().into_owned() };
        r.rtp_connect(&ip, port, &label);
        TS_OK
    })
}

/// Disconnects a session and forgets it.
#[no_mangle]
pub unsafe extern "C" fn ts_rtp_forget(rt: *mut TsRuntime, ip: *const c_char, port: u16) -> c_int {
    guard(TS_ERR_PANIC, || {
        let (Some(r), Some(ip)) = (runtime(rt), text_arg(ip)) else { return TS_ERR_ARG };
        r.rtp_forget(&ip, port);
        TS_OK
    })
}

/// Frees a string a ts_* function returned. NULL is a no-op.
#[no_mangle]
pub unsafe extern "C" fn ts_string_free(s: *mut c_char) {
    if !s.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(CString::from_raw(s))));
    }
}
