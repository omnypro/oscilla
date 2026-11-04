//! Digests of the sound and the DSP stores of generated files. Each file holds
//! random BRR blocks, a random sample table, random voice and echo settings and
//! a driver that keeps writing random values to the DSP. The digests freeze the
//! machine's behavior: a change that moves one needs a reason.
//!
//! Regenerate `goldens/snes.txt` with
//! `cargo test -p oscilla-snes --release --test goldens -- --ignored --nocapture print`
//! and keep the lines between the markers.

mod common;

use common::*;
use oscilla_snes::{Machine, Observer, Spc};
use std::cell::RefCell;
use std::rc::Rc;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// A store to a random DSP register that a driver may reasonably write.
fn random_store(r: &mut Rng) -> (u8, u8) {
    let voice = r.below(8) as u8;
    match r.below(12) {
        0 => (voice * 16, r.byte()),
        1 => (voice * 16 + 1, r.byte()),
        2 => (voice * 16 + 2, r.byte()),
        3 => (voice * 16 + 3, r.byte() & 0x3F),
        4 => (voice * 16 + 5, r.byte()),
        5 => (voice * 16 + 6, r.byte()),
        6 => (voice * 16 + 7, r.byte()),
        7 => (0x4C, r.byte()),
        8 => (0x5C, if r.below(3) == 0 { r.byte() } else { 0 }),
        9 => (0x3D, r.byte()),
        10 => (0x2D, r.byte()),
        _ => (0x4D, r.byte()),
    }
}

fn file(seed: u64) -> Spc {
    let mut r = Rng(seed);
    let mut dsp: Vec<(u8, u8)> = Vec::new();
    for v in 0..8u8 {
        dsp.push((v * 16, r.byte()));
        dsp.push((v * 16 + 1, r.byte()));
        dsp.push((v * 16 + 2, r.byte()));
        dsp.push((v * 16 + 3, r.byte() & 0x3F));
        dsp.push((v * 16 + 4, r.below(8) as u8));
        dsp.push((v * 16 + 5, r.byte()));
        dsp.push((v * 16 + 6, r.byte()));
        dsp.push((v * 16 + 7, r.byte()));
    }
    for reg in [0x0C, 0x1C, 0x2C, 0x3C, 0x0D] {
        dsp.push((reg, r.byte()));
    }
    for k in 0..8u8 {
        dsp.push((0x0F + 0x10 * k, r.byte()));
    }
    dsp.push((0x5D, 0x10));
    dsp.push((0x6D, 0x40 + r.below(0x20) as u8));
    dsp.push((0x7D, r.below(6) as u8));
    dsp.push((0x6C, r.byte() & 0x3F));
    dsp.push((0x2D, r.byte()));
    dsp.push((0x3D, r.byte()));
    dsp.push((0x4D, r.byte()));
    dsp.push((0x4C, r.byte() | 1));
    let writes: Vec<(u8, u8)> = (0..40 + r.below(60)).map(|_| random_store(&mut r)).collect();
    // The driver spaces its stores with a short delay loop and starts over.
    let mut program = Vec::new();
    for (reg, value) in &writes {
        program.extend_from_slice(&[0x8F, *reg, 0xF2, 0x8F, *value, 0xF3]);
        program.extend_from_slice(&[0xCD, 0x20 + r.below(200) as u8, 0x1D, 0xD0, 0xFD]);
    }
    program.extend_from_slice(&[0x2F, 0xFE]);
    // Eight sources of random BRR blocks, each of one to four blocks that
    // loop, with a random sample table.
    let mut blocks = Vec::new();
    let mut table = Vec::new();
    for source in 0..8u16 {
        let at = 0x2000 + source * 0x100;
        let count = 1 + r.below(4) as u16;
        for b in 0..count {
            let mut blk = vec![r.byte() & 0xFC | if b + 1 == count { 3 } else { 0 }];
            blk.extend((0..8).map(|_| r.byte()));
            blocks.push((at + b * 9, blk));
        }
        table.extend_from_slice(&[at as u8, (at >> 8) as u8, at as u8, (at >> 8) as u8]);
    }
    blocks.push((0x1000, table));
    // The echo buffer starts with noise.
    blocks.push((0x4000, (0..0x400).map(|_| r.byte()).collect()));
    dsp_file(&program, &dsp, &blocks, 0x2000, 0x2000)
}

struct Writes(Rc<RefCell<Vec<u8>>>);

impl Observer for Writes {
    fn dsp_write(&mut self, cycle: u64, register: u8, value: u8) {
        let mut v = self.0.borrow_mut();
        v.extend_from_slice(&cycle.to_le_bytes());
        v.push(register);
        v.push(value);
    }
    fn port_write(&mut self, _: u64, _: u8, _: u8) {}
}

fn run(seed: u64) -> (u64, u64) {
    let spc = file(seed);
    let mut m = Machine::new(&spc, 48000).unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    m.set_observer(Some(Box::new(Writes(log.clone()))));
    let out = render(&mut m, 48000);
    let stores = fnv(log.borrow().iter().copied());
    (digest(&out), stores)
}

const SEEDS: u64 = 32;

#[test]
fn the_goldens_hold() {
    let text = include_str!("goldens/snes.txt");
    let want: Vec<(u64, u64, u64)> = text
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (
                f[0].parse().unwrap(),
                u64::from_str_radix(f[1], 16).unwrap(),
                u64::from_str_radix(f[2], 16).unwrap(),
            )
        })
        .collect();
    assert_eq!(want.len() as u64, SEEDS);
    for (seed, samples, stores) in want {
        assert_eq!(run(seed), (samples, stores), "seed {seed}");
    }
}

#[test]
#[ignore]
fn print() {
    println!("--- begin");
    for seed in 0..SEEDS {
        let (samples, stores) = run(seed);
        println!("{seed} {samples:016x} {stores:016x}");
    }
    println!("--- end");
}
