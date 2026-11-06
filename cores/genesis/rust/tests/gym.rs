//! A GYM log plays as the VGM file that has the same writes and waits.

mod common;

use common::*;
use oscilla_genesis::{EndState, Machine};
use oscilla_gym::Gym;

/// The writes of an FM tone, then a PSG tone, as a GYM log and as VGM
/// commands, with `frames` waits after them.
fn song(frames: usize) -> (Vec<u8>, Vec<u8>) {
    let mut gym = Vec::new();
    let mut vgm = Vec::new();
    let mut fm = |reg: u8, val: u8| {
        gym.extend([1, reg, val]);
        vgm.extend([0x52, reg, val]);
    };
    for (offset, level) in [(0u8, 0u8), (4, 127), (8, 127), (12, 127)] {
        fm(0x30 + offset, 0x01);
        fm(0x40 + offset, level);
        fm(0x50 + offset, 0x1F);
        fm(0x60 + offset, 0);
        fm(0x70 + offset, 0);
        fm(0x80 + offset, 0x0F);
    }
    fm(0xB0, 0x07);
    fm(0xB4, 0xC0);
    fm(0xA4, 4 << 3 | 4);
    fm(0xA0, 0x3B);
    fm(0x28, 0x10);
    for b in [0x8E, 0x0F, 0x90] {
        gym.extend([3, b]);
        vgm.extend([0x50, b]);
    }
    for _ in 0..frames {
        gym.push(0);
        vgm.push(0x62);
    }
    vgm.push(0x66);
    (gym, vgm)
}

fn machine(bytes: &[u8]) -> Machine {
    let mut m = Machine::new(&parse(bytes), 44_100).unwrap();
    m.set_highpass_hz(0.0);
    m
}

#[test]
fn a_gym_log_renders_as_the_vgm_with_the_same_commands() {
    let (gym, vgm) = song(20);
    let converted = Gym::parse(&gym).unwrap().to_vgm(false);
    let mut a = machine(&converted);
    let mut b = machine(&file(true, true, &vgm, None));
    let (x, y) = (render(&mut a, 20 * 735), render(&mut b, 20 * 735));
    assert_eq!(x, y);
    assert!(x.iter().any(|s| s.abs() > 500));
}

#[test]
fn a_pal_log_runs_at_fifty_frames_a_second() {
    let (gym, _) = song(10);
    let g = Gym::parse(&gym).unwrap();
    let mut ntsc = machine(&g.to_vgm(false));
    let mut pal = machine(&g.to_vgm(true));
    render(&mut ntsc, 10 * 735 + 1000);
    render(&mut pal, 10 * 735 + 1000);
    assert_eq!(ntsc.end_state(), EndState::Finished);
    assert_eq!(pal.end_state(), EndState::Playing);
    render(&mut pal, 10 * 882);
    assert_eq!(pal.end_state(), EndState::Finished);
}

#[test]
fn a_looping_log_comes_round_again() {
    let (mut gym, _) = song(10);
    let mut header = vec![0u8; 428];
    header[..4].copy_from_slice(b"GYMX");
    header[420..424].copy_from_slice(&4u32.to_le_bytes());
    header.append(&mut gym);
    let g = Gym::parse(&header).unwrap();
    let mut m = machine(&g.to_vgm(false));
    render(&mut m, 40 * 735);
    assert!(m.loops() >= 1);
}
