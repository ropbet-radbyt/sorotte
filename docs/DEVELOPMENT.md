# Development Guide

This guide covers the local workflow for contributors and agents working on `sorotte`.

Use the [current architecture and verification index](CURRENT_ARCHITECTURE.md)
for crate responsibilities, owned boundaries, normative contracts, and proof
entrypoints. Historical audits and release ledgers describe their recorded
snapshots; they do not replace the current source or attest later candidates.
The [0.2.9 implementation ledger](audits/v0.2.9-implementation.md) records that
release's audit closure. Use the [testing process](TESTING_PROCESS.md) for the
supported preflight, command ladder, required checks, test execution accounting and
evidence reuse. Its [implementation ledger](audits/testing-process-implementation-2026-09-06.md)
separates executed validation from pending hosted or native acceptance.

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
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

For all-features checks:

```powershell
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
```

For the ordinary default-feature CI obligation on a clean checkout, use the
bounded evidence writer and a fresh output directory:

```powershell
python scripts/verify.py run --lane workspace-default --output target/verification/workspace-default-attempt-1 --deadline-seconds 1200
```

Both Linux and Windows PR test workers require this Cargo harness run,
all-feature nextest, and all-feature doctests. Cargo's concurrent tests within a
binary exercise a different execution mode from nextest's process isolation;
default features also select different code. Keep both modes and do not add
retries or a `RUST_TEST_THREADS` override to make the default obligation pass.
The wrapper uses the exact PR head, which must contain its current main base,
requires unchanged clean inputs, streams bounded diagnostics, and retains failed
attempts and owned cleanup. Its 1,200-second phase limit and the 55/45-minute
Linux/Windows job limits are ceilings; additional runtime must be measured from
the resulting receipts rather than inferred from these budgets.

Rust crates are internal workspace components. Change their APIs and all workspace
callers together; do not retain aliases, no-op methods, or alternate types solely
for hypothetical downstream Rust consumers. Syncplay protocol interoperability
and settings import remain supported compatibility contracts and retain their
behavioral tests.

Use [the terminology glossary](../CONTEXT.md) and the
[compatibility cleanup audit](audits/compatibility-terminology-cleanup-2026-09-11.md)
to distinguish supported Syncplay behavior, missing peer capabilities, and
persisted formats. Name the actual feature or format instead of calling it `legacy`.

Pull-request CI keeps the public `Rust all-feature behavior (Windows)` and
`coverage-diff` checks stable, but their expensive work is deliberately
parallel. Windows default Cargo tests plus nextest/doctests, the release/package checks, and exact-head
Windows process coverage run as three independent workers; the public Windows
check succeeds only after all three do. Linux merged coverage starts
immediately in a separate producer, and `coverage-diff` consumes the Linux and
Windows artifacts only to resolve the immutable base, enforce the two-map
changed-line policy, and finalize evidence. Do not make either producer depend
on the public Windows aggregate or move profile generation back into
`coverage-diff`; `scripts/tests/test_ci_policy.py` treats either change as a
critical-path and fail-closed policy regression.

The retained timing checkpoints are intentionally observational, not an SLA.
Full-matrix run `30674012574` spent 33m30 executing before the split; the first
complete parallel run, `30677728038`, finished in 19m33. Exact commands,
worker timings, the retained failed attempt, and artifact identities are in
[`hosted-ci-closure-20260801.md`](evidence/test-coverage/hosted-ci-closure-20260801.md).

Repository-owned workflows pin first-party JavaScript actions by full commit
SHA to Node 24 majors: `actions/checkout` v7, `actions/setup-python` v7,
`actions/upload-artifact` v7, and `actions/download-artifact` v8. Upgrade the
reviewed major and immutable SHA together, then run the workflow-policy suites
and actionlint. Do not suppress runtime-deprecation annotations in place of an
action upgrade.

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

The suite writes `window.png`, `semantic-tree.json`, and `manifest.json` beneath
`target/gui-visual/` using isolated local fixtures. Theme and window size are
explicit. Each schema-3 capture measures native Windows DPI and records the
separate application zoom input. Fonts remain an environmental/build input.
Use `-Scenario <id>`, `-Theme light|dark`, `-UiScale 1.5`, and
`-ExpectedNativeDpi 144` for focused captures; `-NoBuild` requires a current
binary. The [display matrix](GUI_DISPLAY_MATRIX.md) defines native DPI profiles,
long-content scroll/focus and modal/error checks, artifact identities, visual
review, and the distinction from screen-reader evidence.

For agent-driven UI inspection through the egui MCP server, opt in when launching the app:

```powershell
$env:EGUI_INSPECTION = "1"
cargo run -p sorotte-gui --bin sorotte-gui
```

The inspection endpoint is disabled unless `EGUI_INSPECTION` is set and listens on egui's
loopback default (`127.0.0.1:5719`).

## GUI Release Publishing

GUI packages are qualified before merge by `.github/workflows/qualify-gui-archive.yml`
under the coordinated candidate campaign, and staged locally by:

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

The workflow keeps the immutable Actions artifact. Version tags `v*` publish the
original qualified bytes in `ropbet-radbyt/sorotte`, with fresh protected-main and
PR provenance checks. Application and archive runtime checks finish before
merge; publication verifies the sealed file identities and anonymous public
bytes. See [release qualification](RELEASE_QUALIFICATION.md).

Main pushes no longer build or automatically update `sorotte-gui-dev`. The existing
moving prerelease remains untouched; a future development-channel publisher must
also promote a candidate qualified before merge. Local dev-channel packaging
continues to accept `-Channel dev`.

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
Standalone release verification keeps the full default command above. The
first clean parallel run exposed the Windows verifier as the 19m28 critical
path; the successful exact-head run after this deduplication took 10m49. Treat
those values as retained observations rather than timeout targets.

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

GUI filesystem fixtures can use `app::testing::support::test_temp_dir(label)`.
Keep its `TempDir` owner alive until files and workers are closed; it allocates a
separate directory for each call and removes it on drop, including assertion
unwinding. Put fixture files underneath `root.path()`. Avoid timestamp-derived
paths or deleting a candidate path before allocation. Existing `test_temp_root`
callers retain their explicit cleanup responsibility.

Every ignored Rust test is registered in `coverage/ignored-tests.toml`.
Subprocess helpers use the `subprocess-fixture` tier, which binds a nonignored
parent test in the same source file. Required CI invokes that exact parent with
nextest and `--no-tests fail`; the helper's ignore marker keeps it from running
without the parent's role variables, temporary paths, and cleanup ownership.

- Protocol, wire format, server state, room fanout, and TLS behavior: `sorotte-protocol`, `sorotte-server`, or `sorotte-compat`
- Client session, reconnect, readiness, playlist, controller, and desync behavior: `sorotte-client-core`
- CLI parsing, stored settings, `sorotte.ini`, local commands, language, and startup compatibility: `sorotte-client-app` or `sorotte-cli`
- GUI workflows and rendering state: `sorotte-gui` semantic scenarios and app tests
- Real player behavior: `sorotte-player-mpv` plus ignored/manual real-`mpv` smoke tests when local `mpv` and media fixtures are available

Participant-status and adjacent transport changes must also satisfy
[`PARTICIPANT_STATUS_INVARIANTS.md`](PARTICIPANT_STATUS_INVARIANTS.md). Its
matrix is intentionally cross-layer: a unit test for the edited function does
not replace the A -> server -> B acceptance path, causal write-receipt test, or
advisory non-interference assertion.

The shared vocabulary and decision history live in [`../CONTEXT.md`](../CONTEXT.md),
[`ADR 0001`](adr/0001-advisory-participant-status.md), and
[`ADR 0002`](adr/0002-delivery-fenced-player-effects.md). Update those documents
when a change alters status authority, clock meaning, or the definition of a
causally delivered player effect.

## Coding Rules

- Use Rust `1.98.1` and edition `2024`.
- Keep public API surfaces narrow. Add shared CLI/GUI behavior to `sorotte-client-app::app_boundary` where a cross-crate API is needed.
- Prefer small, test-backed vertical slices over broad refactors.
- Do not add non-`mpv` player backend work unless product scope is explicitly changed.
- Do not use old planning docs as source of truth; use live code, tests, and the current docs.

## Coverage

Install once:

```powershell
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.9.1 --locked
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
and run a scheduled shard (for example `privacy-secret`, `server-auth`,
`protocol-codec`, `participant-status-protocol`, `client-participant-status`,
`client-participant-status-runtime`, `client-participant-status-outbox`,
`server-participant-status`, `gui-participant-status`,
`gui-playlist-delivery-fence`, `player-mpv-explicit-ipc-retry`,
`client-app-participant-status-lifecycle`, or
`cli-participant-status-lifecycle`) with a fresh results root. For example:

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
python scripts/mutation_ci.py verify-report `
  --repo-root . `
  --policy coverage/mutation-policy.toml `
  --shard protocol-codec `
  --report target/verification/mutation-protocol-codec.json
```

Use a fresh results root for every local run. The wrapper rejects an existing
`mutants.out` directory so stale artifacts cannot be mistaken for new
evidence. Before handoff, verify every retained report against the final source
tree; mutated-source and workspace test-input hashes are checked both when the
campaign runs and when the report is consumed. `verify-report` also reruns the
exact `cargo test --list --format terse` command and rejects a changed test
inventory. For participant-status handoff, keep only the reports selected by
`coverage/mutation-report-set.json` and run:

```powershell
python scripts/mutation_ci.py verify-report-set `
  --repo-root . `
  --policy coverage/mutation-policy.toml `
  --manifest coverage/mutation-report-set.json
```

The aggregate verifier requires exactly one manifest-selected current passing
report for every listed shard, so historical failed or stale attempts cannot be
mistaken for release evidence. Policy schema 3 binds package-wide versus library testing, any
focused Rust test selector prefix, and an optional source-bound mutant-name
regular expression; the wrapper reconciles that scope against every producer
phase and rejects inventory outside the declared expression. A survivor or
product defect discovered by a coverage-only branch should be characterized
and recorded; do not change production behavior just to make the mutation
shard green.
