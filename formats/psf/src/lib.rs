//! The PSF container: a header, a zlib-compressed program and optional
//! "[TAG]" text of key=value lines. Cores read the program in their own way.

use flate2::{Decompress, FlushDecompress, Status};

pub const MAX_TAGS: usize = 64;
pub const MAX_PROGRAM: usize = 64 << 20;

/// The bytes of a C string: everything before the first NUL.
pub fn cstr(bytes: &[u8]) -> &[u8] {
    match bytes.iter().position(|&b| b == 0) {
        Some(n) => &bytes[..n],
        None => bytes,
    }
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = s {
        if *first > b' ' {
            break;
        }
        s = rest;
    }
    while let [rest @ .., last] = s {
        if *last > b' ' {
            break;
        }
        s = rest;
    }
    s
}

/// A little-endian word. The slice must hold at least four bytes.
pub fn le32(p: &[u8]) -> u32 {
    u32::from_le_bytes([p[0], p[1], p[2], p[3]])
}

pub struct Container<'a> {
    /// The compressed program.
    pub program: &'a [u8],
    /// Everything after the program, where the tag text starts.
    pub tags: &'a [u8],
}

#[derive(Debug, PartialEq, Eq)]
pub enum ContainerError {
    /// Too short, wrong magic or wrong version byte.
    NotPsf,
    /// The reserved area and program run past the end of the file.
    PastEnd,
}

/// Splits a PSF file of the given version byte (0x22 for GSF).
pub fn open(file: &[u8], version: u8) -> Result<Container<'_>, ContainerError> {
    if file.len() < 16 || &file[..3] != b"PSF" || file[3] != version {
        return Err(ContainerError::NotPsf);
    }
    let reserved = u64::from(le32(&file[4..]));
    let packed = u64::from(le32(&file[8..]));
    if 16 + reserved + packed > file.len() as u64 {
        return Err(ContainerError::PastEnd);
    }
    let program_at = 16 + reserved as usize;
    let tags_at = program_at + packed as usize;
    Ok(Container {
        program: &file[program_at..tags_at],
        tags: &file[tags_at..],
    })
}

/// Parses "[TAG]" text. A key that repeats joins its values with newlines.
pub fn parse_tags(p: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    if p.len() < 5 || &p[..5] != b"[TAG]" {
        return out;
    }
    let body = &p[5..];
    let mut s = 0;
    while s < body.len() {
        let eol = body[s..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(body.len(), |i| s + i);
        let line = &body[s..eol];
        if let Some(eq) = line.iter().position(|&b| b == b'=') {
            let key = trim(&line[..eq]);
            let value = cstr(trim(&line[eq + 1..]));
            if !key.is_empty() {
                let found = out
                    .iter()
                    .position(|(k, _)| k.as_slice() == key && !key.contains(&0));
                if let Some(i) = found {
                    out[i].1.push(b'\n');
                    out[i].1.extend_from_slice(value);
                } else if out.len() < MAX_TAGS {
                    out.push((key.to_vec(), value.to_vec()));
                }
            }
        }
        s = eol + 1;
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum InflateError {
    /// The program is larger than MAX_PROGRAM.
    TooLarge,
    /// The zlib stream is damaged or incomplete.
    Stream,
}

pub fn inflate(src: &[u8]) -> Result<Vec<u8>, InflateError> {
    let mut cap = src
        .len()
        .saturating_mul(4)
        .saturating_add(1024)
        .min(MAX_PROGRAM);
    let mut buf = vec![0u8; cap];
    let mut len = 0;
    let mut z = Decompress::new(true);
    loop {
        if len == cap {
            if cap >= MAX_PROGRAM {
                return Err(InflateError::TooLarge);
            }
            cap *= 2;
            buf.resize(cap, 0);
        }
        let consumed = z.total_in() as usize;
        let status = z
            .decompress(&src[consumed..], &mut buf[len..], FlushDecompress::None)
            .map_err(|_| InflateError::Stream)?;
        len = z.total_out() as usize;
        match status {
            Status::Ok => {}
            Status::StreamEnd => break,
            Status::BufError => return Err(InflateError::Stream),
        }
    }
    buf.truncate(len);
    Ok(buf)
}

/// "75", "75.5", "1:15", "1:15.500", "0:01:15" and a comma for the point, in
/// milliseconds, or -1 when absent or unreadable.
pub fn parse_time_ms(s: Option<&[u8]>) -> i32 {
    let Some(s) = s else { return -1 };
    let (mut total, mut part, mut frac, mut scale) = (0.0f64, 0.0f64, 0.0f64, 0.1f64);
    let (mut seen, mut in_frac) = (false, false);
    for &ch in s {
        match ch {
            b'0'..=b'9' => {
                let digit = f64::from(ch - b'0');
                if in_frac {
                    frac += digit * scale;
                    scale /= 10.0;
                } else {
                    part = part * 10.0 + digit;
                }
                seen = true;
            }
            b':' => {
                if in_frac {
                    return -1;
                }
                total = (total + part) * 60.0;
                part = 0.0;
            }
            b'.' | b',' => {
                if in_frac {
                    return -1;
                }
                in_frac = true;
            }
            b' ' | b'\t' => {}
            _ => break,
        }
    }
    if !seen {
        return -1;
    }
    let ms = (total + part + frac) * 1000.0;
    if ms > 2147483647.0 {
        return -1;
    }
    (ms + 0.5) as i32
}
