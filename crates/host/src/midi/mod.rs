//! One MIDI input, chosen by NAME and kept open from its own thread.
//!
//! Windows renumbers MIDI devices when one comes or goes, so the name is the only stable
//! handle: the port is reopened whenever a port of that name reappears (a USB interface
//! replugged, a network MIDI driver reconnecting). Every OS MIDI call happens on the
//! worker; the driver's callback only queues bytes, and the worker delivers whole
//! messages. Built from Headlink's tools/preprobe/midiport.cpp (demo/pre-probe), input only.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::sys::{now_ms, Worker};

#[cfg(windows)]
mod winmm;
#[cfg(windows)]
use winmm as backend;

#[cfg(not(windows))]
mod none;
#[cfg(not(windows))]
use none as backend;

fn data_len(s: u8) -> usize {
    match s & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 2,
        0xC0 | 0xD0 => 1,
        _ => match s {
            0xF1 | 0xF3 => 1,
            0xF2 => 2,
            _ => 0,
        },
    }
}

/// A byte stream as whole messages: running status expanded, realtime bytes inside a
/// message come out on their own, an incomplete message at the end is dropped.
pub fn split_midi(p: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let (mut running, mut need, mut sysex) = (0u8, 0usize, false);
    for &b in p {
        if b >= 0xF8 {
            out.push(vec![b]);
            continue;
        }
        if sysex {
            if b == 0xF7 {
                cur.push(b);
                out.push(std::mem::take(&mut cur));
                sysex = false;
                continue;
            }
            if b & 0x80 != 0 {
                cur.clear(); // cut short by a status: dropped
                sysex = false;
            } else {
                if cur.len() < 4096 {
                    cur.push(b);
                }
                continue;
            }
        }
        if b & 0x80 != 0 {
            cur = vec![b];
            if b == 0xF0 {
                sysex = true;
                running = 0;
                continue;
            }
            need = data_len(b);
            running = if b < 0xF0 { b } else { 0 };
            if need == 0 {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if cur.is_empty() {
            if running == 0 {
                continue;
            }
            cur = vec![running];
            need = data_len(running);
        }
        cur.push(b);
        need = need.saturating_sub(1);
        if need == 0 {
            out.push(std::mem::take(&mut cur));
        }
    }
    out
}

pub type MidiFn = Box<dyn Fn(&str, &[u8]) + Send + Sync>;
pub type LogFn = Box<dyn Fn(&str) + Send + Sync>;

pub(crate) struct Shared {
    run: AtomicBool,
    state: Mutex<State>,
    cv: Condvar,
    last_ms: AtomicU64,
    midi: MidiFn,
    log: LogFn,
}

#[derive(Default)]
struct State {
    want: String,
    changed: bool,
    ports: Vec<String>,
    open: String,
    queue: VecDeque<Vec<u8>>,
}

impl Shared {
    /// From the driver's callback (any thread): queue bytes for the worker.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn incoming(&self, p: &[u8]) {
        if p.is_empty() {
            return;
        }
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.queue.len() >= 4096 {
                return;
            }
            s.queue.push_back(p.to_vec());
        }
        self.cv.notify_all();
    }
}

pub struct MidiIn {
    shared: Arc<Shared>,
    worker: Mutex<Option<Worker>>,
}

impl MidiIn {
    pub fn start(midi: MidiFn, log: LogFn) -> MidiIn {
        let shared = Arc::new(Shared { run: AtomicBool::new(true), state: Mutex::new(State::default()), cv: Condvar::new(), last_ms: AtomicU64::new(0), midi, log });
        let s2 = shared.clone();
        let w = Worker::spawn("tunesync-midi", move || run(s2));
        MidiIn { shared, worker: Mutex::new(Some(w)) }
    }

    /// Closes the port and ends the worker (bounded).
    pub fn stop(&self, wait: Duration) {
        self.shared.run.store(false, Ordering::SeqCst);
        self.shared.cv.notify_all();
        if let Some(mut w) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            w.join_for(wait);
        }
    }

    /// "" = none. Any thread; never waits on a driver.
    pub fn want(&self, name: &str) {
        {
            let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.want == name {
                return;
            }
            s.want = name.to_string();
            s.changed = true;
        }
        self.shared.cv.notify_all();
    }

    /// The inputs as of the last rescan (every 2 s).
    pub fn ports(&self) -> Vec<String> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).ports.clone()
    }

    /// The port open now, "" when none.
    pub fn open_name(&self) -> String {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).open.clone()
    }

    /// now_ms() of the last message, 0 = none.
    pub fn last_ms(&self) -> u64 {
        self.shared.last_ms.load(Ordering::Relaxed)
    }
}

fn run(sh: Arc<Shared>) {
    let mut port: Option<backend::Port> = None;
    let mut next_scan = Instant::now();
    let mut last_note = String::new();
    let note = |last: &mut String, s: String| {
        if *last != s {
            (sh.log)(&s);
            *last = s;
        }
    };
    while sh.run.load(Ordering::SeqCst) {
        let (changed, want) = {
            let mut s = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            let c = std::mem::take(&mut s.changed);
            (c, s.want.clone())
        };
        if changed && port.is_some() {
            let was = std::mem::take(&mut sh.state.lock().unwrap_or_else(|e| e.into_inner()).open);
            port = None;
            note(&mut last_note, format!("MIDI in: closed {was}"));
        }
        if changed || Instant::now() >= next_scan {
            next_scan = Instant::now() + Duration::from_secs(2);
            let names = backend::list();
            let there = names.iter().any(|n| *n == want);
            sh.state.lock().unwrap_or_else(|e| e.into_inner()).ports = names;
            if port.is_some() && !there {
                port = None;
                sh.state.lock().unwrap_or_else(|e| e.into_inner()).open.clear();
                note(&mut last_note, format!("MIDI in: {want} went away; it is reopened when it comes back"));
            }
            if port.is_none() && !want.is_empty() {
                if there {
                    match backend::Port::open(&want, sh.clone()) {
                        Ok(p) => {
                            port = Some(p);
                            sh.state.lock().unwrap_or_else(|e| e.into_inner()).open = want.clone();
                            note(&mut last_note, format!("MIDI in: listening on {want}"));
                        }
                        Err(e) => note(&mut last_note, format!("MIDI in: cannot open {want}: {e}")),
                    }
                } else if changed {
                    note(&mut last_note, format!("MIDI in: {want} is not connected; it is opened when it appears"));
                }
            }
        }
        if let Some(p) = port.as_mut() {
            p.service();
        }
        let chunks: Vec<Vec<u8>> = {
            let mut s = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.queue.is_empty() && !s.changed {
                s = sh.cv.wait_timeout(s, Duration::from_millis(200)).unwrap_or_else(|e| e.into_inner()).0;
            }
            s.queue.drain(..).collect()
        };
        if !chunks.is_empty() {
            let name = sh.state.lock().unwrap_or_else(|e| e.into_inner()).open.clone();
            for c in chunks {
                for m in split_midi(&c) {
                    sh.last_ms.store(now_ms(), Ordering::Relaxed);
                    (sh.midi)(&name, &m);
                }
            }
        }
    }
    drop(port);
    sh.state.lock().unwrap_or_else(|e| e.into_inner()).open.clear();
}

#[cfg(test)]
mod tests {
    use super::split_midi;

    #[test]
    fn split() {
        assert_eq!(split_midi(&[0x9F, 57, 100]), vec![vec![0x9F, 57, 100]]);
        assert_eq!(split_midi(&[0xBF, 20, 3, 21, 4]), vec![vec![0xBF, 20, 3], vec![0xBF, 21, 4]], "running status");
        assert_eq!(split_midi(&[0x9F, 0xF8, 57, 100]), vec![vec![0xF8], vec![0x9F, 57, 100]], "realtime inside a message");
        assert_eq!(split_midi(&[0xF0, 1, 2, 0xF7, 0xCF, 5]), vec![vec![0xF0, 1, 2, 0xF7], vec![0xCF, 5]]);
        assert_eq!(split_midi(&[0x9F, 57]), Vec::<Vec<u8>>::new(), "incomplete dropped");
        assert_eq!(split_midi(&[57, 100]), Vec::<Vec<u8>>::new(), "data with no status");
    }
}
