#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "cgb.h"

/* oscilla-gbs: plays one song of a GBS file.
 *
 *   oscilla-gbs FILE.gbs [--song N] [--seconds S] [--rate HZ] [--out FILE.wav]
 *                        [--writes FILE.txt] [--highpass HZ]
 *   oscilla-gbs FILE.gbs --info
 *   oscilla-gbs FILE.gbs [--song N] --bench [--seconds S] [--rate HZ] [--highpass HZ]
 *
 * The default is song 0 for 30 seconds at 48000 Hz, written to out.wav.
 * --writes adds a text file with one line per store to the sound registers:
 * the song, the frame (-1 for the init routine), the cycle since that
 * frame's start, the address and the value. --info prints the header.
 * --highpass moves the bass cutoff from the console's 28 Hz.
 * Bench mode renders with no output and prints how long that took. */

typedef struct {
    FILE *writes;
    cgb_machine *machine;
    int song;
    long long frame;
    uint64_t frame_start;
} trace;

static void on_write(void *ctx, uint32_t address, uint32_t value) {
    trace *t = ctx;
    uint64_t cycle = cgb_cycles(t->machine) - t->frame_start;
    fprintf(t->writes, "%d %lld %llu %04x %02x\n", t->song, t->frame,
            (unsigned long long)cycle, address, value);
}

static void on_call(void *ctx, int64_t frame) {
    trace *t = ctx;
    t->frame = frame;
    t->frame_start = frame < 0 ? 0 : cgb_cycles(t->machine);
}

static void put32(unsigned char *p, uint32_t v) {
    p[0] = (unsigned char)v; p[1] = (unsigned char)(v >> 8);
    p[2] = (unsigned char)(v >> 16); p[3] = (unsigned char)(v >> 24);
}

static int wav_header(FILE *f, uint32_t frames, uint32_t rate) {
    unsigned char h[44];
    uint32_t bytes = frames * 4;
    memcpy(h, "RIFF", 4); put32(h + 4, 36 + bytes);
    memcpy(h + 8, "WAVEfmt ", 8); put32(h + 16, 16);
    h[20] = 1; h[21] = 0; h[22] = 2; h[23] = 0;
    put32(h + 24, rate); put32(h + 28, rate * 4);
    h[32] = 4; h[33] = 0; h[34] = 16; h[35] = 0;
    memcpy(h + 36, "data", 4); put32(h + 40, bytes);
    return fwrite(h, 1, 44, f) == 44;
}

static double now_seconds(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

int main(int argc, char **argv) {
    const char *path = NULL, *out_path = "out.wav", *writes_path = NULL;
    int song = 0, rate = 48000, info = 0, bench = 0;
    double seconds = 30, highpass = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--song") && i + 1 < argc) song = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atof(argv[++i]);
        else if (!strcmp(argv[i], "--rate") && i + 1 < argc) rate = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--out") && i + 1 < argc) out_path = argv[++i];
        else if (!strcmp(argv[i], "--writes") && i + 1 < argc) writes_path = argv[++i];
        else if (!strcmp(argv[i], "--highpass") && i + 1 < argc) highpass = atof(argv[++i]);
        else if (!strcmp(argv[i], "--info")) info = 1;
        else if (!strcmp(argv[i], "--bench")) bench = 1;
        else if (argv[i][0] != '-' && !path) path = argv[i];
        else { fprintf(stderr, "unknown argument %s\n", argv[i]); return 2; }
    }
    if (!path) { fprintf(stderr, "usage: oscilla-gbs FILE.gbs [options]\n"); return 2; }

    cgb_image *image = NULL;
    char error[256];
    if (cgb_image_load_gbs(path, &image, error, sizeof error) != CGB_OK) {
        fprintf(stderr, "%s\n", error);
        return 1;
    }
    cgb_header h;
    cgb_image_header(image, &h);
    if (info) {
        printf("title     %s\nauthor    %s\ncopyright %s\n", cgb_image_title(image),
               cgb_image_author(image), cgb_image_copyright(image));
        printf("songs %d  first %d  load %04x  init %04x  play %04x  sp %04x  tma %02x  tac %02x\n",
               h.songs, h.first_song, h.load, h.init, h.play, h.sp, h.tma, h.tac);
        printf("tick %u clocks\n", cgb_image_tick_clocks(image));
        cgb_image_free(image);
        return 0;
    }
    cgb_machine *m = cgb_new(image, rate);
    cgb_image_free(image);
    if (!m) { fprintf(stderr, "the rate %d is out of range\n", rate); return 1; }
    if (!cgb_select_song(m, song)) {
        fprintf(stderr, "the file has %d songs\n", h.songs);
        cgb_free(m);
        return 1;
    }

    if (highpass != 0 && !cgb_set_highpass_hz(m, highpass)) {
        fprintf(stderr, "the cutoff must be between %.0f and %.0f Hz\n", CGB_HIGHPASS_MIN_HZ,
                CGB_HIGHPASS_MAX_HZ);
        cgb_free(m);
        return 2;
    }

    /* A WAV file holds at most 4 GiB. */
    if (!(seconds >= 0) || seconds * rate > 1e9) {
        fprintf(stderr, "the length must be between 0 and 1e9 frames\n");
        cgb_free(m);
        return 2;
    }
    size_t total = (size_t)(seconds * rate);
    size_t chunk = 4096;
    int16_t *buf = malloc(chunk * 2 * sizeof *buf);
    if (!buf) return 1;

    if (bench) {
        double start = now_seconds();
        for (size_t done = 0; done < total;) {
            size_t n = total - done < chunk ? total - done : chunk;
            cgb_render_s16(m, buf, n);
            done += n;
        }
        double elapsed = now_seconds() - start;
        printf("%.1f s of sound in %.3f s: %.0fx real time\n", seconds, elapsed, seconds / elapsed);
        free(buf);
        cgb_free(m);
        return 0;
    }

    trace t = { NULL, m, song, -1, 0 };
    if (writes_path) {
        t.writes = fopen(writes_path, "w");
        if (!t.writes) { fprintf(stderr, "cannot write %s\n", writes_path); return 1; }
        cgb_observer o = { &t, on_write, on_call };
        cgb_set_observer(m, &o);
    }
    FILE *wav = fopen(out_path, "wb");
    if (!wav) { fprintf(stderr, "cannot write %s\n", out_path); return 1; }
    int ok = wav_header(wav, (uint32_t)total, (uint32_t)rate);
    for (size_t done = 0; done < total;) {
        size_t n = total - done < chunk ? total - done : chunk;
        cgb_render_s16(m, buf, n);
        ok &= fwrite(buf, 4, n, wav) == n;
        done += n;
    }
    ok &= fclose(wav) == 0;
    if (t.writes) ok &= fclose(t.writes) == 0;
    if (!ok) {
        fprintf(stderr, "writing failed\n");
        free(buf);
        cgb_free(m);
        return 1;
    }
    const char *warnings = cgb_warnings(m);
    if (warnings[0]) fprintf(stderr, "%s", warnings);
    free(buf);
    cgb_free(m);
    return 0;
}
