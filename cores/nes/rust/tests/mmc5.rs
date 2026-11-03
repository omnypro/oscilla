//! The MMC5 expansion chip: its pulse channels, the sample register, the
//! multiplier, the RAM and the mix.

mod common;

use common::*;
use oscilla_nes::Machine;

const SECOND: u64 = 1_789_773;
const CLOCK: u64 = 7424;

fn init_of(stores: &[(u16, u8)]) -> Vec<u8> {
    stores.iter().flat_map(|&(a, v)| set(a, v)).collect()
}

fn with(stores: &[(u16, u8)]) -> Machine {
    mmc5_machine(&init_of(stores), &[])
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

/// Both pulse channels on, duty 50% (2), halt set, constant volume 15, and the
/// period `period` with the longest length.
fn steady(period: u16) -> Vec<(u16, u8)> {
    vec![
        (0x5015, 3),
        (0x5000, 0xBF),
        (0x5002, period as u8),
        (0x5003, 0xF8 | (period >> 8) as u8),
    ]
}

#[test]
fn a_pulse_channel_plays_the_pitch_its_period_names() {
    // A step takes 2 * (255 + 1) cycles and the sequence has eight, so the
    // pitch is 1789773 / 4096 = 437 Hz.
    let mut m = with(&steady(255));
    let levels = sample(&mut m, 0, 8, SECOND);
    let edges = rising_edges(&levels);
    assert!((436..=438).contains(&edges), "{edges}");
    assert_eq!(*levels.iter().max().unwrap(), 15);
}

#[test]
fn the_second_pulse_channel_has_its_own_registers() {
    let mut m = with(&[
        (0x5015, 3),
        (0x5004, 0xBF),
        (0x5006, 0xFF),
        (0x5007, 0xF8),
    ]);
    assert!(sample(&mut m, 1, 8, SECOND / 4).contains(&15));
    assert!(sample(&mut m, 0, 8, 10_000).iter().all(|&l| l == 0));
}

#[test]
fn the_duty_settings_follow_the_2a03() {
    for (duty, eighths) in [(0u8, 1.0), (1, 2.0), (2, 4.0), (3, 6.0)] {
        let mut m = with(&[
            (0x5015, 3),
            (0x5000, duty << 6 | 0x3F),
            (0x5002, 0x40),
            (0x5003, 0xF8),
        ]);
        let levels = sample(&mut m, 0, 8, SECOND / 2);
        let high = levels.iter().filter(|&&l| l > 0).count() as f64 / levels.len() as f64;
        assert!((high - eighths / 8.0).abs() < 0.01, "duty {duty}: {high}");
    }
}

#[test]
fn a_period_below_eight_does_not_silence_the_channel() {
    // A step takes 6 cycles and a wave 48, so 2000 cycles hold about 41.
    let mut m = with(&steady(2));
    let levels = sample(&mut m, 0, 1, 2000);
    let edges = rising_edges(&levels);
    assert!((40..=43).contains(&edges), "{edges}");
}

#[test]
fn the_envelope_steps_down_once_in_each_clock() {
    // Decay mode, volume setting 0, looping: the decay level is 15 after the
    // first clock and falls by one at each clock after it.
    let mut m = with(&[
        (0x5015, 3),
        (0x5000, 0xA0),
        (0x5002, 0x08),
        (0x5003, 0xF8),
    ]);
    m.run_cycles(CLOCK);
    for want in (6..=15).rev() {
        m.run_cycles(CLOCK / 2);
        let top = sample(&mut m, 0, 7, CLOCK / 2).into_iter().max().unwrap();
        assert_eq!(top, want);
    }
}

#[test]
fn the_length_counter_runs_down_in_two_clocks() {
    // Length index 3 loads 2. Without the halt bit, two clocks end the sound.
    let mut m = with(&[
        (0x5015, 3),
        (0x5000, 0x9F),
        (0x5002, 0x40),
        (0x5003, 3 << 3),
    ]);
    m.run_cycles(200);
    assert_eq!(m.peek(0x5015), 1);
    assert!(sample(&mut m, 0, 8, 2000).contains(&15));
    m.run_cycles(2 * CLOCK + 100);
    assert_eq!(m.peek(0x5015), 0);
    assert!(sample(&mut m, 0, 8, 20_000).iter().all(|&l| l == 0));
}

#[test]
fn a_disabled_channel_loads_no_length() {
    let mut m = with(&[(0x5015, 0), (0x5000, 0xBF), (0x5002, 0x40), (0x5003, 0xF8)]);
    assert_eq!(m.peek(0x5015), 0);
    assert!(sample(&mut m, 0, 8, 20_000).iter().all(|&l| l == 0));
}

#[test]
fn clearing_an_enable_bit_clears_the_length() {
    let mut stores = steady(0x40);
    stores.push((0x5015, 0));
    let mut m = with(&stores);
    assert_eq!(m.peek(0x5015), 0);
    assert!(sample(&mut m, 0, 8, 20_000).iter().all(|&l| l == 0));
}

#[test]
fn the_sample_register_holds_what_it_is_given() {
    let mut m = with(&[(0x5011, 0x80)]);
    m.run_cycles(100);
    assert_eq!(m.expansion_levels()[2], 0x80);
}

#[test]
fn a_store_of_zero_leaves_the_sample_register() {
    let mut m = with(&[(0x5011, 0x40), (0x5011, 0x00)]);
    m.run_cycles(100);
    assert_eq!(m.expansion_levels()[2], 0x40);
}

#[test]
fn the_sample_register_ignores_stores_in_read_mode() {
    let mut m = with(&[(0x5011, 0x40), (0x5010, 0x01), (0x5011, 0x70)]);
    m.run_cycles(100);
    assert_ne!(m.expansion_levels()[2], 0x70);
}

#[test]
fn the_sample_register_takes_what_the_cpu_reads_in_read_mode() {
    // The register starts at 0, and every read of the program in read mode
    // sets it to a byte that is not zero.
    let mut m = with(&[(0x5010, 0x01)]);
    m.run_cycles(100);
    assert_ne!(m.expansion_levels()[2], 0);
}

#[test]
fn the_multiplier_gives_the_product_of_its_operands() {
    let mut m = with(&[(0x5205, 200), (0x5206, 100)]);
    m.run_cycles(100);
    // 200 * 100 = 20000 = 0x4E20
    assert_eq!(m.peek(0x5205), 0x20);
    assert_eq!(m.peek(0x5206), 0x4E);
    let mut m = with(&[(0x5205, 255), (0x5206, 255)]);
    m.run_cycles(100);
    assert_eq!((m.peek(0x5206), m.peek(0x5205)), (0xFE, 0x01));
}

#[test]
fn the_ram_holds_a_kilobyte_less_the_bank_registers() {
    let mut m = with(&[(0x5C00, 0x12), (0x5FF7, 0x34), (0x5E00, 0x56)]);
    m.run_cycles(100);
    assert_eq!(m.peek(0x5C00), 0x12);
    assert_eq!(m.peek(0x5FF7), 0x34);
    assert_eq!(m.peek(0x5E00), 0x56);
}

#[test]
fn stores_to_the_registers_do_nothing_without_the_chip() {
    let init = init_of(&[(0x5015, 3), (0x5000, 0xBF), (0x5002, 0x40), (0x5003, 0xF8)]);
    let mut m = machine(&init, &[]);
    m.run_cycles(10_000);
    assert_eq!(m.expansion_levels(), [0; 8]);
    assert_eq!(m.peek(0x5015), 0);
}

#[test]
fn the_chip_raises_no_warning() {
    let m = with(&[]);
    assert_eq!(m.warnings_nul(), [0]);
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
fn a_full_volume_pulse_is_as_loud_as_one_of_the_2a03() {
    let mut mmc5 = with(&steady(0xFF));
    let mut own = machine(
        &init_of(&[(0x4015, 1), (0x4000, 0xBF), (0x4002, 0xFF), (0x4003, 0x09)]),
        &[],
    );
    assert!(mmc5.set_highpass_hz(1.0));
    assert!(own.set_highpass_hz(1.0));
    let ratio = swing(&mut mmc5) / swing(&mut own);
    assert!((ratio - 1.0).abs() < 0.05, "{ratio}");
}

#[test]
fn the_channel_mask_reaches_the_mmc5_channels() {
    let mut a = with(&steady(0xFF));
    let mut b = with(&steady(0xFF));
    assert_eq!(b.channel_mask(), 0x7FFFFF);
    b.set_channel_mask(!(1 << 20));
    assert!(a.set_highpass_hz(1.0));
    assert!(b.set_highpass_hz(1.0));
    assert!(swing(&mut a) > 1000.0);
    let out = render(&mut b, 24000);
    assert!(out[2 * 12000..].iter().all(|&v| v.abs() < 30));
    assert!(sample(&mut b, 0, 8, 10_000).iter().any(|&l| l > 0));
}

#[test]
fn the_mask_bit_of_the_sample_register_silences_it() {
    let stores = [(0x5011, 0xFF)];
    let mut a = with(&stores);
    let mut b = with(&stores);
    b.set_channel_mask(!(1 << 22));
    let sum = |m: &mut Machine| -> i64 {
        render(m, 4000).iter().map(|&v| i64::from(v)).sum()
    };
    assert_ne!(sum(&mut a), sum(&mut b));
}

#[test]
fn the_output_does_not_depend_on_the_size_of_the_render_chunks() {
    let stores = steady(0x120);
    let mut a = with(&stores);
    let mut b = with(&stores);
    let whole = render(&mut a, 6000);
    let mut parts = Vec::new();
    for n in [1, 7, 100, 333, 1, 2000, 3558] {
        parts.extend(render(&mut b, n));
    }
    assert_eq!(digest(&whole), digest(&parts));
}
