//! The 2A03's sound unit: two pulse channels, the triangle, the noise channel,
//! the delta modulation channel and the frame counter that clocks them.
//! Everything runs from events, so time that no register touches costs
//! nothing. Written from public documentation of the registers and the mixer.

use crate::bus::WARN_CHIP;
use crate::state::*;
use oscilla_nsf::Region;
use std::sync::OnceLock;

const LENGTHS: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

const DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

const TRIANGLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

const NOISE_NTSC: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];
const NOISE_PAL: [u16; 16] = [
    4, 8, 14, 30, 60, 88, 118, 148, 188, 236, 354, 472, 708, 944, 1890, 3778,
];

const DMC_NTSC: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];
const DMC_PAL: [u16; 16] = [
    398, 354, 316, 298, 276, 236, 210, 198, 176, 148, 132, 118, 98, 78, 66, 50,
];

/// What a step of the frame sequence does.
const QUARTER: u8 = 1;
const HALF: u8 = 2;
const IRQ: u8 = 4;
const WRAP: u8 = 8;

type Sequence = [(u64, u8); 6];

const NTSC_4: Sequence = [
    (7457, QUARTER),
    (14913, QUARTER | HALF),
    (22371, QUARTER),
    (29828, IRQ),
    (29829, QUARTER | HALF | IRQ),
    (29830, IRQ | WRAP),
];
const NTSC_5: Sequence = [
    (7457, QUARTER),
    (14913, QUARTER | HALF),
    (22371, QUARTER),
    (29829, 0),
    (37281, QUARTER | HALF),
    (37282, WRAP),
];
const PAL_4: Sequence = [
    (8313, QUARTER),
    (16627, QUARTER | HALF),
    (24939, QUARTER),
    (33252, IRQ),
    (33253, QUARTER | HALF | IRQ),
    (33254, IRQ | WRAP),
];
const PAL_5: Sequence = [
    (8313, QUARTER),
    (16627, QUARTER | HALF),
    (24939, QUARTER),
    (33253, 0),
    (41565, QUARTER | HALF),
    (41566, WRAP),
];

/// The corner frequency of the console's output high-pass filter.
pub const CONSOLE_HIGHPASS_HZ: f64 = 90.0;
/// The lowest and highest cutoff that `set_highpass_hz` accepts.
pub const MIN_HIGHPASS_HZ: f64 = 1.0;
pub const MAX_HIGHPASS_HZ: f64 = 1000.0;
/// How far the capacitor moves in one tick, about. A higher cutoff gets a
/// shorter tick, down to HPF_TICK_MIN cycles.
const HPF_STEP: f64 = 0.0107;
const HPF_TICK_MIN: u64 = 16;
const HPF_TICK_MAX: u64 = 256;
/// Level units per mixer unit at the resampler's input.
pub(crate) const LEVEL_SCALE: f64 = 65536.0;

/// The high-pass filter's cutoff and the charge factors that follow from it. A
/// table gives the factor for each count of cycles up to one tick, built by
/// repeated multiplication, which gives the same bits on every platform.
#[derive(Clone)]
pub(crate) struct Highpass {
    hz: f64,
    pub(crate) tick: u64,
    table: [f64; HPF_TICK_MAX as usize + 1],
}

impl Highpass {
    /// A filter with its corner at `hz` for a CPU clock of `clock` cycles per
    /// second. The per-cycle factor is exp(-x) with x = 2 pi hz / clock, from
    /// its series, which holds only basic arithmetic.
    pub(crate) fn at(hz: f64, clock: (u64, u64)) -> Highpass {
        let x = 2.0 * std::f64::consts::PI * hz / (clock.0 as f64 / clock.1 as f64);
        let x2 = x * x;
        let x3 = x2 * x;
        let x4 = x2 * x2;
        let x5 = x4 * x;
        let charge = 1.0 - x + x2 / 2.0 - x3 / 6.0 + x4 / 24.0 - x5 / 120.0;
        let tick = ((HPF_STEP / x) as u64).clamp(HPF_TICK_MIN, HPF_TICK_MAX);
        let mut table = [0.0; HPF_TICK_MAX as usize + 1];
        let mut d = 1.0f64;
        table[0] = d;
        for slot in table.iter_mut().take(tick as usize + 1).skip(1) {
            d *= charge;
            *slot = d;
        }
        Highpass { hz, tick, table }
    }

    pub(crate) fn console(clock: (u64, u64)) -> Highpass {
        Highpass::at(CONSOLE_HIGHPASS_HZ, clock)
    }

    pub(crate) fn hz(&self) -> f64 {
        self.hz
    }

    /// The charge factor for n cycles.
    fn decay(&self, n: u64) -> f64 {
        let mut d = self.table[(n % self.tick) as usize];
        let full = self.table[self.tick as usize];
        for _ in 0..n / self.tick {
            d *= full;
            if d < 1e-18 {
                return 0.0;
            }
        }
        d
    }
}

struct Mixer {
    pulse: [f64; 31],
    tnd: [f64; 203],
}

fn mixer() -> &'static Mixer {
    static MIXER: OnceLock<Mixer> = OnceLock::new();
    MIXER.get_or_init(|| {
        let mut m = Mixer {
            pulse: [0.0; 31],
            tnd: [0.0; 203],
        };
        for (n, slot) in m.pulse.iter_mut().enumerate().skip(1) {
            *slot = 95.52 / (8128.0 / n as f64 + 100.0);
        }
        for (n, slot) in m.tnd.iter_mut().enumerate().skip(1) {
            *slot = 163.67 / (24329.0 / n as f64 + 100.0);
        }
        m
    })
}

fn noise_period(region: Region, index: u8) -> u64 {
    let table = match region {
        Region::Ntsc => &NOISE_NTSC,
        Region::Pal => &NOISE_PAL,
    };
    u64::from(table[usize::from(index & 15)])
}

fn dmc_period(region: Region, index: u8) -> u64 {
    let table = match region {
        Region::Ntsc => &DMC_NTSC,
        Region::Pal => &DMC_PAL,
    };
    u64::from(table[usize::from(index & 15)])
}

fn sequence(region: Region, five_step: bool) -> &'static Sequence {
    match (region, five_step) {
        (Region::Ntsc, false) => &NTSC_4,
        (Region::Ntsc, true) => &NTSC_5,
        (Region::Pal, false) => &PAL_4,
        (Region::Pal, true) => &PAL_5,
    }
}

impl Envelope {
    fn clock(&mut self, volume: u8, looping: bool) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = volume;
        } else if self.divider == 0 {
            self.divider = volume;
            if self.decay > 0 {
                self.decay -= 1;
            } else if looping {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }
}

impl Pulse {
    /// The period that the sweep would set. The first pulse channel negates
    /// with a ones' complement.
    fn target(&self, channel: usize) -> u32 {
        let period = u32::from(self.period);
        let change = period >> self.sweep.shift;
        if self.sweep.negate {
            period.saturating_sub(change + u32::from(channel == 0))
        } else {
            period + change
        }
    }

    /// Whether the channel makes no sound whatever the sequencer does.
    fn silent(&self, channel: usize) -> bool {
        self.len == 0 || self.period < 8 || self.target(channel) > 0x7FF
    }

    fn volume_out(&self) -> u8 {
        if self.constant {
            self.volume
        } else {
            self.env.decay
        }
    }

    fn output(&self, channel: usize) -> u8 {
        if self.silent(channel) || DUTY[usize::from(self.duty)][usize::from(self.pos)] == 0 {
            0
        } else {
            self.volume_out()
        }
    }

    fn period_cycles(&self) -> u64 {
        2 * (u64::from(self.period) + 1)
    }

    /// Brings a stale timer past `t`, with the sequencer where it would be.
    fn settle(&mut self, t: u64) {
        if self.next_step <= t {
            let p = self.period_cycles();
            let k = (t - self.next_step) / p + 1;
            self.pos = self.pos.wrapping_sub((k % 8) as u8) & 7;
            self.next_step += k * p;
        }
    }

    fn sweep_clock(&mut self, channel: usize) {
        let target = self.target(channel);
        let mute = self.period < 8 || target > 0x7FF;
        let s = &mut self.sweep;
        if s.divider == 0 && s.enabled && s.shift != 0 && !mute {
            self.period = target as u16;
        }
        let s = &mut self.sweep;
        if s.divider == 0 || s.reload {
            s.divider = s.period;
            s.reload = false;
        } else {
            s.divider -= 1;
        }
    }
}

impl Triangle {
    fn gated(&self) -> bool {
        self.linear == 0 || self.len == 0
    }

    fn settle(&mut self, t: u64) {
        if self.next_step <= t {
            let p = u64::from(self.period) + 1;
            self.next_step += ((t - self.next_step) / p + 1) * p;
        }
    }

    fn linear_clock(&mut self) {
        if self.linear_reload {
            self.linear = self.linear_value;
        } else if self.linear > 0 {
            self.linear -= 1;
        }
        if !self.control {
            self.linear_reload = false;
        }
    }
}

impl Noise {
    fn output(&self) -> u8 {
        if self.lfsr & 1 != 0 || self.len == 0 {
            0
        } else if self.constant {
            self.volume
        } else {
            self.env.decay
        }
    }

    fn step(&mut self) {
        let tap = if self.mode { 6 } else { 1 };
        let feedback = (self.lfsr ^ (self.lfsr >> tap)) & 1;
        self.lfsr = (self.lfsr >> 1) | (feedback << 14);
    }
}

impl Apu {
    pub(crate) fn new(region: Region) -> Apu {
        let mut a = Apu {
            pulse: [Pulse::default(), Pulse::default()],
            tri: Triangle::default(),
            noise: Noise {
                lfsr: 1,
                ..Noise::default()
            },
            dmc: Dmc {
                silent: true,
                bits: 8,
                ..Dmc::default()
            },
            enabled: 0,
            frame: FrameCounter::default(),
            time: 0,
            input: 0.0,
            cap: 0.0,
            hpf_time: 0,
            next_hpf: NEVER,
            stall: 0,
        };
        for p in &mut a.pulse {
            p.next_step = p.period_cycles();
        }
        a.tri.next_step = 1;
        a.noise.next_step = noise_period(region, 0);
        a.dmc.next_step = dmc_period(region, 0);
        a.frame.next = sequence(region, false)[0].0;
        a
    }

    fn settle(&mut self, t: u64) {
        for p in &mut self.pulse {
            p.settle(t);
        }
        self.tri.settle(t);
    }

    /// The five levels before the mixer: pulse 1, pulse 2, triangle, noise
    /// (0 to 15) and the delta channel (0 to 127).
    pub(crate) fn levels(&self) -> [u8; 5] {
        [
            self.pulse[0].output(0),
            self.pulse[1].output(1),
            TRIANGLE[usize::from(self.tri.pos)],
            self.noise.output(),
            self.dmc.level,
        ]
    }

    /// The mix of the channels that `mask` lets through, in the mixer's units.
    pub(crate) fn mix(&self, mask: u8) -> f64 {
        let l = self.levels();
        let on = |n: usize| {
            if mask >> n & 1 != 0 {
                usize::from(l[n])
            } else {
                0
            }
        };
        let m = mixer();
        m.pulse[on(0) + on(1)] + m.tnd[3 * on(2) + 2 * on(3) + on(4)]
    }

    fn quarter_clock(&mut self) {
        for p in &mut self.pulse {
            p.env.clock(p.volume, p.halt);
        }
        let n = &mut self.noise;
        n.env.clock(n.volume, n.halt);
        self.tri.linear_clock();
    }

    fn half_clock(&mut self) {
        for p in &mut self.pulse {
            if !p.halt && p.len > 0 {
                p.len -= 1;
            }
        }
        if !self.tri.control && self.tri.len > 0 {
            self.tri.len -= 1;
        }
        if !self.noise.halt && self.noise.len > 0 {
            self.noise.len -= 1;
        }
        for (i, p) in self.pulse.iter_mut().enumerate() {
            p.sweep_clock(i);
        }
    }
}

impl Machine {
    /// Runs every event up to and including cycle `t`.
    pub(crate) fn apu_advance(&mut self, t: u64) {
        loop {
            let a = &self.apu;
            let mut te = a.frame.next;
            let mut which = 0u8;
            let candidates = [
                if a.pulse[0].silent(0) {
                    NEVER
                } else {
                    a.pulse[0].next_step
                },
                if a.pulse[1].silent(1) {
                    NEVER
                } else {
                    a.pulse[1].next_step
                },
                if a.tri.gated() {
                    NEVER
                } else {
                    a.tri.next_step
                },
                a.noise.next_step,
                a.dmc.next_step,
                a.next_hpf,
            ];
            for (i, at) in candidates.into_iter().enumerate() {
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
                0 => self.frame_event(te),
                1 | 2 => {
                    let p = &mut self.apu.pulse[usize::from(which - 1)];
                    p.pos = p.pos.wrapping_sub(1) & 7;
                    p.next_step += p.period_cycles();
                }
                3 => {
                    let tri = &mut self.apu.tri;
                    tri.pos = (tri.pos + 1) & 31;
                    tri.next_step += u64::from(tri.period) + 1;
                }
                4 => {
                    let region = self.region;
                    let n = &mut self.apu.noise;
                    n.step();
                    n.next_step += noise_period(region, n.period_index);
                }
                5 => self.dmc_step(te),
                _ => {}
            }
            self.apu_emit(te);
        }
        self.apu.time = t;
    }

    fn frame_event(&mut self, te: u64) {
        self.apu.settle(te);
        let region = self.region;
        let f = &mut self.apu.frame;
        if f.immediate {
            f.immediate = false;
            f.next = f.origin + sequence(region, f.five_step)[f.step].0;
            self.apu.quarter_clock();
            self.apu.half_clock();
            return;
        }
        let (offset, flags) = sequence(region, f.five_step)[f.step];
        if flags & IRQ != 0 && !f.five_step && !f.inhibit {
            f.irq = true;
        }
        if flags & WRAP != 0 {
            f.origin += offset;
            f.step = 0;
        } else {
            f.step += 1;
        }
        f.next = f.origin + sequence(region, f.five_step)[f.step].0;
        if flags & QUARTER != 0 {
            self.apu.quarter_clock();
        }
        if flags & HALF != 0 {
            self.apu.half_clock();
        }
    }

    /// One expiry of the delta channel's timer.
    fn dmc_step(&mut self, te: u64) {
        let region = self.region;
        let d = &mut self.apu.dmc;
        d.next_step += dmc_period(region, d.rate_index);
        if !d.silent {
            if d.shift & 1 != 0 {
                if d.level < 126 {
                    d.level += 2;
                }
            } else if d.level > 1 {
                d.level -= 2;
            }
            d.shift >>= 1;
        }
        d.bits -= 1;
        if d.bits == 0 {
            d.bits = 8;
            match d.buffer.take() {
                Some(byte) => {
                    d.silent = false;
                    d.shift = byte;
                }
                None => d.silent = true,
            }
            self.dmc_fill(te);
        }
    }

    /// Reads the next sample byte into an empty buffer. The read takes the
    /// CPU's time.
    fn dmc_fill(&mut self, _te: u64) {
        let d = &self.apu.dmc;
        if d.buffer.is_some() || d.remaining == 0 {
            return;
        }
        let byte = self.read_rom(d.address);
        let d = &mut self.apu.dmc;
        d.buffer = Some(byte);
        d.address = if d.address == 0xFFFF {
            0x8000
        } else {
            d.address + 1
        };
        d.remaining -= 1;
        if d.remaining == 0 {
            if d.looping {
                d.address = d.sample_address;
                d.remaining = d.sample_length;
            } else if d.irq_enable {
                d.irq = true;
            }
        }
        self.apu.stall += 4;
    }

    /// Recomputes the mix and the filter's output at cycle `te` and hands the
    /// level to the resampler.
    pub(crate) fn apu_emit(&mut self, te: u64) {
        let a = &mut self.apu;
        let n = te - a.hpf_time;
        let d = self.hp.decay(n);
        a.cap = a.input + (a.cap - a.input) * d;
        a.hpf_time = te;
        a.input = a.mix(self.channel_mask);
        let diff = a.input - a.cap;
        let level = (diff * LEVEL_SCALE + 0.5).floor() as i32;
        // The output stays at its level when the capacitor has caught up, so
        // there is nothing to recompute until the mix changes again.
        a.next_hpf = if diff.abs() * LEVEL_SCALE < 0.25 {
            NEVER
        } else {
            te + self.hp.tick
        };
        if level != self.out.level()[0] {
            self.last_change = te;
        }
        self.out.step(te, level, level);
    }

    pub(crate) fn apu_read(&mut self, address: u16) -> u8 {
        self.sync_apu();
        if address != 0x4015 {
            return self.bus;
        }
        let a = &mut self.apu;
        let mut v = u8::from(a.pulse[0].len > 0)
            | u8::from(a.pulse[1].len > 0) << 1
            | u8::from(a.tri.len > 0) << 2
            | u8::from(a.noise.len > 0) << 3
            | u8::from(a.dmc.remaining > 0) << 4
            | (self.bus & 0x20)
            | u8::from(a.frame.irq) << 6
            | u8::from(a.dmc.irq) << 7;
        a.frame.irq = false;
        v &= 0xFF;
        v
    }

    /// Brings the sound unit to the CPU's cycle and charges the CPU for the
    /// sample reads that happened on the way.
    pub(crate) fn sync_apu(&mut self) {
        loop {
            let t = self.now;
            self.apu_advance(t);
            if self.apu.stall == 0 {
                break;
            }
            self.now += self.apu.stall;
            self.apu.stall = 0;
        }
    }

    pub(crate) fn apu_write(&mut self, address: u16, v: u8) {
        self.sync_apu();
        let t = self.now;
        self.apu.settle(t);
        let region = self.region;
        let a = &mut self.apu;
        let enabled = |n: u8, a: &Apu| a.enabled >> n & 1 != 0;
        match address {
            0x4000 | 0x4004 => {
                let p = &mut a.pulse[usize::from(address >> 2 & 1)];
                p.duty = v >> 6;
                p.halt = v & 0x20 != 0;
                p.constant = v & 0x10 != 0;
                p.volume = v & 15;
            }
            0x4001 | 0x4005 => {
                let s = &mut a.pulse[usize::from(address >> 2 & 1)].sweep;
                s.enabled = v & 0x80 != 0;
                s.period = v >> 4 & 7;
                s.negate = v & 8 != 0;
                s.shift = v & 7;
                s.reload = true;
            }
            0x4002 | 0x4006 => {
                let p = &mut a.pulse[usize::from(address >> 2 & 1)];
                p.period = p.period & 0x700 | u16::from(v);
            }
            0x4003 | 0x4007 => {
                let i = usize::from(address >> 2 & 1);
                let load = enabled(i as u8, a);
                let p = &mut a.pulse[i];
                p.period = p.period & 0xFF | u16::from(v & 7) << 8;
                if load {
                    p.len = LENGTHS[usize::from(v >> 3)];
                }
                p.pos = 0;
                p.env.start = true;
            }
            0x4008 => {
                a.tri.control = v & 0x80 != 0;
                a.tri.linear_value = v & 0x7F;
            }
            0x400A => a.tri.period = a.tri.period & 0x700 | u16::from(v),
            0x400B => {
                let load = enabled(2, a);
                a.tri.period = a.tri.period & 0xFF | u16::from(v & 7) << 8;
                if load {
                    a.tri.len = LENGTHS[usize::from(v >> 3)];
                }
                a.tri.linear_reload = true;
            }
            0x400C => {
                let n = &mut a.noise;
                n.halt = v & 0x20 != 0;
                n.constant = v & 0x10 != 0;
                n.volume = v & 15;
            }
            0x400E => {
                a.noise.mode = v & 0x80 != 0;
                a.noise.period_index = v & 15;
            }
            0x400F => {
                let load = enabled(3, a);
                if load {
                    a.noise.len = LENGTHS[usize::from(v >> 3)];
                }
                a.noise.env.start = true;
            }
            0x4010 => {
                let d = &mut a.dmc;
                d.irq_enable = v & 0x80 != 0;
                if !d.irq_enable {
                    d.irq = false;
                }
                d.looping = v & 0x40 != 0;
                d.rate_index = v & 15;
            }
            0x4011 => a.dmc.level = v & 0x7F,
            0x4012 => a.dmc.sample_address = 0xC000 | u16::from(v) << 6,
            0x4013 => a.dmc.sample_length = u16::from(v) << 4 | 1,
            0x4015 => {
                a.enabled = v & 0x1F;
                if v & 1 == 0 {
                    a.pulse[0].len = 0;
                }
                if v & 2 == 0 {
                    a.pulse[1].len = 0;
                }
                if v & 4 == 0 {
                    a.tri.len = 0;
                }
                if v & 8 == 0 {
                    a.noise.len = 0;
                }
                let d = &mut a.dmc;
                d.irq = false;
                if v & 0x10 == 0 {
                    d.remaining = 0;
                } else if d.remaining == 0 {
                    d.address = d.sample_address;
                    d.remaining = d.sample_length;
                    self.dmc_fill(t);
                }
            }
            0x4017 => {
                let f = &mut a.frame;
                f.five_step = v & 0x80 != 0;
                f.inhibit = v & 0x40 != 0;
                if f.inhibit {
                    f.irq = false;
                }
                // The new sequence starts three or four cycles after the write.
                f.origin = t + 3 + (t & 1);
                f.step = 0;
                f.immediate = f.five_step;
                f.next = if f.immediate {
                    f.origin
                } else {
                    f.origin + sequence(region, false)[0].0
                };
            }
            _ => {}
        }
        // A change may alter the mix at once.
        self.apu_emit(t);
    }

    /// Notes the expansion chips that a file asks for and the core does not
    /// play.
    pub(crate) fn warn_chips(&mut self) {
        if self.chips != 0 {
            self.warn(
                WARN_CHIP,
                "The file asks for expansion sound that the core does not play.",
            );
        }
    }
}
