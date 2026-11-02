# Changes

## 0.3.1

- The VRC6 expansion chip: two pulse channels and a saw, played for files that
  ask for it. The mask has bits 5 to 7 for them, and
  `cnes_expansion_levels_get` shows their levels.
- A triangle step that fell on the same cycle as a frame counter event is no
  longer lost.
- The other expansion chips are not played yet.

## 0.3.0

- NES core in Rust: a 6502 CPU, the 2A03 sound unit with the console's
  filters, and NSF loading for both consoles.
- A C interface, `cnes.h`, and a `CNes` product in the Swift package. The
  XCFramework carries all three cores, and the static library has cargo
  features `gba`, `gb` and `nes`, all on by default.
- Snapshots, channel levels with a channel mask, end-of-song signals, an
  observer, warnings and a bass cutoff, as in the Game Boy core.
- `oscilla-nsf`: renders an NSF file to a WAV file and writes the stream of
  sound register stores.
- The resampler takes a clock given as a ratio, which the NES needs. The Game
  Boy and GBA output is unchanged.
- `CNES_OUTPUT_REVISION` (1) in `cnes.h`: bumped whenever the same input
  renders different samples or traces.
- Expansion sound chips are not played yet.

## 0.2.0

- Game Boy core in Rust: an SM83 CPU, timer, sound unit and GBS loading. It
  plays a game's music by running the game's own sound driver.
- A C interface, `cgb.h`, and a `CGb` product in the Swift package. The
  XCFramework carries both cores. The static library has cargo features `gba`
  and `gb`, and both are on by default.
- Snapshots, channel levels with a channel mask, end-of-song signals, an
  observer and warnings, as in the GBA core.
- A bass cutoff: `cgb_set_highpass_hz` moves the high-pass filter between 1 and
  1000 Hz during playback, and `cgb_use_console_highpass` restores the
  console's own filter, which is the default.
- `oscilla-gbs`: renders a GBS file to a WAV file and writes the stream of
  sound register stores.
- The band-limited resampler is its own crate, shared by both cores.
- `CGB_OUTPUT_REVISION` (1) in `cgb.h`: bumped whenever the same input renders
  different samples or traces.
- The GBA output is unchanged.

## 0.1.1

- Release assets include static libraries for Windows (`x86_64-pc-windows-msvc`)
  and Linux (`x86_64-unknown-linux-gnu`). 0.1.0 shipped without them.
- The source is the same as 0.1.0.

## 0.1.0

- GBA core in Rust: ARM and Thumb CPU, bus, timers, DMA, interrupts, sound
  hardware, a high-level BIOS and GSF loading.
- A C interface, `cgba.h`, and a Swift package. SwiftPM links the Rust library
  as an XCFramework that `scripts/build-xcframework.sh` builds.
- Snapshots, channel levels with a channel mask, and end-of-song signals.
- No math library calls. The output does not depend on the platform's C
  library.
- `oscilla-gsf`: renders a GSF or minigsf to a WAV file and writes the
  register and FIFO traces.
- `CGBA_OUTPUT_REVISION` (1) in `cgba.h`: bumped whenever the same input
  renders different samples or traces.
