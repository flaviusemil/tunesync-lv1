<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/tunesync-logo.svg">
    <img src="assets/logo/tunesync-logo-light.svg" width="440" alt="TuneSync">
  </picture>
</p>

<p align="center">
  <strong>Change the key of every Waves Tune Real-Time in eMotion LV1 with one MIDI note.</strong>
</p>

<p align="center">
  <a href="#license"><img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-3fb37f"></a>
  <img alt="Platform: Windows 10/11 x64" src="https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-1f2228">
  <img alt="Tested with eMotion LV1 17.3" src="https://img.shields.io/badge/eMotion%20LV1-17.3%20tested-1f2228">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-1f2228">
</p>

---

In a live show the playback rig knows which song is playing; the pitch correction on the vocals
does not. TuneSync closes that gap. The tracks session sends one MIDI message at the top of each
song, and every instance of Waves Tune Real-Time in eMotion LV1 follows: same key, and optionally
the same scale. The key lives with the song in the tracks session, so reordering the set can
never leave the vocal tuning behind.

TuneSync runs inside LV1 as a control-surface module. It needs no extra application, no
snapshots and no Bonjour installation.

<p align="center">
  <img src="docs/images/window.png" width="420" alt="The TuneSync settings window">
</p>

## Features

- **One note, every Tune.** A Note On sets the key of every Tune RT on every input channel,
  group and FX return, including instances added during the show.
- **Scale by CC.** A control change (CC 20 by default) selects any of Tune's 11 scales.
- **Respects Lock.** Instances switched to *Lock* on Tune's own Receive button are never touched.
- **Keeps the spelling.** A Tune showing E♭ stays a flat key; C♯ stays a sharp key.
- **Local or network MIDI.** Any MIDI input on the LV1 computer, opened by name and reopened
  when it comes back, and/or RTP-MIDI (AppleMIDI) with several rigs in one session, for
  example a main and a backup playback machine.
- **Finds Macs by itself.** TuneSync advertises itself through Windows' built-in DNS-SD, so it
  appears in macOS *Audio MIDI Setup → Network* without any setup on either side.
- **Verified changes.** Every change is read back from LV1 and reported in one log line, for
  example `A Natural Minor: set 2 (Ch 43 Vox 1, Ch 44 Vox 2), skipped 1 locked (Ch 45)`.
- **Never in LV1's way.** LV1's own threads never wait on MIDI, the network or the window.
  Measured on a show system during cues: LV1's UI thread stayed at a median of 1.7 ms.
- **Touch-first settings.** One page with large targets, built for the touchscreens LV1 hosts
  often use.

## How it works

```mermaid
flowchart LR
    rig["Playback rig<br/>(tracks session)"] -- "Note On + CC 20<br/>MIDI or RTP-MIDI" --> ts["TuneSync<br/>inside eMotion LV1"]
    ts -- "key + scale" --> a["Tune RT · Ch 43 · Rcv"]
    ts -- "key + scale" --> b["Tune RT · Ch 44 · Rcv"]
    ts -. "left alone" .-> c["Tune RT · Ch 45 · Lock"]
```

Messages that arrive within 30 ms of each other are applied as one change, so a clip that sends
the scale CC and the key note together, or two rigs sending the same cue, produces a single pass
over the Tunes.

## Requirements

- Windows 10 or 11, 64-bit
- Waves eMotion LV1 (tested with 17.3.108; the control-surface interface is the same in 16.5)
- Waves Tune Real-Time in the session
- For network MIDI from a Mac: the Mac's network session (Audio MIDI Setup)

## Installation

1. Close eMotion LV1.
2. Run `TuneSync-<version>-setup.exe` as administrator. Keep *Add TuneSync to eMotion LV1's
   control surfaces* selected.
3. Start LV1. TuneSync appears under **Setup → Control Surfaces**; its settings button opens
   the TuneSync window.

The setup installs `TuneSync_Control.dll` into LV1's `Modules` folder. Settings and the log are
kept in `%ProgramData%\TuneSync`. Uninstall from Windows Settings → Apps, with LV1 closed.

## Usage

Send these on the configured MIDI channel (16 by default) at the top of each song:

| Message | Effect |
|---|---|
| Note On, any octave, velocity above 0 | Key = the note's pitch class |
| CC 20, value 0–10 | Scale (table below); other values are ignored and logged |

| Value | Scale | Value | Scale |
|---|---|---|---|
| 1 | Chromatic | 6 | Melodic Minor (Asc+Dec) |
| 2 | Major | 7 | Major Justonic |
| 3 | Natural Minor | 8 | Minor Justonic |
| 4 | Harmonic Minor | 9 | Dorian |
| 5 | Melodic Minor (Asc) | 10 | Mixolydian |
| 0 | User Scale | | |

A note without a CC changes only the key. Both the channel and the CC number can be changed in
the settings window, which also offers **Apply now** for checking the setup at soundcheck.

> **Tip:** controller buttons that send CC 20 = 127 when pressed and 0 when released select
> *User Scale* on release. Configure the button to send the scale's own number instead.

### Connecting a Mac over the network

1. In the TuneSync window, open **RTP-MIDI** and switch the network session on.
2. On the Mac, open **Audio MIDI Setup → Window → Show MIDI Studio**, then the **Network** icon.
3. *TuneSync* is listed under **Directory**. Select it and click **Connect**. Use the Mac's
   session as the MIDI output of the tracks session.

TuneSync can also invite a Mac itself: Macs that publish a network session are listed in the
RTP-MIDI sheet under *Found on the network*. Typing the PC's address and port 5004 on the Mac
works as well.

<p align="center">
  <img src="docs/images/window-rtp.png" width="420" alt="The RTP-MIDI sheet">
</p>

## Building from source

Everything builds on Linux in Docker; nothing else needs to be installed.

```sh
# Once: a base image with LLVM and Microsoft's CRT and Windows SDK (fetched by xwin).
# Building it accepts Microsoft's licence for those files; read it first.
docker build -t tunesync-xwin:1 -f tools/docker/xwin.Dockerfile tools/docker
docker build -t tunesync-build:1 tools/docker

tools/cargo.sh test --workspace     # all tests
tools/build-dll.sh                  # dist/TuneSync_Control.dll
tools/wine/run-sim.sh               # the DLL against a simulated LV1, under Wine
tools/make-installer.sh             # dist/TuneSync-<version>-setup.exe
```

`tools/wine/run-sim.sh` loads the DLL the way LV1 does, against a fake mixer core with three
Tune instances, and drives it end to end: interface checks, an RTP-MIDI cue, Lock, and teardown.

### Project layout

| Crate | Purpose |
|---|---|
| [`crates/tunesync`](crates/tunesync) | The library: Tune tables and spelling, MIDI interpretation, discovery and apply against a `Mixer` trait, settings, log. No LV1 or OS code, so it can be embedded elsewhere. |
| [`crates/rtpmidi`](crates/rtpmidi) | RTP-MIDI / AppleMIDI: codec and a session several peers can join. |
| [`crates/host`](crates/host) | The runtime: worker threads, MIDI input by name, the RTP-MIDI service and DNS-SD, settings and log files. |
| [`crates/lv1-surface`](crates/lv1-surface) | eMotion LV1's control-surface driver interface in Rust: the ABI, the exported entry points and guarded calls into LV1. |
| [`crates/ui`](crates/ui) | The settings window as a portable view model, with an SVG renderer for previews and tests. |
| [`crates/control`](crates/control) | `TuneSync_Control.dll`, the thin module LV1 loads, with the Direct2D window. |
| [`crates/sim`](crates/sim) | A simulated LV1 for testing the DLL without LV1. |

The design and its decisions are documented in [docs/DESIGN.md](docs/DESIGN.md).

## Status

TuneSync 0.1 has been tested on a live LV1 system (LV1 17.3.108, Windows 11): RTP-MIDI and
local MIDI cues, Lock, the settings window and LV1's responsiveness. Still to be verified:
a Mac joining over a real network, and the stereo Tune instance.

Planned: a macOS version, and a C API so the library can be embedded in other hosts.

## License

TuneSync is licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option. Unless you explicitly state otherwise, any contribution you submit for inclusion
in TuneSync, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.

TuneSync is an independent project. It is not affiliated with, endorsed by or supported by Waves
Audio Ltd. eMotion LV1, Tune Real-Time and SoundGrid are trademarks of their respective owners.
TuneSync communicates with LV1 through its control-surface driver interface, which was studied
for interoperability.
