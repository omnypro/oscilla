//! The machine's side of rate conversion. The resampler itself is the
//! `oscilla-resample` crate. The hardware holds each native sample until the
//! next one, and every stair goes in as a band-limited step at its exact cycle.

use crate::state::*;
use oscilla_resample::BLEP_SHIFT;

impl Machine {
    pub(crate) fn out_reset(&mut self) {
        self.out.reset();
    }

    pub(crate) fn out_step(&mut self, cycle: u64, left: i32, right: i32) {
        if [left, right] != self.out.level() {
            self.last_change = cycle;
        }
        self.out.step(cycle, left, right);
    }

    pub(crate) fn out_finalize(&mut self, cycle: u64) {
        self.out.finalize(cycle);
    }

    pub(crate) fn out_available(&self) -> usize {
        self.out.available()
    }

    /// Takes up to `frames` frames as 16-bit stereo. With no destination the
    /// frames are discarded.
    pub(crate) fn out_take_s16(&mut self, dst: Option<&mut [i16]>, frames: usize) -> usize {
        // One level unit is 48 int16 steps: the 10-bit range fills three
        // quarters of int16 and leaves headroom.
        self.out.take_s16(dst, frames, 3, BLEP_SHIFT - 4)
    }

    pub(crate) fn out_take_f32(&mut self, dst: &mut [f32], frames: usize) -> usize {
        // The same three quarters of full scale as the 16-bit output.
        let scale = 0.75 / (512.0 * f64::from(1u32 << BLEP_SHIFT));
        self.out.take_f32(dst, frames, scale)
    }
}
