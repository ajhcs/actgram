//! Two-stack traversal of validated action grammars.

mod tape;

pub use tape::{Choices, TapeError};

use std::error::Error;
use std::fmt;
use std::num::NonZeroU64;

use actgram_core::{ActionId, Grammar, Node, NodeId, Rank};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Goal {
    #[default]
    Random,
    Next(ActionId),
    Eventually(ActionId),
}

pub use Goal::{Eventually, Next, Random};

impl Goal {
    pub fn target(self) -> Option<ActionId> {
        match self {
            Random => None,
            Next(target) | Eventually(target) => Some(target),
        }
    }
}

/// Policies may use the shared choice stream to break ties. The engine
/// offers only decisions consistent with the policy's current goal.
pub trait Strategy {
    fn goal(&self) -> Goal {
        Random
    }

    fn choose(&mut self, weights: &[u32], choices: &mut Choices<'_>) -> Result<usize, EngineError> {
        let total = weights
            .iter()
            .try_fold(0u64, |sum, &weight| sum.checked_add(u64::from(weight)));
        let total = total
            .and_then(NonZeroU64::new)
            .ok_or(EngineError::InvalidWeights)?;
        let mut ticket = choices.draw(total)?;
        for (index, &weight) in weights.iter().enumerate() {
            let weight = u64::from(weight);
            if ticket < weight {
                return Ok(index);
            }
            ticket -= weight;
        }
        Err(EngineError::InvalidWeights)
    }

    /// Return true to remove this policy after the emitted action.
    fn on_action(&mut self, action: ActionId) -> bool {
        self.goal().target() == Some(action)
    }
}

impl Strategy for Goal {
    fn goal(&self) -> Goal {
        *self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EngineError {
    Unreachable(Goal),
    ExpansionLimit(u64),
    InvalidWeights,
    InvalidChoice { chosen: usize, available: usize },
    Tape(TapeError),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(goal) => {
                write!(f, "{goal:?} is unreachable from the remaining grammar")
            }
            Self::ExpansionLimit(limit) => write!(
                f,
                "used {limit} expansions without emitting an action or ending"
            ),
            Self::InvalidWeights => {
                write!(f, "choices must have a nonzero weight sum that fits in u64")
            }
            Self::InvalidChoice { chosen, available } => {
                write!(f, "strategy chose index {chosen} from {available} options")
            }
            Self::Tape(error) => error.fmt(f),
        }
    }
}

impl Error for EngineError {}

impl From<TapeError> for EngineError {
    fn from(error: TapeError) -> Self {
        Self::Tape(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Alternative {
    pub node: NodeId,
    /// Index in the original list of arms, including disabled arms.
    pub arm: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Advance {
    pub alternatives: Vec<Alternative>,
    pub action: Option<ActionId>,
}

#[derive(Clone, Copy)]
enum Frame {
    Node(NodeId),
    Repeat(Repeat),
}

#[derive(Clone, Copy)]
struct Repeat {
    body: NodeId,
    min: u32,
    max: Option<u32>,
}

impl Repeat {
    fn remaining(self) -> Self {
        Self {
            min: self.min.saturating_sub(1),
            max: self.max.map(|max| max - 1),
            ..self
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Plan {
    complete: Rank,
    target: Option<Rank>,
}

impl Plan {
    fn then(self, suffix: Self) -> Self {
        let after = suffix.target.map(|target| self.complete + target);
        Self {
            complete: self.complete + suffix.complete,
            target: match (self.target, after) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        }
    }
}

/// The object stack holds unexpanded grammar frames. The top of the strategy
/// stack controls choices until it is popped or its target is emitted.
pub struct Engine<'g> {
    grammar: &'g Grammar,
    objects: Vec<Frame>,
    strategies: Vec<Box<dyn Strategy>>,
    expansion_limit: u64,
}

impl<'g> Engine<'g> {
    pub const DEFAULT_EXPANSION_LIMIT: u64 = 100_000;

    pub fn new(grammar: &'g Grammar) -> Self {
        Self {
            grammar,
            objects: vec![Frame::Node(grammar.root())],
            strategies: vec![Box::new(Random)],
            expansion_limit: Self::DEFAULT_EXPANSION_LIMIT,
        }
    }

    /// Bound expansions per call, including paths through nullable recursion.
    pub fn set_expansion_limit(&mut self, limit: u64) {
        self.expansion_limit = limit;
    }

    pub fn push_strategy(&mut self, strategy: impl Strategy + 'static) {
        self.strategies.push(Box::new(strategy));
    }

    /// The bottom random policy stays on the stack.
    pub fn pop_strategy(&mut self) -> bool {
        if self.strategies.len() == 1 {
            return false;
        }
        self.strategies.pop();
        true
    }

    pub fn strategy(&self) -> Goal {
        self.strategies.last().expect("base strategy").goal()
    }

    pub fn is_exhausted(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn shortest_distance(&self, target: ActionId) -> Option<u64> {
        self.pending_plan(target).target.map(|rank| rank.actions)
    }

    /// Emit one action or finish the derivation. An unreachable goal leaves
    /// the object stack intact. Limit or tape errors may consume some frames.
    pub fn advance(&mut self, choices: &mut Choices<'_>) -> Result<Advance, EngineError> {
        let goal = self.strategy();
        let target = goal.target();
        if let Some(target) = target {
            let distance = self.shortest_distance(target);
            if distance.is_none() || (matches!(goal, Next(_)) && distance != Some(0)) {
                return Err(EngineError::Unreachable(goal));
            }
        }

        let mut alternatives = Vec::new();
        let mut expanded = 0;
        while let Some(&frame) = self.objects.last() {
            if expanded == self.expansion_limit {
                return Err(EngineError::ExpansionLimit(self.expansion_limit));
            }
            self.objects.pop();
            expanded += 1;
            let node = match frame {
                Frame::Node(node) => node,
                Frame::Repeat(repeat) => {
                    self.expand_repeat(repeat, goal, choices)?;
                    continue;
                }
            };
            match &self.grammar[node] {
                Node::Action(action) => {
                    if self
                        .strategies
                        .last_mut()
                        .expect("base strategy")
                        .on_action(*action)
                    {
                        self.pop_strategy();
                    }
                    return Ok(Advance {
                        alternatives,
                        action: Some(*action),
                    });
                }
                Node::Symbol(name) => {
                    let definition = self.grammar.rule(name).expect("validated symbol");
                    self.objects.push(Frame::Node(definition));
                }
                Node::Seq(children) => {
                    self.objects
                        .extend(children.iter().rev().copied().map(Frame::Node));
                }
                Node::Alt(arms) => {
                    let tail = target.map(|target| self.pending_plan(target));
                    let options: Vec<_> = arms
                        .iter()
                        .enumerate()
                        .filter(|(_, arm)| arm.weight != 0)
                        .map(|(index, arm)| {
                            let rank = target.zip(tail).and_then(|(target, tail)| {
                                self.node_plan(arm.node, target).then(tail).target
                            });
                            (index, arm.weight, rank)
                        })
                        .collect();
                    let arm = self.select(goal, &options, choices)?;
                    alternatives.push(Alternative { node, arm });
                    self.objects.push(Frame::Node(arms[arm].node));
                }
                Node::Repeat { body, min, max } => {
                    self.expand_repeat(
                        Repeat {
                            body: *body,
                            min: *min,
                            max: *max,
                        },
                        goal,
                        choices,
                    )?;
                }
            }
        }
        Ok(Advance {
            alternatives,
            action: None,
        })
    }

    fn expand_repeat(
        &mut self,
        repeat: Repeat,
        goal: Goal,
        choices: &mut Choices<'_>,
    ) -> Result<(), EngineError> {
        if repeat.max == Some(0) {
            return Ok(());
        }
        let remaining = repeat.remaining();
        let go = if repeat.min > 0 {
            true
        } else {
            let (mut stop, mut go) = (None, None);
            if let Some(target) = goal.target() {
                let tail = self.pending_plan(target);
                stop = tail.target;
                go = self
                    .node_plan(repeat.body, target)
                    .then(self.repeat_plan(remaining, target))
                    .then(tail)
                    .target;
            }
            self.select(goal, &[(0, 1, stop), (1, 1, go)], choices)? == 1
        };
        if go {
            self.objects.push(Frame::Repeat(remaining));
            self.objects.push(Frame::Node(repeat.body));
        }
        Ok(())
    }

    fn select(
        &mut self,
        goal: Goal,
        options: &[(usize, u32, Option<Rank>)],
        choices: &mut Choices<'_>,
    ) -> Result<usize, EngineError> {
        let eligible: Vec<_> = if goal.target().is_some() {
            let best = options
                .iter()
                .filter_map(|option| option.2)
                .min()
                .ok_or(EngineError::Unreachable(goal))?;
            options
                .iter()
                .filter(|option| option.2 == Some(best))
                .collect()
        } else {
            options.iter().collect()
        };
        if let [option] = eligible.as_slice() {
            return Ok(option.0);
        }
        let weights: Vec<_> = eligible.iter().map(|option| option.1).collect();
        let chosen = self
            .strategies
            .last_mut()
            .expect("base strategy")
            .choose(&weights, choices)?;
        eligible
            .get(chosen)
            .map(|option| option.0)
            .ok_or(EngineError::InvalidChoice {
                chosen,
                available: eligible.len(),
            })
    }

    fn pending_plan(&self, target: ActionId) -> Plan {
        self.objects.iter().fold(Plan::default(), |suffix, frame| {
            let plan = match *frame {
                Frame::Node(node) => self.node_plan(node, target),
                Frame::Repeat(repeat) => self.repeat_plan(repeat, target),
            };
            plan.then(suffix)
        })
    }

    fn node_plan(&self, node: NodeId, target: ActionId) -> Plan {
        let summary = &self.grammar.analysis()[node];
        Plan {
            complete: summary.completion_rank().expect("productive node"),
            target: summary.target_rank(target),
        }
    }

    fn repeat_plan(&self, repeat: Repeat, target: ActionId) -> Plan {
        let body = self.node_plan(repeat.body, target);
        Plan {
            complete: body.complete.repeat(repeat.min),
            target: if repeat.max == Some(0) {
                None
            } else {
                body.target.map(|rank| Rank::STEP + rank)
            },
        }
    }
}
