//! No MIDI ports (a Linux development build). macOS gets CoreMIDI here next.
use std::sync::Arc;

use super::Shared;

pub fn list() -> Vec<String> {
    Vec::new()
}

pub struct Port;

impl Port {
    pub fn open(_name: &str, _sh: Arc<Shared>) -> Result<Port, String> {
        Err("no MIDI on this platform".into())
    }
    pub fn service(&mut self) {}
}
