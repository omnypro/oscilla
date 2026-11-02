//! The clock and everything that runs from it: the display's line timing
//! (only its flags and interrupts, since nothing is drawn), the four timers,
//! the four DMA channels and the interrupt controller. Events carry the exact
//! master-clock cycle they belong to, so a late handler never drifts the
//! schedule: a timer's next overflow is computed from its last one.

use crate::state::*;

impl Machine {
    pub(crate) fn sys_warn(&mut self, text: &str) {
        // The buffer holds a trailing NUL and never grows past 2048 bytes.
        let n = text.len();
        let len = self.warnings.len() - 1;
        if len + n + 2 >= 2048 {
            return;
        }
        self.warnings.pop();
        self.warnings.extend_from_slice(text.as_bytes());
        self.warnings.push(b'\n');
        self.warnings.push(0);
    }

    pub(crate) fn sys_schedule(&mut self) {
        let mut t = self.ev_video;
        if self.ev_sample < t {
            t = self.ev_sample;
        }
        if self.ev_frameseq < t {
            t = self.ev_frameseq;
        }
        for i in 0..4 {
            if self.ev_timer[i] < t {
                t = self.ev_timer[i];
            }
        }
        self.next_event = t;
    }

    pub(crate) fn sys_raise_irq(&mut self, bits: u16) {
        self.iflags |= bits;
        if self.cpu.halted && (self.ie & self.iflags) != 0 {
            self.cpu.halted = false;
        }
    }

    // ---- Timers --------------------------------------------------------

    fn timer_reschedule(&mut self, i: usize) {
        let t = &self.timer[i];
        self.ev_timer[i] = if t.running {
            t.base_time + (u64::from(0x1_0000u32 - u32::from(t.counter)) << t.shift)
        } else {
            NEVER
        };
    }

    pub(crate) fn timer_read(&self, i: usize) -> u16 {
        let t = &self.timer[i];
        if !t.running {
            return t.counter;
        }
        let steps = self.now.wrapping_sub(t.base_time) >> t.shift;
        let mut v = u64::from(t.counter).wrapping_add(steps);
        if v > 0xFFFF {
            // Read between an overflow and its event: count on from the reload.
            let period = u64::from(0x1_0000u32 - u32::from(t.reload));
            v = u64::from(t.reload) + (v - 0x1_0000) % period;
        }
        v as u16
    }

    fn timer_overflow(&mut self, i: usize, when: u64) {
        let t = &mut self.timer[i];
        t.counter = t.reload;
        if t.running {
            t.base_time = when;
            self.timer_reschedule(i);
        }
        if self.timer[i].control & 0x40 != 0 {
            self.sys_raise_irq(IRQ_TIMER0 << i);
        }
        if i < 2 {
            self.apu_timer_overflow(i);
        }
        if i < 3 {
            let n = &mut self.timer[i + 1];
            if n.cascade {
                n.counter = n.counter.wrapping_add(1);
                if n.counter == 0 {
                    self.timer_overflow(i + 1, when);
                }
            }
        }
    }

    pub(crate) fn timer_write_control(&mut self, i: usize, value: u16) {
        const SHIFTS: [u32; 4] = [0, 6, 8, 10];
        let was_on = self.timer[i].control & 0x80 != 0;
        let was_running = self.timer[i].running;
        let current = self.timer_read(i);
        let on = value & 0x80 != 0;
        let cascade = i > 0 && value & 4 != 0;
        let shift = SHIFTS[(value & 3) as usize];
        let now = self.now;
        let t = &mut self.timer[i];
        t.control = value;
        if on && !was_on {
            t.counter = t.reload;
            t.base_time = now;
        } else if !(was_running && on && !cascade && shift == t.shift) {
            t.counter = current;
            t.base_time = now;
        }
        t.shift = shift;
        t.running = on && !cascade;
        t.cascade = on && cascade;
        self.timer_reschedule(i);
        self.sys_schedule();
    }

    // ---- DMA -----------------------------------------------------------

    fn dma_units(d: &DmaChannel, i: usize) -> u32 {
        let mut n = u32::from(d.cnt_l) & if i == 3 { 0xFFFF } else { 0x3FFF };
        if n == 0 {
            n = if i == 3 { 0x1_0000 } else { 0x4000 };
        }
        n
    }

    pub(crate) fn dma_write_control(&mut self, i: usize, mut value: u16) {
        let d = &mut self.dma[i];
        let old = d.cnt_h;
        if i != 3 {
            value &= !0x0800;
        }
        d.cnt_h = value;
        if old & 0x8000 == 0 && value & 0x8000 != 0 {
            d.src = d.sad & if i == 0 { 0x07FF_FFFF } else { 0x0FFF_FFFF };
            d.dst = d.dad & if i == 3 { 0x0FFF_FFFF } else { 0x07FF_FFFF };
            d.count = Self::dma_units(d, i);
            if (value >> 12) & 3 == 0 {
                d.pending = true;
                self.dma_any_pending = true;
            }
        }
        if value & 0x8000 == 0 {
            self.dma[i].pending = false;
        }
    }

    fn dma_trigger(&mut self, timing: u16) {
        for i in 0..4 {
            let d = &mut self.dma[i];
            if d.cnt_h & 0x8000 != 0 && (d.cnt_h >> 12) & 3 == timing {
                if timing == 3 && (i == 1 || i == 2) {
                    continue; // sound DMA waits for its FIFO
                }
                d.pending = true;
                self.dma_any_pending = true;
            }
        }
    }

    /// When a FIFO has room for 16 more bytes, the sound DMA aimed at it starts.
    pub(crate) fn dma_fifo_request(&mut self, ch: usize) {
        let target = 0x0400_00A0u32 + 4 * ch as u32;
        for i in 1..=2 {
            let d = &mut self.dma[i];
            if d.cnt_h & 0x8000 != 0 && (d.cnt_h >> 12) & 3 == 3 && (d.dad & 0x0FFF_FFFC) == target
            {
                d.pending = true;
                self.dma_any_pending = true;
            }
        }
    }

    fn dma_transfer(&mut self, i: usize) {
        self.dma[i].pending = false;
        let cnt = self.dma[i].cnt_h;
        let timing = (cnt >> 12) & 3;
        let fifo = timing == 3 && (i == 1 || i == 2);
        let word = fifo || cnt & 0x400 != 0;
        let units = if fifo { 4 } else { self.dma[i].count };
        let sctl = (cnt >> 7) & 3;
        let dctl = if fifo { 2 } else { (cnt >> 5) & 3 };
        let step: i32 = if word { 4 } else { 2 };
        let mut sd: i32 = match sctl {
            1 => -step,
            2 => 0,
            _ => step,
        };
        let dd: i32 = match dctl {
            1 => -step,
            2 => 0,
            _ => step,
        };
        let sregion = (self.dma[i].src >> 24) & 0xF;
        if (0x8..=0xD).contains(&sregion) {
            sd = step; // cartridge reads always advance
        }
        let mut cycles: u64 = 2;
        let mask = !((step - 1) as u32);
        for k in 0..units {
            let s = self.dma[i].src & mask;
            let t = self.dma[i].dst & mask;
            if word {
                let v = if s >= 0x0200_0000 {
                    self.bus_read32(s)
                } else {
                    self.dma_latch
                };
                self.dma_latch = v;
                self.bus_write32(t, v, Source::Dma);
                cycles += self.bus_cycles(s, 4, k > 0) as u64 + self.bus_cycles(t, 4, k > 0) as u64;
            } else {
                let v = if s >= 0x0200_0000 {
                    self.bus_read16(s)
                } else {
                    (self.dma_latch >> ((t & 2) * 8)) & 0xFFFF
                };
                self.dma_latch = v | (v << 16);
                self.bus_write16(t, v, Source::Dma);
                cycles += self.bus_cycles(s, 2, k > 0) as u64 + self.bus_cycles(t, 2, k > 0) as u64;
            }
            let d = &mut self.dma[i];
            d.src = d.src.wrapping_add(sd as u32);
            d.dst = d.dst.wrapping_add(dd as u32);
        }
        self.now += cycles;
        if cnt & 0x4000 != 0 {
            self.sys_raise_irq(IRQ_DMA0 << i);
        }
        if cnt & 0x200 != 0 && timing != 0 {
            let d = &mut self.dma[i];
            if !fifo {
                d.count = Self::dma_units(d, i);
            }
            if dctl == 3 {
                d.dst = d.dad & if i == 3 { 0x0FFF_FFFF } else { 0x07FF_FFFF };
            }
        } else {
            self.dma[i].cnt_h &= 0x7FFF;
        }
    }

    pub(crate) fn dma_run_pending(&mut self) {
        if self.dma_active {
            return;
        }
        self.dma_active = true;
        loop {
            let mut i = 0;
            while i < 4 && !self.dma[i].pending {
                i += 1;
            }
            if i == 4 {
                break;
            }
            self.dma_transfer(i);
        }
        self.dma_any_pending = false;
        self.dma_active = false;
    }

    // ---- Display timing ------------------------------------------------

    fn video_event(&mut self) {
        let stat = u16::from(self.io[4]) | (u16::from(self.io[5]) << 8);
        if !self.video_hblank {
            self.hblank_flag = true;
            if stat & 0x10 != 0 {
                self.sys_raise_irq(IRQ_HBLANK);
            }
            if self.vcount < 160 {
                self.dma_trigger(2);
            }
            self.video_hblank = true;
            self.ev_video += LINE_CYCLES - HDRAW_CYCLES;
            return;
        }
        self.video_hblank = false;
        self.hblank_flag = false;
        self.ev_video += HDRAW_CYCLES;
        self.vcount = (self.vcount + 1) % LINES_PER_FRAME as i32;
        if self.vcount == 160 {
            self.vblank_flag = true;
            if stat & 0x08 != 0 {
                self.sys_raise_irq(IRQ_VBLANK);
            }
            self.dma_trigger(1);
        } else if self.vcount == 227 {
            self.vblank_flag = false;
        }
        let matched = self.vcount == i32::from(stat >> 8);
        if matched && !self.vmatch_flag && stat & 0x20 != 0 {
            self.sys_raise_irq(IRQ_VCOUNT);
        }
        self.vmatch_flag = matched;
    }

    // ---- I/O registers -------------------------------------------------

    fn raw16(&self, off: u32) -> u16 {
        u16::from(self.io[off as usize]) | (u16::from(self.io[off as usize + 1]) << 8)
    }

    pub(crate) fn io_read16(&self, addr: u32) -> u16 {
        let off = addr & 0x3FE;
        if (0x60..0xA8).contains(&off) {
            return self.apu_read(off);
        }
        match off {
            0x004 => {
                return (self.raw16(4) & 0xFFF8)
                    | u16::from(self.vblank_flag)
                    | (u16::from(self.hblank_flag) << 1)
                    | (u16::from(self.vmatch_flag) << 2);
            }
            0x006 => return self.vcount as u16,
            0x100 => return self.timer_read(0),
            0x104 => return self.timer_read(1),
            0x108 => return self.timer_read(2),
            0x10C => return self.timer_read(3),
            0x102 => return self.timer[0].control,
            0x106 => return self.timer[1].control,
            0x10A => return self.timer[2].control,
            0x10E => return self.timer[3].control,
            0x0BA => return self.dma[0].cnt_h,
            0x0C6 => return self.dma[1].cnt_h,
            0x0D2 => return self.dma[2].cnt_h,
            0x0DE => return self.dma[3].cnt_h,
            0x130 => return 0x03FF,
            0x200 => return self.ie,
            0x202 => return self.iflags,
            0x208 => return self.ime,
            0x300 => return u16::from(self.io[0x300]),
            _ => {}
        }
        if (0xB0..0xE0).contains(&off) {
            return 0; // the rest of DMA is write-only
        }
        self.raw16(off)
    }

    fn dma_register_write(&mut self, off: u32, value: u16) {
        let i = ((off - 0xB0) / 12) as usize;
        let d = &mut self.dma[i];
        match (off - 0xB0) % 12 {
            0 => d.sad = (d.sad & 0xFFFF_0000) | u32::from(value),
            2 => d.sad = (d.sad & 0x0000_FFFF) | (u32::from(value) << 16),
            4 => d.dad = (d.dad & 0xFFFF_0000) | u32::from(value),
            6 => d.dad = (d.dad & 0x0000_FFFF) | (u32::from(value) << 16),
            8 => d.cnt_l = value,
            _ => self.dma_write_control(i, value),
        }
    }

    fn halt_write(&mut self, value: u8) {
        if value & 0x80 != 0 && !self.warned_misc {
            self.warned_misc = true;
            self.sys_warn("The game asked for Stop mode; it is treated as Halt.");
        }
        self.cpu.halted = true;
        if self.ie & self.iflags != 0 {
            self.cpu.halted = false;
        }
    }

    pub(crate) fn io_write16(&mut self, addr: u32, value: u16) {
        let off = addr & 0x3FE;
        if (0x60..0xA8).contains(&off) {
            self.apu_write(off, value, 0xFFFF);
            return;
        }
        match off {
            0x004 => {
                let v = (self.raw16(4) & 7) | (value & 0xFFF8);
                self.io[4] = v as u8;
                self.io[5] = (v >> 8) as u8;
                let matched = self.vcount == i32::from(v >> 8);
                if matched && !self.vmatch_flag && v & 0x20 != 0 {
                    self.sys_raise_irq(IRQ_VCOUNT);
                }
                self.vmatch_flag = matched;
                return;
            }
            0x006 => return,
            0x100 | 0x104 | 0x108 | 0x10C => {
                self.timer[((off - 0x100) / 4) as usize].reload = value;
            }
            0x102 | 0x106 | 0x10A | 0x10E => {
                self.timer_write_control(((off - 0x102) / 4) as usize, value);
            }
            0x200 => {
                self.ie = value & 0x3FFF;
                if self.cpu.halted && self.ie & self.iflags != 0 {
                    self.cpu.halted = false;
                }
            }
            0x202 => {
                self.iflags &= !value;
                return;
            }
            0x204 => {
                self.io[0x204] = value as u8;
                self.io[0x205] = ((value >> 8) & 0x7F) as u8;
                self.bus_update_waits();
                return;
            }
            0x208 => self.ime = value & 1,
            0x300 => {
                self.io[0x300] = (value & 1) as u8;
                self.halt_write((value >> 8) as u8);
                return;
            }
            _ => {
                if (0xB0..0xE0).contains(&off) {
                    self.dma_register_write(off, value);
                }
            }
        }
        self.io[off as usize] = value as u8;
        self.io[off as usize + 1] = (value >> 8) as u8;
    }

    pub(crate) fn io_write8(&mut self, addr: u32, value: u8) {
        let off = addr & 0x3FF;
        let half = off & !1;
        let hi = off & 1 != 0;
        if (0x60..0xA8).contains(&off) {
            let v = if hi {
                u16::from(value) << 8
            } else {
                u16::from(value)
            };
            self.apu_write(half, v, if hi { 0xFF00 } else { 0x00FF });
            return;
        }
        match off {
            0x202 => {
                self.iflags &= !(if hi {
                    u16::from(value) << 8
                } else {
                    u16::from(value)
                });
                return;
            }
            0x203 => {
                self.iflags &= !(u16::from(value) << 8);
                return;
            }
            0x300 => {
                self.io[0x300] = value & 1;
                return;
            }
            0x301 => {
                self.halt_write(value);
                return;
            }
            _ => {}
        }
        let cur = match half {
            0x200 => self.ie,
            0x208 => self.ime,
            0x102 | 0x106 | 0x10A | 0x10E => self.timer[((half - 0x102) / 4) as usize].control,
            0x100 | 0x104 | 0x108 | 0x10C => self.timer[((half - 0x100) / 4) as usize].reload,
            0x0BA | 0x0C6 | 0x0D2 | 0x0DE => self.dma[((half - 0xB0) / 12) as usize].cnt_h,
            _ => self.raw16(half),
        };
        let merged = if hi {
            (cur & 0x00FF) | (u16::from(value) << 8)
        } else {
            (cur & 0xFF00) | u16::from(value)
        };
        self.io_write16(half, merged);
    }

    // ---- The run loop --------------------------------------------------

    fn process_events(&mut self) {
        while self.next_event <= self.now {
            let t = self.next_event;
            if self.ev_video == t {
                self.video_event();
            } else if self.ev_timer.contains(&t) {
                let mut i = 0;
                while self.ev_timer[i] != t {
                    i += 1;
                }
                self.timer_overflow(i, t);
            } else if self.ev_sample == t {
                let saved = self.now;
                self.now = t;
                self.apu_sample();
                self.now = saved;
                self.ev_sample += 1u64 << self.apu.sample_shift;
            } else if self.ev_frameseq == t {
                let saved = self.now;
                self.now = t;
                self.apu_frame_step();
                self.now = saved;
                self.ev_frameseq += 32768;
            }
            self.sys_schedule();
        }
    }

    pub(crate) fn sys_run_until(&mut self, target: u64) {
        while self.now < target {
            if self.dma_any_pending {
                self.dma_run_pending();
            }
            let mut limit = self.next_event.min(target);
            if self.cpu.halted {
                if self.ie & self.iflags != 0 {
                    self.cpu.halted = false;
                } else if self.now < limit {
                    self.now = limit;
                }
            }
            while !self.cpu.halted && self.now < limit {
                self.now += self.cpu_step() as u64;
                if self.dma_any_pending {
                    self.dma_run_pending();
                    if self.next_event < limit {
                        limit = self.next_event;
                    }
                }
                if self.next_event < limit {
                    limit = self.next_event;
                }
            }
            self.process_events();
        }
    }

    pub(crate) fn sys_reset(&mut self) {
        self.ewram.fill(0);
        self.iwram.fill(0);
        self.io.fill(0);
        self.pal.fill(0);
        self.vram.fill(0);
        self.oam.fill(0);
        if let Some(image) = &self.ewram_image {
            self.ewram.copy_from_slice(image);
        }
        self.dma = Default::default();
        self.timer = Default::default();
        self.now = 0;
        self.ie = 0;
        self.iflags = 0;
        self.ime = 0;
        self.dma_active = false;
        self.dma_any_pending = false;
        self.dma_latch = 0;
        // What the BIOS leaves behind when it starts a cartridge.
        self.io[0x000] = 0x80;
        self.io[0x088] = 0x00;
        self.io[0x089] = 0x02;
        self.io[0x300] = 0x01;
        self.bios_latch = 0xE129_F000;
        self.bus_update_waits();
        self.pf_addr = 1; // no cartridge address is odd, so the buffer starts empty
        self.pf_count = 0;
        self.pf_progress = 0;
        self.pf_fresh = false;
        self.pf_busy = 0;
        self.pf_time = 0;
        // The BIOS hands over partway through a frame. Line 126, 884 cycles in,
        // puts the first VBlank at cycle 41004.
        self.vcount = 126;
        self.vblank_flag = false;
        self.hblank_flag = false;
        self.vmatch_flag = false;
        self.video_hblank = false;
        self.ev_video = HDRAW_CYCLES - 884;
        self.ev_timer = [NEVER; 4];
        self.ev_frameseq = 32768;
        self.cpu_reset_post_bios();
        self.apu_reset();
        self.ev_sample = 1u64 << self.apu.sample_shift;
        self.out_reset();
        self.sys_schedule();
    }
}
