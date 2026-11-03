//! Helpers that the suites share: tiny programs built from bytes, and a
//! recorder for what a driver does.

#![allow(dead_code)]

use oscilla_nes::{Machine, Observer};
use oscilla_nsf::Nsf;
use std::cell::RefCell;
use std::rc::Rc;

pub const LOAD: u16 = 0x8000;
pub const INIT: u16 = 0x8000;
pub const PLAY: u16 = 0x8400;

/// `lda #v; sta address`.
pub fn set(address: u16, v: u8) -> Vec<u8> {
    vec![0xA9, v, 0x8D, address as u8, (address >> 8) as u8]
}

/// The bytes of an NSF file whose init and play routines are `init` and `play`
/// (each ends with RTS). Data after `play` is appended at the offset `extra_at`
/// from the load address.
pub fn file_with(
    init: &[u8],
    play: &[u8],
    us: u16,
    region: u8,
    banks: [u8; 8],
    extra: Option<(usize, &[u8])>,
) -> Vec<u8> {
    let mut data = vec![0u8; usize::from(PLAY - LOAD)];
    data[..init.len()].copy_from_slice(init);
    data[init.len()] = 0x60;
    data.extend_from_slice(play);
    data.push(0x60);
    if let Some((at, bytes)) = extra {
        if data.len() < at + bytes.len() {
            data.resize(at + bytes.len(), 0);
        }
        data[at..at + bytes.len()].copy_from_slice(bytes);
    }
    let mut f = vec![0u8; 0x80];
    f[..5].copy_from_slice(b"NESM\x1A");
    f[5] = 1;
    f[6] = 4;
    f[7] = 1;
    f[8..10].copy_from_slice(&LOAD.to_le_bytes());
    f[0x0A..0x0C].copy_from_slice(&INIT.to_le_bytes());
    f[0x0C..0x0E].copy_from_slice(&PLAY.to_le_bytes());
    f[0x0E..0x12].copy_from_slice(b"Test");
    f[0x6E..0x70].copy_from_slice(&us.to_le_bytes());
    f[0x70..0x78].copy_from_slice(&banks);
    f[0x78..0x7A].copy_from_slice(&20000u16.to_le_bytes());
    f[0x7A] = region;
    f.extend_from_slice(&data);
    f
}

pub fn file(init: &[u8], play: &[u8]) -> Vec<u8> {
    file_with(init, play, 16666, 0, [0; 8], None)
}

pub fn nsf(init: &[u8], play: &[u8]) -> Nsf {
    Nsf::parse(&file(init, play)).unwrap()
}

pub fn machine(init: &[u8], play: &[u8]) -> Machine {
    Machine::new(&nsf(init, play), 48000).unwrap()
}

/// What the observer saw.
#[derive(Default)]
pub struct Seen {
    pub writes: Vec<(u64, u16, u8)>,
    pub calls: Vec<(u64, i64)>,
}

pub struct Recorder(pub Rc<RefCell<Seen>>);

impl Observer for Recorder {
    fn sound_write(&mut self, cycle: u64, address: u16, value: u8) {
        self.0.borrow_mut().writes.push((cycle, address, value));
    }
    fn play_call(&mut self, cycle: u64, frame: i64) {
        self.0.borrow_mut().calls.push((cycle, frame));
    }
}

pub fn watch(m: &mut Machine) -> Rc<RefCell<Seen>> {
    let seen = Rc::new(RefCell::new(Seen::default()));
    m.set_observer(Some(Box::new(Recorder(seen.clone()))));
    seen
}

pub fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; frames * 2];
    m.render_s16(&mut out);
    out
}

/// The 64-bit FNV-1a hash of a byte string.
pub fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn digest(samples: &[i16]) -> u64 {
    fnv(samples.iter().flat_map(|s| s.to_le_bytes()))
}

/// A machine that runs a plain program from 0x8000, for the tests of the CPU.
pub fn cpu(code: &[u8]) -> Machine {
    let mut rom = vec![0xEA; 0x4000];
    rom[..code.len()].copy_from_slice(code);
    rom[0x3FFC] = 0x00;
    rom[0x3FFD] = 0x80;
    Machine::from_rom(&rom, 48000).unwrap()
}

/// Runs `n` instructions and returns the cycles that they took.
pub fn steps(m: &mut Machine, n: usize) -> u64 {
    let start = m.cycles();
    for _ in 0..n {
        m.step_instruction();
    }
    m.cycles() - start
}

/// The same as `machine`, for a file that asks for the VRC6.
pub fn vrc6_machine(init: &[u8], play: &[u8]) -> Machine {
    let mut f = file(init, play);
    f[0x7B] = oscilla_nsf::chips::VRC6;
    Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap()
}

/// The same as `machine`, for a file that asks for the Sunsoft 5B.
pub fn s5b_machine(init: &[u8], play: &[u8]) -> Machine {
    let mut f = file(init, play);
    f[0x7B] = oscilla_nsf::chips::S5B;
    Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap()
}

/// Stores `v` in register `r` of the Sunsoft 5B through its two ports.
pub fn s5b_reg(r: u8, v: u8) -> Vec<u8> {
    [set(0xC000, r), set(0xE000, v)].concat()
}

/// The same as `machine`, for a file that asks for the Namco 163.
pub fn n163_machine(init: &[u8], play: &[u8]) -> Machine {
    let mut f = file(init, play);
    f[0x7B] = oscilla_nsf::chips::N163;
    Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap()
}

/// Stores `bytes` in the Namco 163's sound RAM from `at` on, through its two
/// ports with the address stepping on each store.
pub fn n163_ram(at: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = set(0xF800, 0x80 | at);
    for &b in bytes {
        out.extend(set(0x4800, b));
    }
    out
}

/// The same as `machine`, for a file that asks for the Famicom Disk System.
pub fn fds_machine(init: &[u8], play: &[u8]) -> Machine {
    let mut f = file(init, play);
    f[0x7B] = oscilla_nsf::chips::FDS;
    Machine::new(&Nsf::parse(&f).unwrap(), 48000).unwrap()
}
