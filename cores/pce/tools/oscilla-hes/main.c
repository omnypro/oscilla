#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "cpce.h"

/* oscilla-hes: plays an HES file.
 *
 *   oscilla-hes FILE.hes [--song N] [--seconds S] [--rate HZ] [--out FILE.wav]
 *                        [--mask HEX] [--writes FILE.txt]
 *   oscilla-hes FILE.hes --info
 *   oscilla-hes FILE.hes --bench [--song N] [--seconds S] [--rate HZ]
 *
 * The default is the file's first song, 30 seconds at 44100 Hz, written to
 * out.wav. --mask picks the channels that reach the mix (bits 0 to 5).
 * --writes writes one line for each store to the sound generator: the time in
 * samples of 44100 Hz, the channel, the register and the value. --info prints
 * the first song. Bench mode renders with no output and prints how long that
 * took. */

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

static void on_write(void *context, int channel, uint32_t reg, uint32_t value, uint64_t sample) {
    fprintf((FILE *)context, "%llu %d %u %u\n", (unsigned long long)sample, channel,
            (unsigned)reg, (unsigned)value);
}

int main(int argc, char **argv) {
    const char *path = NULL, *out_path = "out.wav", *writes_path = NULL;
    int rate = 44100, info = 0, bench = 0, have_mask = 0, song = -1;
    uint64_t mask = 0;
    double seconds = 30;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atof(argv[++i]);
        else if (!strcmp(argv[i], "--rate") && i + 1 < argc) rate = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--song") && i + 1 < argc) song = atoi(argv[++i]);
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
    if (!path) { fprintf(stderr, "usage: oscilla-hes FILE.hes [options]\n"); return 2; }

    cpce_image *image = NULL;
    char error[256];
    if (cpce_image_load_hes(path, &image, error, sizeof error) != CPCE_OK) {
        fprintf(stderr, "%s\n", error);
        return 1;
    }
    if (info) {
        printf("first song %u\n", (unsigned)cpce_image_start_song(image));
        cpce_image_free(image);
        return 0;
    }
    if (song < 0) song = (int)cpce_image_start_song(image);
    cpce_machine *m = cpce_new(image, rate, song);
    cpce_image_free(image);
    if (!m) { fprintf(stderr, "the rate %d is out of range\n", rate); return 1; }
    if (have_mask) cpce_set_channel_mask(m, mask);

    FILE *writes = NULL;
    cpce_observer observer;
    if (writes_path) {
        writes = fopen(writes_path, "w");
        if (!writes) { fprintf(stderr, "cannot write %s\n", writes_path); cpce_free(m); return 1; }
        observer.context = writes;
        observer.psg_write = on_write;
        cpce_set_observer(m, &observer);
    }

    /* A WAV file holds at most 4 GiB. */
    if (!(seconds >= 0) || seconds * rate > 1e9) {
        fprintf(stderr, "the length must be between 0 and 1e9 frames\n");
        cpce_free(m);
        if (writes) fclose(writes);
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
            cpce_render_s16(m, buf, n);
            done += n;
        }
        double elapsed = now_seconds() - start;
        printf("%.1f s of sound in %.3f s: %.0fx real time\n", seconds, elapsed, seconds / elapsed);
        free(buf);
        cpce_free(m);
        if (writes) fclose(writes);
        return 0;
    }

    FILE *wav = fopen(out_path, "wb");
    if (!wav) { fprintf(stderr, "cannot write %s\n", out_path); return 1; }
    int ok = wav_header(wav, (uint32_t)total, (uint32_t)rate);
    for (size_t done = 0; done < total;) {
        size_t n = total - done < chunk ? total - done : chunk;
        cpce_render_s16(m, buf, n);
        ok &= fwrite(buf, 4, n, wav) == n;
        done += n;
    }
    ok &= fclose(wav) == 0;
    if (writes) ok &= fclose(writes) == 0;
    if (!ok) {
        fprintf(stderr, "writing failed\n");
        free(buf);
        cpce_free(m);
        return 1;
    }
    free(buf);
    cpce_free(m);
    return 0;
}
