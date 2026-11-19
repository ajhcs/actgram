use std::cell::{Cell, RefCell};
use std::future::poll_fn;
use std::rc::Rc;
use std::task::Poll;

use actgram_engine::Goal;

use crate::{ActionError, ActionResult, EventKind};

pub(crate) struct Runtime<S> {
    pub system: RefCell<S>,
    pub tick: Cell<u64>,
}

#[derive(Default)]
pub(crate) struct Control {
    pub effects: Vec<Effect>,
    pub waiting: bool,
    pub expired: Option<u64>,
}

pub(crate) enum Effect {
    Event(EventKind),
    Push(Goal),
    Pop,
    Cancel(usize),
}

/// An action's access to the shared system and virtual executor. Access
/// closures release their borrow before the action can yield.
pub struct Ctx<S> {
    runtime: Rc<Runtime<S>>,
    control: Rc<RefCell<Control>>,
    automaton: usize,
    automata: usize,
    wait_ticks: u64,
}

impl<S> Ctx<S> {
    pub(crate) fn new(
        runtime: Rc<Runtime<S>>,
        control: Rc<RefCell<Control>>,
        automaton: usize,
        automata: usize,
        wait_ticks: u64,
    ) -> Self {
        Self {
            runtime,
            control,
            automaton,
            automata,
            wait_ticks,
        }
    }

    pub fn now(&self) -> u64 {
        self.runtime.tick.get()
    }

    pub fn automaton(&self) -> usize {
        self.automaton
    }

    pub fn with_system<T>(&self, access: impl FnOnce(&S) -> T) -> T {
        access(&self.runtime.system.borrow())
    }

    pub fn with_system_mut<T>(&mut self, access: impl FnOnce(&mut S) -> T) -> T {
        access(&mut self.runtime.system.borrow_mut())
    }

    /// Push a policy for this automaton's next traversal.
    pub fn push_strategy(&mut self, strategy: Goal) {
        self.control
            .borrow_mut()
            .effects
            .push(Effect::Push(strategy));
    }

    pub fn pop_strategy(&mut self) {
        self.control.borrow_mut().effects.push(Effect::Pop);
    }

    /// Request cancellation of a peer after this poll. Returns false for
    /// the caller's own index or an index outside the run.
    pub fn cancel_peer(&mut self, automaton: usize) -> bool {
        if automaton >= self.automata || automaton == self.automaton {
            return false;
        }
        self.control
            .borrow_mut()
            .effects
            .push(Effect::Cancel(automaton));
        true
    }

    /// Wait for a level condition for up to the run's default wait budget.
    pub async fn wait_for(&self, condition: impl FnMut(&S) -> bool) -> ActionResult {
        self.wait_for_within(self.wait_ticks, condition).await
    }

    /// Test the condition on the current poll, then on each scheduled poll
    /// through the deadline. A true condition at the deadline succeeds.
    pub async fn wait_for_within(
        &self,
        ticks: u64,
        mut condition: impl FnMut(&S) -> bool,
    ) -> ActionResult {
        let deadline = self.now().saturating_add(ticks);
        let mut blocked = false;
        poll_fn(|_| {
            if self.with_system(&mut condition) {
                if blocked {
                    self.control
                        .borrow_mut()
                        .effects
                        .push(Effect::Event(EventKind::Woke));
                }
                return Poll::Ready(Ok(()));
            }
            let mut control = self.control.borrow_mut();
            if self.now() >= deadline {
                control.expired.get_or_insert(deadline);
                return Poll::Ready(Err(ActionError::WaitExpired { deadline }));
            }
            control.waiting = true;
            if !blocked {
                control
                    .effects
                    .push(Effect::Event(EventKind::Waiting { deadline }));
                blocked = true;
            }
            Poll::Pending
        })
        .await
    }
}
