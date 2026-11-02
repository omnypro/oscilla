# Changes

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
