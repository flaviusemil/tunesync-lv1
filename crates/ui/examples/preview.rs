//! Writes the window's states as SVG, with sample data like docs/settings-onepage.html:
//!   cargo run -p tunesync-ui --example preview -- <out dir>
//! then e.g. `rsvg-convert -z 2 page.svg > page.png`.
use tunesync::log::{Level, LogLine};
use tunesync::settings::Settings;
use tunesync_ui::model::*;
use tunesync_ui::svg::to_svg;
use tunesync_ui::view::*;

pub fn sample() -> Model {
    let line = |s: &str, l: Level, t: &str| LogLine { stamp: s.into(), level: l, text: t.into() };
    Model {
        version: "0.1.0".into(),
        settings: Settings { midi_port: "MIDISPORT 2x2 In A".into(), rtp_enabled: true, ..Default::default() },
        midi_ports: vec!["MIDISPORT 2x2 In A".into(), "WING USB MIDI".into()],
        midi_open: "MIDISPORT 2x2 In A".into(),
        midi_active: true,
        rtp: RtpModel {
            open: true,
            active: true,
            advert: "advertised".into(),
            peers: vec![
                PeerRow { name: "Playback-Mac".into(), ip: "192.0.2.41".into(), port: 5004, status: "now".into(), rtt: "3 ms".into(), connected: true, invited: false, midi_at_ms: 0 },
                PeerRow { name: "Playback-Backup".into(), ip: "192.0.2.42".into(), port: 5004, status: "3 s ago".into(), rtt: "5 ms".into(), connected: true, invited: true, midi_at_ms: 0 },
            ],
            found: vec![FoundRow { name: "Studio-MacBook".into(), ip: "192.0.2.77".into(), port: 5004, joined: false }],
            ..Default::default()
        },
        last: Some(LastChange { label: "A Natural Minor".into(), meta: "21:04:12 · RTP Playback-Mac · set 2, 1 locked".into(), problem: false }),
        tunes: vec![
            TuneRow { label: "Ch 43".into(), name: "Vox 1".into(), key: "A".into(), scale: "Natural Minor".into(), receive: true, ok: true },
            TuneRow { label: "Ch 44".into(), name: "Vox 2".into(), key: "A".into(), scale: "Natural Minor".into(), receive: true, ok: true },
            TuneRow { label: "Ch 45".into(), name: "BV".into(), key: "E♭".into(), scale: "Major".into(), receive: false, ok: true },
        ],
        log: vec![
            line("21:04:12", Level::Info, "RTP Playback-Mac Note On 57 (A) vel 100 ch 16"),
            line("21:04:12", Level::Info, "RTP Playback-Mac CC 20 = 3 (Natural Minor) ch 16"),
            line("21:04:12", Level::Ok, "A Natural Minor: set 2 (Ch 43 Vox 1 -> A, Ch 44 Vox 2 -> A), skipped 1 locked (Ch 45 BV)"),
        ],
        lv1_problem: None,
        local_ips: vec!["192.0.2.172".into()],
    }
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let m = sample();
    let (w, h) = (WIDTH, DEFAULT_HEIGHT);
    let mut v = View::new();
    let write = |name: &str, v: &View| {
        let f = layout(&m, v, w, h);
        std::fs::write(dir.join(format!("{name}.svg")), to_svg(&f, w, h)).unwrap();
    };
    write("page", &v);
    for (name, sheet) in [
        ("sheet-rtp", Sheet::Rtp),
        ("sheet-midi", Sheet::MidiIn),
        ("sheet-channel", Sheet::Channel),
        ("sheet-scale", Sheet::Scale),
        ("sheet-key", Sheet::Key),
        ("sheet-cc", Sheet::Number(NumberField::ScaleCc, "2".into())),
        ("sheet-log", Sheet::Log),
    ] {
        v.sheet = Some(sheet);
        write(name, &v);
    }
}
