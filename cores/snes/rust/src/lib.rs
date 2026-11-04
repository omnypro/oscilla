//! A music-only SNES: the SPC700, the S-DSP and their 64 KiB of RAM, started
//! from the state that an SPC file holds. The machine plays a song by running
//! the sound driver as it stands, and it hands back the sound as interleaved
//! stereo at a rate the caller chooses.
//!
//! This crate holds no unsafe code. The C interface lives in `oscilla-ffi`.

#![forbid(unsafe_code)]

mod boot;
mod dsp;
mod gauss;
mod machine;
mod smp;

pub use machine::{
    ALL_CHANNELS, CLOCK, CYCLES_PER_SAMPLE, EndState, Machine, Observer, RestoreError, SAMPLE_RATE,
    Snapshot,
};
pub use oscilla_spc::{MAX_FILE, Spc, SpcError};
