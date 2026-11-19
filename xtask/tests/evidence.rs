use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

#[test]
fn generated_report_retains_every_seed_and_is_reproducible() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for folder in [&first, &second] {
        let run = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["evidence", "--seeds", "32", "--out-dir"])
            .arg(folder.path())
            .output()
            .unwrap();
        assert!(run.status.success(), "{run:?}");
    }
    for file in [
        "EVIDENCE.md",
        "evidence/seeds.csv",
        "evidence/summary.csv",
        "evidence/protocol.md",
    ] {
        assert_eq!(
            fs::read(first.path().join(file)).unwrap(),
            fs::read(second.path().join(file)).unwrap()
        );
    }

    let data = fs::read_to_string(first.path().join("evidence/seeds.csv")).unwrap();
    let mut groups = BTreeMap::new();
    for row in data.lines().skip(1) {
        let [bug, _, method, seed, started, detected, shrunk] =
            row.split(',').collect::<Vec<_>>()[..]
        else {
            panic!("malformed evidence row {row}");
        };
        let next_seed = groups.entry((bug, method)).or_insert(0);
        assert_eq!(seed.parse::<u64>().unwrap(), *next_seed);
        *next_seed += 1;
        if detected.parse::<bool>().unwrap() {
            assert!(shrunk.parse::<usize>().unwrap() <= started.parse::<usize>().unwrap());
        } else {
            assert!(shrunk.is_empty());
        }
    }
    assert_eq!(groups.len(), 30);
    assert!(groups.into_values().all(|seeds| seeds == 32));
}
