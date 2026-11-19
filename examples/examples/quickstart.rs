use actgram_engine::Eventually;
use actgram_exec::{run, Actions, Automaton, Limits, System};
use actgram_macros::grammar;
use actgram_trace::Trace;

struct Queue(Option<u8>);
impl System for Queue {}

fn main() {
    let grammar = grammar! { start test; test = alt(99 => seq(), 1 => pop); }.unwrap();
    let mut actions = Actions::new();
    actions.insert("pop", |ctx| {
        Box::pin(async move {
            assert_eq!(ctx.with_system_mut(|q: &mut Queue| q.0.take()), Some(7));
            Ok(())
        })
    });
    let pop = grammar.action("pop").unwrap();
    let plan = [Automaton::new(&grammar, &actions).with_goal(Eventually(pop))];
    let trace = Trace::record(17, |c| run(Queue(Some(7)), &plan, Limits::default(), c));
    assert!(trace.execution.fault.is_none());
    println!("{:?}", trace.action_starts().collect::<Vec<_>>());
}
