//! An observer hears every store to the sound generator, and changes nothing.

use oscilla_hes::Hes;
use oscilla_pce::{Machine, Observer};
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<(u8, u8, u8, u64)>>>;

struct Recorder(Log);

impl Observer for Recorder {
    fn psg_write(&mut self, channel: u8, register: u8, value: u8, sample: u64) {
        self.0.borrow_mut().push((channel, register, value, sample));
    }
}

/// A file whose request routine runs `code` from 0xE100.
fn file(code: &[u8]) -> Hes {
    let mut bank = vec![0u8; 0x2000];
    bank[0x100..0x100 + code.len()].copy_from_slice(code);
    bank[0x1FF0] = 0x40;
    for v in [0x1FF6, 0x1FF8, 0x1FFA, 0x1FFC] {
        bank[v..v + 2].copy_from_slice(&[0xF0, 0xFF]);
    }
    let mut f = b"HESM".to_vec();
    f.extend([0, 0, 0x00, 0xE1, 0xFF, 0xF8, 0, 0, 1, 2, 3, 0]);
    f.extend(b"DATA");
    f.extend((bank.len() as u32).to_le_bytes());
    f.extend(0u32.to_le_bytes());
    f.extend([0; 4]);
    f.extend(bank);
    Hes::parse(&f).unwrap()
}

fn sta(address: u16, value: u8) -> Vec<u8> {
    vec![0xA9, value, 0x8D, address as u8, (address >> 8) as u8]
}

fn machine(hes: &Hes) -> Machine {
    let mut m = Machine::new(hes, 44_100, 0).unwrap();
    m.set_highpass_hz(0.0);
    m
}

fn observed(hes: &Hes) -> (Machine, Log) {
    let log = Log::default();
    let mut m = machine(hes);
    m.set_observer(Some(Box::new(Recorder(Rc::clone(&log)))));
    (m, log)
}

fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; 2 * frames];
    m.render_s16(&mut out);
    out
}

/// Stores to the channel registers of channels 3 and then 1.
fn stores() -> Hes {
    let mut code = Vec::new();
    code.extend(sta(0x0800, 2));
    code.extend(sta(0x0802, 0x10));
    code.extend(sta(0x0801, 0xFF));
    code.extend(sta(0x0800, 0));
    code.extend(sta(0x0805, 0xFF));
    code.extend(sta(0x0804, 0xDF));
    code.extend(sta(0x0806, 31));
    code.extend(sta(0x0809, 0x01));
    code.push(0x60);
    file(&code)
}

#[test]
fn every_store_is_reported_with_its_channel_register_value_and_time() {
    let (mut m, log) = observed(&stores());
    render(&mut m, 500);
    assert_eq!(
        *log.borrow(),
        vec![
            (2, 0, 2, 0),
            (2, 2, 0x10, 0),
            (2, 1, 0xFF, 0),
            (0, 0, 0, 0),
            (0, 5, 0xFF, 0),
            (0, 4, 0xDF, 0),
            (0, 6, 31, 1),
            (0, 9, 0x01, 1),
        ]
    );
}

#[test]
fn a_store_late_in_the_song_has_its_time_in_samples() {
    // Sixteen passes of a 256-step countdown, about 20,000 cycles of the low
    // speed (12 master clocks each): about 500 samples.
    let mut code = vec![0xA2, 0xFF, 0xA0, 0x10, 0xCA, 0xD0, 0xFD, 0x88, 0xD0, 0xFA];
    code.extend(sta(0x0800, 1));
    code.push(0x60);
    let (mut m, log) = observed(&file(&code));
    render(&mut m, 4000);
    let got = log.borrow().clone();
    assert_eq!(got.len(), 1);
    assert!((100..1500).contains(&got[0].3), "{:?}", got[0]);
}

#[test]
fn an_observer_does_not_change_the_sound() {
    let hes = stores();
    let plain = render(&mut machine(&hes), 3000);
    let (mut m, log) = observed(&hes);
    assert_eq!(render(&mut m, 3000), plain);
    assert_eq!(log.borrow().len(), 8);
}

#[test]
fn reset_and_restore_keep_the_observer_and_a_copy_has_none() {
    let (mut m, log) = observed(&stores());
    render(&mut m, 100);
    assert_eq!(log.borrow().len(), 8);
    m.reset();
    render(&mut m, 100);
    assert_eq!(log.borrow().len(), 16);
    let saved = m.clone();
    m.reset();
    assert!(m.restore(&saved));
    m.reset();
    render(&mut m, 100);
    assert_eq!(log.borrow().len(), 24);
    let mut copy = m.clone();
    copy.reset();
    render(&mut copy, 100);
    assert_eq!(log.borrow().len(), 24);
    m.set_observer(None);
    m.reset();
    render(&mut m, 100);
    assert_eq!(log.borrow().len(), 24);
}
