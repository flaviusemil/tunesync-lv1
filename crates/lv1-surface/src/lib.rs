//! eMotion LV1's control-surface driver interface ("WMSD", interface version 6) in Rust.
//!
//! LV1 loads every `Modules\*Control.dll` (Windows) / `Modules/*Control.bundle` (macOS)
//! and resolves 18 C exports by name. A surface is created for each entry in LV1's
//! preferences (`<SurfaceDrivers>`), or when it is added in Setup -> Control Surfaces;
//! Create hands the driver a `core` pointer and MixerCoreSG's callback table, the way
//! back into LV1's mixer.
//!
//! - [`abi`]: the packed structs and constants, size-checked at compile time.
//! - [`host`]: [`host::Lv1`], guarded calls into LV1, and the callback-table check.
//! - [`driver`]: the [`driver::SurfaceDriver`] trait and [`export_surface_driver!`].
//!
//! Reverse-engineered from LV1 17.3.50 and proven on LV1 16.5 / 17.3 (Headlink's surface
//! and fader spikes). Not affiliated with or endorsed by Waves.
pub mod abi;
pub mod driver;
pub mod host;
mod os;

pub use driver::{DriverInfo, SurfaceDriver};
pub use host::{Fault, Lv1};
