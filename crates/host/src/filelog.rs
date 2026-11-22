//! The log file beside the settings (`tunesync.log`, 1 MB, one rotation). Lines are
//! queued from any thread and written by the worker, never by a MIDI or LV1 thread.
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use tunesync::log::{Level, LogLine};

use crate::sys::local_time;

pub const MAX_BYTES: u64 = 1 << 20;

pub struct FileLog {
    path: PathBuf,
    q: Mutex<VecDeque<String>>,
}

impl FileLog {
    pub fn new(path: PathBuf) -> Self {
        FileLog { path, q: Mutex::new(VecDeque::new()) }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Queue a line. `detail` = packet-level noise, marked as such.
    pub fn add(&self, l: &LogLine, detail: bool) {
        let (y, mo, d, h, mi, s, ms) = local_time();
        let tag = if detail {
            "  .  "
        } else {
            match l.level {
                Level::Ok => "[ok] ",
                Level::Warn => "[!!] ",
                Level::Info => "     ",
            }
        };
        let line = format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{ms:03} {tag}{}\r\n", l.text);
        let mut q = self.q.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() < 5000 {
            q.push_back(line);
        }
    }

    /// Write what is queued (the worker, a few times a second).
    pub fn flush(&self) {
        let lines: Vec<String> = self.q.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect();
        if lines.is_empty() {
            return;
        }
        if std::fs::metadata(&self.path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1")); // one rotation
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = f.write_all(lines.concat().as_bytes());
        }
    }
}

/// A line straight to the file, for a surface that stays passive (no runtime runs).
pub fn early_line(text: &str) {
    let dir = crate::sys::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let f = FileLog::new(dir.join("tunesync.log"));
    f.add(&LogLine { stamp: String::new(), level: Level::Warn, text: text.to_string() }, false);
    f.flush();
}
