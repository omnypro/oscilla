//! The VRC6 expansion chip: its pulse channels, its saw, the frequency control
//! and the mix.

mod common;

use common::*;
use oscilla_nes::Machine;

const SECOND: u64 = 1_789_773;

fn init_of(stores: &[(u16, u8)]) -> Vec<u8> {
    stores.iter().flat_map(|&(a, v)| set(a, v)).collect()
}

fn with(stores: &[(u16, u8)]) -> Machine {
    vrc6_machine(&init_of(stores), &[])
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

#[test]
fn a_pulse_channel_plays_the_pitch_its_period_names() {
    // 1789773 / (16 * (255 + 1)) is 437 Hz.
    let mut m = with(&[(0x9000, 0x7F), (0x9001, 0xFF), (0x9002, 0x80)]);
    let levels = sample(&mut m, 0, 8, SECOND);
    let edges = rising_edges(&levels);
    assert!((436..=438).contains(&edges), "{edges}");
    assert_eq!(*levels.iter().max().unwrap(), 15);
}

#[test]
fn the_second_pulse_channel_has_its_own_registers() {
    let mut m = with(&[(0xA000, 0x7F), (0xA001, 0xFF), (0xA002, 0x80)]);
    let b = sample(&mut m, 1, 8, SECOND / 2);
    assert!(b.iter().any(|&l| l > 0));
    assert!(sample(&mut m, 0, 8, 10_000).iter().all(|&l| l == 0));
}

#[test]
fn the_duty_settings_give_one_sixteenth_more_each() {
    for duty in 0..8u8 {
        let mut m = with(&[(0x9000, duty << 4 | 0x0F), (0x9001, 0x40), (0x9002, 0x80)]);
        let levels = sample(&mut m, 0, 8, SECOND / 2);
        let high = levels.iter().filter(|&&l| l > 0).count() as f64 / levels.len() as f64;
        let want = f64::from(duty + 1) / 16.0;
        assert!((high - want).abs() < 0.01, "duty {duty}: {high}");
    }
}

#[test]
fn the_mode_bit_holds_the_channel_high() {
    let mut m = with(&[(0x9000, 0x8A), (0x9001, 0x40), (0x9002, 0x80)]);
    m.run_cycles(200);
    assert!(sample(&mut m, 0, 8, 50_000).iter().all(|&l| l == 10));
}

#[test]
fn a_disabled_channel_is_silent() {
    let mut m = with(&[(0x9000, 0x7F), (0x9001, 0x40), (0x9002, 0x00)]);
    assert!(sample(&mut m, 0, 8, 50_000).iter().all(|&l| l == 0));
}

#[test]
fn the_saw_climbs_in_six_steps_and_falls_back() {
    // Rate 8: the accumulator takes 8, 16 up to 48 and then resets, so the
    // top five bits run 1 to 6 and 0. A step comes every 2 * (50 + 1) cycles.
    let mut m = with(&[(0xB000, 0x08), (0xB001, 50), (0xB002, 0x80)]);
    let levels = sample(&mut m, 2, 8, 20_000);
    let mut distinct = vec![levels[0]];
    for &l in &levels[1..] {
        if l != *distinct.last().unwrap() {
            distinct.push(l);
        }
    }
    assert!(distinct.len() > 20);
    for pair in distinct.windows(2).skip(1) {
        let (a, b) = (pair[0], pair[1]);
        assert!(b == a + 1 || (a == 6 && b == 0), "{distinct:?}");
    }
}

#[test]
fn the_saw_plays_the_pitch_its_period_names() {
    // 1789773 / (14 * (63 + 1)) is 1997 Hz.
    let mut m = with(&[(0xB000, 0x08), (0xB001, 63), (0xB002, 0x80)]);
    let levels = sample(&mut m, 2, 4, SECOND / 2);
    let drops = levels.windows(2).filter(|w| w[0] == 6 && w[1] == 0).count();
    assert!((997..=1000).contains(&drops), "{drops}");
}

#[test]
fn a_disabled_saw_is_silent_and_starts_again_from_zero() {
    let mut m = with(&[(0xB000, 0x20), (0xB001, 20), (0xB002, 0x00)]);
    assert!(sample(&mut m, 2, 8, 20_000).iter().all(|&l| l == 0));
}

#[test]
fn the_frequency_control_shifts_every_period() {
    let base = [(0x9000, 0x7F), (0x9001, 0xFF), (0x9002, 0x80)];
    let slow = rising_edges(&sample(&mut with(&base), 0, 8, SECOND / 16));
    let mut fast_stores = vec![(0x9003, 0x02)];
    fast_stores.extend(base);
    let fast = rising_edges(&sample(&mut with(&fast_stores), 0, 8, SECOND / 16));
    let ratio = fast as f64 / slow as f64;
    assert!((15.0..17.0).contains(&ratio), "{slow} then {fast}");
}

#[test]
fn the_halt_bit_stops_every_oscillator() {
    let mut m = with(&[
        (0x9000, 0x7F),
        (0x9001, 0x40),
        (0x9002, 0x80),
        (0xB000, 0x10),
        (0xB001, 0x40),
        (0xB002, 0x80),
        (0x9003, 0x01),
    ]);
    m.run_cycles(1000);
    let held = m.expansion_levels();
    m.run_cycles(100_000);
    assert_eq!(m.expansion_levels(), held);
}

/// The distance between the two plateaus of a square wave: the mean of the
/// samples above zero less the mean of those below.
fn swing(m: &mut Machine) -> f64 {
    let out = render(m, 24000);
    let tail: Vec<f64> = out[2 * 12000..]
        .iter()
        .step_by(2)
        .map(|&v| f64::from(v))
        .collect();
    let mean = |side: Vec<&f64>| side.iter().copied().sum::<f64>() / side.len() as f64;
    mean(tail.iter().filter(|&&v| v > 0.0).collect())
        - mean(tail.iter().filter(|&&v| v < 0.0).collect())
}

#[test]
fn a_full_volume_pulse_is_nine_tenths_as_loud_as_one_of_the_2a03() {
    let mut vrc6 = with(&[(0x9000, 0x7F), (0x9001, 0xFF), (0x9002, 0x81)]);
    let mut own = machine(
        &init_of(&[(0x4015, 1), (0x4000, 0xBF), (0x4002, 0xFF), (0x4003, 0x09)]),
        &[],
    );
    assert!(vrc6.set_highpass_hz(1.0));
    assert!(own.set_highpass_hz(1.0));
    let ratio = swing(&mut vrc6) / swing(&mut own);
    assert!((ratio - 0.9).abs() < 0.05, "{ratio}");
}

#[test]
fn the_channel_mask_reaches_the_expansion_channels() {
    let stores = [(0x9000, 0x7F), (0x9001, 0xFF), (0x9002, 0x81)];
    let mut a = with(&stores);
    let mut b = with(&stores);
    assert_eq!(b.channel_mask(), 0x1FFFFFFF);
    b.set_channel_mask(!0x20);
    assert!(a.set_highpass_hz(1.0));
    assert!(b.set_highpass_hz(1.0));
    assert!(swing(&mut a) > 1000.0);
    let out = render(&mut b, 24000);
    assert!(out[2 * 12000..].iter().all(|&v| v.abs() < 30));
    assert!(sample(&mut b, 0, 8, 10_000).iter().any(|&l| l > 0));
}

#[test]
fn stores_to_the_registers_do_nothing_without_the_chip() {
    let init = init_of(&[(0x9000, 0x7F), (0x9001, 0xFF), (0x9002, 0x80)]);
    let mut m = machine(&init, &[]);
    m.run_cycles(10_000);
    assert_eq!(m.expansion_levels(), [0; 8]);
}

#[test]
fn the_observer_sees_the_chips_stores() {
    let mut m = with(&[(0x9000, 0x7F)]);
    let seen = watch(&mut m);
    m.run_cycles(100);
    assert!(
        seen.borrow()
            .writes
            .iter()
            .any(|w| w.1 == 0x9000 && w.2 == 0x7F)
    );
}

#[test]
fn only_the_other_chips_are_reported() {
    let m = with(&[]);
    assert_eq!(m.warnings_nul(), [0]);
    let mut f = file(&[], &[]);
    f[0x7B] = oscilla_nsf::chips::VRC6 | oscilla_nsf::chips::VT02;
    let m = Machine::new(&oscilla_nsf::Nsf::parse(&f).unwrap(), 48000).unwrap();
    assert!(String::from_utf8_lossy(m.warnings_nul()).contains("expansion"));
}

#[test]
fn a_snapshot_continues_exactly() {
    let init = init_of(&[
        (0x9000, 0x7F),
        (0x9001, 0x90),
        (0x9002, 0x81),
        (0xB000, 0x0C),
        (0xB001, 0x60),
        (0xB002, 0x81),
    ]);
    let mut a = vrc6_machine(&init, &[]);
    render(&mut a, 7777);
    let snap = a.snapshot();
    let from_a = render(&mut a, 9600);
    let mut b = vrc6_machine(&init, &[]);
    b.restore(&snap).unwrap();
    assert_eq!(digest(&from_a), digest(&render(&mut b, 9600)));
}
