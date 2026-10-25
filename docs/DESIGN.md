# Tune Remote: design

*Tune Remote is a working name.* It is an eMotion LV1 control module that sets the key and
scale of every Waves Tune Real-Time instance from MIDI. It is its own product with its own
DLL, settings and installer. It is not part of Headlink and does not need Headlink installed.

## Why

In a show, the backing tracks know which song is playing, but Tune doesn't. Today the only
way to change Tune's key in LV1 is a snapshot or a hand on the plug-in. With Tune Remote, the
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
- macOS. Windows comes first; LV1's surface-driver ABI is the same on Mac, so it can follow.

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
runs on its own thread with its own message loop and never on LV1's UI thread. Mockup:
`docs/settings-mockup.html`.

**Layout: tabs** (chosen 2026-10-26 over collapsible sections; `docs/settings-layout-options.html`).
The window is about 470 px wide. The status strip sits above the tabs and is always visible.
The content of each tab is in `docs/settings-mockup.html`, which shows all the sections on one
page.

- **Status strip (always visible).** The input is listening (port name, or the RTP peer's
  name), the number of Tunes found and locked, and the last change with its time. Each
  input's dot is green when MIDI arrived in the last few seconds.
- **Tunes** (the tab the window opens on):
  - Every instance found, with its track name, slot, current key and scale, and Receive or
    Lock. Refreshed when the window opens and after each change.
  - **Apply now:** key and scale dropdowns, so the link can be checked at soundcheck without
    the tracks rig. It goes through the same apply path, so Locked Tunes are skipped.
    (Builder's suggestion.)
- **Input:**
  - MIDI port: a list of the host's input ports, chosen by name. The list includes "None".
    The port is reopened when it reappears, because Windows renumbers devices.
  - RTP-MIDI: off by default. When on, it shows the session name (default "Tune Remote"), UDP
    port 5004, connected peers, and a hint telling the Mac side what to add in Audio MIDI
    Setup → Network (this host's IP and port).
- **Mapping:**
  - Channel: 1-16 or Omni. The default is 16 (builder's call: Omni would also react to a
    keyboard on the same port).
  - Scale CC number: default 20, an undefined CC.
  - Default spelling: Sharps or Flats.
  - A read-only reference: notes → keys, and CC values 0..10 → scale names.
- **Log:**
  - The newest lines first, with a timestamp on each: every message received, what it did and
    every skip.
  - Ignored messages, such as a keyboard on another channel, get at most one summary line
    every 5 s.
  - **Copy** and **Clear** buttons.
  - The same lines go to `%ProgramData%\Tune Remote\tune-remote.log` (1 MB, one rotation).

**Settings storage (builder's call).** `%ProgramData%\Tune Remote\settings.ini`. LV1's own
preset blob (WMSD_GetPreset) only holds a version marker, because LV1 writes its preferences
file only when it closes cleanly.

## Structure

| Path | Contents |
|---|---|
| `core/` | Portable C++17 with no Windows or LV1 headers: Tune tables and spelling, the MIDI-to-intent mapper, discovery and apply against an abstract `Mixer`, settings (ini), the log ring. Unit-tested on Linux (`make test`) with a fake mixer. |
| `win/` | (next) The WMSD exports (ABI from Headlink's `tools/surfaceprobe/wmsd_abi.h`), the `Mixer` implemented on LV1's Get/SetMixerControl callbacks, local MIDI, the settings window and the worker threads. |
| `net/` | (next) RTP-MIDI. A copy of Headlink's `tools/preprobe/rtpmidi.*` (responder mode) and `midiport.*` (WinMM by name), from the `demo/pre-probe` branch. Copied, not linked, so the two products stay independent. |

**Install.** Copy `TuneRemote_Control.dll` into `C:\Program Files (x86)\Waves\eMotion LV1\Modules`
(this needs elevation). Then either add the surface in Setup → Control Surfaces, or have the
installer write the `<SurfaceDriver>` preferences entry while LV1 is closed. The DLL is signed
the same way as Headlink's, or Smart App Control blocks it.

## Open (Flavius)
- Product name, and whether it is free or paid.
- Mac version: when.
