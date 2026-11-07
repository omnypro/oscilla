//! The FM patch of a voice and the register blocks of a chip, for a display.

/// The FM chip that a patch belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FmChip {
    Ym2612 = 0,
    Ym2413 = 1,
}

/// One operator of an FM voice. A field that the chip has no counterpart for
/// is 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FmOperator {
    /// The multiple's register code, 0 to 15. The YM2612 reads 0 as one half.
    /// The YM2413 reads the codes 0 to 15 as 1/2, 1, 2, 3, 4, 5, 6, 7, 8, 9,
    /// 10, 10, 12, 12, 15, 15.
    pub multiple: u8,
    /// The YM2612's DT1 register code, 0 to 7: bits 0 and 1 are the size and
    /// bit 2 makes it negative.
    pub detune: u8,
    /// The YM2612's total level, 0 to 127, in steps of 0.75 dB. The YM2413
    /// modulator's total level, 0 to 63, in steps of 0.75 dB. The YM2413
    /// carrier has none, and this field holds the channel's volume register
    /// times 4 (0 to 60), in the same step.
    pub total_level: u8,
    /// The rate scaling. The YM2612's RS code, 0 to 3. The YM2413's KSR bit,
    /// 0 or 1.
    pub key_scale: u8,
    /// The attack rate. The YM2612's 0 to 31, the YM2413's 0 to 15.
    pub attack_rate: u8,
    /// The first decay rate. The YM2612's 0 to 31, the YM2413's 0 to 15.
    pub decay_rate: u8,
    /// The YM2612's second decay rate, 0 to 31. The YM2413 has none.
    pub sustain_rate: u8,
    /// The release rate, 0 to 15.
    pub release_rate: u8,
    /// The level at which the decay ends, 0 to 15.
    pub sustain_level: u8,
    /// The YM2612's SSG-EG: 0 while it is off, otherwise the register's low
    /// four bits with bit 3 (enable) set.
    pub ssg_eg: u8,
    /// Amplitude modulation by the low frequency oscillator is on.
    pub am: bool,
    /// The YM2413's vibrato is on.
    pub vibrato: bool,
    /// The YM2413's EG type: the operator holds its sustain level until the
    /// key goes off.
    pub sustained: bool,
    /// The YM2413's wave: 0 sine, 1 half sine.
    pub waveform: u8,
    /// 0 attack, 1 decay, 2 sustain, 3 release, 4 off.
    pub phase: u8,
    /// The envelope's attenuation with 0 the loudest, before the total level
    /// and the low frequency oscillator, after the SSG-EG inversion. The
    /// YM2612's range is 0 to 1023 in steps of 0.09375 dB. The YM2413's is 0
    /// to 127 in steps of 0.375 dB.
    pub envelope_level: u16,
    /// The operator's own frequency in Hz: the channel's pitch with the
    /// multiple (and on the YM2612 the detune) applied, without the vibrato
    /// of the low frequency oscillator.
    pub frequency_hz: f32,
}

/// The patch that an FM voice uses now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FmPatch {
    pub chip: FmChip,
    /// The YM2612's algorithm, 0 to 7. 0 for the YM2413.
    pub algorithm: u8,
    /// The feedback of the first operator (the modulator on the YM2413), 0 to
    /// 7.
    pub feedback: u8,
    /// The YM2413's instrument: 0 is the user tone and 1 to 15 are the
    /// built-in instruments. 0 for the YM2612.
    pub instrument: u8,
    /// 4 for the YM2612 and 2 for the YM2413.
    pub operators: u8,
    /// The YM2612's operators 1 to 4 (the registers' slots S1, S3, S2, S4
    /// are operators 1, 3, 2, 4). The YM2413's modulator, then its carrier.
    pub op: [FmOperator; 4],
}

/// One block of registers of one chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterBlock {
    /// The chip that the block belongs to.
    pub chip: crate::Chip,
    /// "YM2612", "YM2612 (second)", "SN76489", "SN76489 (second)", "YM2413" or
    /// "YM2413 (second)".
    pub name: &'static str,
    /// The address of byte 0 in the chip's own register space.
    pub base: u32,
    /// The number of bytes.
    pub size: usize,
}

/// The size of the YM2612's block: the registers of its two ports.
pub const YM2612_BLOCK: usize = 0x200;
/// The size of the SN76489's block.
pub const SN76489_BLOCK: usize = 12;
/// The size of the YM2413's block.
pub const YM2413_BLOCK: usize = 0x40;
