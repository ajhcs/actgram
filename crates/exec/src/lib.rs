//! Single-threaded execution of async actions against a steppable system.

mod context;
mod runner;

pub use context::Ctx;
pub use runner::run;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use actgram_core::Grammar;
use actgram_engine::{Engine, Goal};
use serde::{Deserialize, Serialize};

/// The system under test, with one call per virtual-clock advance.
pub trait System {
    fn step(&mut self, _tick: u64) {}
}

impl System for () {}

pub type ActionResult = Result<(), ActionError>;
pub type ActionFuture<'a> = Pin<Box<dyn Future<Output = ActionResult> + 'a>>;
pub type ActionFn<S> = for<'a> fn(&'a mut Ctx<S>) -> ActionFuture<'a>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionError {
    Failure { code: String, message: String },
    WaitExpired { deadline: u64 },
}

impl ActionError {
    /// Use a stable code to identify the assertion across tape edits.
    pub fn failed(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Failure {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failure { code, message } => write!(f, "{code}: {message}"),
            Self::WaitExpired { deadline } => {
                write!(f, "condition expired at virtual tick {deadline}")
            }
        }
    }
}

impl Error for ActionError {}

pub struct Actions<S>(BTreeMap<String, ActionFn<S>>);

impl<S> Default for Actions<S> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<S> Actions<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an action, usually with a closure like |ctx| Box::pin(action(ctx)).
    pub fn insert(&mut self, name: impl Into<String>, action: ActionFn<S>) -> Option<ActionFn<S>> {
        self.0.insert(name.into(), action)
    }
}

/// An independent traversal sharing a system with the other automata.
pub struct Automaton<'a, S> {
    grammar: &'a Grammar,
    actions: &'a Actions<S>,
    goal: Goal,
}

impl<'a, S> Automaton<'a, S> {
    pub fn new(grammar: &'a Grammar, actions: &'a Actions<S>) -> Self {
        Self {
            grammar,
            actions,
            goal: Goal::Random,
        }
    }

    pub fn with_goal(mut self, goal: Goal) -> Self {
        self.goal = goal;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Largest tick at which an action can be polled.
    pub max_tick: u64,
    pub max_actions: u64,
    pub max_expansions_per_action: u64,
    pub default_wait_ticks: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tick: 20_000,
            max_actions: 10_000,
            max_expansions_per_action: Engine::DEFAULT_EXPANSION_LIMIT,
            default_wait_ticks: 100,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Event {
    pub tick: u64,
    pub automaton: usize,
    pub kind: EventKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    Alternative { node: usize, arm: usize },
    Started { action: String },
    Completed { action: String },
    Waiting { deadline: u64 },
    Woke,
    Cancelled { action: Option<String> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Fault {
    pub tick: u64,
    pub automaton: Option<usize>,
    pub action: Option<String>,
    pub kind: FaultKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FaultKind {
    Assertion { code: String, message: String },
    Panic { message: String },
    Hang { deadline: u64 },
    MissingAction,
    UnsupportedPending,
    Traversal { detail: String },
    ChoiceTape { detail: String },
    TickLimit { limit: u64 },
    ActionLimit { limit: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Execution {
    pub events: Vec<Event>,
    pub final_tick: u64,
    pub fault: Option<Fault>,
}
