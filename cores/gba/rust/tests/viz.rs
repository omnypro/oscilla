//! The data that a player can show: the state of each channel and the recent
//! output of each channel.

mod common;

use common::rom_from_words;
use oscilla_gba::image::Image;
use oscilla_gba::{Machine, SCOPE_LEN, VoiceKind};

/// ARM code that stores 16-bit values to the I/O registers and then parks.
struct Program(Vec<u32>);

impl Program {
    fn new() -> Program {
        Program(vec![0xE3A0_0301]) // mov r0, #0x04000000
    }

    fn poke(mut self, off: u32, value: u32) -> Program {
        let low = off & 0xFF;
        self.0.extend([
            0xE59F_1000, // ldr r1, [pc, #0]
            0xEA00_0000, // b +4
            value,
            0xE280_CC00 | (off >> 8), // add r12, r0, #(off & ~0xFF)
            0xE1CC_10B0 | ((low >> 4) << 8) | (low & 0xF), // strh r1, [r12, #low]
        ]);
        self
    }

    /// Sound on, with SOUNDCNT_L and SOUNDCNT_H set.
    fn sound(self, l: u32, h: u32) -> Program {
        self.poke(0x84, 0x80).poke(0x80, l).poke(0x82, h)
    }

    /// Writes 32 samples to the wave RAM bank `bank` and plays that bank. The
    /// CPU reaches the bank that is not selected for play.
    fn wave_ram(self, bank: u32, halfwords: [u32; 8]) -> Program {
        let mut p = self.poke(0x70, (bank ^ 1) << 6);
        for (k, h) in halfwords.into_iter().enumerate() {
            p = p.poke(0x90 + 2 * k as u32, h);
        }
        p.poke(0x70, 0x80 | (bank << 6))
    }

    fn rom(mut self) -> Vec<u8> {
        self.0.push(0xEAFF_FFFE); // b .
        rom_from_words(&self.0)
    }
}

/// 16 samples of 15, then 16 of 0.
const SQUARE: [u32; 8] = [0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0, 0, 0, 0];
const RAMP: [u32; 8] = [
    0x2301, 0x6745, 0xAB89, 0xEFCD, 0xDCFE, 0x98BA, 0x5476, 0x1032,
];

/// Pulse 1 at 439.84 Hz and volume 15 on both sides, pulse 2 at 512 Hz and
/// volume 10 on the left, the wave channel at 256 Hz on the right, and 7-bit
/// noise at volume 15 on both sides.
fn tones(wave: [u32; 8]) -> Program {
    Program::new()
        .sound(0xBD77, 0x0002)
        .poke(0x62, 0xF080) // duty 2, volume 15
        .poke(0x64, 0x8000 | 1750)
        .poke(0x68, 0xA040) // duty 1, volume 10
        .poke(0x6C, 0x8000 | 1792)
        .wave_ram(1, wave)
        .poke(0x72, 0x2000) // output level 100%
        .poke(0x74, 0x8000 | 1792)
        .poke(0x78, 0xF000) // volume 15
        .poke(0x7C, 0x8008) // 7-bit noise
}

/// Sound on, SOUNDCNT_H set to `h`, DMA1 feeding FIFO A from the buffer that
/// follows the program (sound FIFO timing, repeat, 32-bit), and timer 0 run
/// with `timer` as its reload value and control.
fn fifo_program(h: u32, timer: u32, buffer: &[u8]) -> Vec<u8> {
    let mut rom = rom_from_words(&[
        0xE3A0_0301, // mov r0, #0x04000000
        0xE3A0_1080, // mov r1, #0x80
        0xE1C0_18B4, // strh r1, [r0, #0x84]
        0xE59F_1024, // ldr r1, =h
        0xE1C0_18B2, // strh r1, [r0, #0x82]
        0xE28F_102C, // add r1, pc, #0x2C: the buffer
        0xE580_10BC, // str r1, [r0, #0xBC]
        0xE59F_1018, // ldr r1, =0x040000A0
        0xE580_10C0, // str r1, [r0, #0xC0]
        0xE59F_1014, // ldr r1, =0xB6400004
        0xE580_10C4, // str r1, [r0, #0xC4]
        0xE59F_1010, // ldr r1, =timer
        0xE580_1100, // str r1, [r0, #0x100]
        0xEAFF_FFFE, // b .
        h,
        0x0400_00A0,
        0xB640_0004,
        timer,
    ]);
    rom.extend_from_slice(buffer);
    rom
}

/// One second of a square wave of 64 samples at 32,768 Hz (512 Hz) that
/// swings between 100 and -100.
fn fifo_square() -> Vec<u8> {
    (0..32768)
        .map(|i| if i % 64 < 32 { 100i8 } else { -100 } as u8)
        .collect()
}

fn machine(rom: &[u8], rate: i32) -> Machine {
    Machine::new(&Image::from_rom(rom).unwrap(), rate).unwrap()
}

fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; 2 * frames];
    m.render_s16(&mut out);
    out
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.001
}

fn crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

#[test]
fn the_tone_wave_and_noise_channels_report_their_registers() {
    let mut m = machine(&tones(SQUARE).rom(), 48_000);
    render(&mut m, 2000);
    assert_eq!(m.voice_count(), 6);

    let p1 = m.voice(0).unwrap();
    assert_eq!(
        (p1.name, p1.mask_bit, p1.kind),
        ("Pulse 1", 0, VoiceKind::Tone)
    );
    assert!(p1.active && p1.key_on);
    assert!(
        close(p1.frequency_hz, 131_072.0 / 298.0),
        "{}",
        p1.frequency_hz
    );
    assert!(close(p1.volume, 1.0) && close(p1.envelope, 1.0));
    assert!(close(p1.left, 1.0) && close(p1.right, 1.0));
    assert_eq!((p1.instrument, p1.phase), (2, 0));

    let p2 = m.voice(1).unwrap();
    assert_eq!((p2.name, p2.mask_bit), ("Pulse 2", 1));
    assert!(close(p2.frequency_hz, 512.0));
    assert!(close(p2.volume, 10.0 / 15.0) && close(p2.envelope, 10.0 / 15.0));
    assert!(close(p2.left, 10.0 / 15.0) && p2.right == 0.0);
    assert_eq!(p2.instrument, 1);

    let w = m.voice(2).unwrap();
    assert_eq!((w.name, w.kind), ("Wave", VoiceKind::Tone));
    assert!(w.active && w.key_on);
    assert!(close(w.frequency_hz, 256.0), "{}", w.frequency_hz);
    assert!(close(w.volume, 1.0) && w.envelope == -1.0);
    assert!(w.left == 0.0 && close(w.right, 1.0));
    assert!(w.instrument >= 0);

    let n = m.voice(3).unwrap();
    assert_eq!((n.name, n.kind), ("Noise", VoiceKind::Noise));
    assert!(n.active && n.frequency_hz == 0.0);
    assert!(close(n.volume, 1.0) && close(n.envelope, 1.0));
    assert_eq!(n.instrument, 1);

    for (i, name) in [(4, "Direct A"), (5, "Direct B")] {
        let f = m.voice(i).unwrap();
        assert_eq!(
            (f.name, f.mask_bit, f.kind),
            (name, i as u32, VoiceKind::Sample)
        );
        assert!(!f.active && !f.key_on);
        assert!(f.frequency_hz == 0.0 && f.volume == 0.0);
        assert_eq!((f.envelope, f.instrument), (-1.0, -1));
    }
    assert!(m.voice(6).is_none());
}

#[test]
fn the_wave_number_follows_the_samples_of_the_playing_bank() {
    let id = |p: Program| {
        let mut m = machine(&p.rom(), 48_000);
        render(&mut m, 500);
        m.voice(2).unwrap().instrument
    };
    let square = id(tones(SQUARE));
    let in_bank_0 = id(Program::new()
        .sound(0x0400, 0x0002)
        .wave_ram(0, SQUARE)
        .poke(0x72, 0x2000)
        .poke(0x74, 0x8000 | 1792));
    assert_eq!(square, in_bank_0);
    assert_ne!(square, id(tones(RAMP)));
}

#[test]
fn the_sweep_moves_the_pitch_of_pulse_1() {
    // Sweep period 1, decrease, shift 2.
    let p = Program::new()
        .sound(0x1177, 0x0002)
        .poke(0x60, 0x001A)
        .poke(0x62, 0xF080)
        .poke(0x64, 0x8400);
    let mut m = machine(&p.rom(), 48_000);
    render(&mut m, 100);
    let before = m.voice(0).unwrap().frequency_hz;
    render(&mut m, 2000);
    let after = m.voice(0).unwrap().frequency_hz;
    assert!(before <= 128.0 && after < before, "{before} {after}");
}

#[test]
fn a_channel_that_stops_or_that_no_side_hears_is_not_active() {
    let p = Program::new()
        .sound(0x1177, 0x0002)
        .poke(0x62, 0xF0BF) // length 1
        .poke(0x64, 0xC400) // length on
        .poke(0x68, 0xF080)
        .poke(0x6C, 0x8700) // on, but neither side has it
        .poke(0x70, 0x0000) // the wave channel's DAC is off
        .poke(0x74, 0x8700)
        .poke(0x78, 0x0000) // volume 0 and no envelope turn the DAC off
        .poke(0x7C, 0x8000);
    let mut m = machine(&p.rom(), 48_000);
    render(&mut m, 2000);
    let v: Vec<_> = (0..6).map(|i| m.voice(i).unwrap()).collect();
    assert!(!v[0].key_on && !v[0].active);
    assert_eq!((v[0].volume, v[0].envelope, v[0].left), (0.0, 0.0, 0.0));
    assert!(v[1].key_on && !v[1].active);
    assert!(close(v[1].volume, 1.0) && v[1].left == 0.0 && v[1].right == 0.0);
    for i in [2, 3] {
        assert!(!v[i].key_on && !v[i].active && v[i].volume == 0.0, "{i}");
    }
}

#[test]
fn a_fifo_voice_reports_its_timer_rate_and_its_sides() {
    // FIFO A at 100% on both sides from timer 0, FIFO B on both sides from
    // timer 1, which does not run.
    let mut m = machine(&fifo_program(0x7B04, 0x0080_FE00, &fifo_square()), 48_000);
    render(&mut m, 2000);
    let a = m.voice(4).unwrap();
    assert!(a.active && a.key_on && a.kind == VoiceKind::Sample);
    assert!(close(a.frequency_hz, 32_768.0), "{}", a.frequency_hz);
    assert!(close(a.volume, 1.0) && close(a.left, 1.0) && close(a.right, 1.0));
    assert_eq!((a.envelope, a.instrument, a.phase), (-1.0, -1, 0));
    let b = m.voice(5).unwrap();
    assert!(!b.active && !b.key_on);
    assert_eq!((b.frequency_hz, b.volume), (0.0, 0.0));

    // FIFO A at 50% on the left, from timer 0 with a prescaler of 64.
    let mut m = machine(&fifo_program(0x0A00, 0x0081_FE00, &fifo_square()), 48_000);
    render(&mut m, 2000);
    let a = m.voice(4).unwrap();
    assert!(a.active);
    assert!(close(a.frequency_hz, 512.0), "{}", a.frequency_hz);
    assert!(close(a.volume, 0.5) && close(a.left, 0.5) && a.right == 0.0);
}

#[test]
fn the_scope_follows_a_tone_and_a_muted_voice_still_shows() {
    let mut m = machine(&tones(SQUARE).rom(), 44_100);
    render(&mut m, 8000);
    m.set_channel_mask(0b11_1110);
    render(&mut m, 500);
    let mut scope = vec![0f32; 4096];
    assert!(m.scope(0, &mut scope));
    // 439.84 Hz over 4,096 frames of 44,100 Hz: 81.7 crossings.
    let n = crossings(&scope);
    assert!((79..=84).contains(&n), "{n}");
    // Volume 15 on both sides is full scale, about the middle.
    let max = scope.iter().cloned().fold(f32::MIN, f32::max);
    let min = scope.iter().cloned().fold(f32::MAX, f32::min);
    assert!(max > 0.99 && min < -0.99, "{max} {min}");
    // The wave channel is on one side, which is half of full scale.
    let mut wave = vec![0f32; 4096];
    m.scope(2, &mut wave);
    let max = wave.iter().cloned().fold(f32::MIN, f32::max);
    assert!(close(max, 0.5), "{max}");
    // 256 Hz: 47.6 crossings.
    let n = crossings(&wave);
    assert!((45..=50).contains(&n), "{n}");
    // A channel that does not play is silent in its scope.
    let mut idle = vec![1f32; 128];
    m.scope(5, &mut idle);
    assert!(idle.iter().all(|s| *s == 0.0));
}

#[test]
fn the_scope_resamples_to_another_output_rate_and_reaches_back_93_ms() {
    let mut m = machine(&tones(SQUARE).rom(), 192_000);
    render(&mut m, 40_000);
    let mut scope = vec![0f32; 4096];
    m.scope(0, &mut scope);
    // 4,096 frames of 192 kHz are 21 ms: 9.4 cycles, 18.8 crossings.
    let n = crossings(&scope);
    assert!((17..=21).contains(&n), "{n}");
    // At 44,100 Hz the ring holds 4,096 frames and no more.
    let mut m = machine(&tones(SQUARE).rom(), 44_100);
    render(&mut m, 8000);
    let mut long = vec![0f32; 2 * SCOPE_LEN];
    m.scope(0, &mut long);
    assert!(long[..SCOPE_LEN].iter().all(|s| *s == 0.0));
    assert!(long[SCOPE_LEN..].iter().any(|s| *s != 0.0));
}

#[test]
fn the_scope_of_a_fifo_follows_its_stream() {
    let mut m = machine(&fifo_program(0x0B04, 0x0080_FE00, &fifo_square()), 44_100);
    render(&mut m, 8000);
    let mut scope = vec![0f32; 4096];
    m.scope(4, &mut scope);
    // 512 Hz over 4,096 frames of 44,100 Hz: 95.1 crossings.
    let n = crossings(&scope);
    assert!((92..=98).contains(&n), "{n}");
    // 100 of 128 at 100% on both sides.
    let max = scope.iter().cloned().fold(f32::MIN, f32::max);
    assert!(close(max, 100.0 / 128.0), "{max}");
}

#[test]
fn a_copy_a_restore_and_a_reset_start_with_an_empty_scope() {
    let mut m = machine(&tones(SQUARE).rom(), 44_100);
    render(&mut m, 8000);
    let mut scope = vec![1f32; 256];
    let mut copy = m.clone();
    copy.scope(0, &mut scope);
    assert!(scope.iter().all(|s| *s == 0.0));
    render(&mut copy, 500);
    copy.scope(0, &mut scope);
    assert!(scope.iter().any(|s| *s != 0.0), "a copy samples on");

    let snap = m.snapshot();
    render(&mut m, 500);
    m.restore(&snap).unwrap();
    m.scope(0, &mut scope);
    assert!(scope.iter().all(|s| *s == 0.0));

    render(&mut m, 500);
    m.reset();
    m.scope(0, &mut scope);
    assert!(scope.iter().all(|s| *s == 0.0));
}

#[test]
fn reading_voices_and_scopes_does_not_change_the_sound() {
    let play = |read: bool| {
        let mut m = machine(&tones(RAMP).rom(), 48_000);
        let mut out = Vec::new();
        let mut scope = vec![0f32; 1024];
        for _ in 0..40 {
            out.extend(render(&mut m, 500));
            if read {
                for i in 0..m.voice_count() {
                    assert!(m.voice(i).is_some());
                    assert!(m.scope(i, &mut scope));
                }
            }
        }
        out
    };
    assert_eq!(play(false), play(true));
}

#[test]
fn a_voice_past_the_last_has_no_scope() {
    let m = machine(&tones(SQUARE).rom(), 48_000);
    let mut scope = vec![1f32; 16];
    assert!(!m.scope(6, &mut scope));
    assert!(scope.iter().all(|s| *s == 1.0));
    assert!(m.scope(0, &mut []));
}
