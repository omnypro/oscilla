//! The memory map. Every access goes through here, so the open-bus rules,
//! the protection of BIOS reads, the mirrors and the 8-bit video write rule live
//! in one place.

use crate::state::*;

#[inline]
fn ld32(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}
#[inline]
fn ld16(p: &[u8], at: usize) -> u32 {
    u32::from(u16::from_le_bytes([p[at], p[at + 1]]))
}
#[inline]
fn st32(p: &mut [u8], at: usize, v: u32) {
    p[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn st16(p: &mut [u8], at: usize, v: u16) {
    p[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

#[inline]
fn vram_offset(addr: u32) -> usize {
    let mut o = addr & 0x1_FFFF;
    if o >= 0x1_8000 {
        o -= 0x8000;
    }
    o as usize
}

const EW: u32 = EWRAM_SIZE as u32;
const IW: u32 = IWRAM_SIZE as u32;
const PAL: u32 = PAL_SIZE as u32;
const OAMS: u32 = OAM_SIZE as u32;

impl Machine {
    /// Cycles one access takes: width 2 or 4 (a byte costs what a halfword
    /// does), sequential or not, from the WAITCNT tables.
    #[inline]
    pub(crate) fn bus_cycles(&self, addr: u32, width: u32, seq: bool) -> i32 {
        let r = (addr >> 24) as usize;
        if r > 0xF {
            return 1;
        }
        i32::from(if width == 4 {
            if seq {
                self.wait_s32[r]
            } else {
                self.wait_n32[r]
            }
        } else if seq {
            self.wait_s16[r]
        } else {
            self.wait_n16[r]
        })
    }

    pub(crate) fn bus_update_waits(&mut self) {
        const FIRST: [i32; 4] = [4, 3, 2, 8];
        let w = u32::from(self.io[0x204]) | (u32::from(self.io[0x205]) << 8);
        let sram = FIRST[(w & 3) as usize];
        let n0 = FIRST[((w >> 2) & 3) as usize];
        let s0 = if w & 0x10 != 0 { 1 } else { 2 };
        let n1 = FIRST[((w >> 5) & 3) as usize];
        let s1 = if w & 0x80 != 0 { 1 } else { 4 };
        let n2 = FIRST[((w >> 8) & 3) as usize];
        let s2 = if w & 0x400 != 0 { 1 } else { 8 };
        for r in 0..16usize {
            let (mut n, mut s, mut bus16) = (1, 1, false);
            match r {
                0x2 => {
                    n = 3;
                    s = 3;
                    bus16 = true;
                }
                0x5 | 0x6 => bus16 = true,
                0x8 | 0x9 => {
                    n = 1 + n0;
                    s = 1 + s0;
                    bus16 = true;
                }
                0xA | 0xB => {
                    n = 1 + n1;
                    s = 1 + s1;
                    bus16 = true;
                }
                0xC | 0xD => {
                    n = 1 + n2;
                    s = 1 + s2;
                    bus16 = true;
                }
                0xE | 0xF => {
                    n = 1 + sram;
                    s = 1 + sram;
                }
                _ => {}
            }
            self.wait_n16[r] = n as u8;
            self.wait_s16[r] = s as u8;
            self.wait_n32[r] = (if bus16 { n + s } else { n }) as u8;
            self.wait_s32[r] = (if bus16 { 2 * s } else { s }) as u8;
        }
        self.prefetch = w & 0x4000 != 0;
    }

    /// Code reads used by the open-bus rule: they bypass the BIOS protection
    /// because they model what the CPU itself already fetched.
    fn code_peek16(&self, addr: u32) -> u32 {
        let addr = addr & !1;
        match addr >> 24 {
            0x00 => {
                if addr < BIOS_SIZE {
                    ld16(&*self.bios, addr as usize)
                } else {
                    0
                }
            }
            0x02 => ld16(&*self.ewram, (addr & (EW - 1)) as usize),
            0x03 => ld16(&*self.iwram, (addr & (IW - 1)) as usize),
            0x05 => ld16(&*self.pal, (addr & (PAL - 1)) as usize),
            0x06 => ld16(&*self.vram, vram_offset(addr)),
            0x07 => ld16(&*self.oam, (addr & (OAMS - 1)) as usize),
            0x08..=0x0D => {
                let o = addr & (ROM_MAX - 1);
                if (o as usize) + 1 < self.rom.len() {
                    ld16(&self.rom, o as usize)
                } else {
                    (o >> 1) & 0xFFFF
                }
            }
            _ => 0,
        }
    }

    /// What a read of unmapped memory returns: the opcode the CPU prefetched.
    pub(crate) fn bus_open(&self) -> u32 {
        let pc = self.cpu.cur_pc;
        if self.cpu.cpsr & PSR_T == 0 {
            return self.code_peek16(pc.wrapping_add(8))
                | (self.code_peek16(pc.wrapping_add(10)) << 16);
        }
        let (lo, hi);
        match pc >> 24 {
            0x00 | 0x07 => {
                if pc & 2 == 0 {
                    lo = self.code_peek16(pc.wrapping_add(4));
                    hi = self.code_peek16(pc.wrapping_add(6));
                } else {
                    lo = self.code_peek16(pc.wrapping_add(2));
                    hi = self.code_peek16(pc.wrapping_add(4));
                }
            }
            0x03 => {
                if pc & 2 == 0 {
                    lo = self.code_peek16(pc.wrapping_add(4));
                    hi = self.code_peek16(pc.wrapping_add(2));
                } else {
                    lo = self.code_peek16(pc.wrapping_add(2));
                    hi = self.code_peek16(pc.wrapping_add(4));
                }
            }
            _ => {
                lo = self.code_peek16(pc.wrapping_add(4));
                hi = lo;
            }
        }
        lo | (hi << 16)
    }

    fn bios_read32(&self, addr: u32) -> u32 {
        if self.cpu.cur_pc < BIOS_SIZE {
            return ld32(&*self.bios, (addr & 0x3FFC) as usize);
        }
        self.bios_latch
    }

    /// I/O reads above 0x04000400 that are not the one mirrored port.
    fn io_mapped(addr: u32) -> bool {
        (addr & 0x00FF_FFFF) < IO_SIZE
    }

    pub(crate) fn bus_read32(&self, addr: u32) -> u32 {
        let addr = addr & !3;
        match addr >> 24 {
            0x00 => {
                if addr < BIOS_SIZE {
                    self.bios_read32(addr)
                } else {
                    self.bus_open()
                }
            }
            0x02 => ld32(&*self.ewram, (addr & (EW - 1)) as usize),
            0x03 => ld32(&*self.iwram, (addr & (IW - 1)) as usize),
            0x04 => {
                if !Self::io_mapped(addr) {
                    return self.bus_open();
                }
                u32::from(self.io_read16(addr)) | (u32::from(self.io_read16(addr + 2)) << 16)
            }
            0x05 => ld32(&*self.pal, (addr & (PAL - 1)) as usize),
            0x06 => ld32(&*self.vram, vram_offset(addr)),
            0x07 => ld32(&*self.oam, (addr & (OAMS - 1)) as usize),
            0x08..=0x0D => {
                let o = addr & (ROM_MAX - 1);
                if (o as usize) + 3 < self.rom.len() {
                    return ld32(&self.rom, o as usize);
                }
                ((o >> 1) & 0xFFFF) | ((((o + 2) >> 1) & 0xFFFF) << 16)
            }
            0x0E | 0x0F => 0xFFFF_FFFF,
            _ => self.bus_open(),
        }
    }

    pub(crate) fn bus_read16(&self, addr: u32) -> u32 {
        let addr = addr & !1;
        let half = (addr & 2) * 8;
        match addr >> 24 {
            0x00 => {
                if addr < BIOS_SIZE {
                    (self.bios_read32(addr) >> half) & 0xFFFF
                } else {
                    (self.bus_open() >> half) & 0xFFFF
                }
            }
            0x02 => ld16(&*self.ewram, (addr & (EW - 1)) as usize),
            0x03 => ld16(&*self.iwram, (addr & (IW - 1)) as usize),
            0x04 => {
                if !Self::io_mapped(addr) {
                    return (self.bus_open() >> half) & 0xFFFF;
                }
                u32::from(self.io_read16(addr))
            }
            0x05 => ld16(&*self.pal, (addr & (PAL - 1)) as usize),
            0x06 => ld16(&*self.vram, vram_offset(addr)),
            0x07 => ld16(&*self.oam, (addr & (OAMS - 1)) as usize),
            0x08..=0x0D => {
                let o = addr & (ROM_MAX - 1);
                if (o as usize) + 1 < self.rom.len() {
                    return ld16(&self.rom, o as usize);
                }
                (o >> 1) & 0xFFFF
            }
            0x0E | 0x0F => 0xFFFF,
            _ => (self.bus_open() >> half) & 0xFFFF,
        }
    }

    pub(crate) fn bus_read8(&self, addr: u32) -> u32 {
        let byte = (addr & 3) * 8;
        match addr >> 24 {
            0x00 => {
                if addr < BIOS_SIZE {
                    (self.bios_read32(addr) >> byte) & 0xFF
                } else {
                    (self.bus_open() >> byte) & 0xFF
                }
            }
            0x02 => u32::from(self.ewram[(addr & (EW - 1)) as usize]),
            0x03 => u32::from(self.iwram[(addr & (IW - 1)) as usize]),
            0x04 => {
                if !Self::io_mapped(addr) {
                    return (self.bus_open() >> byte) & 0xFF;
                }
                (u32::from(self.io_read16(addr & !1)) >> ((addr & 1) * 8)) & 0xFF
            }
            0x05 => u32::from(self.pal[(addr & (PAL - 1)) as usize]),
            0x06 => u32::from(self.vram[vram_offset(addr)]),
            0x07 => u32::from(self.oam[(addr & (OAMS - 1)) as usize]),
            0x08..=0x0D => {
                let o = addr & (ROM_MAX - 1);
                if (o as usize) < self.rom.len() {
                    return u32::from(self.rom[o as usize]);
                }
                ((o >> 1) >> ((o & 1) * 8)) & 0xFF
            }
            0x0E | 0x0F => 0xFF,
            _ => (self.bus_open() >> byte) & 0xFF,
        }
    }

    pub(crate) fn bus_fetch32(&self, addr: u32) -> u32 {
        match addr >> 24 {
            0x08..=0x0D => {
                let o = addr & (ROM_MAX - 4);
                if (o as usize) + 3 < self.rom.len() {
                    return ld32(&self.rom, o as usize);
                }
            }
            0x03 => return ld32(&*self.iwram, (addr & (IW - 4)) as usize),
            0x02 => return ld32(&*self.ewram, (addr & (EW - 4)) as usize),
            0x00 => {
                if addr < BIOS_SIZE {
                    return ld32(&*self.bios, (addr & 0x3FFC) as usize);
                }
            }
            _ => {}
        }
        self.bus_read32(addr)
    }

    pub(crate) fn bus_fetch16(&self, addr: u32) -> u32 {
        match addr >> 24 {
            0x08..=0x0D => {
                let o = addr & (ROM_MAX - 2);
                if (o as usize) + 1 < self.rom.len() {
                    return ld16(&self.rom, o as usize);
                }
            }
            0x03 => return ld16(&*self.iwram, (addr & (IW - 2)) as usize),
            0x02 => return ld16(&*self.ewram, (addr & (EW - 2)) as usize),
            0x00 => {
                if addr < BIOS_SIZE {
                    return ld16(&*self.bios, (addr & 0x3FFE) as usize);
                }
            }
            _ => {}
        }
        self.bus_read16(addr)
    }

    fn trace_store(&mut self, addr: u32, value: u32, width: i32) {
        if (0x0400_0060..0x0400_00A8).contains(&addr) {
            let now = self.now;
            if let Some(o) = self.observer.0.as_mut() {
                o.sound_write(now, addr, value, width);
            }
        }
    }

    /// Bitmap modes (3, 4 and 5) move the BG/OBJ boundary in VRAM up to 0x14000.
    fn vram_bg_limit(&self) -> usize {
        if self.io[0] & 7 >= 3 {
            0x14000
        } else {
            0x10000
        }
    }

    pub(crate) fn bus_write32(&mut self, addr: u32, value: u32, _source: Source) {
        let addr = addr & !3;
        match addr >> 24 {
            0x02 => st32(&mut *self.ewram, (addr & (EW - 1)) as usize, value),
            0x03 => st32(&mut *self.iwram, (addr & (IW - 1)) as usize, value),
            0x04 => {
                if !Self::io_mapped(addr) {
                    return;
                }
                self.trace_store(addr, value, 4);
                if (addr & 0x3FC) == 0xA0 || (addr & 0x3FC) == 0xA4 {
                    self.apu_fifo_push(addr, value, 4);
                    return;
                }
                self.io_write16(addr, value as u16);
                self.io_write16(addr + 2, (value >> 16) as u16);
            }
            0x05 => st32(&mut *self.pal, (addr & (PAL - 1)) as usize, value),
            0x06 => st32(&mut *self.vram, vram_offset(addr), value),
            0x07 => st32(&mut *self.oam, (addr & (OAMS - 1)) as usize, value),
            _ => {}
        }
    }

    pub(crate) fn bus_write16(&mut self, addr: u32, value: u32, _source: Source) {
        let addr = addr & !1;
        let value = value & 0xFFFF;
        match addr >> 24 {
            0x02 => st16(&mut *self.ewram, (addr & (EW - 1)) as usize, value as u16),
            0x03 => st16(&mut *self.iwram, (addr & (IW - 1)) as usize, value as u16),
            0x04 => {
                if !Self::io_mapped(addr) {
                    return;
                }
                self.trace_store(addr, value, 2);
                if (addr & 0x3F8) == 0xA0 {
                    self.apu_fifo_push(addr, value, 2);
                    return;
                }
                self.io_write16(addr, value as u16);
            }
            0x05 => st16(&mut *self.pal, (addr & (PAL - 1)) as usize, value as u16),
            0x06 => st16(&mut *self.vram, vram_offset(addr), value as u16),
            0x07 => st16(&mut *self.oam, (addr & (OAMS - 1)) as usize, value as u16),
            _ => {}
        }
    }

    pub(crate) fn bus_write8(&mut self, addr: u32, value: u32, _source: Source) {
        let value = value & 0xFF;
        match addr >> 24 {
            0x02 => self.ewram[(addr & (EW - 1)) as usize] = value as u8,
            0x03 => self.iwram[(addr & (IW - 1)) as usize] = value as u8,
            0x04 => {
                if !Self::io_mapped(addr) {
                    return;
                }
                self.trace_store(addr, value, 1);
                if (addr & 0x3F8) == 0xA0 {
                    self.apu_fifo_push(addr, value, 1);
                    return;
                }
                self.io_write8(addr, value as u8);
            }
            0x05 => {
                // An 8-bit store to palette writes the byte to both halves.
                st16(
                    &mut *self.pal,
                    (addr & (PAL - 2)) as usize,
                    (value * 0x101) as u16,
                );
            }
            0x06 => {
                // Same rule for BG VRAM; OBJ VRAM ignores 8-bit stores.
                let o = vram_offset(addr) & !1;
                if o < self.vram_bg_limit() {
                    st16(&mut *self.vram, o, (value * 0x101) as u16);
                }
            }
            _ => {} // OAM ignores 8-bit stores; ROM, BIOS and SRAM are read-only here.
        }
    }
}
