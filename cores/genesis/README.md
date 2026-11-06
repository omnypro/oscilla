# Genesis core

A music-only Mega Drive (Genesis) in Rust. It plays VGM, VGZ and GYM files: it reads
the log of register writes, drives a YM2612 and an SN76489 with them on one
timeline, and returns the sound as interleaved stereo at the rate the caller
chooses. Nothing is drawn, and nothing reads input.

It also plays the YM2413 (OPLL) of the Master System and the MSX: nine FM
channels, the 15 built-in instruments, the user tone and the bass drum of the
rhythm mode.

It covers the YM2612 (six FM channels of four operators, the envelope generator
with SSG-EG, the LFO, the timers, CSM mode and the DAC on channel 6) and the
SN76489 (three tones, noise, the Game Gear stereo write).

## Layout

- `rust/src`: `ym2612.rs` and `eg.rs` (the FM chip and its envelope generator),
  `ym2413.rs` (the OPLL),
  `psg.rs` (the SN76489), `machine.rs` (the public type, the scheduler and the
  output).
- `rust/tests`: the Rust tests.
- `tests/`: Swift Testing suites for the C interface.
- `DESIGN.md`: how the core works.
- `tools/oscilla-vgm`: the command-line player described below.

The VGM reader is the `oscilla-vgm` crate in `formats/vgm`. The GYM reader is
the `oscilla-gym` crate in `formats/gym`. The resampler is
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

`cgenesis_image_load_gym` and `cgenesis_image_from_gym` read a GYM file into the
same kind of image. The file does not name its region, so the call takes a `pal`
flag that picks 50 Hz frames and the PAL clocks.

A player has three more groups of calls.

- `cgenesis_snapshot_take` copies the whole machine. `cgenesis_snapshot_restore`
  puts a machine with the same file and output rate back to that point, and the
  same calls then render the same samples.
- `cgenesis_set_channel_mask` chooses which of the six FM channels and four PSG
  channels reach the mix. A file with a second YM2612 or SN76489 adds bits for
  its channels.
- `cgenesis_channel_levels_get` reads what each of the ten channels of the first
  chips puts out now, for meters.
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
the FM channels and bits 6 to 9 the PSG's, then bits 10 to 15 the second
YM2612's and bits 16 to 19 the second SN76489's. `--info` prints the tags and the
times. `--bench` renders without output and prints the time it took.

## Tests

`cargo test -p oscilla-genesis --release` runs these groups.

- unit tests of the YM2612, the envelope generator and the SN76489.
- `machine`: pitch, loops, the end of a stream, the DAC, the masks and the
  volume modifier.
- `goldens`: a hash of the samples for each of 32 generated files. A change to
  the sound changes a hash.
- `ym2413`: the user tone's pitch, the built-in instruments, the second chip,
  the mask, the bass drum and random register writes.
- `gym`: a GYM log renders the same samples as the VGM file with the same
  commands, a PAL log runs at 50 frames a second, and a loop comes round.
- `fuzz`: files of random bytes run without a panic and render the same samples
  twice.

`cargo test -p oscilla-vgm --release` and `cargo test -p oscilla-gym --release`
run the readers' tests.

## Compatibility

The levels and the modulation depth were fitted to renders from libgme of 12
songs and 12 synthetic files. DESIGN.md lists the fits, the approximations and
the unknowns.

Not emulated: other sound chips that a VGM file can name, the snare drum,
hi-hat, tom-tom and top cymbal of the YM2413's rhythm mode, and the analog
stages after the chips. The output has a high-pass and no low-pass.

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
- Yamaha's YM2413 application manual and datasheet, and the Silicon Prawn page
  on the Yamaha OPL2, which lists the YM2413's built-in patches as measured on
  the chip.
- Two public descriptions of the GYM format.
