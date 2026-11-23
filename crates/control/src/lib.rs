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
use tunesync_host::{Runtime, RuntimeConfig};

pub mod mixer;
pub mod model;
#[cfg(windows)]
pub mod window;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The surface type LV1 stores and finds TuneSync's status row by.
pub const SURFACE_TYPE: &str = "TuneSync";

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
        let rt = Runtime::start(
            RuntimeConfig { health: Some(Box::new(health)), ..RuntimeConfig::new(data_dir(), format!("TuneSync {VERSION}")) },
            Box::new(mixer::Lv1Mixer { lv1: lv1.clone() }),
        );
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
