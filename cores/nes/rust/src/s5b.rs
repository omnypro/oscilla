//! The Sunsoft 5B expansion chip: three tone generators, a noise generator
//! and an envelope in the registers of a YM2149-type chip. Written from public
//! documentation.

use crate::state::*;

/// The share of a 2A03 pulse channel's full-volume level that one 5B channel
/// has at its top level.
pub(crate) const S5B_GAIN: f64 = 3.0;

/// The step of the chip's volume curve for one level of the five-bit scale, as
/// a ratio: 1.5 dB, 10^(-0.075).
const STEP: f64 = 0.841_395_141_645_195;

/// The amplitude of each of the 32 levels, 1.0 at the top. The two lowest
/// levels are silent. Built by repeated multiplication.
pub(crate) fn amplitudes() -> [f64; 32] {
    let mut table = [0.0; 32];
    table[31] = 1.0;
    for level in (2..31).rev() {
        table[level] = table[level + 1] * STEP;
    }
    table
}

impl S5b {
    fn tone_period(&self, c: usize) -> u64 {
        (u64::from(self.regs[2 * c]) | u64::from(self.regs[2 * c + 1] & 15) << 8).max(1)
    }

    fn noise_period(&self) -> u64 {
        u64::from(self.regs[6] & 31).max(1)
    }

    fn env_period(&self) -> u64 {
        (u64::from(self.regs[0x0B]) | u64::from(self.regs[0x0C]) << 8).max(1)
    }

    /// A tone generator flips every 16 cycles of its period.
    pub(crate) fn tone_event(&mut self, c: usize, te: u64) {
        let p = self.tone_period(c);
        let t = &mut self.tone[c];
        t.out = !t.out;
        t.last = te;
        t.next_step = te + 16 * p;
    }

    /// The noise generator shifts every 32 cycles of its period.
    pub(crate) fn noise_event(&mut self, te: u64) {
        let feedback = (self.lfsr ^ (self.lfsr >> 3)) & 1;
        self.lfsr = self.lfsr >> 1 | feedback << 16;
        self.noise_out = self.lfsr & 1 != 0;
        self.noise_last = te;
        self.noise_next = te + 32 * self.noise_period();
    }

    /// The envelope steps every 16 cycles of its period through a ramp of 32
    /// levels.
    pub(crate) fn env_event(&mut self, te: u64) {
        self.env_step += 1;
        if self.env_step < 32 {
            self.env_level = if self.env_up {
                self.env_step
            } else {
                31 - self.env_step
            };
        } else {
            let shape = self.regs[0x0D];
            let (cont, alt, hold) = (shape & 8 != 0, shape & 2 != 0, shape & 1 != 0);
            if !cont {
                self.env_level = 0;
                self.env_holding = true;
            } else if hold {
                let end = if self.env_up { 31 } else { 0 };
                self.env_level = if alt { 31 - end } else { end };
                self.env_holding = true;
            } else {
                if alt {
                    self.env_up = !self.env_up;
                }
                self.env_step = 0;
                self.env_level = if self.env_up { 0 } else { 31 };
            }
        }
        self.env_last = te;
        self.env_next = if self.env_holding {
            NEVER
        } else {
            te + 16 * self.env_period()
        };
    }

    /// The five-bit level of tone channel `c`.
    pub(crate) fn level(&self, c: usize) -> u8 {
        if !self.active {
            return 0;
        }
        let mixer = self.regs[7];
        let tone = self.tone[c].out || mixer >> c & 1 != 0;
        let noise = self.noise_out || mixer >> (c + 3) & 1 != 0;
        if !(tone && noise) {
            return 0;
        }
        let volume = self.regs[8 + c];
        if volume & 16 != 0 {
            self.env_level
        } else if volume & 15 == 0 {
            0
        } else {
            (volume & 15) * 2 + 1
        }
    }
}

impl Machine {
    /// A store to the 5B's two ports: 0xC000..=0xDFFF selects a register and
    /// 0xE000..=0xFFFF writes it.
    pub(crate) fn s5b_write(&mut self, address: u16, v: u8) {
        self.sync_apu();
        let t = self.now;
        let x = &mut self.apu.s5b;
        if address < 0xE000 {
            x.select = v & 15;
            x.locked = v & 0xF0 != 0;
        } else if !x.locked {
            let reg = usize::from(x.select);
            x.regs[reg] = v;
            match reg {
                0..=5 => {
                    let c = reg / 2;
                    let at = x.tone[c].last + 16 * x.tone_period(c);
                    x.tone[c].next_step = at.max(t);
                }
                6 => x.noise_next = (x.noise_last + 32 * x.noise_period()).max(t),
                0x0B | 0x0C => {
                    if !x.env_holding {
                        x.env_next = (x.env_last + 16 * x.env_period()).max(t);
                    }
                }
                0x0D => {
                    x.env_up = v & 4 != 0;
                    x.env_step = 0;
                    x.env_holding = false;
                    x.env_level = if x.env_up { 0 } else { 31 };
                    x.env_last = t;
                    x.env_next = t + 16 * x.env_period();
                }
                _ => {}
            }
        }
        self.apu_emit(t);
    }
}
