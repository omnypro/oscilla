//! The fixed programs behind the checked-in digests must do something: reach
//! many addresses in both instruction sets, take interrupts, enter handlers,
//! run to their end, and move data through the BIOS calls. A digest of a
//! program that did nothing would pass for any machine.

mod common;

use common::{
    Machine, Rust, arm_program, biased_arm_program, biased_thumb_program, io_program, swi_program,
    thumb_program,
};

#[test]
fn the_programs_exercise_varied_code() {
    let mut thumb_steps = 0u64;
    let mut arm_steps = 0u64;
    let mut pcs = std::collections::HashSet::new();
    let mut modes = std::collections::HashSet::new();
    let mut writes_ewram = 0u64;
    for seed in 1..=40 {
        let rom = if seed % 2 == 0 {
            arm_program(seed, 600)
        } else {
            thumb_program(seed, 1_200)
        };
        let mut r = Rust::from_rom(&rom);
        for _ in 0..6_000 {
            r.run_cycles(1);
            pcs.insert(r.next_pc());
            modes.insert(r.register(16) & 0x1F);
            if r.register(16) & 0x20 != 0 {
                thumb_steps += 1
            } else {
                arm_steps += 1
            }
        }
        if (0..64).any(|i| r.peek(0x0200_0000 + 4 * i, 4) != 0) {
            writes_ewram += 1;
        }
    }
    eprintln!(
        "arm {arm_steps} thumb {thumb_steps} pcs {} modes {modes:?} ewram-writers {writes_ewram}",
        pcs.len()
    );
    assert!(thumb_steps > 10_000 && arm_steps > 10_000);
    assert!(pcs.len() > 2_000);
    assert!(modes.len() >= 3);
}

#[test]
fn the_biased_programs_run_to_the_end() {
    let mut alive = 0;
    let mut ram_writers = 0;
    let mut pcs = std::collections::HashSet::new();
    for seed in 1..=60 {
        let rom = if seed % 2 == 0 {
            biased_arm_program(seed, 800)
        } else {
            biased_thumb_program(seed, 1_600)
        };
        let mut r = Rust::from_rom(&rom);
        let mut last = 0;
        let mut moved = 0;
        for _ in 0..8_000 {
            r.run_cycles(1);
            if r.next_pc() != last {
                moved += 1;
            }
            last = r.next_pc();
            pcs.insert(last);
        }
        if moved > 6_000 {
            alive += 1;
        }
        if (0..4096).any(|i| {
            r.peek(0x0200_0000 + 4 * i, 4) != 0 || r.peek(0x0300_0000 + 4 * (i % 2048), 4) != 0
        }) {
            ram_writers += 1;
        }
    }
    eprintln!(
        "alive {alive}/60 ram-writers {ram_writers}/60 pcs {}",
        pcs.len()
    );
    assert!(alive >= 30 && ram_writers >= 30);
}

#[test]
fn the_io_programs_exercise_interrupts_timers_and_dma() {
    let (mut irq_seen, mut svc_seen, mut counted, mut reached_end, mut dma_wrote) = (0, 0, 0, 0, 0);
    let mut total_cycles = 0u64;
    for seed in 1..=60 {
        let rom = io_program(seed, 150);
        let end = 0x0800_0000 + rom.len() as u32 - 4;
        let mut r = Rust::from_rom(&rom);
        let mut irq = false;
        let mut svc = false;
        let mut at_end = false;
        for _ in 0..40_000 {
            r.run_cycles(1);
            let mode = r.register(16) & 0x1F;
            irq |= mode == 0x12;
            svc |= mode == 0x13;
            at_end |= r.next_pc() == end;
        }
        irq_seen += irq as u32;
        svc_seen += svc as u32;
        reached_end += at_end as u32;
        counted += (r.register(7) > 0) as u32;
        dma_wrote += (0..2048).any(|i| r.peek(0x0200_0000 + 4 * i, 4) != 0) as u32;
        total_cycles += r.cycles();
    }
    eprintln!(
        "irq {irq_seen} svc {svc_seen} handler-ran {counted} reached-end {reached_end} ewram-written {dma_wrote} cycles {total_cycles}"
    );
    assert!(irq_seen >= 10 && svc_seen >= 10 && counted >= 10 && reached_end >= 10);
}

#[test]
fn the_data_calls_write_memory() {
    for n in [
        0x0Bu32, 0x0C, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
    ] {
        let mut wrote = 0;
        for seed in 1..=40 {
            let rom = swi_program(n, seed);
            let mut r = Rust::from_rom(&rom);
            for _ in 0..60 {
                r.run_cycles(1);
            }
            if (0..0x4000).any(|i| r.peek(0x0200_0000 + 4 * i, 4) != 0) {
                wrote += 1;
            }
        }
        eprintln!("swi {n:#04x}: memory written in {wrote}/40 seeds");
        assert!(wrote > 0, "swi {n:#04x} never wrote");
    }
}

fn render(m: &mut Rust, frames: usize) -> Vec<i16> {
    m.render_s16(frames)
}

#[test]
fn the_sound_programs_are_heard() {
    let mut audible = 0;
    for seed in 1..=30 {
        let mut m = Rust::from_rom(&common::sound_program(seed));
        let out = render(&mut m, 30_000);
        let distinct: std::collections::HashSet<i16> = out.iter().copied().collect();
        if distinct.len() > 100 {
            audible += 1;
        }
    }
    eprintln!("audible {audible}/30");
    assert!(audible >= 24, "{audible}");
}

#[test]
fn the_io_programs_keep_their_base_register_through_bios_calls() {
    let mut kept = 0;
    for seed in 1..=30 {
        let mut m = Rust::from_rom(&io_program(seed, 150));
        for _ in 0..40_000 {
            m.run_cycles(1);
        }
        kept += (m.register(0) == 0x0400_0000) as u32;
    }
    assert!(kept >= 25, "{kept}");
}

#[test]
fn the_thumb_io_programs_run_in_thumb_and_finish() {
    let mut finished = 0;
    let mut thumb_steps = 0u64;
    for seed in 1..=20 {
        let rom = common::thumb_io_program(seed);
        let end = 0x0800_0000 + rom.len() as u32 - 2;
        let mut m = Rust::from_rom(&rom);
        let mut at_end = false;
        for _ in 0..6_000 {
            m.run_cycles(1);
            thumb_steps += (m.register(16) & 0x20 != 0) as u64;
            at_end |= m.next_pc() == end;
        }
        for _ in 0..200 {
            m.run_cycles(70_000);
            at_end |= m.next_pc() == end;
        }
        finished += at_end as u32;
    }
    eprintln!("thumb steps {thumb_steps}, finished {finished}/20");
    assert!(
        thumb_steps > 3_000 && finished >= 15,
        "{thumb_steps} {finished}"
    );
}

#[test]
fn the_bus_programs_write_every_kind_of_memory() {
    let mut video = 0;
    let mut palette = 0;
    let mut objects = 0;
    let mut finished = 0;
    for seed in 1..=30 {
        let rom = common::bus_program(seed);
        let end = 0x0800_0000 + 4 * ((rom.len() - 0x400) / 4) as u32 - 4;
        let mut m = Rust::from_rom(&rom);
        let mut at_end = false;
        for _ in 0..4_000 {
            m.run_cycles(1);
            at_end |= m.next_pc() == end;
        }
        let nonzero = |base: u32, size: u32| (0..size / 4).any(|i| m.peek(base + 4 * i, 4) != 0);
        video += nonzero(0x0600_0000, 0x1_8000) as u32;
        palette += nonzero(0x0500_0000, 0x400) as u32;
        objects += nonzero(0x0700_0000, 0x400) as u32;
        finished += at_end as u32;
    }
    eprintln!("video {video} palette {palette} objects {objects} finished {finished}");
    assert!(video >= 20 && palette >= 20 && objects >= 20 && finished >= 20);
}

#[test]
fn the_corner_programs_switch_modes_and_finish() {
    let mut modes = std::collections::HashSet::new();
    let mut finished = 0;
    for seed in 1..=30 {
        let rom = common::corner_program(seed);
        let end = 0x0800_0000 + rom.len() as u32 - 4;
        let mut m = Rust::from_rom(&rom);
        let mut at_end = false;
        for _ in 0..3_500 {
            m.run_cycles(1);
            modes.insert(m.register(16) & 0x1F);
            at_end |= m.next_pc() == end;
        }
        finished += at_end as u32;
    }
    eprintln!("modes {modes:?} finished {finished}/30");
    assert!(modes.len() >= 5 && finished >= 24, "{modes:?} {finished}");
}

#[test]
fn the_dma_programs_move_data_and_finish() {
    let (mut moved, mut finished, mut irq) = (0, 0, 0);
    for seed in 1..=30 {
        let rom = common::dma_program(seed);
        let end = 0x0800_0000 + 4 * ((rom.len() - 0x5000) / 4) as u32 - 4;
        let _ = end;
        let mut m = Rust::from_rom(&rom);
        let mut was_irq = false;
        for _ in 0..4_000 {
            m.run_cycles(1);
            was_irq |= m.register(16) & 0x1F == 0x12;
        }
        for _ in 0..400 {
            m.run_cycles(70_000);
            was_irq |= m.register(16) & 0x1F == 0x12;
        }
        let nonzero = |base: u32, size: u32| (0..size / 4).any(|i| m.peek(base + 4 * i, 4) != 0);
        moved += (nonzero(0x0200_2000, 0x400) || nonzero(0x0300_2000, 0x1400)) as u32;
        finished += (m.next_pc() >= 0x0800_0000 && m.next_pc() < 0x0800_4000) as u32;
        irq += was_irq as u32;
    }
    eprintln!("dma moved {moved} finished {finished} interrupts {irq} of 30");
    assert!(moved >= 24 && finished >= 24 && irq >= 20);
}

#[test]
fn the_ram_programs_run_from_ram() {
    let mut in_ram = [0u64; 4];
    let mut pcs = std::collections::HashSet::new();
    for seed in 1..=8 {
        for (i, (thumb, base)) in [
            (false, 0x0300_4000u32),
            (false, 0x0202_0000),
            (true, 0x0300_4000),
            (true, 0x0202_0000),
        ]
        .into_iter()
        .enumerate()
        {
            let mut m = Rust::from_rom(&common::ram_program(seed, thumb, base));
            for _ in 0..6_000 {
                m.run_cycles(1);
                let pc = m.next_pc();
                in_ram[i] += (pc >> 24 == base >> 24) as u64;
                pcs.insert(pc);
            }
        }
    }
    eprintln!("steps in RAM {in_ram:?}, distinct addresses {}", pcs.len());
    assert!(in_ram.iter().all(|&n| n > 8_000), "{in_ram:?}");
}

#[test]
fn the_reset_programs_read_back_what_soft_reset_leaves() {
    for seed in 1..=6 {
        let mut m = Rust::from_rom(&common::reset_program(seed));
        for _ in 0..900 {
            m.run_cycles(1);
        }
        assert_eq!(m.register(7), 0x0300_7FE0, "supervisor stack, seed {seed}");
        assert_eq!(m.register(8), 0x0300_7FA0, "interrupt stack, seed {seed}");
        assert_eq!(m.register(9), 0x0300_7F00, "system stack, seed {seed}");
        assert_eq!(
            (m.register(10), m.register(11)),
            (0, 0),
            "the cleared range, seed {seed}"
        );
        assert_ne!(
            m.register(0),
            0,
            "the word below the cleared range survives, seed {seed}"
        );
    }
}
