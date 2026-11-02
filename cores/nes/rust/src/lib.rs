//! A music-only NES: the 6502, memory and sound unit that run an NSF file's
//! sound driver, with no picture. It plays a song by calling the driver's
//! init and play routines the way the file asks, and it hands back the sound
//! as interleaved stereo at a rate the caller chooses.
//!
//! This crate holds no unsafe code. The C interface lives in `oscilla-ffi`.

#![forbid(unsafe_code)]

mod apu;
mod bus;
mod cpu;
mod machine;
mod state;

pub use apu::{CONSOLE_HIGHPASS_HZ, MAX_HIGHPASS_HZ, MIN_HIGHPASS_HZ};
pub use machine::{EndState, RestoreError, Snapshot};
pub use oscilla_nsf::Region;
pub use state::{Machine, NTSC_CLOCK, Observer, PAL_CLOCK};
