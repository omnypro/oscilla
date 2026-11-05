//! Digests of the sound of generated files. Each file holds a PCM data block
//! and a few hundred random commands: stores to the YM2612's registers, writes
//! to the SN76489, DAC commands and waits. The digests freeze the machine's
//! behavior: a change that moves one needs a reason.
//!
//! Regenerate `goldens/genesis.txt` with
//! `cargo test -p oscilla-genesis --release --test goldens -- --ignored --nocapture print`
//! and keep the lines between the markers.

mod common;

use common::*;
use oscilla_genesis::Machine;

/// A register of the YM2612 that a driver may reasonably write.
fn fm_register(r: &mut Rng) -> u8 {
    match r.below(10) {
        0 => [0x22, 0x24, 0x25, 0x26, 0x27, 0x2B, 0x2C][r.below(7) as usize],
        1 => 0xA0 + [0, 1, 2, 4, 5, 6][r.below(6) as usize],
        2 => 0xA8 + r.below(3) as u8,
        3 => 0xB0 + r.below(3) as u8 + 4 * r.below(2) as u8,
        _ => {
            let group = 3 + r.below(7) as u8;
            let slot = r.below(3) as u8 + 4 * r.below(4) as u8;
            group * 16 + slot
        }
    }
}

fn stream(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    let mut s = vec![0x67, 0x66, 0x00, 64, 0, 0, 0];
    s.extend((0..64).map(|_| r.byte()));
    // Every file starts with audible keyed channels.
    for ch in [0u8, 1, 2, 4, 5, 6] {
        s.extend([0x52, 0x28, 0xF0 | ch]);
    }
    for _ in 0..400 {
        match r.below(10) {
            0..=4 => {
                let reg = fm_register(&mut r);
                let value = r.byte();
                s.extend([0x52 + (r.below(2) as u8), reg, value]);
            }
            5 => s.extend([0x52, 0x28, (r.byte() & 0xF0) | [0, 1, 2, 4, 5, 6][r.below(6) as usize]]),
            6 | 7 => s.extend([0x50, r.byte()]),
            8 => s.push(0x80 + r.below(16) as u8),
            _ => {
                let n = 1 + r.below(1500) as u16;
                s.extend(wait(n));
            }
        }
    }
    s.extend(wait(2000));
    s.push(0x66);
    s
}

fn run(seed: u64) -> u64 {
    let v = parse(&file(true, true, &stream(seed), None));
    let mut m = Machine::new(&v, 48_000).unwrap();
    digest(&render(&mut m, 48_000))
}

const SEEDS: u64 = 32;

#[test]
fn the_goldens_hold() {
    let text = include_str!("goldens/genesis.txt");
    let want: Vec<(u64, u64)> = text
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f[0].parse().unwrap(), u64::from_str_radix(f[1], 16).unwrap())
        })
        .collect();
    assert_eq!(want.len() as u64, SEEDS);
    for (seed, samples) in want {
        assert_eq!(run(seed), samples, "seed {seed}");
    }
}

#[test]
#[ignore]
fn print() {
    println!("--- begin");
    for seed in 0..SEEDS {
        println!("{seed} {:016x}", run(seed));
    }
    println!("--- end");
}

#[test]
fn the_generated_files_make_sound() {
    for seed in 0..SEEDS {
        let v = parse(&file(true, true, &stream(seed), None));
        let mut m = Machine::new(&v, 48_000).unwrap();
        let out = render(&mut m, 48_000);
        assert!(out.iter().any(|s| s.abs() > 200), "seed {seed}");
    }
}
