//! The HuC6280's programmable sound generator: six channels that play a
//! 32-step waveform of 5-bit words, two of them (5 and 6) switchable to noise,
//! a low frequency oscillator that modulates channel 1 with channel 2's
//! waveform, and a direct D/A mode. Written from the manufacturer's sound
//! generator manual. DESIGN.md lists what the manual leaves open.

/// A channel steps its waveform every F ticks, and a tick is two cycles of the
/// 7.16 MHz clock (3.579545 MHz).
pub const TICK_HZ: u64 = 3_579_545;

/// The gain of 1.5 dB steps in 16.16 fixed point, for 0 to 31 steps down.
/// `round(65536 * 10^(-1.5 * k / 20))`.
const GAIN: [u32; 32] = [
    65536, 55142, 46396, 39037, 32846, 27636, 23253, 19565, 16462, 13851, 11654, 9806, 8250, 6942,
    5841, 4915, 4135, 3479, 2927, 2463, 2072, 1744, 1467, 1234, 1039, 874, 735, 619, 521, 438, 369,
    310,
];

/// The period that a frequency of 0 stands for. The manual forbids the value,
/// and a 12-bit counter that starts from 0 counts a full turn.
const ZERO_PERIOD: u32 = 4096;

/// The period of the noise clock when the complement of its register is 0,
/// which the manual leaves undefined.
const ZERO_NOISE: u32 = 32;

#[derive(Clone, Debug, Default)]
struct Channel {
    frequency: u16,
    on: bool,
    dda: bool,
    amplitude: u8,
    balance: u8,
    wave: [u8; 32],
    /// The address counter that writes and playback share.
    address: u8,
    /// The value that direct D/A mode holds.
    direct: u8,
    noise_enable: bool,
    noise_frequency: u8,
    /// Ticks until the waveform steps.
    wave_left: u32,
    noise_left: u32,
    lfsr: u16,
}

/// The samples of each channel's output that the scope keeps: 4,096 at 44,100
/// Hz, which is 93 ms.
pub const SCOPE_LEN: usize = 4096;

/// The rate of the scope's samples.
pub const SCOPE_HZ: u64 = 44_100;

/// The recent output of each channel before the mask and the mixer: the channel's
/// word about its midpoint times its two gains, averaged and scaled to 16 bits.
/// It is not part of a copy of the generator, which starts with an empty one.
#[derive(Debug)]
struct Scope {
    /// The ring, allocated by the first sample so that a copy costs nothing.
    data: Option<Box<[i16]>>,
    /// The index that the next sample goes to.
    at: usize,
}

impl Scope {
    fn new() -> Scope {
        Scope { data: None, at: 0 }
    }

    /// Adds `n` samples of the same values.
    fn push(&mut self, samples: [i16; 6], n: u64) {
        let data = self
            .data
            .get_or_insert_with(|| vec![0; 6 * SCOPE_LEN].into_boxed_slice());
        for _ in 0..n.min(SCOPE_LEN as u64) {
            // The channels of one moment sit side by side.
            data[self.at * 6..(self.at + 1) * 6].copy_from_slice(&samples);
            self.at += 1;
            if self.at == SCOPE_LEN {
                self.at = 0;
            }
        }
    }

    fn get(&self, channel: usize, back: usize) -> i16 {
        match &self.data {
            Some(data) if back < SCOPE_LEN => {
                data[((self.at + SCOPE_LEN - 1 - back) % SCOPE_LEN) * 6 + channel]
            }
            _ => 0,
        }
    }
}

impl Clone for Scope {
    fn clone(&self) -> Scope {
        Scope::new()
    }
}

/// What a channel is doing, from its registers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsgVoice {
    /// The channel's on bit.
    pub on: bool,
    /// Direct D/A mode: the channel plays the words that the program writes.
    pub direct: bool,
    /// The channel makes noise: only channels 5 and 6 can.
    pub noise: bool,
    /// The 12-bit frequency register.
    pub frequency: u16,
    /// The channel's amplitude register, 0 to 31.
    pub amplitude: u8,
    /// The gain to the left and to the right from the main level, the balance
    /// and the amplitude, in 16.16 (65,536 is a gain of 1).
    pub gain: (u32, u32),
    /// A number that is the same for equal waveforms.
    pub wave_id: u32,
    /// The word that the channel puts out now, 0 to 31.
    pub word: u8,
    /// The low frequency oscillator modulates this channel or runs it.
    pub lfo: bool,
}

#[derive(Clone, Debug)]
pub struct Psg {
    select: u8,
    main: u8,
    lfo_frequency: u8,
    lfo_control: u8,
    lfo_reset: bool,
    ch: [Channel; 6],
    mask: u8,
    level: (i32, i32),
    scope: Scope,
    /// What each channel adds to the left and right output now, with no mask:
    /// the mix is their masked sum and the scope holds them.
    contrib: [(i64, i64); 6],
    /// Ticks of the scope's sample that have not made a whole sample yet,
    /// counted in 1/3,579,545 of a scope sample.
    scope_acc: u64,
}

impl Default for Psg {
    fn default() -> Psg {
        Psg::new()
    }
}

impl Psg {
    pub fn new() -> Psg {
        let mut ch: [Channel; 6] = Default::default();
        for c in &mut ch {
            c.lfsr = 0x7FFF;
            c.wave_left = ZERO_PERIOD;
            c.noise_left = ZERO_NOISE * 64;
        }
        let mut psg = Psg {
            select: 0,
            main: 0,
            lfo_frequency: 0,
            lfo_control: 0,
            lfo_reset: false,
            ch,
            mask: 0x3F,
            level: (0, 0),
            scope: Scope::new(),
            contrib: [(0, 0); 6],
            scope_acc: 0,
        };
        psg.level = psg.mix();
        psg
    }

    /// Chooses the channels that reach the output: bit 0 is channel 1.
    pub fn set_mask(&mut self, mask: u8) {
        self.mask = mask & 0x3F;
        self.level = self.mix();
    }

    pub fn mask(&self) -> u8 {
        self.mask
    }

    /// The output level now, left and right, all channels added. A channel at
    /// full level with the word 31 gives 31 * 65536 and with the word 0 gives
    /// -31 * 65536; a channel that is off gives 0.
    pub fn level(&self) -> (i32, i32) {
        self.level
    }

    /// What each channel puts out now, as a 5-bit word before the amplitude
    /// controls, for meters.
    pub fn words(&self) -> [u8; 6] {
        let mut out = [0; 6];
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.word(i);
        }
        out
    }

    /// The channel that register 0 has selected.
    pub fn selected(&self) -> u8 {
        self.select
    }

    /// What each channel puts out now before the amplitude controls: the word
    /// about its midpoint in half steps (-31 to 31), or 0 for a channel that
    /// is off.
    pub fn levels(&self) -> [i32; 6] {
        let mut out = [0; 6];
        for (i, o) in out.iter_mut().enumerate() {
            if self.ch[i].on {
                *o = i32::from(self.word(i)) * 2 - 31;
            }
        }
        out
    }

    fn lfo_active(&self) -> bool {
        self.lfo_control & 3 != 0 && !self.lfo_reset && self.ch[0].on && self.ch[1].on
    }

    /// A store to one of the ten registers. The registers 2 to 7 belong to the
    /// channel that register 0 selects.
    pub fn write(&mut self, register: u8, value: u8) {
        match register & 15 {
            0 => self.select = value & 7,
            1 => self.main = value,
            8 => self.lfo_frequency = value,
            9 => {
                self.lfo_control = value & 3;
                self.lfo_reset = value & 0x80 != 0;
                if self.lfo_reset {
                    self.ch[1].address = 0;
                }
            }
            r @ 2..=7 => {
                if let Some(c) = self.ch.get_mut(usize::from(self.select)) {
                    Psg::write_channel(c, r, value);
                }
            }
            _ => {}
        }
        self.level = self.mix();
    }

    fn write_channel(c: &mut Channel, register: u8, value: u8) {
        match register {
            2 => c.frequency = c.frequency & 0xF00 | u16::from(value),
            3 => c.frequency = c.frequency & 0x0FF | u16::from(value & 15) << 8,
            4 => {
                c.on = value & 0x80 != 0;
                c.dda = value & 0x40 != 0;
                c.amplitude = value & 0x1F;
                if c.dda {
                    c.address = 0;
                }
            }
            5 => c.balance = value,
            6 => {
                let word = value & 0x1F;
                if c.dda {
                    c.direct = word;
                } else if !c.on {
                    c.wave[usize::from(c.address)] = word;
                    c.address = (c.address + 1) & 31;
                }
            }
            7 => {
                c.noise_enable = value & 0x80 != 0;
                c.noise_frequency = value & 0x1F;
            }
            _ => {}
        }
    }

    /// The 5-bit word that channel `i` puts out now.
    fn word(&self, i: usize) -> u8 {
        let c = &self.ch[i];
        if !c.on {
            return 0;
        }
        if c.dda {
            return c.direct;
        }
        if i >= 4 && c.noise_enable {
            return if c.lfsr & 1 != 0 { 31 } else { 0 };
        }
        c.wave[usize::from(c.address)]
    }

    /// The gain of channel `i` to one side, in 16.16, from the main level, the
    /// channel's amplitude and its balance. The three add as steps of 3 dB, 1.5
    /// dB and 3 dB, and the channel is silent when the steps of 3 dB come to
    /// more than 15.
    fn gain(&self, i: usize, left: bool) -> u32 {
        let c = &self.ch[i];
        let (main, balance) = if left {
            (self.main >> 4, c.balance >> 4)
        } else {
            (self.main & 15, c.balance & 15)
        };
        let coarse =
            u32::from(15 - main) + u32::from(15 - balance) + u32::from(15 - (c.amplitude >> 1));
        if coarse > 15 {
            return 0;
        }
        let steps = coarse * 2 + u32::from(1 - (c.amplitude & 1));
        GAIN[steps as usize]
    }

    /// What each channel adds to the left and right output, with no mask.
    fn contribution(&self, i: usize) -> (i64, i64) {
        if !self.ch[i].on {
            return (0, 0);
        }
        // The words run from 0 to 31. The output takes them about their
        // midpoint, in half steps, so a channel puts out no offset.
        let w = i64::from(self.word(i)) * 2 - 31;
        (
            w * i64::from(self.gain(i, true)),
            w * i64::from(self.gain(i, false)),
        )
    }

    /// The state of channel `i`.
    pub fn voice(&self, i: usize) -> PsgVoice {
        let i = i % 6;
        let c = &self.ch[i];
        let mut wave_id = 17u32;
        for w in c.wave {
            wave_id = wave_id.wrapping_mul(31).wrapping_add(u32::from(w));
        }
        PsgVoice {
            on: c.on,
            direct: c.dda,
            noise: i >= 4 && c.noise_enable,
            frequency: c.frequency,
            amplitude: c.amplitude,
            gain: (self.gain(i, true), self.gain(i, false)),
            wave_id: wave_id & 0x7FFF_FFFF,
            word: self.word(i),
            lfo: self.lfo_active() && i < 2,
        }
    }

    /// The sample `back` steps before the newest of a channel's scope.
    pub fn scope_sample(&self, channel: usize, back: usize) -> i16 {
        self.scope.get(channel % 6, back)
    }

    /// Works out what each channel adds and returns the sum of the channels
    /// that the mask lets through.
    fn mix(&mut self) -> (i32, i32) {
        let (mut l, mut r) = (0i64, 0i64);
        for i in 0..6 {
            self.contrib[i] = self.contribution(i);
            if self.mask >> i & 1 != 0 {
                l += self.contrib[i].0;
                r += self.contrib[i].1;
            }
        }
        (l as i32, r as i32)
    }

    /// The ticks that channel `i`'s waveform waits before its next step.
    fn wave_period(&self, i: usize) -> u32 {
        let c = &self.ch[i];
        let mut f = u32::from(c.frequency);
        if self.lfo_active() {
            if i == 1 {
                let lfo = u32::from(self.lfo_frequency);
                let lfo = if lfo == 0 { 256 } else { lfo };
                let f2 = if f == 0 { ZERO_PERIOD } else { f };
                return f2 * lfo;
            }
            if i == 0 {
                let word = u32::from(self.ch[1].wave[usize::from(self.ch[1].address)]);
                let signed = (word ^ 0x10) as i32 - if word ^ 0x10 >= 16 { 32 } else { 0 };
                let shift = match self.lfo_control & 3 {
                    1 => 0,
                    2 => 2,
                    _ => 4,
                };
                f = (f as i32 + (signed << shift)) as u32 & 0xFFF;
            }
        }
        if f == 0 { ZERO_PERIOD } else { f }
    }

    fn noise_period(&self, i: usize) -> u32 {
        let complement = u32::from(!self.ch[i].noise_frequency & 31);
        64 * if complement == 0 {
            ZERO_NOISE
        } else {
            complement
        }
    }

    /// Runs the channels for `ticks` ticks. `emit` gets the tick (counted from
    /// the start of this call) and the new level each time the level changes.
    pub fn run(&mut self, ticks: u64, emit: &mut impl FnMut(u64, i32, i32)) {
        let mut done = 0u64;
        while done < ticks {
            let mut step = ticks - done;
            for c in &self.ch {
                step = step.min(u64::from(c.wave_left.max(1)));
                step = step.min(u64::from(c.noise_left.max(1)));
            }
            self.scope_acc += step * SCOPE_HZ;
            let n = self.scope_acc / TICK_HZ;
            self.scope_acc %= TICK_HZ;
            if n > 0 {
                let mut held = [0i16; 6];
                for (h, (l, r)) in held.iter_mut().zip(self.contrib) {
                    *h = (((l + r) / 2) >> 6).clamp(-32768, 32767) as i16;
                }
                self.scope.push(held, n);
            }
            done += step;
            let step = step as u32;
            for i in 0..6 {
                let c = &mut self.ch[i];
                c.wave_left = c.wave_left.saturating_sub(step);
                c.noise_left = c.noise_left.saturating_sub(step);
            }
            for i in 0..6 {
                if self.ch[i].noise_left == 0 {
                    let period = self.noise_period(i);
                    let c = &mut self.ch[i];
                    let bit = (c.lfsr ^ c.lfsr >> 1) & 1;
                    c.lfsr = c.lfsr >> 1 | bit << 14;
                    c.noise_left = period;
                }
                if self.ch[i].wave_left == 0 {
                    let steps = self.ch[i].on && !self.ch[i].dda;
                    if steps {
                        let c = &mut self.ch[i];
                        c.address = (c.address + 1) & 31;
                    }
                    self.ch[i].wave_left = self.wave_period(i);
                }
            }
            let level = self.mix();
            if level != self.level {
                self.level = level;
                emit(done, level.0, level.1);
            }
        }
    }
}
