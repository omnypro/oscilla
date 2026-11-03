# Changes

## 0.3.5

- NSFe files. The reader fills the same fields as for an NSF header, so the
  core plays the file as it plays the NSF. The image also gives the ripper, a
  comment, a name and an author for each song, the time and fade of each song
  in milliseconds, the length as the two added, and the playlist:
  `cnes_image_is_nsfe`, `cnes_image_ripper`, `cnes_image_comment`,
  `cnes_image_song_title`, `cnes_image_song_author`,
  `cnes_image_song_time_ms`, `cnes_image_song_fade_ms`,
  `cnes_image_song_length_ms`, `cnes_image_playlist_count` and
  `cnes_image_playlist_get`.
- `oscilla-nsf --info` prints the song data of an NSFe file.

## 0.3.4

- The Famicom Disk System expansion chip: the wavetable channel with its
  volume envelope and modulation unit, played for files that ask for it. The
  channel mask has bit 19 for it, and `cnes_expansion_levels_get` shows its
  level.
- `cnes_channel_mask` returns `0xFFFFF` at the start, with a bit for each
  channel of the 2A03 and of the four expansion chips.

## 0.3.3

- The Namco 163 expansion chip: eight wavetable channels, played for files that
  ask for it. The channel mask has bits 11 to 18 for them, and
  `cnes_expansion_levels_get` shows their levels.
- `cnes_channel_mask` returns `0x7FFFF` at the start, with a bit for each
  channel of the 2A03, the VRC6, the 5B and the Namco 163.

## 0.3.2

- The Sunsoft 5B expansion chip: three tone channels with noise and an
  envelope, played for files that ask for it. The channel mask has bits 8 to 10
  for them, and `cnes_expansion_levels_get` shows their levels.
- `cnes_channel_mask` returns `0x7FF` at the start, with a bit for each channel
  of the 2A03, the VRC6 and the 5B.

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
