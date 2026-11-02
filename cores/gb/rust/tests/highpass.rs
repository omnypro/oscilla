//! The high-pass filter's cutoff: the range, the default, the decay it gives a
//! held level, and a change in the middle of a song.

mod common;

use common::*;
use oscilla_gb::{CONSOLE_HIGHPASS_HZ, MAX_HIGHPASS_HZ, MIN_HIGHPASS_HZ, Machine};

/// Sets the wave channel to a constant level at the top frequency, so that the
/// mix steps once and then holds.
fn held_level() -> Vec<u8> {
    let mut init = [set(0x26, 0x80), set(0x24, 0x77), set(0x25, 0xFF)].concat();
    for i in 0..16 {
        init.extend(set(0x30 + i, 0xFF));
    }
    init.extend(set(0x1A, 0x80));
    init.extend(set(0x1C, 0x20));
    init.extend(set(0x1D, 0xFF));
    init.extend(set(0x1E, 0x87));
    init
}

fn index_of_peak(s: &[i16]) -> usize {
    let mut best = 0;
    for i in 0..s.len() / 2 {
        if s[2 * i].abs() > s[2 * best].abs() {
            best = i;
        }
    }
    best
}

/// The ratio of the held level `2 * k` frames after the step to the level `k`
/// frames after it, past the resampler's ringing.
fn ratio_after(hz: Option<f64>, k: usize) -> f64 {
    let mut m = machine(&held_level(), &[]);
    if let Some(hz) = hz {
        assert!(m.set_highpass_hz(hz));
    }
    let out = render(&mut m, 48000 * 3);
    let p = index_of_peak(&out) + 16;
    f64::from(out[2 * (p + 2 * k)]) / f64::from(out[2 * (p + k)])
}

#[test]
fn the_console_filter_is_the_default() {
    let m = machine(&held_level(), &[]);
    assert_eq!(m.highpass_hz(), CONSOLE_HIGHPASS_HZ);
    assert!((CONSOLE_HIGHPASS_HZ - 28.04).abs() < 1e-9);
}

#[test]
fn the_cutoff_sets_the_decay_of_a_held_level() {
    for hz in [1.0, 5.0, 28.04, 100.0, 300.0, 1000.0] {
        let k = (48000.0 / (2.0 * std::f64::consts::PI * hz))
            .round()
            .max(4.0) as usize;
        let want = (-2.0 * std::f64::consts::PI * hz * k as f64 / 48000.0).exp();
        let got = ratio_after(Some(hz), k);
        assert!(
            (got - want).abs() < 0.03,
            "{hz} Hz: ratio {got:.4}, want {want:.4}"
        );
    }
}

#[test]
fn the_default_decays_like_the_console_cutoff() {
    let k = 1000;
    let want = (-2.0 * std::f64::consts::PI * CONSOLE_HIGHPASS_HZ * k as f64 / 48000.0).exp();
    let got = ratio_after(None, k);
    assert!((got - want).abs() < 0.01, "ratio {got:.4}, want {want:.4}");
}

#[test]
fn a_value_outside_the_range_is_refused() {
    let mut m = machine(&held_level(), &[]);
    for bad in [
        0.0,
        MIN_HIGHPASS_HZ - 0.01,
        MAX_HIGHPASS_HZ + 0.01,
        -50.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert!(!m.set_highpass_hz(bad), "{bad}");
        assert_eq!(m.highpass_hz(), CONSOLE_HIGHPASS_HZ);
    }
    assert!(m.set_highpass_hz(MIN_HIGHPASS_HZ));
    assert_eq!(m.highpass_hz(), MIN_HIGHPASS_HZ);
    assert!(m.set_highpass_hz(MAX_HIGHPASS_HZ));
    assert!(!m.set_highpass_hz(f64::NAN));
    assert_eq!(m.highpass_hz(), MAX_HIGHPASS_HZ);
}

#[test]
fn the_console_filter_comes_back_exactly() {
    let mut plain = machine(&held_level(), &[]);
    let mut changed = machine(&held_level(), &[]);
    assert!(changed.set_highpass_hz(400.0));
    assert_eq!(changed.highpass_hz(), 400.0);
    changed.use_console_highpass();
    assert_eq!(changed.highpass_hz(), CONSOLE_HIGHPASS_HZ);
    assert_eq!(
        digest(&render(&mut plain, 48000)),
        digest(&render(&mut changed, 48000))
    );
}

#[test]
fn a_change_during_play_leaves_the_output_continuous() {
    // At 2 Hz the held level lasts long enough to see the change.
    let mut m = machine(&held_level(), &[]);
    let mut slow = machine(&held_level(), &[]);
    assert!(m.set_highpass_hz(2.0));
    assert!(slow.set_highpass_hz(2.0));
    let before = render(&mut m, 2400);
    render(&mut slow, 2400);
    let held = i32::from(before[before.len() - 2]);
    assert!(held.abs() > 2000, "{held}");
    assert!(m.set_highpass_hz(100.0));
    let after = render(&mut m, 2400);
    let slow_after = render(&mut slow, 2400);
    let jump = (i32::from(after[0]) - held).abs();
    assert!(jump < 100, "jump {jump} from {held}");
    assert!(i32::from(slow_after[2 * 2000]).abs() > 1500);
    assert!(i32::from(after[2 * 2000]).abs() < 50, "{}", after[2 * 2000]);
}

#[test]
fn a_reset_and_a_restore_keep_the_choice() {
    let mut m = machine(&held_level(), &[]);
    assert!(m.set_highpass_hz(300.0));
    m.reset();
    assert_eq!(m.highpass_hz(), 300.0);
    m.select_song(0);
    assert_eq!(m.highpass_hz(), 300.0);
    render(&mut m, 4800);
    let snap = m.snapshot();
    assert!(m.set_highpass_hz(80.0));
    m.restore(&snap).unwrap();
    assert_eq!(m.highpass_hz(), 80.0);
}

#[test]
fn a_snapshot_taken_with_one_cutoff_plays_on_with_another() {
    let mut a = machine(&held_level(), &[]);
    assert!(a.set_highpass_hz(2.0));
    render(&mut a, 4800);
    let snap = a.snapshot();
    let mut b: Machine = machine(&held_level(), &[]);
    assert!(b.set_highpass_hz(500.0));
    b.restore(&snap).unwrap();
    let mut c = machine(&held_level(), &[]);
    assert!(c.set_highpass_hz(500.0));
    c.restore(&snap).unwrap();
    let from_b = render(&mut b, 9600);
    assert_eq!(digest(&from_b), digest(&render(&mut c, 9600)));
    assert_ne!(digest(&from_b), digest(&render(&mut a, 9600)));
}

#[test]
fn the_console_value_gives_the_console_filter() {
    let mut plain = machine(&held_level(), &[]);
    let mut set = machine(&held_level(), &[]);
    let read = set.highpass_hz();
    assert!(set.set_highpass_hz(read));
    assert_eq!(
        digest(&render(&mut plain, 96000)),
        digest(&render(&mut set, 96000))
    );
}
