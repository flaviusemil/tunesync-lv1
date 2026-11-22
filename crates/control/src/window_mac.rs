//! The settings window on macOS: an AppKit window whose view paints the portable view
//! model's frames (crates/ui) with CoreGraphics and CoreText ([`crate::paint_mac`]).
//!
//! AppKit only runs on the main thread, which inside LV1 is LV1's own UI thread. So:
//! - every AppKit call is a job on the main queue ([`main_async`]); LV1's ShowConfigWindow
//!   and Destroy only post and return, and nothing on the main thread ever waits;
//! - the main thread does no TuneSync work either: a feeder thread builds the window's
//!   [`Model`] (four times a second while the window is shown) and runs the window's
//!   commands (settings, Apply now, Connect), then posts the new model to the main queue.
//!   Building a model takes the runtime's locks, which an RTP-MIDI restart can hold for a
//!   second; that wait stays on the feeder.
//!
//! One NSWindow per process, made on first show and kept (hidden) when LV1 destroys the
//! surface: a surface made again (LV1 loading a session) shows the same window with the
//! new runtime. The window works while LV1 runs a modal dialog (Setup), opens at the
//! level of LV1's key window so it is never behind it, joins a full-screen LV1's space,
//! takes the first click even when inactive (touch-first), and closing it only hides it.
//! When it had to float to clear LV1's Setup, it hides while LV1 is in the background.
//!
//! Every AppKit handler is guarded: a panic is caught there (it must never unwind into
//! AppKit and so abort LV1), and a handler that AppKit calls back from inside one of our
//! own AppKit calls (makeFirstResponder ending an edit, makeKeyAndOrderFront drawing at
//! once) returns without touching the window's state, which its caller holds.
use std::collections::VecDeque;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tunesync::tune::Spelling;
use tunesync_host::mac::{is_main_thread, main_async, to_string, CfString};
use tunesync_host::sys::Worker;
use tunesync_host::Runtime;
use tunesync_ui::draw::theme;
use tunesync_ui::model::Model;
use tunesync_ui::view::{layout, Command, Frame, View, DEFAULT_HEIGHT, MIN_HEIGHT, WIDTH};

use crate::objc::{class, msg, msg_rect, sel, subclass, with_pool, Id, NSPoint, NSRect, NSSize, Sel, BOOL_ENC, NIL};
use crate::paint_mac::{ns_font, Painter};

pub type Problem = Arc<dyn Fn() -> Option<String> + Send + Sync>;

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSPasteboardTypeString: Id;
    static NSAppearanceNameDarkAqua: Id;
}

const KEY_ESCAPE: u16 = 53;
const STYLE: usize = 1 | 2 | 4 | 8; // titled, closable, miniaturizable, resizable
const BACKING_BUFFERED: usize = 2;
const FULLSCREEN_AUXILIARY: usize = 1 << 8;
const MOVE_TO_ACTIVE_SPACE: usize = 1 << 1;
const FOCUS_RING_NONE: usize = 1;

// ---------------------------------------------------------------- the driver's side

/// What the feeder thread and the main thread share.
struct Shared {
    rt: Arc<Runtime>,
    problem: Problem,
    run: AtomicBool,
    q: Mutex<Queue>,
    cv: Condvar,
}

#[derive(Default)]
struct Queue {
    cmds: VecDeque<Command>,
    visible: bool,
    kick: bool,
}

impl Shared {
    fn push(&self, f: impl FnOnce(&mut Queue)) {
        f(&mut self.q.lock().unwrap_or_else(|e| e.into_inner()));
        self.cv.notify_all();
    }
}

/// The window, from the driver's side.
pub struct Window {
    sh: Arc<Shared>,
    feeder: Mutex<Option<Worker>>,
}

impl Window {
    pub fn new(rt: Arc<Runtime>, problem: Problem) -> Self {
        let sh = Arc::new(Shared { rt, problem, run: AtomicBool::new(true), q: Mutex::new(Queue::default()), cv: Condvar::new() });
        let s2 = sh.clone();
        let feeder = Worker::spawn("tunesync-window", move || feed(s2));
        Window { sh, feeder: Mutex::new(Some(feeder)) }
    }

    /// From LV1 (any thread): show the window, later, on the main queue. Returns at once.
    pub fn show(&self) {
        let sh = self.sh.clone();
        main_async(move || with_pool(|| show_on_main(sh)));
    }

    /// LV1 destroys the surface: hide the window and end the feeder (bounded, short). On
    /// the main thread (where LV1 calls it) a session name being typed is committed first,
    /// and the feeder carries it out before it ends.
    pub fn stop(&self) {
        let sh = self.sh.clone();
        if is_main_thread() {
            with_pool(|| detach_on_main(&sh));
            self.sh.run.store(false, Ordering::SeqCst);
        } else {
            self.sh.run.store(false, Ordering::SeqCst);
            main_async(move || with_pool(|| detach_on_main(&sh)));
        }
        self.sh.cv.notify_all();
        if let Some(mut w) = self.feeder.lock().unwrap_or_else(|e| e.into_inner()).take() {
            w.join_for(Duration::from_millis(500));
        }
    }
}

/// The feeder: runs the window's commands and builds its model, off the main thread.
fn feed(sh: Arc<Shared>) {
    let ips = crate::model::local_ips();
    let mut last: Option<Model> = None;
    while sh.run.load(Ordering::SeqCst) {
        let (cmds, visible) = {
            let g = sh.q.lock().unwrap_or_else(|e| e.into_inner());
            let mut g = sh.cv.wait_timeout_while(g, Duration::from_millis(250), |q| sh.run.load(Ordering::SeqCst) && q.cmds.is_empty() && !q.kick).unwrap_or_else(|e| e.into_inner()).0;
            g.kick = false;
            (std::mem::take(&mut g.cmds), g.visible)
        };
        let forced = !cmds.is_empty();
        for c in cmds {
            run_command(&sh.rt, c);
        }
        if !visible {
            last = None;
            continue;
        }
        let m = crate::model::build(&sh.rt, (sh.problem)(), &ips);
        if forced || last.as_ref() != Some(&m) {
            last = Some(m.clone());
            post_model(sh.clone(), m);
        }
    }
    // The settings the window still had queued (a name committed at Destroy).
    let rest = std::mem::take(&mut sh.q.lock().unwrap_or_else(|e| e.into_inner()).cmds);
    for c in rest {
        if !matches!(c, Command::ApplyNow(_) | Command::Rescan) {
            run_command(&sh.rt, c);
        }
    }
}

/// Hands a new model to the window on the main queue (again later if the window's state
/// is held just then).
fn post_model(sh: Arc<Shared>, m: Model) {
    main_async(move || {
        with_pool(|| {
            if BUSY.load(Ordering::SeqCst) {
                post_model(sh, m);
                return;
            }
            guarded((), || {
                if let Some(ui) = ui_for(&sh) {
                    ui.set_model(m);
                }
            })
        })
    });
}

fn run_command(rt: &Runtime, c: Command) {
    match c {
        Command::SetMidiPort(p) => rt.update(|s| s.midi_port = p),
        Command::SetRtpEnabled(b) => rt.update(|s| s.rtp_enabled = b),
        Command::SetRtpName(n) => rt.update(|s| s.rtp_name = n),
        Command::SetRtpPort(p) => rt.update(|s| s.rtp_port = p),
        Command::SetChannel(ch) => rt.update(|s| s.channel = ch),
        Command::SetScaleCc(cc) => rt.update(|s| s.scale_cc = cc),
        Command::SetSpelling(sp) => rt.update(|s| s.spelling = if sp == Spelling::Flat { Spelling::Flat } else { Spelling::Sharp }),
        Command::ApplyNow(i) => rt.apply_now(i),
        Command::Rescan => rt.rescan(),
        Command::ClearLog => rt.log().clear(),
        Command::Connect { ip, port, name } => rt.rtp_connect(&ip, port, &name),
        Command::Forget { ip, port } => rt.rtp_forget(&ip, port),
        Command::CopyLog => {} // done on the main thread (the pasteboard)
    }
}

// ---------------------------------------------------------------- the main thread

/// The window's state, main thread only.
struct Ui {
    sh: Arc<Shared>,
    win: Id,
    view: Id,
    field: Id,
    model: Model,
    state: View,
    frame: Frame,
    size: (f32, f32),
    painter: Painter,
    field_shown: bool,
    field_rect: NSRect,
}

/// The one Ui (main thread only; null when no surface owns the window).
static UI: AtomicPtr<Ui> = AtomicPtr::new(std::ptr::null_mut());
/// Set while main-thread code of ours holds the Ui (see the module notes).
static BUSY: AtomicBool = AtomicBool::new(false);

struct Held;

impl Drop for Held {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

/// Runs `f` holding the Ui: `fallback` when it is already held (a nested call from AppKit)
/// or when `f` panics. Every entry from AppKit and every main-queue job goes through here.
fn guarded<T: Copy>(fallback: T, f: impl FnOnce() -> T) -> T {
    if BUSY.swap(true, Ordering::SeqCst) {
        return fallback;
    }
    let _held = Held;
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}
/// The NSWindow, its view and the session-name field, made once (main thread only).
static NATIVE: Mutex<Option<(usize, usize, usize)>> = Mutex::new(None);

fn ui() -> Option<&'static mut Ui> {
    unsafe { UI.load(Ordering::SeqCst).as_mut() }
}

/// The Ui, if it belongs to `sh` (a job posted by an older surface finds nothing).
fn ui_for(sh: &Arc<Shared>) -> Option<&'static mut Ui> {
    ui().filter(|u| Arc::ptr_eq(&u.sh, sh))
}

/// The Ui an AppKit callback is for: ours and still attached.
fn ui_of(view: Id) -> Option<&'static mut Ui> {
    ui().filter(|u| u.view == view)
}

fn show_on_main(sh: Arc<Shared>) {
    guarded((), || show_held(sh))
}

fn show_held(sh: Arc<Shared>) {
    if !sh.run.load(Ordering::SeqCst) {
        return;
    }
    if ui_for(&sh).is_none() {
        let Some((win, view, field)) = native() else { return };
        let old = UI.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !old.is_null() {
            drop(unsafe { Box::from_raw(old) });
        }
        let ui = Box::new(Ui {
            sh: sh.clone(),
            win,
            view,
            field,
            model: Model::default(),
            state: View::new(),
            frame: Frame::default(),
            size: (WIDTH, DEFAULT_HEIGHT),
            painter: Painter::default(),
            field_shown: false,
            field_rect: NSRect::default(),
        });
        UI.store(Box::into_raw(ui), Ordering::SeqCst);
    }
    let Some(ui) = ui_for(&sh) else { return };
    ui.commit_field();
    sh.push(|q| {
        q.visible = true;
        q.kick = true;
        q.cmds.push_back(Command::Rescan);
    });
    unsafe {
        // At the level of LV1's key window (a floating Setup must not cover us); a window
        // that floats hides while LV1 is in the background, so it never covers other apps.
        let app: Id = msg![Id; class(c"NSApplication"), "sharedApplication"];
        let key: Id = msg![Id; app, "keyWindow"];
        let level: isize = if key.is_null() || key == ui.win { 0 } else { msg![isize; key, "level"] };
        msg![(); ui.win, "setLevel:", level.max(0) => isize];
        msg![(); ui.win, "setHidesOnDeactivate:", level > 0 => bool];
        msg![(); ui.win, "makeKeyAndOrderFront:", NIL => Id];
        msg![(); app, "activateIgnoringOtherApps:", true => bool];
        msg![(); ui.win, "makeFirstResponder:", ui.view => Id];
    }
    ui.relayout();
}

/// The surface is going: hide the window and let go of the runtime.
fn detach_on_main(sh: &Arc<Shared>) {
    guarded((), || detach_held(sh))
}

fn detach_held(sh: &Arc<Shared>) {
    let Some(ui) = ui_for(sh) else { return };
    ui.commit_field();
    unsafe { msg![(); ui.win, "orderOut:", NIL => Id] };
    let p = UI.swap(std::ptr::null_mut(), Ordering::SeqCst);
    if !p.is_null() {
        drop(unsafe { Box::from_raw(p) });
    }
}

impl Ui {
    fn set_model(&mut self, m: Model) {
        self.model = m;
        self.relayout();
    }

    fn view_size(&self) -> (f32, f32) {
        let b = unsafe { msg_rect(self.view, c"bounds") };
        ((b.size.w as f32).max(1.0), (b.size.h as f32).max(1.0))
    }

    /// A new frame from the model and the view state, the text field placed, and a repaint.
    fn relayout(&mut self) {
        self.size = self.view_size();
        self.frame = layout(&self.model, &self.state, self.size.0, self.size.1);
        self.place_field();
        unsafe { msg![(); self.view, "setNeedsDisplay:", true => bool] };
    }

    fn place_field(&mut self) {
        match self.frame.edit.clone() {
            Some((r, text)) => unsafe {
                // a 14 pt line, centred in the frame's box
                let h = 20.0;
                let nr = NSRect::new(r.x as f64 + 4.0, r.y as f64 + (r.h as f64 - h) / 2.0, r.w as f64 - 8.0, h);
                if !self.field_shown {
                    let t = CfString::new(&text);
                    msg![(); self.field, "setStringValue:", t.as_ptr() as Id => Id];
                    msg![(); self.field, "setHidden:", false => bool];
                    self.field_shown = true;
                }
                if nr != self.field_rect {
                    msg![(); self.field, "setFrame:", nr => NSRect];
                    self.field_rect = nr;
                }
            },
            None => {
                if self.field_shown {
                    self.commit_field();
                    unsafe { msg![(); self.field, "setHidden:", true => bool] };
                    self.field_shown = false;
                    self.field_rect = NSRect::default();
                    unsafe { msg![(); self.win, "makeFirstResponder:", self.view => Id] };
                }
            }
        }
    }

    /// The session name typed into the field, applied when editing ends or the sheet closes.
    fn commit_field(&mut self) {
        if !self.field_shown {
            return;
        }
        let t = unsafe { to_string(msg![Id; self.field, "stringValue"] as *const c_void) };
        let t = t.trim().to_string();
        if !t.is_empty() && t.len() <= 63 && t != self.model.settings.rtp_name {
            self.model.settings.rtp_name = t.clone(); // shown at once; the feeder confirms
            self.sh.push(|q| q.cmds.push_back(Command::SetRtpName(t)));
        }
    }

    fn run(&mut self, cmds: Vec<Command>) {
        if cmds.is_empty() {
            return;
        }
        let mut rest = Vec::new();
        for c in cmds {
            if c == Command::CopyLog {
                self.copy_log();
            } else {
                rest.push(c);
            }
        }
        self.sh.push(|q| {
            q.cmds.extend(rest);
            q.kick = true;
        });
    }

    fn copy_log(&self) {
        let text: String = self.model.log.iter().rev().map(|l| format!("{} {}\n", l.stamp, l.text)).collect();
        let s = CfString::new(&text);
        unsafe {
            let pb: Id = msg![Id; class(c"NSPasteboard"), "generalPasteboard"];
            msg![isize; pb, "clearContents"];
            msg![i8; pb, "setString:forType:", s.as_ptr() as Id => Id, NSPasteboardTypeString => Id];
        }
    }

    fn hide(&mut self) {
        self.commit_field();
        self.state.back();
        self.sh.push(|q| q.visible = false);
        unsafe { msg![(); self.win, "orderOut:", NIL => Id] };
        self.relayout();
    }

    fn point(&self, event: Id) -> (f32, f32) {
        unsafe {
            let p: NSPoint = msg![NSPoint; event, "locationInWindow"];
            let v: NSPoint = msg![NSPoint; self.view, "convertPoint:fromView:", p => NSPoint, NIL => Id];
            (v.x as f32, v.y as f32)
        }
    }
}

// ---------------------------------------------------------------- AppKit callbacks

extern "C" fn yes(_this: Id, _cmd: Sel) -> bool {
    true
}

extern "C" fn yes1(_this: Id, _cmd: Sel, _a: Id) -> bool {
    true
}

extern "C" fn draw_rect(this: Id, _cmd: Sel, _dirty: NSRect) {
    // (nested inside our own call: skipped; that call ends with setNeedsDisplay)
    guarded((), || draw_held(this))
}

fn draw_held(this: Id) {
    let Some(ui) = ui_of(this) else {
        // no surface owns the window: just the background
        let mut p = Painter::default();
        unsafe { paint(&mut p, this, &[]) };
        return;
    };
    if ui.view_size() != ui.size {
        ui.relayout();
    }
    let items = std::mem::take(&mut ui.frame.items);
    unsafe { paint(&mut ui.painter, this, &items) };
    ui.frame.items = items;
}

unsafe fn paint(p: &mut Painter, view: Id, items: &[tunesync_ui::draw::Item]) {
    let gc: Id = msg![Id; class(c"NSGraphicsContext"), "currentContext"];
    if gc.is_null() {
        return;
    }
    let ctx: *mut c_void = msg![*mut c_void; gc, "CGContext"];
    let b = msg_rect(view, c"bounds");
    p.paint(ctx, b.size.w, b.size.h, theme::BG, items);
}

/// (NSWindow has already made the view first responder, which ended any edit in the field.)
extern "C" fn mouse_down(this: Id, _cmd: Sel, event: Id) {
    guarded((), || {
        let Some(ui) = ui_of(this) else { return };
        let (x, y) = ui.point(event);
        ui.state.pointer_down(&ui.frame, x, y);
        ui.relayout();
    })
}

extern "C" fn mouse_dragged(this: Id, _cmd: Sel, event: Id) {
    guarded((), || {
        let Some(ui) = ui_of(this) else { return };
        let (x, y) = ui.point(event);
        ui.state.pointer_move(x, y);
        ui.relayout();
    })
}

extern "C" fn mouse_up(this: Id, _cmd: Sel, event: Id) {
    guarded((), || {
        let Some(ui) = ui_of(this) else { return };
        let (x, y) = ui.point(event);
        ui.relayout();
        let m = ui.model.clone();
        let cmds = ui.state.pointer_up(&ui.frame, x, y, &m);
        ui.run(cmds);
        ui.relayout();
    })
}

extern "C" fn scroll_wheel(this: Id, _cmd: Sel, event: Id) {
    guarded((), || {
        let Some(ui) = ui_of(this) else { return };
        let (x, y) = ui.point(event);
        let (dy, precise) = unsafe { (msg![f64; event, "scrollingDeltaY"], msg![i8; event, "hasPreciseScrollingDeltas"] != 0) };
        // a touchpad gives points, a wheel gives lines
        let dy = if precise { -dy } else { -dy * 24.0 };
        ui.state.wheel(&ui.frame, x, y, dy as f32);
        ui.relayout();
    })
}

extern "C" fn key_down(this: Id, _cmd: Sel, event: Id) {
    guarded((), || {
        let Some(ui) = ui_of(this) else { return };
        let code: u16 = unsafe { msg![u16; event, "keyCode"] };
        if code == KEY_ESCAPE {
            ui.commit_field();
            if !ui.state.back() {
                ui.hide();
            }
            ui.relayout();
        }
    })
}

/// The field's Escape (and the window's): `cancelOperation:` comes up the responder chain.
extern "C" fn cancel_operation(this: Id, _cmd: Sel, _sender: Id) {
    guarded((), || {
        let Some(ui) = ui_of(this) else { return };
        ui.commit_field();
        if !ui.state.back() {
            ui.hide();
        }
        ui.relayout();
    })
}

extern "C" fn window_should_close(this: Id, _cmd: Sel, win: Id) -> bool {
    guarded((), || {
        if let Some(ui) = ui_of(this) {
            ui.hide();
        } else {
            unsafe { msg![(); win, "orderOut:", NIL => Id] };
        }
    });
    false // only hidden, never closed
}

extern "C" fn window_did_resize(this: Id, _cmd: Sel, _note: Id) {
    guarded((), || {
        if let Some(ui) = ui_of(this) {
            ui.relayout();
        }
    })
}

/// The session-name field: Return, Tab or a click elsewhere ended the edit.
extern "C" fn field_done(this: Id, _cmd: Sel, _x: Id) {
    guarded((), || {
        if let Some(ui) = ui_of(this) {
            ui.commit_field();
        }
    })
}

/// Our classes, registered once per load of this bundle (the names carry this copy's
/// address, so a second copy of the bundle can never pick up our methods).
fn classes() -> Option<(*mut c_void, *mut c_void)> {
    static MADE: Mutex<Option<(usize, usize)>> = Mutex::new(None);
    let mut g = MADE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((w, v)) = *g {
        return Some((w as *mut c_void, v as *mut c_void));
    }
    let tag = &MADE as *const _ as usize;
    let b = BOOL_ENC;
    let mut w = subclass(c"NSWindow", &format!("TuneSyncWindow_{tag:x}"))?;
    w.method(c"worksWhenModal", yes as *const c_void, &format!("{b}@:"));
    w.method(c"canBecomeKeyWindow", yes as *const c_void, &format!("{b}@:"));
    let wc = w.register();
    let mut v = subclass(c"NSView", &format!("TuneSyncView_{tag:x}"))?;
    v.method(c"isFlipped", yes as *const c_void, &format!("{b}@:"))
        .method(c"acceptsFirstResponder", yes as *const c_void, &format!("{b}@:"))
        .method(c"acceptsFirstMouse:", yes1 as *const c_void, &format!("{b}@:@"))
        .method(c"drawRect:", draw_rect as *const c_void, "v@:{CGRect={CGPoint=dd}{CGSize=dd}}")
        .method(c"mouseDown:", mouse_down as *const c_void, "v@:@")
        .method(c"mouseDragged:", mouse_dragged as *const c_void, "v@:@")
        .method(c"mouseUp:", mouse_up as *const c_void, "v@:@")
        .method(c"scrollWheel:", scroll_wheel as *const c_void, "v@:@")
        .method(c"keyDown:", key_down as *const c_void, "v@:@")
        .method(c"cancelOperation:", cancel_operation as *const c_void, "v@:@")
        .method(c"windowShouldClose:", window_should_close as *const c_void, &format!("{b}@:@"))
        .method(c"windowDidResize:", window_did_resize as *const c_void, "v@:@")
        .method(c"controlTextDidEndEditing:", field_done as *const c_void, "v@:@")
        .method(c"nameEntered:", field_done as *const c_void, "v@:@");
    let vc = v.register();
    *g = Some((wc as usize, vc as usize));
    Some((wc, vc))
}

fn srgb(c: tunesync_ui::draw::Color) -> Id {
    unsafe {
        msg![Id; class(c"NSColor"), "colorWithSRGBRed:green:blue:alpha:",
            c.r() as f64 / 255.0 => f64, c.g() as f64 / 255.0 => f64, c.b() as f64 / 255.0 => f64, c.a() as f64 / 255.0 => f64]
    }
}

/// The NSWindow, its view and the session-name field (made on first use, then kept).
fn native() -> Option<(Id, Id, Id)> {
    let mut g = NATIVE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((w, v, f)) = *g {
        return Some((w as Id, v as Id, f as Id));
    }
    let (wc, vc) = classes()?;
    unsafe {
        // 480 x 700 points, never taller than the screen.
        let screen: Id = msg![Id; class(c"NSScreen"), "mainScreen"];
        let vis = if screen.is_null() { NSRect::new(0.0, 0.0, 1440.0, 900.0) } else { msg_rect(screen, c"visibleFrame") };
        let h = (DEFAULT_HEIGHT as f64).min(vis.size.h - 40.0).max(MIN_HEIGHT as f64);
        let content = NSRect::new(0.0, 0.0, WIDTH as f64, h);
        let win: Id = msg![Id; wc, "alloc"];
        let win: Id = msg![Id; win, "initWithContentRect:styleMask:backing:defer:", content => NSRect, STYLE => usize, BACKING_BUFFERED => usize, false => bool];
        if win.is_null() {
            return None;
        }
        msg![(); win, "setReleasedWhenClosed:", false => bool];
        let title = CfString::new("TuneSync"); // as on Windows: the log's first line has the version
        msg![(); win, "setTitle:", title.as_ptr() as Id => Id];
        msg![(); win, "setContentMinSize:", NSSize { w: WIDTH as f64, h: MIN_HEIGHT as f64 } => NSSize];
        msg![(); win, "setContentMaxSize:", NSSize { w: WIDTH as f64, h: 100_000.0 } => NSSize];
        msg![(); win, "setBackgroundColor:", srgb(theme::BG) => Id];
        let dark: Id = msg![Id; class(c"NSAppearance"), "appearanceNamed:", NSAppearanceNameDarkAqua => Id];
        if !dark.is_null() {
            msg![(); win, "setAppearance:", dark => Id];
        }
        msg![(); win, "setCollectionBehavior:", FULLSCREEN_AUXILIARY | MOVE_TO_ACTIVE_SPACE => usize];
        let view: Id = msg![Id; vc, "alloc"];
        let view: Id = msg![Id; view, "initWithFrame:", content => NSRect];
        msg![(); win, "setContentView:", view => Id];
        msg![(); view, "release"]; // the window holds it
        msg![(); win, "setDelegate:", view => Id];

        let field: Id = msg![Id; class(c"NSTextField"), "alloc"];
        let field: Id = msg![Id; field, "initWithFrame:", NSRect::new(0.0, 0.0, 10.0, 10.0) => NSRect];
        msg![(); field, "setBezeled:", false => bool];
        msg![(); field, "setBordered:", false => bool];
        msg![(); field, "setDrawsBackground:", true => bool];
        msg![(); field, "setBackgroundColor:", srgb(theme::PANEL2) => Id];
        msg![(); field, "setTextColor:", srgb(theme::FG) => Id];
        msg![(); field, "setFont:", ns_font(false, 14.0, 400) => Id];
        msg![(); field, "setFocusRingType:", FOCUS_RING_NONE => usize];
        msg![(); field, "setDelegate:", view => Id];
        msg![(); field, "setTarget:", view => Id];
        msg![(); field, "setAction:", sel(c"nameEntered:") => Sel];
        msg![(); field, "setHidden:", true => bool];
        msg![(); view, "addSubview:", field => Id];
        msg![(); field, "release"]; // the view holds it
        msg![(); win, "center"];
        *g = Some((win as usize, view as usize, field as usize));
        Some((win, view, field))
    }
}
