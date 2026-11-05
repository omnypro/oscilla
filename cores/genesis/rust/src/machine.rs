//! The machine: the chips of one VGM file, the command stream that drives
//! them and the resampler that turns their output into the caller's rate.
//!
//! Time is counted in units of 1/(44100 * 4096) s. A VGM wait is a whole
//! number of 4096-unit samples, and a chip event lands on the unit nearest
//! below its own clock cycle, which is within 6 ns of the exact time.

use std::sync::Arc;

use oscilla_resample::{BLEP_SHIFT, OUT_RING, Resampler};
use oscilla_vgm::{Banks, Command, SN76489, Vgm, YM2612};

use crate::psg::Psg;
use crate::ym2612::Ym2612;

/// Time units in one second.
pub const UNIT: u64 = oscilla_vgm::SAMPLE_RATE as u64 * 4096;
/// Time units in one sample of the stream.
const PER_SAMPLE: u64 = 4096;

/// The volume modifier's 2^(k/32) in 16.16 fixed point, k = 0 to 31.
const GAIN: [i64; 32] = [
    65536, 66971, 68438, 69936, 71468, 73032, 74632, 76266, 77936, 79642, 81386, 83169, 84990,
    86851, 88752, 90696, 92682, 94711, 96785, 98905, 101070, 103283, 105545, 107856, 110218,
    112631, 115098, 117618, 120194, 122825, 125515, 128263,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineError {
    /// The output rate is outside 8000 to 192000 Hz.
    Rate,
    /// The file drives no chip that this core has.
    NoChip,
}

impl std::fmt::Display for MachineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MachineError::Rate => write!(f, "The output rate must be 8000 to 192000 Hz."),
            MachineError::NoChip => write!(f, "The file uses no chip that this core plays."),
        }
    }
}

impl std::error::Error for MachineError {}

/// The size of a YM2612 channel's 9-bit value in the output. A full-scale
/// channel peaks at 0.258 of full scale, as libgme renders one.
const FM_GAIN: i32 = 33;

/// The SN76489's level against the chip's own scale, in 64ths. Fitted to
/// renders of one and three full-volume tones.
const PSG_GAIN: i32 = 55;

/// The default cutoff of the output high-pass. The documents give no value for
/// the console's output coupling.
const DEFAULT_HIGHPASS_HZ: f64 = 20.0;

fn psg_scaled((l, r): (i32, i32)) -> (i32, i32) {
    ((l * PSG_GAIN) >> 6, (r * PSG_GAIN) >> 6)
}

#[derive(Clone)]
struct FmSlot {
    chip: Ym2612,
    clock: u64,
    /// Samples of the chip (input clock / 144) run so far.
    samples: u64,
    level: (i32, i32),
}

#[derive(Clone)]
struct PsgSlot {
    chip: Psg,
    clock: u64,
    /// Ticks of the chip's own clock (input clock / 16) run so far.
    ticks: u64,
}

/// Where the stream stands when a machine is not playing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndState {
    /// The stream is running.
    Playing,
    /// The stream ended without a loop.
    Finished,
}

#[derive(Clone)]
pub struct Machine {
    vgm: Arc<Vgm>,
    pos: usize,
    /// Time units left of the wait that the stream is in.
    wait_left: u64,
    /// The time to which the chips have run.
    now: u64,
    finished: bool,
    loops: u32,
    waited_since_loop: bool,
    psg: Option<PsgSlot>,
    fm: Option<FmSlot>,
    banks: Arc<Banks>,
    /// The read position in the PCM data bank for the YM2612's DAC.
    pcm_at: usize,
    out: Resampler,
    last_event: u64,
    /// The PSG level that the resampler has been told.
    psg_seen: (i32, i32),
    mul: i64,
    shift: u32,
    hp_hz: f64,
    hp_a: f64,
    hp_dc: [f64; 2],
}

impl Machine {
    pub fn new(vgm: &Vgm, output_rate: i32) -> Result<Machine, MachineError> {
        Machine::from_shared(Arc::new(vgm.clone()), output_rate)
    }

    fn from_shared(vgm: Arc<Vgm>, output_rate: i32) -> Result<Machine, MachineError> {
        if !(8000..=192_000).contains(&output_rate) {
            return Err(MachineError::Rate);
        }
        let psg = vgm.chip(SN76489).map(|c| PsgSlot {
            chip: Psg::new(vgm.psg_feedback, vgm.psg_width, vgm.psg_flags),
            clock: u64::from(c.clock),
            ticks: 0,
        });
        let fm = vgm.chip(YM2612).map(|c| FmSlot {
            chip: Ym2612::new(),
            clock: u64::from(c.clock),
            samples: 0,
            level: (0, 0),
        });
        if psg.is_none() && fm.is_none() {
            return Err(MachineError::NoChip);
        }
        let m = i32::from(vgm.volume_modifier);
        let m = if m == -63 { -64 } else { m };
        let (q, r) = (m.div_euclid(32), m.rem_euclid(32));
        let mut machine = Machine {
            pos: vgm.data_at(),
            wait_left: 0,
            now: 0,
            finished: false,
            loops: 0,
            waited_since_loop: false,
            psg,
            fm,
            banks: Arc::new(vgm.banks()),
            vgm,
            pcm_at: 0,
            out: Resampler::with_ratio(output_rate, UNIT, 1),
            last_event: 0,
            psg_seen: (0, 0),
            mul: GAIN[r as usize],
            shift: (BLEP_SHIFT as i32 + 16 - q) as u32,
            hp_hz: 0.0,
            hp_a: 0.0,
            hp_dc: [0.0; 2],
        };
        machine.set_highpass_hz(DEFAULT_HIGHPASS_HZ);
        Ok(machine)
    }

    pub fn output_rate(&self) -> i32 {
        self.out.rate()
    }

    /// Passes through the loop point so far.
    pub fn loops(&self) -> u32 {
        self.loops
    }

    pub fn end_state(&self) -> EndState {
        if self.finished {
            EndState::Finished
        } else {
            EndState::Playing
        }
    }

    /// Stream time so far in samples of 44,100 Hz.
    pub fn samples(&self) -> u64 {
        self.now / PER_SAMPLE
    }

    pub fn file(&self) -> &Vgm {
        &self.vgm
    }

    /// Starts the song again with the same file, rate, channel mask and high-pass.
    pub fn reset(&mut self) {
        let mask = self.channel_mask();
        if let Ok(mut m) = Machine::from_shared(Arc::clone(&self.vgm), self.out.rate()) {
            m.set_channel_mask(mask);
            m.set_highpass_hz(self.hp_hz);
            *self = m;
        }
    }

    /// Returns to the state of `saved`, a copy taken from this machine or one
    /// of the same file and output rate.
    pub fn restore(&mut self, saved: &Machine) -> bool {
        if !Arc::ptr_eq(&self.vgm, &saved.vgm) || self.out.rate() != saved.out.rate() {
            return false;
        }
        *self = saved.clone();
        true
    }

    /// Chooses the channels that reach the mix: bits 0 to 5 are the YM2612's
    /// channels 1 to 6 and bits 6 to 9 the SN76489's channels 0 to 3.
    pub fn set_channel_mask(&mut self, mask: u64) {
        self.set_fm_mask((mask & 0x3F) as u8);
        self.set_psg_mask(((mask >> 6) & 0x0F) as u8);
    }

    pub fn channel_mask(&self) -> u64 {
        let fm = self.fm.as_ref().map_or(0x3F, |f| u64::from(f.chip.mask()));
        let psg = self.psg.as_ref().map_or(0x0F, |p| u64::from(p.chip.mask()));
        fm | psg << 6
    }

    pub fn set_psg_mask(&mut self, mask: u8) {
        if let Some(p) = self.psg.as_mut() {
            p.chip.set_mask(mask);
        }
        self.sync();
    }

    pub fn set_fm_mask(&mut self, mask: u8) {
        if let Some(f) = self.fm.as_mut() {
            f.chip.set_mask(mask);
        }
    }

    /// The output level of the chips together as the resampler knows it.
    fn level(&self) -> (i32, i32) {
        let (fl, fr) = self.fm.as_ref().map_or((0, 0), |f| f.level);
        (self.psg_seen.0 + fl, self.psg_seen.1 + fr)
    }

    /// Sets the resampler to the chips' level at the current time.
    fn sync(&mut self) {
        if let Some(p) = self.psg.as_ref() {
            self.psg_seen = psg_scaled(p.chip.level());
        }
        let (l, r) = self.level();
        let t = self.now.max(self.last_event);
        self.out.step(t, l, r);
        self.last_event = t;
    }

    fn advance(&mut self, to: u64) {
        if self.finished {
            self.now = to;
            return;
        }
        let mut psg_events: Vec<(u64, i32, i32)> = Vec::new();
        if let Some(p) = self.psg.as_mut() {
            let per_tick = 16 * u128::from(UNIT);
            let want = (u128::from(to) * u128::from(p.clock) / per_tick) as u64;
            let (base, clock) = (p.ticks, u128::from(p.clock));
            p.chip.run(want - base, |t, l, r| {
                let at = (u128::from(base + t) * per_tick / clock) as u64;
                let (l, r) = psg_scaled((l, r));
                psg_events.push((at, l, r));
            });
            p.ticks = want;
        }
        let mut fm_events: Vec<(u64, i32, i32)> = Vec::new();
        if let Some(f) = self.fm.as_mut() {
            let per_sample = 144 * u128::from(UNIT);
            let want = (u128::from(to) * u128::from(f.clock) / per_sample) as u64;
            while f.samples < want {
                f.samples += 1;
                let (l, r) = f.chip.sample();
                let at = (u128::from(f.samples) * per_sample / u128::from(f.clock)) as u64;
                fm_events.push((at, l * FM_GAIN, r * FM_GAIN));
            }
        }
        let (mut pi, mut fi) = (0, 0);
        let mut fm_level = self.fm.as_ref().map_or((0, 0), |f| f.level);
        let mut psg_now = self.psg_seen;
        while pi < psg_events.len() || fi < fm_events.len() {
            let take_psg = match (psg_events.get(pi), fm_events.get(fi)) {
                (Some(p), Some(f)) => p.0 <= f.0,
                (Some(_), None) => true,
                _ => false,
            };
            let at;
            if take_psg {
                let e = psg_events[pi];
                pi += 1;
                at = e.0;
                psg_now = (e.1, e.2);
            } else {
                let e = fm_events[fi];
                fi += 1;
                at = e.0;
                fm_level = (e.1, e.2);
            }
            let at = at.max(self.last_event);
            self.out
                .step(at, psg_now.0 + fm_level.0, psg_now.1 + fm_level.1);
            self.last_event = at;
        }
        self.psg_seen = psg_now;
        if let Some(f) = self.fm.as_mut() {
            f.level = fm_level;
        }
        self.now = to;
    }

    fn finish(&mut self) {
        self.finished = true;
        let t = self.now.max(self.last_event);
        self.out.step(t, 0, 0);
        self.last_event = t;
    }

    /// Reads commands until one starts a wait or the stream ends.
    fn read_commands(&mut self) {
        let vgm = Arc::clone(&self.vgm);
        loop {
            let mut cursor = vgm.commands_from(self.pos);
            let command = cursor.read();
            match command {
                Command::End => {
                    match vgm.loop_at() {
                        Some(at) if self.waited_since_loop || self.loops == 0 => {
                            self.pos = at;
                            self.loops += 1;
                            self.waited_since_loop = false;
                        }
                        _ => self.finish(),
                    }
                    if self.finished {
                        return;
                    }
                    continue;
                }
                Command::Wait(n) => {
                    self.pos = cursor.position();
                    if n > 0 {
                        self.wait_left = u64::from(n) * PER_SAMPLE;
                        self.waited_since_loop = true;
                        return;
                    }
                }
                Command::Psg {
                    second: false,
                    value,
                } => {
                    self.pos = cursor.position();
                    if let Some(p) = self.psg.as_mut() {
                        p.chip.write(value);
                    }
                    self.sync();
                }
                Command::PsgStereo {
                    second: false,
                    value,
                } => {
                    self.pos = cursor.position();
                    if let Some(p) = self.psg.as_mut() {
                        p.chip.set_stereo(value);
                    }
                    self.sync();
                }
                Command::Ym2612 {
                    second: false,
                    port,
                    register,
                    value,
                } => {
                    self.pos = cursor.position();
                    if let Some(f) = self.fm.as_mut() {
                        f.chip.write(port, register, value);
                    }
                }
                Command::Ym2612Dac { wait } => {
                    self.pos = cursor.position();
                    let byte = self.banks.bank(0).get(self.pcm_at).copied().unwrap_or(0x80);
                    self.pcm_at += 1;
                    if let Some(f) = self.fm.as_mut() {
                        f.chip.write(0, 0x2A, byte);
                    }
                    if wait > 0 {
                        self.wait_left = u64::from(wait) * PER_SAMPLE;
                        self.waited_since_loop = true;
                        return;
                    }
                }
                Command::Seek(at) => {
                    self.pos = cursor.position();
                    self.pcm_at = at as usize;
                }
                _ => self.pos = cursor.position(),
            }
        }
    }

    fn run_until(&mut self, target: u64) {
        while self.now < target {
            if self.finished {
                self.advance(target);
                return;
            }
            if self.wait_left == 0 {
                self.read_commands();
                continue;
            }
            let step = self.wait_left.min(target - self.now);
            self.advance(self.now + step);
            self.wait_left -= step;
        }
    }

    /// Runs the machine until the resampler holds `frames` more frames.
    fn produce(&mut self, frames: usize) {
        let rate = self.out.rate() as u64;
        while self.out.available() < frames {
            let missing = (frames - self.out.available()) as u64;
            let target = self.now + (missing * UNIT).div_ceil(rate).max(1);
            self.run_until(target);
            let t = self.now;
            self.out.finalize(t);
        }
    }

    /// Fills `out` with interleaved left and right samples.
    pub fn render_s16(&mut self, out: &mut [i16]) {
        let frames = out.len() / 2;
        let mut at = 0;
        let mut tmp = [0f32; OUT_RING / 2];
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            let got = self.take_f32(&mut tmp[..2 * chunk], chunk);
            for (o, v) in out[2 * at..2 * (at + got)].iter_mut().zip(&tmp) {
                *o = (v * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            }
            at += got;
        }
    }

    /// Fills `out` with interleaved left and right floats.
    pub fn render_f32(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut at = 0;
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            at += self.take_f32(&mut out[2 * at..2 * (at + chunk)], chunk);
        }
    }

    fn take_f32(&mut self, out: &mut [f32], chunk: usize) -> usize {
        let gain =
            self.mul as f64 / 65536.0 * (BLEP_SHIFT as f64 + 16.0 - f64::from(self.shift)).exp2();
        let scale = gain / f64::from(1u32 << 15) / f64::from(1u32 << BLEP_SHIFT);
        self.produce(chunk);
        let got = self.out.take_f32(out, chunk, scale);
        if self.hp_a > 0.0 {
            for frame in out[..2 * got].chunks_exact_mut(2) {
                for (v, dc) in frame.iter_mut().zip(self.hp_dc.iter_mut()) {
                    let x = f64::from(*v);
                    *dc += (x - *dc) * self.hp_a;
                    *v = (x - *dc) as f32;
                }
            }
        }
        got
    }

    /// Sets the cutoff of the high-pass that stands for the console's output
    /// coupling and removes the DC offset that the DAC can leave. 0 turns it
    /// off. Returns false, and changes nothing, for a value outside 0 to 1000 Hz.
    pub fn set_highpass_hz(&mut self, hz: f64) -> bool {
        if !(0.0..=1000.0).contains(&hz) {
            return false;
        }
        self.hp_hz = hz;
        self.hp_a = 2.0 * std::f64::consts::PI * hz / f64::from(self.out.rate());
        true
    }

    pub fn highpass_hz(&self) -> f64 {
        self.hp_hz
    }
}
