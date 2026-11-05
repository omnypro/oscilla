//! The VGM container: a header with the clocks of the chips that the file
//! drives, a stream of register writes and waits counted in samples of
//! 44,100 Hz, and an optional GD3 tag. A file can be gzip compressed (VGZ).
//! Cores run the stream in their own way.

use flate2::{Decompress, FlushDecompress, Status};

/// A file larger than this, packed or unpacked, is refused.
pub const MAX_FILE: usize = 64 << 20;

/// The unit of every wait in the stream.
pub const SAMPLE_RATE: u32 = 44_100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VgmError {
    /// Too short, the wrong magic text, or a data offset outside the file.
    NotVgm,
    /// The file is larger than MAX_FILE.
    TooLarge,
    /// The gzip stream is damaged or incomplete.
    Stream,
}

impl std::fmt::Display for VgmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VgmError::NotVgm => write!(f, "Not a VGM file."),
            VgmError::TooLarge => write!(f, "The file is larger than 64 MiB."),
            VgmError::Stream => write!(f, "The gzip data is damaged."),
        }
    }
}

impl std::error::Error for VgmError {}

/// The clock words of the header in the order that the file numbers its chips
/// (the chip number of a stream setup or of the extra header).
const CHIPS: [(usize, &str); 41] = [
    (0x0C, "SN76489"),
    (0x10, "YM2413"),
    (0x2C, "YM2612"),
    (0x30, "YM2151"),
    (0x38, "Sega PCM"),
    (0x40, "RF5C68"),
    (0x44, "YM2203"),
    (0x48, "YM2608"),
    (0x4C, "YM2610"),
    (0x50, "YM3812"),
    (0x54, "YM3526"),
    (0x58, "Y8950"),
    (0x5C, "YMF262"),
    (0x60, "YMF278B"),
    (0x64, "YMF271"),
    (0x68, "YMZ280B"),
    (0x6C, "RF5C164"),
    (0x70, "PWM"),
    (0x74, "AY8910"),
    (0x80, "GB DMG"),
    (0x84, "NES APU"),
    (0x88, "MultiPCM"),
    (0x8C, "uPD7759"),
    (0x90, "OKIM6258"),
    (0x98, "OKIM6295"),
    (0x9C, "K051649"),
    (0xA0, "K054539"),
    (0xA4, "HuC6280"),
    (0xA8, "C140"),
    (0xAC, "K053260"),
    (0xB0, "Pokey"),
    (0xB4, "QSound"),
    (0xB8, "SCSP"),
    (0xC0, "WonderSwan"),
    (0xC4, "VSU"),
    (0xC8, "SAA1099"),
    (0xCC, "ES5503"),
    (0xD0, "ES5506"),
    (0xD8, "X1-010"),
    (0xDC, "C352"),
    (0xE0, "GA20"),
];

pub const SN76489: u8 = 0;
pub const YM2612: u8 = 2;

/// A chip that the header names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// The number that stream setups and the extra header use for the chip.
    pub id: u8,
    pub name: &'static str,
    /// The clock in Hz without the flag bits.
    pub clock: u32,
    /// The file has a second chip of this kind.
    pub dual: bool,
    /// Bit 31 of the clock word, which picks a variant of the chip: the YM3438
    /// for the YM2612, the T6W28 for the SN76489 when the file is also dual.
    pub variant: bool,
    /// The clock of the second chip. It equals `clock` unless the extra header
    /// gives another.
    pub second_clock: u32,
}

/// The tag text of the GD3 block, in the order that the block stores it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gd3 {
    pub title: String,
    pub title_jp: String,
    pub game: String,
    pub game_jp: String,
    pub system: String,
    pub system_jp: String,
    pub author: String,
    pub author_jp: String,
    pub date: String,
    pub ripper: String,
    pub notes: String,
}

/// A parsed file. The stream and the data blocks stay in the file's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vgm {
    /// The BCD version, 0x171 for 1.71.
    pub version: u32,
    pub total_samples: u32,
    pub loop_samples: u32,
    /// The rate of the original machine in Hz (50 or 60), or 0 when the file
    /// asks for no scaling.
    pub rate: u32,
    /// The white noise taps of the SN76489, its shift register width and its
    /// flags.
    pub psg_feedback: u16,
    pub psg_width: u8,
    pub psg_flags: u8,
    /// The volume modifier as the header stores it: the gain is
    /// 2^(modifier / 32).
    pub volume_modifier: i8,
    pub loop_base: i8,
    /// 0 stands for the default of 16.
    pub loop_modifier: u8,
    pub chips: Vec<Chip>,
    pub gd3: Option<Gd3>,
    file: Vec<u8>,
    data_at: usize,
    loop_at: Option<usize>,
}

fn word(file: &[u8], at: usize, limit: usize) -> u32 {
    if at + 4 > limit || at + 4 > file.len() {
        return 0;
    }
    u32::from_le_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]])
}

fn half(file: &[u8], at: usize, limit: usize) -> u16 {
    if at + 2 > limit || at + 2 > file.len() {
        return 0;
    }
    u16::from_le_bytes([file[at], file[at + 1]])
}

fn byte(file: &[u8], at: usize, limit: usize) -> u8 {
    if at >= limit || at >= file.len() {
        return 0;
    }
    file[at]
}

/// Unpacks a gzip member. The header is skipped by hand because the inflater
/// of this workspace reads raw deflate and zlib only.
fn gunzip(src: &[u8]) -> Result<Vec<u8>, VgmError> {
    if src.len() < 18 || src[2] != 8 {
        return Err(VgmError::Stream);
    }
    let flags = src[3];
    let mut at = 10;
    if flags & 4 != 0 {
        let n = usize::from(*src.get(at).ok_or(VgmError::Stream)?)
            | usize::from(*src.get(at + 1).ok_or(VgmError::Stream)?) << 8;
        at += 2 + n;
    }
    for bit in [8, 16] {
        if flags & bit != 0 {
            let n = src
                .get(at..)
                .and_then(|s| s.iter().position(|&b| b == 0))
                .ok_or(VgmError::Stream)?;
            at += n + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    let body = src.get(at..).ok_or(VgmError::Stream)?;
    let mut cap = body
        .len()
        .saturating_mul(4)
        .saturating_add(1024)
        .min(MAX_FILE);
    let mut buf = vec![0u8; cap];
    let mut len = 0;
    let mut z = Decompress::new(false);
    loop {
        if len == cap {
            if cap >= MAX_FILE {
                return Err(VgmError::TooLarge);
            }
            cap = (cap * 2).min(MAX_FILE);
            buf.resize(cap, 0);
        }
        let consumed = z.total_in() as usize;
        let status = z
            .decompress(&body[consumed..], &mut buf[len..], FlushDecompress::None)
            .map_err(|_| VgmError::Stream)?;
        len = z.total_out() as usize;
        match status {
            Status::Ok => {}
            Status::StreamEnd => break,
            Status::BufError => return Err(VgmError::Stream),
        }
    }
    buf.truncate(len);
    Ok(buf)
}

fn utf16_strings(block: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut units: Vec<u16> = Vec::new();
    for pair in block.chunks_exact(2) {
        let u = u16::from_le_bytes([pair[0], pair[1]]);
        if u == 0 {
            out.push(
                char::decode_utf16(units.drain(..))
                    .map(|c| c.unwrap_or('\u{FFFD}'))
                    .collect(),
            );
        } else {
            units.push(u);
        }
    }
    out
}

fn parse_gd3(file: &[u8], at: usize) -> Option<Gd3> {
    if file.get(at..at + 12)?[..4] != *b"Gd3 " {
        return None;
    }
    let len = u32::from_le_bytes(file[at + 8..at + 12].try_into().ok()?) as usize;
    let start = at + 12;
    let end = start.saturating_add(len).min(file.len());
    let mut s = utf16_strings(&file[start..end]).into_iter();
    let mut next = || s.next().unwrap_or_default();
    Some(Gd3 {
        title: next(),
        title_jp: next(),
        game: next(),
        game_jp: next(),
        system: next(),
        system_jp: next(),
        author: next(),
        author_jp: next(),
        date: next(),
        ripper: next(),
        notes: next(),
    })
}

impl Vgm {
    /// Reads a VGM or VGZ file.
    pub fn parse(bytes: &[u8]) -> Result<Vgm, VgmError> {
        if bytes.len() > MAX_FILE {
            return Err(VgmError::TooLarge);
        }
        let file = if bytes.starts_with(&[0x1F, 0x8B]) {
            gunzip(bytes)?
        } else {
            bytes.to_vec()
        };
        if file.len() < 0x40 || &file[..4] != b"Vgm " {
            return Err(VgmError::NotVgm);
        }
        let version = word(&file, 8, file.len());
        let data_at = if version >= 0x150 {
            match word(&file, 0x34, file.len()) {
                0 => 0x40,
                n => 0x34usize.saturating_add(n as usize),
            }
        } else {
            0x40
        };
        if data_at < 0x40 || data_at > file.len() {
            return Err(VgmError::NotVgm);
        }
        // Header bytes that the stream overlaps count as zero.
        let limit = data_at.min(0x100);
        let w = |at| word(&file, at, limit);

        let mut chips: Vec<Chip> = Vec::new();
        for (id, &(at, name)) in CHIPS.iter().enumerate() {
            let raw = w(at);
            if raw & 0x3FFF_FFFF != 0 {
                chips.push(Chip {
                    id: id as u8,
                    name,
                    clock: raw & 0x3FFF_FFFF,
                    dual: raw & 0x4000_0000 != 0,
                    variant: raw & 0x8000_0000 != 0,
                    second_clock: raw & 0x3FFF_FFFF,
                });
            }
        }
        // Files up to 1.01 keep the clock of the YM2612 in the YM2413 word.
        if version < 0x110
            && let Some(at) = chips.iter().position(|c| c.id == 1 && c.clock > 5_000_000)
        {
            chips[at].id = YM2612;
            chips[at].name = "YM2612";
        }

        if version >= 0x170 {
            let extra = w(0xBC);
            if extra != 0 {
                let base = 0xBC + extra as usize;
                let at = |o: usize| word(&file, o, file.len());
                let clocks = at(base + 4);
                if clocks != 0 {
                    let list = base + 4 + clocks as usize;
                    let count = byte(&file, list, file.len()) as usize;
                    for i in 0..count {
                        let entry = list + 1 + i * 5;
                        let id = byte(&file, entry, file.len());
                        let clock = at(entry + 1);
                        if let Some(c) = chips.iter_mut().find(|c| c.id == id) {
                            c.second_clock = clock & 0x3FFF_FFFF;
                        }
                    }
                }
            }
        }

        let loop_word = w(0x1C);
        let loop_at = if loop_word == 0 {
            None
        } else {
            let at = 0x1Cusize.saturating_add(loop_word as usize);
            (at >= data_at && at < file.len()).then_some(at)
        };
        let gd3_word = w(0x14);
        let gd3 = if gd3_word == 0 {
            None
        } else {
            parse_gd3(&file, 0x14usize.saturating_add(gd3_word as usize))
        };

        let (mut psg_feedback, mut psg_width) =
            (half(&file, 0x28, limit), byte(&file, 0x2A, limit));
        if version < 0x110 {
            psg_feedback = 9;
            psg_width = 16;
        }

        Ok(Vgm {
            version,
            total_samples: w(0x18),
            loop_samples: if loop_at.is_some() { w(0x20) } else { 0 },
            rate: if version >= 0x101 { w(0x24) } else { 0 },
            psg_feedback,
            psg_width,
            psg_flags: if version >= 0x151 {
                byte(&file, 0x2B, limit)
            } else {
                0
            },
            volume_modifier: if version >= 0x150 {
                byte(&file, 0x7C, limit) as i8
            } else {
                0
            },
            loop_base: if version >= 0x150 {
                byte(&file, 0x7E, limit) as i8
            } else {
                0
            },
            loop_modifier: if version >= 0x151 {
                byte(&file, 0x7F, limit)
            } else {
                0
            },
            chips,
            gd3,
            file,
            data_at,
            loop_at,
        })
    }

    /// The chip with this number, if the header names it.
    pub fn chip(&self, id: u8) -> Option<&Chip> {
        self.chips.iter().find(|c| c.id == id)
    }

    /// The offset in the file of the first command.
    pub fn data_at(&self) -> usize {
        self.data_at
    }

    /// The offset in the file of the loop point, or None for a file that does
    /// not loop.
    pub fn loop_at(&self) -> Option<usize> {
        self.loop_at
    }

    /// The file's bytes after unpacking.
    pub fn bytes(&self) -> &[u8] {
        &self.file
    }

    /// The gain that the volume modifier asks for.
    pub fn gain(&self) -> f64 {
        let m = i32::from(self.volume_modifier);
        // The value -63 stands for -64 so that the gain can reach 1/4.
        let m = if m == -63 { -64 } else { m };
        (f64::from(m) / 32.0).exp2()
    }

    /// The number of passes through the loop that the file asks for when the
    /// player would play `passes` of them.
    pub fn loop_count(&self, passes: u32) -> u32 {
        let modifier = if self.loop_modifier == 0 {
            16
        } else {
            u32::from(self.loop_modifier)
        };
        let scaled = i64::from(passes) * i64::from(modifier) / 16;
        (scaled - i64::from(self.loop_base)).max(1) as u32
    }

    /// The commands from the first one.
    pub fn commands(&self) -> Cursor<'_> {
        Cursor {
            file: &self.file,
            pos: self.data_at,
            damaged: false,
        }
    }

    /// The commands from a file offset such as `loop_at`.
    pub fn commands_from(&self, pos: usize) -> Cursor<'_> {
        Cursor {
            file: &self.file,
            pos,
            damaged: false,
        }
    }

    /// Collects the data blocks of the stream into one bank for each type.
    pub fn banks(&self) -> Banks {
        Banks::scan(self)
    }
}

/// One command of the stream. A write names the chip with `second`, which is
/// true for the second chip of a file with two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Wait this many samples.
    Wait(u32),
    Psg {
        second: bool,
        value: u8,
    },
    PsgStereo {
        second: bool,
        value: u8,
    },
    Ym2612 {
        second: bool,
        port: u8,
        register: u8,
        value: u8,
    },
    /// Write the next byte of the data bank to register 0x2A of the first
    /// YM2612, then wait this many samples.
    Ym2612Dac {
        wait: u8,
    },
    /// A data block. The payload is `len` bytes from `at` in the file.
    DataBlock {
        kind: u8,
        at: usize,
        len: usize,
    },
    /// Moves the read position of the data bank (command 0xE0).
    Seek(u32),
    StreamSetup {
        stream: u8,
        chip: u8,
        second: bool,
        port: u8,
        register: u8,
    },
    StreamData {
        stream: u8,
        bank: u8,
        step: u8,
        base: u8,
    },
    StreamFrequency {
        stream: u8,
        hz: u32,
    },
    StreamStart {
        stream: u8,
        at: u32,
        mode: u8,
        length: u32,
    },
    StreamStop {
        stream: u8,
    },
    StreamFast {
        stream: u8,
        block: u16,
        flags: u8,
    },
    /// A command for a chip or a purpose that this crate does not decode. Its
    /// operands are skipped.
    Other(u8),
    /// The end of the sound data, the end of the file, or a byte that is not a
    /// command.
    End,
}

/// A position in the stream.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    file: &'a [u8],
    pos: usize,
    damaged: bool,
}

impl Cursor<'_> {
    /// The file offset of the next command.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// True after the stream ended on a byte that is not a command or on a
    /// command cut short by the end of the file.
    pub fn damaged(&self) -> bool {
        self.damaged
    }

    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.file.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }

    /// Reads the next command. At the end the position stays where it is and
    /// every further call returns `End`.
    pub fn read(&mut self) -> Command {
        let start = self.pos;
        let Some(&op) = self.file.get(self.pos) else {
            return Command::End;
        };
        self.pos += 1;
        let c = self.decode(op);
        match c {
            Some(Command::End) | None => {
                self.pos = start;
                self.damaged |= c.is_none();
                Command::End
            }
            Some(c) => c,
        }
    }

    fn decode(&mut self, op: u8) -> Option<Command> {
        Some(match op {
            0x66 => Command::End,
            0x4F | 0x3F => {
                let value = self.take(1)?[0];
                Command::PsgStereo {
                    second: op == 0x3F,
                    value,
                }
            }
            0x50 | 0x30 => {
                let value = self.take(1)?[0];
                Command::Psg {
                    second: op == 0x30,
                    value,
                }
            }
            0x52 | 0x53 | 0xA2 | 0xA3 => {
                let d = self.take(2)?;
                Command::Ym2612 {
                    second: op >= 0xA0,
                    port: op & 1,
                    register: d[0],
                    value: d[1],
                }
            }
            0x61 => {
                let d = self.take(2)?;
                Command::Wait(u32::from(u16::from_le_bytes([d[0], d[1]])))
            }
            0x62 => Command::Wait(735),
            0x63 => Command::Wait(882),
            0x70..=0x7F => Command::Wait(u32::from(op & 15) + 1),
            0x80..=0x8F => Command::Ym2612Dac { wait: op & 15 },
            0x67 => {
                let d = self.take(6)?;
                if d[0] != 0x66 {
                    return None;
                }
                let kind = d[1];
                let len = (u32::from_le_bytes([d[2], d[3], d[4], d[5]]) & 0x7FFF_FFFF) as usize;
                let at = self.pos;
                let len = len.min(self.file.len() - at);
                self.pos = at + len;
                Command::DataBlock { kind, at, len }
            }
            0xE0 => {
                let d = self.take(4)?;
                Command::Seek(u32::from_le_bytes([d[0], d[1], d[2], d[3]]))
            }
            0x90 => {
                let d = self.take(4)?;
                Command::StreamSetup {
                    stream: d[0],
                    chip: d[1] & 0x7F,
                    second: d[1] & 0x80 != 0,
                    port: d[2],
                    register: d[3],
                }
            }
            0x91 => {
                let d = self.take(4)?;
                Command::StreamData {
                    stream: d[0],
                    bank: d[1],
                    step: d[2],
                    base: d[3],
                }
            }
            0x92 => {
                let d = self.take(5)?;
                Command::StreamFrequency {
                    stream: d[0],
                    hz: u32::from_le_bytes([d[1], d[2], d[3], d[4]]),
                }
            }
            0x93 => {
                let d = self.take(10)?;
                Command::StreamStart {
                    stream: d[0],
                    at: u32::from_le_bytes([d[1], d[2], d[3], d[4]]),
                    mode: d[5],
                    length: u32::from_le_bytes([d[6], d[7], d[8], d[9]]),
                }
            }
            0x94 => Command::StreamStop {
                stream: self.take(1)?[0],
            },
            0x95 => {
                let d = self.take(4)?;
                Command::StreamFast {
                    stream: d[0],
                    block: u16::from_le_bytes([d[1], d[2]]),
                    flags: d[3],
                }
            }
            _ => {
                let n = match op {
                    0x00 | 0x62 | 0x63 => 0,
                    0x30..=0x3F | 0x4F | 0x50 => 1,
                    0x40..=0x4E | 0x51..=0x5F => 2,
                    0x64 => 3,
                    0x68 => 11,
                    0xA0..=0xBF => 2,
                    0xC0..=0xDF => 3,
                    0xE1..=0xFF => 4,
                    _ => return None,
                };
                self.take(n)?;
                Command::Other(op)
            }
        })
    }
}

/// The data blocks of a stream, joined into one bank for each type of PCM
/// data (0x00 to 0x3F). A compressed block (0x40 to 0x7E) is unpacked into
/// the bank of its plain type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Banks {
    banks: Vec<Vec<u8>>,
    blocks: Vec<Vec<(usize, usize)>>,
}

impl Banks {
    fn scan(vgm: &Vgm) -> Banks {
        let mut banks = vec![Vec::new(); 0x40];
        let mut blocks = vec![Vec::new(); 0x40];
        let mut tables: Vec<((u8, u8), Vec<u32>)> = Vec::new();
        let file = vgm.bytes();
        let mut cur = vgm.commands();
        loop {
            match cur.read() {
                Command::End => break,
                Command::DataBlock { kind, at, len } => {
                    let payload = &file[at..at + len];
                    match kind {
                        0x00..=0x3F => {
                            let k = usize::from(kind);
                            blocks[k].push((banks[k].len(), payload.len()));
                            banks[k].extend_from_slice(payload);
                        }
                        0x40..=0x7E => {
                            if let Some(data) = unpack(payload, &tables) {
                                let k = usize::from(kind - 0x40);
                                blocks[k].push((banks[k].len(), data.len()));
                                banks[k].extend_from_slice(&data);
                            }
                        }
                        0x7F => {
                            if let Some(t) = read_table(payload) {
                                tables.retain(|(key, _)| *key != t.0);
                                tables.push(t);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Banks { banks, blocks }
    }

    /// The bank of a PCM type. Types above 0x3F have none.
    pub fn bank(&self, kind: u8) -> &[u8] {
        self.banks
            .get(usize::from(kind))
            .map_or(&[], |b| b.as_slice())
    }

    /// The start and length in the bank of each block of the type, in file
    /// order. Command 0x95 names a block by its number here.
    pub fn blocks(&self, kind: u8) -> &[(usize, usize)] {
        self.blocks
            .get(usize::from(kind))
            .map_or(&[], |b| b.as_slice())
    }
}

fn value_bytes(bits: u8) -> usize {
    usize::from(bits).div_ceil(8).max(1)
}

fn read_table(p: &[u8]) -> Option<((u8, u8), Vec<u32>)> {
    let (tt, st, bd, count) = (
        *p.first()?,
        *p.get(1)?,
        *p.get(2)?,
        usize::from(u16::from_le_bytes([*p.get(4)?, *p.get(5)?])),
    );
    let size = value_bytes(bd).min(4);
    let body = p.get(6..)?;
    let mut values = Vec::new();
    for i in 0..count {
        let chunk = body.get(i * value_bytes(bd)..(i + 1) * value_bytes(bd))?;
        let mut v = 0u32;
        for (k, b) in chunk.iter().take(size).enumerate() {
            v |= u32::from(*b) << (8 * k);
        }
        values.push(v);
    }
    Some(((tt, st), values))
}

/// Unpacks a compressed block: the type, the size of the result, the
/// attributes, then the packed bits.
fn unpack(p: &[u8], tables: &[((u8, u8), Vec<u32>)]) -> Option<Vec<u8>> {
    let kind = *p.first()?;
    let size = u32::from_le_bytes([*p.get(1)?, *p.get(2)?, *p.get(3)?, *p.get(4)?]) as usize;
    let (bd, bc, st) = (*p.get(5)?, *p.get(6)?, *p.get(7)?);
    let add = u32::from(u16::from_le_bytes([*p.get(8)?, *p.get(9)?]));
    let data = p.get(10..)?;
    if bc == 0 || bc > 32 || bd == 0 || bd > 32 || size > MAX_FILE {
        return None;
    }
    let table = tables
        .iter()
        .find(|(key, _)| *key == (kind, st))
        .map(|(_, t)| t);
    let each = value_bytes(bd);
    let mask = if bd == 32 { u32::MAX } else { (1u32 << bd) - 1 };
    let mut out = Vec::with_capacity(size);
    let mut state = add & mask;
    let mut bit = 0usize;
    while out.len() + each <= size {
        let mut v = 0u32;
        for _ in 0..bc {
            let byte = *data.get(bit / 8)?;
            v = v << 1 | u32::from(byte >> (7 - bit % 8) & 1);
            bit += 1;
        }
        let value = match kind {
            0 => match st {
                0 => v.wrapping_add(add),
                1 => (v << (bd - bc.min(bd))).wrapping_add(add),
                _ => *table?.get(v as usize)?,
            },
            1 => {
                state = state.wrapping_add(*table?.get(v as usize)?) & mask;
                state
            }
            _ => return None,
        } & mask;
        out.extend_from_slice(&value.to_le_bytes()[..each]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version 1.71 file with a 0x100-byte header and the given stream.
    fn file(version: u32, stream: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 0x100];
        f[..4].copy_from_slice(b"Vgm ");
        f[8..12].copy_from_slice(&version.to_le_bytes());
        f[0x34..0x38].copy_from_slice(&(0x100u32 - 0x34).to_le_bytes());
        f.extend_from_slice(stream);
        let eof = (f.len() - 4) as u32;
        f[4..8].copy_from_slice(&eof.to_le_bytes());
        f
    }

    fn set(f: &mut [u8], at: usize, v: u32) {
        f[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn commands(v: &Vgm) -> Vec<Command> {
        let mut c = v.commands();
        let mut out = Vec::new();
        loop {
            match c.read() {
                Command::End => return out,
                x => out.push(x),
            }
        }
    }

    #[test]
    fn the_header_gives_the_clocks_and_the_flag_bits() {
        let mut f = file(0x171, &[0x66]);
        set(&mut f, 0x0C, 3_579_545 | 0x4000_0000);
        set(&mut f, 0x2C, 7_670_454 | 0x8000_0000);
        set(&mut f, 0x18, 44_100);
        let v = Vgm::parse(&f).unwrap();
        let psg = v.chip(SN76489).unwrap();
        assert_eq!((psg.clock, psg.dual, psg.variant), (3_579_545, true, false));
        let fm = v.chip(YM2612).unwrap();
        assert_eq!((fm.clock, fm.dual, fm.variant), (7_670_454, false, true));
        assert_eq!(v.total_samples, 44_100);
        assert_eq!(v.chips.len(), 2);
        assert_eq!(v.chips[1].name, "YM2612");
    }

    #[test]
    fn an_old_file_keeps_its_data_at_0x40_and_the_ym2612_clock_in_the_ym2413_word() {
        let mut f = vec![0u8; 0x40];
        f[..4].copy_from_slice(b"Vgm ");
        set(&mut f, 8, 0x101);
        set(&mut f, 0x10, 7_670_454);
        f.extend_from_slice(&[0x52, 0x28, 0xF0, 0x66]);
        let v = Vgm::parse(&f).unwrap();
        assert_eq!(v.data_at(), 0x40);
        assert!(v.chip(1).is_none());
        assert_eq!(v.chip(YM2612).unwrap().clock, 7_670_454);
        assert_eq!((v.psg_feedback, v.psg_width), (9, 16));
        assert_eq!(
            commands(&v),
            [Command::Ym2612 {
                second: false,
                port: 0,
                register: 0x28,
                value: 0xF0
            }]
        );
    }

    #[test]
    fn a_data_offset_inside_the_header_hides_the_fields_it_overlaps() {
        let mut f = vec![0u8; 0x80];
        f[..4].copy_from_slice(b"Vgm ");
        set(&mut f, 8, 0x150);
        set(&mut f, 0x34, 0x4C - 0x34);
        set(&mut f, 0x0C, 3_579_545);
        set(&mut f, 0x2C, 7_670_454);
        set(&mut f, 0x4C, 8_000_000);
        f[0x4C..0x50].copy_from_slice(&[0x66, 0, 0, 0]);
        let v = Vgm::parse(&f).unwrap();
        assert_eq!(v.data_at(), 0x4C);
        assert!(v.chip(8).is_none());
        assert!(v.chip(YM2612).is_some());
    }

    #[test]
    fn the_loop_point_and_the_loop_length_come_from_the_header() {
        let mut f = file(0x171, &[0x50, 0x9F, 0x62, 0x66]);
        set(&mut f, 0x1C, 0x102 - 0x1C);
        set(&mut f, 0x20, 735);
        let v = Vgm::parse(&f).unwrap();
        assert_eq!(v.loop_at(), Some(0x102));
        assert_eq!(v.loop_samples, 735);
        let mut c = v.commands_from(0x102);
        assert_eq!(c.read(), Command::Wait(735));
        let mut none = file(0x171, &[0x66]);
        set(&mut none, 0x20, 99);
        assert_eq!(Vgm::parse(&none).unwrap().loop_samples, 0);
    }

    #[test]
    fn the_commands_decode_with_their_operands_and_waits() {
        let v = Vgm::parse(&file(
            0x171,
            &[
                0x50, 0x9F, 0x4F, 0xFF, 0x53, 0xB4, 0xC0, 0xA2, 0x30, 0x01, 0x61, 0x34, 0x12, 0x62,
                0x63, 0x75, 0x8A, 0xB4, 0x00, 0x11, 0xE0, 4, 0, 0, 0, 0x31, 0x07, 0x66, 0x50, 0x00,
            ],
        ))
        .unwrap();
        assert_eq!(
            commands(&v),
            [
                Command::Psg {
                    second: false,
                    value: 0x9F
                },
                Command::PsgStereo {
                    second: false,
                    value: 0xFF
                },
                Command::Ym2612 {
                    second: false,
                    port: 1,
                    register: 0xB4,
                    value: 0xC0
                },
                Command::Ym2612 {
                    second: true,
                    port: 0,
                    register: 0x30,
                    value: 0x01
                },
                Command::Wait(0x1234),
                Command::Wait(735),
                Command::Wait(882),
                Command::Wait(6),
                Command::Ym2612Dac { wait: 10 },
                Command::Other(0xB4),
                Command::Seek(4),
                Command::Other(0x31),
            ]
        );
    }

    #[test]
    fn a_byte_that_is_not_a_command_ends_the_stream_and_marks_it_damaged() {
        let v = Vgm::parse(&file(0x171, &[0x50, 0x9F, 0x60, 0x50, 0x9F])).unwrap();
        let mut c = v.commands();
        assert_eq!(
            c.read(),
            Command::Psg {
                second: false,
                value: 0x9F
            }
        );
        assert_eq!(c.read(), Command::End);
        assert!(c.damaged());
        assert_eq!(c.read(), Command::End);
        let cut = Vgm::parse(&file(0x171, &[0x61, 0x01])).unwrap();
        let mut c = cut.commands();
        assert_eq!(c.read(), Command::End);
        assert!(c.damaged());
        let clean = Vgm::parse(&file(0x171, &[0x66])).unwrap();
        let mut c = clean.commands();
        assert_eq!(c.read(), Command::End);
        assert!(!c.damaged());
    }

    #[test]
    fn the_stream_commands_decode() {
        let v = Vgm::parse(&file(
            0x171,
            &[
                0x90, 0, 0x02, 0, 0x2A, 0x91, 0, 0, 1, 0, 0x92, 0, 0x40, 0x1F, 0, 0, 0x93, 0, 0xFF,
                0xFF, 0xFF, 0xFF, 3, 0, 0, 0, 0, 0x95, 0, 2, 0, 1, 0x94, 0xFF, 0x66,
            ],
        ))
        .unwrap();
        assert_eq!(
            commands(&v),
            [
                Command::StreamSetup {
                    stream: 0,
                    chip: 2,
                    second: false,
                    port: 0,
                    register: 0x2A
                },
                Command::StreamData {
                    stream: 0,
                    bank: 0,
                    step: 1,
                    base: 0
                },
                Command::StreamFrequency {
                    stream: 0,
                    hz: 8000
                },
                Command::StreamStart {
                    stream: 0,
                    at: u32::MAX,
                    mode: 3,
                    length: 0
                },
                Command::StreamFast {
                    stream: 0,
                    block: 2,
                    flags: 1
                },
                Command::StreamStop { stream: 0xFF },
            ]
        );
    }

    fn block(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut b = vec![0x67, 0x66, kind];
        b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        b.extend_from_slice(payload);
        b
    }

    #[test]
    fn data_blocks_join_into_a_bank_and_keep_their_starts() {
        let mut s = block(0, &[1, 2, 3]);
        s.extend(block(0, &[4, 5]));
        s.extend(block(0x80, &[9, 9]));
        s.push(0x66);
        let v = Vgm::parse(&file(0x171, &s)).unwrap();
        let banks = v.banks();
        assert_eq!(banks.bank(0), [1, 2, 3, 4, 5]);
        assert_eq!(banks.blocks(0), [(0, 3), (3, 2)]);
        assert!(banks.bank(1).is_empty());
        assert_eq!(banks.bank(0x80), [] as [u8; 0]);
    }

    #[test]
    fn a_data_block_longer_than_the_file_is_cut() {
        let mut s = vec![0x67, 0x66, 0x00];
        s.extend_from_slice(&100u32.to_le_bytes());
        s.extend_from_slice(&[7, 7]);
        let v = Vgm::parse(&file(0x171, &s)).unwrap();
        assert_eq!(v.banks().bank(0), [7, 7]);
    }

    #[test]
    fn a_compressed_block_unpacks_with_a_table_and_with_a_shift() {
        // 4 bits per value, shifted left by 4 into 8 bits.
        let shift = [0u8, 4, 0, 0, 0, 8, 4, 1, 0, 0, 0x12, 0xF0];
        let mut s = block(0x40, &shift);
        // 2 bits per value into 8 bits through a table.
        let table = [0u8, 2, 8, 2, 4, 0, 10, 20, 30, 40];
        s.extend(block(0x7F, &table));
        let coded = [0u8, 4, 0, 0, 0, 8, 2, 2, 0, 0, 0b0001_1011];
        s.extend(block(0x41, &coded));
        let v = Vgm::parse(&file(0x171, &s)).unwrap();
        let banks = v.banks();
        assert_eq!(banks.bank(0), [0x10, 0x20, 0xF0, 0x00]);
        assert_eq!(banks.bank(1), [10, 20, 30, 40]);
    }

    #[test]
    fn a_dpcm_block_adds_each_delta_to_the_running_value() {
        let table = [1u8, 0, 8, 1, 4, 0, 1, 255, 5, 251];
        let mut s = block(0x7F, &table);
        // 2 bits per value, start value 100, indexes 0 (+1), 2 (+5), 3 (-5), 1 (-1).
        let packed = [1u8, 4, 0, 0, 0, 8, 2, 0, 100, 0, 0b0010_1101];
        s.extend(block(0x42, &packed));
        let v = Vgm::parse(&file(0x171, &s)).unwrap();
        assert_eq!(v.banks().bank(2), [101, 106, 101, 100]);
    }

    #[test]
    fn the_tag_reads_in_order_and_keeps_the_japanese_text() {
        let mut gd3 = Vec::new();
        for s in [
            "Song",
            "曲",
            "Game",
            "ゲーム",
            "Genesis",
            "メガドライブ",
            "Author",
            "作者",
            "1990/01/02",
            "Ripper",
            "Notes",
        ] {
            for u in s.encode_utf16() {
                gd3.extend_from_slice(&u.to_le_bytes());
            }
            gd3.extend_from_slice(&[0, 0]);
        }
        let mut f = file(0x171, &[0x66]);
        let at = f.len();
        f.extend_from_slice(b"Gd3 ");
        f.extend_from_slice(&0x100u32.to_le_bytes());
        f.extend_from_slice(&(gd3.len() as u32).to_le_bytes());
        f.extend_from_slice(&gd3);
        set(&mut f, 0x14, (at - 0x14) as u32);
        let t = Vgm::parse(&f).unwrap().gd3.unwrap();
        assert_eq!((t.title.as_str(), t.title_jp.as_str()), ("Song", "曲"));
        assert_eq!((t.game.as_str(), t.system.as_str()), ("Game", "Genesis"));
        assert_eq!(
            (t.author.as_str(), t.date.as_str()),
            ("Author", "1990/01/02")
        );
        assert_eq!((t.ripper.as_str(), t.notes.as_str()), ("Ripper", "Notes"));
    }

    #[test]
    fn a_short_tag_leaves_the_rest_empty() {
        let mut f = file(0x171, &[0x66]);
        let at = f.len();
        f.extend_from_slice(b"Gd3 ");
        f.extend_from_slice(&0x100u32.to_le_bytes());
        f.extend_from_slice(&4u32.to_le_bytes());
        f.extend_from_slice(&[b'A', 0, 0, 0]);
        set(&mut f, 0x14, (at - 0x14) as u32);
        let t = Vgm::parse(&f).unwrap().gd3.unwrap();
        assert_eq!(t.title, "A");
        assert_eq!(t.notes, "");
    }

    #[test]
    fn the_volume_modifier_and_the_loop_count_follow_the_header() {
        let mut f = file(0x171, &[0x66]);
        f[0x7C] = 0x20;
        f[0x7E] = 1;
        f[0x7F] = 0x20;
        let v = Vgm::parse(&f).unwrap();
        assert!((v.gain() - 2.0).abs() < 1e-12);
        assert_eq!(v.loop_count(2), 3);
        f[0x7C] = 0xC1;
        f[0x7E] = 0;
        f[0x7F] = 0;
        let v = Vgm::parse(&f).unwrap();
        assert!((v.gain() - 0.25).abs() < 1e-12);
        assert_eq!(v.loop_count(2), 2);
    }

    #[test]
    fn the_extra_header_gives_the_clock_of_a_second_chip() {
        let mut f = file(0x171, &[0x66]);
        set(&mut f, 0x2C, 7_670_454 | 0x4000_0000);
        // The extra header sits after the stream: size, clock offset, volume offset, then one entry.
        let at = f.len();
        f.extend_from_slice(&[
            12, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 1, 2, 0x00, 0x2D, 0x6C, 0x00,
        ]);
        set(&mut f, 0xBC, (at - 0xBC) as u32);
        let v = Vgm::parse(&f).unwrap();
        let fm = v.chip(YM2612).unwrap();
        assert_eq!(fm.clock, 7_670_454);
        assert_eq!(fm.second_clock, 0x006C_2D00);
    }

    #[test]
    fn a_gzip_file_unpacks_like_the_plain_one() {
        let plain = file(0x171, &[0x50, 0x9F, 0x62, 0x66]);
        // A stored (uncompressed) deflate block inside a gzip frame with a name field.
        let mut z = vec![0x1F, 0x8B, 8, 8, 0, 0, 0, 0, 0, 3];
        z.extend_from_slice(b"a.vgm\0");
        z.push(1);
        z.extend_from_slice(&(plain.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(plain.len() as u16)).to_le_bytes());
        z.extend_from_slice(&plain);
        z.extend_from_slice(&[0; 8]);
        assert_eq!(Vgm::parse(&z).unwrap(), Vgm::parse(&plain).unwrap());
        z.truncate(20);
        assert_eq!(Vgm::parse(&z), Err(VgmError::Stream));
    }

    #[test]
    fn a_file_with_the_wrong_magic_or_a_bad_offset_is_refused() {
        assert_eq!(Vgm::parse(&[0; 10]), Err(VgmError::NotVgm));
        let mut f = file(0x171, &[0x66]);
        f[0] = b'X';
        assert_eq!(Vgm::parse(&f), Err(VgmError::NotVgm));
        let mut g = file(0x171, &[0x66]);
        set(&mut g, 0x34, 0x10000);
        assert_eq!(Vgm::parse(&g), Err(VgmError::NotVgm));
        let mut h = file(0x171, &[0x66]);
        set(&mut h, 0x34, 0);
        set(&mut h, 8, 0x150);
        assert_eq!(Vgm::parse(&h).unwrap().data_at(), 0x40);
    }
}
