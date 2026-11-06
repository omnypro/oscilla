//! The envelope generator of one operator (slot): a 10-bit attenuation that
//! an attack, a decay, a sustain and a release move on the chip's shared
//! counter, with the SSG-EG modes on top.
//!
//! An attenuation of 0 is the loudest level and 0x3FF the quietest.

/// The attenuation that a slot must reach in SSG-EG mode before the shape
/// repeats, holds or toggles.
const SSG_LEVEL: u16 = 0x200;
const MAX_ATT: u16 = 0x3FF;

/// How much the attenuation grows (or, in the attack, how fast it falls) on
/// each update of a rate, for the eight positions of the counter's selector.
const INC: [[u8; 8]; 64] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 2, 1, 1, 1, 2],
    [1, 2, 1, 2, 1, 2, 1, 2],
    [1, 2, 2, 2, 1, 2, 2, 2],
    [2, 2, 2, 2, 2, 2, 2, 2],
    [2, 2, 2, 4, 2, 2, 2, 4],
    [2, 4, 2, 4, 2, 4, 2, 4],
    [2, 4, 4, 4, 2, 4, 4, 4],
    [4, 4, 4, 4, 4, 4, 4, 4],
    [4, 4, 4, 8, 4, 4, 4, 8],
    [4, 8, 4, 8, 4, 8, 4, 8],
    [4, 8, 8, 8, 4, 8, 8, 8],
    [8, 8, 8, 8, 8, 8, 8, 8],
    [8, 8, 8, 8, 8, 8, 8, 8],
    [8, 8, 8, 8, 8, 8, 8, 8],
    [8, 8, 8, 8, 8, 8, 8, 8],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Attack,
    Decay,
    Sustain,
    Release,
}

/// The parameters that registers $30 to $9F set for a slot, apart from the
/// ones of the phase generator.
#[derive(Clone, Copy, Debug, Default)]
pub struct EgParams {
    pub tl: u8,
    pub ks: u8,
    pub ar: u8,
    pub dr: u8,
    pub sr: u8,
    pub sl: u8,
    pub rr: u8,
    pub am: bool,
    /// The low four bits of the SSG-EG register: enable, attack, alternate,
    /// hold.
    pub ssg: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Eg {
    pub params: EgParams,
    att: u16,
    phase: Phase,
    /// Whether the SSG-EG output inversion has been toggled from the level
    /// that the attack bit sets.
    inverted: bool,
    held: bool,
    key: bool,
    phase_reset: bool,
}

impl Default for Eg {
    fn default() -> Self {
        Eg {
            params: EgParams::default(),
            att: MAX_ATT,
            phase: Phase::Release,
            inverted: false,
            held: false,
            key: false,
            phase_reset: false,
        }
    }
}

/// The effective rate for a register value: twice the value plus the key
/// scale, at most 63, and 0 when the value is 0.
fn rate(register: u8, ks: u8, kc: u8) -> u8 {
    if register == 0 {
        return 0;
    }
    (2 * u16::from(register) + u16::from(kc >> (3 - ks))).min(63) as u8
}

impl Eg {
    /// The envelope's attenuation, 0 (loudest) to 0x3FF, before the total
    /// level.
    pub fn attenuation(&self) -> u16 {
        self.att
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// True once after the slot starts a note or repeats an SSG-EG shape that
    /// restarts the phase generator.
    pub fn take_phase_reset(&mut self) -> bool {
        std::mem::take(&mut self.phase_reset)
    }

    fn ssg(&self) -> bool {
        self.params.ssg & 8 != 0
    }

    fn attack_rate(&self, kc: u8) -> u8 {
        rate(self.params.ar, self.params.ks, kc)
    }

    /// The output inversion that is in effect.
    fn inversion(&self) -> bool {
        self.ssg() && self.phase != Phase::Release && ((self.params.ssg & 4 != 0) ^ self.inverted)
    }

    /// Applies the key state of one sample. A change starts or ends a note.
    pub fn set_key(&mut self, on: bool, kc: u8) {
        if on == self.key {
            return;
        }
        self.key = on;
        if on {
            self.phase = Phase::Attack;
            self.inverted = false;
            self.held = false;
            self.phase_reset = true;
            if self.attack_rate(kc) >= 62 {
                self.att = 0;
            }
        } else {
            self.phase = Phase::Release;
        }
    }

    /// The SSG-EG work of one sample, before the envelope update.
    pub fn ssg_step(&mut self, kc: u8) {
        if !self.ssg() || self.att < SSG_LEVEL {
            return;
        }
        if self.phase == Phase::Release {
            self.att = MAX_ATT;
            return;
        }
        let alternate = self.params.ssg & 2 != 0;
        if self.params.ssg & 1 != 0 {
            if !self.held {
                if alternate {
                    self.inverted = !self.inverted;
                }
                self.held = true;
                self.att = if self.inversion() { SSG_LEVEL } else { MAX_ATT };
            }
            return;
        }
        if alternate {
            self.inverted = !self.inverted;
        } else {
            self.phase_reset = true;
        }
        self.phase = Phase::Attack;
        if self.attack_rate(kc) >= 62 {
            self.att = 0;
        }
    }

    /// One update of the envelope, run once every three samples with the
    /// shared counter already advanced.
    pub fn update(&mut self, counter: u32, kc: u8) {
        if self.held && self.phase != Phase::Release {
            return;
        }
        if self.phase == Phase::Attack && self.att == 0 {
            self.phase = Phase::Decay;
        }
        if self.phase == Phase::Decay && self.att >= self.sustain_level() {
            self.phase = Phase::Sustain;
        }
        let p = &self.params;
        let register = match self.phase {
            Phase::Attack => p.ar,
            Phase::Decay => p.dr,
            Phase::Sustain => p.sr,
            Phase::Release => 2 * p.rr + 1,
        };
        let r = rate(register, p.ks, kc);
        let shift = 11u32.saturating_sub(u32::from(r >> 2));
        if counter & ((1 << shift) - 1) != 0 {
            return;
        }
        let inc = i32::from(INC[usize::from(r)][((counter >> shift) & 7) as usize]);
        if self.phase == Phase::Attack {
            if r < 62 {
                let a = i32::from(self.att);
                self.att = (a + ((!a * inc) >> 4)).max(0) as u16;
            }
        } else {
            let step = if self.ssg() { 4 * inc } else { inc };
            self.att = (i32::from(self.att) + step).min(i32::from(MAX_ATT)) as u16;
        }
    }

    fn sustain_level(&self) -> u16 {
        if self.params.sl == 15 {
            MAX_ATT
        } else {
            u16::from(self.params.sl) << 5
        }
    }

    /// The attenuation that the operator unit uses: the envelope, inverted
    /// for SSG-EG, plus the total level and the amplitude modulation, at most
    /// 0x3FF.
    pub fn output(&self, am: u16) -> u16 {
        let mut a = self.att;
        if self.inversion() {
            a = (SSG_LEVEL.wrapping_sub(a)) & MAX_ATT;
        }
        let am = if self.params.am { am } else { 0 };
        (a + (u16::from(self.params.tl) << 3) + am).min(MAX_ATT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eg(ar: u8, dr: u8, sr: u8, sl: u8, rr: u8) -> Eg {
        Eg {
            params: EgParams {
                ar,
                dr,
                sr,
                sl,
                rr,
                ..EgParams::default()
            },
            ..Eg::default()
        }
    }

    /// Runs updates with the counter counting up from `from + 1`.
    fn run(e: &mut Eg, from: u32, updates: u32, kc: u8) -> u32 {
        let mut c = from;
        for _ in 0..updates {
            c += 1;
            e.update(c, kc);
        }
        c
    }

    #[test]
    fn the_increment_rows_follow_the_documented_sums() {
        let sums: [(usize, u32); 8] = [
            (2, 4),
            (6, 6),
            (11, 7),
            (48, 8),
            (51, 14),
            (53, 20),
            (59, 56),
            (63, 64),
        ];
        for (r, sum) in sums {
            assert_eq!(
                INC[r].iter().map(|v| u32::from(*v)).sum::<u32>(),
                sum,
                "{r}"
            );
        }
        assert!(INC[0].iter().all(|v| *v == 0) && INC[1].iter().all(|v| *v == 0));
    }

    #[test]
    fn the_effective_rate_is_twice_the_register_plus_the_key_scale() {
        assert_eq!(rate(0, 3, 31), 0);
        assert_eq!(rate(5, 0, 0), 10);
        assert_eq!(rate(5, 3, 20), 30);
        assert_eq!(rate(5, 1, 20), 15);
        assert_eq!(rate(31, 0, 0), 62);
        assert_eq!(rate(30, 0, 31), 63);
    }

    #[test]
    fn an_attack_at_rate_62_or_more_jumps_to_full_level_at_key_on() {
        let mut e = eg(31, 0, 0, 5, 0);
        e.set_key(true, 0);
        assert_eq!(e.attenuation(), 0);
        assert_eq!(e.phase(), Phase::Attack);
        e.update(1, 0);
        assert_eq!(e.phase(), Phase::Decay);
    }

    #[test]
    fn a_slower_attack_keeps_its_level_at_key_on_and_falls_to_zero() {
        let mut e = eg(20, 0, 0, 5, 15);
        e.set_key(true, 0);
        assert_eq!(e.attenuation(), MAX_ATT);
        run(&mut e, 0, 200_000, 0);
        assert_eq!(e.attenuation(), 0);
        assert_eq!(e.phase(), Phase::Decay);
    }

    #[test]
    fn the_attack_at_the_fastest_rate_below_62_takes_ten_updates() {
        let mut e = eg(30, 0, 0, 0, 0);
        e.set_key(true, 0);
        assert_eq!(e.attack_rate(0), 60);
        // Rate 60 moves on every update with an increment of 8.
        let c = run(&mut e, 0, 10, 0);
        assert_eq!(c, 10);
        assert_eq!(e.attenuation(), 0);
    }

    #[test]
    fn the_attack_formula_gives_the_documented_values() {
        let mut e = eg(30, 0, 0, 0, 0);
        e.set_key(true, 0);
        let mut seen = vec![e.attenuation()];
        for c in 1..=10 {
            e.update(c, 0);
            seen.push(e.attenuation());
        }
        assert_eq!(
            seen,
            [
                0x3FF, 0x1FF, 0xFF, 0x7F, 0x3F, 0x1F, 0xF, 0x7, 0x3, 0x1, 0x0
            ]
        );
    }

    #[test]
    fn the_decay_stops_at_the_sustain_level_and_the_switch_is_one_way() {
        let mut e = eg(31, 20, 0, 4, 15);
        e.set_key(true, 0);
        run(&mut e, 0, 100_000, 0);
        assert_eq!(e.phase(), Phase::Sustain);
        assert_eq!(e.attenuation(), 4 << 5);
        e.params.sl = 12;
        run(&mut e, 100_000, 10_000, 0);
        assert_eq!(e.phase(), Phase::Sustain);
    }

    #[test]
    fn a_sustain_level_of_zero_skips_the_decay() {
        let mut e = eg(31, 20, 10, 0, 15);
        e.set_key(true, 0);
        e.update(1, 0);
        e.update(2, 0);
        assert_eq!(e.phase(), Phase::Sustain);
        assert_eq!(e.attenuation(), 0);
    }

    #[test]
    fn the_sustain_level_of_fifteen_is_full_attenuation() {
        let mut e = eg(31, 63, 0, 15, 15);
        e.set_key(true, 0);
        run(&mut e, 0, 1000, 0);
        assert_eq!(e.phase(), Phase::Sustain);
        assert_eq!(e.attenuation(), MAX_ATT);
    }

    #[test]
    fn the_fastest_decay_takes_128_updates_and_the_slowest_rate_never_moves() {
        let mut e = eg(31, 31, 0, 15, 15);
        e.params.ks = 3;
        e.set_key(true, 31);
        // Rate 63: increment 8 on every update, so 0x3FF is reached in 128 steps.
        let mut n = 0;
        let mut c = 0;
        while e.attenuation() < MAX_ATT {
            c += 1;
            n += 1;
            e.update(c, 31);
        }
        assert_eq!(n, 128);
        let mut s = eg(31, 0, 0, 5, 15);
        s.set_key(true, 0);
        run(&mut s, 0, 5000, 0);
        assert_eq!(s.attenuation(), 0);
    }

    #[test]
    fn a_release_starts_at_the_level_where_the_key_goes_up() {
        let mut e = eg(20, 0, 0, 5, 7);
        e.set_key(true, 0);
        run(&mut e, 0, 40, 0);
        let level = e.attenuation();
        assert!(level < MAX_ATT && level > 0);
        e.set_key(false, 0);
        assert_eq!(e.phase(), Phase::Release);
        assert_eq!(e.attenuation(), level);
        run(&mut e, 40, 100_000, 0);
        assert_eq!(e.attenuation(), MAX_ATT);
    }

    #[test]
    fn a_repeated_key_on_changes_nothing() {
        let mut e = eg(31, 10, 0, 5, 15);
        e.set_key(true, 0);
        e.take_phase_reset();
        run(&mut e, 0, 50, 0);
        let a = e.attenuation();
        e.set_key(true, 0);
        assert_eq!(e.attenuation(), a);
        assert!(!e.take_phase_reset());
    }

    #[test]
    fn the_output_adds_the_total_level_and_the_amplitude_modulation_and_limits_it() {
        let mut e = eg(31, 0, 0, 0, 15);
        e.params.tl = 0x10;
        e.params.am = true;
        e.set_key(true, 0);
        assert_eq!(e.output(0), 0x80);
        assert_eq!(e.output(5), 0x85);
        e.params.am = false;
        assert_eq!(e.output(5), 0x80);
        e.params.tl = 0x7F;
        e.att = 0x300;
        assert_eq!(e.output(0), MAX_ATT);
    }

    fn ssg_eg(mode: u8) -> Eg {
        let mut e = eg(31, 31, 0, 15, 15);
        e.params.ssg = 8 | mode;
        e.params.ks = 3;
        e.set_key(true, 31);
        e
    }

    /// Runs one sample of SSG work and returns the output level.
    fn sample(e: &mut Eg, counter: &mut u32) -> u16 {
        e.ssg_step(31);
        *counter += 1;
        e.update(*counter, 31);
        e.output(0)
    }

    #[test]
    fn mode_0_falls_and_repeats() {
        let mut e = ssg_eg(0);
        let mut c = 0;
        let mut loops = 0;
        let mut last = e.output(0);
        for _ in 0..2000 {
            let o = sample(&mut e, &mut c);
            if o < last {
                loops += 1;
            }
            last = o;
        }
        assert!(loops >= 3, "{loops}");
    }

    #[test]
    fn mode_1_falls_once_and_holds_at_the_lowest_level() {
        let mut e = ssg_eg(1);
        let mut c = 0;
        for _ in 0..2000 {
            sample(&mut e, &mut c);
        }
        assert_eq!(e.output(0), MAX_ATT);
        assert_eq!(e.attenuation(), MAX_ATT);
    }

    #[test]
    fn mode_3_falls_then_holds_at_the_full_level() {
        let mut e = ssg_eg(3);
        let mut c = 0;
        for _ in 0..2000 {
            sample(&mut e, &mut c);
        }
        assert_eq!(e.output(0), 0);
    }

    #[test]
    fn mode_5_rises_once_and_holds_at_the_full_level() {
        let mut e = ssg_eg(5);
        let mut c = 0;
        let first = e.output(0);
        assert_eq!(first, 0x200);
        for _ in 0..2000 {
            sample(&mut e, &mut c);
        }
        assert_eq!(e.output(0), 0);
    }

    #[test]
    fn mode_7_rises_then_holds_at_the_lowest_level() {
        let mut e = ssg_eg(7);
        let mut c = 0;
        for _ in 0..2000 {
            sample(&mut e, &mut c);
        }
        assert_eq!(e.output(0), MAX_ATT);
    }

    #[test]
    fn the_inversion_is_centered_on_0x200() {
        let mut e = ssg_eg(4);
        for (att, out) in [
            (0x001, 0x1FF),
            (0x110, 0xF0),
            (0x250, 0x3B0),
            (0x300, 0x300),
        ] {
            e.att = att;
            assert_eq!(e.output(0), out, "{att:#x}");
        }
    }

    #[test]
    fn a_key_off_ends_the_inversion_and_forces_silence_once_the_release_reaches_0x200() {
        let mut e = ssg_eg(4);
        e.att = 0x100;
        e.set_key(false, 31);
        assert_eq!(e.output(0), 0x100);
        e.att = 0x200;
        e.ssg_step(31);
        assert_eq!(e.attenuation(), MAX_ATT);
    }

    #[test]
    fn the_ssg_increment_is_four_times_the_plain_one() {
        let mut a = eg(31, 40, 0, 15, 15);
        let mut b = a;
        b.params.ssg = 8;
        a.set_key(true, 0);
        b.set_key(true, 0);
        a.update(1, 0);
        b.update(1, 0);
        a.update(2, 0);
        b.update(2, 0);
        assert_eq!(u32::from(b.attenuation()), 4 * u32::from(a.attenuation()));
    }

    #[test]
    fn the_alternate_bit_without_hold_toggles_the_inversion_on_every_sample_above_the_level() {
        let mut e = ssg_eg(2);
        e.att = 0x250;
        let before = e.inversion();
        e.ssg_step(31);
        e.att = 0x250;
        assert_ne!(e.inversion(), before);
    }
}
