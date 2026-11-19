use std::error::Error;
use std::fmt;
use std::num::NonZeroU64;

use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

/// All nondeterministic choices in a run share this stream. A one-way
/// decision consumes no entry. Entries are range-reduced integer tickets.
pub struct Choices<'a> {
    source: Source<'a>,
    observed: Vec<u64>,
}

enum Source<'a> {
    Seeded(Box<ChaCha8Rng>),
    Replay { tape: &'a [u64], edit: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TapeError {
    Exhausted {
        index: usize,
        bound: u64,
    },
    OutOfRange {
        index: usize,
        ticket: u64,
        bound: u64,
    },
}

impl fmt::Display for TapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted { index, bound } => {
                write!(
                    f,
                    "choice tape ends at entry {index} (expected a ticket below {bound})"
                )
            }
            Self::OutOfRange {
                index,
                ticket,
                bound,
            } => {
                write!(
                    f,
                    "choice tape entry {index} is {ticket} (expected below {bound})"
                )
            }
        }
    }
}

impl Error for TapeError {}

impl Choices<'static> {
    pub fn seeded(seed: u64) -> Self {
        Self {
            source: Source::Seeded(Box::new(ChaCha8Rng::seed_from_u64(seed))),
            observed: Vec::new(),
        }
    }
}

impl<'a> Choices<'a> {
    pub fn replay(tape: &'a [u64]) -> Self {
        Self {
            source: Source::Replay { tape, edit: false },
            observed: Vec::new(),
        }
    }

    /// Reinterpret edited entries modulo their new bounds and supply zero for
    /// missing entries. Persist the resulting observed tape for strict replay.
    pub fn edited(tape: &'a [u64]) -> Self {
        Self {
            source: Source::Replay { tape, edit: true },
            observed: Vec::new(),
        }
    }

    pub fn draw(&mut self, bound: NonZeroU64) -> Result<u64, TapeError> {
        let bound = bound.get();
        if bound == 1 {
            return Ok(0);
        }
        let index = self.observed.len();
        let ticket = match &mut self.source {
            Source::Seeded(rng) => {
                // Reject the short interval so each remainder is equally likely.
                let short = bound.wrapping_neg() % bound;
                loop {
                    let raw = rng.next_u64();
                    if raw >= short {
                        break raw % bound;
                    }
                }
            }
            Source::Replay { tape, edit: true } => tape.get(index).copied().unwrap_or(0) % bound,
            Source::Replay { tape, edit: false } => {
                let ticket = tape
                    .get(index)
                    .copied()
                    .ok_or(TapeError::Exhausted { index, bound })?;
                if ticket >= bound {
                    return Err(TapeError::OutOfRange {
                        index,
                        ticket,
                        bound,
                    });
                }
                ticket
            }
        };
        self.observed.push(ticket);
        Ok(ticket)
    }

    pub fn observed(&self) -> &[u64] {
        &self.observed
    }

    pub fn into_tape(self) -> Vec<u64> {
        self.observed
    }
}
