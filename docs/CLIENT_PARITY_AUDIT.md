# Client Parity Audit

## Audit Date

- 2026-03-09

## Verification Performed

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` (`8/8` scenarios)
- `cargo build -p syncplay-gui --bin syncplay-gui`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`
- Static inspection of:
  - `crates/syncplay-gui/src/main.rs`
  - `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke.rs`
  - `crates/syncplay-gui/src/semantic_scenarios/*`
  - `scripts/gui-native-smoke.ps1`
  - `scripts/gui-semantic-suite.ps1`
- `git grep -n -I -e "drag" -e "drop" -- crates/syncplay-gui/src scripts` to confirm the current drag-and-drop surface

## Current Read On Parity

- Protocol/session core is no longer the main blocker. The GUI can now start and stop a real client-core TCP session from saved host/port settings, including startup auto-connect from persisted configuration and explicit connect/disconnect controls on both the configuration and main-window surfaces.
- The main parity blockers are now the remaining default desktop workflow gaps after connection exists: getting files into a real shared playlist from normal desktop affordances, drag-and-drop ingest, and tightening affordances that still over-promise beyond the runtime-backed paths.

## Highest-Priority Remaining Work

### 1. Replace Preview-Only Shared-Playlist File Opening With Real Runtime Dispatch

- `GuiRuntimeRequest::OpenMediaFiles { load_into_shared_playlist: true }` currently resolves to preview-only actions in the queued runtime owner.
- That path switches back to the main window and announces a shared playlist load in shell state, but it does not dispatch a player operation, a shared-playlist runtime action, or a Syncplay session message.
- The focused regression test `gui_persisted_config_runtime_owner_reports_player_runtime_gaps_explicitly` currently asserts this preview-only behavior.
- Result: opening files while shared playlist mode is enabled can update the visible playlist without actually loading media into a real player or session.

### 2. Implement Desktop Drag-And-Drop For Media And Playlist Ingest

- The current GUI uses `rfd::FileDialog` for manual file selection, but there is no `egui` dropped-file handling or alternate native drop hook in the app code.
- The semantic and native smoke paths do not cover drag-and-drop.
- Result: drag-and-drop parity is still absent rather than partially implemented.

### 3. Tighten Command Availability So The UI Stops Advertising Non-Working Paths

- `Open Media File` becomes enabled whenever shared playlists are enabled, even if there is no attached session or player runtime.
- Room join controls are now gated on an active session runtime, but some other affordances still remain preview-oriented.
- These affordances are useful for shell previews and tests, but they are misleading in the real app.
- Result: the current UI still exposes actions that look production-ready even when they stop at local state projection.

## What Is Already Solid Enough To Build On

- Saved host/port settings can now drive a real GUI connect/disconnect flow, including startup auto-connect and explicit session lifecycle controls.
- Detached public-server connect can bootstrap a real client-core TCP session from GUI state.
- Once a session exists, room join and return-to-default flows are runtime-backed, server-confirmed, and covered over TCP transport, including pre-Hello rejection paths.
- Attached-player media opening works for the non-shared-playlist path.
- Live Python interop and transport churn coverage still give good confidence in the client-core and session layers after connection.

## Secondary Work Outside The Immediate User Blockers

- Continue extracting the large GUI and client-core modules while parity work lands.
- Keep expanding automated real-`mpv` coverage.
- Validate the default GUI workflow cross-platform once the Windows path is functionally honest.
- Port non-`mpv` players only after the default `mpv` GUI workflow is no longer the blocker.

## Practical Priority Order

1. Turn shared-playlist file open and import into real runtime behavior.
2. Add drag-and-drop and cover it in native and semantic smoke tests.
3. Tighten command availability so non-working paths stop looking production-ready.
4. Resume broader parity and maintainability work after the default user flow is reliable.
