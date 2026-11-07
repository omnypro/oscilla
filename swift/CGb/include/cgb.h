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
 * The registers of the sound unit as a program wrote them, in blocks. The
 * count is 2 and does not change:
 *   0 "APU" (23 bytes, base 0xFF10): NR10 to NR52 at 0xFF10 to 0xFF26. Each
 *     byte is the last one stored to its address. The bytes are not read back:
 *     write-only bits show as written, the sweep unit's changes to NR13 and
 *     NR14 do not show, and NR52 is the whole byte stored. A store made while
 *     the sound unit is off is recorded, and turning the unit off does not clear
 *     the bytes. The unused addresses 0xFF15 and 0xFF1F are 0. A song change
 *     puts back the values that the boot program leaves (NR11 0x80, NR12 0xF3,
 *     NR50 0x77, NR51 0xF3, NR52 0x80) and 0 elsewhere.
 *   1 "Wave RAM" (16 bytes, base 0xFF30): the 16 bytes at 0xFF30 to 0xFF3F,
 *     which hold 32 samples of 4 bits, the high nibble first.
 * cgb_register_block_name returns a static string, the same pointer each call,
 * or NULL for an invalid block. cgb_register_block_base returns the address of
 * byte 0 in the console's memory map, or 0 for an invalid block.
 * cgb_register_block_get copies up to `max` bytes and returns the count, or 0
 * for an invalid block, a NULL machine or buffer, a `max` of 0 or a call from
 * an observer callback. The size, base, name and count calls return 0 or NULL
 * from a callback too. None of the calls change the sound.
 */
int cgb_register_block_count(const cgb_machine *machine);
const char *cgb_register_block_name(const cgb_machine *machine, int block);
uint32_t cgb_register_block_base(const cgb_machine *machine, int block);
size_t cgb_register_block_size(const cgb_machine *machine, int block);
size_t cgb_register_block_get(const cgb_machine *machine, int block, uint8_t *out, size_t max);

/*
 * Things the machine met and does not support (a routine that did not
 * return, HALT, STOP or a speed switch in a routine, an opcode that locks the
 * CPU), as a NUL-terminated list of short sentences, one per line. The machine
 * reports each kind once and never prints.
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

/* ---- Voices and scopes ------------------------------------------------- */

/*
 * What each channel is doing, for a display. The four voices are the channels
 * in the order of the channel mask: pulse 1 (the one with the sweep), pulse 2,
 * wave and noise. Call these on the render thread, right after a render, and
 * copy the result out; like every machine call they must not run while
 * another thread renders. They do nothing inside an observer callback. The
 * layout is the same in every core's header, so a viewer can treat the cores
 * alike. The fields:
 *
 *   active          1 while the channel sounds at a volume above 0
 *   key_on          1 while the channel's status bit in NR52 is on
 *   kind            0 tone (the pulses and the wave channel), 1 noise
 *   instrument      the pulse duty from NRx1 bits 7-6 (0 12.5%, 1 25%, 2 50%,
 *                   3 75%); for the wave channel a number that is the same for
 *                   equal wave RAM (a non-negative hash of the 32 samples); the
 *                   noise width from NR43 bit 3 (1 for the 7-bit mode)
 *   frequency_hz    the pitch of the note from the 11-bit period P:
 *                   131072 / (2048 - P) for the pulses, 65536 / (2048 - P) for
 *                   the wave channel, 0 for noise. The sweep rewrites the
 *                   period of pulse 1, so its changes are in it.
 *   volume          0 to 1: the envelope's volume over 15 for the pulses and
 *                   noise, the output level of NR32 (0, 1, 0.5 or 0.25) for
 *                   the wave channel. 0 while the channel does not sound: the
 *                   unit, the status bit or the DAC is off, or the length
 *                   counter has run out.
 *   pan             -1 left to +1 right, from NR51
 *   envelope        0 to 1 for the pulses and noise (the envelope's volume over
 *                   15, which keeps its value after the channel stops), -1 for
 *                   the wave channel
 *   left, right     the volume to each side, 0 when NR51 leaves the channel off
 *                   that side. The master volume of NR50 is not in them.
 *   envelope_phase, pitch_modulated, echo, ended  0
 *   mask_bit        the bit of the channel mask that mutes this voice
 */
typedef struct {
    int32_t active;
    int32_t key_on;
    int32_t kind;
    int32_t instrument;
    float frequency_hz;
    float volume;
    float pan;
    float envelope;
    float left;
    float right;
    int32_t envelope_phase;
    int32_t pitch_modulated;
    int32_t echo;
    int32_t ended;
    int32_t mask_bit;
    int32_t reserved[3];
} cgb_voice;

/* The number of voices: 4. */
int cgb_voice_count(const cgb_machine *machine);

/* Writes at most `max` voices and returns how many. */
int cgb_voices_get(const cgb_machine *machine, cgb_voice *out, int max);

/* "Pulse 1", "Pulse 2", "Wave" and "Noise", or NULL for an index outside 0 to
 * 3. The strings are static. */
const char *cgb_voice_name(const cgb_machine *machine, int index);

/*
 * Fills `out` with the last `frames` frames that one voice put out, oldest
 * first, as floats within -1 to 1 at the machine's output rate. The channel's
 * envelope, its output level and its NR51 routing are in it, and the channel
 * mask and the master volume are not, so a muted voice still shows. The
 * machine keeps 4,096 samples of 32,768 Hz (125 ms), so frames that reach
 * further back are 0. Returns `frames`, or 0 for a voice outside 0 to 3, a
 * NULL buffer, 0 frames, more than 16,384 frames or a call from a callback.
 */
size_t cgb_scope_get(const cgb_machine *machine, int voice, float *out, size_t frames);

/* ---- Bass cutoff ------------------------------------------------------- */

/*
 * The output passes a high-pass filter, and these calls choose its cutoff.
 * The default is the original console's own filter, which cuts below about
 * 28 Hz. A higher cutoff thins the bass. The calls work while a song plays and
 * the output stays continuous across the change. A reset, a song change and
 * cgb_snapshot_restore leave the choice alone.
 */
#define CGB_HIGHPASS_CONSOLE_HZ 28.04
#define CGB_HIGHPASS_MIN_HZ 1.0
#define CGB_HIGHPASS_MAX_HZ 1000.0

/* Sets the cutoff in Hz. Returns 1 when applied, and 0 with nothing changed
 * for a value outside CGB_HIGHPASS_MIN_HZ to CGB_HIGHPASS_MAX_HZ (NaN
 * included) or a call from an observer callback. */
int cgb_set_highpass_hz(cgb_machine *machine, double hz);

/* Goes back to the console's own filter. */
void cgb_use_console_highpass(cgb_machine *machine);

/* The cutoff in use: CGB_HIGHPASS_CONSOLE_HZ for the console's own filter. */
double cgb_highpass_hz(const cgb_machine *machine);

/* ---- The end of a song ------------------------------------------------- */

typedef enum {
    CGB_RUNNING = 0,        /* a channel is sounding, or may be */
    CGB_CHANNELS_OFF,       /* no channel is on; a later play call can start one */
    CGB_STALLED             /* a routine did not return, stopped, or locked the CPU */
} cgb_end_state;

cgb_end_state cgb_end_state_get(const cgb_machine *machine);

/* Native cycles since the output level last changed. */
uint64_t cgb_quiet_cycles(const cgb_machine *machine);

#ifdef __cplusplus
}
#endif

#endif
