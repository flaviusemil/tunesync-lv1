//! CoreMIDI input (macOS), from Headlink's tools/preprobe/midiport.cpp (demo/pre-probe).
//!
//! - **The client is made on the main queue.** CoreMIDI reports sources that come and go
//!   through the run loop of the thread that made the process's first client. The MIDI
//!   worker has no run loop, so the client is made by a job on the main queue (LV1's main
//!   thread, whose loop always runs), and the worker waits for it ([`ready`]) without
//!   blocking anyone: LV1 may be busy loading a session for a while when TuneSync starts.
//!   Only if the main queue has not run the job after 30 s (a host with no main loop) is
//!   the client made on the worker; that host then sees only the sources there at the start.
//! - **The read callback only queues.** It runs on CoreMIDI's own high-priority thread
//!   and hands the bytes to [`Shared::incoming`]; the worker splits and delivers them.
//! - **A source is chosen by name** (its display name, as Audio MIDI Setup shows it), and
//!   the generic worker in `mod.rs` reopens it when a source of that name comes back.
//! - The read callback's context is never freed: CoreMIDI does not promise that no call
//!   is in flight once MIDIPortDispose returns. It is a few bytes per (re)open, and it
//!   holds the worker's state weakly, so a late call after TuneSync stopped does nothing.
use std::ffi::c_void;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::{walk_packet_list, Shared};
use crate::mac::{self, CFStringRef, CfString};

type ObjectRef = u32;
type OsStatus = i32;
type ReadProc = extern "C" fn(list: *const u8, read_ref: *mut c_void, src_ref: *mut c_void);

#[link(name = "CoreMIDI", kind = "framework")]
extern "C" {
    fn MIDIClientCreate(name: CFStringRef, notify: *const c_void, notify_ref: *mut c_void, out: *mut ObjectRef) -> OsStatus;
    // Deprecated since macOS 11 in favour of the block-based MIDIInputPortCreateWithProtocol,
    // but still there, and it takes a plain function: no blocks needed from Rust.
    fn MIDIInputPortCreate(client: ObjectRef, name: CFStringRef, read: ReadProc, read_ref: *mut c_void, out: *mut ObjectRef) -> OsStatus;
    fn MIDIPortConnectSource(port: ObjectRef, source: ObjectRef, conn_ref: *mut c_void) -> OsStatus;
    fn MIDIPortDispose(port: ObjectRef) -> OsStatus;
    fn MIDIGetNumberOfSources() -> usize;
    fn MIDIGetSource(i: usize) -> ObjectRef;
    fn MIDIObjectGetStringProperty(obj: ObjectRef, prop: CFStringRef, out: *mut CFStringRef) -> OsStatus;
    static kMIDIPropertyDisplayName: CFStringRef;
    static kMIDIPropertyName: CFStringRef;
}

fn make_client() -> ObjectRef {
    let name = CfString::new("TuneSync");
    let mut c: ObjectRef = 0;
    let r = unsafe { MIDIClientCreate(name.as_ptr(), std::ptr::null(), std::ptr::null_mut(), &mut c) };
    if r == 0 {
        c
    } else {
        0
    }
}

#[derive(Clone, Copy)]
enum Client {
    None,
    Asked(Instant),
    Made(ObjectRef),
}

static CLIENT: Mutex<Client> = Mutex::new(Client::None);

/// The process's client (see the module notes); 0 while the main queue has not made it
/// yet, or when CoreMIDI refused (MIDIClientCreate failed: then it is asked again later).
fn client() -> ObjectRef {
    let mut g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    match *g {
        Client::Made(c) => c,
        Client::None if mac::is_main_thread() => {
            let c = make_client();
            *g = if c != 0 { Client::Made(c) } else { Client::None };
            c
        }
        Client::None => {
            *g = Client::Asked(Instant::now());
            mac::main_async(|| {
                let mut g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
                if let Client::Asked(_) = *g {
                    // refused: asked again (by the worker's 30 s fallback), not in a loop
                    let c = make_client();
                    *g = if c != 0 { Client::Made(c) } else { Client::Asked(Instant::now()) };
                }
            });
            0
        }
        Client::Asked(t) if t.elapsed() > Duration::from_secs(30) => {
            let c = make_client();
            *g = if c != 0 { Client::Made(c) } else { Client::None };
            c
        }
        Client::Asked(_) => 0,
    }
}

/// The client exists: the worker lists and opens sources only from then on.
pub fn ready() -> bool {
    client() != 0
}

fn name_of(o: ObjectRef) -> String {
    for prop in unsafe { [kMIDIPropertyDisplayName, kMIDIPropertyName] } {
        let mut s: CFStringRef = std::ptr::null();
        if unsafe { MIDIObjectGetStringProperty(o, prop, &mut s) } == 0 && !s.is_null() {
            let t = unsafe { mac::to_string(s) };
            unsafe { mac::release(s) };
            if !t.is_empty() {
                return t;
            }
        }
    }
    "?".into()
}

fn sources() -> Vec<(ObjectRef, String)> {
    if client() == 0 {
        return Vec::new(); // without a client the server does not answer
    }
    let n = unsafe { MIDIGetNumberOfSources() }.min(256);
    (0..n).map(|i| unsafe { MIDIGetSource(i) }).filter(|&e| e != 0).map(|e| (e, name_of(e))).collect()
}

pub fn list() -> Vec<String> {
    sources().into_iter().map(|(_, n)| n).collect()
}

struct Ctx {
    sh: Weak<Shared>,
}

extern "C" fn read(list: *const u8, read_ref: *mut c_void, _src: *mut c_void) {
    let ctx = unsafe { &*(read_ref as *const Ctx) };
    let Some(sh) = ctx.sh.upgrade() else { return };
    // CoreMIDI aligns each packet to 4 bytes on ARM and packs them on Intel.
    unsafe { walk_packet_list(list, usize::MAX, cfg!(target_arch = "aarch64"), |p| sh.incoming(p)) };
}

pub struct Port {
    port: ObjectRef,
}

impl Port {
    pub fn open(name: &str, sh: Arc<Shared>) -> Result<Port, String> {
        let c = client();
        if c == 0 {
            return Err("CoreMIDI refused a client (MIDIClientCreate failed)".into());
        }
        let Some((src, _)) = sources().into_iter().find(|(_, n)| n == name) else { return Err("not connected".into()) };
        let ctx = Box::into_raw(Box::new(Ctx { sh: Arc::downgrade(&sh) })) as *mut c_void; // never freed, see above
        let pname = CfString::new("TuneSync in");
        let mut port: ObjectRef = 0;
        let r = unsafe { MIDIInputPortCreate(c, pname.as_ptr(), read, ctx, &mut port) };
        if r != 0 {
            return Err(format!("MIDIInputPortCreate failed ({r})"));
        }
        let r = unsafe { MIDIPortConnectSource(port, src, std::ptr::null_mut()) };
        if r != 0 {
            unsafe { MIDIPortDispose(port) };
            return Err(format!("MIDIPortConnectSource failed ({r})"));
        }
        Ok(Port { port })
    }

    pub fn service(&mut self) {}
}

impl Drop for Port {
    fn drop(&mut self) {
        unsafe { MIDIPortDispose(self.port) };
    }
}
