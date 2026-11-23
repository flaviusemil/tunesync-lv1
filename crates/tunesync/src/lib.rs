//! TuneSync's portable core: everything that decides what happens, with no OS and no
//! LV1 in it. The LV1 shell implements [`apply::Mixer`] on LV1's surface callbacks and
//! feeds MIDI into the [`engine::Engine`]; the tests use a fake mixer.
pub mod apply;
pub mod clock;
pub mod engine;
pub mod log;
pub mod midi_map;
pub mod settings;
pub mod tune;
