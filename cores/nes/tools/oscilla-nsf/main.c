#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "cnes.h"

/* oscilla-nsf: plays one song of an NSF file.
 *
 *   oscilla-nsf FILE.nsf [--song N] [--seconds S] [--rate HZ] [--out FILE.wav]
 *                        [--writes FILE.txt] [--highpass HZ]
 *                        [--region ntsc|pal]
 *   oscilla-nsf FILE.nsf --info
 *   oscilla-nsf FILE.nsf [--song N] --bench [--seconds S] [--rate HZ] [--highpass HZ]
 *                        [--region ntsc|pal]
 *
 * The default is song 0 for 30 seconds at 48000 Hz, written to out.wav.
 * --writes adds a text file with one line per store to the sound registers:
 * the song, the frame (-1 for the init routine), the cycle since that
 * frame's start, the address and the value. --info prints the header.
 * --highpass moves the bass cutoff from the console's 90 Hz. --region picks
 * the console; the default is the one that the file names.
 * Bench mode renders with no output and prints how long that took. */

typedef struct {
    FILE *writes;
    cnes_machine *machine;
    int song;
    long long frame;
    uint64_t frame_start;
} trace;

static void on_write(void *ctx, uint32_t address, uint32_t value) {
    trace *t = ctx;
    uint64_t cycle = cnes_cycles(t->machine) - t->frame_start;
    fprintf(t->writes, "%d %lld %llu %04x %02x\n", t->song, t->frame,
            (unsigned long long)cycle, address, value);
}

static void on_call(void *ctx, int64_t frame) {
    trace *t = ctx;
    t->frame = frame;
    t->frame_start = frame < 0 ? 0 : cnes_cycles(t->machine);
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
    int region = -1;
    double seconds = 30, highpass = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--song") && i + 1 < argc) song = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atof(argv[++i]);
        else if (!strcmp(argv[i], "--rate") && i + 1 < argc) rate = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--out") && i + 1 < argc) out_path = argv[++i];
        else if (!strcmp(argv[i], "--writes") && i + 1 < argc) writes_path = argv[++i];
        else if (!strcmp(argv[i], "--highpass") && i + 1 < argc) highpass = atof(argv[++i]);
        else if (!strcmp(argv[i], "--region") && i + 1 < argc) {
            const char *name = argv[++i];
            if (!strcmp(name, "ntsc")) region = CNES_NTSC;
            else if (!strcmp(name, "pal")) region = CNES_PAL;
            else { fprintf(stderr, "the region must be ntsc or pal\n"); return 2; }
        }
        else if (!strcmp(argv[i], "--info")) info = 1;
        else if (!strcmp(argv[i], "--bench")) bench = 1;
        else if (argv[i][0] != '-' && !path) path = argv[i];
        else { fprintf(stderr, "unknown argument %s\n", argv[i]); return 2; }
    }
    if (!path) { fprintf(stderr, "usage: oscilla-nsf FILE.nsf [options]\n"); return 2; }

    cnes_image *image = NULL;
    char error[256];
    if (cnes_image_load_nsf(path, &image, error, sizeof error) != CNES_OK) {
        fprintf(stderr, "%s\n", error);
        return 1;
    }
    cnes_header h;
    cnes_image_header(image, &h);
    if (info) {
        printf("title     %s\nauthor    %s\ncopyright %s\n", cnes_image_title(image),
               cnes_image_author(image), cnes_image_copyright(image));
        printf("songs %d  first %d  load %04x  init %04x  play %04x\n",
               h.songs, h.first_song, h.load, h.init, h.play);
        printf("ntsc %u us  pal %u us  region %02x  chips %02x\n", h.ntsc_us, h.pal_us,
               h.region_bits, h.chips);
        printf("banks");
        for (int b = 0; b < 8; b++) printf(" %02x", h.banks[b]);
        printf("\n");
        cnes_image_free(image);
        return 0;
    }
    cnes_machine *m = region < 0 ? cnes_new(image, rate) : cnes_new_for(image, rate, region);
    cnes_image_free(image);
    if (!m) { fprintf(stderr, "the rate %d is out of range\n", rate); return 1; }
    if (!cnes_select_song(m, song)) {
        fprintf(stderr, "the file has %d songs\n", h.songs);
        cnes_free(m);
        return 1;
    }

    if (highpass != 0 && !cnes_set_highpass_hz(m, highpass)) {
        fprintf(stderr, "the cutoff must be between %.0f and %.0f Hz\n", CNES_HIGHPASS_MIN_HZ,
                CNES_HIGHPASS_MAX_HZ);
        cnes_free(m);
        return 2;
    }

    /* A WAV file holds at most 4 GiB. */
    if (!(seconds >= 0) || seconds * rate > 1e9) {
        fprintf(stderr, "the length must be between 0 and 1e9 frames\n");
        cnes_free(m);
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
            cnes_render_s16(m, buf, n);
            done += n;
        }
        double elapsed = now_seconds() - start;
        printf("%.1f s of sound in %.3f s: %.0fx real time\n", seconds, elapsed, seconds / elapsed);
        free(buf);
        cnes_free(m);
        return 0;
    }

    trace t = { NULL, m, song, -1, 0 };
    if (writes_path) {
        t.writes = fopen(writes_path, "w");
        if (!t.writes) { fprintf(stderr, "cannot write %s\n", writes_path); return 1; }
        cnes_observer o = { &t, on_write, on_call };
        cnes_set_observer(m, &o);
    }
    FILE *wav = fopen(out_path, "wb");
    if (!wav) { fprintf(stderr, "cannot write %s\n", out_path); return 1; }
    int ok = wav_header(wav, (uint32_t)total, (uint32_t)rate);
    for (size_t done = 0; done < total;) {
        size_t n = total - done < chunk ? total - done : chunk;
        cnes_render_s16(m, buf, n);
        ok &= fwrite(buf, 4, n, wav) == n;
        done += n;
    }
    ok &= fclose(wav) == 0;
    if (t.writes) ok &= fclose(t.writes) == 0;
    if (!ok) {
        fprintf(stderr, "writing failed\n");
        free(buf);
        cnes_free(m);
        return 1;
    }
    const char *warnings = cnes_warnings(m);
    if (warnings[0]) fprintf(stderr, "%s", warnings);
    free(buf);
    cnes_free(m);
    return 0;
}
