//! DNS-SD without a Bonjour install:
//! - **advertise** `_apple-midi._udp` under the session name and port while RTP-MIDI is on,
//!   so a Mac lists TuneSync in Audio MIDI Setup -> MIDI Network Setup -> Directory by
//!   itself and connects with one click;
//! - **browse** for the Macs' own network sessions, for "Found on the network" in the
//!   window (TuneSync then invites the one you tap).
//!
//! Windows 10 1809+ has this built in (dnsapi DnsServiceRegister / DnsServiceBrowse); the
//! functions are looked up at run time, so an older Windows still loads the DLL and just
//! says it can't advertise (typing the host's address on the Mac still works). macOS has
//! mDNSResponder (dns_sd.h, in libSystem). Elsewhere there is none.

/// A network MIDI session someone else advertises.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoundSession {
    /// "Studio-MacBook" (the advertised session name)
    pub instance: String,
    /// "Studio-MacBook.local"
    pub host: String,
    /// "" until resolved
    pub ip: String,
    /// Its control port.
    pub port: u16,
}

pub type LogFn = std::sync::Arc<dyn Fn(&str, bool) + Send + Sync>;

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::Dnssd;

#[cfg(any(test, target_os = "macos"))]
mod book;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Dnssd;

#[cfg(not(any(windows, target_os = "macos")))]
mod none {
    use super::{FoundSession, LogFn};

    /// No DNS-SD here (a Linux development build).
    #[derive(Default)]
    pub struct Dnssd;

    impl Dnssd {
        pub fn new() -> Self {
            Dnssd
        }
        pub fn start(&mut self, _name: &str, _port: u16, _log: LogFn) {}
        pub fn stop(&mut self) {}
        pub fn found(&self) -> Vec<FoundSession> {
            Vec::new()
        }
        pub fn advert_state(&self) -> String {
            "not available here".into()
        }
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
pub use none::Dnssd;
