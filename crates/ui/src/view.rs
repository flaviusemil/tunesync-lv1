//! The settings window (docs/settings-onepage.html): one page, touch-first. Every tap
//! target is at least 44 DIP, there are no small dropdowns and no tabs, and details open
//! as sheets inside the window. Nothing needed during a show sits below the fold: the
//! Tunes list takes whatever height is left and scrolls.
//!
//! [`layout`] turns a [`Model`] and the window's own [`View`] state into a [`Frame`];
//! the shell paints its items and feeds pointer events back to [`View`], which answers
//! with [`Command`]s for the shell to carry out.
use tunesync::log::Level;
use tunesync::midi_map::Intent;
use tunesync::tune::{pitch_class_name, scale_name, Spelling, NUM_SCALES};

use crate::draw::theme::*;
use crate::draw::{Align, Color, Font, Item, Rect};
use crate::model::{pretty_key, Model};

pub const WIDTH: f32 = 480.0;
pub const MIN_HEIGHT: f32 = 560.0;
pub const DEFAULT_HEIGHT: f32 = 700.0;
const PAD: f32 = 14.0;
const GAP: f32 = 12.0;
const TILE_H: f32 = 56.0;
const ROW_H: f32 = 44.0;
const ROW_GAP: f32 = 6.0;
const OPT_H: f32 = 48.0;
/// Status dots.
const DOT_R: f32 = 4.5;
/// A connected peer's dot breathes once in this time...
const BREATH_MS: u64 = 2400;
/// ...and MIDI from it sends a ripple out for this long.
const RIPPLE_MS: u64 = 900;

/// What the shell must do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SetMidiPort(String),
    SetRtpEnabled(bool),
    SetRtpName(String),
    SetRtpPort(u16),
    SetChannel(u8),
    SetScaleCc(u8),
    SetSpelling(Spelling),
    ApplyNow(Intent),
    Rescan,
    CopyLog,
    ClearLog,
    Connect { ip: String, port: u16, name: String },
    Forget { ip: String, port: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberField {
    ScaleCc,
    RtpPort,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    MidiIn,
    Rtp,
    Channel,
    /// A keypad; the digits typed so far.
    Number(NumberField, String),
    Key,
    Scale,
    Log,
}

/// Something that can be tapped.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    MidiTile,
    RtpTile,
    ChannelTile,
    ScaleCcTile,
    Spelling(Spelling),
    Rescan,
    KeyTile,
    ScaleTile,
    ApplyNow,
    FullLog,
    Done,
    Scrim,
    MidiPort(String),
    RtpSwitch,
    RtpPortField,
    Forget(String, u16),
    Connect(String, u16, String),
    Channel(u8),
    Digit(u8),
    Backspace,
    PickKey(Option<u8>),
    PickScale(Option<u8>),
    CopyLog,
    ClearLog,
    /// A list that scrolls when dragged.
    TunesList,
    SheetList,
}

impl Hit {
    fn scrolls(&self) -> Option<Scroller> {
        match self {
            Hit::TunesList => Some(Scroller::Tunes),
            Hit::SheetList => Some(Scroller::Sheet),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scroller {
    Tunes,
    Sheet,
}

/// One painted frame: items in paint order, tap regions (later ones are on top), and the
/// text field the shell overlays with a native edit control (the RTP session name).
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub items: Vec<Item>,
    pub hits: Vec<(Rect, Hit)>,
    pub edit: Option<(Rect, String)>,
    /// Something on it moves: lay out and paint again in a frame's time (about 30 ms).
    pub animating: bool,
    /// How far each list can scroll, for clamping.
    tunes_max: f32,
    sheet_max: f32,
}

impl Frame {
    pub fn hit_at(&self, x: f32, y: f32) -> Option<&Hit> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| h)
    }
}

#[derive(Clone, Debug)]
struct Drag {
    hit: Hit,
    x: f32,
    y: f32,
    scroll0: f32,
    moved: bool,
}

/// The window's own state: the open sheet, scroll positions, what is pressed, and the
/// key and scale picked for Apply now.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub sheet: Option<Sheet>,
    pub pick_key: Option<u8>,
    pub pick_scale: Option<u8>,
    pub tunes_scroll: f32,
    pub sheet_scroll: f32,
    /// The host's clock, set before each layout: what moves is drawn for this moment, and
    /// the model's times (`PeerRow::midi_at_ms`) are on it. 0 = nothing moves (previews, tests).
    pub now_ms: u64,
    drag: Option<Drag>,
    /// The scrolling list under a press on one of its rows.
    list_under: Option<Hit>,
    tunes_max: f32,
    sheet_max: f32,
}

/// A tap moves less than this; more is a drag.
const SLOP: f32 = 8.0;

impl View {
    pub fn new() -> View {
        View { pick_key: Some(9), pick_scale: Some(3), ..Default::default() }
    }

    /// The hit under the finger, drawn pressed until it lifts.
    pub fn pressed(&self) -> Option<&Hit> {
        self.drag.as_ref().filter(|d| !d.moved && d.hit.scrolls().is_none()).map(|d| &d.hit)
    }

    pub fn pointer_down(&mut self, f: &Frame, x: f32, y: f32) {
        self.tunes_max = f.tunes_max;
        self.sheet_max = f.sheet_max;
        // A press inside a list may still be a tap on one of its rows: keep the row, and
        // remember the list's scroll for a drag.
        let hit = f.hit_at(x, y).cloned();
        let list = f.hits.iter().rev().find(|(r, h)| r.contains(x, y) && h.scrolls().is_some()).map(|(_, h)| h.clone());
        let scroll0 = match list.as_ref().and_then(|h| h.scrolls()) {
            Some(Scroller::Tunes) => self.tunes_scroll,
            Some(Scroller::Sheet) => self.sheet_scroll,
            None => 0.0,
        };
        self.drag = hit.map(|h| Drag { hit: h, x, y, scroll0, moved: false });
        if let (Some(d), Some(l)) = (self.drag.as_mut(), list) {
            if d.hit.scrolls().is_none() {
                // Remember the list too: a row press that moves becomes a scroll.
                d.scroll0 = scroll0;
                self.list_under = Some(l);
            }
        }
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        let Some(d) = self.drag.as_mut() else { return };
        if !d.moved && ((x - d.x).abs() > SLOP || (y - d.y).abs() > SLOP) {
            d.moved = true;
        }
        if !d.moved {
            return;
        }
        let target = d.hit.scrolls().or_else(|| self.list_under.as_ref().and_then(|h| h.scrolls()));
        let s = d.scroll0 - (y - d.y);
        match target {
            Some(Scroller::Tunes) => self.tunes_scroll = s.clamp(0.0, self.tunes_max),
            Some(Scroller::Sheet) => self.sheet_scroll = s.clamp(0.0, self.sheet_max),
            None => {}
        }
    }

    /// The finger lifted: a tap on the hit it went down on (and still is over) acts.
    pub fn pointer_up(&mut self, f: &Frame, x: f32, y: f32, m: &Model) -> Vec<Command> {
        self.list_under = None;
        let Some(d) = self.drag.take() else { return Vec::new() };
        if d.moved || f.hit_at(x, y) != Some(&d.hit) {
            return Vec::new();
        }
        self.tap(&d.hit, m)
    }

    /// A mouse wheel or touchpad scroll over (x, y), in DIPs (positive = down).
    pub fn wheel(&mut self, f: &Frame, x: f32, y: f32, dy: f32) {
        let list = f.hits.iter().rev().find(|(r, h)| r.contains(x, y) && h.scrolls().is_some()).map(|(_, h)| h.clone());
        match list.and_then(|h| h.scrolls()) {
            Some(Scroller::Tunes) => self.tunes_scroll = (self.tunes_scroll + dy).clamp(0.0, f.tunes_max),
            Some(Scroller::Sheet) => self.sheet_scroll = (self.sheet_scroll + dy).clamp(0.0, f.sheet_max),
            None => {}
        }
    }

    /// Escape / the window's close button while a sheet is open.
    pub fn back(&mut self) -> bool {
        if self.sheet.is_some() {
            self.close_sheet();
            return true;
        }
        false
    }

    fn open(&mut self, s: Sheet) {
        self.sheet = Some(s);
        self.sheet_scroll = 0.0;
    }

    fn close_sheet(&mut self) {
        self.sheet = None;
        self.sheet_scroll = 0.0;
    }

    /// What a tap on `h` does.
    pub fn tap(&mut self, h: &Hit, m: &Model) -> Vec<Command> {
        let s = &m.settings;
        let mut out = Vec::new();
        match h {
            Hit::MidiTile => self.open(Sheet::MidiIn),
            Hit::RtpTile => self.open(Sheet::Rtp),
            Hit::ChannelTile => self.open(Sheet::Channel),
            Hit::ScaleCcTile => self.open(Sheet::Number(NumberField::ScaleCc, String::new())),
            Hit::KeyTile => self.open(Sheet::Key),
            Hit::ScaleTile => self.open(Sheet::Scale),
            Hit::FullLog => self.open(Sheet::Log),
            Hit::Spelling(sp) => {
                if *sp != s.spelling {
                    out.push(Command::SetSpelling(*sp));
                }
            }
            Hit::Rescan => out.push(Command::Rescan),
            Hit::ApplyNow => {
                let i = Intent { pitch_class: self.pick_key, scale: self.pick_scale };
                if !i.is_empty() {
                    out.push(Command::ApplyNow(i));
                }
            }
            Hit::Scrim => self.close_sheet(),
            Hit::Done => {
                if let Some(Sheet::Number(field, buf)) = &self.sheet {
                    if let Some(c) = number_command(*field, buf) {
                        out.push(c);
                    }
                    // the port keypad came from the RTP sheet: go back there
                    if *field == NumberField::RtpPort {
                        self.open(Sheet::Rtp);
                        return out;
                    }
                }
                self.close_sheet();
            }
            Hit::MidiPort(name) => {
                if *name != s.midi_port {
                    out.push(Command::SetMidiPort(name.clone()));
                }
                self.close_sheet();
            }
            Hit::RtpSwitch => out.push(Command::SetRtpEnabled(!s.rtp_enabled)),
            Hit::RtpPortField => self.open(Sheet::Number(NumberField::RtpPort, String::new())),
            Hit::Forget(ip, port) => out.push(Command::Forget { ip: ip.clone(), port: *port }),
            Hit::Connect(ip, port, name) => out.push(Command::Connect { ip: ip.clone(), port: *port, name: name.clone() }),
            Hit::Channel(c) => {
                if *c != s.channel {
                    out.push(Command::SetChannel(*c));
                }
                self.close_sheet();
            }
            Hit::Digit(d) => {
                if let Some(Sheet::Number(_, buf)) = &mut self.sheet {
                    if buf.len() < 5 {
                        buf.push(char::from(b'0' + d));
                    }
                }
            }
            Hit::Backspace => {
                if let Some(Sheet::Number(_, buf)) = &mut self.sheet {
                    buf.pop();
                }
            }
            Hit::PickKey(k) => {
                self.pick_key = *k;
                self.close_sheet();
            }
            Hit::PickScale(sc) => {
                self.pick_scale = *sc;
                self.close_sheet();
            }
            Hit::CopyLog => out.push(Command::CopyLog),
            Hit::ClearLog => out.push(Command::ClearLog),
            Hit::TunesList | Hit::SheetList => {}
        }
        out
    }
}

/// The keypad's value, when it is in range.
pub fn number_command(field: NumberField, buf: &str) -> Option<Command> {
    let n: u32 = buf.parse().ok()?;
    match field {
        NumberField::ScaleCc if n <= 119 => Some(Command::SetScaleCc(n as u8)),
        NumberField::RtpPort if (1024..=65534).contains(&n) => Some(Command::SetRtpPort(n as u16)),
        _ => None,
    }
}

// ---------------------------------------------------------------- building a frame

struct B<'a> {
    f: Frame,
    v: &'a View,
}

impl B<'_> {
    fn bx(&mut self, r: Rect, radius: f32, fill: Color, stroke: Option<Color>) {
        self.f.items.push(Item::Box { r, radius, fill, stroke });
    }
    fn text(&mut self, r: Rect, t: &str, size: f32, weight: u16, color: Color, align: Align) {
        self.f.items.push(Item::Text { r, text: t.to_string(), size, weight, color, align, font: Font::Ui });
    }
    fn mono(&mut self, r: Rect, t: &str, size: f32, color: Color) {
        self.f.items.push(Item::Text { r, text: t.to_string(), size, weight: 400, color, align: Align::Left, font: Font::Mono });
    }
    fn label(&mut self, r: Rect, t: &str) {
        self.text(r, &t.to_uppercase(), 11.0, 600, DIM, Align::Left);
    }
    fn dot(&mut self, cx: f32, cy: f32, color: Color, glow: bool) {
        self.f.items.push(Item::Dot { cx, cy, radius: DOT_R, color, glow });
    }
    fn hit(&mut self, r: Rect, h: Hit) {
        self.f.hits.push((r, h));
    }
    fn is_pressed(&self, h: &Hit) -> bool {
        self.v.pressed() == Some(h)
    }
    /// A tappable tile: a label over a value, a chevron at the right.
    fn tile(&mut self, r: Rect, label: &str, value: &str, value_color: Color, dot: Option<(Color, bool)>, h: Hit) {
        let fill = if self.is_pressed(&h) { PRESSED } else { PANEL };
        self.bx(r, 10.0, fill, Some(LINE));
        self.label(Rect::new(r.x + 12.0, r.y + 8.0, r.w - 30.0, 16.0), label);
        let mut vx = r.x + 12.0;
        if let Some((c, glow)) = dot {
            self.dot(vx + 4.5, r.y + 36.0, c, glow);
            vx += 15.0;
        }
        self.text(Rect::new(vx, r.y + 25.0, r.right() - vx - 26.0, 22.0), value, 16.0, 600, value_color, Align::Left);
        self.text(Rect::new(r.right() - 24.0, r.y, 16.0, r.h), "›", 18.0, 400, DIM, Align::Center);
        self.hit(r, h);
    }
    /// A small button (Rescan, Full log, Copy).
    fn button(&mut self, r: Rect, t: &str, h: Hit, accent: bool) {
        self.button_in(r, r, t, h, accent);
    }
    /// A button drawn at `r` that takes taps anywhere in the larger `tap`.
    fn button_in(&mut self, r: Rect, tap: Rect, t: &str, h: Hit, accent: bool) {
        let fill = if self.is_pressed(&h) { PRESSED } else if accent { ACCD } else { PANEL2 };
        self.bx(r, 8.0, fill, Some(if accent { ACC } else { LINE }));
        self.text(r, t, 13.0, if accent { 600 } else { 400 }, FG, Align::Center);
        self.hit(tap, h);
    }
    /// A peer's status dot. Connected, it breathes, and each MIDI message from the peer
    /// sends a ripple out of it.
    fn peer_dot(&mut self, cx: f32, cy: f32, connected: bool, midi_at_ms: u64) {
        let now = self.v.now_ms;
        if !connected || now == 0 {
            self.dot(cx, cy, if connected { ACC } else { IDLE_DOT }, connected);
            return;
        }
        let tau = std::f32::consts::TAU;
        let b = 0.5 - 0.5 * ((now % BREATH_MS) as f32 / BREATH_MS as f32 * tau).cos(); // 0 -> 1 -> 0
        let halo = Item::Dot { cx, cy, radius: DOT_R * (1.45 + 0.55 * b), color: ACC.with_alpha((0x1c as f32 + 0x44 as f32 * b) as u8), glow: false };
        self.f.items.push(halo);
        let age = now.saturating_sub(midi_at_ms);
        if midi_at_ms != 0 && age < RIPPLE_MS {
            let t = age as f32 / RIPPLE_MS as f32;
            let out = 1.0 - (1.0 - t).powi(3); // fast, then settling
            self.f.items.push(Item::Ring { cx, cy, radius: DOT_R + 10.0 * out, width: 1.5, color: ACC.with_alpha((0xd0 as f32 * (1.0 - t)) as u8) });
        }
        self.f.items.push(Item::Dot { cx, cy, radius: DOT_R, color: ACC, glow: false });
        self.f.animating = true;
    }
    /// A 48 DIP option row in a sheet.
    fn option(&mut self, r: Rect, t: &str, sub: &str, on: bool, h: Hit) {
        let fill = if self.is_pressed(&h) { PRESSED } else if on { ACC_TINT } else { PANEL };
        self.bx(r, 8.0, fill, Some(if on { ACC } else { LINE }));
        self.text(Rect::new(r.x + 12.0, r.y, r.w - 120.0, r.h), t, 14.0, 400, FG, Align::Left);
        if !sub.is_empty() {
            self.text(Rect::new(r.right() - 220.0, r.y, 180.0, r.h), sub, 12.5, 400, DIM, Align::Right);
        }
        if on {
            self.text(Rect::new(r.right() - 34.0, r.y, 24.0, r.h), "✓", 16.0, 700, ACC, Align::Center);
        }
        self.hit(r, h);
    }
}

fn key_label(pc: Option<u8>) -> String {
    pc.map(|p| pretty_key(&pitch_class_name(p))).unwrap_or_else(|| "Keep".into())
}

fn scale_label(s: Option<u8>) -> String {
    s.and_then(scale_name).unwrap_or("Keep").to_string()
}

/// Lays out the whole window, `w` x `h` DIPs.
pub fn layout(m: &Model, v: &View, w: f32, h: f32) -> Frame {
    let mut b = B { f: Frame::default(), v };
    let s = &m.settings;
    let cw = w - 2.0 * PAD;
    b.bx(Rect::new(0.0, 0.0, w, h), 0.0, BG, None);
    let mut y = GAP;

    // ---- inputs: MIDI in, RTP-MIDI
    let half = (cw - 10.0) / 2.0;
    let (midi_val, midi_col, midi_dot) = if s.midi_port.is_empty() {
        ("None".to_string(), DIM, None)
    } else if m.midi_open == s.midi_port {
        (s.midi_port.clone(), FG, Some((if m.midi_active { ACC } else { IDLE_DOT }, m.midi_active)))
    } else {
        (format!("{} (not connected)", s.midi_port), WARN, Some((WARN, false)))
    };
    b.tile(Rect::new(PAD, y, half, TILE_H), "MIDI in", &midi_val, midi_col, midi_dot, Hit::MidiTile);
    let connected = m.rtp.peers.iter().filter(|p| p.connected).count();
    let (rtp_val, rtp_col, rtp_dot) = if !s.rtp_enabled {
        ("Off".to_string(), DIM, None)
    } else if !m.rtp.open {
        ("Not open".to_string(), WARN, Some((WARN, false)))
    } else if connected == 0 {
        ("On · no peers".to_string(), FG, Some((IDLE_DOT, false)))
    } else {
        (format!("{connected} peer{}", if connected == 1 { "" } else { "s" }), FG, Some((if m.rtp.active { ACC } else { IDLE_DOT }, m.rtp.active)))
    };
    b.tile(Rect::new(PAD + half + 10.0, y, half, TILE_H), "RTP-MIDI", &rtp_val, rtp_col, rtp_dot, Hit::RtpTile);
    y += TILE_H + GAP;

    // ---- mapping: channel, scale CC, spelling
    let unit = (cw - 20.0) / 3.25;
    let (c1, c2) = (unit, unit);
    let c3 = cw - c1 - c2 - 20.0;
    let ch = if s.channel == 0 { "Omni".to_string() } else { s.channel.to_string() };
    b.tile(Rect::new(PAD, y, c1, TILE_H), "Channel", &ch, FG, None, Hit::ChannelTile);
    b.tile(Rect::new(PAD + c1 + 10.0, y, c2, TILE_H), "Scale CC", &s.scale_cc.to_string(), FG, None, Hit::ScaleCcTile);
    let sp = Rect::new(PAD + c1 + c2 + 20.0, y, c3, TILE_H);
    b.bx(sp, 10.0, PANEL, Some(LINE));
    b.label(Rect::new(sp.x + 12.0, sp.y + 6.0, sp.w - 20.0, 14.0), "Spelling");
    let seg = Rect::new(sp.x + 8.0, sp.y + 22.0, sp.w - 16.0, 28.0);
    b.bx(seg, 7.0, PANEL2, Some(LINE));
    for (i, (glyph, which)) in [("♯", Spelling::Sharp), ("♭", Spelling::Flat)].into_iter().enumerate() {
        let r = Rect::new(seg.x + i as f32 * seg.w / 2.0, seg.y, seg.w / 2.0, seg.h);
        let on = s.spelling == which;
        if on {
            b.bx(r.inset(2.0), 6.0, ACCD, None);
        }
        b.text(r, glyph, 17.0, 600, if on { FG } else { DIM }, Align::Center);
        // the whole tile half is the tap target (>= 44 DIP tall)
        b.hit(Rect::new(r.x, sp.y, r.w, sp.h), Hit::Spelling(which));
    }
    y += TILE_H + GAP;

    // ---- the last change, or what is wrong
    let banner = Rect::new(PAD, y, cw, 52.0);
    let (title, meta, accent) = match (&m.lv1_problem, &m.last) {
        (Some(p), _) => ("LV1 stopped answering".to_string(), p.clone(), WARN),
        (None, Some(l)) => (l.label.clone(), l.meta.clone(), if l.problem { WARN } else { ACC }),
        (None, None) => ("No change yet".to_string(), "Waiting for a note from the tracks rig".to_string(), IDLE_DOT),
    };
    b.bx(banner, 6.0, if accent == WARN { WARN_TINT } else if accent == ACC { ACC_TINT } else { PANEL }, None);
    b.bx(Rect::new(banner.x, banner.y, 3.0, banner.h), 1.5, accent, None);
    b.text(Rect::new(banner.x + 14.0, banner.y + 6.0, banner.w - 24.0, 22.0), &title, 16.0, 700, FG, Align::Left);
    b.text(Rect::new(banner.x + 14.0, banner.y + 28.0, banner.w - 24.0, 18.0), &meta, 12.5, 400, DIM, Align::Left);
    y += 52.0 + GAP;

    // ---- the Tunes: whatever height the bottom part leaves
    let bottom = GAP + TILE_H + GAP + 1.0 + GAP + 36.0 + 6.0 + 3.0 * 19.0 + GAP;
    b.label(Rect::new(PAD, y, 200.0, 36.0), &format!("Tunes · {}", m.tunes.len()));
    b.button(Rect::new(w - PAD - 96.0, y, 96.0, 36.0), "Rescan", Hit::Rescan, false);
    y += 36.0 + ROW_GAP;
    let list = Rect::new(PAD, y, cw, (h - bottom - y).max(ROW_H));
    let content = m.tunes.len().max(1) as f32 * (ROW_H + ROW_GAP) - ROW_GAP;
    b.f.tunes_max = (content - list.h).max(0.0);
    let scroll = v.tunes_scroll.clamp(0.0, b.f.tunes_max);
    b.f.items.push(Item::Clip { r: list });
    b.hit(list, Hit::TunesList);
    if m.tunes.is_empty() {
        let r = Rect::new(list.x, list.y, list.w, ROW_H);
        b.bx(r, 8.0, PANEL, Some(LINE));
        b.text(r.inset(12.0), "No Tune RT in this session", 13.0, 400, DIM, Align::Left);
    }
    for (i, t) in m.tunes.iter().enumerate() {
        let r = Rect::new(list.x, list.y + i as f32 * (ROW_H + ROW_GAP) - scroll, list.w, ROW_H);
        if !r.intersects(&list) {
            continue;
        }
        b.bx(r, 8.0, PANEL, Some(LINE));
        b.text(Rect::new(r.x + 12.0, r.y, 62.0, r.h), &t.label, 14.0, 400, FG, Align::Left);
        b.text(Rect::new(r.x + 72.0, r.y, r.w * 0.36 - 72.0, r.h), &t.name, 12.5, 400, DIM, Align::Left);
        let kx = r.x + r.w * 0.36;
        b.text(Rect::new(kx, r.y, 48.0, r.h), &t.key, 16.0, 700, FG, Align::Left);
        b.text(Rect::new(kx + 50.0, r.y, r.right() - kx - 50.0 - 80.0, r.h), &t.scale, 13.0, 400, DIM, Align::Left);
        let (badge, fg, bg) = if !t.ok {
            ("Skipped", WARN, WARN_TINT)
        } else if t.receive {
            ("Rcv", ACC, RCV_TINT)
        } else {
            ("Lock", WARN, WARN_TINT)
        };
        let br = Rect::new(r.right() - 12.0 - 64.0, r.y + 11.0, 64.0, 22.0);
        b.bx(br, 11.0, bg, None);
        b.text(br, badge, 12.0, 600, fg, Align::Center);
    }
    b.f.items.push(Item::Unclip);
    y = h - bottom + GAP;

    // ---- Apply now: key, scale, the big button
    let k = (cw - 20.0) * 0.8 / 3.2;
    let sc = (cw - 20.0) * 1.4 / 3.2;
    let go = cw - 20.0 - k - sc;
    b.tile(Rect::new(PAD, y, k, TILE_H), "Key", &key_label(v.pick_key), FG, None, Hit::KeyTile);
    b.tile(Rect::new(PAD + k + 10.0, y, sc, TILE_H), "Scale", &scale_label(v.pick_scale), FG, None, Hit::ScaleTile);
    let gr = Rect::new(PAD + k + sc + 20.0, y, go, TILE_H);
    let pressed = b.is_pressed(&Hit::ApplyNow);
    b.bx(gr, 10.0, if pressed { ACC } else { ACCD }, Some(ACC));
    b.text(gr, "Apply now", 15.0, 700, FG, Align::Center);
    b.hit(gr, Hit::ApplyNow);
    y += TILE_H + GAP;
    b.bx(Rect::new(PAD, y, cw, 1.0), 0.0, LINE, None);
    y += 1.0 + GAP;

    // ---- the log: the newest three lines
    b.label(Rect::new(PAD, y, 200.0, 36.0), "Log");
    b.button(Rect::new(w - PAD - 110.0, y, 110.0, 36.0), "Full log  ›", Hit::FullLog, false);
    y += 36.0 + 6.0;
    for l in m.log.iter().rev().take(3) {
        log_line(&mut b, Rect::new(PAD, y, cw, 19.0), l);
        y += 19.0;
    }

    if let Some(sheet) = &v.sheet {
        draw_sheet(&mut b, m, v, sheet, w, h);
    }
    b.f
}

fn log_line(b: &mut B, r: Rect, l: &tunesync::log::LogLine) {
    let color = match l.level {
        Level::Ok => OK_TEXT,
        Level::Warn => WARN,
        Level::Info => FG,
    };
    b.mono(Rect::new(r.x, r.y, 62.0, r.h), &l.stamp, 12.0, FAINT);
    b.mono(Rect::new(r.x + 64.0, r.y, r.w - 64.0, r.h), &l.text, 12.0, color);
}

/// A sheet over the page: a scrim, then a panel from the bottom with a grab bar, a title
/// and Done, and its rows (which scroll when they do not fit).
fn draw_sheet(b: &mut B, m: &Model, v: &View, sheet: &Sheet, w: f32, h: f32) {
    // Everything under the sheet stops being tappable.
    b.f.hits.clear();
    b.f.edit = None;
    b.bx(Rect::new(0.0, 0.0, w, h), 0.0, SCRIM, None);
    b.hit(Rect::new(0.0, 0.0, w, h), Hit::Scrim);
    let s = &m.settings;
    let cw = w - 2.0 * PAD;

    // Rows first (into a scratch frame), so the panel can be sized to them.
    let mut rows = B { f: Frame::default(), v };
    let mut y = 0.0f32;
    let title: String;
    let mut done = "Done";
    match sheet {
        Sheet::MidiIn => {
            title = "MIDI in".into();
            rows.option(Rect::new(0.0, y, cw, OPT_H), "None", "", s.midi_port.is_empty(), Hit::MidiPort(String::new()));
            y += OPT_H + ROW_GAP;
            let mut names = m.midi_ports.clone();
            if !s.midi_port.is_empty() && !names.contains(&s.midi_port) {
                names.push(s.midi_port.clone());
            }
            for n in &names {
                let sub = if !m.midi_ports.contains(n) {
                    "not connected"
                } else if *n == m.midi_open {
                    "open"
                } else {
                    ""
                };
                rows.option(Rect::new(0.0, y, cw, OPT_H), n, sub, *n == s.midi_port, Hit::MidiPort(n.clone()));
                y += OPT_H + ROW_GAP;
            }
            rows.text(Rect::new(0.0, y, cw, 36.0), "The port is reopened by name when it comes back.", 12.5, 400, DIM, Align::Left);
            y += 36.0;
        }
        Sheet::Rtp => {
            title = "RTP-MIDI".into();
            // the switch
            let r = Rect::new(0.0, y, cw, OPT_H);
            rows.bx(r, 8.0, PANEL, Some(LINE));
            rows.text(Rect::new(12.0, y, cw - 90.0, OPT_H), "Network session", 14.0, 400, FG, Align::Left);
            let sw = Rect::new(cw - 64.0, y + 9.0, 52.0, 30.0);
            rows.bx(sw, 15.0, if s.rtp_enabled { ACCD } else { PANEL2 }, Some(LINE));
            let knob = if s.rtp_enabled { sw.right() - 15.0 } else { sw.x + 15.0 };
            rows.f.items.push(Item::Dot { cx: knob, cy: sw.y + 15.0, radius: 12.0, color: FG, glow: false });
            rows.hit(r, Hit::RtpSwitch);
            y += OPT_H + ROW_GAP;
            // name (a native edit control sits here) and port
            let nr = Rect::new(0.0, y, cw, 44.0);
            rows.bx(nr, 8.0, PANEL, Some(LINE));
            rows.text(Rect::new(12.0, y, 110.0, 44.0), "Session name", 14.0, 400, DIM, Align::Left);
            // drawn too, for previews; on Windows the edit control covers it
            rows.text(Rect::new(130.0, y, cw - 142.0, 44.0), &s.rtp_name, 14.0, 400, FG, Align::Left);
            rows.f.edit = Some((Rect::new(124.0, y + 7.0, cw - 136.0, 30.0), s.rtp_name.clone()));
            y += 44.0 + ROW_GAP;
            let pr = Rect::new(0.0, y, cw, 44.0);
            rows.bx(pr, 8.0, if rows.is_pressed(&Hit::RtpPortField) { PRESSED } else { PANEL }, Some(LINE));
            rows.text(Rect::new(12.0, y, 110.0, 44.0), "UDP port", 14.0, 400, DIM, Align::Left);
            rows.text(Rect::new(124.0, y, cw - 160.0, 44.0), &format!("{}  (and {})", s.rtp_port, s.rtp_port as u32 + 1), 14.0, 400, FG, Align::Left);
            rows.text(Rect::new(cw - 24.0, y, 16.0, 44.0), "›", 18.0, 400, DIM, Align::Center);
            rows.hit(pr, Hit::RtpPortField);
            y += 44.0 + ROW_GAP;
            if s.rtp_enabled && !m.rtp.open && !m.rtp.error.is_empty() {
                rows.text(Rect::new(0.0, y, cw, 22.0), &m.rtp.error, 12.5, 400, WARN, Align::Left);
                y += 26.0;
            }
            rows.label(Rect::new(0.0, y, cw, 26.0), "Connected");
            y += 26.0;
            if m.rtp.peers.is_empty() {
                rows.text(Rect::new(0.0, y, cw, 30.0), if s.rtp_enabled { "No one yet" } else { "Off" }, 13.0, 400, DIM, Align::Left);
                y += 30.0 + ROW_GAP;
            }
            for p in &m.rtp.peers {
                let r = Rect::new(0.0, y, cw, OPT_H);
                rows.bx(r, 8.0, PANEL, Some(LINE));
                rows.peer_dot(16.5, y + OPT_H / 2.0, p.connected, p.midi_at_ms);
                // the name over its address (an IPv6 one is long), the state at the right
                let right = if p.invited { cw - 60.0 } else { cw - 12.0 };
                let state_w = 132.0;
                let left_w = right - state_w - 8.0 - 30.0;
                rows.text(Rect::new(30.0, y + 5.0, left_w, 22.0), &p.name, 14.0, 400, FG, Align::Left);
                rows.text(Rect::new(30.0, y + 26.0, left_w, 17.0), &p.ip, 12.0, 400, DIM, Align::Left);
                let state = if p.rtt.is_empty() { p.status.clone() } else { format!("{} · {}", p.status, p.rtt) };
                rows.text(Rect::new(right - state_w, y, state_w, OPT_H), &state, 12.5, 400, DIM, Align::Right);
                if p.invited {
                    let xr = Rect::new(cw - 52.0, y + 2.0, 48.0, OPT_H - 4.0);
                    rows.text(xr, "✕", 16.0, 400, if rows.is_pressed(&Hit::Forget(p.ip.clone(), p.port)) { FG } else { DIM }, Align::Center);
                    rows.hit(xr, Hit::Forget(p.ip.clone(), p.port));
                }
                y += OPT_H + ROW_GAP;
            }
            rows.label(Rect::new(0.0, y, cw, 26.0), "Found on the network");
            y += 26.0;
            let found: Vec<_> = m.rtp.found.iter().filter(|f| !f.joined).collect();
            if found.is_empty() {
                let t = if !s.rtp_enabled { "Off" } else if m.rtp.advert.starts_with("not available") { "Browsing needs Windows 10 1809 or later" } else { "Looking…" };
                rows.text(Rect::new(0.0, y, cw, 30.0), t, 13.0, 400, DIM, Align::Left);
                y += 30.0 + ROW_GAP;
            }
            for f in found {
                let r = Rect::new(0.0, y, cw, OPT_H);
                rows.bx(r, 8.0, PANEL, Some(LINE));
                rows.text(Rect::new(12.0, y, cw - 140.0, OPT_H), &f.name, 14.0, 400, FG, Align::Left);
                let h = Hit::Connect(f.ip.clone(), f.port, f.name.clone());
                // a compact button; the whole right end of the row takes the tap
                let br = Rect::new(cw - 96.0, y + (OPT_H - 30.0) / 2.0, 86.0, 30.0);
                rows.button_in(br, Rect::new(cw - 112.0, y, 112.0, OPT_H), "Connect", h, true);
                y += OPT_H + ROW_GAP;
            }
            let ips = if m.local_ips.is_empty() { "this PC's address".to_string() } else { m.local_ips.join(" or ") };
            for line in [
                format!("Macs list \"{}\" by themselves in Audio MIDI Setup →", s.rtp_name),
                "MIDI Network Setup → Directory. Or add it there by hand:".to_string(),
                format!("{ips}, port {}.", s.rtp_port),
            ] {
                rows.text(Rect::new(0.0, y, cw, 20.0), &line, 12.5, 400, DIM, Align::Left);
                y += 20.0;
            }
            y += 6.0;
        }
        Sheet::Channel => {
            title = "MIDI channel".into();
            let bw = (cw - 3.0 * 8.0) / 4.0;
            for c in 1..=16u8 {
                let (col, row) = ((c - 1) % 4, (c - 1) / 4);
                let r = Rect::new(col as f32 * (bw + 8.0), y + row as f32 * (OPT_H + 8.0), bw, OPT_H);
                grid_button(&mut rows, r, &c.to_string(), s.channel == c, Hit::Channel(c));
            }
            y += 4.0 * (OPT_H + 8.0);
            grid_button(&mut rows, Rect::new(0.0, y, cw, OPT_H), "Omni (any channel)", s.channel == 0, Hit::Channel(0));
            y += OPT_H + 8.0;
            rows.text(Rect::new(0.0, y, cw, 22.0), "Omni also reacts to a keyboard on the same port.", 12.5, 400, DIM, Align::Left);
            y += 28.0;
        }
        Sheet::Number(field, buf) => {
            let (t, cur, hint) = match field {
                NumberField::ScaleCc => ("Scale CC", s.scale_cc.to_string(), "0 to 119. The value of this CC picks Tune's scale (the Scale picker lists them)."),
                NumberField::RtpPort => ("UDP port", s.rtp_port.to_string(), "1024 to 65534. The next port up is used too."),
            };
            title = t.into();
            done = "Set";
            let shown = if buf.is_empty() { cur } else { buf.clone() };
            let valid = buf.is_empty() || number_command(*field, buf).is_some();
            let d = Rect::new(0.0, y, cw, 52.0);
            rows.bx(d, 8.0, PANEL, Some(if valid { LINE } else { WARN }));
            rows.text(d, &shown, 24.0, 700, if buf.is_empty() { DIM } else if valid { FG } else { WARN }, Align::Center);
            y += 52.0 + 8.0;
            let bw = (cw - 2.0 * 8.0) / 3.0;
            let keys: [(&str, Hit); 12] = [
                ("1", Hit::Digit(1)),
                ("2", Hit::Digit(2)),
                ("3", Hit::Digit(3)),
                ("4", Hit::Digit(4)),
                ("5", Hit::Digit(5)),
                ("6", Hit::Digit(6)),
                ("7", Hit::Digit(7)),
                ("8", Hit::Digit(8)),
                ("9", Hit::Digit(9)),
                ("", Hit::SheetList),
                ("0", Hit::Digit(0)),
                ("⌫", Hit::Backspace),
            ];
            for (i, (k, h)) in keys.into_iter().enumerate() {
                if k.is_empty() {
                    continue;
                }
                let r = Rect::new((i % 3) as f32 * (bw + 8.0), y + (i / 3) as f32 * (OPT_H + 8.0), bw, OPT_H);
                grid_button(&mut rows, r, k, false, h);
            }
            y += 4.0 * (OPT_H + 8.0);
            rows.text(Rect::new(0.0, y, cw, 22.0), hint, 12.5, 400, DIM, Align::Left);
            y += 28.0;
        }
        Sheet::Key => {
            title = "Key for Apply now".into();
            let bw = (cw - 3.0 * 8.0) / 4.0;
            for pc in 0..12u8 {
                let (col, row) = (pc % 4, pc / 4);
                let r = Rect::new(col as f32 * (bw + 8.0), y + row as f32 * (OPT_H + 8.0), bw, OPT_H);
                grid_button(&mut rows, r, &key_label(Some(pc)), v.pick_key == Some(pc), Hit::PickKey(Some(pc)));
            }
            y += 3.0 * (OPT_H + 8.0);
            grid_button(&mut rows, Rect::new(0.0, y, cw, OPT_H), "Keep each Tune's key (scale only)", v.pick_key.is_none(), Hit::PickKey(None));
            y += OPT_H + 8.0;
        }
        Sheet::Scale => {
            title = "Scale for Apply now".into();
            rows.text(Rect::new(0.0, y, cw, 22.0), &format!("The number is the CC {} value that selects it from the tracks.", s.scale_cc), 12.5, 400, DIM, Align::Left);
            y += 28.0;
            let colw = (cw - 8.0) / 2.0;
            let order: Vec<u8> = (1..NUM_SCALES).chain([0]).collect();
            for (i, sc) in order.iter().enumerate() {
                let r = Rect::new((i % 2) as f32 * (colw + 8.0), y + (i / 2) as f32 * (OPT_H + ROW_GAP), colw, OPT_H);
                let h = Hit::PickScale(Some(*sc));
                let on = v.pick_scale == Some(*sc);
                let fill = if rows.is_pressed(&h) { PRESSED } else if on { ACC_TINT } else { PANEL };
                rows.bx(r, 8.0, fill, Some(if on { ACC } else { LINE }));
                rows.text(Rect::new(r.x + 8.0, r.y, 26.0, r.h), &sc.to_string(), 13.0, 400, DIM, Align::Right);
                rows.text(Rect::new(r.x + 42.0, r.y, r.w - 50.0, r.h), scale_name(*sc).unwrap_or("?"), 13.5, 400, FG, Align::Left);
                rows.hit(r, h);
            }
            y += order.len().div_ceil(2) as f32 * (OPT_H + ROW_GAP);
            grid_button(&mut rows, Rect::new(0.0, y, cw, OPT_H), "Keep each Tune's scale (key only)", v.pick_scale.is_none(), Hit::PickScale(None));
            y += OPT_H + 8.0;
        }
        Sheet::Log => {
            title = "Log".into();
            rows.button(Rect::new(0.0, y, 100.0, 40.0), "Copy", Hit::CopyLog, false);
            rows.button(Rect::new(110.0, y, 100.0, 40.0), "Clear", Hit::ClearLog, false);
            y += 40.0 + 10.0;
            for l in m.log.iter().rev() {
                log_line(&mut rows, Rect::new(0.0, y, cw, 19.0), l);
                y += 19.0;
            }
            if m.log.is_empty() {
                rows.text(Rect::new(0.0, y, cw, 22.0), "Empty", 13.0, 400, DIM, Align::Left);
                y += 22.0;
            }
            y += 8.0;
        }
    }
    let content_h = y;

    // The panel: as tall as its rows, at most the window less 40 DIP; the rows scroll.
    let head = 10.0 + 5.0 + 10.0 + 30.0 + 10.0;
    let max_body = (h - 40.0 - head - 14.0).max(OPT_H);
    let body_h = content_h.min(max_body);
    let top = h - (head + body_h + 14.0);
    let panel = Rect::new(0.0, top, w, h - top);
    b.bx(panel, 14.0, SHEET, Some(LINE));
    b.bx(Rect::new(0.0, h - 14.0, w, 14.0), 0.0, SHEET, None); // square bottom corners
    b.hit(panel, Hit::SheetList); // taps on the panel's empty parts do nothing
    b.bx(Rect::new(w / 2.0 - 22.0, top + 10.0, 44.0, 5.0), 2.5, GRAB, None);
    let hy = top + 25.0;
    b.text(Rect::new(PAD, hy, cw - 100.0, 30.0), &title, 15.0, 700, FG, Align::Left);
    let dr = Rect::new(w - PAD - 90.0, hy - 7.0, 90.0, 44.0);
    b.text(dr, done, 15.0, 600, ACC, Align::Right);
    b.hit(dr, Hit::Done);

    let body = Rect::new(PAD, top + head, cw, body_h);
    b.f.sheet_max = (content_h - body_h).max(0.0);
    let scroll = v.sheet_scroll.clamp(0.0, b.f.sheet_max);
    b.f.items.push(Item::Clip { r: body });
    b.hit(body, Hit::SheetList);
    let dx = body.x;
    let dy = body.y - scroll;
    b.f.animating |= rows.f.animating;
    for it in rows.f.items {
        b.f.items.push(shift(it, dx, dy));
    }
    b.f.items.push(Item::Unclip);
    for (r, h) in rows.f.hits {
        let r = Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        if r.intersects(&body) {
            b.hit(r, h);
        }
    }
    if let Some((r, t)) = rows.f.edit {
        let r = Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        if r.y >= body.y && r.bottom() <= body.bottom() {
            b.f.edit = Some((r, t));
        }
    }
}

fn grid_button(b: &mut B, r: Rect, t: &str, on: bool, h: Hit) {
    let fill = if b.is_pressed(&h) { PRESSED } else if on { ACCD } else { PANEL };
    b.bx(r, 8.0, fill, Some(if on { ACC } else { LINE }));
    b.text(r, t, 16.0, 600, FG, Align::Center);
    b.hit(r, h);
}

fn shift(it: Item, dx: f32, dy: f32) -> Item {
    let mv = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);
    match it {
        Item::Box { r, radius, fill, stroke } => Item::Box { r: mv(r), radius, fill, stroke },
        Item::Text { r, text, size, weight, color, align, font } => Item::Text { r: mv(r), text, size, weight, color, align, font },
        Item::Dot { cx, cy, radius, color, glow } => Item::Dot { cx: cx + dx, cy: cy + dy, radius, color, glow },
        Item::Ring { cx, cy, radius, width, color } => Item::Ring { cx: cx + dx, cy: cy + dy, radius, width, color },
        Item::Clip { r } => Item::Clip { r: mv(r) },
        Item::Unclip => Item::Unclip,
    }
}
