//! The SPC700: the registers, the 256 opcodes and the I/O ports at 0xF0..0xFF.
//! An instruction runs whole and then the clock advances by the cycles that
//! the instruction takes.

use crate::boot;
use crate::machine::Machine;

const N: u8 = 0x80;
const V: u8 = 0x40;
const P: u8 = 0x20;
const B: u8 = 0x10;
const H: u8 = 0x08;
const I: u8 = 0x04;
const Z: u8 = 0x02;
const C: u8 = 0x01;

impl Machine {
    pub(crate) fn read(&mut self, address: u16) -> u8 {
        match address {
            0xF0 | 0xF1 => 0,
            0xF2 => self.dsp_address,
            0xF3 => {
                self.catch_up();
                self.dsp.read(self.dsp_address)
            }
            0xF4..=0xF7 => self.ports_in[usize::from(address - 0xF4)],
            0xFD..=0xFF => {
                self.catch_up();
                let t = &mut self.timers[usize::from(address - 0xFD)];
                std::mem::take(&mut t.out)
            }
            0xFFC0..=0xFFFF if self.control & 0x80 != 0 => {
                boot::ROM[usize::from(address - boot::BASE)]
            }
            _ => self.ram[usize::from(address)],
        }
    }

    pub(crate) fn write(&mut self, address: u16, value: u8) {
        self.ram[usize::from(address)] = value;
        if let Some((first, last)) = self.observer.1
            && (first..=last).contains(&address)
        {
            self.deferred_rams.push((address, value));
        }
        if !(0xF0..=0xFF).contains(&address) {
            return;
        }
        match address {
            0xF1 => self.set_control(value),
            0xF2 => self.dsp_address = value,
            0xF3 => {
                if self.dsp_address < 0x80 {
                    self.deferred.push((self.dsp_address, value));
                }
            }
            0xF4..=0xF7 => {
                let port = (address - 0xF4) as u8;
                self.ports_out[usize::from(port)] = value;
                self.deferred_ports.push((port, value));
            }
            0xFA..=0xFC => self.timers[usize::from(address - 0xFA)].target = value,
            _ => {}
        }
    }

    /// The control register: timer enables, input latch resets and the boot
    /// ROM switch.
    pub(crate) fn set_control(&mut self, value: u8) {
        for (n, t) in self.timers.iter_mut().enumerate() {
            let on = value & (1 << n) != 0;
            if on && !t.enabled {
                t.stage = 0;
                t.out = 0;
            }
            if !on {
                t.out = 0;
            }
            t.enabled = on;
        }
        if value & 0x10 != 0 {
            self.ports_in[0] = 0;
            self.ports_in[1] = 0;
        }
        if value & 0x20 != 0 {
            self.ports_in[2] = 0;
            self.ports_in[3] = 0;
        }
        self.control = value;
    }

    fn fetch(&mut self) -> u8 {
        self.fetched += 1;
        let v = self.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn fetch_word(&mut self) -> u16 {
        let lo = self.fetch();
        let hi = self.fetch();
        u16::from(hi) << 8 | u16::from(lo)
    }

    fn page(&self) -> u16 {
        if self.psw & P != 0 { 0x100 } else { 0 }
    }

    /// A direct page address. The offset wraps inside the page.
    fn direct(&self, offset: u8) -> u16 {
        self.page() | u16::from(offset)
    }

    /// The word at a direct page address. The high byte wraps inside the page.
    fn direct_word(&mut self, offset: u8) -> u16 {
        let lo = self.read(self.direct(offset));
        let hi = self.read(self.direct(offset.wrapping_add(1)));
        u16::from(hi) << 8 | u16::from(lo)
    }

    fn push(&mut self, value: u8) {
        self.write(0x100 | u16::from(self.sp), value);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pop(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.ram[0x100 | usize::from(self.sp)]
    }

    fn push_word(&mut self, value: u16) {
        self.push((value >> 8) as u8);
        self.push(value as u8);
    }

    fn pop_word(&mut self) -> u16 {
        let lo = self.pop();
        let hi = self.pop();
        u16::from(hi) << 8 | u16::from(lo)
    }

    fn flag(&mut self, mask: u8, on: bool) {
        if on {
            self.psw |= mask;
        } else {
            self.psw &= !mask;
        }
    }

    fn nz(&mut self, v: u8) -> u8 {
        self.flag(N, v & 0x80 != 0);
        self.flag(Z, v == 0);
        v
    }

    fn nz16(&mut self, v: u16) -> u16 {
        self.flag(N, v & 0x8000 != 0);
        self.flag(Z, v == 0);
        v
    }

    fn adc(&mut self, a: u8, b: u8) -> u8 {
        let carry = u32::from(self.psw & C);
        let sum = u32::from(a) + u32::from(b) + carry;
        let result = sum as u8;
        self.flag(V, (a ^ result) & (b ^ result) & 0x80 != 0);
        self.flag(H, u32::from(a & 15) + u32::from(b & 15) + carry > 15);
        self.flag(C, sum > 0xFF);
        self.nz(result)
    }

    fn sbc(&mut self, a: u8, b: u8) -> u8 {
        self.adc(a, !b)
    }

    fn cmp(&mut self, a: u8, b: u8) {
        self.flag(C, a >= b);
        self.nz(a.wrapping_sub(b));
    }

    /// OR, AND, EOR, CMP, ADC and SBC by number.
    fn alu(&mut self, op: u8, a: u8, b: u8) -> u8 {
        match op {
            0 => self.nz(a | b),
            1 => self.nz(a & b),
            2 => self.nz(a ^ b),
            3 => {
                self.cmp(a, b);
                a
            }
            4 => self.adc(a, b),
            _ => self.sbc(a, b),
        }
    }

    /// ASL, ROL, LSR, ROR, DEC and INC by number.
    fn rmw(&mut self, op: u8, v: u8) -> u8 {
        let carry = self.psw & C;
        match op {
            0 => {
                self.flag(C, v & 0x80 != 0);
                self.nz(v << 1)
            }
            1 => {
                self.flag(C, v & 0x80 != 0);
                self.nz(v << 1 | carry)
            }
            2 => {
                self.flag(C, v & 1 != 0);
                self.nz(v >> 1)
            }
            3 => {
                self.flag(C, v & 1 != 0);
                self.nz(v >> 1 | carry << 7)
            }
            4 => self.nz(v.wrapping_sub(1)),
            _ => self.nz(v.wrapping_add(1)),
        }
    }

    fn branch(&mut self, condition: bool, base: u32) -> u32 {
        let offset = self.fetch() as i8;
        if condition {
            self.pc = self.pc.wrapping_add(offset as u16);
            base + 2
        } else {
            base
        }
    }

    /// A bit operand: a 13-bit address and a bit number from one 16-bit word.
    fn bit_operand(&mut self) -> (u16, u8) {
        let word = self.fetch_word();
        (word & 0x1FFF, (word >> 13) as u8)
    }

    /// Runs one instruction and returns the cycles it took.
    pub(crate) fn step(&mut self) -> u32 {
        self.fetched = 0;
        self.elapsed = 0;
        if self.halted {
            return 2;
        }
        let op = self.fetch();
        let hi = op >> 4;
        let lo = op & 15;

        if op < 0xC0 && (4..=9).contains(&lo) {
            return self.alu_op(hi, lo);
        }

        match op {
            // Moves between registers.
            0x7D => {
                self.a = self.x;
                self.nz(self.a);
                2
            }
            0x5D => {
                self.x = self.a;
                self.nz(self.x);
                2
            }
            0xDD => {
                self.a = self.y;
                self.nz(self.a);
                2
            }
            0xFD => {
                self.y = self.a;
                self.nz(self.y);
                2
            }
            0x9D => {
                self.x = self.sp;
                self.nz(self.x);
                2
            }
            0xBD => {
                self.sp = self.x;
                2
            }
            // Immediate loads.
            0xE8 => {
                let v = self.fetch();
                self.a = self.nz(v);
                2
            }
            0xCD => {
                let v = self.fetch();
                self.x = self.nz(v);
                2
            }
            0x8D => {
                let v = self.fetch();
                self.y = self.nz(v);
                2
            }
            // Loads.
            0xE4 | 0xF4 | 0xE5 | 0xF5 | 0xF6 | 0xE6 | 0xBF | 0xF7 | 0xE7 => self.load_a(op),
            0xF8 | 0xF9 | 0xE9 => {
                let address = match op {
                    0xF8 => {
                        let d = self.fetch();
                        self.direct(d)
                    }
                    0xF9 => {
                        let d = self.fetch();
                        self.direct(d.wrapping_add(self.y))
                    }
                    _ => self.fetch_word(),
                };
                let v = self.read(address);
                self.x = self.nz(v);
                if op == 0xF8 { 3 } else { 4 }
            }
            0xEB | 0xFB | 0xEC => {
                let address = match op {
                    0xEB => {
                        let d = self.fetch();
                        self.direct(d)
                    }
                    0xFB => {
                        let d = self.fetch();
                        self.direct(d.wrapping_add(self.x))
                    }
                    _ => self.fetch_word(),
                };
                let v = self.read(address);
                self.y = self.nz(v);
                if op == 0xEB { 3 } else { 4 }
            }
            0xBA => {
                let d = self.fetch();
                let word = self.direct_word(d);
                self.a = word as u8;
                self.y = (word >> 8) as u8;
                self.nz16(word);
                5
            }
            // Stores.
            0x8F => {
                let v = self.fetch();
                let d = self.fetch();
                let address = self.direct(d);
                self.read(address);
                self.write(address, v);
                5
            }
            0xFA => {
                let source = self.fetch();
                let d = self.fetch();
                let v = self.read(self.direct(source));
                let address = self.direct(d);
                self.write(address, v);
                5
            }
            0xC4 | 0xD8 | 0xCB => {
                let d = self.fetch();
                let address = self.direct(d);
                let v = match op {
                    0xC4 => self.a,
                    0xD8 => self.x,
                    _ => self.y,
                };
                self.read(address);
                self.write(address, v);
                4
            }
            0xD4 | 0xDB | 0xD9 => {
                let d = self.fetch();
                let index = if op == 0xD9 { self.y } else { self.x };
                let address = self.direct(d.wrapping_add(index));
                let v = match op {
                    0xD4 => self.a,
                    0xDB => self.y,
                    _ => self.x,
                };
                self.read(address);
                self.write(address, v);
                5
            }
            0xC5 | 0xC9 | 0xCC => {
                let address = self.fetch_word();
                let v = match op {
                    0xC5 => self.a,
                    0xC9 => self.x,
                    _ => self.y,
                };
                self.read(address);
                self.write(address, v);
                5
            }
            0xD5 | 0xD6 => {
                let base = self.fetch_word();
                let index = if op == 0xD5 { self.x } else { self.y };
                let address = base.wrapping_add(u16::from(index));
                self.read(address);
                self.write(address, self.a);
                6
            }
            0xAF => {
                let address = self.direct(self.x);
                self.write(address, self.a);
                self.x = self.x.wrapping_add(1);
                4
            }
            0xC6 => {
                let address = self.direct(self.x);
                self.read(address);
                self.write(address, self.a);
                4
            }
            0xD7 | 0xC7 => {
                let d = self.fetch();
                let address = if op == 0xD7 {
                    let base = self.direct_word(d);
                    base.wrapping_add(u16::from(self.y))
                } else {
                    self.direct_word(d.wrapping_add(self.x))
                };
                self.read(address);
                self.write(address, self.a);
                7
            }
            0xDA => {
                let d = self.fetch();
                let address = self.direct(d);
                self.read(address);
                self.write(address, self.a);
                let next = self.direct(d.wrapping_add(1));
                self.write(next, self.y);
                5
            }
            // Stack.
            0x2D => {
                self.push(self.a);
                4
            }
            0x4D => {
                self.push(self.x);
                4
            }
            0x6D => {
                self.push(self.y);
                4
            }
            0x0D => {
                self.push(self.psw);
                4
            }
            0xAE => {
                let v = self.pop();
                self.a = v;
                4
            }
            0xCE => {
                let v = self.pop();
                self.x = v;
                4
            }
            0xEE => {
                let v = self.pop();
                self.y = v;
                4
            }
            0x8E => {
                self.psw = self.pop();
                4
            }
            // Compares with the index registers.
            0xC8 | 0xAD => {
                let v = self.fetch();
                let r = if op == 0xC8 { self.x } else { self.y };
                self.cmp(r, v);
                2
            }
            0x3E | 0x7E => {
                let d = self.fetch();
                let v = self.read(self.direct(d));
                let r = if op == 0x3E { self.x } else { self.y };
                self.cmp(r, v);
                3
            }
            0x1E | 0x5E => {
                let address = self.fetch_word();
                let v = self.read(address);
                let r = if op == 0x1E { self.x } else { self.y };
                self.cmp(r, v);
                4
            }
            // Shifts, rotates, increments and decrements.
            0x0B | 0x1B | 0x2B | 0x3B | 0x4B | 0x5B | 0x6B | 0x7B | 0x8B | 0x9B | 0xAB | 0xBB => {
                let d = self.fetch();
                let offset = if lo == 0xB && hi & 1 == 1 { d.wrapping_add(self.x) } else { d };
                let address = self.direct(offset);
                let v = self.read(address);
                let r = self.rmw(hi >> 1, v);
                self.write(address, r);
                if hi & 1 == 1 { 5 } else { 4 }
            }
            0x0C | 0x2C | 0x4C | 0x6C | 0x8C | 0xAC => {
                let address = self.fetch_word();
                let v = self.read(address);
                let r = self.rmw(hi >> 1, v);
                self.write(address, r);
                5
            }
            0x1C | 0x3C | 0x5C | 0x7C | 0x9C | 0xBC => {
                self.a = self.rmw(hi >> 1, self.a);
                2
            }
            0x1D => {
                self.x = self.rmw(4, self.x);
                2
            }
            0x3D => {
                self.x = self.rmw(5, self.x);
                2
            }
            0xDC => {
                self.y = self.rmw(4, self.y);
                2
            }
            0xFC => {
                self.y = self.rmw(5, self.y);
                2
            }
            // 16-bit operations.
            0x7A | 0x9A | 0x5A => {
                let d = self.fetch();
                let m = u32::from(self.direct_word(d));
                let ya = u32::from(self.y) << 8 | u32::from(self.a);
                match op {
                    0x7A => {
                        let sum = ya + m;
                        let r = sum as u16;
                        self.flag(V, (ya ^ m) & 0x8000 == 0 && (ya ^ u32::from(r)) & 0x8000 != 0);
                        self.flag(H, (ya & 0xFFF) + (m & 0xFFF) > 0xFFF);
                        self.flag(C, sum > 0xFFFF);
                        self.nz16(r);
                        self.a = r as u8;
                        self.y = (r >> 8) as u8;
                        5
                    }
                    0x9A => {
                        let r = ya.wrapping_sub(m) as u16;
                        self.flag(V, (ya ^ m) & 0x8000 != 0 && (ya ^ u32::from(r)) & 0x8000 != 0);
                        self.flag(H, (ya & 0xFFF) >= (m & 0xFFF));
                        self.flag(C, ya >= m);
                        self.nz16(r);
                        self.a = r as u8;
                        self.y = (r >> 8) as u8;
                        5
                    }
                    _ => {
                        self.flag(C, ya >= m);
                        self.nz16(ya.wrapping_sub(m) as u16);
                        4
                    }
                }
            }
            0x3A | 0x1A => {
                let d = self.fetch();
                let word = self.direct_word(d);
                let r = if op == 0x3A { word.wrapping_add(1) } else { word.wrapping_sub(1) };
                self.nz16(r);
                let lo_address = self.direct(d);
                let hi_address = self.direct(d.wrapping_add(1));
                self.write(lo_address, r as u8);
                self.write(hi_address, (r >> 8) as u8);
                6
            }
            0xCF => {
                let r = u16::from(self.y) * u16::from(self.a);
                self.a = r as u8;
                self.y = (r >> 8) as u8;
                self.nz(self.y);
                9
            }
            0x9E => {
                // The nine-step division of the documentation, with its
                // overflow and half-carry effects.
                let x = u32::from(self.x);
                let divisor = x * 0x200;
                self.flag(H, (self.x & 15) <= (self.y & 15));
                let mut temp = u32::from(self.y) << 8 | u32::from(self.a);
                for _ in 0..9 {
                    temp <<= 1;
                    if temp & 0x2_0000 != 0 {
                        temp ^= 0x2_0001;
                    }
                    if temp >= divisor {
                        temp ^= 1;
                    }
                    if temp & 1 != 0 {
                        temp = temp.wrapping_sub(divisor) & 0x1_FFFF;
                    }
                }
                self.a = temp as u8;
                self.y = (temp >> 9) as u8;
                self.flag(V, temp & 0x100 != 0);
                self.nz(self.a);
                12
            }
            // Bit operations.
            0x02 | 0x22 | 0x42 | 0x62 | 0x82 | 0xA2 | 0xC2 | 0xE2 | 0x12 | 0x32 | 0x52
            | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                let d = self.fetch();
                let address = self.direct(d);
                let v = self.read(address);
                let bit = 1u8 << (op >> 5);
                self.write(address, if hi & 1 == 0 { v | bit } else { v & !bit });
                4
            }
            0x0A | 0x2A | 0x4A | 0x6A | 0x8A | 0xAA => {
                let (address, bit) = self.bit_operand();
                let b = self.read(address) >> bit & 1 != 0;
                let c = self.psw & C != 0;
                let (result, cycles) = match op {
                    0x0A => (c | b, 5),
                    0x2A => (c | !b, 5),
                    0x4A => (c & b, 4),
                    0x6A => (c & !b, 4),
                    0x8A => (c ^ b, 5),
                    _ => (b, 4),
                };
                self.flag(C, result);
                cycles
            }
            0xCA => {
                let (address, bit) = self.bit_operand();
                let v = self.read(address);
                let c = self.psw & C != 0;
                self.write(address, v & !(1 << bit) | u8::from(c) << bit);
                6
            }
            0xEA => {
                let (address, bit) = self.bit_operand();
                let v = self.read(address);
                self.write(address, v ^ (1 << bit));
                5
            }
            0x60 => {
                self.flag(C, false);
                2
            }
            0x80 => {
                self.flag(C, true);
                2
            }
            0xED => {
                self.psw ^= C;
                3
            }
            0xE0 => {
                self.psw &= !(V | H);
                2
            }
            0x20 => {
                self.flag(P, false);
                2
            }
            0x40 => {
                self.flag(P, true);
                2
            }
            0xA0 => {
                self.flag(I, true);
                3
            }
            0xC0 => {
                self.flag(I, false);
                3
            }
            0xDF => {
                if self.psw & C != 0 || self.a > 0x99 {
                    self.a = self.a.wrapping_add(0x60);
                    self.flag(C, true);
                }
                if self.psw & H != 0 || self.a & 15 > 9 {
                    self.a = self.a.wrapping_add(6);
                }
                self.nz(self.a);
                3
            }
            0xBE => {
                if self.psw & C == 0 || self.a > 0x99 {
                    self.a = self.a.wrapping_sub(0x60);
                    self.flag(C, false);
                }
                if self.psw & H == 0 || self.a & 15 > 9 {
                    self.a = self.a.wrapping_sub(6);
                }
                self.nz(self.a);
                3
            }
            0x9F => {
                self.a = self.a.rotate_left(4);
                self.nz(self.a);
                5
            }
            0x0E | 0x4E => {
                let address = self.fetch_word();
                let v = self.read(address);
                self.nz(self.a.wrapping_sub(v));
                self.write(address, if op == 0x0E { v | self.a } else { v & !self.a });
                6
            }
            // Branches.
            0x10 => self.branch(self.psw & N == 0, 2),
            0x30 => self.branch(self.psw & N != 0, 2),
            0x50 => self.branch(self.psw & V == 0, 2),
            0x70 => self.branch(self.psw & V != 0, 2),
            0x90 => self.branch(self.psw & C == 0, 2),
            0xB0 => self.branch(self.psw & C != 0, 2),
            0xD0 => self.branch(self.psw & Z == 0, 2),
            0xF0 => self.branch(self.psw & Z != 0, 2),
            0x2F => self.branch(true, 2),
            0x03 | 0x23 | 0x43 | 0x63 | 0x83 | 0xA3 | 0xC3 | 0xE3 | 0x13 | 0x33 | 0x53
            | 0x73 | 0x93 | 0xB3 | 0xD3 | 0xF3 => {
                let d = self.fetch();
                let v = self.read(self.direct(d));
                let set = v >> (op >> 5) & 1 != 0;
                self.branch(set == (hi & 1 == 0), 5)
            }
            0x2E => {
                let d = self.fetch();
                let v = self.read(self.direct(d));
                self.branch(self.a != v, 5)
            }
            0xDE => {
                let d = self.fetch();
                let v = self.read(self.direct(d.wrapping_add(self.x)));
                self.branch(self.a != v, 6)
            }
            0xFE => {
                self.y = self.y.wrapping_sub(1);
                self.branch(self.y != 0, 4)
            }
            0x6E => {
                let d = self.fetch();
                let address = self.direct(d);
                let v = self.read(address).wrapping_sub(1);
                self.write(address, v);
                self.branch(v != 0, 5)
            }
            // Jumps and calls.
            0x5F => {
                self.pc = self.fetch_word();
                3
            }
            0x1F => {
                let base = self.fetch_word();
                let address = base.wrapping_add(u16::from(self.x));
                let lo = self.read(address);
                let hi = self.read(address.wrapping_add(1));
                self.pc = u16::from(hi) << 8 | u16::from(lo);
                6
            }
            0x3F => {
                let target = self.fetch_word();
                self.push_word(self.pc);
                self.pc = target;
                8
            }
            0x4F => {
                let low = self.fetch();
                self.push_word(self.pc);
                self.pc = 0xFF00 | u16::from(low);
                6
            }
            0x01 | 0x11 | 0x21 | 0x31 | 0x41 | 0x51 | 0x61 | 0x71 | 0x81 | 0x91 | 0xA1
            | 0xB1 | 0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                self.push_word(self.pc);
                let vector = 0xFFDE - u16::from(hi) * 2;
                let lo = self.read(vector);
                let high = self.read(vector + 1);
                self.pc = u16::from(high) << 8 | u16::from(lo);
                8
            }
            0x6F => {
                self.pc = self.pop_word();
                5
            }
            0x7F => {
                self.psw = self.pop();
                self.pc = self.pop_word();
                6
            }
            0x0F => {
                self.push_word(self.pc);
                self.push(self.psw);
                self.flag(B, true);
                self.flag(I, false);
                let lo = self.read(0xFFDE);
                let high = self.read(0xFFDF);
                self.pc = u16::from(high) << 8 | u16::from(lo);
                8
            }
            0x00 => 2,
            0xEF | 0xFF => {
                self.halted = true;
                self.pc = self.pc.wrapping_sub(1);
                2
            }
            _ => 2,
        }
    }

    /// The loads into A that share the table of addressing modes.
    fn load_a(&mut self, op: u8) -> u32 {
        let (address, cycles) = match op {
            0xE4 => {
                let d = self.fetch();
                (self.direct(d), 3)
            }
            0xF4 => {
                let d = self.fetch();
                (self.direct(d.wrapping_add(self.x)), 4)
            }
            0xE5 => (self.fetch_word(), 4),
            0xF5 => (self.fetch_word().wrapping_add(u16::from(self.x)), 5),
            0xF6 => (self.fetch_word().wrapping_add(u16::from(self.y)), 5),
            0xE6 => (self.direct(self.x), 3),
            0xBF => {
                let address = self.direct(self.x);
                self.x = self.x.wrapping_add(1);
                (address, 4)
            }
            0xF7 => {
                let d = self.fetch();
                let base = self.direct_word(d);
                (base.wrapping_add(u16::from(self.y)), 6)
            }
            _ => {
                let d = self.fetch();
                (self.direct_word(d.wrapping_add(self.x)), 6)
            }
        };
        let v = self.read(address);
        self.a = self.nz(v);
        cycles
    }

    /// The six logic and arithmetic operations in their addressing modes.
    fn alu_op(&mut self, hi: u8, lo: u8) -> u32 {
        let operation = hi >> 1;
        let odd = hi & 1 == 1;
        let compare = operation == 3;
        let a = self.a;
        match (lo, odd) {
            (4..=7, _) => {
                let (address, cycles) = match (lo, odd) {
                    (4, false) => {
                        let d = self.fetch();
                        (self.direct(d), 3)
                    }
                    (4, true) => {
                        let d = self.fetch();
                        (self.direct(d.wrapping_add(self.x)), 4)
                    }
                    (5, false) => (self.fetch_word(), 4),
                    (5, true) => (self.fetch_word().wrapping_add(u16::from(self.x)), 5),
                    (6, false) => (self.direct(self.x), 3),
                    (6, true) => (self.fetch_word().wrapping_add(u16::from(self.y)), 5),
                    (7, false) => {
                        let d = self.fetch();
                        (self.direct_word(d.wrapping_add(self.x)), 6)
                    }
                    _ => {
                        let d = self.fetch();
                        let base = self.direct_word(d);
                        (base.wrapping_add(u16::from(self.y)), 6)
                    }
                };
                let b = self.read(address);
                self.a = self.alu(operation, a, b);
                cycles
            }
            (8, false) => {
                let b = self.fetch();
                self.a = self.alu(operation, a, b);
                2
            }
            (8, true) => {
                let b = self.fetch();
                let d = self.fetch();
                let address = self.direct(d);
                let v = self.read(address);
                let r = self.alu(operation, v, b);
                if !compare {
                    self.write(address, r);
                }
                5
            }
            (9, false) => {
                let source = self.fetch();
                let d = self.fetch();
                let b = self.read(self.direct(source));
                let address = self.direct(d);
                let v = self.read(address);
                let r = self.alu(operation, v, b);
                if !compare {
                    self.write(address, r);
                }
                6
            }
            _ => {
                let b = self.read(self.direct(self.y));
                let address = self.direct(self.x);
                let v = self.read(address);
                let r = self.alu(operation, v, b);
                if !compare {
                    self.write(address, r);
                }
                5
            }
        }
    }
}
