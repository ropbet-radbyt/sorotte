# Port Maintainability Plan

Working plan for keeping the Rust port shippable while finishing client parity.

## Audit date

- 2026-03-11

## Purpose

The port is already functionally advanced, but the main client crates have accumulated enough code in single files that change risk is rising faster than feature coverage. This document turns that audit into a working plan that can be executed incrementally.

## Scope

- Keep using the upstream Python client as the behavioral oracle.
- Keep `mpv` as the active parity target.
- Improve maintainability without pausing client parity work indefinitely.
- Bias toward changes that make future agent work easier to navigate, test, and review.

## Current read

### Strengths

- Workspace tests, `clippy`, and GUI semantic coverage are green in the latest audit.
- The repo already has good crate separation at the workspace level.
- `syncplay-client-app` provides a useful shared boundary for commands, persistence, language, diagnostics, and startup/session planning.
- The GUI has meaningful semantic and Python-interop coverage rather than only unit tests.

### Main risks

- `crates/syncplay-gui/src/app.rs` is still the largest production risk concentration, but it is now down to roughly `10.0k` lines after the shell state/projection pass. The crate is now library-first and the active working split has already moved legacy GUI UI-state persistence into `app_ui_state.rs`, startup/bootstrap support into `app_startup.rs` and `app_startup_support.rs`, shared formatting/normalization helpers into `app_support.rs`, the widget-tree model and test preview renderer into `app_widget_tree.rs`, queued-runtime/native host plumbing into `app_runtime_queue.rs` and `app_native_host.rs`, configuration-draft round-tripping into `app_configuration_draft.rs`, shell/reducer projection layers into `app_reducer.rs`, `app_widget_projection.rs`, and `app_shell_projection.rs`, lower shell/runtime workflows into `app_feedback_workflows.rs`, `app_shell_workflows.rs`, `app_runtime_updates.rs`, `app_connection_workflows.rs`, `app_media_workflows.rs`, and state-integrity helpers/tests into `app_state_integrity.rs` and `app_tests.rs`. The remaining hotspot is now the egui renderer/action mapping, runtime-owner orchestration, and top-level app wiring still living in `app.rs`, and that is where future extraction work should stay focused.
- `crates/syncplay-cli/src/main.rs` still owns too much business logic even though `syncplay-client-app` exists.
- `crates/syncplay-client-core/src/lib.rs` has a good runtime/session seam, but too much behavior still lives in one file.
- `crates/syncplay-compat/src/lib.rs` is also large enough to become a maintenance problem, but it is lower priority than GUI, CLI, and client-core.
- The remaining parity backlog is now narrow enough that architecture debt is the main source of delivery risk.

## Guiding rules

1. Keep landing parity work as small, test-backed vertical slices.
2. When touching a giant file, extract the touched area in the same change if the edit increases complexity.
3. Prefer library modules over growing binaries.
4. Preserve existing test behavior during extraction; refactors should be behavior-neutral unless explicitly closing a parity gap.
5. Keep public seams stable while moving internals.
6. Avoid introducing new global state or new cross-crate duplication.

## Working order

### 1. Establish extraction rules

Outcome:

- New work stops making the large files larger by default.

Actions:

- Treat roughly `1-2k` lines as a soft module target.
- Treat roughly `3k` lines as the point where extraction is mandatory.
- Do not add new feature slices directly to the current GUI/CLI/client-core monoliths unless the same change extracts the touched area.
- Keep tests close to the module they exercise once extraction begins.

Definition of done:

- The team agrees to enforce these limits for new work.

### 2. Turn `syncplay-gui` into a real library-first crate

Why first:

- It is the biggest single-file risk.
- It is also where remaining user-visible parity work will land.

Actions:

- Move app code out of `src/main.rs` into `src/lib.rs` plus `src/app/...`.
- Leave `src/main.rs` as a thin launcher.
- Move `semantic_smoke`, `semantic_driver`, and `live_python_interop` out of the binary file and into normal library modules.
- Preserve existing semantic-smoke and live-interop entrypoints so scripts do not break.

Progress so far:

- `src/main.rs` is now a thin launcher.
- `src/lib.rs` exposes the GUI entrypoint and semantic wrappers.
- `app_ui_state.rs` now owns legacy GUI UI-state persistence, QSettings parsing/writing, and the persisted update-check/media-dialog/public-server state model.
- `app_startup.rs` now owns config-path resolution, startup action planning, and startup host wiring.
- `app_startup_support.rs` now owns startup environment lookup, client-core chat bootstrap parsing, and startup-source descriptors shared by startup planning and tests.
- `app_support.rs` now owns shared shell helper functions for optional text/number formatting, editable-text normalization, runtime timestamps, and autoplay/offset helpers.
- `app_widget_tree.rs` now owns the widget-tree model, widget renderer trait, and test preview renderer used by semantic and shell preview code.
- `app_runtime_queue.rs` now owns the queued runtime request/action bridge plus the owner pump.
- `app_native_host.rs` now owns the egui-native host/app wiring and related host-side helpers.
- `app_configuration_draft.rs` now owns the editable configuration-draft round-tripping layer, including the split between raw control values and parsed stored settings.
- `app_reducer.rs`, `app_widget_projection.rs`, and `app_shell_projection.rs` now own the action reducer plus the shell/widget projection layers that were previously inline in `app.rs`.
- `app_feedback_workflows.rs`, `app_shell_workflows.rs`, `app_runtime_updates.rs`, `app_connection_workflows.rs`, `app_media_workflows.rs`, `app_state_integrity.rs`, and `app_tests.rs` now own the lower-half workflow/update/state-integrity/test bodies that were previously in `app.rs`.
- The remaining work is to keep peeling `app.rs` apart along egui renderer/interaction mapping, runtime-owner boundaries, and the remaining top-level shell/app wiring.

Suggested module split:

- `app/state.rs`
- `app/actions.rs`
- `app/reducer.rs`
- `app/runtime_bridge.rs`
- `app/runtime_owner.rs`
- `app/render.rs`
- `app/views/configuration.rs`
- `app/views/main_window.rs`
- `app/views/public_servers.rs`
- `app/views/media_search.rs`
- `app/testing/semantic_driver.rs`
- `app/testing/semantic_smoke.rs`
- `app/testing/live_python_interop.rs`

Definition of done:

- `src/main.rs` is small and contains launch wiring only.
- `src/lib.rs` exposes the GUI app surface and test entrypoints directly.
- Existing GUI semantic and interop commands still work unchanged.

### 3. Thin down `syncplay-cli`

Why second:

- The CLI is a large binary with a lot of logic that should be library-owned.
- It already has a natural destination crate in `syncplay-client-app`.

Actions:

- Add `crates/syncplay-cli/src/lib.rs`.
- Move parsing, startup planning, persistence wiring, and network-loop orchestration out of `main.rs`.
- Prefer moving reusable behavior into `syncplay-client-app` when it is not CLI-specific.
- Leave `main.rs` responsible only for process entry, stdout/stderr behavior, and exit handling.

Definition of done:

- `syncplay-cli/src/main.rs` becomes a thin entrypoint.
- Shared startup behavior lives in library code and can be reused by tests or future frontends.

### 4. Split `syncplay-client-core` around existing seams

Why third:

- The runtime/session boundary is already present, so this refactor has a clear path.
- Many future parity slices depend on touching this crate safely.

Actions:

- Keep `ClientRuntimeAction` and `ClientRuntimeControl` as the initial stable seam.
- Split `lib.rs` into focused modules:
  - `session.rs`
  - `runtime.rs`
  - `chat.rs`
  - `playlist.rs`
  - `reconnect.rs`
  - `desync.rs`
  - `notifications.rs`
  - `types.rs`
- Move tests with the modules they primarily exercise where practical.
- Keep the public API shape stable while internal modules move.

Definition of done:

- `lib.rs` becomes a small re-export and crate-wiring layer.
- Session state and runtime orchestration are no longer maintained in one file.

### 5. Only then resume feature slices in the highest-risk GUI areas

Priority order for parity work after the first extraction round:

1. GUI slash-command handling.
2. Configuration dialog completion.
3. Runtime localization and language-sensitive service calls.
4. Explicit decision on whether non-`mpv` backends remain deferred or are re-opened.

Execution rule:

- Each feature slice must use the Python client as the reference behavior.
- Each slice must add the lowest-sensible failing test first.
- If a slice touches a still-large module, extract before or during the feature change.

### 6. Improve agent-facing development ergonomics

Goal:

- Make it easier for an agent to work in a targeted area without loading tens of thousands of lines of mixed concerns.

Actions:

- Prefer one concern per module and one major state owner per file.
- Keep adapter traits and reducer entrypoints explicit and easy to grep.
- Keep test helpers in dedicated modules instead of embedding every helper in giant `mod tests`.
- Add short module-level docs for major boundaries once extraction lands.
- Maintain a small set of stable commands for verification:
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`

### 7. Secondary cleanup after the main extractions

Lower priority, but worth scheduling after the main three crates improve:

- Split `syncplay-compat/src/lib.rs` into protocol probes, live Python harnesses, and server-runtime scenarios.
- Revisit whether some GUI-only shell state types belong in their own modules or a separate crate later.
- Add a compact compatibility matrix generated from tests when the remaining parity backlog is smaller.

## Immediate backlog

- [x] Create `syncplay-gui` library-first module structure without changing behavior.
- [x] Move GUI semantic and Python-interop helpers out of `crates/syncplay-gui/src/main.rs`.
- [ ] Continue splitting `crates/syncplay-gui/src/app.rs` into state, runtime, render, and test modules.
- [ ] Add a repeatable full GUI smoke gate that bundles semantic, native, and real-`mpv` smoke coverage into one documented validation pass.
- [ ] Add `syncplay-cli/src/lib.rs` and reduce `main.rs` to entrypoint code.
- [ ] Split `syncplay-client-core/src/lib.rs` into runtime/session-focused modules.
- [ ] Implement GUI slash-command handling using `syncplay-client-app` command planning.
- [ ] Finish missing configuration dialog controls.
- [ ] Localize runtime strings and language-sensitive service calls.
- [ ] Decide and document whether additional player backends remain deferred after `mpv`.

## Validation expectations

For extraction-only changes:

- `cargo fmt --all`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`

For GUI changes:

- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`

For Windows GUI changes that affect startup, rendering, or end-to-end flow:

- `cargo build -p syncplay-gui --bin syncplay-gui`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`

For full GUI smoke validation:

- `cargo build -p syncplay-gui --bin syncplay-gui`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`
- `cargo test -p syncplay-gui gui_persisted_config_runtime_owner_starts_real_managed_mpv_from_saved_config -- --ignored`
  Requires local `SYNCPLAY_MPV_SMOKE_BIN` and media fixture setup.

## Non-goals for this plan

- Reopening non-`mpv` player parity immediately.
- Large-scale crate reshuffling without first using the existing seams.
- Rewriting working behavior for style alone.

## Success criteria

This plan is succeeding if:

- the giant files are shrinking instead of growing,
- parity work can land in smaller slices,
- tests stay green through extractions,
- and future audits focus on behavior gaps rather than codebase navigability.
