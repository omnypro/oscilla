#![forbid(unsafe_code)]
//! A music-only PC Engine: the HuC6280 and its sound generator, driven by the
//! code of an HES file.

pub mod cpu;
mod machine;
pub mod psg;

pub use machine::{Machine, MachineError, Observer, UNIT};
pub use psg::{PsgVoice, SCOPE_LEN};
