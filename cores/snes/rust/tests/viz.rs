//! The data that a player can show: the state of each voice, the recent
//! output of each voice and the map of memory accesses.

mod common;

use common::*;
use oscilla_snes::{CPU_READ, CPU_WRITE, DSP_READ, DSP_WRITE, EXECUTED, Machine, SCOPE_LEN};

const DC: [u8; 16] = [7; 16];

/// Voice 0 plays a block of equal nibbles with the pitch 0x1000, the gain 0x40
/// and both volumes 0x20. `code` is the SPC700 program.
fn machine_with(code: &[u8], extra: &[(u8, u8)]) -> Machine {
    let mut regs = voice0(0x1000, 0x40, 0x20);
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

#[test]
fn a_playing_voice_reports_its_registers_and_its_envelope() {
    let mut m = machine_with(&[0x2F, 0xFE], &[]);
    m.dsp_samples(300);
    let v = m.voice_info(0);
    assert_eq!(v.pitch, 0x1000);
    assert_eq!(v.volume, (0x20, 0x20));
    assert_eq!(v.gain, 0x40);
    assert_eq!(v.source, 0);
    assert!(v.envelope > 0);
    assert!(v.phase < 3, "phase {}", v.phase);
    assert!(!v.noise && !v.echo && !v.pitch_modulated);
    // A voice that was never keyed is in its release and silent.
    let idle = m.voice_info(3);
    assert_eq!((idle.phase, idle.envelope), (3, 0));
    assert_eq!(idle.output, 0);
}

#[test]
fn the_noise_echo_and_modulation_bits_show() {
    let mut m = machine_with(&[0x2F, 0xFE], &[(0x3D, 0x01), (0x4D, 0x01), (0x2D, 0x02)]);
    m.dsp_samples(10);
    let v = m.voice_info(0);
    assert!(v.noise && v.echo);
    assert!(!v.pitch_modulated);
    assert!(m.voice_info(1).pitch_modulated);
    assert!(!m.voice_info(0).pitch_modulated);
}

#[test]
fn the_scope_holds_what_the_voice_put_out_and_a_muted_voice_still_shows() {
    let mut m = machine_with(&[0x2F, 0xFE], &[]);
    m.dsp_samples(600);
    let mut scope = vec![0f32; 256];
    m.scope(0, &mut scope);
    // The newest samples are the steady level of the voice, which is not
    // zero.
    assert!(scope[255] > 0.01, "{}", scope[255]);
    assert!((scope[255] - scope[250]).abs() < 0.002);
    // A voice that does not play is silent in its scope.
    let mut other = vec![1f32; 64];
    m.scope(5, &mut other);
    assert!(other.iter().all(|s| *s == 0.0));
    // Muting the voice leaves its scope alone.
    m.set_channel_mask(0);
    m.dsp_samples(300);
    let mut muted = vec![0f32; 64];
    m.scope(0, &mut muted);
    assert!(muted[63] > 0.01);
}

#[test]
fn the_scope_resamples_to_the_output_rate_and_reaches_back_128_ms() {
    let mut m = machine_with(&[0x2F, 0xFE], &[]);
    m.dsp_samples(SCOPE_LEN + 600);
    // 4,096 DSP samples are 5,461 frames at 42,667 Hz... at 44,100 Hz a scope of
    // 4,096 frames reaches back 3,000 DSP samples, so all of it is filled.
    let mut scope = vec![0f32; 4096];
    m.scope(0, &mut scope);
    assert!(scope[0] > 0.01 && scope[4095] > 0.01);
    // Asking for more than the ring holds gives zeros for the oldest part.
    let mut long = vec![0f32; 8192];
    m.scope(0, &mut long);
    assert_eq!(long[0], 0.0);
    assert!(long[8191] > 0.01);
}

#[test]
fn the_access_map_is_off_until_it_is_asked_for_and_marks_reads_writes_and_fetches() {
    // MOV A,$30 / MOV $31,A / BRA back: reads $30, writes $31, fetches the
    // three instructions at their addresses.
    let code = [0xE4, 0x30, 0xC4, 0x31, 0x2F, 0xFA];
    let mut m = machine_with(&code, &[]);
    assert!(m.access_map().is_none());
    m.set_access_tracking(true);
    m.run_cycles(2000);
    let map = m.access_map().unwrap();
    let ram = m.ram();
    let start = (0..0xFFFF - code.len())
        .find(|&a| ram[a..a + code.len()] == code)
        .unwrap();
    for n in 0..code.len() {
        assert!(map[start + n] & EXECUTED != 0, "byte {n}");
    }
    assert_ne!(map[0x30] & CPU_READ, 0);
    assert_ne!(map[0x31] & CPU_WRITE, 0);
    // The fetches are not reads of data.
    assert_eq!(map[start] & CPU_READ, 0);
    m.clear_access_map();
    assert!(m.access_map().unwrap().iter().all(|b| *b == 0));
    m.set_access_tracking(false);
    assert!(m.access_map().is_none());
}

#[test]
fn the_dsps_reads_of_samples_and_the_echo_buffer_are_marked_too() {
    let mut m = machine_with(&[0x2F, 0xFE], &[(0x6D, 0x30), (0x7D, 0x01), (0x6C, 0x00)]);
    m.set_access_tracking(true);
    m.dsp_samples(2000);
    let map = m.access_map().unwrap();
    // The directory entry at 0x1000 and the block at 0x2000 are read.
    assert_ne!(map[0x1000] & DSP_READ, 0);
    assert_ne!(map[0x2000] & DSP_READ, 0);
    // The echo buffer at 0x3000 is read and written.
    assert_ne!(map[0x3000] & DSP_READ, 0);
    assert_ne!(map[0x3000] & DSP_WRITE, 0);
}

#[test]
fn a_snapshot_has_no_map_and_restore_keeps_the_one_that_is_set() {
    let mut m = machine_with(&[0x2F, 0xFE], &[]);
    m.set_access_tracking(true);
    m.dsp_samples(100);
    let snap = m.snapshot();
    m.restore(&snap).unwrap();
    assert!(m.access_tracking());
    let mut other = machine_with(&[0x2F, 0xFE], &[]);
    other.restore(&snap).unwrap();
    assert!(!other.access_tracking());
}

#[test]
fn tracking_does_not_change_the_sound() {
    let play = |track: bool| {
        let mut m = machine_with(&[0xE4, 0x30, 0xC4, 0x31, 0x2F, 0xFA], &[]);
        m.set_access_tracking(track);
        let mut out = vec![0i16; 8000];
        m.render_s16(&mut out);
        out
    };
    assert_eq!(play(false), play(true));
}
