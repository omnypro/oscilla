//! The data that a player can show: the state of each voice and the recent
//! output of each voice.

mod common;

use common::*;
use oscilla_genesis::{Machine, VoiceKind};

fn machine(bytes: &[u8]) -> Machine {
    let mut m = Machine::new(&parse(bytes), 44_100).unwrap();
    m.set_highpass_hz(0.0);
    m
}

/// A sine on channel 1 of the first YM2612 at about 440 Hz, keyed on, then a
/// wait.
fn fm_tone(n: u16) -> Vec<u8> {
    let mut s = Vec::new();
    let mut w = |reg: u8, val: u8| s.extend([0x52, reg, val]);
    for (offset, level) in [(0u8, 0u8), (4, 127), (8, 127), (12, 127)] {
        w(0x30 + offset, 0x01);
        w(0x40 + offset, level);
        w(0x50 + offset, 0x1F);
        w(0x60 + offset, 0);
        w(0x70 + offset, 0);
        w(0x80 + offset, 0x0F);
    }
    w(0xB0, 0x07);
    w(0xB4, 0xC0);
    w(0xA4, 4 << 3 | 4);
    w(0xA0, 0x3B);
    w(0x28, 0x10);
    s.extend(wait(n));
    s
}

/// The first SN76489 channel at 440.4 Hz and full volume.
fn psg_tone(n: u16) -> Vec<u8> {
    let mut s = vec![
        0x50,
        0x80 | (254 & 15) as u8,
        0x50,
        (254 >> 4) as u8,
        0x50,
        0x90,
    ];
    s.extend(wait(n));
    s
}

/// A user tone on the first YM2413's channel 1 at 440 Hz.
fn opll_tone(n: u16) -> Vec<u8> {
    let mut s = Vec::new();
    let mut w = |reg: u8, val: u8| s.extend([0x51, reg, val]);
    for (i, v) in [0x21u8, 0x21, 0x3F, 0x00, 0xF0, 0xF0, 0x0F, 0x0F]
        .iter()
        .enumerate()
    {
        w(i as u8, *v);
    }
    w(0x30, 0x00);
    w(0x10, 0x22);
    w(0x20, 0x19);
    s.extend(wait(n));
    s
}

fn opll_file(stream: &[u8]) -> Vec<u8> {
    let mut f = file(false, false, stream, None);
    f[0x10..0x14].copy_from_slice(&3_579_545u32.to_le_bytes());
    f
}

fn finished(mut s: Vec<u8>) -> Vec<u8> {
    s.push(0x66);
    s
}

fn crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

#[test]
fn the_voices_are_the_channels_of_the_chips_in_the_order_of_the_mask() {
    let both = machine(&file(true, true, &[0x66], None));
    assert_eq!(both.voice_count(), 10);
    let names: Vec<String> = (0..10).map(|i| both.voice(i).unwrap().name).collect();
    assert_eq!(names[0], "FM 1");
    assert_eq!(names[5], "FM 6");
    assert_eq!(names[6], "PSG Tone 1");
    assert_eq!(names[9], "PSG Noise");
    assert!(both.voice(10).is_none());
    assert_eq!(both.voice(9).unwrap().mask_bit, 9);
    assert_eq!(both.voice(9).unwrap().kind, VoiceKind::Noise);

    let mut dual = file(true, true, &[0x66], None);
    dual[0x0F] |= 0x40;
    dual[0x2F] |= 0x40;
    let dual = machine(&dual);
    assert_eq!(dual.voice_count(), 20);
    assert_eq!(dual.voice(10).unwrap().name, "FM 1 (2nd chip)");
    assert_eq!(dual.voice(10).unwrap().mask_bit, 10);
    assert_eq!(dual.voice(16).unwrap().mask_bit, 16);

    let opll = machine(&opll_file(&[0x66]));
    assert_eq!(opll.voice_count(), 9);
    assert_eq!(opll.voice(0).unwrap().mask_bit, 20);
    assert_eq!(opll.voice(8).unwrap().mask_bit, 28);
    assert_eq!(opll.voice(0).unwrap().name, "OPLL 1");

    let mut all = file(true, true, &[0x66], None);
    all[0x10..0x14].copy_from_slice(&3_579_545u32.to_le_bytes());
    assert_eq!(machine(&all).voice_count(), 19);
    assert_eq!(machine(&all).voice(10).unwrap().mask_bit, 20);
    assert_eq!(machine(&file(true, false, &[0x66], None)).voice_count(), 4);
}

#[test]
fn an_fm_channel_reports_its_pitch_envelope_and_algorithm() {
    let mut m = machine(&file(false, true, &finished(fm_tone(8000)), None));
    let mut out = vec![0i16; 2 * 6000];
    m.render_s16(&mut out);
    let v = m.voice(0).unwrap();
    assert_eq!(v.kind, VoiceKind::Fm);
    assert!(v.key_on && v.active);
    assert!((v.frequency_hz - 440.0).abs() < 3.0, "{}", v.frequency_hz);
    assert!(
        v.envelope > 0.95 && v.volume > 0.95,
        "{} {}",
        v.envelope,
        v.volume
    );
    assert_eq!((v.left, v.right), (v.volume, v.volume));
    assert_eq!(v.instrument, 7 * 8);
    assert_eq!(v.phase, 2);
    let idle = m.voice(1).unwrap();
    assert!(!idle.key_on && !idle.active);
}

#[test]
fn a_psg_tone_reports_its_pitch_and_the_noise_channel_its_kind() {
    let mut m = machine(&file(true, true, &finished(psg_tone(8000)), None));
    let mut out = vec![0i16; 2 * 6000];
    m.render_s16(&mut out);
    let t = m.voice(6).unwrap();
    assert_eq!(t.kind, VoiceKind::Tone);
    assert!(t.key_on && t.active && (t.volume - 1.0).abs() < 1e-6);
    assert!((t.frequency_hz - 440.4).abs() < 0.5, "{}", t.frequency_hz);
    assert_eq!(t.envelope, -1.0);
    let silent = m.voice(7).unwrap();
    assert!(!silent.key_on && !silent.active && silent.volume == 0.0);
    // Two steps of attenuation are 4 dB.
    let mut s = psg_tone(100);
    s.extend([0x50, 0x92]);
    s.extend(wait(100));
    let mut m = machine(&file(true, false, &finished(s), None));
    let mut out = vec![0i16; 2 * 300];
    m.render_s16(&mut out);
    let db = 20.0 * m.voice(0).unwrap().volume.log10();
    assert!((db + 4.0).abs() < 0.05, "{db}");
}

#[test]
fn a_ym2413_channel_reports_its_pitch_and_instrument() {
    let mut m = machine(&opll_file(&finished(opll_tone(20_000))));
    let mut out = vec![0i16; 2 * 10_000];
    m.render_s16(&mut out);
    let v = m.voice(0).unwrap();
    assert_eq!(v.kind, VoiceKind::Fm);
    assert!(v.key_on && v.active);
    assert!((v.frequency_hz - 440.0).abs() < 1.0, "{}", v.frequency_hz);
    assert_eq!(v.instrument, 0);
    assert!(v.volume > 0.9);
    assert!(!m.voice(1).unwrap().key_on);
}

#[test]
fn the_scopes_follow_each_chips_voices_and_a_muted_voice_still_shows() {
    let mut s = fm_tone(0);
    s.extend(psg_tone(0));
    s.extend(wait(20_000));
    let mut m = machine(&file(true, true, &finished(s), None));
    let mut out = vec![0i16; 2 * 10_000];
    m.render_s16(&mut out);
    m.set_channel_mask(0);
    m.render_s16(&mut out[..1000]);
    for voice in [0, 6] {
        let mut scope = vec![0f32; 4096];
        assert!(m.scope(voice, &mut scope));
        // 440 Hz over 4,096 frames of 44,100 Hz: about 82 crossings.
        let n = crossings(&scope);
        assert!((78..=86).contains(&n), "voice {voice}: {n}");
        assert!(scope.iter().any(|v| *v > 0.1) && scope.iter().any(|v| *v < -0.1));
    }
    let mut idle = vec![1f32; 64];
    assert!(m.scope(1, &mut idle));
    assert!(idle.iter().all(|v| *v == 0.0));
    assert!(!m.scope(10, &mut idle));
}

#[test]
fn a_ym2413_scope_follows_its_channel() {
    let mut m = machine(&opll_file(&finished(opll_tone(30_000))));
    let mut out = vec![0i16; 2 * 12_000];
    m.render_s16(&mut out);
    let mut scope = vec![0f32; 4096];
    assert!(m.scope(0, &mut scope));
    let n = crossings(&scope);
    assert!((78..=86).contains(&n), "{n}");
}

#[test]
fn a_copy_has_an_empty_scope_and_the_sound_is_unchanged() {
    let mut m = machine(&file(false, true, &finished(fm_tone(30_000)), None));
    let mut out = vec![0i16; 2 * 8000];
    m.render_s16(&mut out);
    let copy = m.clone();
    let mut scope = vec![1f32; 128];
    assert!(copy.scope(0, &mut scope));
    assert!(scope.iter().all(|v| *v == 0.0));
    let mut a = m.clone();
    let mut b = m;
    let (mut x, mut y) = (vec![0i16; 4000], vec![0i16; 4000]);
    a.render_s16(&mut x);
    b.render_s16(&mut y);
    assert_eq!(x, y);
}
