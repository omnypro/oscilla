# GBA core

A music-only Game Boy Advance in Rust. It runs a game's own code so that the
game's sound driver plays, and returns the sound as interleaved stereo at the
rate the caller chooses. Nothing is drawn: the display exists only as line
timing, status flags and interrupts, because boot code and sound drivers wait
on them.

It covers the ARM7TDMI, the bus, timers, DMA, interrupts, the sound hardware,
a high-level BIOS and the GSF loader.

## Layout

- `rust/src`: `cpu.rs` (ARM and Thumb), `bus.rs` (memory map), `system.rs`
  (clock, display timing, timers, DMA, interrupts, I/O), `apu.rs` (sound),
  `resample.rs` (band-limited rate conversion), `bios.rs`, `image.rs` (GSF
  loading), `machine.rs` (the public type), and `exact.rs` with `tables.rs`
  (arithmetic that does not depend on the platform's math library).
- `rust/tests`: the Rust tests.
- `tests/`: Swift Testing suites for the C interface.
- `DESIGN.md`: how the core works.
- `tools/bios.s`, `tools/bios-table.sh`: the ARM part of the BIOS and the
  script that prints the table `bios.rs` holds.
- `tools/oscilla-gsf`: the command-line renderer described below.

The C header is `swift/CGba/include/cgba.h` at the root of the repository.

## API

```c
char error[256];
cgba_image *image;
cgba_image_load_gsf("song.minigsf", &image, error, sizeof error);
cgba_machine *m = cgba_new(image, 48000);
cgba_image_free(image);
cgba_render_s16(m, buffer, frames);         /* interleaved stereo */
cgba_reset(m);
cgba_free(m);
```

A machine holds all of its state and renders the same samples on every run.
`cgba_set_observer` reports every store to the sound registers and every byte
the Direct Sound FIFOs play. Rust code uses `oscilla_gba::Machine` directly.

A player has four more groups of calls.

- `cgba_snapshot_take` copies the whole machine. `cgba_snapshot_restore`
  puts a machine with the same image and output rate back to that point, and
  the same calls then render the same samples. A seek is a restore followed by
  a short render.
- `cgba_set_channel_mask` chooses which of the six channels reach the mix.
  `cgba_channel_levels_get` shows what each channel outputs, muted or not.
  `cgba_voices_get` returns what each channel is doing (its pitch, volume,
  sides, envelope and duty or wave number), and `cgba_scope_get` returns the
  last frames that one channel put out, for a display.
- `cgba_register_block_count`, `_name`, `_base`, `_size` and `_get` return the
  registers as the program stored them, in five blocks: "Sound" (SOUND1CNT_L
  to SOUNDBIAS, 42 bytes at 0x04000060), "Wave RAM" (both banks, 32 bytes at
  0x04000090), "FIFO" (8 bytes at 0x040000A0), "Timers and DMA sound" (the four
  timers' reload and control values, 16 bytes at 0x04000100) and "DMA 1 and 2"
  (24 bytes at 0x040000BC).
- `cgba_end_state_get` reports a CPU that waits for an interrupt that no
  register enables, and a CPU that runs a branch to itself with interrupts
  off. `cgba_quiet_cycles` counts the cycles since the output level changed.

## oscilla-gsf

```
oscilla-gsf FILE.minigsf [--seconds N] [--out-dir DIR] [--cycles]
oscilla-gsf FILE.minigsf --bench [--seconds N] [--rate HZ]
oscilla-gsf --test-rom FILE.gba [--frames N]
```

The seconds default to 20 and the directory to the current one. The first
form writes `fifo-a.bin` and `fifo-b.bin` (one signed byte per sample each
FIFO plays), `sound-writes.txt` (one line per store to
0x04000060..0x040000A7 in program order: address, value, width in bytes) and
`mix.wav` (stereo 16-bit at 32768 Hz). The run ends at the first VBlank at or
after N seconds, where the reference ends, and nothing after that cycle is
written. `--cycles` adds `sound-writes-cycles.txt`. Each line there begins
with its cycle. `--bench` renders without tracing and prints the time it
took.

## Tests

`cargo test -p oscilla-gba --release` runs these groups.

- `goldens`: a hash of the CPU registers after every instruction, of memory,
  and of 16-bit samples for each of a few hundred fixed programs. The programs
  cover ARM and Thumb code, I/O, interrupts, DMA, sound, every BIOS call and
  every width of memory access. A change to the sound or to any trace changes
  a hash.
- `coverage`: checks that those programs do their work. They run, reach their
  ends, make sound and touch every kind of memory.
- `hooks` and `bios_edges`: snapshots, channel masks, end-of-song signals and
  BIOS arguments at the edge of the cost arithmetic.
- `viz`: the state of each channel and its scope, and no change to the sound.
- `regs`: the register blocks: their bytes after a program's stores, a
  sound-off store, byte and word stores, DMA feeds, invalid arguments, a
  restore, and no change to the sound.
- `libm_equivalence` and `tables`: the platform-independent arithmetic and the
  constant tables against the platform's math library. The check of every
  ArcTan2 argument pair takes two minutes and runs with `--ignored`.

## Compatibility

The core plays GSF and minigsf sets and bare cartridge images. Its sound
output is checked against the output of a reference emulator and against
public test cartridges.

Not emulated: SRAM, video output, input, the link port and cartridge hardware
other than ROM. The cartridge prefetch buffer is modelled by its effect on
timing.

## References

Written from public documentation.

- GBATEK, Martin Korth: https://problemkaputt.de/gbatek.htm
- ARM Architecture Reference Manual (ARMv4T):
  https://developer.arm.com/documentation/ddi0100/latest/
- The PSF and GSF container layout.
