//! One AppleMIDI network session that several peers join at once, the way a Mac's own
//! network session does: one port pair, each participant told apart by its SSRC. A main
//! and a backup playback rig can both be connected; MIDI from any of them is delivered
//! with the peer it came from.
//!
//! Both roles, per peer:
//! - **the peer invites us**: IN on the control port, then on the data port; we answer
//!   OK. It runs clock sync; we answer CK0 with CK1. Dropped after BY or 60 s of silence.
//! - **we invite the peer** (Connect in the window, or a remembered peer): IN to its
//!   control port, then its data port, one a second; then we run clock sync. Refused,
//!   unanswered or gone quiet: invited again later. After a BY we wait longer (30 s).
//!   Why it waits is logged once; the retries of a peer that stays away are not.
//!
//! Receiver feedback (RS) goes to every connected peer's control port once a second
//! while its sequence moves.
//!
//! The interop rules learned with Headlink's preprobe hold here:
//! replies always go to the port a packet came from (rtpmidid answers the data
//! invitation from control + 1, a NAT may remap it), RS carries the 32-bit extended
//! sequence, and an invited peer may run its own clock sync, which we answer.
//!
//! Threads: `open` / `close` / `poll` belong to ONE thread (the RTP thread). `invite` /
//! `forget` / `send` / `peers` may be called from any thread; they never wait on the
//! network. Callbacks run on the poll thread, outside the session's lock.
use std::collections::HashSet;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, UdpSocket};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::codec::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerState {
    /// It invited our control port; waiting for its data-port invitation.
    Accepting,
    /// We sent IN to its control port.
    Inviting,
    /// Control accepted, IN sent to its data port.
    InvitingData,
    /// Both ports accepted: MIDI flows.
    Connected,
    /// We invite it, and will again (refused, no answer, BY, silent).
    Waiting,
}

pub fn peer_state_name(s: PeerState) -> &'static str {
    match s {
        PeerState::Accepting => "accepting",
        PeerState::Inviting => "inviting",
        PeerState::InvitingData => "inviting (data port)",
        PeerState::Connected => "connected",
        PeerState::Waiting => "waiting",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeerView {
    /// 0 until it answered.
    pub ssrc: u32,
    /// Its own session name, "" until it answered.
    pub name: String,
    /// What it was invited as (the name it was found under).
    pub label: String,
    pub ip: String,
    /// Invited: the control port we were asked to invite.
    pub target: u16,
    pub ctl_port: u16,
    pub data_port: u16,
    pub state: PeerState,
    /// We invite it; else it invited us.
    pub invited: bool,
    /// On the session clock (`now_ms`), 0 = never.
    pub last_heard_ms: u64,
    pub last_midi_ms: u64,
    pub midi: u32,
    pub lost: u32,
    pub rtt_ms: Option<f64>,
}

impl PeerView {
    /// How the window and the log name it.
    pub fn display(&self) -> String {
        if !self.name.is_empty() {
            self.name.clone()
        } else if !self.label.is_empty() {
            self.label.clone()
        } else {
            self.ip.clone()
        }
    }
}

pub type Clock10k = Arc<dyn Fn() -> u64 + Send + Sync>;

#[derive(Clone)]
pub struct SessionConfig {
    /// Our session name, what the peers show.
    pub name: String,
    /// Control port; data = port + 1.
    pub port: u16,
    pub max_peers: usize,
    /// Per attempt, one a second (what macOS does).
    pub invite_tries: u32,
    /// After a refusal or no answer.
    pub retry_ms: u64,
    /// After a peer we invited said goodbye.
    pub bye_retry_ms: u64,
    /// A peer that invited us and went quiet is dropped.
    pub silent_ms: u64,
    /// ...or never invited our data port.
    pub accept_ms: u64,
    /// Every session packet, as detail lines.
    pub log_packets: bool,
    /// The session clock in 100 us units, for tests. Default: the steady clock.
    pub clock10k: Option<Clock10k>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            name: "TuneSync".into(),
            port: 5004,
            max_peers: 8,
            invite_tries: 12,
            retry_ms: 5000,
            bye_retry_ms: 30000,
            silent_ms: 60000,
            accept_ms: 10000,
            log_packets: false,
            clock10k: None,
        }
    }
}

/// `detail` = packet-level noise, for the file log only.
pub type LogFn = Box<dyn Fn(&str, bool) + Send + Sync>;
/// One whole MIDI message (running status expanded) from a connected peer, and when the
/// sender stamped it (us, the sender's session clock; see [`Session::on_midi`]).
pub type MidiFn = Box<dyn Fn(&PeerView, &[u8], u64) + Send + Sync>;

const SEC: u64 = 10000; // the session clock counts 100 us units

struct Peer {
    v: PeerView,
    token: u32,
    tries: u32,
    next_at: u64,
    accept_by: u64,
    last_heard: u64,
    next_ck: u64,
    ck_ts1: u64,
    ck_sent: u32,
    ck_missed: u32,
    have_seq: bool,
    seq_in: u16,
    cycles: u32,
    /// The last RTP timestamp in and its 64-bit extension (wraps and reordering absorbed).
    ts_in: Option<(u32, u64)>,
    fed: u32,
    next_rs: u64,
    gone: bool,
    /// Why it went to Waiting, logged once: while it stays away its retries are not
    /// logged again (Connected or a new reason clears it).
    waiting_why: Option<String>,
}

impl Peer {
    /// A packet's 32-bit RTP timestamp (100 us units) as a 64-bit one: it wraps after
    /// ~5 days, and a reordered packet must not look 5 days late.
    fn extend_ts(&mut self, ts: u32) -> u64 {
        let ext = match self.ts_in {
            None => (1u64 << 32) + u64::from(ts),
            Some((last, ext)) => ext.wrapping_add_signed(i64::from(ts.wrapping_sub(last) as i32)),
        };
        self.ts_in = Some((ts, ext));
        ext
    }

    fn new(ip: &str) -> Peer {
        Peer {
            v: PeerView {
                ssrc: 0,
                name: String::new(),
                label: String::new(),
                ip: ip.to_string(),
                target: 0,
                ctl_port: 0,
                data_port: 0,
                state: PeerState::Waiting,
                invited: false,
                last_heard_ms: 0,
                last_midi_ms: 0,
                midi: 0,
                lost: 0,
                rtt_ms: None,
            },
            token: 0,
            tries: 0,
            next_at: 0,
            accept_by: 0,
            last_heard: 0,
            next_ck: 0,
            ck_ts1: 0,
            ck_sent: 0,
            ck_missed: 0,
            have_seq: false,
            ts_in: None,
            seq_in: 0,
            cycles: 0,
            fed: 0,
            next_rs: 0,
            gone: false,
            waiting_why: None,
        }
    }
    // A peer that accepted or sent an invitation holds a session with us: it gets a BY.
    fn knows_us(&self) -> bool {
        matches!(self.v.state, PeerState::Connected | PeerState::InvitingData | PeerState::Accepting)
    }
    fn heard(&mut self, now: u64) {
        self.last_heard = now;
        self.v.last_heard_ms = now / 10;
    }
}

struct Sockets {
    ctl: UdpSocket,
    data: UdpSocket,
    port: u16,
    /// Both sockets take IPv6 as well as IPv4 (IPv4 peers arrive as ::ffff:a.b.c.d).
    dual: bool,
}

/// A UDP socket on [::]:port that takes IPv4 too. Needed because a host advertised over
/// DNS-SD is published with its IPv6 link-local address next to its IPv4 one (Windows and
/// macOS both do this), and macOS's network MIDI then connects over IPv6.
fn bind_dual(port: u16) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_only_v6(false)?;
    s.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
    Ok(s.into())
}

/// Control on `port`, data on `port + 1`: dual stack when the system has IPv6, else IPv4
/// only. (The dual attempt's sockets are dropped at the end of its statement, so a half-made
/// pair is closed before the IPv4 binds.)
fn bind_pair(port: u16) -> std::io::Result<(UdpSocket, UdpSocket, bool)> {
    if let (Ok(c), Ok(d)) = (bind_dual(port), bind_dual(port + 1)) {
        return Ok((c, d, true));
    }
    let bind4 = |p: u16| UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, p));
    Ok((bind4(port)?, bind4(port + 1)?, false))
}

/// How a peer's address is kept: "192.0.2.42" (also for an IPv4 peer that reached a
/// dual-stack socket as ::ffff:192.0.2.42), "fe80::1%3" for a link-local IPv6 peer (the
/// scope is the interface, needed to answer it).
fn addr_text(a: &SocketAddr) -> String {
    match a {
        SocketAddr::V4(a) => a.ip().to_string(),
        SocketAddr::V6(a) => match a.ip().to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None if a.scope_id() != 0 => format!("{}%{}", a.ip(), a.scope_id()),
            None => a.ip().to_string(),
        },
    }
}

/// Where to send to a peer address in that form, from dual-stack sockets or IPv4-only ones.
fn send_addr(ip: &str, port: u16, dual: bool) -> Option<SocketAddr> {
    if port == 0 {
        return None;
    }
    if let Ok(v4) = ip.parse::<Ipv4Addr>() {
        return Some(if dual { SocketAddrV6::new(v4.to_ipv6_mapped(), port, 0, 0).into() } else { SocketAddrV4::new(v4, port).into() });
    }
    if !dual {
        return None;
    }
    let (a, scope) = match ip.split_once('%') {
        Some((a, sc)) => (a, sc.parse().ok()?),
        None => (ip, 0),
    };
    Some(SocketAddrV6::new(a.parse().ok()?, port, 0, scope).into())
}

struct Inner {
    cfg: SessionConfig,
    t0: Instant,
    ssrc: u32,
    seq_out: u16,
    peers: Vec<Peer>,
    strangers: HashSet<u32>,
    rng: u64,
}

// Collected under the lock, delivered after it.
#[derive(Default)]
struct Out {
    logs: Vec<(String, bool)>,
    midi: Vec<(PeerView, Vec<u8>, u64)>,
    sends: Vec<(bool, String, u16, Vec<u8>)>, // (data socket?, ip, port, bytes)
}

impl Out {
    fn log(&mut self, l: impl Into<String>, detail: bool) {
        self.logs.push((l.into(), detail));
    }
}

pub struct Session {
    log: Option<LogFn>,
    midi: Option<MidiFn>,
    sock: Mutex<Option<Arc<Sockets>>>,
    inner: Mutex<Inner>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

fn seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let a = &t as *const u64 as u64;
    t ^ a.rotate_left(32) ^ 0x9E37_79B9_7F4A_7C15
}

impl Inner {
    fn now10k(&self) -> u64 {
        match &self.cfg.clock10k {
            Some(c) => c(),
            None => self.t0.elapsed().as_micros() as u64 / 100 + 1,
        }
    }
    fn random32(&mut self) -> u32 {
        loop {
            // xorshift64*
            self.rng ^= self.rng >> 12;
            self.rng ^= self.rng << 25;
            self.rng ^= self.rng >> 27;
            let v = (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32;
            if v != 0 {
                return v;
            }
        }
    }
    fn live(&self) -> usize {
        self.peers.iter().filter(|p| !p.gone).count()
    }
    fn by_ssrc(&mut self, ssrc: u32, ip: &str) -> Option<usize> {
        if ssrc == 0 {
            return None;
        }
        self.peers.iter().position(|p| !p.gone && p.v.ssrc == ssrc && p.v.ip == ip)
    }
    fn invited_at(&self, ip: &str, target: u16) -> Option<usize> {
        self.peers.iter().position(|p| !p.gone && p.v.invited && p.v.ip == ip && p.v.target == target)
    }
    fn send(&self, out: &mut Out, data: bool, ip: &str, port: u16, pkt: Vec<u8>, what: &str) {
        if self.cfg.log_packets {
            let d = am_parse(&pkt).map(|a| describe(&a)).unwrap_or_default();
            out.log(format!("RTP-MIDI {} {what} -> {ip}:{port}  {d}", if data { "data" } else { "ctl " }), true);
        }
        out.sends.push((data, ip.to_string(), port, pkt));
    }
    fn set_state(&self, p: &mut Peer, s: PeerState, why: &str, out: &mut Out) {
        if p.v.state == s {
            return;
        }
        p.v.state = s;
        // A remembered peer that is away is invited again every few seconds, all night if
        // need be: say why once, then stay quiet until it connects or the reason changes.
        let repeat = match s {
            PeerState::Waiting => p.waiting_why.as_deref() == Some(why),
            PeerState::Inviting | PeerState::InvitingData => p.waiting_why.is_some(),
            _ => false,
        };
        match s {
            PeerState::Waiting => p.waiting_why = Some(why.to_string()),
            PeerState::Connected => p.waiting_why = None,
            _ => {}
        }
        if repeat {
            return;
        }
        let d = p.v.display();
        let mut l = if d == p.v.ip { format!("RTP-MIDI: {d} {}", peer_state_name(s)) } else { format!("RTP-MIDI: {d} ({}) {}", p.v.ip, peer_state_name(s)) };
        if !why.is_empty() {
            l += &format!(" - {why}");
        }
        if s == PeerState::Waiting {
            l += "; will keep inviting it";
        }
        // An invitation in flight is detail; arrivals, departures and why it waits are not.
        out.log(l, matches!(s, PeerState::Inviting | PeerState::InvitingData | PeerState::Accepting));
    }
    fn start_attempt(&mut self, i: usize, now: u64) {
        let token = self.random32();
        let p = &mut self.peers[i];
        p.token = token;
        p.tries = 0;
        p.v.ssrc = 0;
        p.have_seq = false;
        p.ts_in = None;
        p.cycles = 0;
        p.fed = 0;
        p.ck_sent = 0;
        p.ck_ts1 = 0;
        p.ck_missed = 0;
        p.v.rtt_ms = None;
        p.v.ctl_port = p.v.target;
        p.v.data_port = p.v.target.wrapping_add(1);
        p.next_at = now;
    }
}

impl Session {
    pub fn new() -> Self {
        Session {
            log: None,
            midi: None,
            sock: Mutex::new(None),
            inner: Mutex::new(Inner {
                cfg: SessionConfig::default(),
                t0: Instant::now(),
                ssrc: 0,
                seq_out: 0,
                peers: Vec::new(),
                strangers: HashSet::new(),
                rng: seed(),
            }),
        }
    }

    /// Before open().
    pub fn on_log(&mut self, f: impl Fn(&str, bool) + Send + Sync + 'static) {
        self.log = Some(Box::new(f));
    }
    /// Before open(). Each message comes with when the sender stamped it, in microseconds
    /// on the sender's session clock (the packet's RTP timestamp plus the message's delta
    /// time, 100 us units): MIDI clock can be timed on the sender's clock, free of network
    /// jitter.
    pub fn on_midi(&mut self, f: impl Fn(&PeerView, &[u8], u64) + Send + Sync + 'static) {
        self.midi = Some(Box::new(f));
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn sockets(&self) -> Option<Arc<Sockets>> {
        self.sock.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn is_open(&self) -> bool {
        self.sockets().is_some()
    }
    pub fn port(&self) -> u16 {
        self.sockets().map_or(0, |s| s.port)
    }
    pub fn now_ms(&self) -> u64 {
        self.inner().now10k() / 10
    }

    /// Binds control on cfg.port and data on cfg.port + 1, every interface, IPv6 and IPv4.
    pub fn open(&self, cfg: SessionConfig) -> Result<(), String> {
        self.open_from(cfg, 1).map(|_| ())
    }

    /// [`open`](Self::open), moving up when cfg.port's pair can't be had: in use (another
    /// RTP-MIDI session, such as macOS's own network MIDI or rtpMIDI on Windows, holds
    /// 5004) or not allowed (a Windows excluded port range). It tries the next pairs 2 apart,
    /// `tries` pairs in all, and stops at any other error. Ok(the control port it opened).
    pub fn open_from(&self, cfg: SessionConfig, tries: u16) -> Result<u16, String> {
        self.close();
        if cfg.port == 0 || cfg.port == 65535 {
            return Err("bad UDP port".into());
        }
        let mut tried = Vec::new();
        let mut last = String::new();
        for i in 0..tries.max(1) {
            let Some(port) = cfg.port.checked_add(i.saturating_mul(2)).filter(|p| *p < 65535) else { break };
            match bind_pair(port) {
                Ok((ctl, data, dual)) => {
                    ctl.set_nonblocking(true).map_err(|e| e.to_string())?;
                    {
                        let mut g = self.inner();
                        g.t0 = Instant::now();
                        g.ssrc = g.random32();
                        g.peers.clear();
                        g.strangers.clear();
                        g.cfg = SessionConfig { port, ..cfg.clone() };
                    }
                    *self.sock.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Sockets { ctl, data, port, dual }));
                    return Ok(port);
                }
                Err(e) => {
                    tried.push(port);
                    last = e.to_string();
                    if !matches!(e.kind(), ErrorKind::AddrInUse | ErrorKind::PermissionDenied) {
                        break;
                    }
                }
            }
        }
        Err(match tried.as_slice() {
            [p] => format!("UDP port {p} is in use or not allowed ({last})"),
            [first, .., end] => format!("UDP ports {first} to {} are all in use or not allowed ({last})", end + 1),
            [] => format!("no UDP port to try from {}", cfg.port),
        })
    }

    /// BY to every peer that holds a session with us, then the sockets close. The peer
    /// table is cleared.
    pub fn close(&self) {
        let Some(s) = self.sock.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        let mut out = Out::default();
        {
            let mut g = self.inner();
            let ssrc = g.ssrc;
            let peers = std::mem::take(&mut g.peers);
            for p in peers.iter().filter(|p| !p.gone && p.knows_us()) {
                g.send(&mut out, false, &p.v.ip, p.v.ctl_port, am_bye(p.token, ssrc), "BY");
                out.log(format!("RTP-MIDI: said goodbye to {}", p.v.display()), false);
            }
        }
        self.flush(&s, &mut out);
        self.deliver(out);
    }

    fn flush(&self, s: &Sockets, out: &mut Out) {
        for (data, ip, port, pkt) in out.sends.drain(..) {
            let Some(to) = send_addr(&ip, port, s.dual) else { continue };
            let sock = if data { &s.data } else { &s.ctl };
            if sock.send_to(&pkt, to).is_err() {
                out.logs.push((format!("RTP-MIDI: send to {ip}:{port} failed"), true));
            }
        }
    }

    fn deliver(&self, out: Out) {
        if let Some(l) = &self.log {
            for (line, detail) in &out.logs {
                l(line, *detail);
            }
        }
        if let Some(m) = &self.midi {
            for (p, msg, sent_us) in &out.midi {
                m(p, msg, *sent_us);
            }
        }
    }

    /// We invite ip:port (its control port). A peer already there is invited again now.
    pub fn invite(&self, ip: &str, port: u16, label: &str) {
        let Some(s) = self.sockets() else { return };
        if ip.is_empty() || port == 0 {
            return;
        }
        let mut out = Out::default();
        {
            let mut g = self.inner();
            let now = g.now10k();
            let i = match g.invited_at(ip, port) {
                Some(i) => Some(i),
                None if g.live() >= g.cfg.max_peers => {
                    out.log(format!("RTP-MIDI: cannot invite {ip}: the session already has {} peers", g.cfg.max_peers), false);
                    None
                }
                None => {
                    let mut p = Peer::new(ip);
                    p.v.invited = true;
                    p.v.target = port;
                    g.peers.push(p);
                    Some(g.peers.len() - 1)
                }
            };
            if let Some(i) = i {
                if !label.is_empty() {
                    g.peers[i].v.label = label.to_string();
                }
                if g.peers[i].v.state == PeerState::Waiting {
                    g.start_attempt(i, now);
                    let mut p = std::mem::replace(&mut g.peers[i], Peer::new(""));
                    p.waiting_why = None; // asked for again: log this attempt afresh
                    g.set_state(&mut p, PeerState::Inviting, "", &mut out);
                    g.peers[i] = p;
                }
            }
        }
        self.flush(&s, &mut out);
        self.deliver(out);
    }

    /// Stop inviting ip:port; BY first when it holds a session with us.
    pub fn forget(&self, ip: &str, port: u16) {
        let Some(s) = self.sockets() else { return };
        let mut out = Out::default();
        {
            let mut g = self.inner();
            let ssrc = g.ssrc;
            let mut peers = std::mem::take(&mut g.peers);
            for p in peers.iter_mut().filter(|p| !p.gone && p.v.ip == ip && (p.v.target == port || p.v.ctl_port == port)) {
                if p.knows_us() {
                    g.send(&mut out, false, &p.v.ip, p.v.ctl_port, am_bye(p.token, ssrc), "BY");
                }
                p.gone = true;
                out.log(format!("RTP-MIDI: {} ({ip}) disconnected", p.v.display()), false);
            }
            peers.retain(|p| !p.gone);
            g.peers = peers;
        }
        self.flush(&s, &mut out);
        self.deliver(out);
    }

    /// MIDI to every connected peer (tests, the sender tool). Returns how many got it.
    pub fn send(&self, msgs: &[&[u8]]) -> usize {
        let Some(s) = self.sockets() else { return 0 };
        let mut g = self.inner();
        let targets: Vec<(String, u16)> = g.peers.iter().filter(|p| !p.gone && p.v.state == PeerState::Connected).map(|p| (p.v.ip.clone(), p.v.data_port)).collect();
        if targets.is_empty() {
            return 0;
        }
        let pkt = rtp_build(g.seq_out, g.now10k() as u32, g.ssrc, msgs);
        g.seq_out = g.seq_out.wrapping_add(1);
        drop(g);
        targets
            .iter()
            .filter(|(ip, port)| send_addr(ip, *port, s.dual).is_some_and(|to| s.data.send_to(&pkt, to).is_ok()))
            .count()
    }

    pub fn peers(&self) -> Vec<PeerView> {
        self.inner().peers.iter().filter(|p| !p.gone).map(|p| p.v.clone()).collect()
    }

    /// Waits up to timeout_ms for datagrams, handles them, runs the timers.
    pub fn poll(&self, timeout_ms: u64) {
        let Some(s) = self.sockets() else {
            std::thread::sleep(Duration::from_millis(timeout_ms));
            return;
        };
        let mut got: Vec<(bool, Vec<u8>, SocketAddr)> = Vec::new();
        let mut buf = [0u8; 2048];
        let drain = |sock: &UdpSocket, data: bool, got: &mut Vec<(bool, Vec<u8>, SocketAddr)>, buf: &mut [u8]| {
            for _ in 0..64 {
                match sock.recv_from(buf) {
                    Ok((n, a)) => got.push((data, buf[..n].to_vec(), a)),
                    Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => break,
                    Err(_) => continue, // Windows: an ICMP port-unreachable for an earlier send
                }
            }
        };
        drain(&s.ctl, false, &mut got, &mut buf);
        if got.is_empty() {
            // Wait on the data port (MIDI wakes us at once); control packets are picked up
            // at the latest when the wait ends.
            let _ = s.data.set_read_timeout(Some(Duration::from_millis(timeout_ms.max(1))));
            if let Ok((n, a)) = s.data.recv_from(&mut buf) {
                got.push((true, buf[..n].to_vec(), a));
            }
            drain(&s.ctl, false, &mut got, &mut buf);
        }
        if s.data.set_nonblocking(true).is_ok() {
            drain(&s.data, true, &mut got, &mut buf);
            let _ = s.data.set_nonblocking(false);
        }
        let mut out = Out::default();
        {
            let mut g = self.inner();
            for (data, pkt, from) in &got {
                self.handle(&mut g, *data, pkt, &addr_text(from), from.port(), &mut out);
            }
            self.timers(&mut g, &mut out);
            g.peers.retain(|p| !p.gone);
        }
        self.flush(&s, &mut out);
        self.deliver(out);
    }

    fn handle(&self, g: &mut Inner, data: bool, p: &[u8], ip: &str, port: u16, out: &mut Out) {
        let which = if data { "data" } else { "ctl " };
        if am_is_session(p) {
            let Some(a) = am_parse(p) else {
                out.log(format!("RTP-MIDI {which} <- {ip}:{port} short session packet [{}]", hex(p)), true);
                return;
            };
            if g.cfg.log_packets {
                out.log(format!("RTP-MIDI {which} <- {ip}:{port}  {}", describe(&a)), true);
            }
            self.handle_session(g, data, &a, ip, port, out);
            return;
        }
        if data && rtp_is_midi(p) {
            self.handle_midi(g, p, ip, port, out);
            return;
        }
        out.log(format!("RTP-MIDI {which} <- {ip}:{port} unrecognised datagram ({} bytes)", p.len()), true);
    }

    fn handle_session(&self, g: &mut Inner, data: bool, a: &AmPacket, ip: &str, port: u16, out: &mut Out) {
        let now = g.now10k();
        let ssrc = g.ssrc;
        let name = g.cfg.name.clone();
        match a.cmd {
            AmCmd::In if !data => {
                // A peer invites us. The same SSRC again (it restarted its side), or a peer we
                // are inviting at this address that got in first: that entry takes the invitation.
                let mut i = g.by_ssrc(a.ssrc, ip);
                if i.is_none() {
                    i = g.peers.iter().position(|q| {
                        !q.gone && q.v.invited && q.v.ip == ip && q.v.state != PeerState::Connected && (q.v.target == port || q.v.ctl_port == port)
                    });
                }
                let i = match i {
                    Some(i) => i,
                    None if g.live() >= g.cfg.max_peers => {
                        g.send(out, false, ip, port, am_invite(AmCmd::No, a.token, ssrc, &name), "NO");
                        out.log(format!("RTP-MIDI: refused '{}' ({ip}): the session already has {} peers", a.name, g.cfg.max_peers), false);
                        return;
                    }
                    None => {
                        g.peers.push(Peer::new(ip));
                        g.peers.len() - 1
                    }
                };
                let accept_ms = g.cfg.accept_ms;
                let mut p = std::mem::replace(&mut g.peers[i], Peer::new(""));
                p.token = a.token;
                p.v.ssrc = a.ssrc;
                p.v.name = a.name.clone();
                p.v.ctl_port = port;
                p.v.data_port = port.wrapping_add(1);
                p.have_seq = false;
                p.ts_in = None;
                p.cycles = 0;
                p.fed = 0;
                p.accept_by = now + accept_ms * 10;
                p.heard(now);
                g.send(out, false, ip, port, am_invite(AmCmd::Ok, a.token, ssrc, &name), "OK");
                p.v.state = PeerState::Waiting; // so the change below is logged even on a re-invite
                g.set_state(&mut p, PeerState::Accepting, "invited our control port", out);
                g.peers[i] = p;
            }
            AmCmd::In => {
                let Some(i) = g.peers.iter().position(|q| !q.gone && q.v.state == PeerState::Accepting && q.token == a.token && q.v.ip == ip) else {
                    g.send(out, true, ip, port, am_invite(AmCmd::No, a.token, ssrc, &name), "NO");
                    out.log(format!("RTP-MIDI: data-port invitation from {ip}:{port} without one on the control port - refused"), true);
                    return;
                };
                let mut p = std::mem::replace(&mut g.peers[i], Peer::new(""));
                p.v.data_port = port;
                g.send(out, true, ip, port, am_invite(AmCmd::Ok, a.token, ssrc, &name), "OK");
                p.heard(now);
                p.next_rs = now + SEC;
                g.set_state(&mut p, PeerState::Connected, "", out);
                g.peers[i] = p;
            }
            AmCmd::Ok => {
                let Some(i) = g.peers.iter().position(|q| !q.gone && q.v.invited && q.token == a.token && q.v.ip == ip) else { return };
                let mut p = std::mem::replace(&mut g.peers[i], Peer::new(""));
                p.heard(now);
                if !data && p.v.state == PeerState::Inviting {
                    p.v.ssrc = a.ssrc;
                    p.v.name = a.name.clone();
                    p.v.ctl_port = port;
                    p.v.data_port = port.wrapping_add(1);
                    p.tries = 0;
                    p.next_at = now;
                    g.set_state(&mut p, PeerState::InvitingData, "control port accepted", out);
                } else if data && p.v.state == PeerState::InvitingData {
                    p.v.data_port = port;
                    p.next_ck = now;
                    p.ck_sent = 0;
                    p.ck_missed = 0;
                    p.ck_ts1 = 0;
                    p.next_rs = now + SEC;
                    g.set_state(&mut p, PeerState::Connected, "", out);
                }
                g.peers[i] = p;
            }
            AmCmd::No => {
                let retry = g.cfg.retry_ms;
                for i in 0..g.peers.len() {
                    let q = &g.peers[i];
                    if !q.gone && q.v.invited && q.token == a.token && q.v.ip == ip && matches!(q.v.state, PeerState::Inviting | PeerState::InvitingData) {
                        let mut p = std::mem::replace(&mut g.peers[i], Peer::new(""));
                        p.next_at = now + retry * 10;
                        g.set_state(&mut p, PeerState::Waiting, "it refused the invitation", out);
                        g.peers[i] = p;
                    }
                }
            }
            AmCmd::By => {
                // A BY before the SSRC was known cancels our invitation: match the token then.
                let i = g.by_ssrc(a.ssrc, ip).or_else(|| g.peers.iter().position(|q| !q.gone && q.token == a.token && q.v.ip == ip));
                let Some(i) = i else { return };
                let bye_retry = g.cfg.bye_retry_ms;
                let mut p = std::mem::replace(&mut g.peers[i], Peer::new(""));
                if p.v.invited {
                    p.next_at = now + bye_retry * 10;
                    g.set_state(&mut p, PeerState::Waiting, "it said goodbye", out);
                } else {
                    p.gone = true;
                    out.log(format!("RTP-MIDI: {} ({ip}) left the session", p.v.display()), false);
                }
                g.peers[i] = p;
            }
            AmCmd::Ck => {
                let Some(i) = g.by_ssrc(a.ssrc, ip) else { return };
                if g.peers[i].v.state != PeerState::Connected {
                    return;
                }
                g.peers[i].heard(now);
                match a.count {
                    0 => g.send(out, true, ip, port, am_clock(ssrc, 1, a.ts[0], now, 0), "CK1"),
                    1 => {
                        if g.peers[i].ck_ts1 != 0 && a.ts[0] == g.peers[i].ck_ts1 {
                            g.send(out, true, ip, port, am_clock(ssrc, 2, a.ts[0], a.ts[1], now), "CK2");
                            let p = &mut g.peers[i];
                            p.v.rtt_ms = Some(now.saturating_sub(a.ts[0]) as f64 / 10.0);
                            p.ck_ts1 = 0;
                            p.ck_missed = 0;
                        }
                    }
                    2 if a.ts[2] >= a.ts[0] => g.peers[i].v.rtt_ms = Some((a.ts[2] - a.ts[0]) as f64 / 10.0),
                    _ => {}
                }
            }
            AmCmd::Rs | AmCmd::Rl => {
                if let Some(i) = g.by_ssrc(a.ssrc, ip) {
                    g.peers[i].heard(now);
                }
            }
            _ => {}
        }
    }

    fn handle_midi(&self, g: &mut Inner, p: &[u8], ip: &str, port: u16, out: &mut Out) {
        let (m, res) = rtp_parse(p);
        let Some(i) = g.by_ssrc(m.ssrc, ip).filter(|&i| g.peers[i].v.state == PeerState::Connected) else {
            if g.strangers.len() < 64 && g.strangers.insert(m.ssrc) {
                out.log(format!("RTP-MIDI: MIDI from {ip}:{port} (SSRC 0x{:08X}), which is not in the session - ignored", m.ssrc), false);
            }
            return;
        };
        let now = g.now10k();
        let peer = &mut g.peers[i];
        peer.heard(now);
        if peer.have_seq {
            let gap = m.seq.wrapping_sub(peer.seq_in).wrapping_sub(1);
            if gap != 0 && gap < 1000 {
                peer.v.lost += gap as u32;
            }
            if m.seq < peer.seq_in && m.seq.wrapping_sub(peer.seq_in) < 1000 {
                peer.cycles += 1; // wrapped
            }
        }
        peer.have_seq = true;
        peer.seq_in = m.seq;
        peer.v.midi += 1;
        if let Err(e) = res {
            out.log(format!("RTP-MIDI: a packet from {} did not decode ({e}); {} message(s) before the fault kept", peer.v.display(), m.events.len()), false);
        }
        if m.events.is_empty() {
            return; // a journal-only packet
        }
        peer.v.last_midi_ms = now / 10;
        let mut t = peer.extend_ts(m.ts);
        for e in m.events {
            t += u64::from(e.delta); // each delta counts from the command before it
            out.midi.push((peer.v.clone(), e.bytes, t * 100));
        }
    }

    fn timers(&self, g: &mut Inner, out: &mut Out) {
        let now = g.now10k();
        let ssrc = g.ssrc;
        let name = g.cfg.name.clone();
        for i in 0..g.peers.len() {
            if g.peers[i].gone {
                continue;
            }
            let mut p = std::mem::replace(&mut g.peers[i], Peer::new(""));
            let s = p.v.state;
            if p.v.invited && matches!(s, PeerState::Inviting | PeerState::InvitingData) {
                if now >= p.next_at {
                    if p.tries >= g.cfg.invite_tries {
                        p.next_at = now + g.cfg.retry_ms * 10;
                        let why = format!("no answer from its {} port", if s == PeerState::Inviting { "control" } else { "data" });
                        g.set_state(&mut p, PeerState::Waiting, &why, out);
                    } else {
                        p.tries += 1;
                        let ctl = s == PeerState::Inviting;
                        g.send(out, !ctl, &p.v.ip, if ctl { p.v.ctl_port } else { p.v.data_port }, am_invite(AmCmd::In, p.token, ssrc, &name), "IN");
                        p.next_at = now + SEC;
                    }
                }
            } else if p.v.invited && s == PeerState::Waiting {
                if now >= p.next_at {
                    g.peers[i] = p;
                    g.start_attempt(i, now);
                    p = std::mem::replace(&mut g.peers[i], Peer::new(""));
                    g.set_state(&mut p, PeerState::Inviting, "", out);
                }
            } else if s == PeerState::Accepting {
                if now > p.accept_by {
                    if p.v.invited {
                        p.next_at = now + g.cfg.retry_ms * 10;
                        g.set_state(&mut p, PeerState::Waiting, "it never invited our data port", out);
                    } else {
                        p.gone = true;
                        out.log(format!("RTP-MIDI: {} ({}) never invited our data port - dropped", p.v.display(), p.v.ip), false);
                    }
                }
            } else if s == PeerState::Connected {
                let mut alive = true;
                if p.v.invited && now >= p.next_ck {
                    // We invited it, so we run clock sync: a burst at the start, then a keep-alive.
                    if p.ck_ts1 != 0 {
                        p.ck_missed += 1;
                    }
                    if p.ck_missed >= 4 && now.saturating_sub(p.last_heard) > 30 * SEC {
                        p.next_at = now + 2 * SEC;
                        g.set_state(&mut p, PeerState::Waiting, "it stopped answering", out);
                        alive = false;
                    } else {
                        p.ck_sent += 1;
                        p.ck_ts1 = now;
                        g.send(out, true, &p.v.ip, p.v.data_port, am_clock(ssrc, 0, now, 0, 0), "CK0");
                        p.next_ck = now + if p.ck_sent < 6 { 15000 } else { 10 * SEC };
                    }
                }
                if alive && !p.v.invited && now.saturating_sub(p.last_heard) > g.cfg.silent_ms * 10 {
                    p.gone = true;
                    out.log(format!("RTP-MIDI: nothing from {} ({}) for {} s - dropped", p.v.display(), p.v.ip, g.cfg.silent_ms / 1000), false);
                    alive = false;
                }
                if alive && now >= p.next_rs {
                    p.next_rs = now + SEC;
                    let ext = (p.cycles << 16) | p.seq_in as u32;
                    if p.have_seq && ext != p.fed {
                        p.fed = ext;
                        g.send(out, false, &p.v.ip, p.v.ctl_port, am_feedback(ssrc, ext), "RS");
                    }
                }
            }
            g.peers[i] = p;
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod addr_tests {
    use super::*;

    #[test]
    fn peer_addresses() {
        let mapped: SocketAddr = SocketAddrV6::new(Ipv4Addr::new(192, 0, 2, 42).to_ipv6_mapped(), 5004, 0, 0).into();
        assert_eq!(addr_text(&mapped), "192.0.2.42", "IPv4 through a dual-stack socket");
        let ll: SocketAddr = SocketAddrV6::new("fe80::1234:5678:9abc:def0".parse().unwrap(), 5004, 0, 3).into();
        assert_eq!(addr_text(&ll), "fe80::1234:5678:9abc:def0%3", "link-local keeps its interface");
        assert_eq!(addr_text(&"[2001:db8::1]:5004".parse().unwrap()), "2001:db8::1");
        assert_eq!(addr_text(&"192.0.2.7:5004".parse().unwrap()), "192.0.2.7");

        assert_eq!(send_addr("192.0.2.7", 5004, true), Some(mapped_of("192.0.2.7", 5004)));
        assert_eq!(send_addr("192.0.2.7", 5004, false), Some("192.0.2.7:5004".parse().unwrap()));
        assert_eq!(send_addr("fe80::1234:5678:9abc:def0%3", 5005, true), Some(SocketAddrV6::new("fe80::1234:5678:9abc:def0".parse().unwrap(), 5005, 0, 3).into()));
        assert_eq!(send_addr("fe80::1%3", 5005, false), None, "no IPv6 from IPv4-only sockets");
        assert_eq!(send_addr("192.0.2.7", 0, true), None, "port 0");
        assert_eq!(send_addr("not an address", 5004, true), None);
        assert_eq!(send_addr("fe80::1%x", 5004, true), None, "bad scope");
    }

    fn mapped_of(ip: &str, port: u16) -> SocketAddr {
        SocketAddrV6::new(ip.parse::<Ipv4Addr>().unwrap().to_ipv6_mapped(), port, 0, 0).into()
    }
}
