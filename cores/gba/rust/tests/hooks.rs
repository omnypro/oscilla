//! The hooks a player needs: snapshots, channel levels and masks, and the end
//! of a song.

mod common;

use common::{io_program, rom_from_words};
use oscilla_gba::image::Image;
use oscilla_gba::{EndState, Machine, RestoreError};

fn machine(rom: &[u8], rate: i32) -> Machine {
    Machine::new(&Image::from_rom(rom).unwrap(), rate).unwrap()
}

fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; frames * 2];
    m.render_s16(&mut out);
    out
}

#[test]
fn a_restored_snapshot_replays_the_same_sound() {
    for seed in 1..=20 {
        let rom = io_program(seed, 150);
        let mut a = machine(&rom, 48000);
        render(&mut a, 3_000);
        let snap = a.snapshot();
        let first = render(&mut a, 20_000);
        // The same machine, put back.
        a.restore(&snap).unwrap();
        assert_eq!(render(&mut a, 20_000), first, "seed {seed}: same machine");
        // A different machine on the same image.
        let mut b = machine(&rom, 48000);
        b.restore(&snap).unwrap();
        assert_eq!(render(&mut b, 20_000), first, "seed {seed}: fresh machine");
        assert_eq!(a.cycles(), b.cycles());
    }
}

#[test]
fn a_snapshot_taken_mid_run_does_not_change_the_run() {
    let rom = io_program(7, 150);
    let mut plain = machine(&rom, 32768);
    let mut taken = machine(&rom, 32768);
    render(&mut plain, 5_000);
    render(&mut taken, 5_000);
    let _snap = taken.snapshot();
    assert_eq!(render(&mut plain, 10_000), render(&mut taken, 10_000));
}

#[test]
fn a_snapshot_will_not_restore_into_the_wrong_machine() {
    let rom = io_program(3, 150);
    let other = io_program(4, 150);
    let mut a = machine(&rom, 48000);
    render(&mut a, 1_000);
    let snap = a.snapshot();
    assert_eq!(machine(&rom, 44100).restore(&snap), Err(RestoreError::Rate));
    assert_eq!(
        machine(&other, 48000).restore(&snap),
        Err(RestoreError::Image)
    );
}

#[test]
fn a_restore_keeps_the_listeners_channel_mask() {
    let rom = io_program(3, 150);
    let mut a = machine(&rom, 48000);
    let snap = a.snapshot();
    a.set_channel_mask(0b01_0101);
    a.restore(&snap).unwrap();
    assert_eq!(a.channel_mask(), 0b01_0101);
}

/// Switches the sound unit on and starts two tone channels and the noise
/// channel at full volume.
fn tone_program() -> Vec<u8> {
    let mut words = vec![0xE3A0_0301u32]; // mov r0, #0x04000000
    let mut poke = |off: u32, value: u32| {
        words.push(0xE59F_1000); // ldr r1, [pc, #0]
        words.push(0xEA00_0000); // b +4
        words.push(value);
        words.push(0xE280_CC00 | (off >> 8)); // add r12, r0, #(off & ~0xFF)
        let low = off & 0xFF;
        words.push(0xE1CC_10B0 | ((low >> 4) << 8) | (low & 0xF)); // strh r1, [r12, #low]
    };
    poke(0x84, 0x80); // sound on
    poke(0x80, 0xFF77); // every tone channel to both sides, volume 7
    poke(0x82, 0x0002); // tone channels at 100%
    poke(0x62, 0xF040); // channel 1: duty, volume 15
    poke(0x64, 0x8400); // channel 1: trigger
    poke(0x68, 0xF080); // channel 2: duty, volume 15
    poke(0x6C, 0x8500); // channel 2: trigger
    poke(0x78, 0xF000); // noise: volume 15
    poke(0x7C, 0x8000); // noise: trigger
    words.push(0xEAFF_FFFE); // b .
    rom_from_words(&words)
}

/// The mix after its start-up transient, as the frames from 400 on.
fn settled(m: &mut Machine, frames: usize) -> Vec<i16> {
    render(m, frames)[800..].to_vec()
}

#[test]
fn muting_every_channel_flattens_the_mix_and_one_channel_less_changes_it() {
    let rom = tone_program();
    let full = settled(&mut machine(&rom, 48000), 20_000);
    assert!(
        full.iter().any(|&s| s != full[0]),
        "the tone program should be heard"
    );

    let mut silent = machine(&rom, 48000);
    silent.set_channel_mask(0);
    let flat = settled(&mut silent, 20_000);
    assert!(flat.iter().all(|&s| s == flat[0]), "a muted mix is flat");

    for bit in [0u8, 1, 3] {
        let mut m = machine(&rom, 48000);
        m.set_channel_mask(0x3F & !(1 << bit));
        assert_ne!(
            settled(&mut m, 20_000),
            full,
            "muting channel {bit} must change the mix"
        );
        let mut solo = machine(&rom, 48000);
        solo.set_channel_mask(1 << bit);
        let only = settled(&mut solo, 20_000);
        assert_ne!(only, flat, "channel {bit} alone must be heard");
        assert_ne!(only, full, "channel {bit} alone is not the whole mix");
    }
}

#[test]
fn the_default_mask_changes_nothing() {
    for seed in 1..=20 {
        let rom = io_program(seed, 150);
        let untouched = render(&mut machine(&rom, 48000), 10_000);
        let mut m = machine(&rom, 48000);
        m.set_channel_mask(0x3F);
        assert_eq!(render(&mut m, 10_000), untouched, "seed {seed}");
    }
}

#[test]
fn channel_levels_follow_the_hardware() {
    let rom = tone_program();
    let mut m = machine(&rom, 48000);
    let mut seen = [false; 4];
    for _ in 0..200 {
        render(&mut m, 100);
        let l = m.channel_levels();
        assert!(l.psg.iter().all(|v| (0..=15).contains(v)), "{l:?}");
        assert!(l.fifo.iter().all(|v| (-128..=127).contains(v)), "{l:?}");
        for (flag, level) in seen.iter_mut().zip(l.psg) {
            *flag |= level != 0;
        }
    }
    assert_eq!(
        seen,
        [true, true, false, true],
        "tone 1 and 2 and noise move; wave RAM is empty"
    );

    // A muted channel still shows its level.
    let mut muted = machine(&rom, 48000);
    muted.set_channel_mask(0);
    let mut shown = false;
    for _ in 0..200 {
        render(&mut muted, 100);
        shown |= muted.channel_levels().psg[0] != 0;
    }
    assert!(shown);
}

#[test]
fn the_end_of_a_song_is_told_apart_from_a_game_waiting_for_an_interrupt() {
    // b .  with nothing enabled
    let parked = rom_from_words(&[0xEAFF_FFFE]);
    let mut m = machine(&parked, 32768);
    m.run_cycles(100);
    assert_eq!(m.end_state(), EndState::Parked);

    // IME on and an interrupt enabled: the loop can be broken into.
    let waiting = rom_from_words(&[
        0xE3A0_0301, // mov r0, #0x04000000
        0xE3A0_1001, // mov r1, #1
        0xE5C0_1208, // strb r1, [r0, #0x208]  (IME)
        0xE3A0_1001,
        0xE5C0_1200, // strb r1, [r0, #0x200]  (IE: VBlank)
        0xEAFF_FFFE,
    ]);
    let mut m = machine(&waiting, 32768);
    m.run_cycles(200);
    assert_eq!(m.end_state(), EndState::Running);

    // swi Halt with no interrupt enabled
    let halted = rom_from_words(&[0xEF02_0000, 0xEAFF_FFFE]);
    let mut m = machine(&halted, 32768);
    m.run_cycles(2_000);
    assert_eq!(m.end_state(), EndState::HaltedWithoutWake);

    // ordinary code is running
    let counting = rom_from_words(&[0xE280_0001, 0xEAFF_FFFD]);
    let mut m = machine(&counting, 32768);
    m.run_cycles(100);
    assert_eq!(m.end_state(), EndState::Running);
}

#[test]
fn quiet_cycles_count_silence_and_reset_on_sound() {
    let silent = rom_from_words(&[0xE280_0001, 0xEAFF_FFFD]);
    let mut m = machine(&silent, 32768);
    render(&mut m, 4_000);
    let early = m.quiet_cycles();
    render(&mut m, 4_000);
    assert!(m.quiet_cycles() > early, "silence keeps counting");
    let mut heard = 0;
    for seed in 1..=30 {
        let rom = io_program(seed, 150);
        let mut m = machine(&rom, 48000);
        render(&mut m, 60_000);
        if m.quiet_cycles() < m.cycles() {
            heard += 1;
        }
    }
    assert!(heard >= 5, "{heard}");
}

#[test]
fn a_parked_thumb_loop_is_found_too() {
    // add lr, pc, #1; bx lr; b . (Thumb)
    let mut rom = rom_from_words(&[0xE28F_E001, 0xE12F_FF1E]);
    rom.extend_from_slice(&0xE7FEu16.to_le_bytes());
    let mut m = machine(&rom, 32768);
    m.run_cycles(200);
    assert_eq!(m.end_state(), EndState::Parked);
}

#[test]
fn peek_reads_each_width_and_quiet_cycles_restart_on_sound() {
    let m = machine(&rom_from_words(&[0x1122_3344, 0xEAFF_FFFE]), 32768);
    assert_eq!(m.peek(0x0800_0000, 4), 0x1122_3344);
    assert_eq!(m.peek(0x0800_0002, 2), 0x1122);
    assert_eq!(m.peek(0x0800_0000, 1), 0x44);
    assert_eq!(m.peek(0x0800_0001, 1), 0x33);

    let mut tone = machine(&tone_program(), 48000);
    render(&mut tone, 10_000);
    assert!(
        tone.quiet_cycles() < 20_000,
        "a ringing tone changes the level often"
    );
}
