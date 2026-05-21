# Development Guide

This guide covers the local workflow for contributors and agents working on `syncplay-rs`.

## Workspace Layout

- `syncplay-protocol`: typed protocol models and fixture coverage
- `syncplay-core`: shared domain helpers
- `syncplay-server`: server runtime library and executable
- `syncplay-client-core`: client session/runtime logic
- `syncplay-client-app`: app-level settings, compatibility, local commands, and shared client behavior
- `syncplay-player-api`: player abstraction
- `syncplay-player-mpv`: `mpv` JSON IPC adapter
- `syncplay-cli`: headless client binary
- `syncplay-gui`: desktop client
- `syncplay-compat`: Python Syncplay interop and compatibility support
- `syncplay-sim`: deterministic simulation helpers

Use the sibling Python checkout in `../syncplay/` as the behavioral reference for compatibility work.

## Standard Checks

Run these before finishing general code changes:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

For all-features checks:

```powershell
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## GUI Checks

Run semantic smoke coverage for GUI workflow changes:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
```

Run Windows native smoke coverage for rendering, accessibility, startup, and end-to-end GUI changes:

```powershell
cargo build -p syncplay-gui --bin syncplay-gui
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000
```

`scripts/gui-native-smoke.ps1` uses the existing `target/debug/syncplay-gui.exe`, so rebuild first after GUI code changes.

## GUI Release Publishing

GUI packages are built by `.github/workflows/gui-release.yml` and staged locally by:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-gui-release.ps1 -Channel stable
```

The workflow always keeps the private Actions artifact for maintainers. On push events, it also publishes the package, checksum, and `syncplay-update-manifest.json` to the public `ropbet-radbyt/syncplay-rs-downloads` release repository when the private source repository has a `SYNCPLAY_DOWNLOADS_TOKEN` secret with contents write access to that public repository. Version tags `v*` publish stable releases; branch pushes update the `syncplay-gui-dev` prerelease used by dev-channel GUI update checks.

## Server Release Checks

Install Python prerequisites:

```powershell
python -m pip install -r requirements/legacy-python-interop.txt
```

Run release verification:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1
```

Package release artifacts:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
```

The verification script writes JSON and Markdown reports under `target/server-release-verify/`.

## Compatibility Workflow

When matching Python Syncplay behavior:

1. Identify the matching Python source first.
2. Add or update tests at the lowest sensible layer.
3. Implement the Rust behavior in a focused slice.
4. Run the relevant standard, GUI, player, or server checks.
5. Update docs when supported user behavior changes.

Useful Python reference files:

- `../syncplay/syncplay/client.py`
- `../syncplay/syncplay/ui/gui.py`
- `../syncplay/syncplay/ui/GuiConfiguration.py`
- `../syncplay/syncplay/players/*.py`
- `../syncplay/syncplay/protocols.py`
- `../syncplay/syncplay/server.py`

## Test Placement

- Protocol, wire format, server state, room fanout, and TLS behavior: `syncplay-protocol`, `syncplay-server`, or `syncplay-compat`
- Client session, reconnect, readiness, playlist, controller, and desync behavior: `syncplay-client-core`
- CLI parsing, stored settings, `syncplay.ini`, local commands, language, and startup compatibility: `syncplay-client-app` or `syncplay-cli`
- GUI workflows and rendering state: `syncplay-gui` semantic scenarios and app tests
- Real player behavior: `syncplay-player-mpv` plus ignored/manual real-`mpv` smoke tests when local `mpv` and media fixtures are available

## Coding Rules

- Use Rust `1.95.0` and edition `2024`.
- Keep public API surfaces narrow. Add shared CLI/GUI behavior to `syncplay-client-app::app_boundary` where a cross-crate API is needed.
- Prefer small, test-backed vertical slices over broad refactors.
- Do not add non-`mpv` player backend work unless product scope is explicitly changed.
- Do not use old planning docs as source of truth; use live code, tests, and the current docs.

## Coverage

Install once:

```powershell
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked
```

Generate LCOV:

```powershell
cargo llvm-cov --workspace --lcov --output-path target/lcov.info
```
