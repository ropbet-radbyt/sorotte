# Agent Implementation Guide

## Goal

Use the upstream Python client as the behavioral oracle and land parity work as small, test-backed vertical slices.

Current scope: complete `mpv` parity first. Do not treat non-`mpv` player support as active work unless the user explicitly reprioritizes it.

## Source Of Truth

When implementing a feature, start from the matching Python source:

- session/runtime behavior: `../../syncplay/syncplay/client.py`
- GUI behavior: `../../syncplay/syncplay/ui/gui.py` and `../../syncplay/syncplay/ui/GuiConfiguration.py`
- player behavior: `../../syncplay/syncplay/players/*.py`
- protocol/server interactions: `../../syncplay/syncplay/protocols.py` and `../../syncplay/syncplay/server.py`

Do not rely on old workspace trackers as the primary source of truth. Use the live Python code plus the current Rust tests.

## Implementation Rules

1. Pick one parity slice at a time.
2. Identify the exact upstream Python behavior before editing Rust.
3. Add a failing test at the lowest sensible layer before or with the implementation.
4. Keep the change as small as possible, but extract a submodule if the target file is already too large.
5. Update the audit/status docs when a parity gap is closed or deliberately scoped out.

## Where Tests Belong

### Protocol Or Session Semantics

Use:

- `crates/syncplay-protocol`
- `crates/syncplay-client-core`
- `crates/syncplay-server`

Add tests here when the change affects wire format, runtime actions, reconnect, readiness, chat, playlist state, controller logic, privacy, or desync correction.

### Config, Startup, Persistence, Or Local Commands

Use:

- `crates/syncplay-client-app`
- `crates/syncplay-cli`

Add tests here when the change affects CLI parsing, `syncplay.ini`, legacy QSettings compatibility, player startup args, language/config normalization, or slash/local commands.

### GUI Shell And User Workflow

Use:

- `crates/syncplay-gui/src/semantic_scenarios/*.txt`
- `crates/syncplay-gui/src/app/`
- `crates/syncplay-gui/src/app/semantic_smoke.rs`

Add or extend a semantic scenario when the change affects configuration editing, main-window state, pending operations, playlist projection, chat flow, reconnect flow, modal behavior, or persistence/reset behavior.

### Live Interop With Python

Use:

- `crates/syncplay-gui/src/app/live_python_interop.rs`
- `crates/syncplay-compat`

Add or extend live interop coverage when the Rust behavior must match an actual Python peer or legacy server end to end.

### Real Player Behavior

Use:

- `crates/syncplay-player-mpv`
- ignored/manual smoke tests already present in `syncplay-cli` and `syncplay-player-mpv`

For player-launch or adapter changes, add unit tests first and then extend the real-player smoke path when the environment-dependent behavior matters.

## Required Commands

Run from `syncplay-rs/`.

Always:

- `cargo fmt --all`
- `cargo test --workspace`

Before finishing any user-visible change:

- `cargo clippy --workspace --all-targets -- -D warnings`

For GUI changes:

- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`

For Windows GUI changes that touch rendering, accessibility, startup, or end-to-end user flow:

- `cargo build -p syncplay-gui --bin syncplay-gui`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`

For player integration work:

- run the relevant crate tests
- if `mpv` behavior changed, run the ignored/manual real-`mpv` smoke path when the environment is available

For Python-interop-sensitive changes:

- prefer the live Python GUI scenarios or targeted `syncplay-compat` coverage, not just local mock tests

## Test Expectations By Feature Type

### Detached GUI Connect Or Search Work

- Add a semantic scenario that succeeds from a disconnected/configuration state.
- Add or extend native smoke coverage on Windows.
- If the flow talks to the legacy server, add a live Python interop assertion.

### Startup Or Launch Parity

- Add `syncplay-client-app` or `syncplay-cli` tests for CLI/config precedence.
- Cover both stored-config and explicit-override paths.
- Cover both managed launch and attach mode if the feature applies to both.

### Session Or Reconnect Behavior

- Add `syncplay-client-core` tests first.
- Add GUI semantic or live interop coverage if the behavior is user-visible.

## Definition Of Done

A parity change is not done until:

- the Python reference behavior has been identified
- a Rust test proves the behavior
- the standard command set has been run
- any skipped manual/native/player validation is explicitly called out
- the docs are updated if the supported behavior changed
- the change is committed to git

## Current Priority Order For Agents

1. Localization and language-sensitive service-call parity
2. Remaining `mpv`-scope GUI/runtime parity gaps surfaced by the Python diff or smoke failures
3. Opportunistic extraction of large modules while making the above changes
4. GUI-only runtime-setting parity decisions where behavior still intentionally no-ops
5. Non-`mpv` player support only after `mpv` parity is no longer the primary blocker

## Deferred Work

Non-`mpv` player backends are deferred for now. If that changes later, the expected work is:

- add adapter unit tests for command translation and state updates
- add startup/config tests for path detection and argument routing
- add GUI coverage for backend selection/discovery if the GUI exposes it
- add at least one end-to-end smoke or interop validation path for the new backend if practical
