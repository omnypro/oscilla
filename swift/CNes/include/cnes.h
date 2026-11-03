#ifndef CNES_H
#define CNES_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A music-only NES: the 6502, memory and sound unit that run an NSF file's
 * sound driver, with no picture. The machine plays a song the way the file
 * asks. It calls the init routine with the song number in A and the region in
 * X (0 for NTSC, 1 for PAL), and then it calls the play routine at the
 * interval that the header names. It hands back the sound as interleaved
 * stereo at a rate the caller chooses.
 *
 * Two objects. An image is a parsed NSF file. A machine plays one song of an
 * image at a time. Every piece of state lives in the machine, so several
 * machines can run side by side. A machine allocates once, when it is created,
 * and never while it renders. With the same image, song, region and calls, a
 * machine produces the same output on every run.
 *
 * The machine plays the 2A03's five channels (two pulse channels, the
 * triangle, the noise channel and the delta channel) and the three channels of
 * the VRC6 or the Sunsoft 5B expansion chip. A file that asks for other
 * expansion sound gets the channels it can play, and cnes_warnings says so.
 */

/* ---- Images ------------------------------------------------------------ */

typedef struct cnes_image cnes_image;

typedef enum {
    CNES_OK = 0,
    CNES_ERR_IO,     /* a file could not be opened or read */
    CNES_ERR_FORMAT  /* not an NSF file, wrong version, or too large */
} cnes_status;

/*
 * Parses an .nsf file. A NULL `path` or `out` returns CNES_ERR_IO. On failure
 * *out is NULL and, when `error` is given, it holds a sentence that says why.
 * The path must name a regular file. A file larger than 4 MiB is refused
 * without being read in full.
 */
cnes_status cnes_image_load_nsf(const char *path, cnes_image **out,
                                char *error, size_t error_size);

/* The same for a file already in memory. The bytes are copied. */
cnes_status cnes_image_from_nsf(const uint8_t *bytes, size_t size, cnes_image **out,
                                char *error, size_t error_size);

void cnes_image_free(cnes_image *image);

typedef struct {
    int songs;            /* 1 to 255 */
    int first_song;       /* the song to start with, counted from 0 */
    uint16_t load;        /* the address of the first byte of data */
    uint16_t init;
    uint16_t play;
    uint16_t ntsc_us;     /* microseconds between play calls on NTSC; 0 names none */
    uint16_t pal_us;      /* the same on PAL */
    uint8_t banks[8];     /* the bank in each 4 KiB window from 0x8000; all 0 for no banking */
    uint8_t region_bits;  /* bit 0: PAL; bit 1: the file works on both */
    uint8_t chips;        /* expansion sound the file asks for, one bit per chip:
                             1 VRC6, 2 VRC7, 4 FDS, 8 MMC5, 16 N163, 32 Sunsoft 5B,
                             64 VT02. The core plays the VRC6 and the 5B. */
} cnes_header;

void cnes_image_header(const cnes_image *image, cnes_header *out);

/* The header's text fields, as C strings that live until the image is freed. */
const char *cnes_image_title(const cnes_image *image);
const char *cnes_image_author(const cnes_image *image);
const char *cnes_image_copyright(const cnes_image *image);

/* ---- Machines ---------------------------------------------------------- */

typedef struct cnes_machine cnes_machine;

typedef enum {
    CNES_NTSC = 0,
    CNES_PAL = 1
} cnes_region;

/* The CPU clock of each region: cycles per second. */
#define CNES_NTSC_CLOCK_HZ 1789772.7272727273
#define CNES_PAL_CLOCK_HZ 1662607.03125

/*
 * Changes whenever the same input renders different samples or write streams.
 * Anything that caches rendered results keys them on this value.
 */
#define CNES_OUTPUT_REVISION 1

/*
 * A machine for `image` that produces `output_rate` frames per second (8000 to
 * 192000), at the image's first song. The region is the one the file names: PAL
 * when the header says PAL only, and NTSC otherwise. The image may be freed
 * afterwards: the machine keeps its own copy. NULL when the rate is out of
 * range.
 */
cnes_machine *cnes_new(const cnes_image *image, int output_rate);

/* The same for a region that the caller picks. The region sets the CPU clock,
 * the sound unit's timing and the play interval. NULL when the rate is out of
 * range or `region` is neither CNES_NTSC nor CNES_PAL. */
cnes_machine *cnes_new_for(const cnes_image *image, int output_rate, cnes_region region);

void cnes_free(cnes_machine *machine);

/* The region the machine plays in, as a cnes_region. */
int cnes_region_get(const cnes_machine *machine);

/* Resets the machine and starts `song`, counted from 0. Returns 1 on success
 * and 0 when the image has no such song. */
int cnes_select_song(cnes_machine *machine, int song);

/* Back to the start of the current song. */
void cnes_reset(cnes_machine *machine);

int cnes_song(const cnes_machine *machine);

/* Runs the machine until `frames` stereo frames exist and writes them,
 * interleaved left then right. Always fills the whole buffer. The mix is
 * scaled to match a reference player, and the loudest drums of some songs pass
 * full scale: the 16-bit output clamps there, and the float output goes past
 * 1.0. A host that needs headroom scales the samples. */
void cnes_render_s16(cnes_machine *machine, int16_t *out, size_t frames);
void cnes_render_f32(cnes_machine *machine, float *out, size_t frames);

/* CPU cycles run since the song started. */
uint64_t cnes_cycles(const cnes_machine *machine);

/* Play calls begun since the song started. */
uint64_t cnes_frames(const cnes_machine *machine);

/* ---- Observation ------------------------------------------------------- */

/*
 * Callbacks for tracing what the driver does. They run on the rendering
 * thread, inside cnes_render_*, in program order. A callback may call
 * cnes_cycles, which returns the cycle of the event. Any other call that takes
 * the machine does nothing there and returns zero, NULL or an empty list
 * (cnes_warnings); cnes_snapshot_restore returns 3.
 *
 * sound_write sees every store to 0x4000..0x4017, whether or not the unit
 * accepts it: the address and the value as stored. The range holds the sound
 * registers, the delta channel's registers and the frame counter's, and also
 * the addresses 0x4014 and 0x4016, which the unit ignores.
 *
 * play_call fires just before the machine calls a routine. `frame` is -1 for
 * the init routine and counts the play calls from 0 after it. Each play call
 * is the driver's own tick.
 */
typedef struct {
    void *context;
    void (*sound_write)(void *context, uint32_t address, uint32_t value);
    void (*play_call)(void *context, int64_t frame);
} cnes_observer;

/* Copies the observer in. Pass NULL to stop observing. */
void cnes_set_observer(cnes_machine *machine, const cnes_observer *observer);

/* ---- Inspection, for tests and tools ----------------------------------- */

/* Runs `cycles` CPU cycles. Sound produced meanwhile is discarded. */
void cnes_run_cycles(cnes_machine *machine, uint64_t cycles);

/* A, X, Y, P, S by index 0 to 4; 5 is PC. */
uint32_t cnes_register(const cnes_machine *machine, int index);

/* The address of the next instruction the CPU executes. */
uint32_t cnes_next_pc(const cnes_machine *machine);

/* Reads memory as the CPU would see it, with no side effects. */
uint32_t cnes_peek(const cnes_machine *machine, uint32_t address);

/*
 * Things the machine met and does not support (a routine that did not
 * return, an opcode that locks the CPU or that the core does not run, BRK,
 * expansion sound other than the VRC6 and the Sunsoft 5B), as a NUL-terminated
 * list of short sentences, one per line.
 * The machine reports each kind once and never prints.
 */
const char *cnes_warnings(const cnes_machine *machine);

/* ---- Snapshots --------------------------------------------------------- */

/*
 * A copy of everything a machine is: the CPU, memory, sound unit and the
 * sound that has not been handed out yet. A snapshot restores into a machine
 * that plays the same image in the same region at the same output rate, and
 * from there the same calls make the same sound. The observer, the channel
 * mask and the bass cutoff belong to the listener and are not part of it.
 */
typedef struct cnes_snapshot cnes_snapshot;

cnes_snapshot *cnes_snapshot_take(const cnes_machine *machine);

/* 0 on success, 1 when the output rate differs, 2 when the image or the region
 * differs (or the snapshot is NULL), 3 when called from an observer callback. */
int cnes_snapshot_restore(cnes_machine *machine, const cnes_snapshot *snapshot);

size_t cnes_snapshot_size(const cnes_snapshot *snapshot);
void cnes_snapshot_free(cnes_snapshot *snapshot);

/* ---- Channels ---------------------------------------------------------- */

/*
 * What each of the five channels puts out now, before the mixer. The order is
 * pulse 1, pulse 2, the triangle, the noise channel (each 0 to 15) and the
 * delta channel (0 to 127). A muted channel still shows its level.
 */
void cnes_channel_levels_get(const cnes_machine *machine, int32_t out[5]);

/*
 * What each channel of the expansion chip puts out now, before the mixer. A
 * file that asks for the VRC6 shows its pulse 1 and pulse 2 (0 to 15) and its
 * saw (0 to 31) in the first three entries. A file that asks for the Sunsoft
 * 5B and not the VRC6 shows the 5B's three channels (0 to 31). The other
 * entries, and all of them for a file without a chip that the core plays, are
 * 0. A muted channel still shows its level.
 */
void cnes_expansion_levels_get(const cnes_machine *machine, int32_t out[8]);

/* Chooses the channels that reach the mix. Bit n is channel n in the order
 * above for bits 0 to 4. Bits 5 to 7 are the VRC6's pulse 1, pulse 2 and saw,
 * and bits 8 to 10 are the Sunsoft 5B's three channels. Every channel is on at
 * the start, and a reset leaves the choice alone. Only those eleven bits count,
 * and cnes_channel_mask returns 0x7FF at the start. */
void cnes_set_channel_mask(cnes_machine *machine, uint32_t mask);
uint32_t cnes_channel_mask(const cnes_machine *machine);

/* ---- Bass cutoff ------------------------------------------------------- */

/*
 * The output passes a high-pass filter, and these calls choose its cutoff.
 * The default is the original console's own filter, which cuts below about
 * 90 Hz. A higher cutoff thins the bass. The calls work while a song plays and
 * the output stays continuous across the change. A reset, a song change and
 * cnes_snapshot_restore leave the choice alone.
 */
#define CNES_HIGHPASS_CONSOLE_HZ 90.0
#define CNES_HIGHPASS_MIN_HZ 1.0
#define CNES_HIGHPASS_MAX_HZ 1000.0

/* Sets the cutoff in Hz. Returns 1 when applied, and 0 with nothing changed
 * for a value outside CNES_HIGHPASS_MIN_HZ to CNES_HIGHPASS_MAX_HZ (NaN
 * included) or a call from an observer callback. */
int cnes_set_highpass_hz(cnes_machine *machine, double hz);

/* Goes back to the console's own filter. */
void cnes_use_console_highpass(cnes_machine *machine);

/* The cutoff in use: CNES_HIGHPASS_CONSOLE_HZ for the console's own filter. */
double cnes_highpass_hz(const cnes_machine *machine);

/* ---- The end of a song ------------------------------------------------- */

typedef enum {
    CNES_RUNNING = 0,        /* a channel is sounding, or may be */
    CNES_CHANNELS_OFF,       /* no channel is on; a later play call can start one */
    CNES_STALLED             /* a routine did not return, or locked the CPU */
} cnes_end_state;

cnes_end_state cnes_end_state_get(const cnes_machine *machine);

/* CPU cycles since the output level last changed. */
uint64_t cnes_quiet_cycles(const cnes_machine *machine);

#ifdef __cplusplus
}
#endif

#endif
