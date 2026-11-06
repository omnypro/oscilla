//! The machine runs the code of an HES file: the request routine, the timer
//! and the vertical sync interrupts, and the sound generator.

use oscilla_hes::Hes;
use oscilla_pce::Machine;

/// A file whose first ROM bank holds `request` at 0xE100, a handler for the
/// vertical sync (0xFFF8) at 0xE180 and a handler for the timer (0xFFFA) at
/// 0xE200. The other vectors point at an RTI.
fn program(request: &[u8], vsync: &[u8], timer: &[u8]) -> Vec<u8> {
    let mut bank = vec![0u8; 0x2000];
    bank[0x100..0x100 + request.len()].copy_from_slice(request);
    bank[0x180..0x180 + vsync.len()].copy_from_slice(vsync);
    bank[0x200..0x200 + timer.len()].copy_from_slice(timer);
    bank[0x1FF0] = 0x40;
    bank[0x1FF6..0x1FF8].copy_from_slice(&[0xF0, 0xFF]);
    bank[0x1FF8..0x1FFA].copy_from_slice(&[0x80, 0xE1]);
    bank[0x1FFA..0x1FFC].copy_from_slice(&[0x00, 0xE2]);
    bank[0x1FFC..0x1FFE].copy_from_slice(&[0xF0, 0xFF]);
    let mut f = b"HESM".to_vec();
    f.extend([0, 0, 0x00, 0xE1, 0xFF, 0xF8, 0, 0, 1, 2, 3, 0]);
    f.extend(b"DATA");
    f.extend((bank.len() as u32).to_le_bytes());
    f.extend(0u32.to_le_bytes());
    f.extend([0; 4]);
    f.extend(bank);
    f
}

fn machine(file: &[u8], song: u8) -> Machine {
    let mut m = Machine::new(&Hes::parse(file).unwrap(), 44_100, song).unwrap();
    m.set_highpass_hz(0.0);
    m
}

fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; 2 * frames];
    m.render_s16(&mut out);
    out.chunks(2).map(|f| f[0]).collect()
}

fn crossings(samples: &[i16]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0) != (w[1] < 0))
        .count()
}

/// LDA #value, STA address.
fn sta(address: u16, value: u8) -> Vec<u8> {
    vec![0xA9, value, 0x8D, address as u8, (address >> 8) as u8]
}

/// The sound generator's registers are at logical 0x0800.
const PSG: u16 = 0x0800;

/// Selects channel 1 at full level in direct D/A mode.
fn direct_channel() -> Vec<u8> {
    [
        sta(PSG, 0),
        sta(PSG + 1, 0xFF),
        sta(PSG + 5, 0xFF),
        sta(PSG + 4, 0xDF),
    ]
    .concat()
}

/// A handler that flips the direct D/A word between 0 and 31, with a copy of
/// the word in the RAM at 0x2000, and then acknowledges the timer.
const FLIP_AND_ACK: [u8; 15] = [
    0xAD, 0x00, 0x20, // LDA $2000
    0x49, 0x1F, // EOR #$1F
    0x8D, 0x00, 0x20, // STA $2000
    0x8D, 0x06, 0x08, // STA $0806
    0x8D, 0x03, 0x14, // STA $1403
    0x40, // RTI
];

#[test]
fn the_request_routine_runs_with_the_song_number_in_the_accumulator() {
    // The routine keeps A, sets up channel 1 and writes A to its direct D/A
    // register.
    let mut code = vec![0x48];
    code.extend(direct_channel());
    code.extend([0x68, 0x8D, 0x06, 0x08, 0x60]);
    let f = program(&code, &[0x40], &[0x40]);
    let low = render(&mut machine(&f, 3), 2000);
    let high = render(&mut machine(&f, 28), 2000);
    assert!(low[1500] < -1000, "{}", low[1500]);
    assert!(high[1500] > 1000, "{}", high[1500]);
    assert_eq!(render(&mut machine(&f, 3), 2000), low);
}

#[test]
fn a_waveform_plays_at_the_frequency_of_its_registers() {
    // A square wave of 16 words at 31 and 16 at 0, with F = 254 (440 Hz).
    let mut code = sta(PSG, 0);
    code.extend(sta(PSG + 1, 0xFF));
    code.extend(sta(PSG + 5, 0xFF));
    code.extend(sta(PSG + 4, 0x40));
    code.extend(sta(PSG + 4, 0x00));
    for i in 0..32 {
        code.extend(sta(PSG + 6, if i < 16 { 31 } else { 0 }));
    }
    code.extend(sta(PSG + 2, 254));
    code.extend(sta(PSG + 3, 0));
    code.extend(sta(PSG + 4, 0x9F));
    code.push(0x60);
    let f = program(&code, &[0x40], &[0x40]);
    let out = render(&mut machine(&f, 0), 44_100);
    // 440.45 Hz for 43,100 of the 44,100 samples.
    let n = crossings(&out[1000..]);
    assert!((855..=867).contains(&n), "{n}");
}

#[test]
fn the_timer_interrupt_calls_its_handler_at_the_documented_rate() {
    // Reload 7 counts 8 times of 1024 prescaler steps: 874 interrupts a
    // second, and the word flips at each.
    let mut code = direct_channel();
    code.extend(sta(0x0C00, 7));
    code.extend(sta(0x0C01, 1));
    code.extend(sta(0x1402, 0));
    code.extend([0x58, 0x60]);
    let f = program(&code, &[0x40], &FLIP_AND_ACK);
    let out = render(&mut machine(&f, 0), 44_100);
    // 874 flips a second for 43,100 of the 44,100 samples.
    let n = crossings(&out[1000..]);
    assert!((845..=862).contains(&n), "{n}");
}

#[test]
fn a_masked_timer_interrupt_does_not_run() {
    let mut code = direct_channel();
    code.extend(sta(0x0C00, 7));
    code.extend(sta(0x0C01, 1));
    code.extend(sta(0x1402, 4));
    code.extend([0x58, 0x60]);
    let f = program(&code, &[0x40], &FLIP_AND_ACK);
    let out = render(&mut machine(&f, 0), 20_000);
    assert_eq!(crossings(&out[1000..]), 0);
}

#[test]
fn the_vertical_sync_interrupt_runs_once_a_frame_when_enabled() {
    // The routine enables the vertical sync interrupt in the video chip's
    // control register. The handler reads the status register, which clears
    // the request, and flips the word.
    let mut code = direct_channel();
    code.extend(sta(0x0000, 5));
    code.extend(sta(0x0002, 0x08));
    code.extend([0x58, 0x60]);
    let handler = [
        0xAD, 0x00, 0x00, // LDA $0000
        0xAD, 0x00, 0x20, 0x49, 0x1F, 0x8D, 0x00, 0x20, 0x8D, 0x06, 0x08, 0x40,
    ];
    let f = program(&code, &handler, &[0x40]);
    let out = render(&mut machine(&f, 0), 44_100 * 2);
    let n = crossings(&out[1000..]);
    // 262 lines of 1365 clocks of 21.477 MHz: about 60.05 frames a second.
    assert!((117..=122).contains(&n), "{n}");
}

#[test]
fn the_vertical_sync_does_nothing_until_the_program_enables_it() {
    let mut code = direct_channel();
    code.extend([0x58, 0x60]);
    let handler = [
        0xAD, 0x00, 0x00, 0xAD, 0x00, 0x20, 0x49, 0x1F, 0x8D, 0x00, 0x20, 0x8D, 0x06, 0x08, 0x40,
    ];
    let f = program(&code, &handler, &[0x40]);
    let out = render(&mut machine(&f, 0), 44_100);
    assert_eq!(crossings(&out[1000..]), 0);
}

#[test]
fn a_program_that_never_returns_keeps_running() {
    // An endless loop that flips channel 1's word as fast as the processor
    // goes sounds, and the machine does not wait for an idle address.
    let mut code = direct_channel();
    code.extend([
        0xA9, 0x1F, // loop: LDA #$1F
        0x8D, 0x06, 0x08, // STA $0806
        0xA9, 0x00, // LDA #0
        0x8D, 0x06, 0x08, // STA $0806
        0x80, 0xF4, // BRA loop
    ]);
    let f = program(&code, &[0x40], &[0x40]);
    let out = render(&mut machine(&f, 0), 4410);
    assert!(out.iter().any(|s| s.abs() > 100));
}

#[test]
fn a_copy_of_the_machine_plays_the_same_samples() {
    let mut code = direct_channel();
    code.extend(sta(0x0C00, 20));
    code.extend(sta(0x0C01, 1));
    code.extend(sta(0x1402, 0));
    code.extend([0x58, 0x60]);
    let f = program(&code, &[0x40], &FLIP_AND_ACK);
    let mut a = machine(&f, 0);
    render(&mut a, 3000);
    let mut b = a.clone();
    assert_eq!(render(&mut a, 5000), render(&mut b, 5000));
}

#[test]
fn the_channel_mask_keeps_a_channel_out() {
    let mut code = direct_channel();
    code.extend([0xA9, 0x1F, 0x8D, 0x06, 0x08, 0x60]);
    let f = program(&code, &[0x40], &[0x40]);
    let mut m = machine(&f, 0);
    assert!(render(&mut m, 1000)[900] > 1000);
    m.set_channel_mask(0b111110);
    assert_eq!(m.channel_mask(), 0b111110);
    assert_eq!(render(&mut m, 1000)[900], 0);
}

/// Channel 1 as a 440 Hz square wave, then `code`.
fn square_then(code: &[u8]) -> Vec<u8> {
    let mut c = sta(PSG, 0);
    c.extend(sta(PSG + 1, 0xFF));
    c.extend(sta(PSG + 5, 0xFF));
    c.extend(sta(PSG + 4, 0x40));
    c.extend(sta(PSG + 4, 0x00));
    for i in 0..32 {
        c.extend(sta(PSG + 6, if i < 16 { 31 } else { 0 }));
    }
    c.extend(sta(PSG + 2, 254));
    c.extend(sta(PSG + 3, 0));
    c.extend(sta(PSG + 4, 0x9F));
    c.extend(code);
    c
}

fn machine_at(file: &[u8], rate: i32) -> Machine {
    let mut m = Machine::new(&Hes::parse(file).unwrap(), rate, 0).unwrap();
    m.set_highpass_hz(0.0);
    m
}

#[test]
fn a_block_transfer_of_65536_bytes_does_not_overrun_the_output_at_192_khz() {
    // TIN of length 0 moves 65,536 bytes: 393,233 cycles, 0.22 s at the low
    // speed. The sound before and after it must keep its pitch.
    let code = square_then(&[0xD3, 0x00, 0xE0, 0x00, 0x22, 0x00, 0x00, 0x60]);
    let f = program(&code, &[0x40], &[0x40]);
    let rate = 192_000usize;
    let mut m = machine_at(&f, rate as i32);
    let mut out = vec![0i16; 2 * rate / 2];
    m.render_s16(&mut out);
    let left: Vec<i16> = out.chunks(2).map(|f| f[0]).collect();
    // 440.45 Hz: about 2.3 crossings per 1,000 frames at 192 kHz.
    for start in [1000usize, 17_384, 41_000, 60_000] {
        let n = crossings(&left[start..start + 20_000]);
        let want = 20_000.0 / 192_000.0 * 880.9;
        assert!(
            (n as f64 - want).abs() <= 3.0,
            "start {start}: {n} crossings, expected about {want:.1}"
        );
    }
}

#[test]
fn a_snapshot_restores_into_a_second_machine_of_the_same_file() {
    let code = square_then(&[0x60]);
    let f = program(&code, &[0x40], &[0x40]);
    let mut a = machine(&f, 0);
    let mut b = machine(&f, 0);
    render(&mut a, 3000);
    let saved = a.clone();
    assert!(b.restore(&saved));
    assert_eq!(render(&mut a, 2000), render(&mut b, 2000));
    let other = program(&sta(PSG, 1), &[0x40], &[0x40]);
    assert!(!machine(&other, 0).restore(&saved));
}
