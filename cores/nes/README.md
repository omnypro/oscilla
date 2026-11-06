# NES core

A music-only NES in Rust. It plays NSF and NSFe files: it runs an NES game's sound
driver and returns the sound as interleaved stereo at the rate the caller
chooses. Nothing is drawn, and nothing reads input.

It covers the 6502 CPU, the memory map with the NSF bank registers, the 2A03's
sound unit, the VRC6, VRC7, Sunsoft 5B, Namco 163, Disk System, MMC5 and VT02 expansion chips and the NSF
player that calls the driver's
init and play routines.
Both consoles are covered: the clock, the tables and the play rate follow the
region.

## Layout

- `rust/src`: `cpu.rs` (the instruction set), `bus.rs` (memory map and bank
  registers), `apu.rs` (sound), `machine.rs` (the public type and the
  player's schedule), `state.rs`.
- `rust/tests`: the Rust tests.
- `tests/`: Swift Testing suites for the C interface.
- `DESIGN.md`: how the core works.
- `tools/oscilla-nsf`: the command-line player described below.

The NSF and NSFe reader is the `oscilla-nsf` crate in `formats/nsf`. The
resampler is the `oscilla-resample` crate in `audio/resample`. The C header is
`swift/CNes/include/cnes.h`, a path from the root of the repository.

## API

```c
char error[256];
cnes_image *image;
cnes_image_load_nsf("song.nsf", &image, error, sizeof error);
cnes_machine *m = cnes_new(image, 48000);
cnes_image_free(image);
cnes_select_song(m, 3);                    /* counted from 0 */
cnes_render_s16(m, buffer, frames);        /* interleaved stereo */
cnes_free(m);
```

A machine holds all of its state and renders the same samples on every run.
`cnes_new` picks the console from the header. A header that names both
consoles gets NTSC, and `cnes_new_for` takes the console from the caller.
`cnes_set_observer` reports every store to `0x4000` through `0x4017` (and to the
registers of the expansion chip in a file that asks for the chip) and every call of the init
and play routines. Each play call is the driver's own tick, so
a host can digest the stores per call. Rust code uses `oscilla_nes::Machine`
directly.

An NSFe file loads through the same calls. The image then also gives the
ripper, a comment, a name and an author for each song, the time and fade of
each song in milliseconds, the length as the two added, and the playlist.
`cnes_image_is_nsfe` tells the two containers apart. A value that the file
leaves out is an empty string or -1. An NSF file leaves all of them out unless
it is a version 2 file with NSFe chunks after its program. The same calls read
those chunks.

A player has four more groups of calls.

- `cnes_snapshot_take` copies the whole machine. `cnes_snapshot_restore` puts a
  machine with the same file, console and output rate back to that point, and
  the same calls then render the same samples.
- `cnes_set_channel_mask` chooses which of the thirty-four channels reach the mix. The mask is 64 bits wide.
  `cnes_channel_levels_get` shows what each channel of the 2A03 outputs, and
  `cnes_expansion_levels_get` shows the expansion chip's, muted or not.
  `cnes_voices_get` returns what each channel of the file's chips is doing
  (its pitch, volume, envelope, duty, waveform or patch, and its mask bit), and
  `cnes_scope_get` returns the last frames that one of them put out, for a
  display.
- `cnes_set_highpass_hz` moves the bass cutoff, from 1 to 1000 Hz, while a song
  plays. The default is the console's own filter at 90 Hz, and
  `cnes_use_console_highpass` returns to it. `cnes_highpass_hz` reads the
  cutoff in use. A reset, a song change and a snapshot restore keep the choice.
- `cnes_end_state_get` reports a unit with no channel on and a routine that
  never returned. `cnes_quiet_cycles` counts the cycles since the output level
  changed.

## oscilla-nsf

```
oscilla-nsf FILE.nsf [--song N] [--seconds S] [--rate HZ] [--region ntsc|pal]
                     [--highpass HZ] [--out FILE.wav] [--writes FILE.txt]
oscilla-nsf FILE.nsf --info
oscilla-nsf FILE.nsf [--song N] --bench [--seconds S] [--rate HZ]
```

The first form plays song N (default 0) for S seconds (default 30) and writes
a 16-bit stereo WAV file. `--writes` adds a text file with one line per store
to the sound registers: the song, the play call (-1 for the init routine), the
cycle since that call began, the address and the value. `--info` prints the
header and, for an NSFe file, the song data. `--bench` renders without output
and prints the time it took.

## Tests

`cargo test -p oscilla-nes --release` runs these groups.

- `cpu`: results, flags and the cycles of each addressing mode.
- `sound`: one behavior of the sound unit at a time, with drivers built from
  bytes.
- `vrc6`: the expansion chip's pulse channels, saw, frequency control and mix.
- `s5b`: the Sunsoft 5B's tones, noise, envelope shapes and volume curve.
- `n163`: the Namco 163's ports, sound RAM and channels.
- `nsf2`: the interrupt timer, the vector, the NMI call of the play routine
  and the init routine that does not return.
- `fds`: the Disk System chip's wavetable, envelope, modulation unit and RAM.
- `mmc5`: the MMC5's pulse channels, sample register, multiplier and RAM.
- `vrc7`: the VRC7's registers, envelope, pitch, low frequency oscillators and mix.
- `vt02`: the VT02's second generator, output switches, raw PCM and DMA.
- `viz`: the voices of the chips that a file has, their state and their
  scopes, and no change to the sound.
- `player`: the schedule of init and play calls, both consoles, bank
  switching, routines that misbehave, snapshots and the end state.
- `goldens`: a hash of the samples and of the stores for each of 64 generated
  drivers, and for 16 each that drive the VRC6, the 5B, the Namco 163, the FDS, the MMC5, the VT02 and the VRC7. A change to the sound or to any trace changes a hash.
- `highpass`: the bass cutoff's range, default, decay time and behavior across
  a change, a reset and a restore.
- `cartridges`: public test cartridges for the CPU and the sound unit. They are
  not in the repository. `OSCILLA_NES_TESTS` names a folder that holds them and
  a `manifest.txt` that lists each one with a cycle limit, and optionally the
  path of a log of the CPU's state. The CPU test with a log matches every
  line, and the cartridges that report at 0x6000 pass: the instruction tests,
  the instruction timing tests and the sound unit tests for the length
  counters, the frame counter's flag, jitter and the delta channel. The test
  skips without the variable.

## Compatibility

The core plays NSF versions 1 and 2, up to the program length of version 2, and
NSFe files. A
comparison with the register-store streams of a reference player gave the same
stores in the same order in every play call for all 300 songs of six files,
two of them bank-switched, one each with the VRC6, the 5B, the Namco 163 and
the FDS (12 of its 56 songs), over 3,600 play calls each.

An NSFe file runs the same program as the NSF it came from. The core reads the
chunks INFO, DATA, BANK, RATE, NSF2, regn, auth, text, tlbl, taut, plst, time
and fade. It skips mixe, psfx and VRC7 (levels are the ones the core documents,
and the VRC7 plays the chip's 15 patches), and it refuses a file with a mandatory chunk
(first letter uppercase) that it does not know. Of regn it uses the NTSC and
PAL bits and the preferred region. Dendy has no region of its own here, so a
file that names only Dendy keeps the region bits of INFO. A chunk that runs past the end of the file
is left out, except DATA, which keeps the bytes that are there. Files
converted from NSF files play the same stores and samples as the originals.

The version 2 features run. A file that sets bit 4 or bit 5 of its feature
byte gets the interrupt timer at 0x401B to 0x401D, the IRQ vector as RAM at
0xFFFE, interrupts from the timer, the frame counter and the delta channel, and
the play routine as an NMI. Bit 5 also gives the second init routine that need
not return, and bit 6 stops the calls of the play routine. The tests check the core against the specification with drivers built from
bytes. DESIGN.md lists the approximations.

Not emulated: the MMC5's interrupt from its sample register, the VT02's two
loudness bits (the documents give them no meaning) and its successors' extra features,
interrupts in a file
that does not ask for them in its version 2 flags (a driver that waits for the
frame counter's flag can poll it, and one that enables interrupts never
receives them), BRK, the display, input, the undocumented opcodes whose results depend on the
chip's analog behavior (the core skips them and warns), and the exact timing
of a sample read inside an instruction (a read costs the CPU four cycles). The
output has the console's 90 Hz high-pass filter and its 14 kHz low-pass filter.
`cnes_set_highpass_hz` moves the high-pass corner. The loudest drums reach past
full scale, and a host that needs headroom scales the output.

## References

Written from public documentation.

- The NESdev wiki: the APU pages, the CPU pages and the NSF specification.
