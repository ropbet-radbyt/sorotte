# Port Maintainability Plan

Working plan for keeping the Rust port shippable while finishing client parity.

## Independent evaluation

- Audit date: 2026-03-19
- Last updated: 2026-03-19
- Verification performed for this reassessment:
  - `cargo test -p syncplay-gui` (`270` passed, `1` ignored local `mpv` smoke)
  - `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`
    (`11/11` scenarios)
  - Static comparison of `../syncplay/syncplay/ui/gui.py` and
    `../syncplay/syncplay/ui/GuiConfiguration.py`
  - Spot checks of Rust GUI renderer, persistence, runtime-owner, and live-Python interop paths

## Verdict

The maintainability round for the GUI app layer can stay closed. The useful work has already
happened:

- `syncplay-gui` is library-first,
- `src/main.rs` is thin,
- `app_tests.rs` is gone,
- area-owned GUI test modules now exist,
- `src/app/mod.rs` owns the GUI router instead of a flat `app.rs` path table,
- `src/semantic_smoke/` owns parser, catalog, CLI, and code-driven smoke submodules,
- and the remaining large GUI files are tooling or local hotspots rather than crate-global dumping
  grounds.

With non-`mpv` player backends explicitly deferred, the Rust GUI is no longer a maintainability
problem, but it is not yet fully at the end of the valuable Python-GUI delta work. The current
tree has test-backed coverage for the configuration surface, runtime-backed main-window
playback/chat/room flows, shared playlist workflows, public-server browsing, missing-media search,
controlled-room/controller-auth flows, localized update/public-server service calls, reconnect
behavior, persistence, drag-and-drop ingest, and live Python interop.

The remaining GUI delta now looks like a short polish tail:

- the About, Help, and TLS certificate flows exist but are still simpler than the Python desktop
  dialogs,
- the configuration surface still treats `Player Path` as text entry instead of Python-style
  browse/icon discovery UX,
- and legacy main-window size/position persistence is not ported.

That means app-tree maintainability churn should stay closed, and the GUI can be treated as
effectively done for the `mpv`-first scope unless the team decides the optional desktop polish
matters for release. Do not reopen broad internal refactors; keep any remaining work narrow and
behavior-led.

## Scope

- Keep using the upstream Python client as the behavioral oracle for GUI behavior.
- Keep `mpv` as the active parity target.
- Explicitly defer additional player backends; do not let them block the GUI assessment in this
  plan.
- Treat the remaining non-player GUI delta as optional polish unless it clearly improves release
  readiness.
- Prefer changes that keep review, testing, and agent-driven work targeted.

## Python GUI delta checkpoint

### Covered in the Rust GUI today

- Configuration surface parity includes player/startup toggles, room/trusted-domain/media-
  directory editing, chat appearance settings, OSD timing controls, save/reload/reset, and
  connect-from-saved-config flows.
- Main-window parity includes room/user/file browser state, connect/disconnect/reconnect flows,
  play/pause/seek/undo/offset controls, readiness/autoplay, chat, hide-empty-room behavior, and
  persisted playback/autoplay visibility.
- Shared-playlist parity includes add/open URL, add files, load/save playlist dialogs, text
  editing, shuffle remaining/entire, undo, drag-and-drop ingest, open-selected,
  open-containing-folder, and trust-domain actions.
- Runtime/service parity includes public-server browse/refresh/connect, missing-media search,
  localized update checks, TLS prompt handling, controlled-room/controller-auth flows,
  reconnect/state-restore behavior, and live-Python interop coverage.
- Startup lifecycle parity now includes the Python-style configuration-confirm handoff for the
  `mpv`-first scope: configuration-surface `Connect` persists the draft, syncs the managed-player
  startup path, joins the configured room, and switches into the main window through the existing
  detached connect runtime.

### Remaining GUI-only delta worth doing only if asked

- Rich About/help/TLS dialog fidelity: Python exposes version/license/dependencies and certificate
  metadata; Rust currently exposes simpler modal flows.
- Player-path browse UX: Python offers browse/autodetect/icon feedback for player selection; Rust
  currently keeps `Player Path` and `Player Arguments` as editable controls without that richer
  discovery layer.
- Main-window desktop persistence: Python persists window size and position; Rust currently
  persists view/toggle/cache state but not geometry.

## Current measured hotspots

GUI hotspots were remeasured on 2026-03-14. This pass now includes `src/bin/` so the native smoke
tooling binary is visible in the figures. Non-GUI crate figures below are carried forward from the
2026-03-13 audit and were not remeasured in this pass.

### `syncplay-gui` library and app production

- `src/app_shell_state.rs` - 1153 lines
- `src/live_python_interop.rs` - 1000 lines
- `src/app_shell_projection.rs` - 909 lines
- `src/app_runtime_stack.rs` - 901 lines
- `src/app_widget_views/main_window.rs` - 900 lines
- `src/semantic_driver.rs` - 866 lines

### `syncplay-gui` binaries and tooling

- `src/bin/syncplay-gui-native-smoke/platform_driver.rs` - 1598 lines
- `src/bin/syncplay-gui-native-smoke.rs` - 1161 lines
- `src/bin/syncplay-gui-native-smoke/native_smoke_runner.rs` - 930 lines
- `src/bin/syncplay-gui-native-smoke/native_smoke_runner/baseline_contract.rs` - 800 lines
- `src/bin/syncplay-gui-semantic-suite.rs` - 227 lines
- `src/bin/syncplay-gui-semantic-smoke.rs` - 30 lines

### `syncplay-gui` tests and smoke

- `src/app_runtime_owner/tests/transport_tests.rs` - 919 lines
- `src/app_shell_state/tests/runtime_snapshot_tests/configuration_runtime_tests.rs` - 851 lines
- `src/app_render_egui/tests.rs` - 817 lines
- `src/app_runtime_owner/tests/connection_runtime_tests.rs` - 810 lines
- `src/app_startup/tests.rs` - 760 lines
- `src/app_shell_state/tests/command_tests.rs` - 728 lines
- `src/app_shell_state/tests/runtime_snapshot_tests/menu_runtime_tests.rs` - 684 lines

### `syncplay-cli` from the previous audit

- `src/main.rs` - 25872 lines

### `syncplay-client-core` from the previous audit

- `src/lib.rs` - 15115 lines

### `syncplay-compat` from the previous audit

- `src/lib.rs` - 10172 lines

## Current read

### Strengths

- Workspace-level crate boundaries are already useful.
- `syncplay-client-app` provides a real shared seam for commands, persistence, language, and
  startup/session planning.
- `syncplay-gui` is already library-first and keeps its launcher surface stable.
- The crate-global GUI test hotspot has been broken apart into area-owned test modules.
- `src/app/mod.rs` now acts as the real GUI module root instead of a flat router file.
- `src/app_widget_views/` and `src/app_runtime_stack/` now use area-owned production submodules
  rather than monolithic top-level files.
- `src/app_shell_state/` now owns browser/playlist helpers plus the extracted configuration and
  main-window state models, leaving `app_shell_state.rs` below the soft cap.
- `src/semantic_smoke/` now separates external-script parsing, scenario cataloging, CLI handling,
  and code-driven smoke flows while keeping the public semantic smoke entrypoints stable.
- `src/bin/syncplay-gui-native-smoke/` now separates CLI/process orchestration, platform-driver
  integration, a shared runner root, and scenario-owned contract modules instead of concentrating
  all tooling behavior in one file.
- `src/app/testing/support.rs` now provides a shared fixture and harness seam for GUI tests.
- The GUI has semantic and Python-interop coverage rather than only narrow unit tests.
- The 2026-03-19 reassessment passed `cargo test -p syncplay-gui` and the semantic suite without
  finding a new `mpv`-scope blocker.
- The repository already has stable smoke commands for semantic and native GUI validation.

### Main risks

- The remaining GUI-local maintainability hotspot is now
  `src/bin/syncplay-gui-native-smoke/platform_driver.rs`, which is the only native-smoke
  production file still above the soft cap.
- `src/bin/syncplay-gui-native-smoke.rs` still sits slightly above the smoke-harness target
  because it owns shared launch, config-seeding, and mock-server helpers.
- `live_python_interop.rs` still sits at the interop split range and should not absorb more
  unrelated behavior.
- `app_runtime_owner/tests/transport_tests.rs`, `app_shell_state/tests/runtime_snapshot_tests/configuration_runtime_tests.rs`,
  and `app_render_egui/tests.rs` now contain the next local test files to watch.
- The remaining GUI delta against Python is now mostly optional desktop polish: richer
  About/help/TLS dialogs, player-path browse UX, and window geometry persistence.
- `syncplay-cli/src/main.rs` and `syncplay-client-core/src/lib.rs` remain large enough to dominate
  change risk outside the GUI.
- The plan should now describe measured state and decisions, not continue growing as a progress log.

## Guiding rules

1. Keep landing parity work as small, test-backed vertical slices.
2. When touching a large module, extract the touched concern in the same change if complexity would
   otherwise increase.
3. Prefer real module trees over flat file fan-out once an area has many related source files.
4. Preserve behavior during extractions unless a change explicitly closes a parity gap.
5. Keep public seams stable while moving internals.
6. Avoid widening visibility only to make tests easier.
7. Keep scripts and smoke entrypoints stable while their implementation moves underneath them.
8. Do not do layout-only churn; a file move is only justified when it also reduces a real hotspot
   or clarifies ownership.

## Size policy

These are working thresholds, not style rules:

- Production modules: target roughly 400-900 lines.
- Production modules: treat roughly 1200 lines as a soft cap.
- Production modules: require extraction before or during new feature work once a touched module is
  above roughly 1500 lines.
- Test modules: target roughly 150-400 lines.
- Test modules: split once a test module crosses roughly 600-800 lines or mixes multiple unrelated
  concerns.
- Smoke and interop harnesses: split once a file crosses roughly 800-1200 lines or mixes unrelated
  flows that could be reviewed independently.
- Binary entrypoints should stay thin and focused on process entry, argument handling, and exit
  behavior.

## GUI architecture decision

The library-first step is complete, the crate-global GUI test breakup is complete, the flat router
is retired behind `src/app/mod.rs`, and `semantic_smoke.rs` is now split into area-owned
submodules. The next GUI maintainability work, if any, should target standalone tooling binaries
and shared harness helpers rather than more churn inside the app tree.

### Direction

- Keep `src/main.rs` as a thin launcher.
- Keep `src/lib.rs` as the crate entry surface.
- Keep `src/app/mod.rs` as the real GUI module root and continue using area-owned submodules when
  touched concerns need extraction.
- Keep `src/semantic_smoke/` as an area-owned harness module tree rather than letting the root file
  regrow.
- Keep moving dependencies toward explicit area imports rather than parent-wide namespace imports.
- If native smoke tooling keeps growing, move shared launch, config, and mock-server helpers out of
  `src/bin/syncplay-gui-native-smoke.rs` and split `platform_driver.rs` before expanding the
  Windows/UIA driver further.

### Suggested target layout

```text
crates/syncplay-gui/src/
  lib.rs
  main.rs
  app/
    mod.rs
    state/
      mod.rs
      configuration.rs
      selection.rs
      runtime_snapshots.rs
    runtime/
      mod.rs
      owner.rs
      stack.rs
      detached.rs
      projection.rs
      requests.rs
      player.rs
      transport.rs
    views/
      mod.rs
      configuration.rs
      main_window.rs
      public_servers.rs
      media_search.rs
      widget_projection.rs
    workflows/
      mod.rs
      configuration.rs
      playlist.rs
      main_window.rs
      connection.rs
      feedback.rs
    render/
      mod.rs
      egui.rs
      actions.rs
      io.rs
    testing/
      mod.rs
      support.rs
      semantic_driver.rs
      semantic_smoke.rs
      live_python_interop.rs
  semantic_scenarios/
    *.txt
```

This layout is not required as one giant move. It should be reached incrementally by moving the
largest remaining hotspots first.

## Test organization decision

### Decision

For this repository, prefer separate source-owned unit-test modules and folders over large inline
test blocks or one crate-global GUI test file. The current tree validates that this is the right
direction: `app_tests.rs` is gone and the remaining test problems are now local hotspot files that
can be split without widening visibility.

In practice, that means:

- do not keep growing `app_tests.rs`,
- do not move most GUI tests into crate-root `tests/`,
- and do not default to long inline `mod tests` blocks in production files.

### Why this is the right Rust choice here

- Most GUI behavior depends on crate-private state and internal reducers/runtime owners.
- Top-level integration tests in `tests/` only see the public API and would force unnecessary
  visibility widening.
- Small colocated unit-test modules preserve private access while keeping ownership local.
- The current problem is navigability and ownership; a large inline test block would recreate the
  same problem inside each production file.

### Prescribed test layout

Use sibling unit-test modules or test folders owned by the module they exercise.

Examples:

```text
src/app/runtime/stack.rs
src/app/runtime/stack/tests.rs

src/app/views/configuration.rs
src/app/views/configuration/tests.rs

src/app/workflows/playlist.rs
src/app/workflows/playlist/tests.rs
```

The production file should declare:

```rust
#[cfg(test)]
mod tests;
```

### Rules

- Keep tiny, obviously local tests inline only when they stay very small and directly explain one
  helper or parser.
- Put most GUI unit tests in sibling `tests.rs` files or `tests/` folders under the owning module.
- Reserve crate-root `tests/` for true integration tests:
  public API behavior, CLI behavior, binary entrypoints, and cross-crate end-to-end flows.
- Keep reusable fixtures and harness helpers in `src/app/testing/support.rs` or
  `src/app/testing/support/`, not in a giant general-purpose test file.
- Keep semantic scenario data in `src/semantic_scenarios/` as external text fixtures.
- Do not make internal types `pub` only to satisfy tests.

### Immediate action for the GUI crate

No new GUI-crate maintainability action is required today. Keep the current split module/test
layout stable and only split local hotspots when real feature work expands them again.

If more GUI work is opened intentionally, prefer one of these small, explicit targets:

- optional polish parity: richer About/help/TLS dialogs,
- optional polish parity: player-path browse/autodetect/icon UX,
- conditional maintainability: split `platform_driver.rs` only if more Windows/UIA logic lands
  there,
- and continued subdivision inside `app_shell_state/tests/` only when a local file crosses the
  test-size threshold.

## Working order

### 1. Keep the native GUI smoke tooling bounded

Why first:

- The app/library side of the GUI is now in a defensible state.
- The native smoke runner is now scenario-owned and no longer the main GUI-local review hazard.
- The only remaining GUI-local extraction targets are the Windows/UIA driver and, secondarily, the
  shared helper layer in the native-smoke root binary.

Actions:

- Keep `src/bin/syncplay-gui-native-smoke.rs` focused on process entry plus shared launch/config
  helpers; do not let scenario logic drift back into it.
- Keep `src/bin/syncplay-gui-native-smoke/native_smoke_runner.rs` as the orchestration/helper
  layer and add new scenarios in owned submodules.
- Split `src/bin/syncplay-gui-native-smoke/platform_driver.rs` further only if more UIA/Windows
  driver behavior lands there.
- Revisit the native-smoke root helper layer only if more launch/config/mock-server behavior lands
  there.
- Revisit `live_python_interop.rs` only if more behavior lands there.
- Keep `app/testing/support.rs` as the shared support seam.
- Keep semantic, native, and real-`mpv` smoke entrypoints stable while their internals move.

Definition of done:

- The GUI app/library layer stays under the current thresholds and keeps its module ownership clear.
- The native smoke runner stays scenario-owned and under its current size range.
- No new GUI extraction starts unless parity work touches `platform_driver.rs`,
  `syncplay-gui-native-smoke.rs`, or `live_python_interop.rs` enough to justify moving a concern.
- Existing semantic and Python-interop commands still work unchanged.

### 2. Thin down `syncplay-cli`

Why second:

- `syncplay-cli/src/main.rs` is still a major monolith.
- It has a clear destination seam in `syncplay-client-app`.

Actions:

- Add `crates/syncplay-cli/src/lib.rs`.
- Move parsing, startup planning, persistence wiring, and network-loop orchestration out of
  `main.rs`.
- Move reusable behavior into `syncplay-client-app` when it is not CLI-specific.
- Leave `main.rs` responsible only for process entry, stdout/stderr behavior, and exit handling.

Definition of done:

- `syncplay-cli/src/main.rs` is a thin entrypoint.
- Shared startup behavior lives in library code that can be reused by tests or future frontends.

### 3. Split `syncplay-client-core` around existing seams

Why third:

- The runtime/session boundary already exists.
- Many future parity slices depend on touching this crate safely.

Actions:

- Keep `ClientRuntimeAction` and `ClientRuntimeControl` as the stable seam.
- Split `lib.rs` into focused modules such as:
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

- `lib.rs` becomes a small crate-wiring layer.
- Session state and runtime orchestration no longer live in one file.

### 4. Only take optional `mpv`-first GUI polish if it is explicitly wanted

Priority order:

1. Rich About/help/TLS dialog fidelity
2. Player-path browse/autodetect/icon UX
3. Main-window geometry persistence
4. Revisit additional player backends only when the scope is deliberately reopened

Execution rule:

- Use the Python client as the reference behavior.
- Add the narrowest failing test or smoke assertion first.
- Do not reopen broad maintainability extractions in `src/app/` just to land these polish items.

### 5. Improve agent-facing ergonomics

- Prefer one concern per module and one major state owner per file.
- Keep reducer entrypoints, runtime adapters, and transport seams explicit and easy to grep.
- Add short module-level docs for major boundaries once the extraction lands.
- Keep a small stable set of verification commands for routine use.

### 6. Secondary cleanup after the main extractions

- Split `syncplay-compat/src/lib.rs` into protocol probes, Python harnesses, and server-runtime
  scenarios.
- Revisit whether some GUI-only shell state types belong in narrower modules or a separate crate.
- Add a compact compatibility matrix generated from tests once the parity backlog is smaller.

## Immediate backlog

- [x] Create `syncplay-gui` library-first structure and reduce `src/main.rs` to a thin launcher.
- [x] Move GUI semantic and Python-interop helpers out of the GUI binary entrypoint.
- [x] Dissolve `crates/syncplay-gui/src/app_tests.rs` into area-owned unit-test modules and
      `app/testing/support`.
- [x] Add `crates/syncplay-gui/src/app/testing/support.rs` as the shared GUI test harness seam.
- [x] Create a real `crates/syncplay-gui/src/app/` hierarchy and retire the flat `app.rs`
      `#[path]` router.
- [x] Finish splitting `crates/syncplay-gui/src/app_runtime_stack.rs`.
- [x] Split `crates/syncplay-gui/src/app_widget_views.rs`.
- [x] Split `crates/syncplay-gui/src/app_shell_state.rs`.
- [x] Split `crates/syncplay-gui/src/app_runtime_owner/tests.rs`.
- [x] Split `crates/syncplay-gui/src/app_runtime_stack/tests.rs`.
- [x] Split `crates/syncplay-gui/src/app_smoke.rs` into smaller scenario-owned smoke modules.
- [x] Keep `semantic_smoke.rs` public entrypoints stable while splitting parser, catalog, CLI, and
      code-driven smoke internals into `src/semantic_smoke/`.
- [x] Split `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke.rs` into root, platform-driver,
      and scenario-runner modules.
- [x] Split
      `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke/native_smoke_runner.rs` into
      scenario-owned submodules while keeping the runner root under 1000 lines.
- [ ] Split `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke/platform_driver.rs` further
      only if more Windows driver logic lands there.
- [ ] Split shared launch/config/mock-server helpers out of
      `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke.rs` only if more behavior lands there.
- [x] Keep the documented full GUI smoke gate for semantic, native, and real-`mpv` coverage.
- [ ] Add `crates/syncplay-cli/src/lib.rs` and reduce `main.rs` to entrypoint code.
- [ ] Split `crates/syncplay-client-core/src/lib.rs` into runtime/session-focused modules.
- [x] Implement GUI slash-command handling using `syncplay-client-app` command planning.
- [x] Finish missing configuration dialog controls.
- [x] Localize runtime strings and language-sensitive service calls.
- [x] Decide and document that additional player backends remain deferred after `mpv` in this
      plan.
- [x] Port Python-style config-confirm startup handoff so confirming settings also saves the draft,
      starts the saved session, and reuses the existing managed-player startup path.
- [ ] Port richer Python-style About/help/TLS dialog details only if release UX requires them.
- [ ] Add Python-style player-path browse/icon/autodetect UX only if text-entry configuration
      proves insufficient.
- [ ] Port legacy main-window size/position persistence only if users miss that desktop behavior.

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

## Non-goals

- Reopening non-`mpv` player parity immediately.
- Large-scale crate reshuffling before using the seams that already exist.
- Rewriting working behavior for style alone.

## Success criteria

This plan is succeeding if:

- the largest files are shrinking instead of growing,
- test ownership stays local instead of converging back into giant shared files,
- parity work lands in smaller vertical slices,
- tests stay green through extractions,
- and future audits focus more on behavior gaps than on codebase navigability.

For the GUI specifically, this plan is succeeding if the remaining discussion is about optional
desktop polish or explicitly deferred player backends rather than broad app-tree churn or missing
`mpv`-scope session, playlist, chat, configuration, or service behavior.

The current GUI app/library maintainability round is done:

- `app.rs` no longer acts as a flat path router,
- `semantic_smoke.rs` no longer concentrates unrelated parser, catalog, CLI, and code-driven
  smoke concerns in one file,
- no GUI app/library production module remains above roughly 1500 lines,
- the current overlarge GUI test and smoke library files are back under the working thresholds,
- `native_smoke_runner.rs` is now scenario-owned and back under the working threshold,
- and the next GUI audit is more about behavior gaps plus the native smoke driver/helper tooling
  than about app-tree ownership.

If another GUI maintainability round is opened, it should start with
`src/bin/syncplay-gui-native-smoke/platform_driver.rs` or the shared helper layer in
`src/bin/syncplay-gui-native-smoke.rs`, not with more churn in `src/app/`.
