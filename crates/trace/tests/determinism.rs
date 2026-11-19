use actgram_core::{Builder, Grammar};
use actgram_engine::Choices;
use actgram_exec::{run, Actions, Automaton, EventKind, Execution, Limits, System};
use actgram_trace::{Coverage, ReplayError, Trace};

fn grammar() -> Grammar {
    let mut b = Builder::new();
    let wait = b.action("wait");
    let check = b.action("check");
    let empty = b.seq([]);
    let extra = b.alt([(4, empty), (3, check)]);
    let root = b.seq([wait, extra, check]);
    b.build(root).unwrap()
}

#[derive(Default)]
struct State(bool);
impl System for State {
    fn step(&mut self, tick: u64) {
        self.0 |= tick == 2;
    }
}

fn execute(choices: &mut Choices<'_>) -> Execution {
    let grammar = grammar();
    let mut actions = Actions::new();
    actions.insert("wait", |ctx| {
        Box::pin(async move { ctx.wait_for_within(3, |state: &State| state.0).await })
    });
    actions.insert("check", |ctx| {
        Box::pin(async move {
            assert!(ctx.with_system(|state| state.0));
            Ok(())
        })
    });
    let automata = [
        Automaton::new(&grammar, &actions),
        Automaton::new(&grammar, &actions),
    ];
    run(State::default(), &automata, Limits::default(), choices)
}

#[test]
fn seeded_trace_hashes_and_strict_replay() {
    let seeds = [1, 17, u64::MAX];
    let expected = [
        "5733dd71633649314cd943d97bccb0b2432f06d6a82400ea24db629993498e23",
        "4f5674618562cfc2cfda1423c89faf13653466b5a203149bcafbae8cec93f101",
        "789293d6d16e903de16c93cf06d5a7cbf76f6da3b87a6c74034467c5e7ba8226",
    ];
    let mut hashes = Vec::new();
    for seed in seeds {
        let trace = Trace::record(seed, execute);
        assert_eq!(trace, Trace::record(seed, execute));
        assert_eq!(trace.execution.fault, None);
        assert_eq!(
            trace
                .execution
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Woke)
                .count(),
            2
        );
        let bytes = serde_json::to_vec(&trace).unwrap();
        let decoded = serde_json::from_slice::<Trace>(&bytes).unwrap();
        decoded.replay(execute).unwrap();
        assert_eq!(decoded, trace);
        hashes.push(trace.hash());
    }
    assert_eq!(hashes, expected);
}

#[test]
fn replay_rejects_changed_outcome_and_missing_or_extra_tickets() {
    let trace = Trace::record(17, execute);
    let mut wrong = trace.clone();
    wrong.execution.final_tick += 1;
    assert_eq!(wrong.replay(execute), Err(ReplayError::DifferentOutcome));

    let mut wrong = trace.clone();
    wrong.tape.pop();
    assert!(matches!(
        wrong.replay(execute),
        Err(ReplayError::InvalidTape(_))
    ));

    let mut wrong = trace.clone();
    wrong.tape.push(0);
    assert!(matches!(
        wrong.replay(execute),
        Err(ReplayError::UnconsumedTape { .. })
    ));
}

#[test]
fn coverage_counts_pairs_independently_in_interleaved_automata() {
    let mut coverage = Coverage::default();
    let trace = Trace::record(17, execute);
    coverage.add(&trace).unwrap();
    coverage.add(&trace).unwrap();
    for automaton in 0..2 {
        assert_eq!(coverage.actions[&(automaton, "wait".into())], 2);
        assert_eq!(
            coverage.pairs[&(automaton, "wait".into(), "check".into())],
            2
        );
    }
    assert_eq!(coverage.alternatives.values().sum::<u64>(), 4);
    assert!(!coverage.pairs.keys().any(|(_, _, next)| next == "wait"));
}
