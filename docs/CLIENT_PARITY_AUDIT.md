# Client Parity Audit

## Audit Date

- 2026-03-11

## Verification Performed For This Refresh

- `cargo fmt --all`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` (`10/10` scenarios)
- `cargo build -p syncplay-gui --bin syncplay-gui`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`
- Local real-`mpv` managed-startup smoke:
  - `cargo test -p syncplay-gui gui_persisted_config_runtime_owner_starts_real_managed_mpv_from_saved_config -- --ignored`
  - run with `SYNCPLAY_MPV_SMOKE_BIN=C:\Users\shaun\Documents\workspace\syncplay-rust\mpv\mpv.exe`
- Static comparison of the Python client reference:
  - `../syncplay/syncplay/client.py`
  - `../syncplay/syncplay/ui/gui.py`
  - `../syncplay/syncplay/ui/GuiConfiguration.py`
  - `../syncplay/syncplay/utils.py`
- Static comparison of the Rust client implementation:
- `crates/syncplay-gui/src/app.rs`
  - `crates/syncplay-gui/src/remote_services.rs`
  - `crates/syncplay-client-core/src/lib.rs`
  - `crates/syncplay-client-app/src/legacy_settings.rs`
  - `crates/syncplay-client-app/src/legacy_language.rs`
  - `crates/syncplay-player-api/src/lib.rs`
  - `crates/syncplay-player-mpv/src/lib.rs`

## Current Read On Parity

- The Rust client-core is still ahead of the Rust GUI overall, but the GUI now covers the main-window playback/autoplay/offset slice that had been one of the most obvious P1 gaps.
- The default `mpv`-backed GUI startup path is no longer blocked on manual environment setup. Saved `playerPath` plus `perPlayerArguments` now drive a GUI-owned `mpv` launch, legacy Syncplay `mpv` OSD/chat settings are applied, GUI notifications/chat are mirrored into `mpv`, and the GUI owns relaunch/failure handling for that path.
- The biggest remaining parity blockers are now:
  - GUI slash-command handling still trails the Python client,
  - the language setting is mostly persistence-only because runtime text is still English,
  - only `mpv` is represented as a first-class Rust player backend today.

## What No Longer Needs Assignment

- The GUI now launches and owns saved-config `mpv` without requiring `SYNCPLAY_CLIENT_MPV_IPC_PATH` or `SYNCPLAY_MPV_IPC_PATH`.
- Saved `perPlayerArguments` are applied in the GUI-owned `mpv` launch path.
- Legacy Syncplay `mpv` UI settings now apply in both explicit-IPC attach mode and GUI-owned startup, including chat input/output and timeout-backed OSD behavior.
- GUI chat/system notifications now forward into attached `mpv` via the legacy OSD/chat path, and `mpv` chat input now routes back into the GUI session runtime.
- GUI-owned `mpv` lifecycle is managed across startup, save/reload/reset, on-demand reopen, and unexpected process exit reporting.
- Saved host/port settings can drive a real GUI connect/disconnect flow, including startup auto-connect.
- Room join and return-to-default flows are runtime-backed over a real session.
- Detached GUI sessions now keep legacy-server connections alive with periodic `State.ping` heartbeats, so a successful join no longer drops after the initial timeout window.
- Shared-playlist import/open now routes through the real runtime owner instead of stopping at shell projection.
- Detached media-open and shared-playlist drag-and-drop ingest are covered by semantic and Windows native smoke flows.
- Public-server browsing, refresh, custom-entry editing, and runtime-backed connect flows exist.
- Missing-media search exists as a real GUI flow.
- The main window now projects a Python-style room/user/file browser, including room grouping, per-user file metadata and difference cues, runtime-backed room/file/folder/trusted-domain actions, and hide-empty-room behavior.
- Main-window playback parity now includes explicit play/pause actions, undo seek, set-offset prompts, autoplay toggle/threshold controls, countdown/status feedback, and persisted playback/autoplay control visibility.
- Python playlist workflow parity now includes shuffle remaining, shuffle entire, undo playlist change, add-URL/open-URL flows, playlist text editing, dedicated load/save playlist dialogs, and playlist context actions for opening selected items, opening containing folders, and trusting selected playlist domains.
- The matching Playback/Advanced/Window menu affordances are runtime-backed, and detached local player/session synchronization no longer leaks hidden session state into the visible shell.
- TLS prompt, update-check, chat, reconnect, and controlled-room interop coverage are present.
- Controlled-room/controller-auth parity now includes create-controlled-room UX, generated-password surfacing, manual identify-as-controller flows, runtime-backed controller-auth requests, Python-style autoplay reset on controlled-room creation, and set-others-readiness actions when the server advertises support.
- The client-core still implements some operations the GUI does not fully expose yet, including slash-command routing beyond the current chat box behavior.

## Remaining Python Client Parity Tasks

### P1. Port controlled-room and controller-auth UX

Current status: completed. The Rust GUI now matches the Python client for the main controlled-room/controller-auth slice: create-controlled-room UX is present, generated room/password details are surfaced back to the user, manual identify-as-controller requests are runtime-backed, controller-auth retries no longer depend on shell-only toggles, controlled-room creation resets autoplay state like Python, and set-others-readiness is wired from the user browser when supported by the server and verified against a live Python peer.

No further assignment is needed here beyond normal regression coverage and smoke-harness stabilization.

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

1. GUI slash-command handling.
2. Configuration dialog completion.
3. Localization and language-sensitive service calls.
4. Additional player backends.

## Outside Strict Python-Client Feature Parity

These still matter, but they should not replace the feature tasks above in a client-parity tracker:

- End-to-end release packaging, installers, signing, and changelog flow.
- Cross-platform GUI validation beyond the current Windows-heavy evidence.
- Automated real-`mpv` smoke coverage in CI or a stricter scripted release gate.
- Ongoing refactor work for very large modules in `syncplay-gui`, `syncplay-cli`, and `syncplay-client-core`.
