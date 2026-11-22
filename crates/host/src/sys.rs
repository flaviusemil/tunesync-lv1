//! Small OS things: the steady clock, local time for log stamps, the data folder, and a
//! bounded thread join for teardown.
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

static T0: OnceLock<Instant> = OnceLock::new();

/// The steady millisecond clock every part of TuneSync uses (never 0).
pub fn now_ms() -> u64 {
    T0.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// The same clock in microseconds (`now_us() / 1000` is `now_ms()`), for timing MIDI clock.
pub fn now_us() -> u64 {
    T0.get_or_init(Instant::now).elapsed().as_micros() as u64 + 1000
}

/// The same clock in 100 us units, for the RTP-MIDI session: its times in milliseconds
/// (`now_10k() / 10`) are then `now_ms()` times, so the window can animate on them.
pub fn now_10k() -> u64 {
    T0.get_or_init(Instant::now).elapsed().as_micros() as u64 / 100 + 10
}

/// Local wall-clock time: (year, month, day, hour, minute, second, millisecond).
#[cfg(windows)]
pub fn local_time() -> (u16, u16, u16, u16, u16, u16, u16) {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    (t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds)
}

#[cfg(unix)]
pub fn local_time() -> (u16, u16, u16, u16, u16, u16, u16) {
    // struct tm on Linux and macOS (both carry tm_gmtoff and tm_zone after the ints).
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: *const u8,
    }
    extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() as i64;
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    if unsafe { localtime_r(&secs, &mut tm) }.is_null() {
        return (1970, 1, 1, 0, 0, 0, 0);
    }
    (tm.year as u16 + 1900, tm.mon as u16 + 1, tm.mday as u16, tm.hour as u16, tm.min as u16, tm.sec as u16, now.subsec_millis() as u16)
}

/// "21:04:12": the stamp the window shows.
pub fn clock_text() -> String {
    let (_, _, _, h, m, s, _) = local_time();
    format!("{h:02}:{m:02}:{s:02}")
}

/// Where settings.ini and tunesync.log live: `%ProgramData%\TuneSync` on Windows (the
/// installer creates it with Users-modify, because a folder an elevated installer makes
/// is otherwise not writable by LV1's normal token), `/Library/Application Support/TuneSync`
/// on macOS (made by install.sh, group staff, writable by every local user) - or, when that
/// is not there or not writable (a bundle installed without sudo),
/// `~/Library/Application Support/TuneSync`. `TUNESYNC_DIR` overrides it (tests, the fake LV1).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("TUNESYNC_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        let pd = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        pd.join("TuneSync")
    }
    #[cfg(target_os = "macos")]
    {
        static DIR: OnceLock<PathBuf> = OnceLock::new();
        DIR.get_or_init(|| {
            extern "C" {
                fn access(path: *const std::ffi::c_char, mode: i32) -> i32;
            }
            let writable = |p: &std::path::Path| {
                use std::os::unix::ffi::OsStrExt;
                let c = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap_or_default();
                p.is_dir() && unsafe { access(c.as_ptr(), 2 /* W_OK */) } == 0
            };
            let shared = PathBuf::from("/Library/Application Support/TuneSync");
            if writable(&shared) {
                return shared;
            }
            match std::env::var_os("HOME") {
                Some(h) => PathBuf::from(h).join("Library/Application Support/TuneSync"),
                None => std::env::temp_dir().join("TuneSync"),
            }
        })
        .clone()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::temp_dir().join("TuneSync")
    }
}

/// A thread that says when it ended, so teardown can wait for it with a deadline.
pub struct Worker {
    handle: Option<JoinHandle<()>>,
    done: mpsc::Receiver<()>,
}

impl Worker {
    pub fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Worker {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                f();
                let _ = tx.send(());
            })
            .expect("spawn a thread");
        Worker { handle: Some(handle), done: rx }
    }

    /// Joins if it ends within `wait`; otherwise leaves it running detached (everything it
    /// touches is reference-counted). True when it ended.
    pub fn join_for(&mut self, wait: Duration) -> bool {
        let Some(h) = self.handle.take() else { return true };
        match self.done.recv_timeout(wait) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = h.join();
                true
            }
            Err(mpsc::RecvTimeoutError::Timeout) => false,
        }
    }
}
