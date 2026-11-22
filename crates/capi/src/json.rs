//! The JSON the C API hands out (status, the Tunes list, the log), written by hand: three
//! small shapes do not need a serializer crate.
use std::fmt::Write;

use rtpmidi::session::{PeerState, PeerView};
use tunesync::apply::TuneInstance;
use tunesync::log::{Level, LogSince};
use tunesync::tune::{key_name, scale_name};
use tunesync_host::dnssd::FoundSession;
use tunesync_host::Runtime;

/// A JSON string literal.
pub fn string(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// `{"k":v,...}`, keys in the order they are added.
#[derive(Default)]
pub struct Obj(String);

impl Obj {
    pub fn new() -> Self {
        Obj::default()
    }

    /// `v` is JSON already.
    pub fn raw(mut self, k: &str, v: impl AsRef<str>) -> Self {
        self.0.push(if self.0.is_empty() { '{' } else { ',' });
        self.0 += &string(k);
        self.0.push(':');
        self.0 += v.as_ref();
        self
    }

    pub fn str(self, k: &str, v: &str) -> Self {
        self.raw(k, string(v))
    }

    pub fn int(self, k: &str, v: impl Into<u64>) -> Self {
        self.raw(k, v.into().to_string())
    }

    pub fn bool(self, k: &str, v: bool) -> Self {
        self.raw(k, if v { "true" } else { "false" })
    }

    pub fn opt<T>(self, k: &str, v: Option<T>, f: impl FnOnce(T) -> String) -> Self {
        self.raw(k, v.map_or_else(|| "null".to_string(), f))
    }

    pub fn end(self) -> String {
        if self.0.is_empty() {
            "{}".into()
        } else {
            self.0 + "}"
        }
    }
}

/// `[a,b,...]` of JSON values.
pub fn array(items: impl IntoIterator<Item = String>) -> String {
    let v: Vec<String> = items.into_iter().collect();
    format!("[{}]", v.join(","))
}

fn level(l: Level) -> &'static str {
    match l {
        Level::Info => "info",
        Level::Ok => "ok",
        Level::Warn => "warn",
    }
}

fn peer_state(s: PeerState) -> &'static str {
    match s {
        PeerState::Accepting => "accepting",
        PeerState::Inviting => "inviting",
        PeerState::InvitingData => "inviting_data",
        PeerState::Connected => "connected",
        PeerState::Waiting => "waiting",
    }
}

fn peer(p: &PeerView) -> String {
    Obj::new()
        .str("display", &p.display())
        .str("name", &p.name)
        .str("label", &p.label)
        .str("ip", &p.ip)
        .int("target", p.target)
        .int("ctl_port", p.ctl_port)
        .int("data_port", p.data_port)
        .str("state", peer_state(p.state))
        .bool("invited", p.invited)
        .int("ssrc", p.ssrc)
        .int("last_heard_ms", p.last_heard_ms)
        .int("last_midi_ms", p.last_midi_ms)
        .int("midi", p.midi)
        .int("lost", p.lost)
        .opt("rtt_ms", p.rtt_ms.filter(|r| r.is_finite()), |r| r.to_string())
        .end()
}

fn found(f: &FoundSession) -> String {
    Obj::new().str("instance", &f.instance).str("host", &f.host).str("ip", &f.ip).int("port", f.port).end()
}

/// What the LV1 window shows, as one object. `midi` / `rtp` are null when the host runs
/// those inputs itself. Times are on the runtime's clock, `now_ms`; RTP peers' on
/// `rtp.session_now_ms`.
pub fn status(rt: &Runtime) -> String {
    let st = rt.status();
    let last = (st.changes > 0).then(|| {
        Obj::new()
            .str("label", &st.last.label)
            .str("stamp", &st.last.stamp)
            .str("source", &st.last.source)
            .str("brief", &st.last.brief)
            .bool("problem", st.last.problem)
            .end()
    });
    let midi = rt.has_local_midi().then(|| {
        Obj::new().raw("ports", array(rt.midi_ports().iter().map(|p| string(p)))).str("open", &rt.midi_open()).int("last_ms", rt.midi_last_ms()).end()
    });
    let rtp = rt.has_rtp().then(|| {
        let v = rt.rtp();
        Obj::new()
            .bool("enabled", v.enabled)
            .bool("open", v.open)
            .str("error", &v.error)
            .str("name", &v.name)
            .int("port", v.port)
            .str("advert", &v.advert)
            .int("session_now_ms", v.session_now_ms)
            .int("last_midi_ms", v.last_midi_ms)
            .raw("peers", array(v.peers.iter().map(peer)))
            .raw("found", array(v.found.iter().map(found)))
            .end()
    });
    Obj::new()
        .int("now_ms", rt.now_ms())
        .int("tunes", st.tunes as u64)
        .int("locked", st.locked as u64)
        .int("changes", st.changes)
        .int("scans", st.scans)
        .int("last_midi_ms", st.last_midi_ms)
        .str("last_source", &st.last_source)
        .str("last_result", &st.last_result)
        .opt("last", last, |s| s)
        .opt("midi", midi, |s| s)
        .opt("rtp", rtp, |s| s)
        .end()
}

pub fn tune(t: &TuneInstance) -> String {
    Obj::new()
        .int("track_type", t.track_type)
        .int("track_index", t.track_index)
        .int("slot", t.slot)
        .str("label", &t.label)
        .str("name", &t.name)
        .str("where", &t.where_)
        .opt("key", t.key, |k| k.to_string())
        .opt("key_name", t.key.and_then(key_name), string)
        .opt("scale", t.scale, |s| s.to_string())
        .opt("scale_name", t.scale.and_then(scale_name), string)
        .bool("receive", t.receive)
        .bool("layout_ok", t.layout_ok)
        .str("why", &t.why)
        .end()
}

/// The Tunes as of the last scan or change.
pub fn tunes(rt: &Runtime) -> String {
    array(rt.tunes().iter().map(tune))
}

pub fn log(s: &LogSince) -> String {
    let lines = s.lines.iter().map(|l| Obj::new().str("stamp", &l.stamp).str("level", level(l.level)).str("text", &l.text).end());
    Obj::new().int("generation", s.generation).bool("reset", s.reset).raw("lines", array(lines)).end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings() {
        assert_eq!(string("a\"b\\c\nd\u{1}é"), r#""a\"b\\c\nd\u0001é""#);
        assert_eq!(Obj::new().end(), "{}");
        let o = Obj::new().str("a", "x").int("b", 3u32).bool("c", false).opt("d", None::<u8>, |v| v.to_string()).end();
        assert_eq!(o, r#"{"a":"x","b":3,"c":false,"d":null}"#);
        assert_eq!(array(Vec::new()), "[]");
    }

    #[test]
    fn big_numbers_stay_whole() {
        assert_eq!(Obj::new().int("g", u64::MAX).end(), r#"{"g":18446744073709551615}"#);
    }
}
