# Design

The core runs the sound driver that an HES file carries. The file holds code
and data of a game, a start song and the address of a request routine. The
machine runs a HuC6280 against a memory map, a timer, the interrupt registers,
a stand-in for the video chip's vertical sync and the sound generator, on one
timeline, and a resampler turns the sound generator's output into the caller's
rate. It is deterministic: the same file, song, output rate and calls produce
the same samples on every run, and `reset` repeats them. No step of the chips
calls the platform's math library.

## Machine

Time is counted in master clocks of 21,477,273 Hz. A cycle of the processor is
3 of them at high speed and 12 at low speed. The sound generator ticks every 6,
and the timer's prescaler every 3,072 (3 times 1,024, from the hardware
manual). The machine starts in the low speed state that the manual gives for a
reset, and a program that wants the high speed sets it.

The file's first block loads at its physical address into a 1 MiB ROM image
(bytes that no block covers read 0xFF). The eight memory mapping registers
start as the header gives them. RAM is 32 KB at physical 0x1F0000 and the I/O
page is at 0x1FE000: the video chip's registers, the sound generator, the
timer, the I/O port (reads 0xFF) and the interrupt registers at 0x1FF402 and
0x1FF403.

The machine calls the request routine as a subroutine. It sets the accumulator
to the song number, puts a return address on the stack that points at an idle
address (logical 0x1FFF) and starts the processor at the request address. The
code that HES files carry at that address (checked in five files of four
games) saves the song number, sets up the game's sound driver and returns. While
the processor stands at the idle address the machine skips ahead to the next
timer count or vertical sync. An interrupt that comes then runs its handler,
and the handler's RTI returns to the idle address. A routine that never
returns runs for as long as the machine does.

The timer is the 7-bit down counter of the hardware manual: a write to
0x1FEC00 sets the reload value, a write of 1 to 0x1FEC01 loads the counter from
it and starts it, and the counter borrows when it counts below 0, which sets
the timer request and reloads it. An interval is reload + 1 counts. The
request clears with a write to the interrupt request register. The interrupt
disable register masks the timer (bit 2), IRQ1 (bit 1) and IRQ2 (bit 0).
Interrupts take the vectors 0xFFFA for the timer, 0xFFF8 for IRQ1 and 0xFFF6
for IRQ2, with the timer first.

The video chip's stand-in answers what the sound drivers use. A store to its
address register (0x1FE000) selects a register and a store to the low data
port (0x1FE002) sets the control register when that is register 5. When bit 3
of the control register is set (the vertical blanking interrupt enable of the
HuC6270 manual), the stand-in sets the vertical blanking status bit once a
frame and pulls IRQ1. A read of the status register returns the bit and clears
it. A frame is 262 lines of 1,365 master clocks (60.05 Hz, the NTSC line
rate), and the blanking starts at line 240.

## Processor

`cpu.rs` is written from the HuC6280 hardware and software manuals. Its
header lists what the manuals leave open and the choice for each (blank opcodes
as NOPs, no extra cycle at a page crossing, where the zero page and the stack
are, how a block transfer restores its registers, when an interrupt comes
after CLI, SEI and RTI, the cycles of CSL and CSH, the flags of TSB and TRB).
The manuals contradict themselves in a few places, and the choices follow the
instruction pages over the summary tables: TII increments both addresses, TDD
decrements both, BSR returns to the instruction after it, DEX works on X, and
SBC sets the carry when no borrow occurs.

A block transfer runs whole inside one step, which is up to 393,233 cycles
(about 55 ms at high speed). Timer and sync events wait until it ends, and the
sound generator's stores inside it land at the time the transfer began.

## Sound generator

`psg.rs` is written from the HuC6280 sound generator manual. A channel steps
its 32-word waveform every F ticks of 3.579545 MHz, so its frequency is the
master clock over 2 * 32 * F, the formula and the scale table of the manual.
The registers, the write modes of the waveform register (writing in order,
resetting the counter, the direct D/A mode and the mixing mode that ignores
writes), the amplitude controls and the noise are as the manual gives them.

The amplitude of a channel is the sum of three attenuations: the main level
and the left or right balance in steps of 3 dB, and the channel level in steps
of 1.5 dB. A channel is silent when the steps of 3 dB come to more than 15.

Noise on channels 5 and 6 shifts its register every 64 times the complement of
the five frequency bits, from the manual's formula for the noise clock.

The low frequency oscillator modulates channel 1 with channel 2's waveform.
While it runs, channel 2 steps at its frequency times the LFO register, and
channel 1's frequency takes the waveform word of channel 2 as a signed 5-bit
value (the sign bit inverted, from the manual's table) shifted by 0, 2 or 4
bits. It acts only while both channels are on.

## Levels

The documents give no level against full scale. The words of the sound
generator are about their midpoint, in half steps (a word w contributes 2w - 31
times the gain), and a channel that is off contributes 0. The gain constant is
fitted to libgme's renders of 29 songs of four games (R-Type, Dracula X,
Tokimeki Memorial and Ys III): their mean level is within 0.4 dB of each other
and the constant sets the mean to equal libgme's. Taking the words unsigned
left a bass excess of 16 dB on the jingles against those renders, which the
midpoint removes. The output has a 20 Hz high-pass.

## Unknowns

These choices rest on documents that do not say, or on a single measurement.

- The noise register's taps and length. The core uses 15 bits with taps on
  bits 0 and 1, and the output is 31 or 0 before the midpoint.
- The unit of the noise clock formula. The core shifts the register once per
  clock of the formula, which is every 64 times the complement of the
  frequency bits in ticks. A shift rate 4 to 8 times as high matches libgme.
- The period for a frequency of 0 (4,096, a full turn of the 12-bit counter),
  and for a noise frequency whose complement is 0 (32).
- Whether a store to the waveform register in the mixing mode is dropped. The
  core drops it.
- The initial phase of a channel's frequency counter after a write.
- Whether the output carries an offset on the real console. The core has none
  (see Levels).
- The vertical sync's place in the frame and the frame length. The core takes
  the NTSC line rate and 262 lines, and a PAL or other rate is not modeled.
- The video chip beyond its vertical sync: a program that reads the scanline
  counter or other status bits sees none of them.
- RAM at 32 KB without mirrors. The console has 8 KB and the SuperGrafx 32 KB.

## Checks against a reference player

Level and spectrum were compared with libgme's renders of 29 songs (20 s each,
44,100 Hz). The levels agree within 0.4 dB of each other, and the bands from 20
Hz to 16 kHz agree within about 1 dB. Above 16 kHz the core is about 9 dB
louder, because libgme applies a treble cut that the core does not. One song
(R-Type II, song 4) has about 6 dB more bass than libgme's render. It sweeps
the noise frequency of channel 6. The match of its bands needs a noise shift
rate 4 to 8 times the one that the manual's formula gives, so libgme's noise
is brighter than the manual's. The core keeps the manual's formula.

## Snapshots

A snapshot is a copy of the whole machine: the processor, RAM, the timer, the
sound generator and the sound that has not been handed out yet. The observer
is not part of it: a copy has none, and `reset` and `restore` keep the one
that is set.
