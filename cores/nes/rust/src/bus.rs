//! The CPU's view of memory: the 2 KiB of RAM, the sound registers, the 8 KiB
//! at 0x6000 and the program in eight windows of 4 KiB.

use crate::state::*;

/// Warning bits, one per kind, so that each is recorded once.
pub(crate) const WARN_STALL: u32 = 1;
pub(crate) const WARN_LOCK: u32 = 2;
pub(crate) const WARN_BRK: u32 = 4;
pub(crate) const WARN_UNSTABLE: u32 = 8;
pub(crate) const WARN_CHIP: u32 = 16;
pub(crate) const WARN_FLAGS: u32 = 32;

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

    /// Copies the 4 KiB bank `bank` of the program over one window of the FDS's
    /// RAM, counted from 0x8000, or over one of the two halves of the RAM at
    /// 0x6000 (windows 6 and 7 here stand for the halves).
    pub(crate) fn fds_load_window(&mut self, window: usize, bank: u8) {
        let from = usize::from(bank) * 0x1000;
        let mut chunk = [0u8; 0x1000];
        for (i, slot) in chunk.iter_mut().enumerate() {
            *slot = self.image.get(from + i).copied().unwrap_or(0);
        }
        if window < 6 {
            self.fds_ram[window * 0x1000..(window + 1) * 0x1000].copy_from_slice(&chunk);
        } else {
            let at = (window - 6) * 0x1000;
            self.wram[at..at + 0x1000].copy_from_slice(&chunk);
        }
    }

    /// A byte of the program as the CPU would read it at `address`.
    pub(crate) fn read_rom(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        if address <= 0xDFFF && self.apu.fds.active {
            return self.fds_ram[usize::from(address - 0x8000)];
        }
        let window = usize::from((address - 0x8000) >> 12);
        let offset = usize::from(self.banks[window]) * 0x1000 + usize::from(address & 0xFFF);
        self.image.get(offset).copied().unwrap_or(0)
    }

    /// The screen status of a test cartridge: the vertical blank flag is set
    /// once in each frame of 29781 cycles and cleared by a read.
    fn ppu_status(&self) -> u8 {
        if self.now / 29781 + 1 > self.ppu_read {
            0x80
        } else {
            0
        }
    }

    /// Reads memory as the CPU would see it, with no side effects.
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address & 0x7FF)],
            0x2002 if self.cartridge => self.ppu_status(),
            0x4015 => {
                let a = &self.apu;
                u8::from(a.pulse[0].len > 0)
                    | u8::from(a.pulse[1].len > 0) << 1
                    | u8::from(a.tri.len > 0) << 2
                    | u8::from(a.noise.len > 0) << 3
                    | u8::from(a.dmc.remaining > 0) << 4
                    | (self.bus & 0x20)
                    | u8::from(a.frame.irq) << 6
                    | u8::from(a.dmc.irq) << 7
            }
            0x4040..=0x4097 if self.apu.fds.active => {
                self.apu.fds.read(address, self.bus).unwrap_or(self.bus)
            }
            0x4800..=0x4FFF if self.apu.n163.active => self.apu.n163.peek_data(),
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)],
            0x8000..=0xFFFF => self.read_rom(address),
            _ => self.bus,
        }
    }

    /// Brings the sound unit up to date when the delta channel may read a
    /// sample byte, so that the CPU loses the cycles at the time of the read.
    fn settle_sample_reads(&mut self) {
        if self.apu.dmc.remaining > 0 && self.now >= self.apu.dmc.next_step {
            self.sync_apu();
        }
    }

    /// A read bus cycle.
    pub(crate) fn read(&mut self, address: u16) -> u8 {
        self.settle_sample_reads();
        let v = match address {
            0x0000..=0x1FFF => self.ram[usize::from(address & 0x7FF)],
            0x2000..=0x3FFF => {
                if self.cartridge && address & 7 == 2 {
                    let v = self.ppu_status();
                    self.ppu_read = self.now / 29781 + 1;
                    v
                } else {
                    self.bus
                }
            }
            0x4000..=0x401F => {
                if address == 0x4015 {
                    self.apu_read(address)
                } else if self.cartridge && address >= 0x4016 {
                    0
                } else {
                    self.bus
                }
            }
            0x4040..=0x4097 if self.apu.fds.active => {
                self.sync_apu();
                self.apu.fds.read(address, self.bus).unwrap_or(self.bus)
            }
            0x4800..=0x4FFF if self.apu.n163.active => {
                self.sync_apu();
                self.apu.n163.read_data()
            }
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)],
            0x8000..=0xFFFF => self.read_rom(address),
            _ => self.bus,
        };
        // A read of the status register leaves the bit that comes from the bus
        // where it was.
        if address != 0x4015 {
            self.bus = v;
        }
        self.now += 1;
        v
    }

    /// A write bus cycle.
    pub(crate) fn write(&mut self, address: u16, v: u8) {
        self.settle_sample_reads();
        self.bus = v;
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address & 0x7FF)] = v,
            0x4000..=0x4017 => {
                // The cycles that sample reads took are charged before the
                // store, so the observer sees the cycle it happens on.
                self.sync_apu();
                let t = self.now;
                if let Some(o) = self.observer.0.as_mut() {
                    o.sound_write(t, address, v);
                }
                if address != 0x4014 && address != 0x4016 {
                    self.apu_write(address, v);
                }
            }
            0x9000..=0xBFFF if self.chips & oscilla_nsf::chips::VRC6 != 0 => {
                self.sync_apu();
                let t = self.now;
                if let Some(o) = self.observer.0.as_mut() {
                    o.sound_write(t, address, v);
                }
                self.vrc6_write(address, v);
            }
            0x4040..=0x4097 if self.chips & oscilla_nsf::chips::FDS != 0 => {
                self.sync_apu();
                let t = self.now;
                if let Some(o) = self.observer.0.as_mut() {
                    o.sound_write(t, address, v);
                }
                self.apu.fds.write(address, v);
            }
            0x8000..=0xDFFF if self.chips & oscilla_nsf::chips::FDS != 0 => {
                self.fds_ram[usize::from(address - 0x8000)] = v;
            }
            0x5FF6..=0x5FFD if self.banked && self.chips & oscilla_nsf::chips::FDS != 0 => {
                // The banks of the FDS's RAM hold copies of the program.
                let window = match address {
                    0x5FF6 | 0x5FF7 => usize::from(address - 0x5FF6) + 6,
                    _ => usize::from(address - 0x5FF8),
                };
                self.fds_load_window(window, v);
            }
            0x4800..=0x4FFF if self.chips & oscilla_nsf::chips::N163 != 0 => {
                self.sync_apu();
                let t = self.now;
                if let Some(o) = self.observer.0.as_mut() {
                    o.sound_write(t, address, v);
                }
                self.apu.n163.write_data(v);
            }
            0xF800..=0xFFFF if self.chips & oscilla_nsf::chips::N163 != 0 => {
                self.sync_apu();
                let t = self.now;
                if let Some(o) = self.observer.0.as_mut() {
                    o.sound_write(t, address, v);
                }
                self.apu.n163.write_address(v);
                if self.chips & oscilla_nsf::chips::S5B != 0 {
                    self.s5b_write(address, v);
                }
            }
            0xC000..=0xFFFF if self.chips & oscilla_nsf::chips::S5B != 0 => {
                self.sync_apu();
                let t = self.now;
                if let Some(o) = self.observer.0.as_mut() {
                    o.sound_write(t, address, v);
                }
                self.s5b_write(address, v);
            }
            0x5FF8..=0x5FFF if self.banked => {
                self.banks[usize::from(address - 0x5FF8)] = v;
            }
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)] = v,
            _ => {}
        }
        self.now += 1;
    }
}
