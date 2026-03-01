# syncplay-rs Project Status

Audit snapshot for the Rust Syncplay rewrite.

## Audit date

- 2026-02-24

## What was verified in this audit

- `cargo test --workspace` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- Local `mpv` smoke tests are present but remain `ignored` by default (manual environment-dependent validation).
- `target/debug/syncplay-server.exe --help` prints a real CLI help surface.
- `target/debug/syncplay-server.exe --port 0` starts the listener/network loop (startup smoke verified via timeout-killed process after bind).

## Summary

`syncplay-rs` is already beyond a skeleton rewrite: it has a working headless client stack, a substantial tested server runtime library, typed protocol handling, compatibility-focused tests, and a real `mpv` adapter. The project is not yet a full end-user replacement for Syncplay because GUI/runtime parity, packaging polish, broader validation, and full Python server CLI/operational parity are still outstanding.

## Documentation set (current)

These are the Markdown files that should remain in the repo root:

- `README.md` (overview + commands)
- `PROJECT_STATUS.md` (this audit + checklist)
- `ALPHA_CLI_PREVIEW.md` (developer/alpha run and packaging guide)

Older planning/handoff docs have been archived outside this repo (workspace `old-docs/`) and are not canonical project status.

Workspace-level parity audit/checklist docs are maintained one directory up:

- `../PROJECT_STATUS.md`
- `../PARITY_CHECKLIST.md`

## Completed (checked)

- [x] Cargo workspace with separated crates for protocol, client core, server, player API, `mpv` adapter, CLI, compat, and simulation support.
- [x] Headless CLI client binary (`syncplay-cli`).
- [x] Substantial server runtime library implementation and coverage in `crates/syncplay-server`.
- [x] Typed Syncplay protocol message models and fixture decoding coverage (`Hello`, `Set`, `List`, `State`, `Chat`, `Error`, `TLS` families).
- [x] Client session logic with reconnect/state restoration behaviors covered by tests.
- [x] Playlist and local command handling in the CLI (including controller/playlist command paths covered by tests).
- [x] `mpv` JSON IPC integration with attach/control/property updates and unit coverage in `syncplay-player-mpv`.
- [x] Managed `mpv` launch and explicit-IPC attach flows (with additional real-`mpv` smokes available as ignored tests).
- [x] Compatibility/interop test infrastructure comparing Rust runtime behavior to captured Python Syncplay traces/scenarios.
- [x] Server features with test coverage for room/state fanout, controlled rooms, playlist scoping, TLS upgrade paths, and persistent/permanent room behavior.
- [x] Rust server executable alpha entrypoint with `--help`, core startup flags, and listener/network-loop startup wiring over the server runtime.
- [x] CI/automation basics (`rust-ci.yml`) and coverage workflow (`rust-coverage.yml`), plus local cargo aliases in `.cargo/config.toml`.

## Remaining work (priority checklist)

- [ ] Expand `syncplay-server` CLI/runtime parity beyond the current alpha slice (remaining gaps include dual-interface binding parity and binary-level operational smoke coverage; `--password` now accepts both raw and Python-style MD5 tokens for compatibility).
- [ ] GUI client/runtime implementation (or explicit decision to scope this repo to CLI/server only).
- [ ] GUI settings/config parity strategy and implementation (current compatibility is CLI/headless-focused and partial).
- [ ] End-to-end release packaging process (artifacts, versioning, changelog, signing strategy if needed).
- [ ] Automated real-`mpv` smoke coverage in CI (or documented repeatable manual gate with scripts + fixtures).
- [ ] Cross-platform validation beyond the current Windows-oriented alpha workflow (`ALPHA_CLI_PREVIEW.md`).
- [ ] Public user-facing configuration reference (flags/env vars) for `syncplay-cli` and `syncplay-server`.
- [ ] Refactor/maintainability work for very large modules (notably `crates/syncplay-cli/src/main.rs`) to reduce change risk.
- [ ] Production-readiness hardening: observability defaults, failure recovery docs, and long-run soak guidance formalized.

## Optional/next improvements

- [ ] Add a small `docs/` index or generated command reference to keep README concise as the CLI surface grows.
- [ ] Add a compatibility matrix table (Python Syncplay feature vs Rust status) sourced from tests to replace ad-hoc notes.
- [ ] Track manual alpha validation results by date/build in a compact changelog section (instead of freeform notes).

## Notes on scope

- Current evidence supports "substantially implemented CLI/headless rewrite," not "full replacement" parity.
- The server runtime library remains further along than the user-facing `syncplay-server` CLI parity surface, even though a real alpha executable entrypoint now exists.
- Real `mpv` integration exists, but some validation remains environment-specific and intentionally excluded from default test runs.
- Non-`mpv` player integration is not represented as a first-class implemented runtime adapter in this workspace today.
