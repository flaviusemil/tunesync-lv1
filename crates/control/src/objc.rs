//! Just enough of the Objective-C runtime to drive AppKit from Rust with no bindings
//! crate: classes and selectors by name, [`msg!`] (objc_msgSend cast to the exact C type
//! of each call), struct returns, and building our own NSView / NSWindow subclasses.
//!
//! ABI notes: every argument and return type given to [`msg!`] must be the method's real
//! C type (BOOL is `bool`, NSInteger `isize`, CGFloat `f64`, NSRect [`NSRect`]). A method
//! that returns an NSRect needs objc_msgSend_stret on x86_64 (it is returned through
//! memory there) and plain objc_msgSend on arm64: [`msg_rect`].
use std::ffi::{c_char, c_void, CStr};

pub type Id = *mut c_void;
pub type Sel = *const c_void;
pub type Class = *mut c_void;

pub const NIL: Id = std::ptr::null_mut();

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NSPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NSSize {
    pub w: f64,
    pub h: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}

impl NSRect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect { origin: NSPoint { x, y }, size: NSSize { w, h } }
    }
}

#[link(name = "objc")]
extern "C" {
    pub fn objc_getClass(name: *const c_char) -> Class;
    pub fn sel_registerName(name: *const c_char) -> Sel;
    pub fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
    pub fn objc_registerClassPair(cls: Class);
    pub fn class_addMethod(cls: Class, name: Sel, imp: *const c_void, types: *const c_char) -> bool;
    pub fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    pub fn objc_msgSend_stret();
    pub fn objc_autoreleasePoolPush() -> *mut c_void;
    pub fn objc_autoreleasePoolPop(pool: *mut c_void);
}

pub fn class(name: &CStr) -> Class {
    unsafe { objc_getClass(name.as_ptr()) }
}

pub fn sel(name: &CStr) -> Sel {
    unsafe { sel_registerName(name.as_ptr()) }
}

/// `msg!(Ret; receiver, "selector:with:", a => A, b => B)` sends a message. Unsafe: the
/// types must be the method's own.
macro_rules! msg {
    ($ret:ty; $obj:expr, $sel:literal $(, $arg:expr => $t:ty)* $(,)?) => {{
        let f: unsafe extern "C" fn($crate::objc::Id, $crate::objc::Sel $(, $t)*) -> $ret =
            ::std::mem::transmute($crate::objc::objc_msgSend as unsafe extern "C" fn());
        f($obj as $crate::objc::Id, $crate::objc::sel_registerName(concat!($sel, "\0").as_ptr() as *const ::std::ffi::c_char) $(, $arg)*)
    }};
}
pub(crate) use msg;

/// A message that returns an NSRect (`frame`, `bounds`, `visibleFrame`).
///
/// # Safety
/// `sel` must name a method of `obj` that takes no arguments and returns NSRect.
pub unsafe fn msg_rect(obj: Id, sel_name: &CStr) -> NSRect {
    #[cfg(target_arch = "x86_64")]
    let f: unsafe extern "C" fn(Id, Sel) -> NSRect = std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
    #[cfg(not(target_arch = "x86_64"))]
    let f: unsafe extern "C" fn(Id, Sel) -> NSRect = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(sel_name))
}

/// The C type encoding of BOOL: `signed char` on x86_64, `bool` on arm64.
#[cfg(target_arch = "x86_64")]
pub const BOOL_ENC: &str = "c";
#[cfg(not(target_arch = "x86_64"))]
pub const BOOL_ENC: &str = "B";

/// A new subclass of `superclass`, `None` if the runtime refused (the name is taken).
pub fn subclass(superclass: &CStr, name: &str) -> Option<ClassBuilder> {
    let n: Vec<u8> = name.bytes().chain([0]).collect();
    let cls = unsafe { objc_allocateClassPair(class(superclass), n.as_ptr() as *const c_char, 0) };
    (!cls.is_null()).then_some(ClassBuilder { cls })
}

pub struct ClassBuilder {
    cls: Class,
}

impl ClassBuilder {
    /// Adds `imp` (an `extern "C" fn(Id, Sel, ...)`) as `name` with the type encoding `types`.
    pub fn method(&mut self, name: &CStr, imp: *const c_void, types: &str) -> &mut Self {
        let t: Vec<u8> = types.bytes().chain([0]).collect();
        unsafe { class_addMethod(self.cls, sel(name), imp, t.as_ptr() as *const c_char) };
        self
    }

    pub fn register(self) -> Class {
        unsafe { objc_registerClassPair(self.cls) };
        self.cls
    }
}

/// An autorelease pool around `f`, for code that runs outside AppKit's own event handling
/// (jobs on the main queue).
pub fn with_pool<T>(f: impl FnOnce() -> T) -> T {
    let p = unsafe { objc_autoreleasePoolPush() };
    let r = f();
    unsafe { objc_autoreleasePoolPop(p) };
    r
}
