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

A file without the version 2 interrupt flags has no interrupts. The flags that
would raise one stay readable, so a driver that polls the frame counter's flag
works. The section on interrupts covers the other files.

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

## Interrupts

A version 2 file with bit 4 (interrupts) or bit 5 (non-returning init) of its
feature byte set gets the interrupts of the NSF2 specification, in `irq.rs`.
The run loop for any other file never reaches that code.

The IRQ vector at 0xFFFE and 0xFFFF is RAM. It starts each song with the bytes
that the program has there. A store to either address reaches the RAM and also
the chips that decode the address, such as the Namco 163's address port. The
NMI and reset vectors belong to the player and are not modeled.

The timer has a reload value at 0x401B (low byte) and 0x401C and a control
register at 0x401D. A store with bit 0 set starts it, and the period is the
reload value plus one cycles, counted from the cycle of the store. Each expiry
sets the timer's flag. A read of 0x401D returns the flag in bit 7 and the
active state in bit 0, and clears the flag. A store with bit 0 clear stops the
timer and leaves the flag. A new reload value takes effect at the next expiry.
The next expiry is computed from the cycle, so the timer costs nothing between
expiries and does not depend on how the host divides its calls.

The IRQ line is held while the timer's flag, the frame counter's flag or the
delta channel's flag is set. The CPU takes it between instructions when the I
flag is clear: seven cycles that read the next instruction twice, push the
program counter and the flags with the break bit clear, set I and read the
vector. CLI, SEI and PLP change the flag the poll sees one instruction late, as
on the chip. The poll looks at the end of an instruction, where the chip looks
before its last cycle, so an interrupt that arrives in that cycle is taken one
instruction early. BRK still stops the routine with a warning.

The play routine arrives as an NMI. The CPU pushes the program counter and the
flags, the player's wrapper saves A, X and Y, and the routine runs with the I
flag set. The routine returns to the wrapper, and the wrapper puts everything
back. The entry
takes seven cycles and the wrapper none. A call that falls due while the play
routine runs waits for its return and then runs at once, and an NMI can
interrupt an interrupt routine. The first init routine gets no NMIs and is
limited to a second, like any routine.

When no routine runs, the CPU is in the player's loop, an address in the vector
area. Interrupts reach it there and return to it. The machine jumps to the next
event, so the loop costs no time.

With bit 5, the first init routine has Y = 0x80 and must return. The player
then calls the init routine again with the same A and X and Y = 0x81. That call
has no time limit and the NMIs interrupt it. If it returns, the machine falls
back to the loop, and the play routine goes on. Bit 6 stops the NMIs.

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

The VRC7 has six channels of two operators, a modulator and a carrier. The
chip makes one sample for every 72 cycles of its own 3.579545 MHz clock, 49716
Hz. That is 36 cycles of the NTSC console, and a count that is not whole on the
PAL console, because the chip's clock does not follow the console's. Each
operator keeps a 19-bit phase that moves on by the F-number shifted left by the
octave, times the multiplier of the table. The vibrato changes that step. The
top ten bits of the phase pick a place in the sine. The sine comes from the two
tables whose formulas the decapsulation report of the related YM3812 gives, a
log-sine of 256 samples for a quarter wave and an exponential of 256 samples.
Both are in constants, and a test derives them again from the formulas. The unit
is 1/256 of an octave, so an envelope step of 0.375 dB is 16 units. The output
of an operator is the exponential of the log-sine plus its attenuation, which
is the envelope, the total level in steps of 0.75 dB (modulator only), the key
level scale of the manual's table in the setting that the register names, the
tremolo, and for the carrier the channel volume in steps of 3 dB. A carrier
at full level peaks at 4088. The modulator's output moves the carrier's phase
by half its value, so that a full output is 4 pi, the largest index of the
feedback table. The feedback moves the modulator's own phase by the mean of its
last two outputs scaled the same way, by the index of the feedback table. The
waveform bits select the sine or its positive half.

The envelope's attenuation runs from 0 to 127 steps. The manual's table gives
the time that the whole range takes to fall, from rate 0 to 63, where the rate
is four times the parameter plus the key scale offset of its table (the
offset is 0 when the parameter is 0, and then the stage halts). The core takes
8128 samples at rate 32, half as long at each rate four higher, and shorter by
4 / (4 + n) at the n-th rate within a group of four. The manual labels the times
as 40 dB; they match the whole range of 127 steps of 64 samples at rate 32, and
the core reads them that way. The manual calls the table error-prone in its
lower rows. The core follows the structure that the upper rows show. The
fastest group of four takes 1.27 ms whatever its position. An attack takes the
attenuation down by a share of what is left at each sample, so that it falls
from the whole range to under one step in the table's attack time. Rates of 60
and above attack at once. The manual's attack column departs from the pattern
at rates 48 to 59, and the core uses its printed values there. A decay falls
in a straight line to the sustain level, which is in steps of 3 dB. Then a
sustained operator holds, and a percussive operator (sustain bit clear) goes on
into its release. A release starts from the current level. After a key off,
a channel with its sustain bit set releases at rate 5, a percussive operator at
rate 7 and a sustained one at its patch's rate. A modulator with its sustain
bit set ignores the key off. When a key goes on while the envelope is not at
its end, the envelope first falls to the end in about 10 ms and the phase
restarts there. The manufacturer's four-page datasheet gives that time and the
rate 7 of a percussive release in a figure that is read from a scan.

The tremolo is a triangle of 210 steps that moves once in 64 samples (3.7 Hz),
with a peak of 4.3 dB. The vibrato is a triangle of eight steps that moves once
in 1024 samples (6.07 Hz), with a peak of 14 cents. The test register speeds
them up as the wiki describes. The reset bit of 0xE000 clears the registers and
the tremolo and leaves the vibrato where it is.

The documents leave these open, and the core chooses as follows. The manual's
EG resolution of 0.325 dB is read as 0.375 dB, because the manual's other steps
are multiples of that and 48 dB divided by it is 128. Two entries of the
manual's key level table (2.525 and 14.825) are read as 2.625 and 14.625. The
shape of the tremolo and the vibrato is a triangle, and the vibrato rate follows
the wiki's ratio of 1024 samples a step with eight steps a cycle, where the
manual says 6.4 Hz. The tremolo depth is the 4.3 dB of the hardware note, where
the related YM3812's documents say 4.8 dB. The feedback input is the mean of the
last two outputs. The scale of the modulator's output against the carrier's
phase is the feedback table's largest index. The log-sine and exponential
tables are the YM3812's, and the VRC7 is taken to use the same ones. The
envelope time of the damp before a key on and the rate 7 come from a scan. The
core does not model the delay that the chip needs between two stores to its
ports, and it does not model the DAC, which sends the channels out one at a
time. The level against the 2A03 is not documented. The gain is 2.32 times the
level at which a channel's full output equals a pulse step, which puts the
mean of three songs at the level of a reference player's render. Against that
render the songs lie between 1.6 dB below and 1.5 dB above, and the register
stores of the songs match it exactly over 1800 play calls each. Bits 23 to 28
of the mask take the channels out.

The MMC5 has two pulse channels, a sample register and a multiplier. A pulse
channel is the 2A03's pulse channel without the sweep unit: the same duty
sequences, the same timer (a step every two times the period plus one cycles)
and the same length table. A period below 8 does not silence it. The chip has
no frame sequencer. One counter clocks the envelopes and the length counters
every 7424 cycles, about 241 Hz, so a length counter runs twice as fast as the
2A03's. The documentation takes that rate from die images and says it has not
been measured, and it is unsure whether the length counter shares the
envelope's clock. The core follows the documentation as written. The counter
starts at the first cycle of a song, because the phase is not documented.
Register 0x5015 enables the two channels and reports their length counters. A
pulse channel is as loud as a 2A03 pulse channel at the same level, as the
documentation says, summed with the 2A03's mix. The sample register at 0x5011
holds eight bits and has the step size of the delta channel at 127, so its full
value is twice as loud as the delta channel's. A store of zero leaves the
register as it is and sets its interrupt flag, and the core does not deliver
that interrupt. In read mode (bit 0 of 0x5010) a read of 0x8000 to 0xBFFF sets
the register to the byte read, unless that byte is zero. The multiplier
returns the product of its operands at once from 0x5205 and 0x5206. The core
starts both operands at 0 and the sample register at 0, because the
documentation gives no power-on values. RAM covers 0x5C00 to 0x5FF7; the
format keeps 0x5FF8 to 0x5FFF for the bank registers. The core writes nothing
to 0x5015 at the start of a song, because the format gives no such step. Bits
20 to 22 of the mask take the pulse channels and the sample register out. No
reference player and no file with the chip were available, so the tests check
the documentation's statements with drivers built from bytes.

The VT02 puts a second sound generator beside the 2A03's. Its sources are the
NSF specification (bit 6 of the chip byte), the NESdev page on VT02+ sound and
the manufacturer's VT02 datasheet, revision A5. The second generator has
pulse 3, pulse 4, triangle 2 and noise 2 at 0x4020 to 0x402F, laid out as the
first generator's registers at 0x4000 to 0x400F, and the datasheet's register
tables for the two sets are the same. A channel of the second generator is the
matching 2A03 channel: the same sweep, envelope, length table, timer periods
and noise table. The tests run the same stores through both generators and
require the same output at every sample. The datasheet sets the beat clocks of
both generators with one register, 0x4017, so the second generator takes its
quarter-frame and half-frame clocks from the same frame counter. Register
0x4035 enables the four channels and reports their length counters, as 0x4015
does for the first generator. Both generators swap the 25% and 50% duty
settings, as the datasheet's table and the NESdev page say.

Register 0x4030 switches the outputs. Bit 2 set mutes the first generator and
bit 3 set turns the second on. The register is 0 at the start, which is the
console's own behavior, so a driver turns the second generator on itself. The
datasheet gives each generator its own output pin. The core mixes each
generator through the 2A03's two tables and adds the results.

Bit 4 of 0x4030 makes the delta channel play its bytes as 8-bit PCM. Each
timer expiry, at the delta channel's rates, moves the next byte to the PCM
level, which is the level that a store to 0x4031 sets. A byte costs the CPU
four cycles, as a delta channel byte does. The PCM level goes to the second
generator's output. The datasheet counts the DMA address bits 14 and 15 as the
complement of bits 0 and 1 of 0x4030, so the DMA can read RAM and registers as
well as the program. The address counts through its low 15 bits and keeps bit
15. Bit 5 changes the length from 16n+1 bytes for a register value n to 16(n+1)
bytes. A store to 0x4030 applies the new bits to the last stores to 0x4012
and 0x4013.

What the public documents leave out, and what the core does about it:

- The level of the second generator against the first is not documented. The
  core uses the same tables for both.
- The PCM step is not documented. The core makes the full eight-bit value as
  loud as the delta channel at 127, so a step is half the delta channel's.
- Bits 6 and 7 of 0x4030 are called loudness bits with no meaning given. The
  core keeps them and does nothing with them.
- The datasheet does not say whether the second generator has a frame counter
  of its own. The core shares the first generator's.
- The negate sweep of pulse 3 and pulse 4 is taken to match pulse 1 and
  pulse 2.
- The meaning of the maximum length bit comes from its two stated values, 4081
  and 4096 bytes. The core reads them as 16n+1 and 16(n+1).
- The PCM level holds its last value when the sample ends. The end raises the
  delta channel's interrupt flag when the interrupt is enabled, as in delta
  mode.
- The other V.R. Technology chips (VT03 and later) add features that the NSF
  format does not name, and the core has none of them.

No reference player and no file with the chip were available. The tests check
the documents' statements with drivers built from bytes. Bits 29 to 33 of the
mask take pulse 3, pulse 4, triangle 2, noise 2 and the PCM out. The mask is
64 bits wide for them.

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
version 2 file may give the length of the program, and the bytes after it are
NSFe chunks: the same chunks as below except INFO, DATA, BANK and NSF2, which
are skipped there. Bit 7 of the feature byte says that one of them is
mandatory; with it, an unknown mandatory chunk refuses the file, and without
it that chunk is skipped. A field that the auth chunk leaves empty keeps the
header's text, and RATE and regn replace the header's speeds and region when
the chunk is there. A file larger than 4 MiB is refused. A load address below
0x8000 is an error.

An NSFe file (the first four bytes are `NSFE`) is a list of chunks, each with a
length, a four-letter ID and a body. INFO gives the addresses, the console and
chip bytes, the song count and the first song, counted from 0. DATA gives the
program. BANK and RATE give the banks and the play speeds, and NSF2 the
feature bits. The reader fills the same fields as for an NSF header, so the
core runs the file as it runs any other. The chunks auth, text, tlbl, taut,
plst, time and fade give the title, the author, the copyright, the ripper, a
comment, a name and an author for each song, the playlist and the time and
fade of each song. They reach the player and the core does not use them. A
chunk whose ID starts with an uppercase letter is mandatory, and a file with
one that the reader does not know is refused. INFO must come before DATA, and the reader stops at NEND or at the end of the
file. A chunk that runs past the end of the file is
dropped, and DATA keeps the bytes that are there. Text that is not UTF-8
becomes U+FFFD. A playlist entry that names a song the file does not have is
dropped. A time or fade of a song beyond the end of its chunk is missing, and a
negative value is missing too.

## Observer

`cnes_set_observer` registers callbacks that run on the rendering thread,
inside `cnes_render_*`, in program order: one for each store to 0x4000..0x4017
and one for each call of a routine. A callback reads the machine only through
`cnes_cycles`. That call returns the cycle of the event. Every other call on
that machine returns at once with a neutral value, because the render that
runs the callback holds the machine.

## Visualization

`Machine::voice` reads one channel's registers, counters and envelope. The
voices follow the order of the channel mask, and only the chips that the header
names have voices, so each voice carries its mask bit. A pitch is the rate at
which the channel's own timer, as the events run it, goes through its waveform:
8 steps of 2 (P + 1) cycles for a 2A03 or MMC5 pulse with the period P, 32
steps of P + 1 cycles for a triangle, 16 steps of P + 1 cycles for a VRC6 pulse
and 14 for its saw (P shifted as 0x9003 asks), 32 P cycles for a 5B tone, 2^24
/ (64 F) updates of 16 cycles for the FDS, 65536 L / F updates of 15 n cycles
for an N163 channel with a waveform of L samples and n channels enabled, and
2^19 / (F 2^B) samples of 72 clocks of 3.579545 MHz for the VRC7. The FDS's
modulation and the VRC7's vibrato are not in the pitch, and `pitch_modulated`
is set while they move it. A volume is a volume register or an envelope over
its largest value, and 0 while a counter, an enable bit, a period or sweep
limit, the VRC6's halt bit or the VT02's output switch keeps the channel
silent. The 5B's follows its volume curve, the VRC7's is the linear level of
the carrier's attenuation and the volume register, and the FDS's is the gain
times the master volume. A wavetable's number is a hash of its samples, so
equal waveforms have equal numbers. The voice of an N163 channel that is not
enabled is empty, because its registers may hold waveforms.

The scope keeps the last 4,096 samples of each voice at 44,100 Hz. The channels
change only at the sound unit's events, so the event loop takes the samples
that are due before each event that can change a channel, and once at the end
of each catch-up. The samples hold the values that the channels had until then,
and the values are worked out again only after an event or a register store
that can have changed them. A tick of the output filter, a step of a noise
generator that reaches no output, a step of the delta channel with no bits to
play, a flip of a 5B tone that is off or silent, a step of an envelope that no
5B channel uses and a tick of a halted FDS change nothing, and no samples are
taken before them. A tone or noise sample is the channel's output about the
middle of its range at its own level, so that a tone swings about 0 and a
silent channel is 0: a pulse is plus or minus its volume, a triangle twice its
step less 15, a 5B channel plus or minus its amplitude, an N163 channel its
last output (its sample less 8, times its volume), the FDS twice its sample
less 63 times its gain and master volume, and a VRC7 channel its output. A
triangle that its counters stop is 0, although the chip holds its last step.
The delta channel and the PCM registers are their levels from 0. The mask does
not touch the scope. `scope` resamples it to the output rate by linear
interpolation. The ring is not part of a copy of the machine, which starts with
an empty one and keeps the clock, so a restore lines the scope up with the
restored cycle. Reading the voices and the scopes changes nothing in the sound.

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
