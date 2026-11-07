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
 * triangle, the noise channel and the delta channel) and the channels of the
 * VRC6, VRC7, Sunsoft 5B, Namco 163, Famicom Disk System, MMC5 or VT02
 * expansion chip. A
 * file that asks for other expansion sound gets the channels it can play, and
 * cnes_warnings says so.
 */

/* ---- Images ------------------------------------------------------------ */

typedef struct cnes_image cnes_image;

typedef enum {
    CNES_OK = 0,
    CNES_ERR_IO,     /* a file could not be opened or read */
    CNES_ERR_FORMAT  /* not an NSF or NSFe file, wrong version, malformed, or too large */
} cnes_status;

/*
 * Parses an .nsf or .nsfe file, by the first bytes of the file. A NULL `path`
 * or `out` returns CNES_ERR_IO. On failure
 * *out is NULL and, when `error` is given, it holds a sentence that says why.
 * The path must name a regular file. A file larger than 4 MiB is refused
 * without being read in full.
 */
cnes_status cnes_image_load_nsf(const char *path, cnes_image **out,
                                char *error, size_t error_size);

/* The same for a file already in memory (an NSF or an NSFe file). The bytes
 * are copied. */
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
                             64 VT02. The core plays the VRC6, the VRC7, the FDS, the MMC5, the N163, the 5B and the VT02. */
} cnes_header;

void cnes_image_header(const cnes_image *image, cnes_header *out);

/* The header's text fields, as C strings that live until the image is freed. */
const char *cnes_image_title(const cnes_image *image);
const char *cnes_image_author(const cnes_image *image);
const char *cnes_image_copyright(const cnes_image *image);

/*
 * More of the file's text and the data that an NSFe file adds. Strings are
 * UTF-8 in an NSFe file; the fields of an NSF header hold whatever bytes the
 * file's maker wrote. A field that the file leaves out is the empty string, so
 * a missing field and an empty one look the same. Each pointer lives until the
 * image is freed. A NULL image gives NULL (or -1 or 0 as noted).
 *
 * An NSF file has none of this beyond the three fields above, unless it is a
 * version 2 file that carries NSFe chunks after its program. Without them the
 * ripper and the comment are empty, every song name and song author is empty,
 * the playlist is empty and every time and fade is -1. These calls read the
 * same data for an NSF file with chunks as for an NSFe file, so a player
 * should not choose between them with cnes_image_is_nsfe. That call tells
 * only the container.
 *
 * `song` counts from 0. cnes_image_song_title and cnes_image_song_author
 * return NULL for a song outside the file.
 *
 * Times are milliseconds. A song's time is how long it plays before the fade
 * starts, and its fade is the length of the fade. -1 means that the file gives
 * no value (a negative value in the file counts as none). The length is the
 * two added, or the time alone when the fade is missing, and -1 when the time
 * is missing.
 *
 * The playlist is the order that the file asks a player to follow, as song
 * numbers. Entries that name a song the file does not have are left out.
 * cnes_image_playlist_get returns -1 for an index outside the list.
 *
 * The core reads the NSFe chunks mixe, psfx and VRC7 and does not use them:
 * levels are the ones the core documents, and the VRC7 plays the 15 patches of
 * the chip. Of the
 * regn chunk it uses the NTSC and PAL bits and the preferred region; Dendy has
 * no region of its own, so a file that names only Dendy keeps its INFO bits.
 */
int cnes_image_is_nsfe(const cnes_image *image);
const char *cnes_image_ripper(const cnes_image *image);
const char *cnes_image_comment(const cnes_image *image);
const char *cnes_image_song_title(const cnes_image *image, int song);
const char *cnes_image_song_author(const cnes_image *image, int song);
int32_t cnes_image_song_time_ms(const cnes_image *image, int song);
int32_t cnes_image_song_fade_ms(const cnes_image *image, int song);
int64_t cnes_image_song_length_ms(const cnes_image *image, int song);
int cnes_image_playlist_count(const cnes_image *image);
int cnes_image_playlist_get(const cnes_image *image, int index);

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
 * The registers of the chips as the program wrote them, in blocks: the bytes
 * of its stores. A read of a register would hide its write-only bits. A
 * file has the 2A03's block and one block for each expansion chip that its
 * header names, in this order:
 *
 *   name        size  base    bytes
 *   "2A03 APU"    24  0x4000  0x4000 to 0x4017. The player's own stores
 *                             (0x0F to 0x4015, 0x40 to 0x4017) are in them.
 *   "VT02"        22  0x4020  0x4020 to 0x4035
 *   "VRC6"        12  0x9000  0 to 3 are 0x9000 to 0x9003, 4 to 7 are 0xA000
 *                             to 0xA003, 8 to 11 are 0xB000 to 0xB003. A store
 *                             to an alias of an address (0x9004) is in the
 *                             byte of the address that the chip decodes.
 *   "5B"          16  0       the YM2149-style register file, registers 0 to 15
 *   "N163"       128  0       the chip's internal RAM: the waveforms, and the
 *                             channel registers at 0x40 to 0x7F
 *   "FDS"         88  0x4040  0x4040 to 0x4097. The first 64 bytes are the
 *                             wave RAM as the chip holds it: 6 bits in each
 *                             byte, and unchanged by a store while the chip
 *                             has writes to it disabled.
 *   "MMC5"        22  0x5000  0x5000 to 0x5015
 *   "VRC7"        54  0       the internal registers 0x00 to 0x35 as written
 *                             through 0x9010 and 0x9030. The addresses that
 *                             the chip does not decode read 0. A store of
 *                             0x80 or more to 0xE000 clears them.
 *
 * Byte i of a block is the register at the base plus i. The count and the
 * index of a block depend on the chips, so a caller looks a block up by its
 * name. A song change or a reset clears the bytes, and a snapshot holds them.
 *
 * cnes_register_block_name returns a static string, the same pointer on every
 * call, or NULL for an invalid block. cnes_register_block_base returns the
 * address of byte 0 in the chip's own register space. cnes_register_block_get
 * copies up to `max` bytes and returns the count, or 0 for an invalid block, a
 * NULL machine or buffer, a `max` of 0 or a call from an observer callback.
 * The other calls return 0 or NULL in a callback too. None of the calls change
 * the sound.
 */
int cnes_register_block_count(const cnes_machine *machine);
const char *cnes_register_block_name(const cnes_machine *machine, int block);
uint32_t cnes_register_block_base(const cnes_machine *machine, int block);
size_t cnes_register_block_size(const cnes_machine *machine, int block);
size_t cnes_register_block_get(const cnes_machine *machine, int block, uint8_t *out, size_t max);

/*
 * Things the machine met and does not support (a routine that did not
 * return, an opcode that locks the CPU or that the core does not run, BRK,
 * expansion sound other than the VRC6, the VRC7, the FDS, the MMC5, the
 * Namco 163, the Sunsoft 5B and the VT02), as a NUL-terminated list of short sentences, one per line.
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
 * 5B and not the VRC6 shows the 5B's three channels (0 to 31). A file with the
 * Namco 163 and neither of those shows its eight channels, channel 1 first, as
 * the size of each channel's last output (0 to 120). A file with the FDS and
 * none of those shows the chip's output level in the first entry (0 to 2016).
 * A file with the MMC5 and none of those shows its pulse 1 and pulse 2 (0 to
 * 15) and its sample register (0 to 255) in the first three entries. A file
 * with the VRC7 and neither the VRC6 nor the Namco 163 shows the size of the
 * last output of its six channels (0 to about 4100) in the first six entries.
 * A file with the VT02 and none of those shows the second generator's pulse 3,
 * pulse 4, triangle 2 and noise 2 (each 0 to 15) and its raw PCM level (0 to
 * 255) in the first five entries. The other entries, and all
 * of them for a file without a chip that the core plays, are 0. A muted
 * channel still shows its level.
 */
void cnes_expansion_levels_get(const cnes_machine *machine, int32_t out[8]);

/* Chooses the channels that reach the mix. Bit n is channel n in the order
 * above for bits 0 to 4. Bits 5 to 7 are the VRC6's pulse 1, pulse 2 and saw,
 * bits 8 to 10 are the Sunsoft 5B's three channels, bits 11 to 18 are the
 * Namco 163's channels 1 to 8, bit 19 is the FDS's channel, bits 20 and 21 are
 * the MMC5's pulse 1 and pulse 2, bit 22 is its sample register, and bits 23
 * to 28 are the VRC7's channels 1 to 6. Bits 29 and 30 are the VT02's pulse 3
 * and pulse 4, bit 31 is its triangle 2, bit 32 is its noise 2 and bit 33 is
 * its raw PCM. Every channel is on at the start, and a reset leaves the choice
 * alone. Only those thirty-four bits count, and cnes_channel_mask returns
 * 0x3FFFFFFFF at the start. The mask is 64 bits wide because 34 channels do
 * not fit in 32. */
void cnes_set_channel_mask(cnes_machine *machine, uint64_t mask);
uint64_t cnes_channel_mask(const cnes_machine *machine);

/* ---- Voices and scopes ------------------------------------------------- */

/*
 * What each voice is doing, for a display. The voices are the channels of the
 * chips that the file has, in the order of the channel mask, and only for the
 * chips that the header's `chips` byte names: the 2A03's pulse 1, pulse 2,
 * triangle, noise and DMC (every file has them), then the VRC6's pulse 1,
 * pulse 2 and saw, the Sunsoft 5B's three channels, the Namco 163's channels
 * 1 to 8, the FDS's channel, the MMC5's pulse 1, pulse 2 and sample register,
 * the VRC7's six channels, and the VT02's pulse 3, pulse 4, triangle 2,
 * noise 2 and raw PCM. A voice's index is not its mask bit when a chip before
 * it is missing, so mask_bit gives the bit. Call these on the render thread,
 * right after a render, and copy the result out; like every machine call they
 * must not run while another thread renders. They do nothing inside an
 * observer callback. The layout is the same in every core's header, so a
 * viewer can treat the cores alike. The fields:
 *
 *   active          1 while the voice makes sound: its volume is above 0 (for
 *                   the VRC7, above -46 dB). A sample voice is active while
 *                   the DMC plays sample bits or has bytes left, or while the
 *                   MMC5's or the VT02's PCM level is not 0.
 *   key_on          1 while the channel is on as its chip defines it: a
 *                   length counter above 0 (the 2A03, MMC5 and VT02 pulses and
 *                   noise channels, and the DMC's bytes left), the length and
 *                   linear counters both above 0 (the triangles), the enable
 *                   bit (VRC6), a frequency above 0 in an enabled channel
 *                   (N163), the wave running at a frequency above 0 (FDS),
 *                   the key bit (VRC7), the envelope, or a volume above 0
 *                   with the tone or the noise on (5B), a level above 0 (MMC5
 *                   PCM), the raw PCM mode with bytes left (VT02 PCM)
 *   kind            0 tone (the pulses, the triangles, the saw, the 5B's
 *                   channels and the wavetables of the N163 and the FDS),
 *                   1 noise (the noise channels, and a 5B channel with its
 *                   noise on and its tone off), 2 sample (the DMC and the
 *                   MMC5's and the VT02's PCM), 3 FM (the VRC7). The core
 *                   has no voice of kind 4 (drum).
 *   instrument      the duty that sounds for a 2A03, MMC5 or VT02 pulse (0
 *                   12.5%, 1 25%, 2 50%, 3 75%; the VT02 swaps the register's
 *                   25% and 50% settings, and the field gives the one that
 *                   sounds); the duty setting of a VRC6 pulse (0 to 7, a
 *                   sixteenth more each), or 8 in the mode that ignores the
 *                   duty; the noise mode, 0 or 1; for the N163 and the FDS a
 *                   non-negative number that is the same for equal waveforms;
 *                   the VRC7's patch (0 the custom patch, 1 to 15 the built-in
 *                   ones); -1 for the others
 *   frequency_hz    the pitch that the period register names, from the CPU
 *                   clock of the machine's region (CPU below): CPU / (16 (P +
 *                   1)) for a 2A03, MMC5 or VT02 pulse, CPU / (32 (P + 1)) for
 *                   a triangle, CPU / (16 (P' + 1)) for a VRC6 pulse and CPU /
 *                   (14 (P' + 1)) for the saw, where P' is the period shifted
 *                   right as 0x9003 asks, CPU / (32 P) for a 5B tone (a period
 *                   of 0 reads as 1), CPU F / (15 n L 65536) for an N163
 *                   channel with n channels enabled and a waveform of L
 *                   samples, CPU F / 4194304 for the FDS (the modulation
 *                   unit's change is not in it), and F 2^B 49716 / 2^19 for
 *                   the VRC7 on both regions, because its clock is its own.
 *                   0 for noise, samples and a 5B channel with its tone off.
 *   volume          0 to 1: the volume register or the envelope over its
 *                   largest value, and 0 while the chip keeps the channel
 *                   silent (a length or linear counter at 0, a pulse period
 *                   below 8 or a sweep target past 0x7FF, an enable bit
 *                   clear, the VRC6's halt bit, the VT02's switch of the
 *                   channel's generator off). A triangle's is 1 or 0. The
 *                   5B's follows its volume curve. The FDS's is the gain
 *                   times the master volume. The VRC7's is the carrier's
 *                   envelope times the channel's volume register, each turned
 *                   from decibels into a linear level. A VRC6 saw's is its
 *                   rate over 42, the rate past which the accumulator
 *                   overflows. A sample voice's is its level over its full
 *                   scale (127 for the DMC, 255 for PCM).
 *   pan             0: the machine is mono
 *   envelope        0 to 1 while the volume comes from an envelope generator:
 *                   the decay level of a 2A03, MMC5 or VT02 pulse or noise
 *                   channel whose constant-volume bit is clear, the FDS's gain
 *                   while its volume envelope runs, the 5B's envelope for a
 *                   channel that uses it, and the VRC7 carrier's envelope.
 *                   -1 otherwise.
 *   left, right     the volume
 *   envelope_phase  the VRC7 carrier's stage: 0 attack, 1 decay, 2 sustain,
 *                   3 release. 0 for the other chips, which have no stages.
 *   pitch_modulated 1 while something besides the period register moves the
 *                   pitch: the FDS's modulation unit (its gain is above 0 and
 *                   it runs or holds a counter that is not 0), or the vibrato
 *                   bit of a VRC7 carrier's patch
 *   echo, ended     0
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
} cnes_voice;

/* The number of voices that the file's chips have: 5 and the channels of each
 * expansion chip that the header names. */
int cnes_voice_count(const cnes_machine *machine);

/* Writes at most `max` voices and returns how many. */
int cnes_voices_get(const cnes_machine *machine, cnes_voice *out, int max);

/* "Pulse 1", "Pulse 2", "Triangle", "Noise", "DMC", "VRC6 Pulse 1", "VRC6
 * Saw", "5B 1", "N163 1", "FDS", "MMC5 Pulse 1", "MMC5 PCM", "VRC7 1", "VT02
 * Pulse 3", "VT02 Triangle 2", "VT02 Noise 2", "VT02 PCM" and the rest in the
 * same pattern, or NULL for an index outside the voices. The string lives as
 * long as the machine. */
const char *cnes_voice_name(const cnes_machine *machine, int index);

/*
 * Fills `out` with the last `frames` frames that one voice put out, oldest
 * first, as floats within -1 to 1 at the machine's output rate. A tone or
 * noise voice swings about 0 at its own level and is 0 while the chip keeps
 * it silent; a triangle that its counters stopped is 0 too, although the chip
 * holds its last step. A sample voice is its level from 0. The channel mask
 * and the mixer are not in it, so a muted voice still shows. The machine keeps
 * 4,096 samples of 44,100 Hz (93 ms), each the channels' state at its moment,
 * so frames that reach further back are 0. Returns `frames`, or 0 for a voice
 * outside the voices, a NULL buffer, 0 frames, more than 16,384 frames or a
 * call from a callback.
 */
size_t cnes_scope_get(const cnes_machine *machine, int voice, float *out, size_t frames);

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
