//! The HuC6280: a 65C02 with eight memory mapping registers, a high and a low
//! speed, block transfers and the registers that the PC Engine adds. Written
//! from the HuC6280 hardware and software manuals. Where they say nothing, the
//! core follows the MOS and WDC documentation of the 6502 family. It runs one
//! instruction at a time and counts the cycles that the manuals give.
//!
//! The zero page is logical 0x2000 to 0x20FF and the stack is logical 0x2100
//! to 0x21FF. The software manual (3.3 to 3.6) gives 20h as the high byte of
//! every zero-page address, and the hardware manual (2.1.4) gives 21h as the
//! high byte of every stack address. Both pages lie in the 8 KB that MPR1
//! maps. The hardware manual's memory map shows them at physical 0x1F0000 and
//! 0x1F0100 when MPR1 holds 0xF8.

// Unknown: the manuals leave these behaviors open. The core takes the simplest
// choice that does not contradict them, and each line gives that choice.
// - The 22 opcodes left blank in the instruction map run as one-byte NOPs of
//   2 cycles.
// - An indexed address that crosses a page takes no extra cycle. The manuals
//   give one count for each addressing mode.
// - A zero-page pointer at 0xFF takes its high byte from 0x00 of the zero
//   page, in (IND), (IND,X) and (IND),Y.
// - The interrupt sequence takes 8 cycles. BRK takes 8, and it pushes the same
//   three bytes and reads a vector the same way.
// - The core checks for an interrupt before each instruction with the I flag
//   as it stands then. An interrupt can come right after CLI, PLP or RTI
//   clears I, and none comes after SEI sets it.
// - No interrupt comes during a block transfer. The hardware manual says the
//   chip samples the interrupt inputs in the last bus cycle of an
//   instruction, so the core takes a pending interrupt after the transfer
//   ends.
// - A block transfer reads A, X and Y back from the stack at its end, as the
//   flowcharts of TAI and TIA show. A transfer that overwrites those three
//   stack bytes leaves the new values in the registers.
// - TAM writes A to every MPR whose bit is set in its operand, and to none
//   when no bit is set. TMA with more than one bit set reads the lowest
//   numbered of those MPRs. TMA with no bit set leaves A as it was.
// - ST0, ST1 and ST2 write to physical 0x1FE000, 0x1FE002 and 0x1FE003. The
//   hardware manual names only A20 (high), CE7 and the code on A1 and A0. The
//   other bits come from its device table, which puts the HuC6270 at 0x1FE000
//   to 0x1FE3FF.
// - In decimal mode ADC and SBC set N and Z from the decimal result. A digit
//   above 9 goes through the same adjustment as a valid one.
// - Under the T flag, ADC, AND, EOR and ORA set their flags from the result
//   that they store in M(X). A decimal ADC under the T flag takes both extra
//   counts: 3 for the memory operation and 1 for the decimal adjustment.
// - TSB and TRB set Z from A AND M, as the WDC 65C02 documentation gives. The
//   HuC6280 manual marks Z as changed and does not say from what.
// - An interrupt pushes B as 0 and leaves the B bit of the live status
//   register as it was.
// - PLP and RTI load all eight bits of the status register, T included. A T
//   pulled as 1 applies to the next instruction.
// - CSL and CSH take 3 cycles, and the manuals do not say at which speed. The
//   core counts them at the new speed. That is the speed that `high_speed`
//   shows when `step` returns.
// - `reset` leaves A, X, Y, S and the N, V, B, Z and C flags as they were. The
//   hardware manual calls S invalid after a reset and says nothing of the
//   others.
// - `reset` leaves the MPRs alone. The chip sets MPR7 to 0x00 at a reset; the
//   machine sets the MPRs itself before it calls `reset`.
// - SBC, CMP, CPX and CPY set C when no borrow occurs. The software manual
//   gives SBC as A - M - (not C) with "carry not, borrow", while the hardware
//   manual (2.1.5) says C is set if a borrow occurs. The core follows the
//   software manual and the 6502 family.

/// What the processor sees outside itself. Addresses are the 21-bit physical
/// addresses that the memory mapping registers make from the 16-bit logical
/// ones.
pub trait Bus {
    fn read(&mut self, physical: u32) -> u8;
    fn write(&mut self, physical: u32, value: u8);
    /// The vector address (0xFFF6 for IRQ2, 0xFFF8 for IRQ1, 0xFFFA for the
    /// timer) of the interrupt that is pending and not masked, or `None`. The
    /// processor asks between instructions while its I flag is clear.
    fn pending_interrupt(&mut self) -> Option<u16>;
}

/// The carry flag.
pub const C: u8 = 0x01;
/// The zero flag.
pub const Z: u8 = 0x02;
/// The interrupt disable flag.
pub const I: u8 = 0x04;
/// The decimal flag.
pub const D: u8 = 0x08;
/// The break command.
pub const B: u8 = 0x10;
/// The memory operation flag, which SET sets for the next instruction.
pub const T: u8 = 0x20;
/// The overflow flag.
pub const V: u8 = 0x40;
/// The negative flag.
pub const N: u8 = 0x80;

/// The vector of IRQ2 and of BRK.
pub const VECTOR_IRQ2: u16 = 0xFFF6;
pub const VECTOR_IRQ1: u16 = 0xFFF8;
pub const VECTOR_TIMER: u16 = 0xFFFA;
pub const VECTOR_NMI: u16 = 0xFFFC;
pub const VECTOR_RESET: u16 = 0xFFFE;

/// Where ST0 writes. ST1 and ST2 write 2 and 3 above it.
pub const VDC: u32 = 0x1F_E000;

/// The cycles of the interrupt sequence.
const INTERRUPT_CYCLES: u32 = 8;

/// The cycles of each opcode before the extras that depend on what it does: a
/// branch taken, the decimal flag, the T flag and the length of a block
/// transfer. The rows follow the hardware manual's instruction map, except
/// 0x34 and 0xB4, whose cells the map misprints; those come from the software
/// manual's pages for BIT and LDY. The blank cells of the map take 2.
#[rustfmt::skip]
const CYCLES: [u8; 256] = [
//  0  1  2   3  4  5  6  7  8  9  A  B  C  D  E  F
    8, 7, 3,  4, 6, 4, 6, 7, 3, 2, 2, 2, 7, 5, 7, 6, // 0
    2, 7, 7,  4, 6, 4, 6, 7, 2, 5, 2, 2, 7, 5, 7, 6, // 1
    7, 7, 3,  4, 4, 4, 6, 7, 4, 2, 2, 2, 5, 5, 7, 6, // 2
    2, 7, 7,  2, 4, 4, 6, 7, 2, 5, 2, 2, 5, 5, 7, 6, // 3
    7, 7, 3,  4, 8, 4, 6, 7, 3, 2, 2, 2, 4, 5, 7, 6, // 4
    2, 7, 7,  5, 3, 4, 6, 7, 2, 5, 3, 2, 2, 5, 7, 6, // 5
    7, 7, 2,  2, 4, 4, 6, 7, 4, 2, 2, 2, 7, 5, 7, 6, // 6
    2, 7, 7, 17, 4, 4, 6, 7, 2, 5, 4, 2, 7, 5, 7, 6, // 7
    4, 7, 2,  7, 4, 4, 4, 7, 2, 2, 2, 2, 5, 5, 5, 6, // 8
    2, 7, 7,  8, 4, 4, 4, 7, 2, 5, 2, 2, 5, 5, 5, 6, // 9
    2, 7, 2,  7, 4, 4, 4, 7, 2, 2, 2, 2, 5, 5, 5, 6, // A
    2, 7, 7,  8, 4, 4, 4, 7, 2, 5, 2, 2, 5, 5, 5, 6, // B
    2, 7, 2, 17, 4, 4, 6, 7, 2, 2, 2, 2, 5, 5, 7, 6, // C
    2, 7, 7, 17, 3, 4, 6, 7, 2, 5, 3, 2, 2, 5, 7, 6, // D
    2, 7, 2, 17, 4, 4, 6, 7, 2, 2, 2, 2, 5, 5, 7, 6, // E
    2, 7, 7, 17, 2, 4, 6, 7, 2, 5, 4, 2, 2, 5, 7, 6, // F
];

/// The addressing modes, in the software manual's notation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// IMPLID.
    Imp,
    /// ACC.
    Acc,
    /// IMM.
    Imm,
    /// ZP.
    Zp,
    /// ZP,X.
    Zpx,
    /// ZP,Y.
    Zpy,
    /// ABS.
    Abs,
    /// ABS,X.
    Abx,
    /// ABS,Y.
    Aby,
    /// (IND): a pointer in the zero page.
    Ind,
    /// (IND,X).
    Izx,
    /// (IND),Y.
    Izy,
    /// (ABS): JMP through a pointer anywhere.
    Iab,
    /// (ABS,X).
    Iax,
    /// REL.
    Rel,
}

/// How a block transfer moves an address after each byte.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Inc,
    Dec,
    Fixed,
    /// Up after the first byte, down after the second, and so on.
    Alt,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Ora,
    And,
    Eor,
    Adc,
    Sbc,
    Cmp,
    Cpx,
    Cpy,
    Lda,
    Ldx,
    Ldy,
    Sta,
    Stx,
    Sty,
    Stz,
    Bit,
    Tsb,
    Trb,
    Tst,
    Asl,
    Lsr,
    Rol,
    Ror,
    Inc,
    Dec,
    Rmb(u8),
    Smb(u8),
    Bbr(u8),
    Bbs(u8),
    /// A branch on the flag in the first field being set (true) or clear.
    Branch(u8, bool),
    Bra,
    Bsr,
    Jmp,
    Jsr,
    Rts,
    Rti,
    Brk,
    Pha,
    Php,
    Phx,
    Phy,
    Pla,
    Plp,
    Plx,
    Ply,
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
    Set,
    Cla,
    Clx,
    Cly,
    Sax,
    Say,
    Sxy,
    Csl,
    Csh,
    /// ST0, ST1 or ST2, with the code that goes out on A1 and A0.
    St(u8),
    Tam,
    Tma,
    /// A block transfer, with the steps of its source and its destination.
    Transfer(Step, Step),
    Nop,
    Undefined,
}

/// The operation and addressing mode of an opcode.
fn decode(opcode: u8) -> (Op, Mode) {
    use Mode::*;
    use Op::*;
    let bit = opcode >> 4 & 7;
    match opcode {
        // The ALU group of the 6502 has BIT in the cell where STA # would be.
        0x89 => (Bit, Imm),
        _ if opcode & 0x03 == 0x01 || opcode & 0x1F == 0x12 => {
            let op = [Ora, And, Eor, Adc, Sta, Lda, Cmp, Sbc][usize::from(opcode >> 5)];
            let mode = match opcode & 0x1F {
                0x01 => Izx,
                0x05 => Zp,
                0x09 => Imm,
                0x0D => Abs,
                0x11 => Izy,
                0x12 => Ind,
                0x15 => Zpx,
                0x19 => Aby,
                _ => Abx,
            };
            (op, mode)
        }
        _ if opcode & 0x0F == 0x07 => {
            let op = if opcode & 0x80 == 0 {
                Rmb(bit)
            } else {
                Smb(bit)
            };
            (op, Zp)
        }
        _ if opcode & 0x0F == 0x0F => {
            let op = if opcode & 0x80 == 0 {
                Bbr(bit)
            } else {
                Bbs(bit)
            };
            (op, Zp)
        }
        0x00 => (Brk, Imp),
        0x02 => (Sxy, Imp),
        0x03 => (St(0), Imm),
        0x04 => (Tsb, Zp),
        0x06 => (Asl, Zp),
        0x08 => (Php, Imp),
        0x0A => (Asl, Acc),
        0x0C => (Tsb, Abs),
        0x0E => (Asl, Abs),
        0x10 => (Branch(N, false), Rel),
        0x13 => (St(2), Imm),
        0x14 => (Trb, Zp),
        0x16 => (Asl, Zpx),
        0x18 => (Clc, Imp),
        0x1A => (Inc, Acc),
        0x1C => (Trb, Abs),
        0x1E => (Asl, Abx),
        0x20 => (Jsr, Abs),
        0x22 => (Sax, Imp),
        0x23 => (St(3), Imm),
        0x24 => (Bit, Zp),
        0x26 => (Rol, Zp),
        0x28 => (Plp, Imp),
        0x2A => (Rol, Acc),
        0x2C => (Bit, Abs),
        0x2E => (Rol, Abs),
        0x30 => (Branch(N, true), Rel),
        0x34 => (Bit, Zpx),
        0x36 => (Rol, Zpx),
        0x38 => (Sec, Imp),
        0x3A => (Dec, Acc),
        0x3C => (Bit, Abx),
        0x3E => (Rol, Abx),
        0x40 => (Rti, Imp),
        0x42 => (Say, Imp),
        0x43 => (Tma, Imm),
        0x44 => (Bsr, Rel),
        0x46 => (Lsr, Zp),
        0x48 => (Pha, Imp),
        0x4A => (Lsr, Acc),
        0x4C => (Jmp, Abs),
        0x4E => (Lsr, Abs),
        0x50 => (Branch(V, false), Rel),
        0x53 => (Tam, Imm),
        0x54 => (Csl, Imp),
        0x56 => (Lsr, Zpx),
        0x58 => (Cli, Imp),
        0x5A => (Phy, Imp),
        0x5E => (Lsr, Abx),
        0x60 => (Rts, Imp),
        0x62 => (Cla, Imp),
        0x64 => (Stz, Zp),
        0x66 => (Ror, Zp),
        0x68 => (Pla, Imp),
        0x6A => (Ror, Acc),
        0x6C => (Jmp, Iab),
        0x6E => (Ror, Abs),
        0x70 => (Branch(V, true), Rel),
        0x73 => (Transfer(Step::Inc, Step::Inc), Imp),
        0x74 => (Stz, Zpx),
        0x76 => (Ror, Zpx),
        0x78 => (Sei, Imp),
        0x7A => (Ply, Imp),
        0x7C => (Jmp, Iax),
        0x7E => (Ror, Abx),
        0x80 => (Bra, Rel),
        0x82 => (Clx, Imp),
        0x83 => (Tst, Zp),
        0x84 => (Sty, Zp),
        0x86 => (Stx, Zp),
        0x88 => (Dey, Imp),
        0x8A => (Txa, Imp),
        0x8C => (Sty, Abs),
        0x8E => (Stx, Abs),
        0x90 => (Branch(C, false), Rel),
        0x93 => (Tst, Abs),
        0x94 => (Sty, Zpx),
        0x96 => (Stx, Zpy),
        0x98 => (Tya, Imp),
        0x9A => (Txs, Imp),
        0x9C => (Stz, Abs),
        0x9E => (Stz, Abx),
        0xA0 => (Ldy, Imm),
        0xA2 => (Ldx, Imm),
        0xA3 => (Tst, Zpx),
        0xA4 => (Ldy, Zp),
        0xA6 => (Ldx, Zp),
        0xA8 => (Tay, Imp),
        0xAA => (Tax, Imp),
        0xAC => (Ldy, Abs),
        0xAE => (Ldx, Abs),
        0xB0 => (Branch(C, true), Rel),
        0xB3 => (Tst, Abx),
        0xB4 => (Ldy, Zpx),
        0xB6 => (Ldx, Zpy),
        0xB8 => (Clv, Imp),
        0xBA => (Tsx, Imp),
        0xBC => (Ldy, Abx),
        0xBE => (Ldx, Aby),
        0xC0 => (Cpy, Imm),
        0xC2 => (Cly, Imp),
        0xC3 => (Transfer(Step::Dec, Step::Dec), Imp),
        0xC4 => (Cpy, Zp),
        0xC6 => (Dec, Zp),
        0xC8 => (Iny, Imp),
        0xCA => (Dex, Imp),
        0xCC => (Cpy, Abs),
        0xCE => (Dec, Abs),
        0xD0 => (Branch(Z, false), Rel),
        0xD3 => (Transfer(Step::Inc, Step::Fixed), Imp),
        0xD4 => (Csh, Imp),
        0xD6 => (Dec, Zpx),
        0xD8 => (Cld, Imp),
        0xDA => (Phx, Imp),
        0xDE => (Dec, Abx),
        0xE0 => (Cpx, Imm),
        0xE3 => (Transfer(Step::Inc, Step::Alt), Imp),
        0xE4 => (Cpx, Zp),
        0xE6 => (Inc, Zp),
        0xE8 => (Inx, Imp),
        0xEA => (Nop, Imp),
        0xEC => (Cpx, Abs),
        0xEE => (Inc, Abs),
        0xF0 => (Branch(Z, true), Rel),
        0xF3 => (Transfer(Step::Alt, Step::Inc), Imp),
        0xF4 => (Set, Imp),
        0xF6 => (Inc, Zpx),
        0xF8 => (Sed, Imp),
        0xFA => (Plx, Imp),
        0xFE => (Inc, Abx),
        _ => (Undefined, Imp),
    }
}

/// The logical address of a byte of the zero page.
fn zero_page(offset: u8) -> u16 {
    0x2000 | u16::from(offset)
}

/// The logical address of a byte of the stack.
fn stack(offset: u8) -> u16 {
    0x2100 | u16::from(offset)
}

/// Where a block transfer address goes after the byte numbered `byte`.
fn advance(address: u16, step: Step, byte: u32) -> u16 {
    match step {
        Step::Inc => address.wrapping_add(1),
        Step::Dec => address.wrapping_sub(1),
        Step::Fixed => address,
        Step::Alt if byte & 1 == 0 => address.wrapping_add(1),
        Step::Alt => address.wrapping_sub(1),
    }
}

#[derive(Clone, Debug)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub pc: u16,
    pub p: u8,
    pub mpr: [u8; 8],
    pub high_speed: bool,
    /// An NMI that has come and that the processor has not taken yet.
    nmi_pending: bool,
}

impl Cpu {
    pub fn new() -> Cpu {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            s: 0xFF,
            pc: 0,
            p: 0x04,
            mpr: [0; 8],
            high_speed: false,
            nmi_pending: false,
        }
    }

    /// The physical address of a logical one under the current mapping.
    pub fn physical(&self, logical: u16) -> u32 {
        u32::from(self.mpr[usize::from(logical >> 13)]) << 13 | u32::from(logical & 0x1FFF)
    }

    /// Starts the processor the way that its RESET input does. The machine
    /// sets the MPRs first: the chip sets MPR7 to 0x00, and this method leaves
    /// all eight as they are. A reset sets I, clears D and T and selects low
    /// speed, and the program counter comes from the vector at 0xFFFE.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        self.p = (self.p | I) & !(D | T);
        self.high_speed = false;
        self.nmi_pending = false;
        self.pc = self.read_word(bus, VECTOR_RESET);
    }

    /// Signals the NMI input. The input senses an edge, so one call is one
    /// interrupt. The processor takes it before its next instruction, and the
    /// I flag does not mask it.
    pub fn nmi(&mut self) {
        self.nmi_pending = true;
    }

    /// Runs one instruction, or takes one interrupt, and returns the cycles
    /// that it took at the current speed: the speed that `high_speed` shows
    /// when this returns.
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        if self.nmi_pending {
            self.nmi_pending = false;
            return self.interrupt(bus, VECTOR_NMI);
        }
        if self.p & I == 0
            && let Some(vector) = bus.pending_interrupt()
        {
            return self.interrupt(bus, vector);
        }
        // T applies to the instruction after SET only: the chip clears it in
        // the fetch cycle of that instruction.
        let t = self.p & T != 0;
        self.p &= !T;
        let opcode = self.fetch(bus);
        let (op, mode) = decode(opcode);
        let mut cycles = u32::from(CYCLES[usize::from(opcode)]);
        match op {
            Op::Ora | Op::And | Op::Eor | Op::Adc if t => {
                // The memory operation: M(X) takes the place of A.
                let addr = self.address(bus, mode);
                let m = self.read(bus, addr);
                let target = zero_page(self.x);
                let v = self.read(bus, target);
                let r = self.alu(op, v, m);
                self.write(bus, target, r);
                cycles += 3;
                if op == Op::Adc && self.p & D != 0 {
                    cycles += 1;
                }
            }
            Op::Ora | Op::And | Op::Eor | Op::Adc | Op::Sbc => {
                let addr = self.address(bus, mode);
                let m = self.read(bus, addr);
                let a = self.a;
                self.a = self.alu(op, a, m);
                if matches!(op, Op::Adc | Op::Sbc) && self.p & D != 0 {
                    cycles += 1;
                }
            }
            Op::Cmp | Op::Cpx | Op::Cpy => {
                let addr = self.address(bus, mode);
                let m = self.read(bus, addr);
                let reg = match op {
                    Op::Cmp => self.a,
                    Op::Cpx => self.x,
                    _ => self.y,
                };
                self.set_flag(C, reg >= m);
                self.set_nz(reg.wrapping_sub(m));
            }
            Op::Lda | Op::Ldx | Op::Ldy => {
                let addr = self.address(bus, mode);
                let m = self.read(bus, addr);
                match op {
                    Op::Lda => self.a = m,
                    Op::Ldx => self.x = m,
                    _ => self.y = m,
                }
                self.set_nz(m);
            }
            Op::Sta | Op::Stx | Op::Sty | Op::Stz => {
                let addr = self.address(bus, mode);
                let v = match op {
                    Op::Sta => self.a,
                    Op::Stx => self.x,
                    Op::Sty => self.y,
                    _ => 0,
                };
                self.write(bus, addr, v);
            }
            Op::Bit => {
                let addr = self.address(bus, mode);
                let m = self.read(bus, addr);
                // N and V come from M7 and M6 in every mode. The manual's BIT
                // page gives them for the immediate mode too.
                self.test(self.a & m, m);
            }
            Op::Tst => {
                let imm = self.fetch(bus);
                let addr = self.address(bus, mode);
                let m = self.read(bus, addr);
                self.test(imm & m, m);
            }
            Op::Tsb | Op::Trb => {
                let addr = self.address(bus, mode);
                let m = self.read(bus, addr);
                self.test(self.a & m, m);
                let r = if op == Op::Tsb {
                    m | self.a
                } else {
                    m & !self.a
                };
                self.write(bus, addr, r);
            }
            Op::Asl | Op::Lsr | Op::Rol | Op::Ror | Op::Inc | Op::Dec => {
                if mode == Mode::Acc {
                    let a = self.a;
                    self.a = self.modify(op, a);
                } else {
                    let addr = self.address(bus, mode);
                    let v = self.read(bus, addr);
                    let r = self.modify(op, v);
                    self.write(bus, addr, r);
                }
            }
            Op::Rmb(n) | Op::Smb(n) => {
                let addr = self.address(bus, mode);
                let v = self.read(bus, addr);
                let mask = 1u8 << (n & 7);
                let r = if matches!(op, Op::Smb(_)) {
                    v | mask
                } else {
                    v & !mask
                };
                self.write(bus, addr, r);
            }
            Op::Bbr(n) | Op::Bbs(n) => {
                let addr = self.address(bus, mode);
                let v = self.read(bus, addr);
                let offset = self.fetch(bus);
                let set = v >> (n & 7) & 1 != 0;
                if set == matches!(op, Op::Bbs(_)) {
                    self.branch(offset);
                    cycles += 2;
                }
            }
            Op::Branch(flag, on) => {
                let offset = self.fetch(bus);
                if (self.p & flag != 0) == on {
                    self.branch(offset);
                    cycles += 2;
                }
            }
            Op::Bra => {
                let offset = self.fetch(bus);
                self.branch(offset);
            }
            Op::Bsr => {
                let offset = self.fetch(bus);
                // The stack gets the address of the last byte of BSR, and RTS
                // adds the 1.
                let last = self.pc.wrapping_sub(1);
                self.push_word(bus, last);
                self.branch(offset);
            }
            Op::Jmp => self.pc = self.address(bus, mode),
            Op::Jsr => {
                let target = self.fetch_word(bus);
                let last = self.pc.wrapping_sub(1);
                self.push_word(bus, last);
                self.pc = target;
            }
            Op::Rts => {
                let lo = u16::from(self.pull(bus));
                let hi = u16::from(self.pull(bus));
                self.pc = (hi << 8 | lo).wrapping_add(1);
            }
            Op::Rti => {
                self.p = self.pull(bus);
                let lo = u16::from(self.pull(bus));
                let hi = u16::from(self.pull(bus));
                self.pc = hi << 8 | lo;
            }
            Op::Brk => {
                // The return address skips the byte after BRK.
                let ret = self.pc.wrapping_add(1);
                self.push_word(bus, ret);
                let p = self.p | B;
                self.push(bus, p);
                self.p = (self.p | I | B) & !D;
                self.pc = self.read_word(bus, VECTOR_IRQ2);
            }
            Op::Pha => self.push(bus, self.a),
            Op::Php => self.push(bus, self.p | B),
            Op::Phx => self.push(bus, self.x),
            Op::Phy => self.push(bus, self.y),
            Op::Pla => {
                self.a = self.pull(bus);
                self.set_nz(self.a);
            }
            Op::Plp => self.p = self.pull(bus),
            Op::Plx => {
                self.x = self.pull(bus);
                self.set_nz(self.x);
            }
            Op::Ply => {
                self.y = self.pull(bus);
                self.set_nz(self.y);
            }
            Op::Tax => {
                self.x = self.a;
                self.set_nz(self.x);
            }
            Op::Tay => {
                self.y = self.a;
                self.set_nz(self.y);
            }
            Op::Txa => {
                self.a = self.x;
                self.set_nz(self.a);
            }
            Op::Tya => {
                self.a = self.y;
                self.set_nz(self.a);
            }
            Op::Tsx => {
                self.x = self.s;
                self.set_nz(self.x);
            }
            Op::Txs => self.s = self.x,
            Op::Inx => {
                self.x = self.x.wrapping_add(1);
                self.set_nz(self.x);
            }
            Op::Iny => {
                self.y = self.y.wrapping_add(1);
                self.set_nz(self.y);
            }
            Op::Dex => {
                self.x = self.x.wrapping_sub(1);
                self.set_nz(self.x);
            }
            Op::Dey => {
                self.y = self.y.wrapping_sub(1);
                self.set_nz(self.y);
            }
            Op::Clc => self.p &= !C,
            Op::Sec => self.p |= C,
            Op::Cli => self.p &= !I,
            Op::Sei => self.p |= I,
            Op::Clv => self.p &= !V,
            Op::Cld => self.p &= !D,
            Op::Sed => self.p |= D,
            Op::Set => self.p |= T,
            Op::Cla => self.a = 0,
            Op::Clx => self.x = 0,
            Op::Cly => self.y = 0,
            Op::Sax => core::mem::swap(&mut self.a, &mut self.x),
            Op::Say => core::mem::swap(&mut self.a, &mut self.y),
            Op::Sxy => core::mem::swap(&mut self.x, &mut self.y),
            Op::Csl => self.high_speed = false,
            Op::Csh => self.high_speed = true,
            Op::St(code) => {
                // The MPRs play no part: the write goes to the HuC6270.
                let v = self.fetch(bus);
                bus.write(VDC | u32::from(code), v);
            }
            Op::Tam => {
                let mask = self.fetch(bus);
                for (i, mpr) in self.mpr.iter_mut().enumerate() {
                    if mask >> i & 1 != 0 {
                        *mpr = self.a;
                    }
                }
            }
            Op::Tma => {
                let mask = self.fetch(bus);
                if mask != 0 {
                    self.a = self.mpr[(mask.trailing_zeros() & 7) as usize];
                }
            }
            Op::Transfer(source, destination) => {
                cycles = self.transfer(bus, source, destination);
            }
            Op::Nop | Op::Undefined => {}
        }
        cycles
    }

    /// The interrupt sequence: PCH, PCL and P go to the stack, and the
    /// program counter comes from `vector`.
    fn interrupt(&mut self, bus: &mut impl Bus, vector: u16) -> u32 {
        let pc = self.pc;
        self.push_word(bus, pc);
        // T goes to the stack as it stands. An interrupt that comes between
        // SET and the instruction after it leaves the memory operation for
        // that instruction, once RTI has restored T.
        let p = self.p & !B;
        self.push(bus, p);
        self.p = (self.p | I) & !(D | T);
        self.pc = self.read_word(bus, vector);
        INTERRUPT_CYCLES
    }

    /// TII, TDD, TIN, TIA or TAI: 17 cycles and 6 for each byte.
    fn transfer(&mut self, bus: &mut impl Bus, source: Step, destination: Step) -> u32 {
        let mut src = self.fetch_word(bus);
        let mut dst = self.fetch_word(bus);
        let mut length = self.fetch_word(bus);
        // A, X and Y hold the length, the source and the destination while
        // the transfer runs, so they wait on the stack. S is back where it was
        // at the end.
        let s = self.s;
        self.write(bus, stack(s), self.y);
        self.write(bus, stack(s.wrapping_sub(1)), self.a);
        self.write(bus, stack(s.wrapping_sub(2)), self.x);
        let mut bytes = 0u32;
        loop {
            let v = self.read(bus, src);
            self.write(bus, dst, v);
            src = advance(src, source, bytes);
            dst = advance(dst, destination, bytes);
            bytes += 1;
            // A length of 0 moves 65,536 bytes: the test comes after the
            // count.
            length = length.wrapping_sub(1);
            if length == 0 {
                break;
            }
        }
        self.x = self.read(bus, stack(s.wrapping_sub(2)));
        self.a = self.read(bus, stack(s.wrapping_sub(1)));
        self.y = self.read(bus, stack(s));
        17 + 6 * bytes
    }

    /// The logical address of an operand. An immediate operand's address is
    /// the program counter.
    fn address(&mut self, bus: &mut impl Bus, mode: Mode) -> u16 {
        match mode {
            Mode::Imm => {
                let pc = self.pc;
                self.pc = pc.wrapping_add(1);
                pc
            }
            Mode::Zp => zero_page(self.fetch(bus)),
            Mode::Zpx => zero_page(self.fetch(bus).wrapping_add(self.x)),
            Mode::Zpy => zero_page(self.fetch(bus).wrapping_add(self.y)),
            Mode::Abs => self.fetch_word(bus),
            Mode::Abx => self.fetch_word(bus).wrapping_add(u16::from(self.x)),
            Mode::Aby => self.fetch_word(bus).wrapping_add(u16::from(self.y)),
            Mode::Ind => {
                let z = self.fetch(bus);
                self.zero_page_word(bus, z)
            }
            Mode::Izx => {
                let z = self.fetch(bus).wrapping_add(self.x);
                self.zero_page_word(bus, z)
            }
            Mode::Izy => {
                let z = self.fetch(bus);
                self.zero_page_word(bus, z).wrapping_add(u16::from(self.y))
            }
            Mode::Iab => {
                let pointer = self.fetch_word(bus);
                self.read_word(bus, pointer)
            }
            Mode::Iax => {
                let pointer = self.fetch_word(bus).wrapping_add(u16::from(self.x));
                self.read_word(bus, pointer)
            }
            Mode::Imp | Mode::Acc | Mode::Rel => self.pc,
        }
    }

    /// ORA, AND, EOR, ADC or SBC of `v` and `m`, with the flags set.
    fn alu(&mut self, op: Op, v: u8, m: u8) -> u8 {
        match op {
            Op::Adc => self.add(v, m),
            Op::Sbc => self.subtract(v, m),
            _ => {
                let r = match op {
                    Op::Ora => v | m,
                    Op::And => v & m,
                    _ => v ^ m,
                };
                self.set_nz(r);
                r
            }
        }
    }

    fn add(&mut self, v: u8, m: u8) -> u8 {
        let carry = u16::from(self.p & C);
        if self.p & D == 0 {
            return self.add_binary(v, m);
        }
        // The decimal add leaves V alone.
        let mut lo = u16::from(v & 0x0F) + u16::from(m & 0x0F) + carry;
        let mut hi = u16::from(v >> 4) + u16::from(m >> 4);
        if lo > 9 {
            lo += 6;
        }
        if lo > 0x0F {
            hi += 1;
        }
        if hi > 9 {
            hi += 6;
        }
        self.set_flag(C, hi > 0x0F);
        let r = ((hi & 0x0F) << 4 | lo & 0x0F) as u8;
        self.set_nz(r);
        r
    }

    fn subtract(&mut self, v: u8, m: u8) -> u8 {
        if self.p & D == 0 {
            // The adder takes the complement of M, as the manual describes.
            return self.add_binary(v, !m);
        }
        // The decimal subtract leaves V alone.
        let borrow = i16::from(self.p & C == 0);
        let mut lo = i16::from(v & 0x0F) - i16::from(m & 0x0F) - borrow;
        let mut hi = i16::from(v >> 4) - i16::from(m >> 4);
        if lo < 0 {
            lo -= 6;
            hi -= 1;
        }
        self.set_flag(C, hi >= 0);
        if hi < 0 {
            hi -= 6;
        }
        let r = ((hi & 0x0F) << 4 | lo & 0x0F) as u8;
        self.set_nz(r);
        r
    }

    fn add_binary(&mut self, v: u8, m: u8) -> u8 {
        let sum = u16::from(v) + u16::from(m) + u16::from(self.p & C);
        let r = sum as u8;
        self.set_flag(C, sum > 0xFF);
        self.set_flag(V, (v ^ r) & (m ^ r) & 0x80 != 0);
        self.set_nz(r);
        r
    }

    /// The part of a read-modify-write instruction that changes the byte.
    fn modify(&mut self, op: Op, v: u8) -> u8 {
        let carry = self.p & C;
        let r = match op {
            Op::Asl => {
                self.set_flag(C, v & 0x80 != 0);
                v << 1
            }
            Op::Lsr => {
                self.set_flag(C, v & 1 != 0);
                v >> 1
            }
            Op::Rol => {
                self.set_flag(C, v & 0x80 != 0);
                v << 1 | carry
            }
            Op::Ror => {
                self.set_flag(C, v & 1 != 0);
                v >> 1 | carry << 7
            }
            Op::Inc => v.wrapping_add(1),
            _ => v.wrapping_sub(1),
        };
        self.set_nz(r);
        r
    }

    /// The flags of BIT, TST, TSB and TRB: N and V from bits 7 and 6 of
    /// memory, and Z from the AND.
    fn test(&mut self, and: u8, m: u8) {
        self.p = self.p & !(N | V | Z) | m & (N | V) | if and == 0 { Z } else { 0 };
    }

    fn branch(&mut self, offset: u8) {
        self.pc = self.pc.wrapping_add(offset as i8 as u16);
    }

    fn set_nz(&mut self, v: u8) {
        self.p = self.p & !(N | Z) | v & N | if v == 0 { Z } else { 0 };
    }

    fn set_flag(&mut self, flag: u8, on: bool) {
        if on {
            self.p |= flag;
        } else {
            self.p &= !flag;
        }
    }

    fn read(&self, bus: &mut impl Bus, logical: u16) -> u8 {
        bus.read(self.physical(logical))
    }

    fn write(&self, bus: &mut impl Bus, logical: u16, value: u8) {
        bus.write(self.physical(logical), value);
    }

    fn read_word(&self, bus: &mut impl Bus, logical: u16) -> u16 {
        let lo = u16::from(self.read(bus, logical));
        let hi = u16::from(self.read(bus, logical.wrapping_add(1)));
        hi << 8 | lo
    }

    /// A pointer in the zero page. Its high byte wraps within the page.
    fn zero_page_word(&self, bus: &mut impl Bus, offset: u8) -> u16 {
        let lo = u16::from(self.read(bus, zero_page(offset)));
        let hi = u16::from(self.read(bus, zero_page(offset.wrapping_add(1))));
        hi << 8 | lo
    }

    fn fetch(&mut self, bus: &mut impl Bus) -> u8 {
        let pc = self.pc;
        self.pc = pc.wrapping_add(1);
        self.read(bus, pc)
    }

    fn fetch_word(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = u16::from(self.fetch(bus));
        let hi = u16::from(self.fetch(bus));
        hi << 8 | lo
    }

    fn push(&mut self, bus: &mut impl Bus, v: u8) {
        self.write(bus, stack(self.s), v);
        self.s = self.s.wrapping_sub(1);
    }

    fn push_word(&mut self, bus: &mut impl Bus, v: u16) {
        self.push(bus, (v >> 8) as u8);
        self.push(bus, v as u8);
    }

    fn pull(&mut self, bus: &mut impl Bus) -> u8 {
        self.s = self.s.wrapping_add(1);
        self.read(bus, stack(self.s))
    }
}

impl Default for Cpu {
    fn default() -> Cpu {
        Cpu::new()
    }
}
