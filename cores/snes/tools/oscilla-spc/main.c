#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "csnes.h"

/* oscilla-spc: plays an SPC file.
 *
 *   oscilla-spc FILE.spc [--seconds S] [--rate HZ] [--out FILE.wav]
 *                        [--writes FILE.txt] [--mask HEX]
 *   oscilla-spc FILE.spc --info
 *   oscilla-spc FILE.spc --bench [--seconds S] [--rate HZ]
 *
 * The default is 30 seconds at 32000 Hz, written to out.wav. --writes adds a
 * text file with one line per store to a DSP register: the sample, the SPC700
 * clock, the register and the value. --mask picks the voices and the echo (bit
 * 8) that reach the mix. --info prints the tags and the times. Bench mode
 * renders with no output and prints how long that took. */

typedef struct {
    FILE *writes;
    csnes_machine *machine;
} trace;

static void on_write(void *ctx, uint32_t reg, uint32_t value) {
    trace *t = ctx;
    uint64_t cycle = csnes_cycles(t->machine);
    fprintf(t->writes, "%llu %llu %02x %02x\n", (unsigned long long)(cycle / 32),
            (unsigned long long)cycle, reg, value);
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
    int rate = 32000, info = 0, bench = 0, have_mask = 0;
    uint64_t mask = 0;
    double seconds = 30;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atof(argv[++i]);
        else if (!strcmp(argv[i], "--rate") && i + 1 < argc) rate = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--out") && i + 1 < argc) out_path = argv[++i];
        else if (!strcmp(argv[i], "--writes") && i + 1 < argc) writes_path = argv[++i];
        else if (!strcmp(argv[i], "--mask") && i + 1 < argc) {
            mask = strtoull(argv[++i], NULL, 16);
            have_mask = 1;
        }
        else if (!strcmp(argv[i], "--info")) info = 1;
        else if (!strcmp(argv[i], "--bench")) bench = 1;
        else if (argv[i][0] != '-' && !path) path = argv[i];
        else { fprintf(stderr, "unknown argument %s\n", argv[i]); return 2; }
    }
    if (!path) { fprintf(stderr, "usage: oscilla-spc FILE.spc [options]\n"); return 2; }

    csnes_image *image = NULL;
    char error[256];
    if (csnes_image_load_spc(path, &image, error, sizeof error) != CSNES_OK) {
        fprintf(stderr, "%s\n", error);
        return 1;
    }
    if (info) {
        printf("song      %s\ngame      %s\nartist    %s\ndumper    %s\ncomments  %s\n",
               csnes_image_song(image), csnes_image_game(image), csnes_image_artist(image),
               csnes_image_dumper(image), csnes_image_comments(image));
        printf("time %lld ms  fade %lld ms  length %lld ms\n",
               (long long)csnes_image_time_ms(image), (long long)csnes_image_fade_ms(image),
               (long long)csnes_image_length_ms(image));
        printf("muted voices %02x  amplification %u\n",
               (unsigned)csnes_image_muted_voices(image),
               (unsigned)csnes_image_amplification(image));
        csnes_image_free(image);
        return 0;
    }
    csnes_machine *m = csnes_new(image, rate);
    csnes_image_free(image);
    if (!m) { fprintf(stderr, "the rate %d is out of range\n", rate); return 1; }
    if (have_mask) csnes_set_channel_mask(m, mask);

    /* A WAV file holds at most 4 GiB. */
    if (!(seconds >= 0) || seconds * rate > 1e9) {
        fprintf(stderr, "the length must be between 0 and 1e9 frames\n");
        csnes_free(m);
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
            csnes_render_s16(m, buf, n);
            done += n;
        }
        double elapsed = now_seconds() - start;
        printf("%.1f s of sound in %.3f s: %.0fx real time\n", seconds, elapsed, seconds / elapsed);
        free(buf);
        csnes_free(m);
        return 0;
    }

    trace t = { NULL, m };
    if (writes_path) {
        t.writes = fopen(writes_path, "w");
        if (!t.writes) { fprintf(stderr, "cannot write %s\n", writes_path); return 1; }
        csnes_observer o = { &t, on_write, NULL };
        csnes_set_observer(m, &o);
    }
    FILE *wav = fopen(out_path, "wb");
    if (!wav) { fprintf(stderr, "cannot write %s\n", out_path); return 1; }
    int ok = wav_header(wav, (uint32_t)total, (uint32_t)rate);
    for (size_t done = 0; done < total;) {
        size_t n = total - done < chunk ? total - done : chunk;
        csnes_render_s16(m, buf, n);
        ok &= fwrite(buf, 4, n, wav) == n;
        done += n;
    }
    ok &= fclose(wav) == 0;
    if (t.writes) ok &= fclose(t.writes) == 0;
    if (!ok) {
        fprintf(stderr, "writing failed\n");
        free(buf);
        csnes_free(m);
        return 1;
    }
    free(buf);
    csnes_free(m);
    return 0;
}
