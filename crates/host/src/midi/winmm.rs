//! WinMM MIDI input. The driver calls `proc` on its own thread, under rules that allow
//! almost nothing: it only hands the bytes to the worker (and returned SysEx buffers are
//! added back by the worker, not from the callback).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Media::Audio::{
    midiInAddBuffer, midiInClose, midiInGetDevCapsW, midiInGetErrorTextW, midiInGetNumDevs, midiInOpen, midiInPrepareHeader, midiInReset, midiInStart,
    midiInStop, midiInUnprepareHeader, CALLBACK_FUNCTION, HMIDIIN, MIDIHDR, MIDIINCAPSW,
};
use windows_sys::Win32::Media::{MMSYSERR_ALLOCATED, MMSYSERR_NOERROR, MM_MIM_DATA, MM_MIM_LONGDATA};

use super::{data_len, Shared};

const BUFS: usize = 4;
const BUF_LEN: usize = 1024;

fn wide_to_string(w: &[u16]) -> String {
    let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..n])
}

pub fn list() -> Vec<String> {
    let n = unsafe { midiInGetNumDevs() }.min(64);
    (0..n)
        .map(|i| {
            let mut c: MIDIINCAPSW = unsafe { std::mem::zeroed() };
            if unsafe { midiInGetDevCapsW(i as usize, &mut c, std::mem::size_of::<MIDIINCAPSW>() as u32) } == MMSYSERR_NOERROR {
                let name = c.szPname; // packed: copy out before borrowing
                wide_to_string(&name)
            } else {
                "?".into()
            }
        })
        .collect()
}

fn mm_error(r: u32) -> String {
    let mut t = [0u16; 256];
    unsafe { midiInGetErrorTextW(r, t.as_mut_ptr(), t.len() as u32) };
    format!("{r} {}", wide_to_string(&t))
}

struct Ctx {
    sh: Arc<Shared>,
    closing: AtomicBool,
    done: Mutex<Vec<usize>>, // MIDIHDR pointers the driver returned, to add back
}

unsafe extern "system" fn proc(_h: HMIDIIN, msg: u32, inst: usize, p1: usize, _p2: usize) {
    let ctx = &*(inst as *const Ctx);
    if msg == MM_MIM_DATA {
        let b = [p1 as u8, (p1 >> 8) as u8, (p1 >> 16) as u8];
        if b[0] & 0x80 != 0 {
            ctx.sh.incoming(&b[..1 + data_len(b[0])]);
        }
    } else if msg == MM_MIM_LONGDATA {
        let h = p1 as *mut MIDIHDR;
        let n = (*h).dwBytesRecorded as usize;
        if n > 0 && n <= BUF_LEN {
            ctx.sh.incoming(std::slice::from_raw_parts((*h).lpData as *const u8, n));
        }
        if !ctx.closing.load(Ordering::SeqCst) {
            if let Ok(mut d) = ctx.done.try_lock() {
                d.push(p1);
            }
        }
    }
}

pub struct Port {
    h: HMIDIIN,
    hdrs: Box<[MIDIHDR; BUFS]>,
    _bufs: Box<[[u8; BUF_LEN]; BUFS]>,
    ctx: *const Ctx,
}

// The handle and buffers are used only by the MIDI worker thread.
unsafe impl Send for Port {}

impl Port {
    pub fn open(name: &str, sh: Arc<Shared>) -> Result<Port, String> {
        let Some(idx) = list().iter().position(|n| n == name) else { return Err("not connected".into()) };
        let ctx = Arc::into_raw(Arc::new(Ctx { sh, closing: AtomicBool::new(false), done: Mutex::new(Vec::new()) }));
        let mut h: HMIDIIN = std::ptr::null_mut();
        let r = unsafe { midiInOpen(&mut h, idx as u32, proc as *const () as usize, ctx as usize, CALLBACK_FUNCTION) };
        if r != MMSYSERR_NOERROR {
            unsafe { drop(Arc::from_raw(ctx)) };
            return Err(mm_error(r) + if r == MMSYSERR_ALLOCATED { " - another program has it open" } else { "" });
        }
        let mut bufs: Box<[[u8; BUF_LEN]; BUFS]> = Box::new([[0; BUF_LEN]; BUFS]);
        let mut hdrs: Box<[MIDIHDR; BUFS]> = Box::new(unsafe { std::mem::zeroed() });
        for k in 0..BUFS {
            hdrs[k].lpData = bufs[k].as_mut_ptr();
            hdrs[k].dwBufferLength = BUF_LEN as u32;
            unsafe {
                midiInPrepareHeader(h, &mut hdrs[k], std::mem::size_of::<MIDIHDR>() as u32);
                midiInAddBuffer(h, &mut hdrs[k], std::mem::size_of::<MIDIHDR>() as u32);
            }
        }
        let mut p = Port { h, hdrs, _bufs: bufs, ctx };
        let r = unsafe { midiInStart(h) };
        if r != MMSYSERR_NOERROR {
            let e = mm_error(r);
            p.close();
            return Err(e);
        }
        Ok(p)
    }

    /// Adds back the SysEx buffers the driver returned (the worker, never the callback).
    pub fn service(&mut self) {
        if self.h.is_null() {
            return;
        }
        let ctx = unsafe { &*self.ctx };
        let done: Vec<usize> = std::mem::take(&mut *ctx.done.lock().unwrap_or_else(|e| e.into_inner()));
        for p in done {
            if let Some(k) = self.hdrs.iter().position(|h| h as *const MIDIHDR as usize == p) {
                self.hdrs[k].dwBytesRecorded = 0;
                unsafe { midiInAddBuffer(self.h, &mut self.hdrs[k], std::mem::size_of::<MIDIHDR>() as u32) };
            }
        }
    }

    fn close(&mut self) {
        if self.h.is_null() {
            return;
        }
        let ctx = unsafe { &*self.ctx };
        ctx.closing.store(true, Ordering::SeqCst);
        unsafe {
            midiInStop(self.h);
            midiInReset(self.h);
            for k in 0..BUFS {
                midiInUnprepareHeader(self.h, &mut self.hdrs[k], std::mem::size_of::<MIDIHDR>() as u32);
            }
            midiInClose(self.h);
            // No callback comes after midiInClose returns.
            drop(Arc::from_raw(self.ctx));
        }
        self.h = std::ptr::null_mut();
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        self.close();
    }
}
