//! TuneSync running inside a host: [`runtime::Runtime`] starts the engine's worker (the
//! only caller into the host's [`tunesync::apply::Mixer`]), the MIDI input by name, the
//! RTP-MIDI service with DNS-SD, and keeps the settings and log files.
//!
//! The LV1 surface driver (crates/control) is a shim over this; a C or C++ host such as
//! Headlink drives the same runtime through the C API (crates/capi).
pub mod dnssd;
pub mod filelog;
pub mod midi;
pub mod rtp;
pub mod runtime;
pub mod sys;

pub use runtime::{Runtime, RuntimeConfig};
