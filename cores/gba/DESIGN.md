# Design

The core runs a Game Boy Advance cartridge's own code so that the game's
sound driver plays, and returns the sound the hardware would produce. It has
no display, no input and no link port.

## Machine

A `Machine` holds all state: CPU, memory, timers, DMA, sound and the output
buffer. It allocates when it is created. Rendering allocates only to record a
warning. Use one machine from one thread at a time. Rendering is
deterministic: the same image produces the same samples on every run, and
`reset` repeats them. No step calls the platform's math library (see
Arithmetic).

## Clock and events

One clock drives everything: 16,777,216 cycles per second (`now`). The CPU
executes an instruction and reports the cycles it took. Whatever is due by
then runs next: display line timing, the four timers, the sound sample and
the 512 Hz PSG frame sequencer. Each event stores the exact cycle it belongs
to, so a handler that runs late never moves the next event. A timer's next
overflow is computed from its last one. `sys_schedule` keeps the earliest
event in `next_event`. A halted CPU waits for it.

## CPU

`cpu.rs` interprets ARMv4T: ARM and Thumb, the barrel shifter, banked
registers and exception entry. r15 reads as the instruction's address plus 8
in ARM state and plus 4 in Thumb state. The pipeline is not modelled; each
instruction is fetched when it executes. An instruction's cost comes from the
access it makes: sequential or non-sequential, by region, from the wait-state
tables that `bus_update_waits` rebuilds when WAITCNT changes. The cartridge
prefetch buffer is modelled by its effect on timing.

## Bus

Every access goes through `bus.rs`. It applies the region masks and mirrors,
open-bus values, the protection of BIOS reads and the rule for 8-bit writes
to video memory. I/O registers live in a byte array; writes with side effects
go through `io_write*`. Stores to the sound registers and the sound FIFOs are
reported to the observer.

## Timers, DMA and interrupts

`system.rs` keeps each of the four timers as a base time and a counter value.
A read works out the current count, and cascaded timers count the previous
timer's overflows. The four DMA channels start immediately, at VBlank, at
HBlank or from a sound FIFO request. A FIFO channel asks for more data when
fewer than 16 bytes remain and then moves four words. DMA runs between
instructions and adds its cycles to the clock. Display timing produces the
VBlank, HBlank and VCount flags and interrupts. Nothing is drawn.

## Sound

`apu.rs` implements the two square channels (the first with sweep), the wave
channel with its two banks, the noise channel and the two Direct Sound FIFOs.
The PSG channels are digital and unipolar: a channel at volume v outputs v or
0, and a stopped channel keeps its last level. A timer overflow shifts one
byte out of a FIFO into that channel's latch.

A native sample is formed every 512 cycles by default (32,768 Hz) and every
2^n cycles for the resolutions SOUNDBIAS selects. The mixer sums the PSG
channels per side, applies the master and PSG volumes, adds the FIFO latches
at 50% or 100%, adds the bias, clips to 10 bits and drops the low bits that
the selected resolution discards. A mask of six bits removes channels from
that sum. The levels it leaves out stay visible through `channel_levels`.

## Output

The hardware holds each native sample until the next. The output is a
staircase. `resample.rs` converts it to the caller's rate by adding every
stair as a band-limited step at its exact cycle. The step is a windowed-sinc
kernel with its cutoff at 0.45 of the output rate, stored as per-sample
differences and integrated on the way out. The method works for any native
rate, and the rate can change mid-stream. The kernel is a table of fixed-point
values in `tables.rs`, and each phase sums to exactly one, so the running
integral does not drift.

One level unit is 48 steps of a 16-bit sample, so the 10-bit range fills
three quarters of it. Float output uses the same scale.

## Arithmetic

The resampler's kernel and four BIOS calls (sine and cosine in the affine
routines, ArcTan, ArcTan2 and MidiKey2Freq) need math functions. A BIOS result
lands in guest memory, and math libraries round differently, so the core calls
none of them. These run on code that gives the same bits everywhere.

- The kernel and the sine and cosine of the 256 BIOS angles are constants in
  `tables.rs`.
- ArcTan, ArcTan2 and the power in MidiKey2Freq are in `exact.rs`. They use
  addition, subtraction, multiplication, division and fused multiply-add on
  `f64`. IEEE 754 defines each of those operations exactly.
- `tests/libm_equivalence.rs` compares them with the platform's math library
  on every input a game can pass. The two results agree on all of them.
- The power equals Apple's `exp2`. That function is one unit in the last
  place off at 84 keys, and `exact.rs` stores its values for those keys.

## BIOS

The BIOS is a small ARM program in the BIOS region plus routines written in
Rust. The program holds the interrupt trampoline and the Halt, Stop,
IntrWait and VBlankIntrWait loops. The trampoline saves r0-r3, r12 and lr and
calls the handler the game stores at 0x03007FFC. The loops have to let
interrupts run while they wait. `tools/bios.s` is its source and `tools/bios-table.sh` prints the table
in `bios.rs`. Every other call runs in Rust at the SWI instruction, through
the same bus the CPU uses, so its stores reach the observer like the game's
own. Routines that take a count from the guest cap it.

## Loading

`image.rs` reads GSF and minigsf files. Both are PSF containers. The
container reader is the `oscilla-psf` crate. A program section gives an entry
address, a load address and a size; its bytes are zlib-compressed and a Rust
inflater expands them. `_lib`, `_lib2` and so on name library files in the
same folder. They load first, depth first, and each file's own data lands on
top. A tag from a nearer file wins over a library's. A library name is a plain
file name, and a set opens at most 64 files.

## Observer

`cgba_set_observer` registers callbacks that run on the rendering thread,
inside `cgba_render_*`, in program order: one for each store to the sound
registers (address, value, width) and one for each byte a Direct Sound FIFO
plays. A callback reads the machine only through `cgba_cycles`. That call returns
the cycle of the event. Every other call on that machine returns at once with
a neutral value, because the render that runs the callback holds the machine.

## Snapshots

`snapshot` copies everything a machine is except the observer and the channel
mask. Those belong to the listener. The ROM is shared between copies. A
restore checks the output rate and the image, then replaces the state, and the
same calls render the same samples from there. The output ring is the largest
part of a copy.

## End of a song

`end_state` reports two conditions in which the machine cannot make sound
again: the CPU waits for an interrupt and no register enables one, or it runs
a branch to itself and interrupts cannot break in. `quiet_cycles` counts the
cycles since the output level last changed. The machine has no rule for how
long a silence lasts before a song has ended.
