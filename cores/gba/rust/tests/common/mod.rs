//! Test support: a view of a machine from the outside, the programs that the
//! digests and checks run, and the digest itself.

#![allow(dead_code, clippy::collapsible_if, clippy::manual_is_multiple_of)]

mod more;
pub use more::*;

/// What a machine shows from the outside: the view `cgba.h` gives.
pub trait Machine {
    /// Runs until at least `cycles` more native cycles have passed.
    fn run_cycles(&mut self, cycles: u64);
    /// r0..r15 by index 0..15; 16 is CPSR.
    fn register(&self, index: i32) -> u32;
    fn next_pc(&self) -> u32;
    fn cycles(&self) -> u64;
    /// A side-effect-free read of width 1, 2 or 4.
    fn peek(&self, address: u32, width: i32) -> u32;
    /// Renders interleaved stereo frames.
    fn render_s16(&mut self, _frames: usize) -> Vec<i16> {
        unimplemented!("this machine does not render")
    }
}

/// The Rust machine, seen through the same view.
pub struct Rust(pub oscilla_gba::Machine);

impl Rust {
    pub fn from_rom(rom: &[u8]) -> Rust {
        let image = oscilla_gba::image::Image::from_rom(rom).expect("a ROM image");
        Rust(oscilla_gba::Machine::new(&image, 32768).expect("a machine"))
    }
}

impl Machine for Rust {
    fn run_cycles(&mut self, cycles: u64) {
        self.0.run_cycles(cycles)
    }
    fn register(&self, index: i32) -> u32 {
        self.0.register(index)
    }
    fn next_pc(&self) -> u32 {
        self.0.next_pc()
    }
    fn cycles(&self) -> u64 {
        self.0.cycles()
    }
    fn peek(&self, address: u32, width: i32) -> u32 {
        self.0.peek(address, width)
    }
    fn render_s16(&mut self, frames: usize) -> Vec<i16> {
        let mut out = vec![0i16; frames * 2];
        self.0.render_s16(&mut out);
        out
    }
}

/// A small deterministic generator, so a failing seed can be replayed.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

/// A cartridge image from ARM words, little endian, loaded at 0x08000000.
pub fn rom_from_words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// I/O registers worth writing: (offset, whether a 32-bit write is natural).
const IO_WRITES: &[u32] = &[
    0x004, 0x0B0, 0x0B4, 0x0B8, 0x0BA, 0x0BC, 0x0C0, 0x0C4, 0x0C6, 0x0C8, 0x0CC, 0x0D0, 0x0D2,
    0x0D4, 0x0D8, 0x0DC, 0x0DE, 0x100, 0x102, 0x104, 0x106, 0x108, 0x10A, 0x10C, 0x10E, 0x200,
    0x202, 0x204, 0x208, 0x060, 0x062, 0x064, 0x068, 0x06C, 0x070, 0x072, 0x074, 0x078, 0x07C,
    0x080, 0x082, 0x084, 0x088, 0x090, 0x0A0, 0x0A4, 0x301, 0x203, 0x209,
];

/// Registers to read back.
const IO_READS: &[u32] = &[
    0x004, 0x006, 0x100, 0x104, 0x108, 0x10C, 0x102, 0x0BA, 0x0C6, 0x200, 0x202, 0x084, 0x060,
    0x090, 0x208, 0x0DE,
];

struct Asm(Vec<u32>);

impl Asm {
    /// ldr r1, =value
    fn load_value(&mut self, value: u32) {
        self.0.push(0xE59F_1000); // ldr r1, [pc, #0]
        self.0.push(0xEA00_0000); // b +4 (over the word)
        self.0.push(value);
    }
}

pub fn io_program(seed: u64, ops: usize) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F).wrapping_add(5));
    let mut a = Asm(Vec::new());
    // r0 = 0x04000000 (I/O), r2 = 0x03008000 (top of work RAM).
    a.0.push(0xE3A0_0301);
    a.0.push(0xE3A0_2403);
    a.0.push(0xE282_2C80);
    // Handler address into 0x03007FFC; the handler follows the setup.
    let handler_at = 0x0800_0000u32 + 4 * 5;
    a.load_value(handler_at);
    a.0.push(0xE502_1004); // str r1, [r2, #-4]
    // Jump over the handler to the main body.
    let handler = [
        0xE3A0_3301u32, // mov r3, #0x04000000
        0xE283_3C02,    // add r3, r3, #0x200
        0xE593_0000,    // ldr r0, [r3]
        0xE1A0_0820,    // mov r0, r0, lsr #16
        0xE1C3_00B2,    // strh r0, [r3, #2]
        0xE287_7001,    // add r7, r7, #1
        0xE12F_FF1E,    // bx lr
    ];
    // The handler must sit at handler_at: instruction index 5. The setup above
    // already used 3 + 3 + 1 = 7 words, so place a branch there and move on.
    let _ = handler_at;
    let body_skip = a.0.len();
    a.0.push(0); // patched: b over the handler
    let handler_index = a.0.len();
    a.0.extend_from_slice(&handler);
    let after = a.0.len();
    a.0[body_skip] = 0xEA00_0000 | ((after - body_skip - 2) as u32 & 0x00FF_FFFF);
    // The real handler address:
    let real = 0x0800_0000u32 + 4 * handler_index as u32;
    for w in a.0.iter_mut() {
        if *w == handler_at {
            *w = real;
        }
    }
    // Interrupts on: DISPSTAT VBlank, HBlank and VCount, IE for the display and
    // the four timers, IME.
    a.0.push(0xE3A0_1038); // mov r1, #0x38
    a.0.push(0xE1C0_10B4); // strh r1, [r0, #4]
    a.load_value(0x0F0F);
    a.0.push(0xE280_CC02); // add r12, r0, #0x200
    a.0.push(0xE1CC_10B0); // strh r1, [r12]
    a.0.push(0xE3A0_1001); // mov r1, #1
    a.0.push(0xE1CC_10B8); // strh r1, [r12, #8]
    for _ in 0..ops {
        match rng.next() % 10 {
            0..=4 => {
                let off = IO_WRITES[(rng.next() as usize) % IO_WRITES.len()];
                let mut value = rng.next() ^ (rng.next() << 16);
                // Make enabling bits likely.
                match off {
                    0x102 | 0x106 | 0x10A | 0x10E => {
                        value = (value & 0x00C7) | 0x80 | ((rng.next() & 1) << 6)
                    }
                    0x200 => value |= 0x0009,
                    0x208 => value = 1 | (value & 0xFFFE),
                    0x204 => value &= 0x7FFF,
                    0x084 => value |= 0x80,
                    0x0BA | 0x0C6 | 0x0D2 | 0x0DE => {
                        value = (value & 0x7FFF) | if rng.next() % 3 != 0 { 0x8000 } else { 0 };
                    }
                    _ => {}
                }
                a.load_value(value);
                let wide = rng.next() % 3;
                if wide == 0 && off & 3 == 0 {
                    a.0.push(0xE580_1000 | off); // str r1, [r0, #off]
                } else if wide == 1 {
                    // strh r1, [r0, #off]: the offset must fit 8 bits, so form the address.
                    a.0.push(0xE280_CC00 | (off >> 8)); // add r12, r0, #(off & ~0xFF)
                    let low = off & 0xFF;
                    a.0.push(0xE1CC_01B0 | ((low >> 4) << 8) | (low & 0xF)); // strh r0?? fixed below
                    let n = a.0.len();
                    a.0[n - 1] = 0xE1CC_10B0 | ((low >> 4) << 8) | (low & 0xF); // strh r1, [r12, #low]
                } else {
                    a.0.push(0xE280_CC00 | (off >> 8));
                    a.0.push(0xE5CC_1000 | (off & 0xFF)); // strb r1, [r12, #low]
                }
            }
            5 | 6 => {
                let off = IO_READS[(rng.next() as usize) % IO_READS.len()];
                a.0.push(0xE280_CC00 | (off >> 8));
                let low = off & 0xFF;
                a.0.push(0xE1DC_20B0 | ((low >> 4) << 8) | (low & 0xF)); // ldrh r2, [r12, #low]
                a.0.push(0xE085_5002); // add r5, r5, r2
            }
            7 | 8 => {
                let n = 1 + rng.next() % 120;
                a.0.push(0xE3A0_6000 | n); // mov r6, #n
                a.0.push(0xE256_6001); // subs r6, r6, #1
                a.0.push(0x1AFF_FFFD); // bne -2 (to the subs)
            }
            _ => {
                let n = [0x02u32, 0x06, 0x07, 0x08, 0x0B, 0x0C, 0x0E, 0x0F, 0x19]
                    [(rng.next() % 9) as usize];
                a.0.push(0xEF00_0000 | (n << 16)); // swi n
                a.0.push(0xE3A0_0301); // mov r0, #0x04000000 (the call may have used r0)
                a.0.push(0xE3A0_8020); // mov r8, #0x20
                a.0.push(0xE598_4000); // ldr r4, [r8]: a read of BIOS memory from outside it
                a.0.push(0xE085_5004); // add r5, r5, r4
            }
        }
    }
    a.0.push(0xEAFF_FFFE); // b .
    rom_from_words(&a.0)
}

/// Registers get plausible values first, so loads and stores land in memory
/// that exists. Then random ARM words follow.
pub fn arm_program(seed: u64, words: usize) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1));
    let mut program = vec![
        0xE3A0_0402, // mov r0, #0x02000000
        0xE3A0_1403, // mov r1, #0x03000000
        0xE280_2C01, // add r2, r0, #0x100
        0xE281_3C01, // add r3, r1, #0x100
        0xE3A0_4008, // mov r4, #8
        0xE3A0_5CFF, // mov r5, #0xFF00
        0xE3E0_6000, // mvn r6, #0
        0xE3A0_7001, // mov r7, #1
        0xE280_8040, // add r8, r0, #0x40
        0xE281_9040, // add r9, r1, #0x40
    ];
    for _ in 0..words {
        program.push(rng.next() ^ (rng.next() << 16));
    }
    rom_from_words(&program)
}

/// Enters Thumb state at once, then runs random halfwords.
pub fn thumb_program(seed: u64, halfwords: usize) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D).wrapping_add(7));
    let mut bytes = rom_from_words(&[
        0xE3A0_0402, // mov r0, #0x02000000
        0xE3A0_1403, // mov r1, #0x03000000
        0xE280_2C01, // add r2, r0, #0x100
        0xE281_3C01, // add r3, r1, #0x100
        0xE3A0_4008, // mov r4, #8
        0xE3A0_5CFF, // mov r5, #0xFF00
        0xE28F_E001, // add lr, pc, #1  (the Thumb block below)
        0xE12F_FF1E, // bx lr
    ]);
    // lr = pc + 8 + 1 where pc is the add at offset 0x18: 0x08000021 is
    // not the block; the block starts at 0x20, so aim there.
    for _ in 0..halfwords {
        bytes.extend_from_slice(&(rng.next() as u16).to_le_bytes());
    }
    bytes
}

/// ARM words biased toward the always-execute condition, with base registers
/// kept pointing into work RAM so that memory instructions do real work.
pub fn biased_arm_program(seed: u64, words: usize) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03).wrapping_add(3));
    let mut program: Vec<u32> = vec![
        0xE3A0_0402,
        0xE3A0_1403,
        0xE280_2C01,
        0xE281_3C01,
        0xE3A0_4008,
        0xE3A0_5CFF,
        0xE3E0_6000,
        0xE3A0_7001,
    ];
    for i in 0..words {
        if i % 6 == 0 {
            program.push(0xE3A0_0402); // r0 -> EWRAM
            program.push(0xE3A0_1403); // r1 -> IWRAM
        }
        let mut w = rng.next() ^ (rng.next() << 16);
        if rng.next() % 5 != 0 {
            w = (w & 0x0FFF_FFFF) | 0xE000_0000;
        }
        // Memory and block transfers use r0..r3 as the base most of the time.
        let class = (w >> 25) & 7;
        if (class == 2 || class == 3 || class == 4) && rng.next() % 4 != 0 {
            w = (w & !0x000F_0000) | ((rng.next() & 3) << 16);
        }
        // Keep branches inside the program.
        if class == 5 && rng.next() % 4 != 0 {
            w = (w & 0xFF00_0000) | (rng.next() & 0x3F);
        }
        program.push(w);
    }
    rom_from_words(&program)
}

pub fn biased_thumb_program(seed: u64, halfwords: usize) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0xA076_1D64_78BD_642F).wrapping_add(11));
    let mut bytes = rom_from_words(&[
        0xE3A0_0402,
        0xE3A0_1403,
        0xE280_2C01,
        0xE281_3C01,
        0xE3A0_4008,
        0xE3A0_5CFF,
        0xE28F_E001,
        0xE12F_FF1E,
    ]);
    let mut push = |h: u16| bytes.extend_from_slice(&h.to_le_bytes());
    for i in 0..halfwords {
        if i % 8 == 0 {
            push(0x2002); // movs r0, #2
            push(0x0600); // lsls r0, r0, #24
            push(0x2103); // movs r1, #3
            push(0x0609); // lsls r1, r1, #24
        }
        let mut h = rng.next() as u16;
        // Keep the base register of the load/store formats in r0..r1.
        let top = h >> 11;
        if (0x0C..=0x11).contains(&top) && rng.next() % 4 != 0 {
            h = (h & !0x0038) | (((rng.next() & 1) as u16) << 3);
        }
        push(h);
    }
    bytes
}

/// A register value for the math calls: small, signed 16-bit, large negative
/// or any 32-bit pattern.
fn wide_value(rng: &mut Rng) -> u32 {
    match rng.next() % 5 {
        4 => [0, 0xFFFF_FFFF, 0x8000_0000, 1][(rng.next() % 4) as usize],
        0 => rng.next() % 64,
        1 => rng.next() as u16 as i16 as i32 as u32,
        2 => 0u32.wrapping_sub(rng.next() % 40_000),
        _ => rng.next() ^ (rng.next() << 16),
    }
}

pub fn swi_program(n: u32, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed
        .wrapping_mul(0x9FB2_1C65_1E98_DF25)
        .wrapping_add(u64::from(n) + 1));
    let mut words: Vec<u32> = Vec::new();
    let pick_count = |rng: &mut Rng| -> u32 {
        let c = rng.next() % 0x180;
        match rng.next() % 4 {
            0 => c,
            1 => c | (1 << 24),
            2 => c | (1 << 26),
            _ => c | (1 << 24) | (1 << 26),
        }
    };
    // r0 points at a stream header or plain data, r2 at the unpack info
    // structure for BitUnPack and otherwise holds a count.
    let block = 0x0800_0000 + 0x100 + 0x40 * (rng.next() % 64);
    let r0 = match n {
        // The math calls take their arguments in registers.
        0x06..=0x0A => wide_value(&mut rng),
        0x2A => 0x0300_0000 + (rng.next() % 0x1000) * 4,
        _ if rng.next() % 4 == 0 => 0x0800_0100 + (rng.next() % 0x300) * 4,
        _ => block,
    };
    let r2 = if n == 0x10 {
        0x0800_00E0
    } else if n == 0x1F {
        rng.next()
    } else if n == 0x0B || n == 0x0C {
        (1 + rng.next() % 0x40) | [0, 1 << 24, 1 << 26, (1 << 24) | (1 << 26)][(seed % 4) as usize]
    } else if rng.next() % 2 == 0 {
        pick_count(&mut rng)
    } else {
        rng.next() % 0x40
    };
    let values = [
        r0,
        if (0x06..=0x0A).contains(&n) || n == 0x1F {
            wide_value(&mut rng)
        } else {
            0x0200_0000 + (rng.next() % 0x4000) * 4 // r1: EWRAM
        },
        r2,
        if rng.next() % 2 == 0 {
            rng.next() % 0x20
        } else {
            rng.next()
        },
    ];
    // r0..r3 loaded from literals placed after the loads.
    for (i, v) in values.iter().enumerate() {
        words.push(0xE59F_0000 | ((i as u32) << 12)); // ldr ri, [pc, #0]
        words.push(0xEA00_0000); // b +4
        words.push(*v);
    }
    words.push(0xEF00_0000 | (n << 16)); // swi n
    words.push(0xE3A0_5001); // mov r5, #1
    words.push(0xEAFF_FFFE); // b .
    let mut bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    bytes.resize(0x100, 0);
    // The BitUnPack info structure: length, source and destination widths, offset.
    let sw = [1u8, 2, 4, 8][(rng.next() % 4) as usize];
    let dw = [2u8, 4, 8, 16, 32][(rng.next() % 5) as usize].max(sw);
    let offset = (rng.next() & 0xFF) | if rng.next() % 2 == 0 { 0x8000_0000 } else { 0 };
    bytes[0xE0..0xE2].copy_from_slice(&(16 + (rng.next() % 16) as u16).to_le_bytes());
    bytes[0xE2] = sw;
    bytes[0xE3] = dw;
    bytes[0xE4..0xE8].copy_from_slice(&offset.to_le_bytes());
    // Data in 64-byte blocks, each led by the header of a compressed stream.
    for j in 0..64u32 {
        let kind = [0x10u32, 0x30, 0x80, 0x81, 0x28][(j % 5) as usize];
        let header = kind | ((0x40 + rng.next() % 0x100) << 8);
        bytes.extend_from_slice(&header.to_le_bytes());
        for _ in 4..64 {
            bytes.push(rng.next() as u8);
        }
    }
    for _ in 0..0xC00 {
        bytes.push(rng.next() as u8);
    }
    bytes
}

/// A 64-bit FNV-1a hash, for digests that are checked in.
pub struct Fnv(pub u64);

impl Fnv {
    pub fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= u64::from(x);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
}

/// Memory the digest covers: both work RAMs, the I/O registers, palette, video
/// RAM including its mirror, and object memory.
const REGIONS: [(u32, u32); 6] = [
    (0x0200_0000, 0x4_0000),
    (0x0300_0000, 0x8000),
    (0x0400_0000, 0x400),
    (0x0500_0000, 0x400),
    (0x0600_0000, 0x2_0000),
    (0x0700_0000, 0x400),
];

fn hash_memory(m: &dyn Machine, h: &mut Fnv, stride: u32) {
    for (base, size) in REGIONS {
        for offset in (0..size).step_by(stride as usize) {
            h.u32(m.peek(base + offset, 4));
        }
    }
}

/// Runs a machine for `steps` instructions and then renders `frames` frames,
/// and hashes everything a test can see: the CPU after every step, memory at
/// intervals and at the end, and the sound.
pub fn trace_digest(m: &mut dyn Machine, steps: u64, chunks: u32, frames: usize) -> u64 {
    let mut h = Fnv::new();
    let state = |m: &dyn Machine, h: &mut Fnv| {
        for i in 0..=16 {
            h.u32(m.register(i));
        }
        h.u32(m.next_pc());
        h.u64(m.cycles());
    };
    for step in 1..=steps {
        m.run_cycles(1);
        state(m, &mut h);
        if step % 2_000 == 0 {
            hash_memory(m, &mut h, 256);
        }
    }
    // Then coarse steps of about a frame, which carry a program through its
    // waits for interrupts.
    for _ in 0..chunks {
        m.run_cycles(70_000);
        state(m, &mut h);
        hash_memory(m, &mut h, 1024);
    }
    hash_memory(m, &mut h, 4);
    if frames > 0 {
        for s in m.render_s16(frames) {
            h.u32(u32::from(s as u16));
        }
    }
    h.0
}

pub struct Case {
    pub name: String,
    pub rom: Vec<u8>,
    pub steps: u64,
    pub chunks: u32,
    pub frames: usize,
}

/// The programs whose digests are checked in.
pub fn golden_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    let mut add = |name: String, rom: Vec<u8>, steps: u64, chunks: u32, frames: usize| {
        cases.push(Case {
            name,
            rom,
            steps,
            chunks,
            frames,
        });
    };
    for seed in 1..=30 {
        add(
            format!("arm-biased-{seed}"),
            biased_arm_program(seed, 800),
            6_000,
            0,
            2_000,
        );
    }
    for seed in 1..=60 {
        add(
            format!("thumb-biased-{seed}"),
            biased_thumb_program(seed, 1_600),
            6_000,
            0,
            2_000,
        );
    }
    for seed in 1..=20 {
        add(
            format!("arm-random-{seed}"),
            arm_program(seed, 600),
            6_000,
            0,
            0,
        );
        add(
            format!("thumb-random-{seed}"),
            thumb_program(seed, 1_200),
            6_000,
            0,
            0,
        );
    }
    for seed in 1..=40 {
        add(
            format!("io-{seed}"),
            io_program(seed, 150),
            30_000,
            20,
            10_000,
        );
    }
    for n in 0..0x30u32 {
        for seed in 1..=8 {
            add(
                format!("swi-{n:02x}-{seed}"),
                swi_program(n, seed),
                60,
                0,
                0,
            );
        }
    }
    for seed in 1..=60 {
        add(
            format!("sound-{seed}"),
            sound_program(seed),
            3_000,
            40,
            30_000,
        );
    }
    for seed in 1..=90 {
        add(format!("corner-{seed}"), corner_program(seed), 3_500, 0, 0);
    }
    for seed in 1..=60 {
        add(format!("bus-{seed}"), bus_program(seed), 4_000, 0, 0);
    }
    for seed in 1..=40 {
        add(
            format!("thumb-io-{seed}"),
            thumb_io_program(seed),
            6_000,
            30,
            6_000,
        );
    }
    for seed in 1..=40 {
        add(format!("dma-{seed}"), dma_program(seed), 4_000, 40, 4_000);
    }
    for seed in 1..=6 {
        add(format!("reset-{seed}"), reset_program(seed), 900, 0, 0);
    }
    for seed in 1..=10 {
        for (name, thumb, base) in [
            ("arm-iwram", false, 0x0300_4000),
            ("arm-ewram", false, 0x0202_0000),
            ("thumb-iwram", true, 0x0300_4000),
            ("thumb-ewram", true, 0x0202_0000),
        ] {
            add(
                format!("{name}-{seed}"),
                ram_program(seed, thumb, base),
                6_000,
                0,
                1_000,
            );
        }
    }
    cases
}
