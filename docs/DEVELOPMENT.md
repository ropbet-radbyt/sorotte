# Development Guide

This guide covers the local workflow for contributors and agents working on `sorotte`.

## Workspace Layout

- `sorotte-protocol`: typed protocol models and fixture coverage
- `sorotte-core`: shared domain helpers
- `sorotte-server`: server runtime library and executable
- `sorotte-client-core`: client session/runtime logic
- `sorotte-client-app`: app-level settings, compatibility, local commands, and shared client behavior
- `sorotte-player-api`: player abstraction
- `sorotte-player-mpv`: `mpv` JSON IPC adapter
- `sorotte-cli`: headless client binary
- `sorotte-gui`: desktop client
- `sorotte-compat`: Python Syncplay interop and compatibility support
- `sorotte-sim`: deterministic simulation helpers

Use the sibling Python checkout in `../syncplay/` as the behavioral reference for compatibility work.

## Standard Checks

Run these before finishing general code changes:

For documentation-only edits, record the skipped code checks in the PR notes.

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
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 80000
```

`scripts/gui-native-smoke.ps1` performs a locked build before the watchdog
starts, requires all ten implemented scenarios by default, and preserves
structured evidence under `target/verification/gui-native-smoke/`. Its native
menu contract requires exact UIA/AccessKit IDs and structured outcomes for
detached and attached Open Media behavior. The baseline also requires 25
single-delivery physical File-menu transactions. Each physical endpoint is
bound to the target's absolute virtual-desktop coordinate in the same
`SendInput` call; menu toggles are never blindly redelivered. The baseline also
requires an exact File -> Exit lifecycle trace and process exit within four
seconds. Connectivity
scenarios must declare a typed detached or loopback mode; non-loopback TCP
targets are rejected before process launch, and scenario-owned servers remain
live until explicit teardown. Live-Python scenarios use a two-sided roster
handshake rather than UI polling alone. Any semantic wait for a peer to observe
GUI-originated compound protocol work must continue pumping the runtime owner;
an optimistic shell projection is not proof that every receipt-owned transport
frame was delivered. Live-Python fixtures must advertise the protocol features
their assertions exercise. Top-tab actions are not complete when UIA reports
success or focus alone: the expected tab content must appear. The baseline
deliberately proves exact focused-keyboard activation of Interface & System.
The native-smoke binary's unit contracts are included automatically by
`cargo test --workspace --all-features`. Failures preserve a screenshot and
credential-redacted accessibility tree when a live window remains. Pass
`-BinaryPath` only when deliberately validating a caller-supplied executable;
the wrapper records and binds that executable's path and digest.

Generate the deterministic native Settings review packet on Windows with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-visual-suite.ps1
```

The suite writes `window.png`, `semantic-tree.json`, and `manifest.json` for
`settings.first-run.player-missing`, `settings.connection.clean`,
`settings.connection.dirty`, and `settings.validation-errors` beneath
`target/gui-visual/`. Each run uses an isolated configuration fixture and the native smoke
driver's fixed 1700x1100 window bounds. The manifest records the remaining environmental
inputs: the GUI currently follows the Windows theme, DPI scale, and egui system fonts because
there are no application test overrides for those values. Use `-Scenario <id>` for a focused
capture and `-NoBuild` only when `target/debug/sorotte-gui.exe` is already current.

For agent-driven UI inspection through the egui MCP server, opt in when launching the app:

```powershell
$env:EGUI_INSPECTION = "1"
cargo run -p sorotte-gui --bin sorotte-gui
```

The inspection endpoint is disabled unless `EGUI_INSPECTION` is set and listens on egui's
loopback default (`127.0.0.1:5719`).

## GUI Release Publishing

GUI packages are built by `.github/workflows/sorotte-gui-release.yml` and staged locally by:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-gui-release.ps1 -Channel stable
```

The workflow always keeps the Actions artifact. Version tags `v*` publish stable releases in `ropbet-radbyt/sorotte`; pushes to the current `main` tip update the moving `sorotte-gui-dev` prerelease in the same repository for dev-channel GUI update checks. Publication rechecks the remote `main` tip so rerunning an older workflow cannot roll dev clients backward.

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

- Protocol, wire format, server state, room fanout, and TLS behavior: `sorotte-protocol`, `sorotte-server`, or `sorotte-compat`
- Client session, reconnect, readiness, playlist, controller, and desync behavior: `sorotte-client-core`
- CLI parsing, stored settings, `sorotte.ini`, local commands, language, and startup compatibility: `sorotte-client-app` or `sorotte-cli`
- GUI workflows and rendering state: `sorotte-gui` semantic scenarios and app tests
- Real player behavior: `sorotte-player-mpv` plus ignored/manual real-`mpv` smoke tests when local `mpv` and media fixtures are available

## Coding Rules

- Use Rust `1.97.1` and edition `2024`.
- Keep public API surfaces narrow. Add shared CLI/GUI behavior to `sorotte-client-app::app_boundary` where a cross-crate API is needed.
- Prefer small, test-backed vertical slices over broad refactors.
- Do not add non-`mpv` player backend work unless product scope is explicitly changed.
- Do not use old planning docs as source of truth; use live code, tests, and the current docs.

## Coverage

Install once:

```powershell
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.8.4 --locked
```

Generate the pinned native producer views and source-bound physical-line map:

```powershell
cargo llvm-cov --locked --workspace --all-features --no-report
cargo llvm-cov report --json --skip-functions `
  --output-path target/coverage.json
cargo llvm-cov report --text `
  --output-path target/coverage.txt
python scripts/llvm_cov_line_map.py `
  --repo-root . `
  --llvm-json target/coverage.json `
  --llvm-text target/coverage.txt `
  --output target/coverage-line-map.json
```

The LLVM component must be present before captured/headless execution:
cargo-llvm-cov otherwise prompts interactively and can look hung. Pull-request
policy is based on unique changed physical production lines, while LLVM's
aggregate line-instance summary is retained as separate diagnostic evidence;
see [`coverage/README.md`](../coverage/README.md).

## Targeted Mutation Testing

Mutation testing is intentionally shard-based. Install the pinned producer and
run the currently required privacy shard with:

```powershell
cargo install cargo-mutants --version 27.1.0 --locked
python scripts/mutation_ci.py validate `
  --repo-root . `
  --policy coverage/mutation-policy.toml `
  --shard privacy-secret
python scripts/mutation_ci.py run `
  --repo-root . `
  --policy coverage/mutation-policy.toml `
  --shard privacy-secret `
  --results-root target/mutation-ci/privacy-secret `
  --output target/verification/mutation-privacy-secret.json
```

Use a fresh results root for every local run. The wrapper rejects an existing
`mutants.out` directory so stale artifacts cannot be mistaken for new
evidence. A survivor or product defect discovered by a coverage-only branch
should be characterized and recorded; do not change production behavior just
to make the mutation shard green.
