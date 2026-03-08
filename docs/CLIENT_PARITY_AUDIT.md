# Client Parity Audit

## Audit Date

- 2026-03-08

## Verification Performed

- `cargo test -p syncplay-gui gui_queued_runtime_bridge_and_preview_owner_cover_runtime_requests`
- `cargo test -p syncplay-gui gui_persisted_config_runtime_owner_reports_player_runtime_gaps_explicitly`
- `cargo test -p syncplay-gui gui_persisted_config_runtime_owner_bootstraps_detached_public_server_connect`
- `cargo test -p syncplay-gui gui_persisted_config_runtime_owner_routes_room_changes_over_tcp_transport`
- Static inspection of:
  - `crates/syncplay-gui/src/main.rs`
  - `crates/syncplay-gui/src/bin/syncplay-gui-native-smoke.rs`
  - `crates/syncplay-gui/src/semantic_scenarios/*`
  - `scripts/gui-native-smoke.ps1`
  - `scripts/gui-semantic-suite.ps1`
- `git grep -n -I -e "drag" -e "drop" -- crates/syncplay-gui/src scripts` to confirm the current drag-and-drop surface

## Current Read On Parity

- Protocol/session core is no longer the main blocker. The detached public-server path can bootstrap a real client-core TCP session, active-session room changes propagate over transport, and the smoke/interop suite still covers reconnect, chat, and playlist state once a runtime already exists.
- The main parity blockers are now the default desktop workflow: launching the GUI, connecting through saved settings, joining or switching rooms without lying to the user, and getting files into a real shared playlist from normal desktop affordances.

## Highest-Priority Remaining Work

### 1. Add A First-Class GUI Connect Path For Saved Host/Port Settings

- The only exposed GUI connect command is public-server connect.
- `gui_startup_host_and_settings()` only creates an attached TCP or loopback session runtime when explicit environment bootstrap flags are present.
- Normal configuration load still restores host, port, username, and room values, but it does not start a session from them.
- Result: booting the app from a normal saved configuration does not produce a connected client, so room, chat, readiness, and playlist controls begin from a disconnected shell.

### 2. Make Room Join/Leave Runtime-Authoritative Instead Of Optimistic Shell State

- `JoinMainWindowRoom` and `LeaveMainWindowRoom` immediately mutate visible shell state through `join_main_window_room()` and `leave_main_window_room()`.
- The real network operation is a separate queued `GuiRuntimeRequest::SetRoom`.
- The queued runtime owner only forwards that request when `self.session` exists; otherwise nothing happens and there is no rollback.
- Even when a session exists, failures such as "server Hello has not completed yet" currently appear only as a later notification after the local room label has already changed.
- Result: the GUI can claim that the user joined or left a room when no successful server-side room change happened.

### 3. Replace Preview-Only Shared-Playlist File Opening With Real Runtime Dispatch

- `GuiRuntimeRequest::OpenMediaFiles { load_into_shared_playlist: true }` currently resolves to preview-only actions in the queued runtime owner.
- That path switches back to the main window and announces a shared playlist load in shell state, but it does not dispatch a player operation, a shared-playlist runtime action, or a Syncplay session message.
- The focused regression test `gui_persisted_config_runtime_owner_reports_player_runtime_gaps_explicitly` currently asserts this preview-only behavior.
- Result: opening files while shared playlist mode is enabled can update the visible playlist without actually loading media into a real player or session.

### 4. Implement Desktop Drag-And-Drop For Media And Playlist Ingest

- The current GUI uses `rfd::FileDialog` for manual file selection, but there is no `egui` dropped-file handling or alternate native drop hook in the app code.
- The semantic and native smoke paths do not cover drag-and-drop.
- Result: drag-and-drop parity is still absent rather than partially implemented.

### 5. Tighten Command Availability So The UI Stops Advertising Non-Working Paths

- `Open Media File` becomes enabled whenever shared playlists are enabled, even if there is no attached session or player runtime.
- Room join controls are enabled whenever a draft room exists, regardless of session state.
- These affordances are useful for shell previews and tests, but they are misleading in the real app.
- Result: the current UI still exposes actions that look production-ready even when they stop at local state projection.

## What Is Already Solid Enough To Build On

- Detached public-server connect can bootstrap a real client-core TCP session from GUI state.
- Once a session exists, room changes over TCP transport are exercised and verified.
- Attached-player media opening works for the non-shared-playlist path.
- Live Python interop and transport churn coverage still give good confidence in the client-core and session layers after connection.

## Secondary Work Outside The Immediate User Blockers

- Continue extracting the large GUI and client-core modules while parity work lands.
- Keep expanding automated real-`mpv` coverage.
- Validate the default GUI workflow cross-platform once the Windows path is functionally honest.
- Port non-`mpv` players only after the default `mpv` GUI workflow is no longer the blocker.

## Practical Priority Order

1. Add saved-config connect, disconnect, and session lifecycle to the GUI.
2. Make room join and leave reflect server-confirmed state rather than local optimism.
3. Turn shared-playlist file open and import into real runtime behavior.
4. Add drag-and-drop and cover it in native and semantic smoke tests.
5. Resume broader parity and maintainability work after the default user flow is reliable.
