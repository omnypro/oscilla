//! The VRC7 expansion chip: its registers, the envelope, the pitch, the low
//! frequency oscillators and the mix.

mod common;

use common::*;
use oscilla_nes::Machine;

/// Stores `v` in register `r` of the chip through its two ports.
fn reg(r: u8, v: u8) -> Vec<u8> {
    [set(0x9010, r), set(0x9030, v)].concat()
}

fn init_of(regs: &[(u8, u8)]) -> Vec<u8> {
    regs.iter().flat_map(|&(r, v)| reg(r, v)).collect()
}

fn with(regs: &[(u8, u8)]) -> Machine {
    vrc7_machine(&init_of(regs), &[])
}

/// A custom patch that makes a plain sine: the modulator is nearly silent and
/// the carrier has the multiplier 1, the sustain bit, an instant attack and a
/// decay and release that `dr_rr` and `sl` give.
fn sine(dr: u8, sl: u8, rr: u8) -> Vec<(u8, u8)> {
    vec![
        (0x00, 0x01),
        (0x01, 0x21),
        (0x02, 0x3F),
        (0x03, 0x00),
        (0x04, 0xF0),
        (0x05, 0xF0 | dr),
        (0x06, 0x00),
        (0x07, sl << 4 | rr),
    ]
}

/// The registers of channel 0 at 194 Hz: F-number 256 in octave 3.
fn note(on: bool) -> Vec<(u8, u8)> {
    vec![(0x30, 0x00), (0x10, 0x00), (0x20, u8::from(on) << 4 | 3 << 1 | 1)]
}

/// A busy loop of about 23 ms times `y` / 32.
fn delay(y: u8) -> Vec<u8> {
    vec![0xA0, y, 0xA2, 0xFF, 0xCA, 0xD0, 0xFD, 0x88, 0xD0, 0xF8]
}

fn left(m: &mut Machine, frames: usize) -> Vec<i32> {
    render(m, frames).iter().step_by(2).map(|&v| i32::from(v)).collect()
}

fn rms(v: &[i32]) -> f64 {
    (v.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

fn crossings(v: &[i32]) -> usize {
    v.windows(2).filter(|w| w[0] <= 0 && w[1] > 0).count()
}

fn playing(extra: &[(u8, u8)]) -> Machine {
    let mut regs = sine(0, 0, 0);
    regs.extend_from_slice(extra);
    regs.extend(note(true));
    let mut m = with(&regs);
    assert!(m.set_highpass_hz(1.0));
    m
}

#[test]
fn a_channel_plays_the_pitch_its_frequency_names() {
    // 49716 * 256 / 2^16 = 194.2 Hz.
    let mut m = playing(&[]);
    left(&mut m, 4800);
    let v = left(&mut m, 48000);
    let n = crossings(&v);
    assert!((193..=196).contains(&n), "{n}");
    assert!(rms(&v) > 500.0);
}

#[test]
fn a_higher_octave_doubles_the_pitch() {
    let mut regs = sine(0, 0, 0);
    regs.extend_from_slice(&[(0x30, 0), (0x10, 0), (0x20, 0x10 | 4 << 1 | 1)]);
    let mut m = with(&regs);
    assert!(m.set_highpass_hz(1.0));
    left(&mut m, 4800);
    let n = crossings(&left(&mut m, 48000));
    assert!((386..=391).contains(&n), "{n}");
}

#[test]
fn the_multiplier_scales_the_pitch() {
    let mut regs = sine(0, 0, 0);
    regs.push((0x01, 0x22));
    regs.extend(note(true));
    let mut m = with(&regs);
    assert!(m.set_highpass_hz(1.0));
    left(&mut m, 4800);
    let n = crossings(&left(&mut m, 48000));
    assert!((386..=391).contains(&n), "{n}");
}

#[test]
fn nothing_sounds_without_a_key() {
    let mut regs = sine(0, 0, 0);
    regs.extend(note(false));
    let mut m = with(&regs);
    assert!(m.set_highpass_hz(1.0));
    assert!(left(&mut m, 9600).iter().all(|&v| v.abs() < 8));
}

#[test]
fn the_volume_attenuates_in_steps_of_three_decibels() {
    let level = |vol: u8| {
        let mut regs = sine(0, 0, 0);
        regs.extend_from_slice(&[(0x30, vol), (0x10, 0), (0x20, 0x17)]);
        let mut m = with(&regs);
        assert!(m.set_highpass_hz(1.0));
        left(&mut m, 4800);
        rms(&left(&mut m, 24000))
    };
    let (a, b, c) = (level(0), level(2), level(4));
    // Two steps are 6 dB, a factor of 2.
    assert!((a / b - 2.0).abs() < 0.1, "{}", a / b);
    assert!((b / c - 2.0).abs() < 0.1, "{}", b / c);
}

#[test]
fn the_decay_takes_the_time_of_the_table() {
    // Octave 3 adds 1 to rate 32, which makes 33: the range of 47.6 dB takes
    // 163.49 / 1.25 = 130.8 ms. The windows are centred on 20 ms and 110 ms, so
    // the second is 90 ms later: 32.8 dB below the first.
    let mut m = playing(&[(0x05, 0xF8), (0x07, 0xF0)]);
    let window = |m: &mut Machine, skip: usize| {
        left(m, skip);
        rms(&left(m, 960))
    };
    let early = window(&mut m, 480);
    let late = window(&mut m, 3360);
    let db = 20.0 * (early / late).log10();
    assert!((db - 32.8).abs() < 2.0, "{db}");
}

#[test]
fn a_percussive_envelope_falls_through_the_sustain_level_to_silence() {
    // The sustain bit is clear: after the decay to 9 dB the release goes on.
    let mut m = playing(&[(0x01, 0x01), (0x05, 0xF8), (0x07, 0x38)]);
    let early = rms(&left(&mut m, 480));
    let end = left(&mut m, 480_000);
    assert!(early > 500.0);
    assert!(rms(&end[end.len() - 4800..]) < 8.0);
}

#[test]
fn a_sustained_envelope_holds_the_sustain_level() {
    // Sustain level 3 is 9 dB below the top.
    let mut m = playing(&[(0x05, 0xF8), (0x07, 0x30)]);
    left(&mut m, 24000);
    let a = rms(&left(&mut m, 4800));
    left(&mut m, 96000);
    let b = rms(&left(&mut m, 4800));
    assert!((a / b - 1.0).abs() < 0.05, "{a} {b}");
    let peak = rms(&left(&mut playing(&[]), 4800)[480..]);
    let db = 20.0 * (peak / a).log10();
    assert!((db - 9.0).abs() < 1.5, "{db}");
}

/// A machine that plays channel 0, waits about 47 ms and stores `off` in
/// register 0x20.
fn released(rr: u8, off: u8) -> Machine {
    let mut regs = sine(0, 0, rr);
    regs.extend(note(true));
    let mut init = init_of(&regs);
    init.extend(delay(64));
    init.extend(reg(0x20, off));
    let mut m = vrc7_machine(&init, &[]);
    assert!(m.set_highpass_hz(1.0));
    m
}

#[test]
fn a_key_off_starts_the_release() {
    let mut m = released(15, 3 << 1 | 1);
    let before = left(&mut m, 1200);
    assert!(rms(&before[600..]) > 500.0);
    left(&mut m, 4800);
    let after = left(&mut m, 4800);
    assert!(rms(&after) < 8.0);
}

#[test]
fn the_release_rate_sets_how_long_the_sound_lasts() {
    // Release 8 is rate 33: 130.8 ms for the whole range. After 100 ms the level
    // is 36 dB down and after 200 ms gone.
    let mut m = released(8, 3 << 1 | 1);
    left(&mut m, 2400);
    let a = rms(&left(&mut m, 480));
    left(&mut m, 4800);
    let b = rms(&left(&mut m, 480));
    assert!(a > 300.0 && b < a / 10.0, "{a} {b}");
}

#[test]
fn the_channel_sustain_bit_slows_the_release_to_rate_five() {
    // Rate 5 is rate 21 with the key scale: 1046 ms for the whole range.
    let mut m = released(15, 1 << 5 | 3 << 1 | 1);
    left(&mut m, 2400 + 9600);
    let slow = rms(&left(&mut m, 960));
    assert!(slow > 300.0, "{slow}");
    let mut m = released(15, 3 << 1 | 1);
    left(&mut m, 2400 + 9600);
    assert!(rms(&left(&mut m, 960)) < 8.0);
}

#[test]
fn a_zero_attack_never_sounds() {
    let mut m = playing(&[(0x05, 0x00)]);
    assert!(left(&mut m, 9600).iter().all(|&v| v.abs() < 8));
}

#[test]
fn an_instant_attack_reaches_full_level_in_a_millisecond() {
    let mut m = playing(&[]);
    let v = left(&mut m, 2400);
    assert!(v[200..].iter().map(|x| x.abs()).max().unwrap() > 3000);
}

#[test]
fn the_tremolo_swings_the_level_by_about_four_decibels() {
    let mut m = playing(&[(0x01, 0xA1)]);
    left(&mut m, 4800);
    let mut lo = f64::MAX;
    let mut hi = 0.0f64;
    for _ in 0..30 {
        let r = rms(&left(&mut m, 480));
        lo = lo.min(r);
        hi = hi.max(r);
    }
    let db = 20.0 * (hi / lo).log10();
    assert!((3.0..=5.5).contains(&db), "{db}");
    // Without the bit the level holds.
    let mut m = playing(&[]);
    left(&mut m, 4800);
    let a = rms(&left(&mut m, 480));
    left(&mut m, 4800);
    let b = rms(&left(&mut m, 480));
    assert!((a / b - 1.0).abs() < 0.02);
}

#[test]
fn the_vibrato_moves_the_pitch() {
    let mut m = playing(&[(0x01, 0x61)]);
    left(&mut m, 4800);
    let mut counts = Vec::new();
    for _ in 0..20 {
        counts.push(crossings(&left(&mut m, 2400)));
    }
    assert!(counts.iter().max() != counts.iter().min(), "{counts:?}");
}

#[test]
fn the_feedback_changes_the_sound() {
    let mut a = playing(&[(0x02, 0x00), (0x03, 0x00)]);
    let mut b = playing(&[(0x02, 0x00), (0x03, 0x07)]);
    left(&mut a, 4800);
    left(&mut b, 4800);
    let (x, y) = (left(&mut a, 4800), left(&mut b, 4800));
    assert_ne!(digest(&x.iter().map(|&v| v as i16).collect::<Vec<_>>()), digest(&y.iter().map(|&v| v as i16).collect::<Vec<_>>()));
}

#[test]
fn a_built_in_patch_sounds() {
    let mut m = with(&[(0x30, 0x10), (0x10, 0x00), (0x20, 0x17)]);
    assert!(m.set_highpass_hz(1.0));
    left(&mut m, 4800);
    assert!(rms(&left(&mut m, 4800)) > 100.0);
}

#[test]
fn the_reset_silences_the_chip_and_blocks_its_registers() {
    let mut regs = sine(0, 0, 0);
    regs.extend(note(true));
    let mut init = init_of(&regs);
    init.extend(set(0xE000, 0x80));
    let mut m = vrc7_machine(&init, &[]);
    assert!(m.set_highpass_hz(1.0));
    assert!(left(&mut m, 9600).iter().all(|&v| v.abs() < 8));
    // Writes during the reset are ignored.
    let mut init = set(0xE000, 0x80);
    init.extend(init_of(&regs));
    let mut m = vrc7_machine(&init, &[]);
    assert!(m.set_highpass_hz(1.0));
    assert!(left(&mut m, 9600).iter().all(|&v| v.abs() < 8));
    // Releasing the reset lets them through.
    let mut init = set(0xE000, 0x80);
    init.extend(set(0xE000, 0x00));
    init.extend(init_of(&regs));
    let mut m = vrc7_machine(&init, &[]);
    assert!(m.set_highpass_hz(1.0));
    left(&mut m, 4800);
    assert!(rms(&left(&mut m, 4800)) > 500.0);
}

#[test]
fn the_channels_are_independent() {
    let mut regs = sine(0, 0, 0);
    regs.extend_from_slice(&[(0x31, 0x00), (0x11, 0x00), (0x21, 0x10 | 4 << 1 | 1)]);
    let mut m = with(&regs);
    left(&mut m, 4800);
    let l = m.expansion_levels();
    assert_eq!(l[0], 0);
    let mut peak = [0i32; 8];
    for _ in 0..2000 {
        m.run_cycles(997);
        let l = m.expansion_levels();
        for i in 0..8 {
            peak[i] = peak[i].max(l[i]);
        }
    }
    assert!(peak[1] > 3000, "{peak:?}");
    assert_eq!(peak[0], 0);
    assert!(peak[2..].iter().all(|&v| v == 0));
}

#[test]
fn the_channel_mask_reaches_the_vrc7_channels() {
    let mut a = playing(&[]);
    let mut b = playing(&[]);
    assert_eq!(b.channel_mask(), 0x1FFFFFFF);
    b.set_channel_mask(!(1 << 23));
    left(&mut a, 4800);
    left(&mut b, 4800);
    assert!(rms(&left(&mut a, 4800)) > 500.0);
    assert!(rms(&left(&mut b, 4800)) < 8.0);
}

#[test]
fn stores_to_the_ports_do_nothing_without_the_chip() {
    let mut regs = sine(0, 0, 0);
    regs.extend(note(true));
    let mut m = machine(&init_of(&regs), &[]);
    m.run_cycles(20_000);
    assert_eq!(m.expansion_levels(), [0; 8]);
}

#[test]
fn the_chip_raises_no_warning() {
    let m = with(&[]);
    assert_eq!(m.warnings_nul(), [0]);
}

#[test]
fn the_output_does_not_depend_on_the_size_of_the_render_chunks() {
    let mut a = playing(&[(0x01, 0xE1)]);
    let mut b = playing(&[(0x01, 0xE1)]);
    let whole = render(&mut a, 6000);
    let mut parts = Vec::new();
    for n in [1, 7, 100, 333, 1, 2000, 3558] {
        parts.extend(render(&mut b, n));
    }
    assert_eq!(digest(&whole), digest(&parts));
}
