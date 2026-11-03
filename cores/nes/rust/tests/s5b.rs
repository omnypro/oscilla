//! The Sunsoft 5B expansion chip: its tone generators, noise, envelope and
//! volume curve.

mod common;

use common::*;
use oscilla_nes::Machine;

const SECOND: u64 = 1_789_773;

fn regs(stores: &[(u8, u8)]) -> Vec<u8> {
    stores.iter().flat_map(|&(r, v)| s5b_reg(r, v)).collect()
}

fn with(stores: &[(u8, u8)]) -> Machine {
    s5b_machine(&regs(stores), &[])
}

/// Samples an expansion channel every `step` cycles for `cycles` cycles.
fn sample(m: &mut Machine, channel: usize, step: u64, cycles: u64) -> Vec<i32> {
    let mut v = Vec::new();
    let end = m.cycles() + cycles;
    while m.cycles() < end {
        m.run_cycles(step);
        v.push(m.expansion_levels()[channel]);
    }
    v
}

fn rising_edges(levels: &[i32]) -> usize {
    levels.windows(2).filter(|w| w[0] == 0 && w[1] > 0).count()
}

/// Tone A at full volume, no noise.
const TONE_A: [(u8, u8); 2] = [(7, 0b0011_1110), (8, 15)];

#[test]
fn a_tone_flips_every_sixteen_cycles_of_its_period() {
    // 1789773 / (32 * 100) is 559 Hz.
    let mut stores = vec![(0, 100), (1, 0)];
    stores.extend(TONE_A);
    let mut m = with(&stores);
    let levels = sample(&mut m, 0, 8, SECOND);
    let edges = rising_edges(&levels);
    assert!((558..=560).contains(&edges), "{edges}");
    assert_eq!(*levels.iter().max().unwrap(), 31);
}

#[test]
fn period_zero_plays_like_period_one() {
    let mut stores = vec![(0, 0), (1, 0)];
    stores.extend(TONE_A);
    let zero = rising_edges(&sample(&mut with(&stores), 0, 4, SECOND / 8));
    stores[0] = (0, 1);
    let one = rising_edges(&sample(&mut with(&stores), 0, 4, SECOND / 8));
    assert_eq!(zero, one);
    assert!(zero > 6000, "{zero}");
}

#[test]
fn each_four_bit_volume_step_is_two_levels() {
    for volume in 1..=15u8 {
        let mut m = with(&[(7, 0b0011_1110), (8, volume), (0, 100)]);
        let levels = sample(&mut m, 0, 16, 50_000);
        assert_eq!(*levels.iter().max().unwrap(), i32::from(volume) * 2 + 1);
    }
    let mut m = with(&[(7, 0b0011_1110), (8, 0), (0, 100)]);
    assert!(sample(&mut m, 0, 16, 50_000).iter().all(|&l| l == 0));
}

#[test]
fn a_channel_with_tone_and_noise_off_holds_its_volume() {
    let mut m = with(&[(7, 0xFF), (8, 9)]);
    m.run_cycles(200);
    assert!(sample(&mut m, 0, 8, 20_000).iter().all(|&l| l == 19));
}

#[test]
fn each_channel_has_its_own_period_and_volume() {
    let mut m = with(&[
        (7, 0b0011_1000),
        (0, 100),
        (2, 60),
        (4, 30),
        (8, 15),
        (9, 12),
        (10, 7),
    ]);
    let mut seen = [0i32; 3];
    for _ in 0..2000 {
        m.run_cycles(16);
        let l = m.expansion_levels();
        for c in 0..3 {
            seen[c] = seen[c].max(l[c]);
        }
    }
    assert_eq!(seen, [31, 25, 15]);
}

fn noise_reference(steps: u64) -> bool {
    let mut lfsr = 1u32;
    for _ in 0..steps {
        let feedback = (lfsr ^ (lfsr >> 3)) & 1;
        lfsr = lfsr >> 1 | feedback << 16;
    }
    lfsr & 1 != 0
}

#[test]
fn the_noise_generator_follows_its_shift_register() {
    // Period 3: a shift every 96 cycles. Tone and noise both gate channel A.
    let mut m = with(&[(6, 3), (7, 0b0011_0110), (8, 15)]);
    m.run_cycles(60);
    for k in 1..=300u64 {
        while m.cycles() < 96 * k + 20 {
            m.run_cycles(4);
        }
        let expect_noise = noise_reference(k);
        // The tone is off (bit 0 set in the mixer would disable it): with the
        // tone disabled the channel is the noise alone.
        let level = m.expansion_levels()[0];
        assert_eq!(level == 31, expect_noise, "step {k}");
    }
}

fn envelope_levels(shape: u8, steps: usize) -> Vec<i32> {
    // Period 10: a step every 160 cycles. Tone and noise are off, so channel A
    // follows the envelope.
    let mut m = with(&[(7, 0xFF), (8, 16), (0x0B, 10), (0x0C, 0), (0x0D, shape)]);
    let mut out = Vec::new();
    for k in 0..steps as u64 {
        while m.cycles() < 160 * k + 90 {
            m.run_cycles(4);
        }
        out.push(m.expansion_levels()[0]);
    }
    out
}

#[test]
fn the_envelope_shapes_follow_the_documented_ramps() {
    let down: Vec<i32> = (0..32).rev().collect();
    let up: Vec<i32> = (0..32).collect();
    let twice = |a: &Vec<i32>, b: &Vec<i32>| [a.clone(), b.clone()].concat();
    // 0x08: a repeating downward sawtooth.
    assert_eq!(envelope_levels(0x08, 64), twice(&down, &down));
    // 0x0C: a repeating upward sawtooth.
    assert_eq!(envelope_levels(0x0C, 64), twice(&up, &up));
    // 0x0A: down and up again; 0x0E: up and down.
    assert_eq!(envelope_levels(0x0A, 64), twice(&down, &up));
    assert_eq!(envelope_levels(0x0E, 64), twice(&up, &down));
    // 0x09: one downward ramp, then zero. 0x0B: then the maximum.
    let held = |ramp: &Vec<i32>, level: i32| [ramp.clone(), vec![level; 8]].concat();
    assert_eq!(envelope_levels(0x09, 40), held(&down, 0));
    assert_eq!(envelope_levels(0x0B, 40), held(&down, 31));
    // 0x0D: one upward ramp, then the maximum. 0x0F: then zero.
    assert_eq!(envelope_levels(0x0D, 40), held(&up, 31));
    assert_eq!(envelope_levels(0x0F, 40), held(&up, 0));
    // The shapes below 0x08 do one ramp and stop at zero.
    assert_eq!(envelope_levels(0x00, 40), held(&down, 0));
    assert_eq!(envelope_levels(0x04, 40), held(&up, 0));
}

#[test]
fn a_select_store_with_high_bits_blocks_the_data_port() {
    let mut m = s5b_machine(
        &[
            set(0xC000, 0x10),
            set(0xE000, 0xFF),
            set(0xC000, 0x07),
            set(0xE000, 0xFF),
            set(0xC000, 0x08),
            set(0xE000, 9),
        ]
        .concat(),
        &[],
    );
    m.run_cycles(200);
    // Register 7 was not written (blocked), and register 8 was written
    // through the port that the last select opened.
    assert!(
        sample(&mut m, 0, 8, 5000)
            .iter()
            .all(|&l| l == 0 || l == 19)
    );
}

#[test]
fn the_channel_mask_reaches_the_chips_channels() {
    let mut stores = vec![(0, 100), (1, 0)];
    stores.extend(TONE_A);
    let mut a = with(&stores);
    let mut b = with(&stores);
    assert_eq!(b.channel_mask(), 0x7FFFF);
    b.set_channel_mask(u32::MAX);
    assert_eq!(
        b.channel_mask(),
        0x7FFFF,
        "only the bits that name a channel count"
    );
    b.set_channel_mask(0x7FFFF & !0x100);
    assert!(a.set_highpass_hz(1.0));
    assert!(b.set_highpass_hz(1.0));
    let loud = render(&mut a, 24000);
    let quiet = render(&mut b, 24000);
    assert!(loud[2 * 12000..].iter().any(|&v| v.abs() > 1000));
    assert!(quiet[2 * 12000..].iter().all(|&v| v.abs() < 30));
    assert!(sample(&mut b, 0, 8, 10_000).iter().any(|&l| l > 0));
}

#[test]
fn a_full_channel_is_three_times_a_2a03_pulse_and_each_step_is_three_decibels() {
    let swing = |m: &mut Machine| {
        let out = render(m, 24000);
        let tail: Vec<f64> = out[2 * 12000..]
            .iter()
            .step_by(2)
            .map(|&v| f64::from(v))
            .collect();
        let mean = |side: Vec<&f64>| side.iter().copied().sum::<f64>() / side.len() as f64;
        mean(tail.iter().filter(|&&v| v > 0.0).collect())
            - mean(tail.iter().filter(|&&v| v < 0.0).collect())
    };
    let mut full = with(&[(7, 0b0011_1110), (8, 15), (0, 0xFF), (1, 0)]);
    let mut lower = with(&[(7, 0b0011_1110), (8, 9), (0, 0xFF), (1, 0)]);
    let mut pulse = machine(
        &[
            set(0x4015, 1),
            set(0x4000, 0xBF),
            set(0x4002, 0xFF),
            set(0x4003, 0x09),
        ]
        .concat(),
        &[],
    );
    for m in [&mut full, &mut lower, &mut pulse] {
        assert!(m.set_highpass_hz(1.0));
    }
    let (f, l, p) = (swing(&mut full), swing(&mut lower), swing(&mut pulse));
    assert!((f / p - 3.0).abs() < 0.3, "{}", f / p);
    // Six volume steps down: 0.708 to the sixth.
    assert!((l / f - 0.126).abs() < 0.01, "{}", l / f);
}

#[test]
fn stores_to_the_ports_do_nothing_without_the_chip() {
    let mut m = machine(&regs(&[(7, 0xFF), (8, 9)]), &[]);
    m.run_cycles(10_000);
    assert_eq!(m.expansion_levels(), [0; 8]);
}

#[test]
fn the_observer_sees_the_ports() {
    let mut m = with(&[(8, 9)]);
    let seen = watch(&mut m);
    m.run_cycles(100);
    let w = &seen.borrow().writes;
    assert!(w.iter().any(|x| x.1 == 0xC000 && x.2 == 8));
    assert!(w.iter().any(|x| x.1 == 0xE000 && x.2 == 9));
}

#[test]
fn the_chip_is_not_reported_as_unsupported() {
    assert_eq!(with(&[]).warnings_nul(), [0]);
}

#[test]
fn a_snapshot_continues_exactly() {
    let init = regs(&[
        (0, 100),
        (1, 0),
        (2, 70),
        (6, 5),
        (7, 0b0010_1000),
        (8, 12),
        (9, 0x10),
        (0x0B, 12),
        (0x0D, 0x0E),
    ]);
    let mut a = s5b_machine(&init, &[]);
    render(&mut a, 7777);
    let snap = a.snapshot();
    let from_a = render(&mut a, 9600);
    let mut b = s5b_machine(&init, &[]);
    b.restore(&snap).unwrap();
    assert_eq!(digest(&from_a), digest(&render(&mut b, 9600)));
}
