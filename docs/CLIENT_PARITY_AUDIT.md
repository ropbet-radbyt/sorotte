# Client Parity Audit

## Audit Date

- 2026-03-10

## Verification Performed For This Refresh

- Reviewed the existing same-day verification record already captured in this audit's previous revision:
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` (`9/9` scenarios)
  - `cargo build -p syncplay-gui --bin syncplay-gui`
  - `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`
- Static comparison of the Python client reference:
  - `../syncplay/syncplay/client.py`
  - `../syncplay/syncplay/ui/gui.py`
  - `../syncplay/syncplay/ui/GuiConfiguration.py`
  - `../syncplay/syncplay/utils.py`
- Static comparison of the Rust client implementation:
  - `crates/syncplay-gui/src/main.rs`
  - `crates/syncplay-gui/src/remote_services.rs`
  - `crates/syncplay-client-core/src/lib.rs`
  - `crates/syncplay-client-app/src/legacy_settings.rs`
  - `crates/syncplay-client-app/src/legacy_language.rs`
  - `crates/syncplay-player-api/src/lib.rs`
  - `crates/syncplay-player-mpv/src/lib.rs`

## Current Read On Parity

- The previous version of this audit understated the amount of Python-client work that still remains on the Rust client side.
- The Rust client-core is materially ahead of the Rust GUI. A meaningful slice of Python behavior already exists in `syncplay-client-core` and `syncplay-client-app`, but the GUI still exposes only a subset of it.
- The default Python-style GUI workflow is still not at parity. The biggest blockers are:
  - the GUI does not launch and own the configured player from saved settings,
  - the main window does not project Python's room/user/file browser,
  - many Python playlist/controller/offset/undo workflows are still missing or only exist as shell-only mutations,
  - the language setting is mostly persistence-only because runtime text is still English,
  - only `mpv` is represented as a first-class Rust player backend today.

## What No Longer Needs Assignment

- Saved host/port settings can drive a real GUI connect/disconnect flow, including startup auto-connect.
- Room join and return-to-default flows are runtime-backed over a real session.
- Shared-playlist import/open now routes through the real runtime owner instead of stopping at shell projection.
- Detached media-open and shared-playlist drag-and-drop ingest are covered by semantic and Windows native smoke flows.
- Public-server browsing, refresh, custom-entry editing, and runtime-backed connect flows exist.
- Missing-media search exists as a real GUI flow.
- TLS prompt, update-check, chat, reconnect, and controlled-room interop coverage are present.
- The client-core already implements more than the GUI exposes, including controller-auth requests, set-others-readiness, undo-seek, playlist undo, and playlist shuffle operations.

## Remaining Python Client Parity Tasks

### P0. Launch and own `mpv` from GUI settings

Current status: `syncplay-gui` only attaches to an explicit `mpv` JSON IPC path or a test player. The saved `playerPath` and `perPlayerArguments` values are not yet used to start a player from the GUI, so the default Python-style "configure player, start GUI, open media" flow is still blocked.

Work to assign:

- Start `mpv` from the saved `playerPath` instead of requiring `SYNCPLAY_CLIENT_MPV_IPC_PATH` or `SYNCPLAY_MPV_IPC_PATH`.
- Translate and apply saved per-player arguments in the GUI-owned launch path.
- Manage player lifecycle, IPC bootstrap, reconnect/relaunch, and user-facing launch failures.
- Apply legacy Syncplay `mpv` UI settings in the GUI-owned player path, including OSD/chat placement and timeout behavior.
- Add semantic/native coverage plus a real-`mpv` scripted smoke for the no-manual-env-vars GUI startup path.

### P0. Replace the shell-style main window with Python's room/user/file browser

Current status: the Rust main window only tracks `username`, `is_ready`, and `is_controller` per user. The Python GUI shows room grouping, controlled-room icons, filesize/duration/filename columns, file-difference highlighting, URL trust cues, and hide-empty-room behavior.

Work to assign:

- Project room-grouped runtime state into the GUI instead of a flat in-room username list.
- Show per-user file metadata and controlled-room state.
- Port file-difference highlighting and "no file" states.
- Port row actions that Python exposes from the room/user browser: join room, open/switch to another user's file or stream, open containing folder, and add trusted domain from a URL.
- Add hide-empty-rooms behavior once room grouping exists.
- Remove or replace the current shell-only add/edit/remove-user controls, which are not Python-client parity behavior.

### P1. Port main-window playback, autoplay, and offset workflows

Current status: the Rust GUI exposes pause toggle, ready toggle, and seek-by-offset. The Python GUI also exposes play/pause buttons, undo seek, autoplay controls, and set-offset workflows.

Work to assign:

- Add explicit play and pause actions instead of only a toggle.
- Wire undo-seek through the existing client-core capability.
- Reuse the existing offset-command/runtime logic for a real GUI `Set Offset` flow.
- Add Python-style autoplay controls and feedback in the main window, not just a passive status field.
- Add the corresponding File/Playback/Window menu entries and toolbar/button affordances that Python exposes.

### P1. Port Python playlist workflows instead of only the minimal shared-playlist slice

Current status: the Rust GUI wires queue/select/remove/reorder, and shared-playlist file import now works. The Python GUI also exposes shuffle remaining, shuffle entire, undo playlist change, add URLs, edit playlist text, dedicated load/save playlist dialogs, and richer playlist context actions.

Work to assign:

- Wire the existing client-core operations for shuffle remaining, shuffle entire, and undo playlist change into the GUI.
- Add URL entry dialogs for playlist items and detached/open-file URL flows.
- Add a real playlist text editor flow rather than only per-row editing.
- Add dedicated load/save playlist dialogs. The current shared-playlist import path only covers part of Python's "load playlist from file" behavior and does not cover save/export.
- Port playlist context actions: open selected item, open containing folder, add trusted domain, load-and-shuffle-from-file, and other Python menu affordances.

### P1. Port controlled-room and controller-auth UX

Current status: auto-auth from a stored controlled-room password works. Python also lets the user create a controlled room and manually identify as controller from the GUI. The Rust GUI currently still has shell-style controller toggles instead of Python's controller-auth flows.

Work to assign:

- Add the create-controlled-room dialog and surface the generated room/password information.
- Add manual identify-as-controller and retry flows.
- Replace shell-only controller-state toggles with runtime-backed controller-auth requests and result handling.
- Wire set-others-readiness from the user list/context menu when the server supports it.

### P1. Port Python slash-command handling in the GUI chat box

Current status: the Python GUI chat box routes slash-prefixed local commands before falling back to chat send. The Rust GUI currently treats the field as chat-only input.

Work to assign:

- Reuse `syncplay-client-app` local-command parsing in the GUI path.
- Support the Python-visible seek, undo, offset, playlist, readiness, and controller-related commands.
- Preserve normal chat behavior for non-command text and literal `/` handling.
- Add semantic coverage that proves commands no longer fall through as chat messages.

### P2. Finish configuration dialog parity

Current status: Rust persists a broad legacy settings model, but the GUI only exposes part of it. Several Python settings are stored in the Rust model or supported elsewhere in the workspace without a matching GUI control.

Work to assign:

- Add the missing player/startup behavior controls: per-player arguments, loop-at-end-of-playlist, loop-single-files, autosave-joins-to-list, and force-GUI-prompt behavior.
- Add the missing chat/OSD appearance and timing controls: chat input position, chat output mode, font size/weight/color, margins, notification timeout, alert timeout, chat timeout, and slowdown OSD.
- Decide whether room-list/media-directory/trusted-domain editing should remain split across separate Rust views or be brought back into Python-like dialog flows, and then make that surface consistent.

### P2. Localize runtime strings and language-sensitive service calls

Current status: language tags normalize and persist, but most runtime text is still English. Some public-server refresh paths also hardcode English instead of using the selected language.

Work to assign:

- Port the Python message catalog behavior for GUI/runtime notifications, dialogs, and help/error text.
- Use the active GUI language in every public-server and update-check request path.
- Add at least one non-English smoke scenario so language selection stops being a persistence-only feature.

### P2. Additional player backend parity

Current status: the Rust workspace only has a first-class `mpv` backend. The Python client supports `mpv`, `mpvnet`, `MPC-HC`, `MPC-BE`, `VLC`, `MPlayer`, `IINA`, and `Memento`.

Work to assign:

- Decide whether "client parity" still means Python's full player matrix or an explicit `mpv`-only milestone.
- If full parity remains the target, split ports by platform:
  - Windows: `mpvnet`, `MPC-HC`, `MPC-BE`
  - macOS: `IINA`
  - cross-platform/legacy: `VLC`, `MPlayer`, `Memento`

## Practical Assignment Order

1. GUI-owned `mpv` launch and legacy `mpv` UI/OSD behavior.
2. Python-style room/user/file browser in the main window.
3. Playback/autoplay/offset/undo parity in the GUI.
4. Playlist workflows and context menus.
5. Controlled-room/controller-auth UX and set-others-readiness.
6. GUI slash-command handling.
7. Configuration dialog completion.
8. Localization and language-sensitive service calls.
9. Additional player backends.

## Outside Strict Python-Client Feature Parity

These still matter, but they should not replace the feature tasks above in a client-parity tracker:

- End-to-end release packaging, installers, signing, and changelog flow.
- Cross-platform GUI validation beyond the current Windows-heavy evidence.
- Automated real-`mpv` smoke coverage in CI or a stricter scripted release gate.
- Ongoing refactor work for very large modules in `syncplay-gui`, `syncplay-cli`, and `syncplay-client-core`.
