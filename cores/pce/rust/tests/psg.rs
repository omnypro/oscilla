//! The sound generator, checked against the manual's formulas.

use oscilla_pce::psg::{Psg, TICK_HZ};

fn write(p: &mut Psg, channel: u8, register: u8, value: u8) {
    p.write(0, channel);
    p.write(register, value);
}

/// Writes a 32-word waveform to a channel (it must be off).
fn wave(p: &mut Psg, channel: u8, words: &[u8; 32]) {
    write(p, channel, 4, 0x40);
    write(p, channel, 4, 0x00);
    for w in words {
        write(p, channel, 6, *w);
    }
}

fn square() -> [u8; 32] {
    let mut w = [0u8; 32];
    for v in &mut w[..16] {
        *v = 31;
    }
    w
}

/// Turns a channel on at full level on both sides.
fn open(p: &mut Psg, channel: u8, frequency: u16) {
    p.write(1, 0xFF);
    write(p, channel, 2, frequency as u8);
    write(p, channel, 3, (frequency >> 8) as u8);
    write(p, channel, 5, 0xFF);
    write(p, channel, 4, 0x9F);
}

/// The number of level changes in `ticks` ticks.
fn changes(p: &mut Psg, ticks: u64) -> usize {
    let mut n = 0;
    p.run(ticks, &mut |_, _, _| n += 1);
    n
}

#[test]
fn a_waveform_repeats_at_the_manuals_frequency() {
    // f = fmaster / (2 * 32 * F): 440.45 Hz for F = 254, as in the manual's
    // scale table. A square wave changes twice in a cycle.
    let mut p = Psg::new();
    wave(&mut p, 0, &square());
    open(&mut p, 0, 254);
    let n = changes(&mut p, TICK_HZ);
    assert!((879..=883).contains(&n), "{n}");
}

#[test]
fn the_scale_table_value_for_a_low_note_holds() {
    // C1 is R3 = 0x0D and R2 = 0x5D: F = 3421, 32.70 Hz.
    let mut p = Psg::new();
    wave(&mut p, 0, &square());
    open(&mut p, 0, 0x0D5D);
    let n = changes(&mut p, TICK_HZ * 2);
    assert!((130..=132).contains(&n), "{n}");
}

#[test]
fn the_amplitude_registers_step_by_the_manuals_decibels() {
    let level_with = |main: u8, balance: u8, amplitude: u8| {
        let mut p = Psg::new();
        p.write(1, main);
        write(&mut p, 0, 5, balance);
        write(&mut p, 0, 4, 0xC0 | amplitude);
        write(&mut p, 0, 6, 31);
        p.level()
    };
    let (full, _) = level_with(0xFF, 0xFF, 31);
    assert_eq!(full, 31 * 65536);
    // One step of the channel amplitude is 1.5 dB, a step of the main level
    // or the balance 3 dB.
    let ratio = |a: i32| f64::from(a) / f64::from(full);
    let db = |r: f64| 20.0 * r.log10();
    assert!((db(ratio(level_with(0xFF, 0xFF, 30).0)) + 1.5).abs() < 0.02);
    assert!((db(ratio(level_with(0xFF, 0xFF, 29).0)) + 3.0).abs() < 0.02);
    assert!((db(ratio(level_with(0xEF, 0xFF, 31).0)) + 3.0).abs() < 0.02);
    assert!((db(ratio(level_with(0xFF, 0xDF, 31).0)) + 6.0).abs() < 0.02);
    // The sides are independent.
    let (l, r) = level_with(0x8F, 0xFF, 31);
    assert!(l < r);
}

#[test]
fn a_channel_is_silent_when_the_steps_of_three_decibels_pass_fifteen() {
    let level_with = |main: u8, balance: u8| {
        let mut p = Psg::new();
        p.write(1, main);
        write(&mut p, 0, 5, balance);
        write(&mut p, 0, 4, 0xDF);
        write(&mut p, 0, 6, 31);
        p.level().0
    };
    // 7 + 8 = 15 steps is heard, 16 is not.
    assert!(level_with(0x80, 0x70 | 0x0F) > 0);
    assert_eq!(level_with(0x80, 0x60 | 0x0F), 0);
    assert_eq!(level_with(0x00, 0x00), 0);
}

#[test]
fn the_waveform_register_writes_in_order_and_the_counter_resets() {
    let mut p = Psg::new();
    let mut words = [0u8; 32];
    for (i, w) in words.iter_mut().enumerate() {
        *w = i as u8;
    }
    wave(&mut p, 0, &words);
    // Direct D/A mode resets the counter and takes the data as output.
    write(&mut p, 0, 5, 0xFF);
    p.write(1, 0xFF);
    write(&mut p, 0, 4, 0xDF);
    write(&mut p, 0, 6, 20);
    assert_eq!(p.words()[0], 20);
    // A write in the mixing mode does not change the waveform.
    write(&mut p, 0, 4, 0x9F);
    write(&mut p, 0, 6, 1);
    write(&mut p, 0, 4, 0x1F);
    assert_eq!(p.words()[0], 0);
    write(&mut p, 0, 4, 0x40);
    write(&mut p, 0, 4, 0x00);
    write(&mut p, 0, 6, 9);
    write(&mut p, 0, 4, 0x9F);
    write(&mut p, 0, 4, 0x40);
    write(&mut p, 0, 4, 0x80);
    assert_eq!(p.words()[0], 9);
}

#[test]
fn the_mask_keeps_a_channel_out_of_the_output() {
    let mut p = Psg::new();
    open(&mut p, 1, 100);
    write(&mut p, 1, 6, 0);
    write(&mut p, 1, 4, 0xDF);
    write(&mut p, 1, 6, 31);
    assert!(p.level().0 > 0);
    p.set_mask(0b101);
    assert_eq!(p.level(), (0, 0));
    assert_eq!(p.mask(), 0b101);
}

#[test]
fn the_noise_clock_follows_the_complement_of_its_register() {
    // Noise frequency 27: the complement is 4, so the register shifts every
    // 64 * 4 = 256 ticks.
    let mut p = Psg::new();
    p.write(1, 0xFF);
    write(&mut p, 4, 5, 0xFF);
    write(&mut p, 4, 7, 0x80 | 27);
    write(&mut p, 4, 4, 0x9F);
    let mut times = Vec::new();
    p.run(256 * 2000, &mut |t, _, _| times.push(t));
    assert!(times.iter().all(|t| t % 256 == 0));
    // The bit is random, so about half of the shifts change the level.
    assert!((700..1300).contains(&times.len()), "{}", times.len());
    let mut q = Psg::new();
    q.write(1, 0xFF);
    write(&mut q, 4, 5, 0xFF);
    write(&mut q, 4, 7, 0x80 | 27);
    write(&mut q, 4, 4, 0x9F);
    let mut again = Vec::new();
    q.run(256 * 2000, &mut |t, _, _| again.push(t));
    assert_eq!(times, again);
}

#[test]
fn noise_does_not_exist_on_the_first_four_channels() {
    let mut p = Psg::new();
    p.write(1, 0xFF);
    write(&mut p, 0, 5, 0xFF);
    write(&mut p, 0, 7, 0x80 | 27);
    write(&mut p, 0, 4, 0x9F);
    assert_eq!(changes(&mut p, 256 * 200), 0);
}

#[test]
fn the_lfo_adds_channel_two_to_the_frequency_of_channel_one() {
    // Channel 2 holds the word 0x1F, which adds 15 to channel 1's frequency
    // with no shift. The period of channel 1 grows from 200 to 215 ticks.
    let run = |lfo: bool| {
        let mut p = Psg::new();
        wave(&mut p, 0, &square());
        open(&mut p, 0, 200);
        wave(&mut p, 1, &[0x1F; 32]);
        write(&mut p, 1, 5, 0x00);
        write(&mut p, 1, 2, 0xFF);
        write(&mut p, 1, 3, 0x0F);
        write(&mut p, 1, 4, 0x9F);
        p.write(8, 255);
        p.write(9, if lfo { 0x01 } else { 0x00 });
        changes(&mut p, TICK_HZ)
    };
    let plain = run(false);
    let modulated = run(true);
    let ratio = modulated as f64 / plain as f64;
    assert!((ratio - 200.0 / 215.0).abs() < 0.01, "{ratio}");
}

#[test]
fn the_lfo_shifts_by_two_or_four_bits_for_the_other_settings() {
    let run = |control: u8| {
        let mut p = Psg::new();
        wave(&mut p, 0, &square());
        open(&mut p, 0, 400);
        wave(&mut p, 1, &[0x11; 32]);
        write(&mut p, 1, 5, 0x00);
        write(&mut p, 1, 2, 0xFF);
        write(&mut p, 1, 3, 0x0F);
        write(&mut p, 1, 4, 0x9F);
        p.write(8, 255);
        p.write(9, control);
        changes(&mut p, TICK_HZ) as f64
    };
    // The word 0x11 adds 1, 4 or 16 ticks to the period of 400.
    let base = run(0);
    assert!((run(1) / base - 400.0 / 401.0).abs() < 0.005);
    assert!((run(2) / base - 400.0 / 404.0).abs() < 0.005);
    assert!((run(3) / base - 400.0 / 416.0).abs() < 0.005);
}

#[test]
fn resetting_the_lfo_stops_the_modulation() {
    let mut p = Psg::new();
    wave(&mut p, 0, &square());
    open(&mut p, 0, 200);
    wave(&mut p, 1, &[0x1F; 32]);
    write(&mut p, 1, 5, 0x00);
    write(&mut p, 1, 2, 0xFF);
    write(&mut p, 1, 3, 0x0F);
    write(&mut p, 1, 4, 0x9F);
    p.write(8, 255);
    p.write(9, 0x81);
    let held = changes(&mut p, TICK_HZ);
    let mut q = Psg::new();
    wave(&mut q, 0, &square());
    open(&mut q, 0, 200);
    assert_eq!(held, changes(&mut q, TICK_HZ));
}
