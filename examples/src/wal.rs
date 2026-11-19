use std::collections::BTreeMap;

use actgram_exec::{ActionError, ActionResult};

use crate::{compare, wrong_operation, Bug, Op};

type Map = BTreeMap<char, u8>;

#[derive(Clone, Copy)]
enum Record {
    Put(char, u8),
    Delete(char),
}

impl Record {
    fn apply(self, state: &mut Map) {
        match self {
            Self::Put(key, value) => {
                state.insert(key, value);
            }
            Self::Delete(key) => {
                state.remove(&key);
            }
        }
    }
}

#[derive(Default)]
struct Wal {
    live: Map,
    volatile: Vec<Record>,
    log: Vec<Record>,
    snapshot: Map,
}

impl Wal {
    fn append(&mut self, record: Record) {
        record.apply(&mut self.live);
        self.volatile.push(record);
    }

    fn flush(&mut self, bug: Option<Bug>) {
        if self.volatile.len() > 1 && bug == Some(Bug::WalTail) {
            self.volatile.pop();
        }
        self.log.append(&mut self.volatile);
    }

    fn crash(&mut self, bug: Option<Bug>) {
        if bug == Some(Bug::WalGhost) {
            // A volatile tail is incorrectly retained in the persistent log.
            self.log.extend(self.volatile.pop());
        }
        self.volatile.clear();
        self.live.clear();
    }

    fn recover(&mut self, bug: Option<Bug>) {
        self.live.clone_from(&self.snapshot);
        for &record in &self.log {
            match record {
                Record::Delete(_) if bug == Some(Bug::WalTombstone) => {}
                Record::Put(key, _)
                    if bug == Some(Bug::WalStale) && self.live.contains_key(&key) => {}
                _ => record.apply(&mut self.live),
            }
        }
    }

    fn checkpoint(&mut self, bug: Option<Bug>) {
        if bug == Some(Bug::WalCheckpoint) {
            self.log.pop();
        }
        for record in self.log.drain(..) {
            record.apply(&mut self.snapshot);
        }
    }
}

#[derive(Default)]
pub(crate) struct WalTest {
    actual: Wal,
    // The reference model persists the visible map at flush, independent of log replay.
    visible: Map,
    persisted: Map,
    crashed: bool,
}

impl WalTest {
    pub fn apply(&mut self, op: Op, bug: Option<Bug>) -> ActionResult {
        use Op::*;
        match op {
            Reset => {
                *self = Self::default();
                return Ok(());
            }
            Recover if self.crashed => {
                self.actual.recover(bug);
                self.visible.clone_from(&self.persisted);
                self.crashed = false;
                return Ok(());
            }
            _ if self.crashed || op == Recover => {
                return Err(ActionError::failed(
                    "workload",
                    "invalid WAL crash/recovery ordering",
                ));
            }
            _ => {}
        }
        match op {
            PutA | ReplaceA | PutB | Delete => {
                let record = match op {
                    PutA => Record::Put('a', 1),
                    ReplaceA => Record::Put('a', 2),
                    PutB => Record::Put('b', 1),
                    _ => Record::Delete('a'),
                };
                record.apply(&mut self.visible);
                self.actual.append(record);
            }
            Flush => {
                self.persisted.clone_from(&self.visible);
                self.actual.flush(bug);
            }
            Crash => {
                self.visible.clear();
                self.actual.crash(bug);
                self.crashed = true;
            }
            Checkpoint => self.actual.checkpoint(bug),
            ReadA | ReadB => {
                let key = if op == ReadA { 'a' } else { 'b' };
                return compare(self.actual.live.get(&key), self.visible.get(&key), bug);
            }
            _ => return Err(wrong_operation(op)),
        }
        Ok(())
    }
}
