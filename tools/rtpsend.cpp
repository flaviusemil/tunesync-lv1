// rtpsend - a stand-in playback rig: joins a Tune Remote network session over RTP-MIDI
// (AppleMIDI) and sends cues, for tests without a Mac.
//
//   rtpsend [--name N] [--port P] <host> <hostPort> <cue>...
//     cue:  note <n>        Note On n vel 100 (and Note Off after it), channel --ch
//           cc <n> <v>      Control Change
//           key <name>      note for that key name (C, C#, Db, ... B), octave 4
//           wait <ms>
//     --ch <1..16>          default 16
//     --stay <ms>           keep the session open after the last cue (default 1000)
//
// Exit code 0 when the session connected and every cue went out.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <chrono>
#include <string>
#include <thread>
#include <vector>
#include "../net/rtp_session.h"

using namespace rtpmidi;

static int key_note(const std::string& k) {
    static const char* const names[] = {"C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"};
    static const char* const flats[] = {"C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"};
    for (int i = 0; i < 12; ++i)
        if (k == names[i] || k == flats[i]) return 60 + i;
    return -1;
}

int main(int argc, char** argv) {
    std::string name = "rtpsend";
    int port = 5014, ch = 16, stay = 1000;
    std::vector<std::string> a;
    for (int i = 1; i < argc; ++i) {
        const std::string s = argv[i];
        if (s == "--name" && i + 1 < argc) name = argv[++i];
        else if (s == "--port" && i + 1 < argc) port = atoi(argv[++i]);
        else if (s == "--ch" && i + 1 < argc) ch = atoi(argv[++i]);
        else if (s == "--stay" && i + 1 < argc) stay = atoi(argv[++i]);
        else a.push_back(s);
    }
    if (a.size() < 2 || ch < 1 || ch > 16) {
        fprintf(stderr, "usage: rtpsend [--name N] [--port P] [--ch C] [--stay ms] <host> <hostPort> <cue>...\n");
        return 2;
    }
    Session s;
    s.on_log([](const std::string& l, bool detail) { if (!detail) printf("  %s\n", l.c_str()); });
    SessionConfig c;
    c.name = name;
    c.port = uint16_t(port);
    std::string err;
    if (!s.open(c, &err)) { fprintf(stderr, "open: %s\n", err.c_str()); return 1; }
    s.invite(a[0], uint16_t(atoi(a[1].c_str())), "");
    auto connected = [&] { for (const auto& p : s.peers()) if (p.state == PeerState::Connected) return true; return false; };
    const auto end = std::chrono::steady_clock::now() + std::chrono::seconds(15);
    while (!connected() && std::chrono::steady_clock::now() < end) s.poll(20);
    if (!connected()) { fprintf(stderr, "no session with %s:%s\n", a[0].c_str(), a[1].c_str()); return 1; }
    auto pump = [&](int ms) {
        const auto e = std::chrono::steady_clock::now() + std::chrono::milliseconds(ms);
        while (std::chrono::steady_clock::now() < e) s.poll(5);
    };
    pump(300);
    const uint8_t st = uint8_t(ch - 1);
    int rc = 0;
    for (size_t i = 2; i < a.size(); ++i) {
        std::vector<std::vector<uint8_t>> msgs;
        if ((a[i] == "note" || a[i] == "key") && i + 1 < a.size()) {
            const int n = a[i] == "note" ? atoi(a[i + 1].c_str()) : key_note(a[i + 1]);
            ++i;
            if (n < 0 || n > 127) { fprintf(stderr, "bad note %s\n", a[i].c_str()); return 2; }
            msgs = {{uint8_t(0x90 | st), uint8_t(n), 100}};
            printf("send Note On %d ch %d\n", n, ch);
            if (!s.send(msgs)) rc = 1;
            pump(100);
            msgs = {{uint8_t(0x80 | st), uint8_t(n), 0}};
        } else if (a[i] == "cc" && i + 2 < a.size()) {
            msgs = {{uint8_t(0xB0 | st), uint8_t(atoi(a[i + 1].c_str()) & 0x7F), uint8_t(atoi(a[i + 2].c_str()) & 0x7F)}};
            printf("send CC %s = %s ch %d\n", a[i + 1].c_str(), a[i + 2].c_str(), ch);
            i += 2;
        } else if (a[i] == "wait" && i + 1 < a.size()) {
            pump(atoi(a[++i].c_str()));
            continue;
        } else {
            fprintf(stderr, "bad cue %s\n", a[i].c_str());
            return 2;
        }
        if (!s.send(msgs)) rc = 1;
        pump(20);
    }
    pump(stay);
    s.close();
    return rc;
}
