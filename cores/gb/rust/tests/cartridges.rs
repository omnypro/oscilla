//! Public test cartridges for the CPU, the timer and the sound unit. They are
//! not part of the repository: point `OSCILLA_GB_TESTS` at a folder that holds
//! them and a `manifest.txt` that lists them, one per line, as a path relative
//! to the folder and a limit in cycles. A cartridge reports on the serial port
//! or in cart RAM and ends its text with "Passed" or "Failed". A line that
//! starts with `#` is a comment.

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

#[test]
fn the_listed_cartridges_pass() {
    let Some(dir) = folder() else {
        eprintln!("OSCILLA_GB_TESTS is not set; skipping");
        return;
    };
    let manifest = std::fs::read_to_string(dir.join("manifest.txt")).unwrap();
    let mut failures = Vec::new();
    let mut ran = 0;
    for line in manifest.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (path, limit) = line.rsplit_once(' ').expect("path and cycle limit");
        let text = run(&dir.join(path), limit.parse().expect("a cycle limit"));
        eprintln!("{path}: {}", text.trim().replace('\n', " | "));
        if !text.contains("Passed") {
            failures.push(format!("{path}:\n{text}"));
        }
        ran += 1;
    }
    assert!(ran > 0, "the manifest lists no cartridge");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
