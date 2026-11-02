//! The state of a machine. Every piece lives here so that a copy of the
//! struct is a snapshot.

use oscilla_nsf::Region;
use oscilla_resample::Resampler;
use std::sync::Arc;

/// The NTSC CPU clock in cycles per second, as a ratio: the master clock of
/// 236.25 MHz / 11 divided by 12.
pub const NTSC_CLOCK: (u64, u64) = (19_687_500, 11);
/// The PAL CPU clock: the master clock of 26.6017125 MHz divided by 16.
pub const PAL_CLOCK: (u64, u64) = (53_203_425, 32);
pub const NEVER: u64 = u64::MAX;

/// The CPU clock of `region` in cycles per second, as a ratio.
pub fn clock_of(region: Region) -> (u64, u64) {
    match region {
        Region::Ntsc => NTSC_CLOCK,
        Region::Pal => PAL_CLOCK,
    }
}

/// Watches what the driver does to the sound hardware. Callbacks run on the
/// rendering thread, in program order.
pub trait Observer {
    /// A store to 0x4000..=0x4017, as the bus received it, at the cycle of the
    /// store.
    fn sound_write(&mut self, cycle: u64, address: u16, value: u8);
    /// The player is about to call a routine: the init routine with `frame`
    /// -1, then the play routine with `frame` counting the calls from 0.
    fn play_call(&mut self, cycle: u64, frame: i64);
}

/// Holds the observer outside the machine state: a copy of the state has none.
#[derive(Default)]
pub struct ObserverSlot(pub Option<Box<dyn Observer>>);

impl Clone for ObserverSlot {
    fn clone(&self) -> Self {
        ObserverSlot(None)
    }
}

#[derive(Clone, Default)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    /// The flags N V D I Z C as bits 7, 6, 3, 2, 1, 0. Bit 5 reads as set and
    /// bit 4 only exists on the stack.
    pub p: u8,
    /// The CPU met an opcode that locks it up, or BRK.
    pub locked: bool,
}

#[derive(Clone, Default)]
pub struct Envelope {
    pub start: bool,
    pub divider: u8,
    pub decay: u8,
}

#[derive(Clone, Default)]
pub struct Sweep {
    pub enabled: bool,
    pub period: u8,
    pub negate: bool,
    pub shift: u8,
    pub divider: u8,
    pub reload: bool,
}

#[derive(Clone, Default)]
pub struct Pulse {
    pub duty: u8,
    pub halt: bool,
    pub constant: bool,
    pub volume: u8,
    pub env: Envelope,
    pub sweep: Sweep,
    pub period: u16,
    pub len: u8,
    /// The sequencer's place, 0 to 7. It counts down.
    pub pos: u8,
    /// The cycle of the next timer expiry. While the channel is silent the
    /// value may lie in the past: `settle` brings it forward before any change.
    pub next_step: u64,
}

#[derive(Clone, Default)]
pub struct Triangle {
    pub control: bool,
    pub linear_value: u8,
    pub linear: u8,
    pub linear_reload: bool,
    pub period: u16,
    pub len: u8,
    pub pos: u8,
    pub next_step: u64,
}

#[derive(Clone, Default)]
pub struct Noise {
    pub halt: bool,
    pub constant: bool,
    pub volume: u8,
    pub env: Envelope,
    pub mode: bool,
    pub period_index: u8,
    pub len: u8,
    pub lfsr: u16,
    pub next_step: u64,
}

#[derive(Clone, Default)]
pub struct Dmc {
    pub irq_enable: bool,
    pub looping: bool,
    pub rate_index: u8,
    pub level: u8,
    pub sample_address: u16,
    pub sample_length: u16,
    pub address: u16,
    pub remaining: u16,
    pub buffer: Option<u8>,
    pub shift: u8,
    pub bits: u8,
    pub silent: bool,
    pub irq: bool,
    pub next_step: u64,
}

#[derive(Clone, Default)]
pub struct FrameCounter {
    pub five_step: bool,
    pub inhibit: bool,
    pub irq: bool,
    /// The cycle that the sequence counts from.
    pub origin: u64,
    /// The next event in the sequence.
    pub step: usize,
    pub next: u64,
    /// A write in five-step mode clocks the units once when it takes effect.
    pub immediate: bool,
}

#[derive(Clone)]
pub struct Apu {
    pub pulse: [Pulse; 2],
    pub tri: Triangle,
    pub noise: Noise,
    pub dmc: Dmc,
    /// The enable bits of $4015, bit n for channel n.
    pub enabled: u8,
    pub frame: FrameCounter,
    /// The events up to this cycle have run.
    pub time: u64,
    /// The mix before the filter.
    pub input: f64,
    /// The high-pass filter's capacitor.
    pub cap: f64,
    /// The output of the low-pass filter after the high-pass filter.
    pub lowpassed: f64,
    /// The cycle up to which the capacitor is current.
    pub hpf_time: u64,
    /// The next cycle at which the filter's output is recomputed.
    pub next_hpf: u64,
    /// Cycles that the CPU lost to sample reads and has not yet been charged.
    pub stall: u64,
}

pub type Ram = Box<[u8]>;

/// Where the player is in its schedule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The CPU runs a routine; it ends when PC reaches RETURN_ADDRESS.
    Routine,
    /// The CPU waits for the next call.
    Idle,
}

/// The address that a routine returns to. It lies in the vector area, which no
/// routine runs from.
pub const RETURN_ADDRESS: u16 = 0xFFFF;

#[derive(Clone)]
pub struct Machine {
    pub(crate) cpu: Cpu,
    pub(crate) now: u64,
    /// The last value on the data bus, which an unmapped read returns.
    pub(crate) bus: u8,
    pub(crate) ram: Ram,
    pub(crate) wram: Ram,
    /// The program image: byte i sits at ROM address 0x8000 + i when the banks
    /// are 0 to 7.
    pub(crate) image: Arc<[u8]>,
    pub(crate) banks: [u8; 8],
    pub(crate) banked: bool,
    pub(crate) header_banks: [u8; 8],
    pub(crate) init: u16,
    pub(crate) play: u16,
    pub(crate) region: Region,
    pub(crate) clock: (u64, u64),
    /// Microseconds between calls of the play routine.
    pub(crate) interval_us: u32,
    pub(crate) chips: u8,
    /// The header's version 2 feature bits.
    pub(crate) flags: u8,
    pub(crate) songs: u8,
    pub(crate) first_song: u8,
    pub(crate) apu: Apu,
    pub(crate) out: Resampler,
    /// The high-pass filter's cutoff. A player's choice, outside the snapshot.
    pub(crate) hp: crate::apu::Highpass,
    /// The console's low-pass filter, which follows the high-pass filter.
    pub(crate) lp: crate::apu::Lowpass,
    /// The channel mask: bit n is channel n.
    pub(crate) channel_mask: u8,
    pub(crate) last_change: u64,

    pub(crate) phase: Phase,
    pub(crate) song: u8,
    pub(crate) frame: u64,
    pub(crate) next_call: u64,
    pub(crate) call_start: u64,
    pub(crate) stalled: bool,
    /// The routine that runs is the init routine.
    pub(crate) in_init: bool,
    /// The observer has been told about the init call.
    pub(crate) announced: bool,
    /// A plain program image, for the tests of the CPU: the PPU registers read
    /// as a screen that is always in its vertical blank and 0x6000 holds the
    /// program's own memory.
    pub(crate) cartridge: bool,
    /// The frame in which a test cartridge last read the screen's status.
    pub(crate) ppu_read: u64,

    pub(crate) observer: ObserverSlot,
    pub(crate) warnings: Vec<u8>,
    pub(crate) warned: u32,
}
