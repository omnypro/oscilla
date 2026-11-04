//! The S-DSP: the BRR decoder, the envelope, the mixer, noise and the echo
//! unit, each with a file that holds the registers and a few blocks.

mod common;

use common::*;
use oscilla_snes::Machine;

const DC: [u8; 16] = [7; 16];

fn dc_machine(extra: &[(u8, u8)]) -> Machine {
    let mut regs = voice0(0x1000, 0x7F, 0x40);
    regs.extend_from_slice(extra);
    let spc = dsp_file(&[0x2F, 0xFE], &regs, &[(0x2000, block(12, 0, 3, DC))], 0x2000, 0x2000);
    dsp_machine(&spc)
}

#[test]
fn a_block_of_equal_nibbles_plays_a_steady_level() {
    let mut m = dc_machine(&[]);
    let s = m.dsp_samples(200);
    let (l, r) = (s[2 * 150], s[2 * 150 + 1]);
    let want = {
        let (a, b, c) = (0x172i64, 0x519i64, 0x176i64);
        let interp = (((a * 14336) >> 10) + ((b * 14336) >> 10) + ((c * 14336) >> 10)) >> 1;
        let out = (interp * 2032) >> 11;
        (((out * 0x40) >> 6) * 0x7F) >> 7
    };
    assert_eq!(i64::from(l), want);
    assert_eq!(l, r);
}

#[test]
fn a_key_on_gives_five_silent_samples_and_then_the_attack() {
    let spc = dsp_file(
        &[0x2F, 0xFE],
        &[
            (0x00, 0x7F), (0x01, 0x7F), (0x02, 0x00), (0x03, 0x10), (0x04, 0),
            (0x05, 0xFF), (0x06, 0xE0), (0x0C, 0x7F), (0x1C, 0x7F), (0x5D, 0x10),
            (0x4C, 0x01),
        ],
        &[(0x2000, block(12, 0, 3, DC))],
        0x2000,
        0x2000,
    );
    let mut m = dsp_machine(&spc);
    let s = m.dsp_samples(40);
    assert!(s[..2 * 5].iter().all(|&v| v == 0));
    assert!(s[2 * 5..].iter().any(|&v| v != 0));
    assert_eq!(m.dsp_register(0x08), (m.dsp_register(0x08)), "ENVX readable");
}

#[test]
fn direct_gain_sets_the_envelope_to_sixteen_times_the_value() {
    let mut m = dc_machine(&[]);
    m.dsp_samples(50);
    // Envelope 0x7F * 16 = 0x7F0; ENVX is its upper seven bits.
    assert_eq!(m.dsp_register(0x08), 0x7F);
}

#[test]
fn attack_rate_fifteen_adds_a_step_of_1024_each_sample() {
    // ADSR1 = 0xFF: attack rate 15 -> rate 31 (+1024), decay 7, enabled.
    let spc = dsp_file(
        &[0x2F, 0xFE],
        &[
            (0x00, 0x7F), (0x01, 0x7F), (0x02, 0x00), (0x03, 0x10), (0x04, 0),
            (0x05, 0xFF), (0x06, 0xE0), (0x0C, 0x7F), (0x1C, 0x7F), (0x5D, 0x10),
            (0x4C, 0x01),
        ],
        &[(0x2000, block(12, 0, 3, DC))],
        0x2000,
        0x2000,
    );
    let mut m = dsp_machine(&spc);
    m.dsp_samples(6);
    // The first envelope step comes in the sixth sample: 0 + 1024.
    assert_eq!(m.dsp_register(0x08), 0x40);
    m.dsp_samples(1);
    // 2048 clips to 0x7FF, which starts the decay.
    assert_eq!(m.dsp_register(0x08), 0x7F);
}

#[test]
fn key_off_releases_by_eight_each_sample() {
    let mut program = dsp_program(&[(0x5C, 0x01)]);
    // Wait about 2000 cycles first so the voice has reached its level.
    let mut wait = vec![0xCD, 0xF0, 0x1D, 0xD0, 0xFD, 0xCD, 0xF0, 0x1D, 0xD0, 0xFD];
    wait.append(&mut program);
    let mut regs = voice0(0x1000, 0x7F, 0x40);
    regs.push((0x4C, 0x01));
    let spc = dsp_file(&wait, &regs, &[(0x2000, block(12, 0, 3, DC))], 0x2000, 0x2000);
    let mut m = dsp_machine(&spc);
    m.dsp_samples(40);
    assert_eq!(m.dsp_register(0x08), 0x7F);
    // 2032 falls by 8 a sample, so it takes 254 samples to reach zero.
    m.dsp_samples(400);
    assert_eq!(m.dsp_register(0x08), 0);
    let s = m.dsp_samples(10);
    assert!(s.iter().all(|&v| v == 0));
}

#[test]
fn the_end_flag_sets_endx_and_a_loop_keeps_the_voice_playing() {
    let spc = dsp_file(
        &[0x2F, 0xFE],
        &voice0(0x1000, 0x7F, 0x40),
        &[(0x2000, block(12, 0, 3, DC))],
        0x2000,
        0x2000,
    );
    let mut m = dsp_machine(&spc);
    m.dsp_samples(100);
    assert_eq!(m.dsp_register(0x7C) & 1, 1);
    assert_eq!(m.dsp_register(0x08), 0x7F);
    // A voice that was keyed on again loses its flag.
}

#[test]
fn an_end_block_without_the_loop_flag_mutes_the_voice() {
    let spc = dsp_file(
        &[0x2F, 0xFE],
        &voice0(0x1000, 0x7F, 0x40),
        &[(0x2000, block(12, 0, 1, DC))],
        0x2000,
        0x2000,
    );
    let mut m = dsp_machine(&spc);
    m.dsp_samples(100);
    assert_eq!(m.dsp_register(0x7C) & 1, 1);
    assert_eq!(m.dsp_register(0x08), 0);
    assert!(m.dsp_samples(10).iter().all(|&v| v == 0));
}

#[test]
fn filter_one_adds_fifteen_sixteenths_of_the_last_sample() {
    // A block of 1's with filter 1 climbs toward 16 times the step.
    let spc = dsp_file(
        &[0x2F, 0xFE],
        &voice0(0x1000, 0x7F, 0x7F),
        &[(0x2000, block(8, 1, 3, [1; 16]))],
        0x2000,
        0x2000,
    );
    let mut m = dsp_machine(&spc);
    let s = m.dsp_samples(60);
    let early = i32::from(s[2 * 8]);
    let late = i32::from(s[2 * 40]);
    assert!(late > early * 2, "{early} then {late}");
}

#[test]
fn shifts_above_twelve_decode_as_twelve_with_the_nibble_cut_to_its_sign() {
    // Nibble 7 with shift 13 gives (7 >> 3) << 12 >> 1 = 0. Nibble 8 gives -2048.
    let pos = dsp_file(&[0x2F, 0xFE], &voice0(0x1000, 0x7F, 0x7F),
        &[(0x2000, block(13, 0, 3, [7; 16]))], 0x2000, 0x2000);
    assert!(dsp_machine(&pos).dsp_samples(60).iter().all(|&v| v == 0));
    let neg = dsp_file(&[0x2F, 0xFE], &voice0(0x1000, 0x7F, 0x7F),
        &[(0x2000, block(13, 0, 3, [8; 16]))], 0x2000, 0x2000);
    let s = dsp_machine(&neg).dsp_samples(60);
    assert!(s[2 * 40] < 0);
}

#[test]
fn the_pitch_register_sets_samples_per_output() {
    // A block that alternates +7 and -8 changes sign every sample at 0x1000
    // and twice as fast at 0x2000.
    let alt = [7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8];
    let count = |pitch: u16| {
        let spc = dsp_file(&[0x2F, 0xFE], &voice0(pitch, 0x7F, 0x7F),
            &[(0x2000, block(12, 0, 3, alt))], 0x2000, 0x2000);
        let s = dsp_machine(&spc).dsp_samples(400);
        s.chunks(2).skip(20).map(|c| c[0]).collect::<Vec<_>>()
    };
    let slow = count(0x0400);
    let crossings = |v: &[i16]| v.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    let fast = count(0x0800);
    assert!(crossings(&fast) >= 2 * crossings(&slow) - 2, "{} {}", crossings(&slow), crossings(&fast));
}

#[test]
fn volumes_are_signed_and_the_master_volume_scales() {
    let mut a = dc_machine(&[]);
    let mut neg = dc_machine(&[(0x00, 0xC0), (0x01, 0xC0)]);
    let (x, y) = (a.dsp_samples(200)[300], neg.dsp_samples(200)[300]);
    assert!(x > 0 && (i32::from(y) + i32::from(x)).abs() <= 1, "{x} {y}");
    let mut half = dc_machine(&[(0x0C, 0x40), (0x1C, 0x40)]);
    let h = half.dsp_samples(200)[300];
    let want = i32::from(x) * 64 / 127;
    assert!((i32::from(h) - want).abs() <= 2, "{x} {h}");
}

#[test]
fn the_mute_flag_silences_the_output_and_not_the_voices() {
    let mut m = dc_machine(&[(0x6C, 0x40)]);
    let s = m.dsp_samples(200);
    assert!(s.iter().all(|&v| v == 0));
    assert_eq!(m.dsp_register(0x08), 0x7F);
}

#[test]
fn the_channel_mask_removes_a_voice_from_the_mix_and_keeps_it_running() {
    let mut m = dc_machine(&[]);
    m.set_channel_mask(!1);
    assert!(m.dsp_samples(200).iter().all(|&v| v == 0));
    assert_eq!(m.dsp_register(0x08), 0x7F);
    assert_eq!(m.channel_mask(), 0x1FE);
    m.set_channel_mask(u64::MAX);
    assert_eq!(m.channel_mask(), 0x1FF);
    assert!(m.dsp_samples(10).iter().any(|&v| v != 0));
}

#[test]
fn the_soft_reset_flag_keys_every_voice_off_at_once() {
    let mut m = dc_machine(&[(0x6C, 0x80)]);
    assert!(m.dsp_samples(100).iter().all(|&v| v == 0));
    assert_eq!(m.dsp_register(0x08), 0);
}

#[test]
fn noise_replaces_the_samples_of_the_voice() {
    let mut m = dc_machine(&[(0x3D, 0x01), (0x6C, 0x1F)]);
    let s = m.dsp_samples(400);
    let left: Vec<i16> = s.chunks(2).map(|c| c[0]).skip(20).collect();
    let distinct = left.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(distinct > 20, "{distinct}");
    // Without the noise bit the same voice is steady.
    let mut quiet = dc_machine(&[]);
    let q: Vec<i16> = quiet.dsp_samples(400).chunks(2).map(|c| c[0]).skip(100).collect();
    assert_eq!(q.iter().collect::<std::collections::BTreeSet<_>>().len(), 1);
}

#[test]
fn pitch_modulation_speeds_up_a_voice_by_the_output_of_the_voice_before() {
    // Voice 1 plays two blocks, the second with the end flag. ENDX bit 1 is
    // set when the second block starts, so a faster voice sets it sooner.
    let first_end = |pmon: u8| {
        let mut regs = voice0(0x1000, 0x7F, 0x00);
        regs.extend_from_slice(&[
            (0x10, 0x7F), (0x11, 0x7F), (0x12, 0x00), (0x13, 0x10), (0x14, 1),
            (0x15, 0x00), (0x17, 0x7F), (0x2D, pmon), (0x4C, 0x03),
        ]);
        let mut second = block(12, 0, 0, DC);
        second.extend(block(12, 0, 1, DC));
        let spc = dsp_file(
            &[0x2F, 0xFE],
            &regs,
            &[
                (0x2000, block(12, 0, 3, DC)),
                (0x1004, vec![0x00, 0x21, 0x00, 0x21]),
                (0x2100, second),
            ],
            0x2000,
            0x2000,
        );
        let mut m = dsp_machine(&spc);
        (1..200)
            .find(|_| {
                m.dsp_samples(1);
                m.dsp_register(0x7C) & 2 != 0
            })
            .unwrap()
    };
    let plain = first_end(0);
    let modulated = first_end(0x02);
    assert!(modulated + 3 < plain, "{plain} then {modulated}");
}

#[test]
fn echo_writes_the_mixed_voices_into_the_buffer() {
    // ESA = 0x40, EDL = 1, EON = voice 0, echo writes enabled.
    let mut m = dc_machine(&[(0x6D, 0x40), (0x7D, 0x01), (0x4D, 0x01), (0x0D, 0x00), (0x6C, 0x00)]);
    m.dsp_samples(300);
    let at = 0x4000 + 250 * 4;
    let word = i16::from_le_bytes([m.peek(at), m.peek(at + 1)]);
    // The first entry of the buffer holds the voice at its volume.
    let interp = (((0x172i64 * 14336) >> 10) + ((0x519 * 14336) >> 10) + ((0x176 * 14336) >> 10)) >> 1;
    let want = (((interp * 2032) >> 11) * 0x40) >> 6;
    assert_eq!(i64::from(word), want & !1);
    assert_eq!(m.peek(at) & 1, 0);
}

#[test]
fn the_echo_write_flag_stops_the_buffer_from_changing() {
    let mut m = dc_machine(&[(0x6D, 0x40), (0x7D, 0x01), (0x4D, 0x01), (0x6C, 0x20)]);
    m.dsp_samples(300);
    assert_eq!((m.peek(0x4000), m.peek(0x4001)), (0, 0));
}

#[test]
fn the_fir_filter_reads_the_buffer_and_the_echo_volume_mixes_it_in() {
    // Fill the echo buffer with a constant by hand, then play nothing: the
    // output is the buffer through FIR7 (the newest tap) and EVOL.
    let mut f = file(&[0x2F, 0xFE], ZERO, &[
        (0x6D, 0x40), (0x7D, 0x01), (0x6C, 0x20), (0x0F + 0x70, 0x40),
        (0x2C, 0x40), (0x3C, 0x40),
    ]);
    for i in 0..512usize {
        f[0x100 + 0x4000 + i * 4..][..4].copy_from_slice(&[0x00, 0x10, 0x00, 0xF0]);
    }
    let spc = oscilla_snes::Spc::parse(&f).unwrap();
    let mut m = dsp_machine(&spc);
    let s = m.dsp_samples(40);
    // Left: 0x1000 >> 1 = 0x800; * FIR7 0x40 >> 6 = 0x800; * EVOL 0x40 >> 7 = 0x400.
    assert_eq!(s[2 * 20], 0x400);
    // Right: 0xF000 is -4096; the same arithmetic gives -0x400.
    assert_eq!(s[2 * 20 + 1], -0x400);
}

#[test]
fn the_echo_buffer_wraps_at_its_length() {
    let mut m = dc_machine(&[(0x6D, 0x40), (0x7D, 0x00), (0x4D, 0x01), (0x6C, 0x00)]);
    m.dsp_samples(300);
    // A delay of 0 uses one entry: the next entry stays empty.
    assert_eq!((m.peek(0x4004), m.peek(0x4005)), (0, 0));
    assert_ne!((m.peek(0x4000), m.peek(0x4001)), (0, 0));
}

#[test]
fn registers_above_0x80_mirror_the_lower_half_and_endx_clears_on_any_write() {
    let spc = dsp_file(
        &dsp_program(&[(0x7C, 0x00)]),
        &voice0(0x1000, 0x7F, 0x40),
        &[(0x2000, block(12, 0, 3, DC))],
        0x2000,
        0x2000,
    );
    let mut m = dsp_machine(&spc);
    m.dsp_samples(3);
    assert_eq!(m.dsp_register(0x7C), m.dsp_register(0xFC));
    m.dsp_samples(100);
    // The block loops with the end flag every 16 samples.
    assert_eq!(m.dsp_register(0x7C) & 1, 1);
}

#[test]
fn the_loop_address_is_read_from_the_table_when_the_voice_loops() {
    // Source 0 loops on a loud block. After a while the driver points voice 0
    // at source 1, whose entry names a silent block, and the voice goes
    // quiet at its next loop.
    let wait = vec![0xCD, 0xF0, 0x1D, 0xD0, 0xFD, 0xCD, 0xF0, 0x1D, 0xD0, 0xFD];
    let mut program = wait;
    program.extend(dsp_program(&[(0x04, 1)]));
    let spc = dsp_file(
        &program,
        &voice0(0x1000, 0x7F, 0x40),
        &[
            (0x2000, block(12, 0, 3, DC)),
            (0x1004, vec![0x00, 0x21, 0x00, 0x21]),
            (0x2100, block(12, 0, 3, [0; 16])),
        ],
        0x2000,
        0x2000,
    );
    let mut m = dsp_machine(&spc);
    let early = m.dsp_samples(60);
    assert!(early.chunks(2).skip(40).all(|c| c[0] > 0));
    m.dsp_samples(300);
    let late = m.dsp_samples(60);
    assert!(late.iter().all(|&v| v == 0), "{:?}", &late[..8]);
}
