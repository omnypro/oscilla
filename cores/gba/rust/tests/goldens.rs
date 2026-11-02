//! Digests of full runs of fixed programs, checked in. A digest covers the CPU
//! registers after every instruction, memory at intervals and the 16-bit
//! samples. The programs are built from fixed seeds, so no recording of a game
//! is needed. A change that alters a digest changes the sound, and
//! `CGBA_OUTPUT_REVISION` in cgba.h goes up with it.

mod common;

use common::{Rust, golden_cases, trace_digest};

const GOLDENS: &str = include_str!("goldens/gba.txt");

#[test]
fn the_rust_machine_matches_the_checked_in_digests() {
    let expected: std::collections::HashMap<&str, &str> = GOLDENS
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split_once(' ').expect("name and digest"))
        .collect();
    let cases = golden_cases();
    assert_eq!(
        expected.len(),
        cases.len(),
        "the digest file and the case list differ"
    );
    let mut bad = Vec::new();
    for case in &cases {
        let digest = format!(
            "{:016x}",
            trace_digest(
                &mut Rust::from_rom(&case.rom),
                case.steps,
                case.chunks,
                case.frames
            )
        );
        if expected.get(case.name.as_str()) != Some(&digest.as_str()) {
            bad.push(case.name.clone());
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} programs differ: {:?}",
        bad.len(),
        cases.len(),
        &bad[..bad.len().min(10)]
    );
}
