//! MIDI in, Tunes set: the whole behaviour without an OS or LV1.
//!
//! Threads: `on_midi` / `apply_now` / `request_scan` / `configure` come from any thread
//! (the MIDI input, the RTP-MIDI thread, the settings window) and only queue. `tick()`
//! runs on ONE worker thread and is the only caller into the [`Mixer`], so LV1 is never
//! entered from two threads and never from LV1's own UI thread.
use std::sync::{Arc, Mutex};

use crate::apply::{apply, default_ranges, discover, Mixer, TrackRange, TuneInstance};
use crate::log::{Level, LogRing};
use crate::midi_map::{interpret, Coalescer, Intent, MsgKind};
use crate::settings::Settings;
use crate::tune::Spelling;

/// The last change applied, for the window's banner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// "A Natural Minor", "C#/Db" (key only)
    pub label: String,
    /// The log's time stamp when it was applied.
    pub stamp: String,
    /// "RTP Playback-Mac", "Apply now", "RTP Main + RTP Backup"
    pub source: String,
    /// "set 2, 1 locked"
    pub brief: String,
    pub problem: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// now_ms of the last message, 0 = none yet.
    pub last_midi_ms: u64,
    pub last_source: String,
    pub last_result: String,
    pub tunes: usize,
    pub locked: usize,
    /// How many changes were applied (the window redraws on a new one).
    pub changes: u64,
    /// How many scans ran.
    pub scans: u64,
    /// Valid when changes > 0.
    pub last: Change,
}

struct State {
    settings: Settings,
    pending: Coalescer,
    sources: Vec<String>,
    scan_wanted: bool,
    ignored: usize,
    last_ignored: String,
    last_ignored_log: Option<u64>,
    status: Status,
    tunes: Vec<TuneInstance>,
}

impl State {
    // ms until an "ignored" line may be logged (0 = now).
    fn ignored_in(&self, now_ms: u64) -> u64 {
        match self.last_ignored_log {
            Some(t) if now_ms.saturating_sub(t) < Engine::IGNORED_LOG_MS => t + Engine::IGNORED_LOG_MS - now_ms,
            _ => 0,
        }
    }

    fn add_source(&mut self, source: &str) {
        if !self.pending.pending() {
            self.sources.clear();
        }
        if !self.sources.iter().any(|s| s == source) {
            self.sources.push(source.to_string());
        }
    }
}

pub struct Engine {
    log: Arc<LogRing>,
    ranges: Vec<TrackRange>,
    wake: Option<Box<dyn Fn() + Send + Sync>>,
    st: Mutex<State>,
}

impl Engine {
    /// At most one "ignored" line per 5 s.
    pub const IGNORED_LOG_MS: u64 = 5000;

    pub fn new(log: Arc<LogRing>) -> Self {
        Self::with_ranges(log, default_ranges())
    }

    pub fn with_ranges(log: Arc<LogRing>, ranges: Vec<TrackRange>) -> Self {
        Engine {
            log,
            ranges,
            wake: None,
            st: Mutex::new(State {
                settings: Settings::default(),
                pending: Coalescer::default(),
                sources: Vec::new(),
                scan_wanted: false,
                ignored: 0,
                last_ignored: String::new(),
                last_ignored_log: None,
                status: Status::default(),
                tunes: Vec::new(),
            }),
        }
    }

    /// Called whenever there is new work for tick(): the host's worker waits on it.
    pub fn set_wake(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.wake = Some(Box::new(f));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wake(&self) {
        if let Some(w) = &self.wake {
            w();
        }
    }

    pub fn configure(&self, s: &Settings) {
        let mut st = self.lock();
        st.settings = s.clone();
        if st.settings.spelling == Spelling::Natural {
            st.settings.spelling = Spelling::Sharp;
        }
    }

    pub fn settings(&self) -> Settings {
        self.lock().settings.clone()
    }

    /// One whole MIDI message from `source` ("MIDI <port>" / "RTP <peer>").
    pub fn on_midi(&self, source: &str, msg: &[u8], now_ms: u64) {
        let (line, level, due);
        {
            let mut st = self.lock();
            let m = interpret(msg, st.settings.channel, st.settings.scale_cc);
            if m.text.is_empty() {
                return; // clock, MTC, SysEx: not ours, not logged
            }
            st.status.last_midi_ms = now_ms;
            st.status.last_source = source.to_string();
            if m.kind == MsgKind::Ignored {
                // A keyboard on the same port must not flood the log: count, and log one
                // line now and then from tick().
                st.ignored += 1;
                st.last_ignored = format!("{source} {}", m.text);
                due = st.ignored_in(now_ms) == 0;
                line = None;
                level = Level::Info;
            } else {
                if m.kind != MsgKind::OutOfRange {
                    st.add_source(source);
                }
                st.pending.add(&m, now_ms);
                line = Some(format!("{source} {}", m.text));
                level = if m.kind == MsgKind::OutOfRange { Level::Warn } else { Level::Info };
                due = true;
            }
        }
        if let Some(l) = line {
            self.log.add(level, l);
        }
        if due {
            self.wake();
        }
    }

    /// The settings window's Apply now.
    pub fn apply_now(&self, i: Intent, now_ms: u64) {
        if i.is_empty() {
            return;
        }
        {
            let mut st = self.lock();
            st.add_source("Apply now");
            st.pending.add_now(&i, now_ms);
        }
        self.log.add(Level::Info, format!("Apply now: {}", i.label()));
        self.wake();
    }

    /// Refresh the Tunes list (the window opened, Rescan).
    pub fn request_scan(&self) {
        self.lock().scan_wanted = true;
        self.wake();
    }

    /// Worker thread: apply a due change, run a requested scan, log ignored messages.
    /// Returns ms until it wants to run again, `None` = only when woken.
    pub fn tick(&self, mixer: &mut dyn Mixer, now_ms: u64) -> Option<u64> {
        let mut change = Intent::default();
        let mut source = String::new();
        let mut ignored_line = None;
        let (scan, fallback) = {
            let mut st = self.lock();
            if st.pending.due_in(now_ms) == Some(0) {
                change = st.pending.take();
                source = st.sources.join(" + ");
                st.sources.clear();
            }
            if st.ignored > 0 && st.ignored_in(now_ms) == 0 {
                let ch = if st.settings.channel == 0 { "any channel".to_string() } else { format!("ch {}", st.settings.channel) };
                ignored_line = Some(format!(
                    "ignored {} message{} (not a Note On or CC {} on {ch}), last: {}",
                    st.ignored,
                    if st.ignored == 1 { "" } else { "s" },
                    st.settings.scale_cc,
                    st.last_ignored
                ));
                st.ignored = 0;
                st.last_ignored_log = Some(now_ms);
            }
            (std::mem::take(&mut st.scan_wanted), st.settings.spelling)
        };
        if let Some(l) = ignored_line.take() {
            self.log.add(Level::Info, l);
        }

        // LV1 work, outside the lock: a slow scan never blocks a MIDI callback.
        if !change.is_empty() || scan {
            let mut found = discover(mixer, &self.ranges);
            let mut result = None;
            if !change.is_empty() {
                let r = apply(mixer, &found, &change, fallback);
                let summary = r.summary();
                self.log.add(if r.problem() { Level::Warn } else { Level::Ok }, summary.clone());
                result = Some((
                    summary,
                    Change { label: r.label.clone(), stamp: self.log.stamp_now(), source: std::mem::take(&mut source), brief: r.brief(), problem: r.problem() },
                ));
                found = discover(mixer, &self.ranges); // the window shows what LV1 now holds
            }
            let locked = found.iter().filter(|t| t.layout_ok && !t.receive).count();
            let mut st = self.lock();
            st.status.tunes = found.len();
            st.status.locked = locked;
            st.tunes = found;
            st.status.scans += 1;
            if let Some((summary, last)) = result {
                st.status.last_result = summary;
                st.status.last = last;
                st.status.changes += 1;
            }
        }

        let st = self.lock();
        let next = st.pending.due_in(now_ms);
        if st.ignored == 0 {
            return next;
        }
        let ig = st.ignored_in(now_ms);
        Some(next.map_or(ig, |n| n.min(ig)))
    }

    pub fn status(&self) -> Status {
        self.lock().status.clone()
    }

    /// From the last scan or apply.
    pub fn tunes(&self) -> Vec<TuneInstance> {
        self.lock().tunes.clone()
    }
}
