//! A comparison with renders from a reference player. The files are not in the
//! repository. `OSCILLA_SNES_REF` names a folder with NAME.spc and NAME.wav
//! pairs (32 kHz, 16-bit stereo). The test prints how close each render is
//! and skips without the variable.

use oscilla_snes::{Machine, Spc};

fn wav_samples(bytes: &[u8]) -> Vec<i16> {
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        if &bytes[at..at + 4] == b"data" {
            let data = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
            return data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        }
        at += 8 + size + (size & 1);
    }
    Vec::new()
}

#[test]
fn renders_against_the_reference_set() {
    let Some(dir) = std::env::var_os("OSCILLA_SNES_REF") else {
        return;
    };
    let only = std::env::var("OSCILLA_SNES_ONLY").ok();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "spc"))
        .collect();
    names.sort();
    for path in names {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if only.as_ref().is_some_and(|o| !name.starts_with(o.as_str())) {
            continue;
        }
        let spc = Spc::parse(&std::fs::read(&path).unwrap()).unwrap();
        let want = wav_samples(&std::fs::read(path.with_extension("wav")).unwrap());
        let frames = want.len() / 2;
        let shift: i64 = std::env::var("OSCILLA_SNES_SHIFT").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let gain_in: f64 = std::env::var("OSCILLA_SNES_GAIN").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let mut m = Machine::new(&spc, 32000).unwrap();
        let mut got = if shift >= 0 {
            let mut g = m.dsp_samples(frames + shift as usize);
            g.drain(..2 * shift as usize);
            g
        } else {
            let mut g = vec![0i16; 2 * (-shift) as usize];
            g.extend(m.dsp_samples(frames));
            g.truncate(2 * frames);
            g
        };
        for v in got.iter_mut() {
            *v = (f64::from(*v) * gain_in).round().clamp(-32768.0, 32767.0) as i16;
        }
        let (mut err, mut energy, mut cross, mut own, mut exact) = (0f64, 0f64, 0f64, 0f64, 0usize);
        for (g, w) in got.iter().zip(&want) {
            let (g, w) = (f64::from(*g), f64::from(*w));
            err += (g - w) * (g - w);
            energy += w * w;
            own += g * g;
            cross += g * w;
            if g == w {
                exact += 1;
            }
        }
        if let Ok(n) = std::env::var("OSCILLA_SNES_DUMP") {
            let n: usize = n.parse().unwrap_or(20);
            let from: usize = std::env::var("OSCILLA_SNES_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
            let mut shown = 0;
            for i in from..frames {
                if got[2 * i] != want[2 * i] || got[2 * i + 1] != want[2 * i + 1] {
                    println!(
                        "  frame {i}: ours ({}, {}) ref ({}, {})",
                        got[2 * i], got[2 * i + 1], want[2 * i], want[2 * i + 1]
                    );
                    shown += 1;
                    if shown >= n {
                        break;
                    }
                }
            }
        }
        if std::env::var("OSCILLA_SNES_LAG").is_ok() {
            // Correlate the left channels over a window for a range of lags.
            let from = 64_000usize.min(frames / 2);
            let len = 128_000usize.min(frames - from - 32);
            let mut best = (0i64, -2.0f64);
            for lag in -12i64..=12 {
                let (mut xy, mut xx, mut yy) = (0f64, 0f64, 0f64);
                for i in from..from + len {
                    let g = f64::from(got[2 * (i as i64 + lag) as usize]);
                    let w = f64::from(want[2 * i]);
                    xy += g * w;
                    xx += g * g;
                    yy += w * w;
                }
                let r = xy / (xx * yy).sqrt().max(1e-9);
                if r > best.1 {
                    best = (lag, r);
                }
            }
            println!("  best lag {} (ours[i+lag] vs ref[i]), correlation {:.4}", best.0, best.1);
        }
        if std::env::var("OSCILLA_SNES_WIN").is_ok() {
            for w in 0..(frames / 32000).min(30) {
                let from = w * 32000 + 16;
                let mut best = (0i64, -2.0f64, 0.0f64);
                for lag in -8i64..=8 {
                    let (mut xy, mut xx, mut yy) = (0f64, 0f64, 0f64);
                    for i in from..from + 31000 {
                        let g = f64::from(got[2 * (i as i64 + lag) as usize]);
                        let r = f64::from(want[2 * i]);
                        xy += g * r;
                        xx += g * g;
                        yy += r * r;
                    }
                    let c = xy / (xx * yy).sqrt().max(1e-9);
                    if c > best.1 {
                        best = (lag, c, xy / xx.max(1e-9));
                    }
                }
                print!("[s{w}: lag {} r {:.3} g {:.2}] ", best.0, best.1, best.2);
            }
            println!();
        }
        let gain = if own > 0.0 { cross / own } else { 0.0 };
        println!(
            "{name}: ref rms {:.1}, our rms {:.1}, error rms {:.1} ({:.1} dB), best gain {:.4}, exact {:.1}%",
            (energy / want.len() as f64).sqrt(),
            (own / got.len() as f64).sqrt(),
            (err / want.len() as f64).sqrt(),
            10.0 * (err.max(1e-9) / energy.max(1e-9)).log10(),
            gain,
            100.0 * exact as f64 / want.len() as f64
        );
    }
}

struct Writes(std::rc::Rc<std::cell::RefCell<Vec<(u64, u8, u8)>>>);

impl oscilla_snes::Observer for Writes {
    fn dsp_write(&mut self, cycle: u64, register: u8, value: u8) {
        self.0.borrow_mut().push((cycle, register, value));
    }
    fn port_write(&mut self, _cycle: u64, _port: u8, _value: u8) {}
}

fn log_writes(text: &str) -> Vec<(u64, u8, u8)> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((
                f[1].parse().ok()?,
                u8::from_str_radix(f[2], 16).ok()?,
                u8::from_str_radix(f[3], 16).ok()?,
            ))
        })
        .collect()
}

#[test]
fn dsp_writes_against_the_reference_logs() {
    let Some(dir) = std::env::var_os("OSCILLA_SNES_REF") else {
        return;
    };
    let only = std::env::var("OSCILLA_SNES_ONLY").ok();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "spc"))
        .collect();
    names.sort();
    for path in names {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if only.as_ref().is_some_and(|o| !name.starts_with(o.as_str())) {
            continue;
        }
        let spc = Spc::parse(&std::fs::read(&path).unwrap()).unwrap();
        let log = log_writes(&std::fs::read_to_string(path.with_extension("log.txt")).unwrap());
        let mut m = Machine::new(&spc, 32000).unwrap();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        m.set_observer(Some(Box::new(Writes(seen.clone()))));
        m.run_cycles(960_000 * 32);
        let ours = seen.borrow();
        let ours: Vec<_> = ours.iter().filter(|w| w.0 < 960_000 * 32).collect();
        let same = ours.iter().zip(&log).take_while(|(a, b)| **a == *b).count();
        let same_stream = ours.iter().zip(&log).take_while(|(a, b)| a.1 == b.1 && a.2 == b.2).count();
        println!(
            "{name}: ours {} writes, ref {}; identical (with clock) for the first {same}, (registers and values) for the first {same_stream}",
            ours.len(),
            log.len()
        );
        let n = same_stream;
        if n > 0 {
            let d = |i: usize| ours[i].0 as i64 - log[i].0 as i64;
            let mut min = i64::MAX;
            let mut max = i64::MIN;
            for i in 0..n {
                min = min.min(d(i));
                max = max.max(d(i));
            }
            println!("  clock offset ours - ref: first {}, middle {}, last {}, min {min}, max {max}", d(0), d(n / 2), d(n - 1));
        }
        if same < ours.len().min(log.len()) && std::env::var("OSCILLA_SNES_DUMP").is_ok() {
            let at = if std::env::var("OSCILLA_SNES_STREAM").is_ok() { same_stream.saturating_sub(6) } else { same.saturating_sub(3) };
            for i in at..(at + 10).min(ours.len().min(log.len())) {
                println!("  #{i}: ours {:?} ref {:?}", ours[i], log[i]);
            }
        }
    }
}

#[test]
fn timer_phase_scan() {
    let Some(dir) = std::env::var_os("OSCILLA_SNES_REF") else {
        return;
    };
    let files: Vec<String> = std::env::var("OSCILLA_SNES_SCAN").unwrap_or_default().split(',').map(String::from).collect();
    for prefix in files {
        if prefix.is_empty() {
            continue;
        }
        let path = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "spc") && p.file_name().unwrap().to_string_lossy().starts_with(&prefix)).unwrap();
        let spc = Spc::parse(&std::fs::read(&path).unwrap()).unwrap();
        let log = log_writes(&std::fs::read_to_string(path.with_extension("log.txt")).unwrap());
        let mut best: Vec<(usize, u64, i64)> = Vec::new();
        for phase in 0..128u64 {
            let mut m = Machine::new(&spc, 32000).unwrap();
            m.set_timer_phase(phase);
            let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            m.set_observer(Some(Box::new(Writes(seen.clone()))));
            m.run_cycles(30_000_000);
            let ours = seen.borrow();
            let n = ours.len().min(log.len());
            if n == 0 { continue; }
            let off = ours[0].0 as i64 - log[0].0 as i64;
            let _ = off;
            let same = (0..n).take_while(|&i| ours[i].1 == log[i].1 && ours[i].2 == log[i].2).count();
            let exact = (0..same).filter(|&i| ours[i].0 == log[i].0).count();
            let same = exact;
            best.push((same, phase, off));
        }
        best.sort_by(|a, b| b.cmp(a));
        println!("{prefix}: best (matching run, phase, offset): {:?}", &best[..4.min(best.len())]);
        let mut by_phase = best.clone(); by_phase.sort_by_key(|b| b.1); println!("by phase: {:?}", by_phase.iter().map(|b| (b.1, b.0)).collect::<Vec<_>>());
    }
}
