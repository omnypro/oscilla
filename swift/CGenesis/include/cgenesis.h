#ifndef CGENESIS_H
#define CGENESIS_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A music-only Mega Drive (Genesis): the YM2612 FM chip and the SN76489 sound
 * chip, driven by the register writes of a VGM file. The file is a log of
 * writes with waits in samples of 44,100 Hz, so there is no CPU. The machine
 * hands back the sound as interleaved stereo at a rate the caller chooses.
 *
 * Two objects. An image is a parsed VGM or VGZ file. A machine plays the song
 * of one image. Every piece of state lives in the machine, so several machines
 * can run side by side. With the same image and calls, a machine produces the
 * same output on every run.
 *
 * A song with a loop point passes it again and again. The machine does not
 * stop and does not fade: the caller decides how many passes to play, and
 * cgenesis_image_loop_count says how many the file asks for. A song with no
 * loop ends, and the machine then renders silence.
 *
 * Only the SN76489 and the YM2612 play. A file that names other chips is
 * parsed, and those chips are silent.
 */

/* ---- Images ------------------------------------------------------------ */

typedef struct cgenesis_image cgenesis_image;

typedef enum {
    CGENESIS_OK = 0,
    CGENESIS_ERR_IO,     /* a file could not be opened or read */
    CGENESIS_ERR_FORMAT  /* not a VGM or VGZ file, or too large */
} cgenesis_status;

/*
 * Parses a .vgm or .vgz file. A NULL `path` or `out` returns CGENESIS_ERR_IO.
 * On failure *out is NULL and, when `error` is given, it holds a sentence that
 * says why. The path must name a regular file. A file larger than 64 MiB,
 * unpacked, is refused without being read in full.
 */
cgenesis_status cgenesis_image_load_vgm(const char *path, cgenesis_image **out,
                                        char *error, size_t error_size);

/* The same for a file already in memory. The bytes are copied. */
cgenesis_status cgenesis_image_from_vgm(const uint8_t *bytes, size_t size,
                                        cgenesis_image **out, char *error,
                                        size_t error_size);

void cgenesis_image_free(cgenesis_image *image);

/*
 * The fields of the GD3 tag, as UTF-8. Each is an empty string when the file
 * has no tag or leaves the field out. The strings belong to the image and stay
 * valid until it is freed. The _jp fields hold the Japanese text.
 */
const char *cgenesis_image_title(const cgenesis_image *image);
const char *cgenesis_image_title_jp(const cgenesis_image *image);
const char *cgenesis_image_game(const cgenesis_image *image);
const char *cgenesis_image_game_jp(const cgenesis_image *image);
const char *cgenesis_image_system(const cgenesis_image *image);
const char *cgenesis_image_system_jp(const cgenesis_image *image);
const char *cgenesis_image_author(const cgenesis_image *image);
const char *cgenesis_image_author_jp(const cgenesis_image *image);
const char *cgenesis_image_date(const cgenesis_image *image);
const char *cgenesis_image_ripper(const cgenesis_image *image);
const char *cgenesis_image_notes(const cgenesis_image *image);

/* The chips that the file drives and the machine plays: bit 0 is the SN76489, bit 1 the YM2612. */
uint32_t cgenesis_image_chips(const cgenesis_image *image);

/*
 * Lengths from the file's header, in samples of 44,100 Hz. A header can leave
 * them at 0 or state them wrongly. cgenesis_image_has_loop is 1 when the
 * stream has a loop point. The intro is the part before the loop point, or the
 * whole song when there is no loop.
 */
uint32_t cgenesis_image_total_samples(const cgenesis_image *image);
uint32_t cgenesis_image_loop_samples(const cgenesis_image *image);
uint32_t cgenesis_image_intro_samples(const cgenesis_image *image);
int cgenesis_image_has_loop(const cgenesis_image *image);

/*
 * The intro and the loop in frames of `rate`, rounded to the nearest frame.
 * The loop is 0 when the file has none. Both return -1 for a rate of 0 or less.
 */
int64_t cgenesis_image_intro_frames(const cgenesis_image *image, int rate);
int64_t cgenesis_image_loop_frames(const cgenesis_image *image, int rate);

/*
 * The number of passes through the loop that the file's loop base and loop
 * modifier ask for when a player would play `passes` of them. At least 1.
 */
uint32_t cgenesis_image_loop_count(const cgenesis_image *image, uint32_t passes);

/*
 * The gain that the file's volume modifier asks for, as a multiplier. A
 * machine applies it to its output already.
 */
double cgenesis_image_gain(const cgenesis_image *image);

/* The frame rate of the console that the file was recorded from, 50 or 60, or 0 when the header does not say. */
uint32_t cgenesis_image_frame_rate(const cgenesis_image *image);

/* ---- Machines ---------------------------------------------------------- */

typedef struct cgenesis_machine cgenesis_machine;

/*
 * Creates a machine for the song of `image` at `output_rate` Hz, from 8000 to
 * 192000. Returns NULL when the rate is out of range or the file drives
 * neither chip. The image can be freed afterwards.
 */
cgenesis_machine *cgenesis_new(const cgenesis_image *image, int output_rate);

void cgenesis_free(cgenesis_machine *machine);

/* Starts the song again. The channel mask and the high-pass stay. */
void cgenesis_reset(cgenesis_machine *machine);

/*
 * Renders `frames` frames of interleaved stereo. The 16-bit form clips at
 * full scale. The float form does not clip, and its samples can pass 1.0.
 */
void cgenesis_render_s16(cgenesis_machine *machine, int16_t *out, size_t frames);
void cgenesis_render_f32(cgenesis_machine *machine, float *out, size_t frames);

/* The time played so far in samples of 44,100 Hz. */
uint64_t cgenesis_samples(const cgenesis_machine *machine);

/* The passes through the loop point so far. */
uint32_t cgenesis_loops(const cgenesis_machine *machine);

/* 0 while the stream runs, 1 once it has ended without a loop. */
int cgenesis_end_state_get(const cgenesis_machine *machine);

/*
 * Chooses the channels that reach the mix. Bits 0 to 5 are the YM2612's
 * channels 1 to 6 (channel 6 also carries the DAC), and bits 6 to 9 are the
 * SN76489's three tone channels and its noise channel. The default is 0x3FF.
 * A file with a second YM2612 has its channels at bits 10 to 15 and a file with
 * a second SN76489 at bits 16 to 19, and the default then includes them. Bits
 * for a chip that the file lacks are ignored.
 * A muted channel keeps running.
 */
void cgenesis_set_channel_mask(cgenesis_machine *machine, uint64_t mask);
uint64_t cgenesis_channel_mask(const cgenesis_machine *machine);

/*
 * Sets the cutoff in Hz of the high-pass that stands for the console's output
 * coupling and removes the DC offset that the DAC can leave. The default is
 * 20 Hz and 0 turns it off. Returns 1, or 0 and no change for a value outside
 * 0 to 1000.
 */
int cgenesis_set_highpass_hz(cgenesis_machine *machine, double hz);
double cgenesis_highpass_hz(const cgenesis_machine *machine);

/* ---- Snapshots --------------------------------------------------------- */

typedef struct cgenesis_snapshot cgenesis_snapshot;

/* Copies the machine's whole state, the channel mask included. */
cgenesis_snapshot *cgenesis_snapshot_take(const cgenesis_machine *machine);

/*
 * Returns the machine to a snapshot. Returns 0 on success and 2 when the
 * snapshot is NULL or comes from a machine with another image or output rate.
 */
int cgenesis_snapshot_restore(cgenesis_machine *machine, const cgenesis_snapshot *snapshot);

void cgenesis_snapshot_free(cgenesis_snapshot *snapshot);

#ifdef __cplusplus
}
#endif

#endif
