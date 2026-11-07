//! The chip state that a tool can read: the register blocks, the echo output's
//! scope, the position of a voice in its sample, the decoded samples and the
//! timers. None of it changes the sound.

mod common;

use common::*;
use oscilla_snes::{
    BrrPosition, MAX_SAMPLE_BLOCKS, Machine, NO_LOOP_START, REGISTER_BLOCKS, SampleInfo, Spc,
};

const DC: [u8; 16] = [7; 16];

/// Voice 0 plays a looping block of equal nibbles, with the echo set up so that
/// the left channel returns at half volume and the right at a quarter. `code` is
/// the SPC700 program.
fn echo_machine(code: &[u8]) -> Machine {
    echo_machine_with(code, &[])
}

/// `echo_machine` with more DSP registers, which win over the ones it sets.
fn echo_machine_with(code: &[u8], extra: &[(u8, u8)]) -> Machine {
    let mut regs = voice0(0x1000, 0x40, 0x20);
    regs.extend_from_slice(&[
        (0x4D, 0x01),
        (0x6D, 0x30),
        (0x7D, 0x01),
        (0x6C, 0x00),
        (0x0F, 0x7F),
        (0x2C, 0x40),
        (0x3C, 0x20),
    ]);
    regs.extend_from_slice(extra);
    let spc = dsp_file(
        code,
        &regs,
        &[(0x2000, block(12, 0, 3, DC))],
        0x2000,
        0x2000,
    );
    dsp_machine(&spc)
}

/// A file with the sample table at 0x1000, the given DSP registers and the
/// bytes at the given addresses.
fn sample_file(dsp: &[(u8, u8)], ram: &[(u16, Vec<u8>)]) -> Spc {
    let mut f = file(&[0x2F, 0xFE], ZERO, dsp);
    for (address, bytes) in ram {
        f[0x100 + usize::from(*address)..][..bytes.len()].copy_from_slice(bytes);
    }
    Spc::parse(&f).unwrap()
}

fn entry(start: u16, loop_at: u16) -> Vec<u8> {
    [start.to_le_bytes(), loop_at.to_le_bytes()].concat()
}

fn sample_of(m: &Machine, srcn: u8) -> (SampleInfo, Vec<i16>) {
    let mut out = vec![0i16; MAX_SAMPLE_BLOCKS * 16 + 16];
    let info = m.sample(srcn, &mut out);
    out.truncate((info.total_samples as usize).min(out.len()));
    (info, out)
}

// ---- Register blocks ---------------------------------------------------

#[test]
fn the_register_blocks_are_the_dsp_and_the_io_registers() {
    assert_eq!(REGISTER_BLOCKS.len(), 2);
    assert_eq!(REGISTER_BLOCKS[0].name, "S-DSP");
    assert_eq!(
        (REGISTER_BLOCKS[0].base, REGISTER_BLOCKS[0].size),
        (0x00, 128)
    );
    assert_eq!(REGISTER_BLOCKS[1].name, "SPC700 I/O");
    assert_eq!(
        (REGISTER_BLOCKS[1].base, REGISTER_BLOCKS[1].size),
        (0xF0, 16)
    );
}

#[test]
fn the_dsp_block_holds_every_register_including_envx_and_outx() {
    let mut m = echo_machine(&[0x2F, 0xFE]);
    m.dsp_samples(700);
    let mut block = [0u8; 128];
    assert_eq!(m.register_block_get(0, &mut block), 128);
    for (i, b) in block.iter().enumerate() {
        assert_eq!(*b, m.dsp_register(i as u8), "register {i:02X}");
    }
    // Voice 0 sustains at the envelope 0x40 * 16, which ENVX shows as its high
    // byte, and its output shows in OUTX.
    assert_eq!(block[0x08], ((0x40 * 16) >> 4) as u8);
    assert_ne!(block[0x09], 0);
    assert_eq!(block[0x09], m.voice_levels()[0] as u8);
    assert_eq!(block[0x6D], 0x30);
    // A short buffer takes the first bytes, and an empty one takes none.
    let mut short = [0u8; 4];
    assert_eq!(m.register_block_get(0, &mut short), 4);
    assert_eq!(short, block[..4]);
    assert_eq!(m.register_block_get(0, &mut []), 0);
    // A long one takes the size of the block.
    let mut long = [0xAAu8; 200];
    assert_eq!(m.register_block_get(0, &mut long), 128);
    assert_eq!(long[128], 0xAA);
}

#[test]
fn the_io_block_reads_as_a_program_would_and_clears_nothing() {
    // Timer 2 on with the divisor 1, the DSP register number 0x4D selected,
    // then a spin.
    let code = [
        0x8F, 0x01, 0xFC, 0x8F, 0x04, 0xF1, 0x8F, 0x4D, 0xF2, 0x2F, 0xFE,
    ];
    let mut m = machine_ram(&code, ZERO, &[(0xF8, 0x5A), (0xF9, 0xA5)]);
    m.write_port(0, 0x11);
    m.write_port(3, 0x44);
    m.run_cycles(1000);
    let mut io = [0u8; 16];
    assert_eq!(m.register_block_get(1, &mut io), 16);
    // TEST and the control register read 0, and the DSP register number and
    // its value follow.
    assert_eq!(io[0], 0);
    assert_eq!(io[1], 0);
    assert_eq!(io[2], 0x4D);
    assert_eq!(io[3], m.dsp_register(0x4D));
    assert_eq!([io[4], io[7]], [0x11, 0x44]);
    assert_eq!([io[8], io[9]], [0x5A, 0xA5]);
    assert_eq!(io[0xC], 1);
    // Timer 2 counted, and reading the block twice shows the same value.
    assert_ne!(io[0xF], 0);
    let mut again = [0u8; 16];
    m.register_block_get(1, &mut again);
    assert_eq!(io, again);
    assert_eq!(m.timers()[2].output, io[0xF]);
}

#[test]
fn a_block_that_does_not_exist_reads_nothing() {
    let m = echo_machine(&[0x2F, 0xFE]);
    let mut out = [0x77u8; 16];
    assert_eq!(m.register_block_get(2, &mut out), 0);
    assert_eq!(m.register_block_get(usize::MAX, &mut out), 0);
    assert_eq!(out, [0x77; 16]);
}

// ---- The echo scope ----------------------------------------------------

#[test]
fn the_echo_scope_is_what_the_echo_adds_to_the_mix() {
    let capture = |mask: u64| {
        let mut m = echo_machine(&[0x2F, 0xFE]);
        m.set_channel_mask(mask);
        let samples = m.dsp_samples(2000);
        (m, samples)
    };
    let (with, a) = capture(0x1FF);
    let (_, b) = capture(0xFF);
    let mut left = vec![0f32; 1000];
    let mut right = vec![0f32; 1000];
    with.echo_scope(0, &mut left);
    with.echo_scope(1, &mut right);
    // At 32 kHz a frame is a sample, and the echo is the whole difference
    // between a mix with it and one without.
    for i in 0..1000 {
        let at = 1000 + i;
        let dl = f32::from(a[2 * at]) - f32::from(b[2 * at]);
        let dr = f32::from(a[2 * at + 1]) - f32::from(b[2 * at + 1]);
        assert_eq!(left[i] * 32768.0, dl, "left {i}");
        assert_eq!(right[i] * 32768.0, dr, "right {i}");
    }
    assert!(left.iter().any(|s| *s != 0.0));
    assert!(right.iter().any(|s| *s != 0.0));
    // The right volume is half the left volume.
    let l: f32 = left.iter().map(|s| s.abs()).sum();
    let r: f32 = right.iter().map(|s| s.abs()).sum();
    assert!((r * 2.0 - l).abs() < l * 0.02, "{l} {r}");
    assert!(left.iter().chain(&right).all(|s| (-1.0..=1.0).contains(s)));
}

#[test]
fn the_echo_scope_ignores_the_echo_mask_bit_and_the_mute_flag() {
    let scope_of = |mask: u64, flg: u8| {
        let mut m = echo_machine_with(&[0x2F, 0xFE], &[(0x6C, flg)]);
        m.set_channel_mask(mask);
        m.dsp_samples(2000);
        let mut scope = vec![0f32; 800];
        m.echo_scope(0, &mut scope);
        scope
    };
    let open = scope_of(0x1FF, 0x00);
    assert!(open.iter().any(|s| *s != 0.0));
    assert_eq!(scope_of(0x0FF, 0x00), open);
    assert_eq!(scope_of(0x1FF, 0x40), open);
}

#[test]
fn the_echo_scope_is_silent_before_the_echo_returns() {
    let mut m = echo_machine(&[0x2F, 0xFE]);
    m.dsp_samples(300);
    let mut scope = vec![1f32; 300];
    m.echo_scope(0, &mut scope);
    assert!(scope.iter().all(|s| *s == 0.0));
}

#[test]
fn the_echo_scope_resamples_like_the_voice_scopes() {
    let mut m = echo_machine(&[0x2F, 0xFE]);
    m.dsp_samples(5000);
    let mut scope = vec![0f32; 4096];
    m.echo_scope(0, &mut scope);
    assert!(scope[4095] != 0.0);
    // Asking for more than the ring holds gives zeros for the oldest part.
    let mut long = vec![1f32; 8192];
    m.echo_scope(1, &mut long);
    assert_eq!(long[0], 0.0);
}

#[test]
fn a_snapshot_carries_no_echo_scope() {
    let mut m = echo_machine(&[0x2F, 0xFE]);
    m.dsp_samples(2000);
    let snap = m.snapshot();
    let mut other = echo_machine(&[0x2F, 0xFE]);
    other.restore(&snap).unwrap();
    let mut scope = vec![1f32; 64];
    other.echo_scope(0, &mut scope);
    assert!(scope.iter().all(|s| *s == 0.0));
    let mut voice = vec![1f32; 64];
    other.scope(0, &mut voice);
    assert!(voice.iter().all(|s| *s == 0.0));
    // The original still has its ring.
    m.echo_scope(0, &mut scope);
    assert!(scope.iter().any(|s| *s != 0.0));
}

// ---- The position in a sample -----------------------------------------

/// Voice 0 plays a block (range 12, filter 0, no flags) and then a looping
/// block (range 11, filter 2, end and loop) that it loops to.
fn two_block_machine(second_flags: u8, loop_at: u16) -> Machine {
    let spc = dsp_file(
        &[0x2F, 0xFE],
        &voice0(0x1000, 0x40, 0x20),
        &[
            (0x2000, block(12, 0, 0, DC)),
            (0x2009, block(11, 2, second_flags, DC)),
        ],
        0x2000,
        loop_at,
    );
    dsp_machine(&spc)
}

#[test]
fn a_voice_reports_its_block_its_sample_and_its_loops() {
    let mut m = two_block_machine(3, 0x2009);
    // Before the key-on is polled, the voice has no block.
    let idle = m.voice_brr(0);
    assert_eq!(idle.block, 0);
    assert_eq!(
        (idle.flags, idle.filter, idle.range, idle.loops),
        (0, 0, 0, 0)
    );
    assert_eq!(idle.directory_entry, 0x1000);
    assert_eq!(idle.loop_address, 0x2009);
    // The key-on loads the start address and the header comes 5 samples later.
    m.dsp_samples(4);
    let keyed = m.voice_brr(0);
    assert_eq!((keyed.block, keyed.sample_in_block), (0x2000, 0));
    assert_eq!((keyed.flags, keyed.filter, keyed.range), (0, 0, 0));
    m.dsp_samples(1);
    let first = m.voice_brr(0);
    assert_eq!(
        (first.block, first.flags, first.filter, first.range),
        (0x2000, 0, 0, 12)
    );
    assert_eq!(first.source, 0);
    assert_eq!((first.total_samples, first.loop_length_samples), (32, 16));
    // At a pitch of 0x1000 the voice advances one sample for each DSP sample.
    // The second block is entered after 16 and the end of it loops back.
    let mut last: BrrPosition = first;
    let mut loops_seen = 0;
    for step in 0..600 {
        m.dsp_samples(1);
        let p = m.voice_brr(0);
        if p.block == last.block {
            assert_eq!(
                p.sample_in_block,
                (last.sample_in_block + 1) % 16,
                "step {step}"
            );
        } else {
            assert_eq!(
                (last.sample_in_block, p.sample_in_block),
                (15, 0),
                "step {step}"
            );
        }
        if last.block == 0x2009 && p.block == 0x2009 && p.sample_in_block == 0 {
            loops_seen += 1;
        }
        assert_eq!(p.loops, loops_seen, "step {step}");
        if p.block == 0x2009 {
            assert_eq!((p.flags, p.filter, p.range), (3, 2, 11));
        }
        last = p;
    }
    assert!(loops_seen > 30);
}

#[test]
fn the_loop_count_stops_at_255_and_a_key_on_clears_it() {
    // Waits about 8,600 samples, then writes KON for voice 0 again.
    let code = [
        0x8D, 0xB4, 0xCD, 0xFF, 0x1D, 0xD0, 0xFD, 0xDC, 0xD0, 0xF8, 0x8F, 0x4C, 0xF2, 0x8F, 0x01,
        0xF3, 0x2F, 0xFE,
    ];
    let spc = dsp_file(
        &code,
        &voice0(0x1000, 0x40, 0x20),
        &[(0x2000, block(12, 0, 0, DC)), (0x2009, block(11, 2, 3, DC))],
        0x2000,
        0x2009,
    );
    let mut m = dsp_machine(&spc);
    m.dsp_samples(7000);
    assert_eq!(m.voice_brr(0).loops, 255);
    m.dsp_samples(2000);
    let after = m.voice_brr(0).loops;
    assert!(after < 60, "{after}");
}

#[test]
fn the_lengths_follow_the_end_and_loop_flags() {
    // Code 1 ends and mutes: the sample has a total and no loop.
    let m = two_block_machine(1, 0x2009);
    let p = m.voice_brr(0);
    assert_eq!((p.total_samples, p.loop_length_samples), (32, 0));
    // The loop address may be the start: the whole sample loops.
    let m = two_block_machine(3, 0x2000);
    let p = m.voice_brr(0);
    assert_eq!((p.total_samples, p.loop_length_samples), (32, 32));
    // Code 2 is ignored, so no end flag is found in 4,096 blocks of it.
    let m = two_block_machine(2, 0x2009);
    let p = m.voice_brr(0);
    assert_eq!((p.total_samples, p.loop_length_samples), (0, 0));
}

#[test]
fn an_idle_voice_reports_the_entry_of_its_own_srcn() {
    let spc = sample_file(
        &[(0x5D, 0x10), (0x64, 3), (0x24, 3)],
        &[
            (0x1000, entry(0x2000, 0x2000)),
            (0x100C, entry(0x3000, 0x3009)),
            (0x2000, block(4, 0, 1, DC)),
            (0x3000, block(4, 0, 0, DC)),
            (0x3009, block(4, 0, 3, DC)),
        ],
    );
    let m = dsp_machine(&spc);
    let p = m.voice_brr(6);
    assert_eq!(p.source, 3);
    assert_eq!((p.directory_entry, p.loop_address), (0x100C, 0x3009));
    assert_eq!((p.total_samples, p.loop_length_samples), (32, 16));
    // Voice numbers wrap.
    assert_eq!(m.voice_brr(14), p);
    let q = m.voice_brr(2);
    assert_eq!(
        (q.source, q.total_samples, q.loop_length_samples),
        (3, 32, 16)
    );
    let z = m.voice_brr(0);
    assert_eq!(
        (z.source, z.total_samples, z.loop_length_samples),
        (0, 16, 0)
    );
}

// ---- The decoded sample -----------------------------------------------

const A: [u8; 16] = [1, 7, 8, 15, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const B: [u8; 16] = [1; 16];
const C: [u8; 16] = [3, 5, 9, 12, 0, 7, 8, 1, 2, 14, 15, 4, 6, 10, 11, 13];
const D: [u8; 16] = [7, 7, 8, 8, 6, 5, 9, 10, 0, 1, 2, 3, 12, 13, 14, 15];

/// Four blocks, one for each filter, that end with a loop back to the second.
fn filter_machine() -> Machine {
    let spc = sample_file(
        &[(0x5D, 0x10)],
        &[
            (0x1000, entry(0x2000, 0x2009)),
            (0x2000, block(4, 0, 0, A)),
            (0x2009, block(2, 1, 0, B)),
            (0x2012, block(3, 2, 0, C)),
            (0x201B, block(3, 3, 3, D)),
        ],
    );
    dsp_machine(&spc)
}

#[test]
fn a_sample_decodes_with_each_filter_as_the_documentation_computes_it() {
    // Worked by hand from the formulas of the BRR chapter of fullsnes. Block 1
    // (range 4, filter 0): each nibble n gives (n << 4) >> 1 = 8n. Block 2
    // (range 2, filter 1): each nibble 1 gives 2, plus old + ((-old) >> 4), from
    // old = -40: -40 + 2 = -38, so -36; then -34 and -32; and so on. Block 3
    // (range 3, filter 2): the first sample is 12 + 2 * 2 + ((-6) >> 5) - 1 +
    // (1 >> 4) = 12 + 4 - 1 - 1 + 0 = 14. Block 4 (range 3, filter 3): the
    // first sample is 28 - 234 + (1521 >> 6) + 89 + (-267 >> 4) = 28 - 234 + 23 +
    // 89 - 17 = -111.
    #[rustfmt::skip]
    let expected: [i16; 64] = [
        8, 56, -64, -8, 0, 8, 16, 24, 32, 40, 48, 56, -64, -56, -48, -40,
        -36, -32, -28, -25, -22, -19, -16, -13, -11, -9, -7, -5, -3, -1, 1, 2,
        14, 44, 41, 20, -1, 7, -19, -40, -52, -71, -92, -94, -70, -70, -89, -117,
        -111, -77, -81, -116, -120, -102, -115, -149, -175, -190, -192, -179, -182, -195, -212, -227,
    ];
    let m = filter_machine();
    let (info, samples) = sample_of(&m, 0);
    assert_eq!(samples, expected);
    assert_eq!(info.start_address, 0x2000);
    assert_eq!(info.loop_address, 0x2009);
    assert_eq!(info.total_samples, 64);
    assert_eq!(info.loop_start_sample, 16);
    assert!(info.has_loop && !info.truncated);
    // The lengths of the voice's position agree.
    let p = m.voice_brr(0);
    assert_eq!((p.total_samples, p.loop_length_samples), (64, 48));
}

#[test]
fn a_shift_above_12_keeps_only_the_sign_of_the_nibble() {
    // Range 13: the nibble becomes -1 or 0 and the shift 12, so 0 or -2048.
    let spc = sample_file(
        &[(0x5D, 0x10)],
        &[
            (0x1000, entry(0x2000, 0x2000)),
            (
                0x2000,
                block(
                    13,
                    0,
                    1,
                    [7, 8, 0, 15, 1, 9, 2, 14, 3, 10, 4, 11, 5, 12, 6, 13],
                ),
            ),
        ],
    );
    let m = dsp_machine(&spc);
    let (info, samples) = sample_of(&m, 0);
    let expected: Vec<i16> = (0..16)
        .map(|n| if n % 2 == 0 { 0 } else { -2048 })
        .collect();
    assert_eq!(samples, expected);
    // Code 1 mutes at the end, so the loop address is not a loop.
    assert!(!info.has_loop);
    assert_eq!(info.loop_start_sample, 0);
}

#[test]
fn a_sum_past_15_bits_wraps_and_a_sum_past_16_bits_saturates() {
    // Three blocks of the loudest equal nibbles: 14336 each in the first, then
    // filter 2 and filter 3 push the sum past 15 bits (-4544 is 28224 - 32768)
    // and, further on, past 16 bits (the saturated 16383).
    let spc = sample_file(
        &[(0x5D, 0x10)],
        &[
            (0x1000, entry(0x2000, 0x2000)),
            (0x2000, block(12, 0, 0, DC)),
            (0x2009, block(12, 2, 0, DC)),
            (0x2012, block(12, 3, 1, DC)),
        ],
    );
    let (_, samples) = sample_of(&dsp_machine(&spc), 0);
    #[rustfmt::skip]
    let expected: [i16; 48] = [
        14336, 14336, 14336, 14336, 14336, 14336, 14336, 14336,
        14336, 14336, 14336, 14336, 14336, 14336, 14336, 14336,
        -4544, -7766, 3792, -3924, 3300, -8464, -4893, 12943,
        16383, 16383, -2562, -5908, 5474, -2460, 4514, -7522,
        -2849, 15327, 16383, -1448, -1578, 12676, 16383, 16383,
        -2306, -3120, 10602, 16383, 16383, -2306, -3120, 10602,
    ];
    assert_eq!(samples, expected);
}

#[test]
fn a_sample_reads_around_the_end_of_ram_and_through_dir() {
    // The block at 0xFFFC takes its data bytes from 0xFFFD to 0xFFFF and
    // 0x0000 to 0x0004. The table is at 0x3400 and entry 2 points there.
    let spc = sample_file(
        &[(0x5D, 0x34)],
        &[
            (0x3408, entry(0xFFFC, 0xFFFC)),
            (0xFFFC, block(4, 0, 3, [1; 16])[..4].to_vec()),
            (0x0000, block(4, 0, 3, [1; 16])[4..].to_vec()),
        ],
    );
    let m = dsp_machine(&spc);
    let (info, samples) = sample_of(&m, 2);
    assert_eq!(samples, vec![8i16; 16]);
    assert_eq!((info.start_address, info.loop_address), (0xFFFC, 0xFFFC));
    assert!(info.has_loop);
    // Entry 0 is zeros: a start of 0 with an end flag found nowhere nearby.
    let (info, _) = sample_of(&m, 0);
    assert_eq!(info.start_address, 0);
}

#[test]
fn a_sample_with_no_end_flag_stops_at_4096_blocks() {
    let spc = sample_file(
        &[(0x5D, 0x10)],
        &[(0x1000, entry(0x2000, 0x2000)), (0x2000, block(4, 0, 0, A))],
    );
    let m = dsp_machine(&spc);
    let (info, samples) = sample_of(&m, 0);
    assert!(info.truncated && !info.has_loop);
    assert_eq!(info.total_samples, 65536);
    assert_eq!(samples.len(), 65536);
    assert_eq!(info.loop_start_sample, 0);
    // The voice's lengths are 0 for it.
    let p = m.voice_brr(0);
    assert_eq!((p.total_samples, p.loop_length_samples), (0, 0));
}

#[test]
fn a_loop_address_off_the_blocks_has_no_loop_start() {
    let mut spc_ram = vec![(0x1000, entry(0x2000, 0x4000))];
    spc_ram.push((0x2000, block(4, 0, 0, A)));
    spc_ram.push((0x2009, block(4, 0, 3, A)));
    spc_ram.push((0x4000, block(4, 0, 3, A)));
    let m = dsp_machine(&sample_file(&[(0x5D, 0x10)], &spc_ram));
    let (info, samples) = sample_of(&m, 0);
    assert_eq!(samples.len(), 32);
    assert!(info.has_loop);
    assert_eq!(info.loop_start_sample, NO_LOOP_START);
}

#[test]
fn a_short_buffer_takes_the_first_samples_and_the_total_still_shows() {
    let m = filter_machine();
    let mut out = [0x7777i16; 20];
    let info = m.sample(0, &mut out);
    assert_eq!(info.total_samples, 64);
    assert_eq!(out[..4], [8, 56, -64, -8]);
    let mut none: [i16; 0] = [];
    assert_eq!(m.sample(0, &mut none).total_samples, 64);
}

// ---- The timers -------------------------------------------------------

/// Enables timers with these divisors, then NOPs (zero bytes) forever.
fn timer_machine(divisors: [u8; 3], control: u8) -> Machine {
    let mut code = Vec::new();
    for (n, d) in divisors.iter().enumerate() {
        code.extend_from_slice(&[0x8F, *d, 0xFA + n as u8]);
    }
    code.extend_from_slice(&[0x8F, control, 0xF1]);
    let mut m = machine(&code);
    steps(&mut m, 4);
    m
}

/// Steps NOPs and checks that timer `n` increments exactly when
/// `ticks_until_output` said, `rounds` times in a row.
fn check_increments(m: &mut Machine, n: usize, rounds: usize) {
    for round in 0..rounds {
        let t = m.timers()[n];
        assert!(t.enabled);
        assert!(t.ticks_until_output >= 1, "round {round}");
        let (start, out) = (m.cycles(), t.output);
        loop {
            m.step_instruction();
            let now = m.timers()[n].output;
            if m.cycles() - start >= u64::from(t.ticks_until_output) {
                assert_eq!(now, (out + 1) & 15, "timer {n} round {round} ticks {t:?}");
                break;
            }
            assert_eq!(
                now,
                out,
                "timer {n} round {round} early at {}",
                m.cycles() - start
            );
        }
    }
}

#[test]
fn a_timer_reports_its_state_and_when_the_output_increments() {
    let mut m = timer_machine([3, 5, 0], 0x07);
    let t = m.timers();
    assert_eq!(
        t.map(|t| (t.enabled, t.divisor)),
        [(true, 3), (true, 5), (true, 0)]
    );
    check_increments(&mut m, 0, 40);
    check_increments(&mut m, 1, 40);
    // Divisor 0 divides by 256: 256 ticks of 16 cycles.
    check_increments(&mut m, 2, 3);
    let t = m.timers()[2];
    assert!(t.ticks_until_output <= 256 * 16);
}

#[test]
fn the_output_wraps_at_16_and_a_timer_that_is_off_reports_zero() {
    let mut m = timer_machine([1, 1, 1], 0x04);
    let t = m.timers();
    assert_eq!((t[0].enabled, t[0].ticks_until_output), (false, 0));
    assert_eq!((t[1].enabled, t[1].ticks_until_output), (false, 0));
    assert!(t[2].enabled);
    // Timer 2 at divisor 1 adds one every 16 cycles, so 40 rounds pass 16.
    check_increments(&mut m, 2, 40);
}

#[test]
fn a_divisor_below_the_internal_count_waits_for_the_wrap() {
    // Timer 2 counts with the divisor 0xF0, and the program lowers the divisor
    // to 3 after the count passed it.
    let code = [0x8F, 0xF0, 0xFC, 0x8F, 0x04, 0xF1];
    let mut program = code.to_vec();
    program.extend(std::iter::repeat_n(0x00, 400));
    program.extend_from_slice(&[0x8F, 0x03, 0xFC]);
    let mut m = machine(&program);
    steps(&mut m, 2 + 400 + 1);
    let t = m.timers()[2];
    assert_eq!(t.divisor, 3);
    // About 50 ticks passed, so 200 more bring the count to the wrap and 3 past.
    assert!(t.ticks_until_output > 200 * 16, "{t:?}");
    check_increments(&mut m, 2, 2);
}

#[test]
fn reading_the_timers_does_not_clear_them() {
    let mut m = timer_machine([1, 1, 1], 0x07);
    m.run_cycles(5000);
    let first = m.timers();
    assert_eq!(m.timers(), first);
    assert!(first.iter().any(|t| t.output != 0));
}

// ---- No change to the sound -------------------------------------------

fn play(reading: bool) -> (Vec<i16>, Vec<[u8; 128]>, u64) {
    // A program that enables timer 2 and spins, so that reading a timer output
    // would show if a read cleared it.
    let code = [0x8F, 0x01, 0xFC, 0x8F, 0x04, 0xF1, 0x2F, 0xFE];
    let mut m = echo_machine(&code);
    let mut all = Vec::new();
    let mut dsp = Vec::new();
    for _ in 0..12 {
        all.extend(render(&mut m, 700));
        if reading {
            let mut regs = [0u8; 128];
            let mut io = [0u8; 16];
            m.register_block_get(0, &mut regs);
            m.register_block_get(1, &mut io);
            let _ = m.timers();
            for v in 0..8 {
                let _ = m.voice_brr(v);
            }
            for srcn in 0..8 {
                let mut out = [0i16; 64];
                m.sample(srcn, &mut out);
            }
            let mut scope = [0f32; 300];
            m.echo_scope(0, &mut scope);
            m.echo_scope(1, &mut scope);
            m.scope(0, &mut scope);
        }
        let mut regs = [0u8; 128];
        m.register_block_get(0, &mut regs);
        dsp.push(regs);
    }
    // The timer output after the last render is the same with and without the
    // reads, since a read of the block clears nothing.
    let mut io = [0u8; 16];
    m.register_block_get(1, &mut io);
    (all, dsp, u64::from(io[0xF]) << 32 | m.cycles())
}

#[test]
fn reading_the_state_does_not_change_the_sound() {
    let (quiet, quiet_regs, quiet_tail) = play(false);
    let (read, read_regs, read_tail) = play(true);
    assert_eq!(quiet, read);
    assert_eq!(quiet_regs, read_regs);
    assert_eq!(quiet_tail, read_tail);
    assert!(quiet.iter().any(|s| *s != 0));
}
