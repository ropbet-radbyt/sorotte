# syncplay-rs

Rust rewrite workspace for Syncplay.

## Current state (audited 2026-03-08)

- Headless Rust CLI client is implemented, test-covered, and matches the upstream Python client startup/help surface.
- `syncplay-gui` now has a real configuration/main-window shell with semantic smoke coverage, Windows native smoke coverage, and live Python interop scenarios for room, readiness, chat, playlist, reconnect, and controlled-room flows.
- `syncplay-cli` integrates with `mpv` (managed launch and explicit JSON IPC attach); complete `mpv` parity is the active client goal, and non-`mpv` player support is deferred until that is done.
- `crates/syncplay-server` contains a Python-compatible server runtime and binary, backed by runtime tests, binary network smoke tests, real Python client interop, and a strict server release verification gate.
- Full client parity is still in progress; the highest-priority remaining gaps are complete `mpv` parity, detached GUI runtime flows, and remaining GUI/background parity work.

Audit verification run in this session:

- `cargo test --workspace` passed
- `cargo clippy --workspace --all-targets -- -D warnings` passed
- `powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1` passed
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` passed (`7/7`)
- `cargo build -p syncplay-gui --bin syncplay-gui` passed
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000` passed

Manual/local validations (real `mpv`, release packaging) are tracked separately in `PROJECT_STATUS.md` and `ALPHA_CLI_PREVIEW.md`.

## Canonical docs (keep these in this repo)

- `README.md`: repo overview and quick commands
- `PROJECT_STATUS.md`: repo-local audit summary + current priorities
- `docs/CLIENT_PARITY_AUDIT.md`: current parity audit and remaining work list
- `docs/SERVER_RELEASE.md`: strict Rust server verification and packaging guide
- `docs/PORT_MAINTAINABILITY_PLAN.md`: working maintainability and extraction plan for the Rust port
- `docs/AGENT_IMPLEMENTATION_GUIDE.md`: implementation workflow and required test matrix for agents
- `ALPHA_CLI_PREVIEW.md`: Windows/`mpv` CLI alpha packaging and run guide

Archived workspace planning/handoff docs now live one directory up in `../old-docs/`.

## Workspace layout

- `crates/syncplay-protocol`: typed protocol models + fixture coverage
- `crates/syncplay-core`: shared core domain types/helpers
- `crates/syncplay-server`: server runtime library + Python-compatible executable entrypoint
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

Strict Rust server release verification:

- Install Python prerequisites first when the environment does not already have them: `python -m pip install twisted pyopenssl service_identity`
- `powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1`
- This bootstraps the pinned Syncplay `v1.7.5` oracle when needed, runs the normal cargo gates plus ignored release-only `syncplay-server` binary tests, and writes JSON/Markdown reports under `target/server-release-verify`.

Rust server release packaging:

- `powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1`
- See `docs/SERVER_RELEASE.md` for artifact contents, checksum output, and release CI behavior.

Rust server container image:

- `docker build -f Dockerfile.server -t syncplay-rs-server:local .`
- `docker run --rm -p 8999:8999/tcp syncplay-rs-server:local`
- Published GHCR image target: `ghcr.io/ropbet-radbyt/syncplay-rs-server:latest`
- See `docs/SERVER_RELEASE.md` for publishing, persistence, TLS, and NAS/container UI examples.

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
