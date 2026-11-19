use std::cell::Cell;
use std::rc::Rc;

use actgram_core::{ActionId, Builder, Grammar};
use actgram_engine::{Choices, Next};
use actgram_exec::{
    run, ActionResult, Actions, Automaton, Ctx, EventKind, FaultKind, Limits, System,
};

fn action_grammar(name: &str) -> Grammar {
    let mut b = Builder::new();
    let root = b.action(name);
    b.build(root).unwrap()
}

#[derive(Default)]
struct Gate {
    ready: bool,
    opens_at: Option<u64>,
    dropped: Rc<Cell<bool>>,
}

impl System for Gate {
    fn step(&mut self, tick: u64) {
        self.ready |= self.opens_at == Some(tick);
    }
}

async fn wait(ctx: &mut Ctx<Gate>) -> ActionResult {
    ctx.wait_for_within(2, |gate| gate.ready).await
}

#[test]
fn wake_order_is_the_seeded_schedule_and_deadline_is_inclusive() {
    let grammar = action_grammar("wait");
    let mut actions = Actions::new();
    actions.insert("wait", |ctx| Box::pin(wait(ctx)));
    let automata = [
        Automaton::new(&grammar, &actions),
        Automaton::new(&grammar, &actions),
    ];
    let mut choices = Choices::replay(&[0, 0, 1]);
    let execution = run(
        Gate {
            opens_at: Some(2),
            ..Gate::default()
        },
        &automata,
        Limits::default(),
        &mut choices,
    );
    assert_eq!(execution.fault, None);
    assert_eq!(execution.final_tick, 2);
    let wakes: Vec<_> = execution
        .events
        .iter()
        .filter(|e| e.kind == EventKind::Woke)
        .map(|e| (e.tick, e.automaton))
        .collect();
    assert_eq!(wakes, [(2, 1), (2, 0)]);
    assert_eq!(choices.observed(), &[0, 0, 1]);
}

#[test]
fn signal_arriving_before_wait_is_seen_without_a_tick() {
    let grammar = action_grammar("wait");
    let mut actions = Actions::new();
    actions.insert("wait", |ctx| Box::pin(wait(ctx)));
    let execution = run(
        Gate {
            ready: true,
            ..Gate::default()
        },
        &[Automaton::new(&grammar, &actions)],
        Limits::default(),
        &mut Choices::seeded(4),
    );
    assert_eq!(execution.fault, None);
    assert_eq!(execution.final_tick, 0);
    assert_eq!(execution.events.len(), 2);
}

#[test]
fn expired_wait_remains_a_hang_even_if_the_action_ignores_it() {
    let grammar = action_grammar("wait");
    let mut actions = Actions::new();
    actions.insert("wait", |ctx| {
        Box::pin(async move {
            let _ = ctx.wait_for(|_: &Gate| false).await;
            Ok(())
        })
    });
    let execution = run(
        Gate::default(),
        &[Automaton::new(&grammar, &actions)],
        Limits {
            default_wait_ticks: 3,
            ..Limits::default()
        },
        &mut Choices::seeded(4),
    );
    let fault = execution.fault.unwrap();
    assert_eq!(fault.kind, FaultKind::Hang { deadline: 3 });
    assert_eq!(fault.tick, 3);
    assert_eq!(fault.action.as_deref(), Some("wait"));
}

struct DropFlag(Rc<Cell<bool>>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn cancelling_a_peer_drops_its_pending_action_between_polls() {
    let waiter = action_grammar("wait");
    let canceller = action_grammar("cancel");
    let mut actions = Actions::new();
    actions.insert("wait", |ctx| {
        Box::pin(async move {
            let _flag = DropFlag(ctx.with_system(|gate: &Gate| gate.dropped.clone()));
            ctx.wait_for(|_| false).await
        })
    });
    actions.insert("cancel", |ctx| {
        Box::pin(async move {
            assert!(!ctx.cancel_peer(ctx.automaton()));
            assert!(ctx.cancel_peer(0));
            Ok(())
        })
    });
    let gate = Gate::default();
    let dropped = gate.dropped.clone();
    let execution = run(
        gate,
        &[
            Automaton::new(&waiter, &actions),
            Automaton::new(&canceller, &actions),
        ],
        Limits::default(),
        &mut Choices::replay(&[0]),
    );
    assert_eq!(execution.fault, None);
    assert_eq!(execution.final_tick, 0);
    assert!(dropped.get());
    assert!(execution.events.iter().any(|e| e.automaton == 0
        && e.kind
            == (EventKind::Cancelled {
                action: Some("wait".into())
            })));
}

#[test]
fn unsupported_pending_and_action_panics_are_reported() {
    let grammar = action_grammar("bad");
    let mut actions = Actions::new();
    actions.insert("bad", |_| Box::pin(std::future::pending()));
    let execute = |actions: &Actions<()>| {
        run(
            (),
            &[Automaton::new(&grammar, actions)],
            Limits::default(),
            &mut Choices::seeded(1),
        )
        .fault
        .unwrap()
        .kind
    };
    assert_eq!(execute(&actions), FaultKind::UnsupportedPending);
    actions.insert("bad", |_| Box::pin(async { panic!("action failed") }));
    assert_eq!(
        execute(&actions),
        FaultKind::Panic {
            message: "action failed".into()
        }
    );
}

#[test]
fn running_actions_can_change_the_strategy_stack() {
    struct Directive(ActionId);
    impl System for Directive {}
    let mut b = Builder::new();
    let steer = b.action("steer");
    let take = b.action("take");
    let other = b.action("other");
    let alt = b.alt([(1000, other), (1, take)]);
    let root = b.seq([steer, alt]);
    let grammar = b.build(root).unwrap();
    let target = grammar.action("take").unwrap();
    let mut actions = Actions::new();
    actions.insert("steer", |ctx| {
        Box::pin(async move {
            let target = ctx.with_system(|state: &Directive| state.0);
            ctx.push_strategy(Next(target));
            Ok(())
        })
    });
    actions.insert("take", |_| Box::pin(async { Ok(()) }));
    let execution = run(
        Directive(target),
        &[Automaton::new(&grammar, &actions)],
        Limits::default(),
        &mut Choices::replay(&[]),
    );
    assert_eq!(execution.fault, None);
    assert!(execution.events.iter().any(|e| e.kind
        == (EventKind::Started {
            action: "take".into()
        })));
}
