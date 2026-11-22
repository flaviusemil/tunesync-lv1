//! [`tunesync::apply::Mixer`] on LV1's own surface callbacks, the way Headlink's fader
//! probe proved Tune RT control on LV1 16.5: a plug-in parameter is section 4,
//! sectionIndex = insert slot, channel = parameter index, controlId 11 = value.
//!
//! Only the runtime's worker thread calls in here. SetMixerControl is asynchronous (it
//! returns at once and a read straight after still shows the old value), so `set` sends
//! Touch and Control, waits (bounded) until LV1 reads back the new value, then Release:
//! apply()'s read-back then sees what LV1 really holds.
use std::sync::Arc;
use std::time::{Duration, Instant};

use lv1_surface::abi::{ctl, section, MixerControl};
use lv1_surface::Lv1;
use tunesync::apply::{Mixer, ParamRead};
use tunesync::tune::CID_VALUE;

/// How long `set` waits for LV1 to show the value.
pub const SETTLE: Duration = Duration::from_millis(400);

pub struct Lv1Mixer {
    pub lv1: Arc<Lv1>,
}

fn plugin_control(ctl_type: u32, tt: u32, ti: u32, slot: u32, channel: i32, cid: i32) -> MixerControl {
    let mut c = MixerControl::new(ctl_type, tt, ti, section::PLUGINS, cid as u32);
    c.id.section_index = slot;
    c.id.channel = channel as u32;
    c
}

fn read(lv1: &Lv1, tt: u32, ti: u32, slot: u32, channel: i32, cid: i32) -> ParamRead {
    let mut c = plugin_control(ctl::CONTROL, tt, ti, slot, channel, cid);
    match lv1.get_control(&mut c) {
        Ok(0) => ParamRead { ok: true, value: c.value, text: c.text() },
        _ => ParamRead::default(),
    }
}

impl Mixer for Lv1Mixer {
    fn get(&mut self, tt: u32, ti: u32, slot: u32, channel: i32, cid: i32) -> ParamRead {
        read(&self.lv1, tt, ti, slot, channel, cid)
    }

    fn set(&mut self, tt: u32, ti: u32, slot: u32, param: i32, value: f64) -> bool {
        // Touch, Control(value), Release: what a surface knob does. Touch and Release carry
        // the value as text, as Mackie's do.
        let lv1 = &*self.lv1;
        let send = |ctl_type: u32| {
            let mut c = plugin_control(ctl_type, tt, ti, slot, param, CID_VALUE);
            c.value = value;
            if ctl_type != ctl::CONTROL {
                c.set_text(&format!("{value:.6}"));
            }
            matches!(lv1.set_control(&c), Ok(0))
        };
        if !send(ctl::TOUCH) {
            return false;
        }
        let ok = send(ctl::CONTROL);
        if ok {
            let until = Instant::now() + SETTLE;
            loop {
                let r = read(lv1, tt, ti, slot, param, CID_VALUE);
                if !r.ok || (r.value - value).abs() < 1e-6 || Instant::now() >= until {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        send(ctl::RELEASE);
        ok
    }

    fn track_name(&mut self, tt: u32, ti: u32) -> String {
        let mut c = MixerControl::new(ctl::CONTROL, tt, ti, section::NAME, u32::MAX);
        match self.lv1.get_control(&mut c) {
            Ok(0) => c.text(),
            _ => String::new(),
        }
    }
}
