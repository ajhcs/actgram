# Contributing

Use the pinned toolchain in rust-toolchain.toml. The minimum supported
version is Rust 1.88.0.

```sh
cargo fmt --all --check
cargo fmt --manifest-path fuzz/Cargo.toml --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo doc --locked --workspace --all-features --no-deps
cargo deny check
```

A traversal defect report is easiest to use with a grammar, strategy, seed
and action trace. Keep the smallest case that still reproduces the failure.
Changes to branch order, scheduler order, trace encoding or the RNG can
affect saved traces. Explain the expected compatibility impact when
changing any of them.

The executor tests can run under Miri. This is the nightly used by CI.

```sh
rustup toolchain install nightly-2026-11-18 --profile minimal --component miri --component rust-src
cargo +nightly-2026-11-18 miri test --locked -p actgram-exec
```

Run the builder fuzz target and the line coverage check with cargo-fuzz
0.13.2 and cargo-llvm-cov 0.9.1. Coverage uses the llvm-tools-preview
component on the pinned stable toolchain.

```sh
cargo +nightly-2026-11-18 fuzz run builder -- -seed=17 -runs=10000 -max_len=320
cargo llvm-cov --locked --workspace --all-features --fail-under-lines 80
cargo mutants --file crates/core/src/analysis.rs --file crates/engine/src/tape.rs
```

Run `cargo xtask evidence` to regenerate the report and its CSVs.
Describe revisions to the workload or target assignment before measuring
their effect. [Evidence protocol](evidence/protocol.md).
