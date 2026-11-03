//! The Famicom Disk System's expansion chip: its wavetable, volume envelope,
//! modulation unit and RAM.

mod common;

use common::*;
use oscilla_nes::Machine;

const SECOND: u64 = 1_789_773;

fn stores(list: &[(u16, u8)]) -> Vec<u8> {
    list.iter().flat_map(|&(a, v)| set(a, v)).collect()
}

/// The wave RAM filled with a square wave: 32 samples of 63 and 32 of 0.
fn square_wave() -> Vec<u8> {
    let mut init = set(0x4089, 0x80);
    for i in 0..64u16 {
        init.extend(set(0x4040 + i, if i < 32 { 63 } else { 0 }));
    }
    init
}

/// The square wave playing at `freq` at a fixed gain of 32, with the master
/// volume `master`.
fn playing(freq: u16, master: u8) -> Machine {
    let mut init = square_wave();
    init.extend(stores(&[
        (0x4089, master),
        (0x4080, 0x80 | 32),
        (0x4082, freq as u8),
        (0x4083, (freq >> 8) as u8),
    ]));
    fds_machine(&init, &[])
}

/// Samples the chip's level every `step` cycles for `cycles` cycles.
fn sample(m: &mut Machine, step: u64, cycles: u64) -> Vec<i32> {
    let mut v = Vec::new();
    let end = m.cycles() + cycles;
    while m.cycles() < end {
        m.run_cycles(step);
        v.push(m.expansion_levels()[0]);
    }
    v
}

#[test]
fn a_wave_plays_at_the_documented_pitch() {
    // 0x100 times 0x40 per update, 16 cycles apart: a turn of 64 samples takes
    // 2^24 / 0x4000 updates, which is 1024 updates or 16384 cycles: 109 Hz.
    let mut m = playing(0x100, 0);
    let levels = sample(&mut m, 16, SECOND);
    let edges = levels.windows(2).filter(|w| w[0] == 0 && w[1] > 0).count();
    assert!((108..=110).contains(&edges), "{edges}");
    assert_eq!(*levels.iter().max().unwrap(), 63 * 32);
}

#[test]
fn the_master_volume_scales_the_output() {
    for (master, want) in [(0u8, 2016), (1, 1344), (2, 1008), (3, 806)] {
        let mut m = playing(0x100, master);
        assert_eq!(
            *sample(&mut m, 16, 40_000).iter().max().unwrap(),
            want,
            "{master}"
        );
    }
}

#[test]
fn a_gain_above_32_is_clamped() {
    let mut init = square_wave();
    init.extend(stores(&[
        (0x4089, 0),
        (0x4080, 0x80 | 63),
        (0x4082, 0),
        (0x4083, 1),
    ]));
    let mut m = fds_machine(&init, &[]);
    assert_eq!(*sample(&mut m, 16, 40_000).iter().max().unwrap(), 63 * 32);
}

#[test]
fn the_wave_ram_is_protected_unless_it_is_enabled() {
    let mut init = set(0x4089, 0x80);
    init.extend(set(0x4040, 21));
    init.extend(set(0x4089, 0x00));
    init.extend(set(0x4040, 42));
    let mut m = fds_machine(&init, &[]);
    m.run_cycles(200);
    // Protected, a read returns the sample at the playback position, which is
    // the first one while the wave is halted.
    assert_eq!(m.peek(0x4040) & 0x3F, 21);
    assert_eq!(m.peek(0x4041) & 0x3F, 21);
}

#[test]
fn a_halted_wave_holds_the_first_sample() {
    let mut init = square_wave();
    init.extend(stores(&[
        (0x4089, 0),
        (0x4080, 0x80 | 32),
        (0x4082, 0),
        (0x4083, 0x81),
    ]));
    let mut m = fds_machine(&init, &[]);
    let levels = sample(&mut m, 16, 40_000);
    assert!(levels[60..].iter().all(|&l| l == 63 * 32));
}

#[test]
fn the_volume_envelope_climbs_to_32_and_stops() {
    // Increasing, speed 0, with the global speed at 1: a step every 16 cycles.
    let mut init = square_wave();
    init.extend(stores(&[
        (0x408A, 1),
        (0x4080, 0x80),
        (0x4080, 0x40),
        (0x4082, 0),
        (0x4083, 0),
    ]));
    let mut m = fds_machine(&init, &[]);
    m.run_cycles(600);
    let early = m.peek(0x4090) & 0x3F;
    assert!(early > 0 && early < 32, "{early}");
    m.run_cycles(3000);
    assert_eq!(m.peek(0x4090) & 0x3F, 32);
}

#[test]
fn the_volume_envelope_falls_to_zero() {
    let mut init = square_wave();
    init.extend(stores(&[
        (0x408A, 1),
        (0x4080, 0x80 | 20),
        (0x4080, 0x00),
        (0x4082, 0),
        (0x4083, 0),
    ]));
    let mut m = fds_machine(&init, &[]);
    m.run_cycles(3000);
    assert_eq!(m.peek(0x4090) & 0x3F, 0);
}

#[test]
fn the_modulation_counter_steps_through_its_table() {
    // A table of entries that add 1, a modulator that carries once in 16
    // updates, and a gain of 10.
    let mut init = stores(&[(0x4087, 0x80)]);
    for _ in 0..32 {
        init.extend(set(0x4088, 1));
    }
    init.extend(stores(&[
        (0x4085, 0),
        (0x4084, 0x80 | 10),
        (0x4086, 0),
        (0x4087, 0x01),
    ]));
    let mut m = fds_machine(&init, &[]);
    m.run_cycles(16 * 16 * 10 + 100);
    let counter = m.peek(0x4097);
    assert!((9..=11).contains(&counter), "{counter}");
}

#[test]
fn the_modulation_unit_moves_the_pitch() {
    // With the counter at +10 and a gain of 10 the pitch rises: the multiplier
    // is above 0x40, so a wave turns faster than without modulation.
    let mut plain = square_wave();
    plain.extend(stores(&[
        (0x4089, 0),
        (0x4080, 0x80 | 32),
        (0x4082, 0),
        (0x4083, 1),
    ]));
    let mut modulated = plain.clone();
    modulated.extend(stores(&[
        (0x4087, 0x80),
        (0x4084, 0x80 | 32),
        (0x4085, 0x30),
    ]));
    let count = |init: &[u8]| {
        let mut m = fds_machine(init, &[]);
        let levels = sample(&mut m, 16, SECOND / 4);
        levels.windows(2).filter(|w| w[0] == 0 && w[1] > 0).count()
    };
    assert!(count(&modulated) > count(&plain));
}

#[test]
fn the_ram_at_0x8000_takes_stores_and_holds_the_program() {
    // The file loads at 0xC000 in the tests below, so build one by hand.
    let mut f = file(&[], &[]);
    f[0x7B] = oscilla_nsf::chips::FDS;
    let mut init = set(0x8000, 0x5A);
    init.extend(set(0xDFFF, 0xA5));
    let f = {
        let mut g = file_with(&init, &[], 16666, 0, [0; 8], None);
        g[0x7B] = oscilla_nsf::chips::FDS;
        g
    };
    let mut m = Machine::new(&oscilla_nsf::Nsf::parse(&f).unwrap(), 48000).unwrap();
    m.run_cycles(300);
    assert_eq!(m.peek(0x8000), 0x5A);
    assert_eq!(m.peek(0xDFFF), 0xA5);
    // The init routine at 0x8000 was overwritten by the store, and the machine
    // keeps its copy of the program for a new song.
    m.select_song(0);
    assert_ne!(m.peek(0x8000), 0x5A);
}

#[test]
fn banks_are_copied_into_the_ram_and_switch_it() {
    let mut f = file_with(&[], &[], 16666, 0, [0, 1, 2, 3, 4, 5, 6, 7], None);
    f.truncate(0x80);
    for bank in 0..8u8 {
        f.extend(std::iter::repeat_n(0x20 + bank, 0x1000));
    }
    f[0x7B] = oscilla_nsf::chips::FDS;
    // The init routine sits in bank 0, which starts with a store sequence.
    let code = [set(0x5FF9, 7), set(0x5FF6, 3)].concat();
    f[0x80..0x80 + code.len()].copy_from_slice(&code);
    f[0x80 + code.len()] = 0x60;
    f[0x0A..0x0C].copy_from_slice(&0x8000u16.to_le_bytes());
    let mut m = Machine::new(&oscilla_nsf::Nsf::parse(&f).unwrap(), 48000).unwrap();
    assert_eq!(m.peek(0x9000), 0x21);
    assert_eq!(m.peek(0xD000), 0x25);
    m.run_cycles(300);
    assert_eq!(m.peek(0x9000), 0x27, "window 1 now holds bank 7");
    assert_eq!(m.peek(0x6000), 0x23, "the RAM at 0x6000 holds bank 3");
}

#[test]
fn the_channel_mask_reaches_the_chip() {
    let mut a = playing(0x100, 0);
    let mut b = playing(0x100, 0);
    assert_eq!(b.channel_mask(), 0x3_FFFF_FFFF);
    b.set_channel_mask(0x3_FFFF_FFFF & !(1 << 19));
    assert!(a.set_highpass_hz(1.0));
    assert!(b.set_highpass_hz(1.0));
    let loud = render(&mut a, 24000);
    let quiet = render(&mut b, 24000);
    assert!(loud[2 * 12000..].iter().any(|&v| v.abs() > 1000));
    assert!(quiet[2 * 12000..].iter().all(|&v| v.abs() < 30));
    assert!(sample(&mut b, 16, 40_000).iter().any(|&l| l > 0));
}

#[test]
fn stores_to_the_registers_do_nothing_without_the_chip() {
    let mut init = square_wave();
    init.extend(stores(&[(0x4080, 0x80 | 32), (0x4083, 1)]));
    let mut m = machine(&init, &[]);
    m.run_cycles(20_000);
    assert_eq!(m.expansion_levels(), [0; 8]);
}

#[test]
fn the_observer_sees_the_registers() {
    let mut m = fds_machine(&set(0x4080, 0x9F), &[]);
    let seen = watch(&mut m);
    m.run_cycles(100);
    assert!(
        seen.borrow()
            .writes
            .iter()
            .any(|w| w.1 == 0x4080 && w.2 == 0x9F)
    );
}

#[test]
fn the_chip_is_not_reported_as_unsupported() {
    assert_eq!(fds_machine(&[], &[]).warnings_nul(), [0]);
}

#[test]
fn a_snapshot_continues_exactly() {
    let mut init = square_wave();
    init.extend(stores(&[(0x4087, 0x80)]));
    for i in 0..32u8 {
        init.extend(set(0x4088, i % 8));
    }
    init.extend(stores(&[
        (0x4089, 1),
        (0x408A, 0xE8),
        (0x4080, 0x40 | 4),
        (0x4084, 0x80 | 12),
        (0x4086, 0x20),
        (0x4087, 0x01),
        (0x4082, 0x80),
        (0x4083, 0x02),
    ]));
    let mut a = fds_machine(&init, &[]);
    render(&mut a, 7777);
    let snap = a.snapshot();
    let from_a = render(&mut a, 9600);
    let mut b = fds_machine(&init, &[]);
    b.restore(&snap).unwrap();
    assert_eq!(digest(&from_a), digest(&render(&mut b, 9600)));
}
