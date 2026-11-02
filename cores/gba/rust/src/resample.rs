//! Rate conversion by band-limited steps. The hardware holds each native
//! sample until the next one, so its output is a staircase in time. Every
//! stair becomes a band-limited step at its exact cycle: a windowed-sinc step
//! with its cutoff at 0.45 of the output rate. The step is stored as
//! per-sample differences and integrated on the way out. The method works for any
//! native rate, and the rate may change mid-stream with SOUNDBIAS. The kernel
//! is fixed point and each phase sums to exactly 1 << BLEP_SHIFT, so the
//! running integral never drifts.

use crate::state::*;
use std::sync::OnceLock;

const GBA_PI: f64 = std::f64::consts::PI;

fn windowed_sinc(t: f64, fc: f64) -> f64 {
    let x = 2.0 * fc * t;
    let s = if x.abs() < 1e-12 {
        1.0
    } else {
        (GBA_PI * x).sin() / (GBA_PI * x)
    };
    let u = (t + BLEP_HALF as f64) / (2.0 * BLEP_HALF as f64); // 0..1 across the window
    if u <= 0.0 || u >= 1.0 {
        return 0.0;
    }
    let w = 0.42 - 0.5 * (2.0 * GBA_PI * u).cos() + 0.08 * (4.0 * GBA_PI * u).cos();
    2.0 * fc * s * w
}

/// The integral of the kernel from a to b, by Simpson's rule.
fn kernel_area(a: f64, b: f64, fc: f64) -> f64 {
    const N: i32 = 64;
    let h = (b - a) / f64::from(N);
    let mut sum = windowed_sinc(a, fc) + windowed_sinc(b, fc);
    for i in 1..N {
        sum += windowed_sinc(a + f64::from(i) * h, fc) * if i & 1 != 0 { 4.0 } else { 2.0 };
    }
    sum * h / 3.0
}

/// The step kernel, one row of BLEP_TAPS per phase. It depends on nothing a
/// machine owns, so every machine shares one copy. The values are fixed in
/// `tables.rs` so that the output does not depend on the platform's libm.
pub(crate) fn kernel() -> &'static [i64] {
    &crate::tables::KERNEL
}

/// The kernel computed with the platform's libm. `tables::KERNEL` was made
/// from this on Apple silicon, and a test compares the two.
#[doc(hidden)]
pub fn reference_kernel() -> &'static [i64] {
    static KERNEL: OnceLock<Vec<i64>> = OnceLock::new();
    KERNEL.get_or_init(|| {
        let fc = 0.45;
        let mut k = vec![0i64; BLEP_PHASES * BLEP_TAPS];
        for p in 0..BLEP_PHASES {
            let f = p as f64 / BLEP_PHASES as f64;
            let mut d = [0.0f64; BLEP_TAPS];
            let mut total = 0.0;
            for (j, dj) in d.iter_mut().enumerate() {
                let mut hi = (j as f64 - BLEP_HALF as f64 + 1.0) - f;
                let mut lo = hi - 1.0;
                if lo < -(BLEP_HALF as f64) {
                    lo = -(BLEP_HALF as f64);
                }
                if hi > BLEP_HALF as f64 {
                    hi = BLEP_HALF as f64;
                }
                *dj = if hi > lo {
                    kernel_area(lo, hi, fc)
                } else {
                    0.0
                };
                total += *dj;
            }
            let row = &mut k[p * BLEP_TAPS..(p + 1) * BLEP_TAPS];
            let mut sum = 0i64;
            let mut biggest = 0usize;
            for j in 0..BLEP_TAPS {
                row[j] = (d[j] / total * f64::from(1u32 << BLEP_SHIFT)).round() as i64;
                sum += row[j];
                if row[j].abs() > row[biggest].abs() {
                    biggest = j;
                }
            }
            row[biggest] += (1i64 << BLEP_SHIFT) - sum;
        }
        k
    })
}

impl Resampler {
    pub(crate) fn new(rate: i32) -> Resampler {
        Resampler {
            rate,
            buf: vec![0; 2 * OUT_RING],
            integ: [0; 2],
            read_pos: 0,
            avail_pos: 0,
            level: [0; 2],
        }
    }

    fn reset(&mut self) {
        self.buf.fill(0);
        self.integ = [0; 2];
        self.read_pos = 0;
        self.avail_pos = 0;
        self.level = [0; 2];
    }

    /// Output frame n sits at (n - BLEP_HALF) / rate seconds after cycle 0:
    /// the kernel reaches BLEP_HALF frames back from each step, and this keeps
    /// every step's first tap at frame 1 or later. The result is a fixed
    /// latency of BLEP_HALF frames. Positions are in frames scaled by 2^24.
    fn position(&self, cycle: u64) -> u64 {
        cycle * self.rate as u64 + ((BLEP_HALF as u64) << 24)
    }

    fn advance_avail(&mut self, pos: u64) {
        let idx = pos >> 24;
        let done = (idx + 1).saturating_sub(BLEP_HALF as u64);
        if done > self.avail_pos {
            self.avail_pos = done;
        }
    }

    fn step(&mut self, cycle: u64, left: i32, right: i32) {
        let pos = self.position(cycle);
        let dl = left - self.level[0];
        let dr = right - self.level[1];
        if dl != 0 || dr != 0 {
            let idx = pos >> 24;
            let phase = ((pos >> 16) & (BLEP_PHASES as u64 - 1)) as usize;
            let k = &kernel()[phase * BLEP_TAPS..(phase + 1) * BLEP_TAPS];
            let base = idx + 1 - BLEP_HALF as u64;
            for (j, kj) in k.iter().enumerate() {
                let n = ((base + j as u64) & (OUT_RING as u64 - 1)) as usize;
                self.buf[n] += i64::from(dl) * kj;
                self.buf[OUT_RING + n] += i64::from(dr) * kj;
            }
            self.level = [left, right];
        }
        self.advance_avail(pos);
    }

    fn next_value(&mut self, ch: usize) -> i64 {
        let n = ch * OUT_RING + (self.read_pos & (OUT_RING as u64 - 1)) as usize;
        self.integ[ch] += self.buf[n];
        self.buf[n] = 0;
        self.integ[ch]
    }
}

impl Machine {
    pub(crate) fn out_reset(&mut self) {
        self.out.reset();
    }

    pub(crate) fn out_step(&mut self, cycle: u64, left: i32, right: i32) {
        if left != self.out.level[0] || right != self.out.level[1] {
            self.last_change = cycle;
        }
        self.out.step(cycle, left, right);
    }

    pub(crate) fn out_finalize(&mut self, cycle: u64) {
        let pos = self.out.position(cycle);
        self.out.advance_avail(pos);
    }

    pub(crate) fn out_available(&self) -> usize {
        (self.out.avail_pos - self.out.read_pos) as usize
    }

    /// Takes up to `frames` frames as 16-bit stereo. With no destination the
    /// frames are discarded.
    pub(crate) fn out_take_s16(&mut self, mut dst: Option<&mut [i16]>, frames: usize) -> usize {
        let frames = frames.min(self.out_available());
        for i in 0..frames {
            for ch in 0..2usize {
                // One level unit is 48 int16 steps: the 10-bit range fills
                // three quarters of int16 and leaves headroom.
                let mut v = (self.out.next_value(ch) * 3) >> (BLEP_SHIFT - 4);
                v = v.clamp(-32768, 32767);
                if let Some(d) = dst.as_deref_mut() {
                    d[2 * i + ch] = v as i16;
                }
            }
            self.out.read_pos += 1;
        }
        frames
    }

    pub(crate) fn out_take_f32(&mut self, dst: &mut [f32], frames: usize) -> usize {
        let frames = frames.min(self.out_available());
        // The same three quarters of full scale as the 16-bit output.
        let scale = 0.75 / (512.0 * f64::from(1u32 << BLEP_SHIFT));
        for i in 0..frames {
            for ch in 0..2usize {
                let v = self.out.next_value(ch) as f64 * scale;
                dst[2 * i + ch] = v as f32;
            }
            self.out.read_pos += 1;
        }
        frames
    }
}
