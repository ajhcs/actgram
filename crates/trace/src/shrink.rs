use std::error::Error;
use std::fmt;

use actgram_engine::Choices;
use actgram_exec::{Execution, FaultKind};

use crate::{ReplayError, Trace};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShrinkOptions {
    pub max_trials: usize,
}

impl Default for ShrinkOptions {
    fn default() -> Self {
        Self { max_trials: 10_000 }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShrinkResult {
    pub trace: Trace,
    pub trials: usize,
    /// True after a full pass finds no improving tested edit.
    pub locally_minimal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShrinkError {
    Replay(ReplayError),
    NoReproducibleFailure,
}

impl fmt::Display for ShrinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Replay(error) => error.fmt(f),
            Self::NoReproducibleFailure => write!(
                f,
                "shrinking requires an assertion, panic, or virtual wait hang"
            ),
        }
    }
}

impl Error for ShrinkError {}

/// Reduce a failing tape by coarse-to-fine contiguous deletion, then lowering
/// each ticket to zero, half, or its predecessor. A candidate must reproduce
/// the same failure and improve (started actions, tape length, ticket order).
/// Completion establishes local minimality for these edits.
pub fn shrink(
    trace: &Trace,
    options: ShrinkOptions,
    mut execute: impl FnMut(&mut Choices<'_>) -> Execution,
) -> Result<ShrinkResult, ShrinkError> {
    let failure = Signature::of(&trace.execution).ok_or(ShrinkError::NoReproducibleFailure)?;
    trace.replay(&mut execute).map_err(ShrinkError::Replay)?;
    let mut reducer = Reducer {
        best: trace.clone(),
        failure,
        remaining: options.max_trials,
        execute,
    };
    let locally_minimal = loop {
        match reducer.reduce_once() {
            Progress::Improved => {}
            Progress::Converged => break true,
            Progress::Budget => break false,
        }
    };
    Ok(ShrinkResult {
        trace: reducer.best,
        trials: options.max_trials - reducer.remaining,
        locally_minimal,
    })
}

#[derive(Eq, PartialEq)]
struct Signature<'a> {
    automaton: Option<usize>,
    action: Option<&'a str>,
    kind: FailureKind<'a>,
}

#[derive(Eq, PartialEq)]
enum FailureKind<'a> {
    Assertion(&'a str),
    Panic(&'a str),
    Hang,
}

impl<'a> Signature<'a> {
    fn of(execution: &'a Execution) -> Option<Self> {
        let fault = execution.fault.as_ref()?;
        let kind = match &fault.kind {
            FaultKind::Assertion { code, .. } => FailureKind::Assertion(code),
            FaultKind::Panic { message } => FailureKind::Panic(message),
            FaultKind::Hang { .. } => FailureKind::Hang,
            _ => return None,
        };
        Some(Self {
            automaton: fault.automaton,
            action: fault.action.as_deref(),
            kind,
        })
    }
}

enum Progress {
    Improved,
    Converged,
    Budget,
}

struct Reducer<'a, F> {
    best: Trace,
    failure: Signature<'a>,
    remaining: usize,
    execute: F,
}

impl<F: FnMut(&mut Choices<'_>) -> Execution> Reducer<'_, F> {
    fn reduce_once(&mut self) -> Progress {
        let length = self.best.tape.len();
        let mut chunk = length;
        while chunk > 0 {
            for start in (0..length).step_by(chunk) {
                let end = start.saturating_add(chunk).min(length);
                let mut edited = Vec::with_capacity(length - (end - start));
                edited.extend_from_slice(&self.best.tape[..start]);
                edited.extend_from_slice(&self.best.tape[end..]);
                match self.try_edit(&edited) {
                    Progress::Converged => {}
                    other => return other,
                }
            }
            chunk = if chunk == 1 { 0 } else { chunk.div_ceil(2) };
        }

        let mut edited = self.best.tape.clone();
        for index in 0..length {
            let old = edited[index];
            if old == 0 {
                continue;
            }
            let mut previous = None;
            for ticket in [0, old / 2, old - 1] {
                if previous == Some(ticket) {
                    continue;
                }
                previous = Some(ticket);
                edited[index] = ticket;
                match self.try_edit(&edited) {
                    Progress::Converged => {}
                    other => return other,
                }
            }
            edited[index] = old;
        }
        Progress::Converged
    }

    fn try_edit(&mut self, edited: &[u64]) -> Progress {
        if self.remaining == 0 {
            return Progress::Budget;
        }
        self.remaining -= 1;
        let candidate = Trace::capture(self.best.seed, Choices::edited(edited), &mut self.execute);
        let score = |trace: &Trace| (trace.action_starts().count(), trace.tape.len());
        if Signature::of(&candidate.execution).as_ref() == Some(&self.failure)
            && (score(&candidate), &candidate.tape) < (score(&self.best), &self.best.tape)
        {
            self.best = candidate;
            return Progress::Improved;
        }
        Progress::Converged
    }
}
