//! The settings window on Windows: Direct2D + DirectWrite painting the portable view
//! model's frames (crates/ui), on a thread of its own with its own message loop.
//!
//! Never LV1's UI thread: LV1's ShowConfigWindow only posts here and returns. The window
//! has no owner (an owner on LV1's thread would attach the two input queues), is per-
//! monitor DPI aware (set on this thread only), takes mouse and touch (WM_POINTER for
//! touch and pen; the mouse stays on WM_*BUTTON*), and closing it only hides it.
//!
//! Functions newer than Windows 10 1507 are looked up at run time, so the DLL still loads
//! on an old build (it just draws at 96 DPI).
use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, COLORREF, HANDLE, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromWindow, ScreenToClient, SetBkColor, SetTextColor, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, FF_DONTCARE, FW_NORMAL, HBRUSH, HDC, HFONT, MONITORINFO, MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::*;

use tunesync::tune::Spelling;
use tunesync_host::sys::Worker;
use tunesync_host::Runtime;
use tunesync_ui::draw::{Align, Font, Item};
use tunesync_ui::model::Model;
use tunesync_ui::view::{layout, Command, Frame, View, DEFAULT_HEIGHT, MIN_HEIGHT, WIDTH};

const WM_APP_SHOW: u32 = WM_APP + 1;
const WM_APP_QUIT: u32 = WM_APP + 2;
const TIMER_REFRESH: usize = 1;
const EDIT_ID: usize = 100;
const CF_UNICODETEXT: u32 = 13;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;

pub type Problem = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// The window, from the driver's side: started on first show, hidden on close.
pub struct Window {
    rt: Arc<Runtime>,
    problem: Problem,
    hwnd: Arc<AtomicIsize>,
    worker: Mutex<Option<Worker>>,
}

impl Window {
    pub fn new(rt: Arc<Runtime>, problem: Problem) -> Self {
        Window { rt, problem, hwnd: Arc::new(AtomicIsize::new(0)), worker: Mutex::new(None) }
    }

    /// From LV1's thread: start the window thread if needed, else post "show". Returns at once.
    pub fn show(&self) {
        let mut w = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        let h = self.hwnd.load(Ordering::SeqCst);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(HWND(h as _), WM_APP_SHOW, WPARAM(0), LPARAM(0));
            }
            return;
        }
        if w.is_some() {
            return; // being created; it shows itself
        }
        let (rt, problem, hwnd) = (self.rt.clone(), self.problem.clone(), self.hwnd.clone());
        *w = Some(Worker::spawn("tunesync-window", move || run(rt, problem, hwnd)));
    }

    /// Close the window and end its thread (bounded).
    pub fn stop(&self) {
        let h = self.hwnd.load(Ordering::SeqCst);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(HWND(h as _), WM_APP_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(mut w) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // Bounded: LV1's thread is waiting. (Under Wine without a window manager hiding a
            // window takes 2 s; the thread then finishes on its own.)
            w.join_for(Duration::from_millis(1000));
        }
    }
}

// ---------------------------------------------------------------- run-time lookups

type SetThreadDpiAwarenessContextFn = unsafe extern "system" fn(isize) -> isize;
type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;

fn user32(name: &str) -> Option<unsafe extern "system" fn() -> isize> {
    let c: Vec<u8> = name.bytes().chain([0]).collect();
    unsafe {
        let m = GetModuleHandleW(w!("user32.dll")).ok()?;
        GetProcAddress(m, windows::core::PCSTR(c.as_ptr()))
    }
}

fn window_dpi(hwnd: HWND) -> f32 {
    match user32("GetDpiForWindow") {
        Some(f) => {
            let f: GetDpiForWindowFn = unsafe { std::mem::transmute(f) };
            let d = unsafe { f(hwnd) };
            if d == 0 {
                96.0
            } else {
                d as f32
            }
        }
        None => 96.0,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// The window's icons at the sizes its DPI draws them (title bar 16 px at 100 %, 28 px at
/// 175 %; Alt+Tab 32 / 56), each loaded from the .ico's own image of that size. The class
/// icons are the 96-DPI ones, which Windows would stretch on a scaled screen.
unsafe fn set_icons(hwnd: HWND, inst: HMODULE, dpi: f32, icons: &mut [HICON; 2]) {
    for (i, base) in [16.0f32, 32.0].into_iter().enumerate() {
        let px = (base * dpi / 96.0).round() as i32;
        let Ok(h) = LoadImageW(inst, PCWSTR(1 as *const u16), IMAGE_ICON, px, px, LR_DEFAULTCOLOR) else { continue };
        let h = HICON(h.0);
        // WM_SETICON: wParam 0 = ICON_SMALL, 1 = ICON_BIG.
        SendMessageW(hwnd, WM_SETICON, WPARAM(i), LPARAM(h.0 as isize));
        if !icons[i].0.is_null() {
            let _ = DestroyIcon(icons[i]);
        }
        icons[i] = h;
    }
}

// ---------------------------------------------------------------- the window thread

struct Gfx {
    target: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
}

struct State {
    rt: Arc<Runtime>,
    problem: Problem,
    local_ips: Vec<String>,
    view: View,
    model: Model,
    frame: Frame,
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    formats: HashMap<(bool, u32, u16), IDWriteTextFormat>,
    gfx: Option<Gfx>,
    dpi: f32,
    inst: HMODULE,
    icons: [HICON; 2],
    edit: HWND,
    edit_font: HFONT,
    edit_font_dpi: f32,
    edit_brush: HBRUSH,
    edit_shown: bool,
    edit_text: String,
    captured: bool,
}

impl State {
    fn dip(&self, px: i32) -> f32 {
        px as f32 * 96.0 / self.dpi
    }
    fn size(&self, hwnd: HWND) -> (f32, f32) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut r);
        }
        (self.dip(r.right - r.left).max(1.0), self.dip(r.bottom - r.top).max(1.0))
    }
    fn refresh(&mut self) -> bool {
        let m = crate::model::build(&self.rt, (self.problem)(), &self.local_ips);
        let changed = m != self.model;
        self.model = m;
        changed
    }
    fn relayout(&mut self, hwnd: HWND) {
        let (w, h) = self.size(hwnd);
        self.frame = layout(&self.model, &self.view, w, h);
        self.place_edit();
    }
    fn place_edit(&mut self) {
        match &self.frame.edit {
            Some((r, text)) => unsafe {
                let s = self.dpi / 96.0;
                if !self.edit_shown {
                    let t = wide(text);
                    let _ = SetWindowTextW(self.edit, PCWSTR(t.as_ptr()));
                    self.edit_text = text.clone();
                }
                if self.edit_font_dpi != self.dpi {
                    self.edit_font = make_font(self.dpi);
                    self.edit_font_dpi = self.dpi;
                    SendMessageW(self.edit, WM_SETFONT, WPARAM(self.edit_font.0 as usize), LPARAM(1));
                }
                let _ = SetWindowPos(
                    self.edit,
                    HWND_TOP,
                    (r.x * s) as i32,
                    (r.y * s + 4.0 * s) as i32,
                    (r.w * s) as i32,
                    (r.h * s - 6.0 * s) as i32,
                    SWP_SHOWWINDOW | SWP_NOACTIVATE,
                );
                self.edit_shown = true;
            },
            None => {
                if self.edit_shown {
                    self.commit_edit();
                    unsafe {
                        let _ = ShowWindow(self.edit, SW_HIDE);
                    }
                    self.edit_shown = false;
                }
            }
        }
    }
    /// The session name typed into the edit box, applied when it loses focus or the sheet closes.
    fn commit_edit(&mut self) {
        let mut buf = [0u16; 128];
        let n = unsafe { GetWindowTextW(self.edit, &mut buf) } as usize;
        let t = String::from_utf16_lossy(&buf[..n]).trim().to_string();
        if !t.is_empty() && t.len() <= 63 && t != self.model.settings.rtp_name {
            self.edit_text = t.clone();
            self.rt.update(|s| s.rtp_name = t);
        }
    }
    fn run_commands(&mut self, hwnd: HWND, cmds: Vec<Command>) {
        for c in cmds {
            match c {
                Command::SetMidiPort(p) => self.rt.update(|s| s.midi_port = p),
                Command::SetRtpEnabled(b) => self.rt.update(|s| s.rtp_enabled = b),
                Command::SetRtpName(n) => self.rt.update(|s| s.rtp_name = n),
                Command::SetRtpPort(p) => self.rt.update(|s| s.rtp_port = p),
                Command::SetChannel(ch) => self.rt.update(|s| s.channel = ch),
                Command::SetScaleCc(cc) => self.rt.update(|s| s.scale_cc = cc),
                Command::SetSpelling(sp) => self.rt.update(|s| s.spelling = if sp == Spelling::Flat { Spelling::Flat } else { Spelling::Sharp }),
                Command::ApplyNow(i) => self.rt.apply_now(i),
                Command::Rescan => self.rt.rescan(),
                Command::CopyLog => copy_log(hwnd, &self.model),
                Command::ClearLog => self.rt.log().clear(),
                Command::Connect { ip, port, name } => self.rt.rtp_connect(&ip, port, &name),
                Command::Forget { ip, port } => self.rt.rtp_forget(&ip, port),
            }
        }
        self.refresh();
    }
    fn format(&mut self, mono: bool, size: f32, weight: u16) -> Option<IDWriteTextFormat> {
        let key = (mono, (size * 10.0) as u32, weight);
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        let family = if mono { w!("Consolas") } else { w!("Segoe UI") };
        let f = unsafe { self.dw.CreateTextFormat(family, None, DWRITE_FONT_WEIGHT(weight as i32), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("en-us")) }.ok()?;
        unsafe {
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                let t = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = f.SetTrimming(&t, &sign);
            }
        }
        self.formats.insert(key, f.clone());
        Some(f)
    }
    fn paint(&mut self, hwnd: HWND) {
        if self.gfx.is_none() {
            let mut r = RECT::default();
            unsafe {
                let _ = GetClientRect(hwnd, &mut r);
            }
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: self.dpi,
                dpiY: self.dpi,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U { width: (r.right - r.left).max(1) as u32, height: (r.bottom - r.top).max(1) as u32 },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let Ok(target) = (unsafe { self.d2d.CreateHwndRenderTarget(&props, &hp) }) else { return };
            let Ok(brush) = (unsafe { target.CreateSolidColorBrush(&color(0xffffffff), None) }) else { return };
            self.gfx = Some(Gfx { target, brush });
        }
        let items = std::mem::take(&mut self.frame.items);
        let gfx = self.gfx.as_ref().unwrap();
        let (t, b) = (gfx.target.clone(), gfx.brush.clone());
        unsafe {
            t.BeginDraw();
            t.Clear(Some(&color(0x16181cff)));
            for it in &items {
                match it {
                    Item::Box { r, radius, fill, stroke } => {
                        let rect = D2D_RECT_F { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() };
                        b.SetColor(&color(fill.0));
                        if *radius <= 0.0 {
                            t.FillRectangle(&rect, &b);
                        } else {
                            let rr = D2D1_ROUNDED_RECT { rect, radiusX: *radius, radiusY: *radius };
                            t.FillRoundedRectangle(&rr, &b);
                            if let Some(s) = stroke {
                                b.SetColor(&color(s.0));
                                let inner = D2D_RECT_F { left: r.x + 0.5, top: r.y + 0.5, right: r.right() - 0.5, bottom: r.bottom() - 0.5 };
                                t.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: inner, radiusX: *radius, radiusY: *radius }, &b, 1.0, None);
                            }
                        }
                    }
                    Item::Text { r, text, size, weight, color: c, align, font } => {
                        let Some(f) = self.format(*font == Font::Mono, *size, *weight) else { continue };
                        let _ = f.SetTextAlignment(match align {
                            Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                            Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
                        });
                        b.SetColor(&color(c.0));
                        let s: Vec<u16> = text.encode_utf16().collect();
                        let rect = D2D_RECT_F { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() };
                        t.DrawText(&s, &f, &rect, &b, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
                    }
                    Item::Dot { cx, cy, radius, color: c, glow } => {
                        if *glow {
                            b.SetColor(&color((c.0 & 0xffffff00) | 0x50));
                            t.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: *cx, y: *cy }, radiusX: radius * 1.8, radiusY: radius * 1.8 }, &b);
                        }
                        b.SetColor(&color(c.0));
                        t.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: *cx, y: *cy }, radiusX: *radius, radiusY: *radius }, &b);
                    }
                    Item::Clip { r } => {
                        t.PushAxisAlignedClip(&D2D_RECT_F { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() }, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    }
                    Item::Unclip => t.PopAxisAlignedClip(),
                }
            }
            if t.EndDraw(None, None).is_err() {
                self.gfx = None; // D2DERR_RECREATE_TARGET: made again on the next paint
            }
        }
        self.frame.items = items;
    }
}

fn color(rgba: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgba >> 24) & 0xff) as f32 / 255.0,
        g: ((rgba >> 16) & 0xff) as f32 / 255.0,
        b: ((rgba >> 8) & 0xff) as f32 / 255.0,
        a: (rgba & 0xff) as f32 / 255.0,
    }
}

fn make_font(dpi: f32) -> HFONT {
    unsafe {
        CreateFontW(
            -((14.0 * dpi / 96.0) as i32),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            FF_DONTCARE.0 as u32,
            w!("Segoe UI"),
        )
    }
}

fn copy_log(hwnd: HWND, m: &Model) {
    let text: String = m.log.iter().rev().map(|l| format!("{} {}\r\n", l.stamp, l.text)).collect();
    let w: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        if OpenClipboard(hwnd).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(h);
                let _ = SetClipboardData(CF_UNICODETEXT, HANDLE(h.0));
            }
        }
        let _ = CloseClipboard();
    }
}

fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    unsafe { p.as_mut() }
}

fn lparam_xy(l: LPARAM) -> (i32, i32) {
    ((l.0 & 0xffff) as i16 as i32, ((l.0 >> 16) & 0xffff) as i16 as i32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(st) = state(hwnd) else { return DefWindowProcW(hwnd, msg, wp, lp) };
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _hdc = BeginPaint(hwnd, &mut ps);
            st.relayout(hwnd);
            st.paint(hwnd);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            if let Some(g) = &st.gfx {
                let (w, h) = lparam_xy(lp);
                let _ = g.target.Resize(&D2D_SIZE_U { width: w.max(1) as u32, height: h.max(1) as u32 });
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lp.0 as *mut MINMAXINFO);
            let s = st.dpi / 96.0;
            let mut fr = RECT { left: 0, top: 0, right: (WIDTH * s) as i32, bottom: (MIN_HEIGHT * s) as i32 };
            let _ = AdjustWindowRectEx(&mut fr, style(), false, WINDOW_EX_STYLE(0));
            mmi.ptMinTrackSize = POINT { x: fr.right - fr.left, y: fr.bottom - fr.top };
            mmi.ptMaxTrackSize.x = fr.right - fr.left;
            LRESULT(0)
        }
        0x02E0 /* WM_DPICHANGED */ => {
            st.dpi = (wp.0 & 0xffff) as f32;
            st.gfx = None;
            let (inst, dpi) = (st.inst, st.dpi);
            set_icons(hwnd, inst, dpi, &mut st.icons);
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_TIMER => {
            if st.refresh() {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_xy(lp);
            SetCapture(hwnd);
            st.captured = true;
            let (fx, fy) = (st.dip(x), st.dip(y));
            st.view.pointer_down(&st.frame, fx, fy);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if st.captured {
                let (x, y) = lparam_xy(lp);
                st.view.pointer_move(st.dip(x), st.dip(y));
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_xy(lp);
            if st.captured {
                st.captured = false;
                let _ = ReleaseCapture();
            }
            st.relayout(hwnd);
            let m = st.model.clone();
            let cmds = st.view.pointer_up(&st.frame, st.dip(x), st.dip(y), &m);
            st.run_commands(hwnd, cmds);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        // Touch and pen (the mouse is not a pointer here: EnableMouseInPointer is process-
        // wide and LV1's business). Handled, so Windows makes no mouse messages of them.
        // 0x0245 WM_POINTERUPDATE, 0x0246 WM_POINTERDOWN, 0x0247 WM_POINTERUP
        0x0245..=0x0247 => {
            let (sx, sy) = lparam_xy(lp);
            let mut p = POINT { x: sx, y: sy };
            let _ = ScreenToClient(hwnd, &mut p);
            let (x, y) = (st.dip(p.x), st.dip(p.y));
            let in_contact = ((wp.0 >> 16) & 0x0004) != 0;
            match msg {
                0x0246 => st.view.pointer_down(&st.frame, x, y),
                0x0245 if in_contact => st.view.pointer_move(x, y),
                0x0247 => {
                    st.relayout(hwnd);
                    let m = st.model.clone();
                    let cmds = st.view.pointer_up(&st.frame, x, y, &m);
                    st.run_commands(hwnd, cmds);
                }
                _ => {}
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
            let (sx, sy) = lparam_xy(lp);
            let mut p = POINT { x: sx, y: sy };
            let _ = ScreenToClient(hwnd, &mut p);
            let (x, y) = (st.dip(p.x), st.dip(p.y));
            st.view.wheel(&st.frame, x, y, -delta / 120.0 * 48.0);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wp.0 == VK_ESCAPE.0 as usize {
                if !st.view.back() {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            // EN_KILLFOCUS from the session-name box
            if (wp.0 & 0xffff) == EDIT_ID && ((wp.0 >> 16) & 0xffff) == 0x0200 {
                st.commit_edit();
            }
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wp.0 as _);
            SetTextColor(hdc, COLORREF(0x00edeae8));
            SetBkColor(hdc, COLORREF(0x00281f22 & 0x00ffffff | 0x0028221f));
            LRESULT(st.edit_brush.0 as isize)
        }
        WM_CLOSE => {
            if st.edit_shown {
                st.commit_edit();
            }
            st.view.back();
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_APP_SHOW => {
            st.refresh();
            st.rt.rescan();
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(hwnd);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_APP_QUIT => {
            // Hidden first and destroyed after the loop: no activation dance while LV1's
            // Destroy waits for this thread.
            let _ = ShowWindow(hwnd, SW_HIDE);
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn style() -> WINDOW_STYLE {
    // WS_CLIPCHILDREN: Direct2D must not paint over the session-name edit box.
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_THICKFRAME | WS_CLIPCHILDREN
}

fn run(rt: Arc<Runtime>, problem: Problem, hwnd_out: Arc<AtomicIsize>) {
    unsafe {
        // Per-monitor DPI for this thread only (the process's mode is LV1's).
        if let Some(f) = user32("SetThreadDpiAwarenessContext") {
            let f: SetThreadDpiAwarenessContextFn = std::mem::transmute(f);
            f(-4); // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
        }
        let Ok(d2d) = D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) else { return };
        let Ok(dw) = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) else { return };
        // TuneSync's own module (not LV1's exe): the class and the icon resource live there.
        let mut inst = HMODULE::default();
        let _ = windows::Win32::System::LibraryLoader::GetModuleHandleExW(
            windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(wndproc as *const () as *const u16),
            &mut inst,
        );
        let icon = |size: i32| {
            LoadImageW(inst, PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_SHARED).map(|h| HICON(h.0)).unwrap_or_default()
        };
        let class = w!("TuneSyncWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: icon(32),
            hIconSm: icon(16),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let title = wide("TuneSync");
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(title.as_ptr()),
            style(),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            100,
            100,
            None,
            None,
            inst,
            None,
        ) else {
            return;
        };
        let dpi = window_dpi(hwnd);
        // Size: 480 x 700 DIP of client, never taller than the monitor's work area.
        let s = dpi / 96.0;
        let mut fr = RECT { left: 0, top: 0, right: (WIDTH * s) as i32, bottom: (DEFAULT_HEIGHT * s) as i32 };
        let _ = AdjustWindowRectEx(&mut fr, style(), false, WINDOW_EX_STYLE(0));
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(mon, &mut mi);
        let work = mi.rcWork;
        let (fw, fh) = (fr.right - fr.left, (fr.bottom - fr.top).min(work.bottom - work.top));
        let x = work.left + ((work.right - work.left) - fw) / 2;
        let y = work.top + ((work.bottom - work.top) - fh) / 2;
        let _ = SetWindowPos(hwnd, None, x, y, fw, fh, SWP_NOZORDER | SWP_NOACTIVATE);
        let dark: BOOL = true.into();
        let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE as i32), &dark as *const BOOL as _, 4);

        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WINDOW_STYLE(0x0080 /* ES_AUTOHSCROLL */),
            0,
            0,
            10,
            10,
            hwnd,
            HMENU(EDIT_ID as _),
            inst,
            None,
        )
        .unwrap_or_default();
        SendMessageW(edit, 0x00C5 /* EM_LIMITTEXT */, WPARAM(63), LPARAM(0));

        let st = Box::new(State {
            local_ips: crate::model::local_ips(),
            view: View::new(),
            model: Model::default(),
            frame: Frame::default(),
            d2d,
            dw,
            formats: HashMap::new(),
            gfx: None,
            dpi,
            inst,
            icons: [HICON::default(); 2],
            edit,
            edit_font: HFONT::default(),
            edit_font_dpi: 0.0,
            edit_brush: CreateSolidBrush(COLORREF(0x0028221f)),
            edit_shown: false,
            edit_text: String::new(),
            captured: false,
            rt,
            problem,
        });
        let mut st = st;
        set_icons(hwnd, inst, dpi, &mut st.icons);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);
        SetTimer(hwnd, TIMER_REFRESH, 250, None);
        hwnd_out.store(hwnd.0 as isize, Ordering::SeqCst);
        let _ = PostMessageW(hwnd, WM_APP_SHOW, WPARAM(0), LPARAM(0));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Escape and Tab inside the edit box go to the window.
            if msg.message == WM_KEYDOWN && msg.hwnd == edit && msg.wParam.0 == VK_ESCAPE.0 as usize {
                let _ = SendMessageW(hwnd, WM_KEYDOWN, msg.wParam, msg.lParam);
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        hwnd_out.store(0, Ordering::SeqCst);
        let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        let mut icons = [HICON::default(); 2];
        if !p.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0); // the window procedure stops touching State
            icons = Box::from_raw(p).icons;
        }
        let _ = DestroyWindow(hwnd);
        for i in icons.into_iter().filter(|i| !i.0.is_null()) {
            let _ = DestroyIcon(i);
        }
        let _ = UnregisterClassW(class, inst);
    }
}
