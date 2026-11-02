#ifndef CGB_H
#define CGB_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A music-only Game Boy: the CPU, memory, timer and sound unit that run a GBS
 * file's sound driver, with no picture. The machine plays a song the way the
 * file asks. It calls the init routine with the song number in A, and then it
 * calls the play routine at the rate that the header names. It hands back the
 * sound as interleaved stereo at a rate the caller chooses.
 *
 * Two objects. An image is a parsed GBS file. A machine plays one song of an
 * image at a time. Every piece of state lives in the machine, so several
 * machines can run side by side. A machine allocates once, when it is created,
 * and never while it renders. With the same image, song and calls, a machine
 * produces the same output on every run.
 */

/* ---- Images ------------------------------------------------------------ */

typedef struct cgb_image cgb_image;

typedef enum {
    CGB_OK = 0,
    CGB_ERR_IO,     /* a file could not be opened or read */
    CGB_ERR_FORMAT  /* not a GBS file, wrong version, or too large */
} cgb_status;

/*
 * Parses a .gbs file. A NULL `path` or `out` returns CGB_ERR_IO. On failure *out
 * is NULL and, when `error` is given, it holds a sentence that says why. The
 * path must name a regular file. A file larger than 4 MiB is refused without
 * being read in full.
 */
cgb_status cgb_image_load_gbs(const char *path, cgb_image **out,
                              char *error, size_t error_size);

/* The same for a file already in memory. The bytes are copied. */
cgb_status cgb_image_from_gbs(const uint8_t *bytes, size_t size, cgb_image **out,
                              char *error, size_t error_size);

void cgb_image_free(cgb_image *image);

typedef struct {
    int songs;        /* 1 to 255 */
    int first_song;   /* the song to start with, counted from 0 */
    uint16_t load;    /* the address of the first byte of data */
    uint16_t init;
    uint16_t play;
    uint16_t sp;      /* the stack pointer at the start of each routine */
    uint8_t tma;
    uint8_t tac;      /* bit 2 set: the play rate is the timer's; clear: VBlank */
} cgb_header;

void cgb_image_header(const cgb_image *image, cgb_header *out);

/* The header's text fields, as C strings that live until the image is freed.
 * "?" marks an unknown value. */
const char *cgb_image_title(const cgb_image *image);
const char *cgb_image_author(const cgb_image *image);
const char *cgb_image_copyright(const cgb_image *image);

/* Clocks between calls of the play routine, from the header. A driver can
 * change the timer while it plays; the machine follows it. */
uint32_t cgb_image_tick_clocks(const cgb_image *image);

/* ---- Machines ---------------------------------------------------------- */

typedef struct cgb_machine cgb_machine;

/* The native clock: 4,194,304 cycles per second. */
#define CGB_CLOCK_HZ 4194304u

/*
 * Changes whenever the same input renders different samples or write streams.
 * Anything that caches rendered results keys them on this value.
 */
#define CGB_OUTPUT_REVISION 1

/*
 * A machine for `image` that produces `output_rate` frames per second (8000 to
 * 192000), at the image's first song. The image may be freed afterwards: the
 * machine keeps its own copy. NULL when the rate is out of range.
 */
cgb_machine *cgb_new(const cgb_image *image, int output_rate);
void cgb_free(cgb_machine *machine);

/* Resets the machine and starts `song`, counted from 0. Returns 1 on success
 * and 0 when the image has no such song. */
int cgb_select_song(cgb_machine *machine, int song);

/* Back to the start of the current song. */
void cgb_reset(cgb_machine *machine);

int cgb_song(const cgb_machine *machine);

/* Runs the machine until `frames` stereo frames exist and writes them,
 * interleaved left then right. Always fills the whole buffer. */
void cgb_render_s16(cgb_machine *machine, int16_t *out, size_t frames);
void cgb_render_f32(cgb_machine *machine, float *out, size_t frames);

/* Native cycles run since the song started. */
uint64_t cgb_cycles(const cgb_machine *machine);

/* Play calls begun since the song started. */
uint64_t cgb_frames(const cgb_machine *machine);

/* ---- Observation ------------------------------------------------------- */

/*
 * Callbacks for tracing what the driver does. They run on the rendering
 * thread, inside cgb_render_*, in program order. A callback may call
 * cgb_cycles, which returns the cycle of the event. Any other call that takes
 * the machine does nothing there and returns zero, NULL or an empty list
 * (cgb_warnings); cgb_snapshot_restore returns 3.
 *
 * sound_write sees every store to 0xFF10..0xFF3F (the sound registers and
 * wave RAM), whether or not the unit accepts it: the address and the value as
 * stored.
 *
 * play_call fires just before the machine calls a routine. `frame` is -1 for
 * the init routine and counts the play calls from 0 after it. Each play call
 * is the driver's own tick.
 */
typedef struct {
    void *context;
    void (*sound_write)(void *context, uint32_t address, uint32_t value);
    void (*play_call)(void *context, int64_t frame);
} cgb_observer;

/* Copies the observer in. Pass NULL to stop observing. */
void cgb_set_observer(cgb_machine *machine, const cgb_observer *observer);

/* ---- Inspection, for tests and tools ----------------------------------- */

/* Runs `cycles` native cycles. Sound produced meanwhile is discarded. */
void cgb_run_cycles(cgb_machine *machine, uint64_t cycles);

/* A, F, B, C, D, E, H, L by index 0 to 7; 8 is SP and 9 is PC. */
uint32_t cgb_register(const cgb_machine *machine, int index);

/* The address of the next instruction the CPU executes. */
uint32_t cgb_next_pc(const cgb_machine *machine);

/* Reads memory as the CPU would see it, with no side effects. */
uint32_t cgb_peek(const cgb_machine *machine, uint32_t address);

/*
 * Things the machine met and does not support (a routine that did not
 * return, HALT or STOP in a routine, an opcode that locks the CPU), as a
 * NUL-terminated list of short sentences, one per line. The machine reports
 * each kind once and never prints.
 */
const char *cgb_warnings(const cgb_machine *machine);

/* ---- Snapshots --------------------------------------------------------- */

/*
 * A copy of everything a machine is: the CPU, memory, timer, sound unit and
 * the sound that has not been handed out yet. It holds about 330 KB. A
 * snapshot restores into a machine that plays the same image at the same
 * output rate, and from there the same calls make the same sound. The
 * observer and the channel mask belong to the listener and are not part of
 * it.
 */
typedef struct cgb_snapshot cgb_snapshot;

cgb_snapshot *cgb_snapshot_take(const cgb_machine *machine);

/* 0 on success, 1 when the output rate differs, 2 when the image differs (or
 * the snapshot is NULL), 3 when called from an observer callback. */
int cgb_snapshot_restore(cgb_machine *machine, const cgb_snapshot *snapshot);

size_t cgb_snapshot_size(const cgb_snapshot *snapshot);
void cgb_snapshot_free(cgb_snapshot *snapshot);

/* ---- Channels ---------------------------------------------------------- */

/*
 * What each of the four channels puts out now, before the DAC: 0 to 15. The
 * order is the two pulse channels, the wave channel and the noise channel. A
 * muted channel still shows its level.
 */
void cgb_channel_levels_get(const cgb_machine *machine, int32_t out[4]);

/* Chooses the channels that reach the mix. Bit n is channel n + 1. All four
 * are on at the start, and a reset leaves the choice alone. */
void cgb_set_channel_mask(cgb_machine *machine, uint32_t mask);
uint32_t cgb_channel_mask(const cgb_machine *machine);

/* ---- The end of a song ------------------------------------------------- */

typedef enum {
    CGB_RUNNING = 0,        /* a channel is sounding, or may be */
    CGB_CHANNELS_OFF,       /* no channel is on; a later play call can start one */
    CGB_STALLED             /* a routine did not return, halted, or locked the CPU */
} cgb_end_state;

cgb_end_state cgb_end_state_get(const cgb_machine *machine);

/* Native cycles since the output level last changed. */
uint64_t cgb_quiet_cycles(const cgb_machine *machine);

#ifdef __cplusplus
}
#endif

#endif
