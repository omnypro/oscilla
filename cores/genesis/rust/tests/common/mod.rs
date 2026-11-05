//! Builds VGM files in memory.

#![allow(dead_code)]

use oscilla_genesis::Machine;
use oscilla_vgm::Vgm;

pub const PSG_CLOCK: u32 = 3_579_545;

/// A version 1.71 file whose stream is `stream`. `loop_at` is an offset in the
/// stream, and `loop_samples` the header's length of the loop.
pub fn file(psg: bool, fm: bool, stream: &[u8], loop_at: Option<usize>) -> Vec<u8> {
    let mut f = vec![0u8; 0x100];
    f[..4].copy_from_slice(b"Vgm ");
    f[8..12].copy_from_slice(&0x171u32.to_le_bytes());
    f[0x34..0x38].copy_from_slice(&(0x100u32 - 0x34).to_le_bytes());
    if psg {
        f[0x0C..0x10].copy_from_slice(&PSG_CLOCK.to_le_bytes());
        f[0x28..0x2A].copy_from_slice(&9u16.to_le_bytes());
        f[0x2A] = 16;
    }
    if fm {
        f[0x2C..0x30].copy_from_slice(&7_670_454u32.to_le_bytes());
    }
    if let Some(at) = loop_at {
        let abs = 0x100 + at;
        f[0x1C..0x20].copy_from_slice(&((abs - 0x1C) as u32).to_le_bytes());
    }
    f.extend_from_slice(stream);
    let eof = (f.len() - 4) as u32;
    f[4..8].copy_from_slice(&eof.to_le_bytes());
    f
}

pub fn wait(n: u16) -> Vec<u8> {
    vec![0x61, n as u8, (n >> 8) as u8]
}

pub fn parse(bytes: &[u8]) -> Vgm {
    Vgm::parse(bytes).unwrap()
}

pub fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; 2 * frames];
    m.render_s16(&mut out);
    out
}

/// The left channel.
pub fn left(samples: &[i16]) -> Vec<i16> {
    samples.chunks(2).map(|f| f[0]).collect()
}

pub fn crossings(samples: &[i16]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0) != (w[1] < 0))
        .count()
}

pub fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn digest(samples: &[i16]) -> u64 {
    fnv(samples.iter().flat_map(|s| s.to_le_bytes()))
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    pub fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    pub fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}
