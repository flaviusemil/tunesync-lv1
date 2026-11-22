//! macOS DNS-SD through dns_sd.h (mDNSResponder; the functions are in libSystem).
//!
//! One thread per start() owns every DNSServiceRef: it advertises our session
//! (DNSServiceRegister), browses for others (DNSServiceBrowse), resolves each one it finds
//! (DNSServiceResolve, then DNSServiceGetAddrInfo for its IPv4 address) and waits on all
//! their sockets with poll(). dns_sd calls back only from DNSServiceProcessResult, so on
//! that thread too: the callbacks' context is valid for as long as its ref exists, and
//! deallocating the refs before the thread ends is the whole teardown (taking the
//! advertisement down sends mDNS's goodbye).
//!
//! If mDNSResponder restarts (a ref's socket fails), the advertisement and the browse are
//! made again after 10 s.
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::book::Book;
use super::{FoundSession, LogFn};
use crate::sys::{now_ms, Worker};

const REGTYPE: &CStr = c"_apple-midi._udp";
const FLAG_MORE_COMING: u32 = 0x1;
const FLAG_ADD: u32 = 0x2;
const PROTOCOL_IPV4: u32 = 0x1;
const ERR_POLICY_DENIED: i32 = -65570;
const ERR_NAME_CONFLICT: i32 = -65548;
const POLLIN: i16 = 0x1;
const POLLERR: i16 = 0x8;
const POLLHUP: i16 = 0x10;
const AF_INET: u8 = 2;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
const RESTART_AFTER: Duration = Duration::from_secs(10);

type SdRef = *mut c_void;
type RegisterReply = extern "C" fn(SdRef, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
type BrowseReply = extern "C" fn(SdRef, u32, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
type ResolveReply = extern "C" fn(SdRef, u32, u32, i32, *const c_char, *const c_char, u16, u16, *const u8, *mut c_void);
type AddrReply = extern "C" fn(SdRef, u32, u32, i32, *const c_char, *const u8, u32, *mut c_void);

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

extern "C" {
    fn DNSServiceRegister(
        sd: *mut SdRef,
        flags: u32,
        iface: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        host: *const c_char,
        port_be: u16,
        txt_len: u16,
        txt: *const c_void,
        cb: RegisterReply,
        ctx: *mut c_void,
    ) -> i32;
    fn DNSServiceBrowse(sd: *mut SdRef, flags: u32, iface: u32, regtype: *const c_char, domain: *const c_char, cb: BrowseReply, ctx: *mut c_void) -> i32;
    fn DNSServiceResolve(sd: *mut SdRef, flags: u32, iface: u32, name: *const c_char, regtype: *const c_char, domain: *const c_char, cb: ResolveReply, ctx: *mut c_void) -> i32;
    fn DNSServiceGetAddrInfo(sd: *mut SdRef, flags: u32, iface: u32, protocol: u32, host: *const c_char, cb: AddrReply, ctx: *mut c_void) -> i32;
    fn DNSServiceRefSockFD(sd: SdRef) -> i32;
    fn DNSServiceProcessResult(sd: SdRef) -> i32;
    fn DNSServiceRefDeallocate(sd: SdRef);
    fn poll(fds: *mut PollFd, n: u32, timeout_ms: i32) -> i32;
}

unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

#[derive(Default)]
struct Inner {
    advert: String,
    /// The name our registration ended up with (mDNS renames on a conflict).
    own: String,
    book: Book,
    browse_failed_said: bool,
}

struct State {
    name: String,
    port: u16,
    log: LogFn,
    stopping: AtomicBool,
    m: Mutex<Inner>,
}

impl State {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A resolve or address lookup in flight; its Box is the callback's context.
struct Op {
    st: *const State,
    key: String,
    sd: SdRef,
    started: Instant,
    done: bool,
    /// From the resolve: the host to look up next, on this interface.
    then: Option<(CString, u32)>,
}

extern "C" fn on_register(_sd: SdRef, _flags: u32, err: i32, name: *const c_char, _t: *const c_char, _d: *const c_char, ctx: *mut c_void) {
    let st = unsafe { &*(ctx as *const State) };
    let line = {
        let mut g = st.lock();
        if err == 0 {
            let got = unsafe { text(name) };
            let got = if got.is_empty() { st.name.clone() } else { got };
            g.advert = if got == st.name { "advertised".into() } else { format!("advertised as \"{got}\"") };
            g.own = got.clone();
            format!(
                "RTP-MIDI: advertised as \"{got}\" (_apple-midi._udp, port {}) - other Macs list it under MIDI Network Setup -> Directory",
                st.port
            )
        } else {
            g.advert = format!("failed (error {err})");
            match err {
                ERR_POLICY_DENIED => "RTP-MIDI: macOS denied LV1 the local network (System Settings -> Privacy & Security -> Local Network): Macs won't list TuneSync by themselves".to_string(),
                ERR_NAME_CONFLICT => format!("RTP-MIDI: another session on the network is already called \"{}\": rename TuneSync's session", st.name),
                _ => format!("RTP-MIDI: advertising on the network failed (DNS-SD error {err}); on the other Mac, add this host by its address instead"),
            }
        }
    };
    (st.log)(&line, false);
}

extern "C" fn on_browse(_sd: SdRef, flags: u32, iface: u32, err: i32, name: *const c_char, _t: *const c_char, domain: *const c_char, ctx: *mut c_void) {
    let st = unsafe { &*(ctx as *const State) };
    if err != 0 {
        let say = !std::mem::replace(&mut st.lock().browse_failed_said, true);
        if say {
            (st.log)(&format!("RTP-MIDI: browsing the network failed (DNS-SD error {err})"), true);
        }
        return;
    }
    let (name, domain) = unsafe { (text(name), text(domain)) };
    let mut g = st.lock();
    if flags & FLAG_ADD != 0 {
        g.book.add(iface, &name, &domain);
    } else {
        g.book.remove(iface, &name, &domain);
    }
}

extern "C" fn on_resolve(_sd: SdRef, _flags: u32, iface: u32, err: i32, _full: *const c_char, host: *const c_char, port_be: u16, _tl: u16, _txt: *const u8, ctx: *mut c_void) {
    let op = unsafe { &mut *(ctx as *mut Op) };
    let st = unsafe { &*op.st };
    if err == 0 && !host.is_null() {
        let h = unsafe { CStr::from_ptr(host) };
        st.lock().book.target(&op.key, &h.to_string_lossy(), u16::from_be(port_be));
        op.then = Some((h.to_owned(), iface));
    }
    op.done = true;
}

extern "C" fn on_addr(_sd: SdRef, flags: u32, _iface: u32, err: i32, _host: *const c_char, addr: *const u8, _ttl: u32, ctx: *mut c_void) {
    let op = unsafe { &mut *(ctx as *mut Op) };
    let st = unsafe { &*op.st };
    if err == 0 && !addr.is_null() && unsafe { *addr.add(1) } == AF_INET {
        // struct sockaddr_in: len, family, port, then the address
        let a = unsafe { std::slice::from_raw_parts(addr.add(4), 4) };
        st.lock().book.address(&op.key, &format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]));
        op.done = true;
    } else if flags & FLAG_MORE_COMING == 0 {
        op.done = true;
    }
}

#[derive(Default)]
pub struct Dnssd {
    st: Option<Arc<State>>,
    worker: Option<Worker>,
}

impl Dnssd {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advertise `name` on `port` and browse for others. Returns at once.
    pub fn start(&mut self, name: &str, port: u16, log: LogFn) {
        self.stop();
        let st = Arc::new(State {
            name: name.to_string(),
            port,
            log,
            stopping: AtomicBool::new(false),
            m: Mutex::new(Inner { advert: "starting".into(), own: name.to_string(), ..Default::default() }),
        });
        let s2 = st.clone();
        self.worker = Some(Worker::spawn("tunesync-dnssd", move || run(s2)));
        self.st = Some(st);
    }

    /// Takes the advertisement down and stops browsing (bounded).
    pub fn stop(&mut self) {
        if let Some(st) = self.st.take() {
            st.stopping.store(true, Ordering::SeqCst);
            if let Some(mut w) = self.worker.take() {
                w.join_for(Duration::from_millis(1500));
            }
        }
    }

    pub fn found(&self) -> Vec<FoundSession> {
        let Some(st) = &self.st else { return Vec::new() };
        let g = st.lock();
        g.book.found(&g.own)
    }

    pub fn advert_state(&self) -> String {
        self.st.as_ref().map(|s| s.lock().advert.clone()).unwrap_or_else(|| "off".into())
    }
}

fn register(st: &Arc<State>) -> Option<SdRef> {
    let Ok(name) = CString::new(st.name.as_str()) else { return None };
    let mut sd: SdRef = std::ptr::null_mut();
    let ctx = Arc::as_ptr(st) as *mut c_void;
    let r = unsafe {
        DNSServiceRegister(&mut sd, 0, 0, name.as_ptr(), REGTYPE.as_ptr(), std::ptr::null(), std::ptr::null(), st.port.to_be(), 0, std::ptr::null(), on_register, ctx)
    };
    if r != 0 {
        st.lock().advert = format!("failed (error {r})");
        (st.log)(&format!("RTP-MIDI: advertising on the network failed (DNSServiceRegister {r})"), false);
        return None;
    }
    Some(sd)
}

fn browse(st: &Arc<State>) -> Option<SdRef> {
    let mut sd: SdRef = std::ptr::null_mut();
    let r = unsafe { DNSServiceBrowse(&mut sd, 0, 0, REGTYPE.as_ptr(), std::ptr::null(), on_browse, Arc::as_ptr(st) as *mut c_void) };
    if r != 0 {
        (st.log)(&format!("RTP-MIDI: browsing the network failed (DNSServiceBrowse {r})"), true);
        return None;
    }
    Some(sd)
}

fn run(st: Arc<State>) {
    let mut reg: Option<SdRef> = None;
    let mut brw: Option<SdRef> = None;
    let mut reg_at = Some(Instant::now());
    let mut brw_at = Some(Instant::now());
    let mut ops: Vec<Box<Op>> = Vec::new();
    while !st.stopping.load(Ordering::SeqCst) {
        let now = Instant::now();
        if reg.is_none() && reg_at.is_some_and(|t| now >= t) {
            reg = register(&st);
            reg_at = reg.is_none().then(|| now + RESTART_AFTER);
        }
        if brw.is_none() && brw_at.is_some_and(|t| now >= t) {
            brw = browse(&st);
            brw_at = brw.is_none().then(|| now + RESTART_AFTER);
        }
        // start the resolves that are due, at most 4 at a time
        let due = {
            let busy = ops.len();
            st.lock().book.wanted(now_ms(), 4usize.saturating_sub(busy))
        };
        for w in due {
            let (Ok(name), Ok(domain)) = (CString::new(w.name), CString::new(w.domain)) else {
                st.lock().book.ended(&w.key, now_ms());
                continue;
            };
            let mut op = Box::new(Op { st: Arc::as_ptr(&st), key: w.key.clone(), sd: std::ptr::null_mut(), started: now, done: false, then: None });
            let ctx = &mut *op as *mut Op as *mut c_void;
            let r = unsafe { DNSServiceResolve(&mut op.sd, 0, w.iface, name.as_ptr(), REGTYPE.as_ptr(), domain.as_ptr(), on_resolve, ctx) };
            if r == 0 {
                ops.push(op);
            } else {
                st.lock().book.ended(&w.key, now_ms());
            }
        }

        // wait on every socket
        let mut fds: Vec<PollFd> = Vec::new();
        let mut owners: Vec<(u8, usize)> = Vec::new(); // 0 register, 1 browse, 2 op index
        for (kind, sd) in [(0u8, reg), (1u8, brw)] {
            if let Some(sd) = sd {
                fds.push(PollFd { fd: unsafe { DNSServiceRefSockFD(sd) }, events: POLLIN, revents: 0 });
                owners.push((kind, 0));
            }
        }
        for (i, op) in ops.iter().enumerate() {
            fds.push(PollFd { fd: unsafe { DNSServiceRefSockFD(op.sd) }, events: POLLIN, revents: 0 });
            owners.push((2, i));
        }
        let n = unsafe { poll(fds.as_mut_ptr(), fds.len() as u32, 250) };
        if n > 0 {
            for (f, &(kind, i)) in fds.iter().zip(owners.iter()) {
                if f.revents & (POLLIN | POLLERR | POLLHUP) == 0 {
                    continue;
                }
                match kind {
                    0 | 1 => {
                        let slot = if kind == 0 { &mut reg } else { &mut brw };
                        let sd = slot.unwrap();
                        if unsafe { DNSServiceProcessResult(sd) } != 0 {
                            // mDNSResponder went away: make it again later
                            unsafe { DNSServiceRefDeallocate(sd) };
                            *slot = None;
                            let at = Some(Instant::now() + RESTART_AFTER);
                            if kind == 0 {
                                st.lock().advert = "restarting".into();
                                reg_at = at;
                            } else {
                                brw_at = at;
                            }
                        }
                    }
                    _ => {
                        if unsafe { DNSServiceProcessResult(ops[i].sd) } != 0 {
                            ops[i].done = true;
                        }
                    }
                }
            }
        }

        // finished or stuck lookups: the address lookup follows the resolve
        let mut next: Vec<Box<Op>> = Vec::new();
        for mut op in std::mem::take(&mut ops) {
            if !op.done && op.started.elapsed() < RESOLVE_TIMEOUT {
                next.push(op);
                continue;
            }
            unsafe { DNSServiceRefDeallocate(op.sd) };
            match op.then.take() {
                Some((host, iface)) if op.done => {
                    op.sd = std::ptr::null_mut();
                    op.done = false;
                    op.started = Instant::now();
                    let ctx = &mut *op as *mut Op as *mut c_void;
                    let r = unsafe { DNSServiceGetAddrInfo(&mut op.sd, 0, iface, PROTOCOL_IPV4, host.as_ptr(), on_addr, ctx) };
                    if r == 0 {
                        next.push(op);
                    } else {
                        st.lock().book.ended(&op.key, now_ms());
                    }
                }
                _ => st.lock().book.ended(&op.key, now_ms()),
            }
        }
        ops = next;
    }
    for op in ops {
        unsafe { DNSServiceRefDeallocate(op.sd) };
    }
    if let Some(sd) = brw {
        unsafe { DNSServiceRefDeallocate(sd) };
    }
    if let Some(sd) = reg {
        unsafe { DNSServiceRefDeallocate(sd) }; // deregisters: mDNS sends its goodbye
    }
    st.lock().advert = "off".into();
}
