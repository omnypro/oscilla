//! The GYM container: a log of the writes to a Mega Drive's YM2612 and
//! SN76489, one frame of the console at a time. A file can start with a
//! 428-byte "GYMX" header, and its data can be zlib compressed. `Gym::to_vgm`
//! rewrites the log as a VGM file so that a VGM player can run it.

use flate2::{Decompress, FlushDecompress, Status};

/// A file larger than this, packed or unpacked, is refused.
pub const MAX_FILE: usize = 64 << 20;

const HEADER: usize = 428;

/// The clocks of the console's chips. The YM2612 runs at the 68000's clock
/// (the master clock over 7) and the SN76489 at the Z80's (over 15).
const NTSC: (u32, u32) = (7_670_454, 3_579_545);
const PAL: (u32, u32) = (7_600_489, 3_546_895);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GymError {
    /// The file has no commands.
    Empty,
    /// The file is larger than MAX_FILE.
    TooLarge,
    /// The zlib data is damaged or incomplete.
    Stream,
    /// A byte where a command should start is not 0 to 3. The offset counts
    /// from the start of the data.
    Command(usize),
}

impl std::fmt::Display for GymError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GymError::Empty => write!(f, "Not a GYM file."),
            GymError::TooLarge => write!(f, "The file is larger than 64 MiB."),
            GymError::Stream => write!(f, "The zlib data is damaged."),
            GymError::Command(at) => write!(f, "Not a GYM command at byte {at}."),
        }
    }
}

impl std::error::Error for GymError {}

/// A parsed file. The text fields are empty when the file has no header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gym {
    pub song: String,
    pub game: String,
    pub publisher: String,
    pub emulator: String,
    pub dumper: String,
    pub comment: String,
    /// The frame where the loop starts, or 0 when the song does not loop.
    pub loop_start: u32,
    data: Vec<u8>,
    frames: u32,
}

/// The text of a fixed-size field: up to the first zero byte, without the
/// padding spaces at the end.
fn field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end])
        .trim_end()
        .to_string()
}

fn inflate(src: &[u8]) -> Result<Vec<u8>, GymError> {
    let mut cap = src
        .len()
        .saturating_mul(4)
        .saturating_add(1024)
        .min(MAX_FILE);
    let mut buf = vec![0u8; cap];
    let mut len = 0;
    let mut z = Decompress::new(true);
    loop {
        if len == cap {
            if cap >= MAX_FILE {
                return Err(GymError::TooLarge);
            }
            cap = (cap * 2).min(MAX_FILE);
            buf.resize(cap, 0);
        }
        let consumed = z.total_in() as usize;
        let status = z
            .decompress(&src[consumed..], &mut buf[len..], FlushDecompress::None)
            .map_err(|_| GymError::Stream)?;
        len = z.total_out() as usize;
        match status {
            Status::Ok => {}
            Status::StreamEnd => break,
            Status::BufError => return Err(GymError::Stream),
        }
    }
    buf.truncate(len);
    Ok(buf)
}

impl Gym {
    /// Reads a GYM file. A command that the file ends in the middle of is
    /// dropped.
    pub fn parse(bytes: &[u8]) -> Result<Gym, GymError> {
        if bytes.len() > MAX_FILE {
            return Err(GymError::TooLarge);
        }
        let mut gym = Gym {
            song: String::new(),
            game: String::new(),
            publisher: String::new(),
            emulator: String::new(),
            dumper: String::new(),
            comment: String::new(),
            loop_start: 0,
            data: Vec::new(),
            frames: 0,
        };
        let mut body = bytes;
        let mut packed = false;
        if bytes.len() >= HEADER && bytes.starts_with(b"GYMX") {
            let text = |at: usize, len: usize| field(&bytes[at..at + len]);
            gym.song = text(4, 32);
            gym.game = text(36, 32);
            gym.publisher = text(68, 32);
            gym.emulator = text(100, 32);
            gym.dumper = text(132, 32);
            gym.comment = text(164, 256);
            let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            gym.loop_start = word(420);
            packed = word(424) != 0;
            body = &bytes[HEADER..];
        }
        gym.data = if packed {
            inflate(body)?
        } else {
            body.to_vec()
        };
        let mut at = 0;
        let mut frames: u32 = 0;
        let mut commands = 0usize;
        while at < gym.data.len() {
            let size = match gym.data[at] {
                0 => {
                    frames = frames.saturating_add(1);
                    1
                }
                1 | 2 => 3,
                3 => 2,
                _ => return Err(GymError::Command(at)),
            };
            if at + size > gym.data.len() {
                gym.data.truncate(at);
                break;
            }
            at += size;
            commands += 1;
        }
        if commands == 0 {
            return Err(GymError::Empty);
        }
        gym.frames = frames;
        if gym.loop_start >= frames {
            gym.loop_start = 0;
        }
        Ok(gym)
    }

    /// The length of the song in frames.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The song as a version 1.50 VGM file with a YM2612 and an SN76489. The
    /// file does not say which region it came from, so `pal` chooses the
    /// frame length (1/50 s or 1/60 s) and the chips' clocks.
    pub fn to_vgm(&self, pal: bool) -> Vec<u8> {
        let (fm_clock, psg_clock) = if pal { PAL } else { NTSC };
        let (rate, per_frame) = if pal { (50u32, 882u64) } else { (60, 735) };
        let wait = if pal { 0x63 } else { 0x62 };

        let mut f = vec![0u8; 0x40];
        let mut loop_at = None;
        let mut frame = 0;
        let mut at = 0;
        while at < self.data.len() {
            match self.data[at] {
                0 => {
                    f.push(wait);
                    frame += 1;
                    at += 1;
                    if self.loop_start != 0 && frame == self.loop_start {
                        loop_at = Some(f.len());
                    }
                }
                op @ (1 | 2) => {
                    f.extend_from_slice(&[0x51 + op, self.data[at + 1], self.data[at + 2]]);
                    at += 3;
                }
                _ => {
                    f.extend_from_slice(&[0x50, self.data[at + 1]]);
                    at += 2;
                }
            }
        }
        f.push(0x66);

        let samples = |n: u64| u32::try_from(n * per_frame).unwrap_or(u32::MAX);
        let mut gd3 = Vec::new();
        let fields = [
            &self.song,
            "",
            &self.game,
            "",
            "Sega Mega Drive / Genesis",
            "",
            "",
            "",
            "",
            &self.dumper,
            &self.comment,
        ];
        if fields.iter().any(|t| !t.is_empty()) {
            for text in fields {
                for unit in text.encode_utf16().chain([0]) {
                    gd3.extend_from_slice(&unit.to_le_bytes());
                }
            }
            let at = f.len();
            f[0x14..0x18].copy_from_slice(&((at - 0x14) as u32).to_le_bytes());
            f.extend_from_slice(b"Gd3 ");
            f.extend_from_slice(&0x100u32.to_le_bytes());
            f.extend_from_slice(&(gd3.len() as u32).to_le_bytes());
            f.extend_from_slice(&gd3);
        }

        f[..4].copy_from_slice(b"Vgm ");
        f[8..12].copy_from_slice(&0x150u32.to_le_bytes());
        f[0x0C..0x10].copy_from_slice(&psg_clock.to_le_bytes());
        f[0x18..0x1C].copy_from_slice(&samples(u64::from(self.frames)).to_le_bytes());
        if let Some(at) = loop_at {
            f[0x1C..0x20].copy_from_slice(&((at - 0x1C) as u32).to_le_bytes());
            let looped = u64::from(self.frames - self.loop_start);
            f[0x20..0x24].copy_from_slice(&samples(looped).to_le_bytes());
        }
        f[0x24..0x28].copy_from_slice(&rate.to_le_bytes());
        f[0x28..0x2A].copy_from_slice(&9u16.to_le_bytes());
        f[0x2A] = 16;
        f[0x2C..0x30].copy_from_slice(&fm_clock.to_le_bytes());
        f[0x34..0x38].copy_from_slice(&(0x40u32 - 0x34).to_le_bytes());
        let eof = (f.len() - 4) as u32;
        f[4..8].copy_from_slice(&eof.to_le_bytes());
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use oscilla_vgm::{Command, SN76489, Vgm, YM2612};
    use std::io::Write;

    fn header(song: &str, loop_start: u32, packed: u32) -> Vec<u8> {
        let mut h = vec![0u8; HEADER];
        h[..4].copy_from_slice(b"GYMX");
        h[4..4 + song.len()].copy_from_slice(song.as_bytes());
        h[36..40].copy_from_slice(b"Game");
        h[164..171].copy_from_slice(b"a note.");
        h[420..424].copy_from_slice(&loop_start.to_le_bytes());
        h[424..428].copy_from_slice(&packed.to_le_bytes());
        h
    }

    const LOG: [u8; 12] = [
        0x01, 0x28, 0xF0, 0x00, 0x03, 0x9F, 0x02, 0xA4, 0x22, 0x00, 0x00, 0x03,
    ];

    #[test]
    fn a_log_without_a_header_counts_its_frames() {
        let g = Gym::parse(&LOG[..11]).unwrap();
        assert_eq!(g.frames(), 3);
        assert_eq!(g.loop_start, 0);
        assert_eq!(g.song, "");
    }

    #[test]
    fn a_command_cut_off_at_the_end_is_dropped() {
        let g = Gym::parse(&LOG).unwrap();
        assert_eq!(g.frames(), 3);
        assert_eq!(g.data.len(), 11);
    }

    #[test]
    fn a_header_gives_the_tag_and_the_loop() {
        let mut f = header("Song", 2, 0);
        f.extend_from_slice(&LOG[..11]);
        let g = Gym::parse(&f).unwrap();
        assert_eq!(
            (g.song.as_str(), g.game.as_str(), g.comment.as_str()),
            ("Song", "Game", "a note.")
        );
        assert_eq!(g.loop_start, 2);
        assert_eq!(g.frames(), 3);
    }

    #[test]
    fn a_loop_at_or_past_the_end_is_no_loop() {
        let mut f = header("", 3, 0);
        f.extend_from_slice(&LOG[..11]);
        assert_eq!(Gym::parse(&f).unwrap().loop_start, 0);
    }

    #[test]
    fn zlib_data_unpacks() {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(&LOG[..11]).unwrap();
        let packed = z.finish().unwrap();
        let mut f = header("", 0, packed.len() as u32);
        f.extend_from_slice(&packed);
        assert_eq!(Gym::parse(&f).unwrap().frames(), 3);
        f.truncate(f.len() - 4);
        assert_eq!(Gym::parse(&f), Err(GymError::Stream));
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(Gym::parse(&[]), Err(GymError::Empty));
        assert_eq!(Gym::parse(&[0x01, 0x28]), Err(GymError::Empty));
        assert_eq!(Gym::parse(&[0x00, 0x00, 0x07]), Err(GymError::Command(2)));
        assert_eq!(Gym::parse(&vec![0; MAX_FILE + 1]), Err(GymError::TooLarge));
    }

    #[test]
    fn the_vgm_has_the_same_writes_and_waits() {
        let g = Gym::parse(&LOG[..11]).unwrap();
        let vgm = Vgm::parse(&g.to_vgm(false)).unwrap();
        assert_eq!(vgm.chip(YM2612).unwrap().clock, 7_670_454);
        assert_eq!(vgm.chip(SN76489).unwrap().clock, 3_579_545);
        assert_eq!((vgm.total_samples, vgm.rate), (3 * 735, 60));
        let mut c = vgm.commands();
        let mut got = Vec::new();
        loop {
            match c.read() {
                Command::End => break,
                other => got.push(other),
            }
        }
        assert_eq!(got.len(), 6);
        assert!(matches!(
            got[0],
            Command::Ym2612 {
                second: false,
                port: 0,
                register: 0x28,
                value: 0xF0
            }
        ));
        assert!(matches!(got[1], Command::Wait(735)));
        assert!(matches!(
            got[2],
            Command::Psg {
                second: false,
                value: 0x9F
            }
        ));
        assert!(matches!(
            got[3],
            Command::Ym2612 {
                port: 1,
                register: 0xA4,
                value: 0x22,
                ..
            }
        ));
    }

    #[test]
    fn the_loop_starts_after_the_frames_before_it() {
        let mut f = header("Song", 1, 0);
        f.extend_from_slice(&LOG[..11]);
        let vgm = Vgm::parse(&Gym::parse(&f).unwrap().to_vgm(true)).unwrap();
        assert_eq!((vgm.total_samples, vgm.loop_samples), (3 * 882, 2 * 882));
        assert_eq!(vgm.rate, 50);
        assert_eq!(vgm.chip(YM2612).unwrap().clock, 7_600_489);
        // The first frame of the log is three commands, then its wait.
        let at = vgm.loop_at().unwrap();
        assert_eq!(vgm.bytes()[at - 1], 0x63);
        assert_eq!(vgm.bytes()[at], 0x50);
        let tag = vgm.gd3.unwrap();
        assert_eq!((tag.title.as_str(), tag.game.as_str()), ("Song", "Game"));
        assert_eq!(tag.notes, "a note.");
    }
}
