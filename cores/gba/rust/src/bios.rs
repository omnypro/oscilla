//! The BIOS, written for this core (GBATEK, "BIOS Functions").
//!
//! Two halves. A small ARM program sits in the BIOS region: the interrupt
//! trampoline at 0x18 that saves r0-r3, r12 and lr and calls the handler whose
//! address the game stores at 0x03007FFC, and the Halt, Stop, IntrWait and
//! VBlankIntrWait loops, which must let interrupts run while they wait.
//! Every other BIOS call is carried out here at the SWI instruction, through
//! the same bus the CPU uses, so its stores are traced like the game's own.
//!
//! Three data words sit in the image where they are never executed. They are
//! the opcodes that a read of BIOS memory returns from outside the BIOS after
//! startup, after an interrupt and after a BIOS call.

use crate::state::*;

const GBA_PI: f64 = std::f64::consts::PI;

const BIOS_IMAGE: &[(u16, u32)] = &[
    (0x000, 0xEA0000AF), // b 0x2c4 <reset>
    (0x004, 0xEA0000AF), // b 0x2c8 <undef>
    (0x008, 0xEA0000AE), // b 0x2c8 <undef>
    (0x00C, 0xEA0000AE), // b 0x2cc <pabort>
    (0x010, 0xEA0000AE), // b 0x2d0 <dabort>
    (0x014, 0xEAFFFFFE), // b 0x14
    (0x018, 0xEA000042), // b 0x128 <irq>
    (0x01C, 0xEA0000AA), // b 0x2cc <pabort>
    (0x0E4, 0xE129F000), // data
    (0x128, 0xE92D500F), // push {r0, r1, r2, r3, r12, lr}
    (0x12C, 0xE3A00301), // mov r0, #67108864
    (0x130, 0xE28FE000), // add lr, pc, #0
    (0x134, 0xE510F004), // ldr pc, [r0, #-0x4]
    (0x138, 0xE8BD500F), // pop {r0, r1, r2, r3, r12, lr}
    (0x13C, 0xE25EF004), // subs pc, lr, #4
    (0x144, 0xE55EC002), // data
    (0x17C, 0xE8BD1000), // ldm sp!, {r12}
    (0x180, 0xE169F00C), // msr SPSR_fc, r12
    (0x184, 0xE8BD500C), // pop {r2, r3, r12, lr}
    (0x188, 0xE1B0F00E), // movs pc, lr
    (0x190, 0xE3A02004), // data
    (0x200, 0xE92D500C), // push {r2, r3, r12, lr}
    (0x204, 0xE14FC000), // mrs r12, spsr
    (0x208, 0xE92D1000), // stmdb sp!, {r12}
    (0x20C, 0xE3A0C301), // mov r12, #67108864
    (0x210, 0xE3A02000), // mov r2, #0
    (0x214, 0xE5CC2301), // strb r2, [r12, #0x301]
    (0x218, 0xEAFFFFD7), // b 0x17c <swi_exit>
    (0x220, 0xE92D500C), // push {r2, r3, r12, lr}
    (0x224, 0xE14FC000), // mrs r12, spsr
    (0x228, 0xE92D1000), // stmdb sp!, {r12}
    (0x22C, 0xE3A0C301), // mov r12, #67108864
    (0x230, 0xE5CC2301), // strb r2, [r12, #0x301]
    (0x234, 0xEAFFFFD0), // b 0x17c <swi_exit>
    (0x240, 0xE92D500C), // push {r2, r3, r12, lr}
    (0x244, 0xE14FC000), // mrs r12, spsr
    (0x248, 0xE92D1000), // stmdb sp!, {r12}
    (0x24C, 0xE3A0C301), // mov r12, #67108864
    (0x250, 0xE3A02080), // mov r2, #128
    (0x254, 0xE5CC2301), // strb r2, [r12, #0x301]
    (0x258, 0xEAFFFFC7), // b 0x17c <swi_exit>
    (0x260, 0xE3A00001), // mov r0, #1
    (0x264, 0xE3A01001), // mov r1, #1
    (0x268, 0xE92D500C), // push {r2, r3, r12, lr}
    (0x26C, 0xE14FC000), // mrs r12, spsr
    (0x270, 0xE92D1000), // stmdb sp!, {r12}
    (0x274, 0xE20CC080), // and r12, r12, #128
    (0x278, 0xE38CC01F), // orr r12, r12, #31
    (0x27C, 0xE121F00C), // msr CPSR_c, r12
    (0x280, 0xE3A0C301), // mov r12, #67108864
    (0x284, 0xE3A03001), // mov r3, #1
    (0x288, 0xE5CC3208), // strb r3, [r12, #0x208]
    (0x28C, 0xE3500000), // cmp r0, #0
    (0x290, 0x115C30B8), // ldrhne r3, [r12, #-8]
    (0x294, 0x11C33001), // bicne r3, r3, r1
    (0x298, 0x114C30B8), // strhne r3, [r12, #-8]
    (0x29C, 0xE15C30B8), // ldrh r3, [r12, #-8]
    (0x2A0, 0xE0132001), // ands r2, r3, r1
    (0x2A4, 0x1A000002), // bne 0x2b4 <done>
    (0x2A8, 0xE3A02000), // mov r2, #0
    (0x2AC, 0xE5CC2301), // strb r2, [r12, #0x301]
    (0x2B0, 0xEAFFFFF9), // b 0x29c <check>
    (0x2B4, 0xE1C33001), // bic r3, r3, r1
    (0x2B8, 0xE14C30B8), // strh r3, [r12, #-8]
    (0x2BC, 0xE321F093), // msr CPSR_c, #147
    (0x2C0, 0xEAFFFFAD), // b 0x17c <swi_exit>
    (0x2C4, 0xEF000000), // svc #0x0
    (0x2C8, 0xE1B0F00E), // movs pc, lr
    (0x2CC, 0xE25EF004), // subs pc, lr, #4
    (0x2D0, 0xE25EF008), // subs pc, lr, #8
    (0x2D4, 0xE12FFF1E), // bx lr
];

const BIOS_HALT: u32 = 0x200;
const BIOS_CUSTOM_HALT: u32 = 0x220;
const BIOS_STOP: u32 = 0x240;
const BIOS_VBLANK_INTR_WAIT: u32 = 0x260;
const BIOS_INTR_WAIT: u32 = 0x268;
const BIOS_RETURN_STUB: u32 = 0x2D4;

/// Time charged for BIOS calls done here, so they cost the game the frame
/// time they would on the machine. These figures are a calibration, and
/// nothing here models the BIOS code.
const BIOS_CALL_CYCLES: u64 = 100;

/// Games convert a few dozen sets at a time. The count is a 32-bit register
/// the program controls, so it is capped to keep one call from running for
/// minutes.
const MAX_AFFINE_SETS: u32 = 0x10000;

pub(crate) fn bios_image() -> Box<[u8; BIOS_SIZE as usize]> {
    let mut bios = zeroed::<{ BIOS_SIZE as usize }>();
    for &(addr, word) in BIOS_IMAGE {
        let a = usize::from(addr);
        bios[a..a + 4].copy_from_slice(&word.to_le_bytes());
    }
    bios
}

/// Writes one decompressed byte, either directly or paired into halfwords for
/// the variants meant for VRAM.
struct ByteOut {
    dst: u32,
    pending: u32,
    halves: bool,
}

/// Fixed-point sine and cosine of a BIOS angle (0..0xFFFF is one turn; the
/// BIOS looks only at the upper byte), scaled by 1 << 14.
fn sincos14(angle: u32) -> (i32, i32) {
    crate::tables::SINCOS14[((angle >> 8) & 0xFF) as usize]
}

/// The same value computed with the platform's libm, for the test that checks
/// the table.
#[doc(hidden)]
pub fn reference_sincos14(angle: u32) -> (i32, i32) {
    let t = f64::from((angle >> 8) & 0xFF) / 256.0 * 2.0 * GBA_PI;
    (
        (t.sin() * 16384.0).round() as i64 as i32,
        (t.cos() * 16384.0).round() as i64 as i32,
    )
}

impl Machine {
    fn warn_swi(&mut self, n: u32, what: &str) {
        let (word, bit) = ((n >> 5) as usize, 1u32 << (n & 31));
        if self.warned_swi[word] & bit != 0 {
            return;
        }
        self.warned_swi[word] |= bit;
        let mut line = format!("BIOS call 0x{:02X} {}", n & 0xFF, what);
        line.truncate(127);
        self.sys_warn(&line);
    }

    fn rd8(&self, a: u32) -> u32 {
        self.bus_read8(a)
    }
    fn rd16(&self, a: u32) -> u32 {
        self.bus_read16(a)
    }
    fn rd32(&self, a: u32) -> u32 {
        self.bus_read32(a)
    }
    fn wr8(&mut self, a: u32, v: u32) {
        self.bus_write8(a, v, Source::Bios);
    }
    fn wr16(&mut self, a: u32, v: u32) {
        self.bus_write16(a, v, Source::Bios);
    }
    fn wr32(&mut self, a: u32, v: u32) {
        self.bus_write32(a, v, Source::Bios);
    }

    /// The cost sums wrap like the unsigned arithmetic they come from: an
    /// address above 0x0FFFFFFF in a cartridge region gives a cost of 1 from
    /// `bus_cycles`, and the cartridge rule then subtracts 2 from it.
    fn unit_cost(&self, addr: u32, width: u32) -> u64 {
        let r = (addr >> 24) & 0xF;
        if (0x8..=0xD).contains(&r) {
            return if width == 4 {
                (self.bus_cycles(addr, 4, true) as u64).wrapping_sub(2)
            } else {
                self.bus_cycles(addr, 2, false) as u64
            };
        }
        self.bus_cycles(addr, width, false) as u64
    }

    fn burst_cost(&self, addr: u32) -> u64 {
        let r = (addr >> 24) & 0xF;
        if (0x8..=0xD).contains(&r) {
            return (8 * self.bus_cycles(addr, 4, true) as u64).wrapping_sub(2);
        }
        self.bus_cycles(addr, 4, false) as u64 + 7 * self.bus_cycles(addr, 4, true) as u64
    }

    /// CpuSet: per unit, 7 cycles plus the read and the write; a fill reads once.
    fn charge_set(&mut self, src: u32, dst: u32, units: u32, width: u32, fill: bool) {
        let per = if fill {
            5u64.wrapping_add(self.unit_cost(dst, width))
        } else {
            7u64.wrapping_add(self.unit_cost(src, width))
                .wrapping_add(self.unit_cost(dst, width))
        };
        let setup: u64 = if width == 4 {
            7
        } else if fill {
            11
        } else {
            6
        };
        self.now = self.now.wrapping_add(
            per.wrapping_mul(u64::from(units))
                .wrapping_add(BIOS_CALL_CYCLES + setup),
        );
    }

    /// CpuFastSet: per 8 words, 7 cycles plus a burst read and a burst write.
    fn charge_fast(&mut self, src: u32, dst: u32, words: u32, fill: bool) {
        let per = if fill {
            5u64.wrapping_add(self.burst_cost(dst))
        } else {
            7u64.wrapping_add(self.burst_cost(src))
                .wrapping_add(self.burst_cost(dst))
        };
        self.now = self.now.wrapping_add(
            per.wrapping_mul(u64::from(words / 8))
                .wrapping_add(BIOS_CALL_CYCLES + if fill { 19 } else { 9 }),
        );
    }

    /// The decompressors and the rest: a per-byte estimate.
    fn charge(&mut self, src: u32, dst: u32, units: u32, width: u32) {
        let per =
            self.bus_cycles(src, width, true) as u64 + self.bus_cycles(dst, width, true) as u64 + 6;
        self.now = self.now.wrapping_add(
            per.wrapping_mul(u64::from(units))
                .wrapping_add(BIOS_CALL_CYCLES),
        );
    }

    fn soft_reset(&mut self) {
        let to_ram = self.iwram[0x7FFA];
        self.iwram[0x7E00..0x8000].fill(0);
        self.cpu_set_mode(MODE_SVC);
        self.cpu.r[13] = 0x0300_7FE0;
        self.cpu.r[14] = 0;
        self.cpu.spsr = 0;
        self.cpu_set_mode(MODE_IRQ);
        self.cpu.r[13] = 0x0300_7FA0;
        self.cpu.r[14] = 0;
        self.cpu.spsr = 0;
        self.cpu_set_mode(MODE_SYS);
        let c = &mut self.cpu;
        c.r[13] = 0x0300_7F00;
        for i in 0..13 {
            c.r[i] = 0;
        }
        c.r[14] = 0;
        c.cpsr = MODE_SYS;
        c.next_pc = if to_ram != 0 {
            0x0200_0000
        } else {
            0x0800_0000
        };
        c.pc_written = true;
    }

    fn register_ram_reset(&mut self, flags: u32) {
        self.wr16(0x0400_0000, 0x0080);
        if flags & 0x01 != 0 {
            self.ewram.fill(0);
        }
        if flags & 0x02 != 0 {
            self.iwram[..0x7E00].fill(0);
        }
        if flags & 0x04 != 0 {
            self.pal.fill(0);
        }
        if flags & 0x08 != 0 {
            self.vram.fill(0);
        }
        if flags & 0x10 != 0 {
            self.oam.fill(0);
        }
        if flags & 0x20 != 0 {
            for a in (0x120..0x160).step_by(2) {
                self.wr16(0x0400_0000 + a, 0);
            }
        }
        if flags & 0x40 != 0 {
            // Sound off first, then every PSG register and the bias default.
            self.wr16(0x0400_0084, 0);
            for a in (0x60..0x84).step_by(2) {
                self.wr16(0x0400_0000 + a, 0);
            }
            for a in (0x90..0xA0).step_by(2) {
                self.wr16(0x0400_0000 + a, 0);
            }
            self.wr16(0x0400_0088, 0x0200);
        }
        if flags & 0x80 != 0 {
            for a in (0x02..0x60).step_by(2) {
                self.wr16(0x0400_0000 + a, 0);
            }
            for a in (0xB0..0xE0).step_by(2) {
                self.wr16(0x0400_0000 + a, 0);
            }
            for a in (0x100..0x110).step_by(2) {
                self.wr16(0x0400_0000 + a, 0);
            }
            self.wr16(0x0400_0200, 0);
            self.wr16(0x0400_0204, 0);
            self.wr16(0x0400_0208, 0);
        }
    }

    fn cpu_set(&mut self, fast: bool) {
        let mut src = self.cpu.r[0];
        let mut dst = self.cpu.r[1];
        let ctl = self.cpu.r[2];
        let mut count = ctl & 0x1F_FFFF;
        let fill = (ctl >> 24) & 1 != 0;
        let word = fast || (ctl >> 26) & 1 != 0;
        if fast {
            count = (count + 7) & !7;
        }
        let bytes = count.wrapping_mul(if word { 4 } else { 2 });
        let last = if fill {
            src
        } else {
            src.wrapping_add(bytes).wrapping_sub(1)
        };
        if (src >> 24) == 0 || (last >> 24) == 0 {
            // the BIOS refuses to read itself
            self.now = self.now.wrapping_add(BIOS_CALL_CYCLES);
            return;
        }
        if word {
            src &= !3;
            dst &= !3;
            let mut v = if fill { self.rd32(src) } else { 0 };
            for i in 0..count {
                if !fill {
                    v = self.rd32(src.wrapping_add(4u32.wrapping_mul(i)));
                }
                self.wr32(dst.wrapping_add(4u32.wrapping_mul(i)), v);
            }
            if fast {
                self.charge_fast(src, dst, count, fill);
            } else {
                self.charge_set(src, dst, count, 4, fill);
            }
        } else {
            src &= !1;
            dst &= !1;
            let mut v = if fill { self.rd16(src) } else { 0 };
            for i in 0..count {
                if !fill {
                    v = self.rd16(src.wrapping_add(2u32.wrapping_mul(i)));
                }
                self.wr16(dst.wrapping_add(2u32.wrapping_mul(i)), v);
            }
            self.charge_set(src, dst, count, 2, fill);
        }
    }

    fn put_byte(&mut self, o: &mut ByteOut, b: u32) {
        if !o.halves {
            self.wr8(o.dst, b);
            o.dst = o.dst.wrapping_add(1);
            return;
        }
        if o.dst & 1 != 0 {
            self.wr16(o.dst.wrapping_sub(1), o.pending | (b << 8));
        } else {
            o.pending = b & 0xFF;
        }
        o.dst = o.dst.wrapping_add(1);
    }

    fn lz77(&mut self, halves: bool) {
        let (r0, r1) = (self.cpu.r[0], self.cpu.r[1]);
        let mut src = r0;
        let size = self.rd32(src) >> 8;
        let mut o = ByteOut {
            dst: r1,
            pending: 0,
            halves,
        };
        let mut done = 0u32;
        src = src.wrapping_add(4);
        while done < size {
            let mut flags = self.rd8(src);
            src = src.wrapping_add(1);
            let mut k = 0;
            while k < 8 && done < size {
                if flags & 0x80 != 0 {
                    let b1 = self.rd8(src);
                    src = src.wrapping_add(1);
                    let b2 = self.rd8(src);
                    src = src.wrapping_add(1);
                    let len = (b1 >> 4) + 3;
                    let disp = (((b1 & 0xF) << 8) | b2) + 1;
                    let mut i = 0;
                    while i < len && done < size {
                        let b = self.rd8(o.dst.wrapping_sub(disp));
                        self.put_byte(&mut o, b);
                        i += 1;
                        done += 1;
                    }
                } else {
                    let b = self.rd8(src);
                    src = src.wrapping_add(1);
                    self.put_byte(&mut o, b);
                    done += 1;
                }
                k += 1;
                flags <<= 1;
            }
        }
        self.charge(r0, r1, size, 1);
    }

    fn run_length(&mut self, halves: bool) {
        let (r0, r1) = (self.cpu.r[0], self.cpu.r[1]);
        let mut src = r0;
        let size = self.rd32(src) >> 8;
        let mut o = ByteOut {
            dst: r1,
            pending: 0,
            halves,
        };
        let mut done = 0u32;
        src = src.wrapping_add(4);
        while done < size {
            let flag = self.rd8(src);
            src = src.wrapping_add(1);
            if flag & 0x80 != 0 {
                let len = (flag & 0x7F) + 3;
                let b = self.rd8(src);
                src = src.wrapping_add(1);
                let mut i = 0;
                while i < len && done < size {
                    self.put_byte(&mut o, b);
                    i += 1;
                    done += 1;
                }
            } else {
                let len = (flag & 0x7F) + 1;
                let mut i = 0;
                while i < len && done < size {
                    let b = self.rd8(src);
                    src = src.wrapping_add(1);
                    self.put_byte(&mut o, b);
                    i += 1;
                    done += 1;
                }
            }
        }
        self.charge(r0, r1, size, 1);
    }

    fn huffman(&mut self) {
        let (r0, r1) = (self.cpu.r[0], self.cpu.r[1]);
        let src = r0;
        let mut dst = r1;
        let hdr = self.rd32(src);
        let mut bits = hdr & 0xF;
        let size = hdr >> 8;
        if bits == 0 || bits > 8 {
            bits = 8;
        }
        let tree = src.wrapping_add(4);
        let root = tree.wrapping_add(1);
        let mut stream = tree.wrapping_add((self.rd8(tree) + 1) * 2);
        let mut node = root;
        let mut outbits = 0u32;
        let mut done = 0u32;
        let mut inner = 0u32;
        let mut out = 0u64;
        while done < size {
            let word = self.rd32(stream);
            stream = stream.wrapping_add(4);
            let mut b: i32 = 31;
            while b >= 0 && done < size {
                let v = self.rd8(node);
                let next = (node & !1).wrapping_add((v & 0x3F) * 2).wrapping_add(2);
                let one = (word >> b) & 1 != 0;
                let child = if one { next.wrapping_add(1) } else { next };
                let leaf = if one { v & 0x40 != 0 } else { v & 0x80 != 0 };
                if !leaf {
                    node = child;
                    // A tree has at most 256 nodes, so a longer walk is a loop.
                    inner += 1;
                    if inner > 256 {
                        return;
                    }
                    b -= 1;
                    continue;
                }
                inner = 0;
                out |= u64::from(self.rd8(child) & ((1u32 << bits) - 1)) << outbits;
                outbits += bits;
                node = root;
                // The bit widths that do not divide 32 carry into the next word.
                if outbits >= 32 {
                    self.wr32(dst, out as u32);
                    dst = dst.wrapping_add(4);
                    done = done.wrapping_add(4);
                    out >>= 32;
                    outbits -= 32;
                }
                b -= 1;
            }
        }
        self.charge(r0, r1, size, 1);
    }

    fn bit_unpack(&mut self) {
        let (r0, r1, info) = (self.cpu.r[0], self.cpu.r[1], self.cpu.r[2]);
        let src = r0;
        let mut dst = r1;
        let len = self.rd16(info);
        let sw = self.rd8(info.wrapping_add(2));
        let dw = self.rd8(info.wrapping_add(3));
        let mut offset = self.rd32(info.wrapping_add(4));
        let zero = (offset >> 31) & 1 != 0;
        offset &= 0x7FFF_FFFF;
        if sw == 0 || sw > 8 || dw == 0 || dw > 32 {
            return;
        }
        let smask = (1u32 << sw) - 1;
        let dmask = if dw == 32 {
            0xFFFF_FFFF
        } else {
            (1u32 << dw) - 1
        };
        let mut out = 0u32;
        let mut outbits = 0u32;
        dst &= !3;
        for i in 0..len {
            let byte = self.rd8(src.wrapping_add(i));
            let mut b = 0;
            while b < 8 {
                let mut u = (byte >> b) & smask;
                if u != 0 || zero {
                    u = u.wrapping_add(offset);
                }
                out |= (u & dmask) << outbits;
                outbits += dw;
                if outbits >= 32 {
                    self.wr32(dst, out);
                    dst = dst.wrapping_add(4);
                    out = 0;
                    outbits = 0;
                }
                b += sw;
            }
        }
        self.charge(r0, r1, len, 1);
    }

    fn diff_unfilter(&mut self, sixteen: bool, halves: bool) {
        let (r0, r1) = (self.cpu.r[0], self.cpu.r[1]);
        let mut src = r0;
        let dst = r1;
        let size = self.rd32(src) >> 8;
        src = src.wrapping_add(4);
        if sixteen {
            let mut acc = 0u32;
            let mut i = 0u32;
            while 2 * i < size {
                acc = acc.wrapping_add(self.rd16(src.wrapping_add(2 * i))) & 0xFFFF;
                self.wr16(dst.wrapping_add(2 * i), acc);
                i += 1;
            }
        } else {
            let mut o = ByteOut {
                dst,
                pending: 0,
                halves,
            };
            let mut acc = 0u32;
            for i in 0..size {
                acc = acc.wrapping_add(self.rd8(src.wrapping_add(i))) & 0xFF;
                self.put_byte(&mut o, acc);
            }
        }
        self.charge(r0, r1, size, 1);
    }

    fn obj_affine_set(&mut self) {
        let (mut src, mut dst, mut n, off) =
            (self.cpu.r[0], self.cpu.r[1], self.cpu.r[2], self.cpu.r[3]);
        if n > MAX_AFFINE_SETS {
            n = MAX_AFFINE_SETS;
        }
        for _ in 0..n {
            let sx = self.rd16(src) as u16 as i16 as i32;
            let sy = self.rd16(src.wrapping_add(2)) as u16 as i16 as i32;
            let (s, co) = sincos14(self.rd16(src.wrapping_add(4)));
            self.wr16(dst, ((sx * co) >> 14) as u32);
            dst = dst.wrapping_add(off);
            self.wr16(dst, (-((sx * s) >> 14)) as u32);
            dst = dst.wrapping_add(off);
            self.wr16(dst, ((sy * s) >> 14) as u32);
            dst = dst.wrapping_add(off);
            self.wr16(dst, ((sy * co) >> 14) as u32);
            dst = dst.wrapping_add(off);
            src = src.wrapping_add(8);
        }
    }

    fn bg_affine_set(&mut self) {
        let (mut src, mut dst, mut n) = (self.cpu.r[0], self.cpu.r[1], self.cpu.r[2]);
        if n > MAX_AFFINE_SETS {
            n = MAX_AFFINE_SETS;
        }
        for _ in 0..n {
            let ox = self.rd32(src) as i32;
            let oy = self.rd32(src.wrapping_add(4)) as i32;
            let cx = self.rd16(src.wrapping_add(8)) as u16 as i16 as i32;
            let cy = self.rd16(src.wrapping_add(10)) as u16 as i16 as i32;
            let sx = self.rd16(src.wrapping_add(12)) as u16 as i16 as i32;
            let sy = self.rd16(src.wrapping_add(14)) as u16 as i16 as i32;
            let (s, co) = sincos14(self.rd16(src.wrapping_add(16)));
            let pa = (sx * co) >> 14;
            let pb = -((sx * s) >> 14);
            let pc = (sy * s) >> 14;
            let pd = (sy * co) >> 14;
            self.wr16(dst, pa as u32);
            self.wr16(dst.wrapping_add(2), pb as u32);
            self.wr16(dst.wrapping_add(4), pc as u32);
            self.wr16(dst.wrapping_add(6), pd as u32);
            self.wr32(
                dst.wrapping_add(8),
                ox.wrapping_sub(pa.wrapping_mul(cx).wrapping_add(pb.wrapping_mul(cy))) as u32,
            );
            self.wr32(
                dst.wrapping_add(12),
                oy.wrapping_sub(pc.wrapping_mul(cx).wrapping_add(pd.wrapping_mul(cy))) as u32,
            );
            src = src.wrapping_add(20);
            dst = dst.wrapping_add(16);
        }
    }

    fn enter_routine(&mut self, addr: u32) {
        let ret = self
            .cpu
            .cur_pc
            .wrapping_add(if self.cpu.cpsr & PSR_T != 0 { 2 } else { 4 });
        self.cpu_raise_exception(addr, MODE_SVC, ret);
    }

    pub(crate) fn bios_swi(&mut self, n: u32) {
        match n {
            0x02 => return self.enter_routine(BIOS_HALT),
            0x03 => return self.enter_routine(BIOS_STOP),
            0x04 => return self.enter_routine(BIOS_INTR_WAIT),
            0x05 => return self.enter_routine(BIOS_VBLANK_INTR_WAIT),
            0x27 => return self.enter_routine(BIOS_CUSTOM_HALT),
            _ => {}
        }
        match n {
            0x00 => self.soft_reset(),
            0x01 => {
                let flags = self.cpu.r[0];
                self.register_ram_reset(flags);
            }
            0x06 | 0x07 => {
                let r = &self.cpu.r;
                let num = (if n == 0x06 { r[0] } else { r[1] }) as i32;
                let den = (if n == 0x06 { r[1] } else { r[0] }) as i32;
                if den == 0 {
                    self.warn_swi(n, "divided by zero; the real BIOS would hang.");
                    let r = &mut self.cpu.r;
                    r[0] = if num < 0 { 0xFFFF_FFFF } else { 1 };
                    r[1] = num as u32;
                    r[3] = 1;
                } else if num == i32::MIN && den == -1 {
                    let r = &mut self.cpu.r;
                    r[0] = 0x8000_0000;
                    r[1] = 0;
                    r[3] = 0x8000_0000;
                } else {
                    let (q, m) = (num / den, num % den);
                    let r = &mut self.cpu.r;
                    r[0] = q as u32;
                    r[1] = m as u32;
                    r[3] = (if q < 0 { -i64::from(q) } else { i64::from(q) }) as u32;
                }
                self.now += 40;
            }
            0x08 => {
                let mut v = self.cpu.r[0];
                let mut root = 0u32;
                let mut bit = 1u32 << 30;
                while bit > v {
                    bit >>= 2;
                }
                while bit != 0 {
                    if v >= root + bit {
                        v -= root + bit;
                        root = (root >> 1) + bit;
                    } else {
                        root >>= 1;
                    }
                    bit >>= 2;
                }
                self.cpu.r[0] = root;
                self.now += 40;
            }
            0x09 => {
                let t = f64::from(self.cpu.r[0] as u16 as i16) / 16384.0;
                self.cpu.r[0] = ((crate::exact::atan(t) / (GBA_PI / 2.0) * 16384.0).round() as i64)
                    as i32 as u32;
            }
            0x0A => {
                let x = f64::from(self.cpu.r[0] as u16 as i16);
                let y = f64::from(self.cpu.r[1] as u16 as i16);
                let mut a = crate::exact::atan2(y, x);
                if a < 0.0 {
                    a += 2.0 * GBA_PI;
                }
                self.cpu.r[0] = (((a / (2.0 * GBA_PI) * 65536.0).round() as i64) as u32) & 0xFFFF;
            }
            0x0B => self.cpu_set(false),
            0x0C => self.cpu_set(true),
            0x0D => self.cpu.r[0] = 0xBAAE_187F,
            0x0E => self.bg_affine_set(),
            0x0F => self.obj_affine_set(),
            0x10 => self.bit_unpack(),
            0x11 => self.lz77(false),
            0x12 => self.lz77(true),
            0x13 => self.huffman(),
            0x14 => self.run_length(false),
            0x15 => self.run_length(true),
            0x16 => self.diff_unfilter(false, false),
            0x17 => self.diff_unfilter(false, true),
            0x18 => self.diff_unfilter(true, false),
            0x19 => {
                let cur = self.bus_read16(0x0400_0088);
                let on = self.cpu.r[0] != 0;
                self.wr16(0x0400_0088, (cur & !0x3FE) | if on { 0x200 } else { 0 });
            }
            0x1F => {
                let freq = f64::from(self.rd32(self.cpu.r[0].wrapping_add(4)));
                let period = crate::exact::pow2_key(self.cpu.r[1] & 0xFF, self.cpu.r[2] & 0xFF);
                self.cpu.r[0] = (freq / period) as u32;
            }
            0x25 => {
                self.cpu.r[0] = 1;
                self.warn_swi(n, "(MultiBoot) is not supported; it reports failure.");
            }
            0x26 => {
                self.warn_swi(n, "(HardReset) restarts the cartridge.");
                self.soft_reset();
            }
            0x2A => {
                let base = self.cpu.r[0];
                for i in 0..36u32 {
                    self.wr32(base.wrapping_add(4 * i), BIOS_RETURN_STUB);
                }
                self.warn_swi(n, "(SoundGetJumpList) points every entry at a return.");
            }
            0x1A..=0x1E | 0x20..=0x24 | 0x28 | 0x29 => {
                self.warn_swi(
                    n,
                    "(a BIOS sound driver call) is not implemented; it returns at once.",
                );
            }
            _ => {
                self.warn_swi(n, "is not a GBA BIOS function; it returns at once.");
            }
        }
        self.bios_latch = 0xE3A0_2004;
    }
}
