//! The state of a machine. Every piece lives here so that a copy of the
//! struct is a snapshot.

use oscilla_resample::Resampler;
use std::sync::Arc;

/// The CPU clock: 4,194,304 cycles per second.
pub const CLOCK_HZ: u32 = 4_194_304;
/// The clock is 2^22 cycles per second.
pub const CLOCK_SHIFT: u32 = 22;
pub const NEVER: u64 = u64::MAX;

/// Watches what the driver does to the sound hardware. Callbacks run on the
/// rendering thread, in program order.
pub trait Observer {
    /// A store to 0xFF10..0xFF3F, as the bus received it, at the cycle of the
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
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ime: bool,
    /// Instructions until `ei` takes effect: 2 right after `ei`, 1 after the
    /// next instruction, then 0.
    pub ei_delay: u8,
    pub halted: bool,
    pub stopped: bool,
    /// The next opcode fetch does not advance PC (the halt bug).
    pub halt_bug: bool,
    /// The CPU met an opcode that locks it up.
    pub locked: bool,
}

#[derive(Clone, Default)]
pub struct Timer {
    /// The 16-bit system counter at cycle t is (t + sys_offset) & 0xFFFF. DIV
    /// is its upper byte.
    pub sys_offset: u64,
    /// The same for the counter that TIMA counts the edges of. The player
    /// aligns it to the calls of the play routine; a write to DIV joins it
    /// to the system counter again.
    pub tima_offset: u64,
    pub tima: u8,
    pub tma: u8,
    pub tac: u8,
    /// The cycle up to which TIMA is current.
    pub synced: u64,
}

#[derive(Clone, Default)]
pub struct Length {
    pub count: u16,
    pub enabled: bool,
}

#[derive(Clone, Default)]
pub struct Envelope {
    pub volume: u8,
    pub timer: u8,
    pub running: bool,
}

#[derive(Clone, Default)]
pub struct Sweep {
    pub shadow: u16,
    pub timer: u8,
    pub enabled: bool,
    pub negated: bool,
}

#[derive(Clone, Default)]
pub struct Pulse {
    pub active: bool,
    pub length: Length,
    pub env: Envelope,
    pub duty_pos: u8,
    /// False from a trigger until the first duty step, while the output is 0.
    pub started: bool,
    pub next_step: u64,
}

#[derive(Clone, Default)]
pub struct Wave {
    pub active: bool,
    pub length: Length,
    pub pos: u8,
    /// The last sample read, shifted out at the output level.
    pub buffer: u8,
    pub next_step: u64,
}

#[derive(Clone, Default)]
pub struct Noise {
    pub active: bool,
    pub length: Length,
    pub env: Envelope,
    pub lfsr: u16,
    pub next_step: u64,
}

#[derive(Clone)]
pub struct Apu {
    pub on: bool,
    /// The raw registers 0xFF10..=0xFF26, as written.
    pub r: [u8; 0x17],
    pub wave_ram: [u8; 16],
    pub ch1: Pulse,
    pub ch2: Pulse,
    pub ch3: Wave,
    pub ch4: Noise,
    pub sweep: Sweep,
    /// The step that the next frame-sequencer tick runs, 0 to 7.
    pub fs_step: u8,
    /// The events up to this cycle have run.
    pub time: u64,
    /// Whether any DAC is on. With none on, the output is exactly 0 and the
    /// filter's capacitor holds its charge.
    pub gate: bool,
    /// The mixed level before the filter, left and right.
    pub input: [i32; 2],
    /// The high-pass filter's capacitor, left and right.
    pub cap: [f64; 2],
    /// The cycle up to which the capacitor is current.
    pub hpf_time: u64,
    /// The next cycle at which the filter's output is recomputed.
    pub next_hpf: u64,
}

pub type Ram64k = Box<[u8; 0x10000]>;

pub fn ram64k() -> Ram64k {
    vec![0u8; 0x10000]
        .into_boxed_slice()
        .try_into()
        .expect("length matches")
}

/// Where the player is in its schedule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The CPU runs a routine; it ends when PC reaches RETURN_ADDRESS.
    Routine,
    /// The CPU waits for the next call.
    Idle,
}

/// The address that a routine returns to. It lies in the interrupt-enable
/// register, which no routine runs from.
pub const RETURN_ADDRESS: u16 = 0xFFFF;

#[derive(Clone)]
pub struct Machine {
    pub(crate) cpu: Cpu,
    pub(crate) now: u64,
    pub(crate) mem: Ram64k,
    /// The program data. Byte i of the data sits at ROM address load + i.
    pub(crate) rom: Arc<[u8]>,
    pub(crate) load: u16,
    pub(crate) init: u16,
    pub(crate) play: u16,
    pub(crate) sp0: u16,
    pub(crate) tma0: u8,
    pub(crate) tac0: u8,
    /// TAC bit 7 of the header: the clock runs at double speed.
    pub(crate) double_speed: bool,
    /// LY is 144 at this cycle.
    pub(crate) ly_origin: u64,
    /// Clocks between calls of the play routine.
    pub(crate) tick: u64,
    pub(crate) songs: u8,
    pub(crate) first_song: u8,
    pub(crate) bank: u32,
    pub(crate) timer: Timer,
    pub(crate) ie: u8,
    pub(crate) iflag: u8,
    pub(crate) apu: Apu,
    pub(crate) out: Resampler,
    /// The high-pass filter's cutoff. A player's choice, outside the snapshot.
    pub(crate) hp: crate::apu::Highpass,
    /// The channel mask: bit n is channel n + 1.
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
    /// False for the GBS player, which makes no interrupts. True for a
    /// cartridge image, which has them.
    pub(crate) interrupts: bool,
    pub(crate) serial: Vec<u8>,

    pub(crate) observer: ObserverSlot,
    pub(crate) warnings: Vec<u8>,
    pub(crate) warned: u32,
}
