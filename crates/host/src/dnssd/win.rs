//! Windows DNS-SD through dnsapi, every function looked up at run time.
//!
//! One `State` per start(), shared by our thread and dnsapi's callbacks (which arrive on
//! dnsapi's own threads with `pQueryContext`). It is never freed: a callback dnsapi still
//! owes us must find valid memory, and one State is a few hundred bytes per RTP restart.
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{DNS_REQUEST_PENDING, ERROR_SUCCESS};
use windows_sys::Win32::NetworkManagement::Dns::{
    DnsFree, DnsFreeRecordList, DNS_QUERY_REQUEST_VERSION1, DNS_RECORDW, DNS_SERVICE_BROWSE_REQUEST, DNS_SERVICE_BROWSE_REQUEST_0, DNS_SERVICE_CANCEL,
    DNS_SERVICE_INSTANCE, DNS_SERVICE_REGISTER_REQUEST, DNS_SERVICE_RESOLVE_REQUEST, DNS_TYPE_A, DNS_TYPE_PTR, DNS_TYPE_SRV, IP6_ADDRESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW};

use super::{FoundSession, LogFn};
use crate::sys::Worker;

const SERVICE: &str = "_apple-midi._udp.local";

type FnConstruct = unsafe extern "system" fn(*const u16, *const u16, *const u32, *const IP6_ADDRESS, u16, u16, u16, u32, *const *const u16, *const *const u16) -> *mut DNS_SERVICE_INSTANCE;
type FnFreeInstance = unsafe extern "system" fn(*const DNS_SERVICE_INSTANCE);
type FnRegister = unsafe extern "system" fn(*const DNS_SERVICE_REGISTER_REQUEST, *mut DNS_SERVICE_CANCEL) -> u32;
type FnBrowse = unsafe extern "system" fn(*const DNS_SERVICE_BROWSE_REQUEST, *mut DNS_SERVICE_CANCEL) -> i32;
type FnCancel = unsafe extern "system" fn(*const DNS_SERVICE_CANCEL) -> i32;
type FnResolve = unsafe extern "system" fn(*const DNS_SERVICE_RESOLVE_REQUEST, *mut DNS_SERVICE_CANCEL) -> i32;

struct Api {
    construct: FnConstruct,
    free_instance: FnFreeInstance,
    register: FnRegister,
    deregister: FnRegister,
    browse: FnBrowse,
    browse_cancel: FnCancel,
    resolve: FnResolve,
}

fn api() -> Option<&'static Api> {
    static API: std::sync::OnceLock<Option<Api>> = std::sync::OnceLock::new();
    API.get_or_init(|| unsafe {
        let name: Vec<u16> = "dnsapi.dll\0".encode_utf16().collect();
        let dll = LoadLibraryW(name.as_ptr());
        if dll.is_null() {
            return None;
        }
        type Proc = unsafe extern "system" fn() -> isize;
        macro_rules! load {
            ($t:ty, $n:literal) => {
                std::mem::transmute::<Proc, $t>(GetProcAddress(dll, concat!($n, "\0").as_ptr())?)
            };
        }
        Some(Api {
            construct: load!(FnConstruct, "DnsServiceConstructInstance"),
            free_instance: load!(FnFreeInstance, "DnsServiceFreeInstance"),
            register: load!(FnRegister, "DnsServiceRegister"),
            deregister: load!(FnRegister, "DnsServiceDeRegister"),
            browse: load!(FnBrowse, "DnsServiceBrowse"),
            browse_cancel: load!(FnCancel, "DnsServiceBrowseCancel"),
            resolve: load!(FnResolve, "DnsServiceResolve"),
        })
    })
    .as_ref()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

unsafe fn narrow(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 && n < 1024 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

fn ip4(a: u32) -> String {
    let b = a.to_ne_bytes(); // network order in memory
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

fn host_fqdn() -> String {
    let mut b = [0u16; 256];
    let mut n = b.len() as u32;
    unsafe { GetComputerNameExW(ComputerNameDnsHostname, b.as_mut_ptr(), &mut n) };
    format!("{}.local", String::from_utf16_lossy(&b[..n as usize]))
}

#[derive(Default)]
struct Entry {
    s: FoundSession,
    fqdn: String,
    seen: u32,
    resolving: bool,
}

#[derive(Default)]
struct Inner {
    stopping: bool,
    advert: String,
    registered: bool,
    dereg_done: bool,
    browsing: bool,
    round: u32,
    found: BTreeMap<String, Entry>,   // instance FQDN (lower case)
    host_ip: BTreeMap<String, String>, // host (lower case) -> IPv4, from A records
    to_resolve: Vec<String>,
    resolving: u32,
}

struct State {
    name: String,
    host: String,
    port: u16,
    log: LogFn,
    m: Mutex<Inner>,
    cv: Condvar,
    // owned by the thread, kept alive for dnsapi
    reg: Mutex<DNS_SERVICE_REGISTER_REQUEST>,
    browse_cancel: Mutex<DNS_SERVICE_CANCEL>,
}

unsafe impl Send for State {}
unsafe impl Sync for State {}

impl State {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }
}

unsafe extern "system" fn on_register(status: u32, ctx: *const c_void, inst: *const DNS_SERVICE_INSTANCE) {
    let st = &*(ctx as *const State);
    if let (Some(a), false) = (api(), inst.is_null()) {
        (a.free_instance)(inst);
    }
    let line = {
        let mut g = st.lock();
        if g.stopping {
            g.dereg_done = true;
            None
        } else if status == ERROR_SUCCESS {
            g.registered = true;
            g.advert = "advertised".into();
            Some(format!(
                "RTP-MIDI: advertised as \"{}\" (_apple-midi._udp, port {}) - Macs list it under MIDI Network Setup -> Directory",
                st.name, st.port
            ))
        } else {
            g.advert = format!("failed (error {status})");
            Some(format!("RTP-MIDI: advertising on the network failed (DNS-SD error {status}); on the Mac, add this host by its address instead"))
        }
    };
    st.cv.notify_all();
    if let Some(l) = line {
        (st.log)(&l, false);
    }
}

unsafe extern "system" fn on_resolve(status: u32, ctx: *const c_void, inst: *const DNS_SERVICE_INSTANCE) {
    let st = &*(ctx as *const State);
    {
        let mut g = st.lock();
        g.resolving = g.resolving.saturating_sub(1);
        if status == ERROR_SUCCESS && !inst.is_null() {
            let i = &*inst;
            let fq = narrow(i.pszInstanceName).to_lowercase();
            if let Some(e) = g.found.get_mut(&fq) {
                e.resolving = false;
                if !i.pszHostName.is_null() {
                    e.s.host = narrow(i.pszHostName);
                }
                if i.wPort != 0 {
                    e.s.port = i.wPort;
                }
                if !i.ip4Address.is_null() {
                    e.s.ip = ip4(*i.ip4Address);
                }
            }
        }
    }
    if let (Some(a), false) = (api(), inst.is_null()) {
        (a.free_instance)(inst);
    }
    st.cv.notify_all();
}

unsafe extern "system" fn on_browse(status: u32, ctx: *const c_void, recs: *const DNS_RECORDW) {
    let st = &*(ctx as *const State);
    {
        let mut g = st.lock();
        if status != ERROR_SUCCESS {
            g.browsing = false; // finished or cancelled
        }
        let suffix = format!(".{SERVICE}");
        let mut r = recs;
        while !r.is_null() {
            let rec = &*r;
            let owner = narrow(rec.pName).to_lowercase();
            match rec.wType {
                DNS_TYPE_PTR => {
                    let fq_name = narrow(rec.Data.PTR.pNameHost);
                    let fq = fq_name.to_lowercase();
                    if rec.dwTtl == 0 {
                        g.found.remove(&fq); // a goodbye
                    } else {
                        let round = g.round;
                        let e = g.found.entry(fq.clone()).or_default();
                        e.fqdn = fq_name.clone();
                        e.seen = round;
                        if e.s.instance.is_empty() {
                            e.s.instance = match fq.rfind(&suffix) {
                                Some(k) => fq_name[..k].to_string(),
                                None => fq_name.clone(),
                            };
                        }
                    }
                }
                DNS_TYPE_SRV => {
                    if let Some(e) = g.found.get_mut(&owner) {
                        e.s.host = narrow(rec.Data.SRV.pNameTarget);
                        e.s.port = rec.Data.SRV.wPort;
                    }
                }
                DNS_TYPE_A => {
                    g.host_ip.insert(owner, ip4(rec.Data.A.IpAddress));
                }
                _ => {}
            }
            r = rec.pNext;
        }
        let host_ip = g.host_ip.clone();
        let mut todo = Vec::new();
        for e in g.found.values_mut() {
            if e.s.ip.is_empty() {
                if let Some(ip) = host_ip.get(&e.s.host.to_lowercase()) {
                    e.s.ip = ip.clone();
                }
            }
            if (e.s.ip.is_empty() || e.s.port == 0) && !e.resolving {
                e.resolving = true;
                todo.push(e.fqdn.clone());
            }
        }
        g.to_resolve.extend(todo);
    }
    if !recs.is_null() {
        DnsFree(recs as *const c_void, DnsFreeRecordList);
    }
    st.cv.notify_all();
}

#[derive(Default)]
pub struct Dnssd {
    st: Option<&'static State>,
    worker: Option<Worker>,
}

impl Dnssd {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advertise `name` on `port` and browse for others. Returns at once.
    pub fn start(&mut self, name: &str, port: u16, log: LogFn) {
        self.stop();
        let st: &'static State = Box::leak(Box::new(State {
            name: name.to_string(),
            host: host_fqdn(),
            port,
            log,
            m: Mutex::new(Inner { advert: "starting".into(), ..Default::default() }),
            cv: Condvar::new(),
            reg: Mutex::new(unsafe { std::mem::zeroed() }),
            browse_cancel: Mutex::new(DNS_SERVICE_CANCEL { reserved: std::ptr::null_mut() }),
        }));
        self.st = Some(st);
        let Some(a) = api() else {
            st.lock().advert = "not available on this Windows".into();
            (st.log)(
                "RTP-MIDI: this Windows has no DNS-SD (it needs Windows 10 1809 or later): Macs won't list TuneSync by themselves; add this host by its address on the Mac",
                false,
            );
            return;
        };
        self.worker = Some(Worker::spawn("tunesync-dnssd", move || run(st, a)));
    }

    /// Takes the advertisement down and cancels the browse (bounded).
    pub fn stop(&mut self) {
        if let Some(st) = self.st.take() {
            st.lock().stopping = true;
            st.cv.notify_all();
            if let Some(mut w) = self.worker.take() {
                w.join_for(Duration::from_millis(1500));
            }
        }
    }

    pub fn found(&self) -> Vec<FoundSession> {
        let Some(st) = self.st else { return Vec::new() };
        let g = st.lock();
        g.found
            .values()
            .filter(|e| !(e.s.instance == st.name && e.s.host.eq_ignore_ascii_case(&st.host))) // ourselves
            .map(|e| e.s.clone())
            .collect()
    }

    pub fn advert_state(&self) -> String {
        self.st.map(|s| s.lock().advert.clone()).unwrap_or_else(|| "off".into())
    }
}

fn run(st: &'static State, a: &'static Api) {
    let ctx = st as *const State as *mut c_void;
    // ---- advertise
    let inst_name = wide(&format!("{}.{SERVICE}", st.name));
    let host = wide(&st.host);
    let inst = unsafe { (a.construct)(inst_name.as_ptr(), host.as_ptr(), std::ptr::null(), std::ptr::null(), st.port, 0, 0, 0, std::ptr::null(), std::ptr::null()) };
    if !inst.is_null() {
        let mut req = st.reg.lock().unwrap_or_else(|e| e.into_inner());
        req.Version = DNS_QUERY_REQUEST_VERSION1;
        req.InterfaceIndex = 0;
        req.pServiceInstance = inst;
        req.pRegisterCompletionCallback = Some(on_register);
        req.pQueryContext = ctx;
        req.unicastEnabled = 0;
        let r = unsafe { (a.register)(&*req, std::ptr::null_mut()) };
        drop(req);
        if r != DNS_REQUEST_PENDING as u32 {
            st.lock().advert = format!("failed (error {r})");
            (st.log)(&format!("RTP-MIDI: advertising on the network failed (DnsServiceRegister {r})"), false);
        }
    }
    // ---- browse, again every 30 s: entries not seen for two rounds are dropped
    let service = wide(SERVICE);
    let mut next_round = Instant::now();
    let mut resolve_names: Vec<Vec<u16>> = Vec::new(); // kept alive while dnsapi may read them
    loop {
        let mut g = st.lock();
        if g.stopping {
            break;
        }
        if Instant::now() >= next_round {
            if g.browsing {
                drop(g);
                unsafe { (a.browse_cancel)(&*st.browse_cancel.lock().unwrap_or_else(|e| e.into_inner())) };
                g = st.lock();
                g = st.cv.wait_timeout_while(g, Duration::from_millis(500), |g| g.browsing && !g.stopping).unwrap_or_else(|e| e.into_inner()).0;
            }
            g.round += 1;
            let round = g.round;
            g.found.retain(|_, e| round - e.seen <= 2);
            drop(g);
            let req = DNS_SERVICE_BROWSE_REQUEST {
                Version: DNS_QUERY_REQUEST_VERSION1,
                InterfaceIndex: 0,
                QueryName: service.as_ptr(),
                Anonymous: DNS_SERVICE_BROWSE_REQUEST_0 { pBrowseCallback: Some(on_browse) },
                pQueryContext: ctx,
            };
            let r = {
                let mut c = st.browse_cancel.lock().unwrap_or_else(|e| e.into_inner());
                *c = DNS_SERVICE_CANCEL { reserved: std::ptr::null_mut() };
                unsafe { (a.browse)(&req, &mut *c) }
            };
            g = st.lock();
            g.browsing = r == DNS_REQUEST_PENDING;
            if !g.browsing && round == 1 {
                (st.log)(&format!("RTP-MIDI: browsing the network failed (DnsServiceBrowse {r})"), true);
            }
            next_round = Instant::now() + Duration::from_secs(30);
        }
        while g.resolving < 4 {
            let Some(q) = g.to_resolve.pop() else { break };
            g.resolving += 1;
            drop(g);
            resolve_names.push(wide(&q));
            if resolve_names.len() > 64 {
                resolve_names.remove(0);
            }
            let req = DNS_SERVICE_RESOLVE_REQUEST {
                Version: DNS_QUERY_REQUEST_VERSION1,
                InterfaceIndex: 0,
                QueryName: resolve_names.last().unwrap().as_ptr() as *mut u16,
                pResolveCompletionCallback: Some(on_resolve),
                pQueryContext: ctx,
            };
            let mut cancel = DNS_SERVICE_CANCEL { reserved: std::ptr::null_mut() };
            let r = unsafe { (a.resolve)(&req, &mut cancel) };
            g = st.lock();
            if r != DNS_REQUEST_PENDING {
                g.resolving = g.resolving.saturating_sub(1);
            }
        }
        let _ = st.cv.wait_timeout(g, Duration::from_millis(500));
    }
    // ---- stop: cancel the browse, take the advertisement down
    let (browsing, registered) = {
        let g = st.lock();
        (g.browsing, g.registered)
    };
    if browsing {
        unsafe { (a.browse_cancel)(&*st.browse_cancel.lock().unwrap_or_else(|e| e.into_inner())) };
    }
    if registered {
        let req = *st.reg.lock().unwrap_or_else(|e| e.into_inner());
        if unsafe { (a.deregister)(&req, std::ptr::null_mut()) } == DNS_REQUEST_PENDING as u32 {
            let g = st.lock();
            let _ = st.cv.wait_timeout_while(g, Duration::from_millis(800), |g| !g.dereg_done);
        }
    }
}
