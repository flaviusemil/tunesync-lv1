# TuneSync

An eMotion LV1 control module. The tracks rig sends a MIDI note at the top of each song, and
every Waves Tune Real-Time instance in LV1 switches to that key. A CC can set the scale too.
Tunes switched to Lock are left alone. MIDI comes from a local MIDI port or over the network
(RTP-MIDI: a Mac's network session finds TuneSync by itself, no Bonjour install needed).

TuneSync is separate from Headlink. It has its own DLL, settings and installer.

## Use it

1. Close eMotion LV1 and run `TuneSync-<version>-setup.exe` (Windows 10/11 x64, as
   administrator). Leave "Add TuneSync to eMotion LV1's control surfaces" ticked.
2. Start LV1. Setup → Control Surfaces lists TuneSync; its settings button opens the
   TuneSync window: pick the MIDI input and/or turn on RTP-MIDI (network MIDI).
3. In the tracks session, at the top of each song, send on MIDI channel 16 (changeable):

   | Message | Effect |
   |---|---|
   | Note On (any octave, velocity > 0) | every Tune RT on **Rcv** switches to that key |
   | CC 20 (changeable), value 0-10 | the scale: 1 Chromatic, 2 Major, 3 Natural Minor, 4 Harmonic Minor, 5 Melodic Minor (Asc), 6 Melodic Minor (Asc+Dec), 7 Major Justonic, 8 Minor Justonic, 9 Dorian, 10 Mixolydian, 0 User Scale |

   Tunes on **Lock** are left alone, and each Tune keeps its own spelling (E♭ stays a flat key).
4. From a Mac: Audio MIDI Setup → MIDI Studio → Network. "TuneSync" appears under Directory by
   itself (no Bonjour install on the PC); select it and Connect. A main and a backup rig can
   both be connected.

Settings and the log live in `%ProgramData%\TuneSync` (`settings.ini`, `tunesync.log`).

- Design: [docs/DESIGN.md](docs/DESIGN.md). The settings window is in
  [docs/settings-onepage.html](docs/settings-onepage.html): one page, touch-first.
- Code: a Rust workspace. `crates/tunesync` is the library (no LV1 in it), `crates/rtpmidi`
  the network MIDI, `crates/host` runs them, `crates/lv1-surface` is LV1's surface-driver
  interface, and `crates/control` (the DLL LV1 loads) is a thin shim over them.
- Build and test (Docker, nothing else to install):

  ```
  docker build -t tunesync-xwin:1 -f tools/docker/xwin.Dockerfile tools/docker   # once: accepts Microsoft's SDK licence, read it first
  docker build -t tunesync-build:1 tools/docker
  tools/cargo.sh test --workspace
  ```

- The DLL, the fake LV1 under Wine, the installer:

  ```
  tools/build-dll.sh
  tools/wine/run-sim.sh
  tools/make-installer.sh
  ```

- A stand-in playback rig for tests: `tools/cargo.sh run -p rtpmidi --example rtpsend -- <host> 5004 key A cc 20 3`.

## Licence

TuneSync is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise, any
contribution you submit for inclusion in TuneSync, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

TuneSync talks to eMotion LV1 through LV1's control-surface driver interface, which was
reverse-engineered for interoperability. It is not affiliated with or endorsed by Waves.
