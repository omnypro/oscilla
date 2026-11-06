//! The public machine: creation, reset, rendering and inspection.

use crate::image::Image;
use crate::scope::{SCOPE_HZ, Scope, VOICES};
use crate::state::*;
use oscilla_resample::{OUT_RING, Resampler};
use std::sync::Arc;

/// The warning list holds at most 2048 bytes and a closing NUL. Its room is
/// reserved at the start so that the list never moves.
fn new_warnings() -> Vec<u8> {
    let mut w = Vec::with_capacity(2049);
    w.push(0);
    w
}

impl Machine {
    /// A machine for `image` that produces `output_rate` frames per second
    /// (8000 to 192000). The image may be dropped afterwards. None when the
    /// rate is out of range. The machine starts in the state a cartridge is
    /// left in after the BIOS.
    pub fn new(image: &Image, output_rate: i32) -> Option<Machine> {
        if !(8000..=192000).contains(&output_rate) {
            return None;
        }
        let mut m = Machine {
            cpu: CpuState::default(),
            now: 0,
            next_event: 0,
            ev_video: 0,
            video_hblank: false,
            ev_sample: 0,
            ev_frameseq: 0,
            ev_timer: [NEVER; 4],
            bios: crate::bios::bios_image(),
            ewram: zeroed(),
            iwram: zeroed(),
            io: zeroed(),
            pal: zeroed(),
            vram: zeroed(),
            oam: zeroed(),
            rom: Arc::from(image.rom()),
            ewram_image: image.ewram().map(Arc::from),
            entry: image.entry(),
            bios_latch: 0,
            dma_latch: 0,
            wait_n16: [0; 16],
            wait_s16: [0; 16],
            wait_n32: [0; 16],
            wait_s32: [0; 16],
            prefetch: false,
            pf_addr: 0,
            pf_count: 0,
            pf_progress: 0,
            pf_fresh: false,
            pf_busy: 0,
            pf_time: 0,
            vcount: 0,
            vblank_flag: false,
            hblank_flag: false,
            vmatch_flag: false,
            ie: 0,
            iflags: 0,
            ime: 0,
            dma_active: false,
            dma_any_pending: false,
            dma: Default::default(),
            timer: Default::default(),
            apu: ApuState::default(),
            scope: Scope::new(),
            out: Resampler::new(output_rate, 24),
            channel_mask: 0x3F,
            last_change: 0,
            observer: ObserverSlot::default(),
            warnings: new_warnings(),
            warned_swi: [0; 8],
            warned_undefined: false,
            warned_misc: false,
        };
        m.sys_reset();
        Some(m)
    }

    /// Back to the state `new` left it in, with the same image.
    pub fn reset(&mut self) {
        self.last_change = 0;
        self.warnings.clear();
        self.warnings.push(0);
        self.warned_swi = [0; 8];
        self.warned_undefined = false;
        self.warned_misc = false;
        self.scope.clear();
        self.sys_reset();
    }

    /// Runs until at least `frames` output frames are final.
    fn produce(&mut self, frames: usize) {
        while self.out_available() < frames {
            let missing = (frames - self.out_available()) as u64;
            let cycles = missing * u64::from(CLOCK_HZ) / self.out.rate() as u64 + 64;
            self.sys_run_until(self.now + cycles);
            self.out_finalize(self.now);
        }
    }

    /// Fills `out` with interleaved stereo frames, left then right.
    pub fn render_s16(&mut self, out: &mut [i16]) {
        let mut frames = out.len() / 2;
        let mut at = 0;
        while frames > 0 {
            let chunk = frames.min(OUT_RING / 4);
            self.produce(chunk);
            let got = self.out_take_s16(Some(&mut out[2 * at..2 * (at + chunk)]), chunk);
            at += got;
            frames -= got;
        }
    }

    pub fn render_f32(&mut self, out: &mut [f32]) {
        let mut frames = out.len() / 2;
        let mut at = 0;
        while frames > 0 {
            let chunk = frames.min(OUT_RING / 4);
            self.produce(chunk);
            let got = self.out_take_f32(&mut out[2 * at..2 * (at + chunk)], chunk);
            at += got;
            frames -= got;
        }
    }

    /// Runs `cycles` native cycles. Sound produced meanwhile is discarded.
    pub fn run_cycles(&mut self, cycles: u64) {
        let target = self.now.saturating_add(cycles);
        while self.now < target {
            let mut step = target - self.now;
            let slice = (OUT_RING as u64 / 4) * u64::from(CLOCK_HZ) / self.out.rate() as u64;
            if step > slice {
                step = slice;
            }
            self.sys_run_until(self.now + step);
            self.out_finalize(self.now);
            let available = self.out_available();
            self.out_take_s16(None, available);
        }
    }

    /// Native cycles run since the last reset.
    pub fn cycles(&self) -> u64 {
        self.now
    }

    /// Watches the sound hardware; None stops watching.
    pub fn set_observer(&mut self, observer: Option<Box<dyn Observer>>) {
        self.observer = ObserverSlot(observer);
    }

    /// r0..r14 by index, the address of the next instruction for 15, and the
    /// CPSR for 16.
    pub fn register(&self, index: i32) -> u32 {
        match index {
            15 => self.cpu.next_pc,
            0..=14 => self.cpu.r[index as usize],
            16 => self.cpu.cpsr,
            _ => 0,
        }
    }

    pub fn next_pc(&self) -> u32 {
        self.cpu.next_pc
    }

    /// The cycle at which the display next enters VBlank.
    pub fn next_vblank(&self) -> u64 {
        // ev_video is either this line's HBlank start or its end.
        let line_end = if self.video_hblank {
            self.ev_video
        } else {
            self.ev_video + (LINE_CYCLES - HDRAW_CYCLES)
        };
        let lines =
            ((160 - self.vcount - 1 + LINES_PER_FRAME as i32) % LINES_PER_FRAME as i32) as u64;
        line_end + lines * LINE_CYCLES
    }

    /// Reads memory as the CPU would see it from cartridge code, with no side
    /// effects. Width is 1, 2 or 4.
    pub fn peek(&self, address: u32, width: i32) -> u32 {
        match width {
            1 => self.bus_read8(address),
            2 => self.bus_read16(address),
            _ => self.bus_read32(address),
        }
    }

    /// Things the machine met and does not implement, as a NUL-terminated
    /// list of short sentences, one per line. The list stays at one address
    /// for the life of the machine.
    pub fn warnings_nul(&self) -> &[u8] {
        &self.warnings
    }
}

// ---- Snapshots, channels and the end of a song -------------------------

/// A copy of everything a machine is, apart from the observer and the channel
/// mask, which belong to the listener.
#[derive(Clone)]
pub struct Snapshot(Box<Machine>);

impl Snapshot {
    /// About how many bytes the snapshot holds.
    pub fn size(&self) -> usize {
        std::mem::size_of::<Machine>()
            + crate::state::BIOS_SIZE as usize
            + crate::state::EWRAM_SIZE
            + crate::state::IWRAM_SIZE
            + crate::state::IO_SIZE as usize
            + crate::state::PAL_SIZE
            + crate::state::VRAM_SIZE
            + crate::state::OAM_SIZE
            + self.0.out.heap_bytes()
            + self.0.scope.heap_bytes()
    }
}

/// Why a snapshot cannot be restored into a machine.
#[derive(Debug, PartialEq, Eq)]
pub enum RestoreError {
    /// The machine renders at another rate.
    Rate,
    /// The machine runs another image.
    Image,
}

/// What each channel is putting out now, before the mixer scales it: tone,
/// wave and noise channels from 0 to 15, Direct Sound from -128 to 127. A
/// muted channel still shows its level here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelLevels {
    pub psg: [i32; 4],
    pub fifo: [i32; 2],
}

/// What kind of sound a voice makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceKind {
    Tone = 0,
    Noise = 1,
    Sample = 2,
}

/// One channel of the sound unit as a display shows it. The voices follow the
/// channel mask: the two square channels, the wave channel, the noise channel,
/// then Direct Sound A and B. Every field comes from the sound registers and
/// the channel's own state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voice {
    pub name: &'static str,
    /// The bit of the channel mask that mutes this voice.
    pub mask_bit: u32,
    pub kind: VoiceKind,
    /// The channel plays at a volume above 0 to at least one side.
    pub active: bool,
    /// The channel plays: for the tone, wave and noise channels, their bit in
    /// SOUNDCNT_X; for a FIFO, the sound unit is on, a side hears it and its
    /// timer runs.
    pub key_on: bool,
    /// The pitch of the note in Hz, from the frequency register, and 0 for
    /// noise. For a FIFO, the rate of the timer that drives it.
    pub frequency_hz: f32,
    /// 0 to 1: the envelope, the wave channel's output level or a FIFO's 50 or
    /// 100 percent, and 0 while the channel does not play. The mixer's master
    /// volume and its ratio for the tone channels are not in it.
    pub volume: f32,
    /// 0 to 1: the envelope volume over 15 of a square or noise channel, 0
    /// while it does not play, and -1 for the wave channel and the FIFOs.
    pub envelope: f32,
    /// The volume to each side, 0 for a side that the channel is not on.
    pub left: f32,
    pub right: f32,
    /// A square channel's duty (0 to 3), a hash of the wave channel's samples,
    /// 1 for 7-bit noise and 0 for 15-bit, and -1 for a FIFO.
    pub instrument: i32,
    /// 0: the sound unit has no envelope phases.
    pub phase: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndState {
    /// Code is running or waiting for an interrupt that can come.
    Running,
    /// The CPU waits for an interrupt and none is enabled, so it never wakes.
    HaltedWithoutWake,
    /// The CPU runs a branch to itself and no interrupt can break in.
    Parked,
}

impl Machine {
    /// Copies the machine. The copy resumes exactly where the machine is, and
    /// the same calls then produce the same sound.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot(Box::new(self.clone()))
    }

    /// Puts the machine back where `snapshot` was taken. The snapshot must
    /// come from a machine with the same image and output rate.
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), RestoreError> {
        let s = &snapshot.0;
        if s.out.rate() != self.out.rate() {
            return Err(RestoreError::Rate);
        }
        let same_image = (Arc::ptr_eq(&s.rom, &self.rom) || s.rom == self.rom)
            && s.entry == self.entry
            && match (&s.ewram_image, &self.ewram_image) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
                _ => false,
            };
        if !same_image {
            return Err(RestoreError::Image);
        }
        let observer = std::mem::take(&mut self.observer);
        let mask = self.channel_mask;
        // The warning list keeps its allocation, so a pointer to it stays good.
        let mut warnings = std::mem::take(&mut self.warnings);
        *self = Machine::clone(s);
        warnings.clear();
        warnings.extend_from_slice(&s.warnings);
        self.warnings = warnings;
        self.observer = observer;
        self.channel_mask = mask;
        Ok(())
    }

    pub fn channel_levels(&self) -> ChannelLevels {
        let a = &self.apu;
        ChannelLevels {
            psg: [a.ch[0].held, a.ch[1].held, a.ch[2].held, a.ch[3].held],
            fifo: [i32::from(a.fifo[0].latch), i32::from(a.fifo[1].latch)],
        }
    }

    /// Chooses the channels that reach the mix; see the field's description
    /// for the bits. All six are on by default.
    pub fn set_channel_mask(&mut self, mask: u8) {
        self.channel_mask = mask & 0x3F;
    }

    pub fn channel_mask(&self) -> u8 {
        self.channel_mask
    }

    /// The number of voices: the six channels of the mask.
    pub fn voice_count(&self) -> usize {
        VOICES
    }

    /// What voice `index` is doing, or None for an index past the last.
    pub fn voice(&self, index: usize) -> Option<Voice> {
        self.apu_voice(index)
    }

    /// Fills `out` with the last `out.len()` frames that voice `index` put
    /// out, at the machine's output rate, as floats within -1 to 1, oldest
    /// first. The channel's own level and its sides are in it, and the mask
    /// and the mixer's volumes are not, so a muted voice still shows. The
    /// machine keeps 4,096 samples of 44,100 Hz (93 ms), and frames that reach
    /// further back are 0. Returns false, and leaves `out` alone, for an index
    /// past the last voice.
    pub fn scope(&self, index: usize, out: &mut [f32]) -> bool {
        if index >= VOICES {
            return false;
        }
        let ratio = SCOPE_HZ as f64 / f64::from(self.out.rate());
        let n = out.len();
        for (k, o) in out.iter_mut().enumerate() {
            let back = (n - 1 - k) as f64 * ratio;
            let i = back as usize;
            let frac = back - i as f64;
            let a = f64::from(self.scope.get(index, i));
            let b = f64::from(self.scope.get(index, i + 1));
            *o = ((a + (b - a) * frac) / 32768.0) as f32;
        }
        true
    }

    pub fn end_state(&self) -> EndState {
        let c = &self.cpu;
        if c.halted && self.ie == 0 {
            return EndState::HaltedWithoutWake;
        }
        let interrupts_can_run =
            c.cpsr & crate::state::PSR_I == 0 && self.ime & 1 != 0 && self.ie != 0;
        if !c.halted && !interrupts_can_run {
            let pc = c.next_pc;
            let branch_to_self = if c.cpsr & crate::state::PSR_T != 0 {
                self.bus_fetch16(pc) == 0xE7FE
            } else {
                self.bus_fetch32(pc) == 0xEAFF_FFFE
            };
            if branch_to_self {
                return EndState::Parked;
            }
        }
        EndState::Running
    }

    /// Native cycles since the output level last changed. A long silence
    /// shows as a large value; what counts as long is for the caller.
    pub fn quiet_cycles(&self) -> u64 {
        self.now.saturating_sub(self.last_change)
    }
}
