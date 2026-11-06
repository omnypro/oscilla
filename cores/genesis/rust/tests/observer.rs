//! An observer hears every register write that a chip gets, and changes nothing.

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
    Machine::new(&parse(bytes), 44_100).unwrap()
}

fn observed(bytes: &[u8]) -> (Machine, Log) {
    let log = Log::default();
    let mut m = machine(bytes);
    m.set_observer(Some(Box::new(Recorder(Rc::clone(&log)))));
    (m, log)
}

/// Marks the chips as dual (bit 30 of the clock word).
fn dual(mut f: Vec<u8>) -> Vec<u8> {
    f[0x0F] |= 0x40;
    f[0x2F] |= 0x40;
    f
}

#[test]
fn every_write_is_reported_with_its_chip_address_value_and_time() {
    let mut s = vec![0x50, 0x9F, 0x4F, 0xFE, 0x30, 0x90, 0x52, 0x28, 0xF0];
    s.extend(wait(441));
    s.extend([0x53, 0xB4, 0xC0, 0xA2, 0x2B, 0x80, 0xA3, 0xA4, 0x22]);
    s.extend(wait(441));
    s.extend([0x80]);
    s.push(0x66);
    let (mut m, log) = observed(&dual(file(true, true, &s, None)));
    render(&mut m, 1500);
    let got = log.borrow().clone();
    assert_eq!(
        got,
        vec![
            (Chip::Sn76489, 0, 0x9F, 0),
            (Chip::Sn76489, 1, 0xFE, 0),
            (Chip::Sn76489Second, 0, 0x90, 0),
            (Chip::Ym2612, 0x028, 0xF0, 0),
            (Chip::Ym2612, 0x1B4, 0xC0, 441),
            (Chip::Ym2612Second, 0x02B, 0x80, 441),
            (Chip::Ym2612Second, 0x1A4, 0x22, 441),
            (Chip::Ym2612, 0x02A, 0x80, 882),
        ]
    );
}

#[test]
fn a_ym2413_is_reported_with_its_register() {
    let mut f = file(
        false,
        false,
        &[0x51, 0x30, 0x21, 0xA1, 0x0E, 0x20, 0x66],
        None,
    );
    f[0x10..0x14].copy_from_slice(&(3_579_545u32 | 1 << 30).to_le_bytes());
    let (mut m, log) = observed(&f);
    render(&mut m, 100);
    assert_eq!(
        *log.borrow(),
        vec![
            (Chip::Ym2413, 0x30, 0x21, 0),
            (Chip::Ym2413Second, 0x0E, 0x20, 0),
        ]
    );
}

#[test]
fn a_write_to_a_chip_the_file_lacks_is_not_reported() {
    let (mut m, log) = observed(&file(
        true,
        false,
        &[0x52, 0x28, 0xF0, 0x50, 0x9F, 0x66],
        None,
    ));
    render(&mut m, 100);
    assert_eq!(*log.borrow(), vec![(Chip::Sn76489, 0, 0x9F, 0)]);
}

#[test]
fn the_dac_stream_writes_are_reported_at_their_times() {
    // A data bank of two bytes, a stream of the YM2612's register 0x2A at
    // 11,025 Hz that starts at once and writes both bytes.
    let mut s = vec![0x67, 0x66, 0x00, 2, 0, 0, 0, 0x11, 0x22];
    s.extend([0x90, 0, 0x02, 0, 0x2A]);
    s.extend([0x91, 0, 0, 1, 0]);
    s.extend([0x92, 0, 0x01, 0x2B, 0, 0]);
    s.extend([0x93, 0, 0, 0, 0, 0, 1, 2, 0, 0, 0]);
    s.extend(wait(441));
    s.push(0x66);
    let (mut m, log) = observed(&file(false, true, &s, None));
    render(&mut m, 500);
    let got = log.borrow().clone();
    assert_eq!(
        got,
        vec![
            (Chip::Ym2612, 0x02A, 0x11, 0),
            (Chip::Ym2612, 0x02A, 0x22, 4),
        ]
    );
}

#[test]
fn an_observer_does_not_change_the_sound() {
    let mut s = Vec::new();
    for _ in 0..40 {
        s.extend([0x50, 0x90, 0x52, 0x28, 0xF0, 0x50, 0x80, 0x52, 0xA0, 0x40]);
        s.extend(wait(300));
    }
    s.push(0x66);
    let f = file(true, true, &s, None);
    let plain = render(&mut machine(&f), 12_000);
    let (mut m, log) = observed(&f);
    let seen = render(&mut m, 12_000);
    assert_eq!(digest(&plain), digest(&seen));
    assert_eq!(log.borrow().len(), 40 * 4);
}

#[test]
fn reset_and_restore_keep_the_observer_and_a_copy_has_none() {
    let f = file(true, false, &[0x50, 0x9F, 0x66], None);
    let (mut m, log) = observed(&f);
    render(&mut m, 10);
    assert_eq!(log.borrow().len(), 1);
    m.reset();
    render(&mut m, 10);
    assert_eq!(log.borrow().len(), 2);
    let saved = m.clone();
    m.reset();
    assert!(m.restore(&saved));
    m.reset();
    render(&mut m, 10);
    assert_eq!(log.borrow().len(), 3);
    let mut copy = m.clone();
    copy.reset();
    render(&mut copy, 10);
    assert_eq!(log.borrow().len(), 3);
    m.set_observer(None);
    m.reset();
    render(&mut m, 10);
    assert_eq!(log.borrow().len(), 3);
}
