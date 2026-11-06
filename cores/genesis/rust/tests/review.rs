//! Regressions for the defects that the review of 0.6.0 found.

mod common;

use common::*;
use oscilla_genesis::{Chip, Machine, Observer};
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<(Chip, u16, u8, u64)>>>;

struct Recorder(Log);

impl Observer for Recorder {
    fn register_write(&mut self, chip: Chip, address: u16, value: u8, sample: u64) {
        self.0.borrow_mut().push((chip, address, value, sample));
    }
}

fn machine(bytes: &[u8]) -> Machine {
    let mut m = Machine::new(&parse(bytes), 44_100).unwrap();
    m.set_highpass_hz(0.0);
    m
}

/// Channel 3 of the first YM2612 in its special mode, with a sine on the
/// first operator and one note.
fn channel_three_special() -> Vec<u8> {
    let mut s = Vec::new();
    let mut w = |part: u8, reg: u8, val: u8| s.extend([0x52 + part, reg, val]);
    for (offset, level) in [(0u8, 0u8), (4, 127), (8, 127), (12, 127)] {
        w(0, 0x32 + offset, 0x01);
        w(0, 0x42 + offset, level);
        w(0, 0x52 + offset, 0x1F);
        w(0, 0x62 + offset, 0);
        w(0, 0x72 + offset, 0);
        w(0, 0x82 + offset, 0x0F);
    }
    w(0, 0xB2, 0x07);
    w(0, 0xB6, 0xC0);
    w(0, 0x27, 0x40);
    for reg in [0xA8, 0xA9, 0xAA] {
        w(0, reg + 4, 0x2C);
        w(0, reg, 0x40 + (reg & 3) * 0x10);
    }
    w(0, 0x28, 0xF2);
    s
}

#[test]
fn writes_to_the_second_port_do_not_change_the_special_frequencies_of_channel_three() {
    let mut plain = channel_three_special();
    plain.extend(wait(4000));
    plain.push(0x66);
    let mut port_one = channel_three_special();
    port_one.extend([0x53, 0xA9, 0xFF, 0x53, 0xA8, 0x01]);
    port_one.extend(wait(4000));
    port_one.push(0x66);
    let a = render(&mut machine(&file(false, true, &plain, None)), 4000);
    let b = render(&mut machine(&file(false, true, &port_one, None)), 4000);
    assert!(a.iter().any(|s| s.abs() > 500));
    assert_eq!(digest(&a), digest(&b));
}

#[test]
fn a_stream_that_gets_its_rate_late_does_not_write_the_missed_samples() {
    // A bank of two bytes and a stream that starts with no rate, then waits
    // 10 s before it is given 50,000 writes a second.
    let mut s = vec![0x67, 0x66, 0x00, 2, 0, 0, 0, 0x11, 0x22];
    s.extend([0x90, 0, 0x02, 0, 0x2A]);
    s.extend([0x91, 0, 0, 1, 0]);
    s.extend([0x93, 0, 0, 0, 0, 0, 0x83, 0, 0, 0, 0]);
    for _ in 0..7 {
        s.extend(wait(63_000));
    }
    s.extend([0x92, 0, 0x50, 0xC3, 0, 0]);
    s.extend(wait(2000));
    s.push(0x66);
    let log = Log::default();
    let mut m = machine(&file(false, true, &s, None));
    m.set_observer(Some(Box::new(Recorder(Rc::clone(&log)))));
    let started = std::time::Instant::now();
    render(&mut m, 450_000);
    assert!(started.elapsed().as_secs() < 5);
    let writes = log.borrow();
    // Every write comes after the command that gave the rate, at 441,000
    // samples, and the 2,000 samples after it hold about 2,000 of them.
    assert!(
        writes.iter().all(|w| w.3 >= 441_000),
        "{:?}",
        writes.first()
    );
    assert!(
        (1500..=2500).contains(&writes.len()),
        "{} writes",
        writes.len()
    );
}

#[test]
fn a_header_with_the_largest_chip_clocks_renders_in_bounded_time() {
    let mut s = Vec::new();
    s.extend([0x50, 0x90, 0x50, 0x00, 0x50, 0x0F]);
    s.extend([0x52, 0x28, 0xF0]);
    s.extend(wait(4410));
    s.push(0x66);
    let mut f = file(true, true, &s, None);
    f[0x0C..0x10].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    f[0x2C..0x30].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    f[0x10..0x14].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    let started = std::time::Instant::now();
    let out = render(&mut machine(&f), 4410);
    assert!(started.elapsed().as_secs() < 10);
    assert_eq!(out.len(), 2 * 4410);
}

#[test]
fn a_noise_feedback_of_zero_in_a_new_file_means_the_default() {
    // The same noise with the fields filled in and with them left at zero.
    let mut s = vec![0x50, 0xE4, 0x50, 0xF0];
    s.extend(wait(8000));
    s.push(0x66);
    let filled = file(true, false, &s, None);
    let mut zeros = filled.clone();
    zeros[0x28..0x2A].copy_from_slice(&[0, 0]);
    zeros[0x2A] = 0;
    let a = render(&mut machine(&filled), 8000);
    let b = render(&mut machine(&zeros), 8000);
    assert!(crossings(&a) > 100);
    assert_eq!(digest(&a), digest(&b));
}
