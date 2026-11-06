#ifndef CPCE_H
#define CPCE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A music-only PC Engine (TurboGrafx-16): the HuC6280 processor with its
 * memory map, its timer and interrupts, the sound generator of six channels,
 * and a stand-in for the video chip's vertical sync, running the code of an
 * HES file. The file holds the code and data of a game's sound driver. The
 * machine calls the file's request routine with a song number, and the
 * driver's interrupt handlers play the song from then on. The machine hands
 * back the sound as interleaved stereo at a rate the caller chooses.
 *
 * Two objects. An image is a parsed HES file. A machine runs the program of
 * one image at one song. Every piece of state lives in the machine, so several
 * machines can run side by side. With the same image, song and calls, a
 * machine produces the same output on every run.
 *
 * An HES file has no track list and no length. The caller picks the song
 * numbers (a playlist file next to the HES file names them) and decides when
 * to stop. The machine never ends: a driver that stops its music leaves the
 * machine playing silence.
 */

/* ---- Images ------------------------------------------------------------ */

typedef struct cpce_image cpce_image;

typedef enum {
    CPCE_OK = 0,
    CPCE_ERR_IO,     /* a file could not be opened or read */
    CPCE_ERR_FORMAT  /* not an HES file, a block that loads outside the ROM, or too large */
} cpce_status;

/*
 * Parses an .hes file. A NULL `path` or `out` returns CPCE_ERR_IO. On failure
 * *out is NULL and, when `error` is given, it holds a sentence that says why.
 * The path must name a regular file. A file larger than 16 MiB is refused
 * without being read in full.
 */
cpce_status cpce_image_load_hes(const char *path, cpce_image **out, char *error,
                                size_t error_size);

/* The same for a file already in memory. The bytes are copied. */
cpce_status cpce_image_from_hes(const uint8_t *bytes, size_t size,
                                cpce_image **out, char *error, size_t error_size);

void cpce_image_free(cpce_image *image);

/* The song that the file's header names as the first. */
uint32_t cpce_image_start_song(const cpce_image *image);

/* ---- Machines ---------------------------------------------------------- */

typedef struct cpce_machine cpce_machine;

/*
 * Starts the program at `song` (0 to 255, passed to the request routine in the
 * accumulator). Returns NULL when the image is NULL, the song is outside 0 to
 * 255 or the output rate is outside 8000 to 192000 Hz. The image is copied, so it can be freed at once.
 */
cpce_machine *cpce_new(const cpce_image *image, int output_rate, int song);
void cpce_free(cpce_machine *machine);

/* Starts the song again. The channel mask, the high-pass and the observer stay. */
void cpce_reset(cpce_machine *machine);

/*
 * Fills `frames` frames of interleaved left and right samples. The 16-bit form
 * clips at full scale. The float form does not clip, and its samples can pass
 * 1.0: loud songs reach about 1.1 of full scale, and a few reach more.
 */
void cpce_render_s16(cpce_machine *machine, int16_t *out, size_t frames);
void cpce_render_f32(cpce_machine *machine, float *out, size_t frames);

/* Time played so far, in samples at 44,100 Hz. */
uint64_t cpce_samples(const cpce_machine *machine);

/*
 * Chooses the channels that reach the mix: bit 0 is channel 1 and bit 5
 * channel 6. The default is 0x3F. A muted channel keeps running. Channels 5
 * and 6 can make noise.
 */
void cpce_set_channel_mask(cpce_machine *machine, uint64_t mask);
uint64_t cpce_channel_mask(const cpce_machine *machine);

/*
 * What each of the six channels puts out now, before the mask, the amplitude
 * controls and the mixer. A value is signed and lies within -31 to 31 (0 for a
 * channel that is off). A muted channel still shows its level.
 */
void cpce_channel_levels_get(const cpce_machine *machine, int32_t out[6]);

/*
 * Sets the cutoff in Hz of the output high-pass (20 Hz by default). It stands
 * for the console's output coupling. Returns 1 and sets it, or returns 0 and
 * changes nothing for a value outside 0 to 1000 Hz. 0 turns it off.
 */
int cpce_set_highpass_hz(cpce_machine *machine, double hz);
double cpce_highpass_hz(const cpce_machine *machine);

/* ---- Observation ------------------------------------------------------- */

/*
 * A callback that hears every store to the sound generator's registers, in
 * program order, at the time the chip gets it. It runs on the rendering
 * thread, inside cpce_render_*. It does not change the sound or the levels. An
 * observer set before the first render sees every store from the first.
 *
 * `reg` is 0 to 9: 0 selects a channel (0 to 5), 1 is the main level, 2 to 7
 * belong to the selected channel (frequency low and high, on/direct/level,
 * balance, waveform data, noise), 8 is the low frequency oscillator's
 * frequency and 9 its control. `channel` is the channel that register 0 has
 * selected, and after a store to register 0 it is the new one. `sample` is the
 * time of the store in samples of 44,100 Hz since the song started (or since
 * cpce_reset): a frame of 60 Hz is 735 samples.
 *
 * A callback may call cpce_samples, which returns the sample of the store. Any
 * other call that takes the machine does nothing there and returns zero or
 * NULL (cpce_snapshot_restore returns 3).
 */
typedef struct {
    void *context;
    void (*psg_write)(void *context, int channel, uint32_t reg, uint32_t value, uint64_t sample);
} cpce_observer;

/*
 * Copies the observer in. Pass NULL to stop observing. The observer belongs to
 * the listener: cpce_reset and cpce_snapshot_restore keep it, and a snapshot
 * does not hold it.
 */
void cpce_set_observer(cpce_machine *machine, const cpce_observer *observer);

/* ---- Snapshots --------------------------------------------------------- */

typedef struct cpce_snapshot cpce_snapshot;

/* Copies the machine's whole state, the channel mask included. */
cpce_snapshot *cpce_snapshot_take(const cpce_machine *machine);

/*
 * Returns the machine to a snapshot. Returns 0 on success, 2 when the snapshot
 * is NULL or comes from a machine with another image, song or output rate, and
 * 3 when called from an observer callback.
 */
int cpce_snapshot_restore(cpce_machine *machine, const cpce_snapshot *snapshot);

void cpce_snapshot_free(cpce_snapshot *snapshot);

#ifdef __cplusplus
}
#endif

#endif
