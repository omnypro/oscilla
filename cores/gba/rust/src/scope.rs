//! The recent output of each channel of the sound unit, for a display.

use crate::state::CLOCK_HZ;

/// The samples that the scope keeps of each channel: 4,096 at 44,100 Hz,
/// which is 93 ms.
pub const SCOPE_LEN: usize = 4096;

/// The rate of the scope's samples.
pub const SCOPE_HZ: u64 = 44_100;

/// The channels that the scope keeps, in the order of the channel mask.
pub(crate) const VOICES: usize = 6;

/// A ring of 16-bit samples for each channel. It is not part of a copy of the
/// machine, which starts with an empty one.
pub(crate) struct Scope {
    data: Box<[i16]>,
    /// The index that the next sample goes to.
    at: usize,
    /// The cycle that the scope has sampled up to.
    time: u64,
    /// Cycles times 44,100 that have not made a whole sample yet.
    acc: u64,
}

impl Scope {
    pub(crate) fn new() -> Scope {
        Scope {
            data: vec![0; VOICES * SCOPE_LEN].into_boxed_slice(),
            at: 0,
            time: 0,
            acc: 0,
        }
    }

    /// Empties the ring and starts again at cycle 0, without a new allocation.
    pub(crate) fn clear(&mut self) {
        self.data.fill(0);
        self.at = 0;
        self.time = 0;
        self.acc = 0;
    }

    /// The number of samples that fall due between the last call and cycle
    /// `now`. The count follows the cycles, so a change of the native sample
    /// rate does not change the scope's rate.
    pub(crate) fn due(&mut self, now: u64) -> u64 {
        // A gap of more than a second fills the ring anyway, and the cap keeps
        // the product in range.
        let elapsed = now.saturating_sub(self.time).min(u64::from(CLOCK_HZ));
        self.time = now;
        self.acc += elapsed * SCOPE_HZ;
        let n = self.acc / u64::from(CLOCK_HZ);
        self.acc %= u64::from(CLOCK_HZ);
        n
    }

    /// Adds `n` samples that repeat the same values.
    pub(crate) fn push(&mut self, samples: &[i16; VOICES], n: u64) {
        for _ in 0..n.min(SCOPE_LEN as u64) {
            for (v, s) in samples.iter().enumerate() {
                self.data[v * SCOPE_LEN + self.at] = *s;
            }
            self.at = (self.at + 1) & (SCOPE_LEN - 1);
        }
    }

    /// The sample `back` steps before the newest of a channel, 0 for the
    /// newest, or 0 past the ring's reach.
    pub(crate) fn get(&self, voice: usize, back: usize) -> i16 {
        if voice >= VOICES || back >= SCOPE_LEN {
            return 0;
        }
        self.data[voice * SCOPE_LEN + ((self.at + SCOPE_LEN - 1 - back) & (SCOPE_LEN - 1))]
    }

    /// The bytes that the ring holds outside the machine.
    pub(crate) fn heap_bytes(&self) -> usize {
        self.data.len() * std::mem::size_of::<i16>()
    }
}

impl Clone for Scope {
    /// An empty ring that samples on from the same cycle.
    fn clone(&self) -> Scope {
        Scope {
            time: self.time,
            acc: self.acc,
            ..Scope::new()
        }
    }
}
