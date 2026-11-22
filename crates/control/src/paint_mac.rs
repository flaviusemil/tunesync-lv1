//! Paints a [`tunesync_ui::view::Frame`] with CoreGraphics and CoreText, into the
//! flipped NSView of the window (y grows downwards, as in the frame). One point is one
//! of the frame's units; Retina scaling is CoreGraphics' business.
//!
//! Colours are sRGB (the palette of docs/settings-onepage.html); text is the system font
//! (SF) or the monospaced system font for the log, one line, vertically centred, cut with
//! an ellipsis when it does not fit, as DirectWrite does on Windows.
use std::collections::HashMap;
use std::ffi::c_void;

use tunesync_host::mac::{release, CFIndex, CFStringRef, CFTypeRef, CfString};
use tunesync_ui::draw::{Align, Color, Font, Item};

use crate::objc::{class, msg, sel, Id, NSRect};

pub type CGContextRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGAffineTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFAttributedStringCreate(alloc: *const c_void, s: CFStringRef, attrs: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryCreate(alloc: *const c_void, keys: *const CFTypeRef, values: *const CFTypeRef, n: CFIndex, kcb: *const c_void, vcb: *const c_void) -> CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    static kCFBooleanTrue: CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateWithName(name: CFStringRef) -> CFTypeRef;
    fn CGContextSetFillColorSpace(c: CGContextRef, cs: CFTypeRef);
    fn CGContextSetStrokeColorSpace(c: CGContextRef, cs: CFTypeRef);
    fn CGContextSetFillColor(c: CGContextRef, comps: *const f64);
    fn CGContextSetStrokeColor(c: CGContextRef, comps: *const f64);
    fn CGContextFillRect(c: CGContextRef, r: NSRect);
    fn CGContextFillEllipseInRect(c: CGContextRef, r: NSRect);
    fn CGContextAddPath(c: CGContextRef, p: CFTypeRef);
    fn CGContextFillPath(c: CGContextRef);
    fn CGContextStrokePath(c: CGContextRef);
    fn CGContextSetLineWidth(c: CGContextRef, w: f64);
    fn CGPathCreateWithRoundedRect(r: NSRect, rx: f64, ry: f64, t: *const c_void) -> CFTypeRef;
    fn CGPathRelease(p: CFTypeRef);
    fn CGContextSaveGState(c: CGContextRef);
    fn CGContextRestoreGState(c: CGContextRef);
    fn CGContextClipToRect(c: CGContextRef, r: NSRect);
    fn CGContextSetTextMatrix(c: CGContextRef, t: CGAffineTransform);
    fn CGContextSetTextPosition(c: CGContextRef, x: f64, y: f64);
    static kCGColorSpaceSRGB: CFStringRef;
}

#[link(name = "CoreText", kind = "framework")]
extern "C" {
    fn CTLineCreateWithAttributedString(s: CFTypeRef) -> CFTypeRef;
    fn CTLineCreateTruncatedLine(line: CFTypeRef, width: f64, kind: u32, token: CFTypeRef) -> CFTypeRef;
    fn CTLineGetTypographicBounds(line: CFTypeRef, ascent: *mut f64, descent: *mut f64, leading: *mut f64) -> f64;
    fn CTLineDraw(line: CFTypeRef, c: CGContextRef);
    static kCTFontAttributeName: CFStringRef;
    static kCTForegroundColorFromContextAttributeName: CFStringRef;
}

const TRUNCATE_END: u32 = 1; // kCTLineTruncationEnd (Start 0, End 1, Middle 2)

/// NSFont's weights (NSFontWeightRegular, Medium, Semibold, Bold) for CSS-style numbers.
fn ns_weight(w: u16) -> f64 {
    match w {
        0..=349 => -0.4,
        350..=449 => 0.0,
        450..=549 => 0.23,
        550..=649 => 0.3,
        _ => 0.4,
    }
}

/// An NSFont (toll-free a CTFont), with its text attributes and its ellipsis line.
pub struct FontSet {
    font: Id,
    attrs: CFTypeRef,
    ellipsis: CFTypeRef,
}

impl Drop for FontSet {
    fn drop(&mut self) {
        unsafe {
            release(self.ellipsis);
            release(self.attrs);
            msg![(); self.font, "release"];
        }
    }
}

/// Makes the system font (or the monospaced one) at `size` points and `weight`.
pub fn ns_font(mono: bool, size: f64, weight: u16) -> Id {
    unsafe {
        let nsfont = class(c"NSFont");
        let w = ns_weight(weight);
        let f: Id = if !mono {
            msg![Id; nsfont, "systemFontOfSize:weight:", size => f64, w => f64]
        } else if msg![i8; nsfont, "respondsToSelector:", sel(c"monospacedSystemFontOfSize:weight:") => *const c_void] != 0 {
            msg![Id; nsfont, "monospacedSystemFontOfSize:weight:", size => f64, w => f64]
        } else {
            msg![Id; nsfont, "userFixedPitchFontOfSize:", size => f64]
        };
        if f.is_null() {
            msg![Id; nsfont, "systemFontOfSize:", size => f64]
        } else {
            f
        }
    }
}

fn line_of(text: &str, attrs: CFTypeRef) -> CFTypeRef {
    let s = CfString::new(text);
    unsafe {
        let a = CFAttributedStringCreate(std::ptr::null(), s.as_ptr(), attrs);
        if a.is_null() {
            return std::ptr::null();
        }
        let l = CTLineCreateWithAttributedString(a);
        release(a);
        l
    }
}

impl FontSet {
    fn new(mono: bool, size: f32, weight: u16) -> FontSet {
        unsafe {
            let font: Id = msg![Id; ns_font(mono, size as f64, weight), "retain"];
            let keys = [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName];
            let vals: [CFTypeRef; 2] = [font as CFTypeRef, kCFBooleanTrue];
            let attrs = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                vals.as_ptr(),
                2,
                &kCFTypeDictionaryKeyCallBacks as *const u8 as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const u8 as *const c_void,
            );
            let ellipsis = line_of("\u{2026}", attrs);
            FontSet { font, attrs, ellipsis }
        }
    }
}

/// The painter's caches: fonts by (mono, size, weight) and the sRGB colour space.
pub struct Painter {
    fonts: HashMap<(bool, u32, u16), FontSet>,
    srgb: CFTypeRef,
}

impl Default for Painter {
    fn default() -> Self {
        Painter { fonts: HashMap::new(), srgb: unsafe { CGColorSpaceCreateWithName(kCGColorSpaceSRGB) } }
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        self.fonts.clear();
        unsafe { release(self.srgb) };
    }
}

fn rgba(c: Color) -> [f64; 4] {
    [c.r() as f64 / 255.0, c.g() as f64 / 255.0, c.b() as f64 / 255.0, c.a() as f64 / 255.0]
}

fn rect(r: &tunesync_ui::draw::Rect) -> NSRect {
    NSRect::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64)
}

impl Painter {
    /// Paints `items` over a `bg` fill of `w` x `h` points.
    ///
    /// # Safety
    /// `ctx` is the current CGContext of a flipped view, inside drawRect:.
    pub unsafe fn paint(&mut self, ctx: CGContextRef, w: f64, h: f64, bg: Color, items: &[Item]) {
        if ctx.is_null() {
            return;
        }
        CGContextSaveGState(ctx);
        if !self.srgb.is_null() {
            CGContextSetFillColorSpace(ctx, self.srgb);
            CGContextSetStrokeColorSpace(ctx, self.srgb);
        }
        // CoreText draws y-up; the view is flipped.
        CGContextSetTextMatrix(ctx, CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: -1.0, tx: 0.0, ty: 0.0 });
        CGContextSetFillColor(ctx, rgba(bg).as_ptr());
        CGContextFillRect(ctx, NSRect::new(0.0, 0.0, w, h));
        let mut clips = 0usize;
        for it in items {
            match it {
                Item::Box { r, radius, fill, stroke } => {
                    let nr = rect(r);
                    CGContextSetFillColor(ctx, rgba(*fill).as_ptr());
                    let rad = (*radius as f64).min(nr.size.w / 2.0).min(nr.size.h / 2.0).max(0.0);
                    if rad <= 0.0 {
                        CGContextFillRect(ctx, nr);
                    } else {
                        let p = CGPathCreateWithRoundedRect(nr, rad, rad, std::ptr::null());
                        if !p.is_null() {
                            CGContextAddPath(ctx, p);
                            CGContextFillPath(ctx);
                            CGPathRelease(p);
                        }
                    }
                    if let Some(s) = stroke {
                        // a 1 pt line just inside the box, as Direct2D draws it
                        let inner = NSRect::new(nr.origin.x + 0.5, nr.origin.y + 0.5, (nr.size.w - 1.0).max(0.0), (nr.size.h - 1.0).max(0.0));
                        let rad = rad.min(inner.size.w / 2.0).min(inner.size.h / 2.0);
                        let p = CGPathCreateWithRoundedRect(inner, rad, rad, std::ptr::null());
                        if !p.is_null() {
                            CGContextSetStrokeColor(ctx, rgba(*s).as_ptr());
                            CGContextSetLineWidth(ctx, 1.0);
                            CGContextAddPath(ctx, p);
                            CGContextStrokePath(ctx);
                            CGPathRelease(p);
                        }
                    }
                }
                Item::Text { r, text, size, weight, color, align, font } => {
                    if text.is_empty() || r.w <= 0.0 {
                        continue;
                    }
                    let mono = *font == Font::Mono;
                    let fs = self.fonts.entry((mono, (*size * 10.0) as u32, *weight)).or_insert_with(|| FontSet::new(mono, *size, *weight));
                    let mut line = line_of(text, fs.attrs);
                    if line.is_null() {
                        continue;
                    }
                    let (mut asc, mut desc, mut lead) = (0.0, 0.0, 0.0);
                    let mut tw = CTLineGetTypographicBounds(line, &mut asc, &mut desc, &mut lead);
                    if tw > r.w as f64 {
                        let cut = CTLineCreateTruncatedLine(line, r.w as f64, TRUNCATE_END, fs.ellipsis);
                        release(line);
                        if cut.is_null() {
                            continue;
                        }
                        line = cut;
                        tw = CTLineGetTypographicBounds(line, &mut asc, &mut desc, &mut lead);
                    }
                    let x = match align {
                        Align::Left => r.x as f64,
                        Align::Center => r.x as f64 + (r.w as f64 - tw) / 2.0,
                        Align::Right => r.right() as f64 - tw,
                    };
                    let baseline = r.y as f64 + r.h as f64 / 2.0 + (asc - desc) / 2.0;
                    CGContextSetFillColor(ctx, rgba(*color).as_ptr());
                    CGContextSetTextPosition(ctx, x, baseline);
                    CTLineDraw(line, ctx);
                    release(line);
                }
                Item::Dot { cx, cy, radius, color, glow } => {
                    let (cx, cy, rad) = (*cx as f64, *cy as f64, *radius as f64);
                    if *glow {
                        let g = Color((color.0 & 0xffff_ff00) | 0x50);
                        CGContextSetFillColor(ctx, rgba(g).as_ptr());
                        let gr = rad * 1.8;
                        CGContextFillEllipseInRect(ctx, NSRect::new(cx - gr, cy - gr, gr * 2.0, gr * 2.0));
                    }
                    CGContextSetFillColor(ctx, rgba(*color).as_ptr());
                    CGContextFillEllipseInRect(ctx, NSRect::new(cx - rad, cy - rad, rad * 2.0, rad * 2.0));
                }
                Item::Clip { r } => {
                    CGContextSaveGState(ctx);
                    CGContextClipToRect(ctx, rect(r));
                    clips += 1;
                }
                Item::Unclip => {
                    if clips > 0 {
                        CGContextRestoreGState(ctx);
                        clips -= 1;
                    }
                }
            }
        }
        while clips > 0 {
            CGContextRestoreGState(ctx);
            clips -= 1;
        }
        // The text matrix is not part of the graphics state: leave it as we found it.
        CGContextSetTextMatrix(ctx, CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 });
        CGContextRestoreGState(ctx);
    }
}
