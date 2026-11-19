use std::collections::BTreeMap;
use std::ops::{Add, Index};

use crate::{ActionId, Node, NodeId};

/// A distance measured first in actions and then in expansion steps.
/// The second component makes shortest-path traversal descend through
/// nullable recursion. Unrepresentable components saturate at u64::MAX.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Rank {
    pub actions: u64,
    pub expansions: u64,
}

impl Rank {
    pub const ZERO: Self = Self {
        actions: 0,
        expansions: 0,
    };
    pub const STEP: Self = Self {
        actions: 0,
        expansions: 1,
    };

    /// Complete the required copies, then expand the repeat's stop frame.
    pub fn repeat(self, min: u32) -> Self {
        let min = u64::from(min);
        Self::saturated(
            self.actions.saturating_mul(min),
            self.expansions.saturating_mul(min).saturating_add(min + 1),
        )
    }

    fn saturated(actions: u64, expansions: u64) -> Self {
        // Collapse action overflow to one value so concatenation preserves ordering.
        let expansions = if actions == u64::MAX {
            u64::MAX
        } else {
            expansions
        };
        Self {
            actions,
            expansions,
        }
    }
}

impl Add for Rank {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::saturated(
            self.actions.saturating_add(rhs.actions),
            self.expansions.saturating_add(rhs.expansions),
        )
    }
}

/// Static facts about finite derivations of one node.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    complete: Option<Rank>,
    targets: BTreeMap<ActionId, Rank>,
}

impl Summary {
    pub fn nullable(&self) -> bool {
        self.minimum_actions() == Some(0)
    }

    pub fn first(&self) -> impl Iterator<Item = ActionId> + '_ {
        self.targets
            .iter()
            .filter_map(|(&action, rank)| (rank.actions == 0).then_some(action))
    }

    pub fn reachable(&self) -> impl Iterator<Item = ActionId> + '_ {
        self.targets.keys().copied()
    }

    pub fn minimum_actions(&self) -> Option<u64> {
        self.completion_rank().map(|cost| cost.actions)
    }

    /// The minimum number of actions strictly before the target.
    pub fn shortest_distance(&self, target: ActionId) -> Option<u64> {
        self.target_rank(target).map(|cost| cost.actions)
    }

    pub fn completion_rank(&self) -> Option<Rank> {
        self.complete
    }

    pub fn target_rank(&self, target: ActionId) -> Option<Rank> {
        self.targets.get(&target).copied()
    }

    fn add_targets(&mut self, prefix: Rank, child: &Self) {
        for (&action, &cost) in &child.targets {
            let candidate = prefix + cost;
            self.targets
                .entry(action)
                .and_modify(|old| *old = (*old).min(candidate))
                .or_insert(candidate);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Analysis(Vec<Summary>);

impl Analysis {
    pub fn get(&self, node: NodeId) -> Option<&Summary> {
        self.0.get(node.index())
    }

    pub(crate) fn compute(nodes: &[Node], rules: &BTreeMap<String, NodeId>) -> Self {
        let mut analysis = Self(vec![Summary::default(); nodes.len()]);
        loop {
            let mut changed = false;
            for (index, node) in nodes.iter().enumerate() {
                let next = analysis.evaluate(node, rules);
                if analysis.0[index] != next {
                    analysis.0[index] = next;
                    changed = true;
                }
            }
            if !changed {
                return analysis;
            }
        }
    }

    fn evaluate(&self, node: &Node, rules: &BTreeMap<String, NodeId>) -> Summary {
        let mut result = Summary::default();
        match node {
            Node::Action(action) => {
                result.complete = Some(Rank {
                    actions: 1,
                    expansions: 1,
                });
                result.targets.insert(*action, Rank::STEP);
            }
            Node::Symbol(name) => {
                let child = &self[rules[name]];
                result.complete = child.complete.map(|cost| Rank::STEP + cost);
                result.add_targets(Rank::STEP, child);
            }
            Node::Seq(children) => {
                let mut prefix = Rank::STEP;
                for &id in children {
                    let child = &self[id];
                    let Some(complete) = child.complete else {
                        return Summary::default();
                    };
                    result.add_targets(prefix, child);
                    prefix = prefix + complete;
                }
                result.complete = Some(prefix);
            }
            Node::Alt(arms) => {
                for arm in arms.iter().filter(|arm| arm.weight > 0) {
                    let child = &self[arm.node];
                    if let Some(cost) = child.complete {
                        let candidate = Rank::STEP + cost;
                        result.complete =
                            Some(result.complete.map_or(candidate, |old| old.min(candidate)));
                        result.add_targets(Rank::STEP, child);
                    }
                }
            }
            Node::Repeat { body, min, max } => {
                let child = &self[*body];
                result.complete = if *min == 0 {
                    Some(Rank::STEP)
                } else {
                    child.complete.map(|cost| cost.repeat(*min))
                };
                if *max != Some(0) {
                    result.add_targets(Rank::STEP, child);
                }
            }
        }
        result
    }
}

impl Index<NodeId> for Analysis {
    type Output = Summary;

    fn index(&self, id: NodeId) -> &Summary {
        &self.0[id.index()]
    }
}
