//! The machine: the SPC700, the S-DSP and the 64 KiB of RAM that they share,
//! with the clock that drives both and the resampler that turns the 32 kHz
//! samples into the caller's rate.

use std::sync::Arc;

use oscilla_resample::{BLEP_SHIFT, OUT_RING, Resampler};
use oscilla_spc::Spc;

use crate::dsp::{BrrPosition, Dsp, ECHO_BIT, SampleInfo, VOICES, VoiceInfo, decode_sample};

/// SPC700 cycles per second. The DSP makes a sample every 32 of them.
pub const CLOCK: u64 = 1_024_000;
pub const CYCLES_PER_SAMPLE: u64 = 32;
pub const SAMPLE_RATE: u32 = 32_000;

/// The bits of the channel mask: one for each voice, then the echo output.
pub const ALL_CHANNELS: u64 = (1 << (VOICES as u32 + 1)) - 1;

/// A block of registers that `Machine::register_block_get` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterBlock {
    pub name: &'static str,
    /// The address of the first byte in the chip's own register space.
    pub base: u32,
    pub size: usize,
}

/// The register blocks: the S-DSP's 128 registers and the SPC700's I/O
/// registers at 0xF0 to 0xFF.
pub const REGISTER_BLOCKS: [RegisterBlock; 2] = [
    RegisterBlock { name: "S-DSP", base: 0x00, size: 0x80 },
    RegisterBlock { name: "SPC700 I/O", base: 0xF0, size: 0x10 },
];

/// What a timer is doing, without a side effect on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerState {
    pub enabled: bool,
    /// The target register: 0 divides by 256.
    pub divisor: u8,
    /// The 4-bit output as a read of the output register would return it.
    pub output: u8,
    /// SPC700 cycles from now until the output next increments, or 0 for a
    /// timer that is off.
    pub ticks_until_output: u32,
}

/// Watches what the driver does to the sound hardware. Callbacks run on the
/// rendering thread, in program order.
pub trait Observer {
    /// A store to the DSP through ports 0xF2 and 0xF3, at the SPC700 clock of
    /// the store counted from the first sample.
    fn dsp_write(&mut self, cycle: u64, register: u8, value: u8);
    /// A store to one of the four output ports 0xF4 to 0xF7.
    fn port_write(&mut self, cycle: u64, port: u8, value: u8);
    /// A store by the SPC700 to sound RAM inside the range of
    /// `Machine::watch_ram`, at the clock of the end of its instruction. The
    /// stores of one instruction arrive in program order. Stores by the DSP
    /// (the echo buffer) are not reported.
    fn ram_write(&mut self, _cycle: u64, _address: u16, _value: u8) {}
}

/// Holds the observer and the range of RAM that it watches outside the machine
/// state: a copy of the state has neither.
#[derive(Default)]
pub struct ObserverSlot(pub Option<Box<dyn Observer>>, pub Option<(u16, u16)>);

impl Clone for ObserverSlot {
    fn clone(&self) -> Self {
        ObserverSlot(None, None)
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Timer {
    pub enabled: bool,
    pub target: u8,
    /// The divider that counts the input clock up to the target.
    pub stage: u8,
    /// The 4-bit output that a read clears.
    pub out: u8,
}

/// The cycle, modulo 128, at which the 8 kHz timers count. The reference logs
/// fit 0 once a read inside an instruction sees the clock at the cycle of the
/// read.
const TIMER_PHASE: u64 = 0;

/// A level unit is one step of a 16-bit sample.
const OUT_SHIFT: u32 = BLEP_SHIFT;

/// Holds the access map outside the machine state: a copy of the state has none.
#[derive(Default)]
pub(crate) struct AccessMap(pub Option<Box<[u8; 65536]>>);

impl Clone for AccessMap {
    fn clone(&self) -> AccessMap {
        AccessMap(None)
    }
}

/// The bits of the access map. A byte has a bit for each kind of access since
/// the last clear: the SPC700 reading it as data, writing it, or fetching it as
/// an instruction or operand, and the DSP reading or writing it.
pub const CPU_READ: u8 = 1;
pub const CPU_WRITE: u8 = 2;
pub const EXECUTED: u8 = 4;
pub use crate::dsp::{DSP_READ, DSP_WRITE};

#[derive(Clone)]
pub struct Machine {
    pub(crate) a: u8,
    pub(crate) x: u8,
    pub(crate) y: u8,
    pub(crate) sp: u8,
    pub(crate) pc: u16,
    pub(crate) psw: u8,
    pub(crate) ram: Box<[u8; 65536]>,
    pub(crate) dsp: Dsp,
    pub(crate) control: u8,
    pub(crate) dsp_address: u8,
    pub(crate) ports_in: [u8; 4],
    pub(crate) ports_out: [u8; 4],
    pub(crate) timers: [Timer; 3],
    pub(crate) now: u64,
    pub(crate) halted: bool,
    pub(crate) observer: ObserverSlot,
    /// One byte for each address of the sound RAM, with a bit for each kind of
    /// access since the last clear, when tracking is on.
    pub(crate) access: AccessMap,
    image: Arc<Spc>,
    out: Resampler,
    last: (i32, i32),
    last_change: u64,
    capture: Option<Vec<i16>>,
    timer_phase: u64,
    pub(crate) fetched: u32,
    pub(crate) elapsed: u32,
    pub(crate) deferred: Vec<(u8, u8)>,
    pub(crate) deferred_ports: Vec<(u8, u8)>,
    pub(crate) deferred_rams: Vec<(u16, u8)>,
    pending: Vec<(u8, u8)>,
}

/// A copy of everything a machine is, apart from the observer and the channel
/// mask, which belong to the listener.
#[derive(Clone)]
pub struct Snapshot(Box<Machine>);

impl Snapshot {
    pub fn size(&self) -> usize {
        std::mem::size_of::<Machine>() + 65536 + self.0.out.heap_bytes()
    }
}

/// Why a snapshot cannot be restored into a machine.
#[derive(Debug, PartialEq, Eq)]
pub enum RestoreError {
    Rate,
    Image,
}

/// How a song stands, for a host that decides when it has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndState {
    Running,
    /// The SPC700 executed SLEEP or STOP.
    Stalled,
}

impl Machine {
    /// A machine in the state the file holds, producing `output_rate` frames a
    /// second. Returns None when the rate is not positive.
    pub fn new(spc: &Spc, output_rate: i32) -> Option<Machine> {
        Machine::from_image(Arc::new(spc.clone()), output_rate)
    }

    fn from_image(image: Arc<Spc>, output_rate: i32) -> Option<Machine> {
        if !(8000..=192_000).contains(&output_rate) {
            return None;
        }
        let mut m = Machine {
            a: 0,
            x: 0,
            y: 0,
            sp: 0,
            pc: 0,
            psw: 0,
            ram: Box::new([0; 65536]),
            dsp: Dsp::new([0; 128]),
            control: 0,
            dsp_address: 0,
            ports_in: [0; 4],
            ports_out: [0; 4],
            timers: [Timer::default(); 3],
            now: 0,
            halted: false,
            observer: ObserverSlot::default(),
            access: AccessMap::default(),
            image,
            out: Resampler::with_ratio(output_rate, CLOCK, 1),
            last: (0, 0),
            last_change: 0,
            capture: None,
            timer_phase: TIMER_PHASE,
            fetched: 0,
            elapsed: 0,
            deferred: Vec::new(),
            deferred_ports: Vec::new(),
            deferred_rams: Vec::new(),
            pending: Vec::new(),
        };
        m.load();
        Some(m)
    }

    /// Puts the machine in the state of the file.
    fn load(&mut self) {
        let spc = Arc::clone(&self.image);
        let r = spc.registers;
        self.a = r.a;
        self.x = r.x;
        self.y = r.y;
        self.sp = r.sp;
        self.pc = r.pc;
        self.psw = r.psw;
        *self.ram = *spc.ram;
        self.control = 0;
        // The 64 bytes under the boot ROM are the real RAM when the ROM shows.
        if self.ram[0xF1] & 0x80 != 0 {
            self.ram[0xFFC0..].copy_from_slice(&spc.extra);
        }
        let mask = self.dsp.mask;
        self.dsp = Dsp::new(spc.dsp);
        self.dsp.mask = mask;
        self.dsp_address = self.ram[0xF2];
        self.ports_out = [self.ram[0xF4], self.ram[0xF5], self.ram[0xF6], self.ram[0xF7]];
        self.timers = [Timer::default(); 3];
        for (n, t) in self.timers.iter_mut().enumerate() {
            t.target = self.ram[0xFA + n];
        }
        self.set_control(self.ram[0xF1]);
        // The reset bits of the control register act on a write, not on a load.
        self.ports_in = [self.ram[0xF4], self.ram[0xF5], self.ram[0xF6], self.ram[0xF7]];
        for (n, t) in self.timers.iter_mut().enumerate() {
            if t.enabled {
                t.out = self.ram[0xFD + n] & 15;
                t.stage = 1;
            }
        }
        self.now = 0;
        self.halted = false;
        self.out.reset();
        self.last = (0, 0);
        self.last_change = 0;
    }

    /// Starts the song again from the state of the file.
    pub fn reset(&mut self) {
        self.load();
    }

    /// Runs one instruction and the cycles it takes. A store to the DSP lands
    /// in the last cycle of its instruction, and a read of a timer or of the
    /// DSP sees the clock as the instruction has advanced it so far.
    fn execute(&mut self) {
        let cycles = self.step();
        let remaining = cycles.saturating_sub(self.elapsed);
        if self.deferred.is_empty()
            && self.deferred_ports.is_empty()
            && self.deferred_rams.is_empty()
        {
            self.tick(remaining);
            return;
        }
        self.tick(remaining.saturating_sub(1));
        for (register, value) in std::mem::take(&mut self.deferred) {
            self.dsp.write(register, value);
            self.pending.push((register, value));
        }
        self.tick(remaining.min(1));
        let cycle = self.now;
        let writes = std::mem::take(&mut self.pending);
        let ports = std::mem::take(&mut self.deferred_ports);
        let rams = std::mem::take(&mut self.deferred_rams);
        if let Some(o) = self.observer.0.as_mut() {
            for (register, value) in writes {
                o.dsp_write(cycle, register, value);
            }
            for (port, value) in ports {
                o.port_write(cycle, port, value);
            }
            for (address, value) in rams {
                o.ram_write(cycle, address, value);
            }
        }
    }

    /// Brings the clock up to the cycle that the running instruction has
    /// reached.
    pub(crate) fn catch_up(&mut self) {
        let due = self.fetched.saturating_sub(self.elapsed);
        self.elapsed += due;
        self.tick(due);
    }

    /// Advances the timers and the DSP by `cycles` SPC700 cycles.
    fn tick(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.now += 1;
            let t = self.now.wrapping_sub(self.timer_phase);
            if t.is_multiple_of(16) {
                self.step_timer(2);
                if t.is_multiple_of(128) {
                    self.step_timer(0);
                    self.step_timer(1);
                }
            }
            if self.now.is_multiple_of(CYCLES_PER_SAMPLE) {
                let (l, r) = self.dsp.sample(&mut self.ram);
                if let Some(map) = self.access.0.as_mut() {
                    for &(first, len, kind) in &self.dsp.accesses {
                        for n in 0..usize::from(len) {
                            map[(usize::from(first) + n) & 0xFFFF] |= kind;
                        }
                    }
                }
                if (l, r) != self.last {
                    self.last_change = self.now;
                    self.last = (l, r);
                }
                self.out.step(self.now, l, r);
                if let Some(c) = self.capture.as_mut() {
                    c.push(l.clamp(-32768, 32767) as i16);
                    c.push(r.clamp(-32768, 32767) as i16);
                }
            }
        }
    }

    /// One tick of the input clock of a timer. Timers 0 and 1 count 8 kHz and
    /// timer 2 counts 64 kHz.
    fn step_timer(&mut self, n: usize) {
        let t = &mut self.timers[n];
        if !t.enabled {
            return;
        }
        t.stage = t.stage.wrapping_add(1);
        if t.stage == t.target {
            t.stage = 0;
            t.out = (t.out + 1) & 15;
        }
    }

    /// Runs until the clock reaches `end`. An instruction that starts before
    /// `end` finishes, so the clock may pass it.
    fn run_until(&mut self, end: u64) {
        while self.now < end {
            self.execute();
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
                1,
                OUT_SHIFT,
            );
            at += got;
        }
    }

    /// Fills `out` with interleaved left and right floats.
    pub fn render_f32(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut at = 0;
        let scale = 1.0 / f64::from(1u32 << 15) / f64::from(1u32 << BLEP_SHIFT);
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            self.produce(chunk);
            let got = self
                .out
                .take_f32(&mut out[2 * at..2 * (at + chunk)], chunk, scale);
            at += got;
        }
    }

    /// Runs the machine until the resampler holds `frames` more frames.
    fn produce(&mut self, frames: usize) {
        let rate = self.out.rate() as u64;
        while self.out.available() < frames {
            let missing = (frames - self.out.available()) as u64;
            let target = self.now + (missing * CLOCK).div_ceil(rate).max(CYCLES_PER_SAMPLE);
            self.run_until(target);
            let t = self.now;
            self.out.finalize(t);
        }
    }

    /// Runs `cycles` clocks. Sound produced meanwhile is discarded.
    pub fn run_cycles(&mut self, cycles: u64) {
        let end = self.now.saturating_add(cycles);
        while self.now < end {
            let slice = (OUT_RING as u64 / 4) * CLOCK / self.out.rate() as u64;
            self.run_until((self.now + slice).min(end));
            let t = self.now;
            self.out.finalize(t);
            let available = self.out.available();
            self.out.take_s16(None, available, 1, OUT_SHIFT);
        }
    }

    /// Runs until the DSP has made `frames` more samples and returns them as
    /// interleaved stereo at 32 kHz, before any resampling.
    #[doc(hidden)]
    pub fn dsp_samples(&mut self, frames: usize) -> Vec<i16> {
        self.capture = Some(Vec::with_capacity(frames * 2));
        let end = self.now + frames as u64 * CYCLES_PER_SAMPLE;
        while self.now < end {
            self.execute();
        }
        let mut out = self.capture.take().unwrap_or_default();
        out.truncate(frames * 2);
        out
    }

    #[doc(hidden)]
    pub fn set_timer_phase(&mut self, phase: u64) {
        self.timer_phase = phase;
    }

    /// Runs one SPC700 instruction.
    #[doc(hidden)]
    pub fn step_instruction(&mut self) {
        self.execute();
    }

    pub fn cycles(&self) -> u64 {
        self.now
    }

    /// DSP samples made so far.
    pub fn samples(&self) -> u64 {
        self.now / CYCLES_PER_SAMPLE
    }

    /// The registers: PC, A, X, Y, PSW and SP.
    pub fn cpu_state(&self) -> (u16, u8, u8, u8, u8, u8) {
        (self.pc, self.a, self.x, self.y, self.psw, self.sp)
    }

    pub fn set_pc(&mut self, pc: u16) {
        self.pc = pc;
        self.halted = false;
    }

    /// A byte of sound RAM, without side effects.
    pub fn peek(&self, address: u16) -> u8 {
        self.ram[usize::from(address)]
    }

    /// All 64 KB of sound RAM, without side effects. Like `peek`, it is the RAM
    /// under the boot ROM and the ports.
    pub fn ram(&self) -> &[u8; 65536] {
        &self.ram
    }

    /// The registers of block `block` of `REGISTER_BLOCKS` as the chip holds
    /// them, with no side effect: a read of a timer output does not clear it.
    /// Copies the first `out.len()` bytes and returns how many, 0 for a block
    /// that does not exist.
    pub fn register_block_get(&self, block: usize, out: &mut [u8]) -> usize {
        let Some(info) = REGISTER_BLOCKS.get(block) else {
            return 0;
        };
        let n = info.size.min(out.len());
        for (i, o) in out[..n].iter_mut().enumerate() {
            *o = match block {
                0 => self.dsp.regs[i],
                _ => self.io_peek(i as u8),
            };
        }
        n
    }

    /// What a program reads at 0xF0 plus `i`, without the side effects of the
    /// read.
    fn io_peek(&self, i: u8) -> u8 {
        match i {
            0 | 1 => 0,
            2 => self.dsp_address,
            3 => self.dsp.read(self.dsp_address),
            4..=7 => self.ports_in[usize::from(i - 4)],
            0xD..=0xF => self.timers[usize::from(i - 0xD)].out & 15,
            _ => self.ram[0xF0 + usize::from(i)],
        }
    }

    /// The three timers.
    pub fn timers(&self) -> [TimerState; 3] {
        std::array::from_fn(|n| {
            let t = &self.timers[n];
            TimerState {
                enabled: t.enabled,
                divisor: t.target,
                output: t.out & 15,
                ticks_until_output: self.ticks_until_output(n),
            }
        })
    }

    /// The cycles until the next increment of the output of timer `n`.
    fn ticks_until_output(&self, n: usize) -> u32 {
        let t = &self.timers[n];
        if !t.enabled {
            return 0;
        }
        let period: u64 = if n == 2 { 16 } else { 128 };
        // The divider counts up to the target and wraps at 256, so a target
        // that the divider already holds waits for the wrap.
        let counts = match t.target.wrapping_sub(t.stage) {
            0 => 256,
            d => u64::from(d),
        };
        let since = self.now.wrapping_sub(self.timer_phase) % period;
        (period - since + (counts - 1) * period) as u32
    }

    /// A DSP register.
    pub fn dsp_register(&self, register: u8) -> u8 {
        self.dsp.read(register)
    }

    /// Sets what the main CPU would write to an input port. Ports 0 to 3 are
    /// 0xF4 to 0xF7.
    pub fn write_port(&mut self, port: usize, value: u8) {
        self.ports_in[port & 3] = value;
    }

    /// What the SPC700 has written to an output port.
    pub fn read_port(&self, port: usize) -> u8 {
        self.ports_out[port & 3]
    }

    pub fn set_observer(&mut self, observer: Option<Box<dyn Observer>>) {
        self.observer.0 = observer;
    }

    /// Reports the SPC700's stores to the RAM addresses `first` to `last`
    /// (both included) through `Observer::ram_write`. A range with `first`
    /// above `last` turns the report off, which is the start state. A
    /// snapshot restore keeps the range.
    pub fn watch_ram(&mut self, first: u16, last: u16) {
        self.observer.1 = (first <= last).then_some((first, last));
        self.deferred_rams.clear();
    }

    pub fn end_state(&self) -> EndState {
        if self.halted {
            EndState::Stalled
        } else {
            EndState::Running
        }
    }

    /// Cycles since the output level last changed.
    pub fn quiet_cycles(&self) -> u64 {
        self.now.saturating_sub(self.last_change)
    }

    /// Chooses which voices and whether the echo reach the mix. Bit n is voice
    /// n and bit 8 is the echo output. Voices and the echo keep running
    /// whatever the mask says.
    pub fn set_channel_mask(&mut self, mask: u64) {
        self.dsp.mask = mask & ALL_CHANNELS;
    }

    pub fn channel_mask(&self) -> u64 {
        self.dsp.mask & ALL_CHANNELS
    }

    /// The signed high byte of each voice's last output, before the volumes.
    pub fn voice_levels(&self) -> [i32; VOICES] {
        self.dsp.levels()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot(Box::new(self.clone()))
    }

    /// Puts the machine back where `snapshot` was taken. The snapshot must come
    /// from a machine with the same file and output rate.
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), RestoreError> {
        let s = &snapshot.0;
        if s.out.rate() != self.out.rate() {
            return Err(RestoreError::Rate);
        }
        if !(Arc::ptr_eq(&s.image, &self.image) || s.image == self.image) {
            return Err(RestoreError::Image);
        }
        let observer = std::mem::take(&mut self.observer);
        let access = std::mem::take(&mut self.access);
        let mask = self.dsp.mask;
        *self = Machine::clone(s);
        self.observer = observer;
        self.dsp.track = access.0.is_some();
        self.access = access;
        self.dsp.mask = mask & ALL_CHANNELS;
        Ok(())
    }

    /// Turns the tracking of memory accesses on or off. Turning it on starts
    /// with a clear map. The map belongs to the listener, so a copy of the
    /// machine has none, and `restore` keeps the one that is set.
    pub fn set_access_tracking(&mut self, on: bool) {
        self.access.0 = on.then(|| Box::new([0; 65536]));
        self.dsp.track = on;
    }

    pub fn access_tracking(&self) -> bool {
        self.access.0.is_some()
    }

    /// The access map, or `None` when tracking is off.
    pub fn access_map(&self) -> Option<&[u8; 65536]> {
        self.access.0.as_deref()
    }

    /// Clears the access map.
    pub fn clear_access_map(&mut self) {
        if let Some(map) = self.access.0.as_mut() {
            map.fill(0);
        }
    }

    /// What a voice is doing.
    pub fn voice_info(&self, voice: usize) -> VoiceInfo {
        self.dsp.voice_info(voice)
    }

    /// Fills `out` with the last `out.len()` frames of a voice's output at the
    /// machine's output rate, as floats within -1 to 1, oldest first. The
    /// samples come from the DSP's 32 kHz by linear interpolation, and ones
    /// older than 4,096 DSP samples (128 ms) are 0. The voice's volumes are in
    /// it and the master volume is not, and a muted voice still shows.
    pub fn scope(&self, voice: usize, out: &mut [f32]) {
        self.resample_scope(out, |back| self.dsp.scope_sample(voice, back));
    }

    /// Fills `out` as `scope` does with the echo output of the left (0) or
    /// right (1) channel: the filter's output times the echo volume, as it
    /// joins the mix. The channel mask and the mute flag leave it alone.
    pub fn echo_scope(&self, channel: usize, out: &mut [f32]) {
        self.resample_scope(out, |back| self.dsp.echo_scope_sample(channel, back));
    }

    fn resample_scope(&self, out: &mut [f32], sample: impl Fn(usize) -> i16) {
        let ratio = 32_000.0 / f64::from(self.out.rate());
        let n = out.len();
        for (k, o) in out.iter_mut().enumerate() {
            let back = (n - 1 - k) as f64 * ratio;
            let i = back as usize;
            let frac = back - i as f64;
            let a = f64::from(sample(i));
            let b = f64::from(sample(i + 1));
            *o = ((a + (b - a) * frac) / 32768.0) as f32;
        }
    }

    /// Where a voice is in its sample.
    pub fn voice_brr(&self, voice: usize) -> BrrPosition {
        self.dsp.voice_brr(voice, &self.ram)
    }

    /// Decodes sample `source` of the sample table from the sound RAM as it
    /// is now. Writes the first `out.len()` samples to `out` and returns what
    /// the scan found. The number written is `min(total_samples, out.len())`.
    pub fn sample(&self, source: u8, out: &mut [i16]) -> SampleInfo {
        decode_sample(&self.ram, self.dsp.regs[0x5D], source, out)
    }

    /// True when the echo bit of the mask is on.
    pub fn echo_enabled(&self) -> bool {
        self.dsp.mask & (1 << ECHO_BIT) != 0
    }

    pub fn song_info(&self) -> &Spc {
        &self.image
    }
}
