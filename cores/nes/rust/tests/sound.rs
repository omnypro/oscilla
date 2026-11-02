//! The sound unit, seen through the levels of its channels and the registers
//! it exposes. Each test runs a tiny driver and reads the machine back.

mod common;

use common::*;
use oscilla_nes::Machine;

const SECOND: u64 = 1_789_773;

fn init_of(stores: &[(u16, u8)]) -> Vec<u8> {
    stores.iter().flat_map(|&(a, v)| set(a, v)).collect()
}

/// A machine whose init routine makes `stores`, stopped before the first play
/// call.
fn with(stores: &[(u16, u8)]) -> Machine {
    machine(&init_of(stores), &[])
}

/// Samples the level of `channel` every `step` cycles for `cycles` cycles.
fn sample(m: &mut Machine, channel: usize, step: u64, cycles: u64) -> Vec<i32> {
    let mut v = Vec::new();
    let end = m.cycles() + cycles;
    while m.cycles() < end {
        m.run_cycles(step);
        v.push(m.channel_levels()[channel]);
    }
    v
}

fn rising_edges(levels: &[i32]) -> usize {
    levels.windows(2).filter(|w| w[0] == 0 && w[1] > 0).count()
}

#[test]
fn a_pulse_channel_plays_the_pitch_its_period_names() {
    // 1789773 / (16 * (253 + 1)) is 440.4 Hz.
    let mut m = with(&[(0x4015, 1), (0x4000, 0xBF), (0x4002, 0xFD), (0x4003, 0x08)]);
    let levels = sample(&mut m, 0, 8, SECOND);
    let edges = rising_edges(&levels);
    assert!((439..=441).contains(&edges), "{edges}");
    assert_eq!(*levels.iter().max().unwrap(), 15);
}

#[test]
fn the_duty_settings_give_their_share_of_high_time() {
    for (duty, share) in [(0u8, 0.125), (1, 0.25), (2, 0.5), (3, 0.75)] {
        let mut m = with(&[
            (0x4015, 1),
            (0x4000, duty << 6 | 0x3F),
            (0x4002, 0x40),
            (0x4003, 0x08),
        ]);
        let levels = sample(&mut m, 0, 8, SECOND / 2);
        let high = levels.iter().filter(|&&l| l > 0).count() as f64 / levels.len() as f64;
        assert!((high - share).abs() < 0.01, "duty {duty}: {high}");
    }
}

#[test]
fn a_period_below_eight_is_silent() {
    for (period, audible) in [(7u8, false), (8, true)] {
        let mut m = with(&[
            (0x4015, 1),
            (0x4000, 0xBF),
            (0x4002, period),
            (0x4003, 0x08),
        ]);
        let levels = sample(&mut m, 0, 8, 20_000);
        assert_eq!(levels.iter().any(|&l| l > 0), audible, "period {period}");
    }
}

#[test]
fn a_sweep_that_would_overflow_mutes_the_channel() {
    // Period 0x400 with no shift doubles past 0x7FF, even with the sweep off.
    let mut m = with(&[(0x4015, 1), (0x4000, 0xBF), (0x4002, 0x00), (0x4003, 0x0C)]);
    assert!(sample(&mut m, 0, 8, 20_000).iter().all(|&l| l == 0));
    // With the negate flag set the target is lower and the channel plays.
    let mut m = with(&[
        (0x4015, 1),
        (0x4000, 0xBF),
        (0x4001, 0x08),
        (0x4002, 0x00),
        (0x4003, 0x0C),
    ]);
    assert!(sample(&mut m, 0, 8, 20_000).iter().any(|&l| l > 0));
}

#[test]
fn a_negating_sweep_raises_the_pitch() {
    // Period 0x40, shift 1, divider 0, negate: the first half-frame clock sets
    // the period to 0x1F.
    let mut m = with(&[
        (0x4015, 1),
        (0x4000, 0xBF),
        (0x4001, 0x89),
        (0x4002, 0x40),
        (0x4003, 0x08),
    ]);
    m.run_cycles(2000);
    let before = rising_edges(&sample(&mut m, 0, 8, 12_000));
    m.run_cycles(2000);
    let after = rising_edges(&sample(&mut m, 0, 8, 12_000));
    let ratio = after as f64 / before as f64;
    assert!((1.7..2.3).contains(&ratio), "{before} then {after}");
}

#[test]
fn a_length_counter_runs_out_on_the_half_frames() {
    // Index 3 loads 2. Half-frame clocks come at 14916 and 29832 cycles.
    let mut m = with(&[(0x4015, 1), (0x4000, 0x9F), (0x4002, 0x40), (0x4003, 0x18)]);
    m.run_cycles(20_000);
    assert_eq!(m.peek(0x4015) & 1, 1);
    m.run_cycles(9_000);
    assert_eq!(m.peek(0x4015) & 1, 1);
    m.run_cycles(3_000);
    assert_eq!(m.peek(0x4015) & 1, 0);
}

#[test]
fn the_halt_flag_holds_the_length_counter() {
    let mut m = with(&[(0x4015, 1), (0x4000, 0xBF), (0x4002, 0x40), (0x4003, 0x18)]);
    m.run_cycles(200_000);
    assert_eq!(m.peek(0x4015) & 1, 1);
}

#[test]
fn a_disabled_channel_has_no_length_and_cannot_load_one() {
    let mut m = with(&[(0x4015, 0), (0x4000, 0xBF), (0x4002, 0x40), (0x4003, 0x08)]);
    m.run_cycles(1000);
    assert_eq!(m.peek(0x4015) & 0xF, 0);
    assert!(sample(&mut m, 0, 8, 10_000).iter().all(|&l| l == 0));
    let mut on = with(&[
        (0x4015, 1),
        (0x4000, 0xBF),
        (0x4002, 0x40),
        (0x4003, 0x08),
        (0x4015, 0),
    ]);
    on.run_cycles(1000);
    assert_eq!(on.peek(0x4015) & 1, 0);
}

#[test]
fn an_envelope_decays_one_step_per_quarter_frame() {
    // Decay mode with a divider of 0: the first quarter-frame clock starts the
    // envelope at 15, and each later clock lowers it by one.
    let mut m = with(&[(0x4015, 1), (0x4000, 0x80), (0x4002, 0x64), (0x4003, 0x08)]);
    m.run_cycles(300);
    let mut maxima = Vec::new();
    for _ in 0..4 {
        let window = sample(&mut m, 0, 8, 6_000);
        maxima.push(*window.iter().max().unwrap());
        m.run_cycles(1_457);
    }
    assert_eq!(maxima, [0, 15, 14, 13]);
}

#[test]
fn constant_volume_ignores_the_envelope() {
    let mut m = with(&[(0x4015, 1), (0x4000, 0xB9), (0x4002, 0x64), (0x4003, 0x08)]);
    let levels = sample(&mut m, 0, 8, 200_000);
    assert_eq!(*levels.iter().max().unwrap(), 9);
}

#[test]
fn the_triangle_steps_down_and_up_through_sixteen_levels() {
    // Period 127: one step every 128 cycles once the linear counter is loaded.
    let mut m = with(&[(0x4015, 4), (0x4008, 0xFF), (0x400A, 0x7F), (0x400B, 0x08)]);
    let mut changes = Vec::new();
    let mut last = m.channel_levels()[2];
    assert_eq!(last, 15);
    while m.cycles() < 28_000 {
        m.run_cycles(8);
        let l = m.channel_levels()[2];
        if l != last {
            changes.push((m.cycles(), l - last));
            last = l;
        }
    }
    // Nothing moves before the first quarter-frame clock loads the linear counter.
    assert!(changes[0].0 > 7_000, "{:?}", changes[0]);
    let steps: Vec<i32> = changes.iter().map(|c| c.1).collect();
    assert!(steps.len() > 100);
    let down = steps.iter().take_while(|&&s| s == -1).count();
    assert_eq!(down, 15);
    assert!(steps[down..down + 15].iter().all(|&s| s == 1));
    assert!(steps[down + 15..down + 30].iter().all(|&s| s == -1));
    // Between steps: 128 cycles, to the resolution of the sampling.
    let gap = changes[20].0 - changes[19].0;
    assert!((120..=136).contains(&gap), "{gap}");
}

#[test]
fn a_linear_counter_that_runs_out_stops_the_triangle() {
    // The control flag is clear and the reload value is 2: the counter is
    // loaded once and then counts down on each quarter-frame clock.
    let mut m = with(&[(0x4015, 4), (0x4008, 0x02), (0x400A, 0x10), (0x400B, 0x08)]);
    m.run_cycles(60_000);
    let a = m.channel_levels()[2];
    let moves = sample(&mut m, 2, 8, 20_000);
    assert!(moves.iter().all(|&l| l == a));
}

fn noise_reference(steps: u64, mode: bool, mode_from: u64) -> u16 {
    let mut lfsr = 1u16;
    for j in 1..=steps {
        let tap = if mode && j >= mode_from { 6 } else { 1 };
        let feedback = (lfsr ^ (lfsr >> tap)) & 1;
        lfsr = (lfsr >> 1) | (feedback << 14);
    }
    lfsr
}

#[test]
fn the_noise_register_follows_its_shift_register() {
    for mode in [false, true] {
        let mut m = with(&[
            (0x4015, 8),
            (0x400C, 0x3F),
            (0x400E, if mode { 0x80 } else { 0 }),
            (0x400F, 0x08),
        ]);
        let seen = watch(&mut m);
        m.run_cycles(60);
        let mode_write = seen
            .borrow()
            .writes
            .iter()
            .find(|w| w.1 == 0x400E)
            .unwrap()
            .0;
        for _ in 0..2000 {
            m.run_cycles(4);
            let steps = m.cycles() / 4;
            // Steps at cycles up to the store of the mode use the old taps.
            let lfsr = noise_reference(steps, mode, mode_write / 4 + 1);
            let want = if lfsr & 1 == 0 { 15 } else { 0 };
            assert_eq!(m.channel_levels()[3], want, "mode {mode}, step {steps}");
        }
    }
}

#[test]
fn a_whole_sample_moves_the_delta_level_by_two_per_bit() {
    // One byte of ones, at the fastest rate: 8 bits add 16.
    let data = [0xFFu8; 16];
    let init = init_of(&[(0x4010, 0x0F), (0x4012, 0), (0x4013, 0), (0x4015, 0x10)]);
    let f = file_with(&init, &[], 16666, 0, [0; 8], Some((0x4000, &data)));
    let mut m = Machine::new(&oscilla_nsf::Nsf::parse(&f).unwrap(), 48000).unwrap();
    m.run_cycles(5_000);
    assert_eq!(m.channel_levels()[4], 16);
    assert_eq!(m.peek(0x4015) & 0x10, 0);
    m.run_cycles(5_000);
    assert_eq!(m.channel_levels()[4], 16);
}

fn dmc_machine(rate_byte: u8, length: u8) -> Machine {
    let data = [0xFFu8; 0x100];
    let init = init_of(&[
        (0x4010, rate_byte),
        (0x4012, 0),
        (0x4013, length),
        (0x4015, 0x10),
    ]);
    let f = file_with(&init, &[], 16666, 0, [0; 8], Some((0x4000, &data)));
    Machine::new(&oscilla_nsf::Nsf::parse(&f).unwrap(), 48000).unwrap()
}

#[test]
fn a_looping_sample_climbs_to_the_top_and_keeps_going() {
    let mut m = dmc_machine(0x4F, 0);
    m.run_cycles(20_000);
    assert_eq!(m.channel_levels()[4], 126);
    assert_eq!(m.peek(0x4015) & 0x10, 0x10);
}

#[test]
fn a_finished_sample_raises_the_interrupt_flag_until_it_is_cleared() {
    let mut m = dmc_machine(0x8F, 0);
    assert_eq!(m.peek(0x4015) & 0x80, 0);
    m.run_cycles(5_000);
    assert_eq!(m.peek(0x4015) & 0x80, 0x80);
    // A store to the status register clears it.
    let mut f = dmc_machine(0x8F, 0);
    f.run_cycles(5_000);
    assert_eq!(f.peek(0x4015) & 0x80, 0x80);
}

#[test]
fn a_direct_load_sets_the_delta_level() {
    let mut m = with(&[(0x4011, 0x55)]);
    m.run_cycles(100);
    assert_eq!(m.channel_levels()[4], 0x55);
    let mut high = with(&[(0x4011, 0xFF)]);
    high.run_cycles(100);
    assert_eq!(high.channel_levels()[4], 0x7F);
}

#[test]
fn a_sample_read_costs_the_cpu_four_cycles() {
    // A looping sample at the fastest rate reads a byte every 432 cycles. A
    // delay loop of 4000 iterations takes 5 cycles each without the reads.
    let data = [0xFFu8; 0x100];
    let mut init = init_of(&[(0x4010, 0x4F), (0x4012, 0), (0x4013, 0), (0x4015, 0x10)]);
    init.extend([0xA2, 0x00, 0xA0, 0x10, 0xCA, 0xD0, 0xFD, 0x88, 0xD0, 0xFA]);
    init.extend(set(0x4000, 1));
    let f = file_with(&init, &[], 16666, 0, [0; 8], Some((0x4000, &data)));
    let mut m = Machine::new(&oscilla_nsf::Nsf::parse(&f).unwrap(), 48000).unwrap();
    let seen = watch(&mut m);
    m.run_cycles(100_000);
    let store = seen
        .borrow()
        .writes
        .iter()
        .find(|w| w.1 == 0x4000)
        .unwrap()
        .0;
    // 16 times 256 loop passes: 16 * (256 * 5 + 5) = 20560 cycles without reads.
    let base = 20_560 + 17;
    assert!(store > base + 4 * 30, "{store}");
    assert!(store < base + 4 * 80, "{store}");
}

#[test]
fn the_frame_counter_raises_its_flag_in_four_step_mode() {
    // Poll the status register for the flag, as a driver without interrupts
    // does, then read it again.
    let mut init = init_of(&[(0x4017, 0x00)]);
    init.extend([
        0xAD, 0x15, 0x40, 0x29, 0x40, 0xF0, 0xF9, 0xA9, 0x01, 0x85, 0x00, 0xAD, 0x15, 0x40, 0x85,
        0x01,
    ]);
    let mut m = machine(&init, &[]);
    m.run_cycles(20_000);
    assert_eq!(m.peek(0x0000), 0);
    m.run_cycles(12_000);
    assert_eq!(m.peek(0x0000), 1);
    // The first read cleared it before the second.
    assert_eq!(m.peek(0x0001) & 0x40, 0);
}

#[test]
fn five_step_mode_and_the_inhibit_bit_never_raise_the_flag() {
    for v in [0x80, 0x40, 0xC0] {
        let mut m = with(&[(0x4017, v)]);
        m.run_cycles(200_000);
        assert_eq!(m.peek(0x4015) & 0x40, 0, "{v:02x}");
    }
    let mut m = with(&[(0x4017, 0)]);
    m.run_cycles(200_000);
    assert_eq!(m.peek(0x4015) & 0x40, 0x40);
}

#[test]
fn a_write_in_five_step_mode_clocks_the_units_at_once() {
    // Index 3 loads 2. The write clocks one half-frame at once and the first
    // regular half-frame clock follows at 14916 cycles.
    let loads = [(0x4015, 1), (0x4000, 0x9F), (0x4002, 0x40), (0x4003, 0x18)];
    let mut five = init_of(&loads);
    five.extend(set(0x4017, 0x80));
    let mut m = machine(&five, &[]);
    m.run_cycles(17_000);
    assert_eq!(m.peek(0x4015) & 1, 0);
    let mut four = init_of(&loads);
    four.extend(set(0x4017, 0x00));
    let mut n = machine(&four, &[]);
    n.run_cycles(17_000);
    assert_eq!(n.peek(0x4015) & 1, 1);
}

/// The distance between the two plateaus of a square wave: the mean of the
/// samples above zero less the mean of those below, which leaves out the
/// ringing at the edges.
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
fn two_pulse_channels_mix_less_than_twice_as_loud_as_one() {
    let one = [(0x4015, 3), (0x4000, 0xBF), (0x4002, 0xFF), (0x4003, 0x09)];
    let both = [
        (0x4015, 3),
        (0x4000, 0xBF),
        (0x4002, 0xFF),
        (0x4003, 0x09),
        (0x4004, 0xBF),
        (0x4006, 0xFF),
        (0x4007, 0x09),
    ];
    let mut a = with(&one);
    let mut b = with(&both);
    assert!(a.set_highpass_hz(1.0));
    assert!(b.set_highpass_hz(1.0));
    let (pa, pb) = (swing(&mut a), swing(&mut b));
    // 95.52 / (8128 / 30 + 100) against 95.52 / (8128 / 15 + 100).
    let want = (95.52 / (8128.0 / 30.0 + 100.0)) / (95.52 / (8128.0 / 15.0 + 100.0));
    let ratio = pb / pa;
    assert!((ratio - want).abs() < 0.03, "{ratio} against {want}");
}

#[test]
fn the_channel_mask_removes_a_channel_from_the_mix_and_not_from_the_levels() {
    let both = [
        (0x4015, 3),
        (0x4000, 0xBF),
        (0x4002, 0xFF),
        (0x4003, 0x09),
        (0x4004, 0xBF),
        (0x4006, 0xFF),
        (0x4007, 0x09),
    ];
    let mut a = with(&both);
    let mut b = with(&both);
    assert!(a.set_highpass_hz(1.0));
    assert!(b.set_highpass_hz(1.0));
    b.set_channel_mask(0b00001);
    assert_eq!(b.channel_mask(), 1);
    let (pa, pb) = (swing(&mut a), swing(&mut b));
    assert!(pb < pa * 0.7, "{pb} against {pa}");
    let levels = sample(&mut b, 1, 8, 10_000);
    assert!(levels.iter().any(|&l| l > 0));
}

#[test]
fn reads_of_write_only_registers_return_the_bus() {
    // LDA $4000 returns the high byte of the address, which was on the bus.
    let mut m = machine(&[0xAD, 0x00, 0x40, 0x85, 0x00], &[]);
    m.run_cycles(100);
    assert_eq!(m.peek(0), 0x40);
}

/// The strength of the tone at `hz` in a mono stretch of samples.
fn tone(samples: &[f64], hz: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / 48000.0;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, s) in samples.iter().enumerate() {
        re += s * (w * i as f64).cos();
        im += s * (w * i as f64).sin();
    }
    2.0 * (re * re + im * im).sqrt() / samples.len() as f64
}

fn fundamental(period: u8) -> f64 {
    let mut m = with(&[
        (0x4015, 1),
        (0x4000, 0xBF),
        (0x4002, period),
        (0x4003, 0x08),
    ]);
    let out = render(&mut m, 24000);
    let mono: Vec<f64> = out[2 * 12000..]
        .iter()
        .step_by(2)
        .map(|&v| f64::from(v))
        .collect();
    let hz = 1_789_772.7272727273 / (16.0 * (f64::from(period) + 1.0));
    tone(&mono, hz)
}

#[test]
fn the_console_low_pass_takes_the_top_off_the_highest_tones() {
    // A square wave at 10 kHz sits below the 14 kHz corner by 0.73 of an octave,
    // so its fundamental keeps 0.81 of the strength that a square wave at
    // 1.5 kHz has.
    let high = fundamental(10);
    let low = fundamental(73);
    let ratio = high / low;
    assert!((ratio - 0.81).abs() < 0.04, "{ratio}");
}

#[test]
fn a_step_that_lands_on_a_frame_event_is_not_lost() {
    // At period 0 the triangle steps on every cycle, so a step coincides with
    // each event of the frame counter. After a whole number of 32-step turns
    // the level is where it was.
    let mut m = with(&[(0x4015, 4), (0x4008, 0xFF), (0x400A, 0), (0x400B, 0x08)]);
    m.run_cycles(10_000);
    let a = m.channel_levels()[2];
    m.run_cycles(32 * 600);
    assert!(m.cycles() < 29_500);
    assert_eq!(m.channel_levels()[2], a);
}
