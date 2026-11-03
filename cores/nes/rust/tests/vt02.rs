//! The VT02's second sound generator and its control registers: the four
//! channels at 0x4020, the output switches and raw PCM at 0x4030, the enables
//! at 0x4035 and the changes to the first generator and the DMA channel.

mod common;

use common::*;
use oscilla_nes::Machine;

const SECOND: u64 = 1_789_773;

fn init_of(stores: &[(u16, u8)]) -> Vec<u8> {
    stores.iter().flat_map(|&(a, v)| set(a, v)).collect()
}

fn with(stores: &[(u16, u8)]) -> Machine {
    vt02_machine(&init_of(stores), &[])
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

/// Both generators get the same stores in the same order: the first through
/// 0x4000 and 0x4015, the second through 0x4020 and 0x4035. Channel `k` of the
/// first must then put out what channel `k` of the second does, at every
/// sample.
fn twins(k: usize, stores: &[(u16, u8)]) {
    let first: Vec<(u16, u8)> = stores.to_vec();
    let second: Vec<(u16, u8)> = stores
        .iter()
        .map(|&(a, v)| (if a == 0x4015 { 0x4035 } else { a + 0x20 }, v))
        .collect();
    let mut a = with(&first);
    let mut b = with(&second);
    let mut x = Vec::new();
    let mut y = Vec::new();
    for _ in 0..3000 {
        a.run_cycles(997);
        b.run_cycles(997);
        x.push(a.channel_levels()[k]);
        y.push(b.expansion_levels()[k]);
    }
    assert_eq!(x, y);
    assert!(x.iter().any(|&l| l > 0), "the channel never sounded");
    assert!(x.contains(&0), "the channel never fell silent");
}

#[test]
fn pulse_3_follows_pulse_1_register_for_register() {
    // Duty 25%, a decaying envelope, a sweep and a short length.
    twins(
        0,
        &[
            (0x4015, 1),
            (0x4000, 0x44),
            (0x4001, 0x93),
            (0x4002, 0x70),
            (0x4003, 0x41),
        ],
    );
}

#[test]
fn pulse_4_follows_pulse_2_register_for_register() {
    twins(
        1,
        &[
            (0x4015, 2),
            (0x4004, 0xC2),
            (0x4005, 0x00),
            (0x4006, 0xA0),
            (0x4007, 0x38),
        ],
    );
}

#[test]
fn triangle_2_follows_the_triangle_register_for_register() {
    twins(
        2,
        &[
            (0x4015, 4),
            (0x4008, 0x05),
            (0x400A, 0x30),
            (0x400B, 0x18),
        ],
    );
}

#[test]
fn noise_2_follows_the_noise_channel_register_for_register() {
    twins(
        3,
        &[
            (0x4015, 8),
            (0x400C, 0x03),
            (0x400E, 0x84),
            (0x400F, 0x20),
        ],
    );
}

#[test]
fn a_pulse_channel_plays_the_pitch_its_period_names() {
    // 1789773 / (16 * (255 + 1)) is 437 Hz.
    let mut m = with(&[(0x4035, 1), (0x4020, 0x7F), (0x4022, 0xFF), (0x4023, 0x08)]);
    let levels = sample(&mut m, 0, 8, SECOND);
    let edges = rising_edges(&levels);
    assert!((436..=438).contains(&edges), "{edges}");
    assert_eq!(*levels.iter().max().unwrap(), 15);
}

#[test]
fn the_triangle_plays_the_pitch_its_period_names() {
    // 1789773 / (32 * (63 + 1)) is 874 Hz.
    let mut m = with(&[(0x4035, 4), (0x4028, 0xFF), (0x402A, 63), (0x402B, 0x08)]);
    // The linear counter loads at the first quarter frame.
    m.run_cycles(10_000);
    let levels = sample(&mut m, 2, 8, SECOND / 2);
    let crossings = levels.windows(2).filter(|w| w[0] < 8 && w[1] >= 8).count();
    assert!((436..=438).contains(&crossings), "{crossings}");
    assert_eq!(*levels.iter().max().unwrap(), 15);
    assert_eq!(*levels.iter().min().unwrap(), 0);
}

/// The share of samples at which `level` is above zero over half a second.
fn duty_of(m: &mut Machine, level: impl Fn(&Machine) -> i32) -> f64 {
    let mut count = 0usize;
    let mut total = 0usize;
    let end = m.cycles() + SECOND / 2;
    while m.cycles() < end {
        m.run_cycles(8);
        total += 1;
        count += usize::from(level(m) > 0);
    }
    count as f64 / total as f64
}

#[test]
fn the_duty_settings_trade_the_quarter_and_the_half() {
    // The setting 1 is a half on the VT02, and the setting 2 a quarter.
    let want = [0.125, 0.5, 0.25, 0.75];
    for (setting, want) in want.iter().enumerate() {
        let setting = setting as u8;
        let mut first = with(&[
            (0x4015, 1),
            (0x4000, setting << 6 | 0x1F),
            (0x4002, 0x40),
            (0x4003, 0x08),
        ]);
        let high = duty_of(&mut first, |m| m.channel_levels()[0]);
        assert!((high - want).abs() < 0.01, "pulse 1, setting {setting}: {high}");
        let mut second = with(&[
            (0x4035, 1),
            (0x4020, setting << 6 | 0x1F),
            (0x4022, 0x40),
            (0x4023, 0x08),
        ]);
        let high = duty_of(&mut second, |m| m.expansion_levels()[0]);
        assert!((high - want).abs() < 0.01, "pulse 3, setting {setting}: {high}");
    }
}

#[test]
fn a_file_without_the_chip_keeps_the_2a03_duty_order() {
    let mut m = machine(
        &init_of(&[(0x4015, 1), (0x4000, 0x5F), (0x4002, 0x40), (0x4003, 0x08)]),
        &[],
    );
    let high = duty_of(&mut m, |m| m.channel_levels()[0]);
    assert!((high - 0.25).abs() < 0.01, "{high}");
}

#[test]
fn a_channel_is_silent_until_4035_enables_it() {
    let mut m = with(&[(0x4020, 0x7F), (0x4022, 0x40), (0x4023, 0x08)]);
    assert!(sample(&mut m, 0, 8, 50_000).iter().all(|&l| l == 0));
}

#[test]
fn the_status_at_4035_reports_the_length_counters() {
    let mut m = with(&[
        (0x4035, 0x0F),
        (0x4020, 0x1F),
        (0x4022, 0x40),
        (0x4023, 0x18),
        (0x4028, 0x00),
        (0x402B, 0x18),
        (0x402C, 0x1F),
        (0x402F, 0x18),
    ]);
    m.run_cycles(200);
    assert_eq!(m.peek(0x4035), 0b1101);
    let mut m = with(&[
        (0x4035, 0x0F),
        (0x4020, 0x1F),
        (0x4024, 0x1F),
        (0x4023, 0x18),
        (0x4027, 0x18),
        (0x4035, 0x01),
    ]);
    m.run_cycles(200);
    assert_eq!(m.peek(0x4035), 0b0001, "a cleared enable clears the counter");
}

#[test]
fn a_length_counter_runs_down_in_half_frames() {
    // The index 3 loads a length of 2: two half-frames, 29830 cycles.
    let mut m = with(&[
        (0x4035, 1),
        (0x4020, 0x1F),
        (0x4022, 0x40),
        (0x4023, 0x18),
    ]);
    m.run_cycles(20_000);
    assert_eq!(m.peek(0x4035) & 1, 1);
    m.run_cycles(20_000);
    assert_eq!(m.peek(0x4035) & 1, 0);
}

#[test]
fn nothing_reaches_the_output_until_the_second_generator_is_switched_on() {
    let stores = [(0x4035, 1), (0x4020, 0xBF), (0x4022, 0xFF), (0x4023, 0x08)];
    let mut off = with(&stores);
    let mut on = with(&[&[(0x4030, 0x08)][..], &stores[..]].concat());
    let sum = |m: &mut Machine| -> i64 {
        render(m, 4000).iter().map(|&v| i64::from(v).abs()).sum()
    };
    assert_eq!(sum(&mut off), 0);
    assert!(sum(&mut on) > 100_000);
}

/// The samples of the left channel after `skip` frames, enough for the
/// filters to have settled after a change of the mix.
fn tail(m: &mut Machine, skip: usize, frames: usize) -> Vec<i16> {
    let out = render(m, skip + frames);
    out[2 * skip..].iter().step_by(2).copied().collect()
}

#[test]
fn bit_2_of_4030_switches_the_first_generator_off() {
    let stores = [(0x4015, 1), (0x4000, 0xBF), (0x4002, 0xFF), (0x4003, 0x08)];
    let mut on = with(&stores);
    let mut off = with(&[&[(0x4030, 0x04)][..], &stores[..]].concat());
    assert!(tail(&mut on, 12000, 4000).iter().any(|&v| v.abs() > 1000));
    assert!(tail(&mut off, 12000, 4000).iter().all(|&v| v.abs() < 30));
    assert!(sample(&mut off, 0, 8, 1000).iter().all(|&l| l == 0));
    assert_eq!(off.channel_levels()[0], 15, "the level is shown before the switch");
}

/// The distance between the highest and the lowest sample once the filters
/// have settled.
fn swing(m: &mut Machine) -> f64 {
    let t = tail(m, 12000, 12000);
    f64::from(*t.iter().max().unwrap()) - f64::from(*t.iter().min().unwrap())
}

#[test]
fn a_full_volume_pulse_of_the_second_generator_is_as_loud_as_one_of_the_first() {
    let mut second = with(&[
        (0x4030, 0x08),
        (0x4035, 1),
        (0x4020, 0xBF),
        (0x4022, 0xFF),
        (0x4023, 0x08),
    ]);
    let mut first = with(&[(0x4015, 1), (0x4000, 0xBF), (0x4002, 0xFF), (0x4003, 0x08)]);
    assert!(second.set_highpass_hz(1000.0));
    assert!(first.set_highpass_hz(1000.0));
    let ratio = swing(&mut second) / swing(&mut first);
    assert!((ratio - 1.0).abs() < 0.01, "{ratio}");
}

#[test]
fn a_store_to_4031_sets_the_raw_pcm_level() {
    let mut m = with(&[(0x4030, 0x08), (0x4031, 0xC8)]);
    m.run_cycles(200);
    assert_eq!(m.expansion_levels()[4], 0xC8);
    assert_eq!(m.channel_levels()[4], 0, "the delta channel's level is its own");
}

#[test]
fn raw_pcm_has_eight_bits() {
    let level = |v: u8| -> f64 {
        let mut m = with(&[(0x4030, 0x08), (0x4031, v)]);
        assert!(m.set_highpass_hz(1.0));
        let out = render(&mut m, 2000);
        f64::from(out[2 * 20])
    };
    let (a, b, c) = (level(0x20), level(0x40), level(0x60));
    assert!(a > 0.0 && b > a && c > b);
    assert!(((c - b) - (b - a)).abs() < (b - a) * 0.02, "{a} {b} {c}");
}

/// Sixteen bytes that differ from their neighbors.
const RAMP: [u8; 16] = [
    10, 200, 30, 180, 50, 160, 70, 140, 90, 120, 15, 195, 35, 175, 55, 155,
];

fn distinct(levels: &[i32]) -> Vec<i32> {
    let mut v = vec![levels[0]];
    for &l in &levels[1..] {
        if l != *v.last().unwrap() {
            v.push(l);
        }
    }
    v
}

#[test]
fn the_delta_channel_plays_its_bytes_as_they_are_in_raw_mode() {
    // 0x4012 = 0 and A14 = A15 = 1 name 0xC000, which is offset 0x4000 of the
    // program. The rate index 15 takes 54 cycles for a byte.
    let init = init_of(&[
        (0x4030, 0x18),
        (0x4010, 0x0F),
        (0x4012, 0x00),
        (0x4013, 0x01),
        (0x4015, 0x10),
    ]);
    let mut m = vt02_with(&init, &[], Some((0x4000, &RAMP)));
    let levels = sample(&mut m, 4, 8, 54 * 24);
    let seen = distinct(&levels);
    let want: Vec<i32> = std::iter::once(0)
        .chain(RAMP.iter().map(|&b| i32::from(b)))
        .collect();
    assert_eq!(&seen[..want.len().min(seen.len())], &want[..seen.len().min(want.len())]);
    assert!(seen.len() >= 2, "{seen:?}");
}

#[test]
fn each_byte_lasts_one_period_of_the_rate() {
    let init = init_of(&[
        (0x4030, 0x18),
        (0x4010, 0x0F),
        (0x4012, 0x00),
        (0x4013, 0x01),
        (0x4015, 0x10),
    ]);
    let mut m = vt02_with(&init, &[], Some((0x4000, &RAMP)));
    m.run_cycles(500);
    let mut changes = Vec::new();
    let mut last = m.expansion_levels()[4];
    for _ in 0..(54 * 6) {
        m.run_cycles(1);
        let now = m.expansion_levels()[4];
        if now != last {
            changes.push(m.cycles());
            last = now;
        }
    }
    assert!(changes.len() >= 5);
    for w in changes.windows(2) {
        assert_eq!(w[1] - w[0], 54, "{changes:?}");
    }
}

#[test]
fn the_bits_of_4030_move_the_dma_address_out_of_the_program() {
    // Bits 0 and 1 set clear A14 and A15, so 0x4012 = 8 names 0x0200.
    let mut init = Vec::new();
    for (i, &b) in RAMP.iter().enumerate() {
        init.extend(set(0x0200 + i as u16, b));
    }
    init.extend(init_of(&[
        (0x4030, 0x1B),
        (0x4010, 0x0F),
        (0x4012, 0x08),
        (0x4013, 0x01),
        (0x4015, 0x10),
    ]));
    let mut m = vt02_machine(&init, &[]);
    let levels = sample(&mut m, 4, 8, 54 * 24 + 100 * 12);
    let seen = distinct(&levels);
    assert!(seen.len() > 10, "{seen:?}");
    let ramp: Vec<i32> = RAMP.iter().map(|&b| i32::from(b)).collect();
    let start = seen.iter().position(|&l| l == ramp[0]).unwrap();
    assert_eq!(&seen[start..start + 8], &ramp[..8]);
}

#[test]
fn the_length_register_counts_sixteen_bytes_a_step_and_bit_5_drops_the_extra_one() {
    let played = |control: u8| -> usize {
        let mut data = Vec::new();
        for i in 0..64u8 {
            data.push(if i % 2 == 0 { 20 + i } else { 200 - i });
        }
        let init = init_of(&[
            (0x4030, control),
            (0x4010, 0x0F),
            (0x4012, 0x00),
            (0x4013, 0x00),
            (0x4015, 0x10),
        ]);
        let mut m = vt02_with(&init, &[], Some((0x4000, &data)));
        let levels = sample(&mut m, 4, 8, 54 * 100);
        distinct(&levels).len()
    };
    // Length 1 byte: the level goes from 0 to one byte and stays.
    assert_eq!(played(0x18), 2);
    // Length 16 bytes: the level goes through all of them.
    assert_eq!(played(0x38), 17);
}

#[test]
fn raw_pcm_needs_the_second_output() {
    let init = [
        (0x4030, 0x10),
        (0x4031, 0xFF),
    ];
    let mut m = with(&init);
    assert!(tail(&mut m, 0, 2000).iter().all(|&v| v == 0));
}

#[test]
fn ordinary_delta_modulation_is_unchanged_without_the_pcm_bit() {
    // A sample of 0xFF bytes raises the level by two for each bit.
    let init = init_of(&[
        (0x4010, 0x0F),
        (0x4012, 0x00),
        (0x4013, 0x00),
        (0x4015, 0x10),
    ]);
    let ones = [0xFFu8; 16];
    let mut m = vt02_with(&init, &[], Some((0x4000, &ones)));
    m.run_cycles(54 * 40);
    assert!(m.channel_levels()[4] > 10);
    assert_eq!(m.expansion_levels()[4], 0);
}

#[test]
fn the_channel_mask_reaches_the_second_generator() {
    let stores = [
        (0x4030, 0x08),
        (0x4035, 3),
        (0x4020, 0xBF),
        (0x4022, 0xFF),
        (0x4023, 0x08),
        (0x4024, 0xBF),
        (0x4026, 0xFF),
        (0x4027, 0x08),
    ];
    let loud = |mask: u64| -> i32 {
        let mut m = with(&stores);
        assert_eq!(m.channel_mask(), 0x3_FFFF_FFFF);
        m.set_channel_mask(mask);
        let t = tail(&mut m, 12000, 12000);
        i32::from(*t.iter().max().unwrap()) - i32::from(*t.iter().min().unwrap())
    };
    let all = loud(0x3_FFFF_FFFF);
    assert!(all > 1000);
    let no_3 = loud(!(1 << 29));
    let no_4 = loud(!(1 << 30));
    assert!(no_3 > 0 && no_3 < all);
    assert!(no_4 > 0 && no_4 < all);
    assert!(loud(!(3 << 29)) < 30);
}

#[test]
fn the_mask_bit_of_raw_pcm_silences_it() {
    let stores = [(0x4030, 0x08), (0x4031, 0xFF)];
    let last = |mask: u64| -> i16 {
        let mut m = with(&stores);
        assert!(m.set_highpass_hz(1.0));
        m.set_channel_mask(mask);
        let t = tail(&mut m, 4000, 100);
        t[t.len() - 1]
    };
    // The idle triangle holds a level of its own, so the channels without the
    // PCM bit stay a step apart from the full mix.
    let all = i32::from(last(0x3_FFFF_FFFF));
    let no_pcm = i32::from(last(!(1 << 33)));
    assert!(all - no_pcm > 5000, "{all} {no_pcm}");
    assert!(i32::from(last(!(0x1F << 29))).abs() < 30);
}

#[test]
fn mask_bits_above_the_last_channel_are_dropped() {
    let mut m = with(&[]);
    m.set_channel_mask(u64::MAX);
    assert_eq!(m.channel_mask(), 0x3_FFFF_FFFF);
}

#[test]
fn stores_to_the_registers_do_nothing_without_the_chip() {
    let init = init_of(&[(0x4035, 1), (0x4020, 0x7F), (0x4022, 0x40), (0x4023, 0x08)]);
    let mut m = machine(&init, &[]);
    m.run_cycles(10_000);
    assert_eq!(m.expansion_levels(), [0; 8]);
    assert_eq!(m.peek(0x4035), 0);
}

#[test]
fn the_levels_of_the_chip_fill_the_first_five_entries() {
    let mut m = with(&[
        (0x4035, 0x0F),
        (0x4020, 0xBF),
        (0x4022, 0x40),
        (0x4023, 0x08),
        (0x4028, 0xFF),
        (0x402A, 0x10),
        (0x402B, 0x08),
        (0x4031, 0x33),
    ]);
    m.run_cycles(5000);
    let l = m.expansion_levels();
    assert_eq!(l[4], 0x33);
    assert_eq!(l[5..], [0, 0, 0]);
    assert!(l[0] == 0 || l[0] == 15);
}

#[test]
fn the_observer_sees_the_chips_stores() {
    let mut m = with(&[(0x4030, 0x08), (0x4020, 0x7F), (0x4035, 1)]);
    let seen = watch(&mut m);
    m.run_cycles(100);
    let w = &seen.borrow().writes;
    for (a, v) in [(0x4030u16, 0x08u8), (0x4020, 0x7F), (0x4035, 1)] {
        assert!(w.iter().any(|w| w.1 == a && w.2 == v), "{a:x}");
    }
}

#[test]
fn the_chip_is_not_reported() {
    let m = with(&[]);
    assert_eq!(m.warnings_nul(), [0]);
}

#[test]
fn a_snapshot_continues_exactly() {
    let init = init_of(&[
        (0x4030, 0x18),
        (0x4035, 0x0F),
        (0x4020, 0x7F),
        (0x4022, 0x90),
        (0x4023, 0x09),
        (0x4028, 0xFF),
        (0x402A, 0x60),
        (0x402B, 0x09),
        (0x402C, 0x1C),
        (0x402E, 0x05),
        (0x402F, 0x08),
        (0x4010, 0x0E),
        (0x4012, 0x00),
        (0x4013, 0x01),
        (0x4015, 0x10),
    ]);
    let mut data = Vec::new();
    for i in 0..64u8 {
        data.push(i.wrapping_mul(37));
    }
    let mut a = vt02_with(&init, &[], Some((0x4000, &data)));
    render(&mut a, 7777);
    let snap = a.snapshot();
    let from_a = render(&mut a, 9600);
    let mut b = vt02_with(&init, &[], Some((0x4000, &data)));
    b.restore(&snap).unwrap();
    assert_eq!(digest(&from_a), digest(&render(&mut b, 9600)));
}

#[test]
fn the_output_does_not_depend_on_the_size_of_the_render_chunks() {
    let stores = [
        (0x4030, 0x08),
        (0x4035, 0x0F),
        (0x4020, 0x7F),
        (0x4022, 0x90),
        (0x4023, 0x09),
        (0x4028, 0xFF),
        (0x402A, 0x60),
        (0x402B, 0x09),
        (0x402C, 0x1C),
        (0x402E, 0x05),
        (0x402F, 0x08),
    ];
    let mut a = with(&stores);
    let mut b = with(&stores);
    let whole = render(&mut a, 6000);
    let mut parts = Vec::new();
    for n in [1, 7, 100, 333, 1, 2000, 3558] {
        parts.extend(render(&mut b, n));
    }
    assert_eq!(digest(&whole), digest(&parts));
}
