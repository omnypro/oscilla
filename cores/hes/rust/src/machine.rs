//! The machine: the HuC6280 with its memory map, the timer, the interrupt
//! registers, a stand-in for the video chip's vertical sync, and the sound
//! generator, driven by the code of an HES file. The program starts a song
//! when the machine calls its request routine, and its interrupt handlers play
//! it from then on.
//!
//! Time is counted in master clocks of 21.477 MHz. A cycle of the processor
//! is 3 of them at high speed and 12 at low speed, the sound generator ticks
//! every 6 and the timer's prescaler every 3072.

use std::sync::Arc;

use oscilla_hesfile::Hes;
use oscilla_resample::{BLEP_SHIFT, OUT_RING, Resampler};

use crate::cpu::{Bus, Cpu};
use crate::psg::Psg;

/// Master clocks in one second.
pub const UNIT: u64 = 21_477_273;
/// Master clocks in one tick of the sound generator.
const PSG_UNITS: u64 = 6;
/// Master clocks between two counts of the timer: 3 times 1024.
const TIMER_UNITS: u64 = 3 * 1024;
/// A frame of 262 lines of 1365 master clocks, the NTSC line rate.
const FRAME_UNITS: u64 = 262 * 1365;
/// Where in the frame the vertical blanking period starts.
const VBLANK_AT: u64 = 240 * 1365;
/// Where the request routine returns to. Nothing runs there: the machine waits
/// for the next event while the processor stands at this address.
const IDLE: u16 = 0x1FFF;

/// The gain of the output, which is set so that one channel at full level
/// with a full-scale word reaches 0.34 of full scale. The documents give no
/// level against full scale. The value is fitted so that the mean level of 29
/// songs of 4 games equals libgme's render of them (the songs lie within 0.4
/// dB of each other).
const OUTPUT_GAIN: f64 = 0.25 * 1.365 / (31.0 * 65536.0);

/// The default cutoff of the output high-pass.
const DEFAULT_HIGHPASS_HZ: f64 = 20.0;

#[derive(Clone)]
struct Io {
    hes: Arc<Hes>,
    ram: Vec<u8>,
    psg: Psg,
    /// Ticks of the sound generator run so far.
    psg_ticks: u64,
    out: Resampler,
    last_event: u64,
    now: u64,
    timer_reload: u8,
    timer_count: u8,
    timer_on: bool,
    timer_next: u64,
    timer_request: bool,
    /// Bit 0 masks IRQ2, bit 1 IRQ1 and bit 2 the timer.
    irq_disable: u8,
    vdc_select: u8,
    vdc_control: u8,
    vdc_status: u8,
    vblank_next: u64,
}

impl Io {
    fn irq1_line(&self) -> bool {
        self.vdc_status != 0
    }

    /// Runs the sound generator up to the current time.
    fn sync_psg(&mut self) {
        let want = self.now / PSG_UNITS;
        if want <= self.psg_ticks {
            return;
        }
        let base = self.psg_ticks;
        let out = &mut self.out;
        let last = &mut self.last_event;
        self.psg.run(want - base, &mut |t, l, r| {
            let at = ((base + t) * PSG_UNITS).max(*last);
            out.step(at, l, r);
            *last = at;
        });
        self.psg_ticks = want;
    }

    /// Tells the output the sound generator's level at the current time.
    fn push_level(&mut self) {
        let (l, r) = self.psg.level();
        let at = self.now.max(self.last_event);
        self.out.step(at, l, r);
        self.last_event = at;
    }

    /// Makes the timer counts and the vertical blanking starts that fall at or
    /// before `to`, in order.
    fn events(&mut self, to: u64) {
        loop {
            let timer = self.timer_on.then_some(self.timer_next);
            let vblank = self.vblank_next;
            let timer_first = timer.is_some_and(|t| t <= vblank);
            let at = if timer_first {
                timer.unwrap_or(vblank)
            } else {
                vblank
            };
            if at > to {
                return;
            }
            if timer_first {
                if self.timer_count == 0 {
                    self.timer_request = true;
                    self.timer_count = self.timer_reload;
                } else {
                    self.timer_count -= 1;
                }
                self.timer_next += TIMER_UNITS;
            } else {
                if self.vdc_control & 8 != 0 {
                    self.vdc_status |= 0x20;
                }
                self.vblank_next += FRAME_UNITS;
            }
        }
    }

    /// The master clock of the next event, if any.
    fn next_event(&self) -> u64 {
        let vblank = self.vblank_next;
        if self.timer_on {
            self.timer_next.min(vblank)
        } else {
            vblank
        }
    }
}

impl Bus for Io {
    fn read(&mut self, physical: u32) -> u8 {
        let a = physical as usize;
        match a {
            0..=0xF_FFFF => self.hes.rom()[a],
            0x1F_0000..=0x1F_7FFF => self.ram[a - 0x1F_0000],
            0x1F_E000..=0x1F_E3FF => {
                if a & 3 == 0 {
                    let status = self.vdc_status;
                    self.vdc_status = 0;
                    status
                } else {
                    0
                }
            }
            0x1F_EC00..=0x1F_EFFF => {
                if a & 1 == 0 {
                    self.timer_count & 0x7F
                } else {
                    u8::from(self.timer_on)
                }
            }
            0x1F_F400..=0x1F_F7FF => match a & 3 {
                2 => self.irq_disable,
                3 => u8::from(self.timer_request) << 2 | u8::from(self.irq1_line()) << 1,
                _ => 0xFF,
            },
            _ => 0xFF,
        }
    }

    fn write(&mut self, physical: u32, value: u8) {
        let a = physical as usize;
        match a {
            0x1F_0000..=0x1F_7FFF => self.ram[a - 0x1F_0000] = value,
            0x1F_E000..=0x1F_E3FF => match a & 3 {
                0 => self.vdc_select = value & 0x1F,
                2 if self.vdc_select == 5 => self.vdc_control = value,
                _ => {}
            },
            0x1F_E800..=0x1F_EBFF => {
                self.sync_psg();
                self.psg.write((a & 15) as u8, value);
                self.push_level();
            }
            0x1F_EC00..=0x1F_EFFF => {
                if a & 1 == 0 {
                    self.timer_reload = value & 0x7F;
                } else {
                    let on = value & 1 != 0;
                    if on && !self.timer_on {
                        self.timer_count = self.timer_reload;
                        self.timer_next = self.now + TIMER_UNITS;
                    }
                    self.timer_on = on;
                }
            }
            0x1F_F400..=0x1F_F7FF => match a & 3 {
                2 => self.irq_disable = value & 7,
                3 => self.timer_request = false,
                _ => {}
            },
            _ => {}
        }
    }

    fn pending_interrupt(&mut self) -> Option<u16> {
        if self.timer_request && self.irq_disable & 4 == 0 {
            Some(0xFFFA)
        } else if self.irq1_line() && self.irq_disable & 2 == 0 {
            Some(0xFFF8)
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct Machine {
    cpu: Cpu,
    io: Io,
    mask: u8,
    hp_hz: f64,
    hp_a: f64,
    hp_dc: [f64; 2],
}

impl Machine {
    /// A machine with the program loaded, started at `song`.
    pub fn new(hes: &Hes, output_rate: i32, song: u8) -> Result<Machine, MachineError> {
        if !(8000..=192_000).contains(&output_rate) {
            return Err(MachineError::Rate);
        }
        let mut cpu = Cpu::new();
        cpu.mpr = hes.mpr;
        let mut io = Io {
            hes: Arc::new(hes.clone()),
            ram: vec![0; 0x8000],
            psg: Psg::new(),
            psg_ticks: 0,
            out: Resampler::with_ratio(output_rate, UNIT, 1),
            last_event: 0,
            now: 0,
            timer_reload: 0,
            timer_count: 0,
            timer_on: false,
            timer_next: 0,
            timer_request: false,
            irq_disable: 0,
            vdc_select: 0,
            vdc_control: 0,
            vdc_status: 0,
            vblank_next: VBLANK_AT,
        };
        // Call the request routine as a subroutine: its return address is the
        // idle address, and the song number is in the accumulator.
        let ret = IDLE - 1;
        let stack = physical(&cpu.mpr, 0x2100);
        io.write(stack | 0xFF, (ret >> 8) as u8);
        io.write(stack | 0xFE, ret as u8);
        cpu.s = 0xFD;
        cpu.a = song;
        cpu.pc = hes.request;
        let mut m = Machine {
            cpu,
            io,
            mask: 0x3F,
            hp_hz: 0.0,
            hp_a: 0.0,
            hp_dc: [0.0; 2],
        };
        m.set_highpass_hz(DEFAULT_HIGHPASS_HZ);
        Ok(m)
    }

    pub fn output_rate(&self) -> i32 {
        self.io.out.rate()
    }

    /// Time run so far in seconds of 44,100 samples, for display.
    pub fn samples(&self) -> u64 {
        self.io.now * 44_100 / UNIT
    }

    /// Chooses the channels that reach the output: bit 0 is channel 1.
    pub fn set_channel_mask(&mut self, mask: u64) {
        self.mask = (mask & 0x3F) as u8;
        self.io.sync_psg();
        self.io.psg.set_mask(self.mask);
        self.io.push_level();
    }

    pub fn channel_mask(&self) -> u64 {
        u64::from(self.mask)
    }

    fn run_until(&mut self, target: u64) {
        while self.io.now < target {
            let idle = self.cpu.pc == IDLE;
            let can_interrupt = self.cpu.p & 0x04 == 0 && self.io.pending_interrupt().is_some();
            if idle && !can_interrupt {
                let to = self.io.next_event().max(self.io.now + 1).min(target);
                self.io.now = to;
                self.io.events(to);
                continue;
            }
            self.io.sync_psg();
            let cycles = self.cpu.step(&mut self.io);
            let per = if self.cpu.high_speed { 3 } else { 12 };
            self.io.now += u64::from(cycles.max(1)) * per;
            let now = self.io.now;
            self.io.events(now);
        }
    }

    /// Runs the machine until the resampler holds `frames` more frames.
    fn produce(&mut self, frames: usize) {
        let rate = self.io.out.rate() as u64;
        while self.io.out.available() < frames {
            let missing = (frames - self.io.out.available()) as u64;
            let target = self.io.now + (missing * UNIT).div_ceil(rate).max(1);
            self.run_until(target);
            self.io.sync_psg();
            let t = self.io.now;
            self.io.out.finalize(t);
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
        let scale = OUTPUT_GAIN / f64::from(1u32 << BLEP_SHIFT);
        self.produce(chunk);
        let got = self.io.out.take_f32(out, chunk, scale);
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

    /// Sets the cutoff of the output high-pass. 0 turns it off. Returns false,
    /// and changes nothing, for a value outside 0 to 1000 Hz.
    pub fn set_highpass_hz(&mut self, hz: f64) -> bool {
        if !(0.0..=1000.0).contains(&hz) {
            return false;
        }
        self.hp_hz = hz;
        self.hp_a = 2.0 * std::f64::consts::PI * hz / f64::from(self.io.out.rate());
        true
    }

    pub fn highpass_hz(&self) -> f64 {
        self.hp_hz
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineError {
    /// The output rate is outside 8000 to 192000 Hz.
    Rate,
}

impl std::fmt::Display for MachineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MachineError::Rate => write!(f, "The output rate must be 8000 to 192000 Hz."),
        }
    }
}

impl std::error::Error for MachineError {}

/// The physical address that a logical one has under the mapping registers.
fn physical(mpr: &[u8; 8], logical: u16) -> u32 {
    u32::from(mpr[usize::from(logical >> 13)]) << 13 | u32::from(logical & 0x1FFF)
}
