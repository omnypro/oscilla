//! Files with a second YM2612 or a second SN76489.

mod common;

use common::*;
use oscilla_genesis::Machine;

/// Marks the chips of a file as dual (bit 30 of the clock) and, for the
/// SN76489, optionally with bit 31 as well, which names the T6W28.
fn dual(mut f: Vec<u8>, psg: bool, fm: bool, t6w28: bool) -> Vec<u8> {
    if psg {
        f[0x0F] |= 0x40 | if t6w28 { 0x80 } else { 0 };
    }
    if fm {
        f[0x2F] |= 0x40;
    }
    f
}

/// An FM tone on the YM2612 that `cmd` addresses (0x52 first, 0xA2 second).
fn fm_tone(cmd: u8, n: u16) -> Vec<u8> {
    let mut s = Vec::new();
    let mut w = |reg: u8, val: u8| s.extend([cmd, reg, val]);
    for (offset, level) in [(0u8, 0u8), (4, 127), (8, 127), (12, 127)] {
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

fn machine(bytes: &[u8]) -> Machine {
    let mut m = Machine::new(&parse(bytes), 44_100).unwrap();
    m.set_highpass_hz(0.0);
    m
}

fn peak(samples: &[i16]) -> i32 {
    samples.iter().map(|s| i32::from(*s).abs()).max().unwrap()
}

#[test]
fn a_tone_on_the_second_ym2612_sounds_like_the_same_tone_on_the_first() {
    let first = file(false, true, &fm_tone(0x52, 8000), None);
    let second = dual(
        file(false, true, &fm_tone(0xA2, 8000), None),
        false,
        true,
        false,
    );
    let a = render(&mut machine(&first), 8000);
    let b = render(&mut machine(&second), 8000);
    assert!(peak(&a) > 1000);
    assert_eq!(digest(&a), digest(&b));
}

#[test]
fn two_ym2612s_playing_the_same_tone_add() {
    let mut s = fm_tone(0x52, 0);
    s.pop();
    s.pop();
    s.pop();
    s.pop();
    let mut both = fm_tone(0xA2, 8000);
    // The first tone's writes, then the second's, then the wait.
    let mut stream = s.clone();
    stream.append(&mut both);
    let one = dual(
        file(false, true, &fm_tone(0x52, 8000), None),
        false,
        true,
        false,
    );
    let two = dual(file(false, true, &stream, None), false, true, false);
    let (a, b) = (
        render(&mut machine(&one), 8000),
        render(&mut machine(&two), 8000),
    );
    let (pa, pb) = (peak(&a), peak(&b));
    assert!((2 * pa - pb).abs() <= 4, "{pa} {pb}");
}

#[test]
fn the_mask_has_bits_for_the_second_chips_only_when_the_file_has_them() {
    let single = file(true, true, &[0x66], None);
    assert_eq!(machine(&single).channel_mask(), 0x3FF);
    let both = dual(file(true, true, &[0x66], None), true, true, false);
    assert_eq!(machine(&both).channel_mask(), 0xF_FFFF);
    let fm_only = dual(file(true, true, &[0x66], None), false, true, false);
    assert_eq!(machine(&fm_only).channel_mask(), 0xF_FFFF & !(0xF << 16));
    let mut m = machine(&both);
    m.set_channel_mask(0x3FF);
    assert_eq!(m.channel_mask(), 0x3FF);
    // Bits for a chip that the file lacks do nothing.
    let mut m = machine(&single);
    m.set_channel_mask(u64::MAX);
    assert_eq!(m.channel_mask(), 0x3FF);
}

#[test]
fn the_mask_silences_the_second_ym2612() {
    let bytes = dual(
        file(false, true, &fm_tone(0xA2, 8000), None),
        false,
        true,
        false,
    );
    let mut m = machine(&bytes);
    m.set_channel_mask(0x3FF);
    assert_eq!(peak(&render(&mut m, 8000)), 0);
    let mut m = machine(&bytes);
    m.set_channel_mask(0x3FF | 0x3F << 10);
    assert!(peak(&render(&mut m, 8000)) > 1000);
}

fn psg_tone(cmd: u8) -> Vec<u8> {
    let mut s = vec![cmd, 0x8E, cmd, 0x0F, cmd, 0x90];
    s.extend(wait(8000));
    s.push(0x66);
    s
}

#[test]
fn a_tone_on_the_second_sn76489_sounds_like_the_same_tone_on_the_first() {
    let first = file(true, false, &psg_tone(0x50), None);
    let second = dual(file(true, false, &psg_tone(0x30), None), true, false, false);
    let a = render(&mut machine(&first), 8000);
    let b = render(&mut machine(&second), 8000);
    assert!(peak(&a) > 1000);
    assert_eq!(digest(&a), digest(&b));
}

#[test]
fn the_mask_silences_the_second_sn76489() {
    let bytes = dual(file(true, false, &psg_tone(0x30), None), true, false, false);
    let mut m = machine(&bytes);
    m.set_channel_mask(0xFFFF);
    assert_eq!(peak(&render(&mut m, 8000)), 0);
    let mut m = machine(&bytes);
    m.set_channel_mask(0xFFFF | 0xF << 16);
    assert!(peak(&render(&mut m, 8000)) > 1000);
}

#[test]
fn a_t6w28_file_stays_one_chip() {
    let bytes = dual(file(true, false, &psg_tone(0x50), None), true, false, true);
    let mut m = machine(&bytes);
    assert_eq!(m.channel_mask(), 0x3FF);
    assert!(peak(&render(&mut m, 8000)) > 1000);
}

#[test]
fn a_stream_can_write_the_second_ym2612() {
    // Stream chip number 0x82 names the second YM2612.
    let mut s = vec![0x67, 0x66, 0x00, 2, 0, 0, 0, 0xC0, 0x40];
    s.extend([0xA2, 0x2B, 0x80]);
    s.extend([0x90, 0, 0x82, 0, 0x2A, 0x91, 0, 0, 1, 0, 0x92, 0]);
    s.extend(4410u32.to_le_bytes());
    s.extend([0x93, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0]);
    s.extend(wait(100));
    s.push(0x66);
    let bytes = dual(file(false, true, &s, None), false, true, false);
    let out = left(&render(&mut machine(&bytes), 100));
    assert!(peak(&out) > 1000);
    // The first chip has no DAC data, so it stays quiet when the second is muted.
    let mut m = machine(&bytes);
    m.set_channel_mask(0x3FF);
    assert_eq!(peak(&render(&mut m, 100)), 0);
}
