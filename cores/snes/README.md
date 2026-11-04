# SNES core

A music-only SNES in Rust. It plays SPC files: it restores the sound
processor's RAM and registers from the file, runs the sound driver that the
RAM holds, and returns the sound as interleaved stereo at the rate the caller
chooses. Nothing is drawn, and nothing reads input.

It covers the SPC700, the S-DSP with its eight BRR voices, envelopes, noise,
pitch modulation and echo, the timers and ports of the sound unit, and a
stand-in for the 64-byte boot ROM.

## Layout

- `rust/src`: `smp.rs` (the SPC700 and its ports), `dsp.rs` (the S-DSP),
  `machine.rs` (the public type, the clock and the output), `boot.rs` (the boot
  ROM), `gauss.rs` (the interpolation table).
- `rust/tests`: the Rust tests.
- `tests/`: Swift Testing suites for the C interface.
- `DESIGN.md`: how the core works.
- `tools/oscilla-spc`: the command-line player described below.

The SPC reader is the `oscilla-spc` crate in `formats/spc`. The resampler is
the `oscilla-resample` crate in `audio/resample`. The C header is
`swift/CSnes/include/csnes.h`, a path from the root of the repository.

## API

```c
char error[256];
csnes_image *image;
csnes_image_load_spc("song.spc", &image, error, sizeof error);
csnes_machine *m = csnes_new(image, 48000);
csnes_image_free(image);
csnes_render_s16(m, buffer, frames);       /* interleaved stereo */
csnes_free(m);
```

A machine holds all of its state and renders the same samples on every run.
`csnes_set_observer` reports every store to a DSP register and to the four
output ports, with the SPC700 clock of the store. An SPC file holds one moment
of a driver, so there is one song and no init or play routine.

The image gives the tags (song, game, artist, dumper and comments), the time
and fade in milliseconds, the length as the two added, the voices that the file
asks a player to mute, and the amplification of the xid6 chunk. A value that the
file leaves out is an empty string, -1 or the unchanged value.

A player has four more groups of calls.

- `csnes_snapshot_take` copies the whole machine. `csnes_snapshot_restore` puts
  a machine with the same file and output rate back to that point, and the same
  calls then render the same samples.
- `csnes_set_channel_mask` chooses which of the eight voices and the echo reach
  the mix. `csnes_voice_levels_get` shows what each voice outputs, muted or not.
- `csnes_write_port` and `csnes_read_port` stand in for the main CPU: they set
  what the SPC700 reads at 0xF4 to 0xF7 and show what it wrote there.
- `csnes_end_state_get` reports a processor that executed SLEEP or STOP.
  `csnes_quiet_cycles` counts the cycles since the output level changed.

## oscilla-spc

```
oscilla-spc FILE.spc [--seconds S] [--rate HZ] [--out FILE.wav]
                     [--writes FILE.txt] [--mask HEX]
oscilla-spc FILE.spc --info
oscilla-spc FILE.spc --bench [--seconds S] [--rate HZ]
```

The first form plays the file for S seconds (default 30) and writes a 16-bit
stereo WAV file. `--writes` adds a text file with one line per store to a DSP
register: the sample, the SPC700 clock, the register and the value. `--mask`
chooses the voices and the echo (bit 8). `--info` prints the tags and the times.
`--bench` renders without output and prints the time it took.

## Tests

`cargo test -p oscilla-snes --release` runs these groups.

- `smp`: results, flags and the cycles of the SPC700's instructions.
- `dsp`: the BRR decoder, the envelope, the mixer, noise, pitch modulation and
  the echo unit, each with a file that holds a few registers and blocks.
- `machine`: the ports, the timers, the boot ROM, the observer, snapshots and
  the rendered output.
- `goldens`: a hash of the samples and of the DSP stores for each of 32
  generated files. A change to the sound or to the stores changes a hash.
- `fuzz`: files of random bytes run without a panic and render the same samples
  twice.
- `reference`: a comparison with renders and store logs of a reference player.
  The files are not in the repository. `OSCILLA_SNES_REF` names a folder with a
  NAME.spc, NAME.wav and NAME.log.txt for each song, and the test prints how
  close the machine comes. The test skips without the variable.

## Compatibility

A comparison with the logs of the DSP stores that a reference player made for
35 songs from six games gave the same stores in the same order for every song,
over 3.3 million stores, and the same SPC700 clock for every store. DESIGN.md
lists the approximations, the unknowns and the measured differences from the
reference player's audio.

Not emulated: the TEST register (wait states and the switches that stop the
timers or the RAM writes), a boot ROM that takes blocks of more than 256 bytes,
and the analog stages after the DSP. The output has no filter and no phase
inversion.

## References

Written from public documentation.

- The SPC file format, version 0.30, with the xid6 extension.
- nocash's fullsnes: the APU chapters on the SPC700, the DSP registers, BRR,
  the Gaussian table, the envelopes, noise, echo and the low-level timings.
