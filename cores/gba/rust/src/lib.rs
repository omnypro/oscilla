//! A music-only Game Boy Advance: the CPU, memory, timers, DMA, interrupts and
//! sound hardware, with a high-level BIOS and no picture. It runs a game's own
//! code so that the game's sound driver plays, and it hands back the sound as
//! interleaved stereo at a rate the caller chooses.
//!
//! This crate holds no unsafe code. The C interface lives in `oscilla-ffi`.

#![forbid(unsafe_code)]
// Each module follows the order of the hardware documentation it was written
// from. These lints ask for rewrites that would scatter one unit's logic.
#![allow(
    clippy::needless_range_loop,
    clippy::manual_range_patterns,
    clippy::needless_late_init
)]

mod apu;
mod bios;
mod bus;
mod cpu;
mod exact;
pub mod image;
mod machine;
mod resample;
mod state;
mod system;
mod tables;

pub use bios::reference_sincos14;
pub use machine::{ChannelLevels, EndState, RestoreError, Snapshot};
pub use resample::reference_kernel;
pub use state::{CLOCK_HZ, Machine, Observer};

/// The constant tables, for the tests that compare them with libm.
#[doc(hidden)]
pub fn sincos14_table(angle: u32) -> (i32, i32) {
    tables::SINCOS14[((angle >> 8) & 0xFF) as usize]
}

#[doc(hidden)]
pub fn kernel_table() -> &'static [i64] {
    &tables::KERNEL
}
