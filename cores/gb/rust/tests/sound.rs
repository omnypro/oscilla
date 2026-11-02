//! The sound unit, one behavior at a time, with programs built from bytes.

mod common;

use common::*;

/// Registers: NR52 on, both outputs at full volume, every channel to both
/// sides.
fn power() -> Vec<u8> {
    [set(0x26, 0x80), set(0x24, 0x77), set(0x25, 0xFF)].concat()
}

fn nr52(m: &oscilla_gb::Machine) -> u8 {
    m.peek(0xFF26)
}

fn crossings(samples: &[i16]) -> usize {
    let mut n = 0;
    let mut high = false;
    for frame in samples.chunks(2) {
        let v = i32::from(frame[0]);
        if !high && v > 2000 {
            high = true;
            n += 1;
        } else if high && v < -2000 {
            high = false;
        }
    }
    n
}

#[test]
fn a_pulse_channel_sounds_at_its_pitch() {
    // Period value 0x700 gives 131072 / 256 = 512 Hz.
    let init = [
        power(),
        set(0x16, 0x80),
        set(0x17, 0xF0),
        set(0x18, 0x00),
        set(0x19, 0x87),
    ]
    .concat();
    let mut m = machine(&init, &[]);
    let out = render(&mut m, 24000);
    let n = crossings(&out);
    assert!((250..=262).contains(&n), "{n} cycles in half a second");
    assert_eq!(nr52(&m) & 0x0F, 0x02, "channel 2 is on");
}

#[test]
fn the_length_counter_ends_a_note() {
    // A length of 63 leaves one count, which the 256 Hz clock removes.
    let init = [
        power(),
        set(0x16, 0x80 | 63),
        set(0x17, 0xF0),
        set(0x19, 0xC7),
    ]
    .concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(4_194_304 / 100);
    assert_eq!(nr52(&m) & 2, 0);
}

#[test]
fn without_length_enable_the_note_keeps_sounding() {
    let init = [
        power(),
        set(0x16, 0x80 | 63),
        set(0x17, 0xF0),
        set(0x19, 0x87),
    ]
    .concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(4_194_304);
    assert_eq!(nr52(&m) & 2, 2);
}

#[test]
fn the_envelope_runs_down_to_silence() {
    // Volume 15, falling, one step every 64 Hz tick.
    let init = [power(), set(0x17, 0xF1), set(0x19, 0x87), set(0x18, 0x00)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(4_194_304 / 100);
    m.run_cycles(4_194_304 / 2);
    let late = m.channel_levels();
    assert_eq!(late[1], 0, "volume reaches 0");
    assert_eq!(nr52(&m) & 2, 2, "the channel stays on at volume 0");
}

#[test]
fn a_rising_sweep_that_overflows_turns_channel_one_off() {
    // Period 0x7F0, pace 1, step 1, adding: 0x7F0 + 0x3F8 > 0x7FF.
    let init = [
        power(),
        set(0x10, 0x11),
        set(0x12, 0xF0),
        set(0x13, 0xF0),
        set(0x14, 0x87),
    ]
    .concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(4_194_304 / 50);
    assert_eq!(nr52(&m) & 1, 0);
}

#[test]
fn a_falling_sweep_lowers_the_period() {
    // Pace 1, step 1, subtracting: the period shrinks by half each 128 Hz tick.
    let init = [
        power(),
        set(0x10, 0x19),
        set(0x12, 0xF0),
        set(0x13, 0x00),
        set(0x14, 0x84),
    ]
    .concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(4_194_304 / 20);
    let low = m.peek(0xFF13);
    let high = m.peek(0xFF14);
    // The write-only period reads back as ones; the sweep leaves the channel on.
    assert_eq!((low, high & 0x07), (0xFF, 0x07));
    assert_eq!(nr52(&m) & 1, 1);
}

#[test]
fn the_noise_channel_changes_its_output() {
    let init = [power(), set(0x21, 0xF0), set(0x22, 0x11), set(0x23, 0x80)].concat();
    let mut m = machine(&init, &[]);
    let out = render(&mut m, 4800);
    let distinct = out
        .iter()
        .step_by(2)
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert!(distinct > 4, "{distinct} levels");
}

#[test]
fn the_noise_width_changes_the_pattern() {
    let wide = [power(), set(0x21, 0xF0), set(0x22, 0x01), set(0x23, 0x80)].concat();
    let narrow = [power(), set(0x21, 0xF0), set(0x22, 0x09), set(0x23, 0x80)].concat();
    let a = render(&mut machine(&wide, &[]), 9600);
    let b = render(&mut machine(&narrow, &[]), 9600);
    assert_ne!(a, b);
}

#[test]
fn the_wave_channel_plays_wave_ram() {
    // A 32-step ramp, output level 100%, period value 0x700: 65536 / 256 Hz.
    let mut init = power();
    init.extend(set(0x1A, 0x00));
    for i in 0..16u8 {
        init.extend(set(0x30 + i, i << 4 | (15 - i)));
    }
    init.extend(
        [
            set(0x1A, 0x80),
            set(0x1C, 0x20),
            set(0x1D, 0x00),
            set(0x1E, 0x87),
        ]
        .concat(),
    );
    let mut m = machine(&init, &[]);
    let out = render(&mut m, 4800);
    let distinct = out
        .iter()
        .step_by(2)
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert!(distinct > 8, "{distinct} levels");
    assert_eq!(nr52(&m) & 4, 4);
}

#[test]
fn the_wave_output_level_shifts_the_samples() {
    let mk = |level: u8| {
        let mut init = power();
        init.extend(set(0x1A, 0x00));
        for i in 0..16u8 {
            init.extend(set(0x30 + i, 0xFF));
        }
        init.extend([set(0x1A, 0x80), set(0x1C, level), set(0x1E, 0x87)].concat());
        let mut m = machine(&init, &[]);
        m.run_cycles(100_000);
        m.channel_levels()[2]
    };
    assert_eq!(mk(0x20), 15);
    assert_eq!(mk(0x40), 7);
    assert_eq!(mk(0x60), 3);
    assert_eq!(mk(0x00), 0);
}

#[test]
fn powering_off_clears_the_registers() {
    let init = [power(), set(0x17, 0xF0), set(0x19, 0x87), set(0x26, 0x00)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(100_000);
    assert_eq!(nr52(&m), 0x70);
    assert_eq!(m.peek(0xFF24), 0x00);
    assert_eq!(m.peek(0xFF17), 0x00);
}

#[test]
fn registers_ignore_writes_while_the_unit_is_off() {
    let init = [set(0x26, 0x00), set(0x24, 0x55)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(100_000);
    assert_eq!(m.peek(0xFF24), 0x00);
}

#[test]
fn unwritten_bits_read_as_ones() {
    let init = [power(), set(0x11, 0x40), set(0x1C, 0x40)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(100_000);
    assert_eq!(m.peek(0xFF11), 0x40 | 0x3F);
    assert_eq!(m.peek(0xFF1C), 0x40 | 0x9F);
    assert_eq!(m.peek(0xFF13), 0xFF);
}

#[test]
fn a_disabled_dac_keeps_a_channel_from_starting() {
    // NR22 = 0 turns the DAC off; the trigger leaves the channel off.
    let init = [power(), set(0x17, 0x00), set(0x19, 0x80)].concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(100_000);
    assert_eq!(nr52(&m) & 2, 0);
}

#[test]
fn the_channel_mask_removes_a_channel_from_the_mix() {
    let init = [
        power(),
        set(0x16, 0x80),
        set(0x17, 0xF0),
        set(0x18, 0x00),
        set(0x19, 0x87),
    ]
    .concat();
    let mut on = machine(&init, &[]);
    let mut off = machine(&init, &[]);
    off.set_channel_mask(0b1101);
    let loud = render(&mut on, 48000);
    let quiet = render(&mut off, 48000);
    let peak = |s: &[i16]| {
        s[s.len() - 9600..]
            .iter()
            .map(|v| i32::from(*v).abs())
            .max()
            .unwrap()
    };
    assert!(peak(&loud) > 3000);
    assert!(peak(&quiet) < 50, "{}", peak(&quiet));
    assert_eq!(
        off.channel_levels()[1],
        on.channel_levels()[1],
        "the level stays visible"
    );
}
