//! The data that a register viewer and a patch viewer can show: the register
//! blocks of the chips and the FM patch of each FM voice.

mod common;

use common::*;
use oscilla_genesis::{Chip, FmChip, Machine};

fn machine(bytes: &[u8]) -> Machine {
    let mut m = Machine::new(&parse(bytes), 44_100).unwrap();
    m.set_highpass_hz(0.0);
    m
}

/// `command` writes each pair to the chip that it addresses.
fn writes(command: u8, pairs: &[(u8, u8)]) -> Vec<u8> {
    pairs
        .iter()
        .flat_map(|&(register, value)| [command, register, value])
        .collect()
}

fn finished(mut s: Vec<u8>) -> Vec<u8> {
    s.push(0x66);
    s
}

fn opll_file(second: bool, stream: &[u8]) -> Vec<u8> {
    let mut f = file(false, false, stream, None);
    let word = PSG_CLOCK | if second { 1 << 30 } else { 0 };
    f[0x10..0x14].copy_from_slice(&word.to_le_bytes());
    f
}

/// Every chip, each with a second one.
fn everything(stream: &[u8]) -> Vec<u8> {
    let mut f = file(true, true, stream, None);
    f[0x0F] |= 0x40;
    f[0x2F] |= 0x40;
    f[0x10..0x14].copy_from_slice(&(PSG_CLOCK | 1 << 30).to_le_bytes());
    f
}

/// A tone on channel 1 of the YM2612 that `command` addresses, with the
/// frequency of A4 (F-number 1083, block 4) and every operator at multiple 1.
fn plain_tone(command: u8) -> Vec<u8> {
    let mut pairs = Vec::new();
    for offset in [0u8, 4, 8, 12] {
        pairs.extend([
            (0x30 + offset, 0x01),
            (0x40 + offset, if offset == 0 { 0 } else { 127 }),
            (0x50 + offset, 0x1F),
            (0x60 + offset, 0),
            (0x70 + offset, 0),
            (0x80 + offset, 0x0F),
        ]);
    }
    pairs.extend([(0xB0, 0x07), (0xB4, 0xC0), (0xA4, 0x24), (0xA0, 0x3B)]);
    writes(command, &pairs)
}

/// A user tone on channel 1 of the YM2413 that `command` addresses, at 440 Hz
/// and keyed on.
fn plain_opll_tone(command: u8) -> Vec<u8> {
    let mut s = writes(
        command,
        &[
            (0, 0x21),
            (1, 0x21),
            (2, 0x3F),
            (3, 0),
            (4, 0xF0),
            (5, 0xF0),
            (6, 0x0F),
            (7, 0x0F),
            (0x30, 0),
            (0x10, 0x22),
            (0x20, 0x19),
        ],
    );
    s.extend(wait(1));
    s
}

fn psg_tone() -> Vec<u8> {
    vec![0x50, 0x8E, 0x50, 0x0F, 0x50, 0x90]
}

fn read(m: &Machine, block: usize) -> Vec<u8> {
    m.register_block_read(block).unwrap()
}

#[test]
fn there_is_one_block_for_each_chip_in_the_order_of_the_mask() {
    let m = machine(&file(true, true, &[0x66], None));
    assert_eq!(m.register_block_count(), 2);
    let a = m.register_block(0).unwrap();
    let b = m.register_block(1).unwrap();
    assert_eq!((a.name, a.base, a.size), ("YM2612", 0, 512));
    assert_eq!((b.name, b.base, b.size), ("SN76489", 0, 12));
    assert_eq!((a.chip, b.chip), (Chip::Ym2612, Chip::Sn76489));

    let m = machine(&everything(&[0x66]));
    let names: Vec<&str> = (0..m.register_block_count())
        .map(|i| m.register_block(i).unwrap().name)
        .collect();
    assert_eq!(
        names,
        [
            "YM2612",
            "SN76489",
            "YM2612 (second)",
            "SN76489 (second)",
            "YM2413",
            "YM2413 (second)"
        ]
    );
    let sizes: Vec<usize> = (0..6).map(|i| m.register_block(i).unwrap().size).collect();
    assert_eq!(sizes, [512, 12, 512, 12, 64, 64]);
    assert!((0..6).all(|i| m.register_block(i).unwrap().base == 0));
    for (i, size) in sizes.iter().enumerate() {
        assert_eq!(read(&m, i).len(), *size, "block {i}");
    }

    // A chip that the file lacks has no block.
    let m = machine(&file(false, true, &[0x66], None));
    assert_eq!(m.register_block_count(), 1);
    let m = machine(&opll_file(false, &[0x66]));
    assert_eq!(m.register_block_count(), 1);
    assert_eq!(m.register_block(0).unwrap().name, "YM2413");
    let mut psgs = file(true, false, &[0x66], None);
    psgs[0x0F] |= 0x40;
    let m = machine(&psgs);
    assert_eq!(m.register_block(1).unwrap().name, "SN76489 (second)");
    // The T6W28 is one chip.
    psgs[0x0F] |= 0x80;
    let m = machine(&psgs);
    assert_eq!(m.register_block_count(), 1);
}

#[test]
fn a_block_past_the_last_is_none() {
    let m = machine(&file(true, true, &[0x66], None));
    assert!(m.register_block(2).is_none());
    assert!(m.register_block_read(2).is_none());
    assert!(m.register_block(usize::MAX).is_none());
}

#[test]
fn the_ym2612_block_holds_the_registers_of_both_ports() {
    let mut pairs0 = vec![
        (0x22, 0x0D),
        (0x24, 0xA5),
        (0x25, 0x02),
        (0x26, 0x77),
        (0x2A, 0x91),
        (0x2B, 0x80),
        (0x2C, 0x08),
    ];
    // Channel 1: the register at +4 is operator 3 and the one at +8 operator 2.
    for (i, offset) in [0u8, 4, 8, 12].into_iter().enumerate() {
        let i = i as u8;
        pairs0.extend([
            (0x30 + offset, (i + 1) << 4 | (i + 5)),
            (0x40 + offset, 0x10 + i),
            (0x50 + offset, (i << 6) | (0x10 + i)),
            (0x60 + offset, 0x80 | (0x08 + i)),
            (0x70 + offset, 0x0A + i),
            (0x80 + offset, (i + 3) << 4 | (i + 1)),
            (0x90 + offset, 0x0C - i),
        ]);
    }
    pairs0.extend([(0xA4, 0x2C), (0xA0, 0x5A), (0xB0, 0x2D), (0xB4, 0xB5)]);
    // Channel 5 is channel 2 of port 1.
    let pairs1 = [
        (0x31, 0x42),
        (0x45, 0x33),
        (0x59, 0xDF),
        (0x6D, 0x9F),
        (0x75, 0x1B),
        (0x89, 0xF7),
        (0x9D, 0x0A),
        (0xA5, 0x11),
        (0xA1, 0xC3),
        (0xB1, 0x3A),
        (0xB5, 0x67),
    ];
    let mut stream = writes(0x52, &pairs0);
    stream.extend(writes(0x53, &pairs1));
    // Channel 3's special mode: the registers of its three extra frequencies.
    stream.extend(writes(0x52, &[(0xAD, 0x1B), (0xA9, 0x7E), (0x27, 0x4B)]));
    stream.extend(wait(10));
    let mut m = machine(&file(false, true, &finished(stream), None));
    let fresh = read(&m, 0);
    assert_eq!(fresh[0x2A], 0x80);
    assert_eq!(fresh[0xB4], 0xC0);
    assert_eq!(fresh[0x1B6], 0xC0);
    render(&mut m, 100);
    let r = read(&m, 0);
    for &(register, value) in &pairs0 {
        assert_eq!(r[usize::from(register)], value, "{register:#x}");
    }
    for &(register, value) in &pairs1 {
        assert_eq!(r[0x100 + usize::from(register)], value, "{register:#x}");
    }
    assert_eq!((r[0xAD], r[0xA9], r[0x27]), (0x1B, 0x7E, 0x4B));
    // Registers that nothing wrote.
    assert_eq!(r[0x28], 0);
    assert_eq!(r[0x31], 0);
    assert_eq!(r[0x102], 0);
    assert_eq!(r[0xB5], 0xC0);
    assert_eq!(r[0x1B6], 0xC0);
    assert_eq!(r[0xA8], 0);
    assert_eq!(r[0x1A8], 0);
    // A write to a register that does not exist is not in the block.
    let stream = finished(writes(0x52, &[(0x33, 0xFF), (0xB3, 0xFF)]));
    let mut m = machine(&file(false, true, &stream, None));
    render(&mut m, 10);
    let r = read(&m, 0);
    assert_eq!((r[0x33], r[0xB3]), (0, 0));
    // A reset puts the registers back.
    m.reset();
    assert_eq!(read(&m, 0), fresh);
}

#[test]
fn the_key_command_is_not_a_register() {
    let stream = finished([plain_tone(0x52), writes(0x52, &[(0x28, 0xF0)]), wait(4000)].concat());
    let mut m = machine(&file(false, true, &stream, None));
    render(&mut m, 100);
    assert_eq!(read(&m, 0)[0x28], 0);
    assert!(m.voice(0).unwrap().key_on);
}

#[test]
fn the_sn76489_block_holds_the_tones_the_noise_the_attenuations_and_the_stereo_byte() {
    // Tone 0 = 0x1FA, tone 1 = 0x2B3, tone 2 = 0x0C1, white noise at rate 1.
    let stream = finished(vec![
        0x50, 0x8A, 0x50, 0x1F, 0x50, 0xA3, 0x50, 0x2B, 0x50, 0xC1, 0x50, 0x0C, 0x50, 0xE5, 0x50,
        0x93, 0x50, 0xB5, 0x50, 0xD7, 0x50, 0xF9, 0x4F, 0x5A,
    ]);
    let mut m = machine(&file(true, false, &stream, None));
    assert_eq!(
        read(&m, 0),
        [0, 0, 0, 0, 0, 0, 0, 15, 15, 15, 15, 0xFF],
        "the state at the start"
    );
    render(&mut m, 100);
    assert_eq!(
        read(&m, 0),
        [0xFA, 0x01, 0xB3, 0x02, 0xC1, 0x00, 0x05, 3, 5, 7, 9, 0x5A]
    );
    // The second chip has a block of its own, from the second command.
    let mut f = file(true, false, &finished(vec![0x30, 0x84, 0x30, 0x01]), None);
    f[0x0F] |= 0x40;
    let mut m = machine(&f);
    render(&mut m, 10);
    assert_eq!(read(&m, 1)[..2], [0x14, 0x00]);
    assert_eq!(read(&m, 0)[..2], [0, 0]);
}

#[test]
fn the_ym2413_block_holds_the_register_file_with_the_user_patch() {
    let pairs = [
        (0, 0xE5),
        (1, 0x5A),
        (2, 0xC9),
        (3, 0x1D),
        (4, 0x73),
        (5, 0xA4),
        (6, 0x5B),
        (7, 0x2C),
        (0x0E, 0x20),
        (0x0F, 0x03),
        (0x10, 0x81),
        (0x18, 0x42),
        (0x20, 0x3B),
        (0x28, 0x0D),
        (0x30, 0x4A),
        (0x38, 0xF1),
    ];
    let mut m = machine(&opll_file(false, &finished(writes(0x51, &pairs))));
    assert_eq!(read(&m, 0), vec![0u8; 64]);
    render(&mut m, 10);
    let r = read(&m, 0);
    assert_eq!(r.len(), 64);
    for &(register, value) in &pairs {
        assert_eq!(r[usize::from(register)], value, "{register:#x}");
    }
    // The registers that are not there read 0.
    assert_eq!(r[8..0x0E], [0; 6]);
    assert_eq!(r[0x19..0x20], [0; 7]);
    assert_eq!(r[0x29..0x30], [0; 7]);
    assert_eq!(r[0x39..], [0; 7]);
}

#[test]
fn the_second_chips_blocks_follow_their_own_commands() {
    let mut stream = writes(0xA2, &[(0x30, 0x55), (0x40, 0x66)]);
    stream.extend(writes(0xA3, &[(0x31, 0x77)]));
    stream.extend(writes(0xA1, &[(0x10, 0x99), (0x30, 0x4B)]));
    stream.extend(writes(0x52, &[(0x30, 0x11)]));
    stream.extend(writes(0x51, &[(0x10, 0x22)]));
    let mut m = machine(&everything(&finished(stream)));
    render(&mut m, 10);
    let (first, second) = (read(&m, 0), read(&m, 2));
    assert_eq!((first[0x30], first[0x40], first[0x131]), (0x11, 0, 0));
    assert_eq!(
        (second[0x30], second[0x40], second[0x131]),
        (0x55, 0x66, 0x77)
    );
    let (first, second) = (read(&m, 4), read(&m, 5));
    assert_eq!((first[0x10], first[0x30]), (0x22, 0));
    assert_eq!((second[0x10], second[0x30]), (0x99, 0x4B));
}

#[test]
fn an_fm_patch_reports_the_algorithm_the_feedback_and_the_operators_in_order() {
    // Multiples 1, 3, 2, 4 at the register slots S1, S3, S2, S4 are operators
    // 1, 2, 3, 4 with multiples 1, 2, 3, 4.
    let mut pairs = Vec::new();
    for (offset, multiple, level) in [
        (0u8, 1u8, 0x11u8),
        (4, 3, 0x33),
        (8, 2, 0x22),
        (12, 4, 0x44),
    ] {
        pairs.extend([
            (0x30 + offset, 0x30 | multiple),
            (0x40 + offset, level),
            (0x50 + offset, 0x40 | 0x1F),
            (0x60 + offset, 0x80),
            (0x70 + offset, 0),
            (0x80 + offset, 0x0F),
            (0x90 + offset, 0),
        ]);
    }
    // Operator 3 (the register at +4) holds SSG-EG 0x0A and operator 2 holds
    // 0x04, whose enable bit is clear.
    pairs.push((0x94, 0x0A));
    pairs.push((0x98, 0x04));
    pairs.extend([(0xB0, 4 | 5 << 3), (0xB4, 0xC0), (0xA4, 0x24), (0xA0, 0x3B)]);
    let mut m = machine(&file(false, true, &finished(writes(0x52, &pairs)), None));
    render(&mut m, 100);
    let p = m.fm_patch(0).unwrap();
    assert_eq!(p.chip, FmChip::Ym2612);
    assert_eq!(
        (p.algorithm, p.feedback, p.instrument, p.operators),
        (4, 5, 0, 4)
    );
    let multiples: Vec<u8> = p.op.iter().map(|o| o.multiple).collect();
    assert_eq!(multiples, [1, 2, 3, 4]);
    let levels: Vec<u8> = p.op.iter().map(|o| o.total_level).collect();
    assert_eq!(levels, [0x11, 0x22, 0x33, 0x44]);
    for o in &p.op {
        assert_eq!((o.detune, o.key_scale), (3, 1));
        assert_eq!((o.attack_rate, o.decay_rate, o.sustain_rate), (0x1F, 0, 0));
        assert_eq!((o.release_rate, o.sustain_level), (15, 0));
        assert!(o.am);
        assert_eq!((o.vibrato, o.sustained, o.waveform), (false, false, 0));
    }
    let ssg: Vec<u8> = p.op.iter().map(|o| o.ssg_eg).collect();
    assert_eq!(ssg, [0, 0, 0x0A, 0]);
    // Off, and at the quietest level.
    assert!(
        p.op.iter()
            .all(|o| o.phase == 4 && o.envelope_level == 1023)
    );
    // The channel's pitch is 440.1 Hz, and the operator's is the multiple
    // times that with the detune of 3 added to its increment.
    let base = 8664.0 * (7_670_454.0 / 144.0) / 1_048_576.0;
    assert!(
        (p.op[0].frequency_hz - base).abs() < 1.0,
        "{}",
        p.op[0].frequency_hz
    );
    assert!(
        (p.op[1].frequency_hz / p.op[0].frequency_hz - 2.0).abs() < 0.02,
        "{} {}",
        p.op[0].frequency_hz,
        p.op[1].frequency_hz
    );
    assert!((p.op[3].frequency_hz / p.op[0].frequency_hz - 4.0).abs() < 0.04);
}

#[test]
fn an_fm_patch_follows_the_envelope_through_a_note() {
    let mut s = plain_tone(0x52);
    s.extend(writes(0x52, &[(0x28, 0xF0)]));
    s.extend(wait(4000));
    s.extend(writes(0x52, &[(0x28, 0x00)]));
    s.extend(wait(40_000));
    let mut m = machine(&file(false, true, &finished(s), None));
    // All four operators are keyed on, at an attack rate of 31 and a sustain
    // level of 0.
    render(&mut m, 2000);
    let p = m.fm_patch(0).unwrap();
    assert_eq!(p.algorithm, 7);
    assert!(p.op.iter().all(|o| o.phase == 2 && o.envelope_level == 0));
    assert_eq!(m.fm_patch(1).unwrap().op[0].phase, 4);
    render(&mut m, 40_000);
    let p = m.fm_patch(0).unwrap();
    assert!(
        p.op.iter()
            .all(|o| o.phase == 4 && o.envelope_level == 1023)
    );
}

#[test]
fn channel_three_in_special_mode_reports_each_operators_own_frequency() {
    // Mode 1 gives operator 1 the frequency of $AD/$A9, which is A5 (an
    // F-number of 1083 at block 5), and leaves operator 4 on the channel's.
    let mut s = plain_tone(0x52);
    for offset in [0u8, 4, 8, 12] {
        s.extend(writes(
            0x52,
            &[
                (0x32 + offset, 0x01),
                (0x42 + offset, 0x7F),
                (0x52 + offset, 0x1F),
                (0x82 + offset, 0x0F),
            ],
        ));
    }
    s.extend(writes(
        0x52,
        &[
            (0xB2, 7),
            (0xA6, 0x24),
            (0xA2, 0x3B),
            (0xAD, 0x2C),
            (0xA9, 0x3B),
            (0x27, 0x40),
        ],
    ));
    let mut m = machine(&file(false, true, &finished(s), None));
    render(&mut m, 10);
    let p = m.fm_patch(2).unwrap();
    let base = 8664.0 * (7_670_454.0 / 144.0) / 1_048_576.0;
    assert!((p.op[3].frequency_hz - base).abs() < 1.0);
    assert!(
        (p.op[0].frequency_hz - 2.0 * base).abs() < 2.0,
        "{}",
        p.op[0].frequency_hz
    );
}

#[test]
fn only_fm_channels_have_a_patch() {
    let mut s = plain_tone(0x52);
    s.extend(writes(0x52, &[(0x2B, 0x80)]));
    s.extend(psg_tone());
    let mut m = machine(&file(true, true, &finished(s), None));
    render(&mut m, 100);
    for voice in 0..5 {
        assert!(m.fm_patch(voice).is_some(), "voice {voice}");
    }
    // Channel 6 plays the DAC.
    assert!(m.fm_patch(5).is_none());
    // The SN76489's four voices.
    for voice in 6..10 {
        assert!(m.fm_patch(voice).is_none(), "voice {voice}");
    }
    assert!(m.fm_patch(10).is_none());
    assert!(m.fm_patch(usize::MAX).is_none());
}

#[test]
fn a_built_in_instrument_shows_the_bytes_of_its_table() {
    // Instrument 3 with a volume of 4: 13 01 99 00 F2 C4 11 23.
    let mut s = writes(0x51, &[(0x30, 0x34), (0x10, 0x22), (0x20, 0x19)]);
    s.extend(wait(2000));
    let mut m = machine(&opll_file(false, &finished(s)));
    render(&mut m, 100);
    let p = m.fm_patch(0).unwrap();
    assert_eq!(p.chip, FmChip::Ym2413);
    assert_eq!(
        (p.algorithm, p.feedback, p.instrument, p.operators),
        (0, 0, 3, 2)
    );
    let (modulator, carrier) = (p.op[0], p.op[1]);
    assert_eq!(modulator.multiple, 3);
    assert_eq!(modulator.key_scale, 1);
    assert_eq!(modulator.total_level, 0x19);
    assert_eq!((modulator.attack_rate, modulator.decay_rate), (15, 2));
    assert_eq!((modulator.sustain_level, modulator.release_rate), (1, 1));
    assert!(!modulator.am && !modulator.vibrato && !modulator.sustained);
    assert_eq!(carrier.multiple, 1);
    assert_eq!(carrier.key_scale, 0);
    assert_eq!(carrier.total_level, 16);
    assert_eq!((carrier.attack_rate, carrier.decay_rate), (12, 4));
    assert_eq!((carrier.sustain_level, carrier.release_rate), (2, 3));
    assert_eq!((modulator.waveform, carrier.waveform), (0, 0));
    for o in [modulator, carrier] {
        assert_eq!((o.detune, o.sustain_rate, o.ssg_eg), (0, 0, 0));
    }
    assert_eq!(p.op[2], Default::default());
    assert_eq!(p.op[3], Default::default());
    // 440 Hz with the multiples 3 and 1.
    assert!(
        (carrier.frequency_hz - 440.0).abs() < 0.5,
        "{}",
        carrier.frequency_hz
    );
    assert!(
        (modulator.frequency_hz - 1320.0).abs() < 1.5,
        "{}",
        modulator.frequency_hz
    );
    // The note is on, so the carrier's envelope has left its start.
    assert!(carrier.phase < 4 && carrier.envelope_level < 127);
    // Channel 2 is off.
    let idle = m.fm_patch(1).unwrap();
    assert!(
        idle.op
            .iter()
            .take(2)
            .all(|o| o.phase == 4 && o.envelope_level == 127)
    );
}

#[test]
fn the_user_patch_shows_each_field_from_its_own_bit() {
    let pairs = [
        (0, 0xE5),
        (1, 0x5A),
        (2, 0xC9),
        (3, 0x1D),
        (4, 0x73),
        (5, 0xA4),
        (6, 0x5B),
        (7, 0x2C),
        (0x30, 0x07),
        (0x10, 0x22),
        (0x20, 0x09),
    ];
    let mut m = machine(&opll_file(false, &finished(writes(0x51, &pairs))));
    render(&mut m, 10);
    let p = m.fm_patch(0).unwrap();
    assert_eq!((p.instrument, p.feedback), (0, 5));
    let (modulator, carrier) = (p.op[0], p.op[1]);
    assert_eq!((modulator.multiple, modulator.key_scale), (5, 0));
    assert!(modulator.am && modulator.vibrato && modulator.sustained);
    assert_eq!(modulator.total_level, 9);
    assert_eq!((modulator.attack_rate, modulator.decay_rate), (7, 3));
    assert_eq!((modulator.sustain_level, modulator.release_rate), (5, 11));
    assert_eq!(modulator.waveform, 1);
    assert_eq!((carrier.multiple, carrier.key_scale), (10, 1));
    assert!(!carrier.am && carrier.vibrato && !carrier.sustained);
    assert_eq!(carrier.total_level, 28);
    assert_eq!((carrier.attack_rate, carrier.decay_rate), (10, 4));
    assert_eq!((carrier.sustain_level, carrier.release_rate), (2, 12));
    assert_eq!(carrier.waveform, 1);
}

#[test]
fn rhythm_mode_leaves_the_percussion_channels_without_a_patch() {
    let mut s = writes(0x51, &[(0x0E, 0x30), (0x36, 0x12), (0x20, 0x10)]);
    s.extend(wait(100));
    let mut m = machine(&opll_file(false, &finished(s)));
    render(&mut m, 100);
    assert!(m.fm_patch(5).is_some());
    for voice in 6..9 {
        assert!(m.fm_patch(voice).is_none(), "voice {voice}");
    }
    assert!(m.fm_patch(9).is_none());
}

#[test]
fn the_patches_of_the_second_chips_are_their_own() {
    let mut s = plain_tone(0xA2);
    s.extend(plain_opll_tone(0xA1));
    let mut m = machine(&everything(&finished(s)));
    render(&mut m, 100);
    // The first chips never got a write.
    assert_eq!(m.fm_patch(0).unwrap().algorithm, 0);
    assert_eq!(m.fm_patch(10).unwrap().algorithm, 7);
    assert_eq!(m.fm_patch(10).unwrap().op[0].multiple, 1);
    assert!(m.fm_patch(16).is_none());
    assert!(m.fm_patch(20).unwrap().op[1].phase == 4);
    let p = m.fm_patch(29).unwrap();
    assert_eq!(p.chip, FmChip::Ym2413);
    assert!(p.op[1].phase < 4);
    assert!(m.fm_patch(38).is_none());
}

#[test]
fn reading_does_not_change_the_sound() {
    let mut s = plain_tone(0x52);
    s.extend(writes(0x52, &[(0x28, 0xF0)]));
    s.extend(plain_opll_tone(0x51));
    s.extend(psg_tone());
    s.extend(plain_tone(0xA2));
    s.extend(writes(0xA2, &[(0x28, 0xF0)]));
    s.extend(plain_opll_tone(0xA1));
    s.extend(wait(30_000));
    let f = everything(&finished(s));
    let mut quiet = machine(&f);
    let mut curious = machine(&f);
    for _ in 0..40 {
        let a = render(&mut quiet, 531);
        let b = render(&mut curious, 531);
        assert_eq!(a, b);
        for block in 0..curious.register_block_count() {
            let _ = curious.register_block(block);
            let _ = curious.register_block_read(block);
        }
        for voice in 0..curious.voice_count() {
            let _ = curious.fm_patch(voice);
        }
        let _ = curious.register_block_read(99);
        let _ = curious.fm_patch(99);
    }
    assert_eq!(quiet.samples(), curious.samples());
    for block in 0..quiet.register_block_count() {
        assert_eq!(read(&quiet, block), read(&curious, block));
    }
}
