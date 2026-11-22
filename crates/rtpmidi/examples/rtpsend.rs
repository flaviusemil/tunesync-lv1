//! rtpsend - a stand-in playback rig: joins a TuneSync network session over RTP-MIDI
//! (AppleMIDI) and sends cues, for tests without a Mac.
//!
//! ```text
//! cargo run -p rtpmidi --example rtpsend -- [--name N] [--port P] [--ch C] [--stay ms] <host> <hostPort> <cue>...
//!   cue:  note <n>       Note On n vel 100 (then Note Off), channel --ch (default 16)
//!         cc <n> <v>     Control Change
//!         key <name>     the note for a key name (C, C#, Db, ... B)
//!         wait <ms>
//! ```
//! Exit code 0 when the session connected and every cue went out.
use std::process::exit;
use std::time::{Duration, Instant};

use rtpmidi::session::{PeerState, Session, SessionConfig};

fn key_note(k: &str) -> Option<u8> {
    const SHARPS: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    const FLATS: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
    (0..12).find(|&i| SHARPS[i] == k || FLATS[i] == k).map(|i| 60 + i as u8)
}

fn main() {
    let (mut name, mut port, mut ch, mut stay) = ("rtpsend".to_string(), 5014u16, 16u8, 1000u64);
    let mut a = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        match s.as_str() {
            "--name" => name = it.next().unwrap_or_default(),
            "--port" => port = it.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            "--ch" => ch = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--stay" => stay = it.next().and_then(|v| v.parse().ok()).unwrap_or(stay),
            _ => a.push(s),
        }
    }
    if a.len() < 2 || !(1..=16).contains(&ch) {
        eprintln!("usage: rtpsend [--name N] [--port P] [--ch C] [--stay ms] <host> <hostPort> <cue>...");
        exit(2);
    }
    let mut s = Session::new();
    s.on_log(|l, detail| {
        if !detail {
            println!("  {l}")
        }
    });
    if let Err(e) = s.open(SessionConfig { name, port, ..Default::default() }) {
        eprintln!("open: {e}");
        exit(1);
    }
    s.invite(&a[0], a[1].parse().unwrap_or(5004), "");
    let connected = |s: &Session| s.peers().iter().any(|p| p.state == PeerState::Connected);
    let end = Instant::now() + Duration::from_secs(15);
    while !connected(&s) && Instant::now() < end {
        s.poll(20);
    }
    if !connected(&s) {
        eprintln!("no session with {}:{}", a[0], a[1]);
        exit(1);
    }
    let pump = |s: &Session, ms: u64| {
        let e = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < e {
            s.poll(5);
        }
    };
    pump(&s, 300);
    let st = ch - 1;
    let mut rc = 0;
    let mut i = 2;
    while i < a.len() {
        let arg = |k: usize| a.get(i + k).cloned().unwrap_or_default();
        match a[i].as_str() {
            "note" | "key" => {
                let n = if a[i] == "note" { arg(1).parse::<u8>().ok().filter(|n| *n < 128) } else { key_note(&arg(1)) };
                let Some(n) = n else {
                    eprintln!("bad note {}", arg(1));
                    exit(2);
                };
                println!("send Note On {n} ch {ch}");
                if s.send(&[&[0x90 | st, n, 100]]) == 0 {
                    rc = 1;
                }
                pump(&s, 100);
                s.send(&[&[0x80 | st, n, 0]]);
                i += 2;
            }
            "cc" => {
                let (c, v) = (arg(1).parse::<u8>().unwrap_or(0) & 0x7F, arg(2).parse::<u8>().unwrap_or(0) & 0x7F);
                println!("send CC {c} = {v} ch {ch}");
                if s.send(&[&[0xB0 | st, c, v]]) == 0 {
                    rc = 1;
                }
                i += 3;
            }
            "wait" => {
                pump(&s, arg(1).parse().unwrap_or(0));
                i += 2;
                continue;
            }
            other => {
                eprintln!("bad cue {other}");
                exit(2);
            }
        }
        pump(&s, 20);
    }
    pump(&s, stay);
    s.close();
    exit(rc);
}
