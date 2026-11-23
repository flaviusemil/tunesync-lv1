# TuneSync: design

TuneSync is an eMotion LV1 control module that sets the key and scale of every Waves Tune
Real-Time instance from MIDI, and LV1's system tempo from MIDI clock or a CC. It is its own product with its own DLL, settings and installer.
It is not part of Headlink and does not need Headlink installed. It is open source.

## Why

In a show, the backing tracks know which song is playing, but Tune doesn't. Today the only
way to change Tune's key in LV1 is a snapshot or a hand on the plug-in. With TuneSync, the
tracks session sends one MIDI message at the top of each song and every Tune follows. The key
lives with the song in the tracks session, so reordering the set can't put Tune out of sync.

**Goals**
- MIDI from the tracks rig sets the key, and optionally the scale, of every Tune RT in LV1.
- A local MIDI port by default, with RTP-MIDI (network MIDI) as an option, both chosen in a
  settings window that also shows a log.
- Honour Tune's own **Receive / Lock** button: a Locked instance is never changed.
- Keep the spelling the engineer chose (Db stays a flat key; C# stays a sharp key).
- LV1's system tempo follows the tracks rig: its MIDI clock, or an exact CC (added
  2026-11-23, Flavius: "TuneSync sounds like it could relate").

**Non-goals (v1)**
- Following MIDI timecode, song lists or program-change presets on the LV1 side. The tracks
  session holds the key.
- Any other plug-in or parameter. (LV1's tempo is a mixer setting, not a plug-in parameter:
  setting it once moves every tempo-synced plug-in, so TuneSync never touches them itself.)
- macOS in the first release. Windows comes first; LV1's surface-driver ABI is the same on Mac,
  and a Mac version follows (Flavius, 2026-11-22: "This should work with mac too").

**Anti-goals** (failures even if everything else works)
- Touching anything except a Tune's key and scale, and LV1's system tempo.
- Blocking or stalling LV1: no LV1 thread ever waits on MIDI, the network or our window.
- Guessing. If a Tune's parameters don't look the way we expect (a Tune update could reorder
  them), skip that instance and say so in the log.

## How LV1 exposes Tune

Proven on LV1 16.5.189.293 on 2026-10-25. The surface-driver ABI is version 6, the same as
17.3. See `spike/fader-surface` in the Headlink repo: `docs/SPIKE-FADER-SURFACE.md`, "Plugin
parameters".

**Addressing.** A plug-in parameter is section 4 (Plugins), sectionIndex = insert slot,
channel = parameter index, controlId 11 = value. `text` is the display string.

**Slot identity** (channel -1):

| cid | Meaning |
|---|---|
| 0 | Plug-in name: `Tune RT` |
| 1 | 4-char code: `LVLM` for mono Tune RT (the stereo one is probably `LVLS`, untested) |

An empty slot returns empty text.

**Tune RT parameters LV1 exposes** (43 of the plug-in's 90, in LV1's own order):

| Index | Parameter | Values |
|---|---|---|
| 40 | Key | 0..16: C C# Db D D# Eb E F F# Gb G G# Ab A A# Bb B |
| 39 | Scale | 0 User Scale, 1 Chromatic, 2 Major, 3 Natural Minor, 4 Harmonic Minor, 5 Melodic Minor (Asc), 6 Melodic Minor (Asc+Dec), 7 Major Justonic, 8 Minor Justonic, 9 Dorian, 10 Mixolydian |
| 41 | Send to All | Toggle; every flip broadcasts this instance's key and scale |
| 42 | Receive | 1 = "Rcv", 0 = "Lock" (not saved in Tune presets) |

Params 6 and 26 follow the scale.

**Writing.** Touch, Control(value), Release, as a surface fader does. LV1 echoes the new
value and text.

## Behaviour

**MIDI to intent**, on the configured channel (1-16, or Omni):

| Message | Effect |
|---|---|
| Note On, velocity > 0 | Key = the note's pitch class. Octave and velocity are ignored. |
| CC *n* (the scale CC), value 0..10 | Scale = that Tune scale number. Values above 10 are ignored and logged. |
| CC 21, value 40..127 | LV1's tempo = the value in BPM. Below 40 (LV1's floor) is ignored and logged. |
| CC 22, value 0..127 | LV1's tempo = 100 + the value (faster songs: 28 = 128 BPM). A scale CC set to 21 or 22 wins. |
| MIDI clock (F8, FA, FB, FC), any channel | LV1's tempo follows it: see **Tempo** below. |
| Anything else | Ignored, including Note Off and messages on other channels. |

A note without a CC changes only the key. The Note Off that ends a cue's note is part of the
cue: it is neither a change nor logged as ignored.

A momentary controller button that sends CC 20 = 127 on press and 0 on release selects
**User Scale** (0) on release (seen with a WING user button on the rig): set the button to
send the scale's own number.

**Coalescing (builder's call).** Messages that arrive within 30 ms of each other become one
change. A clip that sends CC 20 and a note together gives one log line and one pass over the
Tunes.

**Spelling.** A MIDI note can't tell C# from Db, so each Tune keeps the spelling it already
shows:
- If its current key is a sharp key (C# D# F# G# A#), the new key is spelled with a sharp.
- If its current key is a flat key (Db Eb Gb Ab Bb), the new key is spelled with a flat.
- If its current key is natural (C D E F G A B), the window's default spelling decides.
  Sharps are the default.

**Applying.** On each change, find every Tune RT on every input channel, group and FX return.
This is a fresh scan, so instances added mid-show count. For each instance:
1. Check its layout: the slot code, the key text matching its value, the Receive text and the
   scale text. If any check fails, skip the instance and log "unexpected Tune layout".
2. Receive = Lock → skip it and count it as locked.
3. Set the key with that instance's spelling, then set the scale if the message carried one.
4. Read both back and log any mismatch.

The result is one log line, e.g. `A Natural Minor -> set 3 (Ch 43 Vox 1, Ch 44 Vox 2, Gp 1),
skipped 1 locked (Ch 45)`.

Tune's own Send to All is deliberately not used. Setting each instance directly lets us check
Lock ourselves and report exactly what changed.

**Tempo.** LV1 has one system tempo (Top Bar BPM, Setup > Mixer Settings > Tempo; scenes store
it). Tempo-based Waves plug-ins are in "tempo listen" by default and follow it. TuneSync sets
it with `SetMixerParam` of mixer param **4019** (`MixerSettingTempo`), a `WSMixerParamDouble`:
the 0x35-byte param header and the BPM at +0x35. LV1's own MyMon driver sets the tempo the same
way and refuses anything under 40 BPM. The id was decoded from MixerCoreSG 17.3.50's
`ToString(WEMixerParamType)`, the same table that gives the meter ids proven on the rig.
After setting, TuneSync reads the tempo back and logs `Tempo 128 BPM (was 127) (RTP Mac
clock)`, or a warning if LV1 did not take it. A tempo is applied by itself: no Tune scan.

MIDI clock (`crate::clock`):
- One source at a time. The first that ticks is followed until it has been quiet for 1 s, so a
  main and a backup rig both sending clock never count as one clock at double speed.
- Every half beat, the tick period is the least-squares slope over the last two beats (48
  ticks). Over RTP-MIDI it is timed on the sender's own stamps (the packet's RTP timestamp plus
  delta times, unwrapped per peer). A sender whose stamps disagree with arrival by more than
  20 % is timed on arrival instead.
- It is reported when two estimates in a row agree within 0.5 BPM, and only if it is new: the
  first since the clock started (Start, or ticks after a 1 s gap), or 0.5 BPM or more from the
  last one reported. Whole BPM when within 0.1 of one, else to 0.1. Jitter never nudges LV1,
  and a tempo set another way (CC 21/22, a hand on LV1) holds until the clock's tempo changes.
- Stop is ignored (many rigs keep ticking while stopped). Continue measures afresh. Outside
  40-300 BPM nothing is reported. Clock is never logged per tick and does not count as a cue.

LV1's own MIDI control-surface driver has an undocumented `SyncToMIDITempo` option; a Waves
forum user reported it drifting 4-5 BPM. TuneSync's follower is separate and does not use it.

**Tempo CCs (Flavius, 2026-11-23).** The value is the BPM as typed in the playback app, with
no sums: songs go below 60 BPM, so an offset was out. CC 21 covers 40-127 (LV1's floor is 40),
CC 22 adds 100 for 100-227. Prime (Loop Community) sends no MIDI clock, only note, CC and
program-change cues, so with Prime the tempo always comes from these CCs.

Proven on the rig 2026-11-23 (0.1.6 test build, whose CC 21 was then 60 + value): `CC 21 =
68` -> `[ok] Tempo 128 BPM (was 127)`, read back 28 ms after the CC. TuneSync only logs the tempo it sets: LV1's tempo
when TuneSync loads is its default 120, because the session's tempo loads later.

## Settings window

Opened from LV1's **Setup → Control Surfaces** settings button (WMSD_ShowConfigWindow). It
runs on its own thread with its own message loop and never on LV1's UI thread.

**Layout: one page, touch-first** (2026-10-26, `docs/settings-onepage.html`; it replaces the
tabs and collapsible-section options in `docs/settings-layout-options.html`). LV1 hosts are often
touchscreens:
- Every tap target is at least 44 px.
- No small dropdowns and no tabs.
- Nothing needed during a show sits below the fold.
- Details open as sheets inside the window, the way Headlink's dialogs do, never as separate
  windows.

The window is about 480 px wide. From top to bottom:

1. **Inputs:** two tiles, **MIDI in** (the port name) and **RTP-MIDI** (off / n peers), each
   with a dot that is green when MIDI arrived in the last few seconds.
   - Tapping MIDI in opens a sheet with the host's input ports as 48 px rows, plus "None". The
     port is chosen by name and reopened when it reappears, because Windows renumbers devices.
   - Tapping RTP-MIDI opens the RTP sheet (see RTP-MIDI below).
2. **Mapping:** three tiles.
   - **Channel** opens a sheet with a grid of 1-16 plus Omni. The default is 16 (builder's
     call: Omni would also react to a keyboard on the same port).
   - **Scale CC** opens a number sheet. The default is 20, an undefined CC.
   - **Spelling** is a ♯/♭ switch on the page itself.
3. **Last change:** a banner with the key and scale, the time, the source and the result
   ("set 2, 1 locked").
4. **Tunes:** one 44 px row per instance with its track, key, scale and a Rcv/Lock badge,
   plus a **Rescan** button. The list refreshes when the window opens and after each change.
5. **Apply now:** Key and Scale tiles (each opens a picker sheet) and a big Apply button. It
   checks the link at soundcheck without the tracks rig, and goes through the same apply path,
   so Locked Tunes are skipped.
   - The scale picker lists each scale with its CC value, so it is also the reference table.
   - The key picker has 12 large buttons.
6. **Log:** the last three lines, and **Full log** opens a sheet with everything (newest first,
   timestamped) plus Copy and Clear.
   - Ignored messages, such as a keyboard on another channel, get at most one summary line
     every 5 s.
   - The same lines go to `%ProgramData%\TuneSync\tunesync.log` (1 MB, one rotation).

### RTP-MIDI

- **The sheet:** an on/off switch (off by default), the session name (default "TuneSync"),
  the UDP port (5004), **Connected** peers and **Found on the network**.
- **Several peers at once.** Several peers can join the same session, as AppleMIDI allows:
  one port pair, with each participant told apart by its SSRC. For example, a main and a
  backup playback rig.
  - Each connected peer is listed with its name, its address under the name (an IPv6 one is
    long), when it was last heard and the network round trip the clock sync measured
    ("now · 4 ms"). A connected peer's dot breathes (2.4 s), and each MIDI message from it
    sends a ripple out of the dot, so a cue can be seen arriving from a given rig. The window
    repaints at ~30 fps only while such a dot is on screen (`Frame::animating`); otherwise it
    keeps its 250 ms model refresh. To time the ripple, the RTP-MIDI session runs on the
    host's `now_ms()` clock.
  - A message from any peer applies, and the log names the peer it came from.
  - When main and backup send the same cue together, the 30 ms coalescing makes it one change.
- **Auto-discovery, with no Bonjour install.** Windows 10 and 11 have DNS-SD built in
  (`DnsServiceRegister` / `DnsServiceBrowse` in dnsapi). It works both ways:
  - **Advertise:** while RTP is on, register `_apple-midi._udp` under the session name and
    port. "TuneSync" then appears in the Mac's Audio MIDI Setup → MIDI Network Setup →
    Directory by itself, and the Mac connects with one click.
  - **Browse:** list the Macs' own network sessions under "Found on the network", each with
    a Connect button. TuneSync then invites the Mac as the initiator, so the Mac only needs
    its session enabled. Connections made this way are remembered and re-invited when the Mac
    comes back. A remembered peer that is away is invited again every few seconds, so its
    log line is written once ("waiting - no answer from its control port; will keep inviting
    it"), and nothing more until it connects or the reason changes. Before 0.1.5 it added two
    lines every 17 s, all night, pushing the cues out of the window's log and the 1 MB file.
  - Typing the host IP and port on the Mac still works as a fallback.
  - **IPv6 as well as IPv4.** The host's DNS-SD records carry its IPv6 link-local address
    next to the IPv4 one (Windows publishes both for its host name, and so does macOS), and
    macOS's network MIDI connects to a Directory entry over IPv6. So both UDP sockets are
    dual stack (`[::]:5004` with IPV6_V6ONLY off, through `socket2`), falling back to IPv4
    only on a system without IPv6. IPv4 peers keep their plain address (`192.0.2.42`, not
    `::ffff:…`); a link-local IPv6 peer keeps its interface (`fe80::…%3`) so it can be answered.
  - If the Windows API misbehaves on a host, there are two other routes: preprobe's own mDNS
    code (`discover.*`, which browses with legacy-unicast queries) and a small responder of
    our own. Not tested yet on the LV1 hosts.
  - rtpMIDI (Tobias Erichsen) is why people expect to install Bonjour. We don't use it.

**Settings storage (builder's call).** `%ProgramData%\TuneSync\settings.ini`. LV1's own
preset blob (WMSD_GetPreset) only holds a version marker, because LV1 writes its preferences
file only when it closes cleanly.

## Proven on the rig (2026-11-23)

LV1 17.3.108.193 on the show PC (Windows 11 26200, 3840x2160 at 175 %), TuneSync 0.1.0
installed with its setup (surface added to LV1's preferences), session with Tune RT on
Ch 1-3 (slot 2):

| Check | Result |
|---|---|
| Load | Create verified MixerCoreSG's callback table; RTP-MIDI open on 5004/5005; MIDI in open; DNS-SD advertised ("TuneSync") through Windows' own dnsapi |
| RTP-MIDI cue (rtpsend on the host, loopback) | CC 20 = 4 + Note D: all three Tunes to D Harmonic Minor, 27 ms after the note, read back from LV1; then A Major |
| Lock | Ch 2 on Lock: "E Natural Minor: set 2 (Ch 1, Ch 3), skipped 1 locked (Ch 2)" |
| Local MIDI port | a WING user button over USB ("WING MIDI Control"): CC 20 values and a Note On (G#/Ab, sharps for a natural key) applied; CC 20 = 127 refused |
| LV1's UI thread | WM_NULL round trips every 10 ms for 15 s over two cues: median 1.7 ms, p99 14.9 ms, max 26 ms |
| Settings window | opens from Setup -> Control Surfaces (LV1 lists it under Controls), Direct2D text crisp at 175 %, live Tunes list with Rcv/Lock |
| Surface re-created | loading another session destroyed and re-created the surface: stop + start in 32 ms, UDP 5004 re-bound |

**A Mac over the network (2026-11-23, macOS 12.7.6, Audio MIDI Setup's network session, both
on Wi-Fi, RTT 2-8 ms):** the Mac listed "TuneSync" under Directory (DNS-SD advertising works)
and TuneSync listed the Mac under Found on the network (browsing works). TuneSync inviting the
Mac worked once the Mac's session accepted "Anyone" (refused before, retried every 5 s), and a
goodbye from the Mac was followed by TuneSync's re-invite 30 s later, as designed. The Mac
connecting to TuneSync from Directory timed out every time in 0.1.3: the Mac used the PC's
IPv6 link-local address (DNS-SD hands out both) and TuneSync listened on IPv4 only. Fixed in
0.1.4 (dual-stack sockets, above). The Mac shows latency 0 for a session TuneSync started
(TuneSync runs the clock sync then; its CK timestamps are per spec).

Not yet checked: the stereo Tune (`LVLS`).

## Structure

Rust (Flavius, 2026-11-22), one Cargo workspace. The product logic is a library and the DLL
LV1 loads is a thin shim over it, so Headlink can reuse the library (through a C API) and
the Mac version only adds a shell.

| Crate | Contents |
|---|---|
| `crates/tunesync` | The library, with no OS or LV1 in it: Tune tables and spelling, the MIDI-to-intent mapper and coalescer, discovery and apply against a `Mixer` trait, the engine, settings (ini), the log ring. Tested on Linux with a fake mixer. |
| `crates/rtpmidi` | RTP-MIDI. The codec is a port of Headlink's `tools/preprobe/rtpmidi.*` (`demo/pre-probe`), same test vectors. preprobe's session talks to a single peer, so it is replaced by one that keeps a table of participants keyed by SSRC: invitation on the control port then the data port, CK answered or run, RS feedback, BY and a silence timeout, all per peer, in both roles (they invite us, or we invite a Mac). Tested with raw-byte fake peers on loopback. `examples/rtpsend` is a stand-in playback rig. |
| `crates/lv1-surface` | LV1's surface-driver interface (WMSD, interface 6) in Rust: the packed structs from Headlink's `tools/surfaceprobe/wmsd_abi.h`, a macro that generates the 18 exports, the callback-table check, and calls into LV1 behind a crash guard (a small C shim, because Rust has no `__try`). |
| `crates/host` | TuneSync running: the engine's worker (the only caller into the `Mixer`), local MIDI by name (WinMM), the RTP-MIDI service with DNS-SD (Windows dnsapi, looked up at run time), `settings.ini` and `tunesync.log`. Tested on Linux end to end with a fake LV1 and a real RTP-MIDI rig on loopback. |
| `crates/ui` | The settings window as a portable view model: layout, tap regions, taps and drags to commands. `examples/preview` renders every state as SVG. |
| `crates/capi` | The runtime as a C library for a C/C++ host (Headlink): `tunesync_capi` as staticlib + cdylib, `include/tunesync.h` generated by cbindgen. See [C API](#c-api-headlink). |
| `crates/control` | `TuneSync_Control.dll`: the shim. A `Mixer` on LV1's Get/SetMixerControl (Touch, Control, wait for LV1's asynchronous set to show, Release), one TuneSync per LV1, and the Direct2D window painting the ui frames on its own thread. |
| `crates/sim` | A fake LV1: `MixerCoreSG.dll` (the seven callbacks over three Tunes; sets land 1 ms later, as LV1's do) and `lv1sim.exe`, which loads the DLL like LV1 does and drives it end to end. `tools/wine/run-sim.sh` runs it under Wine. |
| `installer/` | Inno Setup script (`tools/make-installer.sh`) and `lv1-prefs.ps1`, which adds or removes the `<SurfaceDriver>` entry. |

**Build and test**
- `tools/cargo.sh test --workspace`: every crate's tests on Linux.
- `tools/build-dll.sh`: `dist/TuneSync_Control.dll` (+ the fake LV1).
- `tools/wine/run-sim.sh [--window 6 --shot <dir>]`: the DLL under Wine against the fake LV1
  (exports, Create, a rig over RTP-MIDI, Lock, Touch/Release pairs, Destroy; optionally the
  window, captured through GDI). Wine 8 lacks `bcryptprimitives.dll` (Rust's std needs
  `ProcessPrng`), so the script puts a test-only stub beside the exe. Wine's DirectWrite draws
  no text without Segoe UI: check text on Windows.
- `tools/make-installer.sh [--sign]`: `dist/TuneSync-<version>-setup.exe`.
- `tools/capi-header.sh [--check]`: regenerate (or check) `crates/capi/include/tunesync.h`.

`tools/cargo.sh` runs cargo in the `tunesync-build:1` image
(`tools/docker/Dockerfile`: Rust with the `x86_64-pc-windows-msvc` target, on a base with
clang-cl, lld-link and Microsoft's CRT/SDK fetched by xwin, `tools/docker/xwin.Dockerfile`).

**Install.** Copy `TuneSync_Control.dll` into `C:\Program Files (x86)\Waves\eMotion LV1\Modules`
(this needs elevation). Then either add the surface in Setup → Control Surfaces, or have the
installer write the `<SurfaceDriver>` preferences entry while LV1 is closed. The DLL is signed
the same way as Headlink's, or Smart App Control blocks it.

## C API (Headlink)

`crates/capi` puts the whole `tunesync-host` runtime behind a C interface, so a C++ host
embeds TuneSync the way the LV1 DLL does: `tunesync_capi.lib` (static) or
`tunesync_capi.dll`, and `crates/capi/include/tunesync.h`, generated from
`crates/capi/src/lib.rs` by cbindgen 0.27.0 (`tools/capi-header.sh`; `--check` fails
when the committed header is stale) and committed.

**What runs** (Flavius, 2026-11-23: "whole runtime, opt-out"): by default everything the
LV1 DLL runs: the worker, TuneSync's own MIDI input by name, RTP-MIDI with DNS-SD,
`settings.ini` and `tunesync.log` in the data folder. `TS_START_NO_MIDI_IN` and
`TS_START_NO_RTP` leave those inputs to the host; it feeds MIDI from its own inputs with
`ts_feed_midi` (which works in either mode).

**The mixer** is a vtable the host fills, `TsMixer`: `ctx` plus
- `get(ctx, track_type, track_index, slot, channel, cid, double *value, char *text, size_t cap)`,
- `set(ctx, track_type, track_index, slot, param, double value)`,
- `track_name(ctx, track_type, track_index, char *out, size_t cap) -> length`.

`get`/`set` return 0 when LV1 answered, so LV1's own result passes straight through. `set`
does what `crates/control/src/mixer.rs` does: Touch, Control, wait (bounded) until a read
shows the value, Release, because TuneSync reads every change back.

**Threads.** The vtable is called only from the runtime's worker thread, one call at a
time; never from a thread that calls `ts_*`, never from the host's own threads. Each call
holds a gate that `ts_stop` closes after the worker ended (or its 1.5 s timed out), waiting
for a call in flight, so once `ts_stop` returns the mixer is never entered again and `ctx`
may be freed. `ts_stop` from inside a callback returns `TS_ERR_IN_CALLBACK` and does nothing.
Every other `ts_*` only queues work and may be called from any thread. Every exported
function runs under `catch_unwind`: a panic becomes `TS_ERR_PANIC` or NULL, never an unwind
into C.

**Functions.** `ts_start(data_dir, title, &mixer, flags)` → `TsRuntime *` (NULL on a bad
argument); `ts_stop`; `ts_feed_midi(rt, source, bytes, len)`; `ts_apply_now(rt, pitch_class,
scale)` (-1 = leave); `ts_rescan`; `ts_settings_get` / `ts_settings_set` (settings.ini
text; set replaces, saves and applies, returns how many lines it skipped);
`ts_status_json`, `ts_tunes_json`; `ts_log_json(rt, after)` (lines after a generation, and
`reset` when the reader's copy is stale); `ts_rtp_connect` / `ts_rtp_forget`;
`ts_string_free` for every returned string; `ts_abi_version` (`TS_ABI_VERSION`) and
`ts_version`.

**Linking the static library.** Linux: `-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc`.
Windows (MSVC, `/MT`): `kernel32 ntdll userenv ws2_32 dbghelp synchronization` and
`windows.0.52.0.lib`, which ships in the `windows_x86_64_msvc` 0.52 crate's `lib/` folder.
`rustc --print native-static-libs` gives the current list. The DLL needs none of these. A
clang-cl/lld-link program linked this way ran under Wine (start, log, stop; 2026-11-23).

**Tests.** `crates/capi/tests/c_program.rs` builds `tests/c/capi_test.c` with clang
(`-std=c11 -Wall -Wextra -Werror -pedantic`) against the header and the staticlib, and runs
it: a fake LV1 vtable with the two Tunes of `crates/host/tests/runtime.rs`; a cue through
`ts_feed_midi` sets the Rcv one and skips the locked one; Apply now; settings replaced,
saved and applied; log by generation; `ts_stop` refused from a callback; no mixer call after
`ts_stop`, every call on one thread that is not the caller's; then engine-only mode. The
header is also compiled as C++17.

## Open (Flavius)
- One-page or KISS settings window (`docs/settings-kiss.html`).
