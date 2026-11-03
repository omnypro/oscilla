//! The player: how a song starts, when the play routine is called, how the
//! program is mapped, and what a snapshot keeps.

mod common;

use common::*;
use oscilla_nes::{EndState, Machine, Region, RestoreError};
use oscilla_nsf::Nsf;

#[test]
fn init_gets_the_song_in_a_and_the_region_in_x() {
    // STA $00; STX $01; STY $02.
    let init = [0x85, 0x00, 0x86, 0x01, 0x84, 0x02];
    let mut m = machine(&init, &[]);
    assert!(m.select_song(3));
    m.run_cycles(100);
    assert_eq!((m.peek(0), m.peek(1), m.peek(2)), (3, 0, 0));
    let pal = Machine::new_for(&nsf(&init, &[]), 48000, Region::Pal).unwrap();
    let mut pal = pal;
    pal.run_cycles(100);
    assert_eq!(pal.peek(1), 1);
    assert_eq!(pal.region(), Region::Pal);
}

#[test]
fn a_song_that_does_not_exist_is_refused() {
    let mut m = machine(&[], &[]);
    assert!(!m.select_song(4));
    assert!(m.select_song(3));
    assert_eq!(m.song(), 3);
    assert_eq!(m.song_count(), 4);
}

#[test]
fn memory_is_clear_when_a_song_starts() {
    // Every song stores a byte in RAM and in the 8 KiB at $6000.
    let init = [set(0x0010, 0x77), set(0x6000, 0x55)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(100);
    assert_eq!((m.peek(0x0010), m.peek(0x6000)), (0x77, 0x55));
    m.select_song(1);
    assert_eq!((m.peek(0x0010), m.peek(0x6000)), (0, 0));
}

#[test]
fn the_play_routine_is_called_on_a_fixed_schedule() {
    let mut m = machine(&[], &[0xE6, 0x00]);
    let seen = watch(&mut m);
    m.run_cycles(2_000_000);
    let s = seen.borrow();
    assert_eq!(s.calls[0], (0, -1));
    // Call k lands k intervals after the start, rounded down.
    for (cycle, frame) in &s.calls[1..] {
        let k = *frame as u128 + 1;
        let want = k * 16666 * 19_687_500 / (11 * 1_000_000);
        assert_eq!(u128::from(*cycle), want, "frame {frame}");
    }
    assert!(s.calls.len() > 60);
    assert_eq!(m.peek(0), (s.calls.len() - 1) as u8);
}

#[test]
fn the_schedule_does_not_drift() {
    let mut m = machine(&[], &[]);
    let seen = watch(&mut m);
    m.run_cycles(1_789_773 * 100);
    let s = seen.borrow();
    let last = s.calls.last().unwrap();
    let want = (last.1 as u128 + 1) * 16666 * 19_687_500 / (11 * 1_000_000);
    assert_eq!(u128::from(last.0), want);
}

#[test]
fn a_pal_machine_follows_the_pal_clock_and_rate() {
    let n = nsf(&[], &[]);
    let mut m = Machine::new_for(&n, 48000, Region::Pal).unwrap();
    let seen = watch(&mut m);
    m.run_cycles(1_000_000);
    let s = seen.borrow();
    let want = 20000u128 * 53_203_425 / (32 * 1_000_000);
    assert_eq!(u128::from(s.calls[1].0), want);
}

#[test]
fn a_header_names_the_region_unless_it_is_both() {
    let pal = Nsf::parse(&file_with(&[], &[], 16666, 1, [0; 8], None)).unwrap();
    assert_eq!(Machine::new(&pal, 48000).unwrap().region(), Region::Pal);
    let both = Nsf::parse(&file_with(&[], &[], 16666, 3, [0; 8], None)).unwrap();
    assert_eq!(Machine::new(&both, 48000).unwrap().region(), Region::Ntsc);
}

#[test]
fn the_observer_sees_the_drivers_stores_and_not_the_players() {
    let init = set(0x4000, 0x3F);
    let mut m = machine(&init, &set(0x4001, 0x08));
    let seen = watch(&mut m);
    m.run_cycles(100_000);
    let s = seen.borrow();
    assert_eq!(s.writes[0].1, 0x4000);
    assert_eq!(s.writes[0].2, 0x3F);
    assert_eq!(s.writes[1].1, 0x4001);
    assert!(s.writes.iter().all(|w| (0x4000..=0x4017).contains(&w.1)));
    assert_eq!(s.calls[0].1, -1);
    assert_eq!(s.calls[1].1, 0);
}

#[test]
fn a_routine_that_never_returns_stalls_the_song() {
    // JMP $8000.
    let mut m = machine(&[0x4C, 0x00, 0x80], &[]);
    m.run_cycles(1_789_773 * 3);
    assert_eq!(m.end_state(), EndState::Stalled);
    assert!(String::from_utf8_lossy(m.warnings_nul()).contains("did not return"));
}

#[test]
fn a_locked_cpu_stalls_the_song() {
    let mut m = machine(&[0x02], &[]);
    m.run_cycles(1000);
    assert_eq!(m.end_state(), EndState::Stalled);
}

#[test]
fn a_silent_song_has_its_channels_off() {
    let mut m = machine(&[], &[]);
    m.run_cycles(100_000);
    assert_eq!(m.end_state(), EndState::ChannelsOff);
    let mut loud = machine(&[set(0x4000, 0x3F), set(0x4003, 0x08)].concat(), &[]);
    loud.run_cycles(100_000);
    assert_eq!(loud.end_state(), EndState::Running);
}

#[test]
fn quiet_cycles_count_since_the_output_last_changed() {
    let init = [set(0x4000, 0x3F), set(0x4002, 0x20), set(0x4003, 0x00)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(200_000);
    assert!(m.quiet_cycles() < 10_000);
    let mut s = machine(&[], &[]);
    s.run_cycles(200_000);
    assert!(s.quiet_cycles() >= 190_000);
}

fn banked_file(banks: [u8; 8], load: u16) -> Vec<u8> {
    let mut f = file_with(&[], &[], 16666, 0, banks, None);
    f.truncate(0x80);
    f[8..10].copy_from_slice(&load.to_le_bytes());
    // Four banks of 4 KiB, each filled with its own number plus 0x10.
    for bank in 0..4u8 {
        f.extend(std::iter::repeat_n(0x10 + bank, 0x1000));
    }
    f
}

#[test]
fn a_banked_file_maps_the_banks_the_header_names() {
    let n = Nsf::parse(&banked_file([0, 1, 2, 3, 0, 1, 2, 3], 0x8000)).unwrap();
    let m = Machine::new(&n, 48000).unwrap();
    assert_eq!(m.peek(0x8000), 0x10);
    assert_eq!(m.peek(0x9000), 0x11);
    assert_eq!(m.peek(0xB000), 0x13);
    assert_eq!(m.peek(0xC000), 0x10);
    assert_eq!(m.peek(0xFFFF), 0x13);
}

#[test]
fn a_bank_register_store_moves_a_window() {
    let n = Nsf::parse(&banked_file([0, 1, 2, 3, 0, 1, 2, 3], 0x8000)).unwrap();
    let mut init = set(0x5FF8, 3);
    init.extend(set(0x5FFF, 0));
    let mut f = file_with(&init, &[], 16666, 0, [0, 1, 2, 3, 0, 1, 2, 3], None);
    f.truncate(0x80);
    f.extend_from_slice(&n.data);
    // Put the init routine in bank 1, which stays mapped at $9000.
    let mut code = vec![0u8; 0x1000];
    code[..init.len()].copy_from_slice(&init);
    code[init.len()] = 0x60;
    let at = 0x80 + 0x1000;
    f[at..at + 0x1000].copy_from_slice(&code);
    f[0x0A..0x0C].copy_from_slice(&0x9000u16.to_le_bytes());
    let mut m = Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap();
    assert_eq!(m.peek(0x8000), 0x10);
    m.run_cycles(100);
    assert_eq!(m.peek(0x8000), 0x13);
    assert_eq!(m.peek(0xF000), 0x10);
    // A new song starts from the header's banks again.
    m.select_song(0);
    assert_eq!(m.peek(0x8000), 0x10);
}

#[test]
fn the_load_address_of_a_banked_file_sets_the_padding() {
    let n = Nsf::parse(&banked_file([0, 1, 2, 3, 0, 0, 0, 0], 0x8800)).unwrap();
    let m = Machine::new(&n, 48000).unwrap();
    // The first 0x800 bytes of bank 0 are padding.
    assert_eq!(m.peek(0x8000), 0);
    assert_eq!(m.peek(0x87FF), 0);
    assert_eq!(m.peek(0x8800), 0x10);
}

#[test]
fn a_file_without_banks_sits_at_its_load_address() {
    let mut f = banked_file([0; 8], 0x9000);
    f.truncate(0x80 + 0x2000);
    let m = Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap();
    assert_eq!(m.peek(0x8FFF), 0);
    assert_eq!(m.peek(0x9000), 0x10);
    assert_eq!(m.peek(0xA000), 0x11);
    assert_eq!(m.peek(0xB000), 0);
}

#[test]
fn a_store_to_the_bank_registers_does_nothing_without_banks() {
    let init = set(0x5FF8, 1);
    let mut m = machine(&init, &[]);
    let before = m.peek(0x8000);
    m.run_cycles(100);
    assert_eq!(m.peek(0x8000), before);
}

#[test]
fn an_unmapped_read_returns_the_last_bus_value() {
    // LDA $5000 reads the value that the previous cycle left: the high byte of
    // the address.
    let mut m = machine(&[0xAD, 0x00, 0x50, 0x85, 0x00], &[]);
    m.run_cycles(100);
    assert_eq!(m.peek(0x0000), 0x50);
}

#[test]
fn expansion_chips_are_reported() {
    let mut f = file(&[], &[]);
    f[0x7B] = oscilla_nsf::chips::VRC7;
    let m = Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap();
    assert!(String::from_utf8_lossy(m.warnings_nul()).contains("expansion"));
    assert!(machine(&[], &[]).warnings_nul() == [0]);
}

#[test]
fn rates_out_of_range_make_no_machine() {
    let n = nsf(&[], &[]);
    assert!(Machine::new(&n, 7999).is_none());
    assert!(Machine::new(&n, 192001).is_none());
    assert!(Machine::new(&n, 8000).is_some());
}

#[test]
fn a_snapshot_continues_exactly() {
    let init = [set(0x4015, 0x0F), set(0x4000, 0x7F), set(0x4003, 0x08)].concat();
    let play = [0xEE, 0x02, 0x40, 0xE6, 0x00];
    let mut a = machine(&init, &play);
    render(&mut a, 7777);
    let snap = a.snapshot();
    let from_a = render(&mut a, 9600);
    let mut b = machine(&init, &play);
    b.restore(&snap).unwrap();
    assert_eq!(digest(&from_a), digest(&render(&mut b, 9600)));
    // Restoring in the same machine goes back too.
    a.restore(&snap).unwrap();
    assert_eq!(digest(&from_a), digest(&render(&mut a, 9600)));
}

#[test]
fn a_snapshot_refuses_another_file_or_rate() {
    let a = machine(&set(0x4000, 1), &[]);
    let snap = a.snapshot();
    let mut other = machine(&set(0x4000, 2), &[]);
    assert_eq!(other.restore(&snap), Err(RestoreError::Image));
    let mut slow = Machine::new(&nsf(&set(0x4000, 1), &[]), 44100).unwrap();
    assert_eq!(slow.restore(&snap), Err(RestoreError::Rate));
    assert!(snap.size() > 10_000);
}

#[test]
fn the_channel_mask_and_the_observer_stay_with_the_machine_on_restore() {
    let mut m = machine(&set(0x4000, 1), &[]);
    let snap = m.snapshot();
    m.set_channel_mask(0b00101);
    let seen = watch(&mut m);
    m.restore(&snap).unwrap();
    assert_eq!(m.channel_mask(), 0b00101);
    m.run_cycles(100_000);
    assert!(!seen.borrow().calls.is_empty());
}

#[test]
fn a_reset_restarts_the_song() {
    let init = set(0x4000, 0x3F);
    let mut m = machine(&init, &[0xE6, 0x00]);
    m.run_cycles(500_000);
    assert!(m.frames() > 5);
    m.reset();
    assert_eq!(m.frames(), 0);
    assert_eq!(m.cycles(), 0);
    assert_eq!(m.peek(0), 0);
}

#[test]
fn rendering_with_floats_matches_the_16_bit_output_in_scale() {
    let init = [
        set(0x4015, 0x0F),
        set(0x4000, 0xBF),
        set(0x4002, 0x80),
        set(0x4003, 0x08),
    ]
    .concat();
    let mut a = machine(&init, &[]);
    let mut b = machine(&init, &[]);
    let ints = render(&mut a, 20000);
    let mut floats = vec![0f32; 2 * 20000];
    b.render_f32(&mut floats);
    for (i, f) in ints.iter().zip(&floats) {
        assert!((f64::from(*i) / 32768.0 - f64::from(*f)).abs() < 1e-3);
    }
}

#[test]
fn a_hand_built_file_with_a_low_load_address_makes_a_machine() {
    let mut n = nsf(&[], &[]);
    n.load = 0x1000;
    assert!(Machine::new(&n, 48000).is_some());
}

#[test]
fn version_2_features_that_the_core_does_not_run_are_reported() {
    let mut f = file(&[], &[]);
    f[5] = 2;
    f[0x7C] = 32;
    let m = Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap();
    assert!(String::from_utf8_lossy(m.warnings_nul()).contains("non-returning"));
}

#[test]
fn a_file_that_never_calls_play_gets_no_play_calls() {
    let mut f = file(&[], &[0xE6, 0x00]);
    f[5] = 2;
    f[0x7C] = 64;
    let mut m = Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap();
    m.run_cycles(500_000);
    assert_eq!(m.frames(), 0);
    assert_eq!(m.peek(0), 0);
}

/// A looping sample at the fastest rate, so that the delta channel reads a byte
/// every 432 cycles while the play routine stores to the sound registers.
fn sample_player() -> Machine {
    let data = [0x55u8; 0x100];
    let init = [
        set(0x4010, 0x4F),
        set(0x4012, 0),
        set(0x4013, 0),
        set(0x4015, 0x1F),
        set(0x4000, 0xBF),
        set(0x4002, 0x80),
        set(0x4003, 0x08),
    ]
    .concat();
    let play = [0xE6, 0x00, 0xA5, 0x00, 0x8D, 0x02, 0x40];
    let f = file_with(&init, &play, 16666, 0, [0; 8], Some((0x4000, &data)));
    Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap()
}

#[test]
fn the_buffer_size_does_not_change_the_sound() {
    let mut whole = sample_player();
    let want = digest(&render(&mut whole, 60000));
    for chunk in [1usize, 7, 100, 333, 4097] {
        let mut m = sample_player();
        let mut got = Vec::new();
        while got.len() < 2 * 60000 {
            let n = chunk.min(60000 - got.len() / 2);
            got.extend(render(&mut m, n));
        }
        assert_eq!(digest(&got), want, "chunks of {chunk}");
    }
}

#[test]
fn a_sample_read_does_not_move_the_cycle_of_a_store() {
    // Every play call stores at the same distance from its start with the
    // delta channel reading bytes meanwhile.
    let mut m = sample_player();
    let seen = watch(&mut m);
    m.run_cycles(600_000);
    let s = seen.borrow();
    let mut gaps = std::collections::HashSet::new();
    for pair in s.calls.windows(2).skip(1) {
        let start = pair[0].0;
        let store = s
            .writes
            .iter()
            .find(|w| w.0 >= start && w.1 == 0x4002)
            .unwrap()
            .0;
        gaps.insert(store - start);
    }
    assert!(gaps.len() <= 2, "{gaps:?}");
}
