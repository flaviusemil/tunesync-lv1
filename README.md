# TuneSync

An eMotion LV1 control module. The tracks rig sends a MIDI note at the top of each song, and
every Waves Tune Real-Time instance in LV1 switches to that key. A CC can set the scale too.
Tunes switched to Lock are left alone. MIDI comes from a local MIDI port or over the network
(RTP-MIDI: a Mac's network session finds TuneSync by itself, no Bonjour install needed).

TuneSync is separate from Headlink. It has its own DLL, settings and installer.

- Design: [docs/DESIGN.md](docs/DESIGN.md). The settings window is in
  [docs/settings-onepage.html](docs/settings-onepage.html): one page, touch-first.
- Code: a Rust workspace. `crates/tunesync` is the library (no LV1 in it), `crates/rtpmidi`
  the network MIDI, and the DLL LV1 loads is a thin shim over them (in progress).
- Build and test (Docker, nothing else to install):

  ```
  docker build -t tunesync-xwin:1 -f tools/docker/xwin.Dockerfile tools/docker   # once: accepts Microsoft's SDK licence, read it first
  docker build -t tunesync-build:1 tools/docker
  tools/cargo.sh test --workspace
  ```

- A stand-in playback rig for tests: `tools/cargo.sh run -p rtpmidi --example rtpsend -- <host> 5004 key A cc 20 3`.
