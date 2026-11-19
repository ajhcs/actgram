use std::any::Any;
use std::cell::{Cell, RefCell};
use std::num::NonZeroU64;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use actgram_engine::{Choices, Engine, EngineError, Goal};

use crate::context::{Control, Effect, Runtime};
use crate::{
    ActionError, ActionFuture, Automaton, Ctx, Event, EventKind, Execution, Fault, FaultKind,
    Limits, System,
};

struct Active {
    name: String,
    future: ActionFuture<'static>,
    control: Rc<RefCell<Control>>,
}

struct Slot<'a, S> {
    engine: Engine<'a>,
    definition: &'a Automaton<'a, S>,
    active: Option<Active>,
    done: bool,
}

struct Run<S> {
    runtime: Rc<Runtime<S>>,
    limits: Limits,
    action_count: u64,
    events: Vec<Event>,
    automata: usize,
}

/// Each live automaton gets one turn per tick in seeded order. A turn
/// polls one in-flight or newly started action. After the round, the
/// system steps once before any action is polled at the next tick.
pub fn run<S: System + 'static>(
    system: S,
    automata: &[Automaton<'_, S>],
    limits: Limits,
    choices: &mut Choices<'_>,
) -> Execution {
    let mut slots: Vec<_> = automata
        .iter()
        .map(|automaton| {
            let mut engine = Engine::new(automaton.grammar);
            engine.set_expansion_limit(limits.max_expansions_per_action);
            if automaton.goal != Goal::Random {
                engine.push_strategy(automaton.goal);
            }
            Slot {
                engine,
                definition: automaton,
                active: None,
                done: false,
            }
        })
        .collect();
    let mut state = Run {
        runtime: Rc::new(Runtime {
            tick: Cell::new(0),
            system: RefCell::new(system),
        }),
        limits,
        action_count: 0,
        events: Vec::new(),
        automata: slots.len(),
    };
    let fault = state.drive(&mut slots, choices).err();
    if let Some(fault) = &fault {
        for (index, slot) in slots.iter_mut().enumerate() {
            if Some(index) != fault.automaton {
                state.cancel(index, slot);
            }
        }
    }
    Execution {
        events: state.events,
        final_tick: state.runtime.tick.get(),
        fault,
    }
}

impl<S: System + 'static> Run<S> {
    fn drive(&mut self, slots: &mut [Slot<'_, S>], choices: &mut Choices<'_>) -> Result<(), Fault> {
        let mut round = Vec::with_capacity(slots.len());
        loop {
            round.extend(
                slots
                    .iter()
                    .enumerate()
                    .filter_map(|(index, slot)| (!slot.done).then_some(index)),
            );
            while let Some(count) = NonZeroU64::new(round.len() as u64) {
                let position = choices.draw(count).map_err(|error| {
                    self.fault(
                        None,
                        None,
                        FaultKind::ChoiceTape {
                            detail: error.to_string(),
                        },
                    )
                })?;
                let index = round.swap_remove(position as usize);
                if slots[index].active.is_none() {
                    self.start(index, &mut slots[index], choices)?;
                }
                if slots[index].active.is_some() && self.poll(index, slots)? {
                    round.retain(|&index| !slots[index].done);
                }
            }
            if slots.iter().all(|slot| slot.done) {
                return Ok(());
            }
            let tick = self.runtime.tick.get();
            if tick == self.limits.max_tick {
                return Err(self.fault(None, None, FaultKind::TickLimit { limit: tick }));
            }
            self.runtime.tick.set(tick + 1);
            catch_unwind(AssertUnwindSafe(|| {
                self.runtime.system.borrow_mut().step(tick + 1)
            }))
            .map_err(|panic| self.fault(None, None, panic_kind(&*panic)))?;
        }
    }

    fn start(
        &mut self,
        index: usize,
        slot: &mut Slot<'_, S>,
        choices: &mut Choices<'_>,
    ) -> Result<(), Fault> {
        let next = slot.engine.advance(choices).map_err(|error| {
            let detail = error.to_string();
            let kind = match error {
                EngineError::Tape(_) => FaultKind::ChoiceTape { detail },
                _ => FaultKind::Traversal { detail },
            };
            self.fault(Some(index), None, kind)
        })?;
        for alternate in next.alternatives {
            self.emit(
                index,
                EventKind::Alternative {
                    node: alternate.node.index(),
                    arm: alternate.arm,
                },
            );
        }
        let Some(action) = next.action else {
            slot.done = true;
            return Ok(());
        };
        let name = slot
            .definition
            .grammar
            .action_name(action)
            .expect("interned action");
        if self.action_count == self.limits.max_actions {
            return Err(self.fault(
                Some(index),
                Some(name.into()),
                FaultKind::ActionLimit {
                    limit: self.limits.max_actions,
                },
            ));
        }
        let function = slot
            .definition
            .actions
            .0
            .get(name)
            .copied()
            .ok_or_else(|| self.fault(Some(index), Some(name.into()), FaultKind::MissingAction))?;

        self.action_count += 1;
        self.emit(
            index,
            EventKind::Started {
                action: name.into(),
            },
        );
        let control = Rc::new(RefCell::new(Control::default()));
        let mut ctx = Ctx::new(
            self.runtime.clone(),
            control.clone(),
            index,
            self.automata,
            self.limits.default_wait_ticks,
        );
        let future = Box::pin(async move { function(&mut ctx).await });
        slot.active = Some(Active {
            name: name.into(),
            future,
            control,
        });
        Ok(())
    }

    // A true result tells the scheduler to remove cancelled turns in this round.
    fn poll(&mut self, index: usize, slots: &mut [Slot<'_, S>]) -> Result<bool, Fault> {
        let active = slots[index]
            .active
            .as_mut()
            .expect("poll requires an action");
        let poll = catch_unwind(AssertUnwindSafe(|| {
            active
                .future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
        }));
        let control = std::mem::take(&mut *active.control.borrow_mut());
        let mut cancelled = false;
        for effect in control.effects {
            match effect {
                Effect::Event(event) => self.emit(index, event),
                Effect::Push(goal) => slots[index].engine.push_strategy(goal),
                Effect::Pop => {
                    slots[index].engine.pop_strategy();
                }
                Effect::Cancel(peer) => {
                    self.cancel(peer, &mut slots[peer]);
                    cancelled = true;
                }
            }
        }

        let result = match (control.expired, poll) {
            (Some(deadline), _)
            | (_, Ok(Poll::Ready(Err(ActionError::WaitExpired { deadline })))) => {
                Err(FaultKind::Hang { deadline })
            }
            (_, Ok(Poll::Ready(Ok(())))) => Ok(()),
            (_, Ok(Poll::Ready(Err(ActionError::Failure { code, message })))) => {
                Err(FaultKind::Assertion { code, message })
            }
            (_, Ok(Poll::Pending)) if control.waiting => return Ok(cancelled),
            (_, Ok(Poll::Pending)) => Err(FaultKind::UnsupportedPending),
            (_, Err(panic)) => Err(panic_kind(&*panic)),
        };

        let slot = &mut slots[index];
        let active = slot.active.take().expect("polled action");
        match result {
            Ok(()) => {
                self.emit(
                    index,
                    EventKind::Completed {
                        action: active.name,
                    },
                );
                // An action may push a goal even when the object stack is empty.
                slot.done = slot.engine.is_exhausted() && slot.engine.strategy() == Goal::Random;
                Ok(cancelled)
            }
            Err(kind) => Err(self.fault(Some(index), Some(active.name), kind)),
        }
    }

    fn cancel(&mut self, index: usize, slot: &mut Slot<'_, S>) {
        if !slot.done {
            let action = slot.active.take().map(|active| active.name);
            slot.done = true;
            self.emit(index, EventKind::Cancelled { action });
        }
    }

    fn emit(&mut self, automaton: usize, kind: EventKind) {
        self.events.push(Event {
            tick: self.runtime.tick.get(),
            automaton,
            kind,
        });
    }

    fn fault(&self, automaton: Option<usize>, action: Option<String>, kind: FaultKind) -> Fault {
        Fault {
            tick: self.runtime.tick.get(),
            automaton,
            action,
            kind,
        }
    }
}

fn panic_kind(payload: &(dyn Any + Send)) -> FaultKind {
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload");
    FaultKind::Panic {
        message: message.into(),
    }
}
