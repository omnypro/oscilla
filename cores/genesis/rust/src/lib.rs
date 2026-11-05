//! A music-only Sega Genesis: the YM2612 and the SN76489, driven by the
//! register writes of a VGM file.

mod machine;
mod psg;

pub use machine::{EndState, Machine, MachineError, UNIT};
pub use psg::Psg;
