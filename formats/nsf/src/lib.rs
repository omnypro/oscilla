//! The NSF container: a 0x80-byte header followed by the program and data of
//! an NES sound driver. The header names the load, init and play routines, the
//! banks to map at the start and the interval between calls of the play
//! routine. Cores run the program in their own way.

/// A file larger than this is refused. A bank-switched file can hold up to 256
/// banks of 4 KiB; the limit leaves room for the header.
pub const MAX_FILE: usize = 4 << 20;

pub const HEADER_SIZE: usize = 0x80;

/// Bytes that a text field of the header holds.
const TEXT: usize = 32;

/// The play interval when a header names none, in microseconds: the frame
/// period of each region.
pub const DEFAULT_NTSC_US: u16 = 16639;
pub const DEFAULT_PAL_US: u16 = 19997;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NsfError {
    /// Shorter than the header, or the first five bytes are not "NESM" and
    /// 0x1A.
    NotNsf,
    /// A version other than 1 or 2.
    Version(u8),
    /// The header names no songs.
    NoSongs,
    /// The load address lies below 0x8000.
    Load(u16),
    /// The file is larger than MAX_FILE.
    TooLarge,
}

impl std::fmt::Display for NsfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NsfError::NotNsf => write!(f, "Not an NSF file."),
            NsfError::Version(v) => write!(f, "NSF version {v} is not supported."),
            NsfError::NoSongs => write!(f, "The file holds no songs."),
            NsfError::Load(a) => write!(f, "The load address 0x{a:04X} is below the ROM area."),
            NsfError::TooLarge => write!(f, "The file is larger than 4 MiB."),
        }
    }
}

impl std::error::Error for NsfError {}

/// Which console a song is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Ntsc,
    Pal,
}

/// The expansion chips that a file asks for, one bit each.
pub mod chips {
    pub const VRC6: u8 = 1;
    pub const VRC7: u8 = 2;
    pub const FDS: u8 = 4;
    pub const MMC5: u8 = 8;
    pub const N163: u8 = 16;
    pub const S5B: u8 = 32;
    pub const VT02: u8 = 64;
}

/// The feature bits of a version 2 header.
pub mod nsf2 {
    /// Interrupt support: a vector overlay and a timer.
    pub const IRQ: u8 = 16;
    /// Init may run forever, and the play routine runs from an interrupt.
    pub const NON_RETURNING_INIT: u8 = 32;
    /// The play routine is never called.
    pub const NO_PLAY: u8 = 64;
}

/// A parsed file. The data is the program bytes after the header. They belong
/// at the load address, or in banks when the header names any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nsf {
    pub version: u8,
    pub songs: u8,
    /// The first song to play, counted from 0.
    pub first_song: u8,
    pub load: u16,
    pub init: u16,
    pub play: u16,
    /// Microseconds between calls of the play routine on an NTSC console and on
    /// a PAL console.
    pub ntsc_us: u16,
    pub pal_us: u16,
    /// The banks that the eight 4 KiB windows start with.
    pub banks: [u8; 8],
    /// The raw region byte: bit 0 set for PAL, bit 1 set for both.
    pub region_bits: u8,
    /// The expansion chips, as `chips` bits.
    pub chips: u8,
    /// The version 2 feature bits of the header (0 for version 1): see
    /// `nsf2`.
    pub flags: u8,
    title: [u8; TEXT],
    author: [u8; TEXT],
    copyright: [u8; TEXT],
    pub data: Vec<u8>,
}

fn word(p: &[u8]) -> u16 {
    u16::from_le_bytes([p[0], p[1]])
}

/// The bytes of a text field up to the first NUL.
fn text(field: &[u8; TEXT]) -> &[u8] {
    match field.iter().position(|&b| b == 0) {
        Some(n) => &field[..n],
        None => field,
    }
}

impl Nsf {
    pub fn parse(file: &[u8]) -> Result<Nsf, NsfError> {
        if file.len() > MAX_FILE {
            return Err(NsfError::TooLarge);
        }
        if file.len() < HEADER_SIZE || &file[..5] != b"NESM\x1A" {
            return Err(NsfError::NotNsf);
        }
        let version = file[5];
        if version != 1 && version != 2 {
            return Err(NsfError::Version(version));
        }
        let songs = file[6];
        if songs == 0 {
            return Err(NsfError::NoSongs);
        }
        let load = word(&file[8..]);
        if load < 0x8000 {
            return Err(NsfError::Load(load));
        }
        let field = |at: usize| {
            let mut t = [0u8; TEXT];
            t.copy_from_slice(&file[at..at + TEXT]);
            t
        };
        let mut banks = [0u8; 8];
        banks.copy_from_slice(&file[0x70..0x78]);
        let mut data = &file[HEADER_SIZE..];
        // A version 2 file may state the length of the program, with data
        // after it that this reader does not use.
        if version == 2 {
            let len = usize::from(file[0x7D])
                | usize::from(file[0x7E]) << 8
                | usize::from(file[0x7F]) << 16;
            if len != 0 && len < data.len() {
                data = &data[..len];
            }
        }
        Ok(Nsf {
            version,
            songs,
            first_song: file[7].saturating_sub(1).min(songs - 1),
            load,
            init: word(&file[0x0A..]),
            play: word(&file[0x0C..]),
            ntsc_us: word(&file[0x6E..]),
            pal_us: word(&file[0x78..]),
            banks,
            region_bits: file[0x7A],
            chips: file[0x7B],
            flags: if version == 2 { file[0x7C] } else { 0 },
            title: field(0x0E),
            author: field(0x2E),
            copyright: field(0x4E),
            data: data.to_vec(),
        })
    }

    pub fn title(&self) -> &[u8] {
        text(&self.title)
    }

    pub fn author(&self) -> &[u8] {
        text(&self.author)
    }

    pub fn copyright(&self) -> &[u8] {
        text(&self.copyright)
    }

    /// Whether the header names banks. Without them the data sits at the load
    /// address in one piece.
    pub fn is_banked(&self) -> bool {
        self.banks.iter().any(|&b| b != 0)
    }

    /// The console that the file is for: PAL only when the header says PAL and
    /// not both.
    pub fn region(&self) -> Region {
        if self.region_bits & 3 == 1 {
            Region::Pal
        } else {
            Region::Ntsc
        }
    }

    /// Microseconds between calls of the play routine on `region`. A header
    /// that names none gets the frame period of the region.
    pub fn play_interval_us(&self, region: Region) -> u32 {
        let (us, default) = match region {
            Region::Ntsc => (self.ntsc_us, DEFAULT_NTSC_US),
            Region::Pal => (self.pal_us, DEFAULT_PAL_US),
        };
        u32::from(if us == 0 { default } else { us })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(songs: u8, first: u8, load: u16, banks: [u8; 8], region: u8) -> Vec<u8> {
        let mut f = vec![0u8; HEADER_SIZE + 4];
        f[..5].copy_from_slice(b"NESM\x1A");
        f[5] = 1;
        f[6] = songs;
        f[7] = first;
        f[8..10].copy_from_slice(&load.to_le_bytes());
        f[0x0A..0x0C].copy_from_slice(&0x8003u16.to_le_bytes());
        f[0x0C..0x0E].copy_from_slice(&0x8000u16.to_le_bytes());
        f[0x0E..0x13].copy_from_slice(b"Title");
        f[0x2E..0x32].copy_from_slice(b"Name");
        f[0x4E] = b'?';
        f[0x6E..0x70].copy_from_slice(&16666u16.to_le_bytes());
        f[0x70..0x78].copy_from_slice(&banks);
        f[0x78..0x7A].copy_from_slice(&19997u16.to_le_bytes());
        f[0x7A] = region;
        f[HEADER_SIZE..].copy_from_slice(&[1, 2, 3, 4]);
        f
    }

    #[test]
    fn reads_the_header_fields() {
        let g = Nsf::parse(&file(5, 2, 0x8000, [0; 8], 0)).unwrap();
        assert_eq!((g.songs, g.first_song, g.load), (5, 1, 0x8000));
        assert_eq!((g.init, g.play), (0x8003, 0x8000));
        assert_eq!(g.title(), b"Title");
        assert_eq!(g.author(), b"Name");
        assert_eq!(g.copyright(), b"?");
        assert_eq!(g.data, [1, 2, 3, 4]);
        assert_eq!((g.ntsc_us, g.pal_us), (16666, 19997));
        assert!(!g.is_banked());
    }

    #[test]
    fn the_first_song_stays_inside_the_count() {
        assert_eq!(
            Nsf::parse(&file(3, 0, 0x8000, [0; 8], 0))
                .unwrap()
                .first_song,
            0
        );
        assert_eq!(
            Nsf::parse(&file(3, 9, 0x8000, [0; 8], 0))
                .unwrap()
                .first_song,
            2
        );
    }

    #[test]
    fn banks_and_region() {
        let g = Nsf::parse(&file(1, 1, 0x8000, [0, 1, 2, 3, 4, 5, 6, 7], 1)).unwrap();
        assert!(g.is_banked());
        assert_eq!(g.region(), Region::Pal);
        let dual = Nsf::parse(&file(1, 1, 0x8000, [0; 8], 3)).unwrap();
        assert_eq!(dual.region(), Region::Ntsc);
        assert_eq!(dual.play_interval_us(Region::Pal), 19997);
        assert_eq!(dual.play_interval_us(Region::Ntsc), 16666);
    }

    #[test]
    fn a_header_without_a_rate_gets_the_frame_period() {
        let mut f = file(1, 1, 0x8000, [0; 8], 0);
        f[0x6E] = 0;
        f[0x6F] = 0;
        let g = Nsf::parse(&f).unwrap();
        assert_eq!(g.play_interval_us(Region::Ntsc), 16639);
    }

    #[test]
    fn the_feature_bits_are_read_from_version_2_only() {
        let mut f = file(1, 1, 0x8000, [0; 8], 0);
        f[0x7C] = nsf2::NO_PLAY;
        assert_eq!(Nsf::parse(&f).unwrap().flags, 0);
        f[5] = 2;
        assert_eq!(Nsf::parse(&f).unwrap().flags, nsf2::NO_PLAY);
    }

    #[test]
    fn a_version_2_length_cuts_the_data() {
        let mut f = file(1, 1, 0x8000, [0; 8], 0);
        f[5] = 2;
        f[0x7D] = 2;
        assert_eq!(Nsf::parse(&f).unwrap().data, [1, 2]);
    }

    #[test]
    fn refuses_bad_files() {
        assert_eq!(Nsf::parse(b"NESM"), Err(NsfError::NotNsf));
        let mut f = file(1, 1, 0x8000, [0; 8], 0);
        f[5] = 3;
        assert_eq!(Nsf::parse(&f), Err(NsfError::Version(3)));
        assert_eq!(
            Nsf::parse(&file(0, 1, 0x8000, [0; 8], 0)),
            Err(NsfError::NoSongs)
        );
        assert_eq!(
            Nsf::parse(&file(1, 1, 0x7000, [0; 8], 0)),
            Err(NsfError::Load(0x7000))
        );
        let mut g = file(1, 1, 0x8000, [0; 8], 0);
        g[0] = b'X';
        assert_eq!(Nsf::parse(&g), Err(NsfError::NotNsf));
        assert_eq!(
            Nsf::parse(&vec![0u8; MAX_FILE + 1]),
            Err(NsfError::TooLarge)
        );
    }
}
