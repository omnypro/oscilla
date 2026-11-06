//! The recent output of each voice of a chip, for a display.

/// The samples that a scope keeps for each voice: 8,192 of the chip's own rate,
/// so that 4,096 frames of an output rate of 44,100 Hz or more are all there.
pub const SCOPE_LEN: usize = 8192;

/// A ring of 16-bit samples for each voice of one chip. It is not part of a
/// copy of the chip, which starts with an empty one.
#[derive(Debug)]
pub(crate) struct Scope {
    data: Box<[i16]>,
    voices: usize,
    /// The index that the next sample goes to.
    at: usize,
}

impl Scope {
    pub(crate) fn new(voices: usize) -> Scope {
        Scope {
            data: vec![0; voices * SCOPE_LEN].into_boxed_slice(),
            voices,
            at: 0,
        }
    }

    /// Adds one sample for each voice.
    pub(crate) fn push(&mut self, samples: &[i16]) {
        for (v, s) in samples.iter().enumerate().take(self.voices) {
            self.data[v * SCOPE_LEN + self.at] = *s;
        }
        self.at = (self.at + 1) % SCOPE_LEN;
    }

    /// Adds `n` samples that repeat the same values.
    pub(crate) fn push_n(&mut self, samples: &[i16], n: u64) {
        for _ in 0..n.min(SCOPE_LEN as u64) {
            self.push(samples);
        }
    }

    /// The sample `back` steps before the newest of a voice, 0 for the newest.
    pub(crate) fn get(&self, voice: usize, back: usize) -> i16 {
        if voice >= self.voices || back >= SCOPE_LEN {
            return 0;
        }
        self.data[voice * SCOPE_LEN + (self.at + SCOPE_LEN - 1 - back) % SCOPE_LEN]
    }
}

impl Clone for Scope {
    fn clone(&self) -> Scope {
        Scope::new(self.voices)
    }
}
