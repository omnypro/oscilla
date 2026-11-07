//! The inspection calls: processor registers, memory reads and the register
//! blocks. They read the machine and never change what it plays.

use oscilla_hes::Hes;
use oscilla_pce::Machine;

/// A file whose first ROM bank holds `request` at 0xE100. The vectors point at
/// an RTI at 0xFFF0.
fn program(request: &[u8]) -> Vec<u8> {
    let mut bank = vec![0u8; 0x2000];
    bank[0x100..0x100 + request.len()].copy_from_slice(request);
    bank[0x1FF0] = 0x40;
    bank[0x1FF6..0x1FF8].copy_from_slice(&[0xF0, 0xFF]);
    bank[0x1FF8..0x1FFA].copy_from_slice(&[0xF0, 0xFF]);
    bank[0x1FFA..0x1FFC].copy_from_slice(&[0xF0, 0xFF]);
    bank[0x1FFC..0x1FFE].copy_from_slice(&[0xF0, 0xFF]);
    let mut f = b"HESM".to_vec();
    f.extend([0, 0, 0x00, 0xE1, 0xFF, 0xF8, 0, 0, 1, 2, 3, 0]);
    f.extend(b"DATA");
    f.extend((bank.len() as u32).to_le_bytes());
    f.extend(0u32.to_le_bytes());
    f.extend([0; 4]);
    f.extend(bank);
    f
}

fn machine(file: &[u8]) -> Machine {
    let mut m = Machine::new(&Hes::parse(file).unwrap(), 44_100, 0).unwrap();
    m.set_highpass_hz(0.0);
    m
}

fn render(m: &mut Machine, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; 2 * frames];
    m.render_s16(&mut out);
    out
}

/// LDA #value, STA address.
fn sta(address: u16, value: u8) -> Vec<u8> {
    vec![0xA9, value, 0x8D, address as u8, (address >> 8) as u8]
}

const PSG: u16 = 0x0800;

/// Channel 1 as a 440 Hz square wave with distinct register values, then RTS.
fn square() -> Vec<u8> {
    let mut c = sta(PSG, 0);
    c.extend(sta(PSG + 1, 0xFE));
    c.extend(sta(PSG + 5, 0xAB));
    c.extend(sta(PSG + 4, 0x40));
    c.extend(sta(PSG + 4, 0x00));
    for i in 0..32 {
        c.extend(sta(PSG + 6, if i < 16 { 31 } else { i & 7 }));
    }
    c.extend(sta(PSG + 2, 0x34));
    c.extend(sta(PSG + 3, 0x02));
    c.extend(sta(PSG + 4, 0x9F));
    c.push(0x60);
    c
}

fn block(m: &Machine, b: usize) -> Vec<u8> {
    let mut out = vec![0u8; m.register_block_size(b)];
    assert_eq!(m.register_block_get(b, &mut out), out.len());
    out
}

#[test]
fn the_processor_registers_and_a_read_of_memory_follow_the_program() {
    let f = program(&square());
    let mut m = machine(&f);
    // The machine sets A to the song number and calls the request routine at
    // 0xE100 before it runs anything.
    assert_eq!(m.register(4), 0xE100);
    assert_eq!(m.next_pc(), 0xE100);
    assert_eq!(m.peek(0xE100), 0xA9, "the routine's first instruction");
    assert_eq!(m.peek(0xE101), 0x00);
    render(&mut m, 2000);
    // The routine ended with RTS and the machine idles at 0x1FFF.
    assert_eq!(m.register(4), 0x1FFF);
    // MPR7 maps the first ROM bank, and the registers 6 to 13 are the eight.
    assert_eq!(m.register(13), 0x00);
    assert_eq!(m.register(6), 0xFF);
    assert_eq!(m.register(14), 0);
    assert_eq!(m.register(-1), 0);
    assert!(m.cycles() > 0);
}

#[test]
fn the_register_blocks_hold_what_the_program_wrote() {
    let f = program(&square());
    let mut m = machine(&f);
    assert_eq!(m.register_block_count(), 4);
    assert_eq!(m.register_block_name(0), Some("PSG"));
    assert_eq!(m.register_block_name(4), None);
    assert_eq!(m.register_block_size(0), 40);
    assert_eq!(m.register_block_size(1), 192);
    assert_eq!(m.register_block_size(2), 8);
    assert_eq!(m.register_block_size(3), 4);
    assert_eq!(m.register_block_size(4), 0);
    render(&mut m, 4000);

    let psg = block(&m, 0);
    assert_eq!(psg[0], 0, "register 0 selected channel 1");
    assert_eq!(psg[1], 0xFE, "the main balance");
    // Channel 1: frequency low, high, control, balance, direct D/A, noise.
    assert_eq!(&psg[4..8], &[0x34, 0x02, 0x9F, 0xAB]);
    assert_eq!(
        psg[10..40],
        [0u8; 30],
        "the other channels were never written"
    );

    let waves = block(&m, 1);
    assert!(waves[..16].iter().all(|w| *w == 31));
    assert_eq!(waves[16..24], [0, 1, 2, 3, 4, 5, 6, 7]);
    assert!(waves[32..].iter().all(|w| *w == 0));

    assert_eq!(block(&m, 2)[7], 0x00);
    assert_eq!(block(&m, 3), [0, 0, 0, 0]);

    // A short buffer takes the first bytes, and nothing is invalid-safe to
    // read from a block that does not exist.
    let mut short = [0xEEu8; 3];
    assert_eq!(m.register_block_get(0, &mut short), 3);
    assert_eq!(short, [0, 0xFE, 0]);
    assert_eq!(m.register_block_get(9, &mut short), 0);
    assert_eq!(m.register_block_get(0, &mut []), 0);
}

#[test]
fn a_read_of_the_video_status_does_not_clear_it() {
    // Vertical sync enabled, interrupts left disabled: the status flag is set
    // each frame and the program never reads it.
    let mut code = sta(0x0000, 5);
    code.extend(sta(0x0002, 0x08));
    code.push(0x60);
    let f = program(&code);
    let mut m = machine(&f);
    render(&mut m, 2000);
    let first = m.peek(0x0000);
    assert_ne!(first & 0x20, 0, "the vertical blanking flag");
    assert_eq!(m.peek(0x0000), first);
}

#[test]
fn reading_the_machine_does_not_change_the_sound() {
    let f = program(&square());
    let plain = {
        let mut m = machine(&f);
        (0..8).flat_map(|_| render(&mut m, 500)).collect::<Vec<_>>()
    };
    let looked = {
        let mut m = machine(&f);
        let mut all = Vec::new();
        let mut buf = [0u8; 256];
        for _ in 0..8 {
            all.extend(render(&mut m, 500));
            for b in 0..4 {
                m.register_block_get(b, &mut buf);
            }
            for i in 0..14 {
                m.register(i);
            }
            m.peek(0x0000);
            m.peek(0xE100);
            m.cycles();
            m.next_pc();
        }
        all
    };
    assert_eq!(plain, looked);
}
