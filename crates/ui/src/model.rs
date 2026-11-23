//! What the window shows, as plain data. The shell fills one of these from the running
//! TuneSync a few times a second; [`crate::view::layout`] turns it into a frame.
use tunesync::log::LogLine;
use tunesync::settings::Settings;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    /// "0.1.0"
    pub version: String,
    pub settings: Settings,
    /// The host's MIDI inputs, in its order.
    pub midi_ports: Vec<String>,
    /// The input open now, "" when none.
    pub midi_open: String,
    /// MIDI arrived on the local input in the last few seconds.
    pub midi_active: bool,
    pub rtp: RtpModel,
    /// The banner; None before the first change.
    pub last: Option<LastChange>,
    pub tempo: TempoModel,
    pub tunes: Vec<TuneRow>,
    /// Oldest first, as the log ring holds them.
    pub log: Vec<LogLine>,
    /// Set when LV1 stopped answering (TuneSync is passive until LV1 restarts).
    pub lv1_problem: Option<String>,
    /// This computer's IPv4 addresses, for typing on a Mac.
    pub local_ips: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RtpModel {
    pub open: bool,
    /// Why it is not open ("UDP port 5004 is in use ...").
    pub error: String,
    /// MIDI arrived over the network in the last few seconds.
    pub active: bool,
    /// DNS-SD: "advertised", "not available on this Windows", ...
    pub advert: String,
    pub peers: Vec<PeerRow>,
    pub found: Vec<FoundRow>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeerRow {
    pub name: String,
    pub ip: String,
    /// The port to Forget it by.
    pub port: u16,
    /// "now", "3 s ago", "inviting", "waiting"
    pub status: String,
    /// The network round trip clock sync measured, "4 ms"; "" until it has.
    pub rtt: String,
    /// When MIDI last came from it, on the clock of [`crate::view::View::now_ms`]; 0 = never.
    pub midi_at_ms: u64,
    pub connected: bool,
    /// We invited it (Connect / remembered): it can be disconnected and forgotten.
    pub invited: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoundRow {
    pub name: String,
    pub ip: String,
    pub port: u16,
    /// Already a peer.
    pub joined: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TuneRow {
    /// "Ch 43"
    pub label: String,
    /// "Vox 1", "" when the track has LV1's default name
    pub name: String,
    /// "A", "E♭"
    pub key: String,
    pub scale: String,
    /// Rcv (false = Lock)
    pub receive: bool,
    /// The parameters look like Tune RT's (else it is skipped: "unexpected layout").
    pub ok: bool,
}

/// LV1's tempo, and what last set it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TempoModel {
    /// "128", "92.5"; "" when LV1's tempo can't be read.
    pub bpm: String,
    /// "21:04:12 · RTP iPad"; "" until TuneSync sets the tempo.
    pub meta: String,
    /// The last set did not land.
    pub problem: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LastChange {
    /// "A Natural Minor"
    pub label: String,
    /// "21:04:12 · RTP Playback-Mac · set 2, 1 locked"
    pub meta: String,
    pub problem: bool,
}

/// A key value's name the way a musician reads it: "E♭", "C♯".
pub fn pretty_key(name: &str) -> String {
    name.replace('#', "♯").replacen('b', "♭", if name.len() == 2 { 1 } else { 0 })
}

/// "now" under 2 s, else "n s ago" / "n min ago".
pub fn ago(ms: u64) -> String {
    match ms / 1000 {
        0 | 1 => "now".into(),
        s if s < 120 => format!("{s} s ago"),
        s => format!("{} min ago", s / 60),
    }
}

/// A network round trip: "4 ms", "<1 ms" on a fast wired network, "" when not measured.
pub fn rtt(ms: Option<f64>) -> String {
    match ms {
        Some(r) if (0.0..1.0).contains(&r) => "<1 ms".into(),
        Some(r) if r.is_finite() && r >= 1.0 => format!("{r:.0} ms"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_times() {
        assert_eq!(pretty_key("Eb"), "E♭");
        assert_eq!(pretty_key("C#"), "C♯");
        assert_eq!(pretty_key("B"), "B");
        assert_eq!(pretty_key("Bb"), "B♭");
        assert_eq!(ago(500), "now");
        assert_eq!(ago(3200), "3 s ago");
        assert_eq!(ago(300_000), "5 min ago");
        assert_eq!(rtt(Some(4.4)), "4 ms");
        assert_eq!(rtt(Some(0.3)), "<1 ms");
        assert_eq!(rtt(None), "");
        assert_eq!(rtt(Some(f64::NAN)), "");
    }
}
