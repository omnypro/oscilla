//! The version 2 features: the interrupt timer and vector, the NMI that calls
//! the play routine, and an init routine that does not return.

mod common;

use common::*;
use oscilla_nes::{EndState, Machine};
use oscilla_nsf::nsf2;

const IRQ: u8 = nsf2::IRQ;
const NON_RETURNING: u8 = nsf2::NON_RETURNING_INIT;
const NO_PLAY: u8 = nsf2::NO_PLAY;

/// Cycles between two calls of the play routine at 16666 microseconds.
const FRAME: u64 = 29829;

/// `cli`.
const CLI: [u8; 1] = [0x58];

fn counted(machine: &mut Machine, cycles: u64) -> u8 {
    machine.run_cycles(cycles);
    machine.peek(0)
}

#[test]
fn the_timer_interrupts_once_in_each_period() {
    let init = [set_vector(), start_timer(1000), CLI.to_vec()].concat();
    let mut m = nsf2_machine(IRQ, &init, &[], &COUNT_ISR);
    let n = counted(&mut m, 200_000);
    // The timer started a few cycles into the song.
    let expected = (m.cycles() - 30) / 1000;
    assert!(
        (i64::from(n) - expected as i64).abs() <= 1,
        "{n} interrupts, about {expected} expected"
    );
}

#[test]
fn a_file_without_the_flags_has_no_timer() {
    let init = [set_vector(), start_timer(100), CLI.to_vec()].concat();
    let mut m = nsf2_machine(0, &init, &[], &COUNT_ISR);
    assert_eq!(counted(&mut m, 100_000), 0);
    // The registers read as open bus.
    let mut m = nsf2_machine(0, &set(0x401B, 7), &[], &[]);
    m.run_cycles(100);
    assert_eq!(m.peek(0x401B), 0);
}

#[test]
fn the_flag_alone_holds_the_interrupt_back_while_the_i_flag_is_set() {
    // The program never clears the I flag. It reads the timer's status later.
    let init = [
        set_vector(),
        start_timer(50),
        vec![0xEA; 200],
        vec![0xAD, 0x1D, 0x40, 0x85, 0x05],
    ]
    .concat();
    let mut m = nsf2_machine(IRQ, &init, &[], &COUNT_ISR);
    assert_eq!(counted(&mut m, 100_000), 0);
    assert_eq!(m.peek(5), 0x81, "the flag and the active bit");
}

#[test]
fn a_read_of_the_status_clears_the_flag() {
    let init = [
        start_timer(50),
        vec![0xEA; 200],
        vec![0xAD, 0x1D, 0x40, 0x85, 0x05, 0xAD, 0x1D, 0x40, 0x85, 0x06],
    ]
    .concat();
    let mut m = nsf2_machine(IRQ, &init, &[], &[]);
    m.run_cycles(2000);
    assert_eq!(m.peek(5), 0x81);
    // The next read comes a few cycles later, inside the same period.
    assert_eq!(m.peek(6), 0x01);
}

#[test]
fn the_reload_registers_read_back_and_a_stop_leaves_the_flag() {
    let init = [
        set(0x401B, 0x34),
        set(0x401C, 0x12),
        vec![0xAD, 0x1B, 0x40, 0x85, 0x05, 0xAD, 0x1C, 0x40, 0x85, 0x06],
        set(0x401D, 1),
        set(0x401D, 0),
        vec![0xAD, 0x1D, 0x40, 0x85, 0x07],
    ]
    .concat();
    let mut m = nsf2_machine(IRQ, &init, &[], &[]);
    m.run_cycles(100);
    assert_eq!((m.peek(5), m.peek(6)), (0x34, 0x12));
    assert_eq!(m.peek(7), 0, "stopped before the first expiry");
}

#[test]
fn an_interrupt_waits_one_instruction_after_cli() {
    // The flag is set long before the CLI. The instruction after the CLI runs
    // first, so the routine sees the increment.
    let init = [
        set_vector(),
        set(0x401B, 0),
        set(0x401C, 0),
        set(0x401D, 1),
        vec![0xEA; 4],
        CLI.to_vec(),
        vec![0xE6, 0x02, 0xEA, 0xEA],
    ]
    .concat();
    let isr = [
        vec![0xA5, 0x02, 0x85, 0x03],
        set(0x401D, 0),
        vec![0xAD, 0x1D, 0x40, 0x40],
    ]
    .concat();
    let mut m = nsf2_machine(IRQ, &init, &[], &isr);
    m.run_cycles(2000);
    assert_eq!(m.peek(3), 1);
}

#[test]
fn the_vector_is_ram_that_starts_with_the_programs_bytes() {
    let mut m = nsf2_with(IRQ, &set(0xFFFE, 0x99), &[], &[], &[0x34, 0x12]);
    assert_eq!((m.peek(0xFFFE), m.peek(0xFFFF)), (0x34, 0x12));
    m.run_cycles(100);
    assert_eq!((m.peek(0xFFFE), m.peek(0xFFFF)), (0x99, 0x12));
    // A reset puts the program's bytes back.
    m.reset();
    assert_eq!(m.peek(0xFFFE), 0x34);
    // Without the flags the bytes are the program's and stores change nothing.
    let mut plain = nsf2_with(0, &set(0xFFFE, 0x99), &[], &[], &[0x34, 0x12]);
    plain.run_cycles(100);
    assert_eq!(plain.peek(0xFFFE), 0x34);
}

#[test]
fn the_play_routine_is_called_by_nmi_with_the_i_flag_set() {
    // The routine stores the flags it sees, counts and ends with RTS.
    let play = vec![0x08, 0x68, 0x85, 0x01, 0xE6, 0x00];
    let mut m = nsf2_machine(IRQ, &[], &play, &[]);
    m.run_cycles(FRAME * 6 + FRAME / 2);
    assert_eq!(m.frames(), 6);
    assert_eq!(m.peek(0), 6);
    assert_eq!(m.peek(1) & 0x04, 0x04, "the I flag");
}

#[test]
fn the_registers_come_back_after_a_call_of_the_play_routine() {
    let init = vec![0xA9, 0x11, 0xA2, 0x22, 0xA0, 0x33];
    let play = vec![0xA9, 0xAA, 0xA2, 0xBB, 0xA0, 0xCC];
    let mut m = nsf2_machine(IRQ, &init, &play, &[]);
    m.run_cycles(FRAME * 4);
    assert!(m.frames() >= 3);
    assert_eq!(m.register(0), 0x11);
    assert_eq!(m.register(1), 0x22);
    assert_eq!(m.register(2), 0x33);
}

#[test]
fn a_timer_routine_and_the_play_routine_run_side_by_side() {
    let init = [set_vector(), start_timer(777), CLI.to_vec()].concat();
    let play = vec![0xE6, 0x01];
    let mut m = nsf2_machine(IRQ, &init, &play, &COUNT_ISR);
    m.run_cycles(150_000);
    assert!(m.peek(0) > 150, "{}", m.peek(0));
    assert_eq!(u64::from(m.peek(1)), m.frames());
    assert_ne!(m.end_state(), EndState::Stalled);
}

/// `sta $11; sty $12; inc $13; cpy #$81; beq loop; rts; loop: inc $10; jmp loop`
const NON_RETURNING_INIT: [u8; 15] = [
    0x85, 0x11, 0x84, 0x12, 0xE6, 0x13, 0xC0, 0x81, 0xF0, 0x01, 0x60, 0xE6, 0x10, 0x4C, 0x0B,
];

fn non_returning_init() -> Vec<u8> {
    let mut v = NON_RETURNING_INIT.to_vec();
    v.push(0x80);
    v
}

#[test]
fn a_non_returning_init_is_called_twice_and_play_arrives_as_nmi() {
    let play = vec![0xE6, 0x14];
    let mut m = nsf2_machine(NON_RETURNING, &non_returning_init(), &play, &[]);
    assert!(m.select_song(2));
    m.run_cycles(FRAME * 5 + 100);
    assert_eq!(m.peek(0x13), 2, "two calls");
    assert_eq!(m.peek(0x12), 0x81, "the second call has Y = $81");
    assert_eq!(m.peek(0x11), 2, "the song");
    assert!(m.peek(0x10) > 0, "the second call keeps running");
    assert_eq!(m.frames(), 5);
    assert_eq!(u64::from(m.peek(0x14)), m.frames());
    assert_ne!(m.end_state(), EndState::Stalled);
    assert!(!String::from_utf8_lossy(m.warnings_nul()).contains("second"));
}

#[test]
fn the_first_call_of_a_non_returning_init_has_y_80() {
    // `lda $12; bne done; sty $12; done: rts` keeps the first Y.
    let init = vec![0xA5, 0x12, 0xD0, 0x02, 0x84, 0x12];
    let mut m = nsf2_machine(NON_RETURNING, &init, &[], &[]);
    assert_eq!(m.register(2), 0x80);
    let seen = watch(&mut m);
    m.reset();
    m.run_cycles(200);
    assert_eq!(m.peek(0x12), 0x80);
    assert_eq!(seen.borrow().calls[0].1, -1);
}

#[test]
fn a_second_init_that_returns_leaves_the_play_routine_going() {
    let init = vec![0xE6, 0x13, 0x60];
    let play = vec![0xE6, 0x14];
    let mut m = nsf2_machine(NON_RETURNING, &init, &play, &[]);
    m.run_cycles(FRAME * 4 + 100);
    assert_eq!(m.peek(0x13), 2);
    assert_eq!(m.frames(), 4);
    assert_eq!(u64::from(m.peek(0x14)), m.frames());
}

#[test]
fn a_non_returning_init_with_no_play_is_never_interrupted() {
    let mut m = nsf2_machine(
        NON_RETURNING | NO_PLAY,
        &non_returning_init(),
        &[0xE6, 0x14],
        &[],
    );
    m.run_cycles(FRAME * 3);
    assert_eq!(m.frames(), 0);
    assert_eq!(m.peek(0x14), 0);
    assert!(m.peek(0x10) > 0);
}

#[test]
fn the_second_init_has_no_time_limit() {
    let mut m = nsf2_machine(NON_RETURNING, &non_returning_init(), &[], &[]);
    m.run_cycles(3_000_000);
    assert_ne!(m.end_state(), EndState::Stalled);
    assert!(!String::from_utf8_lossy(m.warnings_nul()).contains("second"));
}

#[test]
fn the_first_init_must_return() {
    // It loops on the first call.
    let mut m = nsf2_machine(NON_RETURNING, &[0x4C, 0x00, 0x80], &[], &[]);
    m.run_cycles(3_000_000);
    assert_eq!(m.end_state(), EndState::Stalled);
}

#[test]
fn a_play_routine_that_runs_long_delays_the_next_call() {
    // `ldy #0; again: ldx #0; wait: dex; bne wait; dey; bne again; inc $14`:
    // about 330000 cycles, eleven frames.
    let play = vec![
        0xA0, 0x00, 0xA2, 0x00, 0xCA, 0xD0, 0xFD, 0x88, 0xD0, 0xF8, 0xE6, 0x14,
    ];
    let mut m = nsf2_machine(IRQ, &[], &play, &[]);
    m.run_cycles(1_000_000);
    assert_eq!(
        m.frames(),
        3,
        "a call that is due waits for the one running"
    );
    assert_eq!(m.peek(0x14), 2);
}

#[test]
fn the_frame_counter_interrupt_reaches_the_cpu() {
    // The frame counter runs in its 4-step mode and raises its flag at the end
    // of each frame; the routine reads the status to clear it.
    let init = [set_vector(), set(0x4017, 0x00), CLI.to_vec()].concat();
    let isr = vec![0xE6, 0x00, 0xAD, 0x15, 0x40, 0x40];
    let mut m = nsf2_machine(IRQ, &init, &[], &isr);
    m.run_cycles(298_300);
    let n = m.peek(0);
    assert!((9..=11).contains(&n), "{n}");
}

#[test]
fn the_output_does_not_depend_on_the_size_of_the_chunks() {
    // The interrupt routine and the play routine store to the sound registers.
    let init = [
        set_vector(),
        set(0x4015, 0x0F),
        set(0x4000, 0xBF),
        set(0x4003, 0x08),
        start_timer(1234),
        CLI.to_vec(),
    ]
    .concat();
    let play = set(0x4001, 0x00);
    let isr = [
        vec![0xE6, 0x00, 0xA5, 0x00, 0x8D, 0x02, 0x40],
        vec![0xAD, 0x1D, 0x40, 0x40],
    ]
    .concat();
    let whole = {
        let mut m = nsf2_machine(IRQ, &init, &play, &isr);
        digest(&render(&mut m, 48000))
    };
    let pieces = {
        let mut m = nsf2_machine(IRQ, &init, &play, &isr);
        let mut all = Vec::new();
        for n in [1, 7, 333, 1000, 5, 4096, 77, 10000] {
            all.extend(render(&mut m, n));
        }
        let rest = 48000 - all.len() / 2;
        all.extend(render(&mut m, rest));
        digest(&all)
    };
    assert_eq!(whole, pieces);
}

#[test]
fn a_snapshot_across_a_pending_interrupt_plays_on_the_same() {
    let init = [set_vector(), start_timer(900), CLI.to_vec()].concat();
    let mut m = nsf2_machine(IRQ, &init, &[0xE6, 0x01], &COUNT_ISR);
    m.run_cycles(12_345);
    let snap = m.snapshot();
    let a = render(&mut m, 9600);
    let mut other = nsf2_machine(IRQ, &init, &[0xE6, 0x01], &COUNT_ISR);
    other.restore(&snap).unwrap();
    assert_eq!(digest(&render(&mut other, 9600)), digest(&a));
    assert_eq!(other.peek(0), m.peek(0));
    assert_eq!(other.frames(), m.frames());
}

#[test]
fn the_observer_sees_the_sound_stores_of_interrupt_routines() {
    let init = [set_vector(), start_timer(400), CLI.to_vec()].concat();
    let isr = [
        set(0x4001, 0x00),
        set(0x401D, 0),
        vec![0xAD, 0x1D, 0x40, 0x40],
    ]
    .concat();
    let mut m = nsf2_machine(IRQ, &init, &[], &isr);
    let seen = watch(&mut m);
    m.run_cycles(1000);
    let s = seen.borrow();
    assert!(s.writes.iter().any(|w| w.1 == 0x4001));
    assert!(
        s.writes.iter().all(|w| w.1 < 0x4018),
        "the timer's registers are not sound registers"
    );
    assert_eq!(s.calls[0].1, -1);
}
