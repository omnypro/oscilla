//! The public machine: creation, the player's schedule, rendering and
//! inspection.
//!
//! The player does what a GBS file asks. It starts the init routine with the
//! song number in A, and when that returns it calls the play routine at the
//! rate the header names. A routine ends when it returns to RETURN_ADDRESS.

use crate::bus::WARN_STALL;
use crate::state::*;
use oscilla_gbs::Gbs;
use oscilla_resample::{BLEP_SHIFT, OUT_RING, Resampler};
use std::sync::Arc;

/// The warning list holds at most 2048 bytes and a closing NUL. Its room is
/// reserved at the start so that the list never moves.
fn new_warnings() -> Vec<u8> {
    let mut w = Vec::with_capacity(2049);
    w.push(0);
    w
}

/// Clocks from a call to the next increment of TIMA. Drivers that read the
/// timer to see how late a call is get the answers that the reference player
/// gives with this phase.
const TIMER_PHASE: u64 = 96;

/// A routine that has run this long without returning is abandoned.
const ROUTINE_LIMIT: u64 = CLOCK_HZ as u64;

impl Machine {
    /// A machine for the file `gbs` that produces `output_rate` frames per
    /// second (8000 to 192000). The file may be dropped afterwards. None when
    /// the rate is out of range. The machine starts at the first song.
    pub fn new(gbs: &Gbs, output_rate: i32) -> Option<Machine> {
        if !(8000..=192000).contains(&output_rate) {
            return None;
        }
        crate::apu::warm_decay_table();
        let mut m = Machine::blank(Arc::from(gbs.data.as_slice()), gbs.load, output_rate);
        m.init = gbs.init;
        m.play = gbs.play;
        m.sp0 = gbs.sp;
        m.tma0 = gbs.tma;
        m.tac0 = gbs.tac;
        m.double_speed = gbs.tac & 0x80 != 0;
        m.tick = u64::from(gbs.tick_clocks());
        m.songs = gbs.songs;
        m.first_song = gbs.first_song.min(gbs.songs.saturating_sub(1));
        m.select_song(m.first_song);
        Some(m)
    }

    /// A machine that runs a plain cartridge image from its entry point, with
    /// interrupts, for the tests of the CPU.
    #[doc(hidden)]
    pub fn from_rom(rom: &[u8], output_rate: i32) -> Option<Machine> {
        if !(8000..=192000).contains(&output_rate) || rom.len() > 0x8000 {
            return None;
        }
        let mut m = Machine::blank(Arc::from(rom), 0, output_rate);
        m.interrupts = true;
        m.songs = 1;
        m.restart(0);
        m.cpu.pc = 0x0100;
        m.cpu.sp = 0xFFFE;
        m.cpu.a = 1;
        m.phase = Phase::Routine;
        Some(m)
    }

    fn blank(rom: Arc<[u8]>, load: u16, output_rate: i32) -> Machine {
        Machine {
            cpu: Cpu::default(),
            now: 0,
            mem: ram64k(),
            rom,
            load,
            init: 0,
            play: 0,
            sp0: 0xFFFE,
            tma0: 0,
            tac0: 0,
            double_speed: false,
            ly_origin: 0,
            tick: u64::from(oscilla_gbs::VBLANK_CLOCKS),
            songs: 1,
            first_song: 0,
            bank: 1,
            timer: Timer::default(),
            ie: 0,
            iflag: 0,
            apu: Apu::default(),
            out: Resampler::new(output_rate, CLOCK_SHIFT),
            channel_mask: 0xF,
            last_change: 0,
            phase: Phase::Idle,
            song: 0,
            frame: 0,
            next_call: 0,
            call_start: 0,
            stalled: false,
            in_init: false,
            announced: false,
            interrupts: false,
            serial: Vec::new(),
            observer: ObserverSlot::default(),
            warnings: new_warnings(),
            warned: 0,
        }
    }

    /// Everything but the program goes back to the state before a song.
    fn restart(&mut self, song: u8) {
        self.cpu = Cpu {
            a: song,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: self.sp0,
            pc: self.init,
            ..Cpu::default()
        };
        self.now = 0;
        self.mem.fill(0);
        self.bank = 1;
        // DIV reads 0xAB after the boot program.
        self.timer = Timer {
            sys_offset: 0xAB00,
            tima_offset: 0xAB00,
            tima: 0,
            tma: self.tma0,
            tac: self.tac0 & 7,
            synced: 0,
        };
        self.ie = 0;
        self.iflag = 0;
        self.apu = Apu::default();
        self.apu.on = true;
        // The registers the boot program leaves; every channel is silent.
        self.apu.r[0x01] = 0x80;
        self.apu.r[0x02] = 0xF3;
        self.apu.r[0x14] = 0x77;
        self.apu.r[0x15] = 0xF3;
        // The filter has settled on the level of the idle DACs, as it has by
        // the time a cartridge starts.
        let (input, gate) = self.apu.mix_all(self.channel_mask);
        self.apu.input = input;
        self.apu.gate = gate;
        self.apu.cap = [f64::from(input[0]), f64::from(input[1])];
        self.out.reset();
        self.last_change = 0;
        self.phase = Phase::Idle;
        self.song = song;
        self.frame = 0;
        self.next_call = 0;
        self.call_start = 0;
        self.stalled = false;
        self.in_init = false;
        self.announced = false;
        self.serial.clear();
        self.warnings.clear();
        self.warnings.push(0);
        self.warned = 0;
    }

    /// Resets the machine and starts `song` (counted from 0). False when the
    /// file has no such song.
    pub fn select_song(&mut self, song: u8) -> bool {
        if song >= self.songs {
            return false;
        }
        self.restart(song);
        self.in_init = true;
        self.begin_routine(self.init);
        true
    }

    /// Back to the start of the current song.
    pub fn reset(&mut self) {
        let song = self.song;
        self.select_song(song);
    }

    pub fn song(&self) -> u8 {
        self.song
    }

    pub fn song_count(&self) -> u8 {
        self.songs
    }

    fn poke_ram(&mut self, a: u16, v: u8) {
        if (0x8000..0xFF00).contains(&a) || (0xFF80..0xFFFF).contains(&a) {
            self.write_at(a, v);
        }
    }

    /// Calls the routine at `addr`: the CPU's return address is
    /// RETURN_ADDRESS.
    fn begin_routine(&mut self, addr: u16) {
        self.cpu.halted = false;
        self.cpu.stopped = false;
        self.cpu.locked = false;
        self.cpu.halt_bug = false;
        self.cpu.sp = self.cpu.sp.wrapping_sub(2);
        let sp = self.cpu.sp;
        self.poke_ram(sp, RETURN_ADDRESS as u8);
        self.poke_ram(sp.wrapping_add(1), (RETURN_ADDRESS >> 8) as u8);
        self.cpu.pc = addr;
        self.call_start = self.now;
        self.align_clocks();
        self.phase = Phase::Routine;
    }

    /// A routine starts as if the hardware had just raised its interrupt:
    /// the timer has just reloaded, or the display has just reached VBlank.
    fn align_clocks(&mut self) {
        let t = self.now;
        self.ly_origin = t;
        if self.timer.tac & 4 != 0 {
            self.timer_sync(t);
            self.timer.tima = self.timer.tma;
            let period = 1u64 << (Self::timer_bit_of(self.timer.tac) + 1);
            self.timer.tima_offset = (period - (t + TIMER_PHASE) % period) % period;
        }
    }

    /// Clocks between calls of the play routine, from the timer registers
    /// as they stand.
    fn live_tick(&self) -> u64 {
        if self.timer.tac & 4 == 0 {
            return u64::from(oscilla_gbs::VBLANK_CLOCKS);
        }
        let per_count: u64 = [1024, 16, 64, 256][usize::from(self.timer.tac & 3)];
        let clocks = per_count * (256 - u64::from(self.timer.tma));
        if self.double_speed {
            clocks / 2
        } else {
            clocks
        }
    }

    fn routine_returned(&mut self) {
        let was_init = self.in_init;
        self.in_init = false;
        self.phase = Phase::Idle;
        self.tick = self.live_tick();
        self.next_call = if was_init {
            self.now.max(self.tick)
        } else {
            (self.call_start + self.tick).max(self.now)
        };
    }

    fn run_until(&mut self, target: u64) {
        if self.interrupts {
            while self.now < target {
                self.cpu_step();
            }
            return;
        }
        if !self.announced {
            self.announced = true;
            let t = self.now;
            if let Some(o) = self.observer.0.as_mut() {
                o.play_call(t, -1);
            }
        }
        loop {
            match self.phase {
                Phase::Routine => {
                    if self.now >= target {
                        break;
                    }
                    if self.cpu.pc == RETURN_ADDRESS {
                        self.routine_returned();
                        continue;
                    }
                    let broken = self.cpu.halted || self.cpu.stopped || self.cpu.locked;
                    if broken || self.now - self.call_start > ROUTINE_LIMIT {
                        if !broken {
                            self.warn(WARN_STALL, "A routine did not return within a second.");
                        }
                        self.stalled = true;
                        self.routine_returned();
                        continue;
                    }
                    self.cpu_step();
                }
                Phase::Idle => {
                    if self.next_call > target {
                        self.now = self.now.max(target);
                        break;
                    }
                    self.now = self.now.max(self.next_call);
                    let (t, frame) = (self.now, self.frame);
                    if let Some(o) = self.observer.0.as_mut() {
                        o.play_call(t, frame as i64);
                    }
                    self.frame += 1;
                    self.begin_routine(self.play);
                }
            }
        }
    }

    /// Runs until at least `frames` output frames are final.
    fn produce(&mut self, frames: usize) {
        while self.out.available() < frames {
            let missing = (frames - self.out.available()) as u64;
            let cycles = missing * u64::from(CLOCK_HZ) / self.out.rate() as u64 + 64;
            self.run_until(self.now + cycles);
            let t = self.now;
            self.apu_advance(t);
            self.out.finalize(t);
        }
    }

    /// Fills `out` with interleaved left and right 16-bit samples.
    pub fn render_s16(&mut self, out: &mut [i16]) {
        let frames = out.len() / 2;
        let mut at = 0;
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            self.produce(chunk);
            let got = self.out.take_s16(
                Some(&mut out[2 * at..2 * (at + chunk)]),
                chunk,
                3,
                BLEP_SHIFT + 2,
            );
            at += got;
        }
    }

    /// Fills `out` with interleaved left and right floats.
    pub fn render_f32(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut at = 0;
        // The same three quarters of full scale as the 16-bit output.
        let scale = 0.75 / f64::from(1u32 << (BLEP_SHIFT * 2));
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            self.produce(chunk);
            let got = self
                .out
                .take_f32(&mut out[2 * at..2 * (at + chunk)], chunk, scale);
            at += got;
        }
    }

    /// Runs `cycles` clocks. Sound produced meanwhile is discarded.
    pub fn run_cycles(&mut self, cycles: u64) {
        let end = self.now.saturating_add(cycles);
        while self.now < end {
            let slice = (OUT_RING as u64 / 4) * u64::from(CLOCK_HZ) / self.out.rate() as u64;
            self.run_until((self.now + slice).min(end));
            let t = self.now;
            self.apu_advance(t);
            self.out.finalize(t);
            let available = self.out.available();
            self.out.take_s16(None, available, 3, BLEP_SHIFT + 2);
        }
    }

    /// Clocks run since the song started.
    pub fn cycles(&self) -> u64 {
        self.now
    }

    /// Play calls begun since the song started.
    pub fn frames(&self) -> u64 {
        self.frame
    }

    pub fn set_observer(&mut self, observer: Option<Box<dyn Observer>>) {
        self.observer.0 = observer;
    }

    /// A, F, B, C, D, E, H, L by index 0 to 7; 8 is SP and 9 is PC.
    pub fn register(&self, index: i32) -> u32 {
        let c = &self.cpu;
        match index {
            0 => u32::from(c.a),
            1 => u32::from(c.f),
            2 => u32::from(c.b),
            3 => u32::from(c.c),
            4 => u32::from(c.d),
            5 => u32::from(c.e),
            6 => u32::from(c.h),
            7 => u32::from(c.l),
            8 => u32::from(c.sp),
            9 => u32::from(c.pc),
            _ => 0,
        }
    }

    pub fn next_pc(&self) -> u16 {
        self.cpu.pc
    }

    /// Reads memory as the CPU would see it, with no side effects.
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x7FFF => {
                let linear = if address < 0x4000 {
                    usize::from(address)
                } else {
                    self.bank as usize * 0x4000 + usize::from(address - 0x4000)
                };
                let load = usize::from(self.load);
                if linear < load {
                    0
                } else {
                    self.rom.get(linear - load).copied().unwrap_or(0)
                }
            }
            0xE000..=0xFDFF => self.mem[usize::from(address - 0x2000)],
            0xFEA0..=0xFF00 => 0xFF,
            0xFF04 => (self.sys(self.now) >> 8) as u8,
            0xFF05 => self.timer_at(self.now).0,
            0xFF06 => self.timer.tma,
            0xFF07 => self.timer.tac | 0xF8,
            0xFF0F => {
                let overflowed = self.timer_at(self.now).1;
                (self.iflag | u8::from(overflowed) << 2) | 0xE0
            }
            0xFF44 => self.ly_at(self.now),
            0xFF10..=0xFF25 => {
                let i = usize::from(address - 0xFF10);
                self.apu.r[i] | crate::apu::READ_OR[i]
            }
            0xFF26 => {
                let a = &self.apu;
                u8::from(a.on) << 7
                    | 0x70
                    | u8::from(a.ch1.active)
                    | u8::from(a.ch2.active) << 1
                    | u8::from(a.ch3.active) << 2
                    | u8::from(a.ch4.active) << 3
            }
            0xFF27..=0xFF2F => 0xFF,
            0xFF30..=0xFF3F => self.apu.wave_ram[usize::from(address - 0xFF30)],
            0xFFFF => self.ie,
            _ => self.mem[usize::from(address)],
        }
    }

    /// Things that the machine met and does not support, as a NUL-terminated
    /// list of short sentences, one per line.
    pub fn warnings_nul(&self) -> &[u8] {
        &self.warnings
    }

    /// The bytes that a cartridge image sent to the serial port.
    #[doc(hidden)]
    pub fn serial(&self) -> &[u8] {
        &self.serial
    }

    /// Bytes of one copy of the state, as a snapshot holds them.
    pub fn size(&self) -> usize {
        std::mem::size_of::<Machine>() + 0x10000 + self.out.heap_bytes()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot(Box::new(self.clone()))
    }

    /// Puts the machine back where `snapshot` was taken. The snapshot must
    /// come from a machine with the same file and output rate.
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), RestoreError> {
        let s = &snapshot.0;
        if s.out.rate() != self.out.rate() {
            return Err(RestoreError::Rate);
        }
        let same = (Arc::ptr_eq(&s.rom, &self.rom) || s.rom == self.rom)
            && (s.load, s.init, s.play, s.sp0, s.tma0, s.tac0, s.songs)
                == (
                    self.load, self.init, self.play, self.sp0, self.tma0, self.tac0, self.songs,
                );
        if !same {
            return Err(RestoreError::Image);
        }
        let observer = std::mem::take(&mut self.observer);
        let mask = self.channel_mask;
        let snapshot_mask = s.channel_mask;
        // The warning list keeps its allocation, so a pointer to it stays good.
        let mut warnings = std::mem::take(&mut self.warnings);
        *self = Machine::clone(s);
        warnings.clear();
        warnings.extend_from_slice(&s.warnings);
        self.warnings = warnings;
        self.observer = observer;
        self.channel_mask = mask;
        if mask != snapshot_mask {
            // The snapshot's mix was made with its own mask.
            let t = self.now;
            self.apu_advance(t);
            self.apu_emit(t);
        }
        Ok(())
    }

    /// What each channel puts out now, before the DAC: 0 to 15.
    pub fn channel_levels(&self) -> [i32; 4] {
        let a = &self.apu;
        [0, 1, 2, 3].map(|ch| i32::from(a.digital(ch)))
    }

    /// Chooses the channels that reach the mix; bit n is channel n + 1. All
    /// four are on by default.
    pub fn set_channel_mask(&mut self, mask: u8) {
        self.channel_mask = mask & 0xF;
        let t = self.now;
        self.apu_advance(t);
        self.apu_emit(t);
    }

    pub fn channel_mask(&self) -> u8 {
        self.channel_mask
    }

    pub fn end_state(&self) -> EndState {
        if self.stalled {
            return EndState::Stalled;
        }
        let a = &self.apu;
        if !a.on || !(a.ch1.active || a.ch2.active || a.ch3.active || a.ch4.active) {
            return EndState::ChannelsOff;
        }
        EndState::Running
    }

    /// Clocks since the output level last changed. A long silence shows as a
    /// large value; what counts as long is for the caller.
    pub fn quiet_cycles(&self) -> u64 {
        self.now.saturating_sub(self.last_change)
    }
}

/// How a song stands, for a host that decides when it has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndState {
    /// A channel is sounding, or may be.
    Running,
    /// No channel is on. A later call of the play routine can start one.
    ChannelsOff,
    /// A routine did not return, halted, or hit an opcode that locks the CPU.
    Stalled,
}

/// A copy of everything a machine is, apart from the observer and the channel
/// mask, which belong to the listener.
#[derive(Clone)]
pub struct Snapshot(Box<Machine>);

impl Snapshot {
    pub fn size(&self) -> usize {
        self.0.size()
    }
}

/// Why a snapshot cannot be restored into a machine.
#[derive(Debug, PartialEq, Eq)]
pub enum RestoreError {
    Rate,
    Image,
}
