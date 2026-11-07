# PC Engine core

A music-only PC Engine (TurboGrafx-16) in Rust. It plays HES files: it runs the
sound driver of a game on a HuC6280 with its memory map, its timer and
interrupts, and the six-channel sound generator, and returns the sound as
interleaved stereo at the rate the caller chooses. Nothing is drawn, and
nothing reads input. The video chip has a stand-in that raises the vertical
sync interrupt that drivers use as their frame tick.

## Layout

- `rust/src`: `cpu.rs` (the HuC6280), `psg.rs` (the sound generator),
  `machine.rs` (the memory map, the timer, the interrupts, the vertical sync
  stand-in, the scheduler and the output).
- `rust/tests`: the Rust tests.
- `tests/`: Swift Testing suites for the C interface.
- `DESIGN.md`: how the core works.
- `tools/oscilla-hes`: the command-line player described below.

The HES reader is the `oscilla-hes` crate in `formats/hes`. The resampler is the
`oscilla-resample` crate in `audio/resample`. The C header is
`swift/CPce/include/cpce.h`, a path from the root of the repository.

## API

```c
char error[256];
cpce_image *image;
cpce_image_load_hes("game.hes", &image, error, sizeof error);
cpce_machine *m = cpce_new(image, 44100, song);
cpce_image_free(image);
cpce_render_s16(m, buffer, frames);    /* interleaved stereo */
cpce_free(m);
```

An HES file has no track list and no length. The caller picks the song number
(the playlist file next to an HES file names them) and decides when to stop. A
machine holds all of its state and renders the same samples on every run.

- `cpce_set_channel_mask` chooses which of the six channels reach the mix, and
  `cpce_channel_levels_get` reads what each puts out now, for meters.
  `cpce_voices_get` returns what each channel is doing (its pitch, gains, pan,
  waveform number and mode), and `cpce_scope_get` returns the last frames that
  one channel put out, for a display.
- `cpce_register`, `cpce_next_pc`, `cpce_peek` and `cpce_cycles` read the
  processor and memory with no side effects. `cpce_register_block_*` returns
  the sound generator's registers and waveforms, the mapping registers and the
  timer as the program wrote them, in four named blocks.
- `cpce_set_observer` sets a callback that hears every store to the sound
  generator, with the selected channel, the register, the value and the time in
  samples of 44,100 Hz. It does not change the sound.
- `cpce_snapshot_take` and `cpce_snapshot_restore` copy and return the whole
  machine.
- `cpce_set_highpass_hz` sets the cutoff of the output high-pass (20 Hz by
  default, 0 for none).

## oscilla-hes

```
oscilla-hes FILE.hes [--song N] [--seconds S] [--rate HZ] [--out FILE.wav]
                     [--mask HEX] [--writes FILE.txt]
oscilla-hes FILE.hes --info
oscilla-hes FILE.hes --bench [--song N] [--seconds S] [--rate HZ]
```

The first form plays the song for S seconds (default 30) at 44,100 Hz and writes
a 16-bit stereo WAV file. `--writes` writes one line for each store to the
sound generator. `--info` prints the first song. `--bench` renders without
output and prints the time it took.

## Tests

`cargo test -p oscilla-pce --release` runs these groups.

- `cpu`: every instruction group, written from the manuals, and a table of all
  234 documented opcodes with their bytes and cycles.
- `psg`: the frequency, amplitude, noise and low frequency oscillator formulas
  of the sound generator manual.
- `machine`: the request routine and its song number, the timer, the vertical
  sync, a program that never returns, copies and the channel mask.
- `observer`: the channel, register, value and time of every store, and no
  change to the sound.
- `viz`: the state of a channel and its scope.
- `inspect`: the processor registers, the memory read, the register blocks, and
  no change to the sound.
- `fuzz`: programs of random bytes run without a panic and render the same
  samples twice.

`cargo test -p oscilla-hes --release` runs the reader's tests.

## Compatibility

The level was fitted to libgme's renders of 29 songs of four games (the songs
lie within 0.4 dB of each other, and the fit is the mean). DESIGN.md lists the
fit, the approximations and the unknowns.

Not emulated: the video chip beyond its vertical sync, the CD-ROM and its
audio, the SuperGrafx's second video chip, the console's analog stages, and
programs that need the video chip's other interrupts. The output has a
high-pass and no low-pass.

## References

Written from public documentation.

- The Hudson HuC6280 hardware manual and software manual, the HuC6280
  programmable sound generator manual, and the HuC6270 video display
  controller manual.
- The HES format specification, version 1.00.
