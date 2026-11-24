TuneSync @VERSION@ for eMotion LV1 on macOS
===========================================

A MIDI note from the playback rig sets the key of every Waves Tune Real-Time instance
in LV1 that is on Rcv (Lock is skipped); CC 20 sets the scale.
https://github.com/flaviusemil/tunesync-lv1

Install (quit eMotion LV1 first)
  1. Open Terminal in this folder (Finder: right-click the folder > New Terminal at Folder).
  2. Run:   sudo ./install.sh
     It copies TuneSync_Control.bundle into /Applications/Waves/eMotion LV1/Modules,
     makes /Library/Application Support/TuneSync for the settings and the log, and adds
     TuneSync to LV1's control surfaces (LV1's preferences are backed up first).
  3. Start LV1. TuneSync's settings open from Setup > Control Surfaces.

  sudo ./install.sh --no-surface   install without touching LV1's preferences; then add
                                   TuneSync yourself in Setup > Control Surfaces
  sudo ./install.sh --remove       take it out again (settings and log are kept)

MIDI in
  Any CoreMIDI source, chosen by name: a USB interface, a console's USB MIDI, the IAC
  driver, or the Mac's own network session (Audio MIDI Setup > MIDI Studio > Network,
  "Network Session 1"). TuneSync's own RTP-MIDI session (off by default) also works; when
  the Mac's network session already holds UDP 5004, TuneSync moves up to the next free pair
  (5006, 5008, ...) by itself, and its RTP-MIDI sheet shows the port it got.

The bundle is universal (Intel and Apple silicon) and ad-hoc signed, not notarized.
Not affiliated with or endorsed by Waves.
