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

/// The samples of each voice's output that the scope keeps: 4,096 of the
/// DSP's 32 kHz, which is 128 ms.
pub const SCOPE_LEN: usize = 4096;

/// The scope's channels: the eight voices, then the left and right echo
/// output.
const SCOPE_CHANNELS: usize = VOICES + 2;

/// The most blocks that a scan of a sample follows before it gives up on
/// finding the end flag.
pub const MAX_SAMPLE_BLOCKS: usize = 4096;

/// A value of `SampleInfo::loop_start_sample` for a loop address that is not
/// on a block of the decoded samples.
pub const NO_LOOP_START: u32 = u32::MAX;

/// What the DSP did to the sound RAM in the last sample: a range of addresses
/// and the kind of access.
pub const DSP_READ: u8 = 8;
pub const DSP_WRITE: u8 = 16;

/// The recent output of each voice, as the listener hears it before the master
/// volume: the voice's output times its two volumes, averaged. The echo output
/// (the filter's output times the echo volume, before it joins the mix) shares
/// the ring as two more channels. It is not part of a copy of the DSP, which
/// starts with an empty one.
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

    fn push(&mut self, samples: [i16; SCOPE_CHANNELS]) {
        let data = self
            .data
            .get_or_insert_with(|| vec![0; SCOPE_CHANNELS * SCOPE_LEN].into_boxed_slice());
        // The channels of one moment sit side by side, so that a sample is one
        // contiguous store.
        data[self.at * SCOPE_CHANNELS..(self.at + 1) * SCOPE_CHANNELS].copy_from_slice(&samples);
        self.at += 1;
        if self.at == SCOPE_LEN {
            self.at = 0;
        }
    }

    /// The sample `back` steps before the newest, 0 for the newest.
    fn get(&self, channel: usize, back: usize) -> i16 {
        match &self.data {
            Some(data) if back < SCOPE_LEN => {
                data[((self.at + SCOPE_LEN - 1 - back) % SCOPE_LEN) * SCOPE_CHANNELS + channel]
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

/// Where a voice is in its sample, and what the sample looks like from there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrrPosition {
    /// The address of the sample table entry: DIR * 256 + SRCN * 4, from the
    /// registers as they are now.
    pub directory_entry: u16,
    /// The address of the block being played.
    pub block: u16,
    /// The loop address that the entry holds now.
    pub loop_address: u16,
    /// The sample of the block that the pitch counter points at, 0 to 15.
    pub sample_in_block: u8,
    /// The SRCN register.
    pub source: u8,
    /// The times the voice jumped back to the loop address since its key-on,
    /// up to 255.
    pub loops: u8,
    /// The end (bit 0) and loop (bit 1) flags of the block being played.
    pub flags: u8,
    /// The filter number of the block being played, 0 to 3.
    pub filter: u8,
    /// The shift amount of the block being played, 0 to 15.
    pub range: u8,
    /// The samples from the loop address to the block with the end flag, 16 for
    /// each block, or 0 when that block lacks the loop flag or none is found.
    pub loop_length_samples: u32,
    /// The samples from the start address to the block with the end flag, or 0
    /// when none is found.
    pub total_samples: u32,
}

/// What `decode_sample` found about a sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SampleInfo {
    pub start_address: u16,
    pub loop_address: u16,
    /// The samples from the start address through the block with the end flag.
    /// When the scan gave up it is the samples that it decoded.
    pub total_samples: u32,
    /// The index of the sample where the loop begins, or `NO_LOOP_START` when
    /// the loop address is not on a block of the decoded samples.
    pub loop_start_sample: u32,
    /// The block with the end flag also has the loop flag.
    pub has_loop: bool,
    /// The scan followed `MAX_SAMPLE_BLOCKS` blocks without finding the end
    /// flag.
    pub truncated: bool,
}

/// What a voice is doing, read from its registers and its envelope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceInfo {
    /// 0 attack, 1 decay, 2 sustain, 3 release.
    pub phase: u8,
    /// The envelope level, 0 to 0x7FF.
    pub envelope: i32,
    /// The source number, the index into the sample directory.
    pub source: u8,
    /// The 14-bit pitch register: 0x1000 plays the sample at 32 kHz.
    pub pitch: u16,
    /// The left and right volume registers.
    pub volume: (i8, i8),
    pub adsr: (u8, u8),
    pub gain: u8,
    pub noise: bool,
    pub pitch_modulated: bool,
    pub echo: bool,
    /// The voice has reached the end of its sample since the last read of the
    /// ENDX register.
    pub ended: bool,
    /// The last output of the voice before the volumes, signed 16-bit.
    pub output: i32,
}
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
    /// The flags of the block being played.
    flags: u8,
    /// The header of the block being played.
    header: u8,
    /// The jumps back to the loop address since the key-on, saturating.
    loops: u8,
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
    scope: Scope,
    /// The ranges of sound RAM that the DSP read or wrote in the last sample.
    pub(crate) accesses: Vec<(u16, u16, u8)>,
    /// Whether to note those ranges: the machine turns it on with the access
    /// map.
    pub(crate) track: bool,
}

fn clamp16(v: i32) -> i32 {
    v.clamp(-32768, 32767)
}

fn wrap16(v: i32) -> i32 {
    i32::from(v as i16)
}

/// Decodes the BRR block at `at` into 16 signed 15-bit samples. `old` and
/// `older` are the two samples before it and the block's last two replace them.
fn brr_block(ram: &[u8; 65536], at: usize, old: &mut i32, older: &mut i32) -> [i32; 16] {
    let header = ram[at];
    let range = i32::from(header >> 4);
    let filter = (header >> 2) & 3;
    let mut out = [0i32; 16];
    for (n, slot) in out.iter_mut().enumerate() {
        let byte = ram[(at + 1 + n / 2) & 0xFFFF];
        let nibble = if n % 2 == 0 { byte >> 4 } else { byte & 15 };
        let mut s = i32::from((nibble ^ 8) as i8 - 8);
        let mut shift = range;
        if shift > 12 {
            s >>= 3;
            shift = 12;
        }
        s = (s << shift) >> 1;
        let (o, oo) = (*old, *older);
        s += match filter {
            0 => 0,
            1 => o + ((-o) >> 4),
            2 => o * 2 + ((-o * 3) >> 5) - oo + (oo >> 4),
            _ => o * 2 + ((-o * 13) >> 6) - oo + ((oo * 3) >> 4),
        };
        // Past 16 bits the sum saturates to values that the 15-bit wrap below
        // leaves at 0x3FFF and 0. Between 15 and 16 bits it wraps.
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
        *slot = s;
        *older = o;
        *old = s;
    }
    out
}

/// The start and loop addresses of the sample table entry at `entry`.
fn directory_entry(ram: &[u8; 65536], entry: usize) -> (u16, u16) {
    let word = |at: usize| u16::from_le_bytes([ram[at & 0xFFFF], ram[(at + 1) & 0xFFFF]]);
    (word(entry), word(entry + 2))
}

/// Follows the blocks from `from` to the first one with the end flag. Returns
/// the number of blocks, that one included, and whether it has the loop flag.
/// None when `MAX_SAMPLE_BLOCKS` blocks pass without an end flag.
fn scan_end(ram: &[u8; 65536], from: u16) -> Option<(u32, bool)> {
    let mut at = from;
    for n in 1..=MAX_SAMPLE_BLOCKS as u32 {
        let header = ram[usize::from(at)];
        if header & 1 != 0 {
            return Some((n, header & 2 != 0));
        }
        at = at.wrapping_add(9);
    }
    None
}

/// Decodes the sample at entry `source` of the table at `dir * 256`, from the
/// start address to the first block with the end flag, as the voices decode it
/// with the two samples before the first block at 0. Writes the first
/// `out.len()` samples to `out` as the 15-bit values that the DSP holds, and
/// returns what it found.
pub fn decode_sample(ram: &[u8; 65536], dir: u8, source: u8, out: &mut [i16]) -> SampleInfo {
    let entry = (usize::from(dir) << 8) + usize::from(source) * 4;
    let (start, loop_address) = directory_entry(ram, entry);
    let mut info = SampleInfo {
        start_address: start,
        loop_address,
        loop_start_sample: NO_LOOP_START,
        ..SampleInfo::default()
    };
    let (mut old, mut older) = (0, 0);
    let mut at = start;
    for n in 0..MAX_SAMPLE_BLOCKS {
        if at == loop_address && info.loop_start_sample == NO_LOOP_START {
            info.loop_start_sample = n as u32 * 16;
        }
        let samples = brr_block(ram, usize::from(at), &mut old, &mut older);
        for (k, s) in samples.iter().enumerate() {
            if let Some(slot) = out.get_mut(n * 16 + k) {
                *slot = *s as i16;
            }
        }
        info.total_samples = (n as u32 + 1) * 16;
        let header = ram[usize::from(at)];
        if header & 1 != 0 {
            info.has_loop = header & 2 != 0;
            return info;
        }
        at = at.wrapping_add(9);
    }
    info.truncated = true;
    info
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
            scope: Scope::new(),
            accesses: Vec::new(),
            track: false,
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

    /// Notes a range of sound RAM that the DSP read or wrote, when the access
    /// map is on.
    fn note(&mut self, first: u16, len: u16, kind: u8) {
        if self.track {
            self.accesses.push((first, len, kind));
        }
    }

    /// The state of a voice.
    pub fn voice_info(&self, v: usize) -> VoiceInfo {
        let v = v % VOICES;
        let r = |i: usize| self.regs[v * 16 + i];
        let voice = &self.voices[v];
        VoiceInfo {
            // With the ADSR bit clear the envelope follows the gain register and
            // has no phases: it counts as sustaining until the key goes off.
            phase: match voice.mode {
                Mode::Release => 3,
                _ if r(5) & 0x80 == 0 => 2,
                Mode::Attack => 0,
                Mode::Decay => 1,
                Mode::Sustain => 2,
            },
            envelope: voice.env,
            source: r(4),
            pitch: u16::from_le_bytes([r(2), r(3) & 0x3F]),
            volume: (r(0) as i8, r(1) as i8),
            adsr: (r(5), r(6)),
            gain: r(7),
            noise: self.regs[0x3D] & (1 << v) != 0,
            pitch_modulated: v > 0 && self.regs[0x2D] & (1 << v) != 0,
            echo: self.regs[0x4D] & (1 << v) != 0,
            ended: self.endx & (1 << v) != 0,
            output: voice.out,
        }
    }

    /// The sample `back` steps before the newest of a voice's scope.
    pub fn scope_sample(&self, voice: usize, back: usize) -> i16 {
        self.scope.get(voice % VOICES, back)
    }

    /// The sample `back` steps before the newest of the echo output's scope,
    /// for the left (0) or right (1) channel.
    pub fn echo_scope_sample(&self, channel: usize, back: usize) -> i16 {
        self.scope.get(VOICES + channel % 2, back)
    }

    /// Where a voice is in its sample. `ram` is the sound RAM.
    pub fn voice_brr(&self, v: usize, ram: &[u8; 65536]) -> BrrPosition {
        let v = v % VOICES;
        let voice = &self.voices[v];
        let source = self.regs[v * 16 + 4];
        let entry = (usize::from(self.regs[0x5D]) << 8) + usize::from(source) * 4;
        let (start, loop_address) = directory_entry(ram, entry);
        BrrPosition {
            directory_entry: entry as u16,
            block: voice.block,
            loop_address,
            sample_in_block: ((voice.counter >> 12) & 15) as u8,
            source,
            loops: voice.loops,
            flags: voice.flags,
            filter: (voice.header >> 2) & 3,
            range: voice.header >> 4,
            loop_length_samples: match scan_end(ram, loop_address) {
                Some((blocks, true)) => blocks * 16,
                _ => 0,
            },
            total_samples: scan_end(ram, start).map_or(0, |(blocks, _)| blocks * 16),
        }
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
        self.note(entry as u16, 4, DSP_READ);
        let voice = &mut self.voices[v];
        voice.block = word(entry);
        voice.env = 0;
        voice.mode = Mode::Attack;
        voice.counter = 0;
        voice.window = [0; 19];
        voice.old = 0;
        voice.older = 0;
        voice.flags = 0;
        voice.header = 0;
        voice.loops = 0;
        voice.delay = KEY_ON_DELAY;
        self.endx &= !(1 << v);
    }

    /// Decodes the block at the voice's address into the window.
    fn decode(&mut self, v: usize, ram: &[u8; 65536]) {
        let at = usize::from(self.voices[v].block);
        self.note(at as u16, 9, DSP_READ);
        let voice = &mut self.voices[v];
        let header = ram[at];
        voice.header = header;
        voice.flags = header & 3;
        if voice.flags & 1 != 0 {
            self.endx |= 1 << v;
        }
        voice.window.copy_within(16..19, 0);
        let samples = brr_block(ram, at, &mut voice.old, &mut voice.older);
        voice.window[3..19].copy_from_slice(&samples);
    }

    /// Moves a voice on to the next block, or back to the loop.
    fn next_block(&mut self, v: usize, ram: &[u8; 65536]) {
        let dir = usize::from(self.regs[0x5D]) << 8;
        let entry = (dir + usize::from(self.regs[v * 16 + 4]) * 4 + 2) & 0xFFFF;
        let loop_at = u16::from_le_bytes([ram[entry], ram[(entry + 1) & 0xFFFF]]);
        self.note(entry as u16, 2, DSP_READ);
        let voice = &mut self.voices[v];
        if voice.flags & 1 != 0 {
            voice.loops = voice.loops.saturating_add(1);
            voice.block = loop_at;
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
        self.accesses.clear();
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
        let mut heard = [0i16; SCOPE_CHANNELS];
        // The voice number indexes the registers, the voices and the scope.
        #[allow(clippy::needless_range_loop)]
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
            let (l, r) = (self.regs[v * 16] as i8, self.regs[v * 16 + 1] as i8);
            let (ml, mr) = ((out * i32::from(l)) >> 6, (out * i32::from(r)) >> 6);
            heard[v] = ((ml + mr) >> 1).clamp(-32768, 32767) as i16;
            if self.mask & (1 << v) != 0 {
                main[0] = clamp16(main[0] + ml);
                main[1] = clamp16(main[1] + mr);
                if eon & (1 << v) != 0 {
                    echo[0] = clamp16(echo[0] + ml);
                    echo[1] = clamp16(echo[1] + mr);
                }
            }
        }
        self.regs[0x7C] = self.endx;

        let esa = usize::from(self.regs[0x6D]) << 8;
        let at = (esa + self.echo_index as usize * 4) & 0xFFFF;
        self.note(at as u16, 4, DSP_READ);
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
            self.note(at as u16, 4, DSP_WRITE);
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
            // The echo output as it joins the mix, whatever the mask and the
            // mute flag say.
            heard[VOICES + ch] = clamp16((fir_out[ch] * volume) >> 7) as i16;
        }
        self.scope.push(heard);

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
