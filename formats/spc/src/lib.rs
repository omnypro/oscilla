//! The SPC container: a 0x100-byte header with the SPC700's registers and an
//! ID666 tag, then the 64 KiB of sound RAM, the 128 DSP registers and 64 bytes
//! of RAM that sit under the boot ROM. An optional `xid6` chunk follows with
//! longer tags and the play times. Cores restore the state in their own way.

/// A file larger than this is refused. A real file is 66,048 bytes, plus tags.
pub const MAX_FILE: usize = 1 << 20;

pub const HEADER_SIZE: usize = 0x100;

/// The smallest file that holds the RAM and the DSP registers. The 64 bytes of
/// extra RAM after them are zero when a file leaves them out.
pub const MIN_FILE: usize = 0x10180;

const RAM_AT: usize = 0x100;
const DSP_AT: usize = 0x10100;
const EXTRA_AT: usize = 0x101C0;
const XID6_AT: usize = 0x10200;

const MAGIC: &[u8] = b"SNES-SPC700 Sound File Data";

/// One tick of the xid6 lengths is 1/64000 s.
pub const TICKS_PER_SECOND: i64 = 64_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpcError {
    /// Shorter than the RAM and the registers, or the magic text is wrong.
    NotSpc,
    /// The file is larger than MAX_FILE.
    TooLarge,
}

impl std::fmt::Display for SpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpcError::NotSpc => write!(f, "Not an SPC file."),
            SpcError::TooLarge => write!(f, "The file is larger than 1 MiB."),
        }
    }
}

impl std::error::Error for SpcError {}

/// The SPC700's registers as the file stores them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registers {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub psw: u8,
    pub sp: u8,
}

/// Text that a tag holds, with the bytes up to the first NUL.
fn cut(field: &[u8]) -> &[u8] {
    match field.iter().position(|&b| b == 0) {
        Some(n) => &field[..n],
        None => field,
    }
}

/// The play times and settings of the xid6 chunk. A field that the file
/// leaves out is None.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Xid6 {
    pub song: Option<Vec<u8>>,
    pub game: Option<Vec<u8>>,
    pub artist: Option<Vec<u8>>,
    pub dumper: Option<Vec<u8>>,
    pub comments: Option<Vec<u8>>,
    pub soundtrack: Option<Vec<u8>>,
    pub publisher: Option<Vec<u8>>,
    /// Lengths in ticks of 1/64000 s. The end length can be negative.
    pub intro: Option<i32>,
    pub loop_length: Option<i32>,
    pub end: Option<i32>,
    pub fade: Option<i32>,
    /// A set bit mutes the voice with that number.
    pub muted: Option<u8>,
    /// How many times the loop section plays.
    pub loops: Option<u8>,
    /// 65536 is the unchanged output.
    pub amplification: Option<u32>,
}

/// A parsed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spc {
    pub registers: Registers,
    pub ram: Box<[u8; 65536]>,
    pub dsp: [u8; 128],
    /// The RAM that the boot ROM covers at 0xFFC0 to 0xFFFF.
    pub extra: [u8; 64],
    song: [u8; 32],
    game: [u8; 32],
    dumper: [u8; 16],
    comments: [u8; 32],
    artist: [u8; 32],
    /// Whole seconds before the fade, from the ID666 tag.
    pub seconds: u32,
    /// The fade in milliseconds, from the ID666 tag.
    pub fade_ms: u32,
    /// A set bit is a voice that the dumper disabled.
    pub disabled: u8,
    pub xid6: Xid6,
}

fn digits(field: &[u8]) -> Option<u32> {
    let text = cut(field);
    if text.is_empty() {
        return Some(0);
    }
    let mut n: u32 = 0;
    for &b in text {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add(u32::from(b - b'0'))?;
    }
    Some(n)
}

/// Both layouts of the tag use the same bytes, and nothing in the header says
/// which one a file has. A text date has slashes at 2 and 5. Without a date,
/// the play time and fade must read as decimal digits.
fn is_text_tag(h: &[u8]) -> bool {
    let date = &h[0x9E..0xA9];
    if date[2] == b'/' && date[5] == b'/' {
        return true;
    }
    if date.iter().any(|&b| b != 0) {
        return false;
    }
    let seconds = &h[0xA9..0xAC];
    let fade = &h[0xAC..0xB1];
    digits(seconds).is_some()
        && digits(fade).is_some()
        && seconds.iter().chain(fade).all(|&b| b == 0 || b.is_ascii_digit())
}

fn copy<const N: usize>(from: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&from[..N]);
    out
}

fn parse_xid6(chunk: &[u8]) -> Xid6 {
    let mut x = Xid6::default();
    let mut at = 0;
    while at + 4 <= chunk.len() {
        let id = chunk[at];
        let kind = chunk[at + 1];
        let length = u16::from_le_bytes([chunk[at + 2], chunk[at + 3]]);
        at += 4;
        let mut data: &[u8] = &[];
        if kind != 0 {
            let end = at + usize::from(length);
            if end > chunk.len() {
                break;
            }
            data = &chunk[at..end];
            at = (end + 3) & !3;
        }
        let string = || Some(cut(data).to_vec());
        let integer = || -> Option<i32> {
            (data.len() >= 4).then(|| i32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        };
        let small = || if kind == 0 { Some(length) } else { None };
        match id {
            0x01 if kind == 1 => x.song = string(),
            0x02 if kind == 1 => x.game = string(),
            0x03 if kind == 1 => x.artist = string(),
            0x04 if kind == 1 => x.dumper = string(),
            0x07 if kind == 1 => x.comments = string(),
            0x10 if kind == 1 => x.soundtrack = string(),
            0x13 if kind == 1 => x.publisher = string(),
            0x30 if kind == 4 => x.intro = integer(),
            0x31 if kind == 4 => x.loop_length = integer(),
            0x32 if kind == 4 => x.end = integer(),
            0x33 if kind == 4 => x.fade = integer(),
            0x34 => x.muted = small().map(|v| v as u8),
            0x35 => x.loops = small().map(|v| v as u8),
            0x36 if kind == 4 => x.amplification = integer().map(|v| v as u32),
            _ => {}
        }
    }
    x
}

impl Spc {
    pub fn parse(file: &[u8]) -> Result<Spc, SpcError> {
        if file.len() > MAX_FILE {
            return Err(SpcError::TooLarge);
        }
        if file.len() < MIN_FILE || &file[..MAGIC.len()] != MAGIC {
            return Err(SpcError::NotSpc);
        }
        let h = &file[..HEADER_SIZE];
        let tagged = h[0x23] == 26;
        let mut ram = Box::new([0u8; 65536]);
        ram.copy_from_slice(&file[RAM_AT..RAM_AT + 65536]);
        let mut extra = [0u8; 64];
        if file.len() >= EXTRA_AT + 64 {
            extra.copy_from_slice(&file[EXTRA_AT..EXTRA_AT + 64]);
        }
        let xid6 = match file.get(XID6_AT..) {
            Some(rest) if rest.len() >= 8 && &rest[..4] == b"xid6" => {
                let size = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
                let body = &rest[8..];
                parse_xid6(&body[..size.min(body.len())])
            }
            _ => Xid6::default(),
        };
        let mut spc = Spc {
            registers: Registers {
                pc: u16::from_le_bytes([h[0x25], h[0x26]]),
                a: h[0x27],
                x: h[0x28],
                y: h[0x29],
                psw: h[0x2A],
                sp: h[0x2B],
            },
            ram,
            dsp: copy(&file[DSP_AT..]),
            extra,
            song: [0; 32],
            game: [0; 32],
            dumper: [0; 16],
            comments: [0; 32],
            artist: [0; 32],
            seconds: 0,
            fade_ms: 0,
            disabled: 0,
            xid6,
        };
        if tagged {
            spc.song = copy(&h[0x2E..]);
            spc.game = copy(&h[0x4E..]);
            spc.dumper = copy(&h[0x6E..]);
            spc.comments = copy(&h[0x7E..]);
            if is_text_tag(h) {
                spc.seconds = digits(&h[0xA9..0xAC]).unwrap_or(0);
                spc.fade_ms = digits(&h[0xAC..0xB1]).unwrap_or(0);
                spc.artist = copy(&h[0xB1..]);
                spc.disabled = h[0xD1];
            } else {
                spc.seconds = u32::from_le_bytes([h[0xA9], h[0xAA], h[0xAB], 0]);
                spc.fade_ms = u32::from_le_bytes([h[0xAC], h[0xAD], h[0xAE], h[0xAF]]);
                spc.artist = copy(&h[0xB0..]);
                spc.disabled = h[0xD0];
            }
        }
        Ok(spc)
    }

    pub fn song(&self) -> &[u8] {
        self.xid6.song.as_deref().unwrap_or_else(|| cut(&self.song))
    }

    pub fn game(&self) -> &[u8] {
        self.xid6.game.as_deref().unwrap_or_else(|| cut(&self.game))
    }

    pub fn artist(&self) -> &[u8] {
        self.xid6.artist.as_deref().unwrap_or_else(|| cut(&self.artist))
    }

    pub fn dumper(&self) -> &[u8] {
        self.xid6.dumper.as_deref().unwrap_or_else(|| cut(&self.dumper))
    }

    pub fn comments(&self) -> &[u8] {
        self.xid6.comments.as_deref().unwrap_or_else(|| cut(&self.comments))
    }

    /// The time before the fade starts, in milliseconds, or -1 when the file
    /// gives none. The xid6 lengths win over the ID666 seconds: the intro,
    /// the loop section as many times as the file says (once when it says
    /// nothing) and the end.
    pub fn time_ms(&self) -> i64 {
        let x = &self.xid6;
        if x.intro.is_some() || x.loop_length.is_some() || x.end.is_some() {
            let ticks = i64::from(x.intro.unwrap_or(0))
                + i64::from(x.loop_length.unwrap_or(0)) * i64::from(x.loops.unwrap_or(1))
                + i64::from(x.end.unwrap_or(0));
            return ticks.max(0) * 1000 / TICKS_PER_SECOND;
        }
        if self.seconds == 0 {
            return -1;
        }
        i64::from(self.seconds) * 1000
    }

    /// The fade in milliseconds, or -1 when the file gives none.
    pub fn fade_ms(&self) -> i64 {
        if let Some(t) = self.xid6.fade {
            return i64::from(t).max(0) * 1000 / TICKS_PER_SECOND;
        }
        if self.seconds == 0 && self.fade_ms == 0 {
            return -1;
        }
        i64::from(self.fade_ms)
    }

    /// The voices to leave silent: the xid6 mask when there is one, otherwise
    /// the ID666 byte.
    pub fn muted(&self) -> u8 {
        self.xid6.muted.unwrap_or(self.disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> Vec<u8> {
        let mut f = vec![0u8; 0x10200];
        f[..33].copy_from_slice(b"SNES-SPC700 Sound File Data v0.30");
        f[0x21] = 26;
        f[0x22] = 26;
        f[0x23] = 26;
        f[0x24] = 30;
        f[0x25..0x27].copy_from_slice(&0x0400u16.to_le_bytes());
        f[0x27..0x2C].copy_from_slice(&[1, 2, 3, 0x22, 0xEF]);
        f[0x2E..0x33].copy_from_slice(b"Theme");
        f[0x4E..0x52].copy_from_slice(b"Game");
        f[0x100 + 0x1234] = 0x77;
        f[0x10100 + 0x4C] = 0x81;
        f[0x101C0] = 0x42;
        f
    }

    fn text_tag(f: &mut [u8], seconds: &[u8], fade: &[u8]) {
        f[0x9E..0xA9].copy_from_slice(b"11/03/2026\0");
        f[0xA9..0xA9 + seconds.len()].copy_from_slice(seconds);
        f[0xAC..0xAC + fade.len()].copy_from_slice(fade);
        f[0xB1..0xB6].copy_from_slice(b"Maker");
        f[0xD1] = 0x05;
    }

    #[test]
    fn reads_the_registers_and_memory() {
        let s = Spc::parse(&file()).unwrap();
        let r = s.registers;
        assert_eq!((r.pc, r.a, r.x, r.y, r.psw, r.sp), (0x400, 1, 2, 3, 0x22, 0xEF));
        assert_eq!(s.ram[0x1234], 0x77);
        assert_eq!(s.dsp[0x4C], 0x81);
        assert_eq!(s.extra[0], 0x42);
        assert_eq!(s.song(), b"Theme");
        assert_eq!(s.game(), b"Game");
    }

    #[test]
    fn reads_a_text_tag() {
        let mut f = file();
        text_tag(&mut f, b"120", b"4000");
        let s = Spc::parse(&f).unwrap();
        assert_eq!((s.seconds, s.fade_ms), (120, 4000));
        assert_eq!(s.artist(), b"Maker");
        assert_eq!(s.muted(), 5);
        assert_eq!((s.time_ms(), s.fade_ms()), (120_000, 4000));
    }

    #[test]
    fn reads_a_binary_tag() {
        let mut f = file();
        f[0x9E..0xA2].copy_from_slice(&[0x26, 0x20, 0x11, 0x03]);
        f[0xA9..0xAC].copy_from_slice(&[0x35, 0, 0]);
        f[0xAC..0xB0].copy_from_slice(&5000u32.to_le_bytes());
        f[0xB0..0xB5].copy_from_slice(b"Maker");
        f[0xD0] = 0x81;
        let s = Spc::parse(&f).unwrap();
        assert_eq!((s.seconds, s.fade_ms), (0x35, 5000));
        assert_eq!(s.artist(), b"Maker");
        assert_eq!(s.muted(), 0x81);
    }

    #[test]
    fn a_tag_without_a_date_reads_as_text_when_the_times_are_digits() {
        let mut f = file();
        f[0xA9..0xAC].copy_from_slice(b"90\0");
        f[0xAC..0xB1].copy_from_slice(b"1000\0");
        let s = Spc::parse(&f).unwrap();
        assert_eq!((s.seconds, s.fade_ms), (90, 1000));
    }

    #[test]
    fn a_file_without_a_tag_has_no_times() {
        let mut f = file();
        f[0x23] = 27;
        let s = Spc::parse(&f).unwrap();
        assert_eq!(s.song(), b"");
        assert_eq!((s.time_ms(), s.fade_ms()), (-1, -1));
    }

    fn with_xid6(mut f: Vec<u8>, items: &[u8]) -> Vec<u8> {
        f.extend_from_slice(b"xid6");
        f.extend_from_slice(&(items.len() as u32).to_le_bytes());
        f.extend_from_slice(items);
        f
    }

    #[test]
    fn reads_the_extended_tags_and_times() {
        let mut items = vec![0x01, 1, 6, 0, b'L', b'o', b'n', b'g', b'e', 0, 0, 0];
        for (id, v) in [(0x30u8, 64_000i32), (0x31, 128_000), (0x32, 32_000), (0x33, 192_000)] {
            items.extend_from_slice(&[id, 4, 4, 0]);
            items.extend_from_slice(&v.to_le_bytes());
        }
        items.extend_from_slice(&[0x34, 0, 0x0C, 0]);
        items.extend_from_slice(&[0x35, 0, 2, 0]);
        items.extend_from_slice(&[0x36, 4, 4, 0, 0, 0, 2, 0]);
        let s = Spc::parse(&with_xid6(file(), &items)).unwrap();
        assert_eq!(s.song(), b"Longe");
        assert_eq!(s.muted(), 0x0C);
        assert_eq!(s.xid6.amplification, Some(131_072));
        // 1 s intro, two 2 s loops and 0.5 s end.
        assert_eq!(s.time_ms(), 5500);
        assert_eq!(s.fade_ms(), 3000);
    }

    #[test]
    fn the_end_length_can_be_negative() {
        let mut items = vec![];
        for (id, v) in [(0x30u8, 640_000i32), (0x32, -128_000)] {
            items.extend_from_slice(&[id, 4, 4, 0]);
            items.extend_from_slice(&v.to_le_bytes());
        }
        let s = Spc::parse(&with_xid6(file(), &items)).unwrap();
        assert_eq!(s.time_ms(), 8000);
    }

    #[test]
    fn a_damaged_chunk_is_cut_short_and_never_panics() {
        let items = [0x01, 1, 0xFF, 0x7F, 1, 2, 3];
        let s = Spc::parse(&with_xid6(file(), &items)).unwrap();
        assert_eq!(s.xid6, Xid6::default());
        let mut f = file();
        f.extend_from_slice(b"xid6\xFF\xFF\xFF\xFF");
        assert!(Spc::parse(&f).is_ok());
        f.truncate(0x10200 + 5);
        assert!(Spc::parse(&f).is_ok());
    }

    #[test]
    fn a_file_without_the_extra_ram_is_accepted() {
        let mut f = file();
        f.truncate(MIN_FILE);
        let s = Spc::parse(&f).unwrap();
        assert_eq!(s.extra, [0; 64]);
    }

    #[test]
    fn refuses_bad_files() {
        assert_eq!(Spc::parse(b"SNES"), Err(SpcError::NotSpc));
        let mut f = file();
        f[0] = b'X';
        assert_eq!(Spc::parse(&f), Err(SpcError::NotSpc));
        assert_eq!(Spc::parse(&vec![0; MAX_FILE + 1]), Err(SpcError::TooLarge));
    }
}
