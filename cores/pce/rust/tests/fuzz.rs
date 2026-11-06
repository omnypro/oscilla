//! Programs of random bytes run without a panic and render the same samples
//! twice.

use oscilla_hes::Hes;
use oscilla_pce::Machine;

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

fn file(seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut rom = vec![0u8; 0x8000];
    for b in &mut rom {
        *b = rng.next() as u8;
    }
    // A request routine that enables every interrupt, starts the timer, turns
    // on the sound and then falls into the random bytes.
    let head: &[u8] = &[
        0xA9, 0x03, 0x8D, 0x00, 0x0C, 0xA9, 0x01, 0x8D, 0x01, 0x0C, 0xA9, 0x00, 0x8D, 0x02, 0x14,
        0xA9, 0x08, 0x8D, 0x02, 0x00, 0x58,
    ];
    rom[0x100..0x100 + head.len()].copy_from_slice(head);
    for v in [0x1FF6, 0x1FF8, 0x1FFA, 0x1FFC] {
        let at = 0x2000 + (rng.next() as usize % 0x1000);
        rom[v..v + 2].copy_from_slice(&[at as u8, (at >> 8) as u8 | 0xE0]);
    }
    let mut f = b"HESM".to_vec();
    f.extend([0, 0, 0x00, 0xE1, 0xFF, 0xF8, 0, 0, 1, 2, 3, 0]);
    f.extend(b"DATA");
    f.extend((rom.len() as u32).to_le_bytes());
    f.extend(0u32.to_le_bytes());
    f.extend([0; 4]);
    f.extend(rom);
    f
}

#[test]
fn random_programs_run_and_repeat() {
    for seed in 0..40u64 {
        let hes = Hes::parse(&file(seed)).unwrap();
        let play = |hes: &Hes| {
            let mut m = Machine::new(hes, 44_100, seed as u8).unwrap();
            let mut out = vec![0i16; 2 * 8000];
            m.render_s16(&mut out);
            out
        };
        assert_eq!(play(&hes), play(&hes), "seed {seed}");
    }
}
