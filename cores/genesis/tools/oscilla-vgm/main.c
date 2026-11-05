#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "cgenesis.h"

/* oscilla-vgm: plays a VGM or VGZ file.
 *
 *   oscilla-vgm FILE.vgm [--seconds S] [--rate HZ] [--out FILE.wav] [--mask HEX]
 *   oscilla-vgm FILE.vgm --info
 *   oscilla-vgm FILE.vgm --bench [--seconds S] [--rate HZ]
 *
 * The default is 30 seconds at 44100 Hz, written to out.wav. --mask picks the
 * channels that reach the mix: bits 0 to 5 are the YM2612's channels and bits
 * 6 to 9 the SN76489's. --info prints the tags and the times. Bench mode
 * renders with no output and prints how long that took. */

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
    const char *path = NULL, *out_path = "out.wav";
    int rate = 44100, info = 0, bench = 0, have_mask = 0;
    uint64_t mask = 0;
    double seconds = 30;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atof(argv[++i]);
        else if (!strcmp(argv[i], "--rate") && i + 1 < argc) rate = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--out") && i + 1 < argc) out_path = argv[++i];
        else if (!strcmp(argv[i], "--mask") && i + 1 < argc) {
            mask = strtoull(argv[++i], NULL, 16);
            have_mask = 1;
        }
        else if (!strcmp(argv[i], "--info")) info = 1;
        else if (!strcmp(argv[i], "--bench")) bench = 1;
        else if (argv[i][0] != '-' && !path) path = argv[i];
        else { fprintf(stderr, "unknown argument %s\n", argv[i]); return 2; }
    }
    if (!path) { fprintf(stderr, "usage: oscilla-vgm FILE.vgm [options]\n"); return 2; }

    cgenesis_image *image = NULL;
    char error[256];
    if (cgenesis_image_load_vgm(path, &image, error, sizeof error) != CGENESIS_OK) {
        fprintf(stderr, "%s\n", error);
        return 1;
    }
    if (info) {
        printf("title     %s\ngame      %s\nsystem    %s\nauthor    %s\ndate      %s\nripper    %s\nnotes     %s\n",
               cgenesis_image_title(image), cgenesis_image_game(image),
               cgenesis_image_system(image), cgenesis_image_author(image),
               cgenesis_image_date(image), cgenesis_image_ripper(image),
               cgenesis_image_notes(image));
        printf("chips %u  frame rate %u  loop %s\n", (unsigned)cgenesis_image_chips(image),
               (unsigned)cgenesis_image_frame_rate(image),
               cgenesis_image_has_loop(image) ? "yes" : "no");
        printf("total %u samples  intro %u samples  loop %u samples  gain %.3f\n",
               (unsigned)cgenesis_image_total_samples(image),
               (unsigned)cgenesis_image_intro_samples(image),
               (unsigned)cgenesis_image_loop_samples(image), cgenesis_image_gain(image));
        cgenesis_image_free(image);
        return 0;
    }
    cgenesis_machine *m = cgenesis_new(image, rate);
    cgenesis_image_free(image);
    if (!m) { fprintf(stderr, "the rate %d is out of range\n", rate); return 1; }
    if (have_mask) cgenesis_set_channel_mask(m, mask);

    /* A WAV file holds at most 4 GiB. */
    if (!(seconds >= 0) || seconds * rate > 1e9) {
        fprintf(stderr, "the length must be between 0 and 1e9 frames\n");
        cgenesis_free(m);
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
            cgenesis_render_s16(m, buf, n);
            done += n;
        }
        double elapsed = now_seconds() - start;
        printf("%.1f s of sound in %.3f s: %.0fx real time\n", seconds, elapsed, seconds / elapsed);
        free(buf);
        cgenesis_free(m);
        return 0;
    }

    FILE *wav = fopen(out_path, "wb");
    if (!wav) { fprintf(stderr, "cannot write %s\n", out_path); return 1; }
    int ok = wav_header(wav, (uint32_t)total, (uint32_t)rate);
    for (size_t done = 0; done < total;) {
        size_t n = total - done < chunk ? total - done : chunk;
        cgenesis_render_s16(m, buf, n);
        ok &= fwrite(buf, 4, n, wav) == n;
        done += n;
    }
    ok &= fclose(wav) == 0;
    if (!ok) {
        fprintf(stderr, "writing failed\n");
        free(buf);
        cgenesis_free(m);
        return 1;
    }
    free(buf);
    cgenesis_free(m);
    return 0;
}
