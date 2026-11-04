//! The SPC700: results, flags and cycles of its instructions.

mod common;

use common::*;

fn regs(a: u8, x: u8, y: u8, psw: u8) -> Regs {
    Regs { a, x, y, psw, sp: 0xEF }
}

#[test]
fn immediate_loads_set_n_and_z() {
    let mut m = machine(&[0xE8, 0x80, 0xCD, 0x00, 0x8D, 0x7F]);
    assert_eq!(step(&mut m), 2);
    assert_eq!(flags(&m) & (N | Z), N);
    step(&mut m);
    assert_eq!(flags(&m) & (N | Z), Z);
    step(&mut m);
    assert_eq!(flags(&m) & (N | Z), 0);
    let (pc, a, x, y, _, _) = m.cpu_state();
    assert_eq!((pc, a, x, y), (0x406, 0x80, 0, 0x7F));
}

#[test]
fn register_moves_follow_the_table() {
    // MOV X,A  MOV Y,A  MOV A,X  MOV A,Y  MOV X,SP  MOV SP,X
    let mut m = machine_with(&[0x5D, 0xFD, 0x7D, 0xDD, 0x9D, 0xBD], regs(0x42, 0, 0, 0));
    for _ in 0..6 {
        assert_eq!(step(&mut m), 2);
    }
    let (_, a, x, y, _, sp) = m.cpu_state();
    assert_eq!((a, x, y, sp), (0x42, 0xEF, 0x42, 0xEF));
}

#[test]
fn loads_through_each_addressing_mode() {
    let ram = [(0x10, 0x11), (0x15, 0x22), (0x1234, 0x33), (0x1237, 0x44), (0x1239, 0x55),
               (0x20, 0x00), (0x21, 0x30), (0x3005, 0x66), (0x30, 0x77), (0x31, 0x30), (0x3003, 0x88)];
    // x = 5, y = 3
    let prog = [
        0xE4, 0x10, // A = [10]
        0xF4, 0x10, // A = [10+X]
        0xE5, 0x34, 0x12, // A = [1234]
        0xF5, 0x32, 0x12, // A = [1232+X]
        0xF6, 0x36, 0x12, // A = [1236+Y]
        0xE6, // A = [X]
        0xF7, 0x20, // A = [[20]+Y]
        0xE7, 0x25, // A = [[25+X]]... pointer at 0x2A
    ];
    let mut m = machine_ram(&prog, regs(0, 5, 3, 0), &[(0x2A, 0x00), (0x2B, 0x30), (0x3000, 0x99), (0x00, 0x00)]
        .iter().chain(ram.iter()).copied().collect::<Vec<_>>());
    let mut seen = Vec::new();
    let mut cycles = Vec::new();
    for _ in 0..8 {
        cycles.push(step(&mut m));
        seen.push(m.cpu_state().1);
    }
    assert_eq!(seen, [0x11, 0x22, 0x33, 0x44, 0x55, 0x00, 0x88, 0x99]);
    assert_eq!(cycles, [3, 4, 4, 5, 5, 3, 6, 6]);
}

#[test]
fn stores_write_and_cost_what_the_table_says() {
    let prog = [
        0xC4, 0x30, // [30] = A
        0xD8, 0x31, // [31] = X
        0xCB, 0x32, // [32] = Y
        0xC5, 0x00, 0x20, // [2000] = A
        0xD5, 0x00, 0x20, // [2000+X] = A
        0x8F, 0x7E, 0x40, // [40] = 7E
        0xFA, 0x30, 0x41, // [41] = [30]
        0xAF, // [X]+ = A
    ];
    let mut m = machine_with(&prog, regs(0xAA, 2, 3, 0));
    let cycles: Vec<u64> = (0..8).map(|_| step(&mut m)).collect();
    assert_eq!(cycles, [4, 4, 4, 5, 6, 5, 5, 4]);
    assert_eq!(m.peek(0x30), 0xAA);
    assert_eq!(m.peek(0x31), 2);
    assert_eq!(m.peek(0x32), 3);
    assert_eq!(m.peek(0x2000), 0xAA);
    assert_eq!(m.peek(0x2002), 0xAA);
    assert_eq!(m.peek(0x40), 0x7E);
    assert_eq!(m.peek(0x41), 0xAA);
    let (_, _, x, ..) = m.cpu_state();
    assert_eq!(x, 3);
}

#[test]
fn movw_moves_a_word_and_wraps_inside_the_page() {
    let mut m = machine_ram(
        &[0x40, 0xBA, 0xFF, 0xDA, 0x10],
        ZERO,
        &[(0x1FF, 0x34), (0x100, 0x92)],
    );
    step(&mut m);
    assert_eq!(step(&mut m), 5);
    let (_, a, _, y, psw, _) = m.cpu_state();
    assert_eq!((a, y), (0x34, 0x92));
    assert_eq!(psw & (N | Z), N);
    assert_eq!(step(&mut m), 5);
    assert_eq!((m.peek(0x110), m.peek(0x111)), (0x34, 0x92));
}

#[test]
fn adc_sets_carry_overflow_and_half_carry() {
    // ADC A,#imm
    let mut m = machine_with(&[0x88, 0x01], regs(0xFF, 0, 0, 0));
    assert_eq!(step(&mut m), 2);
    let f = flags(&m);
    assert_eq!(m.cpu_state().1, 0);
    assert_eq!(f & (C | Z | H | V | N), C | Z | H);
    let mut m = machine_with(&[0x88, 0x01], regs(0x7F, 0, 0, 0));
    step(&mut m);
    assert_eq!(flags(&m) & (C | V | N | H), V | N | H);
    let mut m = machine_with(&[0x88, 0x00], regs(0x10, 0, 0, C));
    step(&mut m);
    assert_eq!((m.cpu_state().1, flags(&m) & C), (0x11, 0));
}

#[test]
fn sbc_borrows_through_the_carry_flag() {
    // Carry set means no borrow in.
    let mut m = machine_with(&[0xA8, 0x01], regs(0x10, 0, 0, C));
    step(&mut m);
    assert_eq!((m.cpu_state().1, flags(&m) & (C | H | V)), (0x0F, C));
    let mut m = machine_with(&[0xA8, 0x01], regs(0x10, 0, 0, 0));
    step(&mut m);
    assert_eq!(m.cpu_state().1, 0x0E);
    let mut m = machine_with(&[0xA8, 0x01], regs(0x00, 0, 0, C));
    step(&mut m);
    assert_eq!((m.cpu_state().1, flags(&m) & (C | N)), (0xFF, N));
    let mut m = machine_with(&[0xA8, 0x01], regs(0x80, 0, 0, C));
    step(&mut m);
    assert_eq!(flags(&m) & V, V);
}

#[test]
fn compare_sets_carry_when_the_register_is_not_smaller() {
    let mut m = machine_with(&[0x68, 0x10, 0x68, 0x11, 0x68, 0x0F], regs(0x10, 0, 0, 0));
    step(&mut m);
    assert_eq!(flags(&m) & (C | Z | N), C | Z);
    step(&mut m);
    assert_eq!(flags(&m) & (C | Z | N), N);
    step(&mut m);
    assert_eq!(flags(&m) & (C | Z | N), C);
    assert_eq!(m.cpu_state().1, 0x10);
}

#[test]
fn logic_operations_in_every_mode() {
    let mut m = machine_ram(
        &[0x08, 0x0F, 0x28, 0x3C, 0x48, 0xFF, 0x04, 0x50, 0x18, 0x01, 0x50, 0x09, 0x50, 0x51, 0x19],
        regs(0xF0, 0, 0, 0),
        &[(0x50, 0x81), (0x51, 0x02)],
    );
    let cycles: Vec<u64> = (0..7).map(|_| step(&mut m)).collect();
    assert_eq!(cycles, [2, 2, 2, 3, 5, 6, 5]);
    let (_, a, ..) = m.cpu_state();
    // F0|0F = FF; &3C = 3C; ^FF = C3; |[50]=81 -> C3.
    assert_eq!(a, 0xC3);
    // [50] = 81|01 = 81, and [51] = 02|[50] = 83.
    assert_eq!(m.peek(0x50), 0x81);
    assert_eq!(m.peek(0x51), 0x83);
}

#[test]
fn increment_decrement_and_shifts() {
    let mut m = machine_ram(
        &[0xBC, 0x1C, 0x3C, 0x5C, 0x7C, 0xAB, 0x60, 0x8B, 0x61, 0x0C, 0x00, 0x20],
        regs(0xFF, 0, 0, 0),
        &[(0x60, 0x7F), (0x2000, 0x81)],
    );
    step(&mut m); // INC A -> 0
    assert_eq!(flags(&m) & Z, Z);
    assert_eq!(m.cpu_state().1, 0);
    m.set_pc(0x401);
    step(&mut m); // ASL A
    assert_eq!(m.cpu_state().1, 0);
    let mut m = machine_with(&[0x3C, 0x5C, 0x7C], regs(0x81, 0, 0, C));
    step(&mut m); // ROL: 03, carry out 1
    assert_eq!((m.cpu_state().1, flags(&m) & C), (0x03, C));
    step(&mut m); // LSR: 01, carry 1
    assert_eq!((m.cpu_state().1, flags(&m) & C), (0x01, C));
    step(&mut m); // ROR with carry: 80, carry 1
    assert_eq!((m.cpu_state().1, flags(&m) & (C | N)), (0x80, C | N));
}

#[test]
fn rmw_instructions_write_back() {
    let mut m = machine_ram(
        &[0xAB, 0x60, 0x8B, 0x61, 0x0C, 0x00, 0x20, 0xBB, 0x5F],
        regs(0, 1, 0, 0),
        &[(0x60, 0x7F), (0x61, 0x01), (0x2000, 0x81)],
    );
    assert_eq!(step(&mut m), 4);
    assert_eq!(m.peek(0x60), 0x80);
    assert_eq!(step(&mut m), 4);
    assert_eq!(m.peek(0x61), 0x00);
    assert_eq!(step(&mut m), 5);
    assert_eq!(m.peek(0x2000), 0x02);
    assert_eq!(flags(&m) & C, C);
    assert_eq!(step(&mut m), 5);
    assert_eq!(m.peek(0x60), 0x81);
}

#[test]
fn word_arithmetic() {
    let mut m = machine_ram(&[0x7A, 0x10, 0x9A, 0x12, 0x5A, 0x10, 0x3A, 0x14, 0x1A, 0x16], regs(0xFF, 0, 0x0F, 0),
        &[(0x10, 0x01), (0x11, 0x00), (0x12, 0x00), (0x13, 0x10), (0x14, 0xFF), (0x15, 0xFF), (0x16, 0x00), (0x17, 0x00)]);
    assert_eq!(step(&mut m), 5); // YA = 0FFF + 0001 = 1000
    let (_, a, _, y, psw, _) = m.cpu_state();
    assert_eq!((y, a), (0x10, 0x00));
    assert_eq!(psw & H, H);
    assert_eq!(step(&mut m), 5); // YA = 1000 - 1000 = 0
    assert_eq!(flags(&m) & (Z | C), Z | C);
    assert_eq!(step(&mut m), 4); // CMPW YA(0), 1 -> borrow
    assert_eq!(flags(&m) & (N | C), N);
    assert_eq!(step(&mut m), 6); // INCW FFFF -> 0000
    assert_eq!((m.peek(0x14), m.peek(0x15)), (0, 0));
    assert_eq!(flags(&m) & Z, Z);
    assert_eq!(step(&mut m), 6); // DECW 0 -> FFFF
    assert_eq!((m.peek(0x16), m.peek(0x17)), (0xFF, 0xFF));
}

#[test]
fn multiply_and_divide() {
    let mut m = machine_with(&[0xCF], regs(200, 0, 100, 0));
    assert_eq!(step(&mut m), 9);
    let (_, a, _, y, ..) = m.cpu_state();
    assert_eq!(u16::from(y) << 8 | u16::from(a), 20000);
    assert_eq!(flags(&m) & (N | Z), 0);
    let mut m = machine_with(&[0x9E], regs(0x00, 7, 0x01, 0));
    // YA = 0x100 = 256, X = 7 -> 36 remainder 4.
    assert_eq!(step(&mut m), 12);
    let (_, a, _, y, ..) = m.cpu_state();
    assert_eq!((a, y), (36, 4));
    assert_eq!(flags(&m) & V, 0);
    let mut m = machine_with(&[0x9E], regs(0, 1, 0x02, 0));
    step(&mut m);
    assert_eq!(flags(&m) & V, V);
}

#[test]
fn bit_instructions_use_the_thirteen_bit_address() {
    // SET1 [10].3 ; CLR1 [10].0 ; MOV1 C,[0123].5 ; MOV1 [0124].7,C ; NOT1 [0123].5
    let mut m = machine_ram(
        &[0x62, 0x10, 0x12, 0x10, 0xAA, 0x23, 0xA1, 0xCA, 0x24, 0xE1, 0xEA, 0x23, 0xA1],
        ZERO,
        &[(0x10, 0x01), (0x123, 0x20)],
    );
    assert_eq!(step(&mut m), 4);
    assert_eq!(m.peek(0x10), 0x09);
    assert_eq!(step(&mut m), 4);
    assert_eq!(m.peek(0x10), 0x08);
    assert_eq!(step(&mut m), 4);
    assert_eq!(flags(&m) & C, C);
    assert_eq!(step(&mut m), 6);
    assert_eq!(m.peek(0x124), 0x80);
    assert_eq!(step(&mut m), 5);
    assert_eq!(m.peek(0x123), 0x00);
}

#[test]
fn tset_and_tclr_change_memory_and_flag_from_the_compare() {
    let mut m = machine_ram(&[0x0E, 0x00, 0x20, 0x4E, 0x00, 0x20], regs(0x0F, 0, 0, 0), &[(0x2000, 0xF0)]);
    assert_eq!(step(&mut m), 6);
    assert_eq!(m.peek(0x2000), 0xFF);
    assert_eq!(step(&mut m), 6);
    assert_eq!(m.peek(0x2000), 0xF0);
}

#[test]
fn branches_cost_two_more_when_taken() {
    let mut m = machine(&[0xF0, 0x02, 0xD0, 0x02, 0xE8, 0x00, 0x2F, 0x00]);
    assert_eq!(step(&mut m), 2); // BEQ not taken (Z clear)
    assert_eq!(step(&mut m), 4); // BNE taken
    assert_eq!(m.cpu_state().0, 0x406);
}

#[test]
fn bra_goes_back() {
    let mut m = machine(&[0x00, 0x2F, 0xFD]);
    step(&mut m);
    assert_eq!(step(&mut m), 4);
    assert_eq!(m.cpu_state().0, 0x400);
}

#[test]
fn dbnz_and_cbne() {
    // DBNZ Y,rel loops three times.
    let mut m = machine_with(&[0xFE, 0xFE], regs(0, 0, 3, 0));
    assert_eq!(step(&mut m), 6);
    assert_eq!(step(&mut m), 6);
    assert_eq!(step(&mut m), 4);
    assert_eq!(m.cpu_state().3, 0);
    // CBNE dp,rel
    let mut m = machine_ram(&[0x2E, 0x30, 0x02, 0x00, 0x00, 0x2E, 0x30, 0x00], regs(5, 0, 0, 0), &[(0x30, 5)]);
    assert_eq!(step(&mut m), 5);
    assert_eq!(m.cpu_state().0, 0x403);
    let mut m = machine_ram(&[0x2E, 0x30, 0x02, 0x00, 0x00], regs(4, 0, 0, 0), &[(0x30, 5)]);
    assert_eq!(step(&mut m), 7);
    assert_eq!(m.cpu_state().0, 0x405);
}

#[test]
fn bbs_and_bbc_test_one_bit() {
    let mut m = machine_ram(&[0x23, 0x30, 0x10], ZERO, &[(0x30, 0x02)]);
    // BBS1 taken.
    assert_eq!(step(&mut m), 7);
    assert_eq!(m.cpu_state().0, 0x413);
    let mut m = machine_ram(&[0x13, 0x30, 0x10], ZERO, &[(0x30, 0x01)]);
    // BBC0 with the bit set: not taken.
    assert_eq!(step(&mut m), 5);
}

#[test]
fn call_ret_and_the_stack() {
    let mut m = machine(&[0x3F, 0x00, 0x05]);
    assert_eq!(step(&mut m), 8);
    let (pc, .., sp) = m.cpu_state();
    assert_eq!((pc, sp), (0x500, 0xED));
    // The return address 0x403 is stored high byte first.
    assert_eq!((m.peek(0x1EF), m.peek(0x1EE)), (0x04, 0x03));
    let mut f = file(&[0x6F], Regs { sp: 0xED, ..ZERO }, &[]);
    f[0x100 + 0x1EE] = 0x34;
    f[0x100 + 0x1EF] = 0x12;
    let mut m = oscilla_snes::Machine::new(&oscilla_snes::Spc::parse(&f).unwrap(), 32000).unwrap();
    assert_eq!(step(&mut m), 5);
    assert_eq!(m.cpu_state().0, 0x1234);
}

#[test]
fn tcall_and_pcall() {
    let mut f = file(&[0x01, 0x4F, 0x20], ZERO, &[]);
    // TCALL 0 reads its vector at 0xFFDE.
    f[0x100 + 0xFFDE] = 0x00;
    f[0x100 + 0xFFDF] = 0x06;
    let mut m = oscilla_snes::Machine::new(&oscilla_snes::Spc::parse(&f).unwrap(), 32000).unwrap();
    assert_eq!(step(&mut m), 8);
    assert_eq!(m.cpu_state().0, 0x600);
    let mut m = machine(&[0x4F, 0x20]);
    assert_eq!(step(&mut m), 6);
    assert_eq!(m.cpu_state().0, 0xFF20);
}

#[test]
fn push_and_pop() {
    let mut m = machine_with(&[0x2D, 0xCD, 0x00, 0xCE, 0x6D, 0xAE], regs(0x5A, 0, 9, 0x2));
    assert_eq!(step(&mut m), 4);
    step(&mut m);
    assert_eq!(step(&mut m), 4);
    assert_eq!(m.cpu_state().2, 0x5A);
    step(&mut m);
    step(&mut m);
    assert_eq!(m.cpu_state().1, 9);
}

#[test]
fn the_p_flag_moves_the_direct_page() {
    let mut m = machine_ram(&[0x40, 0xE4, 0x10, 0x20, 0xE4, 0x10], ZERO, &[(0x110, 0x77), (0x10, 0x33)]);
    step(&mut m);
    step(&mut m);
    assert_eq!(m.cpu_state().1, 0x77);
    step(&mut m);
    step(&mut m);
    assert_eq!(m.cpu_state().1, 0x33);
}

#[test]
fn direct_page_indexing_wraps_inside_the_page() {
    let mut m = machine_ram(&[0xF4, 0xFF], regs(0, 2, 0, 0), &[(0x01, 0x5C)]);
    step(&mut m);
    assert_eq!(m.cpu_state().1, 0x5C);
}

#[test]
fn xcn_swaps_nibbles_and_daa_adjusts() {
    let mut m = machine_with(&[0x9F], regs(0x3C, 0, 0, 0));
    assert_eq!(step(&mut m), 5);
    assert_eq!(m.cpu_state().1, 0xC3);
    // 0x19 + 0x28 = 0x41 (binary), DAA gives 0x47.
    let mut m = machine_with(&[0x88, 0x28, 0xDF], regs(0x19, 0, 0, 0));
    step(&mut m);
    assert_eq!(step(&mut m), 3);
    assert_eq!(m.cpu_state().1, 0x47);
}

#[test]
fn jmp_through_a_table() {
    let mut f = file(&[0x1F, 0x00, 0x20], Regs { x: 2, ..ZERO }, &[]);
    f[0x100 + 0x2002] = 0x78;
    f[0x100 + 0x2003] = 0x56;
    let mut m = oscilla_snes::Machine::new(&oscilla_snes::Spc::parse(&f).unwrap(), 32000).unwrap();
    assert_eq!(step(&mut m), 6);
    assert_eq!(m.cpu_state().0, 0x5678);
}

#[test]
fn stop_and_sleep_halt_the_processor() {
    let mut m = machine(&[0xFF]);
    step(&mut m);
    assert_eq!(m.end_state(), oscilla_snes::EndState::Stalled);
    let pc = m.cpu_state().0;
    step(&mut m);
    assert_eq!(m.cpu_state().0, pc);
}
