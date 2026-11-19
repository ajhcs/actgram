//! Bounded queue and in-memory write-ahead-log systems with reference checks.

mod corpus;
mod queue;
mod wal;

pub use corpus::{workloads, Bug, Workload, BLOCKS, BUGS};

use actgram_core::{Builder, Grammar};
use actgram_engine::{Choices, Goal};
use actgram_exec::{run, ActionError, ActionResult, Actions, Automaton, Execution, Limits, System};
use queue::QueueTest;
use wal::WalTest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Domain {
    Queue,
    Wal,
    Long,
}

impl Domain {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "queue" => Some(Self::Queue),
            "wal" => Some(Self::Wal),
            "long" => Some(Self::Long),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Wal => "wal",
            Self::Long => "long",
        }
    }
}

macro_rules! operations {
    ($($operation:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum Op { $($operation),+ }

        impl Op {
            pub fn name(self) -> &'static str {
                match self { $(Self::$operation => $name),+ }
            }
        }

        fn actions() -> Actions<ExampleSystem> {
            let mut actions = Actions::new();
            $(actions.insert($name, |ctx| Box::pin(async move {
                ctx.with_system_mut(|system: &mut ExampleSystem| system.apply(Op::$operation))
            }));)+
            actions
        }
    }
}

operations! {
    Reset => "reset",
    PushA => "push_a",
    PushB => "push_b",
    PushC => "push_c",
    PushD => "push_d",
    Pop => "pop",
    Peek => "peek",
    Shrink => "shrink_capacity",
    Grow => "grow_capacity",
    Clear => "clear",
    Snapshot => "snapshot",
    PutA => "put_a",
    ReplaceA => "replace_a",
    PutB => "put_b",
    Delete => "delete_a",
    Flush => "flush",
    Crash => "crash",
    Recover => "recover",
    Checkpoint => "checkpoint",
    ReadA => "read_a",
    ReadB => "read_b",
    Idle => "idle",
    Arm => "arm",
    Check => "check",
}

enum State {
    Queue(QueueTest),
    Wal(WalTest),
    Long { armed: bool },
}

/// The same system operations and observations serve both test generators.
pub struct ExampleSystem {
    state: State,
    bug: Option<Bug>,
}

impl System for ExampleSystem {}

impl ExampleSystem {
    pub fn new(domain: Domain, bug: Option<Bug>) -> Result<Self, String> {
        if let Some(bug) = bug {
            if bug.domain() != domain {
                return Err(format!(
                    "{} belongs to the {} system",
                    bug.name(),
                    bug.domain().name()
                ));
            }
            if !bug.enabled() {
                return Err(format!("{} requires its seeded-bug feature", bug.name()));
            }
        }
        let state = match domain {
            Domain::Queue => State::Queue(QueueTest::new()),
            Domain::Wal => State::Wal(WalTest::default()),
            Domain::Long => State::Long { armed: false },
        };
        Ok(Self { state, bug })
    }

    pub fn apply(&mut self, op: Op) -> ActionResult {
        match &mut self.state {
            State::Queue(queue) => queue.apply(op, self.bug),
            State::Wal(wal) => wal.apply(op, self.bug),
            State::Long { armed } => match op {
                Op::Idle => Ok(()),
                Op::Arm => {
                    *armed = true;
                    Ok(())
                }
                Op::Check if *armed => Err(ActionError::failed("demo", "the system is armed")),
                Op::Check => Ok(()),
                _ => Err(wrong_operation(op)),
            },
        }
    }
}

fn wrong_operation(op: Op) -> ActionError {
    ActionError::failed("workload", format!("unexpected operation {}", op.name()))
}

fn compare<T: std::fmt::Debug + Eq>(actual: T, expected: T, bug: Option<Bug>) -> ActionResult {
    if actual == expected {
        return Ok(());
    }
    let code = bug.map_or("reference", Bug::name);
    Err(ActionError::failed(
        code,
        format!("expected {expected:?}, observed {actual:?}"),
    ))
}

/// A reusable compiled scenario. Each execution receives a fresh system.
pub struct Scenario {
    grammar: Grammar,
    actions: Actions<ExampleSystem>,
}

impl Scenario {
    pub fn new(domain: Domain) -> Self {
        let mut builder = Builder::new();
        let root = if domain == Domain::Long {
            long_grammar(&mut builder)
        } else {
            let arms: Vec<_> = workloads(domain)
                .iter()
                .map(|case| {
                    let commands: Vec<_> = case
                        .ops
                        .iter()
                        .map(|op| builder.action(op.name()))
                        .collect();
                    (case.weight, builder.seq(commands))
                })
                .collect();
            let block = builder.alt(arms);
            builder.repeat(block, BLOCKS, Some(BLOCKS))
        };
        let grammar = builder.build(root).expect("example grammar");
        Self {
            grammar,
            actions: actions(),
        }
    }

    pub fn grammar(&self) -> &Grammar {
        &self.grammar
    }

    pub fn execute(
        &self,
        system: ExampleSystem,
        goal: Goal,
        choices: &mut Choices<'_>,
    ) -> Execution {
        run(
            system,
            &[Automaton::new(&self.grammar, &self.actions).with_goal(goal)],
            Limits::default(),
            choices,
        )
    }
}

fn long_grammar(b: &mut Builder) -> actgram_core::NodeId {
    let empty = b.seq([]);
    let idle = b.action(Op::Idle.name());
    let mut prefix = empty;
    // Zero tickets take the empty arm. A fixed seed usually takes all 498 idles.
    for _ in 0..498 {
        let more = b.seq([idle, prefix]);
        prefix = b.alt([(1, empty), (u32::MAX, more)]);
    }
    let arm = b.action(Op::Arm.name());
    let check = b.action(Op::Check.name());
    b.seq([prefix, arm, check])
}
