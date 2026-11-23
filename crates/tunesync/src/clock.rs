//! Following MIDI clock (24 ticks per quarter note) to a tempo for LV1.
//!
//! - **One rig at a time.** The first source that ticks is followed until it has been quiet
//!   for [`GAP_MS`]; another rig's clock meanwhile is ignored. A main and a backup rig both
//!   sending clock must not be counted as one clock twice as fast.
//! - **Measured on the sender's clock** where it has one (RTP-MIDI timestamps), so network
//!   jitter does not reach the tempo. A sender whose stamps disagree with arrival by more
//!   than 20 % is timed on arrival instead.
//! - **Every half beat**, the tempo over the last two beats is estimated. It is reported when
//!   two estimates in a row agree within [`SETTLE_BPM`] and it is new: the first since the
//!   clock started, or at least [`STEP_BPM`] away from the last one reported. So jitter never
//!   nudges LV1, and a tempo set another way (a CC, a hand on LV1) holds until the clock's
//!   tempo changes or the clock starts again.
//! - Whole BPM when within 0.1 of one (127.96 -> 128), else to 0.1 (92.5 stays).
use std::collections::VecDeque;

/// The MIDI realtime messages that matter here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    /// 0xF8
    Tick,
    /// 0xFA
    Start,
    /// 0xFB
    Continue,
    /// 0xFC
    Stop,
}

impl Clock {
    pub fn from_status(b: u8) -> Option<Clock> {
        match b {
            0xF8 => Some(Clock::Tick),
            0xFA => Some(Clock::Start),
            0xFB => Some(Clock::Continue),
            0xFC => Some(Clock::Stop),
            _ => None,
        }
    }
}

pub const TICKS_PER_BEAT: usize = 24;
/// The estimate looks back this far.
pub const WINDOW_TICKS: usize = 2 * TICKS_PER_BEAT;
/// An estimate every half beat.
pub const EVERY_TICKS: u64 = 12;
/// No tick from the followed source for this long: its clock stopped.
pub const GAP_MS: u64 = 1000;
/// Two estimates in a row this close: the clock has settled.
pub const SETTLE_BPM: f64 = 0.5;
/// A settled tempo this close to the last one reported is not new.
pub const STEP_BPM: f64 = 0.5;
/// LV1's own floor: its MyMon driver refuses a slower tempo.
pub const MIN_BPM: f64 = 40.0;
pub const MAX_BPM: f64 = 300.0;

#[derive(Debug, Default)]
pub struct ClockFollower {
    source: String,
    /// (sender us, arrival us) of the last ticks, oldest first; at most WINDOW_TICKS + 1.
    ticks: VecDeque<(u64, u64)>,
    /// Arrival of the followed source's last message, 0 = none yet.
    last_us: u64,
    /// Ticks since the measurement (re)started.
    count: u64,
    prev: Option<f64>,
    reported: Option<f64>,
}

impl ClockFollower {
    /// The source followed now, "" before the first.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// One clock message from `source`: `sent_us` on the sender's clock when it has one,
    /// `now_us` on ours. Returns a tempo for LV1 when the clock has settled on a new one.
    pub fn on(&mut self, source: &str, m: Clock, sent_us: Option<u64>, now_us: u64) -> Option<f64> {
        let quiet = self.last_us == 0 || now_us.saturating_sub(self.last_us) >= GAP_MS * 1000;
        if source != self.source {
            if !quiet {
                return None; // another rig's clock while ours ticks
            }
            self.source = source.to_string();
            self.restart();
        } else if quiet {
            self.restart();
        }
        self.last_us = now_us;
        match m {
            Clock::Start => {
                self.restart();
                return None;
            }
            Clock::Continue => {
                self.measure_again(); // the song position jumped: the gap is not a beat
                return None;
            }
            Clock::Stop => return None, // a rig may keep ticking while stopped: still its tempo
            Clock::Tick => {}
        }
        self.ticks.push_back((sent_us.unwrap_or(now_us), now_us));
        while self.ticks.len() > WINDOW_TICKS + 1 {
            self.ticks.pop_front();
        }
        self.count += 1;
        if !self.count.is_multiple_of(EVERY_TICKS) || self.ticks.len() <= TICKS_PER_BEAT {
            return None;
        }
        let est = self.estimate()?;
        let settled = self.prev.is_some_and(|p| (p - est).abs() < SETTLE_BPM);
        self.prev = Some(est);
        if !settled {
            return None;
        }
        let bpm = snap(est);
        if self.reported.is_some_and(|r| (r - bpm).abs() < STEP_BPM) {
            return None;
        }
        self.reported = Some(bpm);
        Some(bpm)
    }

    // A new run of the clock: measure afresh and report its tempo once settled.
    fn restart(&mut self) {
        self.measure_again();
        self.reported = None;
    }

    fn measure_again(&mut self) {
        self.ticks.clear();
        self.count = 0;
        self.prev = None;
    }

    // The tick period as the least-squares slope over the whole window, not end to end:
    // 1 ms of jitter on arrival moves it ~0.05 %, not ~0.2 %.
    fn estimate(&self) -> Option<f64> {
        let sent = slope(self.ticks.iter().map(|t| t.0));
        let arrived = slope(self.ticks.iter().map(|t| t.1));
        let period = match (sent, arrived) {
            (Some(s), Some(a)) if (0.8..1.25).contains(&(s / a)) => s,
            (_, a) => a?,
        };
        let bpm = 60e6 / (TICKS_PER_BEAT as f64 * period);
        (MIN_BPM..=MAX_BPM).contains(&bpm).then_some(bpm)
    }
}

/// Microseconds per tick: the least-squares slope of the times against their index.
fn slope(times: impl ExactSizeIterator<Item = u64> + Clone) -> Option<f64> {
    let n = times.len();
    let first = times.clone().next()?;
    if n < 2 {
        return None;
    }
    let mx = (n - 1) as f64 / 2.0;
    let my = times.clone().map(|t| t.wrapping_sub(first) as i64 as f64).sum::<f64>() / n as f64;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, t) in times.enumerate() {
        let (dx, dy) = (i as f64 - mx, t.wrapping_sub(first) as i64 as f64 - my);
        sxy += dx * dy;
        sxx += dx * dx;
    }
    let s = sxy / sxx;
    (s > 0.0).then_some(s)
}

/// Whole BPM when within 0.1 of one, else to 0.1.
pub fn snap(bpm: f64) -> f64 {
    let r = bpm.round();
    if (bpm - r).abs() <= 0.1 {
        r
    } else {
        (bpm * 10.0).round() / 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `beats` beats of clock at `bpm` from `src`, starting at `t0` us, with `jitter(i)` us
    /// added to the arrival of tick i. Returns the tempos reported and the end time.
    fn run(f: &mut ClockFollower, src: &str, bpm: f64, beats: usize, t0: u64, sent: bool, jitter: impl Fn(usize) -> i64) -> (Vec<f64>, u64) {
        let tick = 60e6 / (bpm * TICKS_PER_BEAT as f64);
        let mut out = Vec::new();
        let mut t = t0;
        for i in 0..beats * TICKS_PER_BEAT {
            t = t0 + (i as f64 * tick) as u64;
            let arrival = (t as i64 + jitter(i)) as u64;
            out.extend(f.on(src, Clock::Tick, sent.then_some(t), arrival));
        }
        (out, t)
    }

    #[test]
    fn a_steady_clock_is_reported_once() {
        let mut f = ClockFollower::default();
        let (got, _) = run(&mut f, "RTP Mac", 128.0, 16, 1_000_000, false, |_| 0);
        assert_eq!(got, vec![128.0]);
        assert_eq!(f.source(), "RTP Mac");
    }

    #[test]
    fn reported_within_two_beats() {
        let mut f = ClockFollower::default();
        let (got, _) = run(&mut f, "RTP Mac", 120.0, 2, 1_000_000, false, |_| 0);
        assert_eq!(got, vec![120.0], "first estimate after a beat and a half, settled at two");
    }

    #[test]
    fn network_jitter_is_ignored_when_the_sender_stamps() {
        let mut f = ClockFollower::default();
        // Wi-Fi: up to 40 ms late, varying wildly; the sender's own stamps are exact
        let (got, _) = run(&mut f, "RTP iPad", 92.5, 16, 1_000_000, true, |i| ((i * 7919) % 40_000) as i64);
        assert_eq!(got, vec![92.5]);
    }

    #[test]
    fn small_jitter_on_arrival_settles_and_snaps() {
        let mut f = ClockFollower::default();
        // a local port: +-1 ms on arrival, no sender stamps
        let (got, _) = run(&mut f, "MIDI A", 128.0, 16, 1_000_000, false, |i| if i % 2 == 0 { 1000 } else { -1000 });
        assert_eq!(got, vec![128.0]);
    }

    #[test]
    fn a_tempo_change_is_followed() {
        let mut f = ClockFollower::default();
        let (a, t) = run(&mut f, "RTP Mac", 120.0, 8, 1_000_000, false, |_| 0);
        let (b, _) = run(&mut f, "RTP Mac", 140.0, 8, t + 17_857, false, |_| 0);
        assert_eq!((a, b), (vec![120.0], vec![140.0]), "one report per tempo, no in-between");
    }

    #[test]
    fn a_backup_rig_is_ignored_while_the_main_ticks() {
        let mut f = ClockFollower::default();
        let tick = 60e6 / (120.0 * 24.0);
        let mut got = Vec::new();
        for i in 0..16 * 24 {
            let t = 1_000_000 + (i as f64 * tick) as u64;
            got.extend(f.on("RTP Main", Clock::Tick, None, t).map(|b| ("main", b)));
            got.extend(f.on("RTP Backup", Clock::Tick, None, t + 3000).map(|b| ("backup", b)));
        }
        assert_eq!(got, vec![("main", 120.0)], "not 240");
        // the main stops: after the gap the backup is followed
        let (b, _) = run(&mut f, "RTP Backup", 120.0, 4, 20_000_000, false, |_| 0);
        assert_eq!((b, f.source()), (vec![120.0], "RTP Backup"));
    }

    #[test]
    fn start_and_a_gap_report_again_stop_does_not() {
        let mut f = ClockFollower::default();
        let (_, t) = run(&mut f, "RTP Mac", 128.0, 4, 1_000_000, false, |_| 0);
        assert_eq!(f.on("RTP Mac", Clock::Stop, None, t + 10_000), None);
        let (again, t) = run(&mut f, "RTP Mac", 128.0, 4, t + 20_000, false, |_| 0);
        assert!(again.is_empty(), "stopped and ticking on: same tempo, not new");
        f.on("RTP Mac", Clock::Start, None, t + 10_000);
        let (started, t) = run(&mut f, "RTP Mac", 128.0, 4, t + 20_000, false, |_| 0);
        assert_eq!(started, vec![128.0], "a new run announces its tempo");
        let (after_gap, _) = run(&mut f, "RTP Mac", 128.0, 4, t + 5_000_000, false, |_| 0);
        assert_eq!(after_gap, vec![128.0], "so does a clock back after a gap");
    }

    #[test]
    fn nonsense_is_not_reported() {
        let mut f = ClockFollower::default();
        let (slow, _) = run(&mut f, "RTP Mac", 30.0, 6, 1_000_000, false, |_| 0);
        assert!(slow.is_empty(), "under LV1's 40 BPM floor");
        let mut g = ClockFollower::default();
        // a sender whose stamps don't move: timed on arrival
        let tick = 60e6 / (100.0 * 24.0);
        let got: Vec<f64> = (0..8 * 24).filter_map(|i| g.on("RTP Odd", Clock::Tick, Some(5), 1_000_000 + (i as f64 * tick) as u64)).collect();
        assert_eq!(got, vec![100.0]);
        assert_eq!((Clock::from_status(0xF8), Clock::from_status(0xFE)), (Some(Clock::Tick), None), "active sensing is not clock");
    }

    #[test]
    fn snapping() {
        assert_eq!((snap(127.96), snap(128.08), snap(92.46), snap(127.84)), (128.0, 128.0, 92.5, 127.8));
    }
}
