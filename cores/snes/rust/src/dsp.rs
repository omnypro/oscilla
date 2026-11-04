//! The S-DSP: eight BRR voices with envelopes, pitch modulation and noise, the
//! mixer and the echo unit. One call of `sample` makes one stereo sample, which
//! the hardware makes every 32 cycles of the SPC700.

use crate::gauss::GAUSS;

/// Samples between two steps of a rate, indexed by the 5-bit rate. Rate 0
/// never steps.
const PERIODS: [u32; 32] = [
    0, 2048, 1536, 1280, 1024, 768, 640, 512, 384, 320, 256, 192, 160, 128, 96, 80, 64, 48, 40,
    32, 24, 20, 16, 12, 10, 8, 6, 5, 4, 3, 2, 1,
];

/// The least common multiple of the periods. The shared counter wraps here.
const COUNTER_WRAP: u32 = 30720;

/// Samples between a key-on and the first output of a voice.
const KEY_ON_DELAY: u8 = 5;

pub const VOICES: usize = 8;

/// Bit of the channel mask that controls the echo output.
pub const ECHO_BIT: u32 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    Attack,
    Decay,
    Sustain,
    #[default]
    Release,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    env: i32,
    mode: Mode,
    /// The address of the block being played.
    block: u16,
    /// Where the loop begins, read from the table at key-on and at each loop.
    loop_at: u16,
    /// The flags of the block being played.
    flags: u8,
    counter: u32,
    /// Three samples of the block before, then the 16 of this one.
    window: [i32; 19],
    old: i32,
    older: i32,
    delay: u8,
    /// The enveloped sample of the last step, before the volumes.
    out: i32,
}

#[derive(Clone)]
pub struct Dsp {
    pub regs: [u8; 128],
    voices: [Voice; VOICES],
    endx: u8,
    /// The key-on flags that wait for the next poll.
    kon: u8,
    counter: u32,
    noise: i32,
    echo_index: u32,
    echo_length: u32,
    fir: [[i32; 8]; 2],
    fir_at: usize,
    samples: u64,
    pub mask: u64,
    levels: [i32; VOICES],
}

fn clamp16(v: i32) -> i32 {
    v.clamp(-32768, 32767)
}

fn wrap16(v: i32) -> i32 {
    i32::from(v as i16)
}

impl Dsp {
    pub fn new(regs: [u8; 128]) -> Dsp {
        let mut d = Dsp {
            regs,
            voices: [Voice::default(); VOICES],
            endx: regs[0x7C],
            kon: regs[0x4C],
            counter: 0,
            noise: 0x4000,
            echo_index: 0,
            echo_length: 0,
            fir: [[0; 8]; 2],
            fir_at: 0,
            samples: 0,
            mask: u64::MAX,
            levels: [0; VOICES],
        };
        d.regs[0x7C] = d.endx;
        d
    }

    pub fn read(&self, reg: u8) -> u8 {
        self.regs[usize::from(reg & 0x7F)]
    }

    pub fn write(&mut self, reg: u8, value: u8) {
        if reg >= 0x80 {
            return;
        }
        match reg {
            0x4C => self.kon = value,
            0x7C => {
                self.endx = 0;
                self.regs[0x7C] = 0;
                return;
            }
            _ => {}
        }
        self.regs[usize::from(reg)] = value;
    }

    /// The signed high byte of each voice's last output, before the volumes.
    pub fn levels(&self) -> [i32; VOICES] {
        self.levels
    }

    /// The next sample of the noise generator, as a signed 15-bit value.
    fn noise_out(&self) -> i32 {
        (self.noise << 17) >> 17
    }

    fn fires(&self, rate: u8) -> bool {
        let period = PERIODS[usize::from(rate & 31)];
        period != 0 && self.counter.is_multiple_of(period)
    }

    fn key_on(&mut self, v: usize, ram: &[u8; 65536]) {
        let dir = usize::from(self.regs[0x5D]) << 8;
        let entry = (dir + usize::from(self.regs[v * 16 + 4]) * 4) & 0xFFFF;
        let word = |at: usize| u16::from_le_bytes([ram[at & 0xFFFF], ram[(at + 1) & 0xFFFF]]);
        let voice = &mut self.voices[v];
        voice.block = word(entry);
        voice.loop_at = word(entry + 2);
        voice.env = 0;
        voice.mode = Mode::Attack;
        voice.counter = 0;
        voice.window = [0; 19];
        voice.old = 0;
        voice.older = 0;
        voice.flags = 0;
        voice.delay = KEY_ON_DELAY;
        self.endx &= !(1 << v);
    }

    /// Decodes the block at the voice's address into the window.
    fn decode(&mut self, v: usize, ram: &[u8; 65536]) {
        let voice = &mut self.voices[v];
        let at = usize::from(voice.block);
        let header = ram[at];
        let range = i32::from(header >> 4);
        let filter = (header >> 2) & 3;
        voice.flags = header & 3;
        if voice.flags & 1 != 0 {
            self.endx |= 1 << v;
        }
        voice.window.copy_within(16..19, 0);
        for n in 0..16usize {
            let byte = ram[(at + 1 + n / 2) & 0xFFFF];
            let nibble = if n % 2 == 0 { byte >> 4 } else { byte & 15 };
            let mut s = i32::from((nibble ^ 8) as i8 - 8);
            let mut shift = range;
            if shift > 12 {
                s >>= 3;
                shift = 12;
            }
            s = (s << shift) >> 1;
            let (old, older) = (voice.old, voice.older);
            s += match filter {
                0 => 0,
                1 => old + ((-old) >> 4),
                2 => old * 2 + ((-old * 3) >> 5) - older + (older >> 4),
                _ => old * 2 + ((-old * 13) >> 6) - older + ((older * 3) >> 4),
            };
            // Past 16 bits the sum saturates to values that the 15-bit wrap
            // below leaves at 0x3FFF and 0. Between 15 and 16 bits it wraps.
            s = if s > 0x7FFF {
                0x3FFF
            } else if s < -0x8000 {
                0
            } else if s >= 0x4000 {
                s - 0x8000
            } else if s < -0x4000 {
                s + 0x8000
            } else {
                s
            };
            voice.window[3 + n] = s;
            voice.older = old;
            voice.old = s;
        }
    }

    /// Moves a voice on to the next block, or back to the loop.
    fn next_block(&mut self, v: usize, ram: &[u8; 65536]) {
        let voice = &mut self.voices[v];
        if voice.flags & 1 != 0 {
            voice.block = voice.loop_at;
            if voice.flags == 1 {
                voice.mode = Mode::Release;
                voice.env = 0;
            }
        } else {
            voice.block = voice.block.wrapping_add(9);
        }
        self.decode(v, ram);
    }

    fn step_envelope(&mut self, v: usize) {
        let adsr1 = self.regs[v * 16 + 5];
        let adsr2 = self.regs[v * 16 + 6];
        let gain = self.regs[v * 16 + 7];
        let voice = self.voices[v];
        let mut env = voice.env;
        let mut mode = voice.mode;
        if mode == Mode::Release {
            env = (env - 8).max(0);
        } else if adsr1 & 0x80 != 0 {
            match mode {
                Mode::Attack => {
                    let rate = (adsr1 & 15) * 2 + 1;
                    if self.fires(rate) {
                        env += if rate == 31 { 1024 } else { 32 };
                    }
                }
                Mode::Decay => {
                    if self.fires(((adsr1 >> 4) & 7) * 2 + 16) {
                        env -= ((env - 1) >> 8) + 1;
                    }
                }
                _ => {
                    if self.fires(adsr2 & 31) {
                        env -= ((env - 1) >> 8) + 1;
                    }
                }
            }
        } else if gain & 0x80 == 0 {
            env = i32::from(gain & 0x7F) * 16;
        } else if self.fires(gain & 31) {
            match (gain >> 5) & 3 {
                0 => env -= 32,
                1 => env -= ((env - 1) >> 8) + 1,
                2 => env += 32,
                _ => env += if env < 0x600 { 32 } else { 8 },
            }
        }
        env = env.clamp(0, 0x7FF);
        if mode != Mode::Release {
            if mode == Mode::Attack && env >= 0x7E0 {
                mode = Mode::Decay;
            }
            let level = if adsr1 & 0x80 != 0 { adsr2 } else { gain };
            let boundary = (i32::from(level >> 5) + 1) * 0x100;
            if mode == Mode::Decay && env <= boundary {
                mode = Mode::Sustain;
            }
        }
        self.voices[v].env = env;
        self.voices[v].mode = mode;
    }

    /// Makes one stereo sample. `ram` is the sound RAM, which holds the
    /// samples, the sample table and the echo buffer.
    pub fn sample(&mut self, ram: &mut [u8; 65536]) -> (i32, i32) {
        let flg = self.regs[0x6C];
        let koff = self.regs[0x5C];
        let pmon = self.regs[0x2D];
        let non = self.regs[0x3D];
        let eon = self.regs[0x4D];

        if self.samples & 1 == 0 {
            for v in 0..VOICES {
                if koff & (1 << v) != 0 {
                    self.voices[v].mode = Mode::Release;
                }
            }
            for v in 0..VOICES {
                if self.kon & (1 << v) != 0 {
                    self.key_on(v, ram);
                }
            }
            self.kon = 0;
        }
        if flg & 0x80 != 0 {
            for v in &mut self.voices {
                v.mode = Mode::Release;
                v.env = 0;
            }
        }
        if self.fires(flg & 31) {
            let n = self.noise;
            self.noise = ((n >> 1) & 0x3FFF) | (((n ^ (n >> 1)) & 1) << 14);
        }

        let mut main = [0i32; 2];
        let mut echo = [0i32; 2];
        let mut previous = 0;
        for v in 0..VOICES {
            let mut out = 0;
            if self.voices[v].delay > 0 {
                self.voices[v].delay -= 1;
                if self.voices[v].delay == 0 {
                    self.decode(v, ram);
                }
            } else {
                self.step_envelope(v);
                let voice = &self.voices[v];
                let mut step = i32::from(u16::from_le_bytes([
                    self.regs[v * 16 + 2],
                    self.regs[v * 16 + 3] & 0x3F,
                ]));
                if v > 0 && pmon & (1 << v) != 0 {
                    let factor = (previous >> 4) + 0x400;
                    step = (step * factor) >> 10;
                }
                step = step.min(0x3FFF);
                let sample = if non & (1 << v) != 0 {
                    self.noise_out()
                } else {
                    let p = (voice.counter >> 12) as usize;
                    let i = ((voice.counter >> 4) & 0xFF) as usize;
                    let w = &voice.window[p..p + 4];
                    let mut sum = (i32::from(GAUSS[0xFF - i]) * w[0]) >> 10;
                    sum = wrap16(sum + ((i32::from(GAUSS[0x1FF - i]) * w[1]) >> 10));
                    sum = wrap16(sum + ((i32::from(GAUSS[0x100 + i]) * w[2]) >> 10));
                    sum = clamp16(sum + ((i32::from(GAUSS[i]) * w[3]) >> 10));
                    sum >> 1
                };
                out = (sample * voice.env) >> 11;
                let counter = voice.counter + step as u32;
                self.voices[v].counter = counter & 0xFFFF;
                if counter > 0xFFFF {
                    self.next_block(v, ram);
                }
            }
            self.voices[v].out = out;
            previous = out;
            self.regs[v * 16 + 8] = (self.voices[v].env >> 4) as u8;
            self.regs[v * 16 + 9] = (out >> 7) as u8;
            self.levels[v] = out >> 7;
            if self.mask & (1 << v) != 0 {
                let (l, r) = (self.regs[v * 16] as i8, self.regs[v * 16 + 1] as i8);
                main[0] = clamp16(main[0] + ((out * i32::from(l)) >> 6));
                main[1] = clamp16(main[1] + ((out * i32::from(r)) >> 6));
                if eon & (1 << v) != 0 {
                    echo[0] = clamp16(echo[0] + ((out * i32::from(l)) >> 6));
                    echo[1] = clamp16(echo[1] + ((out * i32::from(r)) >> 6));
                }
            }
        }
        self.regs[0x7C] = self.endx;

        let esa = usize::from(self.regs[0x6D]) << 8;
        let at = (esa + self.echo_index as usize * 4) & 0xFFFF;
        let mut fir_out = [0i32; 2];
        for ch in 0..2usize {
            let lo = usize::from(ram[(at + ch * 2) & 0xFFFF]);
            let hi = usize::from(ram[(at + ch * 2 + 1) & 0xFFFF]);
            let input = i32::from((hi << 8 | lo) as u16 as i16) >> 1;
            self.fir[ch][self.fir_at & 7] = input;
            let mut sum = 0;
            for k in 0..8usize {
                let coefficient = i32::from(self.regs[0x0F + 0x10 * k] as i8);
                let tap = self.fir[ch][(self.fir_at + 1 + k) & 7];
                let term = (tap * coefficient) >> 6;
                sum = if k < 7 { wrap16(sum + term) } else { clamp16(sum + term) };
            }
            fir_out[ch] = sum;
        }
        self.fir_at = self.fir_at.wrapping_add(1);

        let efb = i32::from(self.regs[0x0D] as i8);
        if flg & 0x20 == 0 {
            for ch in 0..2usize {
                let value = clamp16(echo[ch] + ((fir_out[ch] * efb) >> 7)) & !1;
                ram[(at + ch * 2) & 0xFFFF] = value as u8;
                ram[(at + ch * 2 + 1) & 0xFFFF] = (value >> 8) as u8;
            }
        }
        if self.echo_index == 0 {
            self.echo_length = u32::from(self.regs[0x7D] & 15) << 9;
        }
        self.echo_index += 1;
        if self.echo_index >= self.echo_length {
            self.echo_index = 0;
        }

        let echo_on = self.mask & (1 << ECHO_BIT) != 0;
        let mut out = [0i32; 2];
        for ch in 0..2usize {
            let master = i32::from(self.regs[0x0C + 0x10 * ch] as i8);
            let volume = i32::from(self.regs[0x2C + 0x10 * ch] as i8);
            let mut sum = (main[ch] * master) >> 7;
            if echo_on {
                sum = clamp16(sum + ((fir_out[ch] * volume) >> 7));
            }
            out[ch] = if flg & 0x40 != 0 { 0 } else { sum };
        }

        self.counter = (self.counter + 1) % COUNTER_WRAP;
        self.samples += 1;
        (out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_noise_generator_has_the_longest_cycle_a_fifteen_bit_register_allows() {
        let step = |n: i32| ((n >> 1) & 0x3FFF) | (((n ^ (n >> 1)) & 1) << 14);
        let mut n = 0x4000;
        let mut count = 0;
        loop {
            n = step(n);
            count += 1;
            if n == 0x4000 {
                break;
            }
            assert!(count < 40_000);
        }
        assert_eq!(count, 32767);
    }

    #[test]
    fn the_gauss_table_sums_to_about_one_in_each_group_of_four() {
        for i in 0..256usize {
            let sum = i32::from(GAUSS[i])
                + i32::from(GAUSS[0xFF - i])
                + i32::from(GAUSS[0x100 + i])
                + i32::from(GAUSS[0x1FF - i]);
            assert!((0x7FF..=0x801).contains(&sum), "{i}: {sum:X}");
        }
    }

    #[test]
    fn the_rate_periods_are_ones_threes_and_fives_times_a_power_of_two() {
        for &p in &PERIODS[1..] {
            let mut q = p;
            while q % 2 == 0 {
                q /= 2;
            }
            assert!([1, 3, 5].contains(&q), "{p}");
            assert_eq!(COUNTER_WRAP % p, 0);
        }
    }
}
