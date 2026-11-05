//! The machine: the chips of one VGM file, the command stream that drives
//! them and the resampler that turns their output into the caller's rate.
//!
//! Time is counted in units of 1/(44100 * 4096) s. A VGM wait is a whole
//! number of 4096-unit samples, and a chip event lands on the unit nearest
//! below its own clock cycle, which is within 6 ns of the exact time.

use std::sync::Arc;

use oscilla_resample::{BLEP_SHIFT, OUT_RING, Resampler};
use oscilla_vgm::{Command, SN76489, Vgm};

use crate::psg::Psg;

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
    out: Resampler,
    last_event: u64,
    mul: i64,
    shift: u32,
}

impl Machine {
    pub fn new(vgm: &Vgm, output_rate: i32) -> Result<Machine, MachineError> {
        if !(8000..=192_000).contains(&output_rate) {
            return Err(MachineError::Rate);
        }
        let psg = vgm.chip(SN76489).map(|c| PsgSlot {
            chip: Psg::new(vgm.psg_feedback, vgm.psg_width, vgm.psg_flags),
            clock: u64::from(c.clock),
            ticks: 0,
        });
        if psg.is_none() {
            return Err(MachineError::NoChip);
        }
        let m = i32::from(vgm.volume_modifier);
        let m = if m == -63 { -64 } else { m };
        let (q, r) = (m.div_euclid(32), m.rem_euclid(32));
        Ok(Machine {
            vgm: Arc::new(vgm.clone()),
            pos: vgm.data_at(),
            wait_left: 0,
            now: 0,
            finished: false,
            loops: 0,
            waited_since_loop: false,
            psg,
            out: Resampler::with_ratio(output_rate, UNIT, 1),
            last_event: 0,
            mul: GAIN[r as usize],
            shift: (BLEP_SHIFT as i32 + 16 - q) as u32,
        })
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

    pub fn set_psg_mask(&mut self, mask: u8) {
        if let Some(p) = self.psg.as_mut() {
            p.chip.set_mask(mask);
        }
        self.sync();
    }

    /// The output level of the chips together.
    fn level(&self) -> (i32, i32) {
        self.psg.as_ref().map_or((0, 0), |p| p.chip.level())
    }

    /// Sets the resampler to the chips' level at the current time.
    fn sync(&mut self) {
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
        if let Some(p) = self.psg.as_mut() {
            let per_tick = 16 * u128::from(UNIT);
            let want = (u128::from(to) * u128::from(p.clock) / per_tick) as u64;
            let (base, clock) = (p.ticks, u128::from(p.clock));
            let mut events: Vec<(u64, i32, i32)> = Vec::new();
            p.chip.run(want - base, |t, l, r| {
                let at = (u128::from(base + t) * per_tick / clock) as u64;
                events.push((at, l, r));
            });
            p.ticks = want;
            for (at, l, r) in events {
                let at = at.max(self.last_event);
                self.out.step(at, l, r);
                self.last_event = at;
            }
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
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            self.produce(chunk);
            let got = self.out.take_s16(
                Some(&mut out[2 * at..2 * (at + chunk)]),
                chunk,
                self.mul,
                self.shift,
            );
            at += got;
        }
    }

    /// Fills `out` with interleaved left and right floats.
    pub fn render_f32(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut at = 0;
        let gain =
            self.mul as f64 / 65536.0 * (BLEP_SHIFT as f64 + 16.0 - f64::from(self.shift)).exp2();
        let scale = gain / f64::from(1u32 << 15) / f64::from(1u32 << BLEP_SHIFT);
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            self.produce(chunk);
            let got = self
                .out
                .take_f32(&mut out[2 * at..2 * (at + chunk)], chunk, scale);
            at += got;
        }
    }
}
