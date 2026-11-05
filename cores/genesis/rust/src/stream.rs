//! The DAC streams of a VGM file (commands 0x90 to 0x95): a stream sends the
//! bytes of a data bank to one register of a chip at a fixed rate.

use crate::machine::UNIT;

/// The chip number that the setup command uses for the YM2612.
pub(crate) const YM2612: u8 = 0x02;

/// The highest write rate. A corrupt file can name any 32-bit rate, and the
/// writes of a looping stream would then never catch up with the clock.
const MAX_HZ: u32 = 1_000_000;

/// How a start command counts the writes to make.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Length {
    /// This many writes.
    Writes(u64),
    /// Until the position leaves the bank.
    ToEnd,
}

#[derive(Clone, Debug)]
pub(crate) struct Stream {
    pub chip: u8,
    pub second: bool,
    pub port: u8,
    pub register: u8,
    bank: u8,
    step: usize,
    base: usize,
    hz: u32,
    running: bool,
    /// The index in the bank of the next byte.
    pos: i64,
    length: Length,
    looping: bool,
    reverse: bool,
    /// Where a loop starts again, and how long it runs.
    origin: i64,
    full: Length,
    /// The time of write number zero, and the writes done since.
    started_at: u64,
    written: u64,
}

impl Stream {
    pub fn new(chip: u8, second: bool, port: u8, register: u8) -> Stream {
        Stream {
            chip,
            second,
            port,
            register,
            bank: 0,
            step: 1,
            base: 0,
            hz: 0,
            running: false,
            pos: 0,
            length: Length::ToEnd,
            looping: false,
            reverse: false,
            origin: 0,
            full: Length::ToEnd,
            started_at: 0,
            written: 0,
        }
    }

    pub fn set_data(&mut self, bank: u8, step: u8, base: u8) {
        self.bank = bank;
        self.step = usize::from(step).max(1);
        self.base = usize::from(base);
    }

    pub fn bank(&self) -> u8 {
        self.bank
    }

    pub fn set_frequency(&mut self, hz: u32) {
        if self.running {
            self.started_at = self.due().unwrap_or(self.started_at);
            self.written = 0;
        }
        self.hz = hz.min(MAX_HZ);
    }

    /// Command 0x93. `at` is `u32::MAX` to keep the position. Mode bits 0 to 3
    /// choose the length, bit 4 reverses, bit 7 loops. Mode 0 moves the
    /// position and leaves the rest as it is.
    pub fn start(&mut self, now: u64, at: u32, mode: u8, length: u32) {
        let moved = at != u32::MAX;
        if moved {
            self.pos = i64::from(at) + self.base as i64;
        }
        let kind = mode & 0x0F;
        if kind == 0 || kind > 3 {
            if moved {
                self.origin = self.pos;
            }
            return;
        }
        self.reverse = mode & 0x10 != 0;
        self.looping = mode & 0x80 != 0;
        self.length = match kind {
            1 => Length::Writes(u64::from(length)),
            2 => Length::Writes((u64::from(length) * u64::from(self.hz) + 500) / 1000),
            _ => Length::ToEnd,
        };
        self.origin = self.pos;
        self.full = self.length;
        self.begin(now);
    }

    /// Command 0x95: plays a block of the bank from its first byte (its last
    /// when reversed) to its end.
    pub fn start_block(&mut self, now: u64, block: Option<(usize, usize)>, flags: u8) {
        let Some((start, len)) = block else {
            return;
        };
        self.looping = flags & 1 != 0;
        self.reverse = flags & 0x10 != 0;
        let base = self.base.min(len);
        let writes = (len - base).div_ceil(self.step) as u64;
        self.pos = if self.reverse {
            (start + len) as i64 - 1 - base as i64
        } else {
            (start + base) as i64
        };
        self.length = Length::Writes(writes);
        self.origin = self.pos;
        self.full = self.length;
        self.begin(now);
    }

    fn begin(&mut self, now: u64) {
        self.running = true;
        self.started_at = now;
        self.written = 0;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    /// The time of the next write, if the stream is running at a rate.
    pub fn due(&self) -> Option<u64> {
        if !self.running || self.hz == 0 {
            return None;
        }
        let at = u128::from(self.written) * u128::from(UNIT) / u128::from(self.hz);
        Some(self.started_at + at as u64)
    }

    /// Takes the next byte of `data` and moves on. Returns `None` and stops
    /// the stream, or starts a loop again, when no byte is left.
    pub fn next(&mut self, data: &[u8]) -> Option<u8> {
        loop {
            let in_range = usize::try_from(self.pos).ok().filter(|p| *p < data.len());
            let left = !matches!(self.length, Length::Writes(0));
            match in_range {
                Some(p) if left => {
                    let byte = data[p];
                    let step = self.step as i64;
                    self.pos += if self.reverse { -step } else { step };
                    if let Length::Writes(n) = &mut self.length {
                        *n -= 1;
                    }
                    self.written += 1;
                    return Some(byte);
                }
                _ if self.looping && self.restart(data) => continue,
                _ => {
                    self.running = false;
                    return None;
                }
            }
        }
    }

    /// Goes back to the start of a loop. False when the loop would write
    /// nothing, which would repeat for ever.
    fn restart(&mut self, data: &[u8]) -> bool {
        self.pos = self.origin;
        self.length = self.full;
        let in_range = usize::try_from(self.pos).is_ok_and(|p| p < data.len());
        in_range && self.length != Length::Writes(0)
    }
}
