//! The codec (golden bytes and a fuzz, from Headlink's preprobe_test) and the multi-peer
//! session on loopback UDP. The fake peers are written from the wire format with raw
//! bytes, NOT with the codec, so a mistake in the codec cannot cancel itself out on both
//! ends. Ported from the C++ net_test (branch feature/windows-shell).
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rtpmidi::codec::*;
use rtpmidi::session::*;

// ---------------------------------------------------------------- codec

#[test]
fn session_packets() {
    assert_eq!(am_invite(AmCmd::In, 0x01020304, 0x0A0B0C0D, "abc"), [0xFF, 0xFF, b'I', b'N', 0, 0, 0, 2, 1, 2, 3, 4, 0x0A, 0x0B, 0x0C, 0x0D, b'a', b'b', b'c', 0]);
    assert_eq!(am_invite(AmCmd::Ok, 0x01020304, 0x0A0B0C0D, ""), [0xFF, 0xFF, b'O', b'K', 0, 0, 0, 2, 1, 2, 3, 4, 0x0A, 0x0B, 0x0C, 0x0D, 0]);
    assert_eq!(am_bye(0x01020304, 0x0A0B0C0D), [0xFF, 0xFF, b'B', b'Y', 0, 0, 0, 2, 1, 2, 3, 4, 0x0A, 0x0B, 0x0C, 0x0D]);
    // RS carries a 32-bit extended sequence number, as Apple (and rtpmidid) do.
    assert_eq!(am_feedback(0x0A0B0C0D, 0x0001BEEF), [0xFF, 0xFF, b'R', b'S', 0x0A, 0x0B, 0x0C, 0x0D, 0x00, 0x01, 0xBE, 0xEF]);
    assert_eq!(
        am_clock(0x0A0B0C0D, 1, 0x0102030405060708, 0x1112131415161718, 0),
        [0xFF, 0xFF, b'C', b'K', 0x0A, 0x0B, 0x0C, 0x0D, 1, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    let inv = am_invite(AmCmd::In, 0xCAFEBABE, 0x12345678, "Playback");
    let a = am_parse(&inv).unwrap();
    assert_eq!((a.cmd, a.version, a.token, a.ssrc, a.name.as_str()), (AmCmd::In, 2, 0xCAFEBABE, 0x12345678, "Playback"));
    assert_eq!(am_parse(&inv[..inv.len() - 1]).unwrap().name, "Playback", "a name without its NUL");
    assert!(am_parse(&inv[..16]).unwrap().name.is_empty(), "no name");
    assert!(am_parse(&inv[..15]).is_none(), "short IN");
    let ck = am_clock(1, 2, 100, 200, 300);
    let a = am_parse(&ck).unwrap();
    assert_eq!((a.cmd, a.count, a.ts), (AmCmd::Ck, 2, [100, 200, 300]));
    assert!(am_parse(&ck[..35]).is_none());
    // What rtpmidid sent in the preprobe interop run: seq 2 in the low half.
    assert_eq!(am_parse(&[0xFF, 0xFF, 0x52, 0x53, 0x00, 0x00, 0x5C, 0x35, 0x00, 0x00, 0x00, 0x02]).unwrap().seq, 2);
    let rl = am_parse(&[0xFF, 0xFF, b'R', b'L', 0, 0, 0, 7, 0, 0, 0x10, 0]).unwrap();
    assert_eq!((rl.cmd, rl.limit), (AmCmd::Rl, 0x1000));
    let zz = am_parse(&[0xFF, 0xFF, b'Z', b'Z', 1, 2]).unwrap();
    assert_eq!((zz.cmd, zz.raw), (AmCmd::Unknown, *b"ZZ"));
}

fn rtp(section: &[u8]) -> Vec<u8> {
    let mut v = vec![0x80, 0x61, 0x00, 0x05, 0, 0, 0x10, 0, 0x50, 0x52, 0x45, 0x21];
    v.extend_from_slice(section);
    v
}

#[test]
fn rtp_midi() {
    assert_eq!(rtp_build(0x1234, 0x01020304, 0xAABBCCDD, &[&[0x9F, 0x39, 0x64]]), [0x80, 0x61, 0x12, 0x34, 1, 2, 3, 4, 0xAA, 0xBB, 0xCC, 0xDD, 0x03, 0x9F, 0x39, 0x64]);
    assert_eq!(rtp_build(1, 2, 3, &[&[0xBF, 20, 3], &[0x9F, 57, 100]]), [0x80, 0x61, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0x07, 0xBF, 20, 3, 0x00, 0x9F, 57, 100]);
    let six: Vec<&[u8]> = vec![&[0xB0, 0x04, 0x00]; 6];
    let p = rtp_build(0, 0, 0, &six);
    assert_eq!((p.len(), p[12], p[13]), (12 + 2 + 23, 0x80, 23), "past 15 bytes the header grows to two bytes");
    let (m, r) = rtp_parse(&p);
    assert!(r.is_ok() && m.events.len() == 6 && m.b && m.list_len == 23);

    // What macOS sends: a journal on every packet.
    let (m, r) = rtp_parse(&rtp(&[0x43, 0xB0, 0x49, 0x0F, 0x20, 0x00, 0x04, 0x00, 0x08, 0x0A, 0x80, 0x49, 0x0F]));
    assert!(r.is_ok() && m.j && !m.z && m.events.len() == 1 && m.journal_len == 9 && m.seq == 5 && m.ts == 0x1000 && m.ssrc == 0x50524521);
    assert_eq!(m.events[0].bytes, [0xB0, 0x49, 0x0F]);
    let (m, _) = rtp_parse(&rtp(&[0x25, 0x81, 0x00, 0xB0, 0x07, 0x64]));
    assert!(m.z && m.events[0].delta == 128, "Z: the first command has a delta too");
    let (m, r) = rtp_parse(&rtp(&[0x06, 0xB0, 0x07, 0x64, 0x00, 0x08, 0x65]));
    assert!(r.is_ok() && m.events.len() == 2 && m.events[1].running && m.events[1].bytes == [0xB0, 0x08, 0x65], "running status");
    let (m, r) = rtp_parse(&rtp(&[0x08, 0xB0, 0x07, 0x64, 0x00, 0xF8, 0x00, 0x08, 0x65]));
    assert!(r.is_ok() && m.events.len() == 3 && m.events[1].bytes == [0xF8] && m.events[2].bytes == [0xB0, 0x08, 0x65], "realtime keeps running status");
    let (m, r) = rtp_parse(&rtp(&[0x09, 0xB0, 0x07, 0x64, 0x00, 0xF1, 0x10, 0x00, 0x08, 0x65]));
    assert!(r.is_err() && m.events.len() == 2, "system common clears it");
    let (m, r) = rtp_parse(&rtp(&[0x06, 0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]));
    assert!(r.is_ok() && m.events.len() == 1 && m.events[0].bytes.len() == 6, "SysEx");
    let (m, r) = rtp_parse(&rtp(&[0x40, 0x20, 0x00, 0x04]));
    assert!(r.is_ok() && m.events.is_empty() && m.journal_len == 3, "journal-only packet");
    for bad in [
        rtp(&[0x05, 0xB0, 0x07, 0x64]),
        rtp(&[0x03, 0x07, 0x64, 0x00]),
        rtp(&[0x03, 0xB0, 0x07]),
        rtp(&[0x04, 0xB0, 0x07, 0x64, 0x80]),
        rtp(&[0x04, 0xF0, 0x01, 0x02, 0x03]),
        rtp(&[0x03, 0xB0, 0x87, 0x64]),
        rtp(&[]),
        vec![0x80, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00],
        vec![0x80, 0x61, 0, 0],
    ] {
        assert!(rtp_parse(&bad).1.is_err(), "{}", hex(&bad));
    }
}

#[test]
fn fuzz() {
    // Random and mutated packets: must never panic, and every event starts with a status byte.
    let mut x: u64 = 1234;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let seed = rtp(&[0x46, 0xB0, 0x07, 0x64, 0x00, 0x08, 0x65, 0x20, 0x00, 0x04]);
    for it in 0..200_000 {
        let mut b = seed.clone();
        if it & 1 == 1 {
            b.resize((rnd() % 40) as usize, 0);
            for c in b.iter_mut() {
                *c = rnd() as u8;
            }
            if b.len() > 2 {
                b[0] = 0x80;
                b[1] = 0x61;
            }
        } else {
            for _ in 0..3 {
                let i = 12 + (rnd() as usize) % (b.len() - 12);
                b[i] = rnd() as u8;
            }
            if rnd() % 4 == 0 {
                let n = (rnd() as usize) % b.len();
                b.truncate(n);
            }
        }
        let (m, _) = rtp_parse(&b);
        assert!(m.events.iter().all(|e| !e.bytes.is_empty() && e.bytes[0] & 0x80 != 0));
        if b.len() > 1 {
            b[0] = 0xFF;
            b[1] = 0xFF;
        }
        let _ = am_parse(&b);
    }
}

// ---------------------------------------------------------------- loopback helpers

struct Sock {
    s: UdpSocket,
    port: u16,
}

// Tests run in parallel threads: every port pair comes from one counter, so a pair
// checked free for one test is never handed to another.
static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(40004);

fn next_ports() -> impl Iterator<Item = u16> {
    std::iter::from_fn(|| Some(NEXT_PORT.fetch_add(2, Ordering::SeqCst))).take_while(|p| *p < 60000)
}

impl Sock {
    fn pair() -> (Sock, Sock) {
        for p in next_ports() {
            let (Ok(c), Ok(d)) = (UdpSocket::bind(("127.0.0.1", p)), UdpSocket::bind(("127.0.0.1", p + 1))) else { continue };
            c.set_nonblocking(true).unwrap();
            d.set_nonblocking(true).unwrap();
            return (Sock { s: c, port: p }, Sock { s: d, port: p + 1 });
        }
        panic!("no free loopback port pair");
    }
    fn send(&self, to: u16, b: &[u8]) {
        self.s.send_to(b, SocketAddrV4::new(Ipv4Addr::LOCALHOST, to)).unwrap();
    }
    fn recv(&self) -> Option<(Vec<u8>, u16)> {
        let mut buf = [0u8; 2048];
        self.s.recv_from(&mut buf).ok().map(|(n, a)| (buf[..n].to_vec(), a.port()))
    }
}

fn free_pair() -> u16 {
    let (c, _d) = Sock::pair();
    c.port
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn be64(b: &[u8], o: usize) -> u64 {
    (be32(b, o) as u64) << 32 | be32(b, o + 4) as u64
}
fn is(b: &[u8], cmd: &[u8; 2]) -> bool {
    b.len() >= 4 && b[0] == 0xFF && b[1] == 0xFF && b[2] == cmd[0] && b[3] == cmd[1]
}
fn session_pkt(cmd: &[u8; 2], token: u32, ssrc: u32, name: Option<&str>) -> Vec<u8> {
    let mut r = vec![0xFF, 0xFF, cmd[0], cmd[1]];
    r.extend_from_slice(&2u32.to_be_bytes());
    r.extend_from_slice(&token.to_be_bytes());
    r.extend_from_slice(&ssrc.to_be_bytes());
    if let Some(n) = name {
        r.extend_from_slice(n.as_bytes());
        r.push(0);
    }
    r
}

/// A playback rig that invites us (what a Mac's network session does), byte by byte.
struct FakeInitiator {
    ctl: Sock,
    data: Sock,
    token: u32,
    ssrc: u32,
    name: &'static str,
    seq: u16,
    ok_ctl: bool,
    ok_data: bool,
    refused: bool,
    ck1_ts1: Vec<u64>,
    feedback: Vec<u32>,
}

impl FakeInitiator {
    fn new(token: u32, ssrc: u32, name: &'static str) -> Self {
        let (ctl, data) = Sock::pair();
        FakeInitiator { ctl, data, token, ssrc, name, seq: 100, ok_ctl: false, ok_data: false, refused: false, ck1_ts1: vec![], feedback: vec![] }
    }
    fn step(&mut self) {
        while let Some((b, _)) = self.ctl.recv() {
            if is(&b, b"OK") && be32(&b, 8) == self.token {
                self.ok_ctl = true;
            } else if is(&b, b"NO") {
                self.refused = true;
            } else if is(&b, b"RS") && b.len() >= 12 {
                self.feedback.push(be32(&b, 8));
            }
        }
        while let Some((b, _)) = self.data.recv() {
            if is(&b, b"OK") && be32(&b, 8) == self.token {
                self.ok_data = true;
            } else if is(&b, b"NO") {
                self.refused = true;
            } else if is(&b, b"CK") && b.len() == 36 && b[8] == 1 {
                self.ck1_ts1.push(be64(&b, 12));
            }
        }
    }
    fn invite_ctl(&self, sp: u16) {
        self.ctl.send(sp, &session_pkt(b"IN", self.token, self.ssrc, Some(self.name)));
    }
    fn invite_data(&self, sp: u16) {
        self.data.send(sp + 1, &session_pkt(b"IN", self.token, self.ssrc, Some(self.name)));
    }
    fn ck0(&self, sp: u16, ts1: u64) {
        let mut r = vec![0xFF, 0xFF, b'C', b'K'];
        r.extend_from_slice(&self.ssrc.to_be_bytes());
        r.extend_from_slice(&[0, 0, 0, 0]);
        r.extend_from_slice(&ts1.to_be_bytes());
        r.extend_from_slice(&[0; 16]);
        self.data.send(sp + 1, &r);
    }
    fn midi(&mut self, sp: u16, msg: &[u8]) {
        let mut r = vec![0x80, 0x61];
        r.extend_from_slice(&self.seq.to_be_bytes());
        r.extend_from_slice(&4242u32.to_be_bytes());
        r.extend_from_slice(&self.ssrc.to_be_bytes());
        r.push(0x40 | msg.len() as u8); // J: a journal follows, as macOS sends on every packet
        r.extend_from_slice(msg);
        r.extend_from_slice(&[0x20, 0x00, 0x04]);
        self.data.send(sp + 1, &r);
        self.seq += 1;
    }
    fn bye(&self, sp: u16) {
        self.ctl.send(sp, &session_pkt(b"BY", self.token, self.ssrc, None));
    }
}

/// A session that answers our invitations (a Mac we Connect to), byte by byte.
struct FakeResponder {
    ctl: Sock,
    data: Sock,
    ssrc: u32,
    refuse: bool,
    silent: bool,
    token: u32,
    their_name: String,
    their_ctl: u16,
    their_data: u16,
    ctl_ok: bool,
    data_ok: bool,
    ck2: bool,
    bye: bool,
    midi_in: Vec<Vec<u8>>,
}

impl FakeResponder {
    fn new() -> Self {
        let (ctl, data) = Sock::pair();
        FakeResponder {
            ctl,
            data,
            ssrc: 0x4D41430A,
            refuse: false,
            silent: false,
            token: 0,
            their_name: String::new(),
            their_ctl: 0,
            their_data: 0,
            ctl_ok: false,
            data_ok: false,
            ck2: false,
            bye: false,
            midi_in: vec![],
        }
    }
    fn step(&mut self) {
        while let Some((b, from)) = self.ctl.recv() {
            if self.silent {
                continue;
            }
            if is(&b, b"IN") && b.len() >= 16 {
                self.token = be32(&b, 8);
                self.their_ctl = from;
                self.their_name = String::from_utf8_lossy(&b[16..]).trim_end_matches('\0').to_string();
                let cmd = if self.refuse { b"NO" } else { b"OK" };
                self.ctl.send(from, &session_pkt(cmd, self.token, self.ssrc, Some("Studio-Mac")));
                self.ctl_ok = !self.refuse;
            } else if is(&b, b"BY") {
                self.bye = true;
            }
        }
        while let Some((b, from)) = self.data.recv() {
            if self.silent {
                continue;
            }
            if is(&b, b"IN") && b.len() >= 16 && be32(&b, 8) == self.token && self.ctl_ok {
                self.their_data = from;
                self.data.send(from, &session_pkt(b"OK", self.token, self.ssrc, Some("Studio-Mac")));
                self.data_ok = true;
            } else if is(&b, b"CK") && b.len() == 36 {
                if b[8] == 0 {
                    let mut r = vec![0xFF, 0xFF, b'C', b'K'];
                    r.extend_from_slice(&self.ssrc.to_be_bytes());
                    r.extend_from_slice(&[1, 0, 0, 0]);
                    r.extend_from_slice(&be64(&b, 12).to_be_bytes());
                    r.extend_from_slice(&777777u64.to_be_bytes());
                    r.extend_from_slice(&0u64.to_be_bytes());
                    self.data.send(from, &r);
                } else if b[8] == 2 {
                    self.ck2 = true;
                }
            } else if b.len() >= 13 && b[0] == 0x80 && b[1] == 0x61 {
                self.midi_in.push(b[12..].to_vec());
            }
        }
    }
    fn send_bye(&self) {
        self.ctl.send(self.their_ctl, &session_pkt(b"BY", self.token, self.ssrc, None));
    }
}

/// Runs `f` (a step that answers "done?") until it says so or `ms` pass.
fn until(ms: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if f() {
            return true;
        }
    }
    false
}

type Got = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
type Log = Arc<Mutex<Vec<String>>>;

fn session_with(clock: Option<Arc<AtomicU64>>) -> (Session, Got, Log, SessionConfig) {
    let got: Got = Arc::default();
    let log: Log = Arc::default();
    let mut s = Session::new();
    let g2 = got.clone();
    s.on_midi(move |p, m| g2.lock().unwrap().push((p.display(), m.to_vec())));
    let l2 = log.clone();
    s.on_log(move |l, _| l2.lock().unwrap().push(l.to_string()));
    let mut c = SessionConfig { port: free_pair(), ..Default::default() };
    if let Some(t) = clock {
        c.clock10k = Some(Arc::new(move || t.load(Ordering::SeqCst)));
    }
    (s, got, log, c)
}

// ---------------------------------------------------------------- session tests

/// Main and backup rig invite us: both connected, MIDI from each named, clock sync and
/// feedback answered, a stranger's MIDI refused, BY and silence drop a peer.
#[test]
fn two_rigs_invite_us() {
    let fake = Arc::new(AtomicU64::new(1000));
    let (s, got, log, c) = session_with(Some(fake.clone()));
    s.open(c).unwrap();
    let sp = s.port();
    let mut main = FakeInitiator::new(0x11110001, 0xAAAA0001, "Playback-Main");
    let mut backup = FakeInitiator::new(0x22220002, 0xBBBB0002, "Playback-Backup");
    main.invite_ctl(sp);
    backup.invite_ctl(sp);
    let step = |main: &mut FakeInitiator, backup: &mut FakeInitiator| {
        s.poll(2);
        main.step();
        backup.step();
    };
    let end = Instant::now() + Duration::from_secs(1);
    while !(main.ok_ctl && backup.ok_ctl) && Instant::now() < end {
        step(&mut main, &mut backup);
    }
    assert!(main.ok_ctl && backup.ok_ctl, "both control invitations accepted");
    main.invite_data(sp);
    backup.invite_data(sp);
    let end = Instant::now() + Duration::from_secs(1);
    while !(main.ok_data && backup.ok_data) && Instant::now() < end {
        step(&mut main, &mut backup);
    }
    assert!(main.ok_data && backup.ok_data, "both data invitations accepted");
    let ps = s.peers();
    assert_eq!(ps.len(), 2);
    assert!(ps.iter().all(|p| p.state == PeerState::Connected && !p.invited));
    assert_eq!((ps[0].name.as_str(), ps[1].name.as_str()), ("Playback-Main", "Playback-Backup"));

    // Clock sync started by the rig: CK1 carries its ts1 back.
    main.ck0(sp, 5555);
    let end = Instant::now() + Duration::from_secs(1);
    while main.ck1_ts1.is_empty() && Instant::now() < end {
        step(&mut main, &mut backup);
    }
    assert_eq!(main.ck1_ts1.first(), Some(&5555), "CK0 answered");

    // MIDI from each, named by its peer.
    main.midi(sp, &[0xBF, 20, 3]);
    backup.midi(sp, &[0x9F, 57, 100]);
    let end = Instant::now() + Duration::from_secs(1);
    while got.lock().unwrap().len() < 2 && Instant::now() < end {
        step(&mut main, &mut backup);
    }
    assert_eq!(
        *got.lock().unwrap(),
        vec![("Playback-Main".to_string(), vec![0xBF, 20, 3]), ("Playback-Backup".to_string(), vec![0x9F, 57, 100])],
        "who sent what"
    );

    // Receiver feedback names each rig's last sequence, on its control port.
    fake.fetch_add(15000, Ordering::SeqCst);
    let end = Instant::now() + Duration::from_secs(1);
    while (main.feedback.is_empty() || backup.feedback.is_empty()) && Instant::now() < end {
        step(&mut main, &mut backup);
    }
    assert_eq!(main.feedback.last(), Some(&100), "RS = the last seq");
    assert!(!backup.feedback.is_empty());

    // A stranger (right address, an SSRC nobody joined with): ignored, logged once.
    let mut stranger = vec![0x80, 0x61, 0, 1, 0, 0, 0, 1];
    stranger.extend_from_slice(&0xDEADBEEFu32.to_be_bytes());
    stranger.extend_from_slice(&[0x03, 0x9F, 60, 100]);
    main.data.send(sp + 1, &stranger);
    main.data.send(sp + 1, &stranger);
    let end = Instant::now() + Duration::from_millis(60);
    while Instant::now() < end {
        step(&mut main, &mut backup);
    }
    assert_eq!(got.lock().unwrap().len(), 2, "stranger ignored");
    assert_eq!(log.lock().unwrap().iter().filter(|l| l.contains("0xDEADBEEF")).count(), 1, "logged once");

    // A data invitation with no control one first is refused.
    let mut rogue = FakeInitiator::new(0x33330003, 0xCCCC0003, "Rogue");
    rogue.invite_data(sp);
    assert!(until(1000, || { s.poll(2); rogue.step(); rogue.refused }), "data IN without control refused");

    // The backup says goodbye: gone. The main rig stays.
    backup.bye(sp);
    let end = Instant::now() + Duration::from_secs(1);
    while s.peers().len() != 1 && Instant::now() < end {
        step(&mut main, &mut backup);
    }
    assert_eq!(s.peers().iter().map(|p| p.name.clone()).collect::<Vec<_>>(), ["Playback-Main"]);

    // The main rig goes quiet for 60 s: dropped.
    fake.fetch_add(61 * 10000, Ordering::SeqCst);
    let end = Instant::now() + Duration::from_millis(500);
    while !s.peers().is_empty() && Instant::now() < end {
        step(&mut main, &mut backup);
    }
    assert!(s.peers().is_empty(), "silent peer dropped");
    assert!(log.lock().unwrap().iter().any(|l| l.contains("nothing from Playback-Main")));
}

#[test]
fn max_peers() {
    let (s, _, _, mut c) = session_with(None);
    c.max_peers = 1;
    s.open(c).unwrap();
    let (mut a, mut b) = (FakeInitiator::new(1, 0xA1, "A"), FakeInitiator::new(2, 0xB2, "B"));
    a.invite_ctl(s.port());
    b.invite_ctl(s.port());
    let end = Instant::now() + Duration::from_secs(1);
    while !(a.ok_ctl && b.refused) && Instant::now() < end {
        s.poll(2);
        a.step();
        b.step();
    }
    assert!(a.ok_ctl && b.refused, "one in, one refused");
}

/// We invite a Mac (Connect): both ports, clock sync, MIDI out; BY makes us wait 30 s;
/// forget says goodbye; refused and silent peers are invited again later.
#[test]
fn we_invite() {
    let fake = Arc::new(AtomicU64::new(1000));
    let (s, _, _, c) = session_with(Some(fake.clone()));
    s.open(c).unwrap();
    let mut mac = FakeResponder::new();
    s.invite("127.0.0.1", mac.ctl.port, "Studio-MacBook");
    assert!(until(2000, || { s.poll(2); mac.step(); mac.data_ok }), "both ports invited");
    assert_eq!((mac.their_ctl, mac.their_data, mac.their_name.as_str()), (s.port(), s.port() + 1, "TuneSync"));
    let connected = |s: &Session| s.peers().first().map(|p| p.state) == Some(PeerState::Connected);
    assert!(until(500, || { s.poll(2); connected(&s) }));
    let p = &s.peers()[0];
    assert!(p.invited && p.name == "Studio-Mac" && p.label == "Studio-MacBook");
    assert!(until(1000, || { s.poll(2); mac.step(); mac.ck2 }), "we ran clock sync (CK0, CK1, CK2)");
    assert_eq!(s.send(&[&[0x9F, 57, 100]]), 1);
    assert!(until(1000, || { s.poll(2); mac.step(); !mac.midi_in.is_empty() }));
    assert_eq!(mac.midi_in[0], [0x03, 0x9F, 57, 100]);

    // It says goodbye: we wait 30 s, then invite again.
    mac.ctl_ok = false;
    mac.data_ok = false;
    mac.send_bye();
    assert!(until(1000, || { s.poll(2); mac.step(); s.peers()[0].state == PeerState::Waiting }), "waiting after BY");
    fake.fetch_add(10 * 10000, Ordering::SeqCst);
    until(30, || { s.poll(2); mac.step(); false });
    assert!(!mac.ctl_ok, "not invited again after 10 s");
    fake.fetch_add(25 * 10000, Ordering::SeqCst);
    assert!(until(1000, || { s.poll(2); mac.step(); mac.data_ok }), "invited again after 30 s");

    // Forget: BY, gone, not invited again.
    s.forget("127.0.0.1", mac.ctl.port);
    assert!(until(1000, || { s.poll(2); mac.step(); mac.bye }), "forget sends BY");
    assert!(s.peers().is_empty());
    mac.ctl_ok = false;
    fake.fetch_add(60 * 10000, Ordering::SeqCst);
    until(30, || { s.poll(2); mac.step(); false });
    assert!(!mac.ctl_ok, "a forgotten peer is not invited again");

    // A Mac that refuses: waiting, invited again after the pause.
    let mut no = FakeResponder::new();
    no.refuse = true;
    s.invite("127.0.0.1", no.ctl.port, "Refuser");
    assert!(until(1000, || { s.poll(2); no.step(); s.peers().first().map(|p| p.state) == Some(PeerState::Waiting) }));
    no.refuse = false;
    fake.fetch_add(6 * 10000, Ordering::SeqCst);
    assert!(until(1000, || { s.poll(2); no.step(); no.data_ok }), "invited again after the pause");

    // A Mac that never answers: 12 tries a second apart, then waiting.
    let mut mute = FakeResponder::new();
    mute.silent = true;
    s.invite("127.0.0.1", mute.ctl.port, "Mute");
    for _ in 0..14 {
        until(5, || { s.poll(1); mute.step(); false });
        fake.fetch_add(10000, Ordering::SeqCst);
    }
    assert!(s.peers().iter().any(|p| p.label == "Mute" && p.state == PeerState::Waiting), "no answer -> waiting");
    s.close();
    assert!(until(500, || { no.step(); no.bye }), "close says goodbye to connected peers");
}

/// Two sessions (TuneSync and a sender tool) talk to each other.
#[test]
fn session_to_session() {
    let (a, _, _, mut ca) = session_with(None);
    ca.name = "Sender".into();
    a.open(ca).unwrap();
    let (b, got, _, cb) = session_with(None);
    let bport = cb.port;
    b.open(cb.clone()).unwrap();
    a.invite("127.0.0.1", b.port(), "");
    let both = |a: &Session, b: &Session| {
        a.peers().first().map(|p| p.state) == Some(PeerState::Connected) && b.peers().first().map(|p| p.state) == Some(PeerState::Connected)
    };
    assert!(until(3000, || { a.poll(1); b.poll(1); both(&a, &b) }), "connected both ways");
    assert_eq!(a.send(&[&[0xBF, 20, 2], &[0x9F, 62, 90]]), 1);
    assert!(until(1000, || { a.poll(1); b.poll(1); got.lock().unwrap().len() >= 2 }));
    let g = got.lock().unwrap().clone();
    assert_eq!(g, vec![("Sender".to_string(), vec![0xBF, 20, 2]), ("Sender".to_string(), vec![0x9F, 62, 90])]);
    // A second session on the same port fails cleanly.
    let c = Session::new();
    let err = c.open(SessionConfig { port: bport, ..Default::default() }).unwrap_err();
    assert!(err.contains("in use"), "{err}");
    a.close();
    assert!(until(1000, || { b.poll(1); b.peers().is_empty() }), "a's goodbye reaches b");
}
