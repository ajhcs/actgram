use std::error::Error;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use actgram_engine::{Choices, Eventually, Goal, Next, Random};
use actgram_examples::{Bug, Domain, ExampleSystem, Scenario};
use actgram_exec::{Execution, FaultKind};
use actgram_trace::{shrink, Coverage, ShrinkOptions, Trace};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Execute a built-in scenario.
    Run {
        #[arg(value_parser = ["queue", "wal", "long"])]
        system: String,
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Enable a named demonstration mutation when compiled with its feature.
        #[arg(long)]
        bug: Option<String>,
        /// Steer to this action by its shortest reachable path.
        #[arg(long, conflicts_with = "next")]
        eventually: Option<String>,
        /// Require this action to be the first action.
        #[arg(long)]
        next: Option<String>,
        /// Write a JSON trace to a new file.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Verify that a saved trace reproduces its events and outcome.
    Replay { trace: PathBuf },
    /// Reduce a saved assertion, panic, or virtual wait hang.
    Shrink {
        trace: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = ShrinkOptions::default().max_trials)]
        max_trials: usize,
    },
    /// Count actions, alternatives, and action pairs across saved traces of one system.
    Coverage {
        #[arg(required = true)]
        traces: Vec<PathBuf>,
    },
}

#[derive(Deserialize, Serialize)]
struct SavedRun {
    plan: Plan,
    trace: Trace,
}

#[derive(Deserialize, Serialize)]
struct Plan {
    system: String,
    bug: Option<String>,
    strategy: Strategy,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", content = "action", rename_all = "snake_case")]
enum Strategy {
    Random,
    Next(String),
    Eventually(String),
}

struct Prepared {
    domain: Domain,
    bug: Option<Bug>,
    scenario: Scenario,
    goal: Goal,
}

impl Plan {
    fn prepare(&self) -> Result<Prepared, String> {
        let domain = Domain::named(&self.system)
            .ok_or_else(|| format!("unknown system {:?}", self.system))?;
        let bug = self
            .bug
            .as_deref()
            .map(|name| Bug::named(name).ok_or_else(|| format!("unknown mutation {name:?}")))
            .transpose()?;
        // Validate the mutation and compiled features before creating the run.
        ExampleSystem::new(domain, bug)?;
        let scenario = Scenario::new(domain);
        let id = |name: &str| {
            scenario.grammar().action(name).ok_or_else(|| {
                let actions = scenario
                    .grammar()
                    .actions()
                    .map(|(_, name)| name)
                    .collect::<Vec<_>>();
                format!(
                    "unknown action {name:?} in {}. Available actions are {}",
                    self.system,
                    actions.join(", ")
                )
            })
        };
        let goal = match &self.strategy {
            Strategy::Random => Random,
            Strategy::Next(name) => Next(id(name)?),
            Strategy::Eventually(name) => Eventually(id(name)?),
        };
        Ok(Prepared {
            domain,
            bug,
            scenario,
            goal,
        })
    }
}

impl Prepared {
    fn execute(&self, choices: &mut Choices<'_>) -> Execution {
        let system = ExampleSystem::new(self.domain, self.bug).expect("validated scenario");
        self.scenario.execute(system, self.goal, choices)
    }
}

fn main() -> ExitCode {
    match dispatch(Cli::parse().command) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("actgram error. {error}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(command: Command) -> Result<ExitCode, Box<dyn Error>> {
    match command {
        Command::Run {
            system,
            seed,
            bug,
            eventually,
            next,
            out,
        } => {
            let strategy = match (eventually, next) {
                (Some(name), None) => Strategy::Eventually(name),
                (None, Some(name)) => Strategy::Next(name),
                _ => Strategy::Random,
            };
            let plan = Plan {
                system,
                bug,
                strategy,
            };
            let prepared = plan.prepare()?;
            let trace = Trace::record(seed, |choices| prepared.execute(choices));
            print_trace(&trace);
            let status = match trace.execution.fault.as_ref().map(|fault| &fault.kind) {
                None => ExitCode::SUCCESS,
                Some(
                    FaultKind::Assertion { .. } | FaultKind::Panic { .. } | FaultKind::Hang { .. },
                ) => ExitCode::FAILURE,
                Some(_) => ExitCode::from(2),
            };
            if let Some(out) = out {
                save(&out, &SavedRun { plan, trace })?;
            }
            Ok(status)
        }
        Command::Replay { trace } => {
            let saved = load(&trace)?;
            let prepared = saved.plan.prepare()?;
            saved.trace.replay(|choices| prepared.execute(choices))?;
            println!("verified replay");
            print_trace(&saved.trace);
            Ok(ExitCode::SUCCESS)
        }
        Command::Shrink {
            trace,
            out,
            max_trials,
        } => {
            let SavedRun { plan, trace } = load(&trace)?;
            let prepared = plan.prepare()?;
            let reduced = shrink(&trace, ShrinkOptions { max_trials }, |choices| {
                prepared.execute(choices)
            })?;
            reduced.trace.replay(|choices| prepared.execute(choices))?;
            println!(
                "{} → {} actions in {} trials. {}.",
                trace.action_starts().count(),
                reduced.trace.action_starts().count(),
                reduced.trials,
                if reduced.locally_minimal {
                    "local minimum"
                } else {
                    "trial budget exhausted"
                },
            );
            print_trace(&reduced.trace);
            save(
                &out,
                &SavedRun {
                    plan,
                    trace: reduced.trace,
                },
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Coverage { traces } => {
            coverage(&traces)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn save(path: &Path, saved: &SavedRun) -> Result<(), Box<dyn Error>> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, saved)?;
    file.write_all(b"\n")?;
    file.persist_noclobber(path)
        .map_err(|error| format!("could not save {}. {}", path.display(), error.error))?;
    println!("saved {}", path.display());
    Ok(())
}

fn load(path: &Path) -> Result<SavedRun, Box<dyn Error>> {
    let file =
        File::open(path).map_err(|error| format!("could not open {}. {error}", path.display()))?;
    Ok(serde_json::from_reader(BufReader::new(file))?)
}

fn print_trace(trace: &Trace) {
    let count = trace.action_starts().count();
    println!(
        "seed={} actions={count} tick={} hash={}",
        trace.seed,
        trace.execution.final_tick,
        trace.hash()
    );
    if (1..=16).contains(&count) {
        let actions: Vec<_> = trace.action_starts().map(|(_, name)| name).collect();
        println!("{}", actions.join(" → "));
    }
    if let Some(fault) = &trace.execution.fault {
        let action = fault.action.as_deref().unwrap_or("executor");
        match &fault.kind {
            FaultKind::Assertion { code, message } => {
                println!("{action} failed [{code}]. {message}")
            }
            FaultKind::Panic { message } => println!("{action} panicked. {message}"),
            FaultKind::Hang { deadline } => println!("{action} hung at virtual tick {deadline}"),
            FaultKind::Traversal { detail } | FaultKind::ChoiceTape { detail } => {
                println!("{action} failed. {detail}")
            }
            other => println!("{action} failed ({other:?})"),
        }
    }
}

fn coverage(paths: &[PathBuf]) -> Result<(), Box<dyn Error>> {
    let mut report = Coverage::default();
    let mut system: Option<String> = None;
    for path in paths {
        let saved = load(path)?;
        if system
            .as_ref()
            .is_some_and(|name| *name != saved.plan.system)
        {
            return Err("coverage requires traces of the same system".into());
        }
        system.get_or_insert(saved.plan.system);
        report.add(&saved.trace)?;
    }
    println!("Actions");
    for ((automaton, action), count) in report.actions {
        println!("{automaton}\t{action}\t{count}");
    }
    println!("Alternatives");
    for ((automaton, node, arm), count) in report.alternatives {
        println!("{automaton}\t{node}\t{arm}\t{count}");
    }
    println!("Ordered pairs");
    for ((automaton, previous, next), count) in report.pairs {
        println!("{automaton}\t{previous}\t{next}\t{count}");
    }
    Ok(())
}
