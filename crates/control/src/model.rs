//! The window's [`Model`] from the running TuneSync, a few times a second. Portable:
//! the Windows window and (later) the Mac one share it.
use std::net::UdpSocket;

use rtpmidi::session::{peer_state_name, PeerState};
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
        rtp: RtpModel {
            open: rv.open,
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

/// This computer's IPv4 addresses as the routing table picks them for a few private
/// networks (no packet is sent: connecting a UDP socket only chooses the source address).
pub fn local_ips() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for probe in ["10.255.255.255:9", "192.168.255.255:9", "172.31.255.255:9"] {
        if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
            if s.connect(probe).is_ok() {
                if let Ok(a) = s.local_addr() {
                    let ip = a.ip().to_string();
                    if ip != "0.0.0.0" && !ip.starts_with("127.") && !v.contains(&ip) {
                        v.push(ip);
                    }
                }
            }
        }
    }
    v
}
