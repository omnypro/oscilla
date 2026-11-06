//! The SN76489: three square-wave channels and a noise channel, each with a
//! 4-bit attenuation in steps of 2 dB. The chip divides its clock by 16. Its
//! output holds a level until a counter expires, so the model runs from one
//! expiry to the next and reports each change of level.

use crate::scope::Scope;

/// The peak-to-peak size of a channel at each attenuation. Each step is
/// 10^(-2/20) of the one before, and 15 is silence. The top is a quarter of
/// the largest single YM2612 channel, so that the four channels at full volume
/// together match it.
const SIZE: [i32; 16] = [
    16384, 13014, 10338, 8212, 6522, 5182, 4116, 3270, 2596, 2062, 1638, 1302, 1034, 822, 652, 0,
];

/// The noise counter reloads from these values for the first three rates. The
/// fourth rate follows tone channel 2.
const NOISE_PERIOD: [u16; 3] = [0x10, 0x20, 0x40];

/// Flags from the VGM header.
pub const FLAG_ZERO_IS_0X400: u8 = 1;
pub const FLAG_NEGATE: u8 = 2;
pub const FLAG_XNOR: u8 = 16;

/// The rate of a PSG's scope samples.
pub const SCOPE_HZ: u64 = 44_100;

/// What a channel of the SN76489 is doing, from its registers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsgVoice {
    /// The tone register (10 bits); for the noise channel, the rate bits.
    pub register: u16,
    /// A tone channel whose counter runs (a register of 0 or 1 holds it high).
    pub running: bool,
    pub noise: bool,
    /// White noise (the noise channel's feedback bit).
    pub white: bool,
    /// The 4-bit attenuation, 0 loudest and 15 off.
    pub attenuation: u8,
    pub left: bool,
    pub right: bool,
}

#[derive(Clone, Debug)]
pub struct Psg {
    scope: Scope,
    scope_acc: u64,
    /// The ticks of the chip's own clock in a second.
    tick_hz: u64,
    tone: [u16; 3],
    counter: [u16; 3],
    high: [bool; 3],
    noise: u8,
    noise_counter: u16,
    noise_flip: bool,
    noise_bit: bool,
    shift: u16,
    attenuation: [u8; 4],
    /// The register that a data byte changes: the channel and whether it is
    /// the volume.
    latch: (u8, bool),
    feedback: u16,
    width: u8,
    flags: u8,
    stereo: u8,
    mask: u8,
    last: (i32, i32),
}

impl Psg {
    /// `feedback` is the tap mask of white noise (0x0009 on the Sega chips),
    /// `width` the length of the shift register in bits (16), and `flags` the
    /// header's flag byte.
    pub fn new(feedback: u16, width: u8, flags: u8) -> Psg {
        let width = width.clamp(2, 16);
        let mut p = Psg {
            scope: Scope::new(4),
            scope_acc: 0,
            tick_hz: 223_721,
            tone: [0; 3],
            counter: [0; 3],
            high: [false; 3],
            noise: 0,
            noise_counter: 0,
            noise_flip: false,
            noise_bit: false,
            shift: 1 << (width - 1),
            attenuation: [15; 4],
            latch: (0, false),
            feedback,
            width,
            flags,
            stereo: 0xFF,
            mask: 0x0F,
            last: (0, 0),
        };
        p.last = p.level();
        p
    }

    /// Sets the ticks of the chip's own clock in a second, which the scope
    /// needs to sample at 44,100 Hz.
    pub fn set_tick_hz(&mut self, hz: u64) {
        self.tick_hz = hz.max(1);
    }

    /// What channel `c` (0 to 3, the last being noise) is doing.
    pub fn voice(&self, c: usize) -> PsgVoice {
        let c = c & 3;
        PsgVoice {
            register: if c == 3 {
                u16::from(self.noise & 3)
            } else {
                self.tone[c]
            },
            running: c == 3 || self.period(self.tone[c]).is_some(),
            noise: c == 3,
            white: c == 3 && self.noise & 4 != 0,
            attenuation: self.attenuation[c],
            left: self.stereo & (0x10 << c) != 0,
            right: self.stereo & (1 << c) != 0,
        }
    }

    /// The sample `back` steps before the newest of a channel's scope, which
    /// holds the channel as the listener hears it: its level, once for each
    /// side that it is on, averaged.
    pub fn scope_sample(&self, channel: usize, back: usize) -> i16 {
        self.scope.get(channel, back)
    }

    /// Chooses which of the four channels reach the mix. Bit n is channel n.
    pub fn set_mask(&mut self, mask: u8) {
        self.mask = mask & 0x0F;
        self.last = self.level();
    }

    pub fn mask(&self) -> u8 {
        self.mask
    }

    /// The Game Gear's stereo port: bits 0 to 3 send channels 0 to 3 to the
    /// right and bits 4 to 7 send them to the left.
    pub fn set_stereo(&mut self, value: u8) {
        self.stereo = value;
        self.last = self.level();
    }

    pub fn write(&mut self, value: u8) {
        if value & 0x80 != 0 {
            self.latch = ((value >> 5) & 3, value & 0x10 != 0);
            let data = value & 15;
            match self.latch {
                (c, true) => self.attenuation[usize::from(c)] = data,
                (3, false) => self.set_noise(data),
                (c, false) => {
                    let t = &mut self.tone[usize::from(c)];
                    *t = *t & 0x3F0 | u16::from(data);
                }
            }
        } else {
            match self.latch {
                (c, true) => self.attenuation[usize::from(c)] = value & 15,
                (3, false) => self.set_noise(value & 15),
                (c, false) => {
                    let t = &mut self.tone[usize::from(c)];
                    *t = *t & 15 | u16::from(value & 0x3F) << 4;
                }
            }
        }
        self.last = self.level();
    }

    /// A write to the noise register keeps its low three bits and clears the
    /// shift register to a single 1 in the top bit.
    fn set_noise(&mut self, data: u8) {
        self.noise = data & 7;
        self.shift = 1 << (self.width - 1);
    }

    /// The counter reload value of a tone register, or None when the counter
    /// does not run.
    fn period(&self, register: u16) -> Option<u16> {
        match register {
            0 if self.flags & FLAG_ZERO_IS_0X400 != 0 => Some(0x400),
            0 | 1 => None,
            r => Some(r),
        }
    }

    fn noise_reload(&self) -> Option<u16> {
        match self.noise & 3 {
            3 => self.period(self.tone[2]),
            r => Some(NOISE_PERIOD[usize::from(r)]),
        }
    }

    /// A tone channel that has a register value of 0 or 1 stays at its high
    /// level, which is how drivers play samples through the volume.
    fn tone_out(&self, c: usize) -> bool {
        self.period(self.tone[c]).is_none() || self.high[c]
    }

    /// The output of each channel at the moment, before the channel mask.
    pub fn channels(&self) -> [i32; 4] {
        let level = |bit: bool, a: u8| {
            let size = SIZE[usize::from(a)];
            let half = size / 2;
            let v = if bit { half } else { -half };
            if self.flags & FLAG_NEGATE != 0 { -v } else { v }
        };
        [
            level(self.tone_out(0), self.attenuation[0]),
            level(self.tone_out(1), self.attenuation[1]),
            level(self.tone_out(2), self.attenuation[2]),
            level(self.noise_bit, self.attenuation[3]),
        ]
    }

    /// The left and right output at the moment.
    pub fn level(&self) -> (i32, i32) {
        let ch = self.channels();
        let (mut l, mut r) = (0, 0);
        for (i, v) in ch.iter().enumerate() {
            if self.mask & (1 << i) == 0 {
                continue;
            }
            if self.stereo & (1 << i) != 0 {
                r += v;
            }
            if self.stereo & (0x10 << i) != 0 {
                l += v;
            }
        }
        (l, r)
    }

    fn shift_noise(&mut self) {
        let taps = u32::from(self.shift & self.feedback).count_ones() & 1 == 1;
        let bit = if self.noise & 4 != 0 {
            taps != (self.flags & FLAG_XNOR != 0)
        } else {
            self.shift & 1 != 0
        };
        self.noise_bit = self.shift & 1 != 0;
        self.shift = self.shift >> 1 | u16::from(bit) << (self.width - 1);
    }

    /// Runs `ticks` of the chip's own clock (the input clock divided by 16).
    /// `report(t, left, right)` is called for each change of the output with
    /// `t` ticks since the start of the call.
    pub fn run(&mut self, ticks: u64, mut report: impl FnMut(u64, i32, i32)) {
        let mut t = 0;
        while t < ticks {
            let mut step = ticks - t;
            let mut live = [false; 3];
            for (c, is_live) in live.iter_mut().enumerate() {
                if let Some(period) = self.period(self.tone[c]) {
                    *is_live = true;
                    // A counter that is not running loads its period first.
                    if self.counter[c] == 0 {
                        self.counter[c] = period;
                    }
                    step = step.min(u64::from(self.counter[c]));
                }
            }
            let noise_live = self.noise_reload().is_some();
            if let Some(period) = self.noise_reload() {
                if self.noise_counter == 0 {
                    self.noise_counter = period;
                }
                step = step.min(u64::from(self.noise_counter));
            }
            self.scope_acc += step * SCOPE_HZ;
            let n = self.scope_acc / self.tick_hz;
            self.scope_acc %= self.tick_hz;
            if n > 0 {
                let mut held = [0i16; 4];
                for (c, v) in self.channels().iter().enumerate() {
                    let sides = i32::from(self.stereo & (1 << c) != 0)
                        + i32::from(self.stereo & (0x10 << c) != 0);
                    held[c] = (v * sides / 2).clamp(-32768, 32767) as i16;
                }
                self.scope.push_n(&held, n);
            }
            t += step;
            for (c, _) in live.iter().enumerate().filter(|(_, l)| **l) {
                self.counter[c] = self.counter[c].saturating_sub(step as u16);
                if self.counter[c] == 0 {
                    self.counter[c] = self.period(self.tone[c]).unwrap_or(0);
                    self.high[c] = !self.high[c];
                }
            }
            if noise_live {
                self.noise_counter = self.noise_counter.saturating_sub(step as u16);
                if self.noise_counter == 0 {
                    self.noise_counter = self.noise_reload().unwrap_or(0);
                    self.noise_flip = !self.noise_flip;
                    if self.noise_flip {
                        self.shift_noise();
                    }
                }
            }
            let now = self.level();
            if now != self.last {
                self.last = now;
                report(t, now.0, now.1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn psg() -> Psg {
        Psg::new(0x0009, 16, 0)
    }

    fn changes(p: &mut Psg, ticks: u64) -> Vec<(u64, i32)> {
        let mut out = Vec::new();
        p.run(ticks, |t, l, _| out.push((t, l)));
        out
    }

    #[test]
    fn the_chip_starts_silent() {
        let mut p = psg();
        assert_eq!(p.level(), (0, 0));
        assert!(changes(&mut p, 5000).is_empty());
    }

    #[test]
    fn a_latch_byte_and_a_data_byte_make_a_ten_bit_period_at_once() {
        let mut p = psg();
        p.write(0b1000_1110);
        assert_eq!(p.tone[0], 0x00E);
        p.write(0b0000_1111);
        assert_eq!(p.tone[0], 0xFE);
        p.write(0b1000_0000);
        assert_eq!(p.tone[0], 0xF0);
    }

    #[test]
    fn a_data_byte_follows_the_latch_to_a_volume_and_to_the_noise_register() {
        let mut p = psg();
        p.write(0b1011_1111);
        assert_eq!(p.attenuation[1], 15);
        p.write(0b0000_0000);
        assert_eq!(p.attenuation[1], 0);
        p.write(0b1110_0101);
        assert_eq!(p.noise, 5);
        p.write(0b0000_0100);
        assert_eq!(p.noise, 4);
        // The latch stays on the noise register through data bytes.
        p.write(0b0000_0011);
        assert_eq!(p.noise, 3);
    }

    #[test]
    fn a_tone_flips_every_period_ticks() {
        let mut p = psg();
        p.write(0x90);
        p.write(0x84);
        p.write(0x00);
        let ev = changes(&mut p, 40);
        let times: Vec<u64> = ev.iter().map(|e| e.0).collect();
        assert_eq!(times, [4, 8, 12, 16, 20, 24, 28, 32, 36, 40]);
        assert_eq!(ev[0].1, SIZE[0] / 2);
        assert_eq!(ev[1].1, -SIZE[0] / 2);
    }

    #[test]
    fn a_period_of_zero_or_one_holds_the_high_level() {
        for reg in [0u8, 1] {
            let mut p = psg();
            p.write(0x90);
            p.write(0x80 | reg);
            p.write(0x00);
            assert_eq!(p.level().0, SIZE[0] / 2);
            assert!(changes(&mut p, 100).is_empty());
        }
    }

    #[test]
    fn the_flag_turns_a_period_of_zero_into_0x400() {
        let mut p = Psg::new(9, 16, FLAG_ZERO_IS_0X400);
        p.write(0x90);
        p.write(0x80);
        p.write(0x00);
        let times: Vec<u64> = changes(&mut p, 2100).iter().map(|e| e.0).collect();
        assert_eq!(times, [1024, 2048]);
    }

    #[test]
    fn each_attenuation_step_is_two_decibels() {
        for n in 0..14 {
            let ratio = f64::from(SIZE[n + 1]) / f64::from(SIZE[n]);
            assert!((ratio - 10f64.powf(-0.1)).abs() < 0.003, "{n}: {ratio}");
        }
        assert_eq!(SIZE[15], 0);
    }

    #[test]
    fn the_channels_add_and_the_mask_removes_them() {
        let mut p = psg();
        for c in 0..3u8 {
            p.write(0x90 | c << 5);
            p.write(0x80 | c << 5 | 1);
            p.write(0x00);
        }
        assert_eq!(p.level().0, 3 * (SIZE[0] / 2));
        p.set_mask(0b101);
        assert_eq!(p.level().0, 2 * (SIZE[0] / 2));
        p.set_mask(0);
        assert_eq!(p.level(), (0, 0));
    }

    #[test]
    fn the_stereo_port_sends_channels_to_each_side() {
        let mut p = psg();
        p.write(0x90);
        p.write(0x81);
        p.write(0x00);
        p.set_stereo(0x10);
        assert_eq!(p.level(), (SIZE[0] / 2, 0));
        p.set_stereo(0x01);
        assert_eq!(p.level(), (0, SIZE[0] / 2));
        p.set_stereo(0);
        assert_eq!(p.level(), (0, 0));
    }

    #[test]
    fn periodic_noise_repeats_a_single_bit_every_sixteen_shifts() {
        let mut p = psg();
        p.write(0xF0);
        // Noise rate 0 (counter 0x10), periodic mode; the write clears the register to 0x8000.
        p.write(0xE0);
        p.write(0xF0);
        let mut ones = 0;
        for _ in 0..64 {
            p.shift_noise();
            ones += u32::from(p.noise_bit);
        }
        assert_eq!(ones, 4);
        assert_eq!(p.shift, 0x8000);
    }

    #[test]
    fn white_noise_runs_longer_than_the_periodic_loop_and_a_write_restarts_it() {
        let mut p = psg();
        p.write(0xE4);
        let start = p.shift;
        let mut n = 0u32;
        loop {
            p.shift_noise();
            n += 1;
            if p.shift == start || n > 70_000 {
                break;
            }
        }
        assert!(n > 16 && n <= 65_535, "{n}");
        p.write(0xE4);
        assert_eq!(p.shift, 0x8000);
    }

    #[test]
    fn the_fourth_noise_rate_follows_tone_two() {
        let mut p = psg();
        p.write(0xF0);
        p.write(0xC0 | 3);
        p.write(0x00);
        p.write(0xE7);
        // Tone 2 period 3: the noise counter flips every 3 ticks and shifts on every second flip, starting with the first.
        let mut shifts = 0;
        let mut last = p.shift;
        let mut seen = Vec::new();
        for t in 1..=12u64 {
            p.run(1, |_, _, _| {});
            if p.shift != last {
                shifts += 1;
                seen.push(t);
                last = p.shift;
            }
        }
        assert_eq!(seen, [3, 9]);
        assert_eq!(shifts, 2);
    }

    #[test]
    fn the_negate_flag_inverts_the_output() {
        let mut p = Psg::new(9, 16, FLAG_NEGATE);
        p.write(0x90);
        p.write(0x80);
        p.write(0x00);
        assert_eq!(p.level().0, -SIZE[0] / 2);
    }
}
