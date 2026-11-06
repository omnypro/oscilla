//! The recent output of each voice, for a display.

/// The samples that the scope keeps for each voice: 4,096 at 44,100 Hz, which
/// is 93 ms.
pub const SCOPE_LEN: usize = 4096;

/// The rate of the scope's samples.
pub(crate) const SCOPE_HZ: u64 = 44_100;

/// The most voices that a file can have: the 2A03's five and the channels of
/// every expansion chip, one for each bit of the channel mask.
pub(crate) const MAX_VOICES: usize = 34;

/// A ring of 16-bit samples for each voice. A sample is a channel's state at
/// one moment of a clock of 44,100 Hz. The channels change only at the sound
/// unit's events, so the values that the voices hold are worked out again only
/// after an event that can change them, and each sample in between repeats
/// them. A copy keeps the clock and starts with an empty ring.
#[derive(Debug)]
pub(crate) struct Scope {
    /// A ring of `SCOPE_LEN` samples for each voice, one ring after another.
    data: Box<[i16]>,
    voices: usize,
    /// The moment that the next sample goes to.
    at: usize,
    /// The cycle of the next sample, and the part of a cycle past it in units
    /// of 1/`den`.
    next: u64,
    rem: u64,
    /// The cycles from one sample to the next: `step` and `step_rem` / `den`.
    step: u64,
    step_rem: u64,
    den: u64,
    /// The value that each voice puts out now.
    held: [i16; MAX_VOICES],
    /// A change to the sound unit may have made `held` out of date.
    pub(crate) stale: bool,
}

impl Scope {
    /// An empty ring for `voices` voices of a machine whose CPU runs
    /// `clock.0 / clock.1` cycles a second.
    pub(crate) fn new(voices: usize, clock: (u64, u64)) -> Scope {
        let voices = voices.min(MAX_VOICES);
        let den = clock.1.max(1) * SCOPE_HZ;
        Scope {
            data: vec![0; voices * SCOPE_LEN].into_boxed_slice(),
            voices,
            at: 0,
            next: 0,
            rem: 0,
            step: clock.0 / den,
            step_rem: clock.0 % den,
            den,
            held: [0; MAX_VOICES],
            stale: true,
        }
    }

    pub(crate) fn voices(&self) -> usize {
        self.voices
    }

    /// The cycle of the next sample.
    #[inline]
    pub(crate) fn next(&self) -> u64 {
        self.next
    }

    /// Starts the clock again at cycle 0, for a machine whose cycle count
    /// starts again. The ring keeps what it holds.
    pub(crate) fn restart(&mut self) {
        self.next = 0;
        self.rem = 0;
        self.stale = true;
    }

    /// The values that the voices hold from now on, for the caller to set.
    pub(crate) fn hold(&mut self) -> &mut [i16; MAX_VOICES] {
        self.stale = false;
        &mut self.held
    }

    /// Adds a sample of the held values for each moment of the clock before
    /// cycle `until`.
    pub(crate) fn run(&mut self, until: u64) {
        let mut n = 0;
        while self.next < until && n < SCOPE_LEN {
            n += 1;
            self.next += self.step;
            self.rem += self.step_rem;
            if self.rem >= self.den {
                self.rem -= self.den;
                self.next += 1;
            }
        }
        // The whole ring holds the same values, so the clock moves on to the
        // first moment at or after `until` without counting each one.
        if self.next < until {
            let per = u128::from(self.step) * u128::from(self.den) + u128::from(self.step_rem);
            let gap = u128::from(until - self.next) * u128::from(self.den) - u128::from(self.rem);
            let total = u128::from(self.rem) + gap.div_ceil(per.max(1)) * per;
            let den = u128::from(self.den);
            self.next = self
                .next
                .saturating_add(u64::try_from(total / den).unwrap_or(u64::MAX));
            self.rem = (total % den) as u64;
        }
        let at = self.at;
        for (ring, &value) in self.data.chunks_exact_mut(SCOPE_LEN).zip(&self.held) {
            for k in 0..n {
                ring[(at + k) % SCOPE_LEN] = value;
            }
        }
        self.at = (at + n) % SCOPE_LEN;
    }

    /// The sample `back` steps before the newest of a voice, 0 for the newest.
    pub(crate) fn get(&self, voice: usize, back: usize) -> i16 {
        if voice >= self.voices || back >= SCOPE_LEN {
            return 0;
        }
        let moment = (self.at + SCOPE_LEN - 1 - back) % SCOPE_LEN;
        self.data[voice * SCOPE_LEN + moment]
    }

    /// The bytes that the ring takes.
    pub(crate) fn heap_bytes(&self) -> usize {
        self.data.len() * std::mem::size_of::<i16>()
    }
}

impl Clone for Scope {
    fn clone(&self) -> Scope {
        Scope {
            data: vec![0; self.data.len()].into_boxed_slice(),
            at: 0,
            held: [0; MAX_VOICES],
            stale: true,
            ..*self
        }
    }
}
