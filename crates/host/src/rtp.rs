//! The RTP-MIDI session, DNS-SD and the thread that runs them. Settings changes and
//! Connect / Forget only queue; the thread applies them.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rtpmidi::session::{PeerView, Session, SessionConfig};
use tunesync::settings::Peer;

use crate::dnssd::{Dnssd, FoundSession};
use crate::sys::{now_10k, now_ms, Worker};

pub type LogFn = Arc<dyn Fn(&str, bool) + Send + Sync>;
pub type MidiFn = Arc<dyn Fn(&str, &[u8]) + Send + Sync>;

/// What the window shows of RTP-MIDI.
#[derive(Clone, Debug, Default)]
pub struct RtpView {
    pub enabled: bool,
    pub open: bool,
    /// Why it is not open.
    pub error: String,
    pub name: String,
    pub port: u16,
    /// DNS-SD: "advertised", "not available on this Windows", ...
    pub advert: String,
    pub peers: Vec<PeerView>,
    pub found: Vec<FoundSession>,
    /// The clock `PeerView`'s times are on (`now_ms()`'s).
    pub session_now_ms: u64,
    /// now_ms() of the last MIDI message, 0 = none.
    pub last_midi_ms: u64,
}

#[derive(Default)]
struct Want {
    enabled: bool,
    name: String,
    port: u16,
    peers: Vec<Peer>,
    changed: bool,
    cmds: VecDeque<(bool, String, u16, String)>, // (connect?, ip, port, label)
    open: bool,
    error: String,
}

struct Shared {
    run: AtomicBool,
    want: Mutex<Want>,
    cv: Condvar,
    session: Session,
    dnssd: Mutex<Dnssd>,
    last_midi: Arc<AtomicU64>,
    log: LogFn,
}

pub struct RtpService {
    sh: Arc<Shared>,
    worker: Mutex<Option<Worker>>,
}

impl RtpService {
    pub fn start(log: LogFn, midi: MidiFn) -> RtpService {
        let mut session = Session::new();
        let l2 = log.clone();
        session.on_log(move |l, detail| l2(l, detail));
        let last = Arc::new(AtomicU64::new(0));
        let last2 = last.clone();
        session.on_midi(move |p, m| {
            last2.store(now_ms(), Ordering::Relaxed);
            midi(&format!("RTP {}", p.display()), m);
        });
        let sh = Arc::new(Shared {
            run: AtomicBool::new(true),
            want: Mutex::new(Want { name: "TuneSync".into(), port: 5004, ..Default::default() }),
            cv: Condvar::new(),
            session,
            dnssd: Mutex::new(Dnssd::new()),
            last_midi: last,
            log,
        });
        let s2 = sh.clone();
        let w = Worker::spawn("tunesync-rtp", move || run(s2));
        RtpService { sh, worker: Mutex::new(Some(w)) }
    }

    pub fn stop(&self, wait: Duration) {
        self.sh.run.store(false, Ordering::SeqCst);
        self.sh.cv.notify_all();
        if let Some(mut w) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            w.join_for(wait);
        }
    }

    /// The RTP thread (re)opens the session when the name, port or switch changed.
    pub fn configure(&self, enabled: bool, name: &str, port: u16, peers: &[Peer]) {
        {
            let mut w = self.sh.want.lock().unwrap_or_else(|e| e.into_inner());
            let reopen = enabled != w.enabled || name != w.name || port != w.port;
            w.enabled = enabled;
            w.name = name.to_string();
            w.port = port;
            w.peers = peers.to_vec();
            w.changed |= reopen;
        }
        self.sh.cv.notify_all();
    }

    pub fn connect(&self, ip: &str, port: u16, label: &str) {
        self.sh.want.lock().unwrap_or_else(|e| e.into_inner()).cmds.push_back((true, ip.into(), port, label.into()));
        self.sh.cv.notify_all();
    }

    pub fn forget(&self, ip: &str, port: u16) {
        self.sh.want.lock().unwrap_or_else(|e| e.into_inner()).cmds.push_back((false, ip.into(), port, String::new()));
        self.sh.cv.notify_all();
    }

    pub fn view(&self) -> RtpView {
        let mut v = {
            let w = self.sh.want.lock().unwrap_or_else(|e| e.into_inner());
            RtpView { enabled: w.enabled, open: w.open, error: w.error.clone(), name: w.name.clone(), port: w.port, ..Default::default() }
        };
        if v.open {
            v.peers = self.sh.session.peers();
            v.session_now_ms = self.sh.session.now_ms();
            let d = self.sh.dnssd.lock().unwrap_or_else(|e| e.into_inner());
            v.found = d.found();
            v.advert = d.advert_state();
        }
        v.last_midi_ms = self.sh.last_midi.load(Ordering::Relaxed);
        v
    }
}

fn run(sh: Arc<Shared>) {
    while sh.run.load(Ordering::SeqCst) {
        let (changed, enabled, name, port, peers, cmds) = {
            let mut w = sh.want.lock().unwrap_or_else(|e| e.into_inner());
            (std::mem::take(&mut w.changed), w.enabled, w.name.clone(), w.port, w.peers.clone(), std::mem::take(&mut w.cmds))
        };
        if changed {
            if sh.session.is_open() {
                sh.dnssd.lock().unwrap_or_else(|e| e.into_inner()).stop();
                sh.session.close();
                (sh.log)("RTP-MIDI: session closed", false);
            }
            let mut err = String::new();
            if enabled {
                match sh.session.open(SessionConfig { name: name.clone(), port, clock10k: Some(Arc::new(now_10k)), ..Default::default() }) {
                    Ok(()) => {
                        (sh.log)(&format!("RTP-MIDI: session \"{name}\" open on UDP {port}/{}", port + 1), false);
                        sh.dnssd.lock().unwrap_or_else(|e| e.into_inner()).start(&name, port, sh.log.clone());
                        for p in &peers {
                            sh.session.invite(&p.ip, p.port, &p.name);
                        }
                    }
                    Err(e) => {
                        (sh.log)(&format!("RTP-MIDI: cannot open the session: {e}"), false);
                        err = e;
                    }
                }
            }
            let mut w = sh.want.lock().unwrap_or_else(|e| e.into_inner());
            w.open = sh.session.is_open();
            w.error = err;
        }
        for (connect, ip, port, label) in cmds {
            if connect {
                sh.session.invite(&ip, port, &label);
            } else {
                sh.session.forget(&ip, port);
            }
        }
        if sh.session.is_open() {
            sh.session.poll(50);
        } else {
            let w = sh.want.lock().unwrap_or_else(|e| e.into_inner());
            let _ = sh.cv.wait_timeout_while(w, Duration::from_millis(250), |w| sh.run.load(Ordering::SeqCst) && !w.changed && w.cmds.is_empty());
        }
    }
    sh.dnssd.lock().unwrap_or_else(|e| e.into_inner()).stop();
    sh.session.close();
    sh.want.lock().unwrap_or_else(|e| e.into_inner()).open = false;
}
