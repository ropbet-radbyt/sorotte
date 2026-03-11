# syncplay-rs Project Status

Audit snapshot for the Rust Syncplay rewrite.

## Audit date

- 2026-03-11

## What was verified in this audit

- `cargo fmt --all` passed.
- `cargo test --workspace` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` passed (`10/10` scenarios).
- `cargo build -p syncplay-gui --bin syncplay-gui` passed.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000` passed.
- `cargo test -p syncplay-gui gui_persisted_config_runtime_owner_starts_real_managed_mpv_from_saved_config -- --ignored` passed with local `SYNCPLAY_MPV_SMOKE_BIN`.
- The native smoke interaction trace now intentionally records menu-driven `Open Media File` as disabled until runtime-backed media support is available, while still validating runtime-backed drag/drop ingest.
- `cargo run --quiet -p syncplay-cli -- --help` matches the upstream Python client flag surface.
- `cargo run --quiet -p syncplay-server -- --help` prints a real Rust alpha CLI help surface.
- Local real-`mpv` smoke tests remain `ignored` by default outside environment-dependent manual validation.

## Summary

`syncplay-rs` is well beyond a skeleton rewrite: it has a verified CLI client, a GUI shell with semantic/native smoke coverage and live Python interop coverage, typed protocol handling, compatibility-focused tests, and a real `mpv` adapter. The project is not yet a full end-user replacement for Syncplay because broader packaging, cross-platform validation, and maintainability work remain, but the default GUI flow now includes GUI-owned `mpv` startup from saved `playerPath` settings, saved per-player arguments, and legacy Syncplay `mpv` OSD/chat behavior without requiring manual IPC environment variables. The main window now exposes a Python-style room/user/file browser with room grouping, per-user metadata/difference cues, hide-empty-room behavior, and runtime-backed room/file/folder/trusted-domain actions, and detached GUI sessions now sustain legacy-server connections with periodic heartbeat state updates even before a player is attached. Shared-playlist file opening/import now routes through the real GUI runtime path, desktop drag-and-drop ingest covers both media-open and shared-playlist import flows with semantic and Windows native smoke coverage, and the GUI now covers the Python playlist workflow slice as well: shuffle remaining/entire, undo playlist changes, add/open URL flows, playlist text editing, dedicated load/save dialogs, and playlist context actions are all present. Controlled-room/controller-auth parity is now also runtime-backed in the GUI, including create-controlled-room, manual identify-as-controller, generated-password surfacing, and server-gated set-others-readiness actions. Non-`mpv` players are currently deferred behind the remaining GUI parity backlog.

## Documentation set (current)

These are the Markdown files that should remain in this repo:

- `README.md` (overview + commands)
- `PROJECT_STATUS.md` (this audit + priorities)
- `docs/CLIENT_PARITY_AUDIT.md` (detailed remaining-work list)
- `docs/AGENT_IMPLEMENTATION_GUIDE.md` (required implementation/test workflow)
- `ALPHA_CLI_PREVIEW.md` (developer/alpha run and packaging guide)

Older planning/handoff docs have been archived outside this repo (workspace `old-docs/`) and are not canonical project status.

## Completed (checked)

- [x] Cargo workspace with separated crates for protocol, client core, server, player API, `mpv` adapter, CLI, compat, and simulation support.
- [x] Headless CLI client binary (`syncplay-cli`).
- [x] Python-compatible CLI help/startup surface for the upstream `syncplayClient.py` options.
- [x] Substantial server runtime library implementation and coverage in `crates/syncplay-server`.
- [x] Typed Syncplay protocol message models and fixture decoding coverage (`Hello`, `Set`, `List`, `State`, `Chat`, `Error`, `TLS` families).
- [x] Client session logic with reconnect/state restoration behaviors covered by tests.
- [x] Playlist and local command handling in the CLI (including controller/playlist command paths covered by tests).
- [x] `mpv` JSON IPC integration with attach/control/property updates and unit coverage in `syncplay-player-mpv`.
- [x] Managed `mpv` launch and explicit-IPC attach flows (with additional real-`mpv` smokes available as ignored tests).
- [x] GUI-owned `mpv` startup from saved `playerPath` plus `perPlayerArguments`, including legacy Syncplay `mpv` UI/chat settings, chat/OSD forwarding, and managed relaunch/failure handling.
- [x] GUI configuration/main-window shell with semantic smoke coverage and Windows native accessibility smoke coverage.
- [x] Python-style main-window room/user/file browser parity, including room grouping, file metadata/difference cues, hide-empty-room behavior, and runtime-backed room/file/folder/trusted-domain actions.
- [x] First-class GUI saved-config connect/disconnect flow, including startup auto-connect from persisted host/port settings and explicit session lifecycle controls on the configuration and main-window surfaces.
- [x] GUI session keepalive parity against legacy server timeouts, including periodic ping-backed state heartbeats for detached sessions without attached player telemetry.
- [x] Runtime-backed shared-playlist file opening/import from the GUI, including session playlist replacement and playlist-file import.
- [x] Python-style playlist workflow parity in the GUI, including shuffle remaining/entire, undo playlist change, add/open URL flows, playlist text editing, dedicated load/save dialogs, and playlist context actions.
- [x] Controlled-room/controller-auth GUI parity for create-controlled-room, manual identify-as-controller, generated-password surfacing, and server-gated set-others-readiness actions.
- [x] Desktop drag-and-drop ingest for detached media-open and shared-playlist import, with semantic and Windows native smoke coverage.
- [x] Tightened GUI command availability so config-only room/media/playback paths stop looking production-ready.
- [x] Live Python GUI interop scenarios for readiness/chat/playlist/reconnect/controller flows against the legacy Syncplay server.
- [x] Compatibility/interop test infrastructure comparing Rust runtime behavior to captured Python Syncplay traces/scenarios.
- [x] Server features with test coverage for room/state fanout, controlled rooms, playlist scoping, TLS upgrade paths, and persistent/permanent room behavior.
- [x] Rust server executable alpha entrypoint with `--help`, core startup flags, and listener/network-loop startup wiring over the server runtime.
- [x] CI/automation basics (`rust-ci.yml`) and coverage workflow (`rust-coverage.yml`), plus local cargo aliases in `.cargo/config.toml`.

## Remaining work (priority checklist)

- [x] Make main-window room join/leave runtime-authoritative so disconnected or pre-Hello states cannot fake a successful room change.
- [x] Replace preview-only shared-playlist file-open behavior with real player/session/playlist dispatch.
- [x] Add desktop drag-and-drop for media/playlist ingest plus semantic/native smoke coverage.
- [x] Tighten GUI command availability so non-working room/media paths stop looking production-ready.
- [x] Replace the shell-style main window with the Python room/user/file browser.
- [x] Close the remaining startup/player-launch parity gaps called out as partial in the compatibility matrix (`playerPath`, `perPlayerArguments`, finite explicit-IPC argument translation subset).
- [ ] End-to-end release packaging process (artifacts, versioning, changelog, signing strategy if needed).
- [ ] Automated real-`mpv` smoke coverage in CI (or documented repeatable manual gate with scripts + fixtures).
- [ ] Cross-platform validation beyond the current Windows-oriented GUI workflow.
- [ ] Expand `syncplay-server` CLI/runtime parity beyond the current alpha slice (remaining gaps include dual-interface binding parity and binary-level operational smoke coverage).
- [ ] Refactor/maintainability work for very large modules (notably `crates/syncplay-cli/src/main.rs` and `crates/syncplay-client-core/src/lib.rs`) to reduce change risk.

## Optional/next improvements

- [ ] Add a compatibility matrix table (Python Syncplay feature vs Rust status) sourced from tests to replace ad-hoc notes.
- [ ] Track manual alpha validation results by date/build in a compact changelog section (instead of freeform notes).

## Deferred

- [ ] Port additional player backends after complete `mpv` client parity is reached.

## Notes on scope

- Current evidence supports "substantially implemented client/server rewrite with a verified GUI shell," not "full replacement" parity.
- The GUI is real and test-covered, and saved-config connection plus the Python-style room/user/file browser plus Python-style playlist workflows are now runtime-backed; the major room/media/playback affordances no longer advertise config-only projections as working paths.
- The server runtime library remains further along than the user-facing `syncplay-server` CLI parity surface, even though a real alpha executable entrypoint now exists.
- Real `mpv` integration exists, including saved-config GUI-owned startup, but some validation remains environment-specific and intentionally excluded from default test runs.
- Non-`mpv` player integration is not represented as a first-class implemented runtime adapter in this workspace today, and that work is intentionally deferred behind `mpv` parity.
