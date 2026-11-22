//! RTP-MIDI for TuneSync: [`codec`] builds and parses AppleMIDI / RTP-MIDI datagrams,
//! [`session`] is one network session that several peers (a main and a backup playback
//! rig, a Mac we invite) join at once.
pub mod codec;
pub mod session;
