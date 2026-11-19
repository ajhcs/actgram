//! Arena-backed action grammars with validation and fixed-point analysis.

mod analysis;

pub use analysis::{Analysis, Rank, Summary};

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::ops::Index;

/// An arena index, local to a builder and its resulting grammar.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(usize);

impl NodeId {
    pub const fn from_index(index: usize) -> Self {
        Self(index)
    }

    pub const fn index(self) -> usize {
        self.0
    }
}

/// An interned action name, local to a grammar.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionId(usize);

impl ActionId {
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Arm {
    pub node: NodeId,
    /// A zero weight disables this arm for both analysis and traversal.
    pub weight: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Node {
    Action(ActionId),
    Symbol(String),
    Seq(Vec<NodeId>),
    Alt(Vec<Arm>),
    Repeat {
        body: NodeId,
        min: u32,
        max: Option<u32>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GrammarError {
    InvalidRoot(NodeId),
    InvalidChild { parent: NodeId, child: NodeId },
    InvalidRule { name: String, node: NodeId },
    DuplicateRule(String),
    UndefinedSymbol { node: NodeId, name: String },
    UnproductiveRule(String),
    UnproductiveNode(NodeId),
    EmptyAlternative(NodeId),
    BadRepeatBounds { node: NodeId, min: u32, max: u32 },
}

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoot(root) => write!(f, "invalid root node {}", root.index()),
            Self::InvalidChild { parent, child } => {
                write!(
                    f,
                    "node {} references invalid node {}",
                    parent.index(),
                    child.index()
                )
            }
            Self::InvalidRule { name, node } => {
                write!(f, "rule {name:?} references invalid node {}", node.index())
            }
            Self::DuplicateRule(name) => write!(f, "rule {name:?} is already defined"),
            Self::UndefinedSymbol { node, name } => {
                write!(
                    f,
                    "node {} references undefined rule {name:?}",
                    node.index()
                )
            }
            Self::UnproductiveRule(name) => write!(f, "rule {name:?} has no finite derivation"),
            Self::UnproductiveNode(node) => {
                write!(f, "node {} has no finite derivation", node.index())
            }
            Self::EmptyAlternative(node) => {
                write!(
                    f,
                    "alternative at node {} has no positive-weight arms",
                    node.index()
                )
            }
            Self::BadRepeatBounds { node, min, max } => {
                write!(
                    f,
                    "repeat at node {} has minimum {min} above maximum {max}",
                    node.index()
                )
            }
        }
    }
}

impl Error for GrammarError {}

/// Builds a grammar. Rule references can precede their definitions.
#[derive(Default)]
pub struct Builder {
    nodes: Vec<Node>,
    rules: BTreeMap<String, NodeId>,
    actions: Vec<String>,
    action_ids: BTreeMap<String, ActionId>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn action(&mut self, name: impl Into<String>) -> NodeId {
        let name = name.into();
        let id = *self.action_ids.entry(name.clone()).or_insert_with(|| {
            let id = ActionId(self.actions.len());
            self.actions.push(name);
            id
        });
        self.insert(Node::Action(id))
    }

    pub fn symbol(&mut self, name: impl Into<String>) -> NodeId {
        self.insert(Node::Symbol(name.into()))
    }

    /// An empty sequence represents epsilon.
    pub fn seq(&mut self, nodes: impl IntoIterator<Item = NodeId>) -> NodeId {
        self.insert(Node::Seq(nodes.into_iter().collect()))
    }

    pub fn alt(&mut self, arms: impl IntoIterator<Item = (u32, NodeId)>) -> NodeId {
        let arms = arms.into_iter().map(|(weight, node)| Arm { node, weight });
        self.insert(Node::Alt(arms.collect()))
    }

    /// Both bounds are inclusive. A missing maximum represents an unbounded repeat.
    pub fn repeat(&mut self, body: NodeId, min: u32, max: Option<u32>) -> NodeId {
        self.insert(Node::Repeat { body, min, max })
    }

    pub fn rule(&mut self, name: impl Into<String>, node: NodeId) -> Result<(), GrammarError> {
        let name = name.into();
        if self.rules.contains_key(&name) {
            return Err(GrammarError::DuplicateRule(name));
        }
        self.rules.insert(name, node);
        Ok(())
    }

    pub fn build(self, root: NodeId) -> Result<Grammar, GrammarError> {
        self.validate(root)?;
        let analysis = Analysis::compute(&self.nodes, &self.rules);
        for (name, &node) in &self.rules {
            if analysis[node].minimum_actions().is_none() {
                return Err(GrammarError::UnproductiveRule(name.clone()));
            }
        }
        for index in 0..self.nodes.len() {
            let node = NodeId(index);
            if analysis[node].minimum_actions().is_none() {
                return Err(GrammarError::UnproductiveNode(node));
            }
        }
        Ok(Grammar {
            root,
            nodes: self.nodes,
            rules: self.rules,
            actions: self.actions,
            action_ids: self.action_ids,
            analysis,
        })
    }

    fn insert(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(node);
        id
    }

    fn validate(&self, root: NodeId) -> Result<(), GrammarError> {
        let exists = |id: NodeId| id.index() < self.nodes.len();
        if !exists(root) {
            return Err(GrammarError::InvalidRoot(root));
        }
        for (name, &node) in &self.rules {
            if !exists(node) {
                return Err(GrammarError::InvalidRule {
                    name: name.clone(),
                    node,
                });
            }
        }
        for (index, node) in self.nodes.iter().enumerate() {
            let id = NodeId(index);
            let check_child = |child| {
                if exists(child) {
                    Ok(())
                } else {
                    Err(GrammarError::InvalidChild { parent: id, child })
                }
            };
            match node {
                Node::Action(_) => {}
                Node::Symbol(name) => {
                    if !self.rules.contains_key(name) {
                        return Err(GrammarError::UndefinedSymbol {
                            node: id,
                            name: name.clone(),
                        });
                    }
                }
                Node::Seq(children) => {
                    for &child in children {
                        check_child(child)?;
                    }
                }
                Node::Alt(arms) => {
                    if !arms.iter().any(|arm| arm.weight > 0) {
                        return Err(GrammarError::EmptyAlternative(id));
                    }
                    for arm in arms {
                        check_child(arm.node)?;
                    }
                }
                Node::Repeat { body, min, max } => {
                    check_child(*body)?;
                    if let Some(max) = max.filter(|max| max < min) {
                        return Err(GrammarError::BadRepeatBounds {
                            node: id,
                            min: *min,
                            max,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Grammar {
    root: NodeId,
    nodes: Vec<Node>,
    rules: BTreeMap<String, NodeId>,
    actions: Vec<String>,
    action_ids: BTreeMap<String, ActionId>,
    analysis: Analysis,
}

impl Grammar {
    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    pub fn nodes(&self) -> impl ExactSizeIterator<Item = (NodeId, &Node)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (NodeId(index), node))
    }

    pub fn rule(&self, name: &str) -> Option<NodeId> {
        self.rules.get(name).copied()
    }

    pub fn action(&self, name: &str) -> Option<ActionId> {
        self.action_ids.get(name).copied()
    }

    pub fn action_name(&self, id: ActionId) -> Option<&str> {
        self.actions.get(id.index()).map(String::as_str)
    }

    pub fn actions(&self) -> impl ExactSizeIterator<Item = (ActionId, &str)> {
        self.actions
            .iter()
            .enumerate()
            .map(|(index, name)| (ActionId(index), name.as_str()))
    }

    pub fn analysis(&self) -> &Analysis {
        &self.analysis
    }
}

impl Index<NodeId> for Grammar {
    type Output = Node;

    fn index(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }
}
