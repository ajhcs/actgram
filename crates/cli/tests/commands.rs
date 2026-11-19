use std::process::{Command, Output};

#[test]
fn run_replay_shrink_and_coverage_work_on_the_saved_trace() {
    let folder = tempfile::tempdir().unwrap();
    let invoke = |args: &[&str]| -> Output {
        Command::new(env!("CARGO_BIN_EXE_actgram"))
            .current_dir(folder.path())
            .args(args)
            .output()
            .unwrap()
    };
    let initial = invoke(&["run", "long", "--seed", "17", "--out", "failure.json"]);
    assert_eq!(initial.status.code(), Some(1));
    assert!(String::from_utf8(initial.stdout)
        .unwrap()
        .contains("actions=500"));
    let replay = invoke(&["replay", "failure.json"]);
    assert!(replay.status.success());

    let shrink = invoke(&["shrink", "failure.json", "--out", "small.json"]);
    assert!(shrink.status.success(), "{shrink:?}");
    let output = String::from_utf8(shrink.stdout).unwrap();
    assert!(output.contains("500 → 2 actions"));
    assert!(output.contains("arm → check"));
    assert!(invoke(&["replay", "small.json"]).status.success());
    assert_eq!(
        invoke(&["shrink", "failure.json", "--out", "small.json"])
            .status
            .code(),
        Some(2)
    );

    let coverage = invoke(&["coverage", "small.json"]);
    assert!(coverage.status.success());
    assert!(String::from_utf8(coverage.stdout)
        .unwrap()
        .contains("0\tarm\tcheck\t1"));
}

#[test]
fn next_reports_an_unreachable_first_action() {
    let result = Command::new(env!("CARGO_BIN_EXE_actgram"))
        .args(["run", "long", "--next", "check"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .contains("unreachable"));
}
