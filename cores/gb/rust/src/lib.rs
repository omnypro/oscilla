//! A music-only Game Boy: the CPU, memory, timer and sound unit that run a
//! GBS file's sound driver, with no picture. It plays a song by calling the
//! driver's init and play routines the way the file asks, and it hands back
//! the sound as interleaved stereo at a rate the caller chooses.
//!
//! This crate holds no unsafe code. The C interface lives in `oscilla-ffi`.

#![forbid(unsafe_code)]

mod apu;
mod bus;
mod cpu;
mod machine;
mod state;

pub use apu::{CONSOLE_HIGHPASS_HZ, MAX_HIGHPASS_HZ, MIN_HIGHPASS_HZ};
pub use machine::{EndState, RestoreError, Snapshot, Voice, VoiceKind};
pub use state::{CLOCK_HZ, Machine, Observer, SCOPE_LEN};
