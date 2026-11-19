# Releasing 0.1.0

Use the candidate commit that passed CI. Verify the workspace expands to
seven publishable crates.

```sh
cargo package --workspace --exclude xtask --locked
cargo publish --workspace --exclude xtask --locked --dry-run
```

The workspace command resolves packages in dependency order. The
individual order is core, engine, exec, trace, macros, examples, then CLI.
The CLI package installs the `actgram` binary. The evidence and fuzz packages
stay in the repository.

Crates.io publication requires an account with permission to create the
names. Recheck availability immediately before publishing. Crates.io
treats an uploaded version as permanent. After publication, install the CLI
from the registry with `cargo install actgram-cli --version 0.1.0 --locked`
and verify the quickstart and the replay example from the release checkout.
