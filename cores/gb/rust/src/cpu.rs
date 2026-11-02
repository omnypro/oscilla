//! The CPU: the Sharp SM83 instruction set. Each memory access and each
//! internal step costs one machine cycle of 4 clocks, so an instruction's
//! length is the sum of its steps.

use crate::bus::{WARN_HALT, WARN_LOCK, WARN_SPEED, WARN_STOP};
use crate::state::*;

const FZ: u8 = 0x80;
const FN: u8 = 0x40;
const FH: u8 = 0x20;
const FC: u8 = 0x10;

impl Cpu {
    fn bc(&self) -> u16 {
        u16::from(self.b) << 8 | u16::from(self.c)
    }
    fn de(&self) -> u16 {
        u16::from(self.d) << 8 | u16::from(self.e)
    }
    pub(crate) fn hl(&self) -> u16 {
        u16::from(self.h) << 8 | u16::from(self.l)
    }
    fn af(&self) -> u16 {
        u16::from(self.a) << 8 | u16::from(self.f & 0xF0)
    }
    fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }
    fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }
    fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }
    fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = v as u8 & 0xF0;
    }
    fn flag(&self, m: u8) -> bool {
        self.f & m != 0
    }
    fn set_flag(&mut self, m: u8, on: bool) {
        if on {
            self.f |= m;
        } else {
            self.f &= !m;
        }
    }
    /// A condition by its two-bit code: NZ, Z, NC, C.
    fn cond(&self, cc: u8) -> bool {
        match cc & 3 {
            0 => !self.flag(FZ),
            1 => self.flag(FZ),
            2 => !self.flag(FC),
            _ => self.flag(FC),
        }
    }
}

impl Machine {
    fn fetch8(&mut self) -> u8 {
        let pc = self.cpu.pc;
        let v = self.rd(pc);
        if self.cpu.halt_bug {
            self.cpu.halt_bug = false;
        } else {
            self.cpu.pc = pc.wrapping_add(1);
        }
        v
    }

    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch8();
        let hi = self.fetch8();
        u16::from(hi) << 8 | u16::from(lo)
    }

    pub(crate) fn push16(&mut self, v: u16) {
        self.cpu.sp = self.cpu.sp.wrapping_sub(1);
        let sp = self.cpu.sp;
        self.wr(sp, (v >> 8) as u8);
        self.cpu.sp = self.cpu.sp.wrapping_sub(1);
        let sp = self.cpu.sp;
        self.wr(sp, v as u8);
    }

    fn pop16(&mut self) -> u16 {
        let sp = self.cpu.sp;
        let lo = self.rd(sp);
        self.cpu.sp = sp.wrapping_add(1);
        let sp = self.cpu.sp;
        let hi = self.rd(sp);
        self.cpu.sp = sp.wrapping_add(1);
        u16::from(hi) << 8 | u16::from(lo)
    }

    /// Operand r8 by its three-bit code: B, C, D, E, H, L, (HL), A.
    fn get_r(&mut self, i: u8) -> u8 {
        match i & 7 {
            0 => self.cpu.b,
            1 => self.cpu.c,
            2 => self.cpu.d,
            3 => self.cpu.e,
            4 => self.cpu.h,
            5 => self.cpu.l,
            6 => {
                let hl = self.cpu.hl();
                self.rd(hl)
            }
            _ => self.cpu.a,
        }
    }

    fn set_r(&mut self, i: u8, v: u8) {
        match i & 7 {
            0 => self.cpu.b = v,
            1 => self.cpu.c = v,
            2 => self.cpu.d = v,
            3 => self.cpu.e = v,
            4 => self.cpu.h = v,
            5 => self.cpu.l = v,
            6 => {
                let hl = self.cpu.hl();
                self.wr(hl, v);
            }
            _ => self.cpu.a = v,
        }
    }

    /// Operand r16 with SP: BC, DE, HL, SP.
    fn get_rr(&self, i: u8) -> u16 {
        match i & 3 {
            0 => self.cpu.bc(),
            1 => self.cpu.de(),
            2 => self.cpu.hl(),
            _ => self.cpu.sp,
        }
    }

    fn set_rr(&mut self, i: u8, v: u16) {
        match i & 3 {
            0 => self.cpu.set_bc(v),
            1 => self.cpu.set_de(v),
            2 => self.cpu.set_hl(v),
            _ => self.cpu.sp = v,
        }
    }

    fn alu(&mut self, op: u8, v: u8) {
        let a = self.cpu.a;
        let c = u8::from(self.cpu.flag(FC));
        match op & 7 {
            0 | 1 => {
                let carry = if op & 7 == 1 { c } else { 0 };
                let sum = u16::from(a) + u16::from(v) + u16::from(carry);
                let h = (a & 0xF) + (v & 0xF) + carry > 0xF;
                self.cpu.a = sum as u8;
                self.cpu.f = flags(sum as u8 == 0, false, h, sum > 0xFF);
            }
            2 | 3 | 7 => {
                let carry = if op & 7 == 3 { c } else { 0 };
                let diff = i16::from(a) - i16::from(v) - i16::from(carry);
                let h = i16::from(a & 0xF) - i16::from(v & 0xF) - i16::from(carry) < 0;
                if op & 7 != 7 {
                    self.cpu.a = diff as u8;
                }
                self.cpu.f = flags(diff as u8 == 0, true, h, diff < 0);
            }
            4 => {
                self.cpu.a = a & v;
                self.cpu.f = flags(self.cpu.a == 0, false, true, false);
            }
            5 => {
                self.cpu.a = a ^ v;
                self.cpu.f = flags(self.cpu.a == 0, false, false, false);
            }
            _ => {
                self.cpu.a = a | v;
                self.cpu.f = flags(self.cpu.a == 0, false, false, false);
            }
        }
    }

    fn inc8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.cpu.set_flag(FZ, r == 0);
        self.cpu.set_flag(FN, false);
        self.cpu.set_flag(FH, r & 0xF == 0);
        r
    }

    fn dec8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.cpu.set_flag(FZ, r == 0);
        self.cpu.set_flag(FN, true);
        self.cpu.set_flag(FH, r & 0xF == 0xF);
        r
    }

    /// The rotates, shifts and swap of the 0xCB page, on operand `v`.
    fn cb_shift(&mut self, op: u8, v: u8) -> u8 {
        let old_c = u8::from(self.cpu.flag(FC));
        let (r, c) = match op & 7 {
            0 => (v.rotate_left(1), v >> 7),
            1 => (v.rotate_right(1), v & 1),
            2 => (v << 1 | old_c, v >> 7),
            3 => (v >> 1 | old_c << 7, v & 1),
            4 => (v << 1, v >> 7),
            5 => (v >> 1 | v & 0x80, v & 1),
            6 => (v.rotate_left(4), 0),
            _ => (v >> 1, v & 1),
        };
        self.cpu.f = flags(r == 0, false, false, c != 0);
        r
    }

    fn daa(&mut self) {
        let mut a = self.cpu.a;
        let mut carry = self.cpu.flag(FC);
        if !self.cpu.flag(FN) {
            let mut adj = 0u8;
            if self.cpu.flag(FH) || a & 0xF > 9 {
                adj |= 6;
            }
            if carry || a > 0x99 {
                adj |= 0x60;
                carry = true;
            }
            a = a.wrapping_add(adj);
        } else {
            let mut adj = 0u8;
            if self.cpu.flag(FH) {
                adj |= 6;
            }
            if carry {
                adj |= 0x60;
            }
            a = a.wrapping_sub(adj);
        }
        self.cpu.a = a;
        let n = self.cpu.flag(FN);
        self.cpu.f = flags(a == 0, n, false, carry);
    }

    /// SP plus a signed byte, with the flags of ADD SP,e.
    fn sp_plus(&mut self, e: u8) -> u16 {
        let sp = self.cpu.sp;
        let r = sp.wrapping_add(e as i8 as u16);
        let h = (sp & 0xF) + u16::from(e & 0xF) > 0xF;
        let c = (sp & 0xFF) + u16::from(e) > 0xFF;
        self.cpu.f = flags(false, false, h, c);
        r
    }

    /// Where RST n goes: in a GBS the vectors sit relative to the load
    /// address.
    fn rst_target(&self, v: u8) -> u16 {
        if self.interrupts {
            u16::from(v)
        } else {
            self.load.wrapping_add(u16::from(v))
        }
    }

    fn service_interrupt(&mut self, pending: u8) {
        let bit = pending.trailing_zeros() as u8;
        self.cpu.ime = false;
        self.iflag &= !(1 << bit);
        self.idle();
        self.idle();
        let pc = self.cpu.pc;
        self.push16(pc);
        self.idle();
        self.cpu.pc = 0x40 + 8 * u16::from(bit);
    }

    /// Runs one instruction, or one halted cycle, or an interrupt entry.
    pub(crate) fn cpu_step(&mut self) {
        if self.cpu.ei_delay > 0 {
            self.cpu.ei_delay -= 1;
            if self.cpu.ei_delay == 0 {
                self.cpu.ime = true;
            }
        }
        if self.interrupts {
            let t = self.now;
            self.timer_sync(t);
            let pending = self.ie & self.iflag & 0x1F;
            if pending != 0 {
                self.cpu.halted = false;
                if self.cpu.ime {
                    self.service_interrupt(pending);
                    return;
                }
            }
        }
        if self.cpu.halted || self.cpu.locked {
            self.idle();
            return;
        }
        let op = self.fetch8();
        self.execute(op);
    }

    fn execute(&mut self, op: u8) {
        match op {
            0x00 => {}
            0x01 | 0x11 | 0x21 | 0x31 => {
                let v = self.fetch16();
                self.set_rr(op >> 4, v);
            }
            0x02 => {
                let (a, bc) = (self.cpu.a, self.cpu.bc());
                self.wr(bc, a);
            }
            0x12 => {
                let (a, de) = (self.cpu.a, self.cpu.de());
                self.wr(de, a);
            }
            0x22 | 0x32 => {
                let (a, hl) = (self.cpu.a, self.cpu.hl());
                self.wr(hl, a);
                let n = if op == 0x22 {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                };
                self.cpu.set_hl(n);
            }
            0x0A => {
                let bc = self.cpu.bc();
                self.cpu.a = self.rd(bc);
            }
            0x1A => {
                let de = self.cpu.de();
                self.cpu.a = self.rd(de);
            }
            0x2A | 0x3A => {
                let hl = self.cpu.hl();
                self.cpu.a = self.rd(hl);
                let n = if op == 0x2A {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                };
                self.cpu.set_hl(n);
            }
            0x03 | 0x13 | 0x23 | 0x33 => {
                let v = self.get_rr(op >> 4).wrapping_add(1);
                self.set_rr(op >> 4, v);
                self.idle();
            }
            0x0B | 0x1B | 0x2B | 0x3B => {
                let v = self.get_rr(op >> 4).wrapping_sub(1);
                self.set_rr(op >> 4, v);
                self.idle();
            }
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                let r = op >> 3;
                let v = self.get_r(r);
                let n = self.inc8(v);
                self.set_r(r, n);
            }
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                let r = op >> 3;
                let v = self.get_r(r);
                let n = self.dec8(v);
                self.set_r(r, n);
            }
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
                let v = self.fetch8();
                self.set_r(op >> 3, v);
            }
            0x07 | 0x0F | 0x17 | 0x1F => {
                let a = self.cpu.a;
                let r = self.cb_shift((op >> 3) & 3, a);
                self.cpu.a = r;
                self.cpu.f &= !FZ;
            }
            0x08 => {
                let a = self.fetch16();
                let sp = self.cpu.sp;
                self.wr(a, sp as u8);
                self.wr(a.wrapping_add(1), (sp >> 8) as u8);
            }
            0x09 | 0x19 | 0x29 | 0x39 => {
                let hl = self.cpu.hl();
                let v = self.get_rr(op >> 4);
                let (r, c) = hl.overflowing_add(v);
                self.cpu.set_flag(FN, false);
                self.cpu.set_flag(FH, (hl & 0xFFF) + (v & 0xFFF) > 0xFFF);
                self.cpu.set_flag(FC, c);
                self.cpu.set_hl(r);
                self.idle();
            }
            0x10 => {
                // The second byte is skipped without a cycle of its own.
                self.cpu.pc = self.cpu.pc.wrapping_add(1);
                if self.interrupts {
                    // STOP ends when a button is pressed; no input exists.
                } else if self.mem[0xFF4D] & 1 != 0 {
                    // A speed switch that a color console's driver arms. The
                    // switch does not happen, and the routine goes on.
                    self.mem[0xFF4D] = (self.mem[0xFF4D] ^ 0x80) & 0xFE;
                    self.warn(
                        WARN_SPEED,
                        "A routine switched the speed; the core does not.",
                    );
                } else {
                    self.cpu.stopped = true;
                    self.warn(WARN_STOP, "A routine executed STOP.");
                }
            }
            0x18 => {
                let e = self.fetch8() as i8;
                self.cpu.pc = self.cpu.pc.wrapping_add(e as u16);
                self.idle();
            }
            0x20 | 0x28 | 0x30 | 0x38 => {
                let e = self.fetch8() as i8;
                if self.cpu.cond(op >> 3) {
                    self.cpu.pc = self.cpu.pc.wrapping_add(e as u16);
                    self.idle();
                }
            }
            0x27 => self.daa(),
            0x2F => {
                self.cpu.a = !self.cpu.a;
                self.cpu.set_flag(FN, true);
                self.cpu.set_flag(FH, true);
            }
            0x37 => {
                self.cpu.set_flag(FN, false);
                self.cpu.set_flag(FH, false);
                self.cpu.set_flag(FC, true);
            }
            0x3F => {
                let c = self.cpu.flag(FC);
                self.cpu.set_flag(FN, false);
                self.cpu.set_flag(FH, false);
                self.cpu.set_flag(FC, !c);
            }
            0x76 => self.halt(),
            0x40..=0x7F => {
                let v = self.get_r(op);
                self.set_r(op >> 3, v);
            }
            0x80..=0xBF => {
                let v = self.get_r(op);
                self.alu(op >> 3, v);
            }
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                self.idle();
                if self.cpu.cond(op >> 3) {
                    self.cpu.pc = self.pop16();
                    self.idle();
                }
            }
            0xC9 | 0xD9 => {
                self.cpu.pc = self.pop16();
                self.idle();
                if op == 0xD9 {
                    self.cpu.ime = true;
                }
            }
            0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                let v = self.pop16();
                match op >> 4 & 3 {
                    0 => self.cpu.set_bc(v),
                    1 => self.cpu.set_de(v),
                    2 => self.cpu.set_hl(v),
                    _ => self.cpu.set_af(v),
                }
            }
            0xC5 | 0xD5 | 0xE5 | 0xF5 => {
                let v = match op >> 4 & 3 {
                    0 => self.cpu.bc(),
                    1 => self.cpu.de(),
                    2 => self.cpu.hl(),
                    _ => self.cpu.af(),
                };
                self.idle();
                self.push16(v);
            }
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                let a = self.fetch16();
                if self.cpu.cond(op >> 3) {
                    self.cpu.pc = a;
                    self.idle();
                }
            }
            0xC3 => {
                self.cpu.pc = self.fetch16();
                self.idle();
            }
            0xE9 => self.cpu.pc = self.cpu.hl(),
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let a = self.fetch16();
                if self.cpu.cond(op >> 3) {
                    self.idle();
                    let pc = self.cpu.pc;
                    self.push16(pc);
                    self.cpu.pc = a;
                }
            }
            0xCD => {
                let a = self.fetch16();
                self.idle();
                let pc = self.cpu.pc;
                self.push16(pc);
                self.cpu.pc = a;
            }
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                let v = self.fetch8();
                self.alu(op >> 3, v);
            }
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                self.idle();
                let pc = self.cpu.pc;
                self.push16(pc);
                self.cpu.pc = self.rst_target(op & 0x38);
            }
            0xCB => {
                let cb = self.fetch8();
                self.execute_cb(cb);
            }
            0xE0 => {
                let n = self.fetch8();
                let a = self.cpu.a;
                self.wr(0xFF00 | u16::from(n), a);
            }
            0xF0 => {
                let n = self.fetch8();
                self.cpu.a = self.rd(0xFF00 | u16::from(n));
            }
            0xE2 => {
                let (c, a) = (self.cpu.c, self.cpu.a);
                self.wr(0xFF00 | u16::from(c), a);
            }
            0xF2 => {
                let c = self.cpu.c;
                self.cpu.a = self.rd(0xFF00 | u16::from(c));
            }
            0xEA => {
                let n = self.fetch16();
                let a = self.cpu.a;
                self.wr(n, a);
            }
            0xFA => {
                let n = self.fetch16();
                self.cpu.a = self.rd(n);
            }
            0xE8 => {
                let e = self.fetch8();
                self.cpu.sp = self.sp_plus(e);
                self.idle();
                self.idle();
            }
            0xF8 => {
                let e = self.fetch8();
                let r = self.sp_plus(e);
                self.cpu.set_hl(r);
                self.idle();
            }
            0xF9 => {
                self.cpu.sp = self.cpu.hl();
                self.idle();
            }
            0xF3 => {
                self.cpu.ime = false;
                self.cpu.ei_delay = 0;
            }
            0xFB => {
                if !self.cpu.ime && self.cpu.ei_delay == 0 {
                    self.cpu.ei_delay = 2;
                }
            }
            _ => {
                self.cpu.locked = true;
                self.warn(
                    WARN_LOCK,
                    "A routine executed an opcode that locks the CPU.",
                );
            }
        }
    }

    fn halt(&mut self) {
        if self.interrupts {
            let pending = self.ie & self.iflag & 0x1F != 0;
            if !self.cpu.ime && pending {
                self.cpu.halt_bug = true;
            } else {
                self.cpu.halted = true;
            }
        } else {
            self.cpu.halted = true;
            self.warn(WARN_HALT, "A routine executed HALT.");
        }
    }

    fn execute_cb(&mut self, op: u8) {
        let r = op & 7;
        let bit = (op >> 3) & 7;
        match op >> 6 {
            0 => {
                let v = self.get_r(r);
                let n = self.cb_shift(op >> 3, v);
                self.set_r(r, n);
            }
            1 => {
                let v = self.get_r(r);
                self.cpu.set_flag(FZ, v >> bit & 1 == 0);
                self.cpu.set_flag(FN, false);
                self.cpu.set_flag(FH, true);
            }
            2 => {
                let v = self.get_r(r);
                self.set_r(r, v & !(1 << bit));
            }
            _ => {
                let v = self.get_r(r);
                self.set_r(r, v | 1 << bit);
            }
        }
    }
}

fn flags(z: bool, n: bool, h: bool, c: bool) -> u8 {
    u8::from(z) << 7 | u8::from(n) << 6 | u8::from(h) << 5 | u8::from(c) << 4
}
