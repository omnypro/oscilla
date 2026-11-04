//! The ports, the timers, the boot ROM, snapshots and the rendered output.

mod common;

use common::*;
use oscilla_snes::{Machine, Observer, RestoreError, Spc};

fn run_instructions(m: &mut Machine, n: usize) {
    for _ in 0..n {
        m.step_instruction();
    }
}

#[test]
fn the_boot_rom_shows_while_bit_seven_of_the_control_register_is_set() {
    // MOV A,!FFC0 ; MOV $F1,#80 ; MOV A,!FFC0
    let mut m = machine(&[0xE5, 0xC0, 0xFF, 0x8F, 0x80, 0xF1, 0xE5, 0xC0, 0xFF]);
    run_instructions(&mut m, 1);
    assert_eq!(m.cpu_state().1, 0x00);
    run_instructions(&mut m, 2);
    assert_eq!(m.cpu_state().1, 0xCD);
}

#[test]
fn stores_under_the_boot_rom_reach_the_ram() {
    // With the ROM on, MOV !FFC0,A still writes RAM; turn the ROM off to read it.
    let mut m = machine_with(
        &[0x8F, 0x80, 0xF1, 0xC5, 0xC0, 0xFF, 0x8F, 0x00, 0xF1, 0xE5, 0xC0, 0xFF],
        Regs { a: 0x5A, ..ZERO },
    );
    run_instructions(&mut m, 5);
    assert_eq!(m.cpu_state().1, 0x5A);
}

#[test]
fn the_extra_ram_of_the_file_sits_under_the_rom_when_the_rom_shows() {
    let mut f = file(&[0x8F, 0x00, 0xF1, 0xE5, 0xC0, 0xFF], ZERO, &[]);
    f[0x100 + 0xF1] = 0x80;
    f[0x101C0] = 0x99;
    let mut m = Machine::new(&Spc::parse(&f).unwrap(), 32000).unwrap();
    run_instructions(&mut m, 2);
    assert_eq!(m.cpu_state().1, 0x99);
}

/// The main CPU's side of the upload protocol.
fn upload(m: &mut Machine, dest: u16, bytes: &[u8], entry: u16) {
    let wait = |m: &mut Machine, port: usize, want: u8| {
        for _ in 0..10_000 {
            if m.read_port(port) == want {
                return;
            }
            m.step_instruction();
        }
        panic!("port {port} never read {want:#04X}, it reads {:#04X}", m.read_port(port));
    };
    wait(m, 0, 0xAA);
    wait(m, 1, 0xBB);
    m.write_port(2, dest as u8);
    m.write_port(3, (dest >> 8) as u8);
    m.write_port(1, 1);
    m.write_port(0, 0xCC);
    wait(m, 0, 0xCC);
    for (i, &b) in bytes.iter().enumerate() {
        m.write_port(1, b);
        m.write_port(0, i as u8);
        wait(m, 0, i as u8);
    }
    let kick = ((bytes.len() as u8).wrapping_add(2)) | 1;
    m.write_port(2, entry as u8);
    m.write_port(3, (entry >> 8) as u8);
    m.write_port(1, 0);
    m.write_port(0, kick);
    wait(m, 0, kick);
}

#[test]
fn the_boot_rom_takes_a_block_through_the_ports_and_enters_it() {
    // Jump to the reset address with the ROM on.
    let mut m = machine(&[0x8F, 0x80, 0xF1, 0x5F, 0xC0, 0xFF]);
    // The program: MOV $20,#77 ; BRA self
    let program = [0x8F, 0x77, 0x20, 0x2F, 0xFE];
    upload(&mut m, 0x0500, &program, 0x0500);
    run_instructions(&mut m, 20);
    assert_eq!(m.peek(0x20), 0x77);
    assert_eq!(&[m.peek(0x500), m.peek(0x501), m.peek(0x502)], &[0x8F, 0x77, 0x20]);
}

#[test]
fn the_boot_rom_clears_the_zero_page_before_it_announces() {
    let mut f = file(&[0x8F, 0x80, 0xF1, 0x5F, 0xC0, 0xFF], ZERO, &[]);
    f[0x100 + 0x30] = 0xEE;
    f[0x100 + 0xE0] = 0xEE;
    let mut m = Machine::new(&Spc::parse(&f).unwrap(), 32000).unwrap();
    run_instructions(&mut m, 2000);
    assert_eq!((m.peek(0x30), m.peek(0xE0)), (0, 0));
    assert_eq!((m.read_port(0), m.read_port(1)), (0xAA, 0xBB));
}

#[test]
fn a_jump_to_0xffc9_announces_again_without_clearing() {
    let mut f = file(&[0x8F, 0x80, 0xF1, 0x5F, 0xC9, 0xFF], ZERO, &[]);
    f[0x100 + 0x30] = 0xEE;
    let mut m = Machine::new(&Spc::parse(&f).unwrap(), 32000).unwrap();
    run_instructions(&mut m, 200);
    assert_eq!(m.peek(0x30), 0xEE);
    assert_eq!((m.read_port(0), m.read_port(1)), (0xAA, 0xBB));
}

#[test]
fn the_input_ports_hold_what_the_main_cpu_wrote() {
    // MOV A,$F4 ; MOV A,$F7
    let mut m = machine(&[0xE4, 0xF4, 0xE4, 0xF7]);
    m.write_port(0, 0x12);
    m.write_port(3, 0x34);
    run_instructions(&mut m, 1);
    assert_eq!(m.cpu_state().1, 0x12);
    run_instructions(&mut m, 1);
    assert_eq!(m.cpu_state().1, 0x34);
}

#[test]
fn the_output_ports_are_separate_from_the_inputs() {
    // MOV $F5,#66 ; MOV A,$F5
    let mut m = machine(&[0x8F, 0x66, 0xF5, 0xE4, 0xF5]);
    m.write_port(1, 0x11);
    run_instructions(&mut m, 2);
    assert_eq!(m.read_port(1), 0x66);
    assert_eq!(m.cpu_state().1, 0x11);
    assert_eq!(m.peek(0xF5), 0x66);
}

#[test]
fn the_control_register_clears_the_input_latches() {
    // MOV $F1,#10 clears ports 0 and 1; MOV $F1,#20 clears ports 2 and 3.
    let mut m = machine(&[0x8F, 0x10, 0xF1, 0xE4, 0xF4, 0xE4, 0xF6, 0x8F, 0x20, 0xF1, 0xE4, 0xF6]);
    m.write_port(0, 9);
    m.write_port(2, 8);
    run_instructions(&mut m, 2);
    assert_eq!(m.cpu_state().1, 0);
    run_instructions(&mut m, 1);
    assert_eq!(m.cpu_state().1, 8);
    run_instructions(&mut m, 2);
    assert_eq!(m.cpu_state().1, 0);
}

/// A program that sets a divider, enables a timer, spins `loops` times and
/// reads the timer's output into A.
fn timer_program(timer: u8, divider: u8, loops: u8) -> Vec<u8> {
    vec![
        0x8F, divider, 0xFA + timer,
        0x8F, 1 << timer, 0xF1,
        0xCD, loops,
        0x1D,
        0xD0, 0xFD,
        0xE4, 0xFD + timer,
        0x2F, 0xFE,
    ]
}

fn timer_value(timer: u8, divider: u8, loops: u8) -> (u8, u64) {
    let mut m = machine(&timer_program(timer, divider, loops));
    // The divider store, the enable store, the counter load.
    run_instructions(&mut m, 3);
    let enabled_at = m.cycles();
    while m.cpu_state().0 != 0x40B {
        m.step_instruction();
    }
    m.step_instruction();
    (m.cpu_state().1, m.cycles() - enabled_at)
}

#[test]
fn timer_zero_counts_once_per_128_cycles_times_the_divider() {
    // 6 cycles a loop.
    let (n, cycles) = timer_value(0, 1, 100);
    assert!((cycles / 128).abs_diff(u64::from(n)) <= 1, "{n} after {cycles}");
    let (n, cycles) = timer_value(0, 4, 250);
    assert!((cycles / 512).abs_diff(u64::from(n)) <= 1, "{n} after {cycles}");
}

#[test]
fn timer_two_counts_eight_times_as_fast_as_the_others() {
    let (n, cycles) = timer_value(2, 1, 20);
    assert!((cycles / 16).abs_diff(u64::from(n)) <= 1, "{n} after {cycles}");
    let (slow, _) = timer_value(1, 1, 20);
    assert!(slow <= 1);
}

#[test]
fn the_output_is_four_bits_and_a_read_clears_it() {
    // 255 loops of 6 cycles is about 1530 cycles: 11 ticks at 128.
    let mut m = machine(&[
        0x8F, 0x01, 0xFA, 0x8F, 0x01, 0xF1, 0xCD, 0xFF, 0x1D, 0xD0, 0xFD, 0xE4, 0xFD, 0xE4, 0xFD, 0x2F, 0xFE,
    ]);
    run_instructions(&mut m, 3 + 2 * 255 + 1);
    let first = m.cpu_state().1;
    run_instructions(&mut m, 1);
    assert!((10..=12).contains(&first), "{first}");
    assert_eq!(m.cpu_state().1, 0);
    // 20 ticks wrap the four-bit output.
    let mut m = machine(&[
        0x8F, 0x01, 0xFA, 0x8F, 0x01, 0xF1, 0xCD, 0xFF, 0x1D, 0xD0, 0xFD,
        0xCD, 0xFF, 0x1D, 0xD0, 0xFD, 0xCD, 0xFF, 0x1D, 0xD0, 0xFD, 0xE4, 0xFD, 0x2F, 0xFE,
    ]);
    run_instructions(&mut m, 3 + 3 * 2 * 255 + 1);
    assert!(m.cpu_state().1 < 16);
}

#[test]
fn a_disabled_timer_does_not_count() {
    let mut m = machine(&[0x8F, 0x01, 0xFA, 0xCD, 0xFF, 0x1D, 0xD0, 0xFD, 0xE4, 0xFD, 0x2F, 0xFE]);
    run_instructions(&mut m, 2 + 2 * 255 + 1);
    assert_eq!(m.cpu_state().1, 0);
}

#[test]
fn a_divider_of_zero_counts_to_256() {
    let (n, _) = timer_value(2, 0, 250);
    // 250 loops are about 1500 cycles: far short of 256 * 16.
    assert_eq!(n, 0);
}

#[test]
fn the_observer_sees_dsp_stores_at_the_end_of_their_instruction() {
    struct Log(std::rc::Rc<std::cell::RefCell<Vec<(u64, u8, u8)>>>);
    impl Observer for Log {
        fn dsp_write(&mut self, cycle: u64, register: u8, value: u8) {
            self.0.borrow_mut().push((cycle, register, value));
        }
        fn port_write(&mut self, _: u64, _: u8, _: u8) {}
    }
    let mut m = machine(&dsp_program(&[(0x0C, 0x55), (0x1C, 0x66)]));
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    m.set_observer(Some(Box::new(Log(seen.clone()))));
    run_instructions(&mut m, 4);
    // MOV [F2],#reg takes 5 cycles and MOV [F3],#value takes 5.
    assert_eq!(*seen.borrow(), [(10, 0x0C, 0x55), (20, 0x1C, 0x66)]);
    assert_eq!(m.dsp_register(0x0C), 0x55);
}

#[test]
fn reading_the_dsp_data_port_returns_the_register_that_the_address_port_names() {
    // MOV [F2],#0C ; MOV [F3],#55 ; MOV [F2],#0C ; MOV A,[F3]
    let mut m = machine(&[0x8F, 0x0C, 0xF2, 0x8F, 0x55, 0xF3, 0x8F, 0x0C, 0xF2, 0xE4, 0xF3]);
    run_instructions(&mut m, 4);
    assert_eq!(m.cpu_state().1, 0x55);
}

#[test]
fn a_snapshot_continues_exactly() {
    let mut f = dsp_machine(&dsp_file(
        &dsp_program(&[(0x4C, 0x01)]),
        &voice0(0x0C00, 0x7F, 0x40),
        &[(0x2000, block(12, 0, 3, [7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8]))],
        0x2000,
        0x2000,
    ));
    render(&mut f, 3000);
    let snap = f.snapshot();
    let after = render(&mut f, 4000);
    let mut g = dsp_machine(&dsp_file(
        &dsp_program(&[(0x4C, 0x01)]),
        &voice0(0x0C00, 0x7F, 0x40),
        &[(0x2000, block(12, 0, 3, [7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8]))],
        0x2000,
        0x2000,
    ));
    g.restore(&snap).unwrap();
    assert_eq!(render(&mut g, 4000), after);
}

#[test]
fn a_snapshot_refuses_another_rate_or_file() {
    let a = machine(&[0x2F, 0xFE]);
    let snap = a.snapshot();
    let mut other_rate = Machine::new(&spc(&[0x2F, 0xFE], ZERO, &[]), 48000).unwrap();
    assert_eq!(other_rate.restore(&snap), Err(RestoreError::Rate));
    let mut other_file = machine(&[0x00, 0x2F, 0xFD]);
    assert_eq!(other_file.restore(&snap), Err(RestoreError::Image));
}

#[test]
fn a_restore_keeps_the_channel_mask_of_the_listener() {
    let mut m = machine(&[0x2F, 0xFE]);
    let snap = m.snapshot();
    m.set_channel_mask(0x0F);
    m.restore(&snap).unwrap();
    assert_eq!(m.channel_mask(), 0x0F);
}

#[test]
fn reset_starts_the_song_again() {
    let mut m = machine(&[0xBC, 0x2F, 0xFD]);
    run_instructions(&mut m, 20);
    assert_ne!(m.cpu_state().1, 0);
    m.reset();
    assert_eq!((m.cpu_state().0, m.cpu_state().1, m.cycles()), (START, 0, 0));
}

#[test]
fn float_and_integer_output_agree() {
    let spc = dsp_file(
        &[0x2F, 0xFE],
        &voice0(0x1000, 0x7F, 0x40),
        &[(0x2000, block(12, 0, 3, [7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8]))],
        0x2000,
        0x2000,
    );
    let mut a = dsp_machine(&spc);
    let mut b = dsp_machine(&spc);
    let ints = render(&mut a, 2000);
    let mut floats = vec![0f32; 4000];
    b.render_f32(&mut floats);
    for (i, f) in ints.iter().zip(&floats) {
        assert!((f64::from(*i) / 32768.0 - f64::from(*f)).abs() < 1e-4);
    }
}

#[test]
fn output_at_32000_hz_follows_the_dsp_samples() {
    let spc = dsp_file(
        &[0x2F, 0xFE],
        &voice0(0x1000, 0x7F, 0x40),
        &[(0x2000, block(12, 0, 3, [7; 16]))],
        0x2000,
        0x2000,
    );
    let mut m = dsp_machine(&spc);
    let out = render(&mut m, 2000);
    let mut n = dsp_machine(&spc);
    let raw = n.dsp_samples(2000);
    // A steady level passes through the resampler unchanged.
    assert!((i32::from(out[2 * 1500]) - i32::from(raw[2 * 1500 - 16])).abs() <= 2 || raw[2 * 1500] == out[2 * 1500]);
}

#[test]
fn the_end_state_reports_a_halted_processor() {
    let mut m = machine(&[0xEF]);
    assert_eq!(m.end_state(), oscilla_snes::EndState::Running);
    run_instructions(&mut m, 2);
    assert_eq!(m.end_state(), oscilla_snes::EndState::Stalled);
}

#[test]
fn quiet_cycles_count_since_the_output_last_changed() {
    let mut m = machine(&[0x2F, 0xFE]);
    m.run_cycles(10_000);
    assert!(m.quiet_cycles() >= 9_000);
}

#[test]
fn the_output_rate_must_be_between_8000_and_192000() {
    let s = spc(&[0x2F, 0xFE], ZERO, &[]);
    assert!(Machine::new(&s, 7999).is_none());
    assert!(Machine::new(&s, 8000).is_some());
    assert!(Machine::new(&s, 192_000).is_some());
    assert!(Machine::new(&s, 192_001).is_none());
    assert!(Machine::new(&s, 0).is_none());
    assert!(Machine::new(&s, -5).is_none());
}

#[test]
fn a_load_keeps_the_input_ports_even_when_the_control_byte_asks_for_a_reset() {
    // The control byte 0x31 has both reset bits set, and the file holds 5 and 6
    // at 0xF4 and 0xF5. The reset acts on a write, not on a load.
    let mut f = file(&[0xE4, 0xF4, 0xE4, 0xF5], ZERO, &[]);
    f[0x100 + 0xF1] = 0x31;
    f[0x100 + 0xF4] = 5;
    f[0x100 + 0xF5] = 6;
    let mut m = Machine::new(&Spc::parse(&f).unwrap(), 32000).unwrap();
    run_instructions(&mut m, 1);
    assert_eq!(m.cpu_state().1, 5);
    run_instructions(&mut m, 1);
    assert_eq!(m.cpu_state().1, 6);
    // The output ports start with the same bytes.
    assert_eq!((m.read_port(0), m.read_port(1)), (5, 6));
}
