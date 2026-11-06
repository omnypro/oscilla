//! The HuC6280: results, flags, register effects and cycle counts of each
//! group of instructions. Each test follows the descriptions in the hardware
//! and software manuals.

use oscilla_hes::cpu::{B, Bus, C, Cpu, D, I, N, T, V, Z};

/// The 8 KB of RAM: block 0xF8, which MPR1 maps to the zero page and stack.
const RAM: u32 = 0x1F_0000;
/// Where each program starts: logical 0xE000, which MPR7 = 0x00 maps to
/// physical 0.
const START: u16 = 0xE000;
/// MPR0 to MPR7 for the tests: the I/O page, the RAM, then ROM blocks.
const MPR: [u8; 8] = [0xFF, 0xF8, 0x01, 0x02, 0x03, 0x04, 0x05, 0x00];

/// RAM at 0x1F0000, 64 KB of ROM from physical 0, and a log of every other
/// write.
struct TestBus {
    ram: Vec<u8>,
    rom: Vec<u8>,
    writes: Vec<(u32, u8)>,
    irq: Option<u16>,
    /// An interrupt that the next logged write raises.
    irq_on_write: Option<u16>,
}

impl Bus for TestBus {
    fn read(&mut self, physical: u32) -> u8 {
        if (RAM..RAM + 0x2000).contains(&physical) {
            self.ram[(physical - RAM) as usize]
        } else {
            self.rom.get(physical as usize).copied().unwrap_or(0xFF)
        }
    }

    fn write(&mut self, physical: u32, value: u8) {
        if (RAM..RAM + 0x2000).contains(&physical) {
            self.ram[(physical - RAM) as usize] = value;
        } else {
            self.writes.push((physical, value));
            if let Some(v) = self.irq_on_write.take() {
                self.irq = Some(v);
            }
        }
    }

    fn pending_interrupt(&mut self) -> Option<u16> {
        self.irq
    }
}

/// A processor at `START` with `code` there, every flag clear and S at 0xFF.
fn machine(code: &[u8]) -> (Cpu, TestBus) {
    let mut cpu = Cpu::new();
    cpu.mpr = MPR;
    cpu.pc = START;
    cpu.s = 0xFF;
    cpu.p = 0;
    let mut bus = TestBus {
        ram: vec![0; 0x2000],
        rom: vec![0; 0x10000],
        writes: Vec::new(),
        irq: None,
        irq_on_write: None,
    };
    put(&cpu, &mut bus, START, code);
    (cpu, bus)
}

/// Stores bytes at a logical address under the processor's mapping.
fn put(cpu: &Cpu, bus: &mut TestBus, logical: u16, bytes: &[u8]) {
    for (i, &b) in bytes.iter().enumerate() {
        let p = cpu.physical(logical.wrapping_add(i as u16));
        if (RAM..RAM + 0x2000).contains(&p) {
            bus.ram[(p - RAM) as usize] = b;
        } else {
            bus.rom[p as usize] = b;
        }
    }
}

/// The cycles of each step.
fn run(cpu: &mut Cpu, bus: &mut TestBus, n: usize) -> Vec<u32> {
    (0..n).map(|_| cpu.step(bus)).collect()
}

fn zp(bus: &TestBus, offset: u8) -> u8 {
    bus.ram[usize::from(offset)]
}

fn stack(bus: &TestBus, offset: u8) -> u8 {
    bus.ram[0x100 + usize::from(offset)]
}

/// Each documented opcode with its bytes and cycles, from the instruction
/// pages of the software manual in the order that they list them. CSL and CSH
/// come from the hardware manual's instruction map. No other page lists them.
/// A block transfer's count is the 17 before its bytes.
fn documented() -> Vec<(&'static str, u8, u16, u32)> {
    let mut v = vec![
        ("ADC", 0x69, 2, 2),
        ("ADC", 0x65, 2, 4),
        ("ADC", 0x75, 2, 4),
        ("ADC", 0x72, 2, 7),
        ("ADC", 0x61, 2, 7),
        ("ADC", 0x71, 2, 7),
        ("ADC", 0x6D, 3, 5),
        ("ADC", 0x7D, 3, 5),
        ("ADC", 0x79, 3, 5),
        ("AND", 0x29, 2, 2),
        ("AND", 0x25, 2, 4),
        ("AND", 0x35, 2, 4),
        ("AND", 0x32, 2, 7),
        ("AND", 0x21, 2, 7),
        ("AND", 0x31, 2, 7),
        ("AND", 0x2D, 3, 5),
        ("AND", 0x3D, 3, 5),
        ("AND", 0x39, 3, 5),
        ("ASL", 0x06, 2, 6),
        ("ASL", 0x16, 2, 6),
        ("ASL", 0x0E, 3, 7),
        ("ASL", 0x1E, 3, 7),
        ("ASL", 0x0A, 1, 2),
        ("BCC", 0x90, 2, 2),
        ("BCS", 0xB0, 2, 2),
        ("BEQ", 0xF0, 2, 2),
        ("BIT", 0x89, 2, 2),
        ("BIT", 0x24, 2, 4),
        ("BIT", 0x34, 2, 4),
        ("BIT", 0x2C, 3, 5),
        ("BIT", 0x3C, 3, 5),
        ("BMI", 0x30, 2, 2),
        ("BNE", 0xD0, 2, 2),
        ("BPL", 0x10, 2, 2),
        ("BRA", 0x80, 2, 4),
        ("BRK", 0x00, 1, 8),
        ("BSR", 0x44, 2, 8),
        ("BVC", 0x50, 2, 2),
        ("BVS", 0x70, 2, 2),
        ("CLA", 0x62, 1, 2),
        ("CLC", 0x18, 1, 2),
        ("CLD", 0xD8, 1, 2),
        ("CLI", 0x58, 1, 2),
        ("CLV", 0xB8, 1, 2),
        ("CLX", 0x82, 1, 2),
        ("CLY", 0xC2, 1, 2),
        ("CMP", 0xC9, 2, 2),
        ("CMP", 0xC5, 2, 4),
        ("CMP", 0xD5, 2, 4),
        ("CMP", 0xD2, 2, 7),
        ("CMP", 0xC1, 2, 7),
        ("CMP", 0xD1, 2, 7),
        ("CMP", 0xCD, 3, 5),
        ("CMP", 0xDD, 3, 5),
        ("CMP", 0xD9, 3, 5),
        ("CPX", 0xE0, 2, 2),
        ("CPX", 0xE4, 2, 4),
        ("CPX", 0xEC, 3, 5),
        ("CPY", 0xC0, 2, 2),
        ("CPY", 0xC4, 2, 4),
        ("CPY", 0xCC, 3, 5),
        ("CSH", 0xD4, 1, 3),
        ("CSL", 0x54, 1, 3),
        ("DEC", 0xC6, 2, 6),
        ("DEC", 0xD6, 2, 6),
        ("DEC", 0xCE, 3, 7),
        ("DEC", 0xDE, 3, 7),
        ("DEC", 0x3A, 1, 2),
        ("DEX", 0xCA, 1, 2),
        ("DEY", 0x88, 1, 2),
        ("EOR", 0x49, 2, 2),
        ("EOR", 0x45, 2, 4),
        ("EOR", 0x55, 2, 4),
        ("EOR", 0x52, 2, 7),
        ("EOR", 0x41, 2, 7),
        ("EOR", 0x51, 2, 7),
        ("EOR", 0x4D, 3, 5),
        ("EOR", 0x5D, 3, 5),
        ("EOR", 0x59, 3, 5),
        ("INC", 0xE6, 2, 6),
        ("INC", 0xF6, 2, 6),
        ("INC", 0xEE, 3, 7),
        ("INC", 0xFE, 3, 7),
        ("INC", 0x1A, 1, 2),
        ("INX", 0xE8, 1, 2),
        ("INY", 0xC8, 1, 2),
        ("JMP", 0x4C, 3, 4),
        ("JMP", 0x6C, 3, 7),
        ("JMP", 0x7C, 3, 7),
        ("JSR", 0x20, 3, 7),
        ("LDA", 0xA9, 2, 2),
        ("LDA", 0xA5, 2, 4),
        ("LDA", 0xB5, 2, 4),
        ("LDA", 0xB2, 2, 7),
        ("LDA", 0xA1, 2, 7),
        ("LDA", 0xB1, 2, 7),
        ("LDA", 0xAD, 3, 5),
        ("LDA", 0xBD, 3, 5),
        ("LDA", 0xB9, 3, 5),
        ("LDX", 0xA2, 2, 2),
        ("LDX", 0xA6, 2, 4),
        ("LDX", 0xB6, 2, 4),
        ("LDX", 0xAE, 3, 5),
        ("LDX", 0xBE, 3, 5),
        ("LDY", 0xA0, 2, 2),
        ("LDY", 0xA4, 2, 4),
        ("LDY", 0xB4, 2, 4),
        ("LDY", 0xAC, 3, 5),
        ("LDY", 0xBC, 3, 5),
        ("LSR", 0x46, 2, 6),
        ("LSR", 0x56, 2, 6),
        ("LSR", 0x4E, 3, 7),
        ("LSR", 0x5E, 3, 7),
        ("LSR", 0x4A, 1, 2),
        ("NOP", 0xEA, 1, 2),
        ("ORA", 0x09, 2, 2),
        ("ORA", 0x05, 2, 4),
        ("ORA", 0x15, 2, 4),
        ("ORA", 0x12, 2, 7),
        ("ORA", 0x01, 2, 7),
        ("ORA", 0x11, 2, 7),
        ("ORA", 0x0D, 3, 5),
        ("ORA", 0x1D, 3, 5),
        ("ORA", 0x19, 3, 5),
        ("PHA", 0x48, 1, 3),
        ("PHP", 0x08, 1, 3),
        ("PHX", 0xDA, 1, 3),
        ("PHY", 0x5A, 1, 3),
        ("PLA", 0x68, 1, 4),
        ("PLP", 0x28, 1, 4),
        ("PLX", 0xFA, 1, 4),
        ("PLY", 0x7A, 1, 4),
        ("ROL", 0x26, 2, 6),
        ("ROL", 0x36, 2, 6),
        ("ROL", 0x2E, 3, 7),
        ("ROL", 0x3E, 3, 7),
        ("ROL", 0x2A, 1, 2),
        ("ROR", 0x66, 2, 6),
        ("ROR", 0x76, 2, 6),
        ("ROR", 0x6E, 3, 7),
        ("ROR", 0x7E, 3, 7),
        ("ROR", 0x6A, 1, 2),
        ("RTI", 0x40, 1, 7),
        ("RTS", 0x60, 1, 7),
        ("SAX", 0x22, 1, 3),
        ("SAY", 0x42, 1, 3),
        ("SBC", 0xE9, 2, 2),
        ("SBC", 0xE5, 2, 4),
        ("SBC", 0xF5, 2, 4),
        ("SBC", 0xF2, 2, 7),
        ("SBC", 0xE1, 2, 7),
        ("SBC", 0xF1, 2, 7),
        ("SBC", 0xED, 3, 5),
        ("SBC", 0xFD, 3, 5),
        ("SBC", 0xF9, 3, 5),
        ("SEC", 0x38, 1, 2),
        ("SED", 0xF8, 1, 2),
        ("SEI", 0x78, 1, 2),
        ("SET", 0xF4, 1, 2),
        ("ST0", 0x03, 2, 4),
        ("ST1", 0x13, 2, 4),
        ("ST2", 0x23, 2, 4),
        ("STA", 0x85, 2, 4),
        ("STA", 0x95, 2, 4),
        ("STA", 0x92, 2, 7),
        ("STA", 0x81, 2, 7),
        ("STA", 0x91, 2, 7),
        ("STA", 0x8D, 3, 5),
        ("STA", 0x9D, 3, 5),
        ("STA", 0x99, 3, 5),
        ("STX", 0x86, 2, 4),
        ("STX", 0x96, 2, 4),
        ("STX", 0x8E, 3, 5),
        ("STY", 0x84, 2, 4),
        ("STY", 0x94, 2, 4),
        ("STY", 0x8C, 3, 5),
        ("STZ", 0x64, 2, 4),
        ("STZ", 0x74, 2, 4),
        ("STZ", 0x9C, 3, 5),
        ("STZ", 0x9E, 3, 5),
        ("SXY", 0x02, 1, 3),
        ("TAI", 0xF3, 7, 17),
        ("TAM", 0x53, 2, 5),
        ("TAX", 0xAA, 1, 2),
        ("TAY", 0xA8, 1, 2),
        ("TDD", 0xC3, 7, 17),
        ("TIA", 0xE3, 7, 17),
        ("TII", 0x73, 7, 17),
        ("TIN", 0xD3, 7, 17),
        ("TMA", 0x43, 2, 4),
        ("TRB", 0x14, 2, 6),
        ("TRB", 0x1C, 3, 7),
        ("TSB", 0x04, 2, 6),
        ("TSB", 0x0C, 3, 7),
        ("TST", 0x83, 3, 7),
        ("TST", 0xA3, 3, 7),
        ("TST", 0x93, 4, 8),
        ("TST", 0xB3, 4, 8),
        ("TSX", 0xBA, 1, 2),
        ("TXA", 0x8A, 1, 2),
        ("TXS", 0x9A, 1, 2),
        ("TYA", 0x98, 1, 2),
    ];
    for i in 0..8 {
        v.push(("BBR", 0x0F + 0x10 * i, 3, 6));
        v.push(("BBS", 0x8F + 0x10 * i, 3, 6));
        v.push(("RMB", 0x07 + 0x10 * i, 2, 7));
        v.push(("SMB", 0x87 + 0x10 * i, 2, 7));
    }
    v
}

/// Opcodes that put the program counter somewhere other than after their
/// operands.
fn jumps(opcode: u8) -> bool {
    matches!(
        opcode,
        0x00 | 0x10
            | 0x20
            | 0x30
            | 0x40
            | 0x44
            | 0x4C
            | 0x50
            | 0x60
            | 0x6C
            | 0x70
            | 0x7C
            | 0x80
            | 0x90
            | 0xB0
            | 0xD0
            | 0xF0
    ) || opcode & 0x0F == 0x0F
}

#[test]
fn every_documented_opcode_takes_its_documented_bytes_and_cycles() {
    let table = documented();
    // The hardware manual counts 234 instructions.
    assert_eq!(table.len(), 234);
    let mut seen = [false; 256];
    for &(name, opcode, bytes, base) in &table {
        assert!(
            !seen[usize::from(opcode)],
            "{name} {opcode:02X} listed twice"
        );
        seen[usize::from(opcode)] = true;
        // A source at 0x2010, a destination at 0x2030 and a length of 1 for
        // a block transfer; 0x2010 or 0x10 as an address for the others.
        let (mut cpu, mut bus) = machine(&[opcode, 0x10, 0x20, 0x30, 0x20, 0x01, 0x00]);
        // With every flag clear, BPL, BVC, BCC and BNE branch. BBR branches
        // on the clear byte at 0x2010, and a transfer moves one byte.
        let extra = match opcode {
            0x10 | 0x50 | 0x90 | 0xD0 => 2,
            _ if opcode & 0x8F == 0x0F => 2,
            0x73 | 0xC3 | 0xD3 | 0xE3 | 0xF3 => 6,
            _ => 0,
        };
        assert_eq!(cpu.step(&mut bus), base + extra, "{name} {opcode:02X}");
        if !jumps(opcode) {
            assert_eq!(cpu.pc, START + bytes, "{name} {opcode:02X}");
        }
    }
    let blank: Vec<u8> = (0..=255u8).filter(|&o| !seen[usize::from(o)]).collect();
    assert_eq!(
        blank,
        [
            0x0B, 0x1B, 0x2B, 0x33, 0x3B, 0x4B, 0x5B, 0x5C, 0x63, 0x6B, 0x7B, 0x8B, 0x9B, 0xAB,
            0xBB, 0xCB, 0xDB, 0xDC, 0xE2, 0xEB, 0xFB, 0xFC
        ]
    );
    for opcode in blank {
        let (mut cpu, mut bus) = machine(&[opcode, 0x10, 0x20]);
        assert_eq!(cpu.step(&mut bus), 2, "{opcode:02X}");
        assert_eq!(cpu.pc, START + 1, "{opcode:02X}");
    }
}

#[test]
fn every_opcode_runs_from_any_state_without_panicking() {
    for opcode in 0..=255u8 {
        for fill in [0x00, 0xFF] {
            let (mut cpu, mut bus) = machine(&[opcode, fill, fill, fill, fill, fill, fill]);
            cpu.a = fill;
            cpu.x = fill;
            cpu.y = fill;
            cpu.s = fill;
            cpu.p = fill & !I;
            bus.ram.fill(fill);
            let cycles = cpu.step(&mut bus);
            assert!(cycles >= 2, "{opcode:02X}");
        }
    }
}

#[test]
fn adc_adds_the_carry_and_sets_overflow() {
    // 0x50 + 0x50 overflows into the sign bit.
    let (mut cpu, mut bus) = machine(&[0x18, 0xA9, 0x50, 0x69, 0x50]);
    assert_eq!(run(&mut cpu, &mut bus, 3), [2, 2, 2]);
    assert_eq!(cpu.a, 0xA0);
    assert_eq!(cpu.p & (N | V | Z | C), N | V);
    // 0xFF + 0x00 + 1 carries out and leaves zero.
    let (mut cpu, mut bus) = machine(&[0x38, 0xA9, 0xFF, 0x69, 0x00]);
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.a, 0);
    assert_eq!(cpu.p & (N | V | Z | C), Z | C);
}

#[test]
fn sbc_subtracts_the_inverted_carry() {
    // With C set nothing more is taken: 0x50 - 0xB0 overflows and borrows.
    let (mut cpu, mut bus) = machine(&[0x38, 0xA9, 0x50, 0xE9, 0xB0]);
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.a, 0xA0);
    assert_eq!(cpu.p & (N | V | Z | C), N | V);
    // With C clear one more is taken, and no borrow leaves C set.
    let (mut cpu, mut bus) = machine(&[0x18, 0xA9, 0x05, 0xE9, 0x03]);
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.a, 0x01);
    assert_eq!(cpu.p & (N | V | Z | C), C);
}

#[test]
fn decimal_adc_and_sbc_take_one_more_cycle_and_leave_v() {
    for v in [0, V] {
        let (mut cpu, mut bus) = machine(&[0x69, 0x46]);
        cpu.p = D | C | v;
        cpu.a = 0x58;
        assert_eq!(cpu.step(&mut bus), 3);
        // 58 + 46 + 1 = 105.
        assert_eq!(cpu.a, 0x05);
        assert_eq!(cpu.p & (N | V | Z | C), C | v);
    }
    let (mut cpu, mut bus) = machine(&[0x69, 0x01]);
    cpu.p = D;
    cpu.a = 0x99;
    assert_eq!(cpu.step(&mut bus), 3);
    assert_eq!(cpu.a, 0x00);
    assert_eq!(cpu.p & (Z | C), Z | C);
    for v in [0, V] {
        let (mut cpu, mut bus) = machine(&[0xE9, 0x12]);
        cpu.p = D | C | v;
        cpu.a = 0x46;
        assert_eq!(cpu.step(&mut bus), 3);
        assert_eq!(cpu.a, 0x34);
        assert_eq!(cpu.p & (N | V | Z | C), C | v);
    }
    // 12 - 21 borrows: 91 and C clear.
    let (mut cpu, mut bus) = machine(&[0xE9, 0x21]);
    cpu.p = D | C;
    cpu.a = 0x12;
    assert_eq!(cpu.step(&mut bus), 3);
    assert_eq!(cpu.a, 0x91);
    assert_eq!(cpu.p & C, 0);
}

#[test]
fn compares_set_carry_when_the_register_is_not_below() {
    let (mut cpu, mut bus) = machine(&[0xC9, 0x10, 0xE0, 0x21, 0xC0, 0x01]);
    cpu.a = 0x10;
    cpu.x = 0x20;
    cpu.y = 0x01;
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.p & (N | Z | C), Z | C);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.p & (N | Z | C), N);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.p & (N | Z | C), Z | C);
}

#[test]
fn the_t_flag_turns_alu_ops_into_operations_on_the_zero_page_byte_at_x() {
    // SET, then ORA, AND, EOR or ADC with an immediate operand: M(X) takes
    // the place of A, A stays as it was, and the instruction takes 3 more
    // cycles.
    for (opcode, operand, result, flags) in [
        (0x09, 0x03, 0x0F, 0),
        (0x29, 0x0A, 0x08, 0),
        (0x49, 0xFF, 0xF3, N),
        (0x69, 0x05, 0x12, 0),
        (0x29, 0x30, 0x00, Z),
    ] {
        let (mut cpu, mut bus) = machine(&[0xF4, opcode, operand]);
        cpu.a = 0x5A;
        cpu.x = 0x10;
        cpu.p = C;
        bus.ram[0x10] = 0x0C;
        assert_eq!(cpu.step(&mut bus), 2);
        assert_eq!(cpu.p & T, T);
        assert_eq!(cpu.step(&mut bus), 5, "{opcode:02X}");
        assert_eq!(zp(&bus, 0x10), result, "{opcode:02X}");
        assert_eq!(cpu.a, 0x5A);
        assert_eq!(cpu.p & (N | Z | T), flags, "{opcode:02X}");
    }
    // ADC from the zero page under T: 0x7F + 0x01 overflows in M(X).
    let (mut cpu, mut bus) = machine(&[0xF4, 0x65, 0x20]);
    cpu.x = 0x10;
    bus.ram[0x10] = 0x7F;
    bus.ram[0x20] = 0x01;
    assert_eq!(run(&mut cpu, &mut bus, 2), [2, 7]);
    assert_eq!(zp(&bus, 0x10), 0x80);
    assert_eq!(cpu.p & (N | V | C), N | V);
    // A decimal ADC under T takes both extras.
    let (mut cpu, mut bus) = machine(&[0xF4, 0x69, 0x01]);
    cpu.p = D;
    cpu.x = 0x10;
    bus.ram[0x10] = 0x09;
    assert_eq!(run(&mut cpu, &mut bus, 2), [2, 6]);
    assert_eq!(zp(&bus, 0x10), 0x10);
}

#[test]
fn set_has_no_effect_on_other_instructions_and_lasts_one_instruction() {
    // LDA and SBC after SET work on A.
    let (mut cpu, mut bus) = machine(&[0xF4, 0xA9, 0x33, 0xF4, 0xE9, 0x03]);
    cpu.x = 0x10;
    cpu.p = C;
    bus.ram[0x10] = 0x77;
    assert_eq!(run(&mut cpu, &mut bus, 4), [2, 2, 2, 2]);
    assert_eq!(cpu.a, 0x30);
    assert_eq!(zp(&bus, 0x10), 0x77);
    assert_eq!(cpu.p & T, 0);
    // PHP after SET pushes T as 0 and B as 1.
    let (mut cpu, mut bus) = machine(&[0xF4, 0x08]);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(stack(&bus, 0xFF), B);
}

#[test]
fn tii_copies_upward_and_gives_back_a_x_and_y() {
    let (mut cpu, mut bus) = machine(&[0x73, 0x00, 0x40, 0x80, 0x20, 0x05, 0x00]);
    put(&cpu, &mut bus, 0x4000, &[1, 2, 3, 4, 5]);
    cpu.a = 0xAA;
    cpu.x = 0xBB;
    cpu.y = 0xCC;
    cpu.s = 0xF0;
    assert_eq!(cpu.step(&mut bus), 17 + 6 * 5);
    assert_eq!(&bus.ram[0x80..0x86], [1, 2, 3, 4, 5, 0]);
    assert_eq!((cpu.a, cpu.x, cpu.y, cpu.s), (0xAA, 0xBB, 0xCC, 0xF0));
    assert_eq!(cpu.pc, START + 7);
    // Y, A and X wait in three bytes of the stack.
    assert_eq!(
        (stack(&bus, 0xF0), stack(&bus, 0xEF), stack(&bus, 0xEE)),
        (0xCC, 0xAA, 0xBB)
    );
}

#[test]
fn tdd_copies_downward() {
    let (mut cpu, mut bus) = machine(&[0xC3, 0x04, 0x40, 0x84, 0x20, 0x05, 0x00]);
    put(&cpu, &mut bus, 0x4000, &[1, 2, 3, 4, 5]);
    assert_eq!(cpu.step(&mut bus), 17 + 6 * 5);
    assert_eq!(&bus.ram[0x7F..0x86], [0, 1, 2, 3, 4, 5, 0]);
}

#[test]
fn tin_writes_every_byte_to_one_address() {
    // Logical 0x0002 under MPR0 = 0xFF is physical 0x1FE002, in the range of
    // the HuC6270.
    let (mut cpu, mut bus) = machine(&[0xD3, 0x00, 0x40, 0x02, 0x00, 0x04, 0x00]);
    put(&cpu, &mut bus, 0x4000, &[1, 2, 3, 4]);
    assert_eq!(cpu.step(&mut bus), 17 + 6 * 4);
    assert_eq!(
        bus.writes,
        [(0x1FE002, 1), (0x1FE002, 2), (0x1FE002, 3), (0x1FE002, 4)]
    );
}

#[test]
fn tia_alternates_the_destination_starting_upward() {
    let (mut cpu, mut bus) = machine(&[0xE3, 0x00, 0x40, 0x02, 0x00, 0x05, 0x00]);
    put(&cpu, &mut bus, 0x4000, &[1, 2, 3, 4, 5]);
    assert_eq!(cpu.step(&mut bus), 17 + 6 * 5);
    assert_eq!(
        bus.writes,
        [
            (0x1FE002, 1),
            (0x1FE003, 2),
            (0x1FE002, 3),
            (0x1FE003, 4),
            (0x1FE002, 5)
        ]
    );
}

#[test]
fn tai_alternates_the_source_starting_upward() {
    let (mut cpu, mut bus) = machine(&[0xF3, 0x00, 0x40, 0x80, 0x20, 0x05, 0x00]);
    put(&cpu, &mut bus, 0x4000, &[1, 2, 3]);
    assert_eq!(cpu.step(&mut bus), 17 + 6 * 5);
    assert_eq!(&bus.ram[0x80..0x86], [1, 2, 1, 2, 1, 0]);
}

#[test]
fn a_length_of_zero_moves_65536_bytes() {
    let (mut cpu, mut bus) = machine(&[0xD3, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(cpu.step(&mut bus), 17 + 6 * 65536);
    assert_eq!(bus.writes.len(), 65536);
    assert!(bus.writes.iter().all(|&(p, _)| p == 0x1FE000));
}

#[test]
fn an_interrupt_waits_for_the_end_of_a_block_transfer() {
    let (mut cpu, mut bus) = machine(&[0xD3, 0x00, 0x40, 0x02, 0x00, 0x03, 0x00]);
    put(&cpu, &mut bus, 0xFFF8, &[0x00, 0xE2]);
    bus.irq_on_write = Some(0xFFF8);
    assert_eq!(cpu.step(&mut bus), 17 + 6 * 3);
    assert_eq!(bus.writes.len(), 3);
    assert_eq!(cpu.pc, START + 7);
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(cpu.pc, 0xE200);
    let ret = u16::from(stack(&bus, 0xFF)) << 8 | u16::from(stack(&bus, 0xFE));
    assert_eq!(ret, START + 7);
}

#[test]
fn tam_and_tma_select_registers_by_bit() {
    let (mut cpu, mut bus) = machine(&[
        0x53, 0x04, // TAM2
        0x43, 0x08, // TMA3
        0x53, 0x30, // TAM4 and TAM5
        0x43, 0x30, // TMA with two bits
        0x43, 0x00, // TMA with none
        0xAD, 0x00, 0x40, // LDA 0x4000
    ]);
    cpu.a = 0x07;
    bus.rom[0x7 << 13] = 0x99;
    assert_eq!(cpu.step(&mut bus), 5);
    assert_eq!(cpu.mpr, [0xFF, 0xF8, 0x07, 0x02, 0x03, 0x04, 0x05, 0x00]);
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.a, 0x02);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.mpr, [0xFF, 0xF8, 0x07, 0x02, 0x02, 0x02, 0x05, 0x00]);
    cpu.mpr[5] = 0x06;
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.a, 0x02);
    cpu.a = 0x55;
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.a, 0x55);
    // Logical 0x4000 now reads physical block 7.
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.a, 0x99);
}

#[test]
fn tst_ands_its_immediate_with_memory_in_all_four_modes() {
    let (mut cpu, mut bus) = machine(&[
        0x83, 0xC0, 0x10, // TST #$C0, $10
        0xA3, 0x01, 0x0F, // TST #$01, $0F,X
        0x93, 0x40, 0x11, 0x20, // TST #$40, $2011
        0xB3, 0x80, 0x10, 0x20, // TST #$80, $2010,X
    ]);
    cpu.a = 0xFF;
    cpu.x = 0x01;
    bus.ram[0x10] = 0xC0;
    bus.ram[0x11] = 0x40;
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.p & (N | V | Z), N | V);
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.p & (N | V | Z), N | V | Z);
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(cpu.p & (N | V | Z), V);
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(cpu.p & (N | V | Z), V | Z);
    assert_eq!(cpu.a, 0xFF);
    assert_eq!(cpu.pc, START + 14);
}

#[test]
fn bit_takes_n_and_v_from_memory_in_every_mode() {
    let (mut cpu, mut bus) = machine(&[0x89, 0xC0, 0x24, 0x10]);
    cpu.a = 0x00;
    bus.ram[0x10] = 0x01;
    assert_eq!(cpu.step(&mut bus), 2);
    assert_eq!(cpu.p & (N | V | Z), N | V | Z);
    cpu.a = 0xFF;
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.p & (N | V | Z), 0);
}

#[test]
fn swaps_exchange_registers_and_leave_the_flags() {
    let (mut cpu, mut bus) = machine(&[0x02, 0x22, 0x42]);
    cpu.a = 0;
    cpu.x = 2;
    cpu.y = 0x80;
    assert_eq!(cpu.step(&mut bus), 3);
    assert_eq!((cpu.a, cpu.x, cpu.y), (0, 0x80, 2));
    assert_eq!(cpu.step(&mut bus), 3);
    assert_eq!((cpu.a, cpu.x, cpu.y), (0x80, 0, 2));
    assert_eq!(cpu.step(&mut bus), 3);
    assert_eq!((cpu.a, cpu.x, cpu.y), (2, 0, 0x80));
    assert_eq!(cpu.p, 0);
}

#[test]
fn clears_and_stz_leave_the_flags() {
    let (mut cpu, mut bus) = machine(&[0x62, 0x82, 0xC2, 0x64, 0x10, 0x9C, 0x11, 0x20]);
    cpu.a = 0x80;
    cpu.x = 0x80;
    cpu.y = 0x80;
    cpu.p = N;
    bus.ram[0x10] = 0x55;
    bus.ram[0x11] = 0x55;
    assert_eq!(run(&mut cpu, &mut bus, 5), [2, 2, 2, 4, 5]);
    assert_eq!((cpu.a, cpu.x, cpu.y), (0, 0, 0));
    assert_eq!((zp(&bus, 0x10), zp(&bus, 0x11)), (0, 0));
    assert_eq!(cpu.p, N);
}

#[test]
fn bsr_pushes_the_address_of_its_last_byte() {
    // BSR to START + 6, where RTS returns to START + 2.
    let (mut cpu, mut bus) = machine(&[0x44, 0x04, 0xEA, 0xEA, 0xEA, 0xEA, 0x60]);
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(cpu.pc, START + 6);
    assert_eq!(cpu.s, 0xFD);
    let pushed = u16::from(stack(&bus, 0xFF)) << 8 | u16::from(stack(&bus, 0xFE));
    assert_eq!(pushed, START + 1);
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.pc, START + 2);
    assert_eq!(cpu.s, 0xFF);
}

#[test]
fn jsr_and_rts() {
    let (mut cpu, mut bus) = machine(&[0x20, 0x10, 0xE0]);
    put(&cpu, &mut bus, 0xE010, &[0x60]);
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.pc, 0xE010);
    let pushed = u16::from(stack(&bus, 0xFF)) << 8 | u16::from(stack(&bus, 0xFE));
    assert_eq!(pushed, START + 2);
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.pc, START + 3);
}

#[test]
fn brk_pushes_brk_plus_2_with_b_and_rti_restores() {
    let (mut cpu, mut bus) = machine(&[0x00]);
    put(&cpu, &mut bus, 0xFFF6, &[0x00, 0xE1]);
    put(&cpu, &mut bus, 0xE100, &[0x40]);
    cpu.p = D | C;
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(cpu.pc, 0xE100);
    assert_eq!(cpu.s, 0xFC);
    let pushed = u16::from(stack(&bus, 0xFF)) << 8 | u16::from(stack(&bus, 0xFE));
    assert_eq!(pushed, START + 2);
    assert_eq!(stack(&bus, 0xFD), D | C | B);
    assert_eq!(cpu.p, I | B | C);
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.pc, START + 2);
    assert_eq!(cpu.p, D | C | B);
    assert_eq!(cpu.s, 0xFF);
}

#[test]
fn jmp_through_a_pointer_does_not_wrap_its_page() {
    let (mut cpu, mut bus) = machine(&[0x4C, 0x20, 0xE0]);
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.pc, 0xE020);
    let (mut cpu, mut bus) = machine(&[0x6C, 0xFF, 0x20]);
    bus.ram[0xFF] = 0x78;
    bus.ram[0x100] = 0x56;
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.pc, 0x5678);
    let (mut cpu, mut bus) = machine(&[0x7C, 0x0E, 0x20]);
    cpu.x = 2;
    bus.ram[0x10] = 0x34;
    bus.ram[0x11] = 0x12;
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.pc, 0x1234);
}

#[test]
fn branches_take_two_more_cycles_when_taken() {
    let (mut cpu, mut bus) = machine(&[0xF0, 0x10, 0xD0, 0x02, 0xEA, 0xEA, 0x80, 0xFA]);
    assert_eq!(cpu.step(&mut bus), 2);
    assert_eq!(cpu.pc, START + 2);
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.pc, START + 6);
    // BRA back by 6.
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.pc, START + 2);
}

#[test]
fn bbr_and_bbs_test_one_bit_of_the_zero_page() {
    let (mut cpu, mut bus) = machine(&[0xAF, 0x10, 0x05]);
    bus.ram[0x10] = 0x04;
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(cpu.pc, START + 3 + 5);
    let (mut cpu, mut bus) = machine(&[0x2F, 0x10, 0x05]);
    bus.ram[0x10] = 0x04;
    assert_eq!(cpu.step(&mut bus), 6);
    assert_eq!(cpu.pc, START + 3);
    let (mut cpu, mut bus) = machine(&[0x0F, 0x10, 0xFB]);
    bus.ram[0x10] = 0x04;
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(cpu.pc, START + 3 - 5);
}

#[test]
fn rmb_and_smb_change_one_bit() {
    let (mut cpu, mut bus) = machine(&[0x37, 0x10, 0xF7, 0x11]);
    bus.ram[0x10] = 0xFF;
    assert_eq!(run(&mut cpu, &mut bus, 2), [7, 7]);
    assert_eq!((zp(&bus, 0x10), zp(&bus, 0x11)), (0xF7, 0x80));
    assert_eq!(cpu.p, 0);
}

#[test]
fn tsb_and_trb_set_and_clear_the_bits_of_a() {
    let (mut cpu, mut bus) = machine(&[0x04, 0x10, 0x14, 0x10, 0x0C, 0x11, 0x20, 0x1C, 0x11, 0x20]);
    cpu.a = 0x0F;
    bus.ram[0x10] = 0xC3;
    assert_eq!(cpu.step(&mut bus), 6);
    assert_eq!(zp(&bus, 0x10), 0xCF);
    assert_eq!(cpu.p & (N | V | Z), N | V);
    assert_eq!(cpu.step(&mut bus), 6);
    assert_eq!(zp(&bus, 0x10), 0xC0);
    assert_eq!(cpu.p & (N | V | Z), N | V);
    // Z comes from A AND M before the change.
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(zp(&bus, 0x11), 0x0F);
    assert_eq!(cpu.p & (N | V | Z), Z);
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(zp(&bus, 0x11), 0x00);
    assert_eq!(cpu.p & (N | V | Z), 0);
}

#[test]
fn the_zero_page_and_the_stack_follow_mpr1() {
    let (mut cpu, mut bus) = machine(&[0x85, 0x10, 0x48, 0x85, 0x10, 0x48]);
    cpu.a = 0x42;
    run(&mut cpu, &mut bus, 2);
    // Under MPR1 = 0xF8: physical 0x1F0010 and 0x1F01FF.
    assert_eq!((zp(&bus, 0x10), stack(&bus, 0xFF)), (0x42, 0x42));
    cpu.mpr[1] = 0x02;
    run(&mut cpu, &mut bus, 2);
    assert_eq!(bus.writes, [(0x4010, 0x42), (0x41FE, 0x42)]);
}

#[test]
fn the_indirect_modes_read_pointers_from_the_zero_page() {
    let (mut cpu, mut bus) = machine(&[0xB2, 0x10, 0xA1, 0x0E, 0xB1, 0x10, 0x92, 0xFF]);
    cpu.x = 2;
    cpu.y = 1;
    bus.ram[0x10] = 0x80;
    bus.ram[0x11] = 0x20;
    bus.ram[0x80] = 0x42;
    bus.ram[0x81] = 0x43;
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.a, 0x43);
    // A pointer at 0xFF takes its high byte from 0x00 of the zero page.
    bus.ram[0xFF] = 0x90;
    bus.ram[0x00] = 0x20;
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(zp(&bus, 0x90), 0x43);
}

#[test]
fn pushes_and_pulls() {
    let (mut cpu, mut bus) = machine(&[0xDA, 0x5A, 0x7A, 0xFA, 0x48, 0x68, 0x08, 0x28]);
    cpu.a = 0x00;
    cpu.x = 0x80;
    cpu.y = 0x01;
    assert_eq!(run(&mut cpu, &mut bus, 2), [3, 3]);
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.y, 0x01);
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.x, 0x80);
    assert_eq!(cpu.p & (N | Z), N);
    assert_eq!(run(&mut cpu, &mut bus, 2), [3, 4]);
    assert_eq!(cpu.p & (N | Z), Z);
    cpu.p = C | V;
    assert_eq!(cpu.step(&mut bus), 3);
    assert_eq!(stack(&bus, 0xFF), C | V | B);
    cpu.p = 0;
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.p, C | V | B);
    assert_eq!(cpu.s, 0xFF);
}

#[test]
fn shifts_and_steps_of_memory_and_a() {
    let (mut cpu, mut bus) = machine(&[0x1A, 0x3A, 0x3A, 0x06, 0x10, 0x66, 0x10, 0xEE, 0x10, 0x20]);
    cpu.a = 0xFF;
    bus.ram[0x10] = 0x81;
    assert_eq!(cpu.step(&mut bus), 2);
    assert_eq!((cpu.a, cpu.p & (N | Z)), (0x00, Z));
    assert_eq!(cpu.step(&mut bus), 2);
    assert_eq!((cpu.a, cpu.p & (N | Z)), (0xFF, N));
    assert_eq!(cpu.step(&mut bus), 2);
    assert_eq!(cpu.a, 0xFE);
    // ASL: 0x81 to 0x02 with C set; ROR: 0x02 to 0x81 with C clear.
    assert_eq!(cpu.step(&mut bus), 6);
    assert_eq!((zp(&bus, 0x10), cpu.p & C), (0x02, C));
    assert_eq!(cpu.step(&mut bus), 6);
    assert_eq!((zp(&bus, 0x10), cpu.p & (C | N)), (0x81, N));
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(zp(&bus, 0x10), 0x82);
}

fn vectors(cpu: &Cpu, bus: &mut TestBus) {
    put(cpu, bus, 0xFFF6, &[0x00, 0xE6]);
    put(cpu, bus, 0xFFF8, &[0x00, 0xE8]);
    put(cpu, bus, 0xFFFA, &[0x00, 0xEA]);
    put(cpu, bus, 0xFFFC, &[0x00, 0xEC]);
    put(cpu, bus, 0xFFFE, &[0x00, 0xEE]);
}

#[test]
fn an_interrupt_pushes_pc_and_p_and_takes_its_vector() {
    for (vector, target) in [(0xFFF6, 0xE600), (0xFFF8, 0xE800), (0xFFFA, 0xEA00)] {
        let (mut cpu, mut bus) = machine(&[0xEA]);
        vectors(&cpu, &mut bus);
        cpu.p = D | C;
        bus.irq = Some(vector);
        assert_eq!(cpu.step(&mut bus), 8);
        assert_eq!(cpu.pc, target);
        let pushed = u16::from(stack(&bus, 0xFF)) << 8 | u16::from(stack(&bus, 0xFE));
        assert_eq!(pushed, START);
        // B goes to the stack as 0; I is set and D cleared.
        assert_eq!(stack(&bus, 0xFD), D | C);
        assert_eq!(cpu.p, I | C);
        assert_eq!(cpu.s, 0xFC);
    }
}

#[test]
fn i_masks_the_maskable_interrupts_and_not_nmi() {
    let (mut cpu, mut bus) = machine(&[0xEA, 0xEA]);
    vectors(&cpu, &mut bus);
    put(&cpu, &mut bus, 0xEC00, &[0xEA]);
    cpu.p = I;
    bus.irq = Some(0xFFF8);
    assert_eq!(cpu.step(&mut bus), 2);
    assert_eq!(cpu.pc, START + 1);
    cpu.nmi();
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(cpu.pc, 0xEC00);
    // One edge, one interrupt.
    assert_eq!(cpu.step(&mut bus), 2);
    assert_eq!(cpu.pc, 0xEC01);
}

#[test]
fn an_interrupt_between_set_and_its_instruction_keeps_t_for_after_rti() {
    let (mut cpu, mut bus) = machine(&[0xF4, 0x09, 0x03]);
    vectors(&cpu, &mut bus);
    put(&cpu, &mut bus, 0xE800, &[0x40]);
    cpu.a = 0x5A;
    cpu.x = 0x10;
    bus.ram[0x10] = 0x0C;
    run(&mut cpu, &mut bus, 1);
    bus.irq = Some(0xFFF8);
    assert_eq!(cpu.step(&mut bus), 8);
    assert_eq!(stack(&bus, 0xFD), T);
    assert_eq!(cpu.p & T, 0);
    bus.irq = None;
    assert_eq!(cpu.step(&mut bus), 7);
    assert_eq!(cpu.p & T, T);
    assert_eq!(cpu.step(&mut bus), 5);
    assert_eq!((zp(&bus, 0x10), cpu.a), (0x0F, 0x5A));
}

#[test]
fn csh_and_csl_switch_the_speed() {
    let (mut cpu, mut bus) = machine(&[0xD4, 0x54]);
    assert_eq!(cpu.step(&mut bus), 3);
    assert!(cpu.high_speed);
    assert_eq!(cpu.step(&mut bus), 3);
    assert!(!cpu.high_speed);
}

#[test]
fn st0_st1_and_st2_write_to_the_huc6270_whatever_the_mapping() {
    let (mut cpu, mut bus) = machine(&[0x03, 0x05, 0x13, 0x34, 0x23, 0x12]);
    cpu.mpr = [0x00, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(run(&mut cpu, &mut bus, 3), [4, 4, 4]);
    assert_eq!(
        bus.writes,
        [(0x1FE000, 0x05), (0x1FE002, 0x34), (0x1FE003, 0x12)]
    );
}

#[test]
fn reset_loads_the_vector_through_the_mapping_and_sets_i() {
    let (mut cpu, mut bus) = machine(&[]);
    vectors(&cpu, &mut bus);
    cpu.p = D | T | C;
    cpu.high_speed = true;
    cpu.a = 0x12;
    cpu.s = 0x34;
    cpu.reset(&mut bus);
    assert_eq!(cpu.pc, 0xEE00);
    assert_eq!(cpu.p, I | C);
    assert!(!cpu.high_speed);
    assert_eq!((cpu.a, cpu.s, cpu.mpr), (0x12, 0x34, MPR));
}
