//! The HuC6280: a 65C02 with eight memory mapping registers, a high and a low
//! speed, block transfers and the registers that the PC Engine adds.

/// What the processor sees outside itself. Addresses are the 21-bit physical
/// addresses that the memory mapping registers make from the 16-bit logical
/// ones.
pub trait Bus {
    fn read(&mut self, physical: u32) -> u8;
    fn write(&mut self, physical: u32, value: u8);
    /// The vector address (0xFFF6 for IRQ2, 0xFFF8 for IRQ1, 0xFFFA for the
    /// timer) of the interrupt that is pending and not masked, or `None`. The
    /// processor asks between instructions while its I flag is clear.
    fn pending_interrupt(&mut self) -> Option<u16>;
}

#[derive(Clone, Debug)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub pc: u16,
    pub p: u8,
    pub mpr: [u8; 8],
    pub high_speed: bool,
}

impl Cpu {
    pub fn new() -> Cpu {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            s: 0xFF,
            pc: 0,
            p: 0x04,
            mpr: [0; 8],
            high_speed: false,
        }
    }

    /// Runs one instruction, or takes one interrupt, and returns the cycles
    /// that it took at the current speed.
    pub fn step(&mut self, _bus: &mut impl Bus) -> u32 {
        todo!("the opcodes")
    }
}

impl Default for Cpu {
    fn default() -> Cpu {
        Cpu::new()
    }
}
