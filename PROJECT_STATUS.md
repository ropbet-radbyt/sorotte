# syncplay-rs Project Status

Audit snapshot for the Rust Syncplay rewrite.

## Audit date

- 2026-03-08

## What was verified in this audit

- `cargo test --workspace` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` passed (`7/7` scenarios).
- `cargo build -p syncplay-gui --bin syncplay-gui` passed.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000` passed.
- The native smoke interaction trace still had to skip menu-driven `Open Media File` invocation because the action was not exposed as an enabled native menu/control.
- `cargo run --quiet -p syncplay-cli -- --help` matches the upstream Python client flag surface.
- `cargo run --quiet -p syncplay-server -- --help` prints a real Rust alpha CLI help surface.
- Local real-`mpv` smoke tests are present but remain `ignored` by default (manual environment-dependent validation).

## Summary

`syncplay-rs` is well beyond a skeleton rewrite: it has a verified CLI client, a GUI shell with semantic/native smoke coverage and live Python interop coverage, typed protocol handling, compatibility-focused tests, and a real `mpv` adapter. The project is not yet a full end-user replacement for Syncplay because complete `mpv` parity, fully detached GUI runtime ownership, and several legacy GUI/background behaviors are still outstanding. Non-`mpv` players are currently deferred until `mpv` parity is complete.

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
- [x] GUI configuration/main-window shell with semantic smoke coverage and Windows native accessibility smoke coverage.
- [x] Live Python GUI interop scenarios for readiness/chat/playlist/reconnect/controller flows against the legacy Syncplay server.
- [x] Compatibility/interop test infrastructure comparing Rust runtime behavior to captured Python Syncplay traces/scenarios.
- [x] Server features with test coverage for room/state fanout, controlled rooms, playlist scoping, TLS upgrade paths, and persistent/permanent room behavior.
- [x] Rust server executable alpha entrypoint with `--help`, core startup flags, and listener/network-loop startup wiring over the server runtime.
- [x] CI/automation basics (`rust-ci.yml`) and coverage workflow (`rust-coverage.yml`), plus local cargo aliases in `.cargo/config.toml`.

## Remaining work (priority checklist)

- [ ] Let GUI public-server connect/refresh and missing-media search work without requiring an already attached session runtime.
- [ ] Close the remaining startup/player-launch parity gaps called out as partial in the compatibility matrix (`playerPath`, `perPlayerArguments`, finite explicit-IPC argument translation subset).
- [ ] Implement the remaining GUI/background behaviors still called out as unimplemented in compatibility notes (server-browser behavior, background cache refresh, room-history management, update probing).
- [ ] Make menu-driven `Open Media File` reliably available through the native GUI/accessibility surface instead of relying on a skipped native-smoke step.
- [ ] Decide whether GUI-only stored settings that are currently storage-compatible/no-op in headless mode need real runtime behavior for parity, or should stay explicitly out of scope.
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
- The GUI is real and test-covered, but some operations still depend on an already attached runtime or partial compatibility layers.
- The server runtime library remains further along than the user-facing `syncplay-server` CLI parity surface, even though a real alpha executable entrypoint now exists.
- Real `mpv` integration exists, but some validation remains environment-specific and intentionally excluded from default test runs.
- Non-`mpv` player integration is not represented as a first-class implemented runtime adapter in this workspace today, and that work is intentionally deferred behind `mpv` parity.
