//! More programs, each aimed at ground the first set left alone: audible
//! sound with a running Direct Sound feed, the shifter's edge cases and the
//! multiply, block-transfer and mode-switch instructions, every width of access
//! to every memory region, and Thumb code that talks to the hardware.

use super::{Rng, biased_thumb_program, rom_from_words};

struct Code(Vec<u32>);

impl Code {
    /// ldr rN, =value
    fn load(&mut self, reg: u32, value: u32) {
        self.0.push(0xE59F_0000 | (reg << 12)); // ldr rN, [pc, #0]
        self.0.push(0xEA00_0000); // b +4
        self.0.push(value);
    }

    /// strh of a halfword to an I/O register; r0 holds 0x04000000.
    fn poke16(&mut self, off: u32, value: u32) {
        self.load(1, value & 0xFFFF);
        self.0.push(0xE280_CC00 | (off >> 8)); // add r12, r0, #(off & ~0xFF)
        let low = off & 0xFF;
        self.0.push(0xE1CC_10B0 | ((low >> 4) << 8) | (low & 0xF)); // strh r1, [r12, #low]
    }

    /// str of a word to an I/O register; r0 holds 0x04000000.
    fn poke32(&mut self, off: u32, value: u32) {
        self.load(1, value);
        self.0.push(0xE280_CC00 | (off >> 8));
        self.0.push(0xE58C_1000 | (off & 0xFF)); // str r1, [r12, #low]
    }

    /// r0 = I/O base, r2 = top of work RAM, an interrupt handler that
    /// acknowledges IF and counts in r7, and its address in 0x03007FFC.
    fn interrupt_handler(&mut self) {
        self.0.push(0xE3A0_0301); // mov r0, #0x04000000
        self.0.push(0xE3A0_2403); // mov r2, #0x03000000
        self.0.push(0xE282_2C80); // add r2, r2, #0x8000
        let skip = self.0.len();
        self.0.push(0); // b over the handler
        let handler = self.0.len();
        self.0.extend_from_slice(&[
            0xE3A0_3301, // mov r3, #0x04000000
            0xE283_3C02, // add r3, r3, #0x200
            0xE593_0000, // ldr r0, [r3]
            0xE1A0_0820, // mov r0, r0, lsr #16
            0xE1C3_00B2, // strh r0, [r3, #2]
            0xE287_7001, // add r7, r7, #1
            0xE12F_FF1E, // bx lr
        ]);
        let after = self.0.len();
        self.0[skip] = 0xEA00_0000 | ((after - skip - 2) as u32 & 0x00FF_FFFF);
        self.load(1, 0x0800_0000 + 4 * handler as u32);
        self.0.push(0xE502_1004); // str r1, [r2, #-4]
    }

    /// Turns on the VBlank interrupt.
    fn vblank_interrupts(&mut self) {
        self.poke16(0x004, 0x0008);
        self.poke16(0x200, 0x0001);
        self.poke16(0x208, 1);
    }
}

fn random_halfword(rng: &mut Rng) -> u32 {
    (rng.next() ^ (rng.next() >> 3)) & 0xFFFF
}

/// Random but mostly valid settings for the tone, wave and noise channels.
fn configure_sound(code: &mut Code, rng: &mut Rng) {
    code.poke16(0x84, 0x80); // master on
    code.poke16(
        0x80,
        (random_halfword(rng) & 0x0077) | (0x1100 + ((rng.next() & 0xFF) << 8)) & 0xFF00,
    );
    code.poke16(
        0x82,
        (rng.next() & 0x4F0F) | (rng.next() & 0x3300) | (rng.next() % 3),
    );
    code.poke16(0x88, ((rng.next() & 0x1FF) << 1) | ((rng.next() % 4) << 14));
    // Wave RAM and the wave channel.
    code.poke16(0x70, 0x80 | (rng.next() & 0x60));
    for i in 0..8 {
        code.poke16(0x90 + 2 * i, random_halfword(rng));
    }
    code.poke16(0x70, 0x80 | (rng.next() & 0x60));
    code.poke16(0x72, (rng.next() % 5) << 13 | (rng.next() & 0xFF));
    code.poke16(
        0x74,
        0x8000 | ((rng.next() % 2) << 14) | (0x300 + rng.next() % 0x4F0),
    );
    // Tone 1 with a sweep, tone 2, noise.
    code.poke16(0x60, rng.next() & 0x7F);
    tone(code, rng, 0x62, 0x64);
    tone(code, rng, 0x68, 0x6C);
    noise(code, rng);
}

fn tone(code: &mut Code, rng: &mut Rng, env: u32, freq: u32) {
    code.poke16(
        env,
        ((rng.next() % 16) << 12)
            | ((rng.next() % 2) << 11)
            | ((rng.next() % 8) << 8)
            | ((rng.next() % 4) << 6)
            | (rng.next() % 64),
    );
    code.poke16(
        freq,
        0x8000 | ((rng.next() % 2) << 14) | (0x100 + rng.next() % 0x6F0),
    );
}

fn noise(code: &mut Code, rng: &mut Rng) {
    code.poke16(
        0x78,
        ((rng.next() % 16) << 12)
            | ((rng.next() % 2) << 11)
            | ((rng.next() % 8) << 8)
            | (rng.next() % 64),
    );
    code.poke16(
        0x7C,
        0x8000
            | ((rng.next() % 2) << 14)
            | ((rng.next() % 14) << 4)
            | ((rng.next() % 2) << 3)
            | (rng.next() % 8),
    );
}

/// Sound that can be heard: the whole sound unit configured and triggered,
/// both Direct Sound channels fed by DMA from a table in the cartridge and
/// clocked by a timer, and registers changed again a few frames at a time.
pub fn sound_program(seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0x8CB9_2BA7_2F3D_8DD7).wrapping_add(17));
    let mut c = Code(Vec::new());
    c.interrupt_handler();
    c.vblank_interrupts();
    configure_sound(&mut c, &mut rng);
    // Direct Sound: DMA1 feeds FIFO A, DMA2 feeds FIFO B, timers clock them.
    let (timer_a, timer_b) = ((rng.next() % 2), (rng.next() % 2));
    let select = (timer_a << 10) | (timer_b << 14);
    let soundcnt_h =
        0x1F00 | select | (rng.next() % 3) | ((rng.next() % 2) << 2) | ((rng.next() % 2) << 3);
    c.poke16(0x82, soundcnt_h);
    c.poke32(0xBC, 0x0800_1000);
    c.poke32(0xC0, 0x0400_00A0);
    c.poke16(0xC6, 0xB640);
    c.poke32(0xC8, 0x0800_1400);
    c.poke32(0xCC, 0x0400_00A4);
    c.poke16(0xD2, 0xB640);
    for (timer, period) in [(0u32, 256 + rng.next() % 768), (1, 256 + rng.next() % 768)] {
        c.poke16(0x100 + 4 * timer, 0x1_0000 - period);
        c.poke16(0x102 + 4 * timer, 0x80);
    }
    for _ in 0..(20 + rng.next() % 16) {
        for _ in 0..(rng.next() % 3) {
            c.0.push(0xEF02_0000); // swi 2: Halt until the next interrupt
        }
        match rng.next() % 10 {
            9 => c.poke16(0x84, (rng.next() % 2) * 0x80),
            6 => c.poke16(0x60, rng.next() & 0x7F),
            7 => c.poke16(
                0x74,
                0x8000 | ((rng.next() % 2) << 14) | (0x300 + rng.next() % 0x4F0),
            ),
            8 => c.poke16(0x70, (rng.next() & 0xE0) | ((rng.next() % 2) << 7)),
            0 => tone(&mut c, &mut rng, 0x62, 0x64),
            1 => tone(&mut c, &mut rng, 0x68, 0x6C),
            2 => noise(&mut c, &mut rng),
            3 => c.poke16(0x80, random_halfword(&mut rng) & 0xFF77),
            4 => c.poke16(0x82, (soundcnt_h & !0x3) | (rng.next() % 3)),
            _ => c.poke16(
                0x7C,
                0x8000 | ((rng.next() % 16) << 4) | ((rng.next() % 2) << 3) | (rng.next() % 8),
            ),
        }
    }
    c.0.push(0xEAFF_FFFE); // b .
    assert!(
        c.0.len() * 4 < 0x1000,
        "the code must stay below the sample table"
    );
    let mut rom = rom_from_words(&c.0);
    rom.resize(0x1000, 0);
    for _ in 0..0x800 {
        rom.push(rng.next() as u8);
    }
    rom
}

/// Edge values for the shifter and the multiplier.
const EDGES: [u32; 8] = [
    0,
    1,
    0x7FFF_FFFF,
    0x8000_0000,
    0x8000_0001,
    0xFFFF_FFFF,
    0x0000_FFFF,
    0x1234_5678,
];

/// Data-processing, multiply, swap, block-transfer and status instructions on
/// registers that hold edge values, with shift amounts around 32.
pub fn corner_program(seed: u64) -> Vec<u8> {
    corner_program_at(seed, 0x0800_0000)
}

/// The same program for code that will run at `base`, where its jump to the
/// next instruction after a privileged load of the PC has to land.
#[allow(clippy::needless_late_init)]
pub fn corner_program_at(seed: u64, base: u32) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0xE703_7ED1_A0B4_28DB).wrapping_add(23));
    let mut c = Code(Vec::new());
    if rng.next() % 2 == 0 {
        // Turn the cartridge prefetch buffer on, with random wait states.
        c.load(9, 0x0400_0200);
        c.load(1, 0x4000 | (rng.next() & 0x07FF));
        c.0.push(0xE1C9_10B4); // strh r1, [r9, #4]
    }
    for r in 0..8 {
        c.load(r, EDGES[(rng.next() % 8) as usize]);
    }
    c.0.push(0xE3A0_C403); // mov r12, #0x03000000
    c.0.push(0xE28C_CC02); // add r12, r12, #0x200
    let amounts = [0u32, 1, 2, 31, 32, 33, 63, 64, 255];
    for i in 0..260 {
        if i % 12 == 0 {
            c.load(rng.next() % 8, EDGES[(rng.next() % 8) as usize]);
        }
        let (rd, rn, rm) = ((rng.next() % 8), (rng.next() % 8), (rng.next() % 8));
        match rng.next() % 16 {
            0..=4 => {
                // Data processing: register-specified amount in r8, or an immediate amount.
                let opc = rng.next() % 16;
                let compare = (8..=11).contains(&opc);
                let s = if compare { 1 } else { rng.next() % 2 };
                let kind = rng.next() % 4;
                let word;
                if rng.next() % 2 == 0 {
                    c.0.push(0xE3A0_8000 | amounts[(rng.next() % 9) as usize]);
                    word = 0xE000_0010
                        | (opc << 21)
                        | (s << 20)
                        | (rn << 16)
                        | ((if compare { 0 } else { rd }) << 12)
                        | (8 << 8)
                        | (kind << 5)
                        | rm;
                } else {
                    let amt = [0u32, 1, 15, 31][(rng.next() % 4) as usize];
                    word = 0xE000_0000
                        | (opc << 21)
                        | (s << 20)
                        | (rn << 16)
                        | ((if compare { 0 } else { rd }) << 12)
                        | (amt << 7)
                        | (kind << 5)
                        | rm;
                }
                c.0.push(word);
            }
            5 => {
                // MUL / MLA
                let a = rng.next() % 2;
                c.0.push(
                    0xE000_0090
                        | (a << 21)
                        | ((rng.next() % 2) << 20)
                        | (rd << 16)
                        | (rn << 12)
                        | (rm << 8)
                        | (rng.next() % 8),
                );
            }
            6 => {
                // long multiplies: UMULL, UMLAL, SMULL, SMLAL
                let (hi, lo) = (rd, (rd + 1) % 8);
                c.0.push(
                    0xE080_0090
                        | ((rng.next() % 4) << 21)
                        | ((rng.next() % 2) << 20)
                        | (hi << 16)
                        | (lo << 12)
                        | (rm << 8)
                        | (rn),
                );
            }
            7 => {
                // SWP / SWPB with r12 as the address
                c.0.push(0xE10C_0090 | ((rng.next() % 2) << 22) | (rd << 12) | rm);
            }
            8..=10 => {
                // LDM / STM on r12: every addressing mode, with and without writeback and ^
                let list = match rng.next() % 6 {
                    0 => 0,
                    1 => 1 << (rng.next() % 8),
                    _ => {
                        (rng.next() & 0x00FF)
                            | if rng.next() % 3 == 0 { 1 << 12 } else { 0 }
                            | if rng.next() % 4 == 0 {
                                (1 + rng.next() % 3) << 13 & 0x6000
                            } else {
                                0
                            }
                    }
                };
                let mut load = rng.next() % 2;
                if list == 0 && load == 1 {
                    load = 0; // an empty load list would load the PC and restart the program
                }
                let psr = if rng.next() % 4 == 0 { 1 } else { 0 };
                c.0.push(
                    0xE800_0000
                        | ((rng.next() % 2) << 24)
                        | ((rng.next() % 2) << 23)
                        | (psr << 22)
                        | ((rng.next() % 2) << 21)
                        | (load << 20)
                        | (12 << 16)
                        | list,
                );
                c.0.push(0xE3A0_C403); // mov r12, #0x03000000 (the list may have changed it)
                c.0.push(0xE28C_CC02);
            }
            11 => {
                // MRS / MSR
                match rng.next() % 5 {
                    0 => c.0.push(0xE10F_0000 | (rd << 12)), // mrs rd, cpsr
                    1 => c.0.push(0xE14F_0000 | (rd << 12)), // mrs rd, spsr
                    2 => {
                        c.0.push(0xE328_F000 | ((rng.next() % 16) << 8) | (rng.next() & 0xFF))
                    } // msr cpsr_f, #imm
                    3 => c.0.push(0xE128_F000 | rm),         // msr cpsr_f, rm
                    _ => {
                        c.0.push(0xE368_F000 | ((rng.next() % 16) << 8) | (rng.next() & 0xFF))
                    } // msr spsr_f, #imm
                }
            }
            12 => {
                // Mode switches (interrupts left disabled).
                let mode = [0x10u32, 0x11, 0x12, 0x13, 0x17, 0x1B, 0x1F][(rng.next() % 7) as usize];
                c.0.push(0xE321_F0C0 | mode); // msr cpsr_c, #(I|F|mode)
            }
            14 => {
                // STR / STRH of r15, which stores the address plus 12
                let off = 2 * (rng.next() % 8);
                if rng.next() % 2 == 0 {
                    c.0.push(0xE58C_F000 | (off * 2)); // str r15, [r12, #off]
                } else {
                    c.0.push(0xE1CC_F0B0 | ((off >> 4) << 8) | (off & 0xF)); // strh r15, [r12, #off]
                }
            }
            15 => {
                // Load the PC from memory with the S bit set in a privileged mode,
                // which copies SPSR into CPSR. The target is the next instruction.
                let mode = [0x13u32, 0x12, 0x1B, 0x17][(rng.next() % 4) as usize];
                c.0.push(0xE321_F0C0 | mode); // msr cpsr_c, #(I|F|mode)
                c.0.push(0xE36F_F01F); // msr spsr_cxsf, #0x1F
                let extra = rng.next() % 3 == 0; // r13 and r14 in the list as well
                let target = base + 4 * (c.0.len() as u32 + 5);
                c.load(8, target);
                let before_pc = if extra { 3 } else { 1 };
                c.0.push(0xE58C_8000 | (4 * before_pc)); // str r8, [r12, #pc slot]
                c.0.push(0xE8DC_8200 | if extra { 0x6000 } else { 0 }); // ldmia r12, {r9, [r13, r14,] pc}^
                c.0.push(0xE3A0_C403); // mov r12, #0x03000000
                c.0.push(0xE28C_CC02);
            }
            _ => {
                c.0.push(0xE280_0000 | (rd << 16) | (rd << 12) | (rng.next() & 0xFF))
            } // add rd, rd, #imm
        }
    }
    c.0.push(0xEAFF_FFFE);
    rom_from_words(&c.0)
}

/// Every width of load and store against every kind of address: work RAM and
/// its mirrors, I/O, palette, video RAM and its mirror, object memory, the
/// cartridge, the BIOS, save memory and unmapped space, aligned or not.
pub fn bus_program(seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0xAAAA_AAAB_1234_5679).wrapping_add(29));
    let mut c = Code(Vec::new());
    let regions: [(u32, u32); 21] = [
        (0x0200_0000, 0x1000),
        (0x0204_0000, 0x1000),
        (0x0300_0000, 0x1000),
        (0x0300_8000, 0x1000),
        (0x03FF_FF00, 0x100),
        (0x0400_0000, 0x200),
        (0x0400_0204, 4),
        (0x0500_0000, 0x400),
        (0x0500_0400, 0x400),
        (0x0600_0000, 0x1000),
        (0x0600_FFF0, 0x40),
        (0x0601_3FF0, 0x40),
        (0x0601_8000, 0x1000),
        (0x0700_0000, 0x400),
        (0x0800_0000, 0x1000),
        (0x0A00_0000, 0x1000),
        (0x0E00_0000, 0x100),
        (0x0000_0000, 0x400),
        (0x0000_4000, 0x100),
        (0x0100_0000, 0x100),
        (0x1000_0000, 0x100),
    ];
    for i in 0..170 {
        if i == 0 || i == 85 {
            c.load(2, 0x0400_0000);
            c.load(3, (rng.next() % 6) | (rng.next() & 0x1F00));
            c.0.push(0xE1C2_30B0); // strh r3, [r2]: DISPCNT
        }
        let (base, span) = regions[(rng.next() % 21) as usize];
        let addr = base + rng.next() % span;
        c.load(2, addr);
        c.load(3, rng.next() ^ (rng.next() << 16));
        match rng.next() % 9 {
            0 => c.0.push(0xE582_3000), // str
            1 => c.0.push(0xE1C2_30B0), // strh
            2 => c.0.push(0xE5C2_3000), // strb
            3 => {
                c.0.push(0xE592_4000); // ldr
                c.0.push(0xE085_5004);
            }
            4 => {
                c.0.push(0xE1D2_40B0); // ldrh
                c.0.push(0xE085_5004);
            }
            5 => {
                c.0.push(0xE5D2_4000); // ldrb
                c.0.push(0xE085_5004);
            }
            6 => {
                c.0.push(0xE1D2_40D0); // ldrsb
                c.0.push(0xE085_5004);
            }
            7 => {
                c.0.push(0xE1D2_40F0); // ldrsh
                c.0.push(0xE085_5004);
            }
            _ => {
                c.0.push(0xE582_3000); // str, then ldrb of the same address
                c.0.push(0xE5D2_4000);
                c.0.push(0xE085_5004);
            }
        }
    }
    c.0.push(0xEAFF_FFFE);
    let mut rom = rom_from_words(&c.0);
    for _ in 0..0x400 {
        rom.push(rng.next() as u8);
    }
    rom
}

/// Thumb code that writes the hardware, reads it back, calls the BIOS and
/// halts for interrupts, so that SWI return addresses and the Thumb forms of
/// the I/O accesses are exercised.
pub fn thumb_io_program(seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0xC13F_A9A9_02A6_328F).wrapping_add(31));
    let mut c = Code(Vec::new());
    c.interrupt_handler();
    c.vblank_interrupts();
    c.0.push(0xE28F_E001); // add lr, pc, #1
    c.0.push(0xE12F_FF1E); // bx lr
    let arm_len = c.0.len() as u32;
    let mut t: Vec<u16> = Vec::new();
    let poke = |t: &mut Vec<u16>, off: u32, value: u32| {
        t.push(0x2204); // movs r2, #4
        t.push(0x0612); // lsls r2, r2, #24
        t.push(0x2300 | (off >> 6) as u16); // movs r3, #(off >> 6)
        t.push((6 << 6) | (3 << 3) | 3); // lsls r3, r3, #6
        t.push(0x18D2); // adds r2, r2, r3
        t.push(0x2100 | ((value >> 8) & 0xFF) as u16); // movs r1, #hi
        t.push((8 << 6) | (1 << 3) | 1); // lsls r1, r1, #8
        t.push(0x2400 | (value & 0xFF) as u16); // movs r4, #lo
        t.push(0x1909); // adds r1, r1, r4
        t.push(0x8000 | ((((off & 0x3E) >> 1) as u16) << 6) | (2 << 3) | 1); // strh r1, [r2, #off]
    };
    let sound = [
        0x84u32, 0x80, 0x82, 0x88, 0x62, 0x64, 0x68, 0x6C, 0x78, 0x7C, 0x60, 0x100, 0x102, 0x104,
        0x106,
    ];
    poke(&mut t, 0x84, 0x80);
    poke(&mut t, 0x80, 0xFF77);
    poke(&mut t, 0x82, 0x0002);
    for _ in 0..60 {
        match rng.next() % 8 {
            6 => {
                // empty-list store and push, which move the base by 0x40
                t.push(0x2103); // movs r1, #3
                t.push(0x0609); // lsls r1, r1, #24
                t.push(0x3140); // adds r1, #0x40
                t.push(0xC100); // stmia r1!, {}
                t.push(0xB400); // push {}
            }
            7 => {
                // empty-list load, which loads the PC; the target is the next instruction
                let target = (4 * arm_len + 2 * (t.len() as u32 + 12)) | 1;
                t.push(0x2103); // movs r1, #3
                t.push(0x0609); // lsls r1, r1, #24
                t.push(0x3180); // adds r1, #0x80
                t.push(0x2208); // movs r2, #8
                t.push(0x0612); // lsls r2, r2, #24
                t.push(0x2300 | (target >> 8) as u16); // movs r3, #hi
                t.push(0x021B); // lsls r3, r3, #8
                t.push(0x18D2); // adds r2, r2, r3
                t.push(0x2300 | (target & 0xFF) as u16); // movs r3, #lo
                t.push(0x18D2); // adds r2, r2, r3
                t.push(0x600A); // str r2, [r1, #0]
                t.push(0xC900); // ldmia r1!, {}
            }
            0..=2 => {
                let off = sound[(rng.next() % 15) as usize];
                let mut v = random_halfword(&mut rng);
                if matches!(off, 0x64 | 0x6C | 0x7C) {
                    v |= 0x8000;
                }
                if matches!(off, 0x62 | 0x68 | 0x78) {
                    v |= 0xF000;
                }
                poke(&mut t, off, v);
            }
            3 => {
                // read a register back into the running sum
                let off = [0x004u32, 0x006, 0x100, 0x104, 0x084][(rng.next() % 5) as usize];
                t.push(0x2204);
                t.push(0x0612);
                t.push(0x2300 | (off >> 6) as u16);
                t.push((6 << 6) | (3 << 3) | 3);
                t.push(0x18D2);
                t.push(0x8800 | ((((off & 0x3E) >> 1) as u16) << 6) | (2 << 3) | 4); // ldrh r4, [r2, #off]
                t.push(0x192D); // adds r5, r5, r4
            }
            4 => {
                // a short delay loop
                t.push(0x2600 | (1 + rng.next() % 90) as u16); // movs r6, #n
                t.push(0x3E01); // subs r6, #1
                t.push(0xD1FD); // bne -2
            }
            _ => {
                // a BIOS call, and in half the cases a Halt
                if rng.next() % 2 == 0 {
                    t.push(0xDF02);
                } else {
                    t.push(0x2000 | (rng.next() % 200) as u16); // movs r0, #a
                    t.push(0x2100 | (1 + rng.next() % 9) as u16); // movs r1, #b
                    t.push(0xDF06 + (rng.next() % 3) as u16); // swi 6, 7 or 8
                }
            }
        }
    }
    t.push(0xE7FE); // b .
    let mut rom = rom_from_words(&c.0);
    for h in t {
        rom.extend_from_slice(&h.to_le_bytes());
    }
    rom
}

/// Fills `words` words at `base` with a pattern that depends on `seed`.
fn fill(code: &mut Code, base: u32, words: u32, seed: u32) {
    code.load(3, seed);
    code.load(1, base);
    code.0.push(0xE3A0_2000 | words); // mov r2, #words
    let top = code.0.len();
    code.0.push(0xE481_3004); // str r3, [r1], #4
    code.0
        .push(0xE000_0000 | (1 << 21) | (3 << 16) | (3 << 12) | (13 << 7) | (3 << 5) | 3); // eor r3, r3, r3, ror #13
    code.0.push(0xE283_3035); // add r3, r3, #0x35
    code.0.push(0xE252_2001); // subs r2, r2, #1
    let here = code.0.len() as i32;
    code.0
        .push(0x1A00_0000 | ((top as i32 - (here + 2)) as u32 & 0x00FF_FFFF)); // bne top
}

/// DMA between every pair of regions, in both widths, with every address
/// control, repeats, interrupts and several channels waiting for the same
/// VBlank, so that priority and the open-bus latch matter.
pub fn dma_program(seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0xD6E8_FEB8_6659_FD93).wrapping_add(37));
    let mut c = Code(Vec::new());
    c.interrupt_handler();
    c.poke16(0x004, 0x0018); // VBlank and HBlank interrupts
    c.poke16(0x200, 0x0F03);
    c.poke16(0x208, 1);
    fill(&mut c, 0x0200_0000, 200, rng.next());
    fill(&mut c, 0x0300_0000, 200, rng.next());
    let sources = [
        0x0200_0000u32,
        0x0300_0000,
        0x0800_4000,
        0x0000_0000,
        0x0100_0000,
        0x0400_0000,
        0x0500_0000,
        0x0600_0000,
    ];
    let targets = [
        0x0200_2000u32,
        0x0300_2000,
        0x0500_0000,
        0x0600_0000,
        0x0700_0000,
        0x0600_8000,
    ];
    let setup = |c: &mut Code, rng: &mut Rng, ch: u32, timing: u32| {
        let src = sources[(rng.next() % 8) as usize] + 4 * (rng.next() % 64);
        let dst = targets[(rng.next() % 6) as usize] + 4 * (rng.next() % 64);
        let base = 0xB0 + 12 * ch;
        c.poke32(base, src);
        c.poke32(base + 4, dst);
        c.poke16(base + 8, 1 + rng.next() % 32);
        let word = (rng.next() % 2) << 10;
        let control = 0x8000
            | ((rng.next() % 3 == 0) as u32) << 14
            | (timing << 12)
            | word
            | ((rng.next() % 2) << 9)
            | ((rng.next() % 3) << 7)
            | ((rng.next() % 4) << 5);
        c.poke16(base + 10, control);
    };
    for _ in 0..(18 + rng.next() % 8) {
        match rng.next() % 4 {
            0 => {
                // an immediate transfer
                let ch = rng.next() % 4;
                setup(&mut c, &mut rng, ch, 0);
            }
            1 => {
                // two or three channels waiting for the same VBlank
                for ch in 0..(2 + rng.next() % 2) {
                    setup(&mut c, &mut rng, ch, 1);
                }
                c.0.push(0xEF02_0000);
            }
            2 => {
                // a transfer on HBlank
                let ch = rng.next() % 4;
                setup(&mut c, &mut rng, ch, 2);
                c.0.push(0xEF02_0000);
            }
            _ => {
                // A word transfer leaves a latch with two different halves. A
                // halfword transfer from unmapped space then reads one of them.
                c.poke32(0xD4, 0x0200_0000 + 4 * (rng.next() % 64));
                c.poke32(0xD8, 0x0300_2800);
                c.poke16(0xDC, 4 + rng.next() % 4);
                c.poke16(0xDE, 0x8400);
                c.poke32(0xD4, 0x0100_0000);
                c.poke32(
                    0xD8,
                    0x0300_3000 + 2 * (rng.next() % 2) + 4 * (rng.next() % 4),
                );
                c.poke16(0xDC, 1 + rng.next() % 8);
                c.poke16(0xDE, 0x8000);
            }
        }
        for _ in 0..(rng.next() % 3) {
            c.0.push(0xEF02_0000);
        }
    }
    c.0.push(0xEAFF_FFFE);
    assert!(c.0.len() * 4 < 0x4000, "the code must stay below the data");
    let mut rom = rom_from_words(&c.0);
    rom.resize(0x4000, 0);
    for _ in 0..0x1000 {
        rom.push(rng.next() as u8);
    }
    rom
}

/// A random program copied into work RAM and run there, so that instruction
/// fetch, open-bus values and PC-relative loads see RAM instead of the cartridge.
pub fn ram_program(seed: u64, thumb: bool, base: u32) -> Vec<u8> {
    let image = if thumb {
        biased_thumb_program(seed, 500)
    } else {
        corner_program_at(seed, base)
    };
    let mut c = Code(Vec::new());
    c.load(4, base);
    for word in image.chunks(4) {
        let mut w = [0u8; 4];
        w[..word.len()].copy_from_slice(word);
        c.load(1, u32::from_le_bytes(w));
        c.0.push(0xE484_1004); // str r1, [r4], #4
    }
    c.load(14, base);
    c.0.push(0xE12F_FF1E); // bx lr
    rom_from_words(&c.0)
}

/// A program that calls SoftReset once and then reads what the reset left:
/// the stack pointers of three modes and the part of IWRAM it clears.
pub fn reset_program(seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0xFB2C_9A83_5D31_1C47).wrapping_add(41));
    let mut c = Code(Vec::new());
    c.load(2, 0x0300_0100);
    c.0.push(0xE592_3000); // ldr r3, [r2]
    c.0.push(0xE353_0000); // cmp r3, #0
    let branch = c.0.len();
    c.0.push(0); // bne after
    c.0.push(0xE3A0_3001); // mov r3, #1
    c.0.push(0xE582_3000); // str r3, [r2]
    c.load(4, 0x0300_7E00);
    c.load(5, rng.next() | 0x0101_0101);
    c.0.push(0xE3A0_6080); // mov r6, #128
    let top = c.0.len();
    c.0.push(0xE484_5004); // str r5, [r4], #4
    c.0.push(0xE285_5035); // add r5, r5, #0x35
    c.0.push(0xE256_6001); // subs r6, r6, #1
    let here = c.0.len() as i32;
    c.0.push(0x1A00_0000 | ((top as i32 - (here + 2)) as u32 & 0x00FF_FFFF));
    c.load(4, 0x0300_7DFC);
    c.load(3, rng.next() | 1);
    c.0.push(0xE584_3000); // str r3, [r4]: a word just below the cleared range
    c.load(4, 0x0300_7FFA);
    c.0.push(0xE3A0_3000); // mov r3, #0
    c.0.push(0xE5C4_3000); // strb r3, [r4]: the flag that sends the reset to RAM stays off
    c.0.push(0xEF00_0000); // swi 0: SoftReset
    let after = c.0.len();
    c.0[branch] = 0x1A00_0000 | ((after - branch - 2) as u32 & 0x00FF_FFFF);
    c.0.push(0xE321_F0D3); // msr cpsr_c, #(I|F|SVC)
    c.0.push(0xE1A0_700D); // mov r7, sp
    c.0.push(0xE321_F0D2); // IRQ
    c.0.push(0xE1A0_800D); // mov r8, sp
    c.0.push(0xE321_F0DF); // SYS
    c.0.push(0xE1A0_900D); // mov r9, sp
    c.load(4, 0x0300_7E00);
    c.0.push(0xE594_A000); // ldr r10, [r4]
    c.0.push(0xE594_B1FC); // ldr r11, [r4, #0x1FC]
    c.0.push(0xE514_0004); // ldr r0, [r4, #-4]
    c.0.push(0xEAFF_FFFE);
    rom_from_words(&c.0)
}
