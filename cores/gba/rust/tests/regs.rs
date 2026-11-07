//! The register blocks: the bytes that a program stored to the sound
//! registers, wave RAM, the FIFO ports, the timers and the DMA channels.

mod common;

use common::rom_from_words;
use oscilla_gba::Machine;
use oscilla_gba::image::Image;

/// ARM code that stores to the I/O registers and then parks.
struct Program(Vec<u32>);

impl Program {
    fn new() -> Program {
        Program(vec![0xE3A0_0301]) // mov r0, #0x04000000
    }

    /// A 16-bit store.
    fn half(mut self, off: u32, value: u32) -> Program {
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

    /// A 32-bit store.
    fn word(mut self, off: u32, value: u32) -> Program {
        self.0.extend([
            0xE59F_1000, // ldr r1, [pc, #0]
            0xEA00_0000, // b +4
            value,
            0xE580_1000 | off, // str r1, [r0, #off]
        ]);
        self
    }

    /// An 8-bit store.
    fn byte(mut self, off: u32, value: u32) -> Program {
        self.0.extend([
            0xE59F_1000, // ldr r1, [pc, #0]
            0xEA00_0000, // b +4
            value,
            0xE5C0_1000 | off, // strb r1, [r0, #off]
        ]);
        self
    }

    /// Writes 32 samples to the wave RAM bank `bank` and plays that bank. The
    /// CPU reaches the bank that is not selected for play.
    fn wave_ram(self, bank: u32, halfwords: [u32; 8]) -> Program {
        let mut p = self.half(0x70, (bank ^ 1) << 6);
        for (k, h) in halfwords.into_iter().enumerate() {
            p = p.half(0x90 + 2 * k as u32, h);
        }
        p.half(0x70, 0x80 | (bank << 6))
    }

    fn rom(mut self) -> Vec<u8> {
        self.0.push(0xEAFF_FFFE); // b .
        rom_from_words(&self.0)
    }
}

const SQUARE: [u32; 8] = [0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0, 0, 0, 0];
const RAMP: [u32; 8] = [
    0x2301, 0x6745, 0xAB89, 0xEFCD, 0xDCFE, 0x98BA, 0x5476, 0x1032,
];

fn halves(words: [u32; 8]) -> Vec<u8> {
    words
        .iter()
        .flat_map(|h| (*h as u16).to_le_bytes())
        .collect()
}

/// Sound on, then every sound register set to a value with bits that read
/// back as 0.
fn sound_program() -> Program {
    Program::new()
        .half(0x84, 0xFF)
        .half(0x60, 0x0074)
        .half(0x62, 0xF0C5)
        .half(0x64, 0x86D6)
        .half(0x66, 0xFFFF)
        .half(0x68, 0xA040)
        .half(0x6C, 0x8700)
        .half(0x70, 0x00A0)
        .half(0x72, 0x2005)
        .half(0x74, 0x8700)
        .half(0x78, 0xF03F)
        .half(0x7C, 0x8008)
        .half(0x80, 0xBD77)
        .half(0x82, 0x9B07)
        .half(0x88, 0x4200)
}

/// The sound block that `sound_program` leaves.
fn sound_bytes() -> Vec<u8> {
    let mut b = vec![0u8; 0x2A];
    for (off, v) in [
        (0x60, 0x0074u16),
        (0x62, 0xF0C5),
        (0x64, 0x86D6),
        (0x68, 0xA040),
        (0x6C, 0x8700),
        (0x70, 0x00A0),
        (0x72, 0x2005),
        (0x74, 0x8700),
        (0x78, 0xF03F),
        (0x7C, 0x8008),
        (0x80, 0xBD77),
        (0x82, 0x9B07),
        (0x84, 0x00FF),
        (0x88, 0x4200),
    ] {
        b[off - 0x60..off - 0x5E].copy_from_slice(&v.to_le_bytes());
    }
    b
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

/// 100 and -100 in alternating runs of 32 samples.
fn fifo_square() -> Vec<u8> {
    (0..32768)
        .map(|i| if i % 64 < 32 { 100i8 } else { -100 } as u8)
        .collect()
}

fn machine(rom: &[u8]) -> Machine {
    Machine::new(&Image::from_rom(rom).unwrap(), 48_000).unwrap()
}

fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; 2 * frames];
    m.render_s16(&mut out);
    out
}

fn block(m: &Machine, index: usize) -> Vec<u8> {
    let mut out = vec![0xEEu8; m.register_block_size(index)];
    assert_eq!(m.register_block_get(index, &mut out), out.len());
    out
}

fn ran(rom: &[u8]) -> Machine {
    let mut m = machine(rom);
    render(&mut m, 2000);
    m
}

#[test]
fn the_blocks_have_names_sizes_and_bases() {
    let m = machine(&Program::new().rom());
    let table = [
        ("Sound", 0x0400_0060, 0x2A),
        ("Wave RAM", 0x0400_0090, 32),
        ("FIFO", 0x0400_00A0, 8),
        ("Timers and DMA sound", 0x0400_0100, 16),
        ("DMA 1 and 2", 0x0400_00BC, 24),
    ];
    assert_eq!(m.register_block_count(), table.len());
    for (i, (name, base, size)) in table.into_iter().enumerate() {
        assert_eq!(m.register_block_name(i), Some(name));
        assert_eq!(m.register_block_base(i), base);
        assert_eq!(m.register_block_size(i), size);
    }
    assert_eq!(m.register_block_name(5), None);
    assert_eq!(m.register_block_base(5), 0);
    assert_eq!(m.register_block_size(5), 0);
    assert_eq!(m.register_block_size(usize::MAX), 0);
}

#[test]
fn a_new_machine_holds_the_bias_the_bios_leaves_and_zeros() {
    let m = machine(&sound_program().rom());
    let mut sound = vec![0u8; 0x2A];
    sound[0x29] = 0x02;
    assert_eq!(block(&m, 0), sound);
    for b in 1..5 {
        assert!(block(&m, b).iter().all(|v| *v == 0), "block {b}");
    }
}

#[test]
fn the_sound_block_holds_the_values_that_were_stored() {
    let m = ran(&sound_program().rom());
    assert_eq!(block(&m, 0), sound_bytes());
    // SOUND1CNT_X reads back with only its length flag, and SOUNDCNT_X with the
    // master bit and the status bits.
    assert_eq!(m.peek(0x0400_0064, 2), 0);
    assert_eq!(m.peek(0x0400_0062, 2), 0xF0C0);
}

#[test]
fn byte_and_word_stores_reach_the_sound_block_too() {
    let rom = Program::new()
        .half(0x84, 0x80)
        .word(0x80, 0x9B07_BD77)
        .byte(0x63, 0x3C)
        .byte(0x62, 0xC5)
        .rom();
    let m = ran(&rom);
    let b = block(&m, 0);
    assert_eq!(b[0x20..0x24], [0x77, 0xBD, 0x07, 0x9B]);
    assert_eq!(b[0x02..0x04], [0xC5, 0x3C]);
}

#[test]
fn turning_the_sound_off_keeps_the_bytes_and_stores_while_off_are_recorded() {
    let rom = Program::new()
        .half(0x62, 0xF080) // before the unit is on
        .half(0x84, 0x80)
        .half(0x80, 0x1111)
        .half(0x84, 0x00)
        .half(0x68, 0x2222) // while it is off
        .rom();
    let m = ran(&rom);
    let b = block(&m, 0);
    assert_eq!(b[0x02..0x04], [0x80, 0xF0]);
    assert_eq!(b[0x20..0x22], [0x11, 0x11]);
    assert_eq!(b[0x08..0x0A], [0x22, 0x22]);
    assert_eq!(b[0x24..0x26], [0, 0]);
    assert_eq!(
        m.peek(0x0400_0080, 2),
        0,
        "the registers themselves cleared"
    );
    assert_eq!(m.peek(0x0400_0068, 2), 0);
}

#[test]
fn the_wave_ram_block_holds_both_banks_in_the_order_of_their_numbers() {
    let on = || Program::new().half(0x84, 0x80);
    let m = ran(&on().wave_ram(1, RAMP).wave_ram(0, SQUARE).rom());
    let wave = block(&m, 1);
    assert_eq!(wave[..16], halves(SQUARE)[..]);
    assert_eq!(wave[16..], halves(RAMP)[..]);
    // Bank 0 plays, so the CPU reaches bank 1.
    let sound = block(&m, 0);
    assert_eq!(sound[0x10] & 0x40, 0);
    assert_eq!(m.peek(0x0400_0090, 2), 0x2301);

    let m = ran(&on().wave_ram(0, SQUARE).wave_ram(1, RAMP).rom());
    let wave = block(&m, 1);
    assert_eq!(wave[..16], halves(SQUARE)[..]);
    assert_eq!(wave[16..], halves(RAMP)[..]);
    assert_eq!(block(&m, 0)[0x10] & 0x40, 0x40);
    assert_eq!(m.peek(0x0400_0090, 2), 0xFFFF);
}

#[test]
fn the_fifo_block_holds_the_last_bytes_stored_to_each_address() {
    let rom = Program::new()
        .word(0xA0, 0x1234_5678)
        .word(0xA4, 0x9ABC_DEF0)
        .half(0xA6, 0xBEEF)
        .byte(0xA1, 0x99)
        .rom();
    let m = ran(&rom);
    assert_eq!(
        block(&m, 2),
        [0x78, 0x99, 0x34, 0x12, 0xF0, 0xDE, 0xEF, 0xBE]
    );
}

#[test]
fn stores_to_a_full_fifo_are_still_recorded() {
    let mut p = Program::new();
    for i in 0..12u32 {
        p = p.word(0xA0, 0x0101_0101 * (i + 1));
    }
    let m = ran(&p.rom());
    assert_eq!(block(&m, 2)[..4], [12, 12, 12, 12]);
}

#[test]
fn a_dma_that_feeds_a_fifo_is_recorded_too() {
    let m = ran(&fifo_program(0x7B04, 0x0080_FE00, &fifo_square()));
    let fifo = block(&m, 2);
    assert!(fifo[..4] == [100; 4] || fifo[..4] == [0x9C; 4], "{fifo:?}");
    assert_eq!(fifo[4..], [0; 4]);
}

#[test]
fn the_timer_block_holds_reload_then_control_for_each_timer() {
    let rom = Program::new()
        .word(0x100, 0x0080_FE00)
        .half(0x104, 0x1234)
        .half(0x106, 0x00C3)
        .half(0x10C, 0xFFFF)
        .rom();
    let m = ran(&rom);
    assert_eq!(
        block(&m, 3),
        [
            0x00, 0xFE, 0x80, 0x00, 0x34, 0x12, 0xC3, 0x00, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0
        ]
    );
}

#[test]
fn the_dma_block_holds_the_registers_of_dma_1_and_dma_2() {
    let m = ran(&fifo_program(0x7B04, 0x0080_FE00, &fifo_square()));
    let dma = block(&m, 4);
    let mut want = vec![0u8; 24];
    want[0..4].copy_from_slice(&0x0800_0048u32.to_le_bytes());
    want[4..8].copy_from_slice(&0x0400_00A0u32.to_le_bytes());
    want[8..10].copy_from_slice(&4u16.to_le_bytes());
    want[10..12].copy_from_slice(&0xB640u16.to_le_bytes());
    assert_eq!(dma, want);
    let timers = block(&m, 3);
    assert_eq!(timers[..4], [0x00, 0xFE, 0x80, 0x00]);
}

#[test]
fn the_dma_block_shows_the_values_as_stored() {
    // Bit 11 does not exist on DMA 2, and a one-shot transfer clears the enable
    // bit when it ends.
    let rom = Program::new()
        .word(0xC8, 0x0800_0000)
        .word(0xCC, 0x0300_0000)
        .word(0xD0, 0x8000_0001)
        .half(0xC6, 0x0800)
        .rom();
    let m = ran(&rom);
    assert_eq!(
        m.peek(0x0400_00D2, 2),
        0,
        "the transfer cleared the enable bit"
    );
    assert_eq!(m.peek(0x0300_0000, 2), 0x0301, "the transfer ran");
    let dma = block(&m, 4);
    assert_eq!(dma[10..12], [0x00, 0x08], "DMA 1 control");
    assert_eq!(dma[12..16], 0x0800_0000u32.to_le_bytes());
    assert_eq!(dma[16..20], 0x0300_0000u32.to_le_bytes());
    assert_eq!(
        dma[20..24],
        [0x01, 0x00, 0x00, 0x80],
        "DMA 2 count and control"
    );
}

#[test]
fn a_short_buffer_gets_the_first_bytes_and_an_invalid_block_gets_none() {
    let m = ran(&sound_program().rom());
    let all = block(&m, 0);
    let mut out = [0xEEu8; 5];
    assert_eq!(m.register_block_get(0, &mut out), 5);
    assert_eq!(out, all[..5]);
    let mut big = [0xEEu8; 64];
    assert_eq!(m.register_block_get(0, &mut big), 0x2A);
    assert_eq!(big[..0x2A], all[..]);
    assert_eq!(big[0x2A..], [0xEE; 22]);
    assert_eq!(m.register_block_get(0, &mut []), 0);
    let mut out = [0xEEu8; 4];
    assert_eq!(m.register_block_get(5, &mut out), 0);
    assert_eq!(m.register_block_get(usize::MAX, &mut out), 0);
    assert_eq!(out, [0xEE; 4]);
}

#[test]
fn reading_the_blocks_does_not_change_the_sound() {
    let roms = [
        sound_program().wave_ram(1, RAMP).rom(),
        fifo_program(0x7B04, 0x0080_FE00, &fifo_square()),
    ];
    for rom in roms {
        let play = |read: bool| {
            let mut m = machine(&rom);
            let mut out = Vec::new();
            for _ in 0..40 {
                out.extend(render(&mut m, 500));
                if read {
                    for b in 0..m.register_block_count() {
                        block(&m, b);
                        m.register_block_name(b);
                        m.register_block_base(b);
                    }
                }
            }
            out
        };
        assert_eq!(play(false), play(true));
    }
}

#[test]
fn a_restore_puts_every_block_back() {
    let rom = fifo_program(0x7B04, 0x0080_FE00, &fifo_square());
    let mut m = machine(&rom);
    let start = m.snapshot();
    let before: Vec<Vec<u8>> = (0..5).map(|b| block(&m, b)).collect();
    render(&mut m, 4000);
    let after: Vec<Vec<u8>> = (0..5).map(|b| block(&m, b)).collect();
    assert_ne!(before[2], after[2]);
    assert_ne!(before[3], after[3]);
    assert_ne!(before[4], after[4]);
    let mid = m.snapshot();
    m.restore(&start).unwrap();
    assert_eq!((0..5).map(|b| block(&m, b)).collect::<Vec<_>>(), before);
    m.restore(&mid).unwrap();
    assert_eq!((0..5).map(|b| block(&m, b)).collect::<Vec<_>>(), after);

    let rom = sound_program().wave_ram(1, RAMP).rom();
    let mut m = machine(&rom);
    let start = m.snapshot();
    render(&mut m, 4000);
    assert_ne!(block(&m, 0), block(&machine(&rom), 0));
    m.restore(&start).unwrap();
    assert_eq!(block(&m, 0), block(&machine(&rom), 0));
    assert_eq!(block(&m, 1), [0; 32]);
}

#[test]
fn a_copy_carries_the_bytes_and_a_reset_clears_them() {
    let rom = sound_program().wave_ram(1, RAMP).rom();
    let mut m = ran(&rom);
    let blocks: Vec<Vec<u8>> = (0..5).map(|b| block(&m, b)).collect();
    let copy = m.clone();
    assert_eq!((0..5).map(|b| block(&copy, b)).collect::<Vec<_>>(), blocks);
    m.reset();
    let fresh = machine(&rom);
    for b in 0..5 {
        assert_eq!(block(&m, b), block(&fresh, b), "block {b}");
    }
}
