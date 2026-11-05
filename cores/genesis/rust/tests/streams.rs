mod common;

use common::*;
use oscilla_genesis::Machine;

/// The YM2612 with the DAC on, then `body`.
fn dac_on(body: &[u8]) -> Vec<u8> {
    let mut s = vec![0x52, 0x2B, 0x80];
    s.extend_from_slice(body);
    s
}

/// A data block of bank 0.
fn block(data: &[u8]) -> Vec<u8> {
    let mut b = vec![0x67, 0x66, 0x00];
    b.extend((data.len() as u32).to_le_bytes());
    b.extend_from_slice(data);
    b
}

/// Sets up stream 0 for the DAC register at `hz`.
fn setup(hz: u32) -> Vec<u8> {
    let mut s = vec![0x90, 0, 0x02, 0, 0x2A, 0x91, 0, 0, 1, 0, 0x92, 0];
    s.extend(hz.to_le_bytes());
    s
}

fn start(at: u32, mode: u8, length: u32) -> Vec<u8> {
    let mut s = vec![0x93, 0];
    s.extend(at.to_le_bytes());
    s.push(mode);
    s.extend(length.to_le_bytes());
    s
}

/// The frames that the resampler delays a step by.
const DELAY: usize = 9;

/// Renders the left channel and returns the sign at the middle of each
/// interval of a 4410 Hz stream (ten frames).
fn signs(stream: &[u8], intervals: usize) -> Vec<i32> {
    let v = parse(&file(false, true, stream, None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let out = left(&render(&mut m, 10 * intervals + DELAY + 5));
    (0..intervals)
        .map(|i| i32::from(out[10 * i + DELAY + 5]).signum())
        .collect()
}

#[test]
fn a_stream_plays_its_bytes_at_its_rate() {
    let mut s = block(&[0xC0, 0x40, 0xC0, 0x40, 0xFF, 0x00]);
    s.extend(dac_on(&setup(4410)));
    s.extend(start(0, 3, 0));
    s.extend(wait(200));
    s.push(0x66);
    assert_eq!(signs(&s, 8), [1, -1, 1, -1, 1, -1, -1, -1]);
}

#[test]
fn a_length_in_commands_ends_the_stream() {
    let mut s = block(&[0xC0, 0xC0, 0xC0, 0xC0, 0xC0, 0xC0]);
    s.extend(dac_on(&setup(4410)));
    s.extend(start(0, 1, 3));
    s.extend(wait(200));
    s.push(0x66);
    // The DAC keeps its last value after the third write.
    assert_eq!(signs(&s, 6), [1, 1, 1, 1, 1, 1]);
    let mut s = block(&[0xC0, 0x40, 0xC0, 0x40, 0xC0, 0x40]);
    s.extend(dac_on(&setup(4410)));
    s.extend(start(0, 1, 3));
    s.extend(wait(200));
    s.push(0x66);
    assert_eq!(signs(&s, 6), [1, -1, 1, 1, 1, 1]);
}

#[test]
fn a_length_in_milliseconds_counts_writes_at_the_rate() {
    // 4410 Hz for 1 ms is four writes (4.41 rounds down).
    let mut s = block(&[0xC0, 0x40, 0xC0, 0x40, 0xC0, 0x40]);
    s.extend(dac_on(&setup(4410)));
    s.extend(start(0, 2, 1));
    s.extend(wait(200));
    s.push(0x66);
    assert_eq!(signs(&s, 6), [1, -1, 1, -1, -1, -1]);
}

#[test]
fn a_loop_starts_again_and_stop_ends_it() {
    let mut s = block(&[0xC0, 0x40]);
    s.extend(dac_on(&setup(4410)));
    s.extend(start(0, 0x83, 0));
    s.extend(wait(35));
    s.extend([0x94, 0]);
    s.extend(wait(165));
    s.push(0x66);
    let got = signs(&s, 8);
    assert_eq!(got[..4], [1, -1, 1, -1]);
    // The last write before the stop, at 30 frames, is the second of a pair.
    assert!(got[4..].iter().all(|v| *v == -1), "{got:?}");
}

#[test]
fn reverse_mode_walks_backwards() {
    let mut s = block(&[0xC0, 0xC0, 0x40, 0x40]);
    s.extend(dac_on(&setup(4410)));
    s.extend(start(3, 0x11, 4));
    s.extend(wait(200));
    s.push(0x66);
    assert_eq!(signs(&s, 4), [-1, -1, 1, 1]);
}

#[test]
fn the_fast_call_plays_a_block_with_its_loop_flag() {
    let mut s = block(&[0xC0, 0xC0]);
    s.extend(block(&[0x40, 0xC0]));
    s.extend(dac_on(&setup(4410)));
    s.extend([0x95, 0, 1, 0, 0]);
    s.extend(wait(200));
    s.push(0x66);
    assert_eq!(signs(&s, 4), [-1, 1, 1, 1]);
    let mut s = block(&[0xC0, 0xC0]);
    s.extend(block(&[0x40, 0xC0]));
    s.extend(dac_on(&setup(4410)));
    s.extend([0x95, 0, 1, 0, 1]);
    s.extend(wait(200));
    s.push(0x66);
    assert_eq!(signs(&s, 4), [-1, 1, -1, 1]);
}

#[test]
fn a_step_of_two_reads_every_other_byte() {
    let mut s = block(&[0xC0, 0x40, 0xC0, 0x40, 0xC0, 0x40]);
    s.extend([0x90, 0, 0x02, 0, 0x2A, 0x91, 0, 0, 2, 1, 0x92, 0]);
    s.extend(4410u32.to_le_bytes());
    s.extend(dac_on(&[]));
    s.extend(start(0, 3, 0));
    s.extend(wait(200));
    s.push(0x66);
    // Base 1 and step 2 read the 0x40 bytes.
    assert_eq!(signs(&s, 3), [-1, -1, -1]);
}

#[test]
fn a_new_rate_continues_from_the_next_write() {
    let mut s = block(&[0xC0, 0x40, 0xC0, 0x40, 0xC0, 0x40, 0xC0, 0x40]);
    s.extend(dac_on(&setup(4410)));
    s.extend(start(0, 3, 0));
    s.extend(wait(20));
    // The writes at 0, 10 and 20 frames come at 4410 Hz. The new rate holds
    // from the next one, at 30 frames, with five frames to each.
    s.extend([0x92, 0]);
    s.extend(8820u32.to_le_bytes());
    s.extend(wait(200));
    s.push(0x66);
    let v = parse(&file(false, true, &s, None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    m.set_highpass_hz(0.0);
    let out = left(&render(&mut m, 60));
    let sign = |i: usize| i32::from(out[i]).signum();
    let at = [14, 24, 34, 41, 46, 51].map(sign);
    assert_eq!(at, [1, -1, 1, -1, 1, -1]);
}

#[test]
fn bad_streams_do_nothing_and_cannot_hang() {
    // A stream that was never set up, one with no rate, an empty bank, a
    // looping stream over an empty bank and an absurd rate.
    let mut s = dac_on(&[]);
    s.extend(start(0, 3, 0));
    s.extend([0x90, 1, 0x02, 0, 0x2A]);
    s.extend([0x93, 1, 0, 0, 0, 0, 3, 0, 0, 0, 0]);
    s.extend(setup(0xFFFF_FFFF));
    s.extend(start(0, 0x83, 0));
    s.extend(wait(1000));
    s.push(0x66);
    let v = parse(&file(false, true, &s, None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    render(&mut m, 2000);
    let mut s = block(&[0x10, 0x20]);
    s.extend(dac_on(&setup(0xFFFF_FFFF)));
    s.extend(start(0, 0x83, 0));
    s.extend(wait(100));
    s.push(0x66);
    let v = parse(&file(false, true, &s, None));
    let mut m = Machine::new(&v, 44_100).unwrap();
    render(&mut m, 200);
}

#[test]
fn the_rendering_with_a_stream_is_the_same_at_any_chunk_size() {
    let mut s = block(&(0..=255u8).collect::<Vec<_>>());
    s.extend(dac_on(&setup(11_025)));
    s.extend(start(0, 0x83, 0));
    s.extend(wait(30_000));
    s.push(0x66);
    let v = parse(&file(false, true, &s, None));
    let whole = render(&mut Machine::new(&v, 48_000).unwrap(), 30_000);
    let mut m = Machine::new(&v, 48_000).unwrap();
    let mut parts = Vec::new();
    for n in [1usize, 7, 4096, 9999, 1, 15_896] {
        parts.extend(render(&mut m, n));
    }
    assert_eq!(whole, parts);
}
