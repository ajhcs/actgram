use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;

use actgram_examples::{workloads, Bug, ExampleSystem, BLOCKS};
use actgram_exec::ActionError;
use proptest::strategy::{Just, Union};
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestError, TestRng, TestRunner};
use proptest_state_machine::Sequential;

use crate::{Measurement, SHRINK_LIMIT};

pub(crate) fn run(bug: Bug, seed: u64) -> Result<Measurement, String> {
    let cases = workloads(bug.domain());
    let slots = BLOCKS as usize;
    let strategy = Sequential::new(
        (slots..=slots).into(),
        || Just(()),
        |_, &case| case < cases.len(),
        move |_| {
            Union::new_weighted(
                cases
                    .iter()
                    .enumerate()
                    .map(|(case, work)| (work.weight, Just(case)))
                    .collect(),
            )
        },
        |(), _| (),
    );
    let config = Config {
        cases: 1,
        failure_persistence: None,
        max_shrink_iters: SHRINK_LIMIT as u32,
        max_shrink_time: 0,
        fork: false,
        timeout: 0,
        verbose: 0,
        ..Config::default()
    };
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &bytes));
    let first = Cell::new(None);
    let error = RefCell::new(None);
    let result = runner.run(&strategy, |((), sequence, seen)| {
        let outcome = match probe(bug, &sequence) {
            Ok(outcome) => outcome,
            Err(message) => {
                *error.borrow_mut() = Some(message);
                return Err(TestCaseError::fail("unexpected baseline fault"));
            }
        };
        if let Some(seen) = seen {
            seen.store(outcome.slots, Ordering::SeqCst);
        }
        if first.get().is_none() {
            first.set(Some(outcome));
        }
        if outcome.detected {
            Err(TestCaseError::fail(bug.name()))
        } else {
            Ok(())
        }
    });
    if let Some(error) = error.into_inner() {
        return Err(error);
    }
    let initial = first
        .get()
        .ok_or_else(|| "proptest gave no case".to_owned())?;
    let shrunk = match result {
        Ok(()) if !initial.detected => None,
        Err(TestError::Fail(_, ((), sequence, _))) if initial.detected => {
            let reduced = probe(bug, &sequence)?;
            if !reduced.detected {
                return Err(format!(
                    "baseline shrink lost {} on seed {seed}",
                    bug.name()
                ));
            }
            Some(reduced.actions)
        }
        _ => {
            return Err(format!(
                "proptest reported an unexpected result for {} on seed {seed}: {result:?}",
                bug.name()
            ))
        }
    };
    Ok(Measurement {
        started: initial.actions,
        detected: initial.detected,
        shrunk,
    })
}

#[derive(Clone, Copy)]
struct Probe {
    actions: usize,
    slots: usize,
    detected: bool,
}

fn probe(bug: Bug, sequence: &[usize]) -> Result<Probe, String> {
    let mut system = ExampleSystem::new(bug.domain(), Some(bug))?;
    let mut probe = Probe {
        actions: 0,
        slots: 0,
        detected: false,
    };
    let cases = workloads(bug.domain());
    for &case in sequence {
        probe.slots += 1;
        for &op in cases[case].ops {
            probe.actions += 1;
            match system.apply(op) {
                Ok(()) => {}
                Err(ActionError::Failure { code, .. }) if code == bug.name() => {
                    probe.detected = true;
                    return Ok(probe);
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    Ok(probe)
}
