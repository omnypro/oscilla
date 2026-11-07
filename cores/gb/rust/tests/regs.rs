//! The register blocks: the bytes that a program stored to the sound
//! registers and to wave RAM.

mod common;

use common::*;
use oscilla_gb::Machine;

const BOOT: [u8; 23] = {
    let mut b = [0; 23];
    b[0x01] = 0x80;
    b[0x02] = 0xF3;
    b[0x14] = 0x77;
    b[0x15] = 0xF3;
    b[0x16] = 0x80;
    b
};

fn block(m: &Machine, index: usize) -> Vec<u8> {
    let mut out = vec![0xEEu8; m.register_block_size(index)];
    assert_eq!(m.register_block_get(index, &mut out), out.len());
    out
}

fn stores(pairs: &[(u8, u8)]) -> Vec<u8> {
    pairs.iter().flat_map(|(r, v)| set(*r, *v)).collect()
}

/// A bright pulse 1 with the sweep off, so that nothing rewrites NR13.
const PULSE: [(u8, u8); 8] = [
    (0x26, 0x80),
    (0x24, 0x77),
    (0x25, 0xFF),
    (0x10, 0x00),
    (0x11, 0x8F),
    (0x12, 0xF0),
    (0x13, 0xA5),
    (0x14, 0x87),
];

#[test]
fn the_blocks_have_names_sizes_and_bases() {
    let m = machine(&[], &[]);
    assert_eq!(m.register_block_count(), 2);
    assert_eq!(m.register_block_name(0), Some("APU"));
    assert_eq!(m.register_block_name(1), Some("Wave RAM"));
    assert_eq!(m.register_block_name(2), None);
    assert_eq!(m.register_block_size(0), 23);
    assert_eq!(m.register_block_size(1), 16);
    assert_eq!(m.register_block_size(2), 0);
    assert_eq!(m.register_block_base(0), 0xFF10);
    assert_eq!(m.register_block_base(1), 0xFF30);
    assert_eq!(m.register_block_base(2), 0);
    assert_eq!(m.register_block_base(usize::MAX), 0);
}

#[test]
fn the_blocks_hold_the_boot_values_before_the_program_runs() {
    let m = machine(&stores(&PULSE), &[]);
    assert_eq!(block(&m, 0), BOOT);
    assert_eq!(block(&m, 1), [0; 16]);
}

#[test]
fn a_program_reads_back_what_it_stored() {
    let mut m = machine(&stores(&PULSE), &[]);
    render(&mut m, 2000);
    let apu = block(&m, 0);
    assert_eq!(apu.len(), 23);
    assert_eq!(apu[0x00], 0x00); // NR10
    assert_eq!(apu[0x01], 0x8F); // NR11
    assert_eq!(apu[0x02], 0xF0); // NR12
    assert_eq!(apu[0x03], 0xA5); // NR13, which reads back as 0xFF
    assert_eq!(apu[0x04], 0x87); // NR14, which reads back as 0xBF
    assert_eq!(apu[0x14], 0x77); // NR50
    assert_eq!(apu[0x15], 0xFF); // NR51
    assert_eq!(apu[0x16], 0x80); // NR52
    assert_eq!(m.peek(0xFF13), 0xFF);
    assert_eq!(m.peek(0xFF14), 0xBF);
}

#[test]
fn the_unused_addresses_stay_zero() {
    let init = stores(&[(0x26, 0x80), (0x15, 0x55), (0x1F, 0xAA), (0x20, 0x3F)]);
    let mut m = machine(&init, &[]);
    render(&mut m, 2000);
    let apu = block(&m, 0);
    assert_eq!(apu[0x05], 0);
    assert_eq!(apu[0x0F], 0);
    assert_eq!(apu[0x10], 0x3F); // NR41, the next address
}

#[test]
fn nr52_holds_the_byte_that_was_stored() {
    let mut m = machine(&stores(&[(0x26, 0xFF)]), &[]);
    render(&mut m, 2000);
    assert_eq!(block(&m, 0)[0x16], 0xFF);
}

#[test]
fn wave_ram_holds_the_bytes_stored_to_it() {
    let mut init = stores(&PULSE[..1]);
    let ram: Vec<u8> = (0..16).map(|i| (0x11 * i as u8) ^ 0x5A).collect();
    for (i, b) in ram.iter().enumerate() {
        init.extend(set(0x30 + i as u8, *b));
    }
    let mut m = machine(&init, &[]);
    render(&mut m, 2000);
    assert_eq!(block(&m, 1), ram);
    for (i, b) in ram.iter().enumerate() {
        assert_eq!(m.peek(0xFF30 + i as u16), *b);
    }
}

#[test]
fn the_sweep_does_not_change_the_bytes() {
    // A sweep of period 1 and shift 1 adds half of the period each step.
    let init = stores(&[
        (0x26, 0x80),
        (0x24, 0x77),
        (0x25, 0xFF),
        (0x10, 0x11),
        (0x11, 0x80),
        (0x12, 0xF0),
        (0x13, 0x00),
        (0x14, 0x84),
    ]);
    let mut m = machine(&init, &[]);
    render(&mut m, 100);
    let first = m.voice(0).unwrap().frequency_hz;
    render(&mut m, 12_000);
    let later = m.voice(0).unwrap().frequency_hz;
    assert_ne!(first, later, "the sweep moved the pitch");
    let apu = block(&m, 0);
    assert_eq!(apu[0x03], 0x00);
    assert_eq!(apu[0x04], 0x84);
}

#[test]
fn turning_the_unit_off_keeps_the_bytes_and_stores_are_still_recorded() {
    let init = [
        stores(&PULSE),
        stores(&[(0x26, 0x00), (0x12, 0x77), (0x17, 0x99)]),
    ]
    .concat();
    let mut m = machine(&init, &[]);
    render(&mut m, 2000);
    let apu = block(&m, 0);
    assert_eq!(apu[0x16], 0x00);
    assert_eq!(apu[0x01], 0x8F);
    assert_eq!(apu[0x02], 0x77, "a store while the unit is off");
    assert_eq!(apu[0x07], 0x99);
    assert_eq!(apu[0x03], 0xA5);
    assert_eq!(m.peek(0xFF12), 0x00, "the registers themselves cleared");
}

#[test]
fn the_play_routine_changes_the_bytes_on_each_call() {
    let mut m = machine(&stores(&PULSE), &set(0x13, 0x42));
    render(&mut m, 4000);
    assert_eq!(block(&m, 0)[0x03], 0x42);
}

#[test]
fn a_short_buffer_gets_the_first_bytes_and_an_invalid_block_gets_none() {
    let mut m = machine(&stores(&PULSE), &[]);
    render(&mut m, 2000);
    let all = block(&m, 0);
    let mut out = [0xEEu8; 5];
    assert_eq!(m.register_block_get(0, &mut out), 5);
    assert_eq!(out, all[..5]);
    let mut big = [0xEEu8; 40];
    assert_eq!(m.register_block_get(0, &mut big), 23);
    assert_eq!(big[..23], all[..]);
    assert_eq!(big[23..], [0xEE; 17]);
    assert_eq!(m.register_block_get(0, &mut []), 0);
    let mut out = [0xEEu8; 4];
    assert_eq!(m.register_block_get(2, &mut out), 0);
    assert_eq!(m.register_block_get(usize::MAX, &mut out), 0);
    assert_eq!(out, [0xEE; 4]);
}

#[test]
fn reading_the_blocks_does_not_change_the_sound() {
    let init = [stores(&PULSE), stores(&[(0x30, 0x12), (0x3F, 0x34)])].concat();
    let play = |read: bool| {
        let mut m = machine(&init, &set(0x13, 0x42));
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

#[test]
fn a_restore_puts_the_bytes_back() {
    let mut m = machine(&stores(&PULSE), &set(0x13, 0x42));
    let start = m.snapshot();
    render(&mut m, 4000);
    let ran = block(&m, 0);
    assert_ne!(ran, BOOT);
    let mid = m.snapshot();
    m.restore(&start).unwrap();
    assert_eq!(block(&m, 0), BOOT);
    m.restore(&mid).unwrap();
    assert_eq!(block(&m, 0), ran);
}

#[test]
fn a_copy_carries_the_bytes_and_a_reset_clears_them() {
    let mut m = machine(&stores(&PULSE), &[]);
    render(&mut m, 2000);
    let ran = block(&m, 0);
    let copy = m.clone();
    assert_eq!(block(&copy, 0), ran);
    m.reset();
    assert_eq!(block(&m, 0), BOOT);
    assert_eq!(block(&m, 1), [0; 16]);
    render(&mut m, 2000);
    assert_eq!(block(&m, 0), ran);
}
