//! Public test cartridges for the CPU, the timer and the sound unit. They are
//! not part of the repository: point `OSCILLA_GB_TESTS` at a folder that holds
//! them. A cartridge reports on the serial port and ends its text with
//! "Passed" or "Failed".

use oscilla_gb::Machine;
use std::path::{Path, PathBuf};

fn folder() -> Option<PathBuf> {
    std::env::var_os("OSCILLA_GB_TESTS").map(PathBuf::from)
}

/// The result that a cartridge leaves in cart RAM: a signature at 0xA001, a
/// status at 0xA000 (0x80 while it runs, 0 for a pass) and text from 0xA004.
fn ram_result(m: &Machine) -> Option<(u8, String)> {
    if [0xDE, 0xB0, 0x61] != [m.peek(0xA001), m.peek(0xA002), m.peek(0xA003)] {
        return None;
    }
    let status = m.peek(0xA000);
    if status == 0x80 {
        return None;
    }
    let text: Vec<u8> = (0xA004u16..0xB000)
        .map(|a| m.peek(a))
        .take_while(|&b| b != 0)
        .collect();
    Some((status, String::from_utf8_lossy(&text).into_owned()))
}

/// Runs a cartridge until it reports, and returns the text. A pass always
/// contains the word "Passed".
fn run(rom: &Path, limit: u64) -> String {
    let image = std::fs::read(rom).unwrap();
    let mut m = Machine::from_rom(&image, 48000).unwrap();
    while m.cycles() < limit {
        m.run_cycles(1 << 20);
        let text = String::from_utf8_lossy(m.serial()).into_owned();
        if text.contains("Passed") || text.contains("Failed") {
            return text;
        }
        if let Some((status, text)) = ram_result(&m) {
            return if status == 0 {
                format!("{text}\nPassed")
            } else {
                format!("{text}\nFailed ({status})")
            };
        }
    }
    String::from_utf8_lossy(m.serial()).into_owned()
}

fn check(paths: &[&str], limit: u64) {
    let Some(dir) = folder() else {
        eprintln!("OSCILLA_GB_TESTS is not set; skipping");
        return;
    };
    let mut failures = Vec::new();
    for p in paths {
        let text = run(&dir.join(p), limit);
        eprintln!("{p}: {}", text.trim().replace('\n', " | "));
        if !text.contains("Passed") {
            failures.push(format!("{p}:\n{text}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_cpu_instruction_tests_pass() {
    let names = [
        "01-special",
        "02-interrupts",
        "03-op sp,hl",
        "04-op r,imm",
        "05-op rp",
        "06-ld r,r",
        "07-jr,jp,call,ret,rst",
        "08-misc instrs",
        "09-op r,r",
        "10-bit ops",
        "11-op a,(hl)",
    ];
    let paths: Vec<String> = names
        .iter()
        .map(|n| format!("cpu_instrs/individual/{n}.gb"))
        .collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    check(&refs, 400_000_000);
}

#[test]
fn the_timing_tests_pass() {
    check(&["instr_timing/instr_timing.gb"], 100_000_000);
}

/// The cartridges about wave RAM access while the channel plays (9, 10 and 12)
/// describe corruption and blocked access that the core does not emulate: a
/// sound driver gets plain wave RAM.
#[test]
fn the_sound_tests_pass() {
    let names = [
        "01-registers",
        "02-len ctr",
        "03-trigger",
        "04-sweep",
        "05-sweep details",
        "06-overflow on trigger",
        "07-len sweep period sync",
        "08-len ctr during power",
        "11-regs after power",
    ];
    let paths: Vec<String> = names
        .iter()
        .map(|n| format!("dmg_sound/rom_singles/{n}.gb"))
        .collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    check(&refs, 400_000_000);
}

#[test]
fn the_memory_timing_tests_pass() {
    check(
        &[
            "mem_timing/individual/01-read_timing.gb",
            "mem_timing/individual/02-write_timing.gb",
            "mem_timing/individual/03-modify_timing.gb",
        ],
        100_000_000,
    );
}
