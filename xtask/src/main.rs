mod baseline;

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use actgram_engine::{Choices, Eventually, Goal, Random};
use actgram_examples::{workloads, Bug, Domain, ExampleSystem, Op, Scenario, BLOCKS, BUGS};
use actgram_exec::FaultKind;
use actgram_trace::{shrink, ShrinkOptions, Trace};
use sha2::{Digest, Sha256};

const SHRINK_LIMIT: usize = 512;
const METHODS: [&str; 3] = ["random", "eventually", "proptest"];

#[derive(Clone, Copy, Debug)]
struct Measurement {
    started: usize,
    detected: bool,
    shrunk: Option<usize>,
}

#[derive(Default)]
struct Summary {
    started: Vec<usize>,
    reduced: Vec<usize>,
    first: Option<u64>,
}

impl Summary {
    fn push(&mut self, seed: u64, sample: Measurement) -> Result<(), String> {
        if let Some(reduced) = sample.shrunk {
            if !sample.detected || reduced > sample.started {
                return Err(format!("invalid reduction on seed {seed}: {sample:?}"));
            }
            self.first.get_or_insert(seed);
            self.started.push(sample.started);
            self.reduced.push(reduced);
        } else if sample.detected {
            return Err(format!("missing reproduction on seed {seed}"));
        }
        Ok(())
    }

    fn count(&self) -> usize {
        self.started.len()
    }

    fn medians(&mut self) -> (String, String) {
        (median(&mut self.started), median(&mut self.reduced))
    }
}

fn median(values: &mut [usize]) -> String {
    if values.is_empty() {
        return String::new();
    }
    values.sort_unstable();
    let doubled = values[values.len() / 2] + values[(values.len() - 1) / 2];
    format!("{}.{}", doubled / 2, 5 * (doubled % 2))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some("evidence".as_ref()) {
        return Err("usage: cargo xtask evidence [--seeds N] [--out-dir PATH]".into());
    }
    let mut seeds = 1000;
    let mut output = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_owned();
    while let Some(arg) = args.next() {
        let value = args.next().ok_or("missing option value")?;
        match arg.to_str() {
            Some("--seeds") => {
                seeds = value.to_str().ok_or("invalid seed count")?.parse()?;
                if seeds == 0 {
                    return Err("seed count must be positive".into());
                }
            }
            Some("--out-dir") => output = PathBuf::from(value),
            _ => return Err(format!("unknown argument {arg:?}").into()),
        }
    }
    evidence(seeds, &output)?;
    Ok(())
}

fn evidence(seeds: u64, output: &Path) -> Result<(), Box<dyn Error>> {
    let denominator = seeds
        .checked_mul(BUGS.len() as u64)
        .ok_or("seed count too large")?;
    let runs = denominator
        .checked_mul(METHODS.len() as u64)
        .ok_or("seed count too large")?;
    let queue = Scenario::new(Domain::Queue);
    let wal = Scenario::new(Domain::Wal);
    let mut per_seed = String::from("bug,target,method,seed,started,detected,shrunk_actions\n");
    let mut csv = String::from("bug,target,method,runs,detections,first_detection_seed,median_to_detection,median_shrunk\n");
    let mut detection = String::from(
        "| Mutation | Target | Random | Eventually | Proptest |\n|---|---|---:|---:|---:|\n",
    );
    let mut lengths =
        String::from("| Mutation | Random | Eventually | Proptest |\n|---|---:|---:|---:|\n");
    let mut totals = [0; 3];
    let mut comparison = [0; 3];
    for bug in BUGS {
        let scenario = if bug.domain() == Domain::Queue {
            &queue
        } else {
            &wal
        };
        let target_name = bug.target().name();
        let target = scenario
            .grammar()
            .action(target_name)
            .ok_or("missing evidence target")?;
        let mut stats: [Summary; 3] = Default::default();
        for seed in 0..seeds {
            let samples = [
                actgram(bug, seed, scenario, Random)?,
                actgram(bug, seed, scenario, Eventually(target))?,
                baseline::run(bug, seed)?,
            ];
            for ((method, stats), sample) in METHODS.into_iter().zip(&mut stats).zip(samples) {
                stats.push(seed, sample)?;
                let shrunk = sample
                    .shrunk
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                writeln!(
                    per_seed,
                    "{},{target_name},{method},{seed},{},{},{shrunk}",
                    bug.name(),
                    sample.started,
                    sample.detected
                )?;
            }
        }
        use std::cmp::Ordering;
        let comparison_index = match stats[1].count().cmp(&stats[0].count()) {
            Ordering::Greater => 0,
            Ordering::Equal => 1,
            Ordering::Less => 2,
        };
        comparison[comparison_index] += 1;
        write!(detection, "| {} | {target_name} |", bug.name())?;
        write!(lengths, "| {} |", bug.name())?;
        for ((method, total), stats) in METHODS.into_iter().zip(&mut totals).zip(&mut stats) {
            let count = stats.count();
            *total += count;
            let first = stats.first.map(|seed| seed.to_string()).unwrap_or_default();
            let (started, reduced) = stats.medians();
            writeln!(
                csv,
                "{},{target_name},{method},{seeds},{count},{first},{started},{reduced}",
                bug.name()
            )?;
            write!(detection, " {count}/{seeds} |")?;
            if count == 0 {
                write!(lengths, " — |")?;
            } else {
                write!(lengths, " {started} → {reduced} |")?;
            }
        }
        detection.push('\n');
        lengths.push('\n');
    }
    writeln!(
        detection,
        "| **Total** | | **{}/{denominator}** | **{}/{denominator}** | **{}/{denominator}** |",
        totals[0], totals[1], totals[2]
    )?;
    let [wins, ties, losses] = comparison;
    let corpus_digest = format!(
        "{:x}",
        Sha256::digest(include_bytes!("../../examples/src/corpus.rs"))
    );
    let report = format!(
        "# Directed traversal evidence\n\n\
         Across {seeds} seeds per mutation, Eventually detected more failures than Random for \
         {wins} mutations, tied for {ties}, and detected fewer for {losses}.\n\n\
         ## Detections per run\n\n{detection}\n\
         ## Median actions through failure and after shrinking\n\n\
         Each cell shows the two medians among detected runs. An em dash means the method found \
         zero failures for that mutation.\n\n{lengths}\n\
         ## Reproduction\n\n\
         Run `cargo xtask evidence` from the workspace to repeat the 1,000-seed experiment. Use \
         `--seeds N --out-dir PATH` for a smaller run. \
         The [protocol](evidence/protocol.md) describes the workload, baseline, and limits. \
         The [summary](evidence/summary.csv) and [per-seed data](evidence/seeds.csv) include \
         misses and conditional medians.\n\n\
         Corpus source SHA-256 `{corpus_digest}`.\n"
    );
    let data = output.join("evidence");
    std::fs::create_dir_all(&data)?;
    let protocol = data.join("protocol.md");
    if !protocol.try_exists()? {
        std::fs::write(protocol, include_bytes!("../../evidence/protocol.md"))?;
    }
    std::fs::write(data.join("seeds.csv"), per_seed)?;
    std::fs::write(data.join("summary.csv"), csv)?;
    std::fs::write(output.join("EVIDENCE.md"), report)?;
    println!("wrote {runs} seeded runs to {}", output.display());
    Ok(())
}

fn actgram(
    bug: Bug,
    seed: u64,
    scenario: &Scenario,
    goal: Goal,
) -> Result<Measurement, Box<dyn Error>> {
    let execute = |choices: &mut Choices<'_>| {
        let system =
            ExampleSystem::new(bug.domain(), Some(bug)).expect("all evidence mutations enabled");
        scenario.execute(system, goal, choices)
    };
    let initial = Trace::record(seed, execute);
    let started = recognized_length(bug.domain(), &initial)?;
    let shrunk = match initial.execution.fault.as_ref().map(|fault| &fault.kind) {
        None => None,
        Some(FaultKind::Assertion { code, .. }) if code == bug.name() => {
            let reduced = shrink(
                &initial,
                ShrinkOptions {
                    max_trials: SHRINK_LIMIT,
                },
                execute,
            )?;
            reduced.trace.replay(execute)?;
            Some(recognized_length(bug.domain(), &reduced.trace)?)
        }
        Some(kind) => {
            return Err(format!(
                "{} seed {seed} failed unexpectedly with {kind:?}",
                bug.name()
            )
            .into())
        }
    };
    Ok(Measurement {
        started,
        detected: shrunk.is_some(),
        shrunk,
    })
}

/// Recognize complete words (or failing prefixes) from reset boundaries and
/// workload commands alone, independently of grammar node or tape choices.
fn recognized_length(domain: Domain, trace: &Trace) -> Result<usize, String> {
    let actions: Vec<_> = trace.action_starts().map(|(_, name)| name).collect();
    let mut blocks = actions.split(|name| *name == Op::Reset.name());
    let leading = blocks.next().expect("split includes the leading slice");
    let mut blocks = blocks.peekable();
    let cases = workloads(domain);
    let mut count = 0;
    if leading.is_empty() {
        while let Some(commands) = blocks.next() {
            count += 1;
            let partial = blocks.peek().is_none() && trace.execution.fault.is_some();
            let known = cases.iter().any(|work| {
                work.ops.split_first().is_some_and(|(first, ops)| {
                    *first == Op::Reset
                        && (commands.len() == ops.len() || (partial && commands.len() < ops.len()))
                        && commands
                            .iter()
                            .zip(ops)
                            .all(|(name, op)| *name == op.name())
                })
            });
            if count > BLOCKS || !known {
                return Err(format!(
                    "illegal {} workload on seed {}",
                    domain.name(),
                    trace.seed
                ));
            }
        }
        return Ok(actions.len());
    }
    Err(format!(
        "missing {} workload reset on seed {}",
        domain.name(),
        trace.seed
    ))
}
