# Design

The core runs an NES sound driver the way an NSF file asks. It calls the init
routine with the song number in A and the console in X, and then it calls the
play routine at the rate the header gives. It returns the sound the
console's sound unit would make. It has no display, no input and no
expansion hardware.

## Machine

A `Machine` holds all state: the CPU, the 2 KiB of RAM, the 8 KiB at 0x6000,
the sound unit and the output buffer. It allocates when it is created.
Rendering allocates nothing. Use one machine from one thread at a time.
Rendering is deterministic: the same file, console and song produce the same
samples on every run, and `reset` repeats them. No step calls the platform's
math library (see Arithmetic).

## Clock

One clock drives everything: the CPU clock, counted in cycles (`now`). It is
exactly 19,687,500 / 11 cycles per second for NTSC and 53,203,425 / 32 for PAL,
the master clocks divided by 12 and by 16. The resampler takes the clock as
that ratio, so a position is computed in integers. Every memory access is one
cycle, and the CPU runs instruction by instruction with one access for each
cycle the documentation lists, dummy reads and writes included. The sound unit
is not stepped. It catches up to the cycle of each register access before the
access applies, and to the end of a render before the samples are final.

## CPU

`cpu.rs` interprets the 6502 as the 2A03 has it, with no decimal mode. The
decoder turns an opcode into an operation and an addressing mode, and the
accesses of each mode follow the documentation: the extra read of a page
crossing, the always-taken extra cycle of a store, the wrap inside the zero
page and the missing carry of an indirect jump. The stable undocumented
opcodes run, and so do the three unstable ones that the public test ROMs
check, with the usual results. The others with analog results are skipped: the
core reads their operand, does nothing else and records a warning. BRK and the opcodes that
lock the CPU stop the routine and record a warning.

There are no interrupts. The flags that would raise one stay readable, so a
driver that polls the frame counter's flag works.

## Bus

`bus.rs` maps 2 KiB of RAM at 0x0000, mirrored to 0x1FFF, the sound registers
at 0x4000 to 0x4017, 8 KiB of RAM at 0x6000 and the program at 0x8000 to
0xFFFF in eight windows of 4 KiB. Any other read returns the last value on the
bus.

The program image holds the file's data after as many bytes of padding as the
load address implies. A file without banks sits at its load address, so the
padding is the load address less 0x8000, and each window shows the bank with
its own number. A file with banks has the low twelve bits of the load address
as padding, and a store to 0x5FF8 through 0x5FFF picks the bank of one window.
Stores to the bank registers do nothing in a file without banks. Bytes past the
end of the data read as 0.

## The player

At the start of a song the player clears the RAM, writes zero to 0x4000
through 0x4013, then zero and 0x0F to 0x4015 and 0x40 to 0x4017, puts the banks
of the header in the windows, loads A with the song and X with 0 for NTSC and
1 for PAL, and calls the init routine. Its stores are the player's, so an
observer does not see them.

The player has two phases: a routine runs, or the machine waits for the next
call. A call is a JSR from the player, so a routine ends when it returns to
0xFFFF, an address in the vector area that no routine runs from. The init
routine starts at cycle 0. Call k of the play routine comes at k intervals
after the start, rounded down, or when the previous routine returns if that
is later. The interval is the header's play speed in microseconds, and a speed
of 0 gets the frame period of the console. The schedule is computed from the
start of the song, so it does not drift.

A header that names PAL only gets the PAL clock and speed. A header that names
both consoles gets NTSC, and `new_for` takes the console from the caller. A
routine that has run for a second without returning is abandoned.

## Sound

`apu.rs` implements the two pulse channels, the triangle, the noise channel,
the delta channel and the frame counter. Each channel is an event that runs
when its timer expires: a pulse step every two cycles of its period, a
triangle step every cycle of its period, a noise step by its table and a delta
bit by its rate table. A channel that cannot change the output (a pulse with a
period below 8, a length of 0 or a sweep target that overflows, or a gated
triangle) has its timer settled arithmetically when something next touches it,
so a silent channel costs nothing.

The frame counter is a table of steps for each mode and console. Each step
clocks the envelopes and the triangle's linear counter, the lengths and the
sweeps, or raises the flag. A write to 0x4017 restarts the sequence three or
four cycles later, and in five-step mode it clocks the units once at that
time.

The delta channel reads its sample from the program. Each read takes four
cycles from the CPU. The sound unit counts them and the CPU pays them at the
next register access or at the end of a render, so the cycles are not lost.

## Expansion sound

The VRC6 has two pulse channels and a saw, and a file that asks for it in the
chip byte of its header gets them. Stores to 0x9000 through 0xB003 reach the
chip, and in any other file they do nothing. Each pulse channel counts a
16-step sequence down, one step for each period of its timer (the 12-bit
period plus one cycles) and outputs its volume while the step is at or below
its duty, or always in its mode. The saw adds its rate to an accumulator on
every second timer expiry, six times and then a reset, and outputs the top
five bits. A write to 0x9003 halts all three or shifts their periods right by
4 or 8 bits. Clearing a channel's enable bit silences it and, for a pulse,
restarts the sequence.

The chip's DAC is linear. Each step of it adds 0.9 of one fifteenth of a 2A03
pulse channel at full volume to the mix, and the mask's bits 5 to 7 take the
three channels out of it. A channel that cannot change the output (disabled or
at volume 0) settles its timer arithmetically, like a silent 2A03 channel.

The Sunsoft 5B has the registers of a YM2149-type chip, behind two ports: a
store to 0xC000 through 0xDFFF picks a register, and a store to 0xE000 through
0xFFFF writes it. Three tone generators flip every 16 cycles of their 12-bit
period, a period of 0 counting as 1. A 17-bit shift register makes the noise
and shifts every 32 cycles of its 5-bit period. Each channel gates its tone and
its noise with the mixer register, and a channel with both gated off holds a
constant level. The level is a four-bit volume or a five-bit envelope. The
volume curve falls 1.5 dB for each level of the five-bit scale, and a
four-bit volume is the level twice its value plus one, with 0 silent. The
envelope steps every 16 cycles of its 16-bit period through a ramp of 32
levels, in the sixteen shapes of the documentation. One 5B channel at its top
level is three times a 2A03 pulse channel at full volume, which is the gain
that matches the loudness of a reference player. The mask's bits 8 to 10 take
the channels out of the mix.

The Namco 163 has 128 bytes of sound RAM behind two ports: a store to 0xF800
through 0xFFFF sets the address and whether it steps, and 0x4800 through 0x4FFF
reads and writes the byte at the address. The RAM holds 4-bit waveforms, two
to a byte with the low nibble first, and up to eight 8-byte channel blocks from
0x40: an 18-bit frequency, a 24-bit phase, a waveform length, a waveform
address and a volume, with the number of enabled channels in the top bits of the
last byte. The chip updates one channel every 15 cycles, from channel 8 down
and round again. An update adds the frequency to the phase, modulo the length,
reads the sample at the phase, and outputs the sample less 8, times the volume.
The output is held until the next update, so the chip plays its channels one at
a time, and with eight of them the switching makes a tone at 14.9 kHz. The
core keeps that, and the low-pass filter takes some of it off. One step of the
output is 0.039 of one 2A03 pulse step, which gives the loudness of a
reference player, and the mask's bits 11 to 18 take the channels out.

The Disk System chip has one channel. A wavetable of 64 six-bit samples sits
in RAM at 0x4040, and it is write-protected unless 0x4089 enables it. Every 16
cycles a 24-bit phase moves on by a pitch that the modulation unit sets, and the
top six bits are the sample. The output is the sample times the volume gain
(held at 32 at most) times the master volume of 1, 2/3, 1/2 or 2/5. A halted
wave outputs its first sample. The volume envelope moves the gain up or down by
one every 8 times its speed plus one, times the global speed plus one, cycles.
The modulation unit adds its own pitch to an accumulator, and each carry steps
a counter through a table of 32 three-bit entries. The counter times the
modulation gain gives the multiplier of the wave's pitch, rounded as the
documentation gives. The output passes a one-pole low-pass filter at 2 kHz and
is as loud as the documentation says, 2.4 times a 2A03 pulse channel at full
volume. A file with the chip has 24 KiB of RAM at 0x8000, filled from the
program, and its bank registers copy banks into it. Bit 19 of the mask takes
the channel out.

## Output

The mixer is the pair of lookup tables of the documentation, built at startup
from its formulas: one indexed by the sum of the pulse levels, one by three
times the triangle plus twice the noise plus the delta level. A mask of five
bits removes channels from the sum. The levels it leaves out stay visible
through `channel_levels`.

A first-order high-pass filter follows the mixer, with its corner at 90 Hz as
the console has it, and a first-order low-pass filter at 14 kHz follows that. It runs on the sound unit's events. Between events the
level is constant, and the filter's state moves by a factor that a table gives
for each count of cycles. A tick every few dozen cycles keeps its output
current, and the ticks stop when the output has settled. After a change of the
mix the output moves toward its new level over a few dozen cycles, so the
filter ticks every 16 cycles until it arrives. The capacitor starts at the
level of the idle channels, so a song does not start with a thump.

A player can move the corner to any value from 1 to 1000 Hz. The factor per
cycle is then exp(-2 pi f / clock), summed from its series so that only basic
arithmetic is involved. A higher corner shortens the tick. A change first
brings the capacitor to the current cycle with the old factors, so the output
is continuous. The choice lives outside the machine's snapshot, like the
channel mask.

`audio/resample` turns the stream of level changes into samples at the
caller's rate. Each change is a band-limited step at its exact cycle. One
mixer unit is 29/32 of the 16-bit range, and float output uses the same scale.
The loudest drums, made of delta-channel steps, reach past full scale, and the
host scales the output if it needs headroom.

## Arithmetic

The resampler's kernel is a table of fixed-point values. The filter's factors
are built by repeated multiplication, and its output is rounded with `floor`.
The mixer tables use a division, which is an IEEE 754 basic operation, so the
bits are the same on every platform.

## Loading

`formats/nsf` reads the header: the song count, the first song, the load,
init and play addresses, the play speeds, the banks, the console and chip
bytes and three text fields. The data after the header is the program. A
version 2 file may give the length of the program. A file larger than 4 MiB is
refused. A load address below 0x8000 is an error.

## Observer

`cnes_set_observer` registers callbacks that run on the rendering thread,
inside `cnes_render_*`, in program order: one for each store to 0x4000..0x4017
and one for each call of a routine. A callback reads the machine only through
`cnes_cycles`. That call returns the cycle of the event. Every other call on
that machine returns at once with a neutral value, because the render that
runs the callback holds the machine.

## Snapshots

`snapshot` copies everything a machine is except the observer, the channel
mask and the bass cutoff. Those belong to the listener. The program data is
shared between copies. A restore checks the output rate, the console and the
file, then replaces the state, and the same calls render the same samples from
there. The output ring is the largest part of a copy.

## End of a song

`end_state` reports a unit with no channel on, and a routine that did not
return or locked the CPU. A song does not end in the file itself: the play
routine goes on being called. `quiet_cycles` counts the cycles since the
output level last changed, and it is the signal for the end of a song. The
machine has no rule for how long a silence lasts before a song has ended.
