use actgram_engine::{Choices, Random};
use actgram_examples::{workloads, Domain, ExampleSystem, Op, Scenario, BUGS};
use actgram_exec::{EventKind, FaultKind};
use actgram_trace::{shrink, ShrinkOptions, Trace};

#[test]
fn seeded_500_action_run_shrinks_to_its_minimum_legal_word() {
    let scenario = Scenario::new(Domain::Long);
    let execute = |choices: &mut Choices<'_>| {
        scenario.execute(
            ExampleSystem::new(Domain::Long, None).unwrap(),
            Random,
            choices,
        )
    };
    let trace = Trace::record(17, execute);
    assert_eq!(trace.action_starts().count(), 500);
    let smaller = shrink(&trace, ShrinkOptions::default(), execute).unwrap();
    assert!(smaller.locally_minimal);
    assert_eq!(smaller.trace.tape, [0]);
    smaller.trace.replay(execute).unwrap();
    let word = smaller
        .trace
        .action_starts()
        .map(|(_, op)| op)
        .collect::<Vec<_>>();

    // Independent recognizer: the only legal words in the fixture end in
    // arm/check, after at most 498 copies of idle. Length two is optimal.
    assert_eq!(word, [Op::Arm.name(), Op::Check.name()]);
    assert!(matches!(smaller.trace.execution.fault.unwrap().kind,
        FaultKind::Assertion { code, .. } if code == "demo"));
}

#[test]
fn each_mutation_has_a_witness_and_the_clean_systems_pass_every_workload() {
    for domain in [Domain::Queue, Domain::Wal] {
        for work in workloads(domain) {
            let mut clean = ExampleSystem::new(domain, None).unwrap();
            for &op in work.ops {
                clean
                    .apply(op)
                    .unwrap_or_else(|error| panic!("{}: {error}", work.name));
            }
        }
    }
    for bug in BUGS {
        if !bug.enabled() {
            assert!(ExampleSystem::new(bug.domain(), Some(bug)).is_err());
            continue;
        }
        assert!(
            workloads(bug.domain()).iter().any(|work| {
                let mut system = ExampleSystem::new(bug.domain(), Some(bug)).unwrap();
                work.ops.iter().any(|&op| {
                    matches!(system.apply(op),
                Err(actgram_exec::ActionError::Failure { code, .. }) if code == bug.name())
                })
            }),
            "missing witness for {}",
            bug.name()
        );
    }
}

#[test]
fn no_fault_in_clean_seeded_runs() {
    for domain in [Domain::Queue, Domain::Wal] {
        let scenario = Scenario::new(domain);
        for seed in 0..16 {
            let trace = Trace::record(seed, |choices| {
                scenario.execute(ExampleSystem::new(domain, None).unwrap(), Random, choices)
            });
            assert_eq!(trace.execution.fault, None);
            assert!(trace
                .execution
                .events
                .iter()
                .any(|event| matches!(event.kind, EventKind::Alternative { .. })));
        }
    }
}
