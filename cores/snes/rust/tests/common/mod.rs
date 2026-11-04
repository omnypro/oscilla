#![allow(dead_code)]

use oscilla_snes::{Machine, Spc};

pub const START: u16 = 0x0400;

/// A register file for a test file.
#[derive(Clone, Copy)]
pub struct Regs {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub psw: u8,
    pub sp: u8,
}

pub const ZERO: Regs = Regs { a: 0, x: 0, y: 0, psw: 0, sp: 0xEF };

/// The bytes of an SPC file with the program at START and the given DSP
/// registers. RAM is zero elsewhere, and the control register is zero.
pub fn file(program: &[u8], regs: Regs, dsp: &[(u8, u8)]) -> Vec<u8> {
    let mut f = vec![0u8; 0x10200];
    f[..33].copy_from_slice(b"SNES-SPC700 Sound File Data v0.30");
    f[0x23] = 27;
    f[0x25..0x27].copy_from_slice(&START.to_le_bytes());
    f[0x27..0x2C].copy_from_slice(&[regs.a, regs.x, regs.y, regs.psw, regs.sp]);
    f[0x100 + usize::from(START)..][..program.len()].copy_from_slice(program);
    // Echo writes off, as after a reset, so the echo buffer at 0 leaves the
    // zero page alone.
    f[0x10100 + 0x6C] = 0x20;
    for &(r, v) in dsp {
        f[0x10100 + usize::from(r)] = v;
    }
    f
}

pub fn spc(program: &[u8], regs: Regs, dsp: &[(u8, u8)]) -> Spc {
    Spc::parse(&file(program, regs, dsp)).unwrap()
}

pub fn machine(program: &[u8]) -> Machine {
    machine_with(program, ZERO)
}

pub fn machine_with(program: &[u8], regs: Regs) -> Machine {
    Machine::new(&spc(program, regs, &[]), 32000).unwrap()
}

/// Sets a byte of RAM before the song starts.
pub fn machine_ram(program: &[u8], regs: Regs, ram: &[(u16, u8)]) -> Machine {
    let mut f = file(program, regs, &[]);
    for &(a, v) in ram {
        f[0x100 + usize::from(a)] = v;
    }
    Machine::new(&Spc::parse(&f).unwrap(), 32000).unwrap()
}

/// Runs one instruction and returns the cycles it took.
pub fn step(m: &mut Machine) -> u64 {
    let before = m.cycles();
    m.step_instruction();
    m.cycles() - before
}

pub fn steps(m: &mut Machine, n: usize) {
    for _ in 0..n {
        m.step_instruction();
    }
}

pub fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; frames * 2];
    m.render_s16(&mut out);
    out
}

pub const C: u8 = 0x01;
pub const Z: u8 = 0x02;
pub const H: u8 = 0x08;
pub const V: u8 = 0x40;
pub const N: u8 = 0x80;

pub fn flags(m: &Machine) -> u8 {
    m.cpu_state().4
}

/// Bytes of an SPC700 program that stores each (register, value) pair in the
/// DSP through the ports at 0xF2 and 0xF3 and then spins.
pub fn dsp_program(writes: &[(u8, u8)]) -> Vec<u8> {
    let mut p = Vec::new();
    for &(r, v) in writes {
        p.extend_from_slice(&[0x8F, r, 0xF2, 0x8F, v, 0xF3]);
    }
    p.extend_from_slice(&[0x2F, 0xFE]);
    p
}

/// A file with BRR data at `blocks` (address, bytes), the sample table at
/// 0x1000 pointing source 0 at `start` with loop address `loop_at`, and the
/// given DSP registers at song start.
pub fn dsp_file(
    program: &[u8],
    dsp: &[(u8, u8)],
    blocks: &[(u16, Vec<u8>)],
    start: u16,
    loop_at: u16,
) -> Spc {
    let mut f = file(program, ZERO, dsp);
    let mut put = |a: u16, bytes: &[u8]| {
        f[0x100 + usize::from(a)..][..bytes.len()].copy_from_slice(bytes);
    };
    put(0x1000, &[start as u8, (start >> 8) as u8, loop_at as u8, (loop_at >> 8) as u8]);
    for (a, b) in blocks {
        put(*a, b);
    }
    Spc::parse(&f).unwrap()
}

/// One BRR block: header and sixteen nibbles.
pub fn block(shift: u8, filter: u8, flags: u8, nibbles: [u8; 16]) -> Vec<u8> {
    let mut b = vec![shift << 4 | filter << 2 | flags];
    for pair in nibbles.chunks(2) {
        b.push(pair[0] << 4 | pair[1]);
    }
    b
}

/// The registers of voice 0 playing source 0 with direct gain.
pub fn voice0(pitch: u16, gain: u8, vol: u8) -> Vec<(u8, u8)> {
    vec![
        (0x00, vol),
        (0x01, vol),
        (0x02, pitch as u8),
        (0x03, (pitch >> 8) as u8),
        (0x04, 0),
        (0x05, 0x00),
        (0x07, gain),
        (0x0C, 0x7F),
        (0x1C, 0x7F),
        (0x5D, 0x10),
        (0x4C, 0x01),
    ]
}

pub fn dsp_machine(spc: &Spc) -> Machine {
    Machine::new(spc, 32000).unwrap()
}

pub fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn digest(samples: &[i16]) -> u64 {
    fnv(samples.iter().flat_map(|s| s.to_le_bytes()))
}
