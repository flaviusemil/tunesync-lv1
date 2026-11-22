//! The sessions a DNS-SD browse found, apart from the OS calls so it is tested everywhere
//! (the macOS backend drives it from dns_sd's callbacks).
//!
//! A session is seen once per network interface it answers on, so it stays while any
//! interface still has it. Its host, port and IPv4 address come from a resolve, retried
//! every 30 s while they are missing.
use std::collections::{BTreeMap, BTreeSet};

use super::FoundSession;

const RETRY_MS: u64 = 30_000;

pub(crate) fn key(name: &str, domain: &str) -> String {
    format!("{name}.{domain}").to_lowercase()
}

#[derive(Debug)]
struct Entry {
    s: FoundSession,
    domain: String,
    ifaces: BTreeSet<u32>,
    running: bool,
    next_try_ms: u64,
}

/// What to resolve: the browse's own name, domain and interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Wanted {
    pub key: String,
    pub name: String,
    pub domain: String,
    pub iface: u32,
}

#[derive(Default, Debug)]
pub(crate) struct Book {
    entries: BTreeMap<String, Entry>,
}

impl Book {
    /// A browse "add" on `iface`.
    pub fn add(&mut self, iface: u32, name: &str, domain: &str) {
        let e = self.entries.entry(key(name, domain)).or_insert_with(|| Entry {
            s: FoundSession { instance: name.to_string(), ..Default::default() },
            domain: domain.to_string(),
            ifaces: BTreeSet::new(),
            running: false,
            next_try_ms: 0,
        });
        e.ifaces.insert(iface);
    }

    /// A browse "remove" on `iface`: gone once no interface has it.
    pub fn remove(&mut self, iface: u32, name: &str, domain: &str) {
        let k = key(name, domain);
        if let Some(e) = self.entries.get_mut(&k) {
            e.ifaces.remove(&iface);
            if e.ifaces.is_empty() {
                self.entries.remove(&k);
            }
        }
    }

    /// Sessions still missing their address that are due a resolve; each is marked running.
    pub fn wanted(&mut self, now_ms: u64, max: usize) -> Vec<Wanted> {
        let mut out = Vec::new();
        for (k, e) in self.entries.iter_mut() {
            if out.len() >= max {
                break;
            }
            let complete = !e.s.ip.is_empty() && e.s.port != 0;
            if !complete && !e.running && now_ms >= e.next_try_ms {
                e.running = true;
                let iface = e.ifaces.iter().next().copied().unwrap_or(0);
                out.push(Wanted { key: k.clone(), name: e.s.instance.clone(), domain: e.domain.clone(), iface });
            }
        }
        out
    }

    /// The resolve gave the session's host and port.
    pub fn target(&mut self, k: &str, host: &str, port: u16) {
        if let Some(e) = self.entries.get_mut(k) {
            e.s.host = host.trim_end_matches('.').to_string();
            e.s.port = port;
        }
    }

    /// The host's IPv4 address.
    pub fn address(&mut self, k: &str, ip: &str) {
        if let Some(e) = self.entries.get_mut(k) {
            e.s.ip = ip.to_string();
        }
    }

    /// A resolve finished (answered, failed or timed out): tried again later if incomplete.
    pub fn ended(&mut self, k: &str, now_ms: u64) {
        if let Some(e) = self.entries.get_mut(k) {
            e.running = false;
            e.next_try_ms = now_ms + RETRY_MS;
        }
    }

    /// Everyone but ourselves. mDNS keeps instance names unique on a network, so the
    /// session named `own` (the name our registration ended up with) is ours.
    pub fn found(&self, own: &str) -> Vec<FoundSession> {
        self.entries.values().filter(|e| !e.s.instance.eq_ignore_ascii_case(own)).map(|e| e.s.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_resolve_and_ourselves() {
        let mut b = Book::default();
        b.add(4, "Studio-MacBook", "local.");
        b.add(5, "Studio-MacBook", "local."); // the same session on Wi-Fi too
        b.add(4, "TuneSync", "local.");
        let w = b.wanted(0, 4);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0], Wanted { key: "studio-macbook.local.".into(), name: "Studio-MacBook".into(), domain: "local.".into(), iface: 4 });
        assert!(b.wanted(0, 4).is_empty(), "running: not asked twice");

        let k = key("Studio-MacBook", "local.");
        b.target(&k, "Studio-MacBook.local.", 5004);
        b.address(&k, "192.0.2.10");
        b.ended(&k, 100);
        let f = b.found("TuneSync");
        assert_eq!(f, vec![FoundSession { instance: "Studio-MacBook".into(), host: "Studio-MacBook.local".into(), ip: "192.0.2.10".into(), port: 5004 }]);
        assert_eq!(b.found("tunesync (2)").len(), 2, "another host's TuneSync is listed once ours was renamed");

        // the unanswered one is retried after 30 s; the complete one never again
        b.ended(&key("TuneSync", "local."), 100);
        assert!(b.wanted(20_000, 4).is_empty());
        assert_eq!(b.wanted(30_100, 4).iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), vec!["TuneSync"]);

        // gone from one interface: still there; from both: gone
        b.remove(4, "Studio-MacBook", "local.");
        assert_eq!(b.found("TuneSync").len(), 1);
        b.remove(5, "studio-macbook", "local.");
        assert!(b.found("TuneSync").is_empty());
    }

    #[test]
    fn wanted_respects_the_limit() {
        let mut b = Book::default();
        for i in 0..6 {
            b.add(1, &format!("Mac {i}"), "local.");
        }
        assert_eq!(b.wanted(0, 4).len(), 4);
        assert_eq!(b.wanted(0, 4).len(), 2);
    }
}
