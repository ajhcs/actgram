//! Serializable executions, strict replay, tape reduction, and coverage.

mod coverage;
mod shrink;

pub use coverage::Coverage;
pub use shrink::{shrink, ShrinkError, ShrinkOptions, ShrinkResult};

use std::error::Error;
use std::fmt;

use actgram_engine::Choices;
use actgram_exec::{EventKind, Execution, FaultKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TRACE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Trace {
    pub version: u32,
    /// The original seed. The tape is authoritative for an edited trace.
    pub seed: u64,
    pub tape: Vec<u64>,
    pub execution: Execution,
}

impl Trace {
    pub fn record(seed: u64, execute: impl FnOnce(&mut Choices<'_>) -> Execution) -> Self {
        Self::capture(seed, Choices::seeded(seed), execute)
    }

    fn capture(
        seed: u64,
        mut choices: Choices<'_>,
        execute: impl FnOnce(&mut Choices<'_>) -> Execution,
    ) -> Self {
        let execution = execute(&mut choices);
        Self {
            version: TRACE_VERSION,
            seed,
            tape: choices.into_tape(),
            execution,
        }
    }

    /// Replay with the original system initialization, automata, goals and limits.
    pub fn replay(
        &self,
        execute: impl FnOnce(&mut Choices<'_>) -> Execution,
    ) -> Result<(), ReplayError> {
        self.check_version()?;
        let mut choices = Choices::replay(&self.tape);
        let actual = execute(&mut choices);
        if let Some(FaultKind::ChoiceTape { detail }) =
            actual.fault.as_ref().map(|fault| &fault.kind)
        {
            return Err(ReplayError::InvalidTape(detail.clone()));
        }
        if choices.observed().len() != self.tape.len() {
            return Err(ReplayError::UnconsumedTape {
                consumed: choices.observed().len(),
                total: self.tape.len(),
            });
        }
        let expected = &self.execution;
        if actual.events != expected.events {
            let first = expected
                .events
                .iter()
                .zip(&actual.events)
                .take_while(|(a, b)| a == b)
                .count();
            return Err(ReplayError::DifferentEvent(first));
        }
        if actual != *expected {
            return Err(ReplayError::DifferentOutcome);
        }
        Ok(())
    }

    pub fn action_starts(&self) -> impl Iterator<Item = (usize, &str)> {
        self.execution
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::Started { action } => Some((event.automaton, action.as_str())),
                _ => None,
            })
    }

    /// SHA-256 of the format's compact JSON bytes.
    pub fn hash(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("trace format is JSON serializable");
        format!("{:x}", Sha256::digest(bytes))
    }

    pub fn check_version(&self) -> Result<(), ReplayError> {
        if self.version != TRACE_VERSION {
            return Err(ReplayError::UnsupportedVersion(self.version));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplayError {
    UnsupportedVersion(u32),
    InvalidTape(String),
    UnconsumedTape { consumed: usize, total: usize },
    DifferentEvent(usize),
    DifferentOutcome,
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => write!(f, "unsupported trace format {version}"),
            Self::InvalidTape(detail) => f.write_str(detail),
            Self::UnconsumedTape { consumed, total } => {
                write!(f, "replay consumed {consumed} of {total} tape entries")
            }
            Self::DifferentEvent(index) => write!(f, "replay diverged at event {index}"),
            Self::DifferentOutcome => {
                write!(f, "replay ended with a different outcome or virtual tick")
            }
        }
    }
}

impl Error for ReplayError {}
