//! The Namco 163 expansion chip: 128 bytes of sound RAM and up to eight
//! wavetable channels that the chip plays one at a time. Written from public
//! documentation.

use crate::state::*;

/// Cycles between the updates of two channels.
const UPDATE: u64 = 15;

/// The share of a 2A03 pulse channel's full-volume level that one step of the
/// chip's output (one sample unit at one volume unit) has.
pub(crate) const N163_GAIN: f64 = 0.039;

/// The sample value that the chip takes as the centre of a waveform.
const BIAS: i32 = 8;

impl N163 {
    /// How many channels are enabled: the top three bits of the last register
    /// plus one, counted down from channel 8.
    fn channels(&self) -> u8 {
        (self.ram[0x7F] >> 4 & 7) + 1
    }

    /// One update: the next channel moves its phase on and puts out its
    /// sample, which the chip holds for 15 cycles.
    pub(crate) fn tick(&mut self, te: u64) {
        let count = self.channels();
        self.turn %= count;
        let c = 7 - self.turn;
        self.turn = (self.turn + 1) % count;
        let base = 0x40 + 8 * usize::from(c);
        let freq = u32::from(self.ram[base])
            | u32::from(self.ram[base + 2]) << 8
            | u32::from(self.ram[base + 4] & 3) << 16;
        let length = 256 - u32::from(self.ram[base + 4] & 0xFC);
        let phase = u32::from(self.ram[base + 1])
            | u32::from(self.ram[base + 3]) << 8
            | u32::from(self.ram[base + 5]) << 16;
        let phase = (phase + freq) % (length << 16);
        self.ram[base + 1] = phase as u8;
        self.ram[base + 3] = (phase >> 8) as u8;
        self.ram[base + 5] = (phase >> 16) as u8;
        let at = (phase >> 16) as u8;
        let index = usize::from(at.wrapping_add(self.ram[base + 6]));
        let sample = i32::from(self.ram[index >> 1] >> ((index & 1) * 4) & 15);
        let volume = i32::from(self.ram[base + 7] & 15);
        let out = (sample - BIAS) * volume;
        self.out_channel = c;
        self.out_value = out;
        self.last[usize::from(c)] = out;
        self.next_step = te + UPDATE;
    }

    /// The level of each channel for a meter: the size of its last output.
    pub(crate) fn levels(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        for (slot, v) in out.iter_mut().zip(self.last) {
            *slot = v.unsigned_abs() as u8;
        }
        out
    }

    fn step_address(&mut self) {
        if self.auto_increment {
            self.addr = (self.addr + 1).min(0x7F);
        }
    }

    /// A read of the data port.
    pub(crate) fn read_data(&mut self) -> u8 {
        let v = self.ram[usize::from(self.addr)];
        self.step_address();
        v
    }

    /// A store to the data port.
    pub(crate) fn write_data(&mut self, v: u8) {
        self.ram[usize::from(self.addr)] = v;
        self.step_address();
    }

    /// A store to the address port.
    pub(crate) fn write_address(&mut self, v: u8) {
        self.addr = v & 0x7F;
        self.auto_increment = v & 0x80 != 0;
    }

    /// The value at the data port with no side effect.
    pub(crate) fn peek_data(&self) -> u8 {
        self.ram[usize::from(self.addr)]
    }
}
