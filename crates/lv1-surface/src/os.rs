//! The few OS calls the driver layer needs, declared here so the crate has no
//! dependencies: the guarded call, a safe read of LV1's memory, and MixerCoreSG's exports.
use std::ffi::c_void;

use crate::abi::HostFn;

#[cfg(windows)]
mod imp {
    use super::*;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn GetCurrentProcess() -> *mut c_void;
        fn ReadProcessMemory(process: *mut c_void, base: *const c_void, buf: *mut c_void, size: usize, read: *mut usize) -> i32;
    }
    extern "C" {
        // csrc/guard.c
        fn lv1s_guarded_call(f: HostFn, core: *mut c_void, arg: *mut c_void, rc: *mut i32) -> i32;
    }

    pub unsafe fn guarded_call(f: HostFn, core: *mut c_void, arg: *mut c_void) -> Result<i32, u32> {
        let mut rc = 0i32;
        if lv1s_guarded_call(f, core, arg, &mut rc) != 0 {
            Ok(rc)
        } else {
            Err(rc as u32)
        }
    }

    pub unsafe fn read_memory(dst: *mut u8, src: *const u8, n: usize) -> bool {
        let mut got = 0usize;
        ReadProcessMemory(GetCurrentProcess(), src as *const c_void, dst as *mut c_void, n, &mut got) != 0 && got == n
    }

    pub fn mixer_core_export(name: &str) -> Option<*const c_void> {
        let dll: Vec<u16> = "MixerCoreSG.dll\0".encode_utf16().collect();
        let cname: Vec<u8> = name.bytes().chain([0]).collect();
        unsafe {
            let m = GetModuleHandleW(dll.as_ptr());
            if m.is_null() {
                return None;
            }
            let p = GetProcAddress(m, cname.as_ptr());
            (!p.is_null()).then_some(p as *const c_void)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const u8) -> *mut c_void;
    }
    #[cfg(target_os = "macos")]
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    #[cfg(not(target_os = "macos"))]
    const RTLD_DEFAULT: *mut c_void = std::ptr::null_mut();

    /// No SEH here: a fault in LV1's callback is a signal. The call is made directly.
    pub unsafe fn guarded_call(f: HostFn, core: *mut c_void, arg: *mut c_void) -> Result<i32, u32> {
        Ok(f(core, arg))
    }

    pub unsafe fn read_memory(dst: *mut u8, src: *const u8, n: usize) -> bool {
        std::ptr::copy_nonoverlapping(src, dst, n);
        true
    }

    /// The process's MixerCoreSG framework exports the wrappers by name (on macOS too).
    pub fn mixer_core_export(name: &str) -> Option<*const c_void> {
        let cname: Vec<u8> = name.bytes().chain([0]).collect();
        let p = unsafe { dlsym(RTLD_DEFAULT, cname.as_ptr()) };
        (!p.is_null()).then_some(p as *const c_void)
    }
}

pub use imp::*;
