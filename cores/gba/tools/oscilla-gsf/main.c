#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/stat.h>
#include "cgba.h"

/* oscilla-gsf: renders a GSF and writes the trace files the
 * reference runner writes, so the two runs can be compared file by file.
 *
 *   oscilla-gsf FILE.minigsf [--seconds N] [--out-dir DIR] [--cycles]
 *   oscilla-gsf --test-rom FILE.gba [--frames N]
 *   oscilla-gsf FILE.minigsf --bench [--seconds N] [--rate HZ]
 *
 * Trace mode writes fifo-a.bin, fifo-b.bin, sound-writes.txt and mix.wav
 * (stereo 16-bit at 32768 Hz). The run ends at the first VBlank at or
 * after N seconds of native clock, and nothing after that cycle is written.
 * --cycles adds sound-writes-cycles.txt. Each line there begins with the
 * cycle count at which the instruction or DMA transfer that made the store
 * started.
 * Bench mode renders N seconds at HZ (48000 by default) with no tracing and
 * prints how long that took. Test mode runs a test cartridge until it parks
 * in a branch-to-self loop and prints r7 and r12, where test ROMs of this
 * kind leave their result. */

#define FRAME_CYCLES 280896ull

typedef struct {
    FILE *writes, *fifo[2], *stamped;
    unsigned long long write_count, fifo_count[2];
    cgba_machine *machine;
    uint64_t end;      /* events from this cycle on fall after the run and are not written */
} trace_files;

static void on_write(void *ctx, uint32_t address, uint32_t value, int width) {
    trace_files *t = ctx;
    if (cgba_cycles(t->machine) >= t->end) return;
    fprintf(t->writes, "%08x %0*x %d\n", address, width * 2, value, width);
    if (t->stamped)
        fprintf(t->stamped, "%llu %08x %0*x %d\n", (unsigned long long)cgba_cycles(t->machine),
                address, width * 2, value, width);
    t->write_count++;
}

static void on_fifo(void *ctx, int channel, int8_t sample) {
    trace_files *t = ctx;
    if (cgba_cycles(t->machine) >= t->end) return;
    fputc((unsigned char)sample, t->fifo[channel]);
    t->fifo_count[channel]++;
}

static void put32(unsigned char *p, uint32_t v) {
    p[0] = (unsigned char)v; p[1] = (unsigned char)(v >> 8);
    p[2] = (unsigned char)(v >> 16); p[3] = (unsigned char)(v >> 24);
}

static void wav_header(FILE *f, uint32_t frames, uint32_t rate) {
    unsigned char h[44];
    uint32_t bytes = frames * 4;
    memcpy(h, "RIFF", 4); put32(h + 4, 36 + bytes);
    memcpy(h + 8, "WAVEfmt ", 8); put32(h + 16, 16);
    h[20] = 1; h[21] = 0; h[22] = 2; h[23] = 0;
    put32(h + 24, rate); put32(h + 28, rate * 4);
    h[32] = 4; h[33] = 0; h[34] = 16; h[35] = 0;
    memcpy(h + 36, "data", 4); put32(h + 40, bytes);
    fwrite(h, 1, 44, f);
}

static FILE *open_in(const char *dir, const char *name, const char *mode) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    FILE *f = fopen(path, mode);
    if (!f) fprintf(stderr, "cannot write %s\n", path);
    return f;
}

static double now_seconds(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static int run_test_rom(const char *path, int max_frames) {
    FILE *f = fopen(path, "rb");
    if (!f) { fprintf(stderr, "cannot read %s\n", path); return 2; }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (n <= 0) { fclose(f); fprintf(stderr, "%s is empty or unreadable\n", path); return 2; }
    unsigned char *rom = malloc((size_t)n);
    if (!rom || fread(rom, 1, (size_t)n, f) != (size_t)n) { fclose(f); free(rom); return 2; }
    fclose(f);
    cgba_image *img;
    if (cgba_image_from_rom(rom, (size_t)n, &img) != CGBA_OK) { free(rom); return 2; }
    free(rom);
    cgba_machine *m = cgba_new(img, 32768);
    cgba_image_free(img);
    if (!m) return 2;
    int frame, parked = 0;
    for (frame = 0; frame < max_frames; frame++) {
        cgba_run_cycles(m, FRAME_CYCLES);
        uint32_t pc = cgba_next_pc(m), cpsr = cgba_register(m, 16);
        int self = (cpsr & 0x20) ? cgba_peek(m, pc, 2) == 0xE7FE : cgba_peek(m, pc, 4) == 0xEAFFFFFEu;
        parked = self ? parked + 1 : 0;
        if (parked >= 2) break;
    }
    printf("%s frames=%d parked=%s pc=%08x r7=%u r12=%u\n", path, frame, parked >= 2 ? "yes" : "no",
           cgba_next_pc(m), cgba_register(m, 7), cgba_register(m, 12));
    const char *w = cgba_warnings(m);
    if (w[0]) printf("warnings:\n%s", w);
    cgba_free(m);
    return 0;
}

int main(int argc, char **argv) {
    const char *file = NULL, *out_dir = ".", *test_rom = NULL;
    int stamps = 0, bench = 0, bench_rate = 48000;
    double seconds = 20;
    int frames_limit = 600;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--seconds") && i + 1 < argc) seconds = atof(argv[++i]);
        else if (!strcmp(argv[i], "--out-dir") && i + 1 < argc) out_dir = argv[++i];
        else if (!strcmp(argv[i], "--test-rom") && i + 1 < argc) test_rom = argv[++i];
        else if (!strcmp(argv[i], "--frames") && i + 1 < argc) frames_limit = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--cycles")) stamps = 1;
        else if (!strcmp(argv[i], "--bench")) bench = 1;
        else if (!strcmp(argv[i], "--rate") && i + 1 < argc) bench_rate = atoi(argv[++i]);
        else if (argv[i][0] != '-') file = argv[i];
        else { fprintf(stderr, "unknown option %s\n", argv[i]); return 2; }
    }
    if (test_rom) return run_test_rom(test_rom, frames_limit);
    if (!file) {
        fprintf(stderr, "usage: oscilla-gsf FILE.minigsf [--seconds N] [--out-dir DIR] [--cycles]\n"
                        "       oscilla-gsf FILE.minigsf --bench [--seconds N] [--rate HZ]\n"
                        "       oscilla-gsf --test-rom FILE.gba [--frames N]\n");
        return 2;
    }
    char error[512];
    cgba_image *img;
    if (cgba_image_load_gsf(file, &img, error, sizeof error) != CGBA_OK) {
        fprintf(stderr, "%s\n", error);
        return 1;
    }
    if (bench) {
        cgba_machine *m = cgba_new(img, bench_rate);
        if (!m) { fprintf(stderr, "cannot create the machine\n"); return 1; }
        size_t total = (size_t)(seconds * bench_rate), done = 0;
        int16_t buf[2 * 4096];
        double t0 = now_seconds();
        while (done < total) {
            size_t n = total - done < 4096 ? total - done : 4096;
            cgba_render_s16(m, buf, n);
            done += n;
        }
        double elapsed = now_seconds() - t0;
        printf("%.1f s at %d Hz rendered in %.3f s (%.1f× real time)\n", seconds, bench_rate, elapsed,
               seconds / elapsed);
        cgba_free(m);
        cgba_image_free(img);
        return 0;
    }
    mkdir(out_dir, 0755);
    trace_files t = { 0 };
    t.end = UINT64_MAX;
    t.writes = open_in(out_dir, "sound-writes.txt", "w");
    t.fifo[0] = open_in(out_dir, "fifo-a.bin", "wb");
    t.fifo[1] = open_in(out_dir, "fifo-b.bin", "wb");
    FILE *wav = open_in(out_dir, "mix.wav", "wb");
    if (!t.writes || !t.fifo[0] || !t.fifo[1] || !wav) return 1;

    const uint32_t rate = 32768;
    cgba_machine *m = cgba_new(img, (int)rate);
    if (!m) { fprintf(stderr, "cannot create the machine\n"); return 1; }
    t.machine = m;
    if (stamps) t.stamped = open_in(out_dir, "sound-writes-cycles.txt", "w");
    cgba_observer obs = { &t, on_write, on_fifo };
    cgba_set_observer(m, &obs);

    /* The run ends at the first VBlank at or after N seconds, and the
     * traces stop there; the mix covers the same span. */
    unsigned long long target = (unsigned long long)(seconds * 16777216.0);
    unsigned long long vblank = cgba_next_vblank(m);
    unsigned long long frames_cycles = vblank;
    if (target > vblank) frames_cycles += (target - vblank + FRAME_CYCLES - 1) / FRAME_CYCLES * FRAME_CYCLES;
    t.end = frames_cycles;
    uint32_t total = (uint32_t)((frames_cycles + (16777216ull / rate) - 1) / (16777216ull / rate));
    wav_header(wav, total, rate);
    int16_t buf[2 * 4096];
    double t0 = now_seconds();
    for (uint32_t done = 0; done < total;) {
        uint32_t n = total - done < 4096 ? total - done : 4096;
        cgba_render_s16(m, buf, n);
        fwrite(buf, sizeof(int16_t), 2 * (size_t)n, wav);
        done += n;
    }
    double elapsed = now_seconds() - t0;
    fclose(wav);
    fclose(t.writes);
    fclose(t.fifo[0]);
    fclose(t.fifo[1]);
    if (t.stamped) fclose(t.stamped);
    const char *title = cgba_image_tag(img, "title");
    fprintf(stderr, "%s: %.3f s rendered in %.3f s (%.1f× real time); writes=%llu fifo_a=%llu fifo_b=%llu\n",
            title ? title : file, (double)frames_cycles / 16777216.0, elapsed,
            (double)frames_cycles / 16777216.0 / elapsed, t.write_count, t.fifo_count[0], t.fifo_count[1]);
    const char *w = cgba_warnings(m);
    if (w[0]) fprintf(stderr, "warnings:\n%s", w);
    cgba_free(m);
    cgba_image_free(img);
    return 0;
}
