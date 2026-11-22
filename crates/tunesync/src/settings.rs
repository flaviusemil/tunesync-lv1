//! What the settings window edits, stored as a small ini file
//! (`%ProgramData%\TuneSync\settings.ini` on Windows).
use crate::tune::Spelling;

/// A session TuneSync invited itself (Connect): invited again when it comes back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub ip: String,
    pub port: u16,
    /// As it was found, for the window.
    pub name: String,
}

impl Peer {
    pub fn same(&self, o: &Peer) -> bool {
        self.ip == o.ip && self.port == o.port
    }
}

pub const MAX_RTP_PEERS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Input port name, "" = none.
    pub midi_port: String,
    pub rtp_enabled: bool,
    /// The session name the Mac sees.
    pub rtp_name: String,
    /// Control port; data = +1.
    pub rtp_port: u16,
    /// 1..16, 0 = Omni.
    pub channel: u8,
    /// 0..119
    pub scale_cc: u8,
    /// For Tunes on a natural key; never Natural.
    pub spelling: Spelling,
    pub rtp_peers: Vec<Peer>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            midi_port: String::new(),
            rtp_enabled: false,
            rtp_name: "TuneSync".into(),
            rtp_port: 5004,
            channel: 16,
            scale_cc: 20,
            spelling: Spelling::Sharp,
            rtp_peers: Vec::new(),
        }
    }
}

/// A dotted quad, each part 0..255 written with digits only.
pub fn is_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255))
}

fn int_in(v: &str, lo: i64, hi: i64) -> Option<i64> {
    if v.is_empty() || !v.bytes().enumerate().all(|(i, b)| b.is_ascii_digit() || (i == 0 && b == b'-')) {
        return None;
    }
    v.parse::<i64>().ok().filter(|n| (lo..=hi).contains(n))
}

pub fn to_ini(s: &Settings) -> String {
    let mut o = String::from("[tune-remote]\n");
    o += &format!("midi_port={}\n", s.midi_port);
    o += &format!("rtp_enabled={}\n", if s.rtp_enabled { 1 } else { 0 });
    o += &format!("rtp_name={}\n", s.rtp_name);
    o += &format!("rtp_port={}\n", s.rtp_port);
    o += &format!("channel={}\n", if s.channel == 0 { "omni".to_string() } else { s.channel.to_string() });
    o += &format!("scale_cc={}\n", s.scale_cc);
    o += &format!("spelling={}\n", if s.spelling == Spelling::Flat { "flats" } else { "sharps" });
    for p in &s.rtp_peers {
        o += &format!("rtp_peer={}:{}{}\n", p.ip, p.port, if p.name.is_empty() { String::new() } else { format!(" {}", p.name) });
    }
    o
}

/// Unknown keys and bad values are skipped with a line in `warnings`; the rest is kept.
pub fn from_ini(text: &str, warnings: &mut Vec<String>) -> Settings {
    let mut s = Settings::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            warnings.push(format!("not key=value: {line}"));
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        let ok = match k {
            "midi_port" => {
                s.midi_port = v.to_string();
                true
            }
            "rtp_enabled" => match v {
                "0" | "1" => {
                    s.rtp_enabled = v == "1";
                    true
                }
                _ => false,
            },
            "rtp_name" => {
                let ok = !v.is_empty() && v.len() <= 63;
                if ok {
                    s.rtp_name = v.to_string();
                }
                ok
            }
            // data port = +1
            "rtp_port" => int_in(v, 1024, 65534).map(|n| s.rtp_port = n as u16).is_some(),
            "channel" => {
                if v == "omni" {
                    s.channel = 0;
                    true
                } else {
                    int_in(v, 1, 16).map(|n| s.channel = n as u8).is_some()
                }
            }
            // 120..127 are channel mode messages
            "scale_cc" => int_in(v, 0, 119).map(|n| s.scale_cc = n as u8).is_some(),
            "spelling" => match v {
                "sharps" | "flats" => {
                    s.spelling = if v == "flats" { Spelling::Flat } else { Spelling::Sharp };
                    true
                }
                _ => false,
            },
            "rtp_peer" => {
                // ip:port, then the name it was found under
                let (addr, name) = match v.split_once(' ') {
                    Some((a, n)) => (a, n.trim()),
                    None => (v, ""),
                };
                match addr.rsplit_once(':') {
                    Some((ip, port)) if is_ipv4(ip) => match int_in(port, 1, 65534) {
                        Some(p) => {
                            let peer = Peer { ip: ip.to_string(), port: p as u16, name: name.to_string() };
                            if !s.rtp_peers.iter().any(|q| q.same(&peer)) && s.rtp_peers.len() < MAX_RTP_PEERS {
                                s.rtp_peers.push(peer);
                            }
                            true
                        }
                        None => false,
                    },
                    _ => false,
                }
            }
            _ => {
                warnings.push(format!("unknown setting {k}"));
                continue;
            }
        };
        if !ok {
            warnings.push(format!("bad value for {k}: \"{v}\", kept the default"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings { midi_port: "MIDISPORT 2x2 In A".into(), rtp_enabled: true, channel: 0, scale_cc: 7, spelling: Spelling::Flat, ..Default::default() };
        let mut w = Vec::new();
        let t = from_ini(&to_ini(&s), &mut w);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t, s);
        assert_eq!((t.rtp_port, t.rtp_name.as_str()), (5004, "TuneSync"));
    }

    #[test]
    fn bad_values_keep_defaults() {
        let mut w = Vec::new();
        let t = from_ini("channel=17\nscale_cc=120\nrtp_port=80\nfoo=1\nspelling=x\nmidi_port = A B \n", &mut w);
        assert_eq!((t.channel, t.scale_cc, t.rtp_port, t.spelling), (16, 20, 5004, Spelling::Sharp));
        assert_eq!(t.midi_port, "A B");
        assert_eq!(w.len(), 5, "{w:?}");
    }

    #[test]
    fn peers() {
        let s = Settings {
            rtp_peers: vec![
                Peer { ip: "192.0.2.41".into(), port: 5004, name: "Playback Mac".into() },
                Peer { ip: "192.0.2.42".into(), port: 5008, name: String::new() },
            ],
            ..Default::default()
        };
        let mut w = Vec::new();
        let t = from_ini(&to_ini(&s), &mut w);
        assert!(w.is_empty());
        assert_eq!(t.rtp_peers, s.rtp_peers);
        w.clear();
        let t = from_ini(
            "rtp_peer=192.0.2.41:5004 A\nrtp_peer=192.0.2.41:5004 again\nrtp_peer=host:5004\nrtp_peer=1.2.3:5004\nrtp_peer=1.2.3.4:0\nrtp_peer=1.2.3.4\n",
            &mut w,
        );
        assert_eq!(t.rtp_peers.len(), 1, "duplicates merged");
        assert_eq!(w.len(), 4, "bad peers skipped: {w:?}");
    }

    #[test]
    fn ipv4() {
        assert!(is_ipv4("192.0.2.41") && is_ipv4("0.0.0.0"));
        for bad in ["256.1.1.1", "1.2.3", "1.2.3.4.5", "1..2.3", "+1.2.3.4", "", "1.2.3.1234"] {
            assert!(!is_ipv4(bad), "{bad}");
        }
    }
}
