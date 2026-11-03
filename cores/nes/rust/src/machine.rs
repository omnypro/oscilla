//! The public machine: creation, the player's schedule, rendering and
//! inspection.
//!
//! The player does what an NSF file asks. It starts the init routine with the
//! song number in A and the region in X, and when that returns it calls the
//! play routine at the interval the header names. A routine ends when it
//! returns to RETURN_ADDRESS.

use crate::apu::{CONSOLE_HIGHPASS_HZ, Highpass, Lowpass, MAX_HIGHPASS_HZ, MIN_HIGHPASS_HZ};
use crate::bus::WARN_STALL;
use crate::state::*;
use oscilla_nsf::{Nsf, Region};
use oscilla_resample::{BLEP_SHIFT, OUT_RING, Resampler};
use std::sync::Arc;

/// The bits of the channel mask that name a channel.
const ALL_CHANNELS: u32 = (1 << 20) - 1;

/// The warning list holds at most 2048 bytes and a closing NUL. Its room is
/// reserved at the start so that the list never moves.
fn new_warnings() -> Vec<u8> {
    let mut w = Vec::with_capacity(2049);
    w.push(0);
    w
}

/// A routine that has run this long without returning is abandoned: one second.
fn routine_limit(clock: (u64, u64)) -> u64 {
    clock.0 / clock.1
}

/// A level unit becomes OUT_MUL / 2^OUT_SHIFT of a 16-bit step. The mixer's full
/// scale then sits at 29/32 of a sample's range, which gives the loudness of a
/// reference player. A drum made of delta-channel steps can reach past full
/// scale, and a host that needs headroom scales the output.
const OUT_MUL: i64 = 29;
const OUT_SHIFT: u32 = 5;

fn ram(size: usize) -> Ram {
    vec![0u8; size].into_boxed_slice()
}

impl Machine {
    /// A machine for the file `nsf` that produces `output_rate` frames per
    /// second (8000 to 192000). The file may be dropped afterwards. None when
    /// the rate is out of range. The machine starts at the first song, on the
    /// console that the file names.
    pub fn new(nsf: &Nsf, output_rate: i32) -> Option<Machine> {
        Machine::new_for(nsf, output_rate, nsf.region())
    }

    /// The same for a console that the caller picks.
    pub fn new_for(nsf: &Nsf, output_rate: i32, region: Region) -> Option<Machine> {
        if !(8000..=192000).contains(&output_rate) {
            return None;
        }
        let banked = nsf.is_banked();
        let (pad, banks) = if banked {
            (usize::from(nsf.load & 0xFFF), nsf.banks)
        } else {
            (
                usize::from(nsf.load.saturating_sub(0x8000)),
                [0, 1, 2, 3, 4, 5, 6, 7],
            )
        };
        let mut image = vec![0u8; pad];
        image.extend_from_slice(&nsf.data);
        let mut m = Machine::blank(Arc::from(image), banks, output_rate, region);
        m.banked = banked;
        m.init = nsf.init;
        m.play = nsf.play;
        m.chips = nsf.chips;
        m.flags = nsf.flags;
        m.interval_us = nsf.play_interval_us(region);
        m.songs = nsf.songs;
        m.first_song = nsf.first_song.min(nsf.songs.saturating_sub(1));
        m.select_song(m.first_song);
        Some(m)
    }

    /// A machine that runs a plain program image, for the tests of the CPU. The
    /// image sits at 0x8000, and a 16 KiB one is mirrored at 0xC000.
    #[doc(hidden)]
    pub fn from_rom(rom: &[u8], output_rate: i32) -> Option<Machine> {
        if !(8000..=192000).contains(&output_rate) || rom.len() > 0x8000 {
            return None;
        }
        let mut image = rom.to_vec();
        if rom.len() <= 0x4000 {
            image.extend_from_slice(rom);
        }
        let mut m = Machine::blank(
            Arc::from(image),
            [0, 1, 2, 3, 4, 5, 6, 7],
            output_rate,
            Region::Ntsc,
        );
        m.cartridge = true;
        m.songs = 1;
        m.restart(0);
        m.cpu.pc = u16::from(m.read_rom(0xFFFC)) | u16::from(m.read_rom(0xFFFD)) << 8;
        m.cpu.sp = 0xFD;
        m.cpu.p = 0x24;
        m.now = 7;
        m.phase = Phase::Routine;
        Some(m)
    }

    fn blank(image: Arc<[u8]>, banks: [u8; 8], output_rate: i32, region: Region) -> Machine {
        let clock = clock_of(region);
        Machine {
            cpu: Cpu::default(),
            now: 0,
            bus: 0,
            ram: ram(0x800),
            wram: ram(0x2000),
            fds_ram: ram(0),
            image,
            banks,
            banked: false,
            header_banks: banks,
            init: 0,
            play: 0,
            region,
            clock,
            interval_us: 16639,
            chips: 0,
            flags: 0,
            songs: 1,
            first_song: 0,
            apu: Apu::new(region, 0),
            out: Resampler::with_ratio(output_rate, clock.0, clock.1),
            hp: Highpass::console(clock),
            lp: Lowpass::console(clock),
            channel_mask: ALL_CHANNELS,
            last_change: 0,
            phase: Phase::Idle,
            song: 0,
            frame: 0,
            next_call: 0,
            call_start: 0,
            stalled: false,
            in_init: false,
            announced: false,
            cartridge: false,
            ppu_read: 0,
            observer: ObserverSlot::default(),
            warnings: new_warnings(),
            warned: 0,
        }
    }

    /// Everything but the program goes back to the state before a song.
    fn restart(&mut self, song: u8) {
        self.cpu = Cpu {
            a: song,
            x: u8::from(self.region == Region::Pal),
            y: 0,
            sp: 0xFD,
            pc: self.init,
            p: 0x24,
            locked: false,
        };
        self.now = 0;
        self.bus = 0;
        self.ram.fill(0);
        self.wram.fill(0);
        if self.chips & oscilla_nsf::chips::FDS != 0 {
            if self.fds_ram.is_empty() {
                self.fds_ram = ram(0x6000);
            }
            self.fds_ram.fill(0);
            // The program is copied into the RAM: window by window for a
            // file with banks, and all at once for a file without.
            for window in 0..6usize {
                let bank = if self.banked {
                    self.header_banks[window]
                } else {
                    window as u8
                };
                self.fds_load_window(window, bank);
            }
            if self.banked {
                self.fds_load_window(6, self.header_banks[6]);
                self.fds_load_window(7, self.header_banks[7]);
            }
        }
        self.banks = self.header_banks;
        self.apu = Apu::new(self.region, self.chips);
        // The player writes the sound registers as the format asks: zero to
        // 0x4000..=0x4013, zero then 0x0F to 0x4015 and 0x40 to 0x4017. These
        // are not the driver's stores, so the observer does not see them.
        for a in 0x4000..=0x4013u16 {
            self.apu_write(a, 0);
        }
        self.apu_write(0x4015, 0x00);
        self.apu_write(0x4015, 0x0F);
        self.apu_write(0x4017, 0x40);
        // The filter has settled on the level of the idle channels, as it has by
        // the time a driver starts.
        let a = &mut self.apu;
        a.input = a.mix(self.channel_mask);
        a.cap = a.input;
        a.lowpassed = 0.0;
        a.hpf_time = 0;
        a.next_hpf = NEVER;
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
        self.warnings.clear();
        self.warnings.push(0);
        self.warned = 0;
        self.warn_chips();
        self.warn_flags();
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

    pub fn region(&self) -> Region {
        self.region
    }

    /// Calls the routine at `addr`: the return address is on the stack as a
    /// JSR would leave it.
    fn begin_routine(&mut self, addr: u16) {
        self.cpu.locked = false;
        self.cpu.sp = 0xFD;
        let ret = RETURN_ADDRESS.wrapping_sub(1);
        self.ram[0x1FF] = (ret >> 8) as u8;
        self.ram[0x1FE] = ret as u8;
        self.cpu.pc = addr;
        self.call_start = self.now;
        self.phase = Phase::Routine;
    }

    /// The cycle of play call number `k` (the init routine is call 0), counted
    /// from the start of the song: k intervals, rounded down.
    fn scheduled(&self, k: u64) -> u64 {
        let (num, den) = self.clock;
        let cycles = u128::from(k) * u128::from(self.interval_us) * u128::from(num)
            / (u128::from(den) * 1_000_000);
        cycles as u64
    }

    fn routine_returned(&mut self) {
        let was_init = self.in_init;
        self.in_init = false;
        self.phase = Phase::Idle;
        let due = self.scheduled(self.frame + 1);
        self.next_call = if self.flags & oscilla_nsf::nsf2::NO_PLAY != 0 {
            NEVER
        } else if was_init {
            self.now.max(due)
        } else {
            due.max(self.now)
        };
    }

    fn run_until(&mut self, target: u64) {
        if self.cartridge {
            while self.now < target && !self.cpu.locked {
                self.cpu_step();
            }
            self.now = self.now.max(target);
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
                    if self.cpu.locked || self.now - self.call_start > routine_limit(self.clock) {
                        if !self.cpu.locked {
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
                    // Sample reads while the CPU waits cost it nothing.
                    let t = self.now;
                    self.apu_advance(t);
                    self.apu.stall = 0;
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
            let (num, den) = self.clock;
            let cycles = missing * num / (den * self.out.rate() as u64) + 64;
            self.run_until(self.now + cycles);
            self.sync_apu();
            let t = self.now;
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
                OUT_MUL,
                BLEP_SHIFT + OUT_SHIFT,
            );
            at += got;
        }
    }

    /// Fills `out` with interleaved left and right floats.
    pub fn render_f32(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut at = 0;
        // The same fraction of full scale as the 16-bit output.
        let scale = (OUT_MUL as f64 / f64::from(1u32 << OUT_SHIFT))
            / f64::from(1u32 << 15)
            / f64::from(1u32 << BLEP_SHIFT);
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
        let (num, den) = self.clock;
        while self.now < end {
            let slice = (OUT_RING as u64 / 4) * num / (den * self.out.rate() as u64);
            self.run_until((self.now + slice).min(end));
            self.sync_apu();
            let t = self.now;
            self.out.finalize(t);
            let available = self.out.available();
            self.out
                .take_s16(None, available, OUT_MUL, BLEP_SHIFT + OUT_SHIFT);
        }
    }

    /// Runs one CPU instruction of a test program.
    #[doc(hidden)]
    pub fn step_instruction(&mut self) {
        self.cpu_step();
    }

    /// Moves a test program's PC.
    #[doc(hidden)]
    pub fn set_pc(&mut self, pc: u16) {
        self.cpu.pc = pc;
    }

    /// PC, A, X, Y, P and SP, for the tests of the CPU.
    #[doc(hidden)]
    pub fn cpu_state(&self) -> (u16, u8, u8, u8, u8, u8) {
        let c = &self.cpu;
        (c.pc, c.a, c.x, c.y, c.p, c.sp)
    }

    /// Cycles run since the song started.
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

    /// A, X, Y, P, S by index 0 to 4; 5 is PC.
    pub fn register(&self, index: i32) -> u32 {
        let c = &self.cpu;
        match index {
            0 => u32::from(c.a),
            1 => u32::from(c.x),
            2 => u32::from(c.y),
            3 => u32::from(c.p),
            4 => u32::from(c.sp),
            5 => u32::from(c.pc),
            _ => 0,
        }
    }

    pub fn next_pc(&self) -> u16 {
        self.cpu.pc
    }

    /// Things that the machine met and does not support, as a NUL-terminated
    /// list of short sentences, one per line.
    pub fn warnings_nul(&self) -> &[u8] {
        &self.warnings
    }

    /// Bytes of one copy of the state, as a snapshot holds them.
    pub fn size(&self) -> usize {
        std::mem::size_of::<Machine>() + 0x800 + 0x2000 + self.out.heap_bytes()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot(Box::new(self.clone()))
    }

    /// Puts the machine back where `snapshot` was taken. The snapshot must
    /// come from a machine with the same file, console and output rate.
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), RestoreError> {
        let s = &snapshot.0;
        if s.out.rate() != self.out.rate() {
            return Err(RestoreError::Rate);
        }
        let same = (Arc::ptr_eq(&s.image, &self.image) || s.image == self.image)
            && (
                s.init,
                s.play,
                s.header_banks,
                s.songs,
                s.region,
                s.interval_us,
            ) == (
                self.init,
                self.play,
                self.header_banks,
                self.songs,
                self.region,
                self.interval_us,
            );
        if !same {
            return Err(RestoreError::Image);
        }
        let observer = std::mem::take(&mut self.observer);
        let mask = self.channel_mask;
        let hp = self.hp.clone();
        let snapshot_mask = s.channel_mask;
        // The warning list keeps its allocation, so a pointer to it stays good.
        let mut warnings = std::mem::take(&mut self.warnings);
        *self = Machine::clone(s);
        warnings.clear();
        warnings.extend_from_slice(&s.warnings);
        self.warnings = warnings;
        self.observer = observer;
        self.channel_mask = mask & ALL_CHANNELS;
        self.hp = hp;
        if mask != snapshot_mask {
            // The snapshot's mix was made with its own mask.
            let t = self.now;
            self.apu_advance(t);
            self.apu_emit(t);
        }
        Ok(())
    }

    /// What each channel puts out now, before the mixer: the two pulse
    /// channels, the triangle and the noise channel from 0 to 15, then the
    /// delta channel from 0 to 127.
    pub fn channel_levels(&self) -> [i32; 5] {
        self.apu.levels().map(i32::from)
    }

    /// What each expansion channel puts out now, before the mixer. A file with
    /// the VRC6 shows its two pulse channels (0 to 15) and its saw (0 to 31). A
    /// file with the Sunsoft 5B and without the VRC6 shows the 5B's three
    /// channels (0 to 31). The other entries are 0.
    pub fn expansion_levels(&self) -> [i32; 8] {
        let mut out = [0; 8];
        let levels = if self.chips & oscilla_nsf::chips::VRC6 != 0 {
            Some(self.apu.vrc6_levels())
        } else if self.chips & oscilla_nsf::chips::N163 != 0 {
            None
        } else if self.chips & oscilla_nsf::chips::S5B != 0 {
            Some(self.apu.s5b_levels())
        } else {
            None
        };
        if self.chips & oscilla_nsf::chips::VRC6 == 0
            && self.chips & oscilla_nsf::chips::N163 == 0
            && self.chips & oscilla_nsf::chips::FDS != 0
        {
            out[0] = self.apu.fds.level();
        }
        if self.chips & oscilla_nsf::chips::VRC6 == 0 && self.chips & oscilla_nsf::chips::N163 != 0
        {
            for (slot, level) in out.iter_mut().zip(self.apu.n163.levels()) {
                *slot = i32::from(level);
            }
        }
        if let Some(levels) = levels {
            for (slot, level) in out.iter_mut().zip(levels) {
                *slot = i32::from(level);
            }
        }
        out
    }

    /// Chooses the channels that reach the mix; bit n is channel n: pulse 1,
    /// pulse 2, triangle, noise and delta (bits 0 to 4), the VRC6's pulse 1,
    /// pulse 2 and saw (bits 5 to 7), and the Sunsoft 5B's three tone channels
    /// (bits 8 to 10). All are on by default.
    pub fn set_channel_mask(&mut self, mask: u32) {
        let mask = mask & ALL_CHANNELS;
        if mask == self.channel_mask {
            return;
        }
        self.channel_mask = mask;
        let t = self.now;
        self.apu_advance(t);
        self.apu_emit(t);
    }

    pub fn channel_mask(&self) -> u32 {
        self.channel_mask
    }

    /// Moves the high-pass filter's corner to `hz`, from MIN_HIGHPASS_HZ to
    /// MAX_HIGHPASS_HZ. The capacitor keeps its charge, so the output is
    /// continuous at the change. Returns false and changes nothing for a value
    /// outside the range, NaN included. The console's own filter is the
    /// default, and a reset or a restore leaves the choice alone.
    pub fn set_highpass_hz(&mut self, hz: f64) -> bool {
        if !(MIN_HIGHPASS_HZ..=MAX_HIGHPASS_HZ).contains(&hz) {
            return false;
        }
        if hz != self.hp.hz() {
            self.change_highpass(Highpass::at(hz, self.clock));
        }
        true
    }

    /// Goes back to the console's own filter.
    pub fn use_console_highpass(&mut self) {
        if self.hp.hz() != CONSOLE_HIGHPASS_HZ {
            self.change_highpass(Highpass::console(self.clock));
        }
    }

    /// The corner of the filter in use: CONSOLE_HIGHPASS_HZ for the console's.
    pub fn highpass_hz(&self) -> f64 {
        self.hp.hz()
    }

    fn change_highpass(&mut self, hp: Highpass) {
        let t = self.now;
        self.apu_advance(t);
        // The capacitor is brought to this cycle with the old factors first.
        self.apu_emit(t);
        self.hp = hp;
        self.apu_emit(t);
    }

    pub fn end_state(&self) -> EndState {
        if self.stalled {
            return EndState::Stalled;
        }
        let a = &self.apu;
        if a.pulse[0].len == 0
            && a.pulse[1].len == 0
            && a.tri.len == 0
            && a.noise.len == 0
            && a.dmc.remaining == 0
            && a.dmc.silent
        {
            return EndState::ChannelsOff;
        }
        EndState::Running
    }

    /// Cycles since the output level last changed. A long silence shows as a
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
    /// A routine did not return or locked the CPU.
    Stalled,
}

/// A copy of everything a machine is, apart from the observer, the channel
/// mask and the bass cutoff, which belong to the listener.
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
