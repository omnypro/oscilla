//! Helpers that the suites share: tiny programs built from bytes, and a
//! recorder for what a driver does.

#![allow(dead_code)]

use oscilla_gb::{Machine, Observer};
use oscilla_gbs::Gbs;
use std::cell::RefCell;
use std::rc::Rc;

pub const LOAD: u16 = 0x0400;
pub const PLAY: u16 = 0x0600;

/// `ld a,v; ldh [reg],a`.
pub fn set(reg: u8, v: u8) -> Vec<u8> {
    vec![0x3E, v, 0xE0, reg]
}

/// The bytes of a GBS file whose init and play routines are `init` and `play`
/// (each ends with RET). Data after `play` is appended at the offset `extra_at`
/// from the load address.
pub fn file(init: &[u8], play: &[u8], tma: u8, tac: u8, extra: Option<(usize, &[u8])>) -> Vec<u8> {
    let mut data = vec![0u8; usize::from(PLAY - LOAD)];
    data[..init.len()].copy_from_slice(init);
    data[init.len()] = 0xC9;
    data.extend_from_slice(play);
    data.push(0xC9);
    if let Some((at, bytes)) = extra {
        if data.len() < at + bytes.len() {
            data.resize(at + bytes.len(), 0);
        }
        data[at..at + bytes.len()].copy_from_slice(bytes);
    }
    let mut f = vec![0u8; 0x70];
    f[..4].copy_from_slice(b"GBS\x01");
    f[4] = 4;
    f[5] = 1;
    f[6..8].copy_from_slice(&LOAD.to_le_bytes());
    f[8..10].copy_from_slice(&LOAD.to_le_bytes());
    f[10..12].copy_from_slice(&PLAY.to_le_bytes());
    f[12..14].copy_from_slice(&0xDFFFu16.to_le_bytes());
    f[14] = tma;
    f[15] = tac;
    f[0x10..0x14].copy_from_slice(b"Test");
    f.extend_from_slice(&data);
    f
}

pub fn gbs(init: &[u8], play: &[u8]) -> Gbs {
    Gbs::parse(&file(init, play, 0, 0, None)).unwrap()
}

pub fn machine(init: &[u8], play: &[u8]) -> Machine {
    Machine::new(&gbs(init, play), 48000).unwrap()
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
