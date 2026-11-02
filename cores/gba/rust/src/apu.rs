//! The sound unit: the four tone and noise channels inherited from the Game
//! Boy, the two Direct Sound FIFOs, and the mixer that adds them to the bias
//! and clips the sum to 10 bits.
//!
//! The PSG channels are digital and unipolar: a tone or noise channel at
//! volume v outputs v or 0, and the wave channel outputs its 4-bit sample
//! after the volume shift. A FIFO spans +/-0x200. A channel that stops keeps
//! outputting its last level until it plays again.

use crate::state::*;

/// The Game Boy's duty waveforms, step by step.
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 0],
];

fn tone_period(c: &PsgChannel) -> i64 {
    i64::from(2048 - c.freq) * 16
}
fn wave_period(c: &PsgChannel) -> i64 {
    i64::from(2048 - c.freq) * 8
}
fn noise_period(c: &PsgChannel) -> i64 {
    if c.nr == 0 {
        16i64 << c.ns
    } else {
        i64::from(32 * c.nr) << c.ns
    }
}

fn noise_step(c: &mut PsgChannel) {
    let carry = c.lfsr & 1;
    c.lfsr >>= 1;
    if carry != 0 {
        c.lfsr ^= if c.narrow { 0x60 } else { 0x6000 };
        c.noise_out = 1;
    } else {
        c.noise_out = 0;
    }
}

fn sweep_next(c: &PsgChannel) -> i32 {
    let delta = c.shadow >> c.sweep_shift;
    if c.sweep_dir != 0 {
        c.shadow - delta
    } else {
        c.shadow + delta
    }
}

fn envelope_load(c: &mut PsgChannel, v: u16) {
    c.env_period = i32::from((v >> 8) & 7);
    c.env_dir = i32::from((v >> 11) & 1);
    c.dac = v & 0xF800 != 0;
    if !c.dac {
        c.on = false;
    }
}

impl Machine {
    fn reg(&self, off: u32) -> u16 {
        u16::from(self.io[off as usize]) | (u16::from(self.io[off as usize + 1]) << 8)
    }

    fn set_reg(&mut self, off: u32, v: u16) {
        self.io[off as usize] = v as u8;
        self.io[off as usize + 1] = (v >> 8) as u8;
    }

    /// Moves every PSG channel's waveform position forward to cycle `to`.
    fn psg_advance(&mut self, to: u64) {
        if to <= self.apu.psg_time {
            return;
        }
        let dt = (to - self.apu.psg_time) as i64;
        self.apu.psg_time = to;
        for i in 0..2 {
            let c = &mut self.apu.ch[i];
            if !c.on {
                continue;
            }
            let p = tone_period(c);
            c.timer -= dt;
            if c.timer <= 0 {
                let steps = (-c.timer) / p + 1;
                c.timer += steps * p;
                c.pos = ((i64::from(c.pos) + steps) & 7) as i32;
            }
        }
        let wave_span = if self.reg(0x70) & 0x20 != 0 { 64 } else { 32 };
        let wave_bank = self.apu.wave_bank;
        let wave = self.apu.wave;
        let w = &mut self.apu.ch[2];
        if w.on {
            let p = wave_period(w);
            w.timer -= dt;
            if w.timer <= 0 {
                let steps = (-w.timer) / p + 1;
                w.timer += steps * p;
                let span: i64 = wave_span;
                let last = ((i64::from(w.pos) + steps - 1) % span) as i32;
                w.pos = ((i64::from(last) + 1) % span) as i32;
                let mut bank = wave_bank;
                let mut idx = last;
                if span == 64 {
                    bank ^= idx >> 5;
                    idx &= 31;
                }
                let byte = wave[bank as usize][(idx >> 1) as usize];
                w.sample = if idx & 1 != 0 {
                    i32::from(byte & 0xF)
                } else {
                    i32::from(byte >> 4)
                };
            }
        }
        let n = &mut self.apu.ch[3];
        if n.on && n.ns < 14 {
            let p = noise_period(n);
            n.timer -= dt;
            if n.timer <= 0 {
                let mut steps = (-n.timer) / p + 1;
                n.timer += steps * p;
                let cycle: i64 = if n.narrow { 127 } else { 32767 };
                if steps > cycle {
                    steps = steps % cycle + cycle;
                }
                while steps > 0 {
                    steps -= 1;
                    noise_step(n);
                }
            }
        }
    }

    fn trigger(&mut self, i: usize) {
        let env_tone = if i == 0 {
            self.reg(0x62)
        } else {
            self.reg(0x68)
        };
        let env_noise = self.reg(0x78);
        let bank = (self.reg(0x70) >> 6) & 1;
        let c = &mut self.apu.ch[i];
        c.on = c.dac;
        if c.length == 0 {
            c.length = if i == 2 { 256 } else { 64 };
        }
        match i {
            0 | 1 => {
                c.volume = i32::from(env_tone >> 12);
                c.env_timer = c.env_period;
                c.timer = tone_period(c);
                if i == 0 {
                    c.shadow = c.freq;
                    c.sweep_timer = if c.sweep_period != 0 {
                        c.sweep_period
                    } else {
                        8
                    };
                    c.sweep_on = c.sweep_period != 0 || c.sweep_shift != 0;
                    if c.sweep_shift != 0 && sweep_next(c) > 2047 {
                        c.on = false;
                    }
                }
            }
            2 => {
                // Wave RAM plays as a rotating shift register, so a trigger
                // restarts the timer and leaves the position where it was. The
                // output holds until the first sample shifts out.
                c.timer = wave_period(c);
                c.sample = -1;
                self.apu.wave_bank = i32::from(bank);
            }
            _ => {
                c.volume = i32::from(env_noise >> 12);
                c.env_timer = c.env_period;
                c.lfsr = if c.narrow { 0x40 } else { 0x4000 };
                c.noise_out = 0;
                c.timer = noise_period(c);
            }
        }
    }

    fn power_off(&mut self) {
        for off in 0x60..0x82usize {
            self.io[off] = 0;
        }
        for i in 0..4 {
            self.apu.ch[i] = PsgChannel::default();
        }
    }

    pub(crate) fn apu_update_rate(&mut self) {
        let res = self.reg(0x88) >> 14;
        let shift = 9 - i32::from(res);
        if shift != self.apu.sample_shift {
            self.apu.sample_shift = shift;
            let period = 1u64 << shift;
            self.ev_sample = (self.now / period + 1) * period;
            self.sys_schedule();
        }
    }

    pub(crate) fn apu_write(&mut self, off: u32, value: u16, mask: u16) {
        if (0x90..0xA0).contains(&off) {
            // Wave RAM: the CPU reaches the bank that is not selected for play.
            let bank = (((self.reg(0x70) >> 6) & 1) ^ 1) as usize;
            let o = (off - 0x90) as usize;
            if mask & 0x00FF != 0 {
                self.apu.wave[bank][o] = value as u8;
            }
            if mask & 0xFF00 != 0 {
                self.apu.wave[bank][o + 1] = (value >> 8) as u8;
            }
            return;
        }
        if off >= 0xA0 {
            return; // the FIFOs are fed by apu_fifo_push
        }
        let master = self.io[0x84] & 0x80 != 0;
        if !master && off < 0x82 {
            return; // the PSG registers hold zero while sound is off
        }
        let now = self.now;
        self.psg_advance(now);
        let v = (self.reg(off) & !mask) | (value & mask);
        self.set_reg(off, v);
        match off {
            0x60 => {
                let c = &mut self.apu.ch[0];
                c.sweep_shift = i32::from(v & 7);
                c.sweep_dir = i32::from((v >> 3) & 1);
                c.sweep_period = i32::from((v >> 4) & 7);
            }
            0x62 | 0x68 => {
                let c = &mut self.apu.ch[if off == 0x62 { 0 } else { 1 }];
                c.duty = i32::from((v >> 6) & 3);
                if mask & 0x00FF != 0 {
                    c.length = 64 - i32::from(v & 0x3F);
                }
                if mask & 0xFF00 != 0 {
                    envelope_load(c, v);
                }
            }
            0x64 | 0x6C => {
                let i = if off == 0x64 { 0 } else { 1 };
                let c = &mut self.apu.ch[i];
                c.freq = i32::from(v & 0x7FF);
                c.length_on = (v >> 14) & 1 != 0;
                if mask & 0x8000 != 0 && value & 0x8000 != 0 {
                    self.trigger(i);
                }
            }
            0x70 => {
                let c = &mut self.apu.ch[2];
                c.dac = v & 0x80 != 0;
                if !c.dac {
                    c.on = false;
                }
            }
            0x72 => {
                if mask & 0x00FF != 0 {
                    self.apu.ch[2].length = 256 - i32::from(v & 0xFF);
                }
            }
            0x74 => {
                let c = &mut self.apu.ch[2];
                c.freq = i32::from(v & 0x7FF);
                c.length_on = (v >> 14) & 1 != 0;
                if mask & 0x8000 != 0 && value & 0x8000 != 0 {
                    self.trigger(2);
                }
            }
            0x78 => {
                let c = &mut self.apu.ch[3];
                if mask & 0x00FF != 0 {
                    c.length = 64 - i32::from(v & 0x3F);
                }
                if mask & 0xFF00 != 0 {
                    envelope_load(c, v);
                }
            }
            0x7C => {
                let c = &mut self.apu.ch[3];
                c.nr = i32::from(v & 7);
                c.narrow = (v >> 3) & 1 != 0;
                c.ns = i32::from((v >> 4) & 0xF);
                c.length_on = (v >> 14) & 1 != 0;
                if mask & 0x8000 != 0 && value & 0x8000 != 0 {
                    self.trigger(3);
                }
            }
            0x82 => {
                if mask & 0x0800 != 0 && value & 0x0800 != 0 {
                    self.apu.fifo[0].count = 0;
                    self.apu.fifo[0].read = 0;
                }
                if mask & 0x8000 != 0 && value & 0x8000 != 0 {
                    self.apu.fifo[1].count = 0;
                    self.apu.fifo[1].read = 0;
                }
            }
            0x84 => {
                if v & 0x80 == 0 && master {
                    self.power_off();
                }
                if v & 0x80 != 0 && !master {
                    self.apu.frame_step = 0;
                }
                self.set_reg(0x84, v & 0x80);
            }
            0x88 => self.apu_update_rate(),
            _ => {}
        }
    }

    pub(crate) fn apu_read(&self, off: u32) -> u16 {
        if (0x90..0xA0).contains(&off) {
            let bank = (((self.reg(0x70) >> 6) & 1) ^ 1) as usize;
            let o = (off - 0x90) as usize;
            return u16::from(self.apu.wave[bank][o])
                | (u16::from(self.apu.wave[bank][o + 1]) << 8);
        }
        match off {
            0x60 => self.reg(off) & 0x007F,
            0x62 | 0x68 => self.reg(off) & 0xFFC0,
            0x64 | 0x6C | 0x74 => self.reg(off) & 0x4000,
            0x70 => self.reg(off) & 0x00E0,
            0x72 => self.reg(off) & 0xE000,
            0x78 => self.reg(off) & 0xFF00,
            0x7C => self.reg(off) & 0x40FF,
            0x80 => self.reg(off) & 0xFF77,
            0x82 => self.reg(off) & 0x770F,
            0x84 => {
                let mut v = u16::from(self.io[0x84] & 0x80);
                for i in 0..4 {
                    if self.apu.ch[i].on {
                        v |= 1 << i;
                    }
                }
                v
            }
            0x88 => self.reg(off) & 0xC3FE,
            _ => 0,
        }
    }

    pub(crate) fn apu_fifo_push(&mut self, ch: usize, value: u32, bytes: u32) {
        let f = &mut self.apu.fifo[ch];
        for i in 0..bytes {
            if f.count >= 32 {
                break;
            }
            f.data[((f.read + f.count) & 31) as usize] = (value >> (8 * i)) as i8;
            f.count += 1;
        }
    }

    pub(crate) fn apu_timer_overflow(&mut self, timer: usize) {
        if self.io[0x84] & 0x80 == 0 {
            return;
        }
        let h = self.reg(0x82);
        for ch in 0..2usize {
            if usize::from((h >> (10 + 4 * ch)) & 1) != timer {
                continue;
            }
            if (h >> (8 + 4 * ch)) & 3 == 0 {
                continue; // neither side hears it, so nothing is taken
            }
            let now = self.now;
            let f = &mut self.apu.fifo[ch];
            if f.count > 0 {
                let s = f.data[f.read as usize];
                f.read = (f.read + 1) & 31;
                f.count -= 1;
                f.latch = s;
                if let Some(o) = self.observer.0.as_mut() {
                    o.fifo_sample(now, ch as i32, s);
                }
            }
            // The request comes once fewer than 16 bytes remain, which is the
            // threshold that keeps the stream in step after a restart.
            if self.apu.fifo[ch].count < 16 {
                self.dma_fifo_request(ch);
            }
        }
    }

    pub(crate) fn apu_frame_step(&mut self) {
        if self.io[0x84] & 0x80 == 0 {
            return;
        }
        let now = self.now;
        self.psg_advance(now);
        let step = self.apu.frame_step;
        self.apu.frame_step = (step + 1) & 7;
        let a = &mut self.apu;
        if step & 1 == 0 {
            for c in a.ch.iter_mut() {
                if c.length_on && c.length > 0 {
                    c.length -= 1;
                    if c.length == 0 {
                        c.on = false;
                    }
                }
            }
        }
        if step == 2 || step == 6 {
            let c = &mut a.ch[0];
            c.sweep_timer -= 1;
            if c.sweep_timer <= 0 {
                c.sweep_timer = if c.sweep_period != 0 {
                    c.sweep_period
                } else {
                    8
                };
                if c.sweep_on && c.sweep_period != 0 {
                    let nf = sweep_next(c);
                    if nf > 2047 {
                        c.on = false;
                    } else if c.sweep_shift != 0 {
                        c.shadow = nf;
                        c.freq = nf;
                        if sweep_next(c) > 2047 {
                            c.on = false;
                        }
                    }
                }
            }
        }
        if step == 7 {
            for i in 0..4 {
                if i == 2 {
                    continue;
                }
                let c = &mut a.ch[i];
                if c.env_period == 0 {
                    continue;
                }
                c.env_timer -= 1;
                if c.env_timer <= 0 {
                    c.env_timer = c.env_period;
                    if c.env_dir != 0 && c.volume < 15 {
                        c.volume += 1;
                    } else if c.env_dir == 0 && c.volume > 0 {
                        c.volume -= 1;
                    }
                }
            }
        }
    }

    /// One native output sample: what the PWM stage would emit now.
    pub(crate) fn apu_sample(&mut self) {
        let now = self.now;
        self.psg_advance(now);
        let mut lv = [0i32; 4];
        for i in 0..4 {
            lv[i] = self.apu.ch[i].held;
        }
        for i in 0..2 {
            let c = &self.apu.ch[i];
            if c.on {
                lv[i] = if DUTY_TABLE[c.duty as usize][c.pos as usize] != 0 {
                    c.volume
                } else {
                    0
                };
            }
        }
        let w = &self.apu.ch[2];
        if w.on && w.sample >= 0 {
            let h = self.reg(0x72);
            let s = w.sample;
            if h & 0x8000 != 0 {
                lv[2] = (s * 3) >> 2;
            } else {
                const SHIFT: [i32; 4] = [4, 0, 1, 2];
                lv[2] = s >> SHIFT[((h >> 13) & 3) as usize];
            }
        }
        let n = &self.apu.ch[3];
        if n.on {
            lv[3] = if n.noise_out != 0 { n.volume } else { 0 };
        }
        for i in 0..4 {
            self.apu.ch[i].held = lv[i];
        }

        let l = self.reg(0x80);
        let h = self.reg(0x82);
        let (mut psg_r, mut psg_l) = (0i32, 0i32);
        let mask = self.channel_mask;
        for i in 0..4 {
            if mask & (1 << i) == 0 {
                continue;
            }
            if l & (0x100 << i) != 0 {
                psg_r += lv[i];
            }
            if l & (0x1000 << i) != 0 {
                psg_l += lv[i];
            }
        }
        const PSG_SHIFT: [i32; 4] = [2, 1, 0, 0];
        let sh = PSG_SHIFT[(h & 3) as usize];
        psg_r = (psg_r * (1 + i32::from(l & 7)) * 2) >> sh;
        psg_l = (psg_l * (1 + i32::from((l >> 4) & 7)) * 2) >> sh;
        let (mut left, mut right) = (psg_l, psg_r);
        for ch in 0..2usize {
            if mask & (0x10 << ch) == 0 {
                continue;
            }
            let v = i32::from(self.apu.fifo[ch].latch) * if h & (4 << ch) != 0 { 4 } else { 2 };
            if h & (0x100 << (4 * ch)) != 0 {
                right += v;
            }
            if h & (0x200 << (4 * ch)) != 0 {
                left += v;
            }
        }
        let bias = self.reg(0x88);
        let level = i32::from(bias & 0x3FE);
        let drop = 1 + i32::from(bias >> 14);
        left += level;
        right += level;
        left = left.clamp(0, 0x3FF);
        right = right.clamp(0, 0x3FF);
        left = (left >> drop) << drop;
        right = (right >> drop) << drop;
        self.apu.last_level = [left - 0x200, right - 0x200];
        self.out_step(now, left - 0x200, right - 0x200);
    }

    pub(crate) fn apu_reset(&mut self) {
        self.apu = ApuState::default();
        self.apu.sample_shift = 9 - i32::from(self.reg(0x88) >> 14);
        self.apu.psg_time = self.now;
    }
}
