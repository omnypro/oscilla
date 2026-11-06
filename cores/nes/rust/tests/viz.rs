//! The data that a player can show: the state of each voice and the recent
//! output of each voice.

mod common;

use common::*;
use oscilla_nes::{Machine, NTSC_CLOCK, VoiceKind};
use oscilla_nsf::{Nsf, chips};

const CPU: f64 = NTSC_CLOCK.0 as f64 / NTSC_CLOCK.1 as f64;

fn init_of(stores: &[(u16, u8)]) -> Vec<u8> {
    stores.iter().flat_map(|&(a, v)| set(a, v)).collect()
}

/// A machine at 48,000 Hz for a file with the chip byte `chip_bits`, whose
/// init routine makes `stores`.
fn with_chips(chip_bits: u8, stores: &[(u16, u8)]) -> Machine {
    let mut f = file(&init_of(stores), &[]);
    f[0x7B] = chip_bits;
    Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap()
}

/// Pulse 1 at 437 Hz: half duty, the length counter halted, full volume.
const NOTE: [(u16, u8); 4] = [
    (0x4015, 0x0F),
    (0x4000, 0xBF),
    (0x4002, 0xFF),
    (0x4003, 0x08),
];

fn crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

/// The zero crossings that a tone of `hz` makes in 4,096 frames of 48,000 Hz.
fn crossings_of(hz: f64) -> f64 {
    2.0 * hz * 4096.0 / 48000.0
}

fn assert_follows(m: &Machine, voice: usize, hz: f64) {
    let mut scope = vec![0f32; 4096];
    assert!(m.scope(voice, &mut scope));
    let n = crossings(&scope) as f64;
    let want = crossings_of(hz);
    assert!(
        (n - want).abs() <= 3.0,
        "voice {voice}: {n} crossings, {want}"
    );
    assert!(scope.iter().any(|s| *s > 0.1) && scope.iter().any(|s| *s < -0.1));
}

fn near(a: f32, b: f64) -> bool {
    (f64::from(a) - b).abs() < b * 0.001 + 1e-6
}

#[test]
fn the_voices_are_the_channels_of_the_chips_in_the_order_of_the_mask() {
    let plain = with_chips(0, &[]);
    assert_eq!(plain.voice_count(), 5);
    let names: Vec<&str> = (0..5).map(|i| plain.voice(i).unwrap().name).collect();
    assert_eq!(names, ["Pulse 1", "Pulse 2", "Triangle", "Noise", "DMC"]);
    assert_eq!(plain.voice(4).unwrap().mask_bit, 4);
    assert!(plain.voice(5).is_none());

    let vrc6 = with_chips(chips::VRC6, &[]);
    assert_eq!(vrc6.voice_count(), 8);
    assert_eq!(vrc6.voice(5).unwrap().name, "VRC6 Pulse 1");
    assert_eq!(vrc6.voice(7).unwrap().mask_bit, 7);

    let n163 = with_chips(chips::N163, &[]);
    assert_eq!(n163.voice_count(), 13);
    assert_eq!(n163.voice(5).unwrap().name, "N163 1");
    assert_eq!(n163.voice(5).unwrap().mask_bit, 11);

    let two = with_chips(chips::FDS | chips::VRC7, &[]);
    assert_eq!(two.voice_count(), 12);
    assert_eq!(two.voice(5).unwrap().name, "FDS");
    assert_eq!(two.voice(5).unwrap().mask_bit, 19);
    assert_eq!(two.voice(6).unwrap().name, "VRC7 1");
    assert_eq!(two.voice(6).unwrap().mask_bit, 23);
    assert!(two.voice(12).is_none());

    let all = with_chips(0x7F, &[]);
    assert_eq!(all.voice_count(), 34);
    for i in 0..34 {
        assert_eq!(all.voice(i).unwrap().mask_bit, i as u32);
    }
    assert_eq!(all.voice(8).unwrap().name, "5B 1");
    assert_eq!(all.voice(22).unwrap().name, "MMC5 PCM");
    assert_eq!(all.voice(33).unwrap().name, "VT02 PCM");
}

#[test]
fn a_pulse_reports_its_pitch_duty_and_volume() {
    let mut m = with_chips(0, &NOTE);
    render(&mut m, 2000);
    let v = m.voice(0).unwrap();
    assert_eq!(v.kind, VoiceKind::Tone);
    assert!(v.key_on && v.active);
    assert!(
        near(v.frequency_hz, CPU / (16.0 * 256.0)),
        "{}",
        v.frequency_hz
    );
    assert_eq!((v.volume, v.left, v.right), (1.0, 1.0, 1.0));
    assert_eq!(v.envelope, -1.0);
    assert_eq!((v.instrument, v.phase), (2, 0));
    // Pulse 2 has no length.
    let idle = m.voice(1).unwrap();
    assert!(!idle.key_on && !idle.active && idle.volume == 0.0);
}

#[test]
fn an_envelope_shows_and_a_muted_pulse_has_no_volume() {
    // Pulse 1 decays from 15 at the slowest rate, and pulse 2 has a period
    // below 8, which mutes it.
    let mut m = with_chips(
        0,
        &[
            (0x4015, 0x0F),
            (0x4000, 0x0F),
            (0x4002, 0xFF),
            (0x4003, 0x08),
            (0x4004, 0xBF),
            (0x4006, 0x05),
            (0x4007, 0x08),
        ],
    );
    render(&mut m, 4000);
    let v = m.voice(0).unwrap();
    assert!(v.envelope > 0.0 && v.envelope < 1.0, "{}", v.envelope);
    assert_eq!(v.volume, v.envelope);
    assert_eq!(v.instrument, 0);
    let muted = m.voice(1).unwrap();
    assert!(muted.key_on && !muted.active && muted.volume == 0.0);
    // Clearing the enable bits ends both lengths.
    let mut stores = NOTE.to_vec();
    stores.push((0x4015, 0x00));
    let mut off = with_chips(0, &stores);
    render(&mut off, 100);
    let v = off.voice(0).unwrap();
    assert!(!v.key_on && !v.active && v.volume == 0.0);
}

#[test]
fn the_triangle_the_noise_and_the_dmc_report_their_state() {
    let stores = [
        (0x4015, 0x1F),
        (0x4008, 0xFF),
        (0x400A, 0xFF),
        (0x400B, 0x08),
        (0x400C, 0x3A),
        (0x400E, 0x85),
        (0x400F, 0x08),
        (0x4010, 0x4F),
        (0x4011, 0x40),
        (0x4012, 0x00),
        (0x4013, 0x01),
        (0x4015, 0x1F),
    ];
    // The sample sits at 0xC000: bits that alternate up and down.
    let f = file_with(
        &init_of(&stores),
        &[],
        16666,
        0,
        [0; 8],
        Some((0x4000, &[0x55; 17])),
    );
    let mut m = Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap();
    render(&mut m, 6000);
    let t = m.voice(2).unwrap();
    assert!(t.key_on && t.active && t.volume == 1.0);
    assert!(
        near(t.frequency_hz, CPU / (32.0 * 256.0)),
        "{}",
        t.frequency_hz
    );
    assert_eq!((t.envelope, t.instrument), (-1.0, -1));
    let n = m.voice(3).unwrap();
    assert_eq!(n.kind, VoiceKind::Noise);
    assert!(n.key_on && n.active);
    assert!((n.volume - 10.0 / 15.0).abs() < 1e-6);
    assert_eq!((n.frequency_hz, n.instrument, n.envelope), (0.0, 1, -1.0));
    let d = m.voice(4).unwrap();
    assert_eq!(d.kind, VoiceKind::Sample);
    assert!(d.key_on && d.active);
    assert!((d.volume - 0.5).abs() < 0.05, "{}", d.volume);
    assert_eq!((d.frequency_hz, d.instrument), (0.0, -1));
    assert_follows(&m, 2, CPU / (32.0 * 256.0));
    let mut noise = vec![0f32; 1024];
    assert!(m.scope(3, &mut noise));
    assert!(noise.iter().any(|s| *s > 0.5) && noise.iter().any(|s| *s < -0.5));
    let mut dmc = vec![0f32; 1024];
    assert!(m.scope(4, &mut dmc));
    assert!(dmc.iter().all(|s| *s > 0.4 && *s < 0.6));
}

#[test]
fn a_vrc6_voice_reports_its_registers_and_the_halt_bit_silences_it() {
    let stores = [
        (0x9000, 0x7F),
        (0x9001, 0xFF),
        (0x9002, 0x80),
        (0xB000, 0x15),
        (0xB001, 63),
        (0xB002, 0x80),
    ];
    let mut m = with_chips(chips::VRC6, &stores);
    render(&mut m, 6000);
    let p = m.voice(5).unwrap();
    assert!(p.key_on && p.active && p.volume == 1.0);
    assert!(
        near(p.frequency_hz, CPU / (16.0 * 256.0)),
        "{}",
        p.frequency_hz
    );
    assert_eq!((p.instrument, p.envelope), (7, -1.0));
    let idle = m.voice(6).unwrap();
    assert!(!idle.key_on && !idle.active);
    let saw = m.voice(7).unwrap();
    assert!(saw.key_on && saw.active && saw.volume == 0.5);
    assert!(
        near(saw.frequency_hz, CPU / (14.0 * 64.0)),
        "{}",
        saw.frequency_hz
    );
    assert_follows(&m, 5, CPU / (16.0 * 256.0));

    let mut halted = stores.to_vec();
    halted.push((0x9003, 0x01));
    let mut m = with_chips(chips::VRC6, &halted);
    render(&mut m, 500);
    let p = m.voice(5).unwrap();
    assert!(p.key_on && !p.active && p.volume == 0.0);
}

#[test]
fn a_5b_voice_reports_its_tone_noise_volume_curve_and_envelope() {
    // A plays its tone, B its noise and C the envelope with both off.
    let regs = [
        (0, 100),
        (1, 0),
        (7, 0b10_1110),
        (8, 15),
        (9, 10),
        (10, 16),
        (11, 0x40),
        (13, 0x0E),
    ];
    let init: Vec<u8> = regs.iter().flat_map(|&(r, v)| s5b_reg(r, v)).collect();
    let mut m = s5b_machine(&init, &[]);
    render(&mut m, 6000);
    let a = m.voice(5).unwrap();
    assert_eq!(a.kind, VoiceKind::Tone);
    assert!(a.key_on && a.active && a.volume == 1.0);
    assert!(near(a.frequency_hz, CPU / 3200.0), "{}", a.frequency_hz);
    assert_eq!((a.envelope, a.instrument), (-1.0, -1));
    let b = m.voice(6).unwrap();
    assert_eq!(b.kind, VoiceKind::Noise);
    assert!(b.key_on && b.active && b.frequency_hz == 0.0);
    // Ten levels of 1.5 dB below the top.
    let db = 20.0 * b.volume.log10();
    assert!((db + 15.0).abs() < 0.01, "{db}");
    let c = m.voice(7).unwrap();
    assert_eq!(c.kind, VoiceKind::Tone);
    assert!(c.key_on && c.frequency_hz == 0.0);
    assert!((0.0..=1.0).contains(&c.envelope));
    assert_eq!(c.volume, c.envelope);
    assert_follows(&m, 5, CPU / 3200.0);
}

#[test]
fn an_n163_voice_reports_its_waveform_pitch_and_volume() {
    // Two channels: 8 plays a ramp of eight samples from address 0, and 7 a
    // square of eight samples from address 8.
    let mut init = n163_ram(0, &[0x20, 0x64, 0xA8, 0xEC, 0x00, 0xFF, 0x00, 0xFF]);
    init.extend(n163_ram(0x70, &[0x00, 0, 0x04, 0, 0xF8, 0, 0x08, 0x0A]));
    init.extend(n163_ram(0x78, &[0x00, 0, 0x04, 0, 0xF8, 0, 0x00, 0x1F]));
    let mut m = n163_machine(&init, &[]);
    render(&mut m, 4000);
    // 1789773 * 0x400 / (15 * 2 * 8 * 65536) is 116.5 Hz.
    let hz = CPU * 1024.0 / (15.0 * 2.0 * 8.0 * 65536.0);
    let eight = m.voice(12).unwrap();
    assert_eq!((eight.name, eight.mask_bit), ("N163 8", 18));
    assert!(eight.key_on && eight.active && eight.volume == 1.0);
    assert!(near(eight.frequency_hz, hz), "{}", eight.frequency_hz);
    assert_eq!(eight.envelope, -1.0);
    let seven = m.voice(11).unwrap();
    assert!((seven.volume - 10.0 / 15.0).abs() < 1e-6);
    assert!(eight.instrument >= 0 && seven.instrument >= 0);
    assert_ne!(eight.instrument, seven.instrument);
    // A channel that is not enabled reports nothing.
    let one = m.voice(5).unwrap();
    assert!(!one.key_on && !one.active);
    assert_eq!((one.frequency_hz, one.instrument), (0.0, -1));
    assert_follows(&m, 12, hz);

    // The same waveform from the same address has the same number.
    init = n163_ram(0, &[0x20, 0x64, 0xA8, 0xEC]);
    init.extend(n163_ram(0x70, &[0x00, 0, 0x04, 0, 0xF8, 0, 0x00, 0x0A]));
    init.extend(n163_ram(0x78, &[0x00, 0, 0x02, 0, 0xF8, 0, 0x00, 0x1F]));
    let mut m = n163_machine(&init, &[]);
    render(&mut m, 100);
    assert_eq!(m.voice(11).unwrap().instrument, eight.instrument);
    assert_eq!(m.voice(12).unwrap().instrument, eight.instrument);
}

/// The FDS's wave RAM filled with `wave`, playing at the frequency 0x100 with
/// a fixed gain of 32, then `more`.
fn fds_playing(wave: impl Fn(u16) -> u8, more: &[(u16, u8)]) -> Machine {
    let mut stores = vec![(0x4089, 0x80)];
    stores.extend((0..64).map(|i| (0x4040 + i, wave(i))));
    stores.extend([
        (0x4089, 0),
        (0x4080, 0x80 | 32),
        (0x4082, 0x00),
        (0x4083, 0x01),
    ]);
    stores.extend_from_slice(more);
    let mut m = with_chips(chips::FDS, &stores);
    render(&mut m, 4000);
    m
}

#[test]
fn the_fds_voice_reports_its_wave_gain_and_modulation() {
    let square = |i: u16| if i < 32 { 63 } else { 0 };
    let m = fds_playing(square, &[]);
    let v = m.voice(5).unwrap();
    assert_eq!((v.name, v.kind), ("FDS", VoiceKind::Tone));
    assert!(v.key_on && v.active && v.volume == 1.0);
    // 2^24 / (0x100 * 64) updates of 16 cycles: 109.2 Hz.
    assert!(
        near(v.frequency_hz, CPU * 256.0 / 4_194_304.0),
        "{}",
        v.frequency_hz
    );
    assert_eq!(v.envelope, -1.0);
    assert!(v.instrument >= 0 && !v.pitch_modulated);
    assert_follows(&m, 5, CPU * 256.0 / 4_194_304.0);
    assert_eq!(
        fds_playing(square, &[]).voice(5).unwrap().instrument,
        v.instrument
    );
    let saw = fds_playing(|i| i as u8, &[]).voice(5).unwrap();
    assert_ne!(saw.instrument, v.instrument);

    // The master volume of 2/3, and the volume envelope rising slowly from a
    // gain of 16.
    let m = fds_playing(square, &[(0x4089, 1), (0x4080, 0x80 | 16), (0x4080, 0x7F)]);
    let v = m.voice(5).unwrap();
    assert!((0.5..0.6).contains(&v.envelope), "{}", v.envelope);
    assert!(
        (v.volume - v.envelope * 2.0 / 3.0).abs() < 1e-6,
        "{}",
        v.volume
    );

    // The modulation unit, running with a gain, bends the pitch.
    let m = fds_playing(
        square,
        &[(0x4084, 0x80 | 16), (0x4086, 0x10), (0x4087, 0x00)],
    );
    assert!(m.voice(5).unwrap().pitch_modulated);
}

#[test]
fn a_vrc7_voice_reports_its_patch_stage_and_pitch() {
    // A custom patch that makes a plain sine with the sustain bit, F-number
    // 256 in octave 3.
    let regs = [
        (0x00, 0x01),
        (0x01, 0x21),
        (0x02, 0x3F),
        (0x03, 0x00),
        (0x04, 0xF0),
        (0x05, 0xF0),
        (0x06, 0x00),
        (0x07, 0x00),
        (0x30, 0x00),
        (0x10, 0x00),
        (0x20, 1 << 4 | 3 << 1 | 1),
    ];
    let init: Vec<u8> = regs
        .iter()
        .flat_map(|&(r, v)| [set(0x9010, r), set(0x9030, v)].concat())
        .collect();
    let mut m = vrc7_machine(&init, &[]);
    render(&mut m, 4000);
    let v = m.voice(5).unwrap();
    assert_eq!((v.name, v.kind), ("VRC7 1", VoiceKind::Fm));
    assert!(v.key_on && v.active && v.volume > 0.99);
    assert_eq!((v.instrument, v.phase), (0, 2));
    let hz = 2048.0 * 3_579_545.0 / 72.0 / 524_288.0;
    assert!(near(v.frequency_hz, hz), "{}", v.frequency_hz);
    assert!(!v.pitch_modulated);
    let idle = m.voice(6).unwrap();
    assert!(!idle.key_on && !idle.active && idle.envelope == 0.0);
    assert_eq!(idle.phase, 3);
    assert_follows(&m, 5, hz);
}

#[test]
fn the_mmc5_and_the_vt02_voices_report_their_state() {
    let mut m = mmc5_machine(
        &init_of(&[
            (0x5015, 0x03),
            (0x5000, 0x3F),
            (0x5002, 0xFF),
            (0x5003, 0x08),
            (0x5011, 0x80),
        ]),
        &[],
    );
    render(&mut m, 2000);
    let p = m.voice(5).unwrap();
    assert!(p.key_on && p.active && p.volume == 1.0);
    assert!(near(p.frequency_hz, CPU / (16.0 * 256.0)));
    assert_eq!(p.instrument, 0);
    let pcm = m.voice(7).unwrap();
    assert_eq!(pcm.kind, VoiceKind::Sample);
    assert!(pcm.active && (pcm.volume - 128.0 / 255.0).abs() < 1e-6);

    // The VT02 reads the duty setting 2 as 25%. With 0x4030 at 0x08 both
    // generators play, and at 0x04 neither does.
    let mut stores = NOTE.to_vec();
    stores.extend([
        (0x4035, 0x0F),
        (0x4020, 0xBF),
        (0x4022, 0xFF),
        (0x4023, 0x08),
        (0x4030, 0x08),
    ]);
    let mut m = vt02_with(&init_of(&stores), &[], None);
    render(&mut m, 2000);
    let three = m.voice(5).unwrap();
    assert_eq!((three.name, three.mask_bit), ("VT02 Pulse 3", 29));
    assert!(three.active && three.volume == 1.0);
    assert_eq!(three.instrument, 1);
    assert!(m.voice(0).unwrap().active);
    stores.push((0x4030, 0x04));
    let mut m = vt02_with(&init_of(&stores), &[], None);
    render(&mut m, 2000);
    assert!(!m.voice(0).unwrap().active && !m.voice(5).unwrap().active);
    assert!(m.voice(5).unwrap().key_on);
}

#[test]
fn the_scope_follows_a_tone_and_a_muted_voice_still_shows() {
    let mut m = with_chips(0, &NOTE);
    render(&mut m, 8000);
    m.set_channel_mask(!1);
    render(&mut m, 500);
    assert_follows(&m, 0, CPU / (16.0 * 256.0));
    let mut scope = vec![0f32; 4096];
    assert!(m.scope(0, &mut scope));
    let peak = scope.iter().fold(0f32, |a, s| a.max(s.abs()));
    assert!((peak - 0.9375).abs() < 0.01, "{peak}");
    // A voice that does not play is 0, and there is no voice 5.
    let mut idle = vec![1f32; 128];
    assert!(m.scope(1, &mut idle));
    assert!(idle.iter().all(|s| *s == 0.0));
    assert!(!m.scope(5, &mut idle));
}

#[test]
fn the_scope_resamples_to_another_output_rate() {
    let mut m = Machine::new(&nsf(&init_of(&NOTE), &[]), 192_000).unwrap();
    render(&mut m, 40_000);
    let mut scope = vec![0f32; 4096];
    assert!(m.scope(0, &mut scope));
    // 4,096 frames of 192 kHz are 21 ms: 18.6 crossings.
    let n = crossings(&scope);
    assert!((17..=21).contains(&n), "{n}");
    // Frames older than the 4,096 samples that the machine keeps are 0.
    let mut m = with_chips(0, &NOTE);
    render(&mut m, 10_000);
    let mut long = vec![1f32; 16_384];
    assert!(m.scope(0, &mut long));
    assert!(long[..4000].iter().all(|s| *s == 0.0));
    assert!(long[16_383] != 0.0);
}

#[test]
fn a_copy_has_an_empty_scope_and_the_sound_is_unchanged() {
    let mut m = with_chips(0, &NOTE);
    render(&mut m, 8000);
    let copy = m.clone();
    let mut scope = vec![1f32; 256];
    assert!(copy.scope(0, &mut scope));
    assert!(scope.iter().all(|s| *s == 0.0));
    let mut a = m.clone();
    let mut b = m;
    assert_eq!(render(&mut a, 4000), render(&mut b, 4000));
    // The copy's own scope fills as it plays.
    assert!(a.scope(0, &mut scope));
    assert!(scope.iter().any(|s| *s != 0.0));
}

#[test]
fn a_restore_and_a_song_change_keep_the_scope_running() {
    let mut m = with_chips(0, &NOTE);
    render(&mut m, 4000);
    let snap = m.snapshot();
    render(&mut m, 4000);
    m.restore(&snap).unwrap();
    let mut scope = vec![1f32; 256];
    assert!(m.scope(0, &mut scope));
    assert!(scope.iter().all(|s| *s == 0.0));
    render(&mut m, 6000);
    assert_follows(&m, 0, CPU / (16.0 * 256.0));
    // A new song starts the cycle count again, and the scope goes on with it.
    assert!(m.select_song(1));
    let mut fresh = vec![1f32; 256];
    assert!(m.scope(0, &mut fresh));
    assert!(fresh.iter().all(|s| *s == 0.0), "the old song's samples remain");
    render(&mut m, 6000);
    assert_follows(&m, 0, CPU / (16.0 * 256.0));
}

#[test]
fn reading_the_voices_and_scopes_does_not_change_the_sound() {
    let play = |look: bool| {
        let mut m = with_chips(0x7F, &NOTE);
        let mut out = Vec::new();
        let mut scope = vec![0f32; 1024];
        for _ in 0..20 {
            out.extend(render(&mut m, 512));
            if look {
                for i in 0..m.voice_count() {
                    m.voice(i).unwrap();
                    m.scope(i, &mut scope);
                }
            }
        }
        out
    };
    assert_eq!(play(false), play(true));
}
