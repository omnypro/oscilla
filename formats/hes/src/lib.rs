#![forbid(unsafe_code)]
//! The HES container: a 16-byte header with the start song, the address of
//! the request routine and the eight memory mapping registers, then blocks of
//! the code and data of a PC Engine program that play a song when they run.

/// The physical address space of the ROM side of the memory map.
pub const ROM_SIZE: usize = 0x10_0000;

/// A file larger than this is refused.
pub const MAX_FILE: usize = 16 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HesError {
    /// Too short, or the wrong magic text.
    NotHes,
    /// The file is larger than MAX_FILE.
    TooLarge,
    /// A block loads past the end of the ROM address space.
    Load,
}

impl std::fmt::Display for HesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HesError::NotHes => write!(f, "Not an HES file."),
            HesError::TooLarge => write!(f, "The file is larger than 16 MiB."),
            HesError::Load => write!(f, "A block loads outside the 1 MiB of ROM."),
        }
    }
}

impl std::error::Error for HesError {}

/// A parsed file with its blocks laid into an image of the ROM space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hes {
    pub version: u8,
    pub start_song: u8,
    /// The logical address of the routine that starts a song. It takes the
    /// song number in the accumulator.
    pub request: u16,
    /// The initial values of the memory mapping registers 0 to 7.
    pub mpr: [u8; 8],
    /// The ROM image, `ROM_SIZE` bytes, with 0xFF where no block loads.
    rom: Vec<u8>,
}

impl Hes {
    pub fn parse(bytes: &[u8]) -> Result<Hes, HesError> {
        if bytes.len() > MAX_FILE {
            return Err(HesError::TooLarge);
        }
        if bytes.len() < 16 || &bytes[..4] != b"HESM" {
            return Err(HesError::NotHes);
        }
        let mut mpr = [0u8; 8];
        mpr.copy_from_slice(&bytes[8..16]);
        let mut rom = vec![0xFFu8; ROM_SIZE];
        let mut at = 16;
        while at + 16 <= bytes.len() && &bytes[at..at + 4] == b"DATA" {
            let word = |o: usize| {
                u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as usize
            };
            let size = word(at + 4);
            let load = word(at + 8);
            let start = at + 16;
            let end = start.saturating_add(size).min(bytes.len());
            let body = &bytes[start..end];
            let to = load.checked_add(body.len()).ok_or(HesError::Load)?;
            if to > ROM_SIZE {
                return Err(HesError::Load);
            }
            rom[load..to].copy_from_slice(body);
            at = start.saturating_add(size);
        }
        Ok(Hes {
            version: bytes[4],
            start_song: bytes[5],
            request: u16::from_le_bytes([bytes[6], bytes[7]]),
            mpr,
            rom,
        })
    }

    /// The ROM image: `ROM_SIZE` bytes.
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(blocks: &[(u32, &[u8])]) -> Vec<u8> {
        let mut f = b"HESM".to_vec();
        f.extend([0, 5, 0x00, 0xE1, 0xFF, 0xF8, 0, 0, 1, 0, 2, 0]);
        for (load, data) in blocks {
            f.extend(b"DATA");
            f.extend((data.len() as u32).to_le_bytes());
            f.extend(load.to_le_bytes());
            f.extend([0; 4]);
            f.extend(*data);
        }
        f
    }

    #[test]
    fn the_header_gives_the_song_the_request_and_the_mapping() {
        let h = Hes::parse(&file(&[(0x20, &[1, 2, 3])])).unwrap();
        assert_eq!((h.version, h.start_song, h.request), (0, 5, 0xE100));
        assert_eq!(h.mpr, [0xFF, 0xF8, 0, 0, 1, 0, 2, 0]);
    }

    #[test]
    fn a_block_loads_at_its_physical_address() {
        let h = Hes::parse(&file(&[(0x20, &[1, 2, 3]), (0x2000, &[9])])).unwrap();
        assert_eq!(&h.rom()[0x1F..0x24], &[0xFF, 1, 2, 3, 0xFF]);
        assert_eq!(h.rom()[0x2000], 9);
        assert_eq!(h.rom().len(), ROM_SIZE);
    }

    #[test]
    fn a_short_block_loads_what_the_file_holds() {
        let mut f = file(&[(0, &[1, 2, 3, 4])]);
        f.truncate(f.len() - 2);
        let h = Hes::parse(&f).unwrap();
        assert_eq!(&h.rom()[..4], &[1, 2, 0xFF, 0xFF]);
    }

    #[test]
    fn bad_files_are_refused() {
        assert_eq!(Hes::parse(b"HES"), Err(HesError::NotHes));
        assert_eq!(Hes::parse(&[0; 40]), Err(HesError::NotHes));
        assert_eq!(
            Hes::parse(&file(&[(0xF_FFFF, &[1, 2])])),
            Err(HesError::Load)
        );
        assert_eq!(Hes::parse(&vec![0; MAX_FILE + 1]), Err(HesError::TooLarge));
    }
}
