# Genesis core

A music-only Mega Drive (Genesis) in Rust. It plays VGM and VGZ files: it reads
the log of register writes, drives a YM2612 and an SN76489 with them on one
timeline, and returns the sound as interleaved stereo at the rate the caller
chooses. Nothing is drawn, and nothing reads input.

It covers the YM2612 (six FM channels of four operators, the envelope generator
with SSG-EG, the LFO, the timers, CSM mode and the DAC on channel 6) and the
SN76489 (three tones, noise, the Game Gear stereo write).

## Layout

- `rust/src`: `ym2612.rs` and `eg.rs` (the FM chip and its envelope generator),
  `psg.rs` (the SN76489), `machine.rs` (the public type, the scheduler and the
  output).
- `rust/tests`: the Rust tests.
- `tests/`: Swift Testing suites for the C interface.
- `DESIGN.md`: how the core works.
- `tools/oscilla-vgm`: the command-line player described below.

The VGM reader is the `oscilla-vgm` crate in `formats/vgm`. The resampler is
the `oscilla-resample` crate in `audio/resample`. The C header is
`swift/CGenesis/include/cgenesis.h`, a path from the root of the repository.

## API

```c
char error[256];
cgenesis_image *image;
cgenesis_image_load_vgm("song.vgm", &image, error, sizeof error);
cgenesis_machine *m = cgenesis_new(image, 44100);
cgenesis_image_free(image);
cgenesis_render_s16(m, buffer, frames);    /* interleaved stereo */
cgenesis_free(m);
```

A machine holds all of its state and renders the same samples on every run. A
song with a loop point passes it again and again, and the caller decides how
many passes to play. The image gives the GD3 tag fields (English and
Japanese), the chips that the file drives, the intro and loop lengths in
samples of 44,100 Hz or in frames of any rate, the number of passes that the
file's loop base and modifier ask for, and the gain of its volume modifier.
A field that the file leaves out is an empty string or 0.

A player has three more groups of calls.

- `cgenesis_snapshot_take` copies the whole machine. `cgenesis_snapshot_restore`
  puts a machine with the same file and output rate back to that point, and the
  same calls then render the same samples.
- `cgenesis_set_channel_mask` chooses which of the six FM channels and four PSG
  channels reach the mix.
- `cgenesis_set_highpass_hz` sets the cutoff of the output high-pass (20 Hz by
  default, 0 for none).

## oscilla-vgm

```
oscilla-vgm FILE.vgm [--seconds S] [--rate HZ] [--out FILE.wav] [--mask HEX]
oscilla-vgm FILE.vgm --info
oscilla-vgm FILE.vgm --bench [--seconds S] [--rate HZ]
```

The first form plays the file for S seconds (default 30) at 44,100 Hz and
writes a 16-bit stereo WAV file. `--mask` chooses the channels: bits 0 to 5 are
the FM channels and bits 6 to 9 the PSG's. `--info` prints the tags and the
times. `--bench` renders without output and prints the time it took.

## Tests

`cargo test -p oscilla-genesis --release` runs these groups.

- unit tests of the YM2612, the envelope generator and the SN76489.
- `machine`: pitch, loops, the end of a stream, the DAC, the masks and the
  volume modifier.
- `goldens`: a hash of the samples for each of 32 generated files. A change to
  the sound changes a hash.
- `fuzz`: files of random bytes run without a panic and render the same samples
  twice.

`cargo test -p oscilla-vgm --release` runs the reader's tests.

## Compatibility

The levels and the modulation depth were fitted to renders from libgme of 12
songs and 12 synthetic files. DESIGN.md lists the fits, the approximations and
the unknowns.

Not emulated: other sound chips that a VGM file can name (the second chip of a
pair, the YM2413 and the rest), the DAC stream commands (0x90 to 0x95), the GYM
format, and the analog stages after the chips. The output has a high-pass and
no low-pass.

## References

Written from public documentation.

- The VGM file format, version 1.71, and the GD3 tag format.
- The Yamaha YM2612 and YM3438 documents, the YM2608 application manual and the
  YM2203 manual for the envelope generator and the registers, the Sega Genesis
  Technical Overview, and the Genesis hardware research that Nemesis and others
  published.
- The Texas Instruments SN76489 documents and the SN76489 notes of SMS Power.
- The formulas of the Yamaha OPL log-sine and exponent tables, which the
  YM2612 shares.
