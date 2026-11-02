# Design

The core runs a Game Boy sound driver the way a GBS file asks. It calls the
init routine with the song number in A, and then it calls the play routine at
the rate the header gives. It returns the sound the console's sound unit
would make. It has no display, no input and no link port.

## Machine

A `Machine` holds all state: the CPU, 64 KiB of memory, the timer, the sound
unit and the output buffer. It allocates when it is created. Rendering
allocates only to record a warning. Use one machine from one thread at a
time. Rendering is deterministic: the same file and song produce the same
samples on every run, and `reset` repeats them. No step calls the platform's
math library (see Arithmetic).

## Clock

One clock drives everything: 4,194,304 cycles per second (`now`). Every memory
access and every internal step of an instruction is one machine cycle of four
clocks, and a store happens at the end of its cycle. The CPU runs instruction
by instruction. The sound unit and the timer are not stepped. They catch up
to the cycle of each register access before it applies, and to the end of a
render before the samples are final.

## CPU

`cpu.rs` interprets the SM83 instruction set. An instruction is a sequence of
machine cycles, one for each access and each internal step. The halt bug, the
one-instruction delay of `ei` and interrupt entry follow the documentation.
An opcode that locks the CPU sets a flag, and the player reports it.

A GBS is played without interrupts. A routine that executes `halt` or `stop`
ends, and the player records a warning. `rst` goes to the vector plus the load
address, as the format asks.

## Bus

`bus.rs` maps ROM at 0x0000 to 0x7FFF with the bank register that a write to
0x2000..0x3FFF sets (0 selects 1). The data of the file lies at the load
address, bytes before it read as 0, and bytes past its end read as 0. RAM
covers 0x8000 to 0xDFFF, 0xFE00 to 0xFE9F and 0xFF80 to 0xFFFE. The echo area
mirrors work RAM, and the unusable area reads 0xFF. The timer, interrupt and
sound registers sit in 0xFF00 to 0xFF3F, and the other I/O addresses hold what
a driver stores there. Writes to the RAM banking registers are ignored.

## Timer

DIV is the upper byte of a 16-bit system counter. TIMA counts falling edges of
the counter bit that TAC selects. Both are computed from the cycle when they
are read, so the timer costs nothing between reads. A write to DIV resets the
counter and gives the edges that the hardware gives.

## The player

The player has two phases: a routine runs, or the machine waits for the next
call. The init routine starts at cycle 0. The first play call comes at the
later of one tick and the end of init. A later call comes one tick after the
previous one began, or at the end of the routine if that is later.

The tick follows the timer registers as they stand. With TAC bit 2 clear it
is one video frame, 70,224 clocks. With it set, it is the period of a timer
overflow, and a driver that changes TMA or TAC changes the rate. A routine that
has run for a second without returning is abandoned.

A routine starts as if the hardware had just raised its interrupt. LY reads
144, the start of VBlank. TIMA reads TMA and takes its first step 96 clocks
later. Drivers that read the timer to find out how late a call is get the
answers that a reference player gives with this phase.

## Sound

`apu.rs` implements the two pulse channels (the first with a sweep), the wave
channel, the noise channel and the frame sequencer. Each channel is an event.
It runs when its timer expires: a pulse step every (2048 - period) x 4 clocks,
a wave step every (2048 - period) x 2, a noise step by the divider and shift of
NR43. The frame sequencer ticks on the falling edge of bit 12 of the system
counter and runs lengths, sweep and envelopes on the steps the documentation
gives.

Registers are kept as written, and a read returns them with the unused bits
set. The unit powers off when NR52 bit 7 clears. All registers but the length
counters and wave RAM then clear and ignore writes. A trigger with the DAC off
does what a trigger does except that the channel stays off. The trigger and
length-counter quirks that the documentation lists are implemented.

## Output

The mixer reads the four digital outputs, converts each to its DAC level
(digital 0 is the top level), sums the channels that NR51 routes to each side,
and scales by the NR50 volume. A mask of four bits removes channels from that
sum. The levels it leaves out stay visible through `channel_levels`.

A high-pass filter follows the mixer, with the charge factor of the original
console, 0.999958 per clock. It runs on the mixer's events. Between events the
level is constant, and the filter's state moves by a factor that a table gives
for each count of clocks. A tick every 2048 clocks keeps its output current.
With no DAC on, the output is exactly 0 and the capacitor holds its charge.
The capacitor starts at the level of the idle DACs, so a song does not start
with a thump.

`audio/resample` turns the stream of level changes into samples at the
caller's rate. Each change is a band-limited step at its exact cycle. One
mixer unit is 48 steps of a 16-bit sample, so the full range fills 70 percent
of it, and float output uses the same scale.

## Arithmetic

The resampler's kernel is a table of fixed-point values. The filter's factors
are built by repeated multiplication, and its output is rounded with `floor`.
Each operation is an IEEE 754 basic operation, so the bits are the same on
every platform.

## Loading

`formats/gbs` reads the header: the song count, the first song, the load, init
and play addresses, the stack pointer, TMA and TAC, and three text fields.
The data after the header is the program. A file larger than 4 MiB is refused.
A load address above 0x7FFF is an error.

## Observer

`cgb_set_observer` registers callbacks that run on the rendering thread,
inside `cgb_render_*`, in program order: one for each store to 0xFF10..0xFF3F
and one for each call of a routine. A callback reads the machine only through
`cgb_cycles`. That call returns the cycle of the event. Every other call on
that machine returns at once with a neutral value, because the render that
runs the callback holds the machine.

## Snapshots

`snapshot` copies everything a machine is except the observer and the channel
mask. Those belong to the listener. The program data is shared between
copies. A restore checks the output rate and the file, then replaces the
state, and the same calls render the same samples from there. The output ring
is the largest part of a copy.

## End of a song

`end_state` reports a unit with no channel on, and a routine that did not
return, halted or locked the CPU. A song does not end in the file itself: the
play routine goes on being called. `quiet_cycles` counts the cycles since the
output level last changed. The machine has no rule for how long a silence
lasts before a song has ended.
