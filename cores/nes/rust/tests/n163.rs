//! The Namco 163 expansion chip: its sound RAM and ports, and the channels
//! that it plays one at a time.

mod common;

use common::*;
use oscilla_nes::Machine;

/// Channel 8 alone: a ramp of eight samples, 0 to 7, at full volume, one
/// sample for each update.
fn ramp() -> Vec<u8> {
    let mut init = n163_ram(0, &[0x10, 0x32, 0x54, 0x76]);
    // Channel 8 is at 0x78: frequency 0x10000, length 8, volume 15, one channel.
    init.extend(n163_ram(
        0x78,
        &[0x00, 0x00, 0x00, 0x00, 0xF9, 0x00, 0x00, 0x0F],
    ));
    init
}

#[test]
fn the_data_port_steps_the_address_when_asked() {
    // Write four bytes from address 0 with the address stepping, then read them
    // back with it stepping again.
    let mut init = n163_ram(0, &[0xA1, 0xB2, 0xC3, 0xD4]);
    init.extend(set(0xF800, 0x80));
    init.extend([0xAD, 0x00, 0x48, 0x85, 0x00]);
    init.extend([0xAD, 0x00, 0x48, 0x85, 0x01]);
    init.extend([0xAD, 0x00, 0x48, 0x85, 0x02]);
    let mut m = n163_machine(&init, &[]);
    m.run_cycles(2000);
    assert_eq!((m.peek(0), m.peek(1), m.peek(2)), (0xA1, 0xB2, 0xC3));
}

#[test]
fn without_stepping_the_address_stays() {
    let mut init = set(0xF800, 0x05);
    init.extend(set(0x4800, 0x11));
    init.extend(set(0x4800, 0x22));
    init.extend(set(0xF800, 0x05));
    init.extend([0xAD, 0x00, 0x48, 0x85, 0x00]);
    let mut m = n163_machine(&init, &[]);
    m.run_cycles(1000);
    assert_eq!(m.peek(0), 0x22);
}

#[test]
fn the_address_stops_at_the_end_of_the_ram() {
    let mut init = n163_ram(0x7F, &[0x44, 0x55]);
    init.extend(set(0xF800, 0x7F));
    init.extend([0xAD, 0x00, 0x48, 0x85, 0x00]);
    let mut m = n163_machine(&init, &[]);
    m.run_cycles(1000);
    assert_eq!(m.peek(0), 0x55);
}

#[test]
fn a_channel_steps_through_its_waveform() {
    let mut m = n163_machine(&ramp(), &[]);
    m.run_cycles(400);
    let mut seen = Vec::new();
    for k in 30..90u64 {
        while m.cycles() < 15 * k + 7 {
            m.run_cycles(1);
        }
        seen.push(m.expansion_levels()[7]);
    }
    // The ramp falls from 120 in steps of 15 and starts again.
    for pair in seen.windows(2) {
        assert!(
            pair[1] == pair[0] - 15 || (pair[0] == 15 && pair[1] == 120),
            "{seen:?}"
        );
    }
    assert!(seen.contains(&120) && seen.contains(&15));
}

#[test]
fn a_waveform_with_no_frequency_holds_still() {
    let mut init = n163_ram(0, &[0x10, 0x32, 0x54, 0x76]);
    init.extend(n163_ram(0x78, &[0, 0, 0, 0, 0xF8, 0, 0, 0x0F]));
    let mut m = n163_machine(&init, &[]);
    m.run_cycles(2000);
    let held = m.expansion_levels()[7];
    m.run_cycles(2000);
    assert_eq!(m.expansion_levels()[7], held);
}

#[test]
fn the_channels_take_turns_from_channel_8_down() {
    // Two channels: 8 holds the top sample at full volume and 7 holds the
    // bottom sample at volume 10. Each shows its own output.
    let mut init = n163_ram(0, &[0xFF, 0x00]);
    // Channel 8: waveform at nibble 0 (value 15), length 4 (0xFC), no frequency.
    init.extend(n163_ram(0x78, &[0, 0, 0, 0, 0xFC, 0, 0, 0x1F]));
    // Channel 7: waveform at nibble 2 (value 0), volume 10.
    init.extend(n163_ram(0x70, &[0, 0, 0, 0, 0xFC, 0, 2, 0x0A]));
    let mut m = n163_machine(&init, &[]);
    m.run_cycles(3000);
    let l = m.expansion_levels();
    assert_eq!((l[7], l[6]), (105, 80));
}

fn swing(m: &mut Machine) -> f64 {
    let out = render(m, 24000);
    let tail: Vec<f64> = out[2 * 12000..]
        .iter()
        .step_by(2)
        .map(|&v| f64::from(v))
        .collect();
    let mean = |side: Vec<&f64>| side.iter().copied().sum::<f64>() / side.len() as f64;
    mean(tail.iter().filter(|&&v| v > 0.0).collect())
        - mean(tail.iter().filter(|&&v| v < 0.0).collect())
}

/// Channel 8 plays a 4-sample square wave (15, 15, 0, 0) at `freq` with the
/// given volume byte.
fn square(freq: u32, volume: u8) -> Vec<u8> {
    let mut init = n163_ram(0, &[0xFF, 0x00]);
    init.extend(n163_ram(
        0x78,
        &[
            freq as u8,
            0,
            (freq >> 8) as u8,
            0,
            0xFC | (freq >> 16) as u8,
            0,
            0,
            volume,
        ],
    ));
    init
}

#[test]
fn a_square_wave_plays_at_the_documented_pitch() {
    // f = n * p / (15 * 65536 * l * c) with p = 0x2000 (a quarter of a turn of
    // a 4-sample wave per 65536 units is not needed): choose p = 0x4000 so that
    // a 4-sample wave turns once in 16 updates, or 15 * 16 = 240 cycles.
    let mut m = n163_machine(&square(0x4000, 0x0F), &[]);
    let mut levels = Vec::new();
    while m.cycles() < 1_789_773 / 2 {
        m.run_cycles(8);
        levels.push(m.expansion_levels()[7]);
    }
    let edges = levels
        .windows(2)
        .filter(|w| w[0] != w[1] && w[1] == 105)
        .count();
    // 1789773 / 240 is 7457 Hz; half a second gives 3728 turns.
    assert!((3700..=3760).contains(&edges), "{edges}");
}

#[test]
fn the_channel_mask_reaches_the_channels() {
    let mut a = n163_machine(&square(0x4000, 0x0F), &[]);
    let mut b = n163_machine(&square(0x4000, 0x0F), &[]);
    assert_eq!(b.channel_mask(), 0xFFFFF);
    b.set_channel_mask(u32::MAX);
    assert_eq!(b.channel_mask(), 0xFFFFF);
    b.set_channel_mask(0xFFFFF & !(1 << 18));
    assert!(a.set_highpass_hz(1.0));
    assert!(b.set_highpass_hz(1.0));
    assert!(swing(&mut a) > 500.0);
    let out = render(&mut b, 24000);
    assert!(out[2 * 12000..].iter().all(|&v| v.abs() < 30));
}

#[test]
fn a_louder_volume_register_gives_a_louder_wave() {
    // Small volumes keep the wave inside the range of a sample.
    let mut high = n163_machine(&square(0x2000, 0x06), &[]);
    let mut low = n163_machine(&square(0x2000, 0x02), &[]);
    assert!(high.set_highpass_hz(1.0));
    assert!(low.set_highpass_hz(1.0));
    let ratio = swing(&mut low) / swing(&mut high);
    assert!((ratio - 2.0 / 6.0).abs() < 0.03, "{ratio}");
}

#[test]
fn stores_to_the_ports_do_nothing_without_the_chip() {
    let mut m = machine(&n163_ram(0, &[1, 2, 3]), &[]);
    m.run_cycles(2000);
    assert_eq!(m.expansion_levels(), [0; 8]);
}

#[test]
fn the_observer_sees_the_ports() {
    let mut m = n163_machine(&n163_ram(5, &[0x66]), &[]);
    let seen = watch(&mut m);
    m.run_cycles(200);
    let w = &seen.borrow().writes;
    assert!(w.iter().any(|x| x.1 == 0xF800 && x.2 == 0x85));
    assert!(w.iter().any(|x| x.1 == 0x4800 && x.2 == 0x66));
}

#[test]
fn the_chip_is_not_reported_as_unsupported() {
    assert_eq!(n163_machine(&[], &[]).warnings_nul(), [0]);
}

#[test]
fn a_snapshot_continues_exactly() {
    let mut init = n163_ram(0, &[0x10, 0x32, 0x54, 0x76, 0xFE, 0xDC, 0xBA, 0x98]);
    init.extend(n163_ram(0x78, &[0x30, 0, 0x01, 0, 0xF8, 0, 0, 0x1F]));
    init.extend(n163_ram(0x70, &[0x77, 0, 0x02, 0, 0xF0, 0, 4, 0x0C]));
    let mut a = n163_machine(&init, &[]);
    render(&mut a, 7777);
    let snap = a.snapshot();
    let from_a = render(&mut a, 9600);
    let mut b = n163_machine(&init, &[]);
    b.restore(&snap).unwrap();
    assert_eq!(digest(&from_a), digest(&render(&mut b, 9600)));
}
