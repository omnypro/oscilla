//! The player: how the machine calls the init and play routines, and what it
//! does with a driver that misbehaves.

mod common;

use common::*;
use oscilla_gb::{EndState, Machine, RestoreError};
use oscilla_gbs::Gbs;

const SECOND: u64 = 4_194_304;

fn power() -> Vec<u8> {
    [set(0x26, 0x80), set(0x24, 0x77), set(0x25, 0xFF)].concat()
}

#[test]
fn the_init_routine_runs_first_and_the_play_routine_at_the_vblank_rate() {
    let mut m = machine(&set(0x26, 0x80), &set(0x24, 0x55));
    let seen = watch(&mut m);
    m.run_cycles(SECOND / 2);
    let seen = seen.borrow();
    assert_eq!(seen.calls[0], (0, -1));
    assert_eq!(seen.calls[1], (70224, 0));
    assert_eq!(seen.calls[2], (2 * 70224, 1));
    let frames = seen.calls.len() as u64 - 1;
    assert_eq!(frames, m.frames());
    // The init store comes first and each play call stores once.
    assert_eq!(seen.writes[0].1, 0xFF26);
    assert_eq!(
        seen.writes.iter().filter(|w| w.1 == 0xFF24).count() as u64,
        frames
    );
}

#[test]
fn a_store_is_stamped_with_the_cycle_it_happens() {
    let mut m = machine(&set(0x26, 0x80), &[]);
    let seen = watch(&mut m);
    m.run_cycles(1000);
    // ld a,n takes 8 clocks and the store of ldh takes 12 clocks.
    assert_eq!(seen.borrow().writes[0], (20, 0xFF26, 0x80));
}

#[test]
fn the_song_number_is_in_a_when_init_starts() {
    // Init stores A to NR50, so the observer sees the song number.
    let mut m = machine(&[0xE0, 0x24], &[]);
    assert!(m.select_song(3));
    let seen = watch(&mut m);
    m.run_cycles(1000);
    assert_eq!(seen.borrow().writes[0].2, 3);
    assert!(!m.select_song(4));
    assert_eq!(m.song(), 3);
}

#[test]
fn the_timer_sets_the_rate_when_the_header_says_so() {
    // TAC = 4: 4096 Hz, TMA = 0xF0: 16 counts of 1024 clocks.
    let f = file(&[], &set(0x24, 0x55), 0xF0, 0x04, None);
    let mut m = Machine::new(&Gbs::parse(&f).unwrap(), 48000).unwrap();
    let seen = watch(&mut m);
    m.run_cycles(SECOND);
    let seen = seen.borrow();
    assert_eq!(seen.calls[1].0, 16384);
    assert_eq!(seen.calls[2].0 - seen.calls[1].0, 16384);
    assert!((255..=257).contains(&m.frames()), "{}", m.frames());
}

#[test]
fn a_driver_that_changes_the_timer_changes_the_rate() {
    // The first play call sets TMA to 0x00: 256 counts of 1024 clocks.
    let play = [set(0x06, 0x00), vec![0xAF]].concat();
    let f = file(&[], &play, 0xF0, 0x04, None);
    let mut m = Machine::new(&Gbs::parse(&f).unwrap(), 48000).unwrap();
    let seen = watch(&mut m);
    m.run_cycles(SECOND);
    let seen = seen.borrow();
    assert_eq!(seen.calls[2].0 - seen.calls[1].0, 262144);
}

#[test]
fn a_routine_that_runs_long_delays_the_next_call() {
    // 20,000 iterations of dec bc / jr take longer than a frame.
    let play = vec![0x01, 0x20, 0x4E, 0x0B, 0x78, 0xB1, 0x20, 0xFB];
    let mut m = machine(&[], &play);
    let seen = watch(&mut m);
    m.run_cycles(SECOND / 4);
    let seen = seen.borrow();
    assert!(seen.calls[2].0 - seen.calls[1].0 > 70224);
}

#[test]
fn a_routine_that_never_returns_stalls_the_machine() {
    let mut m = machine(&[], &[0x18, 0xFE]);
    m.run_cycles(SECOND + SECOND / 4);
    assert_eq!(m.end_state(), EndState::Stalled);
    let warnings = String::from_utf8_lossy(m.warnings_nul()).into_owned();
    assert!(warnings.contains("did not return"), "{warnings}");
}

#[test]
fn halt_in_a_routine_ends_it_with_a_warning() {
    let mut m = machine(&[], &[0x76]);
    m.run_cycles(SECOND / 10);
    assert_eq!(m.end_state(), EndState::Stalled);
    let warnings = String::from_utf8_lossy(m.warnings_nul()).into_owned();
    assert!(warnings.contains("HALT"), "{warnings}");
    // The player goes on calling the routine.
    assert!(m.frames() > 3);
}

#[test]
fn an_opcode_that_locks_the_cpu_is_reported() {
    let mut m = machine(&[], &[0xDD]);
    m.run_cycles(SECOND / 10);
    assert_eq!(m.end_state(), EndState::Stalled);
    let warnings = String::from_utf8_lossy(m.warnings_nul()).into_owned();
    assert!(warnings.contains("locks"), "{warnings}");
}

#[test]
fn the_warning_list_lives_at_one_address() {
    let mut m = machine(&[], &[0x76]);
    let before = m.warnings_nul().as_ptr();
    m.run_cycles(SECOND / 10);
    assert_eq!(m.warnings_nul().as_ptr(), before);
    m.reset();
    assert_eq!(m.warnings_nul().as_ptr(), before);
    assert_eq!(m.warnings_nul(), [0]);
}

#[test]
fn writing_to_the_bank_register_switches_the_rom_page() {
    // The marker sits at the start of page 2, linear address 0x8000.
    let marker = 0x8000 - usize::from(LOAD);
    let init = [
        vec![0x3E, 0x02, 0xEA, 0x00, 0x20],
        vec![0xFA, 0x00, 0x40, 0xE0, 0x24],
    ]
    .concat();
    let f = file(&init, &[], 0, 0, Some((marker, &[0x5A])));
    let mut m = Machine::new(&Gbs::parse(&f).unwrap(), 48000).unwrap();
    let seen = watch(&mut m);
    m.run_cycles(1000);
    assert_eq!(seen.borrow().writes[0].2, 0x5A);
    assert_eq!(m.peek(0x4000), 0x5A);
}

#[test]
fn rst_goes_to_the_load_address_plus_the_vector() {
    // Init is rst 08; the code at LOAD + 8 stores 0x55.
    let init = vec![0xCF];
    let f = file(&init, &[], 0, 0, Some((8, &[0x3E, 0x55, 0xE0, 0x24, 0xC9])));
    let mut m = Machine::new(&Gbs::parse(&f).unwrap(), 48000).unwrap();
    let seen = watch(&mut m);
    m.run_cycles(1000);
    assert_eq!(seen.borrow().writes[0].2, 0x55);
}

#[test]
fn ram_is_zero_and_ram_banking_registers_are_ignored() {
    let init = [
        vec![0x3E, 0x77, 0xEA, 0x00, 0xC0],
        vec![0xFA, 0x00, 0xC0, 0xE0, 0x24],
        vec![0xFA, 0x01, 0xC0, 0xE0, 0x25],
        vec![0x3E, 0x03, 0xE0, 0x70],
    ]
    .concat();
    let mut m = machine(&init, &[]);
    let seen = watch(&mut m);
    m.run_cycles(1000);
    let s = seen.borrow();
    assert_eq!(s.writes[0].2, 0x77);
    assert_eq!(s.writes[1].2, 0x00);
    assert_eq!(m.peek(0xFF70), 0x00);
}

#[test]
fn the_same_song_renders_the_same_samples() {
    let init = [
        power(),
        set(0x16, 0x80),
        set(0x17, 0xF3),
        set(0x18, 0x40),
        set(0x19, 0x86),
    ]
    .concat();
    let a = render(&mut machine(&init, &[]), 20000);
    let b = render(&mut machine(&init, &[]), 20000);
    assert_eq!(a, b);
    let mut m = machine(&init, &[]);
    m.reset();
    assert_eq!(render(&mut m, 20000), a);
}

#[test]
fn the_output_is_the_same_however_the_calls_are_cut() {
    let init = [power(), set(0x16, 0x80), set(0x17, 0xF0), set(0x19, 0x87)].concat();
    let whole = render(&mut machine(&init, &[]), 30000);
    let mut m = machine(&init, &[]);
    let mut pieces = Vec::new();
    for n in [1usize, 7, 4095, 4096, 4097, 123, 17581] {
        pieces.extend(render(&mut m, n));
    }
    assert_eq!(pieces, whole);
}

#[test]
fn float_output_is_the_16_bit_output_at_three_quarters_scale() {
    let init = [power(), set(0x16, 0x80), set(0x17, 0xF0), set(0x19, 0x87)].concat();
    let s = render(&mut machine(&init, &[]), 20000);
    let mut f = vec![0f32; 40000];
    machine(&init, &[]).render_f32(&mut f);
    for (a, b) in s.iter().zip(&f) {
        let want = f32::from(*a) / 32768.0;
        assert!((want - b).abs() < 2.0e-4, "{want} {b}");
    }
}

#[test]
fn a_snapshot_restores_the_machine() {
    let init = [power(), set(0x16, 0x80), set(0x17, 0xF3), set(0x19, 0x86)].concat();
    let play = [set(0x18, 0x10), set(0x19, 0x86)].concat();
    let mut m = machine(&init, &play);
    render(&mut m, 5000);
    let snap = m.snapshot();
    let a = render(&mut m, 30000);
    m.restore(&snap).unwrap();
    let b = render(&mut m, 30000);
    assert_eq!(a, b);
    // A second machine restores it too.
    let mut other = machine(&init, &play);
    other.restore(&snap).unwrap();
    assert_eq!(render(&mut other, 30000), a);
}

#[test]
fn a_snapshot_checks_the_rate_and_the_file() {
    let init = power();
    let m = machine(&init, &[]);
    let snap = m.snapshot();
    let mut rate = Machine::new(&gbs(&init, &[]), 44100).unwrap();
    assert_eq!(rate.restore(&snap), Err(RestoreError::Rate));
    let mut file = machine(&set(0x26, 0x80), &[]);
    assert_eq!(file.restore(&snap), Err(RestoreError::Image));
}

#[test]
fn a_restore_leaves_the_observer_and_the_mask_alone() {
    let mut m = machine(&power(), &set(0x24, 0x44));
    m.set_channel_mask(0b0101);
    let seen = watch(&mut m);
    let snap = m.snapshot();
    m.run_cycles(SECOND / 5);
    m.restore(&snap).unwrap();
    assert_eq!(m.channel_mask(), 0b0101);
    let before = seen.borrow().writes.len();
    m.run_cycles(SECOND / 5);
    assert!(seen.borrow().writes.len() > before);
}

#[test]
fn the_end_state_follows_the_channels() {
    let mut m = machine(&power(), &[]);
    m.run_cycles(SECOND / 10);
    assert_eq!(m.end_state(), EndState::ChannelsOff);
    let init = [power(), set(0x17, 0xF0), set(0x19, 0x87)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(SECOND / 10);
    assert_eq!(m.end_state(), EndState::Running);
}

#[test]
fn quiet_cycles_count_the_silence() {
    let mut m = machine(&power(), &[]);
    m.run_cycles(SECOND);
    assert!(m.quiet_cycles() > SECOND / 2, "{}", m.quiet_cycles());
    let init = [power(), set(0x17, 0xF0), set(0x18, 0x00), set(0x19, 0x87)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(SECOND / 10);
    assert!(m.quiet_cycles() < SECOND / 100, "{}", m.quiet_cycles());
}

#[test]
fn the_rate_must_be_in_range() {
    let g = gbs(&[], &[]);
    assert!(Machine::new(&g, 7999).is_none());
    assert!(Machine::new(&g, 192001).is_none());
    assert!(Machine::new(&g, 8000).is_some());
    assert!(Machine::new(&g, 192000).is_some());
}

#[test]
fn a_zero_length_render_is_fine() {
    let mut m = machine(&[], &[]);
    let mut none: [i16; 0] = [];
    m.render_s16(&mut none);
    assert_eq!(m.cycles(), 0);
}

#[test]
fn the_timer_registers_count_from_the_play_call() {
    // TAC = 4, TMA = 0xF0. The play routine stores TIMA to NR50 so the
    // observer sees it; just after a call the timer has barely moved.
    let play = vec![0xF0, 0x05, 0xE0, 0x24];
    let f = file(&[], &play, 0xF0, 0x04, None);
    let mut m = Machine::new(&Gbs::parse(&f).unwrap(), 48000).unwrap();
    let seen = watch(&mut m);
    m.run_cycles(SECOND / 4);
    let s = seen.borrow();
    assert!(s.writes.iter().all(|w| w.2 == 0xF0), "{:?}", &s.writes[..4]);
}

#[test]
fn peek_shows_the_registers_the_cpu_reads() {
    let f = file(&[], &[], 0xF0, 0x05, None);
    let mut m = Machine::new(&Gbs::parse(&f).unwrap(), 48000).unwrap();
    m.run_cycles(SECOND / 100);
    assert_eq!(m.peek(0xFF06), 0xF0);
    assert_eq!(m.peek(0xFF07), 0xFD);
    let tima = m.peek(0xFF05);
    assert!(tima >= 0xF0, "{tima:#x}");
    assert!(m.peek(0xFF44) < 154);
    assert_eq!(m.peek(0xFEA0), 0xFF);
    assert_eq!(m.peek(0xFF00), 0xFF);
    // DIV moves with time.
    let before = m.peek(0xFF04);
    m.run_cycles(SECOND / 10);
    assert_ne!(m.peek(0xFF04), before);
}

#[test]
fn a_restore_into_a_machine_with_another_mask_mixes_with_that_mask() {
    let init = [power(), set(0x16, 0x80), set(0x17, 0xF0), set(0x19, 0x87)].concat();
    let mut a = machine(&init, &[]);
    render(&mut a, 4000);
    let snap = a.snapshot();

    let mut b = machine(&init, &[]);
    b.set_channel_mask(0);
    b.restore(&snap).unwrap();
    let restored = render(&mut b, 4000);

    let mut c = machine(&init, &[]);
    c.restore(&snap).unwrap();
    c.set_channel_mask(0);
    assert_eq!(restored, render(&mut c, 4000));
}

#[test]
fn a_wave_channel_starts_from_silence() {
    // Full-scale wave RAM, the channel on and triggered: until the first
    // sample is read the channel outputs 0.
    let mut init = power();
    for i in 0..16u8 {
        init.extend(set(0x30 + i, 0xFF));
    }
    init.extend(
        [
            set(0x1A, 0x80),
            set(0x1C, 0x20),
            set(0x1D, 0x00),
            set(0x1E, 0x80),
        ]
        .concat(),
    );
    let mut m = machine(&init, &[]);
    m.run_cycles(400);
    assert_eq!(m.channel_levels()[2], 0);
    m.run_cycles(20000);
    assert_eq!(m.channel_levels()[2], 15);
}

#[test]
fn an_idle_unit_costs_next_to_nothing() {
    // No channel has been triggered, so no generator runs.
    let mut m = machine(&power(), &[]);
    let start = std::time::Instant::now();
    m.run_cycles(60 * SECOND);
    assert!(start.elapsed().as_millis() < 500, "{:?}", start.elapsed());
}

#[test]
fn a_first_song_past_the_count_is_clamped() {
    let mut g = gbs(&[], &set(0x24, 0x33));
    g.first_song = 200;
    let mut m = Machine::new(&g, 48000).unwrap();
    assert_eq!(m.song(), 3);
    let seen = watch(&mut m);
    m.run_cycles(SECOND / 10);
    assert_eq!(seen.borrow().calls[0], (0, -1));
}

#[test]
fn peek_agrees_with_the_cpu_on_unmapped_registers() {
    // ldh a,[0xFF08]; ld [0xC000],a
    let init = [0xF0, 0x08, 0xEA, 0x00, 0xC0];
    let mut m = machine(&init, &[]);
    m.run_cycles(1000);
    assert_eq!(m.peek(0xC000), 0xFF);
    for a in [0xFF00, 0xFF08, 0xFF0A, 0xFF0E] {
        assert_eq!(m.peek(a), 0xFF, "{a:04X}");
    }
}
