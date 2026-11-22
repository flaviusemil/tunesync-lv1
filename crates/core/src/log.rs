//! The log the settings window shows (newest last here; the window reverses it) and the
//! host mirrors to a file. Thread-safe.
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Ok,
    Warn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// From the host's stamper, e.g. "21:04:12".
    pub stamp: String,
    pub level: Level,
    pub text: String,
}

type Stamper = Box<dyn Fn() -> String + Send + Sync>;
type Sink = Box<dyn Fn(&LogLine) + Send + Sync>;

struct Inner {
    lines: VecDeque<LogLine>,
    generation: u64,
}

pub struct LogRing {
    cap: usize,
    inner: Mutex<Inner>,
    stamp: Option<Stamper>,
    sink: Option<Sink>,
}

impl Default for LogRing {
    fn default() -> Self {
        Self::new(500)
    }
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        LogRing { cap: capacity.max(1), inner: Mutex::new(Inner { lines: VecDeque::new(), generation: 0 }), stamp: None, sink: None }
    }

    /// The current local time as text, for each line. Set before sharing the ring.
    pub fn with_stamper(mut self, f: impl Fn() -> String + Send + Sync + 'static) -> Self {
        self.stamp = Some(Box::new(f));
        self
    }

    /// Called for every line, outside the lock (the file log). Set before sharing the ring.
    pub fn with_sink(mut self, f: impl Fn(&LogLine) + Send + Sync + 'static) -> Self {
        self.sink = Some(Box::new(f));
        self
    }

    pub fn stamp_now(&self) -> String {
        self.stamp.as_ref().map(|f| f()).unwrap_or_default()
    }

    pub fn add(&self, level: Level, text: impl Into<String>) {
        let line = LogLine { stamp: self.stamp_now(), level, text: text.into() };
        {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.lines.push_back(line.clone());
            while g.lines.len() > self.cap {
                g.lines.pop_front();
            }
            g.generation += 1;
        }
        if let Some(s) = &self.sink {
            s(&line);
        }
    }

    pub fn snapshot(&self) -> Vec<LogLine> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).lines.iter().cloned().collect()
    }

    /// Bumps on every add or clear: the window redraws when it moves.
    pub fn generation(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).generation
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.lines.clear();
        g.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn ring() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let log = LogRing::new(3).with_stamper(|| "12:00:00".into()).with_sink(move |l| s2.lock().unwrap().push(l.text.clone()));
        for i in 0..5 {
            log.add(Level::Info, format!("line {i}"));
        }
        let snap = log.snapshot();
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[0].text, "line 2");
        assert_eq!(snap[2].stamp, "12:00:00");
        assert_eq!(seen.lock().unwrap().len(), 5, "the sink sees every line");
        let g = log.generation();
        log.clear();
        assert!(log.snapshot().is_empty() && log.generation() == g + 1);
    }
}
