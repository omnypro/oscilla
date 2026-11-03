//! The NSF container: a 0x80-byte header followed by the program and data of
//! an NES sound driver. The header names the load, init and play routines, the
//! banks to map at the start and the interval between calls of the play
//! routine. Cores run the program in their own way.
//!
//! The NSFe container holds the same information as a list of chunks, with
//! room for a title per song, a playlist and a length per song. `Nsf::parse`
//! reads both.

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
    /// An NSFe file without the chunk that the name says it needs, or with
    /// the chunks in an order that cannot work.
    Chunk(&'static str),
    /// An NSFe file with a mandatory chunk that this reader does not know.
    Mandatory([u8; 4]),
}

impl std::fmt::Display for NsfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NsfError::NotNsf => write!(f, "Not an NSF file."),
            NsfError::Version(v) => write!(f, "NSF version {v} is not supported."),
            NsfError::NoSongs => write!(f, "The file holds no songs."),
            NsfError::Load(a) => write!(f, "The load address 0x{a:04X} is below the ROM area."),
            NsfError::TooLarge => write!(f, "The file is larger than 4 MiB."),
            NsfError::Chunk(why) => write!(f, "The NSFe file is malformed: {why}."),
            NsfError::Mandatory(id) => write!(
                f,
                "The NSFe file needs the chunk {}, which is not supported.",
                String::from_utf8_lossy(id)
            ),
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
    /// The NSFe chunks after the program include a mandatory one.
    pub const METADATA: u8 = 128;
}

/// A parsed file. The data is the program bytes after the header. They belong
/// at the load address, or in banks when the header names any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nsf {
    /// 1 or 2 for an NSF file, 0 for an NSFe file.
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
    title: Vec<u8>,
    author: Vec<u8>,
    copyright: Vec<u8>,
    ripper: Vec<u8>,
    comment: Vec<u8>,
    /// Per song, from the NSFe chunks `tlbl` and `taut`. A song without an
    /// entry has an empty one.
    labels: Vec<Vec<u8>>,
    song_authors: Vec<Vec<u8>>,
    /// The play order of the NSFe chunk `plst`, with every entry below
    /// `songs`.
    playlist: Vec<u8>,
    /// Milliseconds per song, from the NSFe chunks `time` and `fade`. Negative
    /// when the file gives no value.
    times: Vec<i32>,
    fades: Vec<i32>,
    prefer_pal: bool,
    nsfe: bool,
    pub data: Vec<u8>,
}

fn word(p: &[u8]) -> u16 {
    u16::from_le_bytes([p[0], p[1]])
}

/// The bytes of a text field up to the first NUL.
fn text(field: &[u8]) -> &[u8] {
    match field.iter().position(|&b| b == 0) {
        Some(n) => &field[..n],
        None => field,
    }
}

/// The strings of an NSFe chunk: each ends at a NUL, and the last may end at
/// the end of the chunk. Bytes that are not UTF-8 become U+FFFD.
fn strings(chunk: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = chunk;
    while !rest.is_empty() {
        let n = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        out.push(utf8(&rest[..n]));
        rest = &rest[(n + 1).min(rest.len())..];
    }
    out
}

fn utf8(bytes: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(bytes).into_owned().into_bytes()
}

/// Little-endian 32-bit signed values, one per song; a song past the end of
/// the chunk gets -1.
fn millis(chunk: &[u8], songs: usize) -> Vec<i32> {
    (0..songs)
        .map(|i| match chunk.get(4 * i..4 * i + 4) {
            Some(b) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            None => -1,
        })
        .collect()
}

/// The chunk IDs that this reader understands. A mandatory chunk (first letter
/// uppercase) outside this list refuses the file.
const KNOWN_MANDATORY: [&[u8; 4]; 7] = [
    b"INFO", b"DATA", b"NEND", b"BANK", b"RATE", b"NSF2", b"VRC7",
];

/// The chunks of an NSFe file, or of the metadata that follows the program of
/// a version 2 file. A chunk that is missing is empty.
#[derive(Default)]
struct Chunks<'a> {
    info: Option<&'a [u8]>,
    data: Option<&'a [u8]>,
    data_first: bool,
    bank: &'a [u8],
    rate: Option<&'a [u8]>,
    nsf2: Option<u8>,
    regn: &'a [u8],
    auth: &'a [u8],
    taut: &'a [u8],
    tlbl: &'a [u8],
    plst: &'a [u8],
    time: &'a [u8],
    fade: &'a [u8],
    text: &'a [u8],
}

/// Reads the chunks in `file` from `at`. A chunk whose ID starts with an
/// uppercase letter is mandatory: when `strict` is set, one that this reader
/// does not know refuses the file, and otherwise it is skipped. After the
/// program of a version 2 file the chunks INFO, DATA, BANK and NSF2 are not
/// allowed and are skipped (`tail`).
fn scan(file: &[u8], mut at: usize, strict: bool, tail: bool) -> Result<Chunks<'_>, NsfError> {
    let mut c = Chunks::default();
    while at + 8 <= file.len() {
        let len = u32::from_le_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]]);
        let id: [u8; 4] = [file[at + 4], file[at + 5], file[at + 6], file[at + 7]];
        let start = at + 8;
        if strict && id[0].is_ascii_uppercase() && !KNOWN_MANDATORY.contains(&&id) {
            return Err(NsfError::Mandatory(id));
        }
        let room = file.len() - start;
        let cut = u64::from(len) > room as u64;
        let body = &file[start..start + if cut { room } else { len as usize }];
        if cut && &id != b"DATA" {
            break;
        }
        let allowed = !tail;
        match &id {
            b"INFO" if allowed => c.info = Some(body),
            b"DATA" if allowed => {
                c.data_first |= c.info.is_none();
                c.data = Some(body);
            }
            b"BANK" if allowed => c.bank = body,
            b"NSF2" if allowed => c.nsf2 = Some(body.first().copied().unwrap_or(0)),
            b"RATE" => c.rate = Some(body),
            b"regn" => c.regn = body,
            b"auth" => c.auth = body,
            b"taut" => c.taut = body,
            b"tlbl" => c.tlbl = body,
            b"plst" => c.plst = body,
            b"time" => c.time = body,
            b"fade" => c.fade = body,
            b"text" => c.text = body,
            b"NEND" => break,
            _ => {}
        }
        if cut {
            break;
        }
        at = start + body.len();
    }
    Ok(c)
}

/// The region bits and the preference for PAL after a regn chunk. Dendy has no
/// region of its own here: a chunk that names only Dendy changes nothing.
fn region_from(bits: u8, prefer_pal: bool, regn: &[u8]) -> (u8, bool) {
    match regn.first().map(|&b| (b & 1 != 0, b & 2 != 0)) {
        Some((true, false)) => (0, false),
        Some((false, true)) => (1, false),
        Some((true, true)) => (2, regn.get(1) == Some(&1)),
        _ => (bits, prefer_pal),
    }
}

impl Nsf {
    /// Reads an NSF or an NSFe file, by the first bytes.
    pub fn parse(file: &[u8]) -> Result<Nsf, NsfError> {
        if file.len() > MAX_FILE {
            return Err(NsfError::TooLarge);
        }
        if file.starts_with(b"NSFE") {
            Nsf::parse_nsfe(file)
        } else {
            Nsf::parse_nsf(file)
        }
    }

    fn parse_nsf(file: &[u8]) -> Result<Nsf, NsfError> {
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
        let field = |at: usize| text(&file[at..at + TEXT]).to_vec();
        let mut banks = [0u8; 8];
        banks.copy_from_slice(&file[0x70..0x78]);
        let mut data = &file[HEADER_SIZE..];
        let mut tail: &[u8] = &[];
        let flags = if version == 2 { file[0x7C] } else { 0 };
        // A version 2 file may state the length of the program, with NSFe
        // chunks after it.
        if version == 2 {
            let len = usize::from(file[0x7D])
                | usize::from(file[0x7E]) << 8
                | usize::from(file[0x7F]) << 16;
            if len != 0 && len < data.len() {
                tail = &data[len..];
                data = &data[..len];
            }
        }
        let mut nsf = Nsf {
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
            flags,
            title: field(0x0E),
            author: field(0x2E),
            copyright: field(0x4E),
            ripper: Vec::new(),
            comment: Vec::new(),
            labels: vec![Vec::new(); usize::from(songs)],
            song_authors: vec![Vec::new(); usize::from(songs)],
            playlist: Vec::new(),
            times: vec![-1; usize::from(songs)],
            fades: vec![-1; usize::from(songs)],
            prefer_pal: false,
            nsfe: false,
            data: data.to_vec(),
        };
        if !tail.is_empty() {
            nsf.read_tail(tail, flags & nsf2::METADATA != 0)?;
        }
        Ok(nsf)
    }

    /// Reads the NSFe chunks that follow the program of a version 2 file. A
    /// file whose flags say that a mandatory chunk is there is refused when
    /// the reader does not know one; any other file keeps what it can read.
    fn read_tail(&mut self, tail: &[u8], mandatory: bool) -> Result<(), NsfError> {
        let c = scan(tail, 0, mandatory, true)?;
        let count = usize::from(self.songs);
        if let Some(rate) = c.rate {
            if let Some(us) = rate.get(0..2).map(word) {
                self.ntsc_us = us;
            }
            if let Some(us) = rate.get(2..4).map(word) {
                self.pal_us = us;
            }
        }
        (self.region_bits, self.prefer_pal) =
            region_from(self.region_bits, self.prefer_pal, c.regn);
        let mut auth = strings(c.auth).into_iter();
        for field in [
            &mut self.title,
            &mut self.author,
            &mut self.copyright,
            &mut self.ripper,
        ] {
            if let Some(v) = auth.next().filter(|v| !v.is_empty()) {
                *field = v;
            }
        }
        if !c.text.is_empty() {
            self.comment = text(&utf8(c.text)).to_vec();
        }
        let names = |chunk: &[u8]| {
            let mut v = strings(chunk);
            v.resize(count, Vec::new());
            v
        };
        self.labels = names(c.tlbl);
        self.song_authors = names(c.taut);
        self.playlist = c.plst.iter().copied().filter(|&s| s < self.songs).collect();
        self.times = millis(c.time, count);
        self.fades = millis(c.fade, count);
        Ok(())
    }

    fn parse_nsfe(file: &[u8]) -> Result<Nsf, NsfError> {
        let c = scan(file, 4, true, false)?;
        let (info, data, bank, rate, regn) =
            (c.info, c.data, c.bank, c.rate.unwrap_or(&[]), c.regn);
        let flags = c.nsf2.unwrap_or(0);
        let (auth, taut, tlbl, plst, time, fade, comment) =
            (c.auth, c.taut, c.tlbl, c.plst, c.time, c.fade, c.text);
        let data_first = c.data_first;
        let Some(info) = info else {
            return Err(NsfError::Chunk("there is no INFO chunk"));
        };
        if data_first {
            return Err(NsfError::Chunk("DATA comes before INFO"));
        }
        let Some(data) = data else {
            return Err(NsfError::Chunk("there is no DATA chunk"));
        };
        if info.len() < 9 {
            return Err(NsfError::Chunk("INFO is shorter than 9 bytes"));
        }
        let songs = info[8];
        if songs == 0 {
            return Err(NsfError::NoSongs);
        }
        let load = word(&info[0..]);
        if load < 0x8000 {
            return Err(NsfError::Load(load));
        }
        let count = usize::from(songs);
        let mut banks = [0u8; 8];
        let n = bank.len().min(8);
        banks[..n].copy_from_slice(&bank[..n]);
        let (ntsc_us, pal_us) = (
            rate.get(0..2).map_or(0, word),
            rate.get(2..4).map_or(0, word),
        );
        let (region_bits, prefer_pal) = region_from(info[6], false, regn);
        let names = |chunk: &[u8]| {
            let mut v = strings(chunk);
            v.resize(count, Vec::new());
            v.truncate(count);
            v
        };
        let mut auth = strings(auth).into_iter();
        let mut next = || auth.next().unwrap_or_default();
        let (title, author, copyright, ripper) = (next(), next(), next(), next());
        Ok(Nsf {
            version: 0,
            songs,
            first_song: info.get(9).copied().unwrap_or(0).min(songs - 1),
            load,
            init: word(&info[2..]),
            play: word(&info[4..]),
            ntsc_us,
            pal_us,
            banks,
            region_bits,
            chips: info[7],
            flags,
            title,
            author,
            copyright,
            ripper,
            comment: text(&utf8(comment)).to_vec(),
            labels: names(tlbl),
            song_authors: names(taut),
            playlist: plst.iter().copied().filter(|&s| s < songs).collect(),
            times: millis(time, count),
            fades: millis(fade, count),
            prefer_pal,
            nsfe: true,
            data: data.to_vec(),
        })
    }

    /// The text fields. A field that the file leaves out is empty. An NSFe
    /// file gives UTF-8; the fields of an NSF header are whatever bytes the
    /// ripper wrote.
    pub fn title(&self) -> &[u8] {
        &self.title
    }

    pub fn author(&self) -> &[u8] {
        &self.author
    }

    pub fn copyright(&self) -> &[u8] {
        &self.copyright
    }

    /// Who ripped the file. Only an NSFe file names one.
    pub fn ripper(&self) -> &[u8] {
        &self.ripper
    }

    /// A free text. Only an NSFe file has one.
    pub fn comment(&self) -> &[u8] {
        &self.comment
    }

    /// Whether the file is an NSFe file.
    pub fn is_nsfe(&self) -> bool {
        self.nsfe
    }

    /// The name of `song` (counted from 0), empty when the file gives none or
    /// the song is out of range.
    pub fn song_title(&self, song: usize) -> &[u8] {
        self.labels.get(song).map_or(&[], Vec::as_slice)
    }

    /// The author of `song`, empty when the file gives none.
    pub fn song_author(&self, song: usize) -> &[u8] {
        self.song_authors.get(song).map_or(&[], Vec::as_slice)
    }

    /// The order in which a player plays the songs, counted from 0. Empty when
    /// the file has none.
    pub fn playlist(&self) -> &[u8] {
        &self.playlist
    }

    /// How long `song` plays before it fades, in milliseconds: negative when
    /// the file gives no value or the song is out of range.
    pub fn time_ms(&self, song: usize) -> i32 {
        self.times.get(song).map_or(-1, |&t| t.max(-1))
    }

    /// The length of the fade at the end of `song`, in milliseconds: negative
    /// when the file gives no value.
    pub fn fade_ms(&self, song: usize) -> i32 {
        self.fades.get(song).map_or(-1, |&t| t.max(-1))
    }

    /// The time and the fade of `song` as one number, or -1 when the file
    /// gives no time. A song without a fade length counts the time alone.
    pub fn length_ms(&self, song: usize) -> i64 {
        let time = self.time_ms(song);
        if time < 0 {
            return -1;
        }
        i64::from(time) + i64::from(self.fade_ms(song).max(0))
    }

    /// Whether the header names banks. Without them the data sits at the load
    /// address in one piece.
    pub fn is_banked(&self) -> bool {
        self.banks.iter().any(|&b| b != 0)
    }

    /// The console that the file is for: PAL only when the header says PAL and
    /// not both. An NSFe file for both consoles can name PAL as its choice.
    pub fn region(&self) -> Region {
        if self.region_bits & 3 == 1 || (self.region_bits & 3 == 2 && self.prefer_pal) {
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

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = (body.len() as u32).to_le_bytes().to_vec();
        c.extend_from_slice(id);
        c.extend_from_slice(body);
        c
    }

    fn info(songs: u8, first: u8, chips: u8, region: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0x8000u16.to_le_bytes());
        b.extend_from_slice(&0x8003u16.to_le_bytes());
        b.extend_from_slice(&0x8000u16.to_le_bytes());
        b.extend_from_slice(&[region, chips, songs, first]);
        chunk(b"INFO", &b)
    }

    fn nsfe(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut f = b"NSFE".to_vec();
        for c in chunks {
            f.extend_from_slice(c);
        }
        f
    }

    fn basic(extra: &[Vec<u8>]) -> Vec<u8> {
        let mut chunks = vec![info(3, 1, 4, 0), chunk(b"DATA", &[1, 2, 3, 4])];
        chunks.extend_from_slice(extra);
        chunks.push(chunk(b"NEND", &[]));
        nsfe(&chunks)
    }

    #[test]
    fn an_nsfe_file_gives_the_header_fields() {
        let g = Nsf::parse(&basic(&[])).unwrap();
        assert!(g.is_nsfe());
        assert_eq!((g.version, g.songs, g.first_song), (0, 3, 1));
        assert_eq!((g.load, g.init, g.play), (0x8000, 0x8003, 0x8000));
        assert_eq!((g.chips, g.region_bits, g.flags), (4, 0, 0));
        assert_eq!(g.data, [1, 2, 3, 4]);
        assert_eq!((g.ntsc_us, g.pal_us), (0, 0));
        assert_eq!(g.play_interval_us(Region::Ntsc), 16639);
        assert!(!g.is_banked());
        assert!(g.title().is_empty() && g.ripper().is_empty() && g.comment().is_empty());
        assert_eq!(g.time_ms(0), -1);
        assert_eq!(g.length_ms(0), -1);
        assert!(g.playlist().is_empty());
    }

    #[test]
    fn an_info_chunk_of_9_bytes_starts_at_song_0() {
        let mut i = info(3, 0, 0, 0);
        i.pop();
        i[0] = 9;
        let g = Nsf::parse(&nsfe(&[i, chunk(b"DATA", &[0])])).unwrap();
        assert_eq!(g.first_song, 0);
    }

    #[test]
    fn the_first_song_stays_inside_the_count_in_an_nsfe_file() {
        let g = Nsf::parse(&nsfe(&[info(2, 200, 0, 0), chunk(b"DATA", &[0])])).unwrap();
        assert_eq!(g.first_song, 1);
    }

    #[test]
    fn the_text_chunks() {
        let g = Nsf::parse(&basic(&[
            chunk(b"auth", b"Game\0Artist\0\0Ripper\0"),
            chunk(b"text", b"A note\r\nof two lines\0"),
            chunk(b"tlbl", b"One\0Two\0"),
            chunk(b"taut", b"\0Second\0Third"),
        ]))
        .unwrap();
        assert_eq!(g.title(), b"Game");
        assert_eq!(g.author(), b"Artist");
        assert_eq!(g.copyright(), b"");
        assert_eq!(g.ripper(), b"Ripper");
        assert_eq!(g.comment(), b"A note\r\nof two lines");
        assert_eq!(g.song_title(0), b"One");
        assert_eq!(g.song_title(1), b"Two");
        assert_eq!(g.song_title(2), b"");
        assert_eq!(g.song_title(9), b"");
        assert_eq!(g.song_author(0), b"");
        assert_eq!(g.song_author(1), b"Second");
        assert_eq!(g.song_author(2), b"Third");
    }

    #[test]
    fn text_that_is_not_utf8_is_repaired() {
        let g = Nsf::parse(&basic(&[chunk(b"auth", b"Caf\xE9\0")])).unwrap();
        assert_eq!(g.title(), "Caf\u{FFFD}".as_bytes());
    }

    #[test]
    fn a_title_missing_from_a_short_auth_chunk_is_empty() {
        let g = Nsf::parse(&basic(&[chunk(b"auth", b"")])).unwrap();
        assert!(g.title().is_empty() && g.author().is_empty());
    }

    #[test]
    fn times_and_fades() {
        let mut time = Vec::new();
        for t in [90_000i32, 0, -5] {
            time.extend_from_slice(&t.to_le_bytes());
        }
        let mut fade = Vec::new();
        for t in [4_000i32, -1] {
            fade.extend_from_slice(&t.to_le_bytes());
        }
        let g = Nsf::parse(&basic(&[chunk(b"time", &time), chunk(b"fade", &fade)])).unwrap();
        assert_eq!(
            (g.time_ms(0), g.fade_ms(0), g.length_ms(0)),
            (90_000, 4_000, 94_000)
        );
        assert_eq!((g.time_ms(1), g.fade_ms(1), g.length_ms(1)), (0, -1, 0));
        assert_eq!((g.time_ms(2), g.fade_ms(2), g.length_ms(2)), (-1, -1, -1));
        assert_eq!(g.time_ms(7), -1);
    }

    #[test]
    fn a_time_chunk_that_is_too_short_leaves_the_rest_without_a_value() {
        let g = Nsf::parse(&basic(&[chunk(b"time", &1000i32.to_le_bytes())])).unwrap();
        assert_eq!((g.time_ms(0), g.time_ms(1)), (1000, -1));
        let mut ragged = 1000i32.to_le_bytes().to_vec();
        ragged.extend_from_slice(&[7, 0]);
        let g = Nsf::parse(&basic(&[chunk(b"time", &ragged)])).unwrap();
        assert_eq!((g.time_ms(0), g.time_ms(1)), (1000, -1));
    }

    #[test]
    fn the_playlist_drops_songs_that_do_not_exist() {
        let g = Nsf::parse(&basic(&[chunk(b"plst", &[2, 0, 0, 3, 255, 1])])).unwrap();
        assert_eq!(g.playlist(), [2, 0, 0, 1]);
    }

    #[test]
    fn banks_and_rate() {
        let g = Nsf::parse(&basic(&[
            chunk(b"BANK", &[1, 2, 3]),
            chunk(b"RATE", &[0x10, 0x41, 0x1D, 0x4E, 0x01, 0x02]),
        ]))
        .unwrap();
        assert_eq!(g.banks, [1, 2, 3, 0, 0, 0, 0, 0]);
        assert!(g.is_banked());
        assert_eq!((g.ntsc_us, g.pal_us), (0x4110, 0x4E1D));
        let long =
            Nsf::parse(&basic(&[chunk(b"BANK", &[9; 12]), chunk(b"RATE", &[5, 0])])).unwrap();
        assert_eq!(long.banks, [9; 8]);
        assert_eq!((long.ntsc_us, long.pal_us), (5, 0));
    }

    #[test]
    fn the_nsf2_chunk_sets_the_feature_bits() {
        let g = Nsf::parse(&basic(&[chunk(b"NSF2", &[nsf2::NO_PLAY])])).unwrap();
        assert_eq!(g.flags, nsf2::NO_PLAY);
    }

    #[test]
    fn the_region_chunk_overrides_the_info_bits() {
        let f = |regn: &[u8]| Nsf::parse(&basic(&[chunk(b"regn", regn)])).unwrap();
        assert_eq!(f(&[2]).region(), Region::Pal);
        assert_eq!(f(&[1]).region(), Region::Ntsc);
        assert_eq!(f(&[3, 1]).region(), Region::Pal);
        assert_eq!(f(&[3, 0]).region(), Region::Ntsc);
        assert_eq!(f(&[3]).region(), Region::Ntsc);
        // A file that names only Dendy keeps the INFO bits.
        let pal = nsfe(&[info(1, 0, 0, 1), chunk(b"DATA", &[0]), chunk(b"regn", &[4])]);
        assert_eq!(Nsf::parse(&pal).unwrap().region(), Region::Pal);
    }

    #[test]
    fn unknown_chunks_are_skipped_or_refuse_the_file() {
        let g = Nsf::parse(&basic(&[
            chunk(b"xyzw", b"anything"),
            chunk(b"mixe", &[0; 6]),
        ]))
        .unwrap();
        assert_eq!(g.songs, 3);
        assert_eq!(
            Nsf::parse(&basic(&[chunk(b"XYZW", b"anything")])),
            Err(NsfError::Mandatory(*b"XYZW"))
        );
        // The VRC7 chunk is known.
        assert!(Nsf::parse(&basic(&[chunk(b"VRC7", &[0])])).is_ok());
    }

    #[test]
    fn nothing_after_nend_counts() {
        let mut f = basic(&[]);
        f.extend_from_slice(&chunk(b"XYZW", b"x"));
        f.extend_from_slice(&chunk(b"auth", b"Late\0"));
        let g = Nsf::parse(&f).unwrap();
        assert!(g.title().is_empty());
    }

    #[test]
    fn a_file_without_nend_is_read_to_its_end() {
        let f = nsfe(&[
            info(1, 0, 0, 0),
            chunk(b"DATA", &[5, 6]),
            chunk(b"auth", b"T\0"),
        ]);
        let g = Nsf::parse(&f).unwrap();
        assert_eq!((g.data.as_slice(), g.title()), (&[5u8, 6][..], &b"T"[..]));
    }

    #[test]
    fn a_data_chunk_that_runs_past_the_end_is_used_as_far_as_it_goes() {
        let mut f = nsfe(&[info(1, 0, 0, 0), chunk(b"DATA", &[5, 6, 7, 8])]);
        f.truncate(f.len() - 2);
        assert_eq!(Nsf::parse(&f).unwrap().data, [5, 6]);
    }

    #[test]
    fn another_chunk_that_runs_past_the_end_is_left_out() {
        let mut f = basic(&[chunk(b"auth", b"Game\0")]);
        // Cut inside the auth chunk and drop the NEND chunk.
        f.truncate(f.len() - 8 - 3);
        let g = Nsf::parse(&f).unwrap();
        assert!(g.title().is_empty());
        assert_eq!(g.data, [1, 2, 3, 4]);
    }

    #[test]
    fn a_chunk_length_beyond_the_file_does_not_overflow() {
        let mut f = nsfe(&[info(1, 0, 0, 0), chunk(b"DATA", &[5])]);
        f.extend_from_slice(&u32::MAX.to_le_bytes());
        f.extend_from_slice(b"time");
        f.extend_from_slice(&[1, 2, 3]);
        assert!(Nsf::parse(&f).is_ok());
    }

    #[test]
    fn a_malformed_nsfe_file_is_refused() {
        assert_eq!(
            Nsf::parse(&nsfe(&[chunk(b"DATA", &[1])])),
            Err(NsfError::Chunk("there is no INFO chunk"))
        );
        assert_eq!(
            Nsf::parse(&nsfe(&[info(1, 0, 0, 0)])),
            Err(NsfError::Chunk("there is no DATA chunk"))
        );
        assert_eq!(
            Nsf::parse(&nsfe(&[chunk(b"DATA", &[1]), info(1, 0, 0, 0)])),
            Err(NsfError::Chunk("DATA comes before INFO"))
        );
        assert_eq!(
            Nsf::parse(&nsfe(&[chunk(b"INFO", &[0; 8]), chunk(b"DATA", &[1])])),
            Err(NsfError::Chunk("INFO is shorter than 9 bytes"))
        );
        assert_eq!(
            Nsf::parse(&nsfe(&[info(0, 0, 0, 0), chunk(b"DATA", &[1])])),
            Err(NsfError::NoSongs)
        );
        let mut low = info(1, 0, 0, 0);
        low[8] = 0x00;
        low[9] = 0x70;
        assert_eq!(
            Nsf::parse(&nsfe(&[low, chunk(b"DATA", &[1])])),
            Err(NsfError::Load(0x7000))
        );
        assert_eq!(
            Nsf::parse(b"NSFE"),
            Err(NsfError::Chunk("there is no INFO chunk"))
        );
    }

    #[test]
    fn the_per_song_chunks_may_come_before_info() {
        let f = nsfe(&[
            chunk(b"tlbl", b"A\0B\0C\0"),
            info(3, 0, 0, 0),
            chunk(b"DATA", &[1]),
        ]);
        assert_eq!(Nsf::parse(&f).unwrap().song_title(2), b"C");
    }

    #[test]
    fn an_nsf_file_has_no_song_data() {
        let g = Nsf::parse(&file(2, 1, 0x8000, [0; 8], 0)).unwrap();
        assert!(!g.is_nsfe());
        assert_eq!(g.song_title(0), b"");
        assert_eq!(g.length_ms(1), -1);
        assert!(g.playlist().is_empty() && g.ripper().is_empty());
    }

    /// A version 2 file whose program is the 4 bytes of `file`, then `tail`.
    fn nsf2_with(flags: u8, tail: &[Vec<u8>]) -> Vec<u8> {
        let mut f = file(3, 1, 0x8000, [0; 8], 0);
        f[5] = 2;
        f[0x7C] = flags;
        f[0x7D] = 4;
        for c in tail {
            f.extend_from_slice(c);
        }
        f
    }

    #[test]
    fn a_version_2_file_carries_nsfe_chunks_after_its_program() {
        let mut time = Vec::new();
        for t in [30_000i32, 40_000] {
            time.extend_from_slice(&t.to_le_bytes());
        }
        let f = nsf2_with(
            nsf2::METADATA,
            &[
                chunk(b"auth", b"Longer title\0\0\0Ripper\0"),
                chunk(b"tlbl", b"One\0Two\0"),
                chunk(b"time", &time),
                chunk(b"plst", &[1, 0]),
                chunk(b"text", b"Note\0"),
                chunk(b"RATE", &[0x0A, 0x40]),
                chunk(b"VRC7", &[0]),
                chunk(b"NEND", &[]),
            ],
        );
        let g = Nsf::parse(&f).unwrap();
        assert!(!g.is_nsfe());
        assert_eq!(g.data, [1, 2, 3, 4]);
        // A field that the chunk gives replaces the header's. One that it
        // leaves empty keeps the header's.
        assert_eq!(g.title(), b"Longer title");
        assert_eq!(g.author(), b"Name");
        assert_eq!(g.copyright(), b"?");
        assert_eq!(g.ripper(), b"Ripper");
        assert_eq!(g.comment(), b"Note");
        assert_eq!(
            (g.song_title(0), g.song_title(1), g.song_title(2)),
            (&b"One"[..], &b"Two"[..], &b""[..])
        );
        assert_eq!(
            (g.time_ms(0), g.time_ms(1), g.time_ms(2)),
            (30_000, 40_000, -1)
        );
        assert_eq!(g.playlist(), [1, 0]);
        assert_eq!((g.ntsc_us, g.pal_us), (0x400A, 19997));
    }

    #[test]
    fn the_chunks_a_version_2_file_may_not_repeat_are_skipped() {
        let f = nsf2_with(
            0,
            &[
                chunk(b"INFO", &[0; 10]),
                chunk(b"DATA", &[9, 9]),
                chunk(b"BANK", &[7; 8]),
                chunk(b"NSF2", &[nsf2::NO_PLAY]),
                chunk(b"tlbl", b"Kept\0"),
            ],
        );
        let g = Nsf::parse(&f).unwrap();
        assert_eq!(g.data, [1, 2, 3, 4]);
        assert!(!g.is_banked());
        assert_eq!(g.flags, 0);
        assert_eq!(g.song_title(0), b"Kept");
    }

    #[test]
    fn an_unknown_mandatory_chunk_matters_only_when_the_flag_says_so() {
        let tail = [chunk(b"SAMP", &[1, 2, 3]), chunk(b"tlbl", b"Kept\0")];
        assert_eq!(
            Nsf::parse(&nsf2_with(nsf2::METADATA, &tail)),
            Err(NsfError::Mandatory(*b"SAMP"))
        );
        let g = Nsf::parse(&nsf2_with(0, &tail)).unwrap();
        assert_eq!(g.song_title(0), b"Kept");
    }

    #[test]
    fn a_version_1_file_ignores_what_follows_its_program() {
        let mut f = file(3, 1, 0x8000, [0; 8], 0);
        f[0x7D] = 4;
        f.extend_from_slice(&chunk(b"tlbl", b"Ignored\0"));
        let g = Nsf::parse(&f).unwrap();
        // The program, then the chunk's 8 header bytes and 8 bytes of text.
        assert_eq!(g.data.len(), 4 + 8 + 8);
        assert_eq!(g.song_title(0), b"");
    }

    #[test]
    fn a_version_2_file_without_a_length_has_no_metadata() {
        let mut f = nsf2_with(0, &[chunk(b"tlbl", b"Lost\0")]);
        f[0x7D] = 0;
        let g = Nsf::parse(&f).unwrap();
        assert_eq!(g.song_title(0), b"");
        assert_eq!(g.data.len(), 4 + 8 + 5);
    }

    #[test]
    fn the_region_chunk_of_a_version_2_file_changes_the_region() {
        let g = Nsf::parse(&nsf2_with(0, &[chunk(b"regn", &[2])])).unwrap();
        assert_eq!(g.region(), Region::Pal);
    }

    #[test]
    fn damaged_files_never_panic() {
        let nsfe = basic(&[
            chunk(b"auth", b"Game\0Artist\0Copy\0Ripper\0"),
            chunk(b"tlbl", b"One\0Two\0Three\0"),
            chunk(b"time", &[1, 0, 0, 0, 2, 0, 0, 0]),
            chunk(b"plst", &[0, 1, 2]),
            chunk(b"RATE", &[1, 2, 3, 4]),
        ]);
        let nsf2 = nsf2_with(
            nsf2::METADATA,
            &[
                chunk(b"auth", b"Game\0Artist\0"),
                chunk(b"tlbl", b"One\0Two\0Three\0"),
                chunk(b"time", &[1, 0, 0, 0]),
                chunk(b"plst", &[2, 1, 0]),
                chunk(b"VRC7", &[0]),
            ],
        );
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..40_000 {
            let mut f = if round % 2 == 0 { &nsfe } else { &nsf2 }.clone();
            for _ in 0..1 + next() % 4 {
                let at = (next() % f.len() as u64) as usize;
                match next() % 3 {
                    0 => f[at] = next() as u8,
                    1 => f.truncate(at),
                    _ => f.insert(at, next() as u8),
                }
                if f.is_empty() {
                    break;
                }
            }
            if let Ok(g) = Nsf::parse(&f) {
                for song in 0..usize::from(g.songs) + 2 {
                    let _ = (g.song_title(song), g.length_ms(song));
                }
            }
        }
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
