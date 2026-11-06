//! The data that a player can show: the state of each channel and the recent
//! output of each channel.

use oscilla_hes::Hes;
use oscilla_pce::Machine;

fn program(request: &[u8]) -> Vec<u8> {
    let mut bank = vec![0u8; 0x2000];
    bank[0x100..0x100 + request.len()].copy_from_slice(request);
    bank[0x1FF0] = 0x40;
    for v in [0x1FF6, 0x1FF8, 0x1FFA, 0x1FFC] {
        bank[v..v + 2].copy_from_slice(&[0xF0, 0xFF]);
    }
    let mut f = b"HESM".to_vec();
    f.extend([0, 0, 0x00, 0xE1, 0xFF, 0xF8, 0, 0, 1, 2, 3, 0]);
    f.extend(b"DATA");
    f.extend((bank.len() as u32).to_le_bytes());
    f.extend(0u32.to_le_bytes());
    f.extend([0; 4]);
    f.extend(bank);
    f
}

fn sta(address: u16, value: u8) -> Vec<u8> {
    vec![0xA9, value, 0x8D, address as u8, (address >> 8) as u8]
}

const PSG: u16 = 0x0800;

/// Channel `ch` (0 to 5) as a square wave of 16 words at 31 and 16 at 0, with
/// the frequency `f` and the amplitude register `amplitude`.
fn square(ch: u8, f: u16, amplitude: u8) -> Vec<u8> {
    let mut c = sta(PSG, ch);
    c.extend(sta(PSG + 5, 0xFF));
    c.extend(sta(PSG + 4, 0x40));
    c.extend(sta(PSG + 4, 0x00));
    for i in 0..32 {
        c.extend(sta(PSG + 6, if i < 16 { 31 } else { 0 }));
    }
    c.extend(sta(PSG + 2, f as u8));
    c.extend(sta(PSG + 3, (f >> 8) as u8));
    c.extend(sta(PSG + 4, 0x80 | amplitude));
    c
}

fn machine_with(code: &[u8], rate: i32) -> Machine {
    let mut c = sta(PSG + 1, 0xFF);
    c.extend(code);
    c.push(0x60);
    let mut m = Machine::new(&Hes::parse(&program(&c)).unwrap(), rate, 0).unwrap();
    m.set_highpass_hz(0.0);
    m
}

fn render(m: &mut Machine, frames: usize) {
    let mut out = vec![0i16; 2 * frames];
    m.render_s16(&mut out);
}

fn crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

#[test]
fn a_channel_reports_its_registers_and_its_gain() {
    let code = [square(0, 254, 31), square(1, 100, 29)].concat();
    let mut m = machine_with(&code, 44_100);
    render(&mut m, 2000);
    let a = m.voice_info(0);
    assert!(a.on && !a.direct && !a.noise);
    assert_eq!((a.frequency, a.amplitude), (254, 31));
    assert_eq!(a.gain, (65_536, 65_536));
    // Two steps of the amplitude register are 3 dB.
    let b = m.voice_info(1);
    assert_eq!(b.amplitude, 29);
    let ratio = f64::from(b.gain.0) / f64::from(a.gain.0);
    assert!((20.0 * ratio.log10() + 3.0).abs() < 0.05);
    // Equal waveforms have the same number.
    assert_eq!(a.wave_id, b.wave_id);
    assert_ne!(a.wave_id, m.voice_info(2).wave_id);
    assert!(!m.voice_info(2).on);
}

#[test]
fn noise_and_direct_mode_show_on_the_channels_that_have_them() {
    let mut code = sta(PSG, 4);
    code.extend(sta(PSG + 7, 0x80 | 10));
    code.extend(sta(PSG + 4, 0x9F));
    code.extend(sta(PSG, 0));
    code.extend(sta(PSG + 7, 0x80));
    code.extend(sta(PSG + 4, 0xDF));
    let mut m = machine_with(&code, 44_100);
    render(&mut m, 100);
    assert!(m.voice_info(4).noise);
    assert!(!m.voice_info(4).direct);
    let direct = m.voice_info(0);
    assert!(direct.direct && !direct.noise);
}

#[test]
fn the_scope_follows_the_waveform_and_a_muted_channel_still_shows() {
    let mut m = machine_with(&square(0, 254, 31), 44_100);
    render(&mut m, 8000);
    m.set_channel_mask(0b111110);
    render(&mut m, 500);
    let mut scope = vec![0f32; 4096];
    m.scope(0, &mut scope);
    // 440.45 Hz over 4,096 frames of 44,100 Hz: 81.8 crossings.
    let n = crossings(&scope);
    assert!((79..=84).contains(&n), "{n}");
    assert!(scope.iter().any(|s| *s > 0.1) && scope.iter().any(|s| *s < -0.1));
    // A channel that is off is silent in its scope.
    let mut idle = vec![1f32; 128];
    m.scope(3, &mut idle);
    assert!(idle.iter().all(|s| *s == 0.0));
}

#[test]
fn the_scope_resamples_to_another_output_rate() {
    let mut m = machine_with(&square(0, 254, 31), 192_000);
    render(&mut m, 40_000);
    let mut scope = vec![0f32; 4096];
    m.scope(0, &mut scope);
    // 4,096 frames of 192 kHz are 21 ms: 9.4 cycles, 18.8 crossings.
    let n = crossings(&scope);
    assert!((17..=21).contains(&n), "{n}");
}

#[test]
fn a_copy_has_an_empty_scope_and_the_sound_is_unchanged() {
    let mut m = machine_with(&square(0, 254, 31), 44_100);
    render(&mut m, 8000);
    let copy = m.clone();
    let mut scope = vec![1f32; 256];
    copy.scope(0, &mut scope);
    assert!(scope.iter().all(|s| *s == 0.0));
    let mut a = m.clone();
    let mut b = m;
    let (mut x, mut y) = (vec![0i16; 4000], vec![0i16; 4000]);
    a.render_s16(&mut x);
    b.render_s16(&mut y);
    assert_eq!(x, y);
}
