//! Public test cartridges for the CPU. They are not part of the repository:
//! point `OSCILLA_NES_TESTS` at a folder that holds them and a `manifest.txt`
//! that lists them, one per line, as a path relative to the folder and a limit
//! in cycles. A cartridge that reports in RAM leaves a signature at 0x6001, a
//! status at 0x6000 (0x80 while it runs, 0 for a pass) and text from 0x6004.
//! A line may end with the path of a log of the CPU's state before each
//! instruction; the cartridge then runs from 0xC000 and the log must match.
//! A line that starts with `#` is a comment.

mod common;

use oscilla_nes::Machine;
use std::path::{Path, PathBuf};

fn folder() -> Option<PathBuf> {
    std::env::var_os("OSCILLA_NES_TESTS").map(PathBuf::from)
}

/// The program of an iNES image with a single 16 or 32 KiB bank and no
/// mapper.
fn program(image: &[u8]) -> Result<&[u8], String> {
    if image.len() < 16 || &image[..4] != b"NES\x1A" {
        return Err("not an iNES image".into());
    }
    let mapper = image[6] >> 4 | image[7] & 0xF0;
    let banks = usize::from(image[4]);
    if mapper != 0 || !(1..=2).contains(&banks) {
        return Err(format!(
            "mapper {mapper} with {banks} banks is not supported"
        ));
    }
    let start = 16 + if image[6] & 4 != 0 { 512 } else { 0 };
    image
        .get(start..start + banks * 0x4000)
        .ok_or_else(|| "the image is short".into())
}

/// The result that a cartridge leaves in RAM, as text that ends in "Passed" or
/// "Failed".
fn ram_result(m: &Machine) -> Option<String> {
    if [0xDE, 0xB0, 0x61] != [m.peek(0x6001), m.peek(0x6002), m.peek(0x6003)] {
        return None;
    }
    let status = m.peek(0x6000);
    if status == 0x80 {
        return None;
    }
    let text: Vec<u8> = (0x6004u16..0x8000)
        .map(|a| m.peek(a))
        .take_while(|&b| b != 0)
        .collect();
    let text = String::from_utf8_lossy(&text).into_owned();
    Some(if status == 0 {
        format!("{text}\nPassed")
    } else {
        format!("{text}\nFailed ({status})")
    })
}

fn run(rom: &Path, limit: u64) -> String {
    let image = std::fs::read(rom).unwrap();
    let program = match program(&image) {
        Ok(p) => p,
        Err(e) => return format!("Failed: {e}"),
    };
    let mut m = Machine::from_rom(program, 48000).unwrap();
    while m.cycles() < limit {
        m.run_cycles(1 << 18);
        if let Some(text) = ram_result(&m) {
            return text;
        }
    }
    "Failed: no result".into()
}

/// Runs a cartridge from 0xC000 and compares the CPU's state before each
/// instruction with the lines of a log. Each line holds the program counter,
/// then `A:`, `X:`, `Y:`, `P:`, `SP:` and `CYC:` fields.
fn run_log(rom: &Path, log: &Path) -> String {
    let image = std::fs::read(rom).unwrap();
    let program = match program(&image) {
        Ok(p) => p,
        Err(e) => return format!("Failed: {e}"),
    };
    let mut m = Machine::from_rom(program, 48000).unwrap();
    m.set_pc(0xC000);
    let text = std::fs::read_to_string(log).unwrap();
    let field = |line: &str, name: &str| -> u64 {
        let at = line.find(name).unwrap() + name.len();
        let end = line[at..]
            .find(|c: char| !c.is_ascii_hexdigit())
            .map_or(line.len(), |n| at + n);
        let radix = if name == "CYC:" { 10 } else { 16 };
        u64::from_str_radix(&line[at..end], radix).unwrap()
    };
    let mut n = 0;
    for line in text.lines().filter(|l| l.len() > 20) {
        let (pc, a, x, y, p, sp) = m.cpu_state();
        let want = (
            u64::from_str_radix(&line[..4], 16).unwrap(),
            field(line, "A:"),
            field(line, "X:"),
            field(line, "Y:"),
            field(line, "P:"),
            field(line, "SP:"),
            field(line, "CYC:"),
        );
        let got = (
            u64::from(pc),
            u64::from(a),
            u64::from(x),
            u64::from(y),
            u64::from(p),
            u64::from(sp),
            m.cycles(),
        );
        if got != want {
            return format!("Failed at line {}: got {got:x?}, want {want:x?}", n + 1);
        }
        m.step_instruction();
        n += 1;
    }
    format!("{n} lines matched\nPassed")
}

#[test]
fn the_listed_cartridges_pass() {
    let Some(dir) = folder() else {
        eprintln!("OSCILLA_NES_TESTS is not set; skipping");
        return;
    };
    let manifest = std::fs::read_to_string(dir.join("manifest.txt")).unwrap();
    let mut failures = Vec::new();
    let mut ran = 0;
    for line in manifest.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(' ');
        let rom = parts.next().unwrap();
        let limit: u64 = parts
            .next()
            .and_then(|l| l.parse().ok())
            .expect("a cycle limit");
        let text = match parts.next() {
            Some(log) => run_log(&dir.join(rom), &dir.join(log)),
            None => run(&dir.join(rom), limit),
        };
        eprintln!("{rom}: {}", text.trim().replace('\n', " | "));
        if !text.contains("Passed") {
            failures.push(format!("{rom}:\n{text}"));
        }
        ran += 1;
    }
    assert!(ran > 0, "the manifest lists no cartridge");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_harness_reads_a_result_that_a_program_leaves() {
    // A program that stores the signature, a pass status and "ok", then loops.
    let mut code = Vec::new();
    for (i, b) in [0xDEu8, 0xB0, 0x61].into_iter().enumerate() {
        code.extend(common::set(0x6001 + i as u16, b));
    }
    code.extend(common::set(0x6004, b'o'));
    code.extend(common::set(0x6005, b'k'));
    code.extend(common::set(0x6000, 0));
    code.extend([0x4C, (0x8000 + code.len()) as u8, 0x80]);
    let mut rom = vec![0xEAu8; 0x4000];
    rom[..code.len()].copy_from_slice(&code);
    rom[0x3FFD] = 0x80;
    let mut image = b"NES\x1A\x01\x00\x00\x00".to_vec();
    image.resize(16, 0);
    image.extend(rom);
    let path = std::env::temp_dir().join(format!("oscilla-nes-harness-{}.nes", std::process::id()));
    std::fs::write(&path, image).unwrap();
    let text = run(&path, 10_000);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(text, "ok\nPassed");
}
