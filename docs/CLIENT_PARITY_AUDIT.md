# Client Parity Audit

## Audit Date

- 2026-03-10

## Verification Performed

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` (`9/9` scenarios)
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
- Shared-playlist file opening/import no longer stops at shell projection. The queued runtime owner now routes those requests through the real session/player runtime path, including playlist-file import and player dispatch when an attached playback runtime exists.
- GUI command availability is now aligned with runtime-backed capability instead of config-only projection. `Open Media File`, playback actions, quick-open, and playlist-target drag/drop stay disabled until a working playback or playlist runtime actually arrives.
- Desktop drag-and-drop ingest still covers both detached media-open and shared-playlist import flows, with semantic coverage and Windows native smoke coverage.
- The main remaining work is now broader release, cross-platform, and maintainability follow-through rather than default-flow honesty.

## Recently Completed

### Tighten Command Availability So The UI Stops Advertising Non-Working Paths

- Config-only shell state no longer treats a saved player path or shared-playlist checkbox as proof of a live playback/runtime connection.
- `Open Media File`, playback menu items, and quick-open remain disabled until runtime snapshots expose working playback or playlist support.
- Shared-playlist drop targeting now requires actual runtime playlist control instead of only a configuration toggle.
- Semantic coverage now asserts both the disabled baseline and the runtime-enabled transition, and the Windows native smoke suite verifies the gated pre-runtime state plus runtime-backed drag/drop ingest.

### Desktop Drag-And-Drop For Media And Playlist Ingest

- The native `egui` host now accepts dropped files and routes them either to detached media-open or shared-playlist ingest based on the drop target.
- Playlist-surface drops import media entries and playlist files; window-target drops open media through the detached/player-backed path.
- Semantic coverage now includes a dedicated drag-and-drop scenario, and the Windows native smoke suite validates both window-target and playlist-target ingest.

## Highest-Priority Remaining Work

- Continue extracting the large GUI and client-core modules while parity work lands.
- Keep expanding automated real-`mpv` coverage.
- Validate the default GUI workflow cross-platform now that the Windows path is functionally honest.
- Port non-`mpv` players only after the default `mpv` GUI workflow is no longer the blocker.

## What Is Already Solid Enough To Build On

- Saved host/port settings can now drive a real GUI connect/disconnect flow, including startup auto-connect and explicit session lifecycle controls.
- Detached public-server connect can bootstrap a real client-core TCP session from GUI state.
- Once a session exists, room join and return-to-default flows are runtime-backed, server-confirmed, and covered over TCP transport, including pre-Hello rejection paths.
- Shared-playlist file opening/import now dispatches through the real runtime owner, including session-backed playlist replacement, playlist-file import, and attached-player opening of the first selected file when a playback runtime is present.
- Attached-player media opening works for the non-shared-playlist path.
- Live Python interop and transport churn coverage still give good confidence in the client-core and session layers after connection.

## Secondary Work Outside The Immediate User Blockers

- Continue extracting the large GUI and client-core modules while parity work lands.
- Keep expanding automated real-`mpv` coverage.
- Validate the default GUI workflow cross-platform once the Windows path is functionally honest.
- Port non-`mpv` players only after the default `mpv` GUI workflow is no longer the blocker.

## Practical Priority Order

1. Resume broader parity and maintainability work now that the default GUI flow stops advertising non-working paths.
2. Keep packaging, cross-platform validation, and real-`mpv` follow-through moving as the next user-facing reliability gates.
