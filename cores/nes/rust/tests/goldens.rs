//! Digests of the sound and the register stores of generated drivers. Each
//! program writes random values to the sound registers in its init routine
//! and keeps writing in its play routine, with a sample for the delta channel
//! at 0xC000. The digests freeze the machine's behavior: a change that moves
//! one needs a reason.
//!
//! Regenerate `goldens/nes.txt` with
//! `cargo test -p oscilla-nes --release --test goldens -- --ignored --nocapture print`
//! and keep the lines between the markers. The cutoff block goes in
//! `goldens/nes-highpass.txt`, the VRC6 block in `goldens/nes-vrc6.txt` the 5B block in
//! `goldens/nes-s5b.txt` the Namco 163 block in `goldens/nes-n163.txt` and the FDS block in
//! `goldens/nes-fds.txt`.

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

/// A store to a register of the Sunsoft 5B through its ports.
fn random_s5b_store(r: &mut Rng) -> Vec<u8> {
    let reg = r.below(14) as u8;
    let mut v = r.byte();
    match reg {
        1 | 3 | 5 => v &= 0x0F,
        6 => v &= 0x1F,
        0x0D => v &= 0x0F,
        _ => {}
    }
    let mut out = set(0xC000, reg);
    out.extend(set(0xE000, v));
    out
}

/// A file that asks for the Sunsoft 5B and drives it and the 2A03 together.
fn s5b_program(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed ^ 0xB5B5);
    let mut init = set(0x4015, 0x1F);
    for _ in 0..6 + r.below(8) {
        init.extend(random_store(&mut r));
    }
    for _ in 0..24 + r.below(24) {
        init.extend(random_s5b_store(&mut r));
    }
    let mut play = vec![0xE6, 0x00, 0xA5, 0x00];
    play.extend(set(0xC000, [0, 2, 4][r.below(3) as usize]));
    play.extend([0x8D, 0x00, 0xE0]);
    for _ in 0..r.below(6) {
        play.extend(random_s5b_store(&mut r));
    }
    let mut f = file_with(&init, &play, 16666, r.below(2) as u8, [0; 8], None);
    f[0x7B] = oscilla_nsf::chips::S5B;
    f
}

fn digests_of(file: Vec<u8>) -> (u64, u64) {
    let nsf = Nsf::parse(&file).unwrap();
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

/// A file that asks for the Namco 163 and loads random waveforms and channel
/// registers, then moves a channel's pitch in the play routine.
fn n163_program(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed ^ 0x163);
    let mut init = set(0x4015, 0x1F);
    for _ in 0..4 + r.below(6) {
        init.extend(random_store(&mut r));
    }
    // Waveform RAM from address 0, with the address stepping.
    init.extend(set(0xF800, 0x80));
    for _ in 0..16 + r.below(48) {
        init.extend(set(0x4800, r.byte()));
    }
    // The registers of every channel, and the channel count in the last one.
    for c in 0..8u8 {
        init.extend(set(0xF800, 0x80 | (0x40 + 8 * c)));
        let freq = 0x400 + r.below(0x3000);
        let length_bits = 0xF0 | (r.below(4) as u8) << 2;
        for b in [
            freq as u8,
            0,
            (freq >> 8) as u8,
            0,
            length_bits | (freq >> 16) as u8,
            0,
            r.below(16) as u8 * 4,
            (r.below(16) as u8) | if c == 7 { (r.below(8) as u8) << 4 } else { 0 },
        ] {
            init.extend(set(0x4800, b));
        }
    }
    let mut play = vec![0xE6, 0x00, 0xA5, 0x00];
    play.extend(set(0xF800, 0x78 | 0x80));
    play.extend([0x8D, 0x00, 0x48]);
    for _ in 0..r.below(3) {
        play.extend(random_store(&mut r));
    }
    let mut f = file_with(&init, &play, 16666, r.below(2) as u8, [0; 8], None);
    f[0x7B] = oscilla_nsf::chips::N163;
    f
}

/// A file that asks for the FDS, loads a random wavetable and drives the
/// chip's registers, then moves the pitch in the play routine.
fn fds_program(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed ^ 0xFD5);
    let mut init = set(0x4015, 0x1F);
    for _ in 0..4 + r.below(6) {
        init.extend(random_store(&mut r));
    }
    init.extend(set(0x4089, 0x80));
    for i in 0..64u16 {
        init.extend(set(0x4040 + i, r.byte() & 0x3F));
    }
    init.extend(set(0x4087, 0x80));
    for _ in 0..32 {
        init.extend(set(0x4088, r.byte() & 7));
    }
    init.extend(set(0x4089, r.byte() & 3));
    for _ in 0..8 + r.below(12) {
        let reg = [
            0x4080u16, 0x4082, 0x4083, 0x4084, 0x4085, 0x4086, 0x4087, 0x408A,
        ][r.below(8) as usize];
        let mut v = r.byte();
        if reg == 0x4083 {
            v &= 0x4F;
        }
        if reg == 0x408A && r.below(2) == 0 {
            v = 0xE8;
        }
        init.extend(set(reg, v));
    }
    init.extend(set(0x4083, 0x01 + r.below(4) as u8));
    let mut play = vec![0xE6, 0x00, 0xA5, 0x00, 0x8D, 0x82, 0x40];
    for _ in 0..r.below(3) {
        play.extend(random_store(&mut r));
    }
    let mut f = file_with(&init, &play, 16666, r.below(2) as u8, [0; 8], None);
    f[0x7B] = oscilla_nsf::chips::FDS;
    f
}

const FDS_SEEDS: u64 = 16;
const N163_SEEDS: u64 = 16;
const S5B_SEEDS: u64 = 16;
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
fn the_digests_with_the_fds_match() {
    let want: Vec<(u64, u64, u64)> = include_str!("goldens/nes-fds.txt")
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
    assert_eq!(want.len() as u64, FDS_SEEDS);
    let mut bad = Vec::new();
    for (seed, audio, writes) in want {
        let got = digests_of(fds_program(seed));
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
fn the_digests_with_the_n163_match() {
    let want: Vec<(u64, u64, u64)> = include_str!("goldens/nes-n163.txt")
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
    assert_eq!(want.len() as u64, N163_SEEDS);
    let mut bad = Vec::new();
    for (seed, audio, writes) in want {
        let got = digests_of(n163_program(seed));
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
fn the_digests_with_the_5b_match() {
    let want: Vec<(u64, u64, u64)> = include_str!("goldens/nes-s5b.txt")
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
    assert_eq!(want.len() as u64, S5B_SEEDS);
    let mut bad = Vec::new();
    for (seed, audio, writes) in want {
        let got = digests_of(s5b_program(seed));
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
    println!("-----BEGIN FDS GOLDENS-----");
    for seed in 0..FDS_SEEDS {
        let (audio, writes) = digests_of(fds_program(seed));
        println!("{seed} {audio:016x} {writes:016x}");
    }
    println!("-----END FDS GOLDENS-----");
    println!("-----BEGIN N163 GOLDENS-----");
    for seed in 0..N163_SEEDS {
        let (audio, writes) = digests_of(n163_program(seed));
        println!("{seed} {audio:016x} {writes:016x}");
    }
    println!("-----END N163 GOLDENS-----");
    println!("-----BEGIN 5B GOLDENS-----");
    for seed in 0..S5B_SEEDS {
        let (audio, writes) = digests_of(s5b_program(seed));
        println!("{seed} {audio:016x} {writes:016x}");
    }
    println!("-----END 5B GOLDENS-----");
    println!("-----BEGIN CUTOFF GOLDENS-----");
    for seed in 0..CUTOFF_SEEDS {
        for hz in CUTOFFS {
            println!("{seed} {hz} {:016x}", run_with(seed, Some(hz)).0);
        }
    }
    println!("-----END CUTOFF GOLDENS-----");
}
