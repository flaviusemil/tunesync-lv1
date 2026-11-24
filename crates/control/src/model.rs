//! The window's [`Model`] from the running TuneSync, a few times a second. Portable:
//! the Windows window and (later) the Mac one share it.
use std::net::{IpAddr, Ipv4Addr, UdpSocket};

use rtpmidi::session::{peer_state_name, PeerState};
use tunesync::midi_map::bpm_text;
use tunesync::tune::{key_name, scale_name};
use tunesync_host::rtp::RtpView;
use tunesync_host::Runtime;
use tunesync_ui::model::*;

/// MIDI counts as "arriving" for this long after the last message.
const ACTIVE_MS: u64 = 3000;

pub fn build(rt: &Runtime, lv1_problem: Option<String>, local_ips: &[String]) -> Model {
    let now = rt.now_ms();
    let st = rt.status();
    let last_midi = rt.midi_last_ms();
    let rv: RtpView = rt.rtp();
    let peers: Vec<PeerRow> = rv
        .peers
        .iter()
        .map(|p| {
            let connected = p.state == PeerState::Connected;
            PeerRow {
                name: p.display(),
                ip: p.ip.clone(),
                port: if p.target != 0 { p.target } else { p.ctl_port },
                status: if connected {
                    ago(rv.session_now_ms.saturating_sub(p.last_heard_ms))
                } else {
                    peer_state_name(p.state).to_string()
                },
                rtt: if connected { rtt(p.rtt_ms) } else { String::new() },
                midi_at_ms: p.last_midi_ms, // the session runs on now_ms()'s clock
                connected,
                invited: p.invited,
            }
        })
        .collect();
    let found = rv
        .found
        .iter()
        .filter(|f| !f.ip.is_empty() && f.port != 0)
        .map(|f| FoundRow { name: f.instance.clone(), ip: f.ip.clone(), port: f.port, joined: rv.peers.iter().any(|p| p.ip == f.ip) })
        .collect();
    Model {
        version: crate::VERSION.to_string(),
        settings: rt.settings(),
        midi_ports: rt.midi_ports(),
        midi_open: rt.midi_open(),
        midi_active: last_midi != 0 && now.saturating_sub(last_midi) < ACTIVE_MS,
        midi_at_ms: last_midi,
        rtp: RtpModel {
            open: rv.open,
            port: rv.open_port,
            error: rv.error.clone(),
            active: rv.last_midi_ms != 0 && now.saturating_sub(rv.last_midi_ms) < ACTIVE_MS,
            advert: rv.advert.clone(),
            peers,
            found,
        },
        last: (st.changes > 0).then(|| LastChange {
            label: st.last.label.clone(),
            meta: [st.last.stamp.as_str(), st.last.source.as_str(), st.last.brief.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · "),
            problem: st.last.problem,
        }),
        tempo: TempoModel {
            bpm: st.tempo.map(bpm_text).unwrap_or_default(),
            meta: st
                .tempo_set
                .as_ref()
                .map(|t| {
                    let mut s = format!("{} · {}", t.stamp, t.source);
                    if !t.ok {
                        s += &format!(" · {} BPM did not land", bpm_text(t.bpm));
                    }
                    s
                })
                .unwrap_or_default(),
            problem: st.tempo_set.as_ref().is_some_and(|t| !t.ok),
        },
        tunes: rt
            .tunes()
            .iter()
            .map(|t| TuneRow {
                label: t.label.clone(),
                name: t.name.clone(),
                key: t.key.and_then(key_name).map(pretty_key).unwrap_or_else(|| "?".into()),
                scale: t.scale.and_then(scale_name).unwrap_or("?").to_string(),
                receive: t.receive,
                ok: t.layout_ok,
            })
            .collect(),
        log: rt.log().snapshot(),
        lv1_problem,
        local_ips: local_ips.to_vec(),
    }
}

/// This computer's IPv4 addresses for the RTP sheet's "add it by hand", read again every
/// few seconds (WiFi may come up after LV1 started).
#[derive(Default)]
pub struct LocalIps {
    at_ms: Option<u64>,
    list: Vec<String>,
}

impl LocalIps {
    pub const EVERY_MS: u64 = 5000;

    pub fn get(&mut self, rt: &Runtime) -> &[String] {
        let now = rt.now_ms();
        if self.at_ms.is_none_or(|t| now.saturating_sub(t) >= Self::EVERY_MS) {
            let peers: Vec<IpAddr> = rt.rtp().peers.iter().filter(|p| p.state == PeerState::Connected).filter_map(|p| p.ip.parse().ok()).collect();
            self.list = local_ips(&peers);
            self.at_ms = Some(now);
        }
        &self.list
    }
}

/// Every IPv4 address of this computer's adapters that are up, "192.0.2.172 (WiFi)", the
/// ones a connected peer is reached through first. Loopback and 169.254 link-local
/// addresses are left out: a Mac can't use them.
pub fn local_ips(peers: &[IpAddr]) -> Vec<String> {
    let facing: Vec<Ipv4Addr> = peers.iter().filter_map(|p| if let IpAddr::V4(v4) = p { facing(*v4) } else { None }).collect();
    order(adapters(), &facing)
}

/// `all` (address, adapter name) with the `facing` ones first, labelled.
pub fn order(mut all: Vec<(Ipv4Addr, String)>, facing: &[Ipv4Addr]) -> Vec<String> {
    for f in facing {
        if usable(*f) && !all.iter().any(|(ip, _)| ip == f) {
            all.push((*f, String::new()));
        }
    }
    all.retain(|(ip, _)| usable(*ip));
    all.sort_by_key(|(ip, _)| !facing.contains(ip)); // stable: the adapters' own order after
    all.into_iter().map(|(ip, name)| if name.is_empty() { ip.to_string() } else { format!("{ip} ({name})") }).collect()
}

fn usable(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

/// The address this computer sends to `peer` from (no packet is sent: connecting a UDP
/// socket only chooses the source address).
fn facing(peer: Ipv4Addr) -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect((peer, 9)).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(a) => Some(a),
        IpAddr::V6(_) => None,
    }
}

/// The IPv4 addresses of the adapters that are up, with their names ("WiFi", "WSG Port").
#[cfg(windows)]
fn adapters() -> Vec<(Ipv4Addr, String)> {
    use windows::Win32::NetworkManagement::IpHelper::{GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    let mut out: Vec<(Ipv4Addr, String)> = Vec::new();
    let mut size: u32 = 16 * 1024;
    for _ in 0..3 {
        let mut buf = vec![0u64; (size as usize).div_ceil(8)]; // 8-aligned
        let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `buf` holds `size` bytes; Windows fills it with a linked list inside it.
        let rc = unsafe { GetAdaptersAddresses(u32::from(AF_INET.0), GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER, None, Some(first), &mut size) };
        if rc == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if rc != 0 {
            break;
        }
        let mut a = first.cast_const();
        // SAFETY: every pointer followed here is into `buf` (or null), as Windows wrote it.
        unsafe {
            while let Some(ad) = a.as_ref() {
                if ad.OperStatus == IfOperStatusUp {
                    let name = ad.FriendlyName.to_string().unwrap_or_default();
                    let mut u = ad.FirstUnicastAddress.cast_const();
                    while let Some(ua) = u.as_ref() {
                        let sa = ua.Address.lpSockaddr;
                        if !sa.is_null() && (*sa).sa_family == AF_INET {
                            let b = (*sa.cast::<SOCKADDR_IN>()).sin_addr.S_un.S_un_b;
                            let ip = Ipv4Addr::new(b.s_b1, b.s_b2, b.s_b3, b.s_b4);
                            if !out.iter().any(|(x, _)| *x == ip) {
                                out.push((ip, name.clone()));
                            }
                        }
                        u = ua.Next.cast_const();
                    }
                }
                a = ad.Next.cast_const();
            }
        }
        break;
    }
    out
}

/// Elsewhere (tests, a Mac build without its own list yet): the addresses the routing
/// table picks for the private networks, unnamed.
#[cfg(not(windows))]
fn adapters() -> Vec<(Ipv4Addr, String)> {
    let mut v: Vec<(Ipv4Addr, String)> = Vec::new();
    for probe in [Ipv4Addr::new(10, 255, 255, 255), Ipv4Addr::new(192, 168, 255, 255), Ipv4Addr::new(172, 31, 255, 255)] {
        if let Some(ip) = facing(probe) {
            if !v.iter().any(|(x, _)| *x == ip) {
                v.push((ip, String::new()));
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_a_peer_uses_comes_first() {
        let rig = vec![(Ipv4Addr::new(198, 51, 100, 16), "WSG Port".to_string()), (Ipv4Addr::new(169, 254, 10, 20), "Local Area Connection* 2".to_string()), (Ipv4Addr::new(192, 0, 2, 172), "WiFi".to_string()), (Ipv4Addr::LOCALHOST, "Loopback".to_string())];
        assert_eq!(order(rig.clone(), &[]), ["198.51.100.16 (WSG Port)", "192.0.2.172 (WiFi)"], "no link-local, no loopback");
        assert_eq!(order(rig.clone(), &[Ipv4Addr::new(192, 0, 2, 172)]), ["192.0.2.172 (WiFi)", "198.51.100.16 (WSG Port)"], "the iPad's network first");
        assert_eq!(order(Vec::new(), &[Ipv4Addr::new(203, 0, 113, 9)]), ["203.0.113.9"], "a facing address the list missed");
    }
}
