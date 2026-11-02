//! Images: what a machine runs. A GSF set (a stub file plus its libraries)
//! or a bare cartridge ROM.

use oscilla_psf as psf;
use std::ffi::{CStr, CString};

/// Status codes, with the values `cgba_status` has in `cgba.h`.
pub mod status {
    pub const OK: i32 = 0;
    pub const IO: i32 = 1;
    pub const FORMAT: i32 = 2;
    pub const ZLIB: i32 = 3;
    pub const MEMORY: i32 = 4;
    pub const UNSUPPORTED: i32 = 5;
    pub const LIBRARY: i32 = 6;
}
use status::*;

const MAX_DEPTH: i32 = 10;
/// The limit counts every file that the chain opens as well as its depth, so
/// a wide chain of small files cannot fan out.
const MAX_FILES: i32 = 64;
pub const ROM_MAX: u32 = 0x0200_0000;
pub const EWRAM_SIZE: u32 = 0x0004_0000;
const PATH_MAX_BYTES: usize = 4095;
const GSF_VERSION: u8 = 0x22;

struct Tag {
    key: Vec<u8>,
    value: CString,
    depth: i32,
}

pub struct Image {
    rom: Vec<u8>,
    ewram: Option<Vec<u8>>,
    entry: u32,
    entry_set: bool,
    tags: Vec<Tag>,
}

/// Why a load failed: a status code and a sentence for the caller.
#[derive(Debug)]
pub struct LoadError {
    pub status: i32,
    pub message: Vec<u8>,
}

struct Load {
    opened: i32,
    error: Vec<u8>,
}

impl Load {
    fn fail(&mut self, status: i32, head: &str, tail: &[u8]) -> i32 {
        self.error.clear();
        self.error.extend_from_slice(head.as_bytes());
        self.error.extend_from_slice(tail);
        status
    }
}

/// Reads the file at a path that arrived as the bytes of a C string. Unix
/// paths are raw bytes. Paths on every other platform come in as UTF-8.
#[cfg(unix)]
fn read_path(path: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    std::fs::read(std::ffi::OsStr::from_bytes(path))
}

#[cfg(not(unix))]
fn read_path(path: &[u8]) -> std::io::Result<Vec<u8>> {
    match std::str::from_utf8(path) {
        Ok(text) => std::fs::read(text),
        Err(_) => Err(std::io::ErrorKind::InvalidInput.into()),
    }
}

#[cfg(unix)]
fn is_separator(b: u8) -> bool {
    b == b'/'
}

#[cfg(not(unix))]
fn is_separator(b: u8) -> bool {
    b == b'/' || b == b'\\'
}

fn join_dir(path: &[u8], name: &[u8]) -> Vec<u8> {
    let dir = path
        .iter()
        .rposition(|&b| is_separator(b))
        .map_or(0, |i| i + 1);
    let mut out = path[..dir].to_vec();
    out.extend_from_slice(name);
    out.truncate(PATH_MAX_BYTES);
    out
}

/// A library is a plain file name in the folder of the file that names it.
fn plain_name(name: &[u8]) -> bool {
    if name.is_empty() || name == b"." || name == b".." {
        return false;
    }
    !name.contains(&b'/') && !name.contains(&b'\\')
}

impl Image {
    fn empty() -> Image {
        Image {
            rom: Vec::new(),
            ewram: None,
            entry: 0,
            entry_set: false,
            tags: Vec::new(),
        }
    }

    /// Loads a .gsf or .minigsf and every library it names (_lib, _lib2, ...,
    /// from the same directory). Libraries load first, depth first, and each
    /// file's own data lands on top of them.
    pub fn load_gsf(path: &[u8]) -> Result<Image, LoadError> {
        let mut img = Image::empty();
        let mut ctx = Load {
            opened: 0,
            error: Vec::new(),
        };
        let mut rc = img.load_psf(path, 0, &mut ctx);
        if rc == OK && img.rom.is_empty() && img.ewram.is_none() {
            rc = ctx.fail(FORMAT, "The set holds no program: ", path);
        }
        if rc != OK {
            return Err(LoadError {
                status: rc,
                message: ctx.error,
            });
        }
        if !img.entry_set {
            img.entry = if img.rom.is_empty() {
                0x0200_0000
            } else {
                0x0800_0000
            };
        }
        Ok(img)
    }

    /// A bare cartridge image, loaded at 0x08000000. The bytes are copied.
    pub fn from_rom(rom: &[u8]) -> Result<Image, i32> {
        if rom.is_empty() || rom.len() > ROM_MAX as usize {
            return Err(FORMAT);
        }
        let mut img = Image::empty();
        img.rom = rom.to_vec();
        img.entry = 0x0800_0000;
        img.entry_set = true;
        Ok(img)
    }

    pub fn entry(&self) -> u32 {
        self.entry
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    pub fn ewram(&self) -> Option<&[u8]> {
        self.ewram.as_deref()
    }

    /// A tag from the GSF set, or None. The stub's own tags come first; a
    /// library's tag fills in a missing one. Keys match without regard to case.
    pub fn tag(&self, key: &[u8]) -> Option<&CStr> {
        self.find_tag(key).map(|i| self.tags[i].value.as_c_str())
    }

    /// The length tag in milliseconds, or -1 when absent or unreadable.
    pub fn length_ms(&self) -> i32 {
        psf::parse_time_ms(self.tag(b"length").map(CStr::to_bytes))
    }

    /// The fade tag in milliseconds, or -1 when absent or unreadable.
    pub fn fade_ms(&self) -> i32 {
        psf::parse_time_ms(self.tag(b"fade").map(CStr::to_bytes))
    }

    fn find_tag(&self, key: &[u8]) -> Option<usize> {
        self.tags
            .iter()
            .position(|t| psf::cstr(&t.key).eq_ignore_ascii_case(key))
    }

    fn place_program(&mut self, prog: &[u8]) -> i32 {
        if prog.len() < 12 {
            return FORMAT;
        }
        let entry = psf::le32(prog);
        let offset = psf::le32(&prog[4..]);
        let mut size = psf::le32(&prog[8..]) as usize;
        if size > prog.len() - 12 {
            size = prog.len() - 12;
        }
        let data = &prog[12..12 + size];
        let region = offset >> 24;
        if (0x08..=0x0D).contains(&region) {
            let at = (offset & (ROM_MAX - 1)) as usize;
            if at + size > ROM_MAX as usize {
                return FORMAT;
            }
            let end = at + size;
            if end > self.rom.len() {
                self.rom.resize(end, 0);
            }
            self.rom[at..end].copy_from_slice(data);
        } else if region == 0x02 {
            let at = (offset & (EWRAM_SIZE - 1)) as usize;
            if at + size > EWRAM_SIZE as usize {
                return FORMAT;
            }
            let ew = self
                .ewram
                .get_or_insert_with(|| vec![0u8; EWRAM_SIZE as usize]);
            ew[at..at + size].copy_from_slice(data);
        } else {
            return UNSUPPORTED;
        }
        if !self.entry_set {
            self.entry = entry;
            self.entry_set = true;
        }
        OK
    }

    fn load_psf(&mut self, path: &[u8], depth: i32, ctx: &mut Load) -> i32 {
        if depth > MAX_DEPTH {
            return ctx.fail(LIBRARY, "Libraries nest too deeply at ", path);
        }
        ctx.opened += 1;
        if ctx.opened > MAX_FILES {
            return ctx.fail(LIBRARY, "The set opens too many files at ", path);
        }
        let file = match read_path(path) {
            Ok(f) => f,
            Err(_) => {
                let status = if depth > 0 { LIBRARY } else { IO };
                return ctx.fail(status, "Could not read ", path);
            }
        };
        let container = match psf::open(&file, GSF_VERSION) {
            Ok(c) => c,
            Err(psf::ContainerError::NotPsf) => {
                return ctx.fail(FORMAT, "Not a GSF file: ", path);
            }
            Err(psf::ContainerError::PastEnd) => {
                return ctx.fail(FORMAT, "The program runs past the end of ", path);
            }
        };
        let mut tags = psf::parse_tags(container.tags);

        let mut rc = OK;
        for k in 1..10 {
            let key = if k == 1 {
                "_lib".to_string()
            } else {
                format!("_lib{k}")
            };
            let name = tags
                .iter()
                .find(|(t, _)| psf::cstr(t) == key.as_bytes())
                .map(|(_, v)| v.clone());
            let Some(name) = name else {
                if k == 1 {
                    continue;
                }
                break;
            };
            if !plain_name(&name) {
                rc = ctx.fail(
                    LIBRARY,
                    "A library must be a plain file name in the same folder: ",
                    &name,
                );
                break;
            }
            rc = self.load_psf(&join_dir(path, &name), depth + 1, ctx);
            if rc != OK {
                break;
            }
        }
        if rc == OK && !container.program.is_empty() {
            match psf::inflate(container.program) {
                Ok(prog) => {
                    rc = self.place_program(&prog);
                    if rc != OK {
                        ctx.fail(rc, "The program has an unusable load address in ", path);
                    }
                }
                Err(e) => {
                    let status = match e {
                        psf::InflateError::TooLarge => FORMAT,
                        psf::InflateError::Stream => ZLIB,
                    };
                    rc = ctx.fail(status, "The program does not inflate in ", path);
                }
            }
        }
        if rc == OK {
            // A nearer file's tags win, so a library only fills gaps. The
            // library-naming tags themselves are not kept.
            for (key, value) in tags.drain(..) {
                if key.first() == Some(&b'_') {
                    continue;
                }
                let value = CString::new(value).unwrap_or_default();
                if let Some(at) = self.find_tag(psf::cstr(&key)) {
                    if depth < self.tags[at].depth {
                        self.tags[at].value = value;
                        self.tags[at].depth = depth;
                    }
                    continue;
                }
                if self.tags.len() < psf::MAX_TAGS {
                    self.tags.push(Tag { key, value, depth });
                }
            }
        }
        rc
    }
}
