//! The register blocks: the registers of each chip as the program wrote them.
//! Reading them changes nothing that the machine plays.

mod common;

use common::*;
use oscilla_nes::Machine;
use oscilla_nsf::{Nsf, chips};

fn stores(list: &[(u16, u8)]) -> Vec<u8> {
    list.iter().flat_map(|&(a, v)| set(a, v)).collect()
}

/// A machine for a file with the chip byte `chip_bits`, with `init` and `play`
/// as its routines, run for a few frames so that the init routine has finished.
fn with_chips(chip_bits: u8, init: &[u8], play: &[u8]) -> Machine {
    let mut f = file(init, play);
    f[0x7B] = chip_bits;
    let mut m = Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap();
    render(&mut m, 2000);
    m
}

fn names(m: &Machine) -> Vec<&'static str> {
    (0..m.register_block_count())
        .map(|b| m.register_block_name(b).unwrap())
        .collect()
}

/// The block called `name`.
fn block_named(m: &Machine, name: &str) -> usize {
    names(m)
        .iter()
        .position(|n| *n == name)
        .unwrap_or_else(|| panic!("no block {name}"))
}

fn bytes(m: &Machine, name: &str) -> Vec<u8> {
    let b = block_named(m, name);
    let mut out = vec![0xEEu8; m.register_block_size(b)];
    assert_eq!(m.register_block_get(b, &mut out), out.len());
    out
}

#[test]
fn a_file_without_chips_has_the_2a03_block() {
    let m = with_chips(0, &[], &[]);
    assert_eq!(m.register_block_count(), 1);
    assert_eq!(m.register_block_name(0), Some("2A03 APU"));
    assert_eq!(m.register_block_base(0), 0x4000);
    assert_eq!(m.register_block_size(0), 24);
    // The player's own stores are in the block: 0x0F to 0x4015 and 0x40 to
    // 0x4017.
    let mut want = [0u8; 24];
    want[0x15] = 0x0F;
    want[0x17] = 0x40;
    assert_eq!(bytes(&m, "2A03 APU"), want);
}

#[test]
fn the_blocks_follow_the_chips_in_a_fixed_order() {
    let all = with_chips(0x7F, &[], &[]);
    assert_eq!(
        names(&all),
        [
            "2A03 APU", "VT02", "VRC6", "5B", "N163", "FDS", "MMC5", "VRC7"
        ]
    );
    let table = [
        (0x4000, 24),
        (0x4020, 22),
        (0x9000, 12),
        (0, 16),
        (0, 128),
        (0x4040, 88),
        (0x5000, 22),
        (0, 54),
    ];
    for (b, (base, size)) in table.into_iter().enumerate() {
        assert_eq!(all.register_block_base(b), base, "block {b}");
        assert_eq!(all.register_block_size(b), size, "block {b}");
    }
    let two = with_chips(chips::FDS | chips::VRC7, &[], &[]);
    assert_eq!(names(&two), ["2A03 APU", "FDS", "VRC7"]);
    assert_eq!(two.register_block_base(1), 0x4040);
    assert_eq!(two.register_block_size(2), 54);
    let mixed = with_chips(chips::N163 | chips::VRC6 | chips::MMC5, &[], &[]);
    assert_eq!(names(&mixed), ["2A03 APU", "VRC6", "N163", "MMC5"]);
    assert_eq!(mixed.register_block_size(2), 128);
}

#[test]
fn an_invalid_block_or_buffer_gives_nothing() {
    let m = with_chips(chips::VRC6, &stores(&[(0x4000, 0x55)]), &[]);
    assert_eq!(m.register_block_count(), 2);
    assert_eq!(m.register_block_name(2), None);
    assert_eq!(m.register_block_base(2), 0);
    assert_eq!(m.register_block_size(2), 0);
    assert_eq!(m.register_block_name(usize::MAX), None);
    let mut out = [0xEEu8; 40];
    assert_eq!(m.register_block_get(2, &mut out), 0);
    assert_eq!(m.register_block_get(0, &mut []), 0);
    assert_eq!(out, [0xEE; 40]);
    // A short buffer takes the first bytes, a long one only the block.
    assert_eq!(m.register_block_get(0, &mut out[..3]), 3);
    assert_eq!(out[..3], [0x55, 0, 0]);
    assert_eq!(m.register_block_get(0, &mut out), 24);
    assert_eq!(out[0], 0x55);
    assert_eq!(out[24..], [0xEE; 16]);
}

#[test]
fn the_2a03_block_holds_every_store_to_the_sound_registers() {
    // A distinct value for each of the 24 registers, including the two that
    // the sound unit ignores and the bits that a read would hide.
    let list: Vec<(u16, u8)> = (0..24u16).map(|i| (0x4000 + i, 0x81 + i as u8)).collect();
    let m = with_chips(0, &stores(&list), &[]);
    let want: Vec<u8> = list.iter().map(|&(_, v)| v).collect();
    assert_eq!(bytes(&m, "2A03 APU"), want);
}

#[test]
fn the_vt02_block_holds_the_stores_to_its_registers() {
    let list: Vec<(u16, u8)> = (0..22u16).map(|i| (0x4020 + i, 0xA0 + i as u8)).collect();
    let m = with_chips(chips::VT02, &stores(&list), &[]);
    let want: Vec<u8> = list.iter().map(|&(_, v)| v).collect();
    assert_eq!(bytes(&m, "VT02"), want);
    assert_eq!(bytes(&m, "2A03 APU")[0], 0);
}

#[test]
fn the_vrc6_block_holds_three_banks_of_four_registers() {
    let mut list: Vec<(u16, u8)> = Vec::new();
    for (i, base) in [0x9000u16, 0xA000, 0xB000].into_iter().enumerate() {
        for r in 0..4 {
            list.push((base + r, 0x10 * (i as u8 + 1) + r as u8));
        }
    }
    let m = with_chips(chips::VRC6, &stores(&list), &[]);
    let want: Vec<u8> = list.iter().map(|&(_, v)| v).collect();
    assert_eq!(bytes(&m, "VRC6"), want);
    // A store to an alias lands in the byte of the address that the chip
    // decodes.
    let alias = with_chips(
        chips::VRC6,
        &stores(&[(0x9000, 0x11), (0x9004, 0x22), (0xB7FB, 0x33)]),
        &[],
    );
    let b = bytes(&alias, "VRC6");
    assert_eq!((b[0], b[11]), (0x22, 0x33));
}

#[test]
fn the_5b_block_is_the_register_file() {
    // Registers 0 to 15 through the select port at 0xC000 and the data port
    // at 0xE000.
    let mut list = Vec::new();
    for r in 0..16u8 {
        list.push((0xC000, r));
        list.push((0xE000, 0x20 + r));
    }
    let m = with_chips(chips::S5B, &stores(&list), &[]);
    let want: Vec<u8> = (0..16).map(|r| 0x20 + r).collect();
    assert_eq!(bytes(&m, "5B"), want);
    // A select with bits above the register number locks the data port.
    list.push((0xC000, 0x13));
    list.push((0xE000, 0x77));
    let locked = with_chips(chips::S5B, &stores(&list), &[]);
    assert_eq!(bytes(&locked, "5B"), want);
}

#[test]
fn the_n163_block_is_the_chips_ram() {
    // The address port at 0xF800 with auto-increment, then the data port at
    // 0x4800: a waveform byte pattern at 0 and channel 8's registers at 0x78.
    let mut list = vec![(0xF800, 0x80)];
    for i in 0..8u8 {
        list.push((0x4800, 0x10 + i));
    }
    list.push((0xF800, 0x80 | 0x78));
    for i in 0..8u8 {
        list.push((0x4800, 0x90 + i));
    }
    let m = with_chips(chips::N163, &stores(&list), &[]);
    let ram = bytes(&m, "N163");
    assert_eq!(ram.len(), 128);
    assert_eq!(ram[..8], [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17]);
    assert_eq!(ram[8..0x78], [0u8; 0x70]);
    // The chip advances the phase of a channel it plays, in bytes 1, 3 and 5 of
    // the channel's eight.
    for i in [0, 2, 4, 6, 7] {
        assert_eq!(ram[0x78 + i], 0x90 + i as u8, "byte {i} of channel 8");
    }
}

#[test]
fn the_fds_block_has_the_wave_ram_and_then_the_registers() {
    let mut list = vec![(0x4089, 0x80)];
    for i in 0..64u16 {
        list.push((0x4040 + i, i as u8 + 0x40));
    }
    list.push((0x4089, 0x00));
    // The chip has writes to the wave RAM disabled now.
    list.push((0x4040, 0x15));
    for i in 0..24u16 {
        if i != 9 {
            list.push((0x4080 + i, 0xC0 + i as u8));
        }
    }
    let m = with_chips(chips::FDS, &stores(&list), &[]);
    let b = bytes(&m, "FDS");
    assert_eq!(b.len(), 88);
    // The wave RAM holds six bits of each store.
    let wave: Vec<u8> = (0..64).map(|i| (i as u8 + 0x40) & 0x3F).collect();
    assert_eq!(b[..64], wave[..]);
    assert_eq!(b[64], 0xC0);
    assert_eq!(b[64 + 23], 0xC0 + 23);
    // 0x4089 was stored, the chip's reads of it hide the bits.
    assert_eq!(b[64 + 9], 0x00);
    assert_eq!(m.register_block_base(block_named(&m, "FDS")), 0x4040);
}

#[test]
fn the_mmc5_block_holds_the_stores_to_its_registers() {
    let mut list: Vec<(u16, u8)> = (0..22u16).map(|i| (0x5000 + i, 0x30 + i as u8)).collect();
    // The multiplier operands are not registers of the sound block.
    list.push((0x5205, 0xFF));
    list.push((0x5206, 0xFF));
    let m = with_chips(chips::MMC5, &stores(&list), &[]);
    let want: Vec<u8> = (0..22).map(|i| 0x30 + i).collect();
    assert_eq!(bytes(&m, "MMC5"), want);
}

#[test]
fn the_vrc7_block_holds_the_internal_registers_as_written() {
    fn reg(r: u8, v: u8) -> [(u16, u8); 2] {
        [(0x9010, r), (0x9030, v)]
    }
    let mut list = Vec::new();
    for r in (0..8)
        .chain([0x0F])
        .chain(0x10..0x16)
        .chain(0x20..0x26)
        .chain(0x30..0x36)
    {
        list.extend(reg(r, 0xF0 | r));
    }
    // Registers that the chip does not decode read 0.
    list.extend(reg(0x08, 0x77));
    list.extend(reg(0x16, 0x77));
    list.extend(reg(0x36, 0x77));
    let m = with_chips(chips::VRC7, &stores(&list), &[]);
    let b = bytes(&m, "VRC7");
    assert_eq!(b.len(), 54);
    for r in 0..54u8 {
        let decoded = matches!(r, 0..=7 | 0x0F | 0x10..=0x15 | 0x20..=0x25 | 0x30..=0x35);
        assert_eq!(
            b[usize::from(r)],
            if decoded { 0xF0 | r } else { 0 },
            "{r:#x}"
        );
    }

    // A reset clears them and the chip ignores stores until it is released.
    let mut again = list.clone();
    again.push((0xE000, 0x80));
    again.extend(reg(0x10, 0x55));
    let reset = with_chips(chips::VRC7, &stores(&again), &[]);
    assert_eq!(bytes(&reset, "VRC7"), [0u8; 54]);
    again.push((0xE000, 0x00));
    again.extend(reg(0x11, 0x66));
    let released = with_chips(chips::VRC7, &stores(&again), &[]);
    let b = bytes(&released, "VRC7");
    assert_eq!((b[0x10], b[0x11]), (0, 0x66));
}

#[test]
fn a_new_song_clears_the_blocks() {
    let list = [(0x4000, 0x55), (0x9001, 0x66), (0x5000, 0x77)];
    let mut m = with_chips(chips::VRC6 | chips::MMC5, &stores(&list), &[]);
    assert_eq!(bytes(&m, "2A03 APU")[0], 0x55);
    assert!(m.select_song(0));
    assert_eq!(bytes(&m, "2A03 APU")[0], 0);
    assert_eq!(bytes(&m, "2A03 APU")[0x15], 0x0F);
    assert_eq!(bytes(&m, "VRC6"), [0u8; 12]);
    assert_eq!(bytes(&m, "MMC5"), [0u8; 22]);
    render(&mut m, 2000);
    assert_eq!(bytes(&m, "2A03 APU")[0], 0x55);
}

/// A register to store the count to, with the select store that it needs first.
type Target = (Option<(u16, u8)>, u16);

/// A play routine that counts in zero page 0 and stores the count to the
/// registers in `targets`, and the same for chips that need a select first.
fn counting(targets: &[Target]) -> Vec<u8> {
    let mut play = vec![0xE6, 0x00];
    for &(select, address) in targets {
        if let Some((port, r)) = select {
            play.extend(set(port, r));
        }
        play.extend([0xA5, 0x00, 0x8D, address as u8, (address >> 8) as u8]);
    }
    play
}

fn all_blocks(m: &Machine) -> Vec<Vec<u8>> {
    names(m).iter().map(|n| bytes(m, n)).collect()
}

#[test]
fn reading_the_blocks_does_not_change_the_sound() {
    let play = counting(&[
        (None, 0x4002),
        (None, 0x4022),
        (None, 0x4082),
        (None, 0x5002),
        (Some((0x9010, 0x10)), 0x9030),
        (Some((0xF800, 0x80)), 0x4800),
    ]);
    let bits = chips::VT02 | chips::FDS | chips::MMC5 | chips::VRC7 | chips::N163;
    let init = stores(&[
        (0x4015, 0x0F),
        (0x4000, 0xBF),
        (0x4003, 0x08),
        (0x4083, 0x00),
    ]);
    let mut plain = with_chips(bits, &init, &play);
    let mut looked = with_chips(bits, &init, &play);
    let mut buf = [0u8; 200];
    for _ in 0..12 {
        let a = render(&mut plain, 700);
        let b = render(&mut looked, 700);
        assert_eq!(a, b);
        for block in 0..looked.register_block_count() {
            looked.register_block_get(block, &mut buf);
            looked.register_block_name(block);
            looked.register_block_base(block);
            looked.register_block_size(block);
        }
    }
    assert_eq!(all_blocks(&plain), all_blocks(&looked));
}

#[test]
fn a_restore_puts_the_blocks_back() {
    let rows: [(u8, Vec<Target>); 2] = [
        (
            chips::VT02 | chips::FDS | chips::MMC5 | chips::VRC7 | chips::N163,
            vec![
                (None, 0x4002),
                (None, 0x4014),
                (None, 0x4022),
                (None, 0x4082),
                (None, 0x5002),
                (Some((0x9010, 0x10)), 0x9030),
                (Some((0x9010, 0x31)), 0x9030),
                (Some((0xF800, 0x80 | 0x7F)), 0x4800),
            ],
        ),
        (
            chips::VRC6 | chips::S5B,
            vec![
                (None, 0x4002),
                (None, 0x9001),
                (None, 0xA002),
                (None, 0xB003),
                (Some((0xC000, 5)), 0xE000),
            ],
        ),
    ];
    for (bits, targets) in rows {
        let play = counting(&targets);
        let mut m = with_chips(bits, &[], &play);
        render(&mut m, 3000);
        let snapshot = m.snapshot();
        let before = all_blocks(&m);
        let size = snapshot.size();
        let first = render(&mut m, 9600);
        let after = all_blocks(&m);
        assert_ne!(before, after, "the play routine changes the registers");
        m.restore(&snapshot).unwrap();
        assert_eq!(all_blocks(&m), before, "chips {bits:#x}");
        assert_eq!(render(&mut m, 9600), first);
        assert_eq!(all_blocks(&m), after);
        assert_eq!(snapshot.size(), size);
        assert!(m.size() >= std::mem::size_of::<Machine>());

        // Another machine of the same file takes the blocks too.
        let mut other = with_chips(bits, &[], &play);
        other.restore(&snapshot).unwrap();
        assert_eq!(all_blocks(&other), before);
    }
}
