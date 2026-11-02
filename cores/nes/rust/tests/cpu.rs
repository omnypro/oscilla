//! The 6502: results, flags and the cycle count of each kind of instruction.

mod common;

use common::*;

const C: u8 = 0x01;
const Z: u8 = 0x02;
const V: u8 = 0x40;
const N: u8 = 0x80;

fn flags(m: &oscilla_nes::Machine) -> u8 {
    m.cpu_state().4
}

#[test]
fn a_program_starts_at_the_reset_vector_with_the_power_up_state() {
    let m = cpu(&[0xEA]);
    assert_eq!(m.cpu_state(), (0x8000, 0, 0, 0, 0x24, 0xFD));
    assert_eq!(m.cycles(), 7);
}

#[test]
fn loads_set_the_flags() {
    let mut m = cpu(&[0xA9, 0x00, 0xA2, 0x80, 0xA0, 0x7F]);
    steps(&mut m, 1);
    assert_eq!(flags(&m) & (Z | N), Z);
    steps(&mut m, 1);
    assert_eq!(flags(&m) & (Z | N), N);
    steps(&mut m, 1);
    assert_eq!(flags(&m) & (Z | N), 0);
    assert_eq!(
        (m.cpu_state().1, m.cpu_state().2, m.cpu_state().3),
        (0, 0x80, 0x7F)
    );
}

#[test]
fn adc_and_sbc_set_carry_and_overflow() {
    // 0x50 + 0x50 overflows into the sign bit; 0xFF + 1 carries.
    let mut m = cpu(&[0x18, 0xA9, 0x50, 0x69, 0x50]);
    steps(&mut m, 3);
    assert_eq!(m.cpu_state().1, 0xA0);
    assert_eq!(flags(&m) & (C | V | N), V | N);
    let mut m = cpu(&[0x18, 0xA9, 0xFF, 0x69, 0x01]);
    steps(&mut m, 3);
    assert_eq!(m.cpu_state().1, 0);
    assert_eq!(flags(&m) & (C | Z | V), C | Z);
    // SBC with the carry set subtracts exactly; 0x50 - 0xB0 overflows.
    let mut m = cpu(&[0x38, 0xA9, 0x50, 0xE9, 0xB0]);
    steps(&mut m, 3);
    assert_eq!(m.cpu_state().1, 0xA0);
    assert_eq!(flags(&m) & (C | V | N), V | N);
}

#[test]
fn compares_set_carry_when_the_register_is_not_below() {
    let mut m = cpu(&[0xA9, 0x10, 0xC9, 0x10, 0xC9, 0x11]);
    steps(&mut m, 2);
    assert_eq!(flags(&m) & (C | Z | N), C | Z);
    steps(&mut m, 1);
    assert_eq!(flags(&m) & (C | Z | N), N);
}

#[test]
fn shifts_and_rotates_move_the_carry() {
    let mut m = cpu(&[0x38, 0xA9, 0x81, 0x2A, 0x6A]);
    steps(&mut m, 3);
    assert_eq!((m.cpu_state().1, flags(&m) & C), (0x03, C));
    steps(&mut m, 1);
    assert_eq!((m.cpu_state().1, flags(&m) & C), (0x81, C));
    let mut m = cpu(&[0xA9, 0x81, 0x0A, 0x4A]);
    steps(&mut m, 2);
    assert_eq!((m.cpu_state().1, flags(&m) & C), (0x02, C));
    steps(&mut m, 1);
    assert_eq!((m.cpu_state().1, flags(&m) & C), (0x01, 0));
}

#[test]
fn bit_copies_bits_six_and_seven() {
    // The byte at $00 is 0xC0; A = 0x01 shares no bit with it.
    let mut m = cpu(&[0xA9, 0xC0, 0x85, 0x00, 0xA9, 0x01, 0x24, 0x00]);
    steps(&mut m, 4);
    assert_eq!(flags(&m) & (N | V | Z), N | V | Z);
}

#[test]
fn the_stack_keeps_values_and_flags() {
    // PHA, PHP, then change both and pull them back.
    let mut m = cpu(&[0xA9, 0x42, 0x38, 0x48, 0x08, 0xA9, 0x00, 0x18, 0x28, 0x68]);
    steps(&mut m, 5);
    assert_eq!(m.cpu_state().5, 0xFB);
    steps(&mut m, 1);
    assert_eq!(flags(&m) & C, 0);
    steps(&mut m, 1);
    assert_eq!(flags(&m) & C, C);
    steps(&mut m, 1);
    assert_eq!(m.cpu_state().1, 0x42);
    assert_eq!(m.cpu_state().5, 0xFD);
}

#[test]
fn jsr_and_rts_return_to_the_next_instruction() {
    // JSR $8005; LDA #$07 (skipped on the way out); the routine is RTS.
    let mut m = cpu(&[0x20, 0x06, 0x80, 0xA9, 0x07, 0xEA, 0x60]);
    steps(&mut m, 1);
    assert_eq!(m.cpu_state().0, 0x8006);
    steps(&mut m, 1);
    assert_eq!(m.cpu_state().0, 0x8003);
    steps(&mut m, 1);
    assert_eq!(m.cpu_state().1, 7);
}

#[test]
fn an_indirect_jump_does_not_carry_into_the_next_page() {
    // JMP ($82FF) reads its high byte from $8200, not $8300.
    let mut rom = vec![0xEA; 0x4000];
    rom[0] = 0x6C;
    rom[1] = 0xFF;
    rom[2] = 0x82;
    rom[0x2FF] = 0x34;
    rom[0x200] = 0x12;
    rom[0x300] = 0x56;
    rom[0x3FFC] = 0x00;
    rom[0x3FFD] = 0x80;
    let mut m = oscilla_nes::Machine::from_rom(&rom, 48000).unwrap();
    steps(&mut m, 1);
    assert_eq!(m.cpu_state().0, 0x1234);
}

#[test]
fn branches_take_the_cycles_of_their_outcome() {
    // Not taken 2, taken 3, taken across a page 4.
    let mut m = cpu(&[0xA9, 0x00, 0xD0, 0x00, 0xF0, 0x02, 0xEA, 0xEA]);
    steps(&mut m, 1);
    assert_eq!(steps(&mut m, 1), 2);
    assert_eq!(steps(&mut m, 1), 3);
    let mut rom = vec![0xEA; 0x4000];
    // At 0x80FD: BEQ +0x10 crosses into the page at 0x8100.
    rom[0xFB] = 0xA9;
    rom[0xFC] = 0x00;
    rom[0xFD] = 0xF0;
    rom[0xFE] = 0x10;
    rom[0] = 0x4C;
    rom[1] = 0xFB;
    rom[2] = 0x80;
    rom[0x3FFC] = 0x00;
    rom[0x3FFD] = 0x80;
    let mut m = oscilla_nes::Machine::from_rom(&rom, 48000).unwrap();
    steps(&mut m, 2);
    assert_eq!(steps(&mut m, 1), 4);
    assert_eq!(m.cpu_state().0, 0x810F);
}

#[test]
fn each_addressing_mode_takes_its_documented_cycles() {
    // (code, cycles) with X = 0 and Y = 0, so no read crosses a page.
    let cases: &[(&[u8], u64)] = &[
        (&[0xA9, 0x01], 2),
        (&[0xA5, 0x10], 3),
        (&[0xB5, 0x10], 4),
        (&[0xAD, 0x00, 0x02], 4),
        (&[0xBD, 0x00, 0x02], 4),
        (&[0xB9, 0x00, 0x02], 4),
        (&[0xA1, 0x10], 6),
        (&[0xB1, 0x10], 5),
        (&[0x85, 0x10], 3),
        (&[0x95, 0x10], 4),
        (&[0x8D, 0x00, 0x02], 4),
        (&[0x9D, 0x00, 0x02], 5),
        (&[0x99, 0x00, 0x02], 5),
        (&[0x81, 0x10], 6),
        (&[0x91, 0x10], 6),
        (&[0xE6, 0x10], 5),
        (&[0xF6, 0x10], 6),
        (&[0xEE, 0x00, 0x02], 6),
        (&[0xFE, 0x00, 0x02], 7),
        (&[0x0A], 2),
        (&[0x48], 3),
        (&[0x68], 4),
        (&[0x4C, 0x00, 0x80], 3),
        (&[0x6C, 0x00, 0x80], 5),
        (&[0x20, 0x00, 0x80], 6),
        (&[0xEA], 2),
        (&[0x04, 0x10], 3),
        (&[0x0C, 0x00, 0x02], 4),
        (&[0x1C, 0x00, 0x02], 4),
        (&[0xA7, 0x10], 3),
        (&[0x87, 0x10], 3),
        (&[0xC7, 0x10], 5),
        (&[0xDB, 0x00, 0x02], 7),
        (&[0xE3, 0x10], 8),
        (&[0xF3, 0x10], 8),
    ];
    for (code, cycles) in cases {
        let mut m = cpu(code);
        assert_eq!(steps(&mut m, 1), *cycles, "{code:02x?}");
    }
}

#[test]
fn an_indexed_read_takes_a_cycle_when_it_crosses_a_page() {
    // LDX #$FF; LDA $0201,X reads $0300.
    let mut m = cpu(&[0xA2, 0xFF, 0xBD, 0x01, 0x02, 0xA0, 0xFF, 0xB9, 0x01, 0x02]);
    steps(&mut m, 1);
    assert_eq!(steps(&mut m, 1), 5);
    steps(&mut m, 1);
    assert_eq!(steps(&mut m, 1), 5);
    // A write takes the cycle either way: STA $0200,X is 5 with no crossing.
    let mut m = cpu(&[0xA2, 0x01, 0x9D, 0x00, 0x02]);
    steps(&mut m, 1);
    assert_eq!(steps(&mut m, 1), 5);
}

#[test]
fn zero_page_indexing_wraps_inside_the_page() {
    // LDX #$FF; LDA #$5A; STA $00 (via $FF+1 wrap): STA $01,X stores at $00.
    let mut m = cpu(&[0xA2, 0xFF, 0xA9, 0x5A, 0x95, 0x01, 0xA5, 0x00]);
    steps(&mut m, 4);
    assert_eq!(m.peek(0x0000), 0x5A);
}

#[test]
fn the_ram_is_mirrored_four_times() {
    let mut m = cpu(&[0xA9, 0x33, 0x8D, 0x00, 0x08, 0x8D, 0x01, 0x18]);
    steps(&mut m, 3);
    assert_eq!(m.peek(0x0000), 0x33);
    assert_eq!(m.peek(0x1001), 0x33);
}

#[test]
fn lax_loads_both_registers_and_sax_stores_their_and() {
    let mut m = cpu(&[0xA9, 0x6C, 0x85, 0x10, 0xA7, 0x10, 0xA9, 0x0F, 0x87, 0x11]);
    steps(&mut m, 3);
    assert_eq!((m.cpu_state().1, m.cpu_state().2), (0x6C, 0x6C));
    steps(&mut m, 2);
    assert_eq!(m.peek(0x0011), 0x0F & 0x6C);
}

#[test]
fn the_read_modify_write_combinations_do_both_halves() {
    // SLO: shift left then OR into A.
    let mut m = cpu(&[0xA9, 0x41, 0x85, 0x10, 0xA9, 0x01, 0x07, 0x10]);
    steps(&mut m, 4);
    assert_eq!(m.peek(0x0010), 0x82);
    assert_eq!(m.cpu_state().1, 0x83);
    // DCP: decrement then compare.
    let mut m = cpu(&[0xA9, 0x05, 0x85, 0x10, 0xA9, 0x04, 0xC7, 0x10]);
    steps(&mut m, 4);
    assert_eq!(m.peek(0x0010), 0x04);
    assert_eq!(flags(&m) & (C | Z), C | Z);
    // ISC: increment then subtract.
    let mut m = cpu(&[0xA9, 0x05, 0x85, 0x10, 0x38, 0xA9, 0x0A, 0xE7, 0x10]);
    steps(&mut m, 5);
    assert_eq!(m.peek(0x0010), 0x06);
    assert_eq!(m.cpu_state().1, 0x04);
    // RRA: rotate right then add with the carry that came out.
    let mut m = cpu(&[0x18, 0xA9, 0x03, 0x85, 0x10, 0xA9, 0x10, 0x67, 0x10]);
    steps(&mut m, 5);
    assert_eq!(m.peek(0x0010), 0x01);
    assert_eq!(m.cpu_state().1, 0x12);
}

#[test]
fn the_immediate_combinations_follow_their_rules() {
    // ANC copies bit 7 of the result into the carry.
    let mut m = cpu(&[0xA9, 0xFF, 0x0B, 0x80]);
    steps(&mut m, 2);
    assert_eq!((m.cpu_state().1, flags(&m) & (C | N)), (0x80, C | N));
    // ALR: AND then shift right.
    let mut m = cpu(&[0xA9, 0xFF, 0x4B, 0x07]);
    steps(&mut m, 2);
    assert_eq!((m.cpu_state().1, flags(&m) & C), (0x03, C));
    // AXS: X = (A and X) - imm.
    let mut m = cpu(&[0xA9, 0xF0, 0xA2, 0x3C, 0xCB, 0x10]);
    steps(&mut m, 3);
    assert_eq!(m.cpu_state().2, 0x30 - 0x10);
    // The second SBC opcode is the first.
    let mut m = cpu(&[0x38, 0xA9, 0x50, 0xEB, 0x10]);
    steps(&mut m, 3);
    assert_eq!(m.cpu_state().1, 0x40);
}

#[test]
fn nop_variants_leave_the_state_alone() {
    let mut m = cpu(&[
        0x1A, 0x80, 0x01, 0x04, 0x10, 0x14, 0x10, 0x0C, 0x00, 0x02, 0x1C, 0x00, 0x02,
    ]);
    let before = m.cpu_state();
    steps(&mut m, 6);
    let after = m.cpu_state();
    assert_eq!(
        (after.1, after.2, after.3, after.4, after.5),
        (before.1, before.2, before.3, before.4, before.5)
    );
    assert_eq!(after.0, 0x8000 + 13);
}

#[test]
fn an_opcode_that_locks_the_cpu_is_reported() {
    let mut m = machine(&[0x02], &[]);
    m.run_cycles(100);
    assert!(String::from_utf8_lossy(m.warnings_nul()).contains("locks it up"));
}

#[test]
fn brk_is_reported_and_not_run() {
    let mut m = machine(&[0x00], &[]);
    m.run_cycles(100);
    assert!(String::from_utf8_lossy(m.warnings_nul()).contains("BRK"));
}

#[test]
fn an_unstable_opcode_is_skipped_with_a_warning() {
    // XAA #imm takes two bytes and leaves A alone.
    let mut m = machine(&[0xA9, 0x12, 0x8B, 0xFF, 0x85, 0x00], &[]);
    m.run_cycles(100);
    assert_eq!(m.peek(0x0000), 0x12);
    assert!(String::from_utf8_lossy(m.warnings_nul()).contains("unstable"));
}

#[test]
fn an_indirect_jump_through_the_last_address_wraps_inside_its_page() {
    // JMP ($FFFF) reads the low byte at $FFFF and the high byte at $FF00.
    let mut rom = vec![0xEA; 0x4000];
    rom[0] = 0x6C;
    rom[1] = 0xFF;
    rom[2] = 0xFF;
    rom[0x3FFC] = 0x00;
    rom[0x3FFD] = 0x80;
    rom[0x3FFF] = 0x34;
    let mut m = oscilla_nes::Machine::from_rom(&rom, 48000).unwrap();
    steps(&mut m, 1);
    assert_eq!(m.cpu_state().0, 0xEA34);
}

#[test]
fn lxa_loads_both_registers_with_the_byte() {
    let mut m = cpu(&[0xAB, 0x5A]);
    steps(&mut m, 1);
    assert_eq!((m.cpu_state().1, m.cpu_state().2), (0x5A, 0x5A));
}

#[test]
fn shy_stores_y_anded_with_the_high_byte_plus_one() {
    // SHY $0200,X with Y = $FF and X = 1 stores $FF & $03 at $0201.
    let mut m = cpu(&[0xA0, 0xFF, 0xA2, 0x01, 0x9C, 0x00, 0x02]);
    steps(&mut m, 3);
    assert_eq!(m.peek(0x0201), 0x03);
}
