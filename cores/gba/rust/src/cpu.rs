//! ARM7TDMI interpreter (ARMv4T): the ARM and Thumb instruction sets, the
//! barrel shifter, banked registers and exception entry. r15 reads as the
//! executing instruction's address plus 8 in ARM state and plus 4 in Thumb
//! state, the value the three-stage pipeline exposes. Each step returns the
//! cycles the instruction took on the bus. The timers and the display advance
//! by that amount.

use crate::state::*;

#[inline]
fn ror32(v: u32, n: u32) -> u32 {
    let n = n & 31;
    v.rotate_right(n)
}

#[inline]
fn popcount16(v: u32) -> i32 {
    (v & 0xFFFF).count_ones() as i32
}

#[inline]
fn bank_of(mode: u32) -> usize {
    match mode {
        MODE_FIQ => 1,
        MODE_IRQ => 2,
        MODE_SVC => 3,
        MODE_ABT => 4,
        MODE_UND => 5,
        _ => 0,
    }
}

fn cond_ok(psr: u32, cond: u32) -> bool {
    let n = (psr >> 31) & 1 != 0;
    let z = (psr >> 30) & 1 != 0;
    let cy = (psr >> 29) & 1 != 0;
    let v = (psr >> 28) & 1 != 0;
    match cond {
        0x0 => z,
        0x1 => !z,
        0x2 => cy,
        0x3 => !cy,
        0x4 => n,
        0x5 => !n,
        0x6 => v,
        0x7 => !v,
        0x8 => cy && !z,
        0x9 => !cy || z,
        0xA => n == v,
        0xB => n != v,
        0xC => !z && n == v,
        0xD => z || n != v,
        0xE => true,
        _ => false,
    }
}

/// Barrel shifter. `imm` selects the immediate-amount encodings, where an
/// amount of 0 means LSL #0, LSR #32, ASR #32 or RRX. A register amount of 0
/// passes the value and the carry through. Returns the result and the carry.
#[inline]
fn shifter(kind: u32, v: u32, mut amt: u32, imm: bool, cin: u32) -> (u32, u32) {
    match kind {
        0 => {
            if amt == 0 {
                (v, cin)
            } else if amt < 32 {
                (v << amt, (v >> (32 - amt)) & 1)
            } else if amt == 32 {
                (0, v & 1)
            } else {
                (0, 0)
            }
        }
        1 => {
            if amt == 0 {
                if !imm {
                    return (v, cin);
                }
                amt = 32;
            }
            if amt < 32 {
                (v >> amt, (v >> (amt - 1)) & 1)
            } else if amt == 32 {
                (0, v >> 31)
            } else {
                (0, 0)
            }
        }
        2 => {
            if amt == 0 {
                if !imm {
                    return (v, cin);
                }
                amt = 32;
            }
            if amt < 32 {
                (((v as i32) >> amt) as u32, (v >> (amt - 1)) & 1)
            } else {
                (if v >> 31 != 0 { 0xFFFF_FFFF } else { 0 }, v >> 31)
            }
        }
        _ => {
            if amt == 0 {
                if !imm {
                    return (v, cin);
                }
                return ((cin << 31) | (v >> 1), v & 1);
            }
            amt &= 31;
            if amt == 0 {
                return (v, v >> 31);
            }
            (ror32(v, amt), (v >> (amt - 1)) & 1)
        }
    }
}

/// The sum, the carry out and the overflow flag.
#[inline]
fn add_flags(a: u32, b: u32, cin: u32) -> (u32, u32, u32) {
    let s = u64::from(a) + u64::from(b) + u64::from(cin);
    let r = s as u32;
    (r, (s >> 32) as u32, ((!(a ^ b)) & (a ^ r)) >> 31)
}

impl CpuState {
    #[inline]
    fn set_nz(&mut self, r: u32) {
        self.cpsr = (self.cpsr & !(PSR_N | PSR_Z)) | (r & PSR_N) | if r != 0 { 0 } else { PSR_Z };
    }
    #[inline]
    fn set_nzc(&mut self, r: u32, cy: u32) {
        self.cpsr = (self.cpsr & !(PSR_N | PSR_Z | PSR_C))
            | (r & PSR_N)
            | if r != 0 { 0 } else { PSR_Z }
            | (cy << 29);
    }
    #[inline]
    fn set_nzcv(&mut self, r: u32, cy: u32, v: u32) {
        self.cpsr = (self.cpsr & !(PSR_N | PSR_Z | PSR_C | PSR_V))
            | (r & PSR_N)
            | if r != 0 { 0 } else { PSR_Z }
            | (cy << 29)
            | (v << 28);
    }
    #[inline]
    fn has_spsr(&self) -> bool {
        bank_of(self.cpsr & PSR_MODE) != 0
    }
}

fn mul_cycles(rs: u32, is_signed: bool) -> i32 {
    if (rs >> 8) == 0 || (is_signed && (rs >> 8) == 0xFF_FFFF) {
        return 1;
    }
    if (rs >> 16) == 0 || (is_signed && (rs >> 16) == 0xFFFF) {
        return 2;
    }
    if (rs >> 24) == 0 || (is_signed && (rs >> 24) == 0xFF) {
        return 3;
    }
    4
}

impl Machine {
    pub(crate) fn cpu_set_mode(&mut self, mode: u32) {
        let c = &mut self.cpu;
        let ob = bank_of(c.cpsr & PSR_MODE);
        let nb = bank_of(mode);
        if ob != nb {
            c.bank_sp[ob] = c.r[13];
            c.bank_lr[ob] = c.r[14];
            c.bank_spsr[ob] = c.spsr;
            if ob == 1 {
                for i in 0..5 {
                    c.fiq_hi[i] = c.r[8 + i];
                    c.r[8 + i] = c.usr_hi[i];
                }
            } else if nb == 1 {
                for i in 0..5 {
                    c.usr_hi[i] = c.r[8 + i];
                    c.r[8 + i] = c.fiq_hi[i];
                }
            }
            c.r[13] = c.bank_sp[nb];
            c.r[14] = c.bank_lr[nb];
            c.spsr = c.bank_spsr[nb];
        }
        c.cpsr = (c.cpsr & !PSR_MODE) | (mode & PSR_MODE);
    }

    /// Replaces the whole CPSR. It switches register banks first.
    fn write_cpsr(&mut self, value: u32) {
        self.cpu_set_mode(value & PSR_MODE);
        self.cpu.cpsr = value;
    }

    pub(crate) fn cpu_raise_exception(&mut self, vector: u32, mode: u32, lr: u32) {
        let old = self.cpu.cpsr;
        self.cpu_set_mode(mode);
        let c = &mut self.cpu;
        c.spsr = old;
        c.r[14] = lr;
        c.cpsr = (c.cpsr & !PSR_T) | PSR_I;
        c.next_pc = vector;
        c.pc_written = true;
    }

    fn cpu_take_irq(&mut self) {
        let lr = self.cpu.next_pc.wrapping_add(4);
        self.cpu_raise_exception(0x18, MODE_IRQ, lr);
    }

    pub(crate) fn cpu_reset_post_bios(&mut self) {
        let entry = self.entry;
        let c = &mut self.cpu;
        c.r = [0; 16];
        c.bank_sp = [0; 6];
        c.bank_lr = [0; 6];
        c.bank_spsr = [0; 6];
        c.fiq_hi = [0; 5];
        c.usr_hi = [0; 5];
        c.spsr = 0;
        // The BIOS hands over in system mode with the stacks its SoftReset sets.
        c.cpsr = MODE_SYS;
        c.bank_sp[bank_of(MODE_SVC)] = 0x0300_7FE0;
        c.bank_sp[bank_of(MODE_IRQ)] = 0x0300_7FA0;
        c.r[13] = 0x0300_7F00;
        c.next_pc = entry;
        c.cur_pc = entry;
        c.pc_written = false;
        c.fetch_n = false;
        c.halted = false;
    }

    /// Cycles the pipeline refill costs after a jump to `target`.
    #[inline]
    fn refill(&self, target: u32, thumb: bool) -> i32 {
        let w = if thumb { 2 } else { 4 };
        self.bus_cycles(target, w, false) + self.bus_cycles(target.wrapping_add(w), w, true)
    }

    #[inline]
    fn branch_to(&mut self, target: u32) -> i32 {
        let thumb = self.cpu.cpsr & PSR_T != 0;
        let target = target & if thumb { !1 } else { !3 };
        self.cpu.next_pc = target;
        self.cpu.pc_written = true;
        self.pf_addr = target;
        self.pf_fresh = true;
        self.refill(target, thumb)
    }

    /// A data access. A load's code fetch that follows is non-sequential; a
    /// data access to the cartridge also empties the prefetch buffer.
    #[inline]
    fn data_cycles(&mut self, addr: u32, width: u32) -> i32 {
        let r = addr >> 24;
        if (0x8..=0xD).contains(&r) {
            self.pf_addr = 1; // odd, so it matches no fetch
        }
        self.cpu.fetch_n = true;
        self.bus_cycles(addr, width, false)
    }

    /// A load: its data access and the internal cycle that ends it. When the
    /// prefetch buffer is running ahead of cartridge code, that internal cycle
    /// overlaps the buffer's work and costs nothing. A data access to the
    /// cartridge has just emptied the buffer, so then the cycle is paid.
    #[inline]
    fn load_cycles(&mut self, addr: u32, width: u32) -> i32 {
        let d = self.data_cycles(addr, width);
        let code = self.cpu.cur_pc >> 24;
        let hidden = self.prefetch && (0x8..=0xD).contains(&code) && self.pf_addr != 1;
        d + if hidden { 0 } else { 1 }
    }

    #[inline]
    fn load_end(&self) -> i32 {
        let code = self.cpu.cur_pc >> 24;
        if self.prefetch && (0x8..=0xD).contains(&code) && self.pf_addr != 1 {
            0
        } else {
            1
        }
    }

    /// A store ends on its data cycle, so the following code fetch cannot be
    /// sequential.
    #[inline]
    fn store_cycles(&mut self, addr: u32, width: u32) -> i32 {
        self.cpu.fetch_n = true;
        self.bus_cycles(addr, width, false)
    }

    fn undefined(&mut self) -> i32 {
        if !self.warned_undefined {
            self.warned_undefined = true;
            self.sys_warn("The game executed an undefined instruction.");
        }
        let lr = self
            .cpu
            .cur_pc
            .wrapping_add(if self.cpu.cpsr & PSR_T != 0 { 2 } else { 4 });
        self.cpu_raise_exception(0x04, MODE_UND, lr);
        self.refill(0x04, false)
    }

    /// User-bank register access for LDM/STM with the S bit.
    fn user_reg(&self, i: usize) -> u32 {
        let c = &self.cpu;
        let b = bank_of(c.cpsr & PSR_MODE);
        if (8..=12).contains(&i) && b == 1 {
            return c.usr_hi[i - 8];
        }
        if i == 13 && b != 0 {
            return c.bank_sp[0];
        }
        if i == 14 && b != 0 {
            return c.bank_lr[0];
        }
        c.r[i]
    }

    fn set_user_reg(&mut self, i: usize, v: u32) {
        let c = &mut self.cpu;
        let b = bank_of(c.cpsr & PSR_MODE);
        if (8..=12).contains(&i) && b == 1 {
            c.usr_hi[i - 8] = v;
        } else if i == 13 && b != 0 {
            c.bank_sp[0] = v;
        } else if i == 14 && b != 0 {
            c.bank_lr[0] = v;
        } else {
            c.r[i] = v;
        }
    }

    // ---- ARM -----------------------------------------------------------

    fn arm_data_processing(&mut self, op: u32) -> i32 {
        let opc = (op >> 21) & 0xF;
        let s = (op >> 20) & 1 != 0;
        let rn_i = ((op >> 16) & 0xF) as usize;
        let rd_i = ((op >> 12) & 0xF) as usize;
        let cin = (self.cpu.cpsr >> 29) & 1;
        let mut sc = cin;
        let mut rn = self.cpu.r[rn_i];
        let op2;
        let mut cycles = 0;
        if op & (1 << 25) != 0 {
            let rot = ((op >> 8) & 0xF) * 2;
            op2 = ror32(op & 0xFF, rot);
            if rot != 0 {
                sc = op2 >> 31;
            }
        } else {
            let rm_i = (op & 0xF) as usize;
            let kind = (op >> 5) & 3;
            let mut rm = self.cpu.r[rm_i];
            if op & 0x10 != 0 {
                // Shift by register: one internal cycle, and r15 reads 12 ahead.
                let amt = self.cpu.r[((op >> 8) & 0xF) as usize] & 0xFF;
                if rm_i == 15 {
                    rm = rm.wrapping_add(4);
                }
                if rn_i == 15 {
                    rn = rn.wrapping_add(4);
                }
                (op2, sc) = shifter(kind, rm, amt, false, cin);
                cycles += 1;
                self.pf_busy = self.pf_busy.wrapping_add(1);
            } else {
                (op2, sc) = shifter(kind, rm, (op >> 7) & 0x1F, true, cin);
            }
        }
        let (mut cy, mut v) = (cin, (self.cpu.cpsr >> 28) & 1);
        let (mut logical, mut write) = (false, true);
        let res;
        match opc {
            0x0 => {
                res = rn & op2;
                logical = true;
            }
            0x1 => {
                res = rn ^ op2;
                logical = true;
            }
            0x2 => (res, cy, v) = add_flags(rn, !op2, 1),
            0x3 => (res, cy, v) = add_flags(op2, !rn, 1),
            0x4 => (res, cy, v) = add_flags(rn, op2, 0),
            0x5 => (res, cy, v) = add_flags(rn, op2, cin),
            0x6 => (res, cy, v) = add_flags(rn, !op2, cin),
            0x7 => (res, cy, v) = add_flags(op2, !rn, cin),
            0x8 => {
                res = rn & op2;
                logical = true;
                write = false;
            }
            0x9 => {
                res = rn ^ op2;
                logical = true;
                write = false;
            }
            0xA => {
                (res, cy, v) = add_flags(rn, !op2, 1);
                write = false;
            }
            0xB => {
                (res, cy, v) = add_flags(rn, op2, 0);
                write = false;
            }
            0xC => {
                res = rn | op2;
                logical = true;
            }
            0xD => {
                res = op2;
                logical = true;
            }
            0xE => {
                res = rn & !op2;
                logical = true;
            }
            _ => {
                res = !op2;
                logical = true;
            }
        }
        if rd_i == 15 && s {
            // With r15 as the destination, S copies SPSR into CPSR instead of
            // setting flags. The compare forms do this too, and since they
            // write no result they do not flush the pipeline.
            if self.cpu.has_spsr() {
                let spsr = self.cpu.spsr;
                self.write_cpsr(spsr);
            }
            return if write {
                cycles + self.branch_to(res)
            } else {
                cycles
            };
        }
        if write && rd_i == 15 {
            return cycles + self.branch_to(res);
        }
        if s {
            if logical {
                self.cpu.set_nzc(res, sc);
            } else {
                self.cpu.set_nzcv(res, cy, v);
            }
        }
        if write {
            self.cpu.r[rd_i] = res;
        }
        cycles
    }

    fn arm_psr(&mut self, op: u32) -> i32 {
        let spsr = (op >> 22) & 1 != 0;
        if op & (1 << 21) == 0 {
            // MRS
            let c = &mut self.cpu;
            c.r[((op >> 12) & 0xF) as usize] = if spsr && c.has_spsr() { c.spsr } else { c.cpsr };
            return 0;
        }
        let value = if op & (1 << 25) != 0 {
            ror32(op & 0xFF, ((op >> 8) & 0xF) * 2)
        } else {
            self.cpu.r[(op & 0xF) as usize]
        };
        let mut mask = 0u32;
        if op & (1 << 16) != 0 {
            mask |= 0x0000_00FF;
        }
        if op & (1 << 17) != 0 {
            mask |= 0x0000_FF00;
        }
        if op & (1 << 18) != 0 {
            mask |= 0x00FF_0000;
        }
        if op & (1 << 19) != 0 {
            mask |= 0xFF00_0000;
        }
        if spsr {
            if self.cpu.has_spsr() {
                self.cpu.spsr = (self.cpu.spsr & !mask) | (value & mask);
            }
            return 0;
        }
        if self.cpu.cpsr & PSR_MODE == MODE_USR {
            mask &= 0xFF00_0000;
        }
        let mut next = (self.cpu.cpsr & !mask) | (value & mask);
        next = (next & !PSR_T) | (self.cpu.cpsr & PSR_T); // MSR does not switch state
        self.write_cpsr(next | 0x10);
        0
    }

    fn arm_multiply(&mut self, op: u32) -> i32 {
        let rd = ((op >> 16) & 0xF) as usize;
        let rn = ((op >> 12) & 0xF) as usize;
        let rs = ((op >> 8) & 0xF) as usize;
        let rm = (op & 0xF) as usize;
        let c = &mut self.cpu;
        let mut r = c.r[rm].wrapping_mul(c.r[rs]);
        let mut cycles = mul_cycles(c.r[rs], true);
        if op & (1 << 21) != 0 {
            r = r.wrapping_add(c.r[rn]);
            cycles += 1;
        }
        c.r[rd] = r;
        if op & (1 << 20) != 0 {
            c.set_nz(r);
        }
        c.fetch_n = true;
        cycles
    }

    fn arm_multiply_long(&mut self, op: u32) -> i32 {
        let hi = ((op >> 16) & 0xF) as usize;
        let lo = ((op >> 12) & 0xF) as usize;
        let rs = ((op >> 8) & 0xF) as usize;
        let rm = (op & 0xF) as usize;
        let is_signed = (op >> 22) & 1 != 0;
        let c = &mut self.cpu;
        let mut r: u64 = if is_signed {
            (i64::from(c.r[rm] as i32).wrapping_mul(i64::from(c.r[rs] as i32))) as u64
        } else {
            u64::from(c.r[rm]) * u64::from(c.r[rs])
        };
        let mut cycles = mul_cycles(c.r[rs], is_signed) + 1;
        if op & (1 << 21) != 0 {
            r = r.wrapping_add((u64::from(c.r[hi]) << 32) | u64::from(c.r[lo]));
            cycles += 1;
        }
        c.r[lo] = r as u32;
        c.r[hi] = (r >> 32) as u32;
        if op & (1 << 20) != 0 {
            c.cpsr = (c.cpsr & !(PSR_N | PSR_Z))
                | (((r >> 32) as u32) & PSR_N)
                | if r != 0 { 0 } else { PSR_Z };
        }
        c.fetch_n = true;
        cycles
    }

    fn arm_swap(&mut self, op: u32) -> i32 {
        let rn = ((op >> 16) & 0xF) as usize;
        let rd = ((op >> 12) & 0xF) as usize;
        let rm = (op & 0xF) as usize;
        let addr = self.cpu.r[rn];
        let src = self.cpu.r[rm];
        if op & (1 << 22) != 0 {
            let tmp = self.bus_read8(addr);
            self.bus_write8(addr, src, Source::Cpu);
            self.cpu.r[rd] = tmp;
            let d = 2 * self.data_cycles(addr, 1);
            return d + self.load_end();
        }
        let tmp = ror32(self.bus_read32(addr), (addr & 3) * 8);
        self.bus_write32(addr, src, Source::Cpu);
        self.cpu.r[rd] = tmp;
        let d = 2 * self.data_cycles(addr, 4);
        d + self.load_end()
    }

    fn arm_halfword(&mut self, op: u32) -> i32 {
        let pre = (op >> 24) & 1 != 0;
        let up = (op >> 23) & 1 != 0;
        let imm = (op >> 22) & 1 != 0;
        let wb = (op >> 21) & 1 != 0;
        let load = (op >> 20) & 1 != 0;
        let rn = ((op >> 16) & 0xF) as usize;
        let rd = ((op >> 12) & 0xF) as usize;
        let sh = (op >> 5) & 3;
        let off = if imm {
            ((op >> 4) & 0xF0) | (op & 0xF)
        } else {
            self.cpu.r[(op & 0xF) as usize]
        };
        let base = self.cpu.r[rn];
        let moved = if up {
            base.wrapping_add(off)
        } else {
            base.wrapping_sub(off)
        };
        let addr = if pre { moved } else { base };
        let cycles;
        if load {
            let v;
            if sh == 1 {
                v = ror32(self.bus_read16(addr), (addr & 1) * 8);
                cycles = self.load_cycles(addr, 2);
            } else if sh == 2 {
                v = self.bus_read8(addr) as u8 as i8 as i32 as u32;
                cycles = self.load_cycles(addr, 1);
            } else {
                if addr & 1 != 0 {
                    v = self.bus_read8(addr) as u8 as i8 as i32 as u32;
                } else {
                    v = self.bus_read16(addr) as u16 as i16 as i32 as u32;
                }
                cycles = self.load_cycles(addr, 2);
            }
            if !pre || wb {
                self.cpu.r[rn] = moved;
            }
            if rd == 15 {
                return cycles + self.branch_to(v);
            }
            self.cpu.r[rd] = v;
            return cycles;
        }
        let cycles = if sh == 1 {
            let v = if rd == 15 {
                self.cpu.r[15].wrapping_add(4)
            } else {
                self.cpu.r[rd]
            };
            self.bus_write16(addr, v, Source::Cpu);
            self.store_cycles(addr, 2)
        } else {
            1 // the doubleword forms belong to ARMv5 and do nothing here
        };
        if !pre || wb {
            self.cpu.r[rn] = moved;
        }
        cycles
    }

    fn arm_single_transfer(&mut self, op: u32) -> i32 {
        let pre = (op >> 24) & 1 != 0;
        let up = (op >> 23) & 1 != 0;
        let byte = (op >> 22) & 1 != 0;
        let wb = (op >> 21) & 1 != 0;
        let load = (op >> 20) & 1 != 0;
        let rn = ((op >> 16) & 0xF) as usize;
        let rd = ((op >> 12) & 0xF) as usize;
        let off = if op & (1 << 25) != 0 {
            shifter(
                (op >> 5) & 3,
                self.cpu.r[(op & 0xF) as usize],
                (op >> 7) & 0x1F,
                true,
                (self.cpu.cpsr >> 29) & 1,
            )
            .0
        } else {
            op & 0xFFF
        };
        let base = self.cpu.r[rn];
        let moved = if up {
            base.wrapping_add(off)
        } else {
            base.wrapping_sub(off)
        };
        let addr = if pre { moved } else { base };
        if load {
            let v;
            let cycles;
            if byte {
                v = self.bus_read8(addr);
                cycles = self.load_cycles(addr, 1);
            } else {
                v = ror32(self.bus_read32(addr), (addr & 3) * 8);
                cycles = self.load_cycles(addr, 4);
            }
            if !pre || wb {
                self.cpu.r[rn] = moved;
            }
            if rd == 15 {
                return cycles + self.branch_to(v);
            }
            self.cpu.r[rd] = v;
            return cycles;
        }
        let v = if rd == 15 {
            self.cpu.r[15].wrapping_add(4)
        } else {
            self.cpu.r[rd]
        };
        let cycles;
        if byte {
            self.bus_write8(addr, v, Source::Cpu);
            cycles = self.store_cycles(addr, 1);
        } else {
            self.bus_write32(addr, v, Source::Cpu);
            cycles = self.store_cycles(addr, 4);
        }
        if !pre || wb {
            self.cpu.r[rn] = moved;
        }
        cycles
    }

    fn arm_block_transfer(&mut self, op: u32) -> i32 {
        let pre = (op >> 24) & 1 != 0;
        let up = (op >> 23) & 1 != 0;
        let psr = (op >> 22) & 1 != 0;
        let wb = (op >> 21) & 1 != 0;
        let load = (op >> 20) & 1 != 0;
        let rn = ((op >> 16) & 0xF) as usize;
        let mut list = op & 0xFFFF;
        let n;
        if list == 0 {
            list = 0x8000; // ARMv4: an empty list moves r15 and 64 bytes
            n = 16;
        } else {
            n = popcount16(list);
        }
        let n4 = 4u32.wrapping_mul(n as u32);
        let base = self.cpu.r[rn];
        let (start, fin);
        if up {
            start = if pre { base.wrapping_add(4) } else { base };
            fin = base.wrapping_add(n4);
        } else {
            start = if pre {
                base.wrapping_sub(n4)
            } else {
                base.wrapping_sub(n4).wrapping_add(4)
            };
            fin = base.wrapping_sub(n4);
        }
        let user = psr && (!load || list & 0x8000 == 0);
        let mut addr = start;
        let mut cycles = 0;
        let mut first = true;
        if load {
            if wb {
                self.cpu.r[rn] = fin;
            }
            for i in 0..16usize {
                if list & (1 << i) == 0 {
                    continue;
                }
                let v = self.bus_read32(addr);
                cycles += self.bus_cycles(addr, 4, !first);
                first = false;
                addr = addr.wrapping_add(4);
                if i == 15 {
                    if psr && self.cpu.has_spsr() {
                        let spsr = self.cpu.spsr;
                        self.write_cpsr(spsr);
                    }
                    cycles += self.branch_to(v);
                } else if user {
                    self.set_user_reg(i, v);
                } else {
                    self.cpu.r[i] = v;
                }
            }
            self.cpu.fetch_n = true;
            return cycles + self.load_end();
        }
        let lowest = list & (!list).wrapping_add(1);
        for i in 0..16usize {
            if list & (1 << i) == 0 {
                continue;
            }
            let v = if i == 15 {
                self.cpu.r[15].wrapping_add(4)
            } else if i == rn && wb && (1u32 << i) != lowest {
                fin
            } else if user {
                self.user_reg(i)
            } else {
                self.cpu.r[i]
            };
            self.bus_write32(addr, v, Source::Cpu);
            cycles += self.bus_cycles(addr, 4, !first);
            first = false;
            addr = addr.wrapping_add(4);
        }
        if wb {
            self.cpu.r[rn] = fin;
        }
        self.cpu.fetch_n = true;
        cycles
    }

    fn arm_exec(&mut self, op: u32) -> i32 {
        match (op >> 25) & 7 {
            0 => {
                if (op & 0x0FC0_00F0) == 0x0000_0090 {
                    return self.arm_multiply(op);
                }
                if (op & 0x0F80_00F0) == 0x0080_0090 {
                    return self.arm_multiply_long(op);
                }
                if (op & 0x0FB0_0FF0) == 0x0100_0090 {
                    return self.arm_swap(op);
                }
                if (op & 0x0FFF_FFF0) == 0x012F_FF10 {
                    let v = self.cpu.r[(op & 0xF) as usize];
                    if v & 1 != 0 {
                        self.cpu.cpsr |= PSR_T;
                    } else {
                        self.cpu.cpsr &= !PSR_T;
                    }
                    return self.branch_to(v);
                }
                if (op & 0x90) == 0x90 && (op & 0x60) != 0 {
                    return self.arm_halfword(op);
                }
                if (op & 0x0190_0000) == 0x0100_0000 {
                    if (op & 0x0FBF_0FFF) == 0x010F_0000 || (op & 0x0FB0_FFF0) == 0x0120_F000 {
                        return self.arm_psr(op);
                    }
                    return self.undefined();
                }
                self.arm_data_processing(op)
            }
            1 => {
                if (op & 0x0190_0000) == 0x0100_0000 {
                    if (op & 0x0FB0_F000) == 0x0320_F000 {
                        return self.arm_psr(op);
                    }
                    return self.undefined();
                }
                self.arm_data_processing(op)
            }
            2 => self.arm_single_transfer(op),
            3 => {
                if op & 0x10 != 0 {
                    return self.undefined();
                }
                self.arm_single_transfer(op)
            }
            4 => self.arm_block_transfer(op),
            5 => {
                let off = ((op << 8) as i32) >> 6;
                if op & (1 << 24) != 0 {
                    self.cpu.r[14] = self.cpu.cur_pc.wrapping_add(4);
                }
                let target = self.cpu.r[15].wrapping_add(off as u32);
                self.branch_to(target)
            }
            6 => self.undefined(),
            _ => {
                if op & (1 << 24) != 0 {
                    self.bios_swi((op >> 16) & 0xFF);
                    return 0;
                }
                self.undefined()
            }
        }
    }

    // ---- Thumb ---------------------------------------------------------

    fn thumb_alu(&mut self, op: u32) -> i32 {
        let rd = (op & 7) as usize;
        let rs = ((op >> 3) & 7) as usize;
        let a = self.cpu.r[rd];
        let b = self.cpu.r[rs];
        let cin = (self.cpu.cpsr >> 29) & 1;
        match (op >> 6) & 0xF {
            0x0 => {
                let r = a & b;
                self.cpu.set_nz(r);
                self.cpu.r[rd] = r;
                0
            }
            0x1 => {
                let r = a ^ b;
                self.cpu.set_nz(r);
                self.cpu.r[rd] = r;
                0
            }
            0x2 | 0x3 | 0x4 | 0x7 => {
                let kind = match (op >> 6) & 0xF {
                    0x2 => 0,
                    0x3 => 1,
                    0x4 => 2,
                    _ => 3,
                };
                let (r, cy) = shifter(kind, a, b & 0xFF, false, cin);
                self.cpu.set_nzc(r, cy);
                self.cpu.r[rd] = r;
                self.pf_busy = self.pf_busy.wrapping_add(1);
                1
            }
            0x5 => {
                let (r, cy, v) = add_flags(a, b, cin);
                self.cpu.set_nzcv(r, cy, v);
                self.cpu.r[rd] = r;
                0
            }
            0x6 => {
                let (r, cy, v) = add_flags(a, !b, cin);
                self.cpu.set_nzcv(r, cy, v);
                self.cpu.r[rd] = r;
                0
            }
            0x8 => {
                self.cpu.set_nz(a & b);
                0
            }
            0x9 => {
                let (r, cy, v) = add_flags(0, !b, 1);
                self.cpu.set_nzcv(r, cy, v);
                self.cpu.r[rd] = r;
                0
            }
            0xA => {
                let (r, cy, v) = add_flags(a, !b, 1);
                self.cpu.set_nzcv(r, cy, v);
                0
            }
            0xB => {
                let (r, cy, v) = add_flags(a, b, 0);
                self.cpu.set_nzcv(r, cy, v);
                0
            }
            0xC => {
                let r = a | b;
                self.cpu.set_nz(r);
                self.cpu.r[rd] = r;
                0
            }
            0xD => {
                let r = a.wrapping_mul(b);
                self.cpu.set_nz(r);
                self.cpu.r[rd] = r;
                self.cpu.fetch_n = true;
                mul_cycles(a, true)
            }
            0xE => {
                let r = a & !b;
                self.cpu.set_nz(r);
                self.cpu.r[rd] = r;
                0
            }
            _ => {
                let r = !b;
                self.cpu.set_nz(r);
                self.cpu.r[rd] = r;
                0
            }
        }
    }

    fn thumb_hi(&mut self, op: u32) -> i32 {
        let rd = ((op & 7) | ((op >> 4) & 8)) as usize;
        let rs = ((op >> 3) & 0xF) as usize;
        let b = self.cpu.r[rs];
        match (op >> 8) & 3 {
            0 => {
                let r = self.cpu.r[rd].wrapping_add(b);
                if rd == 15 {
                    return self.branch_to(r);
                }
                self.cpu.r[rd] = r;
                0
            }
            1 => {
                let (r, cy, v) = add_flags(self.cpu.r[rd], !b, 1);
                self.cpu.set_nzcv(r, cy, v);
                0
            }
            2 => {
                if rd == 15 {
                    return self.branch_to(b);
                }
                self.cpu.r[rd] = b;
                0
            }
            _ => {
                if b & 1 != 0 {
                    self.cpu.cpsr |= PSR_T;
                } else {
                    self.cpu.cpsr &= !PSR_T;
                }
                self.branch_to(b)
            }
        }
    }

    fn thumb_ldm_stm(&mut self, rb: usize, list: u32, load: bool) -> i32 {
        let base = self.cpu.r[rb];
        if list == 0 {
            // The ARMv4 empty-list quirk applies in Thumb state too.
            if load {
                self.cpu.r[rb] = base.wrapping_add(0x40);
                let d = self.load_cycles(base, 4);
                let v = self.bus_read32(base);
                return d + self.branch_to(v);
            }
            let v = self.cpu.r[15].wrapping_add(2);
            self.bus_write32(base, v, Source::Cpu);
            self.cpu.r[rb] = base.wrapping_add(0x40);
            return self.store_cycles(base, 4);
        }
        let n = popcount16(list);
        let fin = base.wrapping_add(4u32.wrapping_mul(n as u32));
        let mut addr = base;
        let mut cycles = 0;
        let mut first = true;
        if load {
            for i in 0..8usize {
                if list & (1 << i) == 0 {
                    continue;
                }
                self.cpu.r[i] = self.bus_read32(addr);
                cycles += self.bus_cycles(addr, 4, !first);
                first = false;
                addr = addr.wrapping_add(4);
            }
            if list & (1 << rb) == 0 {
                self.cpu.r[rb] = fin;
            }
            self.cpu.fetch_n = true;
            return cycles + self.load_end();
        }
        let lowest = list & (!list).wrapping_add(1);
        for i in 0..8usize {
            if list & (1 << i) == 0 {
                continue;
            }
            let v = if i == rb && (1u32 << i) != lowest {
                fin
            } else {
                self.cpu.r[i]
            };
            self.bus_write32(addr, v, Source::Cpu);
            cycles += self.bus_cycles(addr, 4, !first);
            first = false;
            addr = addr.wrapping_add(4);
        }
        self.cpu.r[rb] = fin;
        self.cpu.fetch_n = true;
        cycles
    }

    fn thumb_push_pop(&mut self, op: u32) -> i32 {
        let load = (op >> 11) & 1 != 0;
        let extra = (op >> 8) & 1 != 0;
        let list = op & 0xFF;
        if list == 0 && !extra {
            let sp = self.cpu.r[13];
            if load {
                self.cpu.r[13] = sp.wrapping_add(0x40);
                let d = self.load_cycles(sp, 4);
                let v = self.bus_read32(sp);
                return d + self.branch_to(v);
            }
            self.cpu.r[13] = sp.wrapping_sub(0x40);
            let v = self.cpu.r[15].wrapping_add(2);
            self.bus_write32(sp.wrapping_sub(0x40), v, Source::Cpu);
            return self.store_cycles(sp, 4);
        }
        let n = popcount16(list) + i32::from(extra);
        let mut cycles = 0;
        let mut first = true;
        if load {
            let mut addr = self.cpu.r[13];
            for i in 0..8usize {
                if list & (1 << i) == 0 {
                    continue;
                }
                self.cpu.r[i] = self.bus_read32(addr);
                cycles += self.bus_cycles(addr, 4, !first);
                first = false;
                addr = addr.wrapping_add(4);
            }
            let mut pc = 0;
            if extra {
                pc = self.bus_read32(addr);
                cycles += self.bus_cycles(addr, 4, !first);
                addr = addr.wrapping_add(4);
            }
            self.cpu.r[13] = addr;
            if extra {
                cycles += self.branch_to(pc);
            }
            self.cpu.fetch_n = true;
            return cycles + self.load_end();
        }
        let mut addr = self.cpu.r[13].wrapping_sub(4u32.wrapping_mul(n as u32));
        self.cpu.r[13] = addr;
        for i in 0..8usize {
            if list & (1 << i) == 0 {
                continue;
            }
            let v = self.cpu.r[i];
            self.bus_write32(addr, v, Source::Cpu);
            cycles += self.bus_cycles(addr, 4, !first);
            first = false;
            addr = addr.wrapping_add(4);
        }
        if extra {
            let v = self.cpu.r[14];
            self.bus_write32(addr, v, Source::Cpu);
            cycles += self.bus_cycles(addr, 4, !first);
        }
        self.cpu.fetch_n = true;
        cycles
    }

    fn thumb_exec(&mut self, op: u32) -> i32 {
        let cin = (self.cpu.cpsr >> 29) & 1;
        match op >> 11 {
            0x00 | 0x01 | 0x02 => {
                // format 1: shift by immediate
                let rd = (op & 7) as usize;
                let rs = ((op >> 3) & 7) as usize;
                let amt = (op >> 6) & 0x1F;
                let (r, cy) = shifter(op >> 11, self.cpu.r[rs], amt, true, cin);
                self.cpu.set_nzc(r, cy);
                self.cpu.r[rd] = r;
                0
            }
            0x03 => {
                // format 2: add/subtract
                let rd = (op & 7) as usize;
                let rs = ((op >> 3) & 7) as usize;
                let n = ((op >> 6) & 7) as usize;
                let b = if op & 0x400 != 0 {
                    n as u32
                } else {
                    self.cpu.r[n]
                };
                let (r, cy, v) = if op & 0x200 != 0 {
                    add_flags(self.cpu.r[rs], !b, 1)
                } else {
                    add_flags(self.cpu.r[rs], b, 0)
                };
                self.cpu.set_nzcv(r, cy, v);
                self.cpu.r[rd] = r;
                0
            }
            0x04 => {
                let rd = ((op >> 8) & 7) as usize;
                let r = op & 0xFF;
                self.cpu.set_nz(r);
                self.cpu.r[rd] = r;
                0
            }
            0x05 => {
                let rd = ((op >> 8) & 7) as usize;
                let (r, cy, v) = add_flags(self.cpu.r[rd], !(op & 0xFF), 1);
                self.cpu.set_nzcv(r, cy, v);
                0
            }
            0x06 => {
                let rd = ((op >> 8) & 7) as usize;
                let (r, cy, v) = add_flags(self.cpu.r[rd], op & 0xFF, 0);
                self.cpu.set_nzcv(r, cy, v);
                self.cpu.r[rd] = r;
                0
            }
            0x07 => {
                let rd = ((op >> 8) & 7) as usize;
                let (r, cy, v) = add_flags(self.cpu.r[rd], !(op & 0xFF), 1);
                self.cpu.set_nzcv(r, cy, v);
                self.cpu.r[rd] = r;
                0
            }
            0x08 => {
                if op & 0x400 != 0 {
                    self.thumb_hi(op)
                } else {
                    self.thumb_alu(op)
                }
            }
            0x09 => {
                // format 6: PC-relative load
                let addr = (self.cpu.r[15] & !2).wrapping_add((op & 0xFF) << 2);
                self.cpu.r[((op >> 8) & 7) as usize] = self.bus_read32(addr);
                self.load_cycles(addr, 4)
            }
            0x0A | 0x0B => {
                let rd = (op & 7) as usize;
                let addr = self.cpu.r[((op >> 3) & 7) as usize]
                    .wrapping_add(self.cpu.r[((op >> 6) & 7) as usize]);
                if op & 0x200 == 0 {
                    // format 7
                    match (op >> 10) & 3 {
                        0 => {
                            let v = self.cpu.r[rd];
                            self.bus_write32(addr, v, Source::Cpu);
                            self.store_cycles(addr, 4)
                        }
                        1 => {
                            let v = self.cpu.r[rd];
                            self.bus_write8(addr, v, Source::Cpu);
                            self.store_cycles(addr, 1)
                        }
                        2 => {
                            self.cpu.r[rd] = ror32(self.bus_read32(addr), (addr & 3) * 8);
                            self.load_cycles(addr, 4)
                        }
                        _ => {
                            self.cpu.r[rd] = self.bus_read8(addr);
                            self.load_cycles(addr, 1)
                        }
                    }
                } else {
                    // format 8
                    match (op >> 10) & 3 {
                        0 => {
                            let v = self.cpu.r[rd];
                            self.bus_write16(addr, v, Source::Cpu);
                            self.store_cycles(addr, 2)
                        }
                        1 => {
                            self.cpu.r[rd] = self.bus_read8(addr) as u8 as i8 as i32 as u32;
                            self.load_cycles(addr, 1)
                        }
                        2 => {
                            self.cpu.r[rd] = ror32(self.bus_read16(addr), (addr & 1) * 8);
                            self.load_cycles(addr, 2)
                        }
                        _ => {
                            if addr & 1 != 0 {
                                self.cpu.r[rd] = self.bus_read8(addr) as u8 as i8 as i32 as u32;
                            } else {
                                self.cpu.r[rd] = self.bus_read16(addr) as u16 as i16 as i32 as u32;
                            }
                            self.load_cycles(addr, 2)
                        }
                    }
                }
            }
            0x0C => {
                let a = self.cpu.r[((op >> 3) & 7) as usize].wrapping_add(((op >> 6) & 0x1F) << 2);
                let v = self.cpu.r[(op & 7) as usize];
                self.bus_write32(a, v, Source::Cpu);
                self.store_cycles(a, 4)
            }
            0x0D => {
                let a = self.cpu.r[((op >> 3) & 7) as usize].wrapping_add(((op >> 6) & 0x1F) << 2);
                self.cpu.r[(op & 7) as usize] = ror32(self.bus_read32(a), (a & 3) * 8);
                self.load_cycles(a, 4)
            }
            0x0E => {
                let a = self.cpu.r[((op >> 3) & 7) as usize].wrapping_add((op >> 6) & 0x1F);
                let v = self.cpu.r[(op & 7) as usize];
                self.bus_write8(a, v, Source::Cpu);
                self.store_cycles(a, 1)
            }
            0x0F => {
                let a = self.cpu.r[((op >> 3) & 7) as usize].wrapping_add((op >> 6) & 0x1F);
                self.cpu.r[(op & 7) as usize] = self.bus_read8(a);
                self.load_cycles(a, 1)
            }
            0x10 => {
                let a = self.cpu.r[((op >> 3) & 7) as usize].wrapping_add(((op >> 6) & 0x1F) << 1);
                let v = self.cpu.r[(op & 7) as usize];
                self.bus_write16(a, v, Source::Cpu);
                self.store_cycles(a, 2)
            }
            0x11 => {
                let a = self.cpu.r[((op >> 3) & 7) as usize].wrapping_add(((op >> 6) & 0x1F) << 1);
                self.cpu.r[(op & 7) as usize] = ror32(self.bus_read16(a), (a & 1) * 8);
                self.load_cycles(a, 2)
            }
            0x12 => {
                let a = self.cpu.r[13].wrapping_add((op & 0xFF) << 2);
                let v = self.cpu.r[((op >> 8) & 7) as usize];
                self.bus_write32(a, v, Source::Cpu);
                self.store_cycles(a, 4)
            }
            0x13 => {
                let a = self.cpu.r[13].wrapping_add((op & 0xFF) << 2);
                self.cpu.r[((op >> 8) & 7) as usize] = ror32(self.bus_read32(a), (a & 3) * 8);
                self.load_cycles(a, 4)
            }
            0x14 => {
                self.cpu.r[((op >> 8) & 7) as usize] =
                    (self.cpu.r[15] & !2).wrapping_add((op & 0xFF) << 2);
                0
            }
            0x15 => {
                self.cpu.r[((op >> 8) & 7) as usize] =
                    self.cpu.r[13].wrapping_add((op & 0xFF) << 2);
                0
            }
            0x16 | 0x17 => {
                if (op & 0xF00) == 0x000 {
                    let imm = (op & 0x7F) << 2;
                    self.cpu.r[13] = if op & 0x80 != 0 {
                        self.cpu.r[13].wrapping_sub(imm)
                    } else {
                        self.cpu.r[13].wrapping_add(imm)
                    };
                    return 0;
                }
                if (op & 0x600) == 0x400 {
                    return self.thumb_push_pop(op);
                }
                self.undefined()
            }
            0x18 => self.thumb_ldm_stm(((op >> 8) & 7) as usize, op & 0xFF, false),
            0x19 => self.thumb_ldm_stm(((op >> 8) & 7) as usize, op & 0xFF, true),
            0x1A | 0x1B => {
                let cond = (op >> 8) & 0xF;
                if cond == 0xF {
                    self.bios_swi(op & 0xFF);
                    return 0;
                }
                if cond == 0xE {
                    return self.undefined();
                }
                if !cond_ok(self.cpu.cpsr, cond) {
                    return 0;
                }
                let off = ((op << 24) as i32) >> 23;
                let target = self.cpu.r[15].wrapping_add(off as u32);
                self.branch_to(target)
            }
            0x1C => {
                let off = ((op << 21) as i32) >> 20;
                let target = self.cpu.r[15].wrapping_add(off as u32);
                self.branch_to(target)
            }
            0x1D => self.undefined(),
            0x1E => {
                let off = ((op << 21) as i32) >> 9;
                self.cpu.r[14] = self.cpu.r[15].wrapping_add(off as u32);
                0
            }
            _ => {
                let target = self.cpu.r[14].wrapping_add((op & 0x7FF) << 1);
                self.cpu.r[14] = self.cpu.cur_pc.wrapping_add(2) | 1;
                self.branch_to(target)
            }
        }
    }

    // ---- Cartridge prefetch and the step ------------------------------

    #[inline]
    fn prefetch_run(&mut self, mut cycles: i64, s16: i32) {
        while cycles > 0 && self.pf_count < 8 {
            let need = s16 - self.pf_progress;
            if cycles >= i64::from(need) {
                self.pf_count += 1;
                cycles -= i64::from(need);
                self.pf_progress = 0;
            } else {
                self.pf_progress += cycles as i32;
                cycles = 0;
            }
        }
    }

    fn prefetch_fetch(&mut self, pc: u32, width: u32) -> i32 {
        let s16 = self.bus_cycles(pc, 2, true);
        if self.pf_fresh && pc == self.pf_addr {
            // The refill was charged to the branch; the buffer starts empty now.
            self.pf_fresh = false;
            self.pf_count = 0;
            self.pf_progress = 0;
            self.pf_time = self.now;
            self.pf_busy = 0;
        }
        if pc != self.pf_addr {
            let cost = self.bus_cycles(pc, 2, false) + if width == 4 { s16 } else { 0 };
            self.pf_addr = pc.wrapping_add(width);
            self.pf_count = 0;
            self.pf_progress = 0;
            self.pf_time = self.now + cost as u64;
            self.pf_busy = 0;
            self.pf_fresh = false;
            return cost;
        }
        if self.now > self.pf_time {
            let idle = (self.now - self.pf_time) as i64 - i64::from(self.pf_busy);
            self.prefetch_run(idle, s16);
        }
        self.pf_busy = 0;
        let mut cost = 0;
        for _ in 0..width / 2 {
            if self.pf_count > 0 {
                self.pf_count -= 1;
                cost += 1;
                self.prefetch_run(1, s16);
            } else {
                let wait = s16 - self.pf_progress;
                cost += if wait < 1 { 1 } else { wait };
                self.pf_progress = 0;
            }
        }
        self.pf_addr = pc.wrapping_add(width);
        self.pf_time = self.now + cost as u64;
        cost
    }

    /// Cost of fetching the instruction at `pc`. Cartridge code goes through
    /// the prefetch buffer when WAITCNT enables it: the buffer reads ahead one
    /// halfword per sequential access time whenever the cartridge bus is free.
    #[inline]
    fn fetch_cycles(&mut self, pc: u32, width: u32) -> i32 {
        let r = pc >> 24;
        let seq = !self.cpu.fetch_n;
        self.cpu.fetch_n = false;
        if self.prefetch && (0x8..=0xD).contains(&r) {
            return self.prefetch_fetch(pc, width);
        }
        self.bus_cycles(pc, width, seq)
    }

    pub(crate) fn cpu_step(&mut self) -> i32 {
        if self.cpu.cpsr & PSR_I == 0 && self.ime & 1 != 0 && self.ie & self.iflags != 0 {
            self.cpu_take_irq();
            self.cpu.pc_written = false;
            return self.refill(0x18, false);
        }
        let pc = self.cpu.next_pc;
        self.cpu.cur_pc = pc;
        self.cpu.pc_written = false;
        let mut cycles;
        if self.cpu.cpsr & PSR_T != 0 {
            let op = self.bus_fetch16(pc);
            self.cpu.r[15] = pc.wrapping_add(4);
            cycles = self.fetch_cycles(pc, 2);
            cycles += self.thumb_exec(op);
            if !self.cpu.pc_written {
                self.cpu.next_pc = pc.wrapping_add(2);
            }
        } else {
            let op = self.bus_fetch32(pc);
            if pc < BIOS_SIZE {
                let a = (pc.wrapping_add(8) & 0x3FFC) as usize;
                self.bios_latch = u32::from_le_bytes([
                    self.bios[a],
                    self.bios[a + 1],
                    self.bios[a + 2],
                    self.bios[a + 3],
                ]);
            }
            self.cpu.r[15] = pc.wrapping_add(8);
            cycles = self.fetch_cycles(pc, 4);
            if cond_ok(self.cpu.cpsr, op >> 28) {
                cycles += self.arm_exec(op);
            }
            if !self.cpu.pc_written {
                self.cpu.next_pc = pc.wrapping_add(4);
            }
        }
        cycles
    }
}
