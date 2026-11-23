//! TuneSync running inside a host (LV1 through the surface driver, or Headlink through
//! the C API): the engine's worker thread, the MIDI input, the RTP-MIDI service, the
//! settings file and the log file.
//!
//! Threads (none of them is the host's; the host's threads only call in and return):
//! - **worker**: the engine's `tick()`, the ONLY thread that calls the [`Mixer`] (LV1);
//! - **midi**: the local input by name ([`MidiIn`]), unless the host feeds MIDI itself;
//! - **rtp**: the RTP-MIDI session and DNS-SD ([`RtpService`]), unless switched off.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tunesync::apply::{Mixer, TuneInstance};
use tunesync::engine::{Engine, Status};
use tunesync::log::{Level, LogLine, LogRing};
use tunesync::midi_map::Intent;
use tunesync::settings::{from_ini, to_ini, Peer, Settings, MAX_RTP_PEERS};
use tunesync::tune::Spelling;

use crate::filelog::FileLog;
use crate::midi::{split_midi, MidiIn};
use crate::rtp::{RtpService, RtpView};
use crate::sys::{clock_text, now_ms, now_us, Worker};

pub struct RuntimeConfig {
    /// Where settings.ini and tunesync.log live.
    pub dir: PathBuf,
    /// "TuneSync 0.1.0", for the first log line.
    pub title: String,
    /// Asked once a pass: a reason when the host stopped answering (a fault in LV1).
    pub health: Option<Box<dyn Fn() -> Option<String> + Send + Sync>>,
    /// TuneSync opens the MIDI input named in the settings. Off: the host feeds MIDI
    /// through [`Runtime::feed_midi`] and `midi_port` is kept but not used.
    pub local_midi: bool,
    /// TuneSync runs the RTP-MIDI session and DNS-SD. Off: `rtp_*` settings are kept but
    /// not used.
    pub rtp: bool,
}

impl RuntimeConfig {
    /// The whole runtime: TuneSync's own MIDI input and RTP-MIDI, no health check.
    pub fn new(dir: impl Into<PathBuf>, title: impl Into<String>) -> Self {
        RuntimeConfig { dir: dir.into(), title: title.into(), health: None, local_midi: true, rtp: true }
    }
}

struct Wake {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Wake {
    fn set(&self) {
        *self.flag.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_all();
    }
    fn wait(&self, ms: u64) {
        let g = self.flag.lock().unwrap_or_else(|e| e.into_inner());
        let mut g = self.cv.wait_timeout_while(g, Duration::from_millis(ms), |f| !*f).unwrap_or_else(|e| e.into_inner()).0;
        *g = false;
    }
}

pub struct Runtime {
    log: Arc<LogRing>,
    file: Arc<FileLog>,
    engine: Arc<Engine>,
    settings: Mutex<Settings>,
    midi: Option<MidiIn>,
    rtp: Option<RtpService>,
    run: Arc<AtomicBool>,
    wake: Arc<Wake>,
    dir: PathBuf,
    worker: Mutex<Option<Worker>>,
}

impl Runtime {
    /// Reads the settings, starts every thread and returns. `mixer` moves to the worker.
    pub fn start(cfg: RuntimeConfig, mixer: Box<dyn Mixer + Send>) -> Arc<Runtime> {
        let _ = std::fs::create_dir_all(&cfg.dir);
        let file = Arc::new(FileLog::new(cfg.dir.join("tunesync.log")));
        let f2 = file.clone();
        let log = Arc::new(LogRing::default().with_stamper(clock_text).with_sink(move |l| f2.add(l, false)));
        let wake = Arc::new(Wake { flag: Mutex::new(false), cv: Condvar::new() });
        let mut engine = Engine::new(log.clone());
        let w2 = wake.clone();
        engine.set_wake(move || w2.set());
        let engine = Arc::new(engine);

        let mut warnings = Vec::new();
        let ini = std::fs::read_to_string(cfg.dir.join("settings.ini")).unwrap_or_default();
        let settings = from_ini(&ini, &mut warnings);
        log.add(Level::Info, format!("{} started{}", cfg.title, if ini.is_empty() { " (default settings)" } else { "" }));
        for w in warnings {
            log.add(Level::Warn, format!("settings.ini: {w}"));
        }

        let (e2, l2) = (engine.clone(), log.clone());
        let midi = cfg
            .local_midi
            .then(|| MidiIn::start(Box::new(move |port, m| e2.on_midi_at(&format!("MIDI {port}"), m, now_us(), None)), Box::new(move |l| l2.add(Level::Info, l))));
        let (l3, f3, e3) = (log.clone(), file.clone(), engine.clone());
        let rtp = cfg.rtp.then(|| {
            RtpService::start(
                Arc::new(move |l: &str, detail: bool| {
                    if detail {
                        f3.add(&LogLine { stamp: String::new(), level: Level::Info, text: l.to_string() }, true);
                    } else {
                        l3.add(Level::Info, l);
                    }
                }),
                Arc::new(move |src: &str, m: &[u8], sent_us: u64| e3.on_midi_at(src, m, now_us(), Some(sent_us))),
            )
        });
        if !cfg.local_midi || !cfg.rtp {
            let off: Vec<&str> = [(!cfg.local_midi).then_some("MIDI input"), (!cfg.rtp).then_some("RTP-MIDI")].into_iter().flatten().collect();
            log.add(Level::Info, format!("{} left to the host", off.join(" and ")));
        }

        let rt = Arc::new(Runtime {
            log,
            file,
            engine,
            settings: Mutex::new(settings.clone()),
            midi,
            rtp,
            run: Arc::new(AtomicBool::new(true)),
            wake,
            dir: cfg.dir,
            worker: Mutex::new(None),
        });
        rt.apply(&settings);
        let r2 = rt.clone();
        let health = cfg.health;
        *rt.worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(Worker::spawn("tunesync-worker", move || r2.work(mixer, health)));
        rt
    }

    fn work(&self, mut mixer: Box<dyn Mixer + Send>, health: Option<Box<dyn Fn() -> Option<String> + Send + Sync>>) {
        let mut said = false;
        self.engine.request_scan(); // the Tunes list is ready when the window first opens
        while self.run.load(Ordering::SeqCst) {
            let t0 = now_ms();
            let before = self.engine.status().scans;
            let next = self.engine.tick(&mut *mixer, t0);
            let took = now_ms() - t0;
            if self.engine.status().scans != before && took > 200 {
                self.log.add(Level::Warn, format!("a pass over LV1 took {took} ms"));
            }
            if !said {
                if let Some(why) = health.as_ref().and_then(|h| h()) {
                    said = true;
                    self.log.add(Level::Warn, why);
                }
            }
            self.file.flush();
            self.wake.wait(next.unwrap_or(250).clamp(1, 250));
        }
        self.file.flush();
    }

    /// Stops every thread (bounded). The runtime stays allocated for any late caller.
    pub fn stop(&self) {
        if !self.run.swap(false, Ordering::SeqCst) {
            return;
        }
        self.log.add(Level::Info, "TuneSync stopping");
        self.wake.set();
        if let Some(mut w) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            if !w.join_for(Duration::from_millis(1500)) {
                self.log.add(Level::Warn, "the worker did not stop in time");
            }
        }
        if let Some(m) = &self.midi {
            m.stop(Duration::from_millis(1000));
        }
        if let Some(r) = &self.rtp {
            r.stop(Duration::from_millis(1500));
        }
        self.file.flush();
    }

    fn apply(&self, s: &Settings) {
        self.engine.configure(s);
        if let Some(m) = &self.midi {
            m.want(&s.midi_port);
        }
        if let Some(r) = &self.rtp {
            r.configure(s.rtp_enabled, &s.rtp_name, s.rtp_port, &s.rtp_peers);
        }
    }

    fn save(&self, s: &Settings) {
        let path = self.dir.join("settings.ini");
        let tmp = self.dir.join("settings.ini.tmp");
        // Written beside and moved over, so a crash never leaves half a file.
        let r = std::fs::write(&tmp, to_ini(s)).and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = r {
            let _ = std::fs::remove_file(&tmp);
            self.log.add(Level::Warn, format!("could not save settings.ini ({e}): the change holds until LV1 closes"));
        }
    }

    // ---- for the window

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Change the settings: saved and applied at once.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) {
        let s = {
            let mut g = self.settings.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut g);
            if g.spelling == Spelling::Natural {
                g.spelling = Spelling::Sharp;
            }
            g.clone()
        };
        self.save(&s);
        self.apply(&s);
    }

    /// MIDI from the host's own inputs: one or more whole messages (running status is
    /// expanded within one call, not across calls). `source` names it in the log and the
    /// last-change banner ("Headlink MIDI").
    pub fn feed_midi(&self, source: &str, bytes: &[u8]) {
        for m in split_midi(bytes) {
            self.engine.on_midi_at(source, &m, now_us(), None);
        }
    }

    pub fn apply_now(&self, i: Intent) {
        self.engine.apply_now(i, now_ms());
    }

    pub fn rescan(&self) {
        self.engine.request_scan();
    }

    pub fn status(&self) -> Status {
        self.engine.status()
    }

    pub fn tunes(&self) -> Vec<TuneInstance> {
        self.engine.tunes()
    }

    pub fn log(&self) -> &Arc<LogRing> {
        &self.log
    }

    /// Whether TuneSync runs its own MIDI input ([`RuntimeConfig::local_midi`]).
    pub fn has_local_midi(&self) -> bool {
        self.midi.is_some()
    }

    /// Whether TuneSync runs RTP-MIDI ([`RuntimeConfig::rtp`]).
    pub fn has_rtp(&self) -> bool {
        self.rtp.is_some()
    }

    pub fn midi_ports(&self) -> Vec<String> {
        self.midi.as_ref().map(|m| m.ports()).unwrap_or_default()
    }

    pub fn midi_open(&self) -> String {
        self.midi.as_ref().map(|m| m.open_name()).unwrap_or_default()
    }

    pub fn midi_last_ms(&self) -> u64 {
        self.midi.as_ref().map_or(0, |m| m.last_ms())
    }

    /// Empty (not enabled, not open) when RTP-MIDI is the host's.
    pub fn rtp(&self) -> RtpView {
        self.rtp.as_ref().map(|r| r.view()).unwrap_or_default()
    }

    /// Invite a found session now, and remember it.
    pub fn rtp_connect(&self, ip: &str, port: u16, label: &str) {
        self.update(|s| {
            let p = Peer { ip: ip.into(), port, name: label.into() };
            if !s.rtp_peers.iter().any(|q| q.same(&p)) && s.rtp_peers.len() < MAX_RTP_PEERS {
                s.rtp_peers.push(p);
            }
        });
        if let Some(r) = &self.rtp {
            r.connect(ip, port, label);
        }
    }

    /// Disconnect and forget.
    pub fn rtp_forget(&self, ip: &str, port: u16) {
        self.update(|s| s.rtp_peers.retain(|q| !(q.ip == ip && q.port == port)));
        if let Some(r) = &self.rtp {
            r.forget(ip, port);
        }
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    pub fn now_ms(&self) -> u64 {
        now_ms()
    }
}
