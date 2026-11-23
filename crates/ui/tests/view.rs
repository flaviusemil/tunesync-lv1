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
            peers: vec![PeerRow { name: "Main".into(), ip: "10.0.0.5".into(), port: 5004, status: "now".into(), rtt: "4 ms".into(), connected: true, invited: true }],
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
            if matches!(h, Hit::Scrim | Hit::SheetList | Hit::TunesList) {
                continue;
            }
            assert!(r.h >= 36.0 && r.w >= 44.0, "{h:?} is {}x{}", r.w, r.h);
        }
    };
    check(&v);
    for s in [Sheet::MidiIn, Sheet::Rtp, Sheet::Channel, Sheet::Key, Sheet::Scale, Sheet::Log, Sheet::Number(NumberField::ScaleCc, String::new())] {
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
