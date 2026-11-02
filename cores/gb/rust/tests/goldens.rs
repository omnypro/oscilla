//! Digests of the sound and the register stores of generated drivers. Each
//! program writes random values to the sound registers in its init routine
//! and keeps writing in its play routine, with the three timing sources of a
//! GBS. The digests freeze the machine's behavior: a change that moves one
//! needs a reason.
//!
//! Regenerate `goldens/gb.txt` with
//! `cargo test -p oscilla-gb --release --test goldens -- --ignored --nocapture print`
//! and keep the lines between the markers. The cutoff block goes in
//! `goldens/gb-highpass.txt`.

mod common;

use common::*;
use oscilla_gb::Machine;
use oscilla_gbs::Gbs;

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

/// A register store: random channel, random register, a value that is a
/// trigger half of the time for the control registers.
fn random_store(r: &mut Rng) -> Vec<u8> {
    let ch = r.below(5);
    let reg = match ch {
        0 => 0x10 + r.below(5) as u8,
        1 => 0x16 + r.below(4) as u8,
        2 => 0x1A + r.below(5) as u8,
        3 => 0x20 + r.below(4) as u8,
        _ => 0x24 + r.below(2) as u8,
    };
    let mut v = r.byte();
    if matches!(reg, 0x14 | 0x19 | 0x1E | 0x23) && r.below(2) == 0 {
        v |= 0x80;
    }
    set(reg, v)
}

fn program(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    let mut init = [set(0x26, 0x80), set(0x24, 0x77), set(0x25, r.byte() | 0x11)].concat();
    for i in 0..16 {
        init.extend(set(0x30 + i, r.byte()));
    }
    for _ in 0..20 + r.below(30) {
        init.extend(random_store(&mut r));
    }
    // The play routine counts calls in HRAM and moves a pitch with the count.
    let (low, high) = (0x13 + 5 * r.below(2) as u8, 0x14);
    let mut play = vec![
        0xF0, 0x80, 0x3C, 0xE0, 0x80, 0xE0, low, 0xE6, 0x07, 0xF6, 0x80, 0xE0,
    ];
    play.push(if low == 0x13 { high } else { 0x19 });
    for _ in 0..r.below(6) {
        play.extend(random_store(&mut r));
    }
    let (tma, tac) = match r.below(3) {
        0 => (0, 0),
        1 => (r.byte() | 0xC0, 0x04),
        _ => (r.byte() | 0x80, 0x05 + r.below(3) as u8),
    };
    file(&init, &play, tma, tac, None)
}

fn run(seed: u64) -> (u64, u64) {
    run_with(seed, None)
}

fn run_with(seed: u64, cutoff: Option<f64>) -> (u64, u64) {
    let gbs = Gbs::parse(&program(seed)).unwrap();
    let mut m = Machine::new(&gbs, 48000).unwrap();
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
const CUTOFFS: [f64; 3] = [100.0, 300.0, 1000.0];

fn expected() -> Vec<(u64, u64, u64)> {
    include_str!("goldens/gb.txt")
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
fn the_digests_with_a_cutoff_match() {
    let want: Vec<(u64, u64, u64)> = include_str!("goldens/gb-highpass.txt")
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
            let gbs = Gbs::parse(&program(s)).unwrap();
            let mut m = Machine::new(&gbs, 48000).unwrap();
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
    println!("-----BEGIN CUTOFF GOLDENS-----");
    for seed in 0..CUTOFF_SEEDS {
        for hz in CUTOFFS {
            println!("{seed} {hz} {:016x}", run_with(seed, Some(hz)).0);
        }
    }
    println!("-----END CUTOFF GOLDENS-----");
}
