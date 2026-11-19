//! Fixed workloads and mutation targets used by the evidence experiment.

use crate::{Domain, Op};

pub const BLOCKS: u32 = 8;

#[derive(Clone, Copy, Debug)]
pub struct Workload {
    pub name: &'static str,
    pub weight: u32,
    pub ops: &'static [Op],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bug {
    QueueOverflow,
    QueueDrain,
    QueueShrink,
    QueueGrow,
    QueueClear,
    WalTail,
    WalGhost,
    WalTombstone,
    WalStale,
    WalCheckpoint,
}

pub const BUGS: [Bug; 10] = [
    Bug::QueueOverflow,
    Bug::QueueDrain,
    Bug::QueueShrink,
    Bug::QueueGrow,
    Bug::QueueClear,
    Bug::WalTail,
    Bug::WalGhost,
    Bug::WalTombstone,
    Bug::WalStale,
    Bug::WalCheckpoint,
];

impl Bug {
    pub fn named(name: &str) -> Option<Self> {
        BUGS.into_iter().find(|bug| bug.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::QueueOverflow => "queue-overflow",
            Self::QueueDrain => "queue-drain",
            Self::QueueShrink => "queue-shrink",
            Self::QueueGrow => "queue-grow",
            Self::QueueClear => "queue-clear",
            Self::WalTail => "wal-tail",
            Self::WalGhost => "wal-ghost",
            Self::WalTombstone => "wal-tombstone",
            Self::WalStale => "wal-stale",
            Self::WalCheckpoint => "wal-checkpoint",
        }
    }

    pub fn domain(self) -> Domain {
        match self {
            Self::QueueOverflow
            | Self::QueueDrain
            | Self::QueueShrink
            | Self::QueueGrow
            | Self::QueueClear => Domain::Queue,
            _ => Domain::Wal,
        }
    }

    pub fn target(self) -> Op {
        match self {
            Self::QueueOverflow => Op::PushD,
            Self::QueueDrain => Op::Peek,
            Self::QueueShrink => Op::Shrink,
            Self::QueueGrow => Op::Grow,
            Self::QueueClear => Op::Clear,
            Self::WalTail => Op::PutB,
            Self::WalGhost => Op::ReadA,
            Self::WalTombstone => Op::Delete,
            Self::WalStale => Op::Recover,
            Self::WalCheckpoint => Op::Checkpoint,
        }
    }

    pub fn enabled(self) -> bool {
        match self {
            Self::QueueOverflow => cfg!(feature = "bug-queue-overflow"),
            Self::QueueDrain => cfg!(feature = "bug-queue-drain"),
            Self::QueueShrink => cfg!(feature = "bug-queue-shrink"),
            Self::QueueGrow => cfg!(feature = "bug-queue-grow"),
            Self::QueueClear => cfg!(feature = "bug-queue-clear"),
            Self::WalTail => cfg!(feature = "bug-wal-tail"),
            Self::WalGhost => cfg!(feature = "bug-wal-ghost"),
            Self::WalTombstone => cfg!(feature = "bug-wal-tombstone"),
            Self::WalStale => cfg!(feature = "bug-wal-stale"),
            Self::WalCheckpoint => cfg!(feature = "bug-wal-checkpoint"),
        }
    }
}

use Op::*;

macro_rules! workloads {
    ($($name:literal, $weight:literal => [$($op:ident),* $(,)?]);+ $(;)?) => {
        &[$(Workload { name: $name, weight: $weight, ops: &[$($op),*] }),+]
    };
}

const QUEUE: &[Workload] = workloads! {
    "skip", 16 => [];
    "empty peek", 32 => [Reset, Peek];
    "push then peek", 40 => [Reset, PushA, Peek];
    "FIFO", 32 => [Reset, PushA, PushB, Pop, Snapshot];
    "three items", 24 => [Reset, PushA, PushB, PushC, Snapshot];
    "overflow", 2 => [Reset, PushA, PushB, PushC, PushD, Snapshot];
    "drain and peek", 3 => [Reset, PushA, Pop, Peek];
    "shrink under load", 2 => [Reset, PushA, PushB, PushC, Shrink, Snapshot];
    "grow under load", 2 => [Reset, Shrink, PushA, PushB, Grow, PushC, Snapshot];
    "clear under load", 1 => [Reset, PushA, PushB, Clear, Peek];
};

const WAL: &[Workload] = workloads! {
    "skip", 16 => [];
    "empty read", 32 => [Reset, ReadA];
    "ordinary read", 40 => [Reset, PutA, ReadA];
    "empty restart", 32 => [Reset, Crash, Recover, ReadA];
    "persist and restart", 24 => [Reset, PutA, Flush, Crash, Recover, ReadA];
    "flush batch", 2 => [Reset, PutA, PutB, Flush, Crash, Recover, ReadB];
    "unflushed restart", 3 => [Reset, PutA, Crash, Recover, ReadA];
    "durable delete", 2 => [Reset, PutA, Flush, Delete, Flush, Crash, Recover, ReadA];
    "durable overwrite", 2 => [Reset, PutA, Flush, ReplaceA, Flush, Crash, Recover, ReadA];
    "compact and restart", 1 => [Reset, PutA, Flush, Checkpoint, Crash, Recover, ReadA];
};

pub fn workloads(domain: Domain) -> &'static [Workload] {
    match domain {
        Domain::Queue => QUEUE,
        Domain::Wal => WAL,
        Domain::Long => &[],
    }
}
