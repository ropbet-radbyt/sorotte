# syncplay-rs

Rust rewrite workspace for Syncplay.

## Current state (audited 2026-03-08)

- Headless Rust CLI client is implemented, test-covered, and matches the upstream Python client startup/help surface.
- `syncplay-gui` now has a real configuration/main-window shell with semantic smoke coverage, Windows native smoke coverage, and live Python interop scenarios for room, readiness, chat, playlist, reconnect, and controlled-room flows.
- `syncplay-cli` integrates with `mpv` (managed launch and explicit JSON IPC attach); complete `mpv` parity is the active client goal, and non-`mpv` player support is deferred until that is done.
- `crates/syncplay-server` contains a substantial server runtime library (room/state sync, TLS paths, persistent/permanent room behavior) backed by tests.
- Full client parity is still in progress; the highest-priority remaining gaps are complete `mpv` parity, detached GUI runtime flows, and remaining GUI/background parity work.

Audit verification run in this session:

- `cargo test --workspace` passed
- `cargo clippy --workspace --all-targets -- -D warnings` passed
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` passed (`7/7`)
- `cargo build -p syncplay-gui --bin syncplay-gui` passed
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000` passed

Manual/local validations (real `mpv`, release packaging) are tracked separately in `PROJECT_STATUS.md` and `ALPHA_CLI_PREVIEW.md`.

## Canonical docs (keep these in this repo)

- `README.md`: repo overview and quick commands
- `PROJECT_STATUS.md`: repo-local audit summary + current priorities
- `docs/CLIENT_PARITY_AUDIT.md`: current parity audit and remaining work list
- `docs/AGENT_IMPLEMENTATION_GUIDE.md`: implementation workflow and required test matrix for agents
- `ALPHA_CLI_PREVIEW.md`: Windows/`mpv` CLI alpha packaging and run guide

Archived workspace planning/handoff docs now live one directory up in `../old-docs/`.

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

## Running tests

Run these from the `syncplay-rs` repo root.

Standard workspace test suite:

- `cargo test --workspace`

GUI semantic smoke suite (cross-platform):

- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`

GUI native smoke suite (Windows UI Automation):

- Build the GUI binary first: `cargo build -p syncplay-gui --bin syncplay-gui`
- `scripts/gui-native-smoke.ps1` launches the existing `target/debug/syncplay-gui.exe`; it does not rebuild that binary for you.
- Re-run the build step any time you change `syncplay-gui` code before running native smoke.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`

If you want the current end-to-end repo test pass used for Windows verification, run:

```powershell
cargo test --workspace
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
cargo build -p syncplay-gui --bin syncplay-gui
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000
```

## Other useful commands

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
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
