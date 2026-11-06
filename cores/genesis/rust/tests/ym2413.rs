//! Files with a YM2413.

mod common;

use common::*;
use oscilla_genesis::{Machine, MachineError};
use oscilla_vgm::Vgm;

const CLOCK: u32 = 3_579_545;

/// A file with a YM2413 and, when `psg` is true, an SN76489. `second` marks
/// the YM2413 as dual (bit 30 of its clock).
fn opll_file(psg: bool, second: bool, stream: &[u8]) -> Vec<u8> {
    let mut f = file(psg, false, stream, None);
    let word = CLOCK | if second { 1 << 30 } else { 0 };
    f[0x10..0x14].copy_from_slice(&word.to_le_bytes());
    f
}

fn w(cmd: u8, register: u8, value: u8) -> [u8; 3] {
    [cmd, register, value]
}

/// A user tone that is one sine carrier (the modulator is off by its level)
/// at about 440 Hz on channel `ch`, keyed on, then `n` samples of waiting.
fn user_tone(cmd: u8, ch: u8, n: u16) -> Vec<u8> {
    let mut s = Vec::new();
    let tone = [0x21u8, 0x21, 0x3F, 0x00, 0xF0, 0xF0, 0x0F, 0x0F];
    for (i, v) in tone.iter().enumerate() {
        s.extend(w(cmd, i as u8, *v));
    }
    s.extend(w(cmd, 0x30 + ch, 0x00));
    s.extend(w(cmd, 0x10 + ch, 0x22));
    s.extend(w(cmd, 0x20 + ch, 0x19));
    s.extend(wait(n));
    s.push(0x66);
    s
}

fn machine(bytes: &[u8]) -> Machine {
    let mut m = Machine::new(&parse(bytes), 44_100).unwrap();
    m.set_highpass_hz(0.0);
    m
}

fn peak(samples: &[i16]) -> i32 {
    samples.iter().map(|s| i32::from(*s).abs()).max().unwrap()
}

#[test]
fn a_file_with_only_a_ym2413_plays() {
    let f = opll_file(false, false, &user_tone(0x51, 0, 22050));
    let mut m = Machine::new(&parse(&f), 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let out = left(&render(&mut m, 22050));
    assert!(peak(&out) > 500);
}

#[test]
fn a_user_tone_has_the_pitch_of_its_frequency_registers() {
    let f = opll_file(false, false, &user_tone(0x51, 4, 22050));
    let out = left(&render(&mut machine(&f), 22050));
    // Half a second of 440 Hz crosses zero 440 times.
    let n = crossings(&out[2000..]) as f64 * 22050.0 / (22050.0 - 2000.0);
    assert!((n - 440.0).abs() < 12.0, "{n}");
}

#[test]
fn the_mask_has_nine_bits_for_the_chip_and_mutes_it() {
    let f = opll_file(false, false, &user_tone(0x51, 0, 22050));
    let mut m = machine(&f);
    assert_eq!(m.channel_mask(), 0x3FF | 0x1FF << 20);
    m.set_channel_mask(0x3FF | 0x1FF << 20 & !1 << 20);
    assert_eq!(m.channel_mask(), 0x3FF | 0x1FE << 20);
    let out = render(&mut m, 22050);
    assert_eq!(peak(&out), 0);
    let mut single = machine(&file(true, true, &[0x66], None));
    single.set_channel_mask(u64::MAX);
    assert_eq!(single.channel_mask(), 0x3FF);
}

#[test]
fn a_built_in_instrument_sounds_and_differs_from_the_user_tone() {
    let user = user_tone(0x51, 0, 22050);
    let mut preset = Vec::new();
    preset.extend(w(0x51, 0x30, 0x30));
    preset.extend(w(0x51, 0x10, 0x22));
    preset.extend(w(0x51, 0x20, 0x19));
    preset.extend(wait(22050));
    preset.push(0x66);
    let a = render(&mut machine(&opll_file(false, false, &user)), 22050);
    let b = render(&mut machine(&opll_file(false, false, &preset)), 22050);
    assert!(peak(&b) > 300);
    assert_ne!(digest(&a), digest(&b));
}

#[test]
fn the_second_ym2413_plays_the_tone_of_the_first() {
    let first = opll_file(false, false, &user_tone(0x51, 2, 8000));
    let second = opll_file(false, true, &user_tone(0xA1, 2, 8000));
    let (a, b) = (
        render(&mut machine(&first), 8000),
        render(&mut machine(&second), 8000),
    );
    assert!(peak(&a) > 500);
    assert_eq!(digest(&a), digest(&b));
    assert_eq!(
        machine(&second).channel_mask(),
        0x3FF | 0x1FF << 20 | 0x1FF << 29
    );
}

#[test]
fn a_psg_and_a_ym2413_play_together() {
    let mut s = user_tone(0x51, 0, 0);
    s.pop();
    s.extend([0x50, 0x90, 0x50, 0x8E, 0x50, 0x0F]);
    s.extend(wait(8000));
    s.push(0x66);
    let both = render(&mut machine(&opll_file(true, false, &s)), 8000);
    let opll = render(
        &mut machine(&opll_file(false, false, &user_tone(0x51, 0, 8000))),
        8000,
    );
    assert!(peak(&both) > peak(&opll) / 2);
    assert_ne!(digest(&both), digest(&opll));
}

#[test]
fn rhythm_mode_gives_channel_seven_to_the_bass_drum() {
    // Channel 7 as a bass drum: rhythm mode on, the drum's volume and its
    // frequency, then its own key bit. The key bit of register 0x26 does
    // nothing in this mode.
    let mut s = Vec::new();
    s.extend(w(0x51, 0x36, 0x00));
    s.extend(w(0x51, 0x16, 0x20));
    s.extend(w(0x51, 0x26, 0x05));
    s.extend(w(0x51, 0x0E, 0x20));
    s.extend(wait(4000));
    s.extend(w(0x51, 0x0E, 0x30));
    s.extend(wait(4000));
    s.push(0x66);
    let out = left(&render(&mut machine(&opll_file(false, false, &s)), 8000));
    assert_eq!(peak_of(&out[..3500]), 0);
    assert!(peak_of(&out[4500..]) > 300);
}

fn peak_of(v: &[i16]) -> i32 {
    v.iter().map(|s| i32::from(*s).abs()).max().unwrap_or(0)
}

#[test]
fn registers_written_at_random_run_and_repeat() {
    for seed in 0..4u64 {
        let mut rng = Rng(seed + 11);
        let mut s = Vec::new();
        for _ in 0..4000 {
            let reg = [
                0,
                1,
                2,
                3,
                4,
                5,
                6,
                7,
                0x0E,
                0x0F,
                0x10 + rng.below(9) as u8,
                0x20 + rng.below(9) as u8,
                0x30 + rng.below(9) as u8,
            ][rng.below(13) as usize];
            s.extend(w(0x51, reg, rng.byte()));
            if rng.below(4) == 0 {
                s.extend(wait(rng.below(500) as u16 + 1));
            }
        }
        s.push(0x66);
        let f = opll_file(false, false, &s);
        let a = render(&mut machine(&f), 30000);
        let b = render(&mut machine(&f), 30000);
        assert_eq!(digest(&a), digest(&b));
    }
}

#[test]
fn a_file_with_no_chip_the_core_has_is_still_refused() {
    let mut f = file(false, false, &[0x66], None);
    f[0x30..0x34].copy_from_slice(&3_579_545u32.to_le_bytes());
    assert_eq!(
        Machine::new(&parse(&f), 44_100).err(),
        Some(MachineError::NoChip)
    );
    assert!(Vgm::parse(&f).is_ok());
}
