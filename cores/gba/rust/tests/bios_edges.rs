//! Arguments to BIOS calls that sit at the edge of what the cost arithmetic
//! can hold. The calls must run to the end in a build with overflow checks as
//! they do without them.

mod common;

use common::{Machine, Rust, rom_from_words};

/// A cartridge that loads r0..r3 from literals, calls `swi n`, and parks.
fn call(n: u32, regs: [u32; 4]) -> Vec<u8> {
    let mut words = Vec::new();
    for (i, v) in regs.iter().enumerate() {
        words.push(0xE59F_0000 | ((i as u32) << 12)); // ldr ri, [pc, #0]
        words.push(0xEA00_0000); // b +4
        words.push(*v);
    }
    words.push(0xEF00_0000 | (n << 16));
    words.push(0xE3A0_5001); // mov r5, #1
    words.push(0xEAFF_FFFE); // b .
    rom_from_words(&words)
}

#[test]
fn cpu_set_with_addresses_past_the_cartridge_window_does_not_overflow() {
    // 0x18000000 and 0x1D000000 look like cartridge space to the cost rule
    // (region bits 8 and D) but are above every real region.
    for n in [0x0Bu32, 0x0C] {
        for src in [0x1800_0000u32, 0x1D00_0000, 0x0800_0000, 0x2800_0000] {
            for dst in [0x0200_0000u32, 0x1800_0000, 0x1D00_0000] {
                for control in [
                    4u32,
                    4 | (1 << 24),
                    4 | (1 << 26),
                    4 | (1 << 24) | (1 << 26),
                ] {
                    let mut m = Rust::from_rom(&call(n, [src, dst, control, 0]));
                    for _ in 0..40 {
                        m.run_cycles(1);
                    }
                    assert_eq!(
                        m.register(5),
                        1,
                        "swi {n:#x} src {src:#x} dst {dst:#x} control {control:#x}"
                    );
                }
            }
        }
    }
}
