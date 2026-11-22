/*
 * capi_test.c - TuneSync's C API driven from C against a fake LV1 behind the TsMixer
 * vtable, the way crates/host/tests/runtime.rs drives the runtime from Rust. Built and run
 * by crates/capi/tests/c_program.rs (clang, in the tunesync-build:1 image).
 *
 *   capi_test <data dir>     the folder is created; settings.ini and tunesync.log land there
 *
 * Exit 0 = every check passed; each failure prints a line.
 */
#define _POSIX_C_SOURCE 200809L
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "tunesync.h"

/* Waves Tune RT as LV1 shows it (crates/tunesync/src/tune.rs). */
enum { PARAM_SCALE = 39, PARAM_KEY = 40, PARAM_RECEIVE = 42, SLOT_INFO = -1, CID_SLOT_CODE = 1 };
static const char *KEYS[17] = {"C", "C#", "Db", "D", "D#", "Eb", "E", "F", "F#", "Gb", "G", "G#", "Ab", "A", "A#", "Bb", "B"};
static const char *SCALES[11] = {"User Scale",    "Chromatic",     "Major",          "Natural Minor", "Harmonic Minor", "Melodic Minor (Asc)",
                                 "Melodic Minor (Asc+Dec)", "Major Justonic", "Minor Justonic", "Dorian", "Mixolydian"};

/* Tune RT on Ch 43 (A Natural Minor, Rcv) and Ch 45 (Eb Major, Lock), insert slot 1. */
typedef struct {
    uint32_t ti;
    double key, scale, receive;
} Tune;

typedef struct {
    pthread_mutex_t mu;
    Tune tunes[2];
    unsigned long calls;
    pthread_t worker;
    int have_worker, other_thread;
    TsRuntime *rt;
    int stop_in_callback; /* the next get calls ts_stop(rt) and keeps what it returns */
    int stop_rc;
} Lv1;

static int failures;

#define CHECK(cond, ...)                                    \
    do {                                                    \
        if (!(cond)) {                                      \
            failures++;                                     \
            printf("FAIL %s:%d: %s: ", __FILE__, __LINE__, #cond); \
            printf(__VA_ARGS__);                            \
            printf("\n");                                   \
        }                                                   \
    } while (0)

static Tune *find(Lv1 *l, uint32_t tt, uint32_t ti, uint32_t slot) {
    for (int i = 0; i < 2; i++)
        if (tt == 0 && slot == 0 && l->tunes[i].ti == ti) return &l->tunes[i];
    return NULL;
}

static void called(Lv1 *l) {
    l->calls++;
    if (!l->have_worker) {
        l->worker = pthread_self();
        l->have_worker = 1;
    } else if (!pthread_equal(l->worker, pthread_self())) {
        l->other_thread = 1;
    }
}

static int fake_get(void *ctx, uint32_t tt, uint32_t ti, uint32_t slot, int32_t channel, int32_t cid, double *value, char *text, size_t cap) {
    Lv1 *l = ctx;
    pthread_mutex_lock(&l->mu);
    called(l);
    if (l->stop_in_callback) {
        l->stop_in_callback = 0;
        l->stop_rc = ts_stop(l->rt);
    }
    Tune *t = find(l, tt, ti, slot);
    *value = 0;
    snprintf(text, cap, "%s", "");
    if (t && channel == SLOT_INFO) {
        snprintf(text, cap, "%s", cid == CID_SLOT_CODE ? "LVLM" : "Tune RT");
    } else if (t && channel == PARAM_KEY) {
        *value = t->key;
        snprintf(text, cap, "%s", KEYS[(int)t->key]);
    } else if (t && channel == PARAM_SCALE) {
        *value = t->scale;
        snprintf(text, cap, "%s", SCALES[(int)t->scale]);
    } else if (t && channel == PARAM_RECEIVE) {
        *value = t->receive;
        snprintf(text, cap, "%s", t->receive == 1 ? "Rcv" : "Lock");
    }
    pthread_mutex_unlock(&l->mu);
    return 0; /* LV1 answers for an empty slot too, with no text */
}

static int fake_set(void *ctx, uint32_t tt, uint32_t ti, uint32_t slot, int32_t param, double value) {
    Lv1 *l = ctx;
    int rc = 1;
    pthread_mutex_lock(&l->mu);
    called(l);
    Tune *t = find(l, tt, ti, slot);
    if (t && param == PARAM_KEY) {
        t->key = value;
        rc = 0;
    } else if (t && param == PARAM_SCALE) {
        t->scale = value;
        rc = 0;
    }
    pthread_mutex_unlock(&l->mu);
    return rc;
}

static size_t fake_track_name(void *ctx, uint32_t tt, uint32_t ti, char *out, size_t cap) {
    Lv1 *l = ctx;
    pthread_mutex_lock(&l->mu);
    called(l);
    pthread_mutex_unlock(&l->mu);
    /* Ch 43 has a name; Ch 45 has LV1's default one, which TuneSync leaves out. */
    const char *name = tt != 0 ? "" : ti == 42 ? "Vox 1" : ti == 44 ? "Channel 45" : "";
    return (size_t)snprintf(out, cap, "%s", name);
}

static double key_of(Lv1 *l, int i) {
    pthread_mutex_lock(&l->mu);
    double v = l->tunes[i].key;
    pthread_mutex_unlock(&l->mu);
    return v;
}

static double scale_of(Lv1 *l, int i) {
    pthread_mutex_lock(&l->mu);
    double v = l->tunes[i].scale;
    pthread_mutex_unlock(&l->mu);
    return v;
}

static unsigned long calls_of(Lv1 *l) {
    pthread_mutex_lock(&l->mu);
    unsigned long v = l->calls;
    pthread_mutex_unlock(&l->mu);
    return v;
}

static int stop_rc_of(Lv1 *l) {
    pthread_mutex_lock(&l->mu);
    int v = l->stop_rc;
    pthread_mutex_unlock(&l->mu);
    return v;
}

static void sleep_ms(long ms) {
    struct timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
    nanosleep(&ts, NULL);
}

static double now_s(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

/* CHECK(cond) once it holds, or after ms: cond is polled every 5 ms. */
#define CHECK_SOON(ms, cond, ...)                                          \
    do {                                                                   \
        for (double end_ = now_s() + (ms) / 1000.0; !(cond) && now_s() < end_;) \
            sleep_ms(5);                                                   \
        CHECK(cond, __VA_ARGS__);                                          \
    } while (0)

#define S(x) ((x) ? (x) : "(null)")

/* Whether the JSON (or text) a getter returns holds needle; frees it. */
static int has(char *s, const char *needle) {
    int r = s && strstr(s, needle) != NULL;
    ts_string_free(s);
    return r;
}

/* The number after "key": in a JSON object, -1 when absent. */
static long long num(const char *json, const char *key) {
    char pat[64];
    snprintf(pat, sizeof pat, "\"%s\":", key);
    const char *p = json ? strstr(json, pat) : NULL;
    return p ? strtoll(p + strlen(pat), NULL, 10) : -1;
}

static long long status_num(TsRuntime *rt, const char *key) {
    char *s = ts_status_json(rt);
    long long v = num(s, key);
    ts_string_free(s);
    return v;
}

static char *read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    static char buf[1 << 16];
    size_t n = fread(buf, 1, sizeof buf - 1, f);
    fclose(f);
    buf[n] = 0;
    return buf;
}

static int file_has(const char *dir, const char *name, const char *needle) {
    char path[1024];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    const char *s = read_file(path);
    return s && strstr(s, needle) != NULL;
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: capi_test <data dir>\n");
        return 2;
    }
    const char *dir = argv[1];
    alarm(60); /* a hang is a failure too */

    Lv1 lv1 = {.tunes = {{42, 13, 3, 1}, {44, 5, 2, 0}}, .stop_rc = 1};
    pthread_mutex_init(&lv1.mu, NULL);
    TsMixer mixer = {.ctx = &lv1, .get = fake_get, .set = fake_set, .track_name = fake_track_name};

    /* ---- the edges: versions, bad arguments */
    CHECK(ts_abi_version() == TS_ABI_VERSION, "abi %u", ts_abi_version());
    CHECK(ts_version() && ts_version()[0] >= '0' && ts_version()[0] <= '9', "version %s", S(ts_version()));
    TsMixer partial = mixer;
    partial.set = NULL;
    CHECK(ts_start(dir, NULL, NULL, 0) == NULL, "no mixer");
    CHECK(ts_start(dir, NULL, &partial, 0) == NULL, "a missing callback");
    CHECK(ts_start(dir, NULL, &mixer, 0x80) == NULL, "an unknown flag");
    CHECK(ts_start("", NULL, &mixer, 0) == NULL, "an empty data dir");
    CHECK(ts_stop(NULL) == TS_OK, "stop NULL");
    ts_string_free(NULL);
    CHECK(ts_status_json(NULL) == NULL && ts_settings_get(NULL) == NULL, "NULL handle");
    CHECK(ts_feed_midi(NULL, "x", (const uint8_t *)"\x90", 1) == TS_ERR_ARG, "feed into NULL");
    CHECK(calls_of(&lv1) == 0, "no call before a start");

    /* ---- the whole runtime (TuneSync's own MIDI input and RTP-MIDI, as Headlink gets it
     * by default); the cue comes in through ts_feed_midi. */
    TsRuntime *rt = ts_start(dir, "TuneSync C test", &mixer, 0);
    CHECK(rt != NULL, "start");
    if (!rt) return 1;
    lv1.rt = rt;
    CHECK_SOON(2000, status_num(rt, "tunes") == 2, "first scan finds both Tunes");
    char *tunes = ts_tunes_json(rt);
    CHECK(tunes && strstr(tunes, "\"label\":\"Ch 43\",\"name\":\"Vox 1\",\"where\":\"Ch 43 Vox 1\",\"key\":13,\"key_name\":\"A\""), "%s", S(tunes));
    CHECK(tunes && strstr(tunes, "\"label\":\"Ch 45\",\"name\":\"\""), "LV1's default name left out: %s", S(tunes));
    CHECK(tunes && strstr(tunes, "\"scale_name\":\"Major\",\"receive\":false"), "Ch 45 is on Lock: %s", S(tunes));
    ts_string_free(tunes);
    char *st = ts_status_json(rt);
    CHECK(st && strstr(st, "\"last\":null") && strstr(st, "\"midi\":{\"ports\":") && strstr(st, "\"rtp\":{\"enabled\":false"), "%s", S(st));
    ts_string_free(st);

    /* The cue from the host's own MIDI: CC 20 = 4 (Harmonic Minor) + Note On D, ch 16. */
    const uint8_t cue[] = {0xBF, 20, 4, 0x9F, 62, 100};
    CHECK(ts_feed_midi(rt, "Headlink MIDI", cue, sizeof cue) == TS_OK, "feed");
    CHECK_SOON(2000, key_of(&lv1, 0) == 3, "Ch 43 -> D (key %g)", key_of(&lv1, 0));
    CHECK(scale_of(&lv1, 0) == 4, "Harmonic Minor (scale %g)", scale_of(&lv1, 0));
    CHECK(key_of(&lv1, 1) == 5 && scale_of(&lv1, 1) == 2, "the locked Tune is untouched");
    CHECK_SOON(1000, status_num(rt, "changes") == 1, "one change");
    st = ts_status_json(rt);
    CHECK(st && strstr(st, "\"label\":\"D Harmonic Minor\"") && strstr(st, "\"source\":\"Headlink MIDI\"") && strstr(st, "\"brief\":\"set 1, 1 locked\""),
          "%s", S(st));
    ts_string_free(st);

    /* Apply now. */
    CHECK(ts_apply_now(rt, -1, -1) == TS_ERR_ARG && ts_apply_now(rt, 12, -1) == TS_ERR_ARG && ts_apply_now(rt, 0, 11) == TS_ERR_ARG, "bad Apply now");
    CHECK(ts_apply_now(rt, 9, -1) == TS_OK, "Apply now A");
    CHECK_SOON(2000, key_of(&lv1, 0) == 13, "Apply now -> A (key %g)", key_of(&lv1, 0));

    /* Settings: read, replace (one bad line), saved, applied. */
    CHECK(has(ts_settings_get(rt), "scale_cc=20\n"), "default settings");
    CHECK(ts_settings_set(rt, NULL) == TS_ERR_ARG, "NULL ini");
    CHECK(ts_settings_set(rt, "[tune-remote]\nchannel=omni\nscale_cc=21\nbogus=1\n") == 1, "one line skipped");
    CHECK(has(ts_settings_get(rt), "channel=omni\n"), "replaced");
    CHECK(file_has(dir, "settings.ini", "scale_cc=21"), "saved");
    const uint8_t cue2[] = {0xB0, 21, 9, 0x90, 64, 90, 0x80, 64, 0}; /* Dorian, E, its Note Off: ch 1 */
    CHECK(ts_feed_midi(rt, NULL, cue2, sizeof cue2) == TS_OK, "feed 2");
    CHECK_SOON(2000, key_of(&lv1, 0) == 6 && scale_of(&lv1, 0) == 9, "omni, CC 21: E Dorian (key %g scale %g)", key_of(&lv1, 0), scale_of(&lv1, 0));
    CHECK_SOON(1000, status_num(rt, "changes") == 3, "three changes");
    CHECK(has(ts_status_json(rt), "\"source\":\"Host MIDI\""), "NULL source");

    /* The log: everything, then only what is new. */
    char *log = ts_log_json(rt, 0);
    long long gen = num(log, "generation");
    CHECK(log && strstr(log, "\"reset\":false") && strstr(log, "\"text\":\"TuneSync C test started"), "%s", S(log));
    CHECK(log && strstr(log, "D Harmonic Minor: set 1") && strstr(log, "skipped 1 locked"), "%s", S(log));
    CHECK(log && strstr(log, "\"level\":\"warn\",\"text\":\"settings: unknown setting bogus\""), "%s", S(log));
    CHECK(gen > 0, "generation %lld", gen);
    ts_string_free(log);
    ts_rescan(rt);
    log = ts_log_json(rt, (uint64_t)gen);
    CHECK(log && num(log, "generation") >= gen && strstr(log, "\"reset\":false") && !strstr(log, "D Harmonic Minor"), "only new lines: %s", S(log));
    ts_string_free(log);
    CHECK(has(ts_log_json(rt, (uint64_t)gen + 1000000), "\"reset\":true"), "a generation from elsewhere");

    /* Rescan, and ts_stop from inside a callback refuses. */
    long long scans = status_num(rt, "scans");
    CHECK(ts_rescan(rt) == TS_OK, "rescan");
    CHECK_SOON(2000, status_num(rt, "scans") > scans, "scanned again");
    pthread_mutex_lock(&lv1.mu);
    lv1.stop_in_callback = 1;
    pthread_mutex_unlock(&lv1.mu);
    ts_rescan(rt);
    CHECK_SOON(2000, stop_rc_of(&lv1) == TS_ERR_IN_CALLBACK, "ts_stop in a callback: %d", stop_rc_of(&lv1));

    /* Stop: no call into the mixer after it returns, and all of them came from one thread
     * that is not this one. */
    CHECK(ts_stop(rt) == TS_OK, "stop");
    unsigned long after = calls_of(&lv1);
    sleep_ms(300);
    CHECK(calls_of(&lv1) == after, "no call after ts_stop (%lu then %lu)", after, calls_of(&lv1));
    CHECK(lv1.have_worker && !lv1.other_thread, "every call from one thread");
    CHECK(!pthread_equal(lv1.worker, pthread_self()), "never from the caller's thread");
    CHECK(file_has(dir, "tunesync.log", "TuneSync C test started") && file_has(dir, "tunesync.log", "TuneSync stopping"), "tunesync.log");

    /* ---- engine only: the host keeps MIDI and RTP-MIDI; the saved settings come back. */
    rt = ts_start(dir, NULL, &mixer, TS_START_NO_MIDI_IN | TS_START_NO_RTP);
    CHECK(rt != NULL, "start engine only");
    if (!rt) return 1;
    st = ts_status_json(rt);
    CHECK(st && strstr(st, "\"midi\":null") && strstr(st, "\"rtp\":null"), "%s", S(st));
    ts_string_free(st);
    CHECK(has(ts_settings_get(rt), "scale_cc=21\n"), "settings.ini read back");
    CHECK(has(ts_log_json(rt, 0), "MIDI input and RTP-MIDI left to the host"), "logged");
    CHECK_SOON(2000, status_num(rt, "tunes") == 2, "scan");
    const uint8_t note_c[] = {0x9F, 60, 100};
    CHECK(ts_feed_midi(rt, "Headlink MIDI", note_c, sizeof note_c) == TS_OK, "feed C");
    CHECK_SOON(2000, key_of(&lv1, 0) == 0, "-> C (key %g)", key_of(&lv1, 0));
    CHECK(ts_rtp_connect(rt, "192.0.2.41", 5004, "Playback Mac") == TS_OK, "remember a peer without RTP");
    CHECK(ts_rtp_connect(rt, "not-an-ip", 5004, NULL) == TS_ERR_ARG, "bad ip");
    CHECK(has(ts_settings_get(rt), "rtp_peer=192.0.2.41:5004 Playback Mac"), "peer saved");
    CHECK(ts_rtp_forget(rt, "192.0.2.41", 5004) == TS_OK && !has(ts_settings_get(rt), "rtp_peer="), "peer forgotten");
    CHECK(ts_stop(rt) == TS_OK, "stop 2");

    printf("%s: %d failure%s\n", failures ? "FAILED" : "ok", failures, failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
