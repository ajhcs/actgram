use std::collections::BTreeMap;

use actgram_exec::EventKind;

use crate::{ReplayError, Trace};

/// Counts keyed by automaton. Action pairs follow each automaton's own
/// stream of starts, including the action that raised a fault.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Coverage {
    pub actions: BTreeMap<(usize, String), u64>,
    pub alternatives: BTreeMap<(usize, usize, usize), u64>,
    pub pairs: BTreeMap<(usize, String, String), u64>,
}

impl Coverage {
    /// Combine runs of the same plan, with the same automaton and node indices.
    pub fn add(&mut self, trace: &Trace) -> Result<(), ReplayError> {
        trace.check_version()?;
        let mut previous = BTreeMap::new();
        for event in &trace.execution.events {
            let id = event.automaton;
            match &event.kind {
                EventKind::Started { action } => {
                    *self.actions.entry((id, action.clone())).or_default() += 1;
                    if let Some(last) = previous.insert(id, action) {
                        *self
                            .pairs
                            .entry((id, last.clone(), action.clone()))
                            .or_default() += 1;
                    }
                }
                EventKind::Alternative { node, arm } => {
                    *self.alternatives.entry((id, *node, *arm)).or_default() += 1;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
