//! The data that a player can show: the state of each channel and the recent
//! output of each channel.

mod common;

use common::*;
use oscilla_gb::{Machine, Voice, VoiceKind};

fn machine(init: &[u8], play: &[u8]) -> Machine {
    Machine::new(&gbs(init, play), 44_100).unwrap()
}

/// NR52 on, both outputs at full volume, every channel to both sides.
fn power() -> Vec<u8> {
    [set(0x26, 0x80), set(0x24, 0x77), set(0x25, 0xFF)].concat()
}

/// Pulse 1 at period 0x700 (131072 / 256 = 512 Hz), volume 15, duty 2, with
/// the sweep off.
fn pulse1() -> Vec<u8> {
    [
        set(0x10, 0x00),
        set(0x11, 0x80),
        set(0x12, 0xF0),
        set(0x13, 0x00),
        set(0x14, 0x87),
    ]
    .concat()
}

/// Pulse 2 at 512 Hz, volume 15, duty 1.
fn pulse2() -> Vec<u8> {
    [
        set(0x16, 0x40),
        set(0x17, 0xF0),
        set(0x18, 0x00),
        set(0x19, 0x87),
    ]
    .concat()
}

/// Sample `j` of a triangle that climbs from 0 to 15 and falls back.
const fn triangle(j: usize) -> u8 {
    if j < 16 { j as u8 } else { (31 - j) as u8 }
}

/// The triangle in wave RAM, two samples to a byte, the first in the high
/// nibble.
const TRIANGLE: [u8; 16] = {
    let mut ram = [0; 16];
    let mut i = 0;
    while i < 16 {
        ram[i] = triangle(2 * i) << 4 | triangle(2 * i + 1);
        i += 1;
    }
    ram
};

/// Wave RAM filled with `ram`, then the wave channel at period 0x700
/// (65536 / 256 = 256 Hz) with the output level `code`.
fn wave(ram: [u8; 16], code: u8) -> Vec<u8> {
    let mut s = set(0x1A, 0x00);
    for (i, b) in ram.iter().enumerate() {
        s.extend(set(0x30 + i as u8, *b));
    }
    s.extend(
        [
            set(0x1A, 0x80),
            set(0x1C, code << 5),
            set(0x1D, 0x00),
            set(0x1E, 0x87),
        ]
        .concat(),
    );
    s
}

/// The noise channel at volume 12 with the clock and width of `nr43`.
fn noise(nr43: u8) -> Vec<u8> {
    [set(0x21, 0xC0), set(0x22, nr43), set(0x23, 0x80)].concat()
}

fn all_four() -> Vec<u8> {
    [power(), pulse1(), pulse2(), wave(TRIANGLE, 1), noise(0x11)].concat()
}

fn crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

fn scope(m: &Machine, voice: usize, frames: usize) -> Vec<f32> {
    let mut out = vec![1f32; frames];
    assert!(m.scope(voice, &mut out));
    out
}

fn silent(samples: &[f32]) -> bool {
    samples.iter().all(|v| *v == 0.0)
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0f32, |p, v| p.max(v.abs()))
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|v| v * v).sum()
}

#[test]
fn the_voices_are_the_four_channels_in_the_order_of_the_mask() {
    let m = machine(&power(), &[]);
    assert_eq!(m.voice_count(), 4);
    let names: Vec<&str> = (0..4).map(|i| m.voice(i).unwrap().name).collect();
    assert_eq!(names, ["Pulse 1", "Pulse 2", "Wave", "Noise"]);
    for i in 0..4 {
        assert_eq!(m.voice(i).unwrap().mask_bit, i as u32);
    }
    let kinds: Vec<VoiceKind> = (0..4).map(|i| m.voice(i).unwrap().kind).collect();
    assert_eq!(
        kinds,
        [
            VoiceKind::Tone,
            VoiceKind::Tone,
            VoiceKind::Tone,
            VoiceKind::Noise
        ]
    );
    assert!(m.voice(4).is_none());
    let mut out = [1f32; 8];
    assert!(!m.scope(4, &mut out));
    assert_eq!(out, [1.0; 8]);
}

#[test]
fn each_channel_reports_its_pitch_volume_and_instrument() {
    let mut m = machine(&all_four(), &[]);
    render(&mut m, 4410);
    let v: Vec<Voice> = (0..4).map(|i| m.voice(i).unwrap()).collect();
    for p in &v[..2] {
        assert!(p.key_on && p.active);
        assert!((p.frequency_hz - 512.0).abs() < 1e-3, "{}", p.frequency_hz);
        assert_eq!((p.volume, p.envelope), (1.0, 1.0));
        assert_eq!((p.left, p.right), (1.0, 1.0));
        assert_eq!(p.phase, 0);
    }
    assert_eq!((v[0].instrument, v[1].instrument), (2, 1));
    assert!(v[2].key_on && v[2].active);
    assert!(
        (v[2].frequency_hz - 256.0).abs() < 1e-3,
        "{}",
        v[2].frequency_hz
    );
    assert_eq!((v[2].volume, v[2].envelope), (1.0, -1.0));
    assert!(v[2].instrument >= 0);
    assert!(v[3].key_on && v[3].active);
    assert_eq!(v[3].frequency_hz, 0.0);
    assert!((v[3].volume - 12.0 / 15.0).abs() < 1e-6, "{}", v[3].volume);
    assert_eq!(v[3].envelope, v[3].volume);
    assert_eq!(v[3].instrument, 0);
}

#[test]
fn the_sweep_moves_the_pitch_of_pulse_1() {
    // Pace 1, step 1, subtracting: the period shrinks by half each 128 Hz
    // tick, so the pitch falls. The init routine is done well before the
    // first tick.
    let init = [
        power(),
        set(0x10, 0x19),
        set(0x12, 0xF0),
        set(0x13, 0x00),
        set(0x14, 0x84),
    ]
    .concat();
    let mut m = machine(&init, &[]);
    m.run_cycles(2000);
    let start = m.voice(0).unwrap().frequency_hz;
    m.run_cycles(4_194_304 / 20);
    let later = m.voice(0).unwrap().frequency_hz;
    assert!((start - 131_072.0 / 1024.0).abs() < 1e-3, "{start}");
    assert!(later < start, "{later} {start}");
}

#[test]
fn the_wave_level_sets_the_volume_and_equal_wave_ram_gives_equal_numbers() {
    let voice = |ram: [u8; 16], code: u8| {
        let mut m = machine(&[power(), wave(ram, code)].concat(), &[]);
        m.run_cycles(100_000);
        m.voice(2).unwrap()
    };
    let volumes: Vec<f32> = (0..4).map(|code| voice(TRIANGLE, code).volume).collect();
    assert_eq!(volumes, [0.0, 1.0, 0.5, 0.25]);
    let mute = voice(TRIANGLE, 0);
    assert!(mute.key_on && !mute.active);
    assert_eq!(mute.envelope, -1.0);
    assert_eq!(voice(TRIANGLE, 1).instrument, voice(TRIANGLE, 3).instrument);
    let mut other = TRIANGLE;
    other[5] ^= 0x10;
    assert_ne!(voice(TRIANGLE, 1).instrument, voice(other, 1).instrument);
    assert!(voice(other, 1).instrument >= 0);
}

#[test]
fn the_noise_width_is_its_instrument() {
    for (nr43, width) in [(0x11, 0), (0x19, 1)] {
        let mut m = machine(&[power(), noise(nr43)].concat(), &[]);
        m.run_cycles(100_000);
        let v = m.voice(3).unwrap();
        assert_eq!(v.instrument, width);
        assert!(v.active);
    }
}

#[test]
fn a_channel_that_does_not_sound_has_no_volume() {
    // NR22 = 0 turns the DAC off, and the trigger leaves the channel off.
    let dac_off = [power(), set(0x17, 0x00), set(0x19, 0x80)].concat();
    // A length of 63 leaves one count, which the 256 Hz clock removes.
    let expired = [
        power(),
        set(0x16, 0x80 | 63),
        set(0x17, 0xF0),
        set(0x19, 0xC7),
    ]
    .concat();
    let unit_off = [power(), pulse2(), set(0x26, 0x00)].concat();
    for init in [&dac_off, &expired, &unit_off] {
        let mut m = machine(init, &[]);
        m.run_cycles(4_194_304 / 50);
        let v = m.voice(1).unwrap();
        assert!(!v.key_on && !v.active);
        assert_eq!((v.volume, v.left, v.right), (0.0, 0.0, 0.0));
        assert!(silent(&scope(&m, 1, 64)));
    }
    // The envelope keeps its value after the length runs out.
    let mut m = machine(&expired, &[]);
    m.run_cycles(4_194_304 / 50);
    assert_eq!(m.voice(1).unwrap().envelope, 1.0);
    // An envelope that runs down to 0 leaves the status bit on.
    let faded = [power(), set(0x17, 0xF1), set(0x18, 0x00), set(0x19, 0x87)].concat();
    let mut m = machine(&faded, &[]);
    m.run_cycles(4_194_304 / 2);
    let v = m.voice(1).unwrap();
    assert!(v.key_on && !v.active);
    assert_eq!((v.volume, v.envelope), (0.0, 0.0));
}

#[test]
fn nr51_routes_a_channel_and_nr50_stays_out_of_it() {
    // Pulse 2 to the left only, at the lowest master volume.
    let init = [power(), pulse2(), set(0x25, 0x20), set(0x24, 0x00)].concat();
    let mut m = machine(&init, &[]);
    render(&mut m, 4410);
    let v = m.voice(1).unwrap();
    assert_eq!((v.volume, v.left, v.right), (1.0, 1.0, 0.0));
    // One side of two halves the scope.
    assert!((peak(&scope(&m, 1, 2048)) - 0.5).abs() < 1e-6);
}

#[test]
fn the_scope_follows_a_tone_and_a_muted_voice_still_shows() {
    let mut m = machine(&[power(), pulse2()].concat(), &[]);
    render(&mut m, 10_000);
    m.set_channel_mask(0);
    render(&mut m, 1000);
    let s = scope(&m, 1, 4096);
    // 512 Hz over 4,096 frames of 44,100 Hz: about 95 crossings.
    let n = crossings(&s);
    assert!((92..=98).contains(&n), "{n}");
    assert!(s.iter().any(|v| *v > 0.99) && s.iter().any(|v| *v < -0.99));
    assert!(silent(&scope(&m, 0, 256)));
}

#[test]
fn the_wave_scope_shows_wave_ram_at_its_output_level() {
    let s = |code: u8| {
        let mut m = machine(&[power(), wave(TRIANGLE, code)].concat(), &[]);
        render(&mut m, 8000);
        scope(&m, 2, 4096)
    };
    // Samples 0 to 15 swing 15 each way, once for each side: 30 of 30.
    assert!((peak(&s(1)) - 1.0).abs() < 1e-6);
    assert!((peak(&s(2)) - 14.0 / 30.0).abs() < 1e-6);
    assert!((peak(&s(3)) - 6.0 / 30.0).abs() < 1e-6);
    assert!(silent(&s(0)));
    // 256 Hz over 4,096 frames: about 48 crossings.
    let n = crossings(&s(1));
    assert!((44..=52).contains(&n), "{n}");
}

#[test]
fn a_channel_alone_has_the_sign_of_the_output() {
    let mut m = machine(&[power(), pulse2()].concat(), &[]);
    let out = render(&mut m, 20_000);
    let s = scope(&m, 1, 2048);
    let left: Vec<f32> = out.iter().step_by(2).map(|v| f32::from(*v)).collect();
    // The scope runs a few frames ahead of the frames handed out, so its
    // older half is set against the output at lags of up to 24 frames either
    // way. The strongest match is in phase.
    let end = left.len() - 2048;
    let s = &s[..1024];
    let r = |lag: usize| {
        let w = &left[end - 24 + lag..][..1024];
        let dot: f32 = s.iter().zip(w).map(|(a, b)| a * b).sum();
        dot / (energy(s) * energy(w)).sqrt()
    };
    let strongest = (0..=48)
        .map(r)
        .fold(0f32, |b, x| if x.abs() > b.abs() { x } else { b });
    assert!(strongest > 0.7, "{strongest}");
}

#[test]
fn a_copy_a_restore_and_a_song_change_start_with_an_empty_scope() {
    let mut m = machine(&[power(), pulse2()].concat(), &[]);
    render(&mut m, 8000);
    assert!(!silent(&scope(&m, 1, 512)));
    assert!(silent(&scope(&m.clone(), 1, 512)));
    let snapshot = m.snapshot();
    render(&mut m, 1000);
    m.restore(&snapshot).unwrap();
    assert!(silent(&scope(&m, 1, 512)));
    render(&mut m, 2000);
    assert!(!silent(&scope(&m, 1, 512)));
    assert!(m.select_song(1));
    assert!(silent(&scope(&m, 1, 4096)));
}

#[test]
fn reading_the_voices_and_scopes_leaves_the_sound_alone() {
    let play = [set(0x18, 0x10), set(0x19, 0x86)].concat();
    let mut a = machine(&all_four(), &play);
    let mut b = machine(&all_four(), &play);
    let mut out = vec![0f32; 4096];
    for _ in 0..20 {
        let x = render(&mut a, 2205);
        for i in 0..4 {
            assert!(b.voice(i).is_some());
            assert!(b.scope(i, &mut out));
        }
        assert_eq!(x, render(&mut b, 2205));
    }
}
