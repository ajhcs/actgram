use actgram_core::Builder;
use actgram_engine::Choices;
use actgram_exec::{run, ActionError, Actions, Automaton, Limits, System};
use actgram_trace::{shrink, ShrinkOptions, Trace};

#[test]
fn an_edit_cannot_replace_the_failure_with_another_assertion_at_the_same_action() {
    struct State(&'static str);
    impl System for State {}

    let mut builder = Builder::new();
    let configure = builder.action("configure");
    let check = builder.action("check");
    let longer = builder.seq([configure, check]);
    let root = builder.alt([(1, check), (1, longer)]);
    let grammar = builder.build(root).unwrap();
    let mut actions = Actions::new();
    actions.insert("configure", |ctx| {
        Box::pin(async move {
            ctx.with_system_mut(|state: &mut State| state.0 = "original");
            Ok(())
        })
    });
    actions.insert("check", |ctx| {
        Box::pin(async move {
            Err(ActionError::failed(
                ctx.with_system(|state| state.0),
                "check failed",
            ))
        })
    });
    let automata = [Automaton::new(&grammar, &actions)];
    let execute =
        |choices: &mut Choices<'_>| run(State("different"), &automata, Limits::default(), choices);
    let initial = (0..32)
        .map(|seed| Trace::record(seed, execute))
        .find(|trace| trace.action_starts().count() == 2)
        .unwrap();

    let zero = shrink(&initial, ShrinkOptions { max_trials: 0 }, execute).unwrap();
    assert_eq!(zero.trace, initial);
    assert!(!zero.locally_minimal);
    assert_eq!(zero.trials, 0);

    let reduced = shrink(&initial, ShrinkOptions::default(), execute).unwrap();
    assert_eq!(reduced.trace, initial);
    assert!(reduced.locally_minimal);
}
