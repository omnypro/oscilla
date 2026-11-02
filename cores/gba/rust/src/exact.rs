//! Arithmetic that gives the same result on every platform. The BIOS calls
//! and the resampler need sine, cosine, arctangent and a power. A math library
//! rounds those differently from one platform to the next, and a BIOS result
//! lands in guest memory where the game reads it back. The functions here use
//! only addition, subtraction, multiplication, division and comparison on
//! f64. IEEE 754 defines those operations exactly, and Rust does not fuse them
//! into multiply-adds. The one fused operation is `mul_add` in `two_prod`,
//! where it is written out on purpose: a fused multiply-add rounds once on
//! every platform, in hardware or in software. The tests in
//! `tests/libm_equivalence.rs` check each function against the platform's
//! libm on every input the core can reach.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, LN_2, PI};

/// Arctangent of any finite x.
pub(crate) fn atan(x: f64) -> f64 {
    let a = x.abs();
    // Fold the argument into [0, tan(pi/8)].
    let (r, base) = if a > 2.414_213_562_373_095 {
        (-1.0 / a, FRAC_PI_2)
    } else if a > 0.414_213_562_373_095_1 {
        ((a - 1.0) / (a + 1.0), FRAC_PI_4)
    } else {
        (a, 0.0)
    };
    let r2 = r * r;
    // r - r^3/3 + r^5/5 - ..., as a Horner form in r^2. Twenty-six terms leave
    // a truncation error far below one unit in the last place.
    let mut sum = 0.0;
    let mut k = 26;
    while k >= 0 {
        let coeff = 1.0 / f64::from(2 * k + 1);
        sum = coeff - r2 * sum;
        k -= 1;
    }
    let value = base + r * sum;
    if x < 0.0 { -value } else { value }
}

/// Arctangent of y / x in the quadrant that (x, y) lies in.
pub(crate) fn atan2(y: f64, x: f64) -> f64 {
    if x == 0.0 {
        return if y > 0.0 {
            FRAC_PI_2
        } else if y < 0.0 {
            -FRAC_PI_2
        } else {
            0.0
        };
    }
    let t = atan(y / x);
    if x > 0.0 {
        t
    } else if y >= 0.0 {
        t + PI
    } else {
        t - PI
    }
}

/// A value kept as the sum of two doubles, about 106 bits of precision.
#[derive(Clone, Copy)]
struct Dd {
    hi: f64,
    lo: f64,
}

fn fast_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    Dd {
        hi: s,
        lo: b - (s - a),
    }
}

fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    Dd {
        hi: s,
        lo: (a - (s - bb)) + (b - bb),
    }
}

/// The product of two doubles and its rounding error. `mul_add` rounds once,
/// so the error term is exact on every platform.
fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    Dd {
        hi: p,
        lo: a.mul_add(b, -p),
    }
}

fn dd_add(x: Dd, y: Dd) -> Dd {
    let s = two_sum(x.hi, y.hi);
    fast_two_sum(s.hi, s.lo + x.lo + y.lo)
}

fn dd_mul(x: Dd, y: Dd) -> Dd {
    let p = two_prod(x.hi, y.hi);
    fast_two_sum(p.hi, p.lo + x.hi * y.lo + x.lo * y.hi)
}

fn dd_div_int(x: Dd, k: f64) -> Dd {
    let q1 = x.hi / k;
    let p = two_prod(q1, k);
    let r = (x.hi - p.hi) - p.lo + x.lo;
    fast_two_sum(q1, r / k)
}

/// 2 raised to x, for x between -64 and 64, correctly rounded in all but the
/// rarest cases.
pub(crate) fn exp2(x: f64) -> f64 {
    let n = x.floor();
    // x - n is not exact when x is small and negative, so keep both halves.
    let f = two_sum(x, -n);
    let ln2 = Dd {
        hi: LN_2,
        lo: 2.319_046_813_846_299_6e-17,
    };
    let g = dd_mul(f, ln2);
    // exp(g) = sum of g^k / k!.
    let mut term = Dd { hi: 1.0, lo: 0.0 };
    let mut sum = term;
    for k in 1..=30 {
        term = dd_div_int(dd_mul(term, g), f64::from(k));
        sum = dd_add(sum, term);
    }
    let scale = f64::from_bits(((n as i64 + 1023) as u64) << 52);
    (sum.hi + sum.lo) * scale
}

/// The keys for which Apple's `exp2` is not correctly rounded, with the value
/// it returns. Rounding the true value gives a different last bit there, and a
/// BIOS result must not change.
const POW2_KEY_LIBM: [(u16, u64); 84] = [
    (215, 0x40de7c14b6cbdade),
    (582, 0x40dc0fe06ff301f5),
    (890, 0x40da2d958d09dd68),
    (3287, 0x40ce7c14b6cbdade),
    (3654, 0x40cc0fe06ff301f5),
    (3962, 0x40ca2d958d09dd68),
    (6359, 0x40be7c14b6cbdade),
    (6726, 0x40bc0fe06ff301f5),
    (7034, 0x40ba2d958d09dd68),
    (9431, 0x40ae7c14b6cbdade),
    (9798, 0x40ac0fe06ff301f5),
    (10106, 0x40aa2d958d09dd68),
    (12503, 0x409e7c14b6cbdade),
    (12870, 0x409c0fe06ff301f5),
    (13178, 0x409a2d958d09dd68),
    (15575, 0x408e7c14b6cbdade),
    (15942, 0x408c0fe06ff301f5),
    (16250, 0x408a2d958d09dd68),
    (18647, 0x407e7c14b6cbdade),
    (19014, 0x407c0fe06ff301f5),
    (19322, 0x407a2d958d09dd68),
    (22086, 0x406c0fe06ff301f5),
    (22664, 0x4068a180dda81b2c),
    (23429, 0x4064b9dd92f6db73),
    (23498, 0x406467e64d2e49b6),
    (25158, 0x405c0fe06ff301f5),
    (25736, 0x4058a180dda81b2c),
    (26501, 0x4054b9dd92f6db73),
    (26570, 0x405467e64d2e49b6),
    (28230, 0x404c0fe06ff301f5),
    (28808, 0x4048a180dda81b2c),
    (29573, 0x4044b9dd92f6db73),
    (29642, 0x404467e64d2e49b6),
    (31302, 0x403c0fe06ff301f5),
    (31880, 0x4038a180dda81b2c),
    (32645, 0x4034b9dd92f6db73),
    (32714, 0x403467e64d2e49b6),
    (34298, 0x402c8c211e9731b8),
    (34374, 0x402c0fe06ff301f5),
    (34492, 0x402b53219b3067db),
    (34610, 0x402a9b5847668256),
    (37370, 0x401c8c211e9731b8),
    (37446, 0x401c0fe06ff301f5),
    (37564, 0x401b53219b3067db),
    (37682, 0x401a9b5847668256),
    (40114, 0x400ebd8170f16ef5),
    (40518, 0x400c0fe06ff301f5),
    (40729, 0x400ac1e00a56819d),
    (41653, 0x4005b8d0bdd35fea),
    (42145, 0x40037087ba475539),
    (43217, 0x3ffe86a73be8f788),
    (43264, 0x3ffe3437e7101344),
    (43435, 0x3ffd0f90b554f3d9),
    (43590, 0x3ffc0fe06ff301f5),
    (45632, 0x3ff1b3b08a10c5fb),
    (45722, 0x3ff15897abafd173),
    (46532, 0x3fece5b7cc53cc09),
    (46662, 0x3fec0fe06ff301f5),
    (46858, 0x3fead9190f4c225e),
    (47921, 0x3fe51f634fa24997),
    (48347, 0x3fe32fd3eb59e0e7),
    (48898, 0x3fe0f19a5c6abd4f),
    (49330, 0x3fdebd8170f16ef5),
    (49734, 0x3fdc0fe06ff301f5),
    (49945, 0x3fdac1e00a56819d),
    (50869, 0x3fd5b8d0bdd35fea),
    (51361, 0x3fd37087ba475539),
    (52730, 0x3fcc8c211e9731b8),
    (52806, 0x3fcc0fe06ff301f5),
    (52924, 0x3fcb53219b3067db),
    (53042, 0x3fca9b5847668256),
    (55802, 0x3fbc8c211e9731b8),
    (55878, 0x3fbc0fe06ff301f5),
    (55996, 0x3fbb53219b3067db),
    (56114, 0x3fba9b5847668256),
    (58950, 0x3fac0fe06ff301f5),
    (59528, 0x3fa8a180dda81b2c),
    (60293, 0x3fa4b9dd92f6db73),
    (60362, 0x3fa467e64d2e49b6),
    (62022, 0x3f9c0fe06ff301f5),
    (62600, 0x3f98a180dda81b2c),
    (63365, 0x3f94b9dd92f6db73),
    (63434, 0x3f9467e64d2e49b6),
    (65094, 0x3f8c0fe06ff301f5),
];

/// 2 raised to key / 12, where key = 180 - r1 - r2 / 256: the pitch scale of
/// MidiKey2Freq. The result is the one Apple's `exp2` gives, on every
/// platform.
pub(crate) fn pow2_key(r1: u32, r2: u32) -> f64 {
    let index = ((r1 & 0xFF) << 8 | (r2 & 0xFF)) as u16;
    if let Ok(at) = POW2_KEY_LIBM.binary_search_by_key(&index, |&(k, _)| k) {
        return f64::from_bits(POW2_KEY_LIBM[at].1);
    }
    let key = 180.0 - f64::from(r1 & 0xFF) - f64::from(r2 & 0xFF) / 256.0;
    exp2(key / 12.0)
}
