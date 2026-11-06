//! The Famicom Disk System's expansion chip: one wavetable channel of 64
//! six-bit samples, a volume envelope and a frequency modulation unit that
//! steps through a table. Written from public documentation.

use crate::state::*;

/// One step of the chip's output (a six-bit sample times a gain of one) in the
/// units of a 2A03 pulse channel at full volume. The documentation puts the
/// loudest the chip gets at 2.4 times the loudest pulse channel.
pub(crate) const FDS_GAIN: f64 = 2.4 / 2016.0;

/// The corner of the one-pole low-pass filter that follows the chip.
const LOWPASS_HZ: f64 = 2000.0;

/// What one entry of the modulation table does to the counter.
const MOD_STEP: [i32; 8] = [0, 1, 2, 4, 0, -4, -2, -1];

/// The master volume as a fraction: 1, 2/3, 2/4 and 2/5.
pub(crate) const MASTER: [(i32, i32); 4] = [(1, 1), (2, 3), (2, 4), (2, 5)];

impl Fds {
    /// The filter's share of the old output that remains after one update of
    /// 16 cycles, from the series for exp(-x). `clock` is the CPU clock.
    pub(crate) fn alpha_for(clock: (u64, u64)) -> f64 {
        let x = 2.0 * std::f64::consts::PI * LOWPASS_HZ * 16.0 / (clock.0 as f64 / clock.1 as f64);
        let x2 = x * x;
        let x3 = x2 * x;
        let x4 = x2 * x2;
        let x5 = x4 * x;
        1.0 - (1.0 - x + x2 / 2.0 - x3 / 6.0 + x4 / 24.0 - x5 / 120.0)
    }

    /// Advances the envelopes by `cycles` cycles.
    fn envelopes(&mut self, cycles: u32) {
        if self.halted || self.env_disabled || self.env_speed == 0 {
            return;
        }
        let m = u32::from(self.env_speed) + 1;
        if !self.vol_off {
            let period = 8 * (u32::from(self.vol_speed) + 1) * m;
            self.vol_timer += cycles;
            while self.vol_timer >= period {
                self.vol_timer -= period;
                if self.vol_up && self.vol_gain < 32 {
                    self.vol_gain += 1;
                } else if !self.vol_up && self.vol_gain > 0 {
                    self.vol_gain -= 1;
                }
            }
        }
        if !self.mod_off {
            let period = 8 * (u32::from(self.mod_speed) + 1) * m;
            self.mod_timer += cycles;
            while self.mod_timer >= period {
                self.mod_timer -= period;
                if self.mod_up && self.mod_gain < 32 {
                    self.mod_gain += 1;
                } else if !self.mod_up && self.mod_gain > 0 {
                    self.mod_gain -= 1;
                }
            }
        }
    }

    /// One update of the modulation and wave units, every 16 cycles.
    pub(crate) fn tick(&mut self, te: u64, alpha: f64) {
        self.envelopes(16);
        if !self.mod_halted {
            let carry = if self.mod_carry {
                true
            } else if self.mod_freq == 0 {
                false
            } else {
                let sum = self.mod_accum + u32::from(self.mod_freq);
                self.mod_accum = sum & 0xFFF;
                sum >> 12 != 0
            };
            if carry {
                let entry = usize::from(self.mod_table[usize::from(self.mod_pos >> 1 & 31)]);
                let counter = i32::from(self.mod_counter);
                let next = if entry == 4 {
                    0
                } else {
                    counter + MOD_STEP[entry & 7]
                };
                self.mod_counter = (((next & 0x7F) ^ 0x40) - 0x40) as i8;
                self.mod_pos = (self.mod_pos + 1) & 63;
            }
        }
        let gain = i32::from(self.mod_gain.min(32));
        let mut temp = i32::from(self.mod_counter) * gain;
        if temp & 0xF != 0 && temp & 0x800 == 0 {
            temp += 0x20;
        }
        temp += 0x400;
        temp = (temp >> 4) & 0xFF;
        let pitch = (u32::from(self.freq) * temp as u32) & 0xFFFFF;
        if !self.halted {
            self.accum = (self.accum + pitch) & 0xFF_FFFF;
        }
        let at = if self.halted {
            0
        } else {
            (self.accum >> 18) as usize
        };
        if at == 0 {
            self.vol_out = self.vol_gain;
        }
        let (num, den) = MASTER[usize::from(self.master & 3)];
        self.out = i32::from(self.wave[at]) * i32::from(self.vol_out.min(32)) * num / den;
        self.lowpassed += (f64::from(self.out) - self.lowpassed) * alpha;
        self.next_step = te + 16;
    }

    /// A store to one of the chip's registers.
    pub(crate) fn write(&mut self, address: u16, v: u8) {
        match address {
            0x4040..=0x407F => {
                if self.write_enable {
                    self.wave[usize::from(address - 0x4040)] = v & 0x3F;
                }
            }
            0x4080 => {
                self.vol_off = v & 0x80 != 0;
                self.vol_up = v & 0x40 != 0;
                self.vol_speed = v & 0x3F;
                if self.vol_off {
                    self.vol_gain = v & 0x3F;
                    if self.vol_gain == 0 {
                        self.vol_out = 0;
                    }
                }
                self.vol_timer = 0;
            }
            0x4082 => self.freq = self.freq & 0xF00 | u16::from(v),
            0x4083 => {
                self.freq = self.freq & 0xFF | u16::from(v & 0x0F) << 8;
                self.halted = v & 0x80 != 0;
                if self.halted {
                    self.accum = 0;
                }
                self.env_disabled = v & 0x40 != 0;
                if self.env_disabled {
                    self.vol_timer = 0;
                    self.mod_timer = 0;
                }
            }
            0x4084 => {
                self.mod_off = v & 0x80 != 0;
                self.mod_up = v & 0x40 != 0;
                self.mod_speed = v & 0x3F;
                if self.mod_off {
                    self.mod_gain = v & 0x3F;
                }
                self.mod_timer = 0;
            }
            0x4085 => self.mod_counter = (((i32::from(v) & 0x7F) ^ 0x40) - 0x40) as i8,
            0x4086 => self.mod_freq = self.mod_freq & 0xF00 | u16::from(v),
            0x4087 => {
                self.mod_freq = self.mod_freq & 0xFF | u16::from(v & 0x0F) << 8;
                self.mod_halted = v & 0x80 != 0;
                if self.mod_halted {
                    self.mod_accum = 0;
                }
                self.mod_carry = v & 0x40 != 0;
            }
            0x4088 => {
                if self.mod_halted {
                    self.mod_table[usize::from(self.mod_pos >> 1 & 31)] = v & 7;
                    self.mod_pos = (self.mod_pos + 2) & 63;
                }
            }
            0x4089 => {
                self.master = v & 3;
                self.write_enable = v & 0x80 != 0;
            }
            0x408A => self.env_speed = v,
            _ => {}
        }
    }

    /// A read of one of the chip's registers, with the top two bits left to the
    /// bus where the chip does not drive them. None for an address that the
    /// chip does not answer.
    pub(crate) fn read(&self, address: u16, bus: u8) -> Option<u8> {
        let open = bus & 0xC0;
        match address {
            0x4040..=0x407F => {
                let at = if self.write_enable {
                    usize::from(address - 0x4040)
                } else {
                    (self.accum >> 18) as usize
                };
                Some(self.wave[at] | open)
            }
            0x4090 => Some(self.vol_gain | open),
            0x4091 => Some((self.accum >> 12) as u8),
            0x4092 => Some(self.mod_gain | open),
            0x4097 => Some(self.mod_counter as u8 & 0x7F),
            _ => None,
        }
    }

    /// The chip's output for a meter: its level before the low-pass filter.
    pub(crate) fn level(&self) -> i32 {
        self.out
    }
}
