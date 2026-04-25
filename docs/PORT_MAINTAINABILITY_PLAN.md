# Port Maintainability Plan

Current code-quality plan for keeping the Rust Syncplay port shippable while preserving Python
Syncplay parity.

## Audit Snapshot

- Audited from `syncplay-rs/` on 2026-04-25.
- The canonical Rust workspace is this directory. The outer workspace-level
  `../crates/syncplay-gui/src/app_runtime_localization.rs` file is historical/debris context and
  is not compiled by this Cargo workspace.
- Baseline gates before this cleanup were green:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo test --workspace`
  - `cargo test --workspace --all-features`
  - `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`
  - `cargo build -p syncplay-gui --bin syncplay-gui`
  - `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`

## Current Read

- The workspace is mechanically healthy: Rust 1.95, edition 2024, strict Clippy, broad unit
  coverage, GUI semantic smoke, live Python interop, and Windows native smoke are all active.
- The largest quality risk is maintainability drift, especially in `syncplay-gui`, not immediate
  broken behavior.
- `syncplay-client-app::app_boundary` is the supported cross-crate API for CLI/GUI consumers.
  Legacy modules under that crate are implementation details.
- The active product scope remains `mpv` first. Non-`mpv` player backends are deferred unless
  product scope is deliberately reopened.

## Implemented Cleanup Baseline

- Workspace dependency ownership should live in `[workspace.dependencies]` where possible.
- Each crate inherits the workspace lint baseline through `[lints] workspace = true`.
- The lint baseline intentionally avoids blanket `clippy::pedantic`/`nursery`; those remain audit
  tools. Promote only high-signal lints that the workspace can keep green.
- `syncplay-client-app` should expose `app_boundary` publicly and keep legacy implementation
  modules private.
- Platform-specific fallbacks should use `#[cfg]` rather than suppressing unreachable-code lints.
- Production unsafe blocks should carry `SAFETY:` comments before enabling
  `clippy::undocumented_unsafe_blocks` as a hard gate.

## Working Order

1. Keep docs and source-of-truth references current.
   - Update this file when a cleanup class is closed or deferred.
   - Keep `README.md`, `PROJECT_STATUS.md`, and parity/audit docs pointing at real paths.

2. Tighten low-risk Rust hygiene.
   - Prefer deleting unused deps over leaving manifest drift.
   - Fix redundant clones/closures, needless pass-by-value, and unnecessary `Result` returns when
     the surrounding behavior is already covered by tests.
   - Convert invariant runtime `expect` calls to explicit error handling when the state can be
     invalid due to user input, IO, or runtime ordering.

3. Keep API boundaries narrow.
   - Add shared CLI/GUI behavior to `syncplay-client-app::app_boundary`, not directly to legacy
     modules.
   - Do not widen visibility solely for tests; use source-owned test modules.
   - Prefer `pub(super)` or private items inside private module trees.

4. Refactor hotspots only when touched.
   - `syncplay-gui`: continue splitting request/action/runtime leaves by domain; box large enum
     variants only when it removes a real lint or review burden.
   - `syncplay-player-mpv`: split adapter behavior into command building, event handling,
     property polling, open-file flow, and state storage when making mpv changes.
   - `syncplay-client-core`: split `session/helpers.rs` by playlist, reconnect, desync, autoplay,
     and metadata when those areas change.
   - `syncplay-server`: move binary CLI parsing/config assembly out of `main.rs` when server CLI
     parity work resumes.
   - `syncplay-compat`: keep behavior stable; split interop helpers only alongside parity changes.

5. Consolidate test-only unsafe environment mutation.
   - Existing tests should move toward a mutex-backed helper for scoped env overrides.
   - New tests must not add ad hoc unsafe env mutation blocks.

## Validation Expectations

For ordinary cleanup:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --all-features`

For GUI-affecting cleanup:

- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`
- `cargo build -p syncplay-gui --bin syncplay-gui`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`

For mpv adapter/startup cleanup:

- Run the targeted unit tests first.
- Run ignored real-`mpv` smoke tests when `SYNCPLAY_MPV_SMOKE_BIN` and media fixtures are
  available.

## Non-Goals

- No broad rewrite for style alone.
- No new non-`mpv` backend work as part of maintainability cleanup.
- No blanket pedantic lint gate.
- No public API expansion unless a CLI/GUI/shared consumer actually needs it.
