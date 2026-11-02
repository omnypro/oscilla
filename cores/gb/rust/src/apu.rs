//! The sound unit of the original Game Boy: two pulse channels (the first
//! with a sweep), a wave channel and a noise channel, a frame sequencer that
//! clocks lengths, sweep and envelopes at 512 Hz, a mixer and a high-pass
//! filter. Every channel is an event: it runs when its timer expires, and the
//! unit catches up to the cycle of each register access before it applies
//! the access.

use crate::state::*;
use std::sync::OnceLock;

/// The first register of each channel minus one: NRx1 is base + 1.
const BASE: [usize; 4] = [0, 5, 0xA, 0xF];

/// The bits that read back as 1 for 0xFF10..=0xFF26.
pub(crate) const READ_OR: [u8; 0x17] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, 0xFF, 0x3F, 0x00, 0xFF, 0xBF, 0x7F, 0xFF, 0x9F, 0xFF, 0xBF, 0xFF,
    0xFF, 0x00, 0x00, 0xBF, 0x00, 0x00, 0x70,
];

/// The duty patterns, one step per entry, from NR11 bits 7-6.
const DUTY: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 0],
];

/// Clocks between recomputations of the filter's output.
const HPF_TICK: u64 = 256;
/// The capacitor's charge factor for one clock.
const HPF_CHARGE: f64 = 0.999958;
/// Level units per mixer unit at the resampler's input.
const LEVEL_SCALE: f64 = 64.0;

/// The charge factor for n clocks, for n up to HPF_TICK. Built by repeated
/// multiplication, which gives the same bits on every platform.
fn decay_table() -> &'static [f64] {
    static TABLE: OnceLock<Vec<f64>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = Vec::with_capacity(HPF_TICK as usize + 1);
        let mut d = 1.0f64;
        t.push(d);
        for _ in 0..HPF_TICK {
            d *= HPF_CHARGE;
            t.push(d);
        }
        t
    })
}

fn decay(n: u64) -> f64 {
    let t = decay_table();
    let mut d = t[(n % HPF_TICK) as usize];
    let full = t[HPF_TICK as usize];
    for _ in 0..n / HPF_TICK {
        d *= full;
    }
    d
}

impl Default for Apu {
    fn default() -> Apu {
        Apu {
            on: false,
            r: [0; 0x17],
            wave_ram: [0; 16],
            // No generator runs before its first trigger.
            ch1: Pulse {
                next_step: NEVER,
                ..Pulse::default()
            },
            ch2: Pulse {
                next_step: NEVER,
                ..Pulse::default()
            },
            ch3: Wave {
                next_step: NEVER,
                ..Wave::default()
            },
            ch4: Noise {
                next_step: NEVER,
                ..Noise::default()
            },
            sweep: Sweep::default(),
            fs_step: 0,
            time: 0,
            gate: false,
            input: [0; 2],
            cap: [0.0; 2],
            hpf_time: 0,
            next_hpf: NEVER,
        }
    }
}

impl Apu {
    fn length_mut(&mut self, ch: usize) -> &mut Length {
        match ch {
            0 => &mut self.ch1.length,
            1 => &mut self.ch2.length,
            2 => &mut self.ch3.length,
            _ => &mut self.ch4.length,
        }
    }

    fn set_active(&mut self, ch: usize, on: bool) {
        match ch {
            0 => self.ch1.active = on,
            1 => self.ch2.active = on,
            2 => self.ch3.active = on,
            _ => self.ch4.active = on,
        }
    }

    fn active(&self, ch: usize) -> bool {
        match ch {
            0 => self.ch1.active,
            1 => self.ch2.active,
            2 => self.ch3.active,
            _ => self.ch4.active,
        }
    }

    fn env_mut(&mut self, ch: usize) -> &mut Envelope {
        match ch {
            0 => &mut self.ch1.env,
            1 => &mut self.ch2.env,
            _ => &mut self.ch4.env,
        }
    }

    fn reg(&self, ch: usize, n: usize) -> u8 {
        self.r[BASE[ch] + n]
    }

    /// The 11-bit period value of a pulse or wave channel.
    fn freq(&self, ch: usize) -> u16 {
        u16::from(self.reg(ch, 3)) | u16::from(self.reg(ch, 4) & 7) << 8
    }

    pub(crate) fn dac_on(&self, ch: usize) -> bool {
        if ch == 2 {
            self.r[0xA] & 0x80 != 0
        } else {
            self.reg(ch, 2) & 0xF8 != 0
        }
    }

    /// The digital output of a channel, 0 to 15, before the DAC.
    pub(crate) fn digital(&self, ch: usize) -> u8 {
        match ch {
            0 | 1 => {
                let p = if ch == 0 { &self.ch1 } else { &self.ch2 };
                if p.active && p.started {
                    DUTY[usize::from(self.reg(ch, 1) >> 6)][usize::from(p.duty_pos)] * p.env.volume
                } else {
                    0
                }
            }
            2 => {
                if !self.ch3.active {
                    return 0;
                }
                match self.r[0xC] >> 5 & 3 {
                    0 => 0,
                    1 => self.ch3.buffer,
                    2 => self.ch3.buffer >> 1,
                    _ => self.ch3.buffer >> 2,
                }
            }
            _ => {
                if self.ch4.active && self.ch4.lfsr & 1 != 0 {
                    self.ch4.env.volume
                } else {
                    0
                }
            }
        }
    }

    /// The clocks between steps of the noise channel's LFSR, or None when
    /// the shift stops it.
    fn noise_period(&self) -> Option<u64> {
        let nr43 = self.r[0x12];
        let shift = u32::from(nr43 >> 4);
        if shift >= 14 {
            return None;
        }
        let divider = u64::from(nr43 & 7);
        let base = if divider == 0 { 8 } else { 16 * divider };
        Some(base << shift)
    }

    /// The mixed level before the filter, left and right, and whether any DAC
    /// is on.
    pub(crate) fn mix_all(&self, mask: u8) -> ([i32; 2], bool) {
        self.mix(mask)
    }

    fn mix(&self, mask: u8) -> ([i32; 2], bool) {
        if !self.on {
            return ([0, 0], false);
        }
        let mut sum = [0i32; 2];
        let mut gate = false;
        for ch in 0..4 {
            if !self.dac_on(ch) {
                continue;
            }
            gate = true;
            if mask & (1 << ch) == 0 {
                continue;
            }
            // The DAC's slope is negative: digital 0 is the top level.
            let analog = 15 - 2 * i32::from(self.digital(ch));
            if self.r[0x15] & (0x10 << ch) != 0 {
                sum[0] += analog;
            }
            if self.r[0x15] & (1 << ch) != 0 {
                sum[1] += analog;
            }
        }
        let vl = i32::from(self.r[0x14] >> 4 & 7) + 1;
        let vr = i32::from(self.r[0x14] & 7) + 1;
        ([sum[0] * vl, sum[1] * vr], gate)
    }

    fn envelope_clock(&mut self, ch: usize) {
        let period = self.reg(ch, 2) & 7;
        let up = self.reg(ch, 2) & 8 != 0;
        let env = self.env_mut(ch);
        if !env.running {
            return;
        }
        env.timer = env.timer.saturating_sub(1);
        if env.timer != 0 {
            return;
        }
        env.timer = if period == 0 { 8 } else { period };
        if period == 0 {
            return;
        }
        if up && env.volume < 15 {
            env.volume += 1;
        } else if !up && env.volume > 0 {
            env.volume -= 1;
        } else {
            env.running = false;
        }
    }

    fn length_clock(&mut self, ch: usize) {
        let l = self.length_mut(ch);
        if l.enabled && l.count > 0 {
            l.count -= 1;
            if l.count == 0 {
                self.set_active(ch, false);
            }
        }
    }

    /// The new period of a sweep step. A result above 2047 turns channel 1
    /// off.
    fn sweep_calc(&mut self) -> u16 {
        let step = self.r[0] & 7;
        let delta = self.sweep.shadow >> step;
        let new = if self.r[0] & 8 != 0 {
            self.sweep.negated = true;
            self.sweep.shadow.wrapping_sub(delta)
        } else {
            self.sweep.shadow + delta
        };
        if new > 2047 {
            self.ch1.active = false;
        }
        new
    }

    fn sweep_clock(&mut self) {
        if self.sweep.timer > 0 {
            self.sweep.timer -= 1;
            if self.sweep.timer == 0 {
                let period = self.r[0] >> 4 & 7;
                self.sweep.timer = if period == 0 { 8 } else { period };
                if self.sweep.enabled && period != 0 {
                    let new = self.sweep_calc();
                    if new <= 2047 && self.r[0] & 7 != 0 {
                        self.sweep.shadow = new;
                        self.r[3] = new as u8;
                        self.r[4] = self.r[4] & !7 | (new >> 8) as u8;
                        self.sweep_calc();
                    }
                }
            }
        }
    }

    /// One tick of the 512 Hz frame sequencer.
    pub(crate) fn frame_sequencer_tick(&mut self) {
        let s = self.fs_step;
        self.fs_step = (s + 1) & 7;
        if s & 1 == 0 {
            for ch in 0..4 {
                self.length_clock(ch);
            }
        }
        if s == 2 || s == 6 {
            self.sweep_clock();
        }
        if s == 7 {
            for ch in [0, 1, 3] {
                self.envelope_clock(ch);
            }
        }
    }

    fn pulse_step(&mut self, ch: usize) {
        let freq = u64::from(self.freq(ch));
        let p = if ch == 0 {
            &mut self.ch1
        } else {
            &mut self.ch2
        };
        p.duty_pos = (p.duty_pos + 1) & 7;
        p.started = true;
        p.next_step += (2048 - freq) * 4;
    }

    fn wave_step(&mut self) {
        let freq = u64::from(self.freq(2));
        let w = &mut self.ch3;
        w.pos = (w.pos + 1) & 31;
        let byte = self.wave_ram[usize::from(w.pos >> 1)];
        w.buffer = if w.pos & 1 == 0 {
            byte >> 4
        } else {
            byte & 0xF
        };
        w.next_step += (2048 - freq) * 2;
    }

    fn noise_step(&mut self) {
        let period = self.noise_period();
        let n = &mut self.ch4;
        let l = n.lfsr;
        let bit = u16::from((l ^ l >> 1) & 1 == 0);
        let mut next = l >> 1 | bit << 14;
        if self.r[0x12] & 8 != 0 {
            next = next & !0x40 | bit << 6;
        }
        n.lfsr = next;
        n.next_step = match period {
            Some(p) => n.next_step + p,
            None => NEVER,
        };
    }

    fn power_off(&mut self) {
        self.on = false;
        self.r = [0; 0x17];
        for ch in 0..4 {
            self.set_active(ch, false);
            self.length_mut(ch).enabled = false;
        }
        self.ch1.next_step = NEVER;
        self.ch2.next_step = NEVER;
        self.ch3.next_step = NEVER;
        self.ch4.next_step = NEVER;
        self.ch1.env = Envelope::default();
        self.ch2.env = Envelope::default();
        self.ch4.env = Envelope::default();
        self.ch1.duty_pos = 0;
        self.ch2.duty_pos = 0;
        self.ch1.started = false;
        self.ch2.started = false;
        self.sweep = Sweep::default();
        self.next_hpf = NEVER;
    }

    fn power_on(&mut self) {
        self.on = true;
        self.fs_step = 0;
        self.ch3.buffer = 0;
    }
}

impl Machine {
    /// The cycle of the first frame-sequencer tick after `after`: the system
    /// counter's bit 12 falls when the counter is a multiple of 0x2000.
    fn next_fs(&self, after: u64) -> u64 {
        let x = (after + self.timer.sys_offset) & 0x1FFF;
        after + (0x2000 - x)
    }

    pub(crate) fn apu_frame_sequencer_tick(&mut self) {
        if self.apu.on {
            self.apu.frame_sequencer_tick();
        }
    }

    /// Runs every event up to and including cycle `t`.
    pub(crate) fn apu_advance(&mut self, t: u64) {
        if !self.apu.on {
            self.apu.time = t;
            return;
        }
        loop {
            let mut te = self.next_fs(self.apu.time);
            let mut which = 0u8;
            let a = &self.apu;
            for (i, at) in [
                a.ch1.next_step,
                a.ch2.next_step,
                a.ch3.next_step,
                a.ch4.next_step,
                a.next_hpf,
            ]
            .into_iter()
            .enumerate()
            {
                if at < te {
                    te = at;
                    which = i as u8 + 1;
                }
            }
            if te > t {
                break;
            }
            self.apu.time = te;
            match which {
                0 => self.apu.frame_sequencer_tick(),
                1 => self.apu.pulse_step(0),
                2 => self.apu.pulse_step(1),
                3 => self.apu.wave_step(),
                4 => self.apu.noise_step(),
                _ => {}
            }
            self.apu_emit(te);
        }
        self.apu.time = t;
    }

    /// Recomputes the mix and the filter's output at cycle `te` and hands the
    /// level to the resampler.
    pub(crate) fn apu_emit(&mut self, te: u64) {
        let a = &mut self.apu;
        if a.gate {
            let n = te - a.hpf_time;
            let d = decay(n);
            for i in 0..2 {
                let x = f64::from(a.input[i]);
                a.cap[i] = x + (a.cap[i] - x) * d;
            }
        }
        a.hpf_time = te;
        let (input, gate) = a.mix(self.channel_mask);
        a.input = input;
        a.gate = gate;
        let level = if gate {
            let q =
                |i: usize| ((f64::from(input[i]) - a.cap[i]) * LEVEL_SCALE + 0.5).floor() as i32;
            [q(0), q(1)]
        } else {
            [0, 0]
        };
        a.next_hpf = if gate { te + HPF_TICK } else { NEVER };
        if level != self.out.level() {
            self.last_change = te;
        }
        self.out.step(te, level[0], level[1]);
    }

    pub(crate) fn apu_read(&mut self, a: u16) -> u8 {
        let t = self.now;
        self.apu_advance(t);
        let ap = &self.apu;
        match a {
            0xFF10..=0xFF25 => {
                let i = usize::from(a - 0xFF10);
                ap.r[i] | READ_OR[i]
            }
            0xFF26 => {
                let status = u8::from(ap.ch1.active)
                    | u8::from(ap.ch2.active) << 1
                    | u8::from(ap.ch3.active) << 2
                    | u8::from(ap.ch4.active) << 3;
                u8::from(ap.on) << 7 | 0x70 | status
            }
            0xFF30..=0xFF3F => ap.wave_ram[usize::from(a - 0xFF30)],
            _ => 0xFF,
        }
    }

    pub(crate) fn apu_write(&mut self, a: u16, v: u8) {
        let t = self.now;
        self.apu_advance(t);
        if (0xFF30..=0xFF3F).contains(&a) {
            self.apu.wave_ram[usize::from(a - 0xFF30)] = v;
            return;
        }
        if a == 0xFF26 {
            let on = v & 0x80 != 0;
            if on && !self.apu.on {
                self.apu.power_on();
            } else if !on && self.apu.on {
                self.apu.power_off();
            }
            self.apu_emit(t);
            return;
        }
        if !(0xFF10..=0xFF25).contains(&a) {
            return;
        }
        let i = usize::from(a - 0xFF10);
        if !self.apu.on {
            // The length counters stay writable while the unit is off.
            match i {
                1 | 6 | 0x10 => {
                    self.apu
                        .length_mut(BASE.iter().position(|&b| b + 1 == i).unwrap_or(0))
                        .count = 64 - u16::from(v & 63)
                }
                0xB => self.apu.ch3.length.count = 256 - u16::from(v),
                _ => {}
            }
            return;
        }
        self.write_register(i, v, t);
        self.apu_emit(t);
    }

    fn write_register(&mut self, i: usize, v: u8, t: u64) {
        let ap = &mut self.apu;
        match i {
            0 => {
                let old = ap.r[0];
                ap.r[0] = v;
                if old & 8 != 0 && v & 8 == 0 && ap.sweep.negated {
                    ap.ch1.active = false;
                }
            }
            1 | 6 | 0x10 => {
                ap.r[i] = v;
                let ch = BASE.iter().position(|&b| b + 1 == i).unwrap_or(0);
                ap.length_mut(ch).count = 64 - u16::from(v & 63);
            }
            0xB => {
                ap.r[i] = v;
                ap.ch3.length.count = 256 - u16::from(v);
            }
            2 | 7 | 0x11 => {
                let ch = BASE.iter().position(|&b| b + 2 == i).unwrap_or(0);
                let old = ap.r[i];
                ap.r[i] = v;
                if ap.active(ch) {
                    // Writing the envelope register of a playing channel
                    // changes its volume.
                    let running = ap.env_mut(ch).running;
                    let env = ap.env_mut(ch);
                    let mut vol = env.volume;
                    if old & 7 == 0 && running {
                        vol += 1;
                    } else if old & 8 == 0 {
                        vol += 2;
                    }
                    if (old ^ v) & 8 != 0 {
                        vol = 16u8.wrapping_sub(vol);
                    }
                    env.volume = vol & 15;
                }
                if v & 0xF8 == 0 {
                    ap.set_active(ch, false);
                }
            }
            0xA => {
                ap.r[i] = v;
                if v & 0x80 == 0 {
                    ap.ch3.active = false;
                }
            }
            0x12 => {
                ap.r[i] = v;
                if ap.ch4.active
                    && ap.ch4.next_step == NEVER
                    && let Some(p) = ap.noise_period()
                {
                    ap.ch4.next_step = t + p;
                }
            }
            4 | 9 | 0xE | 0x13 => {
                let ch = BASE.iter().position(|&b| b + 4 == i).unwrap_or(0);
                self.write_nrx4(ch, v, t);
            }
            _ => ap.r[i] = v,
        }
    }

    fn write_nrx4(&mut self, ch: usize, v: u8, t: u64) {
        let ap = &mut self.apu;
        let old_enabled = ap.reg(ch, 4) & 0x40 != 0;
        ap.r[BASE[ch] + 4] = v;
        let enabled = v & 0x40 != 0;
        let next_clocks_length = ap.fs_step & 1 == 0;
        ap.length_mut(ch).enabled = enabled;
        if !next_clocks_length && !old_enabled && enabled && ap.length_mut(ch).count != 0 {
            let l = ap.length_mut(ch);
            l.count -= 1;
            if l.count == 0 && v & 0x80 == 0 {
                ap.set_active(ch, false);
            }
        }
        if v & 0x80 == 0 {
            return;
        }
        // A trigger does everything even when the DAC is off; the channel
        // just does not turn on.
        let dac = ap.dac_on(ch);
        ap.set_active(ch, true);
        let max = if ch == 2 { 256 } else { 64 };
        if ap.length_mut(ch).count == 0 {
            ap.length_mut(ch).count = max;
            if enabled && !next_clocks_length {
                ap.length_mut(ch).count -= 1;
            }
        }
        if ch != 2 {
            let period = ap.reg(ch, 2) & 7;
            let volume = ap.reg(ch, 2) >> 4;
            let next_is_envelope = ap.fs_step == 7;
            let env = ap.env_mut(ch);
            env.volume = volume;
            env.timer = (if period == 0 { 8 } else { period }) + u8::from(next_is_envelope);
            env.running = true;
        }
        match ch {
            0 | 1 => {
                let freq = u64::from(ap.freq(ch));
                let p = if ch == 0 { &mut ap.ch1 } else { &mut ap.ch2 };
                p.next_step = t + (2048 - freq) * 4;
                p.started = false;
                if ch == 0 {
                    let sw = ap.r[0];
                    ap.sweep.shadow = ap.freq(0);
                    let period = sw >> 4 & 7;
                    ap.sweep.timer = if period == 0 { 8 } else { period };
                    ap.sweep.enabled = period != 0 || sw & 7 != 0;
                    ap.sweep.negated = false;
                    if sw & 7 != 0 {
                        ap.sweep_calc();
                    }
                }
            }
            2 => {
                let freq = u64::from(ap.freq(2));
                ap.ch3.pos = 0;
                ap.ch3.next_step = t + (2048 - freq) * 2;
            }
            _ => {
                ap.ch4.lfsr = 0;
                ap.ch4.next_step = match ap.noise_period() {
                    Some(p) => t + p,
                    None => NEVER,
                };
            }
        }
        if !dac {
            ap.set_active(ch, false);
        }
    }
}
