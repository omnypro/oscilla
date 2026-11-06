# Design

The core runs an SNES sound driver from the state that an SPC file holds. It
restores the 64 KiB of RAM, the 128 DSP registers and the SPC700's registers,
then runs the processor and the DSP together. It returns the sound the S-DSP
makes. It has no main CPU, no cartridge and no analog output stage.

## Machine

A `Machine` holds all state: the SPC700, the RAM, the DSP, the timers, the
ports and the output buffer. It allocates when it is created. Rendering
allocates nothing. Use one machine from one thread at a time. Rendering is
deterministic: the same file and output rate produce the same samples on every
run, and `reset` repeats them. No step calls the platform's math library.

## Clock

One clock drives everything: the SPC700 clock, counted in cycles (`now`). It is
1,024,000 cycles per second, the 24.576 MHz master clock divided by 24. The
DSP makes one sample every 32 cycles, so 32,000 samples a second. The resampler
takes the clock as that ratio, and each sample is a step at its cycle.

The processor runs an instruction whole and then the clock advances by the
cycles that the instruction takes. Two accesses see the clock inside an
instruction: a read of a timer output and a read of a DSP register bring the
clock up to the cycle they occur in, which is one cycle for each byte fetched so
far. A store to the DSP lands in the last cycle of its instruction, and the
observer reports the cycle after that. Other stores take effect at the start of
their instruction. A sample that falls inside an instruction sees them, so a
store reaches the DSP up to 10 cycles early, except for the DSP registers.

## SPC700

`smp.rs` interprets the 256 opcodes with the cycles of the public tables:
taken branches cost two more, and a conditional jump on a bit costs 5 or 7.
Direct page addresses wrap inside their page. The opcodes that the tables mark
as storing with a dummy read do the read, because a read of a timer output
clears it. `MOVW` and `INCW` wrap the high byte inside the page.

`DIV` runs the nine-step division of the documentation, so an overflow gives
the 9-bit result in V and A, and H takes the documented value.

`SLEEP` and `STOP` halt the processor. The clock, the timers and the DSP go on.

## Ports and timers

Ports 0xF4 to 0xF7 are four inputs and four outputs. A read returns the input
and a write sets the output and the RAM byte. The inputs start with the bytes
at 0xF4 to 0xF7 of the RAM image, and `write_port` sets them. A write to the
control register with bit 4 or bit 5 set clears the inputs in pairs; a load does
not apply the bits. Some drivers read a command from the ports at the start,
and the reference logs of three of them fit only these starting values.

There are three timers. Timers 0 and 1 count every 128 cycles (8 kHz) and timer
2 every 16 cycles (64 kHz). A timer divides its input by the target (0 means
256) and adds one to a 4-bit output, which a read clears. The divider compares
for equality after it counts, so a target below the divider's value waits for
the wrap. Turning a timer on resets its divider and output. The TEST register
is not emulated.

The timers count at cycles that are multiples of 16 and 128 of the machine's
clock. The reference logs fit this phase for every driver that waits on a
timer.

## Boot ROM

While bit 7 of the control register is set, 0xFFC0 to 0xFFFF read as a 64-byte
program, and writes go to the RAM under it. The program in `boot.rs` is a
stand-in written for this core from the documented behavior, as the GBA core
does with its BIOS. It clears 0x01 to 0xEF, announces 0xAA and 0xBB on ports 0
and 1, waits for 0xCC, and takes blocks of bytes through the ports until a
command of 0 enters the program. Games jump to 0xFFC0 to start over and to
0xFFC9 to announce again, and both start an instruction here.

Limit of the stand-in: a block takes at most 256 bytes. The program does not
match the original byte for byte.

## Restoring a file

An SPC file holds the RAM and the registers of one moment. It has no state for
the voices. The decisions:

- The processor registers come from the header. The program counter, `A`, `X`,
  `Y`, the flags and the stack pointer are as stored.
- The control register comes from RAM byte 0xF1. The timer enables and the boot
  ROM switch follow it. The timer targets come from 0xFA to 0xFC. A timer that is
  on starts with its output at the low four bits of 0xFD to 0xFF and its divider
  one count in; both fit the reference logs. The input ports start with the
  bytes at 0xF4 to 0xF7. The reset bits of the control byte do not apply.
- When the boot ROM switch is on, the 64 bytes of extra RAM in the file replace
  the RAM at 0xFFC0 to 0xFFFF.
- The DSP registers come from the file, including KON. A key-on that the file
  holds starts at the first sample. Voices start silent with the envelope at 0
  and no sample position, so a voice that sounded when the file was made starts
  again at the driver's next key-on. ENDX comes from the file.
- The echo buffer is the RAM at ESA and plays its old contents from the first
  sample. The filter's history starts at 0 and the buffer index at 0.
- The noise generator starts at its reset value.
- The TEST register is not applied. Files hold odd values in it.

## DSP

`dsp.rs` makes one stereo sample per call. It does not step through the 32
cycles of the hardware's pipeline. Each voice runs in order 0 to 7, and the
mixer and echo run after the eighth.

**Key-on and key-off.** KON and KOFF are polled every second sample. KON holds
the last value written until it is polled. A key-on clears the voice's bit in
ENDX, sets the envelope to 0 and the state to attack, and gives 5 silent samples
before the first block is decoded. A key-off puts the voice in release. Bit 7 of
FLG puts every voice in release with the envelope at 0, every sample.

**BRR.** A block is 16 nibbles. `sample = (nibble << shift) >> 1`, and for a
shift of 13 to 15 the nibble becomes its sign (-1 or 0) and the shift 12. The
four filters use the exact integer forms of the documentation. A sum past 16
bits clamps to 0x3FFF or 0 as the documentation says, and a sum between 15 and
16 bits wraps by 0x8000. The two rules of the documentation disagree at exactly
0x7FFF, which wraps to -1; only a malformed block reaches it. The previous two samples feed the next block.

The end flag sets the voice's bit in ENDX when the block is decoded. When the
voice moves on from a block with the end flag it jumps to the loop address,
which it reads from the sample table at that moment, so a changed DIR or SRCN
takes effect at the next loop. With the code 1 (end and mute) the voice goes to
release with the envelope at 0.

**Pitch.** The 16-bit counter adds the 14-bit pitch each sample. Bits 15 to 12
pick the sample and bits 11 to 4 the row of the interpolation table. A carry out
of bit 15 moves on one block. Pitch modulation scales the pitch by the previous
voice's output of the same sample, `(out >> 4) + 0x400` over 0x400. The step is
cut to 0x3FFF.

**Interpolation.** Four samples go through the 512-entry table of the
documentation. The first three products wrap at 16 bits and the last
saturates, then the sum shifts right by 1. The table sums to between 0x7FF and
0x801 in each group of four, as the documentation says, and a test checks that.

**Envelope.** Attack adds 32 (1024 at rate 31), decay and sustain subtract
`((level - 1) >> 8) + 1`, release subtracts 8 every sample, and direct gain
sets `value * 16`. The four gain modes follow the documentation. A rate is a
number of samples between steps. A step falls when the shared counter, which
counts samples modulo 30,720, is a multiple of the period. Unknown: the
hardware's counter offsets for each rate. The core uses 0, so the first step
after a phase change can come earlier or later than on the hardware.

**Noise.** One 15-bit generator steps at the rate in FLG. A voice with its bit in
NON outputs the generator's value in place of the interpolated sample. The
generator has the longest cycle a 15-bit register allows, and a test checks it.

**Mixer.** Each voice's enveloped sample goes through its left and right
volumes (`>> 6`) into 16-bit sums that clamp after each voice. The master
volumes scale the sum (`>> 7`), the echo output adds, and the mute flag zeroes
the result. The output is not inverted: the hardware inverts the signal, and the
analog stage inverts it back.

**Echo.** The buffer lives in RAM at `ESA * 256`, 4 bytes for each sample. Each
sample reads the oldest entry into the 8-tap filter, writes the voices with
their EON bits plus the filtered output times the feedback volume (with bit 0
cleared) unless FLG bit 5 is set, and adds the filtered output times the echo
volume to the mix. The buffer length is read from EDL when the index wraps. The
first seven taps wrap at 16 bits and the last saturates. A change of ESA applies
at once. On the hardware it applies a sample later.

**Channel mask.** Bits 0 to 7 are the voices and bit 8 is the echo output. A
voice that the mask leaves out still runs. Its output stays out of the direct
mix and out of the echo input.

## Output

The DSP's samples are 16-bit values. The machine passes them to the resampler
as steps at their cycles and takes the result at unit gain. There is no
high-pass or low-pass filter, and nothing models the analog stages. The output
can pass full scale and clamps there.

## Checks against a reference player

The reference set is 35 songs from six games and six sound drivers, with a
render and a log of the SPC700's DSP stores for each.

- The log of stores matches in order for all 35 songs, and the clock of every
  store matches: over 3.3 million stores. Three families of drivers beyond the
  first set (three games, then three more) fit without a change.
- The timer fits came from these logs: the initial divider and output, the
  equality compare and the phase. So did the input ports' starting values.
- The renders agree in shape, not sample by sample. The reference player runs
  its processor ahead of its DSP, so a store reaches the DSP a variable number
  of samples after the clock in the log. With a fixed shift of 3 or 4 samples
  and a gain of 1.4, songs with one or two voices and no echo agree to about
  -12 to -17 dB. Songs with echo and many voices agree to about -4 to -7 dB.
  The reference player is about 1.4 times louder than the DSP's own mix.
- The register values that a driver reads back (volumes, pitches, KON, KOFF)
  are the stored ones. The DSP's own reads (ENVX, OUTX, ENDX) are not checked.

## Observer

The observer sees every store to a DSP register and every store to an output
port, in program order, with the clock at the end of the instruction.

## Visualization

The DSP keeps what a display needs. `voice_info` reads a voice's registers and
its envelope, and the C interface turns them into floats: the playback rate is
32,000 Hz times the pitch register over 4,096, and the volume is the larger
volume register over 128 times the envelope over 2,047. The scope keeps the
last 4,096 samples of each voice at the DSP's 32 kHz: the voice's output times
its two volumes, averaged. `scope` resamples them to the output rate by linear
interpolation. The ring is not part of a copy of the machine, which starts with
an empty one.

The access map has one byte for each address of the sound RAM, with a bit for
the SPC700 reading, writing or fetching it and for the DSP reading or writing
it. The SPC700 marks an instruction's opcode and operands as fetched and not as
read, and a store marks only the write (the instruction's own dummy read of the
address still marks it as read). The DSP's accesses are the directory entry
and the block of a voice, and the echo buffer. Tracking is off until asked for,
and the map is not part of a copy of the machine.

## Snapshots

A snapshot copies the whole machine apart from the observer and the channel
mask. It restores into a machine with the same file and output rate, and the
same calls then produce the same samples.

## End of a song

A file has no end. The machine reports a halted processor and the cycles since
the output last changed. The host decides when a song is over, with the time and
fade of the tag.
