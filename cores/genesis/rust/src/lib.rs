//! A music-only Sega Genesis: the YM2612 and the SN76489, and the YM2413 of
//! the Master System and the MSX, driven by the register writes of a VGM file.

mod eg;
mod machine;
mod psg;
mod stream;
mod ym2413;
mod ym2612;

pub use machine::{EndState, Machine, MachineError, UNIT};
pub use psg::Psg;
pub use ym2413::Ym2413;
pub use ym2612::Ym2612;
