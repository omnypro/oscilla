//! The MMC5 expansion chip's sound: two pulse channels, an 8-bit sample
//! register and the 8 by 8 multiplier. Written from public documentation.

use crate::apu::{DUTY, LENGTHS};
use crate::state::*;

/// Cycles between the clocks of the envelopes and length counters: the CPU
/// clock divided by 7424, about 241 Hz. The chip has no frame sequencer.
pub(crate) const CLOCK: u64 = 7424;

impl Mmc5Pulse {
    /// Whether the channel makes no sound whatever the sequencer does. A low
    /// period does not silence it, and there is no sweep to mute it.
    pub(crate) fn silent(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn output(&self) -> u8 {
        if self.silent() || DUTY[usize::from(self.duty)][usize::from(self.pos)] == 0 {
            0
        } else if self.constant {
            self.volume
        } else {
            self.env.decay
        }
    }

    pub(crate) fn period_cycles(&self) -> u64 {
        2 * (u64::from(self.period) + 1)
    }

    /// Brings a stale timer past `t`, with the sequencer where it would be. A
    /// channel that is sounding keeps its pending step.
    pub(crate) fn settle(&mut self, t: u64) {
        if self.next_step <= t && self.silent() {
            let p = self.period_cycles();
            let k = (t - self.next_step) / p + 1;
            self.pos = self.pos.wrapping_sub((k % 8) as u8) & 7;
            self.next_step += k * p;
        }
    }
}

impl Mmc5 {
    /// One clock of the envelopes and length counters.
    pub(crate) fn clock(&mut self) {
        for p in &mut self.pulse {
            p.env.clock(p.volume, p.halt);
            if !p.halt && p.len > 0 {
                p.len -= 1;
            }
        }
    }

    /// The status register at 0x5015: a bit for each pulse channel that still
    /// has length left.
    pub(crate) fn status(&self) -> u8 {
        u8::from(self.pulse[0].len > 0) | u8::from(self.pulse[1].len > 0) << 1
    }

    /// The product at 0x5205 and 0x5206.
    pub(crate) fn product(&self) -> u16 {
        u16::from(self.operands[0]) * u16::from(self.operands[1])
    }
}

impl Machine {
    /// A store to 0x5000 to 0x5015.
    pub(crate) fn mmc5_write(&mut self, address: u16, v: u8) {
        self.sync_apu();
        let t = self.now;
        self.apu.settle(t);
        let m = &mut self.apu.mmc5;
        match address {
            0x5000 | 0x5004 => {
                let p = &mut m.pulse[usize::from(address >> 2 & 1)];
                p.duty = v >> 6;
                p.halt = v & 0x20 != 0;
                p.constant = v & 0x10 != 0;
                p.volume = v & 15;
            }
            0x5002 | 0x5006 => {
                let p = &mut m.pulse[usize::from(address >> 2 & 1)];
                p.period = p.period & 0x700 | u16::from(v);
            }
            0x5003 | 0x5007 => {
                let i = usize::from(address >> 2 & 1);
                let load = m.enabled >> i & 1 != 0;
                let p = &mut m.pulse[i];
                p.period = p.period & 0xFF | u16::from(v & 7) << 8;
                if load {
                    p.len = LENGTHS[usize::from(v >> 3)];
                }
                p.pos = 0;
                p.env.start = true;
            }
            0x5010 => {
                m.pcm_read = v & 1 != 0;
                m.pcm_irq_enable = v & 0x80 != 0;
            }
            0x5011 => {
                // A store of zero leaves the register as it is and raises the
                // interrupt flag instead.
                if !m.pcm_read {
                    if v == 0 {
                        m.pcm_irq = m.pcm_irq_enable;
                    } else {
                        m.pcm = v;
                    }
                }
            }
            0x5015 => {
                m.enabled = v & 3;
                for (i, p) in m.pulse.iter_mut().enumerate() {
                    if v >> i & 1 == 0 {
                        p.len = 0;
                    }
                }
            }
            _ => {}
        }
        self.apu_emit(t);
    }

    /// A read of 0x5010 to 0x5015, 0x5205 or 0x5206; `None` where the chip puts
    /// nothing on the bus.
    pub(crate) fn mmc5_read(&mut self, address: u16) -> Option<u8> {
        self.sync_apu();
        let m = &mut self.apu.mmc5;
        match address {
            0x5010 => {
                let v = u8::from(m.pcm_irq) << 7;
                m.pcm_irq = false;
                Some(v)
            }
            0x5015 => Some(m.status()),
            0x5205 => Some(m.product() as u8),
            0x5206 => Some((m.product() >> 8) as u8),
            _ => None,
        }
    }

    /// The same as `mmc5_read` with no side effect.
    pub(crate) fn mmc5_peek(&self, address: u16) -> Option<u8> {
        let m = &self.apu.mmc5;
        match address {
            0x5010 => Some(u8::from(m.pcm_irq) << 7),
            0x5015 => Some(m.status()),
            0x5205 => Some(m.product() as u8),
            0x5206 => Some((m.product() >> 8) as u8),
            _ => None,
        }
    }

    /// A read of 0x8000 to 0xBFFF while the sample register is in read mode:
    /// the byte read becomes the sample, unless it is zero.
    pub(crate) fn mmc5_sample_read(&mut self, v: u8) {
        if v == 0 {
            let m = &mut self.apu.mmc5;
            m.pcm_irq = m.pcm_irq_enable;
        } else {
            self.sync_apu();
            self.apu.mmc5.pcm = v;
            let t = self.now;
            self.apu_emit(t);
        }
    }
}
