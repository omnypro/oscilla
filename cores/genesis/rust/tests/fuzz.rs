//! Files of random bytes must parse or be refused without a panic, and a
//! machine made from one must render the same samples twice. The header
//! announces both chips, so the random stream drives every register.

mod common;

use common::*;
use oscilla_genesis::Machine;
use oscilla_vgm::Vgm;

fn random_stream(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    let n = 200 + r.below(4000) as usize;
    (0..n).map(|_| r.byte()).collect()
}

#[test]
fn random_streams_run_and_repeat() {
    for seed in 0..60u64 {
        let bytes = file(true, true, &random_stream(seed), None);
        let Ok(v) = Vgm::parse(&bytes) else { continue };
        let mut a = Machine::new(&v, 44_100).unwrap();
        let mut b = Machine::new(&v, 44_100).unwrap();
        let x = render(&mut a, 20_000);
        let y = render(&mut b, 20_000);
        assert_eq!(digest(&x), digest(&y), "seed {seed}");
        let saved = a.clone();
        let rest = render(&mut a, 6000);
        assert!(a.restore(&saved));
        assert_eq!(digest(&rest), digest(&render(&mut a, 6000)), "seed {seed}");
    }
}

#[test]
fn random_streams_run_and_repeat_with_two_chips() {
    for seed in 200..240u64 {
        let mut bytes = file(true, true, &random_stream(seed), None);
        bytes[0x0F] |= 0x40;
        bytes[0x2F] |= 0x40;
        let Ok(v) = Vgm::parse(&bytes) else { continue };
        let mut a = Machine::new(&v, 44_100).unwrap();
        let mut b = Machine::new(&v, 44_100).unwrap();
        let x = render(&mut a, 20_000);
        let y = render(&mut b, 20_000);
        assert_eq!(digest(&x), digest(&y), "seed {seed}");
        let saved = a.clone();
        let rest = render(&mut a, 6000);
        assert!(a.restore(&saved));
        assert_eq!(digest(&rest), digest(&render(&mut a, 6000)), "seed {seed}");
    }
}

#[test]
fn random_headers_parse_or_fail_cleanly() {
    let mut r = Rng(99);
    for _ in 0..500 {
        let n = r.below(600) as usize;
        let mut bytes: Vec<u8> = (0..n).map(|_| r.byte()).collect();
        if r.below(2) == 0 && bytes.len() > 8 {
            bytes[..4].copy_from_slice(b"Vgm ");
        }
        if let Ok(v) = Vgm::parse(&bytes)
            && let Ok(mut m) = Machine::new(&v, 44_100)
        {
            render(&mut m, 2000);
        }
    }
}

#[test]
fn random_masks_resets_and_cutoffs_do_not_panic() {
    let mut r = Rng(7);
    for seed in 100..120u64 {
        let bytes = file(true, true, &random_stream(seed), None);
        let Ok(v) = Vgm::parse(&bytes) else { continue };
        let mut m = Machine::new(&v, 48_000).unwrap();
        for _ in 0..6 {
            m.set_channel_mask(u64::from(r.next()) << 10 | u64::from(r.next()));
            m.set_highpass_hz(f64::from(r.below(1200)));
            render(&mut m, 700);
            if r.below(3) == 0 {
                m.reset();
            }
        }
    }
}
