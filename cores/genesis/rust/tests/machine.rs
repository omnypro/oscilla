mod common;

use common::*;
use oscilla_genesis::{EndState, Machine, MachineError};

/// Tone 0 at period 0xFE (440 Hz) and full volume, then one second.
fn tone_stream() -> Vec<u8> {
    let mut s = vec![0x50, 0x8E, 0x50, 0x0F, 0x50, 0x90];
    s.extend(wait(44_100));
    s.push(0x66);
    s
}

#[test]
fn a_tone_plays_at_the_pitch_of_its_period() {
    let v = parse(&file(true, false, &tone_stream(), None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let out = left(&render(&mut m, 44_100));
    // The first samples ring from the step at time zero.
    let c = crossings(&out[200..]);
    assert!((c as i32 - 877).abs() <= 3, "{c}");
    let peak = out.iter().map(|s| i32::from(*s)).max().unwrap();
    assert!((7000..8600).contains(&peak), "{peak}");
}

#[test]
fn the_rendering_is_the_same_at_any_chunk_size() {
    let v = parse(&file(true, false, &tone_stream(), None));
    let whole = render(&mut Machine::new(&v, 48_000).unwrap(), 30_000);
    let mut m = Machine::new(&v, 48_000).unwrap();
    let mut parts = Vec::new();
    for n in [1usize, 7, 4096, 9999, 1, 15_896] {
        parts.extend(render(&mut m, n));
    }
    assert_eq!(whole, parts);
}

#[test]
fn float_and_integer_output_agree() {
    let v = parse(&file(true, false, &tone_stream(), None));
    let a = render(&mut Machine::new(&v, 44_100).unwrap(), 2000);
    let mut b = vec![0f32; 4000];
    Machine::new(&v, 44_100).unwrap().render_f32(&mut b);
    for (x, y) in a.iter().zip(&b) {
        assert!((f32::from(*x) / 32768.0 - y).abs() < 1.0 / 32768.0);
    }
}

#[test]
fn a_stream_without_a_loop_ends_in_silence() {
    let mut s = vec![0x50, 0x8E, 0x50, 0x0F, 0x50, 0x90];
    s.extend(wait(4410));
    s.push(0x66);
    let v = parse(&file(true, false, &s, None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let out = left(&render(&mut m, 20_000));
    assert_eq!(m.end_state(), EndState::Finished);
    assert!(out[..4000].iter().any(|s| *s != 0));
    assert!(out[6000..].iter().all(|s| *s == 0));
}

#[test]
fn a_loop_returns_to_its_point_and_counts_the_passes() {
    // The first pass sets the tone; the loop holds a wait and a volume change.
    let mut s = vec![0x50, 0x8E, 0x50, 0x0F];
    let at = s.len();
    s.extend([0x50, 0x90]);
    s.extend(wait(2205));
    s.extend([0x50, 0x9F]);
    s.extend(wait(2205));
    s.push(0x66);
    let v = parse(&file(true, false, &s, Some(at)));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let out = left(&render(&mut m, 22_050));
    assert_eq!(m.end_state(), EndState::Playing);
    assert_eq!(m.loops(), 4);
    assert!(out[100..2000].iter().any(|s| *s != 0));
    assert!(out[2300..4300].iter().all(|s| *s == 0));
    assert!(out[4500..6400].iter().any(|s| *s != 0));
}

#[test]
fn a_loop_with_no_wait_ends_the_stream_instead_of_spinning() {
    let s = vec![0x50, 0x90, 0x66];
    let v = parse(&file(true, false, &s, Some(0)));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    render(&mut m, 1000);
    assert_eq!(m.end_state(), EndState::Finished);
}

#[test]
fn the_volume_modifier_scales_the_output() {
    let plain = file(true, false, &tone_stream(), None);
    let mut loud = plain.clone();
    loud[0x7C] = 0x20;
    let a = left(&render(
        &mut Machine::new(&parse(&plain), 44_100).unwrap(),
        4000,
    ));
    let b = left(&render(
        &mut Machine::new(&parse(&loud), 44_100).unwrap(),
        4000,
    ));
    let (pa, pb) = (a.iter().max().unwrap(), b.iter().max().unwrap());
    assert!(
        (i32::from(*pb) - 2 * i32::from(*pa)).abs() <= 2,
        "{pa} {pb}"
    );
}

#[test]
fn the_mask_silences_a_channel() {
    let v = parse(&file(true, false, &tone_stream(), None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    m.set_psg_mask(0b1110);
    let out = render(&mut m, 4000);
    assert!(out.iter().all(|s| *s == 0));
}

#[test]
fn a_bad_rate_or_a_file_without_a_chip_is_refused() {
    let v = parse(&file(true, false, &tone_stream(), None));
    assert_eq!(Machine::new(&v, 7999).err(), Some(MachineError::Rate));
    assert_eq!(Machine::new(&v, 192_001).err(), Some(MachineError::Rate));
    let none = parse(&file(false, false, &[0x66], None));
    assert_eq!(
        Machine::new(&none, 44_100).err(),
        Some(MachineError::NoChip)
    );
}

/// Channel 1, operator 1 alone (algorithm 7) at A4, then `n` samples.
fn fm_stream(n: u16) -> Vec<u8> {
    fm_stream_at(n, 0)
}

/// The same stream with a total level of `tl` on the sounding operator.
fn fm_stream_at(n: u16, tl: u8) -> Vec<u8> {
    let mut s = Vec::new();
    let mut w = |reg: u8, val: u8| s.extend([0x52, reg, val]);
    for (offset, level) in [(0u8, tl), (4, 127), (8, 127), (12, 127)] {
        w(0x30 + offset, 0x01);
        w(0x40 + offset, level);
        w(0x50 + offset, 0x1F);
        w(0x60 + offset, 0);
        w(0x70 + offset, 0);
        w(0x80 + offset, 0x0F);
    }
    w(0xB0, 0x07);
    w(0xB4, 0xC0);
    w(0xA4, 4 << 3 | 4);
    w(0xA0, 0x3B);
    w(0x28, 0x10);
    s.extend(wait(n));
    s.push(0x66);
    s
}

#[test]
fn an_fm_tone_plays_at_the_pitch_of_its_f_number() {
    let v = parse(&file(false, true, &fm_stream(44_100), None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let out = left(&render(&mut m, 44_100));
    let c = crossings(&out[400..]);
    assert!((c as i32 - 877).abs() <= 4, "{c}");
    let peak = out.iter().map(|s| i32::from(*s)).max().unwrap();
    assert!((7000..=8700).contains(&peak), "{peak}");
}

#[test]
fn the_fm_mask_silences_the_chip() {
    let v = parse(&file(false, true, &fm_stream(44_100), None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    m.set_fm_mask(0);
    assert!(render(&mut m, 4000).iter().all(|s| *s == 0));
}

#[test]
fn both_chips_play_together() {
    let mut s = vec![0x50, 0x84, 0x50, 0x05, 0x50, 0x90];
    let fm = fm_stream_at(44_100, 40);
    s.extend(&fm);
    let v = parse(&file(true, true, &s, None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let both = left(&render(&mut m, 8000));
    let alone = left(&render(
        &mut Machine::new(&parse(&file(false, true, &fm, None)), 44_100).unwrap(),
        8000,
    ));
    assert_ne!(both, alone);
    let energy = |v: &[i16]| v.iter().map(|s| i64::from(*s) * i64::from(*s)).sum::<i64>();
    assert!(energy(&both) > energy(&alone));
}

#[test]
fn the_dac_commands_play_the_bank_bytes_on_channel_six() {
    // The bank holds 0x80 then 0xFF; each 0x80 command writes the next byte to register 0x2A.
    let mut s = vec![0x67, 0x66, 0x00, 2, 0, 0, 0, 0x80, 0xFF];
    s.extend([0x52, 0x2B, 0x80, 0x53, 0xB6, 0xC0]);
    s.push(0x80);
    s.extend(wait(2205));
    s.push(0x80);
    s.extend(wait(2205));
    s.push(0x66);
    let v = parse(&file(false, true, &s, None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let out = left(&render(&mut m, 4400));
    assert!(out[1000..2000].iter().all(|s| s.abs() <= 2));
    assert!(
        out[3500..4300]
            .iter()
            .all(|s| (i32::from(*s) - 10461).abs() <= 2)
    );
}

#[test]
fn the_high_pass_removes_the_dc_that_the_dac_leaves() {
    // The DAC sits at its lowest value for a second.
    let mut s = vec![0x52, 0x2B, 0x80, 0x52, 0x2A, 0x00];
    s.extend(wait(44_100));
    s.push(0x66);
    let v = parse(&file(false, true, &s, None));
    let mut off = Machine::new(&v, 44_100).unwrap();
    off.set_highpass_hz(0.0);
    let mut on = Machine::new(&v, 44_100).unwrap();
    assert_eq!(on.highpass_hz(), 20.0);
    let off = left(&render(&mut off, 40_000));
    let on = left(&render(&mut on, 40_000));
    assert!(off[30_000..].iter().all(|s| *s < -4000));
    assert!(on[30_000..].iter().all(|s| s.abs() <= 2), "{}", on[30_000]);
}

#[test]
fn the_high_pass_accepts_only_a_sane_cutoff() {
    let v = parse(&file(true, false, &tone_stream(), None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    assert!(m.set_highpass_hz(55.0));
    assert!(!m.set_highpass_hz(-1.0));
    assert!(!m.set_highpass_hz(1001.0));
    assert!(!m.set_highpass_hz(f64::NAN));
    assert_eq!(m.highpass_hz(), 55.0);
}
