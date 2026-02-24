# syncplay-rs

Rust rewrite workspace for Syncplay.

## Current status

- Headless client/server runtime is substantially implemented and passing workspace checks.
- `syncplay-cli` now supports real `mpv` integration via:
  - managed `mpv` launch (auto-spawn + JSON IPC attach)
  - explicit JSON IPC attach to an existing `mpv`
  - best-effort unmanaged external-player startup compatibility path
- Launch-hardening gates are currently green in this environment (`clippy`, workspace tests, real-`mpv` smoke matrix, and a release build spot-check).

Implemented:
- workspace crate layout and baseline CI/tooling
- typed protocol models for all top-level message families
- protocol fixtures for `Hello`, `Set`, `List`, `State`, `Chat`, `Error`, `TLS`
- typed decode integration in `syncplay-client-core` and fixture decoding in `syncplay-compat`

CLI alpha packaging / run instructions (Windows / `mpv`):
- `ALPHA_CLI_PREVIEW.md`

Detailed continuity checkpoint:
- `../NEXT_AGENT_HANDOFF.md`

## Quick commands

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo build --release`

## Coverage (cargo-llvm-cov)

`cargo-llvm-cov` is the most practical coverage tool for this Rust workspace (LLVM source-based coverage, workspace-aware, CI-friendly).

Install once:

- `rustup component add llvm-tools-preview`
- `cargo install cargo-llvm-cov --locked`

Local coverage commands (via cargo aliases in `.cargo/config.toml`):

- `cargo cov-clean` (clean prior coverage artifacts)
- `cargo cov-lcov` (writes `target/lcov.info`; accepts extra filters like `-p syncplay-client-core --lib`)
- `cargo cov-html` (writes HTML report to `target/llvm-cov/html/`; accepts extra filters)

Examples:

- `cargo cov-lcov -p syncplay-client-core --lib`
- `cargo llvm-cov test -p syncplay-client-core some_test_name -- --nocapture` (test-name filtering requires the `test` subcommand)

CI:

- Manual/scheduled coverage workflow: `.github/workflows/rust-coverage.yml`
