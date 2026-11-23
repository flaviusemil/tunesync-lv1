//! TuneSync_Control: the eMotion LV1 surface driver. A shim: LV1's Create starts the
//! [`tunesync_host::Runtime`] with a [`mixer::Lv1Mixer`] on LV1's callbacks, the settings
//! button opens the window, Destroy stops it all.
//!
//! LV1 loads every `Modules\*Control.dll`; `tools/build-dll.sh` writes this crate's
//! cdylib as `TuneSync_Control.dll`.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use lv1_surface::{export_surface_driver, DriverInfo, Lv1, SurfaceDriver};
use tunesync_host::filelog::early_line;
use tunesync_host::sys::data_dir;
use tunesync::log::Level;
use tunesync_host::{Runtime, RuntimeConfig};

pub mod mixer;
pub mod model;
#[cfg(windows)]
pub mod window;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The surface type LV1 stores and finds TuneSync's status row by.
pub const SURFACE_TYPE: &str = "TuneSync";

/// The picture on TuneSync's tile in LV1's Setup > System Inventory > CONTROLS, beside the
/// DLL (the installer puts it and its @2x there; `assets/lv1-surface`).
pub const PICTURE: &str = "tunesync-surface.png";

/// The tile picture's absolute path, "" when it is missing (LV1 would show an empty frame,
/// not its own picture) or not plain ASCII (how LV1 turns the bytes into text is unknown).
pub fn picture() -> String {
    own_dir()
        .map(|d| d.join(PICTURE))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| s.is_ascii() && s.len() < 1000)
        .unwrap_or_default()
}

/// The folder this DLL was loaded from.
#[cfg(windows)]
fn own_dir() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT};
    let mut m = HMODULE::default();
    let here = own_dir as fn() -> Option<std::path::PathBuf> as *const u16;
    // SAFETY: `here` is an address inside this DLL; the handle is only used to ask its path.
    unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, PCWSTR(here), &mut m) }.ok()?;
    let mut buf = [0u16; 1024];
    // SAFETY: `m` is this DLL's handle; `buf` is ours.
    let n = unsafe { GetModuleFileNameW(m, &mut buf) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    std::path::PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])).parent().map(|p| p.to_path_buf())
}

/// Elsewhere (tests; the Mac build has its bundle's Resources folder): no picture yet.
#[cfg(not(windows))]
fn own_dir() -> Option<std::path::PathBuf> {
    None
}

/// One TuneSync per LV1: a second surface would fight over the MIDI port and UDP 5004.
static RUNNING: AtomicBool = AtomicBool::new(false);

pub struct TuneSync {
    rt: Arc<Runtime>,
    lv1: Arc<Lv1>,
    #[cfg(windows)]
    window: window::Window,
}

impl SurfaceDriver for TuneSync {
    fn info() -> DriverInfo {
        DriverInfo { name: "TuneSync", category: "Surface", type_: SURFACE_TYPE }
    }

    fn create(lv1: Arc<Lv1>) -> Option<Self> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            early_line("another TuneSync surface is already running in this LV1; only the first one acts");
            return None;
        }
        std::panic::set_hook(Box::new(|p| early_line(&format!("TuneSync panicked: {p}"))));
        let l2 = lv1.clone();
        let health = move || l2.fault().map(|c| format!("LV1 refused a control call (exception 0x{c:08X}): TuneSync stays passive until LV1 restarts"));
        let pic = picture();
        let rt = Runtime::start(
            RuntimeConfig { health: Some(Box::new(health)), ..RuntimeConfig::new(data_dir(), format!("TuneSync {VERSION}")) },
            Box::new(mixer::Lv1Mixer { lv1: lv1.clone(), picture: pic.clone() }),
        );
        if pic.is_empty() {
            rt.log().add(Level::Info, format!("{PICTURE} is not beside the DLL: LV1 shows its own picture on TuneSync's tile"));
        }
        Some(TuneSync {
            #[cfg(windows)]
            window: window::Window::new(rt.clone(), {
                let l3 = lv1.clone();
                Arc::new(move || l3.fault().map(|c| format!("LV1 refused a control call (0x{c:08X}); TuneSync is passive until LV1 restarts")))
            }),
            rt,
            lv1,
        })
    }

    fn passive(why: &str) {
        early_line(&format!("{why}: TuneSync stays passive"));
    }

    fn show_config_window(&self) {
        #[cfg(windows)]
        self.window.show();
    }

    fn destroy(&self) {
        #[cfg(windows)]
        self.window.stop();
        // The worker finishes the change it is applying (bounded), then LV1 is shut out:
        // a Touch is never left without its Release.
        self.rt.stop();
        self.lv1.shut();
        RUNNING.store(false, Ordering::SeqCst);
    }

    fn preset() -> &'static [u8] {
        // LV1 writes its preferences only when it closes cleanly, so the settings live in
        // settings.ini; the preset is a version marker.
        b"TS01\0\0\0\0"
    }
}

export_surface_driver!(TuneSync);
