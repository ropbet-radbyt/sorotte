# Port Maintainability Plan

Working plan for keeping the Rust port shippable while finishing client parity.

## Independent evaluation

- Audit date: 2026-03-19
- Last updated: 2026-04-25
- Verification performed for this reassessment:
  - `cargo fmt --all --check`
  - `cargo test -p syncplay-gui` (`431` passed, `1` ignored local `mpv` smoke)
  - `cargo clippy -p syncplay-gui --all-targets -- -D warnings`
  - `cargo test -p syncplay-gui --features gui-semantic-smoke,live-python-interop`
    (`450` passed, `1` ignored local `mpv` smoke)
  - `cargo clippy -p syncplay-gui --features gui-semantic-smoke,live-python-interop --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`
    (`12/12` scenarios)
  - `cargo build -p syncplay-gui --bin syncplay-gui`
  - `cargo clippy -p syncplay-gui --features gui-native-smoke --bin syncplay-gui-native-smoke -- -D warnings`
  - `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`
    (`ok`, accessibility and interaction contracts verified)
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
- `src/app/render_actions.rs`, `src/app/native_host.rs`, and `src/app/runtime_bridge.rs` are now
  narrow routers over behavior-owned child modules,
- and the remaining large GUI files are native smoke tooling or local app leaves rather than
  crate-global dumping grounds.

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

GUI hotspots were remeasured on 2026-04-25 after the renderer, render-action, native-host,
runtime-bridge, runtime-owner, runtime-stack, runtime-localization, shell-state, runtime
request-router, main-window projection, live-Python interop, shell-projection, stream-helper,
semantic-driver, transport, and runtime-update splits. This pass includes `src/bin/` so the native
smoke tooling binary is visible in the figures.
`syncplay-cli`, `syncplay-client-app`, `syncplay-client-core`, `syncplay-player-mpv`,
`syncplay-server`, and `syncplay-compat` production and test hotspots were remeasured on
2026-04-25 after the current production/test hotspot splits.

### `syncplay-gui` library and app production

- `src/app/runtime_stack/client_core_adapter/runtime_adapter_impl.rs` - 811 lines
- `src/app/render_egui.rs` - 739 lines
- `src/app/remote_services.rs` - 713 lines
- `src/app/runtime_stack/media_search.rs` - 710 lines
- `src/app/runtime_owner/player/media_index.rs` - 709 lines
- `src/app/shell_workflows.rs` - 704 lines
- `src/app/shell_state.rs` - 702 lines
- `src/app/mpv_launch.rs` - 692 lines
- `src/app/runtime_stack/client_core_adapter.rs` - 667 lines
- `src/app/runtime_updates/runtime_snapshots.rs` - 661 lines after splitting pending
  configuration operations into `src/app/runtime_updates/configuration_operations.rs`
- `src/app/shell_core.rs` - 623 lines
- `src/app/main_window_workflows.rs` - 620 lines
- `src/app/render_egui/controls.rs` - 610 lines
- `src/app/runtime_detached.rs` - 609 lines
- `src/app/playlist_workflows.rs` - 605 lines
- `src/app/runtime_queue.rs` - 587 lines
- `src/app/shell_state/configuration_dialog_projection.rs` - 577 lines
- `src/app/semantic_driver/steps.rs` - 573 lines after splitting driver execution and scenario
  ownership into sibling modules
- `src/app/live_python_interop/flows.rs` - 540 lines after splitting projection helpers, runtime
  actions, waits, and root public entrypoints
- `src/app/runtime_stack/transport/tcp.rs` - 537 lines after splitting queue handle, loopback
  driver, and TCP tests into child modules
- `src/app/render_actions/buttons.rs` - 529 lines after splitting input, list, menu, surface, and
  helper action concerns out of `src/app/render_actions.rs`
- `src/app/reducer.rs` - 202 lines after splitting shell/runtime, configuration-operation,
  editing, main-window, service, and miscellaneous action domains into `src/app/reducer/` child
  modules; the largest reducer child is `editing_actions.rs` at 313 lines.
- `src/app/runtime_localization/pattern_messages.rs` - 14 lines after splitting public-server,
  reconnect, autoplay, controller/update, and user-status pattern localizers into child modules;
  the largest child is `public_server_media.rs` at 298 lines.
- `src/app/stream_support.rs` - 128 lines after splitting path, process, metadata, discovery,
  runtime snapshot, install/import, and tests into child modules
- `src/app/native_host.rs` - 54 lines after splitting app core, eframe app/host, preview host,
  and tests into `src/app/native_host/`
- `src/app/shell_projection.rs` - 29 lines after splitting app-state, basic-state, main-window,
  menu-dialog, public-server, and media-search projections into child modules
- `src/app/runtime_bridge.rs` - 27 lines after splitting runtime owner traits, preview bridge,
  request routing, pending completions, and tests into `src/app/runtime_bridge/`
- `src/app/render_actions.rs` - 22 lines after splitting renderer action handlers into
  `src/app/render_actions/`
- `src/app/semantic_driver.rs` - 10 lines after splitting DSL steps, driver execution, scenario
  wrapper, and tests into child modules
- `src/app/runtime_stack/transport.rs` - 10 lines after splitting queue/loopback/TCP ownership
  into child modules
- `src/app/runtime_updates.rs` - 2 lines after splitting runtime-snapshot application and local
  configuration operations into child modules
- `src/app/runtime_owner/requests.rs` - 309 lines after splitting playback, session-control,
  stream-helper, and pending-completion handlers into child modules
- `src/app/widget_views/main_window.rs` - 81 lines after splitting browser, chat, editor,
  playlist, and summary projections into child modules

### `syncplay-gui` binaries and tooling

- `src/bin/syncplay-gui-native-smoke/native_smoke_runner/shared_helpers.rs` - 807 lines
- `src/bin/syncplay-gui-native-smoke/native_smoke_runner/baseline_contract.rs` - 660 lines
- `src/bin/syncplay-gui-native-smoke/native_smoke_runner/live_python_contracts.rs` - 562 lines
- `src/bin/syncplay-gui-native-smoke/native_smoke_setup.rs` - 459 lines
- `src/bin/syncplay-gui-native-smoke/native_smoke_runner/relaunch_contract.rs` - 443 lines
- `src/bin/syncplay-gui-native-smoke/native_smoke_accessibility.rs` - 415 lines
- `src/bin/syncplay-gui-native-smoke/platform_driver/windows_control_actions.rs` - 399 lines
- `src/bin/syncplay-gui-native-smoke/platform_driver/windows_edit_controls.rs` - 343 lines
- `src/bin/syncplay-gui-native-smoke/native_smoke_runner.rs` - 268 lines
- `src/bin/syncplay-gui-native-smoke/platform_driver/windows_control_queries.rs` - 135 lines
- `src/bin/syncplay-gui-native-smoke/platform_driver.rs` - 105 lines
- `src/bin/syncplay-gui-native-smoke.rs` - 180 lines
- `src/bin/syncplay-gui-semantic-suite.rs` - 227 lines
- `src/bin/syncplay-gui-semantic-smoke.rs` - 30 lines

### `syncplay-gui` tests and smoke

- largest behavior-owned test files after the 2026-04-24 split:
  - `src/app/runtime_stack/tests/playlist_tests.rs` - 642 lines
  - `src/app/runtime_owner/tests/transport_tests/chat_readiness_media_transport.rs` - 625 lines
  - `src/app/runtime_owner/tests/connection_runtime_tests/config_and_public_servers.rs` - 600 lines
  - `src/app/runtime_owner/tests/player_runtime_tests/media_search_cache.rs` - 600 lines
  - `src/app/shell_state/tests/main_window_playlist_tests/window_runtime_tests.rs` - 592 lines
  - `src/app/shell_state/tests/main_window_playlist_tests/playlist_workflow_tests.rs` - 573 lines
  - `src/app/runtime_stack/tests/session_transition_tests.rs` - 563 lines

### `syncplay-cli` current refactor snapshot

- largest production leaves after the 2026-04-25 split:
  - `src/session_runner/connected_session.rs` - 712 lines
  - `src/mpv_startup/explicit_args.rs` - 670 lines
  - `src/client_config.rs` - 577 lines
  - `src/client_args.rs` - 522 lines
  - `src/session_runner/network_loop.rs` - 414 lines
  - `src/lib.rs` - 359 lines
  - `src/mpv_startup/attached_startup.rs` - 325 lines
  - `src/stored_settings/persistence.rs` - 295 lines
  - `src/stored_settings/ui_settings.rs` - 237 lines
  - `src/stored_settings/media_search.rs` - 235 lines
  - `src/notifications/playback_diagnostics.rs` - 203 lines
- largest behavior-owned test files after the split:
  - `src/tests/connected_session_local_commands/controlled_rooms/room_switches.rs` - 672 lines
  - `src/tests/connected_session_basics/connect_chat.rs` - 644 lines
  - `src/tests/connected_session_desync/reconnect_slowdown.rs` - 602 lines
  - `src/tests/connected_session_local_commands/playlist_mutations/queue_and_delete.rs` - 596 lines
  - `src/tests/connected_session_desync/reconnect_rewind.rs` - 590 lines
- `src/client_config.rs` - 577 lines
- `src/client_args.rs` - 522 lines
- `src/lib.rs` - 359 lines
- `src/tests.rs` - 336 lines
- `src/update_check.rs` - 154 lines
- `src/local_runtime_actions.rs` - 112 lines
- `src/env_support.rs` - 96 lines
- `src/diagnostics_config.rs` - 85 lines
- `src/startup_playlist.rs` - 49 lines
- `src/config_paths.rs` - 47 lines
- `src/language_support.rs` - 38 lines
- `src/stdin_input.rs` - 30 lines
- `src/protocol_io.rs` - 27 lines
- `src/main.rs` - 4 lines

### `syncplay-client-app` current refactor snapshot

- `src/legacy_reconnect_diagnostics.rs` - 738 lines
- `src/legacy_session_loop/tests/connected_session_tests.rs` - 666 lines
- `src/legacy_local_commands/display.rs` - 604 lines
- `src/legacy_local_commands/parser.rs` - 564 lines
- `src/legacy_local_commands/tests.rs` - 533 lines
- `src/legacy_syncplay_ini/writer.rs` - 523 lines
- `src/legacy_notifications.rs` - 27 lines after splitting reconnect, controller-auth, duration,
  user-change, and file-difference notification formatting into child modules; the largest child
  is `legacy_notifications/reconnect.rs` at 210 lines.
- `src/legacy_local_commands.rs`, `src/legacy_session_loop.rs`, `src/legacy_syncplay_ini.rs`, and
  `src/legacy_notifications.rs` are now narrow routers over behavior-owned child modules.

### `syncplay-client-core` current refactor snapshot

- `src/session/helpers.rs` - 757 lines
- `src/session/tests/readiness_autoplay_tests.rs` - 737 lines
- `src/session/tests/reconnect_tests/validation_flow_tests.rs` - 714 lines
- `src/session/tests/reconnect_tests/validation_policy_tests/disable_recovery_tests.rs` - 709 lines
- `src/session/tests/reconnect_tests/reset_tests.rs` - 6 lines after splitting core state,
  fast-forward, feature-flag, rewind, and slowdown reset cases into child modules; the largest
  child is `slowdown.rs` at 266 lines.
- `src/runtime.rs` - 13 lines after becoming a router over `runtime/local_actions.rs`,
  `runtime/queued_control.rs`, `runtime/lifecycle_actions.rs`, and `runtime/accessors.rs`.
- `src/session/playlist.rs` is now a router over `local_playlist.rs`, `reconnect_actions.rs`,
  `local_controls.rs`, `trust.rs`, and `shuffle_helpers.rs`.

### `syncplay-protocol` current refactor snapshot

- `src/tests.rs` - 373 lines
- `src/set.rs` - 302 lines
- `src/state.rs` - 134 lines
- `src/message.rs` - 103 lines
- `src/lib.rs` - 31 lines after becoming a re-export/router layer.

### `syncplay-player-mpv` current refactor snapshot

- `src/adapter.rs` - 770 lines
- `src/tests/ipc_tests.rs` - 649 lines
- `src/tests/legacy_ui_tests.rs` - 407 lines
- `src/adapter/player_adapter.rs` - 273 lines
- `src/ipc.rs` - 262 lines
- `src/lib.rs` - 10 lines after becoming a re-export/router layer.

### `syncplay-server` current refactor snapshot

- `src/runtime_maintenance.rs` - 697 lines
- `src/tests/network_tests.rs` - 668 lines
- `src/tests/runtime_config_tests.rs` - 619 lines
- `src/main.rs` - 615 lines
- `src/runtime_handlers.rs` - 562 lines
- `src/lib.rs` - 222 lines after becoming a re-export/core-type layer.
- `src/tests/session_tests.rs` - 639 lines
- `src/runtime_handlers.rs` - 599 lines
- `src/tests/state_tests.rs` - 541 lines
- `src/network.rs` - 342 lines
- `src/runtime_api.rs` - 281 lines
- `src/lib.rs` - 243 lines after splitting app, auth, compat, messages, network,
  persistence, TLS, runtime API, runtime handlers, runtime maintenance, and tests.

### `syncplay-compat` current refactor snapshot

- `src/tests/assertions/legacy_tls_assertions.rs` - 531 lines
- `src/tests/legacy_client_contract_tests.rs` - 517 lines
- `src/tests/normalization_support.rs` - 453 lines
- `src/legacy_process.rs` - 450 lines
- `src/tests/rooms_motd_fanout_tests/scripted_rooms_motd_tests.rs` - 397 lines
- `src/tests/chat_fanout_tests/scripted_chat_payload_tests.rs` - 385 lines
- `src/tests/chat_fanout_tests/scripted_chat_scoping_tests.rs` - 364 lines

## Current read

### Strengths

- Workspace-level crate boundaries are already useful.
- `syncplay-client-app` provides a real shared seam for commands, persistence, language, and
  startup/session planning.
- `syncplay-gui` is already library-first and keeps its launcher surface stable.
- The crate-global GUI test hotspot has been broken apart into area-owned test modules.
- `src/app/mod.rs` now acts as the real GUI module root instead of a flat router file.
- `src/app/widget_views/` and `src/app/runtime_stack/` now use area-owned production submodules
  rather than monolithic top-level files.
- `src/app/shell_state/` now owns browser/playlist helpers plus the extracted configuration and
  main-window state models, leaving `app_shell_state.rs` below the soft cap.
- `src/app/runtime_owner/requests.rs` is now a small dispatcher with request-domain handlers under
  `src/app/runtime_owner/requests/` for playback, session controls, stream-helper remediation, and
  pending completions.
- `src/app/widget_views/main_window.rs` is now a dashboard assembler with browser, chat, editor,
  playlist, and summary projections under `src/app/widget_views/main_window/`.
- `src/app/live_python_interop.rs`, `src/app/shell_projection.rs`, `src/app/stream_support.rs`,
  `src/app/semantic_driver.rs`, `src/app/runtime_stack/transport.rs`,
  `src/app/runtime_updates.rs`, `src/app/render_actions.rs`, `src/app/native_host.rs`, and
  `src/app/runtime_bridge.rs` are now narrow router/API files with behavior-owned child modules
  for their previously mixed concerns.
- `src/semantic_smoke/` now separates external-script parsing, scenario cataloging, CLI handling,
  and code-driven smoke flows while keeping the public semantic smoke entrypoints stable.
- `src/bin/syncplay-gui-native-smoke/` now separates CLI/process orchestration, platform-driver
  integration, a shared runner root, and scenario-owned contract modules instead of concentrating
  all tooling behavior in one file.
- `src/app/testing/support.rs` now provides a shared fixture and harness seam for GUI tests.
- The remaining overlarge GUI test files have been split into behavior-owned child modules across
  runtime-owner, renderer, widget-view, startup, shell-state, and runtime-stack areas; the largest
  Rust test leaf is now below the documented split threshold.
- The GUI has semantic and Python-interop coverage rather than only narrow unit tests.
- The 2026-03-19 reassessment passed `cargo test -p syncplay-gui` and the semantic suite without
  finding a new `mpv`-scope blocker.
- The repository already has stable smoke commands for semantic and native GUI validation.
- `syncplay-cli` now has a real library entrypoint, a thin binary entrypoint, and focused modules
  for legacy argument parsing, client config/runtime overrides, config paths, diagnostics
  configuration, environment parsing, runtime-language state, managed/explicit-`mpv` startup,
  stored-settings persistence/startup defaults, update-check persistence, runtime
  notification/diagnostic output, protocol I/O, stdin/local runtime actions, startup playlist
  loading, and the connected-session/network retry runner.
- The largest `syncplay-cli` production roots have been split into behavior-owned module trees:
  `mpv_startup/`, `session_runner/`, `stored_settings/`, and `notifications/` now keep parsing,
  process launch, persistence, network-loop, and output categories separate behind stable routers.
- The `syncplay-cli` compatibility and unit test suite is no longer a single crate-level hotspot:
  `src/tests.rs` is now a shared harness/router, and behavior-owned submodules cover argument
  compatibility, connected-session basics, local commands, desync/reconnect correction,
  stored-settings persistence, notification output, startup playlists, and mpv smokes.
- `syncplay-client-app` no longer concentrates legacy local command parsing/planning,
  connected-session/network-loop behavior, legacy `syncplay.ini` parsing/writing, or legacy
  notification formatting in single files; each now routes through behavior-owned child modules.
- `syncplay-client-core` keeps the public session surface stable while playlist concerns now live
  under `src/session/playlist/` by local controls, local playlist mutation, reconnect actions,
  shuffle helpers, and trusted-domain checks; runtime control/action concerns now live under
  `src/runtime/` while `src/runtime.rs` stays as the small public router.
- `syncplay-protocol` no longer concentrates all message families in `src/lib.rs`; message,
  envelope, hello, list, room, set, state, chat, codec, and fixture tests are owned by focused
  modules behind a re-export layer.
- `syncplay-player-mpv` is now split into adapter state, player-trait implementation, JSON IPC
  transport, legacy Syncplay UI/script formatting, constants, and behavior-owned test modules.
- `syncplay-server` no longer uses one crate-sized `lib.rs`: app wiring, controlled-room auth,
  compatibility helpers, message construction, network loop, persistence, TLS loading, runtime API,
  runtime handlers, runtime maintenance, and test categories are owned by separate modules.
- `syncplay-gui` reducer and runtime-localization pattern dispatch are now router modules, so GUI
  action routing and localized runtime pattern families have local ownership instead of single-file
  match tables.

### Main risks

- The GUI app/library production tree and native-smoke tooling now have no file above the 900-line
  working target. The next GUI risk is growth control in the 700-900 line behavior leaves rather
  than a single blocking monolith.
- The test tree is now in a good local-ownership shape; the next test concern is only preventing
  the new behavior leaves from regrowing past the split threshold.
- The remaining GUI delta against Python is now mostly optional desktop polish: richer
  About/help/TLS dialogs, player-path browse UX, and window geometry persistence.
- The `syncplay-cli` test suite now follows the local ownership rule, and the CLI production roots
  are no longer monoliths. The remaining CLI risk is preventing the largest behavior leaves
  (`session_runner/connected_session.rs` and `mpv_startup/explicit_args.rs`) from growing back past
  the split threshold.
- Outside the GUI, the previously named protocol, client-core runtime, client-core reconnect-reset,
  and client-app notification hotspots are now routers. The remaining cleanup surface is local
  700-900 line behavior leaves and tests, especially client-core readiness/reconnect validation
  tests, client-app reconnect diagnostics, and a few GUI runtime/adapter leaves.
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
is retired behind `src/app/mod.rs`, `semantic_smoke.rs` is split into area-owned submodules, and the
egui renderer is now split across `src/app/render_egui/` modules for layout, controls, playback
controls, room browsing, playlist rendering, chat rendering, modal text/actions, display helpers,
and widget-tree rendering. The runtime-owner player surface now routes through
`src/app/runtime_owner/player/` modules for player state, media search/indexing, stream loading,
shared-playlist opening, detached-session control, and attached-session sync. The runtime-owner
root now owns model types while startup/player attachment, transport pumping, room transitions,
player facade helpers, and pump orchestration live in focused child modules. The runtime-stack root
now re-exports adapter types while concrete client-core adapter construction, event draining, and
trait bridging live under `src/app/runtime_stack/client_core_adapter/`. Runtime localization now
separates exact-message, pattern-message, and generic-error matchers; render actions, native
hosting, and runtime bridging now route through focused child modules; and shell state separates
the action vocabulary plus configuration-dialog projection from the central state model.
The next GUI maintainability work should keep following touched concerns: extract from the app tree
only when a hotspot is already being changed, and otherwise prioritize standalone tooling binaries
and shared harness helpers.

### Direction

- Keep `src/main.rs` as a thin launcher.
- Keep `src/lib.rs` as the crate entry surface.
- Keep `src/app/mod.rs` as the real GUI module root and continue using area-owned submodules when
  touched concerns need extraction.
- Keep `src/app/semantic_smoke/` as an area-owned harness module tree rather than letting the root
  file regrow.
- Keep moving dependencies toward explicit area imports rather than parent-wide namespace imports.
- Do not start another app-tree split unless a leaf regrows past the working threshold or a parity
  change already touches it. The current watch list is the reducer, runtime adapter
  implementation, and localization pattern matchers; render actions, native hosting, runtime
  bridge, interop, stream-helper, shell-projection, semantic-driver, transport, and runtime updates
  are already split.
- If native smoke tooling keeps growing, keep new behavior inside the existing focused modules
  (`native_smoke_setup.rs`, `native_smoke_accessibility.rs`, `native_smoke_runner/shared_helpers.rs`,
  and `platform_driver/windows_*`) instead of rebuilding a binary-root helper pile.

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

No new runtime-stack or runtime-owner player action is required today. Keep the current split
module/test layout stable and only split local hotspots when real feature work expands them again.

If more GUI work is opened intentionally, prefer one of these small, explicit targets:

- optional polish parity: richer About/help/TLS dialogs,
- optional polish parity: player-path browse/autodetect/icon UX,
- conditional maintainability: split `native_smoke_runner/shared_helpers.rs` only if more runner
  helper logic lands there,
- conditional maintainability: split runtime-adapter plumbing or another regrown app leaf only when
  those areas are already being changed,
- and continued subdivision inside `app/shell_state/tests/` only when a local file crosses the
  test-size threshold.

## Working order

### 1. Keep the native GUI smoke tooling bounded

Why first:

- The app/library side of the GUI is now in a defensible state.
- The native smoke runner and Windows/UIA driver are now module-owned and no longer the main
  GUI-local review hazard.
- The remaining GUI-local extraction targets are local leaves, not binary/root monoliths.

Actions:

- Keep `src/bin/syncplay-gui-native-smoke.rs` focused on process entry, shared types, constants,
  and module wiring; do not let scenario, launch, or accessibility logic drift back into it.
- Keep `src/bin/syncplay-gui-native-smoke/native_smoke_runner.rs` as orchestration only and add
  new scenarios in owned submodules.
- Split `native_smoke_runner/shared_helpers.rs` only if more runner helper behavior lands there;
  the Windows/UIA control handling is already divided into action, query, edit-control, input, and
  automation modules.
- Keep `app/testing/support.rs` as the shared support seam.
- Keep semantic, native, and real-`mpv` smoke entrypoints stable while their internals move.

Definition of done:

- The GUI app/library layer stays under the current thresholds and keeps its module ownership clear.
- The native smoke runner stays scenario-owned and under its current size range.
- No new GUI extraction starts unless parity work touches a local native-smoke leaf or an already
  split app leaf enough to justify moving a concern.
- Existing semantic and Python-interop commands still work unchanged.

### 2. Thin down `syncplay-cli`

Why second:

- `syncplay-cli/src/main.rs` is now a thin Tokio entrypoint.
- `syncplay-cli/src/lib.rs` is now a small crate entry surface, and `src/tests.rs` is now only the
  compatibility-suite harness/router. The former CLI-size production roots are now routers with
  behavior-owned module trees.
- It has a clear destination seam in `syncplay-client-app`.

Actions:

- Keep the current CLI module trees narrow and split behavior leaves before they cross the local
  hotspot threshold again.
- Keep the already extracted `config_paths`, `diagnostics_config`, `env_support`,
  `language_support`, `mpv_startup`, `notifications`, `stored_settings`, `client_args`,
  `client_config`, `protocol_io`, `stdin_input`, `local_runtime_actions`, `startup_playlist`,
  `session_runner`, and `update_check` modules narrow while moving remaining reusable behavior.
- Move reusable behavior into `syncplay-client-app` when it is not CLI-specific.
- Leave `main.rs` responsible only for process entry, stdout/stderr behavior, and exit handling.

Definition of done:

- `syncplay-cli/src/main.rs` stays a thin entrypoint.
- `syncplay-cli/src/lib.rs` stays a small crate entry surface.
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
- [x] Split remaining large GUI test leaves across runtime-owner, renderer, widget-view, startup,
      shell-state, and runtime-stack areas into behavior-owned child modules.
- [x] Split `crates/syncplay-gui/src/app_smoke.rs` into smaller scenario-owned smoke modules.
- [x] Keep `semantic_smoke.rs` public entrypoints stable while splitting parser, catalog, CLI, and
      code-driven smoke internals into `src/semantic_smoke/`.
- [x] Split `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke.rs` into root, platform-driver,
      and scenario-runner modules.
- [x] Split
      `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke/native_smoke_runner.rs` into
      scenario-owned submodules while keeping the runner root under 1000 lines.
- [x] Split modal, display, widget-tree, playlist, chat, room-browser, layout, controls, and
      playback-control renderer helpers out of
      `crates/syncplay-gui/src/app/render_egui.rs` into `crates/syncplay-gui/src/app/render_egui/`.
- [x] Split runtime-owner attached-player behavior out of
      `crates/syncplay-gui/src/app/runtime_owner/player.rs` into focused player state,
      media-index/search, stream-load, shared-playlist, detached-session, and attached-session
      modules.
- [x] Split `crates/syncplay-gui/src/app/runtime_owner.rs` into model/root, startup-player,
      session-transport, room-transition, player-facade, and runtime-pump modules.
- [x] Split `crates/syncplay-gui/src/app/runtime_stack.rs` into adapter interface, concrete
      client-core adapter, and trait-bridge modules while keeping runtime-stack root as a narrow
      re-export surface.
- [x] Split `crates/syncplay-gui/src/app/runtime_localization.rs` into exact-message,
      pattern-message, generic-error, and test modules.
- [x] Split `crates/syncplay-gui/src/app/shell_state.rs` so the shell action vocabulary and
      configuration-dialog projection live in child modules.
- [x] Split `crates/syncplay-gui/src/app/runtime_owner/requests.rs` into playback,
      session-control, stream-helper, and pending-completion request handlers.
- [x] Split `crates/syncplay-gui/src/app/widget_views/main_window.rs` into browser, chat,
      editor, playlist, and summary projection modules.
- [x] Split `crates/syncplay-gui/src/app/live_python_interop.rs` into flow, projection,
      runtime-action, and wait-helper modules.
- [x] Split `crates/syncplay-gui/src/app/shell_projection.rs` into app-state, basic-state,
      main-window, menu-dialog, public-server, and media-search projection modules.
- [x] Split `crates/syncplay-gui/src/app/stream_support.rs` into path, process, metadata,
      discovery, snapshot, install/import, and test modules.
- [x] Split `crates/syncplay-gui/src/app/semantic_driver.rs` into DSL step parsing, driver
      execution, scenario wrapper, and tests.
- [x] Split `crates/syncplay-gui/src/app/runtime_stack/transport.rs` into queued handle,
      loopback driver, TCP/TLS driver, and transport tests.
- [x] Split `crates/syncplay-gui/src/app/runtime_updates.rs` into runtime-snapshot application
      and local configuration-operation modules.
- [x] Split `crates/syncplay-gui/src/app/render_actions.rs` into button, input, list, menu,
      surface, and helper action modules.
- [x] Split `crates/syncplay-gui/src/app/native_host.rs` into app-core, eframe-app, eframe-host,
      preview-host, and test modules.
- [x] Split `crates/syncplay-gui/src/app/runtime_bridge.rs` into runtime-owner trait, preview
      bridge, request routing, pending-completion, and test modules.
- [x] Split `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke/platform_driver.rs` into a
      small cross-platform contract plus Windows UI Automation/action modules.
- [x] Split shared launch/config/accessibility/mock-server helpers out of
      `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke.rs` and
      `native_smoke_runner.rs` so native-smoke tooling is below the 900-line working target.
- [x] Keep the documented full GUI smoke gate for semantic, native, and real-`mpv` coverage.
- [x] Add `crates/syncplay-cli/src/lib.rs` and reduce `main.rs` to entrypoint code.
- [x] Extract initial `syncplay-cli` support modules for config path resolution, environment
      parsing, runtime-language state, and update-check timestamp persistence.
- [x] Extract `syncplay-cli` runtime notification and diagnostic output helpers into
      `crates/syncplay-cli/src/notifications.rs`.
- [x] Extract `syncplay-cli` diagnostics configuration/env parsing into
      `crates/syncplay-cli/src/diagnostics_config.rs`.
- [x] Extract managed and explicit-IPC `mpv` startup behavior into
      `crates/syncplay-cli/src/mpv_startup.rs`.
- [x] Extract legacy CLI argument parsing/help/compatibility output into
      `crates/syncplay-cli/src/client_args.rs`.
- [x] Extract stored settings, stored player defaults, legacy QSettings cleanup, and media-search
      startup fallback into `crates/syncplay-cli/src/stored_settings.rs`.
- [x] Extract `syncplay-cli` client config/runtime override/session construction behavior into
      `crates/syncplay-cli/src/client_config.rs`.
- [x] Extract `syncplay-cli` startup playlist protocol helpers into
      `crates/syncplay-cli/src/startup_playlist.rs`.
- [x] Extract `syncplay-cli` protocol I/O, stdin input, and local runtime-action helpers into
      `protocol_io.rs`, `stdin_input.rs`, and `local_runtime_actions.rs`.
- [x] Extract `syncplay-cli` connected-session and retrying network-loop execution into
      `crates/syncplay-cli/src/session_runner.rs`.
- [x] Move the large inline `syncplay-cli` compatibility/unit suite out of `lib.rs` into
      `crates/syncplay-cli/src/tests.rs`.
- [x] Split `crates/syncplay-cli/src/tests.rs` by owning module/behavior instead of keeping one
      compatibility-suite hotspot.
- [x] Split `crates/syncplay-cli/src/mpv_startup.rs` into managed-launch, path resolution,
      explicit-IPC argument parsing, attached startup application, and external-launch modules.
- [x] Split `crates/syncplay-cli/src/session_runner.rs` into connected-session and network-retry
      modules.
- [x] Split `crates/syncplay-cli/src/stored_settings.rs` into config application, UI/mpv settings,
      media-search fallback, startup player defaults, and persistence/cleanup modules.
- [x] Split `crates/syncplay-cli/src/notifications.rs` into notification-category modules with
      shared mpv OSD helpers.
- [x] Split `crates/syncplay-client-core/src/lib.rs`/runtime/session concerns into focused
      runtime, session, playlist, config, control, notification, ping, and view modules.
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
- no GUI app/library production module remains above the 900-line working target,
- the current overlarge GUI test and smoke library files are back under the working thresholds,
- `native_smoke_runner.rs`, the native-smoke binary root, and the platform driver are now
  scenario/module-owned and back under the working threshold,
- and the next GUI audit is more about behavior gaps and preventing local leaves from regrowing
  than about app-tree or native-smoke ownership.

If another GUI maintainability round is opened, it should start with a concrete regrown leaf such
as `native_smoke_runner/shared_helpers.rs`, `runtime_stack/client_core_adapter/runtime_adapter_impl.rs`,
or another touched 700-900 line app file, not with broad churn in `src/app/`.
