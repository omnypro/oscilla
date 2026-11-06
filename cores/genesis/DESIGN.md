# Design

The core plays a VGM file: a log of writes to the Mega Drive's sound chips with
waits between them. It models the YM2612 (six FM channels and a DAC) and the
SN76489 (three tone channels and a noise channel), runs both on one timeline,
and returns their sum. It has no Z80, no 68000, no VDP and no analog output
stage beyond a high-pass.

## Machine

A `Machine` holds all state: the file, the position in the command stream, both
chips, the resampler and the output filter. It allocates when it is created
and while it renders. Use one machine from one thread at a time. Rendering is
deterministic: the same file, output rate and calls produce the same samples on
every run, and `reset` repeats them. No step of the chips calls the platform's
math library.

## Reader

`formats/vgm` reads versions 1.00 to 1.71, VGZ (gzip) and the GD3 tag. It
returns the header, the commands and the data blocks. The header gives the
chips' clocks, the loop point, the lengths, the SN76489's feedback taps, shift
register width and flags, the volume modifier, the loop base and the loop
modifier. A clock with bit 30 set names a second chip. A file may be up to
64 MiB unpacked.

The core plays the commands for the YM2612 (ports 0 and 1), the SN76489
(including the Game Gear stereo write), the second chip of either kind, the
waits, the data blocks of type 0 (the YM2612's PCM bank), the 0x8n DAC commands,
the seek of 0xE0 and the DAC stream commands. It reads and skips every other
command by its length.

## YM2413

`ym2413.rs` is the OPLL of the Master System and the MSX: nine channels of
two operators, one user tone (registers 0x00 to 0x07), 15 built-in instruments
and a rhythm mode. A file names it with the clock word at 0x10 (3.579545 MHz)
and writes it with command 0x51, or 0xA1 for a second chip. The chip makes one
sample for every 72 clocks. The machine mixes it with the other chips at full
level.

Sources: the manufacturer's application manual and datasheet give the
registers, the envelope and key scale tables, the step sizes, the slots of the
rhythm sounds and the output of the rhythm sounds twice. The formulas of the
log-sine and exponential tables are the YM3812 decap report's. The depths of
the low frequency oscillators come from a hardware note on the VRC7. The 15
instruments and the three rhythm patches are the community wiki's table, which
was measured on the chip and checked against a die shot of a related part (the
wiki calls it "presumably a YM2413B"). The engine follows the VRC7 core of
`cores/nes`, which is the same chip with six channels, other instruments and no
rhythm mode.

Rhythm mode (register 0x0E bit 5) makes channels 7 to 9 the five drums. The
bass drum is an ordinary two-operator channel with its own patch, key bit
(0x0E bit 4) and volume (register 0x36). The snare drum, hi-hat, tom-tom and
top cymbal need the phase of their operators, which the documents do not give:
they say that a noise oscillator and the frequencies of channels 8 and 9
(in a ratio of 3 to 1) make it. The core plays those four silent. Register
stores for them are kept.

## GYM

`formats/gym` reads a GYM file and writes the VGM file with the same commands.
The core then plays that file, so a GYM log and its VGM twin render the same
samples. A log has four commands: 0x00 waits one frame, 0x01 and 0x02 write a
register of the YM2612's port 0 and port 1, and 0x03 writes the SN76489. The
conversion makes a frame wait 735 samples (60 Hz) or 882 (50 Hz) and sets the
chips' clocks to the console's: YM2612 7,670,454 Hz and SN76489 3,579,545 Hz for
NTSC, 7,600,489 Hz and 3,546,895 Hz for PAL. An optional "GYMX" header of 428
bytes holds the tag text (song, game, publisher, emulator, dumper, comment),
the frame where the loop starts and a flag for zlib compressed data. The loop
starts after the wait that ends that many frames. A start at or past the last
frame is no loop. The data may be 64 MiB unpacked.

The header has no field for the region. The caller says whether the log is PAL.

A command that the data ends in the middle of is dropped. A byte that is not 0
to 3 where a command should start is an error, because the length of the next
command is not known.

## Time

The unit of time is 1/(44100 x 4096) s. A VGM wait is a whole number of 4096
units. A chip's event lands on the unit below its own clock cycle, with the
conversion done in 128-bit integers, so the error is under 6 ns. The two chips
run to the same target time, their level changes are merged in time order, and
the resampler takes the sum as steps at those times.

At the end of a stream without a loop the machine stops both chips and holds
silence. With a loop it jumps to the loop point and counts the pass. A loop
with no wait in it ends the stream, so a malformed file cannot spin.

## Second chips

A YM2612 with bit 30 set in its clock has a second instance, and so does an
SN76489 unless bit 31 is also set (that is the T6W28 of the Neo Geo Pocket, one
chip with two halves, which the core plays as one chip). The second chip takes
the clock of the extra header when the file gives one and the first chip's clock
otherwise. It has its own state and its own commands (0xA2, 0xA3 and 0x30,
0x3F). The levels of all chips add, each at the level it has alone, and nothing
else changes. The DAC commands 0x8n write the first YM2612. The channel mask
has bits 10 to 15 for the second YM2612 and bits 16 to 19 for the second
SN76489. The mask of a file without a second chip is unchanged, so 0x3FF stays
the default there.

## DAC streams

Commands 0x90 to 0x95 set up streams, each a chip register, a data bank, a step
and a base, a rate and a position. Only streams for the YM2612 (chip 0x02, or
0x82 for the second) write anything. A stream writes one byte of its bank every
1/rate seconds, the first at the start command's own time. A write that falls
at the time of a command goes before that command. The start command takes a
length in writes, in milliseconds (rounded to the nearest write) or to the end
of the bank, with loop and reverse bits; the fast call plays a block of the
bank. A stream stops when it passes the end or the start of the bank, or
restarts if it loops and the restart would write something. A new rate takes
effect from the next write. A stream that gets its first rate late starts from
that command and does not write the samples that it missed. The rate is
capped at 65,536 Hz, because the YM2612 reads its DAC register once a sample
(about 53 kHz) and a corrupt file could otherwise make a looping stream spin.

A header can name any 30-bit chip clock. The machine takes at most 16 MHz for
a chip, which is more than twice the real clocks, because the work and memory
of a render grow with the clock. The data blocks of a file together make at
most 64 MiB of banks, a packed block included.

## SN76489

The chip divides its clock by 16. Each tone channel has a 10-bit counter that
flips its output when it expires, and a 4-bit attenuation in steps of 2 dB (15
is silent). The noise channel has a shift register with the feedback taps and
width that the header gives (defaults 0x0009 and 16 bits), white or periodic
feedback, and a rate of 16, 32, 64 or the third tone channel's period. A write
to the noise register resets the register. A tone register of 0 or 1 holds the
channel high, which is how drivers play samples through the volume. The
header's flags can make 0 mean 1024, negate the output or change the feedback
to XNOR. The Game Gear stereo write gates each channel to the left, right or
both.

The model runs from one counter expiry to the next and reports each change of
level.

## YM2612

One call of `sample` is one output sample, 144 cycles of the chip's clock
(about 53,267 Hz). The six channels run in turn, each as four operators.

- Registers: the operator registers sit at +0, +4, +8 and +C in the order
  S1, S3, S2, S4. Part 1 holds channels 4 to 6. Channel 3 can take a separate
  frequency for each operator (special mode), and CSM mode keys channel 3 on
  from timer A.
- Phase generator: a 20-bit phase, the F-number and block with detune, the
  multiple and the LFO's vibrato. The key code follows the published division of
  the octave.
- Envelope generator: a 10-bit attenuation moved by the chip's shared counter.
  The rate is 2R plus the rate scaling (capped at 63). A step happens every three
  samples. The attack uses the published increment table and the exponential
  rule `att += (~att * inc) >> 4`. SSG-EG modes 8 to 15 are implemented.
- Operator: a log-sine and an exponent table (generated by the formulas in the
  source) with a 10-bit phase input and a 10-bit attenuation, giving a value
  of up to +-8191.
- Modulation: a modulator's output shifts down by `MOD_SHIFT` (1) before it
  enters the phase of the operator it modulates. Feedback adds the last two
  outputs of operator 1, shifted by 10 minus the feedback level.
- Output: the carriers of each algorithm, each shifted down by 5, sum into a
  9-bit value that saturates at -256 and 255. The six channels are summed per
  output sample with their panning. Channel 6 outputs the DAC when it is on:
  `$2A` doubled plus bit 3 of `$2C`, minus 256.
- LFO: eight rates, the amplitude modulation depths of the AMS field and the
  vibrato depths of the FMS field. The LFO is held at zero while it is off.
- Timers: A counts (1024 - NA) samples and B counts 16 x (256 - NB) samples.
  They matter only for CSM mode in a VGM, since the file holds no reads.

The DAC register starts at 0x80 so that a file that turns the DAC on before
writing a sample plays silence.

## Levels

The documents give relative sizes, and the gains here were fitted to renders
from libgme (a black box, used for level and shape, never for tables or
logic). The renders came from 12 synthetic files and 12 songs.

- One FM carrier at full scale peaks at 0.258 of full scale (`FM_GAIN` 33).
- A full-scale DAC square is 1.25 times the size of a full-scale FM sine in
  the same units, 1.8 dB above it by RMS (`DAC_GAIN`).
- The PSG's tone at volume 0 peaks at 0.25 of full scale when the header names
  no FM chip, and 0.405 of that when it names the YM2612 (0.344 of the
  chip's own scale, 89/256). The documents put four PSG channels at one FM
  channel, which is a factor of 0.26. The core follows the renders.
- PSG volume steps are the ideal 2 dB. The renders run up to 1 dB lower from
  step 3 to step 8 and further down at the quiet end.
- The modulation shift was fitted to a tone with operator 1 modulating operator
  2 at eight levels. The shift of 1 reproduces the 440 Hz line at all eight
  levels to 0.05 dB. The feedback arithmetic needed no fit: the 440 Hz line and
  the harmonics at feedback 1 to 7 agree within 2 dB.
- The output is followed by a one-pole high-pass at 20 Hz, which stands for the
  console's output coupling and removes the DC that the DAC leaves. The
  documents give no cutoff. `set_highpass_hz` changes it, and 0 turns it off.

Across the 12 songs the RMS of the core is 0.2 dB below the reference on
average, with a spread of 0.5 dB, and the spectrum agrees within 1 dB from 20
Hz to 8 kHz. Above 8 kHz the core carries 1 to 3 dB more, since the reference
player applies a treble cut (-14 dB) that the core does not.

## Unknowns

These choices rest on documents that do not say, or on a single measurement.

- The vibrato depths of the LFO are approximate. The documents give the peak
  depths, not the step values.
- The LFO's phase when it starts, and its output while it is off.
- SSG-EG: how a held end level behaves and what a key off does. Two of the
  twelve reference songs write an SSG-EG register once each.
- DAC streams: what start mode 0 does (the core moves the position and leaves
  the running state alone), where a reversed fast call or a base begins (the
  last byte of the block, less the base), and the order of a stream write and
  a command at the same instant (the write goes first). The data of a stream
  for another chip, and a stream for the SN76489, are ignored.
- YM2413: the phase of the snare drum, hi-hat, tom-tom and top cymbal (not
  played). The level of a YM2413 channel is set to one YM2612 channel's, and
  the SN76489 beside a YM2413 takes the level that it has beside an FM chip in
  libgme. No reference player renders a YM2413, so neither is a fit. Whether a
  rhythm-mode switch in the middle of a note should cut the notes of channels
  7 to 9 (the core cuts them).
- YM2413: the instrument table is from a source that calls the die shot behind
  it "presumably" of a YM2413B, so a few bytes could differ on the older
  YM2413.
- GYM: that a byte above 3 is an error and not something to skip. The two
  descriptions of the format that the core was written from list four commands
  and no others.
- GYM: whether the header's packed flag is a size (the core takes any nonzero
  value as a flag and unpacks the rest of the file).
- Whether two chips of one kind should be mixed at lower levels. The core adds
  them at full level, as the file's other players do.
- The ladder effect (the DAC's resistor ladder adds an offset below zero) is
  built and off. `set_ladder` is not exposed through the C interface.
- The first sample after a key on: its phase and envelope step.
- Whether the console's analog stage should be modeled. Model 1 consoles have
  a low-pass of about 3.4 kHz. The core applies none.
- Multiplexing: the real chip outputs one channel at a time and the core sums
  the six per sample.

## Checks against a reference player

The reference set is 12 songs from 12 games and 12 synthetic files, rendered
by libgme at 44,100 Hz. The reference starts sound up to 12 ms after the core's
first sample, which is its own latency.

The 360 files in the local corpus (14 games from Zophar's Domain, git-excluded)
parse and render without a panic. Rendering runs at 80 times real time or faster.

## Snapshots

A snapshot is a copy of the machine, channel mask and high-pass included. It
restores into a machine with the same file and output rate, and the same calls
then produce the same samples.

## End of a song

The machine reports the passes through the loop point and whether a stream
without a loop has ended. The file's loop base and loop modifier turn the
caller's number of passes into the number the file asks for
(`loop_count`). The host decides when a song is over, and fades it.
