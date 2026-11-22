//! A surface driver in Rust: implement [`SurfaceDriver`], then
//! `lv1_surface::export_surface_driver!(MyDriver);` in the cdylib generates the 18 C
//! exports LV1 resolves by name in every `Modules\*Control.dll` (`Modules/*Control.bundle`
//! on macOS).
//!
//! Every export returns at once and never lets a panic cross into LV1. LV1 calls them on
//! its own threads, the UI thread among them, so a driver must not wait on anything in
//! them: start its own threads in `create`, post to them from `show_config_window`. The
//! broadcasts LV1 sends every surface (controls, params, assignments, messages) are
//! ignored without being read; a driver that wants them can grow hooks here later.
use std::ffi::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::abi::{put_cstr, ControllerInfo, CoreCallbacks};
use crate::host::{safe_copy, verify_callbacks, Lv1};

/// What Setup -> Control Surfaces lists.
pub struct DriverInfo {
    pub name: &'static str,
    /// Mackie and LV1's MIDI driver say "Surface".
    pub category: &'static str,
    /// The key LV1 stores as the surface's `<ControllerType>`.
    pub type_: &'static str,
}

pub trait SurfaceDriver: Send + Sync + Sized + 'static {
    fn info() -> DriverInfo;
    /// LV1 created a surface and its callback table is MixerCoreSG's own. `None` = this
    /// surface stays passive (say why yourself).
    fn create(lv1: Arc<Lv1>) -> Option<Self>;
    /// The surface stays passive: no callbacks, or a table that is not MixerCoreSG's.
    fn passive(why: &str);
    /// Setup -> Control Surfaces -> the settings button. Post and return.
    fn show_config_window(&self) {}
    /// LV1 destroys the surface (it closes, or the surface was removed). Stop the
    /// driver's threads, bounded. Nothing is freed afterwards: LV1 may still call late.
    fn destroy(&self) {}
    /// What LV1 stores as this surface's preset in its preferences.
    fn preset() -> &'static [u8] {
        b"WMSD\0\0\0\0"
    }
}

/// What LV1 holds as the controller pointer.
pub struct Controller<D> {
    driver: Option<D>,
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// # Safety
/// `info` from LV1 (0x602 bytes).
pub unsafe fn get_available_controller_info<D: SurfaceDriver>(info: *mut ControllerInfo) -> i32 {
    guard(-1, || {
        let Some(info) = info.as_mut() else { return -1 };
        let i = D::info();
        put_cstr(&mut info.name, i.name);
        put_cstr(&mut info.category, i.category);
        put_cstr(&mut info.type_, i.type_);
        info.flag200 = 1;
        0
    })
}

/// # Safety
/// The arguments of `WMSD_CreateControllerForType`.
pub unsafe fn create_controller_for_type<D: SurfaceDriver>(core: *mut c_void, cb: *const CoreCallbacks) -> *mut c_void {
    guard(std::ptr::null_mut(), || {
        let mut c = Controller::<D> { driver: None };
        let mut table = std::mem::MaybeUninit::<CoreCallbacks>::zeroed();
        if core.is_null() || !safe_copy(table.as_mut_ptr() as *mut u8, cb as *const u8, std::mem::size_of::<CoreCallbacks>()) {
            D::passive("LV1 passed no surface callbacks");
        } else {
            let table = table.assume_init();
            match verify_callbacks(&table) {
                Err(why) => D::passive(&format!("LV1's surface interface is not the one this driver knows ({why})")),
                Ok(()) => c.driver = D::create(Arc::new(Lv1::new(core, table))),
            }
        }
        Box::into_raw(Box::new(c)) as *mut c_void
    })
}

/// # Safety
/// `ctrl` from create_controller_for_type.
pub unsafe fn destroy_controller<D: SurfaceDriver>(ctrl: *mut c_void) -> i32 {
    guard(0, || {
        if let Some(d) = (ctrl as *const Controller<D>).as_ref().and_then(|c| c.driver.as_ref()) {
            d.destroy();
        }
        0 // the controller stays allocated on purpose: LV1 may still deliver a late callback
    })
}

/// # Safety
/// `ctrl` from create_controller_for_type.
pub unsafe fn show_config_window<D: SurfaceDriver>(ctrl: *mut c_void) -> i32 {
    guard(0, || {
        if let Some(d) = (ctrl as *const Controller<D>).as_ref().and_then(|c| c.driver.as_ref()) {
            d.show_config_window();
        }
        0
    })
}

/// # Safety
/// The arguments of `WMSD_GetPreset`: `buf == NULL` is a size query.
pub unsafe fn get_preset<D: SurfaceDriver>(buf: *mut u8, size: u32, io_size: *mut u32, ver: *mut c_char, ver_len: u32) -> i32 {
    guard(0, || {
        let p = D::preset();
        if let Some(s) = io_size.as_mut() {
            *s = p.len() as u32;
        }
        if !buf.is_null() && size as usize >= p.len() {
            std::ptr::copy_nonoverlapping(p.as_ptr(), buf, p.len());
        }
        if !ver.is_null() && ver_len > 4 {
            let v = std::slice::from_raw_parts_mut(ver as *mut u8, ver_len as usize);
            put_cstr(v, "1.0");
        }
        0
    })
}

/// Generates the 18 `WMSD_*` exports for a [`SurfaceDriver`].
#[macro_export]
macro_rules! export_surface_driver {
    ($d:ty) => {
        const _: () = {
            use ::std::ffi::{c_char, c_void};
            use $crate::abi::{ControllerInfo, CoreCallbacks};

            #[no_mangle]
            pub extern "C" fn WMSD_QueryInterfaceVersion() -> i32 {
                $crate::abi::INTERFACE_VERSION
            }
            #[no_mangle]
            pub unsafe extern "C" fn WMSD_GetAvailableControllerInfo(info: *mut ControllerInfo) -> i32 {
                $crate::driver::get_available_controller_info::<$d>(info)
            }
            #[no_mangle]
            pub extern "C" fn WMSD_TestControllerForType(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> i32 {
                0
            }
            #[no_mangle]
            pub unsafe extern "C" fn WMSD_CreateControllerForType(
                _app: *const c_char,
                core: *mut c_void,
                cb: *const CoreCallbacks,
                _engine: *mut c_void,
                _config: *const c_void,
                _out: *mut bool,
            ) -> *mut c_void {
                $crate::driver::create_controller_for_type::<$d>(core, cb)
            }
            #[no_mangle]
            pub extern "C" fn WMSD_InitControllerForType(_: *mut c_void) -> i32 {
                0
            }
            #[no_mangle]
            pub extern "C" fn WMSD_GetTypeForController(_: *mut c_void, _: *mut c_char) -> i32 {
                0
            }
            #[no_mangle]
            pub unsafe extern "C" fn WMSD_DestroyController(ctrl: *mut c_void, _: bool) -> i32 {
                $crate::driver::destroy_controller::<$d>(ctrl)
            }
            #[no_mangle]
            pub unsafe extern "C" fn WMSD_ShowConfigWindow(ctrl: *mut c_void) -> i32 {
                $crate::driver::show_config_window::<$d>(ctrl)
            }
            #[no_mangle]
            pub extern "C" fn WMSD_LockConfigWindow(_: *mut c_void) -> i32 {
                0
            }
            #[no_mangle]
            pub extern "C" fn WMSD_UnlockConfigWindow(_: *mut c_void) -> i32 {
                0
            }
            #[no_mangle]
            pub extern "C" fn WMSD_IdentifyController(_: *mut c_void, _: bool) -> i32 {
                0
            }
            #[no_mangle]
            pub unsafe extern "C" fn WMSD_GetPreset(_ctrl: *mut c_void, buf: *mut u8, size: u32, io_size: *mut u32, ver: *mut c_char, ver_len: u32) -> i32 {
                $crate::driver::get_preset::<$d>(buf, size, io_size, ver, ver_len)
            }
            #[no_mangle]
            pub extern "C" fn WMSD_SetPreset(_: *mut c_void, _: *const c_void, _: u32, _: *const c_char) -> i32 {
                0
            }
            #[no_mangle]
            pub extern "C" fn WMSD_CommandStatusUpdate(_: *mut c_void, _: *mut c_void) -> i32 {
                0
            }
            /// Meters arrive here ~23/s: not read.
            #[no_mangle]
            pub extern "C" fn WMSD_MixerParamWasChanged(_: *mut c_void, _: *const c_void) -> i32 {
                0
            }
            #[no_mangle]
            pub extern "C" fn WMSD_MixerAssignmentsChanged(_: *mut c_void, _: *const c_void) -> i32 {
                0
            }
            /// Every control LV1 broadcasts: not read.
            #[no_mangle]
            pub extern "C" fn WMSD_ControllerDisplayUpdate(_: *mut c_void, _: *const c_void) -> i32 {
                0
            }
            #[no_mangle]
            pub extern "C" fn WMSD_MixerMessageSent(_: *mut c_void, _: *const c_void) -> i32 {
                0
            }
        };
    };
}
