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

Pull-request CI keeps the public `Rust all-feature behavior (Windows)` and
`coverage-diff` checks stable, but their expensive work is deliberately
parallel. Windows nextest/doctests, the release/package checks, and exact-head
Windows process coverage run as three independent workers; the public Windows
check succeeds only after all three do. Linux merged coverage starts
immediately in a separate producer, and `coverage-diff` consumes the Linux and
Windows artifacts only to resolve the immutable base, enforce the two-map
changed-line policy, and finalize evidence. Do not make either producer depend
on the public Windows aggregate or move profile generation back into
`coverage-diff`; `scripts/tests/test_ci_policy.py` treats either change as a
critical-path and fail-closed policy regression.

## Real mpv Checks

The required `mpv-pr-semantics` job builds the peeled mpv `v0.41.0` commit and
runs the four ignored real-player contracts explicitly. Scheduled and manually
dispatched CI expands that same fail-closed job to a second, immutable reviewed
post-release snapshot. The two endpoints run in parallel, and matrix fail-fast
is disabled so one failure cannot erase the other endpoint's result.

Do not replace either source SHA with a floating tag or branch. The newest
snapshot is the final reviewed mpv revision that builds against Ubuntu 24.04's
native libplacebo dependency; rolling it forward requires reviewing the mpv
and runner dependency boundary together. See
[`mpv-version-matrix-20260801.md`](evidence/test-coverage/mpv-version-matrix-20260801.md)
for the exact identities, local campaign, version-parser regression, and
exact-head minimum/newest result.

## GUI Checks

Run semantic smoke coverage for GUI workflow changes:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
```

For local Windows accessibility iteration without desktop-wide mouse, keyboard,
or cursor injection, run the fixed non-authoritative UIA-only inventory:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 80000 -InputMode UiaOnly
```

The UIA-only lane verifies the AccessKit menu inventory and invokes File -> Exit
through UI Automation. It rejects every Win32 `SendInput` or cursor-movement
fallback and reports physical/focused-keyboard capabilities as
`optional-skip(reason=local-uia-mode)`. It may still display, resize, or
foreground Sorotte. Its report has `authoritative=false` and cannot satisfy the
strict CI contract.

Run authoritative Windows native smoke coverage for rendering, accessibility,
startup, and end-to-end GUI changes only on an isolated interactive desktop:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 80000 -InputMode StrictPhysical
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

Consume the exact staged bytes, including a visible-window launch, installed
updater self-replacement, and faulted rollback:

```powershell
$sourceSha = (git rev-parse HEAD).Trim()
python scripts/verify_gui_release_artifact.py `
  --artifacts-dir target/gui-release/artifacts `
  --expected-source-sha $sourceSha `
  --expected-channel stable `
  --report target/gui-release/artifact-verification.json
```

The workflow always keeps the Actions artifact. Version tags `v*` publish stable releases in `ropbet-radbyt/sorotte`; pushes to the current `main` tip update the moving `sorotte-gui-dev` prerelease in the same repository for dev-channel GUI update checks. Publication rechecks the remote `main` tip so rerunning an older workflow cannot roll dev clients backward.
Both the build and publication jobs independently verify the downloaded
archive, checksum, external update manifest, embedded install manifest, closed
payload inventory, source SHA, and channel. Only the build job executes the
Windows binaries; publication reconsumes the same bytes without repeating the
runtime smoke.

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

The scheduled and manually dispatched CI matrix invokes the verifier with
`-NoWorkspace`. That mode skips only its duplicate `cargo test --workspace`
pass: the same workflow already runs locked, all-feature workspace tests and
doctests on Linux and Windows. The server tests, live compatibility checks,
Clippy gate, and strict server release matrix still run on both platforms.
Standalone release verification keeps the full default command above.

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
python -m pip install -r requirements/legacy-python-interop.txt
git clone https://github.com/Syncplay/syncplay.git `
  .interop-cache/syncplay-legacy
git -C .interop-cache/syncplay-legacy checkout --detach `
  d1c5f85af377c960c5a940707c4d01bc84fd9c3f
```

Generate the merged workspace, GUI semantic, and strict live-TLS profiles,
then export the pinned native producer views and source-bound physical-line
map:

```powershell
$env:SYNCPLAY_LEGACY_ROOT = `
  (Resolve-Path .interop-cache/syncplay-legacy).Path
python scripts/coverage_profile_lanes.py run `
  --repo-root . `
  --output target/verification/coverage-profile-lanes.json
python scripts/coverage_profile_lanes.py validate `
  --report target/verification/coverage-profile-lanes.json
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

Each execution lane must both pass its behavior oracle and create or update a
raw profile. The collector removes only generated raw/merged coverage inputs
below `target` before starting, attests the reset, and requires continuous
current-run profile counts. It hashes profile content, forbids a lane from
removing prior evidence, and requires the merge to leave raw inputs unchanged.
It rejects a stale semantic binary, incomplete scenario inventory, skipped
legacy prerequisite, wrong Syncplay revision, unexpected compatibility
selector/count, or unmergeable profiles. It does not claim native interactive
Windows coverage or the currently red complete legacy fanout matrix.

## Targeted Mutation Testing

Mutation testing is intentionally shard-based. Install the pinned producer
and run a scheduled shard (`privacy-secret`, `server-auth`, or
`protocol-codec`) with a fresh results root. For example:

```powershell
cargo install cargo-mutants --version 27.1.0 --locked
python scripts/mutation_ci.py validate `
  --repo-root . `
  --policy coverage/mutation-policy.toml `
  --shard protocol-codec
python scripts/mutation_ci.py run `
  --repo-root . `
  --policy coverage/mutation-policy.toml `
  --shard protocol-codec `
  --results-root target/mutation-ci/protocol-codec `
  --output target/verification/mutation-protocol-codec.json
```

Use a fresh results root for every local run. The wrapper rejects an existing
`mutants.out` directory so stale artifacts cannot be mistaken for new
evidence. Policy schema 2 binds package-wide versus library testing and any
focused Rust test namespace; the wrapper reconciles that scope against every
producer phase. A survivor or product defect discovered by a coverage-only
branch should be characterized and recorded; do not change production
behavior just to make the mutation shard green.
