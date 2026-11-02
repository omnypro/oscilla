//! The 6502 of the 2A03, one bus cycle at a time. It has no decimal mode and
//! no interrupts here: a sound driver runs in a routine that the player
//! calls. Written from public documentation of the instruction set and its
//! timing.

use crate::bus::{WARN_BRK, WARN_LOCK, WARN_UNSTABLE};
use crate::state::*;

const C: u8 = 0x01;
const Z: u8 = 0x02;
const I: u8 = 0x04;
const D: u8 = 0x08;
const V: u8 = 0x40;
const N: u8 = 0x80;
const UNUSED: u8 = 0x20;
const BREAK: u8 = 0x10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Imp,
    Acc,
    Imm,
    Zp,
    Zpx,
    Zpy,
    Abs,
    Abx,
    Aby,
    Izx,
    Izy,
    Rel,
    Ind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Lda,
    Ldx,
    Ldy,
    Sta,
    Stx,
    Sty,
    Sax,
    Lax,
    Ora,
    And,
    Eor,
    Adc,
    Sbc,
    Cmp,
    Cpx,
    Cpy,
    Bit,
    Asl,
    Lsr,
    Rol,
    Ror,
    Inc,
    Dec,
    Slo,
    Rla,
    Sre,
    Rra,
    Dcp,
    Isc,
    Anc,
    Alr,
    Arr,
    Axs,
    Nop,
    /// A branch on the flag in bit `n` of the first field being equal to the
    /// second.
    Branch(u8, bool),
    Jmp,
    Jsr,
    Rts,
    Rti,
    Brk,
    Pha,
    Php,
    Pla,
    Plp,
    Tax,
    Tay,
    Txa,
    Tya,
    Tsx,
    Txs,
    Inx,
    Iny,
    Dex,
    Dey,
    Clc,
    Sec,
    Cli,
    Sei,
    Clv,
    Cld,
    Sed,
    /// An opcode whose result depends on the chip's analog behavior. The core
    /// reads its operand and does nothing else.
    Unstable,
    /// An opcode that locks the CPU.
    Kil,
}

/// The operation and addressing mode of an opcode.
fn decode(op: u8) -> (Op, Mode) {
    use Mode::*;
    use Op::*;
    let aaa = usize::from(op >> 5);
    let bbb = usize::from(op >> 2 & 7);
    match op {
        0x00 => return (Brk, Imp),
        0x20 => return (Jsr, Abs),
        0x40 => return (Rti, Imp),
        0x60 => return (Rts, Imp),
        0x08 => return (Php, Imp),
        0x28 => return (Plp, Imp),
        0x48 => return (Pha, Imp),
        0x68 => return (Pla, Imp),
        0x88 => return (Dey, Imp),
        0xA8 => return (Tay, Imp),
        0xC8 => return (Iny, Imp),
        0xE8 => return (Inx, Imp),
        0x18 => return (Clc, Imp),
        0x38 => return (Sec, Imp),
        0x58 => return (Cli, Imp),
        0x78 => return (Sei, Imp),
        0x98 => return (Tya, Imp),
        0xB8 => return (Clv, Imp),
        0xD8 => return (Cld, Imp),
        0xF8 => return (Sed, Imp),
        0x8A => return (Txa, Imp),
        0x9A => return (Txs, Imp),
        0xAA => return (Tax, Imp),
        0xBA => return (Tsx, Imp),
        0xCA => return (Dex, Imp),
        0xEA => return (Nop, Imp),
        0x4C => return (Jmp, Abs),
        0x6C => return (Jmp, Ind),
        _ => {}
    }
    if op & 0x1F == 0x10 {
        // N, V, C and Z, each taken on clear then on set.
        let flag = [N, V, C, Z][aaa >> 1];
        return (Branch(flag, aaa & 1 != 0), Rel);
    }
    match op & 3 {
        1 => {
            let ops = [Ora, And, Eor, Adc, Sta, Lda, Cmp, Sbc];
            let modes = [Izx, Zp, Imm, Abs, Izy, Zpx, Aby, Abx];
            if op == 0x89 {
                return (Nop, Imm);
            }
            (ops[aaa], modes[bbb])
        }
        2 => {
            let ops = [Asl, Rol, Lsr, Ror, Stx, Ldx, Dec, Inc];
            match bbb {
                0 => {
                    if op == 0xA2 {
                        (Ldx, Imm)
                    } else if op & 0x1F == 0x02 && aaa < 4 {
                        (Kil, Imp)
                    } else {
                        (Nop, Imm)
                    }
                }
                1 => (ops[aaa], Zp),
                2 => (ops[aaa], Acc),
                3 => (ops[aaa], Abs),
                4 => (Kil, Imp),
                5 => (ops[aaa], if aaa == 4 || aaa == 5 { Zpy } else { Zpx }),
                6 => (Nop, Imp),
                _ => match op {
                    0x9E => (Unstable, Aby),
                    0xBE => (Ldx, Aby),
                    _ => (ops[aaa], Abx),
                },
            }
        }
        3 => {
            let ops = [Slo, Rla, Sre, Rra, Sax, Lax, Dcp, Isc];
            match bbb {
                0 => (ops[aaa], Izx),
                1 => (ops[aaa], Zp),
                2 => match op {
                    0x0B | 0x2B => (Anc, Imm),
                    0x4B => (Alr, Imm),
                    0x6B => (Arr, Imm),
                    0x8B | 0xAB => (Unstable, Imm),
                    0xCB => (Axs, Imm),
                    _ => (Sbc, Imm),
                },
                3 => (ops[aaa], Abs),
                4 => match op {
                    0x93 => (Unstable, Izy),
                    _ => (ops[aaa], Izy),
                },
                5 => (ops[aaa], if aaa == 4 || aaa == 5 { Zpy } else { Zpx }),
                6 => match op {
                    0x9B | 0xBB => (Unstable, Aby),
                    _ => (ops[aaa], Aby),
                },
                _ => match op {
                    0x9F => (Unstable, Aby),
                    0xBF => (Lax, Aby),
                    _ => (ops[aaa], Abx),
                },
            }
        }
        _ => match bbb {
            0 => match aaa {
                5 => (Ldy, Imm),
                6 => (Cpy, Imm),
                7 => (Cpx, Imm),
                _ => (Nop, Imm),
            },
            1 => ([Nop, Bit, Nop, Nop, Sty, Ldy, Cpy, Cpx][aaa], Zp),
            3 => ([Nop, Bit, Nop, Nop, Sty, Ldy, Cpy, Cpx][aaa], Abs),
            5 => match aaa {
                4 => (Sty, Zpx),
                5 => (Ldy, Zpx),
                _ => (Nop, Zpx),
            },
            _ => match aaa {
                4 => (Unstable, Abx),
                5 => (Ldy, Abx),
                _ => (Nop, Abx),
            },
        },
    }
}

impl Machine {
    fn fetch(&mut self) -> u8 {
        let pc = self.cpu.pc;
        self.cpu.pc = pc.wrapping_add(1);
        self.read(pc)
    }

    fn push(&mut self, v: u8) {
        let sp = self.cpu.sp;
        self.write(0x100 | u16::from(sp), v);
        self.cpu.sp = sp.wrapping_sub(1);
    }

    fn pull(&mut self) -> u8 {
        self.cpu.sp = self.cpu.sp.wrapping_add(1);
        let sp = self.cpu.sp;
        self.read(0x100 | u16::from(sp))
    }

    fn set_nz(&mut self, v: u8) {
        self.cpu.p = self.cpu.p & !(N | Z) | (v & N) | if v == 0 { Z } else { 0 };
    }

    fn flag(&self, f: u8) -> bool {
        self.cpu.p & f != 0
    }

    fn set_flag(&mut self, f: u8, on: bool) {
        if on {
            self.cpu.p |= f;
        } else {
            self.cpu.p &= !f;
        }
    }

    /// The address an operand lives at. `store` is true for an instruction
    /// that writes: an indexed address then takes its extra cycle whether or
    /// not it crosses a page.
    fn address(&mut self, mode: Mode, store: bool) -> u16 {
        match mode {
            Mode::Imm => {
                let pc = self.cpu.pc;
                self.cpu.pc = pc.wrapping_add(1);
                pc
            }
            Mode::Zp => u16::from(self.fetch()),
            Mode::Zpx | Mode::Zpy => {
                let z = self.fetch();
                self.read(u16::from(z));
                let index = if mode == Mode::Zpx {
                    self.cpu.x
                } else {
                    self.cpu.y
                };
                u16::from(z.wrapping_add(index))
            }
            Mode::Abs => {
                let lo = u16::from(self.fetch());
                u16::from(self.fetch()) << 8 | lo
            }
            Mode::Abx | Mode::Aby => {
                let lo = u16::from(self.fetch());
                let hi = u16::from(self.fetch());
                let index = if mode == Mode::Abx {
                    self.cpu.x
                } else {
                    self.cpu.y
                };
                self.indexed(hi << 8 | lo, index, store)
            }
            Mode::Izx => {
                let z = self.fetch();
                self.read(u16::from(z));
                let p = z.wrapping_add(self.cpu.x);
                let lo = u16::from(self.read(u16::from(p)));
                u16::from(self.read(u16::from(p.wrapping_add(1)))) << 8 | lo
            }
            Mode::Izy => {
                let z = self.fetch();
                let lo = u16::from(self.read(u16::from(z)));
                let hi = u16::from(self.read(u16::from(z.wrapping_add(1))));
                let y = self.cpu.y;
                self.indexed(hi << 8 | lo, y, store)
            }
            Mode::Imp | Mode::Acc | Mode::Rel | Mode::Ind => 0,
        }
    }

    fn indexed(&mut self, base: u16, index: u8, store: bool) -> u16 {
        let addr = base.wrapping_add(u16::from(index));
        if store || (addr ^ base) & 0xFF00 != 0 {
            self.read(base & 0xFF00 | addr & 0xFF);
        }
        addr
    }

    fn adc(&mut self, v: u8) {
        let a = self.cpu.a;
        let sum = u16::from(a) + u16::from(v) + u16::from(self.cpu.p & C);
        let r = sum as u8;
        self.set_flag(C, sum > 0xFF);
        self.set_flag(V, (a ^ r) & (v ^ r) & 0x80 != 0);
        self.cpu.a = r;
        self.set_nz(r);
    }

    fn compare(&mut self, reg: u8, v: u8) {
        self.set_flag(C, reg >= v);
        self.set_nz(reg.wrapping_sub(v));
    }

    fn asl(&mut self, v: u8) -> u8 {
        self.set_flag(C, v & 0x80 != 0);
        let r = v << 1;
        self.set_nz(r);
        r
    }

    fn lsr(&mut self, v: u8) -> u8 {
        self.set_flag(C, v & 1 != 0);
        let r = v >> 1;
        self.set_nz(r);
        r
    }

    fn rol(&mut self, v: u8) -> u8 {
        let c = self.cpu.p & C;
        self.set_flag(C, v & 0x80 != 0);
        let r = v << 1 | c;
        self.set_nz(r);
        r
    }

    fn ror(&mut self, v: u8) -> u8 {
        let c = self.cpu.p & C;
        self.set_flag(C, v & 1 != 0);
        let r = v >> 1 | c << 7;
        self.set_nz(r);
        r
    }

    /// The part of a read-modify-write instruction that changes the byte.
    fn modify(&mut self, op: Op, v: u8) -> u8 {
        match op {
            Op::Asl => self.asl(v),
            Op::Lsr => self.lsr(v),
            Op::Rol => self.rol(v),
            Op::Ror => self.ror(v),
            Op::Inc => {
                let r = v.wrapping_add(1);
                self.set_nz(r);
                r
            }
            Op::Dec => {
                let r = v.wrapping_sub(1);
                self.set_nz(r);
                r
            }
            Op::Slo => {
                let r = self.asl(v);
                self.cpu.a |= r;
                let a = self.cpu.a;
                self.set_nz(a);
                r
            }
            Op::Rla => {
                let r = self.rol(v);
                self.cpu.a &= r;
                let a = self.cpu.a;
                self.set_nz(a);
                r
            }
            Op::Sre => {
                let r = self.lsr(v);
                self.cpu.a ^= r;
                let a = self.cpu.a;
                self.set_nz(a);
                r
            }
            Op::Rra => {
                let r = self.ror(v);
                self.adc(r);
                r
            }
            Op::Dcp => {
                let r = v.wrapping_sub(1);
                let a = self.cpu.a;
                self.compare(a, r);
                r
            }
            Op::Isc => {
                let r = v.wrapping_add(1);
                self.adc(!r);
                r
            }
            _ => v,
        }
    }

    /// Runs one instruction.
    pub(crate) fn cpu_step(&mut self) {
        let opcode = self.fetch();
        let (op, mode) = decode(opcode);
        match op {
            Op::Branch(flag, on) => {
                let offset = self.fetch() as i8;
                if self.flag(flag) == on {
                    let pc = self.cpu.pc;
                    self.read(pc);
                    let target = pc.wrapping_add(offset as u16);
                    if (target ^ pc) & 0xFF00 != 0 {
                        self.read(pc & 0xFF00 | target & 0xFF);
                    }
                    self.cpu.pc = target;
                }
            }
            Op::Jmp => {
                let lo = u16::from(self.fetch());
                let hi = u16::from(self.fetch());
                let addr = hi << 8 | lo;
                if mode == Mode::Ind {
                    let lo = u16::from(self.read(addr));
                    // The high byte does not carry into the next page.
                    let hi = u16::from(self.read(addr & 0xFF00 | addr.wrapping_add(1) & 0xFF));
                    self.cpu.pc = hi << 8 | lo;
                } else {
                    self.cpu.pc = addr;
                }
            }
            Op::Jsr => {
                let lo = u16::from(self.fetch());
                let sp = self.cpu.sp;
                self.read(0x100 | u16::from(sp));
                let pc = self.cpu.pc;
                self.push((pc >> 8) as u8);
                self.push(pc as u8);
                let hi = u16::from(self.fetch());
                self.cpu.pc = hi << 8 | lo;
            }
            Op::Rts => {
                let pc = self.cpu.pc;
                self.read(pc);
                let sp = self.cpu.sp;
                self.read(0x100 | u16::from(sp));
                let lo = u16::from(self.pull());
                let hi = u16::from(self.pull());
                let pc = hi << 8 | lo;
                self.read(pc);
                self.cpu.pc = pc.wrapping_add(1);
            }
            Op::Rti => {
                let pc = self.cpu.pc;
                self.read(pc);
                let sp = self.cpu.sp;
                self.read(0x100 | u16::from(sp));
                let p = self.pull();
                self.cpu.p = p & !BREAK | UNUSED;
                let lo = u16::from(self.pull());
                let hi = u16::from(self.pull());
                self.cpu.pc = hi << 8 | lo;
            }
            Op::Brk => {
                self.warn(
                    WARN_BRK,
                    "A routine executed BRK, which the core does not run.",
                );
                self.cpu.locked = true;
            }
            Op::Kil => {
                self.warn(WARN_LOCK, "The CPU met an opcode that locks it up.");
                self.cpu.locked = true;
            }
            Op::Pha | Op::Php => {
                let pc = self.cpu.pc;
                self.read(pc);
                let v = if op == Op::Pha {
                    self.cpu.a
                } else {
                    self.cpu.p | BREAK | UNUSED
                };
                self.push(v);
            }
            Op::Pla | Op::Plp => {
                let pc = self.cpu.pc;
                self.read(pc);
                let sp = self.cpu.sp;
                self.read(0x100 | u16::from(sp));
                let v = self.pull();
                if op == Op::Pla {
                    self.cpu.a = v;
                    self.set_nz(v);
                } else {
                    self.cpu.p = v & !BREAK | UNUSED;
                }
            }
            Op::Sta | Op::Stx | Op::Sty | Op::Sax => {
                let addr = self.address(mode, true);
                let v = match op {
                    Op::Sta => self.cpu.a,
                    Op::Stx => self.cpu.x,
                    Op::Sty => self.cpu.y,
                    _ => self.cpu.a & self.cpu.x,
                };
                self.write(addr, v);
            }
            Op::Asl
            | Op::Lsr
            | Op::Rol
            | Op::Ror
            | Op::Inc
            | Op::Dec
            | Op::Slo
            | Op::Rla
            | Op::Sre
            | Op::Rra
            | Op::Dcp
            | Op::Isc => {
                if mode == Mode::Acc {
                    let pc = self.cpu.pc;
                    self.read(pc);
                    let a = self.cpu.a;
                    self.cpu.a = self.modify(op, a);
                } else {
                    let addr = self.address(mode, true);
                    let v = self.read(addr);
                    self.write(addr, v);
                    let r = self.modify(op, v);
                    self.write(addr, r);
                }
            }
            Op::Unstable => {
                self.warn(
                    WARN_UNSTABLE,
                    "A routine ran an opcode with unstable results; the core skips it.",
                );
                let addr = self.address(mode, false);
                self.read(addr);
            }
            Op::Nop if mode != Mode::Imp => {
                let addr = self.address(mode, false);
                self.read(addr);
            }
            Op::Lda
            | Op::Ldx
            | Op::Ldy
            | Op::Lax
            | Op::Ora
            | Op::And
            | Op::Eor
            | Op::Adc
            | Op::Sbc
            | Op::Cmp
            | Op::Cpx
            | Op::Cpy
            | Op::Bit
            | Op::Anc
            | Op::Alr
            | Op::Arr
            | Op::Axs => {
                let addr = self.address(mode, false);
                let v = self.read(addr);
                match op {
                    Op::Lda => {
                        self.cpu.a = v;
                        self.set_nz(v);
                    }
                    Op::Ldx => {
                        self.cpu.x = v;
                        self.set_nz(v);
                    }
                    Op::Ldy => {
                        self.cpu.y = v;
                        self.set_nz(v);
                    }
                    Op::Lax => {
                        self.cpu.a = v;
                        self.cpu.x = v;
                        self.set_nz(v);
                    }
                    Op::Ora => {
                        self.cpu.a |= v;
                        let a = self.cpu.a;
                        self.set_nz(a);
                    }
                    Op::And => {
                        self.cpu.a &= v;
                        let a = self.cpu.a;
                        self.set_nz(a);
                    }
                    Op::Eor => {
                        self.cpu.a ^= v;
                        let a = self.cpu.a;
                        self.set_nz(a);
                    }
                    Op::Adc => self.adc(v),
                    Op::Sbc => self.adc(!v),
                    Op::Cmp => {
                        let a = self.cpu.a;
                        self.compare(a, v);
                    }
                    Op::Cpx => {
                        let x = self.cpu.x;
                        self.compare(x, v);
                    }
                    Op::Cpy => {
                        let y = self.cpu.y;
                        self.compare(y, v);
                    }
                    Op::Bit => {
                        let a = self.cpu.a;
                        self.set_flag(Z, a & v == 0);
                        self.cpu.p = self.cpu.p & !(N | V) | (v & (N | V));
                    }
                    Op::Anc => {
                        self.cpu.a &= v;
                        let a = self.cpu.a;
                        self.set_nz(a);
                        self.set_flag(C, a & 0x80 != 0);
                    }
                    Op::Alr => {
                        let a = self.cpu.a & v;
                        self.cpu.a = self.lsr(a);
                    }
                    Op::Arr => {
                        let a = self.cpu.a & v;
                        let r = self.ror(a);
                        self.cpu.a = r;
                        self.set_flag(C, r & 0x40 != 0);
                        self.set_flag(V, (r >> 6 ^ r >> 5) & 1 != 0);
                    }
                    _ => {
                        // AXS: X = (A and X) - operand, with the compare's flags.
                        let ax = self.cpu.a & self.cpu.x;
                        self.compare(ax, v);
                        self.cpu.x = ax.wrapping_sub(v);
                    }
                }
            }
            _ => {
                // The one-byte instructions spend a second cycle on a read of
                // the byte after the opcode.
                let pc = self.cpu.pc;
                self.read(pc);
                match op {
                    Op::Tax => {
                        self.cpu.x = self.cpu.a;
                        let x = self.cpu.x;
                        self.set_nz(x);
                    }
                    Op::Tay => {
                        self.cpu.y = self.cpu.a;
                        let y = self.cpu.y;
                        self.set_nz(y);
                    }
                    Op::Txa => {
                        self.cpu.a = self.cpu.x;
                        let a = self.cpu.a;
                        self.set_nz(a);
                    }
                    Op::Tya => {
                        self.cpu.a = self.cpu.y;
                        let a = self.cpu.a;
                        self.set_nz(a);
                    }
                    Op::Tsx => {
                        self.cpu.x = self.cpu.sp;
                        let x = self.cpu.x;
                        self.set_nz(x);
                    }
                    Op::Txs => self.cpu.sp = self.cpu.x,
                    Op::Inx => {
                        self.cpu.x = self.cpu.x.wrapping_add(1);
                        let x = self.cpu.x;
                        self.set_nz(x);
                    }
                    Op::Iny => {
                        self.cpu.y = self.cpu.y.wrapping_add(1);
                        let y = self.cpu.y;
                        self.set_nz(y);
                    }
                    Op::Dex => {
                        self.cpu.x = self.cpu.x.wrapping_sub(1);
                        let x = self.cpu.x;
                        self.set_nz(x);
                    }
                    Op::Dey => {
                        self.cpu.y = self.cpu.y.wrapping_sub(1);
                        let y = self.cpu.y;
                        self.set_nz(y);
                    }
                    Op::Clc => self.set_flag(C, false),
                    Op::Sec => self.set_flag(C, true),
                    Op::Cli => self.set_flag(I, false),
                    Op::Sei => self.set_flag(I, true),
                    Op::Clv => self.set_flag(V, false),
                    Op::Cld => self.set_flag(D, false),
                    Op::Sed => self.set_flag(D, true),
                    _ => {}
                }
            }
        }
    }
}
