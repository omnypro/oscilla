//! What a player can show of a machine: the state of each voice and the
//! recent output of each voice. Everything here reads the sound unit and
//! changes nothing that the sound depends on.

use crate::apu::{DUTY, TRIANGLE};
use crate::scope::{MAX_VOICES, SCOPE_HZ};
use crate::state::*;
use oscilla_nsf::chips;
use std::sync::OnceLock;

/// What kind of sound a voice makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceKind {
    Tone = 0,
    Noise = 1,
    Sample = 2,
    Fm = 3,
    Drum = 4,
}

/// One voice as a display shows it. The voices of a file are the 2A03's five
/// channels and the channels of the expansion chips that the file asks for, in
/// the order of the channel mask.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    pub name: &'static str,
    /// The bit of the channel mask that mutes this voice.
    pub mask_bit: u32,
    pub kind: VoiceKind,
    /// The voice makes sound now.
    pub active: bool,
    /// The channel is enabled and has a length, a frequency or a key on, as its
    /// chip defines them.
    pub key_on: bool,
    /// The pitch that the period register names, 0 for noise and samples.
    pub frequency_hz: f32,
    /// 0 to 1: the volume or envelope level over its largest value, 0 while the
    /// channel is muted.
    pub volume: f32,
    /// 0 to 1 while the volume comes from an envelope generator, or -1.
    pub envelope: f32,
    /// The volume to each side. The machine is mono, so both are the volume.
    pub left: f32,
    pub right: f32,
    /// The duty, a waveform's number, the patch or the noise mode, or -1.
    pub instrument: i32,
    /// The VRC7's envelope stage: 0 attack, 1 decay, 2 sustain, 3 release. 0
    /// for the other chips.
    pub phase: i32,
    /// Something besides the period register moves the pitch: the FDS's
    /// modulation unit or the VRC7's vibrato.
    pub pitch_modulated: bool,
}

/// For each bit of the channel mask, the chip that the voice belongs to (0 for
/// the 2A03) and the voice's name.
const VOICES: [(u8, &str); MAX_VOICES] = [
    (0, "Pulse 1"),
    (0, "Pulse 2"),
    (0, "Triangle"),
    (0, "Noise"),
    (0, "DMC"),
    (chips::VRC6, "VRC6 Pulse 1"),
    (chips::VRC6, "VRC6 Pulse 2"),
    (chips::VRC6, "VRC6 Saw"),
    (chips::S5B, "5B 1"),
    (chips::S5B, "5B 2"),
    (chips::S5B, "5B 3"),
    (chips::N163, "N163 1"),
    (chips::N163, "N163 2"),
    (chips::N163, "N163 3"),
    (chips::N163, "N163 4"),
    (chips::N163, "N163 5"),
    (chips::N163, "N163 6"),
    (chips::N163, "N163 7"),
    (chips::N163, "N163 8"),
    (chips::FDS, "FDS"),
    (chips::MMC5, "MMC5 Pulse 1"),
    (chips::MMC5, "MMC5 Pulse 2"),
    (chips::MMC5, "MMC5 PCM"),
    (chips::VRC7, "VRC7 1"),
    (chips::VRC7, "VRC7 2"),
    (chips::VRC7, "VRC7 3"),
    (chips::VRC7, "VRC7 4"),
    (chips::VRC7, "VRC7 5"),
    (chips::VRC7, "VRC7 6"),
    (chips::VT02, "VT02 Pulse 3"),
    (chips::VT02, "VT02 Pulse 4"),
    (chips::VT02, "VT02 Triangle 2"),
    (chips::VT02, "VT02 Noise 2"),
    (chips::VT02, "VT02 PCM"),
];

/// The mask bits of the voices that a file with the chip byte `chip_bits` has.
fn bits(chip_bits: u8) -> impl Iterator<Item = usize> {
    (0..MAX_VOICES).filter(move |&b| VOICES[b].0 == 0 || chip_bits & VOICES[b].0 != 0)
}

/// The number of voices that a file with the chip byte `chip_bits` has.
pub(crate) fn count(chip_bits: u8) -> usize {
    bits(chip_bits).count()
}

/// One step of a level of 0 to 15 in a scope sample.
const STEP: i32 = 2048;

fn clamp(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// The 5B's 32 levels as scope samples: the volume curve, 30,720 at the top.
fn s5b_amplitudes() -> &'static [i32; 32] {
    static TABLE: OnceLock<[i32; 32]> = OnceLock::new();
    TABLE.get_or_init(|| crate::s5b::amplitudes().map(|a| (a * 30_720.0) as i32))
}

/// A number that is the same for equal runs of samples.
fn wave_id(samples: impl Iterator<Item = u8>) -> i32 {
    let mut id = 17u32;
    for s in samples {
        id = id.wrapping_mul(31).wrapping_add(u32::from(s));
    }
    (id & 0x7FFF_FFFF) as i32
}

// A tone channel's sample is its output about the middle of its range at its
// own level, so that a tone swings about 0 and a silent channel is 0. A
// sample channel's is its level from 0.

fn pulse_sample(p: &Pulse, channel: usize, on: bool) -> i16 {
    if !on || p.silent(channel) {
        return 0;
    }
    let v = i32::from(p.volume_out()) * STEP;
    clamp(
        if DUTY[usize::from(p.duty & 3)][usize::from(p.pos & 7)] != 0 {
            v
        } else {
            -v
        },
    )
}

fn triangle_sample(t: &Triangle, on: bool) -> i16 {
    if !on || t.gated() {
        return 0;
    }
    clamp((i32::from(TRIANGLE[usize::from(t.pos & 31)]) * 2 - 15) * STEP)
}

fn noise_sample(n: &Noise, on: bool) -> i16 {
    if !on || n.len == 0 {
        return 0;
    }
    let level = if n.constant { n.volume } else { n.env.decay };
    let v = i32::from(level) * STEP;
    clamp(if n.lfsr & 1 == 0 { v } else { -v })
}

fn s5b_level(s: &S5b, c: usize) -> u8 {
    let volume = s.regs[8 + c];
    if volume & 16 != 0 {
        s.env_level & 31
    } else if volume & 15 == 0 {
        0
    } else {
        (volume & 15) * 2 + 1
    }
}

impl Apu {
    /// Writes the sample that each voice of a file with the chip byte
    /// `chip_bits` puts out now, in the order of the channel mask.
    pub(crate) fn scope_values(&self, chip_bits: u8, out: &mut [i16; MAX_VOICES]) {
        let first = self.vt.first_on();
        out[0] = pulse_sample(&self.pulse[0], 0, first);
        out[1] = pulse_sample(&self.pulse[1], 1, first);
        out[2] = triangle_sample(&self.tri, first);
        out[3] = noise_sample(&self.noise, first);
        out[4] = if first {
            clamp(i32::from(self.dmc.level) * 256)
        } else {
            0
        };
        let mut n = 5;
        if chip_bits & chips::VRC6 != 0 {
            for p in &self.vrc6.pulse {
                out[n] = if p.enabled {
                    let v = i32::from(p.volume) * STEP;
                    clamp(if p.mode || p.counter <= p.duty { v } else { -v })
                } else {
                    0
                };
                n += 1;
            }
            let s = &self.vrc6.saw;
            out[n] = if s.enabled {
                // The ramp runs from 0 to six times the rate, in eighths.
                let peak = (6 * i32::from(s.rate)).min(255) >> 3;
                clamp((i32::from(s.acc >> 3) * 2 - peak) * 1024)
            } else {
                0
            };
            n += 1;
        }
        if chip_bits & chips::S5B != 0 {
            let s = &self.s5b;
            let mixer = s.regs[7];
            let amplitudes = s5b_amplitudes();
            for c in 0..3 {
                let tone = s.tone[c].out || mixer >> c & 1 != 0;
                let noise = s.noise_out || mixer >> (c + 3) & 1 != 0;
                let amp = amplitudes[usize::from(s5b_level(s, c))];
                out[n] = clamp(if tone && noise { amp } else { -amp });
                n += 1;
            }
        }
        if chip_bits & chips::N163 != 0 {
            let lowest = 8 - self.n163.channels();
            for c in 0..8 {
                out[n] = if c >= lowest {
                    clamp(self.n163.last[usize::from(c)] * 256)
                } else {
                    0
                };
                n += 1;
            }
        }
        if chip_bits & chips::FDS != 0 {
            let f = &self.fds;
            out[n] = if f.halted {
                0
            } else {
                let w = i32::from(f.wave[(f.accum >> 18) as usize & 63]);
                let (num, den) = crate::fds::MASTER[usize::from(f.master & 3)];
                clamp((w * 2 - 63) * i32::from(f.vol_out.min(32)) * num / den * 16)
            };
            n += 1;
        }
        if chip_bits & chips::MMC5 != 0 {
            for p in &self.mmc5.pulse {
                out[n] = if p.silent() {
                    0
                } else {
                    let level = if p.constant { p.volume } else { p.env.decay };
                    let v = i32::from(level) * STEP;
                    clamp(
                        if DUTY[usize::from(p.duty & 3)][usize::from(p.pos & 7)] != 0 {
                            v
                        } else {
                            -v
                        },
                    )
                };
                n += 1;
            }
            out[n] = clamp(i32::from(self.mmc5.pcm) * 128);
            n += 1;
        }
        if chip_bits & chips::VRC7 != 0 {
            for c in 0..6 {
                out[n] = if self.vrc7.reset {
                    0
                } else {
                    clamp(self.vrc7.out[c] * 8)
                };
                n += 1;
            }
        }
        if chip_bits & chips::VT02 != 0 {
            let second = self.vt.second_on();
            out[n] = pulse_sample(&self.pulse2[0], 0, second);
            out[n + 1] = pulse_sample(&self.pulse2[1], 1, second);
            out[n + 2] = triangle_sample(&self.tri2, second);
            out[n + 3] = noise_sample(&self.noise2, second);
            out[n + 4] = if second {
                clamp(i32::from(self.vt.pcm) * 128)
            } else {
                0
            };
        }
    }
}

/// A voice with nothing set but its name, its mask bit and its kind.
fn blank(bit: usize, kind: VoiceKind) -> Voice {
    Voice {
        name: VOICES[bit].1,
        mask_bit: bit as u32,
        kind,
        active: false,
        key_on: false,
        frequency_hz: 0.0,
        volume: 0.0,
        envelope: -1.0,
        left: 0.0,
        right: 0.0,
        instrument: -1,
        phase: 0,
        pitch_modulated: false,
    }
}

/// A 2A03 pulse channel, or the VT02's pulse 3 or 4 with `channel` 0 or 1.
/// `on` is false while the VT02's switch keeps the channel's generator out of
/// the output.
fn pulse_voice(v: &mut Voice, p: &Pulse, channel: usize, on: bool, cpu: f64) {
    v.key_on = p.len > 0;
    if on && !p.silent(channel) {
        v.volume = f32::from(p.volume_out()) / 15.0;
    }
    v.envelope = if p.constant {
        -1.0
    } else {
        f32::from(p.env.decay) / 15.0
    };
    v.instrument = i32::from(p.duty);
    v.frequency_hz = (cpu / (16.0 * (f64::from(p.period) + 1.0))) as f32;
}

fn triangle_voice(v: &mut Voice, t: &Triangle, on: bool, cpu: f64) {
    v.key_on = !t.gated();
    if on && v.key_on {
        v.volume = 1.0;
    }
    v.frequency_hz = (cpu / (32.0 * (f64::from(t.period) + 1.0))) as f32;
}

fn noise_voice(v: &mut Voice, n: &Noise, on: bool) {
    v.key_on = n.len > 0;
    let level = if n.constant { n.volume } else { n.env.decay };
    if on && n.len > 0 {
        v.volume = f32::from(level) / 15.0;
    }
    v.envelope = if n.constant {
        -1.0
    } else {
        f32::from(n.env.decay) / 15.0
    };
    v.instrument = i32::from(n.mode);
}

impl Machine {
    /// Adds the scope's samples up to cycle `until`. The sound unit calls it
    /// before each event that can change a channel and at the end of each
    /// catch-up, so that a sample holds the channels' state at its moment.
    #[inline]
    pub(crate) fn scope_fill(&mut self, until: u64) {
        if self.scope.next() < until {
            self.scope_catch_up(until);
        }
    }

    // Out of line, so that the sound unit's event loop stays small.
    #[inline(never)]
    fn scope_catch_up(&mut self, until: u64) {
        if self.scope.stale {
            self.apu.scope_values(self.chips, self.scope.hold());
        }
        self.scope.run(until);
    }

    /// The number of voices that the file's chips have.
    pub fn voice_count(&self) -> usize {
        count(self.chips)
    }

    /// What voice `index` is doing, or `None` for an index past the last.
    pub fn voice(&self, index: usize) -> Option<Voice> {
        let bit = bits(self.chips).nth(index)?;
        let cpu = self.clock.0 as f64 / self.clock.1.max(1) as f64;
        let a = &self.apu;
        let first = a.vt.first_on();
        let second = a.vt.second_on();
        let mut v = blank(bit, VoiceKind::Tone);
        match bit {
            0 | 1 => pulse_voice(&mut v, &a.pulse[bit], bit, first, cpu),
            2 => triangle_voice(&mut v, &a.tri, first, cpu),
            3 => {
                v.kind = VoiceKind::Noise;
                noise_voice(&mut v, &a.noise, first);
            }
            4 => {
                // With the VT02's raw PCM on, the bytes go to that voice.
                let d = &a.dmc;
                let delta = !a.vt.raw_pcm();
                v.kind = VoiceKind::Sample;
                v.key_on = delta && d.remaining > 0;
                if first && delta && (!d.silent || d.remaining > 0) {
                    v.active = true;
                    v.volume = f32::from(d.level) / 127.0;
                }
            }
            5 | 6 => {
                let x = &a.vrc6;
                let p = &x.pulse[bit - 5];
                v.key_on = p.enabled;
                if p.enabled && !x.halted {
                    v.volume = f32::from(p.volume) / 15.0;
                }
                v.instrument = if p.mode { 8 } else { i32::from(p.duty) };
                let period = f64::from(p.period >> x.shift);
                v.frequency_hz = (cpu / (16.0 * (period + 1.0))) as f32;
            }
            7 => {
                let x = &a.vrc6;
                let s = &x.saw;
                v.key_on = s.enabled;
                if s.enabled && !x.halted {
                    // Rates above 42 overflow the accumulator.
                    v.volume = f32::from(s.rate.min(42)) / 42.0;
                }
                let period = f64::from(s.period >> x.shift);
                v.frequency_hz = (cpu / (14.0 * (period + 1.0))) as f32;
            }
            8..=10 => {
                let c = bit - 8;
                let s = &a.s5b;
                let tone = s.regs[7] >> c & 1 == 0;
                let noise = s.regs[7] >> (c + 3) & 1 == 0;
                let register = s.regs[8 + c];
                let env = register & 16 != 0;
                let amplitudes = crate::s5b::amplitudes();
                if !tone && noise {
                    v.kind = VoiceKind::Noise;
                }
                v.key_on = (env || register & 15 != 0) && (tone || noise || env);
                if v.key_on {
                    v.volume = amplitudes[usize::from(s5b_level(s, c))] as f32;
                }
                if env {
                    v.envelope = amplitudes[usize::from(s.env_level & 31)] as f32;
                }
                if tone {
                    v.frequency_hz = (cpu / (32.0 * s.tone_period(c) as f64)) as f32;
                }
            }
            11..=18 => {
                let c = bit - 11;
                let n = &a.n163;
                let channels = n.channels();
                if c >= usize::from(8 - channels) {
                    let base = 0x40 + 8 * c;
                    let ram = &n.ram;
                    let freq = u32::from(ram[base])
                        | u32::from(ram[base + 2]) << 8
                        | u32::from(ram[base + 4] & 3) << 16;
                    let length = 256 - u32::from(ram[base + 4] & 0xFC);
                    let start = ram[base + 6];
                    v.key_on = freq != 0;
                    if v.key_on {
                        v.volume = f32::from(ram[base + 7] & 15) / 15.0;
                    }
                    v.frequency_hz = (cpu * f64::from(freq)
                        / (15.0 * f64::from(channels) * f64::from(length) * 65536.0))
                        as f32;
                    v.instrument = wave_id((0..length).map(|i| {
                        let index = usize::from((i as u8).wrapping_add(start));
                        ram[index >> 1] >> ((index & 1) * 4) & 15
                    }));
                }
            }
            19 => {
                let f = &a.fds;
                let gain = f32::from(f.vol_gain.min(32)) / 32.0;
                let (num, den) = crate::fds::MASTER[usize::from(f.master & 3)];
                v.key_on = !f.halted && f.freq != 0;
                if v.key_on {
                    v.volume = gain * num as f32 / den as f32;
                }
                if !f.vol_off {
                    v.envelope = gain;
                }
                v.instrument = wave_id(f.wave.iter().copied());
                v.frequency_hz = (cpu * f64::from(f.freq) / 4_194_304.0) as f32;
                v.pitch_modulated =
                    v.key_on && f.mod_gain != 0 && (!f.mod_halted || f.mod_counter != 0);
            }
            20 | 21 => {
                let p = &a.mmc5.pulse[bit - 20];
                v.key_on = p.len > 0;
                let level = if p.constant { p.volume } else { p.env.decay };
                if v.key_on {
                    v.volume = f32::from(level) / 15.0;
                }
                if !p.constant {
                    v.envelope = f32::from(p.env.decay) / 15.0;
                }
                v.instrument = i32::from(p.duty);
                v.frequency_hz = (cpu / (16.0 * (f64::from(p.period) + 1.0))) as f32;
            }
            22 => {
                v.kind = VoiceKind::Sample;
                v.key_on = a.mmc5.pcm != 0;
                v.active = v.key_on;
                v.volume = f32::from(a.mmc5.pcm) / 255.0;
            }
            23..=28 => {
                let c = bit - 23;
                let o = &a.vrc7;
                v.kind = VoiceKind::Fm;
                if !o.reset {
                    let (steps, stage, vibrato) = o.carrier(c);
                    let db = |x: f32| 10f32.powf(-x / 20.0);
                    v.key_on = o.key[c];
                    v.envelope = if steps >= 127 {
                        0.0
                    } else {
                        db(steps as f32 * 0.375)
                    };
                    v.volume = v.envelope * db(f32::from(o.instrument[c] & 15) * 3.0);
                    v.instrument = i32::from(o.instrument[c] >> 4);
                    v.phase = stage;
                    v.pitch_modulated = vibrato;
                    let fnum =
                        u32::from(o.frequency_low[c]) | u32::from(o.frequency_high[c] & 1) << 8;
                    let block = u32::from(o.frequency_high[c] >> 1 & 7);
                    v.frequency_hz =
                        (f64::from(fnum << block) * crate::vrc7::SAMPLE_HZ / 524_288.0) as f32;
                }
            }
            29 | 30 => pulse_voice(&mut v, &a.pulse2[bit - 29], bit - 29, second, cpu),
            31 => triangle_voice(&mut v, &a.tri2, second, cpu),
            32 => {
                v.kind = VoiceKind::Noise;
                noise_voice(&mut v, &a.noise2, second);
            }
            _ => {
                v.kind = VoiceKind::Sample;
                v.key_on = a.vt.raw_pcm() && a.dmc.remaining > 0;
                if second && a.vt.pcm != 0 {
                    v.active = true;
                    v.volume = f32::from(a.vt.pcm) / 255.0;
                }
            }
        }
        if v.kind != VoiceKind::Sample {
            v.active = v.volume > if v.kind == VoiceKind::Fm { 0.005 } else { 0.0 };
        }
        v.left = v.volume;
        v.right = v.volume;
        Some(v)
    }

    /// Fills `out` with the last `out.len()` frames that voice `index` put out,
    /// at the machine's output rate, as floats within -1 to 1, oldest first. A
    /// tone or noise voice swings about 0 at its own level and is 0 while it is
    /// silent. A sample voice is its level from 0. The channel mask is not in
    /// it, so a muted voice still shows. The machine keeps 4,096 samples of
    /// 44,100 Hz (93 ms), and frames that reach further back are 0. Returns
    /// false, and leaves `out` alone, for an index past the last voice.
    pub fn scope(&self, index: usize, out: &mut [f32]) -> bool {
        if index >= self.scope.voices() {
            return false;
        }
        let ratio = SCOPE_HZ as f64 / f64::from(self.out.rate());
        let n = out.len();
        for (k, o) in out.iter_mut().enumerate() {
            let back = (n - 1 - k) as f64 * ratio;
            let i = back as usize;
            let frac = back - i as f64;
            let a = f64::from(self.scope.get(index, i));
            let b = f64::from(self.scope.get(index, i + 1));
            *o = ((a + (b - a) * frac) / 32768.0) as f32;
        }
        true
    }
}
