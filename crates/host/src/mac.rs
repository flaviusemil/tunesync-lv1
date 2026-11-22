//! The few CoreFoundation and libdispatch calls the macOS backends and the window share,
//! declared by hand so the crates need no bindings: CFStrings both ways, and running a
//! closure on the main queue.
//!
//! Inside LV1 the main queue is LV1's own main thread (a Qt application; its event loop
//! drains the queue in every run-loop mode, modal dialogs included). Nothing here ever
//! waits on it: [`main_async`] only posts.
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFIndex = isize;

const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithBytes(alloc: *const c_void, bytes: *const u8, n: CFIndex, encoding: u32, bom: bool) -> CFStringRef;
    fn CFStringGetLength(s: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(n: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(s: CFStringRef, buf: *mut u8, size: CFIndex, encoding: u32) -> bool;
}

extern "C" {
    static _dispatch_main_q: u8;
    fn dispatch_async_f(queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn pthread_main_np() -> i32;
}

/// A CFString we own: released when dropped.
pub struct CfString(pub CFStringRef);

impl CfString {
    pub fn new(s: &str) -> CfString {
        CfString(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as CFIndex, UTF8, false) })
    }
    pub fn as_ptr(&self) -> CFStringRef {
        self.0
    }
}

impl Drop for CfString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

/// A CFString's text ("" for NULL).
///
/// # Safety
/// `s` is NULL or a valid CFString.
pub unsafe fn to_string(s: CFStringRef) -> String {
    if s.is_null() {
        return String::new();
    }
    let max = CFStringGetMaximumSizeForEncoding(CFStringGetLength(s), UTF8) + 1;
    let mut buf = vec![0u8; max.clamp(1, 1 << 16) as usize];
    if !CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as CFIndex, UTF8) {
        return String::new();
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// Releases a CF object we were handed with "Create"/"Copy" rules.
///
/// # Safety
/// `cf` is NULL or a CF object the caller owns.
pub unsafe fn release(cf: CFTypeRef) {
    if !cf.is_null() {
        CFRelease(cf);
    }
}

pub fn is_main_thread() -> bool {
    unsafe { pthread_main_np() != 0 }
}

type Job = Box<dyn FnOnce() + Send>;

extern "C" fn run_job(ctx: *mut c_void) {
    let job = unsafe { Box::from_raw(ctx as *mut Job) };
    // A panic must never unwind into libdispatch (and so into LV1).
    let _ = catch_unwind(AssertUnwindSafe(job));
}

/// Runs `f` on the main queue, later. Returns at once, from any thread.
pub fn main_async(f: impl FnOnce() + Send + 'static) {
    let job: Box<Job> = Box::new(Box::new(f));
    unsafe { dispatch_async_f(&_dispatch_main_q as *const u8 as *const c_void, Box::into_raw(job) as *mut c_void, run_job) };
}
