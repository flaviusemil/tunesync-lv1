//! The structs LV1 and a surface driver exchange (interface version 6).
//!
//! Ported from Headlink's `tools/surfaceprobe/wmsd_abi.h` (spike/fader-surface, 85d906e),
//! which was reverse-engineered from LV1 17.3.50 (Mac MixerCoreSG + Mackie_Control with
//! symbols; Windows MixerCoreSG.dll checked against the same constants) and proven on the
//! rig with LV1 17.3 and 16.5. Waves builds with `#pragma pack(1)`, so every struct here
//! is packed and its size is checked at compile time. The layouts are the same on Windows
//! and macOS: none of them holds a `long`.
//!
//! Only what a driver needs to read and write mixer controls is here; meters, the patch
//! and the inventory (also in the C header) are not.
use std::ffi::c_void;

/// `WCSurfaceDriverManager::LoadSurfaceDriverFromPath` requires exactly this.
pub const INTERFACE_VERSION: i32 = 6;

/// `WMSD_GetAvailableControllerInfo(ControllerInfo*)`. LV1 zeroes the 0x602 bytes and
/// writes the driver's own path at +0x202 before the call. An empty `type_` means "list
/// nothing" (how LV1's own GUI driver stays out of Setup -> Control Surfaces).
#[repr(C, packed)]
pub struct ControllerInfo {
    /// +0x000 Mackie: "Mackie"
    pub name: [u8; 0x80],
    /// +0x080 Mackie: "Surface"
    pub category: [u8; 0x80],
    /// +0x100 the key: prefs `<ControllerType>`
    pub type_: [u8; 0x80],
    /// +0x180
    pub extra: [u8; 0x80],
    /// +0x200 Mackie sets 1
    pub flag200: u8,
    pub flag201: u8,
    /// +0x202 in: this driver's path
    pub path: [u8; 0x400],
}
const _: () = assert!(std::mem::size_of::<ControllerInfo>() == 0x602);

/// One host callback: MixerCoreSG's exported `<Name>Wrapper(core, struct*)`.
pub type HostFn = unsafe extern "C" fn(core: *mut c_void, arg: *mut c_void) -> i32;

/// Arg 3 of `WMSD_CreateControllerForType`: MixerCoreSG's seven exported wrappers, each
/// tail-calling the WCMixerCore virtual of the same name.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CoreCallbacks {
    pub set_mixer_control: Option<HostFn>,
    pub get_mixer_control: Option<HostFn>,
    pub set_mixer_assignments: Option<HostFn>,
    pub get_mixer_assignments: Option<HostFn>,
    pub set_mixer_param: Option<HostFn>,
    pub get_mixer_param: Option<HostFn>,
    pub send_mixer_command: Option<HostFn>,
}
const _: () = assert!(std::mem::size_of::<CoreCallbacks>() == 7 * 8);

/// The names MixerCoreSG exports the seven wrappers under, in table order.
pub const CALLBACK_EXPORTS: [&str; 7] = [
    "SetMixerControlWrapper",
    "GetMixerControlWrapper",
    "SetMixerAssignmentsWrapper",
    "GetMixerAssignmentsWrapper",
    "SetMixerParamWrapper",
    "GetMixerParamWrapper",
    "SendMixerCommandWrapper",
];

/// `wvMixerNS::WEControlType`, at MixerControl +0x20.
pub mod ctl {
    pub const CONTROL: u32 = 0;
    pub const DISPLAY: u32 = 7;
    pub const REGISTER: u32 = 14;
    pub const TOUCH: u32 = 15;
    pub const RELEASE: u32 = 16;
}

/// `WSSectionControlID.section` (sWESectionType).
pub mod section {
    pub const INPUT: u32 = 0;
    pub const PLUGINS: u32 = 4;
    pub const OUTPUT: u32 = 7;
    pub const NAME: u32 = 8;
}

/// `WSTrackID.type`.
pub mod track {
    pub const CH: u32 = 0;
    pub const GP: u32 = 1;
    pub const FX: u32 = 2;
    pub const MAIN: u32 = 3;
    pub const DCA: u32 = 12;
}

/// `wvMixerNS::WSControlID`, 0x20 bytes. -1 (u32::MAX) = n/a.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct ControlId {
    pub size: u32,
    pub f04: u32,
    pub track_type: u32,
    pub track_index: u32,
    pub section: u32,
    pub section_index: u32,
    pub channel: u32,
    pub control_id: u32,
}
const _: () = assert!(std::mem::size_of::<ControlId>() == 0x20);

/// `wvMixerNS::WSMixerControl`, 0x1D8 bytes: what SetMixerControl / GetMixerControl take
/// and what `WMSD_ControllerDisplayUpdate` hands every surface.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MixerControl {
    /// +0x000 0x1D8
    pub size: u32,
    /// +0x004 0 = control (SetMixerControl refuses anything else)
    pub struct_type: u32,
    pub flags: u32,
    /// +0x00C set by Mackie only in do-by-group mode
    pub sender: u64,
    pub f14: [u8; 0x0C],
    /// +0x020 [`ctl`]
    pub ctl_type: u32,
    /// +0x024
    pub id: ControlId,
    /// +0x044 0 = absolute real-world (the only kind Set accepts)
    pub value_type: u32,
    /// +0x048
    pub value: f64,
    /// +0x050 the display string (section Name: the track name)
    pub text: [u8; 0x180],
    /// +0x1D0 do-by-group track, -1/-1 = none
    pub grp_type: u32,
    pub grp_index: u32,
}
const _: () = assert!(std::mem::size_of::<MixerControl>() == 0x1D8);

impl MixerControl {
    /// A control addressed to (track, section, control id); sectionIndex and channel -1.
    pub fn new(ctl_type: u32, track_type: u32, track_index: u32, section: u32, control_id: u32) -> Self {
        MixerControl {
            size: 0x1D8,
            struct_type: 0,
            flags: 0,
            sender: 0,
            f14: [0; 0x0C],
            ctl_type,
            id: ControlId { size: 0x20, f04: 0, track_type, track_index, section, section_index: u32::MAX, channel: u32::MAX, control_id },
            value_type: 0,
            value: 0.0,
            text: [0; 0x180],
            grp_type: u32::MAX,
            grp_index: u32::MAX,
        }
    }

    /// The text field as UTF-8 (lossy), up to its NUL.
    pub fn text(&self) -> String {
        let t = self.text;
        let n = t.iter().position(|&b| b == 0).unwrap_or(t.len());
        String::from_utf8_lossy(&t[..n]).into_owned()
    }

    /// Sets the text field (truncated, always NUL-terminated).
    pub fn set_text(&mut self, s: &str) {
        let mut t = [0u8; 0x180];
        let b = s.as_bytes();
        let n = b.len().min(t.len() - 1);
        t[..n].copy_from_slice(&b[..n]);
        self.text = t;
    }
}

/// `wvMixerNS::WEMixerParamType`: what a WSMixerParam's +0x28 names.
///
/// Decoded from MixerCoreSG-win.dll 17.3.50's `ToString(WEMixerParamType)` (the same table
/// gives the ids the probes used on the rig: 7034 MetersBatch, 7048 RegisterMeters).
pub mod param {
    /// LV1's system tempo (Top Bar BPM, Setup > Mixer Settings > Tempo): a
    /// [`MixerParamDouble`](super::MixerParamDouble) in BPM. Setting it runs
    /// `WCMixerCore::SetParamMixerSettingsTempo` -> every tempo-listening plug-in.
    pub const MIXER_SETTING_TEMPO: u32 = 4019;
}

/// `wvMixerNS::WSMixerParamDouble`, 0x3D bytes: the 0x35-byte WSMixerParam header and a
/// double. The header is what `WSMixerParamDouble(WEMixerParamType)` writes (Mac
/// MyMon_Control 17.3.50); `GetParamTrait` / `SetParamMixerSettingsTempo` read and write
/// the value at +0x35. LV1's own MyMon driver sets the tempo with exactly this.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MixerParamDouble {
    pub f00: u32,
    /// 2
    pub f04: u32,
    pub f08: [u8; 0x20],
    /// +0x28 [`param`]
    pub type_: u32,
    /// -1
    pub f2c: u32,
    pub f30: u32,
    /// 1
    pub f34: u8,
    /// +0x35
    pub value: f64,
}
const _: () = assert!(std::mem::size_of::<MixerParamDouble>() == 0x3D);

impl MixerParamDouble {
    pub fn new(type_: u32, value: f64) -> Self {
        MixerParamDouble { f00: 0, f04: 2, f08: [0; 0x20], type_, f2c: u32::MAX, f30: 0, f34: 1, value }
    }
}

/// Copies `s` into a fixed C string field, truncated, NUL-terminated.
pub fn put_cstr(dst: &mut [u8], s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&b[..n]);
    if n < dst.len() {
        dst[n] = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    #[test]
    fn layout_matches_the_c_header() {
        assert_eq!(offset_of!(ControllerInfo, category), 0x080);
        assert_eq!(offset_of!(ControllerInfo, type_), 0x100);
        assert_eq!(offset_of!(ControllerInfo, flag200), 0x200);
        assert_eq!(offset_of!(ControllerInfo, path), 0x202);
        assert_eq!(offset_of!(MixerControl, sender), 0x00C);
        assert_eq!(offset_of!(MixerControl, ctl_type), 0x020);
        assert_eq!(offset_of!(MixerControl, id), 0x024);
        assert_eq!(offset_of!(MixerControl, value_type), 0x044);
        assert_eq!(offset_of!(MixerControl, value), 0x048);
        assert_eq!(offset_of!(MixerControl, text), 0x050);
        assert_eq!(offset_of!(MixerControl, grp_type), 0x1D0);
        assert_eq!(offset_of!(ControlId, section), 0x10);
        assert_eq!(offset_of!(ControlId, control_id), 0x1C);
        assert_eq!(offset_of!(MixerParamDouble, type_), 0x28);
        assert_eq!(offset_of!(MixerParamDouble, f34), 0x34);
        assert_eq!(offset_of!(MixerParamDouble, value), 0x35);
    }

    #[test]
    fn a_double_param_is_what_lv1_builds() {
        let p = MixerParamDouble::new(param::MIXER_SETTING_TEMPO, 128.0);
        let b: [u8; 0x3D] = unsafe { std::mem::transmute(p) };
        assert_eq!(&b[0..8], &[0, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(&b[0x28..0x35], &[0xB3, 0x0F, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 1]);
        assert_eq!(&b[0x35..], &128.0f64.to_le_bytes());
    }

    #[test]
    fn control_init_and_text() {
        let mut c = MixerControl::new(ctl::CONTROL, track::CH, 42, section::PLUGINS, 11);
        let id = c.id;
        assert_eq!({ c.size }, 0x1D8);
        assert_eq!(({ id.track_index }, { id.section }, { id.section_index }, { id.control_id }), (42, 4, u32::MAX, 11));
        assert_eq!(({ c.grp_type }, { c.grp_index }), (u32::MAX, u32::MAX));
        c.set_text("Tune RT");
        assert_eq!(c.text(), "Tune RT");
        c.set_text(&"x".repeat(1000));
        assert_eq!(c.text().len(), 0x17F, "truncated, NUL kept");
        let mut buf = [0xAAu8; 8];
        put_cstr(&mut buf, "TuneSyncX");
        assert_eq!(&buf, b"TuneSyn\0");
    }
}
