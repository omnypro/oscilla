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
 * the four input ports start at 0 and csnes_write_port sets them.
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
 * inputs start at 0. A port number outside 0 to 3 does nothing and reads 0.
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
 */
typedef struct {
    void *context;
    void (*dsp_write)(void *context, uint32_t reg, uint32_t value);
    void (*port_write)(void *context, uint32_t port, uint32_t value);
} csnes_observer;

/* Copies the observer in. Pass NULL to stop observing. */
void csnes_set_observer(csnes_machine *machine, const csnes_observer *observer);

/* ---- Inspection, for tests and tools ----------------------------------- */

/* Runs `cycles` SPC700 cycles. Sound produced meanwhile is discarded. */
void csnes_run_cycles(csnes_machine *machine, uint64_t cycles);

/* A, X, Y, PSW, SP by index 0 to 4; 5 is PC. */
uint32_t csnes_register(const csnes_machine *machine, int index);

/* Reads sound RAM with no side effects. The boot ROM and the ports do not
 * show: the byte is the RAM under them. */
uint32_t csnes_peek(const csnes_machine *machine, uint32_t address);

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

#ifdef __cplusplus
}
#endif

#endif
