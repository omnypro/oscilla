//! The GBS container: a 0x70-byte header followed by the program and data of
//! a Game Boy sound driver. The header names the load, init and play routines,
//! the initial stack pointer and the timer that sets the rate of play calls.
//! Cores run the program in their own way.

/// A file larger than this is refused. The format has at most 255 banks
/// of 16 KiB.
pub const MAX_FILE: usize = 4 << 20;

pub const HEADER_SIZE: usize = 0x70;

/// Bytes that a text field of the header holds.
const TEXT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GbsError {
    /// Shorter than the header, or the first three bytes are not "GBS".
    NotGbs,
    /// A version other than 1.
    Version(u8),
    /// The header names no songs.
    NoSongs,
    /// The load address lies above 0x7FFF, outside the ROM area.
    Load(u16),
    /// The file is larger than MAX_FILE.
    TooLarge,
}

impl std::fmt::Display for GbsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GbsError::NotGbs => write!(f, "Not a GBS file."),
            GbsError::Version(v) => write!(f, "GBS version {v} is not supported."),
            GbsError::NoSongs => write!(f, "The file holds no songs."),
            GbsError::Load(a) => write!(f, "The load address 0x{a:04X} is above the ROM area."),
            GbsError::TooLarge => write!(f, "The file is larger than 4 MiB."),
        }
    }
}

impl std::error::Error for GbsError {}

/// A parsed file. The data is the bytes after the header. They belong at the
/// load address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gbs {
    pub songs: u8,
    /// The first song to play, counted from 0.
    pub first_song: u8,
    pub load: u16,
    pub init: u16,
    pub play: u16,
    pub sp: u16,
    pub tma: u8,
    pub tac: u8,
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

impl Gbs {
    pub fn parse(file: &[u8]) -> Result<Gbs, GbsError> {
        if file.len() > MAX_FILE {
            return Err(GbsError::TooLarge);
        }
        if file.len() < HEADER_SIZE || &file[..3] != b"GBS" {
            return Err(GbsError::NotGbs);
        }
        if file[3] != 1 {
            return Err(GbsError::Version(file[3]));
        }
        let songs = file[4];
        if songs == 0 {
            return Err(GbsError::NoSongs);
        }
        let load = word(&file[6..]);
        if load > 0x7FFF {
            return Err(GbsError::Load(load));
        }
        let field = |at: usize| {
            let mut t = [0u8; TEXT];
            t.copy_from_slice(&file[at..at + TEXT]);
            t
        };
        Ok(Gbs {
            songs,
            first_song: file[5].saturating_sub(1).min(songs - 1),
            load,
            init: word(&file[8..]),
            play: word(&file[10..]),
            sp: word(&file[12..]),
            tma: file[14],
            tac: file[15],
            title: field(0x10),
            author: field(0x30),
            copyright: field(0x50),
            data: file[HEADER_SIZE..].to_vec(),
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

    /// CPU clocks between calls to the play routine: the VBlank period when
    /// both timer registers are 0 or the timer is off, otherwise the period of
    /// the timer interrupt. TAC bit 7 selects the double-speed clock.
    pub fn tick_clocks(&self) -> u32 {
        if self.tac & 0x04 == 0 {
            return VBLANK_CLOCKS;
        }
        let per_count: u32 = match self.tac & 3 {
            0 => 1024,
            1 => 16,
            2 => 64,
            _ => 256,
        };
        let clocks = per_count * (256 - u32::from(self.tma));
        if self.tac & 0x80 != 0 {
            clocks / 2
        } else {
            clocks
        }
    }
}

/// 154 lines of 456 clocks.
pub const VBLANK_CLOCKS: u32 = 70_224;

#[cfg(test)]
mod tests {
    use super::*;

    fn file(songs: u8, first: u8, load: u16, tma: u8, tac: u8) -> Vec<u8> {
        let mut f = vec![0u8; HEADER_SIZE + 4];
        f[..4].copy_from_slice(b"GBS\x01");
        f[4] = songs;
        f[5] = first;
        f[6..8].copy_from_slice(&load.to_le_bytes());
        f[8..10].copy_from_slice(&0x0400u16.to_le_bytes());
        f[10..12].copy_from_slice(&0x0410u16.to_le_bytes());
        f[12..14].copy_from_slice(&0xFFFEu16.to_le_bytes());
        f[14] = tma;
        f[15] = tac;
        f[0x10..0x15].copy_from_slice(b"Title");
        f[0x30..0x34].copy_from_slice(b"Name");
        f[0x50] = b'?';
        f[HEADER_SIZE..].copy_from_slice(&[1, 2, 3, 4]);
        f
    }

    #[test]
    fn reads_the_header_fields() {
        let g = Gbs::parse(&file(5, 2, 0x3F80, 0, 0)).unwrap();
        assert_eq!((g.songs, g.first_song, g.load), (5, 1, 0x3F80));
        assert_eq!((g.init, g.play, g.sp), (0x0400, 0x0410, 0xFFFE));
        assert_eq!(g.title(), b"Title");
        assert_eq!(g.author(), b"Name");
        assert_eq!(g.copyright(), b"?");
        assert_eq!(g.data, [1, 2, 3, 4]);
    }

    #[test]
    fn the_first_song_stays_inside_the_count() {
        assert_eq!(Gbs::parse(&file(3, 0, 0x400, 0, 0)).unwrap().first_song, 0);
        assert_eq!(Gbs::parse(&file(3, 9, 0x400, 0, 0)).unwrap().first_song, 2);
    }

    #[test]
    fn refuses_bad_files() {
        assert_eq!(Gbs::parse(b"GBS"), Err(GbsError::NotGbs));
        let mut f = file(1, 1, 0x400, 0, 0);
        f[3] = 2;
        assert_eq!(Gbs::parse(&f), Err(GbsError::Version(2)));
        assert_eq!(Gbs::parse(&file(0, 1, 0x400, 0, 0)), Err(GbsError::NoSongs));
        assert_eq!(
            Gbs::parse(&file(1, 1, 0x8000, 0, 0)),
            Err(GbsError::Load(0x8000))
        );
        let mut g = file(1, 1, 0x400, 0, 0);
        g[0] = b'X';
        assert_eq!(Gbs::parse(&g), Err(GbsError::NotGbs));
        assert_eq!(
            Gbs::parse(&vec![0u8; MAX_FILE + 1]),
            Err(GbsError::TooLarge)
        );
    }

    #[test]
    fn the_tick_follows_the_timer_registers() {
        let tick = |tma, tac| {
            Gbs::parse(&file(1, 1, 0x400, tma, tac))
                .unwrap()
                .tick_clocks()
        };
        assert_eq!(tick(0, 0), VBLANK_CLOCKS);
        assert_eq!(tick(0xBB, 0x00), VBLANK_CLOCKS);
        assert_eq!(tick(0xBB, 0x04), 69 * 1024);
        assert_eq!(tick(0x00, 0x05), 256 * 16);
        assert_eq!(tick(0x00, 0x86), 256 * 64 / 2);
        assert_eq!(tick(0xFF, 0x07), 256);
    }
}
