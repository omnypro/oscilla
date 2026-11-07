//! The registers of the chips as the program wrote them, in named blocks. The
//! reads here change nothing that the sound depends on.

use crate::state::*;
use oscilla_nsf::chips;

/// The blocks in the order a player lists them: the chip that has the block
/// (0 for the 2A03, which every file has), the name, the address of byte 0 in
/// the chip's own register space and the size in bytes.
const BLOCKS: [(u8, &str, u32, usize); 8] = [
    (0, "2A03 APU", 0x4000, 24),
    (chips::VT02, "VT02", 0x4020, 22),
    (chips::VRC6, "VRC6", 0x9000, 12),
    (chips::S5B, "5B", 0, 16),
    (chips::N163, "N163", 0, 128),
    (chips::FDS, "FDS", 0x4040, 88),
    (chips::MMC5, "MMC5", 0x5000, 22),
    (chips::VRC7, "VRC7", 0, 54),
];

impl Machine {
    /// The row of `BLOCKS` that block number `block` of this file is.
    fn block_row(&self, block: usize) -> Option<usize> {
        let chips = self.chips;
        (0..BLOCKS.len())
            .filter(|&i| BLOCKS[i].0 == 0 || chips & BLOCKS[i].0 != 0)
            .nth(block)
    }

    /// The number of register blocks: the 2A03's, then one for each
    /// expansion chip that the file's header names.
    pub fn register_block_count(&self) -> usize {
        BLOCKS
            .iter()
            .filter(|b| b.0 == 0 || self.chips & b.0 != 0)
            .count()
    }

    /// The name of a block, or None for a block past the last.
    pub fn register_block_name(&self, block: usize) -> Option<&'static str> {
        self.block_row(block).map(|i| BLOCKS[i].1)
    }

    /// The address of byte 0 of a block in its chip's register space, 0 for a
    /// block past the last.
    pub fn register_block_base(&self, block: usize) -> u32 {
        self.block_row(block).map_or(0, |i| BLOCKS[i].2)
    }

    /// The size of a block in bytes, 0 for a block past the last.
    pub fn register_block_size(&self, block: usize) -> usize {
        self.block_row(block).map_or(0, |i| BLOCKS[i].3)
    }

    /// Copies the first `out.len()` bytes of a block, or all of it when `out`
    /// is longer, and returns the number copied. A block past the last copies
    /// nothing.
    pub fn register_block_get(&self, block: usize, out: &mut [u8]) -> usize {
        let Some(row) = self.block_row(block) else {
            return 0;
        };
        let mut image = [0u8; 128];
        match BLOCKS[row].0 {
            0 => image[..24].copy_from_slice(&self.written.apu),
            chips::VT02 => image[..22].copy_from_slice(&self.written.vt),
            chips::VRC6 => image[..12].copy_from_slice(&self.written.vrc6),
            chips::S5B => image[..16].copy_from_slice(&self.apu.s5b.regs),
            chips::N163 => image.copy_from_slice(&self.apu.n163.ram),
            chips::FDS => {
                image[..64].copy_from_slice(&self.apu.fds.wave);
                image[64..88].copy_from_slice(&self.written.fds);
            }
            chips::MMC5 => image[..22].copy_from_slice(&self.written.mmc5),
            _ => image[..54].copy_from_slice(&self.written.vrc7),
        }
        let n = BLOCKS[row].3.min(out.len());
        out[..n].copy_from_slice(&image[..n]);
        n
    }
}
