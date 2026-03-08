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

The Windows native smoke passed overall. Native Win32 menu enumeration is still skipped on this machine (`menu_contract=skipped-no-native-menu`), but the interaction trace now exercises the user-visible `Open Media File` flow without a skip path.

## What Is Already Verified

- The Rust CLI startup/help surface matches the upstream Python client surface for the standard `syncplayClient.py` flags.
- Managed `mpv` startup now follows Python-style `playerPath` resolution and `perPlayerArguments` routing across stored config, CLI overrides, managed launch, and explicit JSON-IPC attach mode.
- `syncplay-client-core` has strong coverage for handshake, readiness, chat, room changes, controlled rooms, playlist operations, desync correction, reconnect state restoration, privacy modes, and user-facing notifications.
- `syncplay-client-app` covers legacy config parsing, `syncplay.ini` round-trips, QSettings cleanup, local command compatibility, and runtime language/config compatibility helpers.
- `syncplay-gui` has a working configuration/main-window shell, semantic smoke scenarios, Windows native smoke coverage, persistence reset coverage, detached GUI runtime ownership for public-server connect/refresh/search flows, verified remote update/public-server service behavior, and live Python interop scenarios against the legacy Syncplay server.
- `syncplay-server` is not the current critical path, but the runtime library is already substantial and test-covered enough to support current client work.

## Current Scope

Complete `mpv` client parity is the immediate goal.

- The active scope is `mpv` behavior, startup, GUI integration, and end-to-end parity against the upstream Python client.
- Other player backends are deferred until the Rust `mpv` path is considered complete enough to stop being the primary blocker.

## Recently Closed

### GUI and Background Parity

The previously-audited GUI/background gaps in this bucket are now covered:

- GUI room history can be edited/restored from the configuration surface and persisted through the GUI-owned state files.
- GUI public-server background/cache behavior now has startup seeding, manual refresh coverage, and deterministic semantic/native smoke coverage.
- Automatic update probing/dialog behavior is implemented, persists `lastCheckedForUpdates`, and is validated through unit coverage plus GUI semantic/native smoke passes.
- The user-visible `Open Media File` flow is exercised by the Windows native smoke without a skip step.

### Headless `mpv` OSD Runtime Parity

The headless `mpv` path now applies a first real slice of the previously GUI-only runtime settings:

- `showOSD`
- `chatOutputEnabled`
- `chatMoveOSD`
- `chatOSDMargin`
- `notificationTimeout`
- `alertTimeout`
- `chatTimeout`

These settings now drive CLI/headless `mpv` runtime behavior for both managed launch and explicit JSON-IPC attach mode:

- localized Syncplay notifications and chat output are mirrored to `mpv` OSD with `show-text`
- chat output can remain visible even when general `showOSD` is disabled, matching the upstream split between chat output and general notifications
- the standard `mpv` OSD is moved away from the chat area when the stored chat/OSD layout rules require it

## Highest-Priority Remaining Work

### 1. GUI-Only Settings vs Runtime Behavior

The storage-only portion of this gap is smaller now, but it is not fully closed yet. The remaining no-op area is the advanced script-driven `mpv` chat UI surface rather than the basic OSD/runtime toggles.

Still unresolved:

- advanced chat presentation settings such as `chatOutputMode`, fonts, chat margins, and chat history layout are still not rendered by the Rust headless path
- mpv-side chat input behavior (`chatInputEnabled`, `chatDirectInput`, and related input styling/options) is still not supported because the current Rust adapter is JSON-IPC-only and does not consume the Lua script stdout/control path

The next decision for this bucket is still the same:

- either extend Rust headless mode with a safe Lua/script integration path for those remaining settings
- or document the remaining script-driven chat/input surface as intentionally out of scope for Rust headless mode

### 2. Maintainability Risk While Closing Parity

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

1. Revisit GUI-only runtime-setting parity decisions after the player/runtime path is steadier.
2. Keep extracting large modules as parity work lands.
3. Port non-`mpv` player backends after `mpv` parity is complete.
4. Expand broader GUI/client workflow validation after the Windows-first path is no longer the main blocker.
