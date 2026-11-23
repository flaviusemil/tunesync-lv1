//! What a frame is made of: rounded rectangles, text, dots and lines in DIPs (1/96 inch),
//! painted in order. A renderer (Direct2D on Windows, CoreGraphics on macOS, SVG here)
//! only has to draw these; text that does not fit its rectangle ends in an ellipsis.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    /// Shrunk by `d` on every side.
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
}

/// 0xRRGGBBAA
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const fn rgb(rgb: u32) -> Color {
        Color((rgb << 8) | 0xFF)
    }
    pub const fn rgba(rgba: u32) -> Color {
        Color(rgba)
    }
    pub fn r(&self) -> u8 {
        (self.0 >> 24) as u8
    }
    pub fn g(&self) -> u8 {
        (self.0 >> 16) as u8
    }
    pub fn b(&self) -> u8 {
        (self.0 >> 8) as u8
    }
    pub fn a(&self) -> u8 {
        self.0 as u8
    }
    pub const fn with_alpha(self, a: u8) -> Color {
        Color((self.0 & 0xFFFF_FF00) | a as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    /// Segoe UI on Windows, the system font elsewhere.
    Ui,
    /// Consolas / a monospace face, for the log.
    Mono,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A filled rounded rectangle, with an optional 1 px stroke.
    Box { r: Rect, radius: f32, fill: Color, stroke: Option<Color> },
    /// One line of text, vertically centred in `r`.
    Text { r: Rect, text: String, size: f32, weight: u16, color: Color, align: Align, font: Font },
    /// A filled circle (status dots), with an optional soft glow.
    Dot { cx: f32, cy: f32, radius: f32, color: Color, glow: bool },
    /// A circle's outline, `width` wide (a connected dot's ripple).
    Ring { cx: f32, cy: f32, radius: f32, width: f32, color: Color },
    /// Clip what follows to `r` until the matching `Unclip` (scrolling lists).
    Clip { r: Rect },
    Unclip,
}

/// The palette of docs/settings-onepage.html.
pub mod theme {
    use super::Color;
    pub const BG: Color = Color::rgb(0x16181c);
    pub const PANEL: Color = Color::rgb(0x1f2228);
    pub const PANEL2: Color = Color::rgb(0x262a31);
    pub const PRESSED: Color = Color::rgb(0x30353e);
    pub const LINE: Color = Color::rgb(0x2e333b);
    pub const FG: Color = Color::rgb(0xe8eaed);
    pub const DIM: Color = Color::rgb(0x8d939e);
    pub const FAINT: Color = Color::rgb(0x6c727d);
    pub const ACC: Color = Color::rgb(0x3fb37f);
    pub const ACCD: Color = Color::rgb(0x2b7d59);
    pub const ACC_TINT: Color = Color::rgba(0x2b7d5926);
    pub const OK_TEXT: Color = Color::rgb(0x9fd8bd);
    pub const WARN: Color = Color::rgb(0xe3a33c);
    pub const WARN_TINT: Color = Color::rgba(0xe3a33c22);
    pub const RCV_TINT: Color = Color::rgba(0x3fb37f1f);
    pub const SHEET: Color = Color::rgb(0x1b1e23);
    pub const SCRIM: Color = Color::rgba(0x000000aa);
    pub const GRAB: Color = Color::rgb(0x444a54);
    pub const IDLE_DOT: Color = Color::rgb(0x59606b);
}
