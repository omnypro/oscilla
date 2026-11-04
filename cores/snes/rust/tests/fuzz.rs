//! Files of random bytes must run without a panic and render the same samples
//! twice. The RAM, the DSP registers and the processor's registers are all
//! random, so the processor runs into every opcode and the DSP into every
//! setting.

mod common;

use common::*;
use oscilla_snes::{Machine, Spc};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

fn random_file(seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    let mut f = vec![0u8; 0x10200];
    f[..33].copy_from_slice(b"SNES-SPC700 Sound File Data v0.30");
    f[0x23] = 27;
    for b in f[0x25..].iter_mut() {
        *b = r.next() as u8;
    }
    // Keep the program counter in RAM most of the time.
    if r.next() % 4 != 0 {
        f[0x26] &= 0x7F;
    }
    f
}

#[test]
fn random_files_run_and_repeat() {
    for seed in 0..40u64 {
        let spc = Spc::parse(&random_file(seed)).unwrap();
        let mut a = Machine::new(&spc, 44100).unwrap();
        let mut b = Machine::new(&spc, 44100).unwrap();
        let x = render(&mut a, 6000);
        let y = render(&mut b, 6000);
        assert_eq!(digest(&x), digest(&y), "seed {seed}");
        // A snapshot taken part of the way continues the same.
        let snap = a.snapshot();
        let rest = render(&mut a, 3000);
        a.restore(&snap).unwrap();
        assert_eq!(digest(&rest), digest(&render(&mut a, 3000)), "seed {seed}");
    }
}

#[test]
fn random_masks_and_resets_do_not_panic() {
    let mut r = Rng(7);
    for seed in 100..120u64 {
        let spc = Spc::parse(&random_file(seed)).unwrap();
        let mut m = Machine::new(&spc, 48000).unwrap();
        for _ in 0..6 {
            m.set_channel_mask(u64::from(r.next()) << 7 | u64::from(r.next()));
            render(&mut m, 700);
            if r.next() % 3 == 0 {
                m.reset();
            }
            m.write_port((r.next() % 4) as usize, r.next() as u8);
        }
    }
}
