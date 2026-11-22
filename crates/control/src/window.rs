//! The settings window (placeholder until the Direct2D window lands).
use std::sync::Arc;

use tunesync::log::Level;
use tunesync_host::Runtime;

pub struct Window {
    rt: Arc<Runtime>,
}

impl Window {
    pub fn new(rt: Arc<Runtime>) -> Self {
        Window { rt }
    }
    pub fn show(&self) {
        self.rt.log().add(Level::Info, "settings window: not in this build yet");
    }
    pub fn stop(&self) {}
}
