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
 * Two objects. An image is a parsed VGM, VGZ or GYM file. A machine plays the song
 * of one image. Every piece of state lives in the machine, so several machines
 * can run side by side. With the same image and calls, a machine produces the
 * same output on every run.
 *
 * A song with a loop point passes it again and again. The machine does not
 * stop and does not fade: the caller decides how many passes to play, and
 * cgenesis_image_loop_count says how many the file asks for. A song with no
 * loop ends, and the machine then renders silence.
 *
 * Only the SN76489, the YM2612 and the YM2413 play. A file that names other
 * chips is parsed, and those chips are silent. The YM2413's rhythm mode plays
 * the bass drum only.
 */

/* ---- Images ------------------------------------------------------------ */

typedef struct cgenesis_image cgenesis_image;

typedef enum {
    CGENESIS_OK = 0,
    CGENESIS_ERR_IO,     /* a file could not be opened or read */
    CGENESIS_ERR_FORMAT  /* not a VGM, VGZ or GYM file, or too large */
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

/*
 * The same for a .gym file, a log of the YM2612 and SN76489 writes of one
 * frame of the console at a time. A file can start with the "GYMX" header,
 * and its data can be zlib compressed. The file does not say where it came
 * from, so `pal` chooses the frame length and the clocks: 0 for a 60 Hz console,
 * anything else for a 50 Hz one. The image then reads like a VGM file: the tag
 * has the title, the game, the dumper (as the ripper) and the comment (as the
 * notes), and cgenesis_image_frame_rate returns 60 or 50. A byte where a command
 * should start that is not 0 to 3 fails with CGENESIS_ERR_FORMAT.
 */
cgenesis_status cgenesis_image_load_gym(const char *path, int pal,
                                        cgenesis_image **out, char *error,
                                        size_t error_size);

cgenesis_status cgenesis_image_from_gym(const uint8_t *bytes, size_t size,
                                        int pal, cgenesis_image **out,
                                        char *error, size_t error_size);

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

/* The chips that the file drives and the machine plays: bit 0 is the SN76489, bit 1 the YM2612, bit 2 the YM2413. */
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
 * a second SN76489 at bits 16 to 19, and the default then includes them. A file
 * with a YM2413 has its nine channels at bits 20 to 28 and a second YM2413 at
 * bits 29 to 37 (in rhythm mode bit 26 is the bass drum). Bits for a chip that
 * the file lacks are ignored.
 * A muted channel keeps running.
 */
void cgenesis_set_channel_mask(cgenesis_machine *machine, uint64_t mask);
uint64_t cgenesis_channel_mask(const cgenesis_machine *machine);

/*
 * What each channel of the first YM2612 and the first SN76489 puts out now,
 * before the mask, the panning and the mixer. The order follows the mask: the
 * six FM channels, then the SN76489's three tone channels and its noise
 * channel. An FM value is signed and lies within -320 to 320 (a full channel
 * is 256, and channel 6 shows the DAC sample while the DAC is on). A PSG value
 * is signed and lies within -8192 to 8192, and is 0 at attenuation 15. A muted
 * channel still shows its level. A chip that the file lacks reads as zeros,
 * a second chip has no entries, and neither has a YM2413.
 */
void cgenesis_channel_levels_get(const cgenesis_machine *machine, int32_t out[10]);

/*
 * What each voice is doing, for a display. The voices are the channels of the
 * chips that the file names, in the order of the channel mask and only for the
 * chips that are present: the first YM2612's six, the first SN76489's four,
 * the second YM2612's six, the second SN76489's four, then the YM2413's nine
 * and a second YM2413's nine. Call these on the render thread, right after a
 * render, and copy the result out; like every machine call they must not run
 * while another thread renders. They do nothing inside an observer callback.
 * The layout is the same in every core's header, so a viewer can treat the
 * cores alike. The fields:
 *
 *   active          1 while the voice is audible (its level is above -46 dB)
 *   key_on          1 while the voice is keyed on (the SN76489: its attenuation
 *                   is below 15)
 *   kind            0 tone (SN76489), 1 noise, 2 sample (the YM2612's DAC),
 *                   3 FM, 4 drum (the YM2413's bass drum)
 *   instrument      the YM2612's algorithm times 8 plus its feedback; the
 *                   YM2413's instrument register (0 the user tone, 1 to 15 the
 *                   built-in ones, 16 the bass drum); the SN76489 noise
 *                   channel's rate bits plus 4 for white noise; -1 for a tone
 *   frequency_hz    the pitch of the note: the F-number and block of the FM
 *                   chips (an operator's multiple is not in it), the SN76489's
 *                   clock over 32 times its register. 0 for noise and the DAC.
 *   volume          0 to 1: the envelope and the level register together
 *   pan             -1 left to +1 right, from the two sides
 *   envelope        0 to 1 for the FM chips (the loudest carrier's envelope),
 *                   -1 for the SN76489
 *   left, right     the volume to each side, 0 when the voice is not on it
 *   envelope_phase  0 attack, 1 decay, 2 sustain, 3 release (the YM2612's and
 *                   YM2413's carrier), 0 for the SN76489
 *   pitch_modulated, echo, ended  0
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
} cgenesis_voice;

/* The number of voices that the file's chips have. */
int cgenesis_voice_count(const cgenesis_machine *machine);

/* Writes at most `max` voices and returns how many. */
int cgenesis_voices_get(const cgenesis_machine *machine, cgenesis_voice *out, int max);

/* "FM 1", "PSG Tone 1", "PSG Noise", "OPLL 1", with " (2nd chip)" added for a
 * second chip, or NULL for an index outside the voices. The string lives as
 * long as the machine. */
const char *cgenesis_voice_name(const cgenesis_machine *machine, int index);

/*
 * Fills `out` with the last `frames` frames that one voice put out, oldest
 * first, as floats within -1 to 1 at the machine's output rate. The voice's own
 * level and panning are in it, and the channel mask and the mixer are not, so a
 * muted voice still shows. Each chip keeps 8,192 samples of its own rate (the
 * YM2612's 53 kHz and the YM2413's 50 kHz, and 44,100 Hz for the SN76489), so
 * 4,096 frames of an output rate of 44,100 Hz or more are all there, and
 * frames that reach further back are 0. Returns `frames`, or 0 for a voice
 * outside the voices, a NULL buffer, 0 frames, more than 16,384 frames or a
 * call from a callback.
 */
size_t cgenesis_scope_get(const cgenesis_machine *machine, int voice, float *out, size_t frames);

/*
 * Sets the cutoff in Hz of the high-pass that stands for the console's output
 * coupling and removes the DC offset that the DAC can leave. The default is
 * 20 Hz and 0 turns it off. Returns 1, or 0 and no change for a value outside
 * 0 to 1000.
 */
int cgenesis_set_highpass_hz(cgenesis_machine *machine, double hz);
double cgenesis_highpass_hz(const cgenesis_machine *machine);

/* ---- Observation ------------------------------------------------------- */

typedef enum {
    CGENESIS_CHIP_SN76489 = 0,
    CGENESIS_CHIP_SN76489_SECOND = 1,
    CGENESIS_CHIP_YM2612 = 2,
    CGENESIS_CHIP_YM2612_SECOND = 3,
    CGENESIS_CHIP_YM2413 = 4,
    CGENESIS_CHIP_YM2413_SECOND = 5
} cgenesis_chip;

/*
 * A callback that hears every register write that the machine applies to a
 * chip, in program order, at the time the chip gets it. It runs on the
 * rendering thread, inside cgenesis_render_*. It does not change the sound or
 * the levels. An observer set before the first render sees every write from
 * the first. A write to a chip that the file lacks is not applied and is not
 * reported.
 *
 * `address` is 0 for the SN76489's data port and 1 for its stereo port. For
 * the YM2612 it is the port number shifted left by 8, with the register: 0x02A
 * is the DAC register, written by the 0x8n commands and by the DAC streams as
 * well as by the 0x52 command, and 0x1B4 is register 0xB4 of port 1. For the
 * YM2413 it is the register. `sample` is the time of the write in samples of
 * 44,100 Hz since the song started (or since cgenesis_reset): one frame is 735
 * samples at 60 Hz and 882 at 50 Hz (see cgenesis_image_frame_rate).
 *
 * A callback may call cgenesis_samples, which returns the sample of the write.
 * Any other call that takes the machine does nothing there and returns zero or
 * NULL (cgenesis_snapshot_restore returns 3).
 */
typedef struct {
    void *context;
    void (*register_write)(void *context, int chip, uint32_t address, uint32_t value, uint64_t sample);
} cgenesis_observer;

/*
 * Copies the observer in. Pass NULL to stop observing. The observer belongs to
 * the listener: cgenesis_reset and cgenesis_snapshot_restore keep it, and a
 * snapshot does not hold it.
 */
void cgenesis_set_observer(cgenesis_machine *machine, const cgenesis_observer *observer);

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
