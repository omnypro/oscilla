//! The platform-independent arithmetic against the platform's libm, on every
//! input the core can reach. The reference values came from Apple's libm, so
//! only that platform has to agree with them. Another libm may round a result
//! the other way. That difference is the reason the arithmetic exists.

#![cfg(target_vendor = "apple")]

#[path = "../src/exact.rs"]
#[allow(dead_code)]
mod exact;

use std::f64::consts::PI;

/// What the BIOS does with an ArcTan2 result.
fn atan2_result(a: f64) -> u32 {
    let mut a = a;
    if a < 0.0 {
        a += 2.0 * PI;
    }
    ((a / (2.0 * PI) * 65536.0).round() as i64 as u32) & 0xFFFF
}

#[test]
fn arctan_matches_for_every_input() {
    let mut worst = 0u64;
    for raw in 0..=0xFFFFu32 {
        let t = f64::from(raw as u16 as i16) / 16384.0;
        let (a, b) = (t.atan(), exact::atan(t));
        let (x, y) = (
            (a / (PI / 2.0) * 16384.0).round() as i64,
            (b / (PI / 2.0) * 16384.0).round() as i64,
        );
        assert_eq!(x, y, "input {raw:#06x}");
        worst = worst.max(a.to_bits().abs_diff(b.to_bits()));
    }
    eprintln!("atan: largest difference {worst} ulp");
}

/// The reference was built with optimization, and LLVM turns the C library's
/// `pow(2.0, x)` into `exp2(x)` there, so `exp2` is the function whose results
/// the overrides record. A build without optimization would call `pow`, which
/// rounds differently on some keys.
#[test]
fn power_matches_for_every_key() {
    let mut worst = 0u64;
    for r1 in 0..256u32 {
        for r2 in 0..256u32 {
            let key = 180.0 - f64::from(r1) - f64::from(r2) / 256.0;
            let (a, b) = ((key / 12.0).exp2(), exact::pow2_key(r1, r2));
            worst = worst.max(a.to_bits().abs_diff(b.to_bits()));
            assert_eq!(a.to_bits(), b.to_bits(), "key {r1} {r2}: {a:e} vs {b:e}");
        }
    }
    eprintln!("pow2_key: largest difference {worst} ulp");
}

/// Every pair of 16-bit signed arguments, folded by symmetry: the result
/// depends on the quadrant and the magnitudes only.
#[test]
#[ignore = "about a billion evaluations; run with --ignored --release"]
fn arctan2_matches_for_every_input() {
    let mut bad = 0u64;
    for y in -32768i32..=32767 {
        for x in -32768i32..=32767 {
            let (fy, fx) = (f64::from(y), f64::from(x));
            if atan2_result(fy.atan2(fx)) != atan2_result(exact::atan2(fy, fx)) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("atan2({y}, {x}) differs");
                }
            }
        }
    }
    assert_eq!(bad, 0);
}

#[test]
fn arctan2_matches_on_a_dense_sample() {
    let mut rng = 0x1234_5678_9ABC_DEF1u64;
    let mut bad = 0;
    for _ in 0..20_000_000u32 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let y = (rng >> 8) as u16 as i16;
        let x = (rng >> 32) as u16 as i16;
        let (fy, fx) = (f64::from(y), f64::from(x));
        if atan2_result(fy.atan2(fx)) != atan2_result(exact::atan2(fy, fx)) {
            bad += 1;
        }
    }
    assert_eq!(bad, 0);
}

#[test]
#[ignore = "diagnostic"]
fn count_power_mismatches() {
    let mut bad = Vec::new();
    for r1 in 0..256u32 {
        for r2 in 0..256u32 {
            let key = 180.0 - f64::from(r1) - f64::from(r2) / 256.0;
            let (a, b) = ((key / 12.0).exp2(), exact::exp2(key / 12.0));
            if a.to_bits() != b.to_bits() {
                bad.push((r1, r2));
            }
        }
    }
    eprintln!("{} of 65536 keys differ", bad.len());
    for (r1, r2) in &bad {
        let key = 180.0 - f64::from(*r1) - f64::from(*r2) / 256.0;
        let x = key / 12.0;
        println!(
            "MISMATCH {r1} {r2} {:016x} {:016x} {:016x}",
            x.to_bits(),
            x.exp2().to_bits(),
            exact::exp2(x).to_bits()
        );
    }
}
