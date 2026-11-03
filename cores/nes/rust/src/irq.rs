//! The interrupts of a version 2 file: the timer at 0x401B to 0x401D, the IRQ
//! vector, the NMI that calls the play routine, and the second init routine
//! of a file whose init does not return.
//!
//! A file without these flags never reaches this code: `Machine::run_until`
//! runs it as before.

use crate::bus::WARN_STALL;
use crate::cpu::{BREAK, I, UNUSED};
use crate::state::*;
use oscilla_nsf::nsf2;

/// A routine that has run this long without returning is abandoned.
fn limit(clock: (u64, u64)) -> u64 {
    clock.0 / clock.1
}

impl Machine {
    /// Brings the timer's flag to cycle `t`.
    fn irq_sync(&mut self, t: u64) {
        let timer = &mut self.irq_timer;
        if timer.active && timer.next <= t {
            let period = u64::from(timer.reload) + 1;
            timer.flag = true;
            timer.next += ((t - timer.next) / period + 1) * period;
        }
    }

    /// A read of 0x401B to 0x401D.
    pub(crate) fn irq_read(&mut self, address: u16) -> u8 {
        let t = self.now;
        self.irq_sync(t);
        let timer = &mut self.irq_timer;
        match address {
            0x401B => timer.reload as u8,
            0x401C => (timer.reload >> 8) as u8,
            _ => {
                // The read hands out the flag and clears it.
                let v = u8::from(timer.flag) << 7 | u8::from(timer.active);
                timer.flag = false;
                v
            }
        }
    }

    /// A store to 0x401B to 0x401D.
    pub(crate) fn irq_write(&mut self, address: u16, v: u8) {
        let t = self.now;
        self.irq_sync(t);
        let timer = &mut self.irq_timer;
        match address {
            0x401B => timer.reload = timer.reload & 0xFF00 | u16::from(v),
            0x401C => timer.reload = timer.reload & 0x00FF | u16::from(v) << 8,
            _ => {
                if v & 1 == 0 {
                    timer.active = false;
                } else if !timer.active {
                    timer.active = true;
                    timer.next = t + u64::from(timer.reload) + 1;
                }
            }
        }
    }

    /// Whether the delta channel or the frame counter may raise its interrupt.
    fn apu_irq_possible(&self) -> bool {
        let a = &self.apu;
        a.frame.irq
            || a.dmc.irq
            || (!a.frame.inhibit && !a.frame.five_step)
            || (a.dmc.irq_enable && a.dmc.remaining > 0)
    }

    /// Whether the IRQ line is held low at this cycle: the timer's flag, the
    /// frame counter's or the delta channel's.
    fn irq_line(&mut self) -> bool {
        let t = self.now;
        self.irq_sync(t);
        if self.irq_timer.flag {
            return true;
        }
        if !self.apu_irq_possible() {
            return false;
        }
        self.sync_apu();
        self.apu.frame.irq || self.apu.dmc.irq
    }

    /// Writes a byte to the stack with no bus cycle: the wrapper that the
    /// player puts around the play routine is not the driver's.
    fn push_free(&mut self, v: u8) {
        let sp = self.cpu.sp;
        self.ram[0x100 | usize::from(sp)] = v;
        self.cpu.sp = sp.wrapping_sub(1);
    }

    /// The seven cycles of an interrupt: two reads of the next instruction,
    /// the program counter and the flags pushed, and the vector read.
    fn interrupt_entry(&mut self, vector: u16) {
        let pc = self.cpu.pc;
        self.read(pc);
        self.read(pc);
        self.push((pc >> 8) as u8);
        self.push(pc as u8);
        let p = self.cpu.p & !BREAK | UNUSED;
        self.push(p);
        self.cpu.p |= I;
        self.poll_i = true;
        let lo = self.read(vector);
        let hi = self.read(vector.wrapping_add(1));
        self.cpu.pc = u16::from(hi) << 8 | u16::from(lo);
    }

    fn enter_irq(&mut self) {
        self.cpu.locked = false;
        self.phase = Phase::Routine;
        self.interrupt_entry(0xFFFE);
    }

    /// Calls the play routine as an NMI would: the interrupted state goes on
    /// the stack, the routine runs with the I flag set, and its return puts
    /// the state back.
    fn enter_play(&mut self) {
        if self.phase == Phase::Idle {
            // Sample reads while the CPU waits cost it nothing.
            let t = self.now;
            self.apu_advance(t);
            self.apu.stall = 0;
        }
        let (t, frame) = (self.now, self.frame);
        if let Some(o) = self.observer.0.as_mut() {
            o.play_call(t, frame as i64);
        }
        self.frame += 1;
        let c = &self.cpu;
        self.interrupted = Interrupted {
            pc: c.pc,
            p: c.p,
            a: c.a,
            x: c.x,
            y: c.y,
            sp: c.sp,
        };
        self.cpu.locked = false;
        self.phase = Phase::Routine;
        self.interrupt_entry(0xFFFA);
        let (a, x, y) = (self.cpu.a, self.cpu.x, self.cpu.y);
        self.push_free(a);
        self.push_free(x);
        self.push_free(y);
        let ret = PLAY_RETURN.wrapping_sub(1);
        self.push_free((ret >> 8) as u8);
        self.push_free(ret as u8);
        self.cpu.pc = self.play;
        self.in_play = true;
        self.play_start = self.now;
    }

    fn leave_play(&mut self) {
        let i = self.interrupted;
        let c = &mut self.cpu;
        (c.pc, c.p, c.a, c.x, c.y, c.sp) = (i.pc, i.p, i.a, i.x, i.y, i.sp);
        c.locked = false;
        self.poll_i = i.p & I != 0;
        self.in_play = false;
        self.next_call = if self.flags & nsf2::NO_PLAY != 0 {
            NEVER
        } else {
            self.scheduled(self.frame + 1).max(self.now)
        };
    }

    fn go_idle(&mut self) {
        self.cpu.locked = false;
        self.cpu.pc = IDLE_ADDRESS;
        self.poll_i = self.cpu.p & I != 0;
        self.phase = Phase::Idle;
    }

    /// The first init routine has returned (or was abandoned).
    fn init_returned(&mut self) {
        if self.init2 {
            self.init2 = false;
            self.go_idle();
            return;
        }
        self.in_init = false;
        if self.flags & nsf2::NO_PLAY == 0 {
            self.nmi_enabled = true;
            self.next_call = self.now.max(self.scheduled(self.frame + 1));
        } else {
            self.next_call = NEVER;
        }
        if self.flags & nsf2::NON_RETURNING_INIT != 0 {
            // The same call again, with Y marking it as the second, and no
            // limit on how long it runs. The play routine arrives as NMIs.
            self.cpu.a = self.song;
            self.cpu.x = u8::from(self.region == oscilla_nsf::Region::Pal);
            self.begin_routine(self.init);
            self.cpu.y = 0x81;
            self.init2 = true;
        } else {
            self.go_idle();
        }
    }

    /// The cycle of the next thing that can happen while the CPU waits in the
    /// player's loop: the next call of the play routine or an interrupt.
    fn idle_wake(&mut self) -> u64 {
        let mut wake = if self.nmi_enabled {
            self.next_call
        } else {
            NEVER
        };
        if self.poll_i {
            // The CPU cannot clear the flag from the loop.
            return wake;
        }
        if self.irq_line() {
            return self.now;
        }
        if self.irq_timer.active {
            wake = wake.min(self.irq_timer.next);
        }
        let a = &self.apu;
        if !a.frame.inhibit && !a.frame.five_step {
            wake = wake.min(a.frame.next);
        }
        if a.dmc.irq_enable && a.dmc.remaining > 0 {
            wake = wake.min(a.dmc.next_step);
        }
        wake
    }

    /// One instruction, then the flag as the interrupt poll sees it: CLI, SEI
    /// and PLP take effect one instruction late.
    fn step_polled(&mut self) {
        let was_set = self.cpu.p & I != 0;
        let opcode = self.peek(self.cpu.pc);
        self.cpu_step();
        self.poll_i = if matches!(opcode, 0x58 | 0x78 | 0x28) {
            was_set
        } else {
            self.cpu.p & I != 0
        };
    }

    /// `run_until` for a file with interrupts.
    pub(crate) fn run_until_irq(&mut self, target: u64) {
        loop {
            match self.phase {
                Phase::Routine => {
                    if self.now >= target {
                        break;
                    }
                    match self.cpu.pc {
                        RETURN_ADDRESS => {
                            self.init_returned();
                            continue;
                        }
                        PLAY_RETURN if self.in_play => {
                            self.leave_play();
                            continue;
                        }
                        IDLE_ADDRESS => {
                            self.go_idle();
                            continue;
                        }
                        _ => {}
                    }
                    let first_init = self.in_init && !self.init2;
                    let overrun = (first_init && self.now - self.call_start > limit(self.clock))
                        || (self.in_play && self.now - self.play_start > limit(self.clock));
                    if self.cpu.locked || overrun {
                        if !self.cpu.locked {
                            self.warn(WARN_STALL, "A routine did not return within a second.");
                        }
                        self.stalled = true;
                        if self.in_play {
                            self.leave_play();
                        } else if first_init {
                            self.init_returned();
                        } else {
                            self.go_idle();
                        }
                        continue;
                    }
                    if self.nmi_enabled && !self.in_play && self.now >= self.next_call {
                        self.enter_play();
                        continue;
                    }
                    if !self.poll_i && self.irq_line() {
                        self.enter_irq();
                        continue;
                    }
                    self.step_polled();
                }
                Phase::Idle => {
                    let wake = self.idle_wake();
                    if wake > target {
                        self.now = self.now.max(target);
                        break;
                    }
                    self.now = self.now.max(wake);
                    if self.nmi_enabled && self.now >= self.next_call {
                        self.enter_play();
                    } else if !self.poll_i && self.irq_line() {
                        self.enter_irq();
                    }
                }
            }
        }
    }
}
