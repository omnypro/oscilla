//! The YM2413 (OPLL): nine FM channels of two operators each, with the 15
//! built-in instruments, one user tone and a rhythm mode. Written from public
//! documents: the manufacturer's application manual and datasheet (registers,
//! envelope times, level scaling, step sizes, the low frequency oscillators,
//! the slots of the rhythm sounds), a decapsulation report of the related
//! YM3812 (the formulas of its log-sine and exponential tables), a hardware
//! note on the related VRC7 (the depths of the low frequency oscillators)
//! and the community wiki's table of the built-in patches, which was
//! measured on the chip and checked against a die shot of a related part.
//! The documents leave some behavior open. DESIGN.md lists what the core had
//! to choose.

/// The chip makes one sample for every 72 cycles of its clock.
pub(crate) const CYCLES_PER_SAMPLE: u64 = 72;

/// The envelope generator's range: 127 steps of 0.375 dB, past which the
/// channel is silent.
const MAX_ATT: u32 = 127 << 16;

/// Log-sine and exponential units: 1/256 of an octave, so 16 units are one
/// envelope step (0.375 dB).
const STEP_UNITS: u32 = 16;

/// `round(-log2(sin((x + 0.5) * pi / 512)) * 256)` for the 256 samples of a
/// quarter of a sine wave.
const LOGSIN: [u16; 256] = [
    2137, 1731, 1543, 1419, 1326, 1252, 1190, 1137, 1091, 1050, 1013, 979, 949, 920, 894, 869, 846,
    825, 804, 785, 767, 749, 732, 717, 701, 687, 672, 659, 646, 633, 621, 609, 598, 587, 576, 566,
    556, 546, 536, 527, 518, 509, 501, 492, 484, 476, 468, 461, 453, 446, 439, 432, 425, 418, 411,
    405, 399, 392, 386, 380, 375, 369, 363, 358, 352, 347, 341, 336, 331, 326, 321, 316, 311, 307,
    302, 297, 293, 289, 284, 280, 276, 271, 267, 263, 259, 255, 251, 248, 244, 240, 236, 233, 229,
    226, 222, 219, 215, 212, 209, 205, 202, 199, 196, 193, 190, 187, 184, 181, 178, 175, 172, 169,
    167, 164, 161, 159, 156, 153, 151, 148, 146, 143, 141, 138, 136, 134, 131, 129, 127, 125, 122,
    120, 118, 116, 114, 112, 110, 108, 106, 104, 102, 100, 98, 96, 94, 92, 91, 89, 87, 85, 83, 82,
    80, 78, 77, 75, 74, 72, 70, 69, 67, 66, 64, 63, 62, 60, 59, 57, 56, 55, 53, 52, 51, 49, 48, 47,
    46, 45, 43, 42, 41, 40, 39, 38, 37, 36, 35, 34, 33, 32, 31, 30, 29, 28, 27, 26, 25, 24, 23, 23,
    22, 21, 20, 20, 19, 18, 17, 17, 16, 15, 15, 14, 13, 13, 12, 12, 11, 10, 10, 9, 9, 8, 8, 7, 7,
    7, 6, 6, 5, 5, 5, 4, 4, 4, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// `round((2^(x / 256) - 1) * 1024)` for x from 0 to 255.
const EXP: [u16; 256] = [
    0, 3, 6, 8, 11, 14, 17, 20, 22, 25, 28, 31, 34, 37, 40, 42, 45, 48, 51, 54, 57, 60, 63, 66, 69,
    72, 75, 78, 81, 84, 87, 90, 93, 96, 99, 102, 105, 108, 111, 114, 117, 120, 123, 126, 130, 133,
    136, 139, 142, 145, 148, 152, 155, 158, 161, 164, 168, 171, 174, 177, 181, 184, 187, 190, 194,
    197, 200, 204, 207, 210, 214, 217, 220, 224, 227, 231, 234, 237, 241, 244, 248, 251, 255, 258,
    262, 265, 268, 272, 276, 279, 283, 286, 290, 293, 297, 300, 304, 308, 311, 315, 318, 322, 326,
    329, 333, 337, 340, 344, 348, 352, 355, 359, 363, 367, 370, 374, 378, 382, 385, 389, 393, 397,
    401, 405, 409, 412, 416, 420, 424, 428, 432, 436, 440, 444, 448, 452, 456, 460, 464, 468, 472,
    476, 480, 484, 488, 492, 496, 501, 505, 509, 513, 517, 521, 526, 530, 534, 538, 542, 547, 551,
    555, 560, 564, 568, 572, 577, 581, 585, 590, 594, 599, 603, 607, 612, 616, 621, 625, 630, 634,
    639, 643, 648, 652, 657, 661, 666, 670, 675, 680, 684, 689, 693, 698, 703, 708, 712, 717, 722,
    726, 731, 736, 741, 745, 750, 755, 760, 765, 770, 774, 779, 784, 789, 794, 799, 804, 809, 814,
    819, 824, 829, 834, 839, 844, 849, 854, 859, 864, 869, 874, 880, 885, 890, 895, 900, 906, 911,
    916, 921, 927, 932, 937, 942, 948, 953, 959, 964, 969, 975, 980, 986, 991, 996, 1002, 1007,
    1013, 1018,
];

/// For each rate, the share of the remaining attenuation that one sample of an
/// attack takes away, in units of 2^-32. The time that the attenuation takes
/// to fall from the whole range to under one step is the attack time of the
/// manual's table.
const ATTACK_K: [u32; 64] = [
    0, 0, 0, 0, 241816, 302268, 362719, 423170, 483619, 604515, 725408, 846298, 967184, 1208946,
    1450694, 1692429, 1934150, 2417551, 2900898, 3384191, 3867429, 4833742, 5799837, 6765715,
    7731375, 9662043, 11591842, 13520771, 15448832, 19302350, 23152398, 26998979, 30842096,
    38517952, 46179990, 53828236, 61462715, 76690468, 91863448, 106981851, 122045873, 152011560,
    181762063, 211298928, 240623689, 298642982, 355831995, 412202611, 470392517, 558098791,
    643793760, 709084433, 760475339, 889266802, 1070189700, 1191086108, 1262265225, 1536851826,
    1795378475, 2153558243, 0, 0, 0, 0,
];

/// The 15 built-in patches, instruments 1 to 15: eight bytes each, in the
/// layout of the user tone registers.
const PATCHES: [[u8; 8]; 15] = [
    [0x71, 0x61, 0x1E, 0x17, 0xD0, 0x78, 0x00, 0x17],
    [0x13, 0x41, 0x1A, 0x0D, 0xD8, 0xF7, 0x23, 0x13],
    [0x13, 0x01, 0x99, 0x00, 0xF2, 0xC4, 0x11, 0x23],
    [0x31, 0x61, 0x0E, 0x07, 0xA8, 0x64, 0x70, 0x27],
    [0x32, 0x21, 0x1E, 0x06, 0xE0, 0x76, 0x00, 0x28],
    [0x31, 0x22, 0x16, 0x05, 0xE0, 0x71, 0x00, 0x18],
    [0x21, 0x61, 0x1D, 0x07, 0x82, 0x81, 0x10, 0x07],
    [0x23, 0x21, 0x2D, 0x14, 0xA2, 0x72, 0x00, 0x07],
    [0x61, 0x61, 0x1B, 0x06, 0x64, 0x65, 0x10, 0x17],
    [0x41, 0x61, 0x0B, 0x18, 0x85, 0xF7, 0x71, 0x07],
    [0x13, 0x01, 0x83, 0x11, 0xFA, 0xE4, 0x10, 0x04],
    [0x17, 0xC1, 0x24, 0x07, 0xF8, 0xF8, 0x22, 0x12],
    [0x61, 0x50, 0x0C, 0x05, 0xC2, 0xF5, 0x20, 0x42],
    [0x01, 0x01, 0x55, 0x03, 0xC9, 0x95, 0x03, 0x02],
    [0x61, 0x41, 0x89, 0x03, 0xF1, 0xE4, 0x40, 0x13],
];
/// The rhythm patches: the bass drum, the snare drum with the hi-hat (the
/// hi-hat is the modulator) and the tom-tom with the top cymbal (the tom-tom
/// is the modulator).
const DRUMS: [[u8; 8]; 3] = [
    [0x01, 0x01, 0x18, 0x0F, 0xDF, 0xF8, 0x6A, 0x6D], // BD
    [0x01, 0x01, 0x00, 0x00, 0xC8, 0xD8, 0xA7, 0x48], // SD/HH
    [0x05, 0x01, 0x00, 0x00, 0xF8, 0xAA, 0x59, 0x55], // TM/TC
];

/// The multiplier codes, doubled so that the first is a whole number.
const MULTIPLE: [u32; 16] = [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 20, 24, 24, 30, 30];

/// The key level scale at the largest octave, for the top four bits of the
/// frequency, in steps of 0.375 dB (3 dB per octave). Each lower octave takes
/// 8 steps off.
const KEY_LEVEL: [u32; 16] = [
    0, 24, 32, 37, 40, 43, 45, 47, 48, 50, 51, 52, 53, 54, 55, 56,
];

/// The vibrato's offset in 1/4096 of the pitch at each of its eight steps: a
/// triangle with a peak of 14 cents.
const VIBRATO: [i32; 8] = [0, 17, 33, 17, 0, -17, -33, -17];

/// Steps of the tremolo's triangle in a cycle: 49716 / 64 / 3.7 Hz.
const TREMOLO_STEPS: u32 = 210;

/// The tremolo's peak in log units: 4.3 dB.
const TREMOLO_PEAK: u32 = 183;

/// Samples between steps of the tremolo and of the vibrato.
const TREMOLO_PERIOD: u8 = 64;
const VIBRATO_PERIOD: u16 = 1024;

/// How fast the envelope falls when a note starts on a sounding channel: the
/// whole range in about 10 ms (497 samples).
const DAMP_INC: u32 = MAX_ATT / 497;

/// The envelope's stages.
const OFF: u8 = 0;
const DAMP: u8 = 1;
const ATTACK: u8 = 2;
const DECAY: u8 = 3;
const SUSTAIN: u8 = 4;
const RELEASE: u8 = 5;

/// For each rate, the steps of the envelope that one sample of a decay takes
/// away, in units of 1/65536 step. The manual's table gives the time that the
/// envelope's whole range takes to fall: 163.49 ms (8128 samples, which is 127
/// steps of 64 samples) at rate 32, half as long for each 4 rates above, and
/// shorter by 4 / (4 + rate mod 4) within a group of four. The fastest group
/// takes no account of the position within it.
fn decay_inc(rate: usize) -> u32 {
    if rate == 0 {
        return 0;
    }
    let group = rate / 4;
    let within = if group >= 15 { 0 } else { rate % 4 };
    (4 + within as u32) << group
}

/// The rate of an envelope parameter after the key scale. A parameter of 0
/// stops the stage.
fn rate_of(parameter: u32, ks: u32) -> usize {
    if parameter == 0 {
        0
    } else {
        (4 * parameter + ks).min(63) as usize
    }
}

/// The value of an operator at a place in its wave and an attenuation, in the
/// way of the decap report's formula: the exponential of the sum of the
/// log-sine and the attenuation.
fn operator(place: i32, half_wave: bool, units: u32) -> i32 {
    let p = (place & 1023) as u32;
    let negative = p & 512 != 0;
    if half_wave && negative {
        return 0;
    }
    let mut x = p & 255;
    if p & 256 != 0 {
        x = 255 - x;
    }
    let level = u32::from(LOGSIN[x as usize]) + units;
    let shift = level >> 8;
    if shift >= 12 {
        return 0;
    }
    let magnitude = ((i32::from(EXP[(255 - (level & 255)) as usize]) + 1024) << 1) >> shift;
    if negative { -magnitude } else { magnitude }
}

/// The key level scale of a channel in log units.
fn key_level(code: u32, block: u32, fnum: u32) -> u32 {
    let base = KEY_LEVEL[(fnum >> 5) as usize];
    let steps = base.saturating_sub(8 * (7 - block));
    let steps = match code {
        1 => steps,
        2 => steps / 2,
        3 => steps * 2,
        _ => 0,
    };
    steps * STEP_UNITS
}

use crate::scope::Scope;

/// What a channel of the YM2413 is doing, from its registers and its envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpllVoice {
    pub key_on: bool,
    pub fnum: u16,
    pub block: u8,
    /// The instrument register's high nibble: 0 is the user tone, 1 to 15 the
    /// built-in instruments. The bass drum of the rhythm mode reads 16.
    pub instrument: u8,
    /// The volume register's low nibble, in steps of 3 dB, 0 loudest.
    pub volume: u8,
    /// The carrier's envelope attenuation in steps of 0.375 dB, 0 to 127.
    pub envelope: u8,
    /// 0 attack, 1 decay, 2 sustain, 3 release.
    pub phase: u8,
    /// Channel 7 playing the bass drum of the rhythm mode.
    pub drum: bool,
    /// The channel's sustain bit.
    pub sustain: bool,
}

/// What one operator keeps between samples.
#[derive(Clone, Copy, Default)]
struct Op {
    /// Attenuation in 1/65536 of an envelope step.
    att: u32,
    state: u8,
    phase: u32,
    /// The last two outputs, for the modulator's feedback.
    prev: [i32; 2],
}

/// The nine channels of the chip. In rhythm mode channels 7 to 9 make the
/// five percussion sounds.
#[derive(Clone)]
pub struct Ym2413 {
    custom: [u8; 8],
    frequency_low: [u8; 9],
    frequency_high: [u8; 9],
    instrument: [u8; 9],
    /// The key bit of each channel's register 0x20 to 0x28.
    key_register: [bool; 9],
    /// Whether each channel's operators are keyed on now.
    key: [bool; 9],
    ops: [[Op; 2]; 9],
    /// Register 0x0E: bit 5 is the rhythm mode and bits 0 to 4 the keys of
    /// the five percussion sounds.
    rhythm: u8,
    test: u8,
    scope: Scope,
    tremolo_step: u8,
    tremolo_tick: u8,
    vibrato_step: u8,
    vibrato_tick: u16,
    mask: u16,
}

impl Default for Ym2413 {
    fn default() -> Ym2413 {
        Ym2413::new()
    }
}

impl Ym2413 {
    /// The chip with its registers cleared.
    pub fn new() -> Ym2413 {
        let mut chip = Ym2413 {
            custom: [0; 8],
            frequency_low: [0; 9],
            frequency_high: [0; 9],
            instrument: [0; 9],
            key_register: [false; 9],
            key: [false; 9],
            ops: [[Op::default(); 2]; 9],
            rhythm: 0,
            test: 0,
            scope: Scope::new(9),
            tremolo_step: 0,
            tremolo_tick: 0,
            vibrato_step: 0,
            vibrato_tick: 0,
            mask: 0x1FF,
        };
        chip.clear_ops();
        chip
    }

    fn clear_ops(&mut self) {
        for ch in &mut self.ops {
            for op in ch.iter_mut() {
                *op = Op {
                    att: MAX_ATT,
                    state: OFF,
                    ..Op::default()
                };
            }
        }
    }

    /// Chooses the channels that reach the output: bit 0 is channel 1. In
    /// rhythm mode bit 6 is the bass drum.
    pub fn set_mask(&mut self, mask: u16) {
        self.mask = mask & 0x1FF;
    }

    pub fn mask(&self) -> u16 {
        self.mask
    }

    /// What channel `ch` (0 to 8) is doing.
    pub fn voice(&self, ch: usize) -> OpllVoice {
        let ch = ch % 9;
        let drum = self.rhythm_mode() && ch == 6;
        let fnum = u16::from(self.frequency_low[ch]) | u16::from(self.frequency_high[ch] & 1) << 8;
        OpllVoice {
            key_on: self.key[ch],
            fnum,
            block: self.frequency_high[ch] >> 1 & 7,
            instrument: if drum { 16 } else { self.instrument[ch] >> 4 },
            volume: self.instrument[ch] & 15,
            envelope: (self.ops[ch][1].att >> 16).min(127) as u8,
            phase: match self.ops[ch][1].state {
                ATTACK | DAMP => 0,
                DECAY => 1,
                SUSTAIN => 2,
                _ => 3,
            },
            drum,
            sustain: self.frequency_high[ch] & 0x20 != 0,
        }
    }

    /// The sample `back` steps before the newest of a channel's scope: the
    /// channel's output times 8.
    pub fn scope_sample(&self, channel: usize, back: usize) -> i16 {
        self.scope.get(channel, back)
    }

    fn rhythm_mode(&self) -> bool {
        self.rhythm & 0x20 != 0
    }

    /// The patch a channel uses.
    fn patch(&self, ch: usize) -> [u8; 8] {
        if self.rhythm_mode() && ch == 6 {
            return DRUMS[0];
        }
        match self.instrument[ch] >> 4 {
            0 => self.custom,
            i => PATCHES[usize::from(i) - 1],
        }
    }

    /// Whether a channel's key is on: in rhythm mode the bass drum has its own
    /// key bit and the two channels above it have none.
    fn wants_key(&self, ch: usize) -> bool {
        if self.rhythm_mode() && ch >= 6 {
            ch == 6 && self.rhythm & 0x10 != 0
        } else {
            self.key_register[ch]
        }
    }

    fn refresh_key(&mut self, ch: usize) {
        let on = self.wants_key(ch);
        if on && !self.key[ch] {
            self.key_on(ch);
        } else if !on && self.key[ch] {
            self.key_off(ch);
        }
        self.key[ch] = on;
    }

    /// A store to a register. The chip needs its address and its data written
    /// one after the other, which the command stream has already done.
    pub fn write(&mut self, register: u8, v: u8) {
        let r = usize::from(register);
        match r {
            0..=7 => self.custom[r] = v,
            0x0E => {
                self.rhythm = v & 0x3F;
                for ch in 6..9 {
                    self.refresh_key(ch);
                }
            }
            0x0F => self.test = v & 15,
            0x10..=0x18 => self.frequency_low[r - 0x10] = v,
            0x20..=0x28 => {
                let ch = r - 0x20;
                self.frequency_high[ch] = v & 0x3F;
                self.key_register[ch] = v & 0x10 != 0;
                self.refresh_key(ch);
            }
            0x30..=0x38 => self.instrument[r - 0x30] = v,
            _ => {}
        }
    }

    fn key_on(&mut self, ch: usize) {
        for op in &mut self.ops[ch] {
            if op.att >= MAX_ATT {
                op.phase = 0;
                op.state = ATTACK;
            } else {
                op.state = DAMP;
            }
        }
    }

    fn key_off(&mut self, ch: usize) {
        let patch = self.patch(ch);
        // A modulator with its sustain bit set keeps sounding.
        if patch[0] & 0x20 == 0 && self.ops[ch][0].state != OFF {
            self.ops[ch][0].state = RELEASE;
        }
        if self.ops[ch][1].state != OFF {
            self.ops[ch][1].state = RELEASE;
        }
    }

    /// One step of an operator's envelope.
    fn envelope(&mut self, ch: usize, which: usize, patch: [u8; 8], key_scale: u32) {
        let (attack_decay, sustain_release) = (patch[4 + which], patch[6 + which]);
        let sustained = patch[which] & 0x20 != 0;
        let ks = if patch[which] & 0x10 != 0 {
            key_scale
        } else {
            key_scale >> 2
        };
        let released_key = !self.key[ch];
        let channel_sustain = self.frequency_high[ch] & 0x20 != 0;
        let op = &mut self.ops[ch][which];
        match op.state {
            DAMP => {
                op.att = (op.att + DAMP_INC).min(MAX_ATT);
                if op.att >= MAX_ATT {
                    op.phase = 0;
                    op.state = ATTACK;
                }
            }
            ATTACK => {
                let rate = rate_of(u32::from(attack_decay >> 4), ks);
                if rate >= 60 {
                    op.att = 0;
                } else if rate > 0 {
                    let take = ((u64::from(op.att) * u64::from(ATTACK_K[rate])) >> 32) as u32;
                    op.att -= take.max(1).min(op.att);
                }
                if op.att < 1 << 16 {
                    op.att = 0;
                    op.state = DECAY;
                }
            }
            DECAY => {
                let rate = rate_of(u32::from(attack_decay & 15), ks);
                op.att += decay_inc(rate);
                let level = (u32::from(sustain_release >> 4) * 8) << 16;
                if op.att >= level {
                    op.att = level;
                    op.state = if sustained { SUSTAIN } else { RELEASE };
                }
            }
            RELEASE => {
                // After the key goes off a channel with its sustain bit releases
                // at rate 5, and a percussive operator at rate 7. A sustained
                // operator and an operator below its sustain level with the key
                // still on use the patch's release rate.
                let r = if released_key && channel_sustain {
                    5
                } else if released_key && !sustained {
                    7
                } else {
                    u32::from(sustain_release & 15)
                };
                op.att += decay_inc(rate_of(r, ks));
                if op.att >= MAX_ATT {
                    op.att = MAX_ATT;
                    op.state = OFF;
                }
            }
            _ => {}
        }
    }

    /// One sample of every channel, mixed: the nine melody channels (or six
    /// and the bass drum) in units of about 4088 for one channel at full
    /// level.
    pub fn sample(&mut self) -> i32 {
        let fast = self.test & 8 != 0;
        self.tremolo_tick += 1;
        if fast || self.tremolo_tick >= TREMOLO_PERIOD {
            self.tremolo_tick = 0;
            self.tremolo_step = (self.tremolo_step + 1) % TREMOLO_STEPS as u8;
        }
        self.vibrato_tick += 1;
        if fast || self.vibrato_tick >= VIBRATO_PERIOD {
            self.vibrato_tick = 0;
            self.vibrato_step = (self.vibrato_step + 1) & 7;
        }
        if self.test & 2 != 0 {
            self.tremolo_step = 0;
            self.tremolo_tick = 0;
            self.vibrato_step = 0;
            self.vibrato_tick = 0;
        }
        let step = u32::from(self.tremolo_step);
        let triangle = if step <= TREMOLO_STEPS / 2 {
            step
        } else {
            TREMOLO_STEPS - step
        };
        let tremolo = triangle * TREMOLO_PEAK / (TREMOLO_STEPS / 2);
        let vibrato = VIBRATO[usize::from(self.vibrato_step)];
        let rhythm = self.rhythm_mode();
        let mut sum = 0;
        let mut heard = [0i16; 9];
        for ch in 0..9 {
            if rhythm && ch > 6 {
                continue;
            }
            let out = self.channel(ch, tremolo, vibrato);
            heard[ch] = (out * 8).clamp(-32768, 32767) as i16;
            if self.mask >> ch & 1 != 0 {
                // The manual has the chip put the percussion sounds out twice,
                // because they seem quieter than the melody.
                sum += if rhythm && ch == 6 { out * 2 } else { out };
            }
        }
        self.scope.push(&heard);
        sum
    }

    fn channel(&mut self, ch: usize, tremolo: u32, vibrato: i32) -> i32 {
        let patch = self.patch(ch);
        let fnum = u32::from(self.frequency_low[ch]) | u32::from(self.frequency_high[ch] & 1) << 8;
        let block = u32::from(self.frequency_high[ch] >> 1 & 7);
        let key_scale = block << 1 | fnum >> 8;
        for which in 0..2 {
            self.envelope(ch, which, patch, key_scale);
        }
        if self.ops[ch][1].state == OFF || self.ops[ch][1].att >= MAX_ATT {
            self.ops[ch][0].prev = [0; 2];
            return 0;
        }
        let volume = u32::from(self.instrument[ch] & 15);
        let eg_off = self.test & 1 != 0;
        let mut modulation = 0;
        let mut carrier = 0;
        for which in 0..2 {
            let flags = patch[which];
            let base = ((fnum << block) * MULTIPLE[usize::from(flags & 15)]) >> 1;
            let inc = if flags & 0x40 != 0 {
                (base as i32 + ((base as i32 * vibrato) >> 12)) as u32
            } else {
                base
            };
            let op = &mut self.ops[ch][which];
            if self.test & 4 != 0 {
                op.phase = 0;
            } else {
                op.phase = (op.phase + inc) & 0x7FFFF;
            }
            let att = if eg_off { 0 } else { op.att >> 12 };
            let mut units = att;
            if flags & 0x80 != 0 {
                units += tremolo;
            }
            let place;
            let half_wave;
            if which == 0 {
                units += u32::from(patch[2] & 63) * 32;
                units += key_level(u32::from(patch[2] >> 6), block, fnum);
                let fb = i32::from(patch[3] & 7);
                let feedback = if fb == 0 {
                    0
                } else {
                    (((op.prev[0] + op.prev[1]) >> 1) >> 1) >> (7 - fb)
                };
                place = (op.phase >> 9) as i32 + feedback;
                half_wave = patch[3] & 8 != 0;
            } else {
                units += key_level(u32::from(patch[3] >> 6), block, fnum);
                units += volume * 128;
                place = (op.phase >> 9) as i32 + (modulation >> 1);
                half_wave = patch[3] & 16 != 0;
            }
            let v = operator(place, half_wave, units);
            if which == 0 {
                op.prev = [op.prev[1], v];
                modulation = v;
            } else {
                carrier = v;
            }
        }
        carrier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_are_the_documented_formulas() {
        for x in 0..256usize {
            let sine = ((x as f64 + 0.5) * std::f64::consts::PI / 512.0).sin();
            let want = -sine.log2() * 256.0;
            assert!(
                (f64::from(LOGSIN[x]) - want).abs() <= 0.5 + 1e-9,
                "logsin {x}"
            );
            let want = ((x as f64 / 256.0).exp2() - 1.0) * 1024.0;
            assert!((f64::from(EXP[x]) - want).abs() <= 0.5 + 1e-9, "exp {x}");
        }
    }

    #[test]
    fn the_decay_times_follow_the_manuals_table() {
        // Rate 32: the whole range (127 steps) in 163.49 ms, which is 8128
        // samples.
        let samples = 127.0 * 65536.0 / f64::from(decay_inc(32));
        assert_eq!(samples, 8128.0);
        // Four rates up halve the time, and a step within a group shortens it.
        assert!((f64::from(decay_inc(36)) / f64::from(decay_inc(32)) - 2.0).abs() < 0.001);
        assert!((f64::from(decay_inc(33)) / f64::from(decay_inc(32)) - 1.25).abs() < 0.001);
        assert_eq!(decay_inc(0), 0);
        // The fastest group ignores the step: 1.27 ms.
        assert_eq!(decay_inc(60), decay_inc(63));
        let ms = 127.0 * 65536.0 / f64::from(decay_inc(60)) / 49.716;
        assert!((ms - 1.27).abs() < 0.02, "{ms}");
    }

    #[test]
    fn the_attack_times_follow_the_manuals_table() {
        // From the whole range to under a step: rate 32 takes 13.52 ms.
        let k = f64::from(ATTACK_K[32]) / 4_294_967_296.0;
        let samples = (127.0f64).ln() / -(1.0 - k).ln();
        assert!((samples / 49.716 - 13.52).abs() < 0.2, "{samples}");
        assert!(ATTACK_K[4] > 0);
    }

    #[test]
    fn the_key_level_scale_matches_the_manuals_table() {
        let dbs = |block, fnum| key_level(1, block, fnum) as f64 / 16.0 * 0.375;
        // Octave 7 at 3 dB per octave, and octave 4 and 1.
        assert_eq!(dbs(7, 15 << 5), 21.0);
        assert_eq!(dbs(7, 1 << 5), 9.0);
        assert_eq!(dbs(4, 15 << 5), 12.0);
        assert_eq!(dbs(4, 9 << 5), 9.75);
        assert_eq!(dbs(1, 15 << 5), 3.0);
        assert_eq!(dbs(1, 5 << 5), 0.0);
        assert_eq!(dbs(3, 0), 0.0);
        // The other settings are half and double.
        assert_eq!(key_level(2, 7, 15 << 5), key_level(1, 7, 15 << 5) / 2);
        assert_eq!(key_level(3, 7, 15 << 5), key_level(1, 7, 15 << 5) * 2);
        assert_eq!(key_level(0, 7, 15 << 5), 0);
    }

    #[test]
    fn an_operator_at_full_level_peaks_near_four_thousand() {
        let peak = operator(256, false, 0);
        assert!((4000..=4100).contains(&peak), "{peak}");
        assert_eq!(operator(256 + 512, false, 0), -peak);
        assert_eq!(operator(256 + 512, true, 0), 0);
        // 6 dB, or 256 units, halves it.
        assert!((operator(256, false, 256) * 2 - peak).abs() <= 2);
        assert!(operator(0, false, 0).abs() < 100);
    }
}
