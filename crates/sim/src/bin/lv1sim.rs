//! lv1sim - loads TuneSync_Control.dll the way eMotion LV1 does and drives it end to end,
//! against the fake MixerCoreSG.dll beside it. Runs on Windows or under Wine (and on
//! macOS with TuneSync_Control.bundle: see `src/mac_sim.rs`):
//!
//! ```text
//! lv1sim.exe [--window <seconds>]     both DLLs next to it; exit code 0 = every check passed
//! lv1sim [--window <s>] [--shot <prefix>] [--bundle <path>] [--core <libmixercoresg.dylib>]   (macOS)
//! ```
//!
//! 1. resolve the 18 WMSD_* exports, check the interface version and the controller info;
//! 2. Create with MixerCoreSG's seven wrappers (the real callback-table check runs);
//! 3. wait for the first scan, then join TuneSync's RTP-MIDI session as a playback rig and
//!    send CC 20 = 4 + Note On D: the two Rcv Tunes go to D Harmonic Minor (Eb keeps its
//!    flat spelling: D stays D), the Lock one is untouched, every Touch has its Release;
//! 4. optionally open the settings window for a while (a screenshot can be taken);
//! 5. Destroy (bounded), and print TuneSync's log.

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("lv1sim runs on Windows (or under Wine) and on macOS");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
#[path = "../mac_sim.rs"]
mod mac;

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(mac::run());
}

#[cfg(windows)]
fn main() {
    std::process::exit(win::run());
}

/// Finds, clicks and captures the settings window (GDI: works under Wine and on the rig).
#[cfg(windows)]
mod shot {
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub fn find() -> isize {
        let class: Vec<u16> = "TuneSyncWindow\0".encode_utf16().collect();
        unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) as isize }
    }

    fn scale(h: HWND) -> f32 {
        unsafe {
            let dc = GetDC(h);
            let d = GetDeviceCaps(dc, LOGPIXELSX as i32);
            ReleaseDC(h, dc);
            if d > 0 { d as f32 / 96.0 } else { 1.0 }
        }
    }

    pub fn click(hwnd: isize, x_dip: f32, y_dip: f32) {
        let h = hwnd as HWND;
        let s = scale(h);
        let lp = (((y_dip * s) as isize) << 16) | ((x_dip * s) as isize & 0xffff);
        unsafe {
            PostMessageW(h, WM_LBUTTONDOWN, 1, lp);
            PostMessageW(h, WM_LBUTTONUP, 0, lp);
        }
    }

    pub fn key_escape(hwnd: isize) {
        unsafe {
            PostMessageW(hwnd as HWND, WM_KEYDOWN, 0x1B, 0);
        }
    }

    /// The window's client area from the screen, as a 24-bit BMP.
    pub fn capture(hwnd: isize, path: &str) -> bool {
        let h = hwnd as HWND;
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(h, &mut r);
            let (w, ht) = (r.right - r.left, r.bottom - r.top);
            if w <= 0 || ht <= 0 {
                return false;
            }
            let src = GetDC(h);
            let mem = CreateCompatibleDC(src);
            let bmp = CreateCompatibleBitmap(src, w, ht);
            let old = SelectObject(mem, bmp);
            BitBlt(mem, 0, 0, w, ht, src, 0, 0, SRCCOPY);
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = ht;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 24;
            let stride = ((w * 3 + 3) / 4) * 4;
            let mut px = vec![0u8; (stride * ht) as usize];
            GetDIBits(mem, bmp, 0, ht as u32, px.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS);
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(h, src);
            let mut f = Vec::new();
            let size = 54 + px.len() as u32;
            f.extend_from_slice(b"BM");
            f.extend_from_slice(&size.to_le_bytes());
            f.extend_from_slice(&[0, 0, 0, 0]);
            f.extend_from_slice(&54u32.to_le_bytes());
            f.extend_from_slice(&40u32.to_le_bytes());
            f.extend_from_slice(&w.to_le_bytes());
            f.extend_from_slice(&ht.to_le_bytes());
            f.extend_from_slice(&1u16.to_le_bytes());
            f.extend_from_slice(&24u16.to_le_bytes());
            f.extend_from_slice(&[0u8; 24]);
            f.extend_from_slice(&px);
            std::fs::write(path, f).is_ok()
        }
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    use super::shot;
    use lv1_surface::abi::{ControllerInfo, CoreCallbacks, HostFn, CALLBACK_EXPORTS};
    use rtpmidi::session::{PeerState, Session, SessionConfig};

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    unsafe fn sym<T>(m: *mut c_void, name: &str) -> T {
        let c: Vec<u8> = name.bytes().chain([0]).collect();
        let p = GetProcAddress(m, c.as_ptr());
        assert!(!p.is_null(), "{name} is not exported");
        std::mem::transmute_copy(&p)
    }

    struct Checks(u32, u32);
    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            if ok {
                self.0 += 1;
                println!("ok    {what}");
            } else {
                self.1 += 1;
                println!("FAIL  {what}");
            }
        }
    }

    fn wait_for(ms: u64, mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    pub fn run() -> i32 {
        let window_secs: u64 = std::env::args().skip_while(|a| a != "--window").nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let shot_prefix: Option<String> = std::env::args().skip_while(|a| a != "--shot").nth(1);
        let mut c = Checks(0, 0);
        let dir = std::env::temp_dir().join(format!("tunesync-sim-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("TUNESYNC_DIR", &dir);
        const PORT: u16 = 47004;
        std::fs::write(dir.join("settings.ini"), format!("[tune-remote]\nrtp_enabled=1\nrtp_port={PORT}\nchannel=16\nscale_cc=20\n")).unwrap();
        println!("lv1sim: data folder {}", dir.display());

        unsafe {
            // LV1 has MixerCoreSG loaded before any driver.
            let core_dll = LoadLibraryW(wide("MixerCoreSG.dll").as_ptr());
            assert!(!core_dll.is_null(), "MixerCoreSG.dll (the fake) not found next to lv1sim.exe");
            let drv = LoadLibraryW(wide("TuneSync_Control.dll").as_ptr());
            assert!(!drv.is_null(), "TuneSync_Control.dll not found next to lv1sim.exe");

            type V = unsafe extern "C" fn() -> i32;
            type Info = unsafe extern "C" fn(*mut ControllerInfo) -> i32;
            type Create = unsafe extern "C" fn(*const u8, *mut c_void, *const CoreCallbacks, *mut c_void, *const c_void, *mut bool) -> *mut c_void;
            type Ctrl = unsafe extern "C" fn(*mut c_void) -> i32;
            type Destroy = unsafe extern "C" fn(*mut c_void, bool) -> i32;
            type Preset = unsafe extern "C" fn(*mut c_void, *mut u8, u32, *mut u32, *mut u8, u32) -> i32;
            for n in [
                "WMSD_QueryInterfaceVersion", "WMSD_GetAvailableControllerInfo", "WMSD_TestControllerForType", "WMSD_CreateControllerForType",
                "WMSD_InitControllerForType", "WMSD_GetTypeForController", "WMSD_DestroyController", "WMSD_ShowConfigWindow", "WMSD_LockConfigWindow",
                "WMSD_UnlockConfigWindow", "WMSD_IdentifyController", "WMSD_GetPreset", "WMSD_SetPreset", "WMSD_CommandStatusUpdate",
                "WMSD_MixerParamWasChanged", "WMSD_MixerAssignmentsChanged", "WMSD_ControllerDisplayUpdate", "WMSD_MixerMessageSent",
            ] {
                let c2: Vec<u8> = n.bytes().chain([0]).collect();
                assert!(!GetProcAddress(drv, c2.as_ptr()).is_null(), "{n} missing");
            }
            c.check(true, "all 18 WMSD_* exports resolve");
            let version: V = sym(drv, "WMSD_QueryInterfaceVersion");
            c.check(version() == 6, "interface version 6");
            let mut info: ControllerInfo = std::mem::zeroed();
            let get_info: Info = sym(drv, "WMSD_GetAvailableControllerInfo");
            get_info(&mut info);
            let name = String::from_utf8_lossy(&info.name[..info.name.iter().position(|&b| b == 0).unwrap_or(0)]).to_string();
            let ty = String::from_utf8_lossy(&info.type_[..info.type_.iter().position(|&b| b == 0).unwrap_or(0)]).to_string();
            c.check(name == "TuneSync" && ty == "TuneSync", &format!("controller info: name \"{name}\", type \"{ty}\""));

            // The callback table, as LV1 builds it: MixerCoreSG's exported wrappers.
            let ptrs: Vec<Option<HostFn>> = CALLBACK_EXPORTS.iter().map(|n| Some(sym::<HostFn>(core_dll, n))).collect();
            let cb = CoreCallbacks {
                set_mixer_control: ptrs[0],
                get_mixer_control: ptrs[1],
                set_mixer_assignments: ptrs[2],
                get_mixer_assignments: ptrs[3],
                set_mixer_param: ptrs[4],
                get_mixer_param: ptrs[5],
                send_mixer_command: ptrs[6],
            };
            let reset: unsafe extern "C" fn() = sym(core_dll, "fakecore_reset");
            let param: unsafe extern "C" fn(u32, u32, u32, u32) -> f64 = sym(core_dll, "fakecore_param");
            let counts: unsafe extern "C" fn(*mut u64) = sym(core_dll, "fakecore_counts");
            let count = || {
                let mut v = [0u64; 4];
                counts(v.as_mut_ptr());
                v
            };
            let status_fn: unsafe extern "C" fn(*mut u8, *mut u8, *mut u8, usize) -> u32 = sym(core_dll, "fakecore_status");
            let status_and_picture = || {
                let (mut ty, mut st, mut pic) = ([0u8; 1024], [0u8; 1024], [0u8; 1024]);
                let color = status_fn(ty.as_mut_ptr(), st.as_mut_ptr(), pic.as_mut_ptr(), 1024);
                let text = |b: &[u8]| String::from_utf8_lossy(&b[..b.iter().position(|&x| x == 0).unwrap_or(0)]).into_owned();
                ((text(&ty), text(&st), color), text(&pic))
            };
            let status = || status_and_picture().0;
            reset();

            let create: Create = sym(drv, "WMSD_CreateControllerForType");
            let mut core_obj = [0u8; 64]; // LV1's core: opaque to the driver, passed back to the callbacks
            let t0 = Instant::now();
            let ctrl = create(c"LV1".as_ptr() as *const u8, core_obj.as_mut_ptr() as *mut c_void, &cb, std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut());
            c.check(!ctrl.is_null(), &format!("Create returned a controller in {} ms", t0.elapsed().as_millis()));
            c.check(t0.elapsed() < Duration::from_millis(200), "Create did not block LV1's thread");
            c.check(wait_for(5000, || count()[0] > 160 * 8), &format!("the first scan ran ({} reads)", count()[0]));
            wait_for(2000, || !status().0.is_empty());
            let s0 = status();
            c.check(s0 == ("TuneSync".into(), "N/A".into(), 0xFF0000), &format!("Control Surfaces shows {s0:?} before anything connects"));
            // build-dll.sh puts the picture beside the DLL, as the installer does
            let pic = status_and_picture().1;
            c.check(pic.ends_with("\\tunesync-surface.png") && std::path::Path::new(&pic).is_file(), &format!("the tile picture is sent: {pic}"));

            // A playback rig joins the session and sends the cue.
            let rig = Session::new();
            rig.open(SessionConfig { name: "lv1sim rig".into(), port: PORT + 100, ..Default::default() }).unwrap();
            rig.invite("127.0.0.1", PORT, "");
            let joined = wait_for(5000, || {
                rig.poll(5);
                rig.peers().iter().any(|p| p.state == PeerState::Connected)
            });
            c.check(joined, "a rig joined TuneSync's RTP-MIDI session");
            let on = wait_for(2000, || {
                rig.poll(5);
                status().1 == "ON"
            });
            c.check(on && status().2 == 0x00FF00, &format!("Control Surfaces shows {:?} once the rig is connected", status()));
            let t1 = Instant::now();
            rig.send(&[&[0xBF, 20, 4], &[0x9F, 62, 100]]);
            let applied = wait_for(3000, || {
                rig.poll(5);
                // key and scale: each Control lands on its own, a moment apart
                param(0, 42, 0, 40) == 3.0 && param(0, 43, 2, 40) == 3.0 && param(0, 42, 0, 39) == 4.0 && param(0, 43, 2, 39) == 4.0
            });
            c.check(applied, &format!("CC 20 = 4 + Note On D: Ch 43 and Ch 44 -> D in {} ms", t1.elapsed().as_millis()));
            c.check(param(0, 42, 0, 39) == 4.0 && param(0, 43, 2, 39) == 4.0, "both on Harmonic Minor");
            c.check(param(0, 44, 0, 40) == 8.0 && param(0, 44, 0, 39) == 2.0, "the Lock Tune (Ch 45) untouched");
            wait_for(500, || false);
            let n = count();
            c.check(n[1] == 4 && n[2] == n[3] && n[2] == 4, &format!("4 Controls, every Touch released (touch {} release {})", n[2], n[3]));

            // A note on another channel changes nothing.
            rig.send(&[&[0x90, 64, 100]]);
            wait_for(300, || {
                rig.poll(5);
                false
            });
            c.check(param(0, 42, 0, 40) == 3.0, "a note on channel 1 is ignored");

            let preset: Preset = sym(drv, "WMSD_GetPreset");
            let mut size = 0u32;
            preset(ctrl, std::ptr::null_mut(), 0, &mut size, std::ptr::null_mut(), 0);
            c.check(size == 8, "GetPreset size query");

            if window_secs > 0 {
                let show: Ctrl = sym(drv, "WMSD_ShowConfigWindow");
                let t2 = Instant::now();
                show(ctrl);
                c.check(t2.elapsed() < Duration::from_millis(100), "ShowConfigWindow returned at once");
                let pump = |ms: u64| {
                    wait_for(ms, || {
                        rig.poll(20);
                        false
                    })
                };
                pump(2500);
                let hwnd = shot::find();
                c.check(hwnd != 0, "the settings window exists");
                if let Some(prefix) = &shot_prefix {
                    c.check(shot::capture(hwnd, &format!("{prefix}-page.bmp")), "screenshot of the page");
                    // a tap on the RTP-MIDI tile (DIP 360, 40) opens its sheet
                    shot::click(hwnd, 360.0, 40.0);
                    pump(800);
                    c.check(shot::capture(hwnd, &format!("{prefix}-rtp.bmp")), "screenshot of the RTP-MIDI sheet");
                    shot::key_escape(hwnd);
                    pump(300);
                }
                pump(window_secs.saturating_sub(4) * 1000);
            }

            rig.close();
            let destroy: Destroy = sym(drv, "WMSD_DestroyController");
            let t3 = Instant::now();
            destroy(ctrl, false);
            c.check(t3.elapsed() < Duration::from_millis(3000), &format!("Destroy finished in {} ms", t3.elapsed().as_millis()));
        }
        let log = std::fs::read_to_string(dir.join("tunesync.log")).unwrap_or_default();
        println!("---- tunesync.log\n{log}----");
        c.check(log.contains("D Harmonic Minor: set 2") && log.contains("skipped 1 locked"), "the change is in the log");
        println!("lv1sim: {} passed, {} failed", c.0, c.1);
        if c.1 == 0 {
            0
        } else {
            1
        }
    }
}
