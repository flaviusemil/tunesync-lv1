//! RTP-MIDI (RFC 6295) and Apple's session protocol ("AppleMIDI"): build and parse
//! datagrams. No sockets, no clock. Ported from Headlink's tools/preprobe/rtpmidi.cpp
//! (demo/pre-probe, 2026-10-20), codec half, same behaviour and the same test vectors.
//!
//! ```text
//! WIRE SUMMARY (all integers big-endian)
//!   Session packets start with FF FF and a two-letter command:
//!     IN/OK/NO  FF FF cc cc | version=2 (4) | initiator token (4) | SSRC (4) | name\0
//!     BY        FF FF 42 59 | version=2 (4) | initiator token (4) | SSRC (4)
//!     CK        FF FF 43 4B | SSRC (4) | count (1) | pad (3) | ts1 ts2 ts3 (8 each)
//!     RS        FF FF 52 53 | SSRC (4) | extended sequence number (4): the last RTP
//!               sequence received, wrap count in the top 16 bits (Apple; reading 16
//!               bits there always gives 0)
//!   The initiator invites the control port (normally 5004), then the data port
//!   (control + 1) from its own data socket. Clock sync (CK) and MIDI run on the data
//!   port; receiver feedback (RS) and BY go to the control port. Timestamps count
//!   100 us units (a 10 kHz clock).
//!
//!   MIDI packets are RTP: 80 61 | seq (2) | timestamp (4) | SSRC (4), then the MIDI
//!   command section: one header byte B J Z P LEN(4) (B = 1: LEN is 12 bits and a
//!   second byte follows), LEN bytes of MIDI list, then the recovery journal when
//!   J = 1. In the list every command after the first is preceded by a delta time
//!   (1-4 bytes, 7 bits each, high bit = more); the first one too when Z = 1. Running
//!   status is allowed after the first command.
//! ```

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AmCmd {
    #[default]
    None,
    In,
    Ok,
    No,
    By,
    Ck,
    Rs,
    Rl,
    Unknown,
}

pub fn am_name(c: AmCmd) -> &'static str {
    match c {
        AmCmd::In => "IN",
        AmCmd::Ok => "OK",
        AmCmd::No => "NO",
        AmCmd::By => "BY",
        AmCmd::Ck => "CK",
        AmCmd::Rs => "RS",
        AmCmd::Rl => "RL",
        AmCmd::Unknown => "??",
        AmCmd::None => "-",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AmPacket {
    pub cmd: AmCmd,
    /// The two command letters as received.
    pub raw: [u8; 2],
    /// IN/OK/NO/BY
    pub version: u32,
    /// IN/OK/NO/BY: the initiator's token.
    pub token: u32,
    pub ssrc: u32,
    /// IN/OK/NO: the sender's session name.
    pub name: String,
    /// CK: 0, 1, 2
    pub count: u8,
    pub ts: [u64; 3],
    /// RS: extended sequence number.
    pub seq: u32,
    pub limit: u32,
}

fn be16(p: &[u8]) -> u16 {
    u16::from_be_bytes([p[0], p[1]])
}
fn be32(p: &[u8]) -> u32 {
    u32::from_be_bytes([p[0], p[1], p[2], p[3]])
}
fn be64(p: &[u8]) -> u64 {
    (be32(p) as u64) << 32 | be32(&p[4..]) as u64
}

/// True when the datagram is a session packet (starts FF FF), whatever its command.
pub fn am_is_session(p: &[u8]) -> bool {
    p.len() >= 4 && p[0] == 0xFF && p[1] == 0xFF
}

/// None for anything too short for its command. A command we do not know parses as
/// Unknown (letters in raw) rather than refused, so the log shows it.
pub fn am_parse(p: &[u8]) -> Option<AmPacket> {
    if !am_is_session(p) {
        return None;
    }
    let mut out = AmPacket { raw: [p[2], p[3]], ..Default::default() };
    match &p[2..4] {
        b"IN" | b"OK" | b"NO" => {
            if p.len() < 16 {
                return None;
            }
            out.cmd = match p[2] {
                b'I' => AmCmd::In,
                b'O' => AmCmd::Ok,
                _ => AmCmd::No,
            };
            out.version = be32(&p[4..]);
            out.token = be32(&p[8..]);
            out.ssrc = be32(&p[12..]);
            // The name is NUL-terminated and optional; stop at the end of the datagram
            // if the terminator is missing.
            let name = &p[16..];
            let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
            out.name = String::from_utf8_lossy(&name[..end]).into_owned();
        }
        b"BY" => {
            if p.len() < 16 {
                return None;
            }
            out.cmd = AmCmd::By;
            out.version = be32(&p[4..]);
            out.token = be32(&p[8..]);
            out.ssrc = be32(&p[12..]);
        }
        b"CK" => {
            if p.len() < 36 {
                return None;
            }
            out.cmd = AmCmd::Ck;
            out.ssrc = be32(&p[4..]);
            out.count = p[8];
            for i in 0..3 {
                out.ts[i] = be64(&p[12 + 8 * i..]);
            }
        }
        b"RS" => {
            if p.len() < 12 {
                return None;
            }
            out.cmd = AmCmd::Rs;
            out.ssrc = be32(&p[4..]);
            out.seq = be32(&p[8..]);
        }
        b"RL" => {
            if p.len() < 12 {
                return None;
            }
            out.cmd = AmCmd::Rl;
            out.ssrc = be32(&p[4..]);
            out.limit = be32(&p[8..]);
        }
        _ => out.cmd = AmCmd::Unknown,
    }
    Some(out)
}

fn header(a: u8, b: u8) -> Vec<u8> {
    vec![0xFF, 0xFF, a, b]
}

/// IN / OK / NO.
pub fn am_invite(cmd: AmCmd, token: u32, ssrc: u32, name: &str) -> Vec<u8> {
    let (a, b) = match cmd {
        AmCmd::In => (b'I', b'N'),
        AmCmd::Ok => (b'O', b'K'),
        _ => (b'N', b'O'),
    };
    let mut v = header(a, b);
    v.extend_from_slice(&2u32.to_be_bytes());
    v.extend_from_slice(&token.to_be_bytes());
    v.extend_from_slice(&ssrc.to_be_bytes());
    v.extend_from_slice(name.as_bytes());
    v.push(0);
    v
}

pub fn am_bye(token: u32, ssrc: u32) -> Vec<u8> {
    let mut v = header(b'B', b'Y');
    v.extend_from_slice(&2u32.to_be_bytes());
    v.extend_from_slice(&token.to_be_bytes());
    v.extend_from_slice(&ssrc.to_be_bytes());
    v
}

pub fn am_clock(ssrc: u32, count: u8, ts1: u64, ts2: u64, ts3: u64) -> Vec<u8> {
    let mut v = header(b'C', b'K');
    v.extend_from_slice(&ssrc.to_be_bytes());
    v.extend_from_slice(&[count, 0, 0, 0]);
    for t in [ts1, ts2, ts3] {
        v.extend_from_slice(&t.to_be_bytes());
    }
    v
}

pub fn am_feedback(ssrc: u32, ext_seq: u32) -> Vec<u8> {
    let mut v = header(b'R', b'S');
    v.extend_from_slice(&ssrc.to_be_bytes());
    v.extend_from_slice(&ext_seq.to_be_bytes());
    v
}

/// One MIDI message from a command section, running status expanded: `bytes` always
/// starts with a status byte.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MidiEvent {
    /// In the session's 100 us units.
    pub delta: u32,
    /// The status byte was not on the wire.
    pub running: bool,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RtpMidiPacket {
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub b: bool,
    pub j: bool,
    pub z: bool,
    pub p: bool,
    /// LEN: bytes of MIDI list.
    pub list_len: usize,
    /// Bytes after the list (the recovery journal), not decoded.
    pub journal_len: usize,
    pub events: Vec<MidiEvent>,
}

/// RTP version 2, payload type 0x61.
pub fn rtp_is_midi(p: &[u8]) -> bool {
    p.len() >= 12 && (p[0] & 0xC0) == 0x80 && (p[1] & 0x7F) == 0x61
}

/// How many data bytes follow a status byte: -1 for SysEx (variable), 0..2 otherwise.
pub fn midi_data_len(s: u8) -> i32 {
    if s < 0x80 {
        return 0;
    }
    match s & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => return 2,
        0xC0 | 0xD0 => return 1,
        _ => {}
    }
    match s {
        0xF0 => -1,
        0xF1 | 0xF3 => 1,
        0xF2 => 2,
        _ => 0, // F4..F7 (F7 = end of SysEx segment) and realtime F8..FF
    }
}

/// Err(why) for a packet that is not RTP-MIDI or whose MIDI list does not decode. The
/// events decoded before the fault are in the packet either way (the second value).
pub fn rtp_parse(p: &[u8]) -> (RtpMidiPacket, Result<(), &'static str>) {
    let mut out = RtpMidiPacket::default();
    let r = parse_into(p, &mut out);
    (out, r)
}

fn parse_into(p: &[u8], out: &mut RtpMidiPacket) -> Result<(), &'static str> {
    if !rtp_is_midi(p) {
        return Err("not RTP-MIDI (version 2, payload type 0x61)");
    }
    let n = p.len();
    let cc = (p[0] & 0x0F) as usize; // CSRC count: none expected, skipped if present
    let mut i = 12 + 4 * cc;
    if p[0] & 0x10 != 0 {
        // RTP header extension: skip it
        if n < i + 4 {
            return Err("truncated RTP header extension");
        }
        i += 4 + 4 * be16(&p[i + 2..]) as usize;
    }
    out.seq = be16(&p[2..]);
    out.ts = be32(&p[4..]);
    out.ssrc = be32(&p[8..]);
    if n < i + 1 {
        return Err("no MIDI command section");
    }
    let h = p[i];
    out.b = h & 0x80 != 0;
    out.j = h & 0x40 != 0;
    out.z = h & 0x20 != 0;
    out.p = h & 0x10 != 0;
    let mut len = (h & 0x0F) as usize;
    i += 1;
    if out.b {
        if n < i + 1 {
            return Err("truncated long LEN");
        }
        len = (len << 8) | p[i] as usize;
        i += 1;
    }
    out.list_len = len;
    if n < i + len {
        return Err("MIDI list longer than the packet");
    }
    let end = i + len;
    out.journal_len = n - end; // with J off this should be 0; kept for the log either way

    let mut running = 0u8;
    let mut first = true;
    while i < end {
        let mut ev = MidiEvent::default();
        if !first || out.z {
            // delta time, 1-4 bytes
            let (mut d, mut k) = (0u32, 0);
            loop {
                if i >= end {
                    return Err("truncated delta time");
                }
                k += 1;
                if k > 4 {
                    return Err("delta time longer than 4 bytes");
                }
                let c = p[i];
                i += 1;
                d = (d << 7) | (c & 0x7F) as u32;
                if c & 0x80 == 0 {
                    break;
                }
            }
            ev.delta = d;
        }
        if i >= end {
            return Err("delta time with no command");
        }
        let mut s = p[i];
        if s & 0x80 != 0 {
            i += 1;
        } else {
            if running == 0 {
                return Err("data byte with no running status");
            }
            s = running;
            ev.running = true;
        }
        ev.bytes.push(s);
        let dl = midi_data_len(s);
        if dl < 0 || s == 0xF7 {
            // SysEx: F0 (start) or F7 (continuation of a segmented one). Runs to F7 (end),
            // F0 (more segments follow) or F4 (cancelled).
            let mut closed = false;
            while i < end {
                let c = p[i];
                i += 1;
                ev.bytes.push(c);
                if c == 0xF7 || c == 0xF0 || c == 0xF4 {
                    closed = true;
                    break;
                }
                if c & 0x80 != 0 {
                    return Err("status byte inside SysEx");
                }
            }
            if !closed {
                return Err("SysEx without an end byte");
            }
            running = 0;
        } else {
            for _ in 0..dl {
                if i >= end {
                    return Err("truncated MIDI command");
                }
                if p[i] & 0x80 != 0 {
                    return Err("status byte where a data byte belongs");
                }
                ev.bytes.push(p[i]);
                i += 1;
            }
            if s < 0xF0 {
                running = s; // channel messages set running status
            } else if s < 0xF8 {
                running = 0; // system common clears it; realtime leaves it
            }
        }
        out.events.push(ev);
        first = false;
    }
    Ok(())
}

/// A packet carrying msgs back to back (delta 0), no journal. Each msg must start with a
/// status byte; nothing is merged into running status.
pub fn rtp_build(seq: u16, ts: u32, ssrc: u32, msgs: &[&[u8]]) -> Vec<u8> {
    let mut list = Vec::new();
    for (k, m) in msgs.iter().enumerate() {
        if k > 0 {
            list.push(0x00); // delta 0 before every command after the first
        }
        list.extend_from_slice(m);
    }
    let mut v = vec![0x80, 0x61];
    v.extend_from_slice(&seq.to_be_bytes());
    v.extend_from_slice(&ts.to_be_bytes());
    v.extend_from_slice(&ssrc.to_be_bytes());
    if list.len() <= 15 {
        v.push(list.len() as u8);
    } else {
        let len = list.len().min(0x0FFF);
        v.push(0x80 | (len >> 8) as u8);
        v.push((len & 0xFF) as u8);
        list.truncate(len);
    }
    v.extend_from_slice(&list);
    v
}

pub fn hex(p: &[u8]) -> String {
    p.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// "IN v2 token 0x.. ssrc 0x.. name 'x'"
pub fn describe(a: &AmPacket) -> String {
    match a.cmd {
        AmCmd::In | AmCmd::Ok | AmCmd::No => {
            format!("{} v{} token 0x{:08X} ssrc 0x{:08X} name '{}'", am_name(a.cmd), a.version, a.token, a.ssrc, a.name)
        }
        AmCmd::By => format!("BY v{} token 0x{:08X} ssrc 0x{:08X}", a.version, a.token, a.ssrc),
        AmCmd::Ck => format!("CK{} ssrc 0x{:08X} ts {} {} {}", a.count, a.ssrc, a.ts[0], a.ts[1], a.ts[2]),
        AmCmd::Rs => format!("RS ssrc 0x{:08X} seq {} (wraps {})", a.ssrc, a.seq & 0xFFFF, a.seq >> 16),
        AmCmd::Rl => format!("RL ssrc 0x{:08X} limit {}", a.ssrc, a.limit),
        _ => format!("unknown session command '{}{}'", a.raw[0] as char, a.raw[1] as char),
    }
}
