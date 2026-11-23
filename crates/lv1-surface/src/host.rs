//! The way back into LV1: the `core` pointer and the callback table it handed the driver,
//! with every call behind the crash guard.
//!
//! Rules (each learned on the rig, see Headlink's LV1-SURFACE-DRIVER.md and
//! SPIKE-FADER-SURFACE.md):
//! - Call LV1 from the driver's OWN threads, never from inside a broadcast LV1 is making.
//! - SetMixerControl is asynchronous: it returns at once, and a read straight after still
//!   shows the old value. Wait for the value to show before judging.
//! - Trust no pointer LV1 hands over: [`safe_copy`] reads through ReadProcessMemory, which
//!   fails cleanly on an unmapped address (a `__try` around memcpy does not: clang-cl drops
//!   the SEH scope around it, which crashed LV1 in the fader spike).
//! - After the first fault, every later call is refused: the driver stays
//!   passive rather than poke a core that just faulted.
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::abi::{CoreCallbacks, HostFn, MixerControl, MixerParamDouble, MixerParamSurfaceStatus, CALLBACK_EXPORTS};
use crate::os;

/// A call into LV1 faulted (the exception code on Windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault(pub u32);

pub struct Lv1 {
    core: *mut c_void,
    cb: CoreCallbacks,
    faulted: AtomicBool,
    fault_code: AtomicU32,
    shut: AtomicBool,
    calls: AtomicU64,
}

// The pointers belong to LV1 and stay valid while the surface exists; LV1's callbacks are
// made to be called from a driver's own threads (the probes and Mackie_Control do).
unsafe impl Send for Lv1 {}
unsafe impl Sync for Lv1 {}

impl Lv1 {
    /// # Safety
    /// `core` and `cb` must be what LV1 passed to `WMSD_CreateControllerForType`.
    pub unsafe fn new(core: *mut c_void, cb: CoreCallbacks) -> Lv1 {
        Lv1 { core, cb, faulted: AtomicBool::new(false), fault_code: AtomicU32::new(0), shut: AtomicBool::new(false), calls: AtomicU64::new(0) }
    }

    fn call(&self, f: Option<HostFn>, arg: *mut c_void) -> Result<i32, Fault> {
        if self.faulted.load(Ordering::SeqCst) {
            return Err(Fault(self.fault_code.load(Ordering::SeqCst)));
        }
        if self.shut.load(Ordering::SeqCst) || self.core.is_null() {
            return Err(Fault(0));
        }
        let Some(f) = f else { return Err(Fault(0)) };
        self.calls.fetch_add(1, Ordering::Relaxed);
        match unsafe { os::guarded_call(f, self.core, arg) } {
            Ok(rc) => Ok(rc),
            Err(code) => {
                self.fault_code.store(code, Ordering::SeqCst);
                self.faulted.store(true, Ordering::SeqCst);
                Err(Fault(code))
            }
        }
    }

    /// GetMixerControl: fills `c` in place. Ok(LV1's return code; 0 = answered).
    pub fn get_control(&self, c: &mut MixerControl) -> Result<i32, Fault> {
        self.call(self.cb.get_mixer_control, c as *mut MixerControl as *mut c_void)
    }

    /// SetMixerControl. Asynchronous on LV1's side: read back to see it land.
    pub fn set_control(&self, c: &MixerControl) -> Result<i32, Fault> {
        let mut copy = *c; // LV1 takes a const pointer; never hand it our caller's memory
        self.call(self.cb.set_mixer_control, &mut copy as *mut MixerControl as *mut c_void)
    }

    /// GetMixerParam of a double-valued param ([`crate::abi::param`]). LV1 writes the whole
    /// struct for the type with no size check, so it gets room to spare.
    pub fn get_param_double(&self, type_: u32) -> Result<Option<f64>, Fault> {
        #[repr(C, packed)]
        struct Roomy {
            p: MixerParamDouble,
            spare: [u8; 0xC3],
        }
        let mut b = Roomy { p: MixerParamDouble::new(type_, f64::NAN), spare: [0; 0xC3] };
        let rc = self.call(self.cb.get_mixer_param, &mut b as *mut Roomy as *mut c_void)?;
        let v = b.p.value;
        Ok((rc == 0 && v.is_finite()).then_some(v))
    }

    /// SetMixerParam of a double-valued param. Ok(LV1's return code; 0 = taken).
    pub fn set_param_double(&self, type_: u32, value: f64) -> Result<i32, Fault> {
        let mut p = MixerParamDouble::new(type_, value);
        self.call(self.cb.set_mixer_param, &mut p as *mut MixerParamDouble as *mut c_void)
    }

    /// SetMixerParam SurfaceStatus: the status Setup > Control Surfaces shows beside this
    /// surface (Mackie: "ON" in 0xFF00, "N/A" in 0xFF0000) and the picture on its tile
    /// (`picture`: an absolute path to a file that exists, or "" for LV1's own). Send the
    /// picture every time: an empty one brings LV1's back. `surface_type` is the type
    /// `WMSD_GetAvailableControllerInfo` gave. Ok(0) = LV1 found the surface.
    pub fn set_surface_status(&self, surface_type: &str, status: &str, color: u32, picture: &str) -> Result<i32, Fault> {
        let mut p = MixerParamSurfaceStatus::new(surface_type, status, color, picture);
        self.call(self.cb.set_mixer_param, &mut *p as *mut MixerParamSurfaceStatus as *mut c_void)
    }

    /// The first fault's code, if a call ever faulted.
    pub fn fault(&self) -> Option<u32> {
        self.faulted.load(Ordering::SeqCst).then(|| self.fault_code.load(Ordering::SeqCst))
    }

    /// Refuse every later call (the surface is being destroyed).
    pub fn shut(&self) {
        self.shut.store(true, Ordering::SeqCst);
    }

    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

/// Is `cb` MixerCoreSG's own table? Each entry must be the address MixerCoreSG exports
/// under its wrapper name, as the probes verified on LV1 16.5 and 17.3. A mismatch means
/// an LV1 whose interface changed: the driver must stay passive.
pub fn verify_callbacks(cb: &CoreCallbacks) -> Result<(), String> {
    let table: [Option<HostFn>; 7] = [
        cb.set_mixer_control,
        cb.get_mixer_control,
        cb.set_mixer_assignments,
        cb.get_mixer_assignments,
        cb.set_mixer_param,
        cb.get_mixer_param,
        cb.send_mixer_command,
    ];
    for (f, name) in table.iter().zip(CALLBACK_EXPORTS) {
        let want = os::mixer_core_export(name).ok_or_else(|| format!("MixerCoreSG does not export {name} (is this LV1?)"))?;
        if f.map(|f| f as usize) != Some(want as usize) {
            return Err(format!("callback {name} is not MixerCoreSG's export"));
        }
    }
    Ok(())
}

/// Copies `n` bytes from a pointer LV1 handed us without trusting it.
///
/// # Safety
/// `dst` must be valid for `n` bytes.
pub unsafe fn safe_copy(dst: *mut u8, src: *const u8, n: usize) -> bool {
    !src.is_null() && os::read_memory(dst, src, n)
}
