// rtp_codec.h - RTP-MIDI (RFC 6295) and Apple's session protocol ("AppleMIDI"):
// build and parse datagrams. No sockets, no clock.
//
// Copied from Headlink's tools/preprobe/rtpmidi.h (branch demo/pre-probe, 2026-10-20),
// codec only and unchanged, so Tune Remote and Headlink stay independent products.
// net/rtp_session.h is the transport.
//
// WIRE SUMMARY (all integers big-endian)
//   Session packets start with FF FF and a two-letter command:
//     IN/OK/NO  FF FF cc cc | version=2 (4) | initiator token (4) | SSRC (4) | name\0
//     BY        FF FF 42 59 | version=2 (4) | initiator token (4) | SSRC (4)
//     CK        FF FF 43 4B | SSRC (4) | count (1) | pad (3) | ts1 ts2 ts3 (8 each)
//     RS        FF FF 52 53 | SSRC (4) | extended sequence number (4): the last
//               RTP sequence received, wrap count in the top 16 bits (Apple; rtpmidid
//               learned this the hard way - reading 16 bits there always gives 0)
//   The initiator invites the control port (normally 5004), then the data port
//   (control + 1) from its own data socket. Clock sync (CK) and MIDI run on the
//   data port; receiver feedback (RS) and BY go to the control port. Timestamps
//   count 100 us units (a 10 kHz clock).
//
//   MIDI packets are RTP: 80 61 | seq (2) | timestamp (4) | SSRC (4), then the
//   MIDI command section: one header byte B J Z P LEN(4) (B = 1: LEN is 12 bits
//   and a second byte follows), LEN bytes of MIDI list, then the recovery
//   journal when J = 1. In the list every command after the first is preceded
//   by a delta time (1-4 bytes, 7 bits each, high bit = more); the first one too
//   when Z = 1. Running status is allowed after the first command.
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <string>
#include <vector>

namespace rtpmidi {

// ---------------------------------------------------------------- codec

// Not IN/OUT: windef.h #defines IN (and OPTIONAL, OUT) to nothing.
enum class AmCmd { None, In, Ok, No, By, Ck, Rs, Rl, Unknown };
const char* am_name(AmCmd c);

struct AmPacket {
    AmCmd       cmd = AmCmd::None;
    char        raw[3] = {0, 0, 0};   // the two command letters as received
    uint32_t    version = 0;          // IN/OK/NO/BY
    uint32_t    token = 0;            // IN/OK/NO/BY: the initiator's token
    uint32_t    ssrc = 0;
    std::string name;                 // IN/OK/NO: the sender's session name
    uint8_t     count = 0;            // CK: 0, 1, 2
    uint64_t    ts[3] = {0, 0, 0};    // CK
    uint32_t    seq = 0;              // RS: extended sequence number
    uint32_t    limit = 0;            // RL
};

// True when the datagram is a session packet (starts FF FF), whatever its command.
bool am_is_session(const uint8_t* p, size_t n);
// False for anything too short for its command. A command we do not know is
// parsed as Unknown (letters in raw) rather than refused, so the log shows it.
bool am_parse(const uint8_t* p, size_t n, AmPacket* out);

std::vector<uint8_t> am_invite(AmCmd cmd, uint32_t token, uint32_t ssrc, const std::string& name);  // IN/OK/NO
std::vector<uint8_t> am_bye(uint32_t token, uint32_t ssrc);
std::vector<uint8_t> am_clock(uint32_t ssrc, uint8_t count, uint64_t ts1, uint64_t ts2, uint64_t ts3);
std::vector<uint8_t> am_feedback(uint32_t ssrc, uint32_t extSeq);

// One MIDI message from a command section, with running status expanded: bytes
// always starts with a status byte.
struct MidiEvent {
    uint32_t             delta = 0;          // in the session's 100 us units
    bool                 running = false;    // the status byte was not on the wire
    std::vector<uint8_t> bytes;
};

struct RtpMidiPacket {
    uint16_t seq = 0;
    uint32_t ts = 0;
    uint32_t ssrc = 0;
    bool     b = false, j = false, z = false, p = false;   // command-section flags
    size_t   listLen = 0;        // LEN: bytes of MIDI list
    size_t   journalLen = 0;     // bytes after the list (the recovery journal), not decoded
    std::vector<MidiEvent> events;
};

bool rtp_is_midi(const uint8_t* p, size_t n);   // RTP version 2, payload type 0x61
// False (and why in *err) for a packet that is not RTP-MIDI or whose MIDI list
// does not decode. Events decoded before the fault are kept in out->events.
bool rtp_parse(const uint8_t* p, size_t n, RtpMidiPacket* out, std::string* err);
// A packet carrying msgs back to back (delta 0), no journal. Each msg must
// start with a status byte; nothing is merged into running status.
std::vector<uint8_t> rtp_build(uint16_t seq, uint32_t ts, uint32_t ssrc,
                               const std::vector<std::vector<uint8_t>>& msgs);

// How many data bytes follow a status byte: -1 for SysEx (variable), 0..2 otherwise.
int midi_data_len(uint8_t status);

std::string hex(const uint8_t* p, size_t n);
std::string hex(const std::vector<uint8_t>& v);
std::string describe(const AmPacket& a);   // "IN v2 token 0x.. ssrc 0x.. name 'x'"

}  // namespace rtpmidi
