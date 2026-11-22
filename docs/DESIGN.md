# TuneSync: design

TuneSync is an eMotion LV1 control module that sets the key and scale of every Waves Tune
Real-Time instance from MIDI. It is its own product with its own DLL, settings and installer.
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

**Non-goals (v1)**
- Following MIDI timecode, song lists or program-change presets on the LV1 side. The tracks
  session holds the key.
- Any other plug-in or parameter.
- macOS in the first release. Windows comes first; LV1's surface-driver ABI is the same on Mac,
  and a Mac version follows (Flavius, 2026-11-22: "This should work with mac too").

**Anti-goals** (failures even if everything else works)
- Touching anything except a Tune's key and scale.
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
| Anything else | Ignored, including Note Off and messages on other channels. |

A note without a CC changes only the key.

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
  - Each connected peer is listed with its name, address and when it was last heard.
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
    comes back.
  - Typing the host IP and port on the Mac still works as a fallback.
  - If the Windows API misbehaves on a host, there are two other routes: preprobe's own mDNS
    code (`discover.*`, which browses with legacy-unicast queries) and a small responder of
    our own. Not tested yet on the LV1 hosts.
  - rtpMIDI (Tobias Erichsen) is why people expect to install Bonjour. We don't use it.

**Settings storage (builder's call).** `%ProgramData%\TuneSync\settings.ini`. LV1's own
preset blob (WMSD_GetPreset) only holds a version marker, because LV1 writes its preferences
file only when it closes cleanly.

## Structure

Rust (Flavius, 2026-11-22), one Cargo workspace. The product logic is a library and the DLL
LV1 loads is a thin shim over it, so Headlink can reuse the library (through a C API) and
the Mac version only adds a shell.

| Crate | Contents |
|---|---|
| `crates/tunesync` | The library, with no OS or LV1 in it: Tune tables and spelling, the MIDI-to-intent mapper and coalescer, discovery and apply against a `Mixer` trait, the engine, settings (ini), the log ring. Tested on Linux with a fake mixer. |
| `crates/rtpmidi` | RTP-MIDI. The codec is a port of Headlink's `tools/preprobe/rtpmidi.*` (`demo/pre-probe`), same test vectors. preprobe's session talks to a single peer, so it is replaced by one that keeps a table of participants keyed by SSRC: invitation on the control port then the data port, CK answered or run, RS feedback, BY and a silence timeout, all per peer, in both roles (they invite us, or we invite a Mac). Tested with raw-byte fake peers on loopback. `examples/rtpsend` is a stand-in playback rig. |
| `crates/lv1-surface` | (next) LV1's surface-driver interface (WMSD, interface 6) in Rust: the packed structs from Headlink's `tools/surfaceprobe/wmsd_abi.h`, a macro that generates the 18 exports, the callback-table check, and calls into LV1 behind a crash guard (a small C shim, because Rust has no `__try`). |
| `crates/control` | (next) `TuneSync_Control.dll`: the shim. A `Mixer` on LV1's Get/SetMixerControl, local MIDI by name, DNS-SD, the settings window and the worker threads, started from the surface's Create. |
| `crates/sim` | (next) A fake LV1 (`MixerCoreSG.dll` with the seven callbacks, plus a host that loads the DLL like LV1 does), to run the DLL under Wine before the rig. |

Build and test: `tools/cargo.sh test --workspace` runs cargo in the `tunesync-build:1` image
(`tools/docker/Dockerfile`: Rust with the `x86_64-pc-windows-msvc` target, on a base with
clang-cl, lld-link and Microsoft's CRT/SDK fetched by xwin, `tools/docker/xwin.Dockerfile`).

**Install.** Copy `TuneSync_Control.dll` into `C:\Program Files (x86)\Waves\eMotion LV1\Modules`
(this needs elevation). Then either add the surface in Setup → Control Surfaces, or have the
installer write the `<SurfaceDriver>` preferences entry while LV1 is closed. The DLL is signed
the same way as Headlink's, or Smart App Control blocks it.

## Open (Flavius)
- The licence (asked 2026-11-22: MIT, or MIT OR Apache-2.0).
- One-page or KISS settings window (`docs/settings-kiss.html`).
