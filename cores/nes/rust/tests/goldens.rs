//! Digests of the sound and the register stores of generated drivers. Each
//! program writes random values to the sound registers in its init routine
//! and keeps writing in its play routine, with a sample for the delta channel
//! at 0xC000. The digests freeze the machine's behavior: a change that moves
//! one needs a reason.
//!
//! Regenerate `goldens/nes.txt` with
//! `cargo test -p oscilla-nes --release --test goldens -- --ignored --nocapture print`
//! and keep the lines between the markers. The cutoff block goes in
//! `goldens/nes-highpass.txt`, and the VRC6 block in `goldens/nes-vrc6.txt`.

mod common;

use common::*;
use oscilla_nes::Machine;
use oscilla_nsf::Nsf;

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

/// A register store: a random sound register with a random value.
fn random_store(r: &mut Rng) -> Vec<u8> {
    let reg = match r.below(24) {
        n @ 0..=19 => 0x4000 + n as u16,
        20 => 0x4015,
        21 => 0x4017,
        n => 0x4000 + (n as u16 - 22) * 4 + 3,
    };
    let mut v = r.byte();
    if reg == 0x4015 && r.below(4) != 0 {
        v |= 0x0F;
    }
    set(reg, v)
}

fn program(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    let mut init = set(0x4015, 0x1F);
    for _ in 0..20 + r.below(30) {
        init.extend(random_store(&mut r));
    }
    // The play routine counts calls and moves a pitch with the count.
    let target = [0x4002u16, 0x4006, 0x400A, 0x400E][r.below(4) as usize];
    let mut play = vec![
        0xE6,
        0x00,
        0xA5,
        0x00,
        0x8D,
        target as u8,
        (target >> 8) as u8,
    ];
    for _ in 0..r.below(6) {
        play.extend(random_store(&mut r));
    }
    let us = [16666u16, 16639, 20000, 8000 + r.below(30000) as u16][r.below(4) as usize];
    let region = r.below(3) as u8;
    let sample: Vec<u8> = (0..0x400).map(|_| r.byte()).collect();
    file_with(&init, &play, us, region, [0; 8], Some((0x4000, &sample)))
}

/// A store to a VRC6 register, with the enable bits set most of the time.
fn random_vrc6_store(r: &mut Rng) -> Vec<u8> {
    let reg = [
        0x9000u16, 0x9001, 0x9002, 0xA000, 0xA001, 0xA002, 0xB000, 0xB001, 0xB002, 0x9003,
    ][r.below(10) as usize];
    let mut v = r.byte();
    if matches!(reg, 0x9002 | 0xA002 | 0xB002) && r.below(4) != 0 {
        v |= 0x80;
    }
    if reg == 0x9003 {
        v &= 0x03;
    }
    set(reg, v)
}

/// A file that asks for the VRC6 and drives it and the 2A03 together.
fn vrc6_program(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed ^ 0x5EED);
    let mut init = set(0x4015, 0x1F);
    for _ in 0..10 + r.below(10) {
        init.extend(random_store(&mut r));
    }
    for _ in 0..20 + r.below(20) {
        init.extend(random_vrc6_store(&mut r));
    }
    let target = [0x9001u16, 0xA001, 0xB001][r.below(3) as usize];
    let mut play = vec![
        0xE6,
        0x00,
        0xA5,
        0x00,
        0x8D,
        target as u8,
        (target >> 8) as u8,
    ];
    for _ in 0..r.below(6) {
        play.extend(random_vrc6_store(&mut r));
    }
    let mut f = file_with(&init, &play, 16666, r.below(2) as u8, [0; 8], None);
    f[0x7B] = oscilla_nsf::chips::VRC6;
    f
}

fn run_vrc6(seed: u64) -> (u64, u64) {
    let nsf = Nsf::parse(&vrc6_program(seed)).unwrap();
    let mut m = Machine::new(&nsf, 48000).unwrap();
    let seen = watch(&mut m);
    let out = render(&mut m, 96000);
    let s = seen.borrow();
    let writes = fnv(s.writes.iter().flat_map(|(c, a, v)| {
        c.to_le_bytes()
            .into_iter()
            .chain(a.to_le_bytes())
            .chain([*v])
    }));
    (digest(&out), writes)
}

const VRC6_SEEDS: u64 = 16;

fn run(seed: u64) -> (u64, u64) {
    run_with(seed, None)
}

fn run_with(seed: u64, cutoff: Option<f64>) -> (u64, u64) {
    let nsf = Nsf::parse(&program(seed)).unwrap();
    let mut m = Machine::new(&nsf, 48000).unwrap();
    if let Some(hz) = cutoff {
        assert!(m.set_highpass_hz(hz));
    }
    let seen = watch(&mut m);
    let out = render(&mut m, 96000);
    let s = seen.borrow();
    let writes = fnv(s.writes.iter().flat_map(|(c, a, v)| {
        c.to_le_bytes()
            .into_iter()
            .chain(a.to_le_bytes())
            .chain([*v])
    }));
    (digest(&out), writes)
}

const SEEDS: u64 = 64;

/// The seeds and cutoffs of the second list: a custom cutoff builds its own
/// factors, so it gets its own digests.
const CUTOFF_SEEDS: u64 = 8;
const CUTOFFS: [f64; 3] = [150.0, 300.0, 1000.0];

fn expected() -> Vec<(u64, u64, u64)> {
    include_str!("goldens/nes.txt")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (
                p[0].parse().unwrap(),
                u64::from_str_radix(p[1], 16).unwrap(),
                u64::from_str_radix(p[2], 16).unwrap(),
            )
        })
        .collect()
}

#[test]
fn the_digests_match() {
    let want = expected();
    assert_eq!(want.len() as u64, SEEDS);
    let mut bad = Vec::new();
    for (seed, audio, writes) in want {
        let got = run(seed);
        if got != (audio, writes) {
            bad.push(format!(
                "seed {seed}: audio {:016x} (want {audio:016x}), writes {:016x} (want {writes:016x})",
                got.0, got.1
            ));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn the_digests_with_the_vrc6_match() {
    let want: Vec<(u64, u64, u64)> = include_str!("goldens/nes-vrc6.txt")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (
                p[0].parse().unwrap(),
                u64::from_str_radix(p[1], 16).unwrap(),
                u64::from_str_radix(p[2], 16).unwrap(),
            )
        })
        .collect();
    assert_eq!(want.len() as u64, VRC6_SEEDS);
    let mut bad = Vec::new();
    for (seed, audio, writes) in want {
        let got = run_vrc6(seed);
        if got != (audio, writes) {
            bad.push(format!(
                "seed {seed}: audio {:016x} (want {audio:016x}), writes {:016x} (want {writes:016x})",
                got.0, got.1
            ));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn the_digests_with_a_cutoff_match() {
    let want: Vec<(u64, u64, u64)> = include_str!("goldens/nes-highpass.txt")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (
                p[0].parse().unwrap(),
                p[1].parse().unwrap(),
                u64::from_str_radix(p[2], 16).unwrap(),
            )
        })
        .collect();
    assert_eq!(want.len() as u64, CUTOFF_SEEDS * CUTOFFS.len() as u64);
    let mut bad = Vec::new();
    for (seed, hz, audio) in want {
        let got = run_with(seed, Some(hz as f64)).0;
        if got != audio {
            bad.push(format!(
                "seed {seed} at {hz} Hz: {got:016x} (want {audio:016x})"
            ));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_cutoff_changes_the_audio_and_leaves_the_writes() {
    for seed in 0..CUTOFF_SEEDS {
        let (audio, writes) = run(seed);
        let (thin_audio, thin_writes) = run_with(seed, Some(300.0));
        assert_eq!(writes, thin_writes, "seed {seed}");
        assert_ne!(audio, thin_audio, "seed {seed}");
    }
}

#[test]
fn the_programs_make_sound_and_differ() {
    let digests: std::collections::HashSet<_> = (0..SEEDS).map(|s| run(s).0).collect();
    assert!(
        digests.len() as u64 > SEEDS - 4,
        "{} distinct",
        digests.len()
    );
    let loud = (0..SEEDS)
        .filter(|&s| {
            let nsf = Nsf::parse(&program(s)).unwrap();
            let mut m = Machine::new(&nsf, 48000).unwrap();
            let out = render(&mut m, 48000);
            out.iter().map(|v| i32::from(*v).abs()).max().unwrap() > 1000
        })
        .count();
    assert!(loud as u64 > SEEDS / 2, "{loud} of {SEEDS} are audible");
}

#[test]
#[ignore = "generator"]
fn print() {
    println!("-----BEGIN GOLDENS-----");
    for seed in 0..SEEDS {
        let (audio, writes) = run(seed);
        println!("{seed} {audio:016x} {writes:016x}");
    }
    println!("-----END GOLDENS-----");
    println!("-----BEGIN VRC6 GOLDENS-----");
    for seed in 0..VRC6_SEEDS {
        let (audio, writes) = run_vrc6(seed);
        println!("{seed} {audio:016x} {writes:016x}");
    }
    println!("-----END VRC6 GOLDENS-----");
    println!("-----BEGIN CUTOFF GOLDENS-----");
    for seed in 0..CUTOFF_SEEDS {
        for hz in CUTOFFS {
            println!("{seed} {hz} {:016x}", run_with(seed, Some(hz)).0);
        }
    }
    println!("-----END CUTOFF GOLDENS-----");
}
