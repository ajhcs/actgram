use std::collections::VecDeque;

use actgram_exec::ActionResult;

use crate::{compare, wrong_operation, Bug, Op};

const CAPACITY: usize = 3;

struct Queue {
    items: VecDeque<u8>,
    capacity: usize,
    last_popped: Option<u8>,
}

impl Queue {
    fn new() -> Self {
        Self {
            items: VecDeque::new(),
            capacity: CAPACITY,
            last_popped: None,
        }
    }

    fn push(&mut self, value: u8, bug: Option<Bug>) {
        if self.items.len() == self.capacity {
            if bug != Some(Bug::QueueOverflow) {
                return;
            }
            self.items.pop_front();
        }
        self.items.push_back(value);
    }

    fn pop(&mut self) -> Option<u8> {
        let value = self.items.pop_front();
        self.last_popped = value;
        value
    }

    fn peek(&self, bug: Option<Bug>) -> Option<u8> {
        if self.items.is_empty() && bug == Some(Bug::QueueDrain) {
            return self.last_popped;
        }
        self.items.front().copied()
    }

    fn resize(&mut self, capacity: usize, bug: Option<Bug>) {
        if capacity > self.capacity
            && self.items.len() == self.capacity
            && bug == Some(Bug::QueueGrow)
        {
            return;
        }
        self.capacity = capacity;
        if bug == Some(Bug::QueueShrink) {
            while self.items.len() > capacity {
                self.items.pop_front();
            }
        } else {
            self.items.truncate(capacity);
        }
    }

    fn clear(&mut self, bug: Option<Bug>) {
        if bug == Some(Bug::QueueClear) {
            self.items.pop_front();
        } else {
            self.items.clear();
        }
        self.last_popped = None;
    }
}

pub(crate) struct QueueTest {
    actual: Queue,
    expected: Vec<u8>,
    capacity: usize,
}

impl QueueTest {
    pub fn new() -> Self {
        Self {
            actual: Queue::new(),
            expected: Vec::new(),
            capacity: CAPACITY,
        }
    }

    pub fn apply(&mut self, op: Op, bug: Option<Bug>) -> ActionResult {
        use Op::*;
        match op {
            Reset => *self = Self::new(),
            PushA | PushB | PushC | PushD => {
                let value = match op {
                    PushA => 1,
                    PushB => 2,
                    PushC => 3,
                    _ => 4,
                };
                if self.expected.len() < self.capacity {
                    self.expected.push(value);
                }
                self.actual.push(value, bug);
            }
            Pop => {
                let expected = (!self.expected.is_empty()).then(|| self.expected.remove(0));
                return compare(self.actual.pop(), expected, bug);
            }
            Peek => return compare(self.actual.peek(bug), self.expected.first().copied(), bug),
            Shrink | Grow => {
                self.capacity = if op == Shrink { 2 } else { 4 };
                self.expected.truncate(self.capacity);
                self.actual.resize(self.capacity, bug);
            }
            Clear => {
                self.expected.clear();
                self.actual.clear(bug);
            }
            Snapshot => {
                let actual_items: Vec<_> = self.actual.items.iter().copied().collect();
                return compare(
                    (self.actual.capacity, &actual_items),
                    (self.capacity, &self.expected),
                    bug,
                );
            }
            _ => return Err(wrong_operation(op)),
        }
        Ok(())
    }
}
