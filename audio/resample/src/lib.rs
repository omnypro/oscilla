//! Rate conversion by band-limited steps. A sound chip holds each output level
//! until the next change, so its output is a staircase in time. Every stair
//! becomes a band-limited step at its exact cycle: a windowed-sinc step with
//! its cutoff at 0.45 of the output rate. The step is stored as per-sample
//! differences and integrated on the way out. The method works for any
//! native rate, and the rate may change mid-stream. The kernel is fixed point
//! and each phase sums to exactly 1 << BLEP_SHIFT, so the running integral
//! never drifts.
//!
//! The chip's clock is either a power of two, 2^`clock_shift` cycles per second
//! and no more than 2^24, or a ratio `num / den` cycles per second. The ratio
//! is exact: a position is `cycle * rate * den / num` in integers.

mod tables;

use std::sync::OnceLock;

pub const BLEP_HALF: usize = 8;
pub const BLEP_TAPS: usize = 2 * BLEP_HALF + 1;
pub const BLEP_PHASES: usize = 256;
pub const BLEP_SHIFT: u32 = 15;
/// Frames in each output ring. A caller takes at most a quarter of it at a time.
pub const OUT_RING: usize = 16384;

/// How cycles turn into positions.
#[derive(Clone, Copy)]
enum Clock {
    Shift(u32),
    Ratio { num: u64, den: u64 },
}

#[derive(Clone)]
pub struct Resampler {
    rate: i32,
    clock: Clock,
    /// Two rings of OUT_RING frames, left then right.
    buf: Vec<i64>,
    integ: [i64; 2],
    read_pos: u64,
    avail_pos: u64,
    level: [i32; 2],
}

impl Resampler {
    /// A resampler for a chip that runs at 2^`clock_shift` cycles per second,
    /// producing `rate` frames per second.
    pub fn new(rate: i32, clock_shift: u32) -> Resampler {
        assert!(clock_shift <= 24);
        Resampler::with(rate, Clock::Shift(clock_shift))
    }

    /// A resampler for a chip that runs at `num / den` cycles per second.
    /// `num` and `den` are at least 1, and `den` is at most 2^16.
    pub fn with_ratio(rate: i32, num: u64, den: u64) -> Resampler {
        assert!(num >= 1 && (1..=1 << 16).contains(&den));
        Resampler::with(rate, Clock::Ratio { num, den })
    }

    fn with(rate: i32, clock: Clock) -> Resampler {
        Resampler {
            rate,
            clock,
            buf: vec![0; 2 * OUT_RING],
            integ: [0; 2],
            read_pos: 0,
            avail_pos: 0,
            level: [0; 2],
        }
    }

    pub fn rate(&self) -> i32 {
        self.rate
    }

    /// Bytes of heap that a copy of this resampler holds.
    pub fn heap_bytes(&self) -> usize {
        self.buf.len() * 8
    }

    pub fn reset(&mut self) {
        self.buf.fill(0);
        self.integ = [0; 2];
        self.read_pos = 0;
        self.avail_pos = 0;
        self.level = [0; 2];
    }

    /// The level that the last step set, left then right.
    pub fn level(&self) -> [i32; 2] {
        self.level
    }

    /// Output frame n sits at (n - BLEP_HALF) / rate seconds after cycle 0:
    /// the kernel reaches BLEP_HALF frames back from each step, and this keeps
    /// every step's first tap at frame 1 or later. The result is a fixed
    /// latency of BLEP_HALF frames. Positions are in frames scaled by 2^24.
    fn position(&self, cycle: u64) -> u128 {
        let scaled = match self.clock {
            Clock::Shift(shift) => (u128::from(cycle) * self.rate as u128) << (24 - shift),
            Clock::Ratio { num, den } => {
                ((u128::from(cycle) * self.rate as u128 * u128::from(den)) << 24) / u128::from(num)
            }
        };
        scaled + ((BLEP_HALF as u128) << 24)
    }

    fn advance_avail(&mut self, pos: u128) {
        let idx = (pos >> 24) as u64;
        let done = (idx + 1).saturating_sub(BLEP_HALF as u64);
        if done > self.avail_pos {
            self.avail_pos = done;
        }
    }

    /// Sets the output level at `cycle`. A step is added when it differs from
    /// the previous level.
    pub fn step(&mut self, cycle: u64, left: i32, right: i32) {
        let pos = self.position(cycle);
        let dl = left - self.level[0];
        let dr = right - self.level[1];
        if dl != 0 || dr != 0 {
            let idx = (pos >> 24) as u64;
            let phase = ((pos >> 16) & (BLEP_PHASES as u128 - 1)) as usize;
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

    /// Marks every frame that `cycle` completes as available.
    pub fn finalize(&mut self, cycle: u64) {
        let pos = self.position(cycle);
        self.advance_avail(pos);
    }

    /// Frames that can be taken now.
    pub fn available(&self) -> usize {
        (self.avail_pos - self.read_pos) as usize
    }

    fn next_value(&mut self, ch: usize) -> i64 {
        let n = ch * OUT_RING + (self.read_pos & (OUT_RING as u64 - 1)) as usize;
        self.integ[ch] += self.buf[n];
        self.buf[n] = 0;
        self.integ[ch]
    }

    /// Takes up to `frames` frames as 16-bit stereo, each value scaled by
    /// `mul >> shift` from the integrated level (which carries BLEP_SHIFT
    /// fractional bits). With no destination the frames are discarded.
    pub fn take_s16(
        &mut self,
        mut dst: Option<&mut [i16]>,
        frames: usize,
        mul: i64,
        shift: u32,
    ) -> usize {
        let frames = frames.min(self.available());
        for i in 0..frames {
            for ch in 0..2usize {
                let v = ((self.next_value(ch) * mul) >> shift).clamp(-32768, 32767);
                if let Some(d) = dst.as_deref_mut() {
                    d[2 * i + ch] = v as i16;
                }
            }
            self.read_pos += 1;
        }
        frames
    }

    /// Takes up to `frames` frames as 32-bit floats, each value multiplied by
    /// `scale` from the integrated level.
    pub fn take_f32(&mut self, dst: &mut [f32], frames: usize, scale: f64) -> usize {
        let frames = frames.min(self.available());
        for i in 0..frames {
            for ch in 0..2usize {
                let v = self.next_value(ch) as f64 * scale;
                dst[2 * i + ch] = v as f32;
            }
            self.read_pos += 1;
        }
        frames
    }
}

/// The step kernel, one row of BLEP_TAPS per phase. The values are fixed in
/// `tables.rs` so that the output does not depend on the platform's libm.
pub fn kernel() -> &'static [i64] {
    &tables::KERNEL
}

const PI: f64 = std::f64::consts::PI;

fn windowed_sinc(t: f64, fc: f64) -> f64 {
    let x = 2.0 * fc * t;
    let s = if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    };
    let u = (t + BLEP_HALF as f64) / (2.0 * BLEP_HALF as f64); // 0..1 across the window
    if u <= 0.0 || u >= 1.0 {
        return 0.0;
    }
    let w = 0.42 - 0.5 * (2.0 * PI * u).cos() + 0.08 * (4.0 * PI * u).cos();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_very_late_cycle_keeps_working() {
        let mut r = Resampler::new(192000, 22);
        let late = 1u64 << 60;
        r.step(late, 100, -100);
        r.finalize(late + 4_194_304);
        assert!(r.available() > 0);
    }

    #[test]
    fn a_ratio_of_a_power_of_two_gives_the_same_positions() {
        let mut a = Resampler::new(48000, 22);
        let mut b = Resampler::with_ratio(48000, 1 << 22, 1);
        for (i, cycle) in [0u64, 1, 77, 4_194_304, 123_456_789, 1 << 40]
            .into_iter()
            .enumerate()
        {
            let level = 100 * (i as i32 + 1);
            a.step(cycle, level, -level);
            b.step(cycle, level, -level);
            assert_eq!(a.position(cycle), b.position(cycle));
        }
        assert_eq!(a.buf, b.buf);
    }

    #[test]
    fn a_ratio_clock_places_a_step_at_its_time() {
        // 19687500 / 11 cycles per second, the clock of an NTSC console.
        let (num, den) = (19_687_500u64, 11u64);
        let mut r = Resampler::with_ratio(48000, num, den);
        r.step(num / den / 4, 1000, 1000);
        r.finalize(num / den / 2);
        let mut out = vec![0i16; 2 * 20_000];
        assert_eq!(r.take_s16(Some(&mut out), 20_000, 1, BLEP_SHIFT), 20_000);
        assert_eq!(out[2 * 11_000], 0, "before the step");
        assert_eq!(out[2 * 13_000], 1000, "after the step");
    }

    #[test]
    fn a_step_shows_after_its_cycle() {
        let mut r = Resampler::new(48000, 22);
        r.step(4_194_304 / 2, 1000, 1000);
        r.finalize(4_194_304);
        let mut out = vec![0i64; 2 * 48000];
        let n = r.take_s16(None, 0, 1, 0);
        assert_eq!(n, 0);
        let mut s16 = vec![0i16; 2 * 40000];
        let n = r.take_s16(Some(&mut s16), 40000, 1, BLEP_SHIFT);
        assert_eq!(n, 40000);
        out.clear();
        assert_eq!(s16[2 * 1000], 0, "before the step");
        assert_eq!(s16[2 * 30000], 1000, "after the step");
    }
}
