//! Taps and drags through the view model, the way a finger would.
use tunesync::settings::Settings;
use tunesync::tune::Spelling;
use tunesync_ui::model::*;
use tunesync_ui::view::*;

fn model() -> Model {
    Model {
        settings: Settings { midi_port: "A".into(), ..Default::default() },
        midi_ports: vec!["A".into(), "B".into()],
        midi_open: "A".into(),
        tunes: (0..12).map(|i| TuneRow { label: format!("Ch {}", i + 1), key: "A".into(), scale: "Major".into(), receive: true, ok: true, ..Default::default() }).collect(),
        rtp: RtpModel {
            open: true,
            found: vec![FoundRow { name: "Studio".into(), ip: "10.0.0.7".into(), port: 5004, joined: false }],
            peers: vec![PeerRow { name: "Main".into(), ip: "10.0.0.5".into(), port: 5004, status: "now".into(), rtt: "4 ms".into(), connected: true, invited: true, midi_at_ms: 0 }],
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Taps the first region showing `h`.
fn tap(v: &mut View, m: &Model, h: &Hit) -> Vec<Command> {
    let f = layout(m, v, WIDTH, DEFAULT_HEIGHT);
    let (r, _) = f.hits.iter().rev().find(|(_, x)| x == h).unwrap_or_else(|| panic!("{h:?} is not on screen"));
    // the scrim is the whole window under the sheet: tap its top
    let (x, y) = if *h == Hit::Scrim { (r.x + 10.0, r.y + 10.0) } else { (r.x + r.w / 2.0, r.y + r.h / 2.0) };
    v.pointer_down(&f, x, y);
    let f = layout(m, v, WIDTH, DEFAULT_HEIGHT);
    v.pointer_up(&f, x, y, m)
}

#[test]
fn every_target_is_big_enough() {
    let m = model();
    let mut v = View::new();
    let check = |v: &View| {
        let f = layout(&m, v, WIDTH, DEFAULT_HEIGHT);
        for (r, h) in &f.hits {
            if matches!(h, Hit::Scrim | Hit::SheetList | Hit::TunesList | Hit::SheetGrab) {
                continue;
            }
            assert!(r.h >= 36.0 && r.w >= 44.0, "{h:?} is {}x{}", r.w, r.h);
        }
    };
    check(&v);
    for s in [Sheet::MidiIn, Sheet::Rtp, Sheet::Channel, Sheet::Key, Sheet::Scale, Sheet::Tempo, Sheet::Log, Sheet::Number(NumberField::ScaleCc, String::new())] {
        v.sheet = Some(s);
        check(&v);
    }
}

#[test]
fn settings_through_sheets() {
    let m = model();
    let mut v = View::new();
    assert!(tap(&mut v, &m, &Hit::MidiTile).is_empty());
    assert_eq!(v.sheet, Some(Sheet::MidiIn));
    assert_eq!(tap(&mut v, &m, &Hit::MidiPort("B".into())), vec![Command::SetMidiPort("B".into())]);
    assert_eq!(v.sheet, None, "choosing closes the sheet");

    tap(&mut v, &m, &Hit::ChannelTile);
    assert_eq!(tap(&mut v, &m, &Hit::Channel(0)), vec![Command::SetChannel(0)], "Omni");

    assert_eq!(tap(&mut v, &m, &Hit::Spelling(Spelling::Flat)), vec![Command::SetSpelling(Spelling::Flat)]);
    assert!(tap(&mut v, &m, &Hit::Spelling(Spelling::Sharp)).is_empty(), "already sharps");

    tap(&mut v, &m, &Hit::ScaleCcTile);
    tap(&mut v, &m, &Hit::Digit(2));
    tap(&mut v, &m, &Hit::Digit(1));
    assert_eq!(tap(&mut v, &m, &Hit::Done), vec![Command::SetScaleCc(21)]);
    tap(&mut v, &m, &Hit::ScaleCcTile);
    for d in [1, 2, 0] {
        tap(&mut v, &m, &Hit::Digit(d));
    }
    assert!(tap(&mut v, &m, &Hit::Done).is_empty(), "120 is a channel-mode CC: refused");
}

#[test]
fn rtp_sheet() {
    let m = model();
    let mut v = View::new();
    tap(&mut v, &m, &Hit::RtpTile);
    assert_eq!(tap(&mut v, &m, &Hit::RtpSwitch), vec![Command::SetRtpEnabled(true)]);
    assert_eq!(
        tap(&mut v, &m, &Hit::Connect("10.0.0.7".into(), 5004, "Studio".into())),
        vec![Command::Connect { ip: "10.0.0.7".into(), port: 5004, name: "Studio".into() }]
    );
    assert_eq!(tap(&mut v, &m, &Hit::Forget("10.0.0.5".into(), 5004)), vec![Command::Forget { ip: "10.0.0.5".into(), port: 5004 }]);
    assert!(layout(&m, &v, WIDTH, DEFAULT_HEIGHT).edit.is_some(), "the session name field is placed");
    tap(&mut v, &m, &Hit::RtpPortField);
    for d in [5, 1, 0, 4] {
        tap(&mut v, &m, &Hit::Digit(d));
    }
    assert_eq!(tap(&mut v, &m, &Hit::Done), vec![Command::SetRtpPort(5104)]);
    assert_eq!(v.sheet, Some(Sheet::Rtp), "back to the RTP sheet");
    assert!(v.back() && v.sheet.is_none(), "Escape closes the sheet");
}

/// A connected peer's dot breathes while the RTP sheet shows it, and MIDI from it sends
/// a ripple out; nothing moves (no frame timer) otherwise.
#[test]
fn a_connected_dot_moves() {
    use tunesync_ui::draw::Item;
    let mut m = model();
    let mut v = View::new();
    let rings = |f: &Frame| f.items.iter().filter(|i| matches!(i, Item::Ring { .. })).count();
    v.now_ms = 10_000;
    assert!(!layout(&m, &v, WIDTH, DEFAULT_HEIGHT).animating, "the page alone is still");
    v.sheet = Some(Sheet::Rtp);
    let f = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    assert!(f.animating && rings(&f) == 0, "connected: breathing, no ripple without MIDI");
    let halo = |f: &Frame| f.items.iter().find_map(|i| match i { Item::Dot { radius, color, glow: false, .. } if color.a() < 0xff => Some(*radius), _ => None });
    let a = halo(&f);
    v.now_ms += 1200;
    assert_ne!(halo(&layout(&m, &v, WIDTH, DEFAULT_HEIGHT)), a, "the halo changes over time");

    m.rtp.peers[0].midi_at_ms = v.now_ms - 100;
    assert_eq!(rings(&layout(&m, &v, WIDTH, DEFAULT_HEIGHT)), 1, "MIDI just arrived: a ripple");
    v.now_ms += 2000;
    assert_eq!(rings(&layout(&m, &v, WIDTH, DEFAULT_HEIGHT)), 0, "the ripple is over");

    m.rtp.peers[0].connected = false;
    assert!(!layout(&m, &v, WIDTH, DEFAULT_HEIGHT).animating, "a waiting peer's dot is still");
    m.rtp.peers[0].connected = true;
    v.now_ms = 0;
    assert!(!layout(&m, &v, WIDTH, DEFAULT_HEIGHT).animating, "no clock (previews): still");
}

#[test]
fn apply_now() {
    let m = model();
    let mut v = View::new();
    tap(&mut v, &m, &Hit::KeyTile);
    tap(&mut v, &m, &Hit::PickKey(Some(2)));
    tap(&mut v, &m, &Hit::ScaleTile);
    tap(&mut v, &m, &Hit::PickScale(None));
    assert_eq!(tap(&mut v, &m, &Hit::ApplyNow), vec![Command::ApplyNow(tunesync::midi_map::Intent { pitch_class: Some(2), scale: None })]);
    assert_eq!(tap(&mut v, &m, &Hit::Rescan), vec![Command::Rescan]);
}

#[test]
fn the_tunes_list_scrolls_and_a_drag_is_not_a_tap() {
    let m = model();
    let mut v = View::new();
    let f = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    let (r, _) = f.hits.iter().find(|(_, h)| *h == Hit::TunesList).unwrap().clone();
    let (x, y) = (r.x + 50.0, r.y + r.h - 10.0);
    v.pointer_down(&f, x, y);
    v.pointer_move(x, y - 120.0);
    let f2 = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    assert!(v.pointer_up(&f2, x, y - 120.0, &m).is_empty());
    assert!((v.tunes_scroll - 120.0).abs() < 0.5, "scrolled {}", v.tunes_scroll);
    // wheel, clamped at the end
    v.wheel(&f2, x, y - 60.0, 10_000.0);
    let content = 12.0 * 50.0 - 6.0;
    assert!((v.tunes_scroll - (content - r.h)).abs() < 0.5, "clamped at {}", v.tunes_scroll);

    // a press on a sheet row that moves becomes a scroll, not a choice
    v.sheet = Some(Sheet::Log);
    let mut m2 = m.clone();
    m2.log = (0..80).map(|i| tunesync::log::LogLine { stamp: "x".into(), level: tunesync::log::Level::Info, text: format!("line {i}") }).collect();
    let f = layout(&m2, &v, WIDTH, DEFAULT_HEIGHT);
    let (r, _) = f.hits.iter().rev().find(|(_, h)| *h == Hit::CopyLog).unwrap().clone();
    v.pointer_down(&f, r.x + 10.0, r.y + 10.0);
    v.pointer_move(r.x + 10.0, r.y - 40.0);
    let f2 = layout(&m2, &v, WIDTH, DEFAULT_HEIGHT);
    assert!(v.pointer_up(&f2, r.x + 10.0, r.y - 40.0, &m2).is_empty(), "no Copy after a drag");
}

#[test]
fn a_sheet_blocks_the_page() {
    let m = model();
    let mut v = View::new();
    v.sheet = Some(Sheet::Channel);
    let f = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    assert!(!f.hits.iter().any(|(_, h)| *h == Hit::ApplyNow), "nothing under a sheet is tappable");
    assert!(tap(&mut v, &m, &Hit::Scrim).is_empty() && v.sheet.is_none(), "tapping the scrim closes it");
}

#[test]
fn small_window_still_fits() {
    let m = model();
    let v = View::new();
    let f = layout(&m, &v, WIDTH, MIN_HEIGHT);
    let apply = f.hits.iter().find(|(_, h)| *h == Hit::ApplyNow).unwrap().0;
    assert!(apply.bottom() < MIN_HEIGHT, "Apply now is on screen at the minimum height");
}

fn texts(f: &Frame) -> Vec<String> {
    f.items.iter().filter_map(|i| if let tunesync_ui::draw::Item::Text { text, .. } = i { Some(text.clone()) } else { None }).collect()
}

/// LV1's tempo sits beside the banner; its sheet says how the tracks set it.
#[test]
fn tempo_tile_and_sheet() {
    let mut m = model();
    let mut v = View::new();
    let page = texts(&layout(&m, &v, WIDTH, DEFAULT_HEIGHT));
    assert!(page.contains(&"—".to_string()), "LV1's tempo not read yet: {page:?}");

    m.tempo = TempoModel { bpm: "128".into(), meta: "21:04:12 · RTP iPad".into(), problem: false };
    assert!(texts(&layout(&m, &v, WIDTH, DEFAULT_HEIGHT)).contains(&"128 BPM".to_string()));
    assert!(tap(&mut v, &m, &Hit::TempoTile).is_empty(), "the sheet only shows");
    assert_eq!(v.sheet, Some(Sheet::Tempo));
    let sheet = texts(&layout(&m, &v, WIDTH, DEFAULT_HEIGHT));
    for t in ["Set 21:04:12 · RTP iPad", "CC 21 = 40 to 127", "CC 22 = 0 to 127", "100 + value · 28 → 128 BPM", "MIDI clock"] {
        assert!(sheet.iter().any(|s| s == t), "{t:?} missing: {sheet:?}");
    }
    assert!(tap(&mut v, &m, &Hit::Done).is_empty());
    assert_eq!(v.sheet, None);

    // the Scale CC on 21 takes it from the tempo
    m.settings.scale_cc = 21;
    v.sheet = Some(Sheet::Tempo);
    let sheet = texts(&layout(&m, &v, WIDTH, DEFAULT_HEIGHT));
    assert!(sheet.iter().any(|s| s == "is the Scale CC: no tempo"), "{sheet:?}");
}

/// On the page, a connected input is green whether or not MIDI is arriving (gray read as
/// "not connected"); MIDI makes it ripple for a moment, then the page is still again.
#[test]
fn connected_inputs_are_green_on_the_page() {
    use tunesync_ui::draw::theme::{ACC, IDLE_DOT, WARN};
    use tunesync_ui::draw::Item;
    let mut m = model();
    m.settings.rtp_enabled = true;
    let mut v = View::new();
    v.now_ms = 10_000;
    let dots = |m: &Model, v: &View| -> Vec<_> { layout(m, v, WIDTH, DEFAULT_HEIGHT).items.iter().filter_map(|i| if let Item::Dot { color, .. } = i { Some(*color) } else { None }).collect() };
    assert_eq!(dots(&m, &v), vec![ACC, ACC], "MIDI in open + a peer connected, no MIDI for a while: both green");
    assert!(!layout(&m, &v, WIDTH, DEFAULT_HEIGHT).animating, "and still");

    m.rtp.peers[0].midi_at_ms = 9_900;
    let f = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    assert!(f.animating && f.items.iter().any(|i| matches!(i, Item::Ring { .. })), "a cue ripples the RTP-MIDI dot");

    m.rtp.peers[0].connected = false;
    m.midi_open.clear();
    assert_eq!(dots(&m, &v), vec![WARN, IDLE_DOT], "port gone: orange; no peer connected: gray");
}

/// Presses at (x, y), drags to y + dy in `steps` moves `ms` apart, and lets go.
fn drag(v: &mut View, m: &Model, x: f32, y: f32, dy: f32, steps: u32, ms: u64) -> Vec<Command> {
    let f = layout(m, v, WIDTH, DEFAULT_HEIGHT);
    v.pointer_down(&f, x, y);
    for i in 1..=steps {
        v.now_ms += ms;
        v.pointer_move(x, y + dy * i as f32 / steps as f32);
    }
    let f = layout(m, v, WIDTH, DEFAULT_HEIGHT);
    v.pointer_up(&f, x, y + dy, m)
}

fn grab_bar(v: &View, m: &Model) -> (f32, f32) {
    let f = layout(m, v, WIDTH, DEFAULT_HEIGHT);
    let (r, _) = f.hits.iter().rev().find(|(_, h)| *h == Hit::SheetGrab).expect("a grab bar").clone();
    (r.x + 30.0, r.y + 12.0)
}

/// The grab bar looks draggable: pulled down, a sheet follows and closes; a short pull
/// springs back; a quick flick closes it too.
#[test]
fn a_sheet_pulled_down_closes() {
    let m = model();
    let mut v = View::new();
    v.now_ms = 1000;

    v.sheet = Some(Sheet::Channel);
    let (x, y) = grab_bar(&v, &m);
    let top = |v: &View| layout(&m, v, WIDTH, DEFAULT_HEIGHT).hits.iter().rev().find(|(_, h)| *h == Hit::SheetGrab).unwrap().0.y;
    let t0 = top(&v);
    let f = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    v.pointer_down(&f, x, y);
    v.pointer_move(x, y + 60.0);
    assert!((top(&v) - t0 - 60.0).abs() < 0.5, "the sheet follows the finger");
    let f = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    v.now_ms += 400; // held, then let go: no flick
    assert!(v.pointer_up(&f, x, y + 60.0, &m).is_empty());
    assert_eq!((v.sheet.clone(), v.sheet_pull), (Some(Sheet::Channel), 0.0), "a short pull springs back");

    assert!(drag(&mut v, &m, x, y, 150.0, 10, 30).is_empty(), "closing sets nothing");
    assert_eq!(v.sheet, None, "pulled far: closed");

    v.sheet = Some(Sheet::Channel);
    drag(&mut v, &m, x, y, 50.0, 3, 16);
    assert_eq!(v.sheet, None, "a quick flick closes it");

    v.sheet = Some(Sheet::Channel);
    drag(&mut v, &m, x, y, -80.0, 4, 30);
    assert_eq!(v.sheet, Some(Sheet::Channel), "pushed up: stays");

    // Done is in the title row and still a tap
    assert!(tap(&mut v, &m, &Hit::Done).is_empty());
    assert_eq!(v.sheet, None);
}

/// Pulled down by its rows: at their top the sheet comes down; scrolled, the rows scroll
/// back first.
#[test]
fn a_sheet_pulled_by_its_rows() {
    let mut m = model();
    m.log = (0..80).map(|i| tunesync::log::LogLine { stamp: "x".into(), level: tunesync::log::Level::Info, text: format!("line {i}") }).collect();
    let mut v = View::new();
    v.now_ms = 1000;
    v.sheet = Some(Sheet::Log);
    let f = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    let body = f.hits.iter().rev().find(|(_, h)| *h == Hit::SheetList).unwrap().0;
    let (x, y) = (body.x + 200.0, body.y + 120.0);

    v.sheet_scroll = 300.0;
    drag(&mut v, &m, x, y, 200.0, 10, 30);
    assert_eq!(v.sheet, Some(Sheet::Log), "scrolled down: the rows scroll back, the sheet stays");
    assert!((v.sheet_scroll - 100.0).abs() < 0.5, "scroll {}", v.sheet_scroll);

    drag(&mut v, &m, x, y, 260.0, 10, 30);
    assert_eq!(v.sheet, None, "100 back to the top, then pulled 160: closed");

    // a press on a row (Copy) that pulls the sheet down closes it, and is no tap
    v.sheet = Some(Sheet::Log);
    let f = layout(&m, &v, WIDTH, DEFAULT_HEIGHT);
    let (r, _) = f.hits.iter().rev().find(|(_, h)| *h == Hit::CopyLog).unwrap().clone();
    assert!(drag(&mut v, &m, r.x + 10.0, r.y + 10.0, 140.0, 10, 30).is_empty(), "no Copy");
    assert_eq!(v.sheet, None);
}

/// Another session held the port in the settings: the RTP sheet shows the pair the session
/// moved to (and why), and the "add it by hand" lines name the port really in use.
#[test]
fn a_taken_port_shows_where_the_session_went() {
    let mut m = model();
    m.settings.rtp_enabled = true;
    m.settings.rtp_port = 5004;
    m.local_ips = vec!["192.0.2.172 (WiFi)".into()];
    m.rtp.port = 5006;
    let mut v = View::new();
    tap(&mut v, &m, &Hit::RtpTile);
    let t = texts(&layout(&m, &v, WIDTH, 2000.0));
    assert!(t.iter().any(|s| s == "5006  (and 5007)  ·  5004 was taken"), "{t:?}");
    assert!(t.iter().any(|s| s == "192.0.2.172 (WiFi), port 5006"), "{t:?}");
    m.rtp.port = 5004;
    let t = texts(&layout(&m, &v, WIDTH, 2000.0));
    assert!(t.iter().any(|s| s == "5004  (and 5005)") && t.iter().any(|s| s == "192.0.2.172 (WiFi), port 5004"), "{t:?}");
}
