//! A music-only Sega Genesis: the YM2612 and the SN76489, and the YM2413 of
//! the Master System and the MSX, driven by the register writes of a VGM file.

mod eg;
mod machine;
mod psg;
mod scope;
mod stream;
mod ym2413;
mod ym2612;

pub use eg::Phase;
pub use machine::{Chip, EndState, Machine, MachineError, Observer, UNIT, Voice, VoiceKind};
pub use psg::{Psg, PsgVoice};
pub use scope::SCOPE_LEN;
pub use ym2413::{OpllVoice, Ym2413};
pub use ym2612::{FmVoice, Ym2612};
