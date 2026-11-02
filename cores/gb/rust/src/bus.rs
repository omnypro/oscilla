//! The address space: cartridge ROM with a bank register, RAM, the timer, the
//! interrupt registers and the sound registers. Every access costs one
//! machine cycle (4 clocks) and happens at the end of it.

use crate::state::*;

/// Warning bits, one per kind, so that each is recorded once.
pub(crate) const WARN_STALL: u32 = 1;
pub(crate) const WARN_HALT: u32 = 2;
pub(crate) const WARN_LOCK: u32 = 4;
pub(crate) const WARN_STOP: u32 = 8;
pub(crate) const WARN_SPEED: u32 = 16;

impl Machine {
    pub(crate) fn warn(&mut self, bit: u32, text: &str) {
        if self.warned & bit != 0 {
            return;
        }
        self.warned |= bit;
        // The list holds at most 2048 bytes and its room is reserved, so a
        // pointer to it stays valid.
        if self.warnings.len() + text.len() + 1 > 2048 {
            return;
        }
        self.warnings.pop();
        self.warnings.extend_from_slice(text.as_bytes());
        self.warnings.push(b'\n');
        self.warnings.push(0);
    }

    fn rom_byte(&self, linear: usize) -> u8 {
        let load = usize::from(self.load);
        if linear < load {
            return 0;
        }
        self.rom.get(linear - load).copied().unwrap_or(0)
    }

    // ---- The system counter and the timer -------------------------------

    pub(crate) fn sys(&self, t: u64) -> u16 {
        (t.wrapping_add(self.timer.sys_offset) & 0xFFFF) as u16
    }

    /// The bit of the system counter that TIMA counts the falling edges of.
    pub(crate) fn timer_bit_of(tac: u8) -> u32 {
        Self::timer_bit(tac)
    }

    fn timer_bit(tac: u8) -> u32 {
        [9, 3, 5, 7][usize::from(tac & 3)]
    }

    /// TIMA at cycle `t`, and whether it overflowed since it was last synced.
    pub(crate) fn timer_at(&self, t: u64) -> (u8, bool) {
        let tm = &self.timer;
        if t <= tm.synced || tm.tac & 4 == 0 {
            return (tm.tima, false);
        }
        let shift = Self::timer_bit(tm.tac) + 1;
        let off = tm.tima_offset;
        let edges = ((t + off) >> shift) - ((tm.synced + off) >> shift);
        if edges == 0 {
            return (tm.tima, false);
        }
        let tima = u64::from(tm.tima);
        if tima + edges <= 0xFF {
            return ((tima + edges) as u8, false);
        }
        let after_first = tima + edges - 0x100;
        let period = 0x100 - u64::from(tm.tma);
        ((u64::from(tm.tma) + after_first % period) as u8, true)
    }

    /// Brings TIMA up to cycle `t`.
    pub(crate) fn timer_sync(&mut self, t: u64) {
        if t <= self.timer.synced {
            return;
        }
        let (tima, overflowed) = self.timer_at(t);
        self.timer.synced = t;
        self.timer.tima = tima;
        if overflowed {
            self.iflag |= 4;
        }
    }

    /// The display line at cycle `t`: 144 when a routine starts.
    pub(crate) fn ly_at(&self, t: u64) -> u8 {
        (((t - self.ly_origin.min(t)) / 456 + 144) % 154) as u8
    }

    fn write_tac(&mut self, v: u8) {
        let t = self.now;
        self.timer_sync(t);
        // Changing the selected bit from set to clear counts an edge.
        let before = self.timer.tac;
        let bit_set = |tac: u8, m: &Machine| {
            tac & 4 != 0 && ((t + m.timer.tima_offset) & 0xFFFF) >> Self::timer_bit(tac) & 1 != 0
        };
        let was = bit_set(before, self);
        let now = bit_set(v, self);
        self.timer.tac = v & 7;
        if was && !now {
            self.timer_bump();
        }
    }

    fn timer_bump(&mut self) {
        if self.timer.tima == 0xFF {
            self.timer.tima = self.timer.tma;
            self.iflag |= 4;
        } else {
            self.timer.tima += 1;
        }
    }

    fn write_div(&mut self) {
        let t = self.now;
        self.apu_advance(t);
        self.timer_sync(t);
        let sys = self.sys(t);
        // The APU's frame sequencer ticks on bit 12 falling; so does TIMA's
        // selected bit.
        if sys & 0x1000 != 0 {
            self.apu_frame_sequencer_tick();
            self.apu_emit(t);
        }
        let counter = ((t + self.timer.tima_offset) & 0xFFFF) as u16;
        if self.timer.tac & 4 != 0 && (counter >> Self::timer_bit(self.timer.tac)) & 1 != 0 {
            self.timer_bump();
        }
        self.timer.sys_offset = (0x10000 - (t & 0xFFFF)) & 0xFFFF;
        self.timer.tima_offset = self.timer.sys_offset;
    }

    // ---- Reads and writes -----------------------------------------------

    /// One machine cycle that reads `a`.
    pub(crate) fn rd(&mut self, a: u16) -> u8 {
        self.now += 4;
        self.read_at(a)
    }

    /// One machine cycle that writes `v` to `a`.
    pub(crate) fn wr(&mut self, a: u16, v: u8) {
        self.now += 4;
        self.write_at(a, v);
    }

    /// One machine cycle with no access.
    pub(crate) fn idle(&mut self) {
        self.now += 4;
    }

    pub(crate) fn read_at(&mut self, a: u16) -> u8 {
        match a {
            0x0000..=0x3FFF => self.rom_byte(usize::from(a)),
            0x4000..=0x7FFF => self.rom_byte(self.bank as usize * 0x4000 + usize::from(a - 0x4000)),
            0x8000..=0xDFFF | 0xFE00..=0xFE9F => self.mem[usize::from(a)],
            0xE000..=0xFDFF => self.mem[usize::from(a - 0x2000)],
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00 => 0xFF,
            0xFF04 => {
                let t = self.now;
                (self.sys(t) >> 8) as u8
            }
            0xFF05 => {
                let t = self.now;
                self.timer_sync(t);
                self.timer.tima
            }
            0xFF06 => self.timer.tma,
            0xFF07 => self.timer.tac | 0xF8,
            0xFF0F => {
                let t = self.now;
                self.timer_sync(t);
                self.iflag | 0xE0
            }
            0xFF10..=0xFF3F => self.apu_read(a),
            0xFF44 => self.ly_at(self.now),
            0xFF80..=0xFFFE | 0xFF01..=0xFF03 | 0xFF40..=0xFF7F => self.mem[usize::from(a)],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }

    pub(crate) fn write_at(&mut self, a: u16, v: u8) {
        match a {
            0x2000..=0x3FFF => self.bank = u32::from(if v == 0 { 1 } else { v }),
            0x0000..=0x7FFF => {}
            0x8000..=0xDFFF | 0xFE00..=0xFE9F => self.mem[usize::from(a)] = v,
            0xE000..=0xFDFF => self.mem[usize::from(a - 0x2000)] = v,
            0xFF02 => {
                self.mem[0xFF02] = v;
                if v & 0x81 == 0x81 {
                    let byte = self.mem[0xFF01];
                    // Only a cartridge image reads its serial output.
                    if self.interrupts && self.serial.len() < 1 << 20 {
                        self.serial.push(byte);
                    }
                    self.mem[0xFF02] = v & 0x7F;
                    self.iflag |= 8;
                }
            }
            0xFF04 => self.write_div(),
            0xFF05 => {
                let t = self.now;
                self.timer_sync(t);
                self.timer.tima = v;
            }
            0xFF06 => {
                let t = self.now;
                self.timer_sync(t);
                self.timer.tma = v;
            }
            0xFF07 => self.write_tac(v),
            0xFF0F => {
                let t = self.now;
                self.timer_sync(t);
                self.iflag = v & 0x1F;
            }
            0xFF10..=0xFF3F => {
                if let Some(o) = self.observer.0.as_mut() {
                    o.sound_write(self.now, a, v);
                }
                self.apu_write(a, v);
            }
            // RAM banking registers are ignored.
            0xFF70 => {}
            0xFF80..=0xFFFE | 0xFF00 | 0xFF01 | 0xFF03 | 0xFF40..=0xFF6F | 0xFF71..=0xFF7F => {
                self.mem[usize::from(a)] = v
            }
            0xFFFF => self.ie = v,
            _ => {}
        }
    }
}
