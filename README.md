# syncplay-rs

Rust rewrite workspace for Syncplay.

## Current state (audited 2026-02-24)

- Headless Rust CLI client is implemented and actively test-covered.
- `syncplay-cli` integrates with `mpv` (managed launch and explicit JSON IPC attach).
- `crates/syncplay-server` contains a substantial server runtime library (room/state sync, TLS paths, persistent/permanent room behavior) backed by tests.
- The `syncplay-server` executable now has a real alpha CLI/help surface and listener startup over the Rust server runtime, but Python server CLI parity is still partial.
- GUI parity is not implemented yet; this is currently a CLI/headless project.

Audit verification run in this session:

- `cargo test --workspace` passed
- `cargo clippy --workspace --all-targets -- -D warnings` passed

Manual/local validations (real `mpv`, release packaging) are tracked separately in `PROJECT_STATUS.md` and `ALPHA_CLI_PREVIEW.md`.

Workspace-level parity and planning docs (one directory up):

- `../PROJECT_STATUS.md`
- `../PARITY_CHECKLIST.md`
- `../GUI_CONFIG_PARITY_BACKLOG.md`

## Canonical docs (keep these in this repo)

- `README.md`: repo overview and quick commands
- `PROJECT_STATUS.md`: repo-local audit summary + completed/pending checklist
- `ALPHA_CLI_PREVIEW.md`: Windows/`mpv` CLI alpha packaging and run guide

## Workspace layout

- `crates/syncplay-protocol`: typed protocol models + fixture coverage
- `crates/syncplay-core`: shared core domain types/helpers
- `crates/syncplay-server`: server runtime library + alpha executable entrypoint (partial Python CLI parity)
- `crates/syncplay-client-core`: client session/runtime logic
- `crates/syncplay-player-api`: player abstraction interface
- `crates/syncplay-player-mpv`: `mpv` JSON IPC adapter
- `crates/syncplay-cli`: headless CLI client binary
- `crates/syncplay-sim`: deterministic simulation helpers
- `crates/syncplay-compat`: compatibility/interop test support

## Quick commands

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo build --release`

## Coverage (cargo-llvm-cov)

Install once:

- `rustup component add llvm-tools-preview`
- `cargo install cargo-llvm-cov --locked`

Aliases (see `.cargo/config.toml`):

- `cargo cov-clean`
- `cargo cov-lcov`
- `cargo cov-html`

CI workflows:

- `.github/workflows/rust-ci.yml`
- `.github/workflows/rust-coverage.yml`
