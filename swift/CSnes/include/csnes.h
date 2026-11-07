#ifndef CSNES_H
#define CSNES_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A music-only SNES: the SPC700, the S-DSP and their 64 KiB of RAM, started
 * from the state that an SPC file holds. The machine plays a song by running
 * the sound driver as the file left it, and it hands back the sound as
 * interleaved stereo at a rate the caller chooses. The DSP makes 32,000
 * samples a second, and the machine converts them to the output rate.
 *
 * Two objects. An image is a parsed SPC file. A machine plays the song of one
 * image. Every piece of state lives in the machine, so several machines can run
 * side by side. A machine allocates once, when it is created, and never while
 * it renders. With the same image and calls, a machine produces the same
 * output on every run.
 *
 * An SPC file holds a moment of a running driver, not a program to start. It
 * has no voice state, so a voice that sounded when the file was made starts
 * again at the driver's next key-on. The main CPU is not part of the machine:
 * the four input ports start with the bytes at 0xF4 to 0xF7 of the file's RAM, and csnes_write_port sets them.
 */

/* ---- Images ------------------------------------------------------------ */

typedef struct csnes_image csnes_image;

typedef enum {
    CSNES_OK = 0,
    CSNES_ERR_IO,     /* a file could not be opened or read */
    CSNES_ERR_FORMAT  /* not an SPC file, or too large */
} csnes_status;

/*
 * Parses an .spc file. A NULL `path` or `out` returns CSNES_ERR_IO. On failure
 * *out is NULL and, when `error` is given, it holds a sentence that says why.
 * The path must name a regular file. A file larger than 1 MiB is refused
 * without being read in full.
 */
csnes_status csnes_image_load_spc(const char *path, csnes_image **out,
                                  char *error, size_t error_size);

/* The same for a file already in memory. The bytes are copied. */
csnes_status csnes_image_from_spc(const uint8_t *bytes, size_t size, csnes_image **out,
                                  char *error, size_t error_size);

void csnes_image_free(csnes_image *image);

/*
 * The text fields of the ID666 tag, or of the xid6 chunk when the file has one.
 * Each is an empty string when the file leaves it out. The strings belong to
 * the image and stay valid until it is freed. The bytes are as the file holds
 * them, usually ASCII or Shift-JIS.
 */
const char *csnes_image_song(const csnes_image *image);
const char *csnes_image_game(const csnes_image *image);
const char *csnes_image_artist(const csnes_image *image);
const char *csnes_image_dumper(const csnes_image *image);
const char *csnes_image_comments(const csnes_image *image);

/*
 * The time before the fade starts and the fade, in milliseconds, or -1 when
 * the file gives none. The xid6 lengths win over the ID666 seconds: the intro,
 * the loop as many times as the file says (once when it says nothing) and the
 * end. csnes_image_length_ms adds the two, or returns -1 when the file gives
 * no time.
 */
int64_t csnes_image_time_ms(const csnes_image *image);
int64_t csnes_image_fade_ms(const csnes_image *image);
int64_t csnes_image_length_ms(const csnes_image *image);

/*
 * The voices the file asks a player to leave silent, one bit for each voice
 * (bit n is voice n). A set bit comes from the xid6 chunk when there is one
 * and otherwise from the ID666 tag. The machine does not apply it: the caller
 * does, through csnes_set_channel_mask.
 */
uint32_t csnes_image_muted_voices(const csnes_image *image);

/* The output gain that the xid6 chunk asks for. 65536 is the unchanged
 * output, and the value is 65536 when the file gives none. The machine does
 * not apply it. */
uint32_t csnes_image_amplification(const csnes_image *image);

/* ---- Machines ---------------------------------------------------------- */

typedef struct csnes_machine csnes_machine;

/* The SPC700 clock in cycles per second. The DSP makes a sample every 32. */
#define CSNES_CLOCK_HZ 1024000
#define CSNES_SAMPLE_RATE 32000

/*
 * Changes whenever the same input renders different samples or write streams.
 * Anything that caches rendered results keys them on this value.
 */
#define CSNES_OUTPUT_REVISION 1

/*
 * A machine for `image` that produces `output_rate` frames per second (8000 to
 * 192000), at the state the file holds. The image may be freed afterwards: the
 * machine keeps its own copy. NULL when the rate is out of range.
 */
csnes_machine *csnes_new(const csnes_image *image, int output_rate);

void csnes_free(csnes_machine *machine);

/* Back to the state the file holds. */
void csnes_reset(csnes_machine *machine);

/* Runs the machine until `frames` stereo frames exist and writes them,
 * interleaved left then right. Always fills the whole buffer. The output is
 * the DSP's own mix at unit gain. A loud song can pass full scale: the 16-bit
 * output clamps there, and the float output goes past 1.0. A host that needs
 * headroom scales the samples. */
void csnes_render_s16(csnes_machine *machine, int16_t *out, size_t frames);
void csnes_render_f32(csnes_machine *machine, float *out, size_t frames);

/* SPC700 cycles run since the machine started. */
uint64_t csnes_cycles(const csnes_machine *machine);

/* DSP samples made since the machine started. */
uint64_t csnes_samples(const csnes_machine *machine);

/* ---- The main CPU's ports ---------------------------------------------- */

/*
 * The SPC700 talks to the main CPU through four ports at 0xF4 to 0xF7. A
 * driver reads the values that the main CPU wrote and writes values of its
 * own. csnes_write_port sets what the SPC700 reads from port 0 to 3 (the main
 * CPU's side). csnes_read_port returns what the SPC700 wrote there last. The
 * inputs start with the bytes at 0xF4 to 0xF7 of the file's RAM. A port number outside 0 to 3 does nothing and reads 0.
 */
void csnes_write_port(csnes_machine *machine, int port, uint32_t value);
uint32_t csnes_read_port(const csnes_machine *machine, int port);

/* ---- Observation ------------------------------------------------------- */

/*
 * Callbacks for tracing what the driver does. They run on the rendering
 * thread, inside csnes_render_*, in program order. A callback may call
 * csnes_cycles, which returns the SPC700 clock at the end of the instruction
 * that made the store. Any other call that takes the machine does nothing there
 * and returns zero or NULL; csnes_snapshot_restore returns 3.
 *
 * dsp_write sees every store to a DSP register through ports 0xF2 and 0xF3:
 * the register number and the value. port_write sees every store to one of
 * the four output ports: the port number 0 to 3 and the value.
 *
 * ram_write sees the SPC700's stores to the addresses that csnes_watch_ram
 * names: the address and the value, in program order, at the clock of the end
 * of the instruction. The DSP's own writes (the echo buffer) do not show.
 */
typedef struct {
    void *context;
    void (*dsp_write)(void *context, uint32_t reg, uint32_t value);
    void (*port_write)(void *context, uint32_t port, uint32_t value);
    void (*ram_write)(void *context, uint32_t address, uint32_t value);
} csnes_observer;

/* Copies the observer in. Pass NULL to stop observing. */
void csnes_set_observer(csnes_machine *machine, const csnes_observer *observer);

/* Chooses the RAM addresses `first` to `last` (both included) that ram_write
 * reports. A `first` above `last` or above 0xFFFF turns the report off, which
 * is the start state; a `last` above 0xFFFF counts as 0xFFFF. A snapshot
 * restore keeps the range. */
void csnes_watch_ram(csnes_machine *machine, uint32_t first, uint32_t last);

/* ---- Inspection, for tests and tools ----------------------------------- */

/* Runs `cycles` SPC700 cycles. Sound produced meanwhile is discarded. */
void csnes_run_cycles(csnes_machine *machine, uint64_t cycles);

/* A, X, Y, PSW, SP by index 0 to 4; 5 is PC. */
uint32_t csnes_register(const csnes_machine *machine, int index);

/* Reads sound RAM with no side effects. The boot ROM and the ports do not
 * show: the byte is the RAM under them. */
uint32_t csnes_peek(const csnes_machine *machine, uint32_t address);

/* Copies all 65536 bytes of sound RAM into `out` with the rules of
 * csnes_peek: no side effects, and the RAM under the boot ROM and the ports.
 * Does nothing if `out` is NULL. */
void csnes_ram_copy(const csnes_machine *machine, uint8_t *out);

/* A DSP register, 0 to 127. A register from 128 to 255 reads as the one 128
 * below it. */
uint32_t csnes_dsp_register(const csnes_machine *machine, uint32_t reg);

/* ---- Snapshots --------------------------------------------------------- */

/*
 * A copy of everything a machine is: the SPC700, RAM, DSP and the sound that
 * has not been handed out yet. A snapshot restores into a machine that plays
 * the same image at the same output rate, and from there the same calls make
 * the same sound. The observer and the channel mask belong to the listener and
 * are not part of it.
 */
typedef struct csnes_snapshot csnes_snapshot;

csnes_snapshot *csnes_snapshot_take(const csnes_machine *machine);

/* 0 on success, 1 when the output rate differs, 2 when the image differs (or
 * the snapshot is NULL), 3 when called from an observer callback. */
int csnes_snapshot_restore(csnes_machine *machine, const csnes_snapshot *snapshot);

size_t csnes_snapshot_size(const csnes_snapshot *snapshot);
void csnes_snapshot_free(csnes_snapshot *snapshot);

/* ---- Voices ------------------------------------------------------------ */

/* What each of the eight voices puts out now, before its volumes: the high
 * byte of the enveloped sample, from -128 to 127. */
void csnes_voice_levels_get(const csnes_machine *machine, int32_t out[8]);

/*
 * What a voice is doing, for a display. Call these on the render thread, right
 * after a render, and copy the result out; like every machine call they must
 * not run while another thread renders. They do nothing inside an observer
 * callback. The layout is the same in every core's header, so a viewer can
 * treat the cores alike. The fields:
 *
 *   active          1 while the voice makes sound (its envelope is above 0)
 *   key_on          1 while the voice is keyed on (attack, decay or sustain)
 *   kind            1 for noise, 2 for a sample (0 tone, 3 FM and 4 drum are
 *                   for the other cores)
 *   instrument      the source number, the index into the sample directory
 *   frequency_hz    the rate at which the sample plays: 32000 * pitch / 4096.
 *                   The pitch of the note depends on the sample, so this is not
 *                   a note frequency. 0 for noise.
 *   volume          0 to 1: the larger of the two volume registers over 128,
 *                   times the envelope
 *   pan             -1 left to +1 right, from the two volume registers
 *   envelope        0 to 1
 *   left, right     each volume register over 128, times the envelope; negative
 *                   when the register is
 *   envelope_phase  0 attack, 1 decay, 2 sustain, 3 release
 *   pitch_modulated 1 when the voice's pitch follows the voice below it
 *   echo            1 when the voice feeds the echo
 *   ended           1 once the voice has reached the end of its sample, until
 *                   the program clears it
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
} csnes_voice;

/* The number of voices: 8. */
int csnes_voice_count(const csnes_machine *machine);

/* Writes at most `max` voices and returns how many. */
int csnes_voices_get(const csnes_machine *machine, csnes_voice *out, int max);

/* "Voice 1" to "Voice 8", or NULL for an index outside 0 to 7. The strings are
 * static. */
const char *csnes_voice_name(const csnes_machine *machine, int index);

/*
 * Fills `out` with the last `frames` frames that one voice put out, oldest
 * first, as floats within -1 to 1 at the machine's output rate. The voice's
 * own volumes are in it and the master volume and echo are not, and a muted
 * voice still shows. The machine keeps 4,096 samples of the DSP's 32 kHz
 * (128 ms), so frames that reach further back are 0. Returns `frames`, or 0
 * for a voice outside 0 to 7, a NULL buffer, 0 frames, more than 16,384 frames
 * or a call from a callback.
 */
size_t csnes_scope_get(const csnes_machine *machine, int voice, float *out, size_t frames);

/*
 * Fills `out` with the last `frames` frames of the echo output, oldest first,
 * as floats within -1 to 1 at the machine's output rate. `channel` is 0 for the
 * left channel and 1 for the right. A frame is the echo filter's output times
 * the echo volume, which is the amount the echo adds to the mix. The channel
 * mask and the mute flag in FLG leave it alone, and the master volume is not in
 * it. The ring holds 4,096 samples of the DSP's 32 kHz (128 ms) and frames that
 * reach further back are 0. Like the voice scopes, the ring is not part of a
 * snapshot, and a machine that restores one starts with an empty ring.
 * Returns `frames`, or 0 for a channel outside 0 to 1, a NULL buffer, 0 frames,
 * more than 16,384 frames or a call from a callback.
 */
int csnes_echo_scope_get(const csnes_machine *machine, int channel, float *out, size_t frames);

/*
 * Where a voice is in its sample, for a display of the sample table. Every
 * field is read from the machine as it stands, and the call changes nothing.
 *
 *   directory_entry_address  DIR * 256 + SRCN * 4, from the registers now (mod
 *                            65536). Changes when DIR or SRCN is written.
 *   block_address            the address of the BRR block being played. The
 *                            voice sets it to the start address at a key-on and
 *                            moves it 9 bytes on each time the pitch counter
 *                            carries into the next block. At the end of a block
 *                            with the end flag it becomes the loop address.
 *                            0 for a voice that was never keyed on.
 *   loop_address             the loop address that the directory entry holds
 *                            now. The voice reads it when it loops.
 *   sample_in_block          the sample of the block that the pitch counter
 *                            points at, 0 to 15. It advances at the voice's
 *                            playback rate and wraps to 0 at the next block.
 *   srcn                     the voice's SRCN register.
 *   loops_done_saturating    the times the voice has jumped back to the loop
 *                            address since its key-on, stopping at 255. A key-on
 *                            clears it.
 *   flags                    the end flag (bit 0) and the loop flag (bit 1) in
 *                            the header of the block being played, as in the
 *                            BRR header. It changes when a block is decoded.
 *                            0 between a key-on and the first block, which the
 *                            voice decodes 5 samples later.
 *   filter                   the filter number of the block being played, 0 to 3
 *   range                    the shift amount of the block being played, 0 to
 *                            15. The filter and the range change when a block is
 *                            decoded and are 0 before the first one.
 *   loop_length_samples      the samples from loop_address through the first
 *                            block at or after it that has the end flag, 16 for
 *                            each block. 0 when that block lacks the loop flag
 *                            (the voice stops at it) or when 4,096 blocks pass
 *                            without an end flag.
 *   total_samples            the samples from the directory entry's start
 *                            address through the first block with the end flag,
 *                            16 for each block. 0 when 4,096 blocks pass
 *                            without an end flag.
 *   reserved                 0
 *
 * loop_length_samples and total_samples are read from the sound RAM and the
 * directory entry as they are now, so they change when the program writes
 * either. A voice that was never keyed on reports them for its SRCN like any
 * other voice. Returns 1, or 0 for a voice outside 0 to 7, a NULL `out` or a
 * call from a callback.
 */
typedef struct {
    uint16_t directory_entry_address;
    uint16_t block_address;
    uint16_t loop_address;
    uint8_t sample_in_block;
    uint8_t srcn;
    uint8_t loops_done_saturating;
    uint8_t flags;
    uint8_t filter;
    uint8_t range;
    uint32_t loop_length_samples;
    uint32_t total_samples;
    uint8_t reserved[4];
} csnes_brr;

int csnes_voice_brr_get(const csnes_machine *machine, int voice, csnes_brr *out);

/*
 * Decodes the BRR sample at entry `srcn` (0 to 255) of the sample table, which
 * starts at DIR * 256, from the sound RAM as it is now. It follows the blocks
 * from the entry's start address to the first block with the end flag and
 * decodes them as the S-DSP does, with the two samples before the first block
 * at 0 and the shift, the four filters and the clamps of the documentation
 * (fullsnes, "SNES APU DSP BRR Samples"). The samples are the DSP's values
 * before the Gaussian interpolation and the envelope: signed 15-bit, from
 * -16384 to 16383, stored in an int16_t. Multiply by 2 for the full 16-bit
 * scale. The call copies the first `max_samples` of them to `out`, changes no
 * state and returns the number it wrote, which is the smaller of
 * `max_samples` and info->total_samples. It returns 0 for a `srcn` outside 0 to
 * 255, a NULL `out`, 0 `max_samples` or a call from a callback, and then leaves
 * `info` alone.
 *
 * A sample that loops plays the decoded samples once and then the part from the
 * loop address on. The first blocks of that part start from the two samples that
 * end the sample, not from 0, so a filter other than 0 on them sounds different
 * on the loop than in the decoded samples.
 *
 * `info` may be NULL. The fields:
 *
 *   start_address      the directory entry's start address
 *   loop_address       the directory entry's loop address
 *   total_samples      the samples from the start address through the block with
 *                      the end flag, 16 for each block. When the scan gave up
 *                      (truncated is 1) it is the 65,536 samples that it decoded.
 *   loop_start_sample  the index in the decoded samples where the loop begins:
 *                      16 times the number of blocks from the start address to
 *                      the loop address. CSNES_NO_LOOP_START when the loop
 *                      address is not on a block of the decoded samples. It
 *                      matters when has_loop is 1.
 *   has_loop           1 when the block with the end flag also has the loop
 *                      flag (code 3), so the voice loops. With code 1 the voice
 *                      mutes at the end instead, and has_loop is 0.
 *   truncated          1 when the scan followed 4,096 blocks without finding an
 *                      end flag
 *   reserved           0
 */
#define CSNES_NO_LOOP_START 0xFFFFFFFFu
#define CSNES_MAX_SAMPLE_BLOCKS 4096

typedef struct {
    uint16_t start_address;
    uint16_t loop_address;
    uint32_t total_samples;
    uint32_t loop_start_sample;
    uint8_t has_loop;
    uint8_t truncated;
    uint8_t reserved[2];
} csnes_sample_info;

size_t csnes_sample_get(const csnes_machine *machine, int srcn, int16_t *out,
                        size_t max_samples, csnes_sample_info *info);

/* ---- The SPC700 timers ------------------------------------------------- */

/*
 * The three timers, read without a side effect: the output counter is not
 * cleared. Timers 0 and 1 take an input clock of 8 kHz (every 128 SPC700
 * cycles) and timer 2 takes 64 kHz (every 16 cycles). A timer divides its
 * input clock by the divisor and adds 1 to a 4-bit output counter each time.
 *
 *   enabled             1 when the control register (0xF1) has the timer's bit
 *                       set
 *   divisor             the divisor register (0xFA to 0xFC) as written: 0 means
 *                       256
 *   output              the output counter, 0 to 15, as a program reads it at
 *                       0xFD to 0xFF. A read by the program clears it.
 *   reserved            0
 *   ticks_until_output  the SPC700 cycles from now (the clock that csnes_cycles
 *                       returns) until the output counter next increments, 1 or
 *                       more. 0 when the timer is off. It assumes that the
 *                       program writes neither the divisor nor the control
 *                       register in the meantime. A divisor that the timer's
 *                       internal count has already passed waits for the count to
 *                       wrap at 256.
 *
 * Returns 1, or 0 for a NULL `out` or a call from a callback. `out` has room for
 * 3 timers.
 */
typedef struct {
    uint8_t enabled;
    uint8_t divisor;
    uint8_t output;
    uint8_t reserved;
    uint32_t ticks_until_output;
} csnes_timer;

int csnes_timers_get(const csnes_machine *machine, csnes_timer out[3]);

/* ---- Register blocks --------------------------------------------------- */

/*
 * The chip's registers as blocks of bytes, the same calls in every core's
 * header. A block has a name, the address of its first byte in the chip's own
 * register space, a size and the values the chip holds now. Reading a block has
 * no side effect: a port or a counter that a program clears by reading stays as
 * it is. A block outside 0 to csnes_register_block_count() - 1 reads as an
 * invalid block. Every call returns 0 or NULL from a callback or for a NULL
 * machine.
 *
 * Block 0, "S-DSP": the 128 registers at 0x00 to 0x7F as the DSP holds them,
 * including ENVX and OUTX of each voice as they read after the last sample, and
 * ENDX.
 *
 * Block 1, "SPC700 I/O": the 16 bytes at 0xF0 to 0xFF of the SPC700's address
 * space as a program reads them. TEST (0xF0) and the control register (0xF1)
 * read 0. 0xF2 is the DSP register number and 0xF3 the value of the DSP
 * register that it selects. 0xF4 to 0xF7 are the input ports that
 * csnes_write_port sets. 0xF8 and 0xF9 are RAM. 0xFA to 0xFC are the divisors
 * as written. 0xFD to 0xFF are the timer outputs, 0 to 15.
 */
int csnes_register_block_count(const csnes_machine *machine);

/* A static NUL-terminated string, the same pointer on every call. NULL for an
 * invalid block. */
const char *csnes_register_block_name(const csnes_machine *machine, int block);

/* The address of byte 0 of the block in the chip's own register space: 0x00
 * for the S-DSP and 0xF0 for the SPC700 I/O. 0 for an invalid block. */
uint32_t csnes_register_block_base(const csnes_machine *machine, int block);

/* The size of the block in bytes: 128 and 16. 0 for an invalid block. */
size_t csnes_register_block_size(const csnes_machine *machine, int block);

/* Copies the first min(size, max) bytes of the block to `out` and returns the
 * number copied. 0 for an invalid block, a NULL `out` or a `max` of 0. */
size_t csnes_register_block_get(const csnes_machine *machine, int block, uint8_t *out, size_t max);

/*
 * The memory access map. With tracking on, each of the 65,536 bytes of the
 * sound RAM carries these bits for what has touched it since the map was last
 * cleared. Tracking is off by default and costs a little time when it is on.
 * Turning it on starts with a clear map. The map belongs to the listener: a
 * snapshot does not hold it and csnes_snapshot_restore keeps the one that is
 * set.
 */
#define CSNES_ACCESS_READ 1       /* the SPC700 read it as data */
#define CSNES_ACCESS_WRITE 2      /* the SPC700 wrote it */
#define CSNES_ACCESS_EXEC 4       /* the SPC700 fetched it as an opcode or operand */
#define CSNES_ACCESS_DSP_READ 8   /* the DSP read it: a sample, the directory, the echo buffer */
#define CSNES_ACCESS_DSP_WRITE 16 /* the DSP wrote it: the echo buffer */

void csnes_set_access_tracking(csnes_machine *machine, int on);
int csnes_access_tracking(const csnes_machine *machine);

/* Copies the map to `out` (65,536 bytes) and returns 1, or fills `out` with
 * zeros and returns 0 when tracking is off. */
int csnes_access_map_get(const csnes_machine *machine, uint8_t *out);

void csnes_access_map_clear(csnes_machine *machine);

/* Copies the map and clears it in one call, so each call shows the accesses
 * since the one before. Returns as csnes_access_map_get does. */
int csnes_access_map_take(csnes_machine *machine, uint8_t *out);

/*
 * Chooses the voices and the echo that reach the mix. Bit n is voice n for
 * bits 0 to 7, and bit 8 is the echo output. A voice that the mask leaves out
 * still runs (its envelope, its end flag and its pitch modulation of the next
 * voice go on) and stays out of the direct mix and of the echo input. With the
 * echo bit clear the filter's output stays out of the mix, and the echo buffer
 * still fills. Every bit is on at the start, and a reset leaves the choice
 * alone. Only those nine bits count, and csnes_channel_mask returns 0x1FF at the
 * start. The mask is 64 bits wide so that later channels need no new type.
 */
void csnes_set_channel_mask(csnes_machine *machine, uint64_t mask);
uint64_t csnes_channel_mask(const csnes_machine *machine);

/* ---- The end of a song ------------------------------------------------- */

typedef enum {
    CSNES_RUNNING = 0,  /* the SPC700 runs */
    CSNES_STALLED       /* the SPC700 executed SLEEP or STOP and will not go on */
} csnes_end_state;

csnes_end_state csnes_end_state_get(const csnes_machine *machine);

/* SPC700 cycles since the output level last changed. */
uint64_t csnes_quiet_cycles(const csnes_machine *machine);

/* The sizes of the structs above are part of the interface. */
#if defined(__cplusplus)
static_assert(sizeof(csnes_brr) == 24, "csnes_brr is 24 bytes");
static_assert(sizeof(csnes_sample_info) == 16, "csnes_sample_info is 16 bytes");
static_assert(sizeof(csnes_timer) == 8, "csnes_timer is 8 bytes");
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(csnes_brr) == 24, "csnes_brr is 24 bytes");
_Static_assert(sizeof(csnes_sample_info) == 16, "csnes_sample_info is 16 bytes");
_Static_assert(sizeof(csnes_timer) == 8, "csnes_timer is 8 bytes");
#endif

#ifdef __cplusplus
}
#endif

#endif
