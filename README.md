# Tune Remote (working name)

An eMotion LV1 control module. The tracks rig sends a MIDI note at the top of each song, and
every Waves Tune Real-Time instance in LV1 switches to that key. A CC can set the scale too.
Tunes switched to Lock are left alone.

Tune Remote is separate from Headlink. It has its own DLL, settings and installer.

- Design: [docs/DESIGN.md](docs/DESIGN.md). The settings window is in
  [docs/settings-mockup.html](docs/settings-mockup.html), laid out as tabs.
- Portable core with tests: `core/`, `test/`. Run `make test`, or `make asan` to run them
  under sanitizers.
- Windows shell (WMSD exports, MIDI, RTP-MIDI, settings window): `win/` and `net/`, which are
  next.
