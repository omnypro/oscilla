# Changes

## 0.5.1

- The Genesis core plays a second YM2612 and a second SN76489, and the DAC
  stream commands (0x90 to 0x95) for the YM2612.
- The channel mask has bits 10 to 15 for the second YM2612 and bits 16 to 19
  for the second SN76489. A file without a second chip keeps 0x3FF.

## 0.5.0

- The Genesis core, which plays VGM and VGZ files. It runs the YM2612 (six
  FM channels, the DAC, the LFO and the timers) and the SN76489 from the
  file's register writes. `cgenesis.h` is its C interface.
- The VGM reader covers the GD3 tags, the loop point, the loop modifier and
  the volume modifier. The core ignores a second chip, the other chips a file
  can name and the DAC stream commands.
- The channel mask has ten bits: the six FM channels and the four PSG
  channels. `cgenesis_channel_mask` returns `0x3FF` at the start.
- The output has a 20 Hz high-pass filter. `cgenesis_set_highpass_hz`
  changes it, and 0 turns it off.
- `oscilla-vgm` plays a VGM file to a WAV file.

## 0.4.1

- `csnes_ram_copy` copies the 64 KB of sound RAM into a buffer.
- `csnes_watch_ram` and a `ram_write` callback in `csnes_observer` report the
  SPC700's stores to a range of RAM. Stores by `PUSH` and `CALL` are included.
- `csnes_observer` has a fourth field, `ram_write`. Swift code that builds it
  with the memberwise initializer needs `ram_write: nil`.

## 0.4.0

- The SNES core, which plays SPC files. It restores the RAM and the DSP
  registers from the file and runs the SPC700 and the S-DSP: eight BRR voices
  with envelopes, noise, pitch modulation and the echo filter, the three
  timers and the four ports. `csnes.h` is its C interface.
- The SPC reader covers the ID666 tags (text and binary) and the xid6 chunk,
  with the time, the fade, the muted voices and the amplification.
- The channel mask has nine bits: the eight voices and the echo output.
  `csnes_channel_mask` returns `0x1FF` at the start.
- The 64 bytes under bit 7 of the control register are a boot ROM written for
  this core. A block it takes holds at most 256 bytes.
- `oscilla-spc` plays an SPC file to a WAV file and writes the DSP stores.

## 0.3.8

- The VT02 expansion chip: a second generator with two pulse channels, a
  triangle and a noise channel at `0x4020`, the output switches at `0x4030`,
  raw 8-bit PCM from `0x4031` or the delta channel's DMA, and the enables at
  `0x4035`. The channel mask has bits 29 to 33 for them, and
  `cnes_expansion_levels_get` shows their levels.
- `cnes_set_channel_mask` and `cnes_channel_mask` use 64 bits.
  `cnes_channel_mask` returns `0x3FFFFFFFF` at the start.
- A file that asks for the VT02 no longer raises the warning about expansion
  sound.

## 0.3.7

- The MMC5 expansion chip: two pulse channels, the 8-bit sample register, the
  8 by 8 multiplier and the RAM at `0x5C00`, played for files that ask for it.
  The channel mask has bits 20 and 21 for the pulse channels and bit 22 for the
  sample register.
- The VRC7 expansion chip: six FM channels with the chip's 15 patches and the
  custom patch, played for files that ask for it. The channel mask has bits 23
  to 28 for them, and `cnes_expansion_levels_get` shows their last outputs.
- `cnes_channel_mask` returns `0x1FFFFFFF` at the start.
- A file that asks for the MMC5 or the VRC7 no longer raises the warning about
  expansion sound.

## 0.3.6

- The NSF2 features. A version 2 file can carry NSFe chunks after its program,
  and the same calls read them. A file that sets bit 4 or bit 5 of its feature
  byte gets the interrupt timer at `0x401B` to `0x401D`, the IRQ vector as RAM
  at `0xFFFE`, interrupts from the timer, the frame counter and the delta
  channel, and the play routine as an NMI. Bit 5 also gives the second init
  routine that need not return, and bit 6 stops the calls of the play routine.
- The warning about interrupts and non-returning init is gone.
- `oscilla-nsf --info` prints the song data of any file that has it.

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
