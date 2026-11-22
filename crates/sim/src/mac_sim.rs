//! lv1sim on macOS: loads TuneSync_Control.bundle the way eMotion LV1 for Mac does
//! (CFBundleCreate, CFBundleLoadExecutableAndReturnError, CFBundleGetFunctionPointerForName)
//! with the fake MixerCoreSG (libmixercoresg.dylib, loaded RTLD_GLOBAL first, as LV1's
//! own framework is) and drives it end to end:
//!
//! - the Windows checks: exports, version, info, Create, the first scan, an RTP-MIDI rig
//!   sending CC 20 = 4 + Note D, Lock untouched, Touch/Release pairs, another channel
//!   ignored, GetPreset;
//! - local MIDI through CoreMIDI: a virtual source "lv1sim MIDI" (TuneSync's MIDI in by
//!   name) sends CC 20 = 2 + Note E; then it is unplugged and plugged in again, and the
//!   next cue (Note G) still lands: the port is reopened by name;
//! - `--window <s>`: ShowConfigWindow from the main thread (as LV1 calls it), the window is
//!   found by its title, and with `--shot <prefix>` the page and the RTP-MIDI sheet (a tap
//!   on its tile) are rendered to PNG from the window's own view (no Screen Recording
//!   permission needed).
//!
//! The main thread runs AppKit's event loop (TuneSync's window and its CoreMIDI client
//! live on the main queue, as inside LV1); the checks run on a second thread. With
//! `--headless` (no window session, e.g. plain ssh) it runs the main run loop without
//! AppKit, and `--window` is ignored.
use std::ffi::{c_char, c_void, CString};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lv1_surface::abi::{ControllerInfo, CoreCallbacks, HostFn, CALLBACK_EXPORTS};
use rtpmidi::session::{PeerState, Session, SessionConfig};

type Id = *mut c_void;
type Sel = *const c_void;
type CFRef = *const c_void;

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(p: *mut c_void);
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(alloc: CFRef, buf: *const u8, len: isize, is_dir: bool) -> CFRef;
    fn CFBundleCreate(alloc: CFRef, url: CFRef) -> CFRef;
    fn CFBundleLoadExecutableAndReturnError(b: CFRef, err: *mut CFRef) -> bool;
    fn CFBundleGetFunctionPointerForName(b: CFRef, name: CFRef) -> *const c_void;
    fn CFStringCreateWithBytes(alloc: CFRef, bytes: *const u8, n: isize, enc: u32, bom: bool) -> CFRef;
    fn CFRelease(cf: CFRef);
    fn CFRunLoopRunInMode(mode: CFRef, seconds: f64, return_after_source: bool) -> i32;
    static kCFRunLoopDefaultMode: CFRef;
}

#[link(name = "CoreMIDI", kind = "framework")]
extern "C" {
    fn MIDIClientCreate(name: CFRef, notify: *const c_void, refcon: *mut c_void, out: *mut u32) -> i32;
    fn MIDISourceCreate(client: u32, name: CFRef, out: *mut u32) -> i32;
    fn MIDIReceived(src: u32, list: *const u8) -> i32;
    fn MIDIEndpointDispose(e: u32) -> i32;
}

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlerror() -> *const c_char;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    static _dispatch_main_q: u8;
    fn dispatch_async_f(q: *const c_void, ctx: *mut c_void, f: extern "C" fn(*mut c_void));
}

const RTLD_NOW: i32 = 2;
const RTLD_GLOBAL: i32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NSRect {
    o: NSPoint,
    w: f64,
    h: f64,
}

macro_rules! msg {
    ($ret:ty; $obj:expr, $sel:literal $(, $arg:expr => $t:ty)* $(,)?) => {{
        let f: unsafe extern "C" fn(Id, Sel $(, $t)*) -> $ret = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f($obj as Id, sel_registerName(concat!($sel, "\0").as_ptr() as *const c_char) $(, $arg)*)
    }};
}

unsafe fn cls(name: &std::ffi::CStr) -> Id {
    objc_getClass(name.as_ptr())
}

unsafe fn rect_of(obj: Id, sel: &std::ffi::CStr) -> NSRect {
    #[cfg(target_arch = "x86_64")]
    let f: unsafe extern "C" fn(Id, Sel) -> NSRect = std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
    #[cfg(not(target_arch = "x86_64"))]
    let f: unsafe extern "C" fn(Id, Sel) -> NSRect = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel_registerName(sel.as_ptr()))
}

/// A CFString (never freed: test code).
fn cfstr(s: &str) -> CFRef {
    unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, 0x0800_0100, false) }
}

// ---------------------------------------------------------------- the main thread

type Job = Box<dyn FnOnce() + Send>;

extern "C" fn run_job(ctx: *mut c_void) {
    let job = unsafe { Box::from_raw(ctx as *mut Job) };
    let p = unsafe { objc_autoreleasePoolPush() };
    job();
    unsafe { objc_autoreleasePoolPop(p) };
}

/// Runs `f` on the main thread and waits for it (the checks thread only).
fn on_main<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let job: Box<Job> = Box::new(Box::new(move || {
        let _ = tx.send(f());
    }));
    unsafe { dispatch_async_f(&_dispatch_main_q as *const u8 as *const c_void, Box::into_raw(job) as *mut c_void, run_job) };
    rx.recv_timeout(Duration::from_secs(10)).expect("the main thread did not answer in 10 s")
}

/// AppKit's loop (or the bare run loop with --headless), until the checks thread is done.
pub fn run() -> i32 {
    let headless = std::env::args().any(|a| a == "--headless");
    unsafe {
        let (tx, rx) = mpsc::channel();
        if headless {
            std::thread::spawn(move || {
                let rc = std::panic::catch_unwind(checks).unwrap_or(1);
                let _ = tx.send(rc);
            });
            loop {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.02, false);
                if let Ok(rc) = rx.try_recv() {
                    return rc;
                }
            }
        }
        let app: Id = msg![Id; cls(c"NSApplication"), "sharedApplication"];
        msg![i8; app, "setActivationPolicy:", 1isize => isize]; // accessory: no Dock icon
        msg![(); app, "finishLaunching"];
        std::thread::spawn(move || {
            let rc = std::panic::catch_unwind(checks).unwrap_or(1);
            let _ = tx.send(rc);
        });
        loop {
            let pool = objc_autoreleasePoolPush();
            let until: Id = msg![Id; cls(c"NSDate"), "dateWithTimeIntervalSinceNow:", 0.02f64 => f64];
            let ev: Id = msg![Id; app, "nextEventMatchingMask:untilDate:inMode:dequeue:", u64::MAX => u64, until => Id, kCFRunLoopDefaultMode as Id => Id, true => bool];
            if !ev.is_null() {
                msg![(); app, "sendEvent:", ev => Id];
            }
            objc_autoreleasePoolPop(pool);
            if let Ok(rc) = rx.try_recv() {
                return rc;
            }
        }
    }
}

// ---------------------------------------------------------------- the checks

struct Checks(u32, u32);
impl Checks {
    fn check(&mut self, ok: bool, what: &str) {
        if ok {
            self.0 += 1;
            println!("ok    {what}");
        } else {
            self.1 += 1;
            println!("FAIL  {what}");
        }
    }
}

fn wait_for(ms: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn arg(name: &str) -> Option<String> {
    std::env::args().skip_while(|a| a != name).nth(1)
}

/// A one-packet MIDIPacketList (CoreMIDI's layout, 4-byte packed).
fn packet_list(bytes: &[u8]) -> Vec<u32> {
    let mut b: Vec<u8> = 1u32.to_ne_bytes().to_vec();
    b.extend_from_slice(&0u64.to_ne_bytes());
    b.extend_from_slice(&(bytes.len() as u16).to_ne_bytes());
    b.extend_from_slice(bytes);
    let mut w = vec![0u32; b.len().div_ceil(4) + 1];
    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), w.as_mut_ptr() as *mut u8, b.len()) };
    w
}

/// TuneSync's window: the visible one whose title starts with "TuneSync" (main thread).
unsafe fn find_window() -> Id {
    let app: Id = msg![Id; cls(c"NSApplication"), "sharedApplication"];
    let wins: Id = msg![Id; app, "windows"];
    let n: usize = msg![usize; wins, "count"];
    for i in 0..n {
        let w: Id = msg![Id; wins, "objectAtIndex:", i => usize];
        let t: Id = msg![Id; w, "title"];
        let s: *const c_char = msg![*const c_char; t, "UTF8String"];
        if !s.is_null() && std::ffi::CStr::from_ptr(s).to_string_lossy().starts_with("TuneSync") && msg![i8; w, "isVisible"] != 0 {
            return w;
        }
    }
    std::ptr::null_mut()
}

/// The window's content as a PNG, drawn by its own view (main thread).
unsafe fn render(win: Id, path: &str) -> bool {
    let view: Id = msg![Id; win, "contentView"];
    let b = rect_of(view, c"bounds");
    let rep: Id = msg![Id; view, "bitmapImageRepForCachingDisplayInRect:", b => NSRect];
    if rep.is_null() {
        return false;
    }
    msg![(); view, "cacheDisplayInRect:toBitmapImageRep:", b => NSRect, rep => Id];
    let props: Id = msg![Id; cls(c"NSDictionary"), "dictionary"];
    let png: Id = msg![Id; rep, "representationUsingType:properties:", 4usize => usize, props => Id]; // NSBitmapImageFileTypePNG
    !png.is_null() && msg![i8; png, "writeToFile:atomically:", cfstr(path) as Id => Id, true => bool] != 0
}

/// A tap at (x, y) in the view's flipped points (main thread).
unsafe fn tap(win: Id, x: f64, y: f64) {
    let view: Id = msg![Id; win, "contentView"];
    let h = rect_of(view, c"bounds").h;
    let num: isize = msg![isize; win, "windowNumber"];
    for ty in [1usize, 2usize] {
        // NSEventTypeLeftMouseDown, LeftMouseUp; window coordinates are y-up
        let ev: Id = msg![Id; cls(c"NSEvent"), "mouseEventWithType:location:modifierFlags:timestamp:windowNumber:context:eventNumber:clickCount:pressure:",
            ty => usize, NSPoint { x, y: h - y } => NSPoint, 0usize => usize, 0.0f64 => f64, num => isize, std::ptr::null_mut::<c_void>() => Id, 0isize => isize, 1isize => isize, 1.0f32 => f32];
        msg![(); win, "sendEvent:", ev => Id];
    }
}

/// Escape to the window's first responder (main thread).
unsafe fn escape(win: Id) {
    let num: isize = msg![isize; win, "windowNumber"];
    let esc = cfstr("\u{1b}") as Id;
    let ev: Id = msg![Id; cls(c"NSEvent"), "keyEventWithType:location:modifierFlags:timestamp:windowNumber:context:characters:charactersIgnoringModifiers:isARepeat:keyCode:",
        10usize => usize, NSPoint::default() => NSPoint, 0usize => usize, 0.0f64 => f64, num => isize, std::ptr::null_mut::<c_void>() => Id, esc => Id, esc => Id, false => bool, 53u16 => u16];
    msg![(); win, "sendEvent:", ev => Id];
}

fn checks() -> i32 {
    let headless = std::env::args().any(|a| a == "--headless");
    let window_secs: u64 = if headless { 0 } else { arg("--window").and_then(|v| v.parse().ok()).unwrap_or(0) };
    let shot: Option<String> = arg("--shot");
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    let bundle = arg("--bundle").map(std::path::PathBuf::from).unwrap_or_else(|| exe_dir.join("TuneSync_Control.bundle"));
    let core_path = arg("--core").map(std::path::PathBuf::from).unwrap_or_else(|| exe_dir.join("libmixercoresg.dylib"));
    let mut c = Checks(0, 0);
    let dir = std::env::temp_dir().join(format!("tunesync-sim-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("TUNESYNC_DIR", &dir);
    const PORT: u16 = 47004;
    const SRC: &str = "lv1sim MIDI";
    std::fs::write(dir.join("settings.ini"), format!("[tune-remote]\nrtp_enabled=1\nrtp_port={PORT}\nchannel=16\nscale_cc=20\nmidi_port={SRC}\n")).unwrap();
    println!("lv1sim: data folder {}", dir.display());
    println!("lv1sim: bundle {}", bundle.display());

    // A virtual CoreMIDI source, made on the main thread (it has the run loop).
    let midi = on_main(|| unsafe {
        let mut client = 0u32;
        let mut src = 0u32;
        let r1 = MIDIClientCreate(cfstr("lv1sim"), std::ptr::null(), std::ptr::null_mut(), &mut client);
        let r2 = MIDISourceCreate(client, cfstr(SRC), &mut src);
        (r1 == 0 && r2 == 0).then_some((client, src))
    });
    c.check(midi.is_some(), "a virtual CoreMIDI source \"lv1sim MIDI\"");
    let (client, mut src) = midi.unwrap_or((0, 0));
    let send_midi = |src: u32, bytes: &[u8]| {
        let pl = packet_list(bytes);
        unsafe { MIDIReceived(src, pl.as_ptr() as *const u8) == 0 }
    };

    unsafe {
        // LV1 has MixerCoreSG loaded before any driver.
        let cp = CString::new(core_path.to_string_lossy().as_bytes()).unwrap();
        let core = dlopen(cp.as_ptr(), RTLD_NOW | RTLD_GLOBAL);
        if core.is_null() {
            println!("FAIL  the fake MixerCoreSG: {}", std::ffi::CStr::from_ptr(dlerror()).to_string_lossy());
            return 1;
        }
        let csym = |n: &str| {
            let cn = CString::new(n).unwrap();
            let p = dlsym(core, cn.as_ptr());
            assert!(!p.is_null(), "{n} is not exported by the fake core");
            p
        };

        // The bundle, as LV1 loads it.
        let bp = bundle.to_string_lossy().into_owned();
        let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bp.as_ptr(), bp.len() as isize, true);
        let b = CFBundleCreate(std::ptr::null(), url);
        let mut err: CFRef = std::ptr::null();
        let loaded = !b.is_null() && CFBundleLoadExecutableAndReturnError(b, &mut err);
        c.check(loaded, "CFBundle loaded TuneSync_Control.bundle");
        if !loaded {
            if !err.is_null() {
                let d: Id = msg![Id; err as Id, "description"];
                let s: *const c_char = msg![*const c_char; d, "UTF8String"];
                println!("      {}", std::ffi::CStr::from_ptr(s).to_string_lossy());
                CFRelease(err);
            }
            return 1;
        }
        let names = [
            "WMSD_QueryInterfaceVersion", "WMSD_GetAvailableControllerInfo", "WMSD_TestControllerForType", "WMSD_CreateControllerForType",
            "WMSD_InitControllerForType", "WMSD_GetTypeForController", "WMSD_DestroyController", "WMSD_ShowConfigWindow", "WMSD_LockConfigWindow",
            "WMSD_UnlockConfigWindow", "WMSD_IdentifyController", "WMSD_GetPreset", "WMSD_SetPreset", "WMSD_CommandStatusUpdate",
            "WMSD_MixerParamWasChanged", "WMSD_MixerAssignmentsChanged", "WMSD_ControllerDisplayUpdate", "WMSD_MixerMessageSent",
        ];
        let f = |n: &str| CFBundleGetFunctionPointerForName(b, cfstr(n));
        let missing: Vec<&str> = names.iter().copied().filter(|n| f(n).is_null()).collect();
        c.check(missing.is_empty(), &format!("all 18 WMSD_* exports resolve through CFBundle{}", if missing.is_empty() { String::new() } else { format!(" (missing {missing:?})") }));
        if !missing.is_empty() {
            return 1;
        }
        type V = unsafe extern "C" fn() -> i32;
        type Info = unsafe extern "C" fn(*mut ControllerInfo) -> i32;
        type Create = unsafe extern "C" fn(*const u8, *mut c_void, *const CoreCallbacks, *mut c_void, *const c_void, *mut bool) -> *mut c_void;
        type Ctrl = unsafe extern "C" fn(*mut c_void) -> i32;
        type Destroy = unsafe extern "C" fn(*mut c_void, bool) -> i32;
        type Preset = unsafe extern "C" fn(*mut c_void, *mut u8, u32, *mut u32, *mut u8, u32) -> i32;
        let version: V = std::mem::transmute(f("WMSD_QueryInterfaceVersion"));
        c.check(version() == 6, "interface version 6");
        let mut info: ControllerInfo = std::mem::zeroed();
        let get_info: Info = std::mem::transmute(f("WMSD_GetAvailableControllerInfo"));
        get_info(&mut info);
        let cs = |a: &[u8]| String::from_utf8_lossy(&a[..a.iter().position(|&b| b == 0).unwrap_or(0)]).to_string();
        let (name, ty) = (cs(&info.name), cs(&info.type_));
        c.check(name == "TuneSync" && ty == "TuneSync", &format!("controller info: name \"{name}\", type \"{ty}\""));

        let ptrs: Vec<Option<HostFn>> = CALLBACK_EXPORTS.iter().map(|n| Some(std::mem::transmute::<*mut c_void, HostFn>(csym(n)))).collect();
        let cb = CoreCallbacks {
            set_mixer_control: ptrs[0],
            get_mixer_control: ptrs[1],
            set_mixer_assignments: ptrs[2],
            get_mixer_assignments: ptrs[3],
            set_mixer_param: ptrs[4],
            get_mixer_param: ptrs[5],
            send_mixer_command: ptrs[6],
        };
        let reset: unsafe extern "C" fn() = std::mem::transmute(csym("fakecore_reset"));
        let param: unsafe extern "C" fn(u32, u32, u32, u32) -> f64 = std::mem::transmute(csym("fakecore_param"));
        let counts: unsafe extern "C" fn(*mut u64) = std::mem::transmute(csym("fakecore_counts"));
        let count = || {
            let mut v = [0u64; 4];
            counts(v.as_mut_ptr());
            v
        };
        reset();

        let create: Create = std::mem::transmute(f("WMSD_CreateControllerForType"));
        let core_obj = Box::leak(Box::new([0u8; 64])); // LV1's core: opaque, handed back to the callbacks
        let t0 = Instant::now();
        let ctrl = create(c"LV1".as_ptr() as *const u8, core_obj.as_mut_ptr() as *mut c_void, &cb, std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut());
        c.check(!ctrl.is_null(), &format!("Create returned a controller in {} ms", t0.elapsed().as_millis()));
        c.check(t0.elapsed() < Duration::from_millis(200), "Create did not block LV1's thread");
        let ctrl_addr = ctrl as usize;
        c.check(wait_for(5000, || count()[0] > 160 * 8), &format!("the first scan ran ({} reads)", count()[0]));

        // RTP-MIDI: a playback rig joins and sends the cue.
        let rig = Session::new();
        rig.open(SessionConfig { name: "lv1sim rig".into(), port: PORT + 100, ..Default::default() }).unwrap();
        rig.invite("127.0.0.1", PORT, "");
        let joined = wait_for(5000, || {
            rig.poll(5);
            rig.peers().iter().any(|p| p.state == PeerState::Connected)
        });
        c.check(joined, "a rig joined TuneSync's RTP-MIDI session");
        let t1 = Instant::now();
        rig.send(&[&[0xBF, 20, 4], &[0x9F, 62, 100]]);
        let applied = wait_for(3000, || {
            rig.poll(5);
            param(0, 42, 0, 40) == 3.0 && param(0, 43, 2, 40) == 3.0
        });
        c.check(applied, &format!("RTP: CC 20 = 4 + Note On D: Ch 43 and Ch 44 -> D in {} ms", t1.elapsed().as_millis()));
        c.check(param(0, 42, 0, 39) == 4.0 && param(0, 43, 2, 39) == 4.0, "both on Harmonic Minor");
        c.check(param(0, 44, 0, 40) == 8.0 && param(0, 44, 0, 39) == 2.0, "the Lock Tune (Ch 45) untouched");
        wait_for(500, || false);
        let n = count();
        c.check(n[1] == 4 && n[2] == n[3] && n[2] == 4, &format!("4 Controls, every Touch released (touch {} release {})", n[2], n[3]));
        rig.send(&[&[0x90, 64, 100]]);
        wait_for(300, || {
            rig.poll(5);
            false
        });
        c.check(param(0, 42, 0, 40) == 3.0, "a note on channel 1 is ignored");

        // CoreMIDI: the virtual source, by name.
        let log_has = |s: &str| std::fs::read_to_string(dir.join("tunesync.log")).unwrap_or_default().matches(s).count();
        c.check(wait_for(6000, || log_has("MIDI in: listening on lv1sim MIDI") >= 1), "CoreMIDI: TuneSync opened \"lv1sim MIDI\" by name");
        let t2 = Instant::now();
        send_midi(src, &[0xBF, 20, 2, 0x9F, 64, 100]);
        send_midi(src, &[0x8F, 64, 0]);
        c.check(
            wait_for(3000, || param(0, 42, 0, 40) == 6.0 && param(0, 43, 2, 40) == 6.0 && param(0, 42, 0, 39) == 2.0),
            &format!("CoreMIDI: CC 20 = 2 + Note On E -> E Major in {} ms", t2.elapsed().as_millis()),
        );
        // unplugged, then back: reopened by name
        on_main(move || MIDIEndpointDispose(src));
        c.check(wait_for(6000, || log_has("lv1sim MIDI went away") >= 1), "CoreMIDI: the source going away is noticed");
        src = on_main(move || {
            let mut s = 0u32;
            MIDISourceCreate(client, cfstr(SRC), &mut s);
            s
        });
        c.check(wait_for(6000, || log_has("MIDI in: listening on lv1sim MIDI") >= 2), "CoreMIDI: and reopened when it is back");
        send_midi(src, &[0x9F, 67, 100]);
        c.check(wait_for(3000, || param(0, 42, 0, 40) == 10.0), "CoreMIDI: after the replug Note On G -> G");

        let preset: Preset = std::mem::transmute(f("WMSD_GetPreset"));
        let mut size = 0u32;
        preset(ctrl, std::ptr::null_mut(), 0, &mut size, std::ptr::null_mut(), 0);
        c.check(size == 8, "GetPreset size query");

        if window_secs > 0 {
            let show: Ctrl = std::mem::transmute(f("WMSD_ShowConfigWindow"));
            let show_addr = show as usize;
            // LV1 calls it on its main thread
            let took = on_main(move || {
                let show: Ctrl = std::mem::transmute(show_addr);
                let t = Instant::now();
                show(ctrl_addr as *mut c_void);
                t.elapsed()
            });
            c.check(took < Duration::from_millis(100), &format!("ShowConfigWindow returned at once ({} ms)", took.as_millis()));
            wait_for(2500, || {
                rig.poll(20);
                false
            });
            let found = on_main(|| find_window() as usize);
            c.check(found != 0, "the settings window is shown");
            if let (Some(prefix), true) = (&shot, found != 0) {
                let (p1, p2, p3) = (format!("{prefix}-page.png"), format!("{prefix}-rtp.png"), format!("{prefix}-back.png"));
                c.check(on_main(move || render(found as Id, &p1)), "PNG of the page");
                on_main(move || tap(found as Id, 360.0, 40.0)); // the RTP-MIDI tile
                wait_for(800, || false);
                c.check(on_main(move || render(found as Id, &p2)), "PNG of the RTP-MIDI sheet (a tap on its tile)");
                on_main(move || escape(found as Id));
                wait_for(500, || false);
                c.check(on_main(move || render(found as Id, &p3)), "PNG after Escape (the page again)");
            }
            wait_for(window_secs.saturating_sub(4) * 1000, || {
                rig.poll(20);
                false
            });
        }

        rig.close();
        let destroy: Destroy = std::mem::transmute(f("WMSD_DestroyController"));
        let destroy_addr = destroy as usize;
        // LV1 destroys surfaces on its main thread too
        let took = on_main(move || {
            let destroy: Destroy = std::mem::transmute(destroy_addr);
            let t = Instant::now();
            destroy(ctrl_addr as *mut c_void, false);
            t.elapsed()
        });
        c.check(took < Duration::from_millis(3000), &format!("Destroy finished in {} ms", took.as_millis()));
        if window_secs > 0 {
            wait_for(300, || false);
            c.check(on_main(|| find_window().is_null()), "the window is hidden after Destroy");
        }
    }
    let log = std::fs::read_to_string(dir.join("tunesync.log")).unwrap_or_default();
    println!("---- tunesync.log\n{log}----");
    c.check(log.contains("D Harmonic Minor: set 2") && log.contains("skipped 1 locked"), "the RTP change is in the log");
    c.check(log.contains("E Major: set 2"), "the CoreMIDI change is in the log");
    println!("lv1sim: {} passed, {} failed", c.0, c.1);
    if c.1 == 0 {
        0
    } else {
        1
    }
}
