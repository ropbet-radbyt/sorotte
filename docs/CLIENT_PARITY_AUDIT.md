# Client Parity Audit

## Audit Date

- 2026-03-08

## Verification Performed

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`
- `cargo build -p syncplay-gui --bin syncplay-gui`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`
- `cargo run --quiet -p syncplay-cli -- --help`
- `python syncplayClient.py --help`
- `cargo test -p syncplay-cli explicit_mpv_ipc_cli_startup_smoke -- --ignored`
- `cargo test -p syncplay-cli managed_mpv_cli_smoke -- --ignored`

The Windows native smoke passed overall, but its interaction trace still had to skip menu-driven `Open Media File` invocation because the action was not exposed as an enabled native menu/control.

## What Is Already Verified

- The Rust CLI startup/help surface matches the upstream Python client surface for the standard `syncplayClient.py` flags.
- Managed `mpv` startup now follows Python-style `playerPath` resolution and `perPlayerArguments` routing across stored config, CLI overrides, managed launch, and explicit JSON-IPC attach mode.
- `syncplay-client-core` has strong coverage for handshake, readiness, chat, room changes, controlled rooms, playlist operations, desync correction, reconnect state restoration, privacy modes, and user-facing notifications.
- `syncplay-client-app` covers legacy config parsing, `syncplay.ini` round-trips, QSettings cleanup, local command compatibility, and runtime language/config compatibility helpers.
- `syncplay-gui` has a working configuration/main-window shell, semantic smoke scenarios, Windows native smoke coverage, persistence reset coverage, detached GUI runtime ownership for public-server connect/refresh/search flows, and live Python interop scenarios against the legacy Syncplay server.
- `syncplay-server` is not the current critical path, but the runtime library is already substantial and test-covered enough to support current client work.

## Current Scope

Complete `mpv` client parity is the immediate goal.

- The active scope is `mpv` behavior, startup, GUI integration, and end-to-end parity against the upstream Python client.
- Other player backends are deferred until the Rust `mpv` path is considered complete enough to stop being the primary blocker.

## Highest-Priority Remaining Work

### 1. Remaining GUI and Background Parity

The repo’s own compatibility notes still call out several client-facing gaps:

- GUI room-history management remains unimplemented.
- GUI background cache refresh remains unimplemented.
- Automatic update probing/dialog behavior is not yet a verified remote feature.

These are smaller than the core runtime and player gaps, but they still block "full client parity."

Done means:

- Room history can be edited/restored from the GUI in a predictable way.
- Missing-media/background search refresh works end to end.
- Update-check behavior is either implemented and tested or explicitly scoped out.
- Menu-driven `Open Media File` is exposed reliably enough to be exercised by the native smoke without a skip path.

### 2. GUI-Only Settings vs Runtime Behavior

Several GUI-oriented settings are storage-compatible today but intentionally no-op in the headless path, including groups of chat presentation and OSD layout settings. That is acceptable for config compatibility, but not equivalent to full runtime parity.

This is a lower-priority gap than players/runtime ownership, but it still needs an explicit decision:

- either wire those settings into real runtime behavior where parity matters
- or document them as intentionally out of scope for Rust headless mode

### 3. Maintainability Risk While Closing Parity

Parity work is landing in very large modules:

- `crates/syncplay-cli/src/main.rs`
- `crates/syncplay-client-core/src/lib.rs`
- `crates/syncplay-server/src/lib.rs`

This is not a parity gap by itself, but it is a delivery risk. New feature work should keep extracting coherent submodules instead of making the monoliths even harder to reason about.

## Secondary Work Outside the Current Client Critical Path

- Port non-`mpv` player backends after `mpv` parity is complete.
- Expand `syncplay-server` CLI/runtime parity beyond the current alpha slice.
- Automate more real-`mpv` coverage in CI instead of relying on ignored/manual smokes.
- Validate GUI/client workflows more broadly outside the current Windows-first smoke path.

## Practical Priority Order

1. Close remaining GUI room-history/background/update gaps.
2. Keep extracting large modules as those features land.
3. Revisit GUI-only runtime-setting parity decisions after the player/runtime path is steadier.
4. Revisit non-`mpv` players only after the `mpv` path is no longer the main parity blocker.
