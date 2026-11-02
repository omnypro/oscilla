# Oscilla

Sound-hardware cores that play game music by running the game's own sound
code. The cores are written in Rust and reached through a C interface and a
Swift package. A core has no display, input or file dependencies beyond what
the music format needs, and it returns stereo samples at the rate the caller
asks for.

## Cores

- `cores/gba`: Game Boy Advance. Plays GSF and minigsf sets and bare
  cartridge images.

## Layout

- `cores/<name>/rust`: one core, as a Rust crate with no unsafe code.
- `cores/<name>/tests`, `tools` and `DESIGN.md`: Swift tests of the C
  interface, command-line tools and design notes.
- `formats/psf`: the PSF container that cores share.
- `ffi`: the C interface. One static library exports every core.
- `swift/CGba`: the C header, `cgba.h`, and the module that Swift imports.
- `scripts`: the XCFramework build.

## Build and test

`rust-toolchain.toml` pins the Rust version and the target platforms. Rustup
installs them on first use.

1. Run `cargo test --workspace --release`. A debug build also works and
   checks every integer operation for overflow.
2. Run `scripts/build-xcframework.sh`. It builds the static library for
   macOS, iOS and the iOS Simulator and wraps the slices in
   `build/OscillaRust.xcframework`.
3. Run `swift build` and `swift test`.

SwiftPM cannot run cargo. Step 2 comes before step 3 and repeats after any
change to the Rust sources.

`cargo test` runs the Rust tests. `swift test` runs the tests of the C
interface. The GBA core also runs GBA test cartridges when `GBA_TESTS_DIR`
points at a folder of them (the layout is in `TestROMTests.swift`), and a
corpus test when `GSF_TEST_FILE` names a GSF file. Without those variables
the two tests skip.

## Platforms

The XCFramework holds slices for macOS (arm64 and x86_64), iOS (arm64) and
the iOS Simulator (arm64 and x86_64). The Rust objects are built for macOS 15
and iOS 18. `Package.swift` declares the same two versions, and the build
script stops when they differ.

SwiftPM accepts only artifact bundles as binary targets on Linux, so the
Swift package builds on Apple platforms only.

The crates compile for `x86_64-pc-windows-msvc`. No traces have been compared
on Windows. A program that links the static library there also links
`kernel32.lib`, `ntdll.lib`, `userenv.lib`, `ws2_32.lib` and `dbghelp.lib`,
and it uses the dynamic C runtime.

An executable links one Rust static library. Two of them define the same
runtime symbols, and the linker reports duplicates. A program that has Rust
code of its own builds a single static library from a crate that depends on
both.

## License

MIT. See `LICENSE`. The Rust library includes six small crates under their
own MIT terms. `THIRD-PARTY-NOTICES.md` holds their notices, and an app that
ships the library ships that file with it.

Console names describe the hardware that a core is compatible with. This
project is not affiliated with any console maker or game publisher.
