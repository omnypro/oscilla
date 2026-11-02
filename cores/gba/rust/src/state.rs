//! The machine's data: everything that decides what the machine does next,
//! grouped by the hardware unit that owns each field.

use oscilla_resample::Resampler;
use std::sync::Arc;

pub const LINE_CYCLES: u64 = 1232;
pub const HDRAW_CYCLES: u64 = 1006;
pub const LINES_PER_FRAME: u64 = 228;
pub const NEVER: u64 = u64::MAX;

pub const BIOS_SIZE: u32 = 0x4000;
pub const EWRAM_SIZE: usize = 0x40000;
pub const IWRAM_SIZE: usize = 0x8000;
pub const IO_SIZE: u32 = 0x400;
pub const PAL_SIZE: usize = 0x400;
pub const VRAM_SIZE: usize = 0x18000;
pub const OAM_SIZE: usize = 0x400;
pub const ROM_MAX: u32 = 0x0200_0000;

pub const PSR_N: u32 = 0x8000_0000;
pub const PSR_Z: u32 = 0x4000_0000;
pub const PSR_C: u32 = 0x2000_0000;
pub const PSR_V: u32 = 0x1000_0000;
pub const PSR_I: u32 = 0x80;
pub const PSR_T: u32 = 0x20;
pub const PSR_MODE: u32 = 0x1F;

pub const MODE_USR: u32 = 0x10;
pub const MODE_FIQ: u32 = 0x11;
pub const MODE_IRQ: u32 = 0x12;
pub const MODE_SVC: u32 = 0x13;
pub const MODE_ABT: u32 = 0x17;
pub const MODE_UND: u32 = 0x1B;
pub const MODE_SYS: u32 = 0x1F;

pub const IRQ_VBLANK: u16 = 0x0001;
pub const IRQ_HBLANK: u16 = 0x0002;
pub const IRQ_VCOUNT: u16 = 0x0004;
pub const IRQ_TIMER0: u16 = 0x0008;
pub const IRQ_DMA0: u16 = 0x0100;

/// Who made a store.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Cpu,
    Dma,
    Bios,
}

/// The native clock: 16,777,216 cycles per second.
pub const CLOCK_HZ: u32 = 16_777_216;

#[derive(Clone, Default)]
pub struct CpuState {
    pub r: [u32; 16],
    pub cpsr: u32,
    pub spsr: u32,
    /// Banked copies, indexed by `bank_of`: 0 user/system, 1 fiq, 2 irq,
    /// 3 supervisor, 4 abort, 5 undefined.
    pub bank_sp: [u32; 6],
    pub bank_lr: [u32; 6],
    pub bank_spsr: [u32; 6],
    pub fiq_hi: [u32; 5],
    pub usr_hi: [u32; 5],
    pub next_pc: u32,
    pub cur_pc: u32,
    pub pc_written: bool,
    pub fetch_n: bool,
    pub halted: bool,
}

#[derive(Clone, Default)]
pub struct DmaChannel {
    pub sad: u32,
    pub dad: u32,
    pub cnt_l: u16,
    pub cnt_h: u16,
    pub src: u32,
    pub dst: u32,
    pub count: u32,
    pub pending: bool,
}

#[derive(Clone, Default)]
pub struct TimerChannel {
    pub reload: u16,
    pub control: u16,
    pub counter: u16,
    pub base_time: u64,
    pub shift: u32,
    pub running: bool,
    pub cascade: bool,
}

#[derive(Clone, Default)]
pub struct PsgChannel {
    pub on: bool,
    pub dac: bool,
    pub length: i32,
    pub length_on: bool,
    pub volume: i32,
    pub env_dir: i32,
    pub env_period: i32,
    pub env_timer: i32,
    pub freq: i32,
    pub timer: i64,
    pub pos: i32,
    pub sample: i32,
    pub duty: i32,
    pub sweep_period: i32,
    pub sweep_dir: i32,
    pub sweep_shift: i32,
    pub sweep_timer: i32,
    pub sweep_on: bool,
    pub shadow: i32,
    pub lfsr: u32,
    pub narrow: bool,
    pub nr: i32,
    pub ns: i32,
    pub noise_out: i32,
    pub held: i32,
}

#[derive(Clone, Default)]
pub struct FifoState {
    pub data: [i8; 32],
    pub read: i32,
    pub count: i32,
    pub latch: i8,
}

#[derive(Clone, Default)]
pub struct ApuState {
    pub ch: [PsgChannel; 4],
    pub wave: [[u8; 16]; 2],
    pub wave_bank: i32,
    pub frame_step: i32,
    pub psg_time: u64,
    pub fifo: [FifoState; 2],
    pub last_level: [i32; 2],
    pub sample_shift: i32,
}

/// Watches what the game does to the sound hardware. Callbacks run on the
/// rendering thread, in program order, with the cycle at which the access
/// started.
pub trait Observer {
    /// A store to 0x04000060..0x040000A7, as the bus received it.
    fn sound_write(&mut self, cycle: u64, address: u32, value: u32, width: i32);
    /// A byte that a Direct Sound FIFO (channel 0 or 1) shifted out.
    fn fifo_sample(&mut self, cycle: u64, channel: i32, sample: i8);
}

/// Holds the observer outside the machine state: a copy of the state has none.
#[derive(Default)]
pub struct ObserverSlot(pub Option<Box<dyn Observer>>);

impl Clone for ObserverSlot {
    fn clone(&self) -> Self {
        ObserverSlot(None)
    }
}

pub type Ram<const N: usize> = Box<[u8; N]>;

pub fn zeroed<const N: usize>() -> Ram<N> {
    vec![0u8; N]
        .into_boxed_slice()
        .try_into()
        .expect("length matches")
}

#[derive(Clone)]
pub struct Machine {
    pub(crate) cpu: CpuState,
    pub(crate) now: u64,
    pub(crate) next_event: u64,

    pub(crate) ev_video: u64,
    pub(crate) video_hblank: bool,
    pub(crate) ev_sample: u64,
    pub(crate) ev_frameseq: u64,
    pub(crate) ev_timer: [u64; 4],

    pub(crate) bios: Ram<{ BIOS_SIZE as usize }>,
    pub(crate) ewram: Ram<EWRAM_SIZE>,
    pub(crate) iwram: Ram<IWRAM_SIZE>,
    pub(crate) io: Ram<{ IO_SIZE as usize }>,
    pub(crate) pal: Ram<PAL_SIZE>,
    pub(crate) vram: Ram<VRAM_SIZE>,
    pub(crate) oam: Ram<OAM_SIZE>,
    pub(crate) rom: Arc<[u8]>,
    /// EWRAM content to restore on reset (multiboot).
    pub(crate) ewram_image: Option<Arc<[u8]>>,
    pub(crate) entry: u32,

    pub(crate) bios_latch: u32,
    pub(crate) dma_latch: u32,

    pub(crate) wait_n16: [u8; 16],
    pub(crate) wait_s16: [u8; 16],
    pub(crate) wait_n32: [u8; 16],
    pub(crate) wait_s32: [u8; 16],
    pub(crate) prefetch: bool,
    pub(crate) pf_addr: u32,
    pub(crate) pf_count: i32,
    pub(crate) pf_progress: i32,
    pub(crate) pf_fresh: bool,
    pub(crate) pf_busy: i32,
    pub(crate) pf_time: u64,

    pub(crate) vcount: i32,
    pub(crate) vblank_flag: bool,
    pub(crate) hblank_flag: bool,
    pub(crate) vmatch_flag: bool,

    pub(crate) ie: u16,
    pub(crate) iflags: u16,
    pub(crate) ime: u16,
    pub(crate) dma_active: bool,
    pub(crate) dma_any_pending: bool,

    pub(crate) dma: [DmaChannel; 4],
    pub(crate) timer: [TimerChannel; 4],
    pub(crate) apu: ApuState,
    pub(crate) out: Resampler,
    /// Channels that reach the mix: bits 0 to 3 are the tone, wave and noise
    /// channels, bits 4 and 5 are Direct Sound A and B. A listener's setting,
    /// so it survives a reset and a restore.
    pub(crate) channel_mask: u8,
    /// The cycle at which the output level last changed.
    pub(crate) last_change: u64,

    pub(crate) observer: ObserverSlot,

    /// NUL-terminated list of short sentences, one per line.
    pub(crate) warnings: Vec<u8>,
    pub(crate) warned_swi: [u32; 8],
    pub(crate) warned_undefined: bool,
    pub(crate) warned_misc: bool,
}
