# Game Boy core

A music-only Game Boy in Rust. It plays GBS files: it runs a Game Boy game's
sound driver and returns the sound as interleaved stereo at the rate the
caller chooses. Nothing is drawn, and nothing reads input.

It covers the SM83 CPU, the memory map with the GBS bank register, the timer,
the sound unit and the GBS player that calls the driver's init and play
routines.

## Layout

- `rust/src`: `cpu.rs` (the instruction set), `bus.rs` (memory map, bank
  register, timer and interrupt registers), `apu.rs` (sound), `machine.rs`
  (the public type and the player's schedule), `state.rs`.
- `rust/tests`: the Rust tests.
- `tests/`: Swift Testing suites for the C interface.
- `DESIGN.md`: how the core works.
- `tools/oscilla-gbs`: the command-line player described below.

The GBS header reader is the `oscilla-gbs` crate in `formats/gbs`. The
resampler is the `oscilla-resample` crate in `audio/resample`. The C header is
`swift/CGb/include/cgb.h`, a path from the root of the repository.

## API

```c
char error[256];
cgb_image *image;
cgb_image_load_gbs("song.gbs", &image, error, sizeof error);
cgb_machine *m = cgb_new(image, 48000);
cgb_image_free(image);
cgb_select_song(m, 3);                     /* counted from 0 */
cgb_render_s16(m, buffer, frames);         /* interleaved stereo */
cgb_free(m);
```

A machine holds all of its state and renders the same samples on every run.
`cgb_set_observer` reports every store to the sound registers and every call
of the init and play routines. Each play call is the driver's own tick, so a
host can digest the stores per call. Rust code uses `oscilla_gb::Machine`
directly.

A player has six more groups of calls.

- `cgb_snapshot_take` copies the whole machine. `cgb_snapshot_restore` puts a
  machine with the same file and output rate back to that point, and the same
  calls then render the same samples.
- `cgb_set_channel_mask` chooses which of the four channels reach the mix.
  `cgb_channel_levels_get` shows what each channel outputs, muted or not.
- `cgb_voices_get` returns what each channel is doing (its pitch, volume, pan,
  envelope, and its duty, wave or noise width), and `cgb_scope_get` returns the
  last frames that one channel put out, for a display.
- `cgb_register_block_count`, `_name`, `_base`, `_size` and `_get` return the
  registers as the program stored them, in two blocks: the 23 sound registers
  NR10 to NR52 ("APU", base 0xFF10) and the 16 bytes of wave RAM ("Wave RAM",
  base 0xFF30).
- `cgb_set_highpass_hz` moves the bass cutoff, from 1 to 1000 Hz, while a song
  plays. The default is the console's own filter at about 28 Hz, and
  `cgb_use_console_highpass` returns to it. `cgb_highpass_hz` reads the cutoff
  in use. A reset, a song change and a snapshot restore keep the choice.
- `cgb_end_state_get` reports a unit with no channel on and a routine that
  never returned. `cgb_quiet_cycles` counts the cycles since the output level
  changed.

## oscilla-gbs

```
oscilla-gbs FILE.gbs [--song N] [--seconds S] [--rate HZ] [--out FILE.wav]
                     [--writes FILE.txt]
oscilla-gbs FILE.gbs --info
oscilla-gbs FILE.gbs [--song N] --bench [--seconds S] [--rate HZ]
```

The first form plays song N (default 0) for S seconds (default 30) and writes
a 16-bit stereo WAV file. `--writes` adds a text file with one line per store
to the sound registers: the song, the play call (-1 for the init routine), the
cycle since that call began, the address and the value. `--info` prints the
header. `--bench` renders without output and prints the time it took.

## Tests

`cargo test -p oscilla-gb --release` runs these groups.

- `sound`: one behavior of the sound unit at a time, with drivers built from
  bytes.
- `player`: the schedule of init and play calls, the timer, bank switching,
  routines that misbehave, snapshots and the end state.
- `goldens`: a hash of the samples and of the stores for each of 64 generated
  drivers. A change to the sound or to any trace changes a hash.
- `highpass`: the bass cutoff's range, default, decay time and behavior across
  a change, a reset and a restore.
- `viz`: the state of each channel, its scope, and no change to the sound.
- `regs`: the register blocks: their bytes after a program's stores, the
  sweep and a power-off, invalid arguments, and no change to the sound.
- `cartridges`: public test cartridges for the CPU, memory timing and sound
  unit. They are not in the repository. `OSCILLA_GB_TESTS` names a folder that
  holds them and a `manifest.txt` that lists each one with a cycle limit. The
  test skips without the variable.

## Compatibility

The core plays GBS version 1 files. A comparison with the register-store
streams of a reference player gave the same stores in the same order in every
play call for all but one of 99 songs. The output is checked against public
test cartridges.

Not emulated: the display (a routine that waits for VBlank through STAT or the
interrupt flag stalls, and one that reads LY works), input, the link port,
interrupts in a GBS (the player calls the routines itself), the color console's
double speed (TAC bit 7 only halves the tick), cartridge hardware other than
the ROM bank register, and the access rules for wave RAM while the wave channel
plays. The output keeps the bass of the console's headphone output, so it has
more bass than a player that cuts it. `cgb_set_highpass_hz` moves that corner.

## References

Written from public documentation.

- Pan Docs: https://gbdev.io/pandocs/
- The GBS file format.
