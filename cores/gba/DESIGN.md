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

## Visualization

`voice` reads a channel's registers and the state that the mixer reads, in the
order of the channel mask. A pitch comes from the periods that the channel
timers use: a pulse cycle is eight duty steps, so 131,072 / (2048 - X) Hz, and
a wave cycle is 32 samples, or 64 when SOUND3CNT_L selects both banks. A FIFO's
rate is its timer's overflow rate, worked out from the reload value and the
prescaler, or for a cascaded timer 1 from timer 0's rate. A channel's volume
is its envelope over 15, the wave channel's output level, or a FIFO's 50 or
100 percent. The master volume and the ratio for the tone channels are left
out, because the mixer applies them to the sum after the mask. The wave number
is a hash of the bank that plays (both banks in 64-sample mode).

The scope keeps the last 4,096 samples of each channel at 44,100 Hz. The tap
is in `apu_sample`: each native sample adds the scope samples that fall due by
its cycle and holds the channel's level between them. The count follows the
cycles, so a change of the native rate does not change the scope's rate. A
pulse, wave or noise sample is the level about the middle of its swing (twice
the level less the level of a high step), so a square at volume 15 swings
from -1 to 1. A FIFO sample is its latch at 50 or 100 percent. Each is taken
as the average of the left and right outputs (so one side alone reads half), and the mask is not in
it. `scope` resamples to the output rate by linear interpolation. The ring is
not part of a copy of the machine, which starts with an empty one, and a reset
clears it. It allocates with the machine, and taking a sample allocates
nothing.

## Register blocks

The register blocks show what a program stored, for a register display. There
are five. Each has a name, a base address and a size, and the player lists them
by name.

| Block | Name                 | Base         | Bytes |
| ----- | -------------------- | ------------ | ----- |
| 0     | Sound                | `0x04000060` | 42    |
| 1     | Wave RAM             | `0x04000090` | 32    |
| 2     | FIFO                 | `0x040000A0` | 8     |
| 3     | Timers | `0x04000100` | 16    |
| 4     | DMA 1 and 2          | `0x040000BC` | 24    |

The sound block runs from SOUND1CNT_L (0x60) to the second byte of SOUNDBIAS
(0x88 and 0x89). The registers between them are 0x60 SOUND1CNT_L, 0x62
SOUND1CNT_H, 0x64 SOUND1CNT_X, 0x68 SOUND2CNT_L, 0x6C SOUND2CNT_H, 0x70
SOUND3CNT_L, 0x72 SOUND3CNT_H, 0x74 SOUND3CNT_X, 0x78 SOUND4CNT_L, 0x7C
SOUND4CNT_H, 0x80 SOUNDCNT_L, 0x82 SOUNDCNT_H, 0x84 SOUNDCNT_X and 0x88
SOUNDBIAS. The halfwords at 0x66, 0x6A, 0x6E, 0x76, 0x7A, 0x7E and 0x86 have no
register and stay 0, so the block is 0x2A bytes.

The I/O byte array is not the source of the sound block, because the sound
rules change it: stores below 0x82 are dropped while the unit is off, turning
the unit off zeroes 0x60 to 0x81, and SOUNDCNT_X keeps only its top bit.
`ApuState::written` is a 42-byte copy that `apu_write` updates first, merging
each store with its byte mask. It records stores made while the unit is off,
keeps its bytes through a power-off, and holds the strobe and trigger bits as
written. A reset starts it at zero except SOUNDBIAS, which takes the 0x0200
that the BIOS leaves. The cost is one halfword merge per sound-register store.

Wave RAM holds bank 0 in bytes 0 to 15 and bank 1 in bytes 16 to 31. A CPU
store to 0x04000090 reaches the bank that bit 6 of SOUND3CNT_L does not select,
and the selected bank plays. Bank 1 has no address of its own, so byte 16 is
not at 0x040000A0. The block reads the two banks directly. They hold the bytes
as written.

The FIFO block is `ApuState::fifo_written`, eight bytes that the bus updates for
every store to 0x040000A0 to 0x040000A7 at the byte positions of the store.
The FIFO itself keeps its queue of 32 bytes and drops stores when it is full,
so it cannot show the last word. A DMA that feeds a FIFO stores through the
same path and appears in the block.

The timer block and the DMA block read the I/O byte array. Stores to those
registers reach it as written, before the masks that the timer and DMA state
apply (bit 11 of DMA 1 and 2, and the enable bit that a finished transfer
clears). An 8-bit store to a DMA control register merges its other byte from
the DMA state, so that byte can differ from the last value stored to it. The
DMA block lists the source address, destination address, count and control
value of DMA 1, then the same of DMA 2, in the order of the registers from
0x040000BC. DMA 0 and DMA 3 do not feed the FIFOs and have no block.

`written` and `fifo_written` are part of the machine state. A copy and a
snapshot carry them, and `size` counts them in `size_of::<Machine>()`.

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
