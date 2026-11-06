#ifndef CGBA_H
#define CGBA_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A music-only Game Boy Advance: the CPU, memory, timers, DMA, interrupts
 * and sound hardware, with a high-level BIOS and no picture. It runs a
 * game's own code so that the game's sound driver plays, and it hands back
 * the sound as interleaved stereo at a rate the caller chooses.
 *
 * Two objects. An image is what gets loaded: a GSF set (a stub file plus
 * its libraries) or a bare cartridge ROM. A machine runs one image. Every
 * piece of state lives in the machine, so several machines can run side by
 * side. A machine allocates once, when it is created, and never while it
 * renders. With the same image and the same calls, a machine produces the
 * same output on every run.
 */

/* ---- Images ------------------------------------------------------------ */

typedef struct cgba_image cgba_image;

typedef enum {
    CGBA_OK = 0,
    CGBA_ERR_IO,          /* a file could not be opened or read */
    CGBA_ERR_FORMAT,      /* not a PSF, wrong version, or a broken program */
    CGBA_ERR_ZLIB,        /* the compressed program did not inflate */
    CGBA_ERR_MEMORY,
    CGBA_ERR_UNSUPPORTED, /* a load address outside ROM and EWRAM */
    CGBA_ERR_LIBRARY      /* a _lib file was missing or nested too deep */
} cgba_status;

/*
 * Loads a .gsf or .minigsf and every library it names (_lib, _lib2, ...,
 * from the same directory). Libraries load first, depth first, and each
 * file's own data lands on top of them. A library name must be a plain file
 * name, and a set may open 64 files in all. `path` and `out` must not be
 * NULL. On failure *out is NULL and, when `error` is given, it holds a
 * sentence that says why.
 */
cgba_status cgba_image_load_gsf(const char *path, cgba_image **out,
                                char *error, size_t error_size);

/* A bare cartridge image (.gba), loaded at 0x08000000. The bytes are copied. */
cgba_status cgba_image_from_rom(const uint8_t *rom, size_t size, cgba_image **out);

void cgba_image_free(cgba_image *image);

/* A tag from the GSF set (title, game, artist, length, fade, ...), or NULL.
 * The stub's own tags come first; a library's tag fills in a missing one. */
const char *cgba_image_tag(const cgba_image *image, const char *key);

/* The length and fade tags in milliseconds, or -1 when absent or unreadable.
 * Both accept seconds ("75.5") and colon forms ("1:15.500", "0:01:15"). */
int32_t cgba_image_length_ms(const cgba_image *image);
int32_t cgba_image_fade_ms(const cgba_image *image);

uint32_t cgba_image_entry(const cgba_image *image);
size_t cgba_image_rom_size(const cgba_image *image);

/* ---- Machines ---------------------------------------------------------- */

typedef struct cgba_machine cgba_machine;

/* The native clock: 16,777,216 cycles per second. */
#define CGBA_CLOCK_HZ 16777216u

/*
 * Changes whenever the same input renders different samples or traces.
 * Anything that caches rendered results keys them on this value.
 */
#define CGBA_OUTPUT_REVISION 1

/*
 * A machine for `image` that produces `output_rate` frames per second
 * (8000 to 192000). The image may be freed afterwards: the machine keeps
 * its own copy. NULL when the rate is out of range. A failed allocation
 * ends the process.
 * The machine starts in the state the reference leaves a cartridge in.
 */
cgba_machine *cgba_new(const cgba_image *image, int output_rate);
void cgba_free(cgba_machine *machine);

/* Back to the state cgba_new left it in, with the same image. */
void cgba_reset(cgba_machine *machine);

/* Runs the machine until `frames` stereo frames exist and writes them,
 * interleaved left then right. Always fills the whole buffer. */
void cgba_render_s16(cgba_machine *machine, int16_t *out, size_t frames);
void cgba_render_f32(cgba_machine *machine, float *out, size_t frames);

/* Native cycles run since the last reset. */
uint64_t cgba_cycles(const cgba_machine *machine);

/* ---- Observation ------------------------------------------------------- */

/*
 * Callbacks for tracing what the game does to the sound hardware. They run
 * on the rendering thread, inside cgba_render_*, in program order. A callback
 * may call cgba_cycles, which returns the cycle of the event. Any other call
 * that takes the machine does nothing there and returns zero or NULL.
 *
 * sound_write sees every store to 0x04000060..0x040000A7, by the CPU, by
 * DMA or by a BIOS function, before the hardware masks or ignores it: the
 * address, the value as stored and the width of the bus access in bytes
 * (1, 2 or 4).
 *
 * fifo_sample sees every byte that Direct Sound FIFO A (channel 0) or B
 * (channel 1) shifts out on a timer overflow. An overflow that finds the
 * FIFO empty shifts nothing and is not reported.
 */
typedef struct {
    void *context;
    void (*sound_write)(void *context, uint32_t address, uint32_t value, int width);
    void (*fifo_sample)(void *context, int channel, int8_t sample);
} cgba_observer;

/* Copies the observer in. Pass NULL to stop observing. */
void cgba_set_observer(cgba_machine *machine, const cgba_observer *observer);

/* ---- Inspection, for tests and tools ----------------------------------- */

/* Runs `cycles` native cycles. Sound produced meanwhile is discarded. */
void cgba_run_cycles(cgba_machine *machine, uint64_t cycles);

/* r0..r15 of the current mode by index 0..15; 16 is CPSR. */
uint32_t cgba_register(const cgba_machine *machine, int index);

/* The address of the next instruction the CPU executes. */
uint32_t cgba_next_pc(const cgba_machine *machine);

/* The cycle at which the display next enters VBlank (line 160 begins).
 * A frame is 280,896 cycles, so later VBlanks follow at that spacing. */
uint64_t cgba_next_vblank(const cgba_machine *machine);

/* Reads memory as the CPU would see it from cartridge code, with no side
 * effects. Width is 1, 2 or 4. */
uint32_t cgba_peek(const cgba_machine *machine, uint32_t address, int width);

/*
 * Things the machine met and does not implement (an unknown BIOS call, an
 * undefined instruction), as a NUL-terminated list of short sentences, one
 * per line. The machine reports each once and never prints.
 */
const char *cgba_warnings(const cgba_machine *machine);

/* ---- Snapshots --------------------------------------------------------- */

/*
 * A copy of everything a machine is: the CPU, memory, timers, sound unit and
 * the sound that has not been handed out yet. It holds about 700 KB. A
 * snapshot restores into a machine that runs the same image at the same
 * output rate, and from there the same calls make the same sound. The
 * observer and the channel mask belong to the listener and are not part of it.
 */
typedef struct cgba_snapshot cgba_snapshot;

cgba_snapshot *cgba_snapshot_take(const cgba_machine *machine);

/* 0 on success, 1 when the output rate differs, 2 when the image differs, 3 when
 * called from an observer callback. */
int cgba_snapshot_restore(cgba_machine *machine, const cgba_snapshot *snapshot);

size_t cgba_snapshot_size(const cgba_snapshot *snapshot);
void cgba_snapshot_free(cgba_snapshot *snapshot);

/* ---- Channels ---------------------------------------------------------- */

/*
 * What each channel puts out now, before the mixer scales it: the tone, wave
 * and noise channels from 0 to 15, Direct Sound A and B from -128 to 127. A
 * muted channel still shows its level.
 */
typedef struct {
    int32_t psg[4];
    int32_t fifo[2];
} cgba_channel_levels;

void cgba_channel_levels_get(const cgba_machine *machine, cgba_channel_levels *out);

/*
 * Chooses the channels that reach the mix. Bits 0 to 3 are the tone, wave and
 * noise channels and bits 4 and 5 are Direct Sound A and B. All six are on at
 * the start, and a reset leaves the choice alone.
 */
void cgba_set_channel_mask(cgba_machine *machine, uint32_t mask);
uint32_t cgba_channel_mask(const cgba_machine *machine);

/* ---- Voices ------------------------------------------------------------ */

/*
 * What each channel is doing, for a display. The six voices are the channels
 * of the mask, in its order: Pulse 1 (the square channel with the sweep),
 * Pulse 2, Wave, Noise, Direct A and Direct B. Call these on the render
 * thread, right after a render, and copy the result out; like every machine
 * call they must not run while another thread renders. They do nothing inside
 * an observer callback. The layout is the same in every core's header, so a
 * viewer can treat the cores alike. Every field comes from the sound registers
 * and the channels' own state. The fields:
 *
 *   active          1 while the channel plays at a volume above 0 to at least
 *                   one side
 *   key_on          1 while the channel plays: for Pulse 1 to Noise, its bit
 *                   in SOUNDCNT_X (a trigger sets it; a length that runs out,
 *                   a DAC that is turned off, a sweep past 2047 or sound
 *                   turned off clears it); for Direct A and B, sound is on, a
 *                   side hears the FIFO (SOUNDCNT_H) and its timer runs
 *   kind            0 tone (Pulse 1, Pulse 2, Wave), 1 noise, 2 sample (Direct
 *                   A and B)
 *   instrument      the duty of a pulse channel (0 12.5%, 1 25%, 2 50%, 3
 *                   75%); for Wave, a number that is the same for equal
 *                   samples (a non-negative hash of the bank that plays, or of
 *                   both banks in 64-sample mode); for Noise, 1 for 7-bit
 *                   noise and 0 for 15-bit; -1 for Direct A and B
 *   frequency_hz    the pitch of the note from the frequency register X:
 *                   131072 / (2048 - X) for a pulse channel (the sweep's
 *                   changes to Pulse 1 are in it), 65536 / (2048 - X) for Wave
 *                   (32768 / (2048 - X) in 64-sample mode), 0 for Noise. For
 *                   Direct A and B, the rate of the timer that SOUNDCNT_H
 *                   selects: 16777216 / (65536 - reload) / prescaler, or for a
 *                   cascaded timer 1 the rate of timer 0 / (65536 - reload),
 *                   and 0 while that timer is stopped.
 *   volume          0 to 1, and 0 while the channel does not play: the
 *                   envelope volume over 15 for Pulse 1, Pulse 2 and Noise;
 *                   Wave's output level (0, 0.25, 0.5, 0.75 or 1); 0.5 or 1
 *                   for Direct A and B from their SOUNDCNT_H volume bits. The
 *                   mixer applies the master volume of SOUNDCNT_L and the
 *                   ratio of SOUNDCNT_H for the tone channels after the mask,
 *                   to the sum of the channels, so neither is in it.
 *   pan             -1 left to +1 right, from the two sides
 *   envelope        the envelope volume over 15 for Pulse 1, Pulse 2 and
 *                   Noise (0 while the channel does not play), -1 for Wave,
 *                   Direct A and Direct B
 *   left, right     the volume, to each side that the channel is on
 *                   (SOUNDCNT_L for Pulse 1 to Noise, SOUNDCNT_H for Direct A
 *                   and B), and 0 to a side that it is not on
 *   envelope_phase  0
 *   pitch_modulated, echo, ended  0
 *   mask_bit        the bit of the channel mask that mutes this channel (0 to
 *                   5, the voice's index)
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
} cgba_voice;

/* The number of voices: 6. */
int cgba_voice_count(const cgba_machine *machine);

/* Writes at most `max` voices and returns how many. */
int cgba_voices_get(const cgba_machine *machine, cgba_voice *out, int max);

/* "Pulse 1", "Pulse 2", "Wave", "Noise", "Direct A" and "Direct B", or NULL
 * for an index outside 0 to 5. The strings are static. */
const char *cgba_voice_name(const cgba_machine *machine, int index);

/*
 * Fills `out` with the last `frames` frames that one channel put out, oldest
 * first, as floats within -1 to 1 at the machine's output rate. A sample is
 * the channel's own output about the middle of its swing, as the average of
 * the left and right outputs, so a channel that only one side hears reads half
 * of a channel that both sides hear: Pulse 1 to Noise from their level (a
 * pulse channel at volume 15 swings from -1 to 1 when both sides hear it),
 * Direct A and B
 * from the last byte that the FIFO played, times its 50 or 100 percent. Pulse
 * 1 to Noise are 0 while they do not play, and Direct A and B keep their last
 * byte. The mask and the mixer's volumes are not in it, so a muted channel
 * still shows. The machine keeps 4,096 samples of 44,100 Hz (93 ms), each the
 * level that the sound unit held then, and frames that reach further back are
 * 0. Returns `frames`, or 0 for a channel outside 0 to 5, a NULL buffer, 0
 * frames, more than 16,384 frames or a call from a callback.
 */
size_t cgba_scope_get(const cgba_machine *machine, int voice, float *out, size_t frames);

/* ---- The end of a song ------------------------------------------------- */

typedef enum {
    CGBA_RUNNING = 0,            /* code runs, or waits for an interrupt that can come */
    CGBA_HALTED_WITHOUT_WAKE,    /* the CPU waits for an interrupt and none is enabled */
    CGBA_PARKED                  /* a branch to itself, and no interrupt can break in */
} cgba_end_state;

cgba_end_state cgba_end_state_get(const cgba_machine *machine);

/* Native cycles since the output level last changed. */
uint64_t cgba_quiet_cycles(const cgba_machine *machine);

#ifdef __cplusplus
}
#endif

#endif
