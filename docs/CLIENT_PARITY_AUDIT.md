# Client Parity Audit

## Audit Date

- 2026-03-19

## Verification Performed For This Refresh

- `cargo fmt --all`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json` (`11/11` scenarios)
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
  - `../syncplay/syncplay/messages_en.py`
  - `../syncplay/syncplay/messages_fr.py`
- Static comparison of the Rust client implementation:
- `crates/syncplay-gui/src/app.rs`
  - `crates/syncplay-gui/src/remote_services.rs`
  - `crates/syncplay-client-core/src/lib.rs`
  - `crates/syncplay-client-app/src/legacy_settings.rs`
  - `crates/syncplay-client-app/src/legacy_language.rs`
  - `crates/syncplay-player-api/src/lib.rs`
  - `crates/syncplay-player-mpv/src/lib.rs`

## Current Read On Parity

- The Rust client-core is still ahead of the Rust GUI overall, but the GUI now also covers the last high-visibility language/runtime gap from the previous audit: runtime notifications, update/public-server service calls, and update-dialog text now honor the selected GUI language.
- The default `mpv`-backed GUI startup path is no longer blocked on manual environment setup. Saved `playerPath` plus `perPlayerArguments` now drive a GUI-owned `mpv` launch, legacy Syncplay `mpv` OSD/chat settings are applied, GUI notifications/chat are mirrored into `mpv`, and the GUI owns relaunch/failure handling for that path.
- The configuration dialog now covers the remaining `mpv`-scope legacy settings from the Python client, including player/startup toggles, multiline room/trusted-domain/media-directory editing, chat appearance settings, and OSD timing controls.
- Configuration-surface `Connect` now matches the Python startup-confirm handoff for the `mpv`-first scope: it saves the draft, reuses the managed-player startup path, joins the selected room, and lands in the main window through the existing detached connect runtime.
- Additional player backend parity remains the main explicitly deferred client-parity question after that `mpv`-first GUI lifecycle slice.

## What No Longer Needs Assignment

- The GUI now launches and owns saved-config `mpv` without requiring `SYNCPLAY_CLIENT_MPV_IPC_PATH` or `SYNCPLAY_MPV_IPC_PATH`.
- Saved `perPlayerArguments` are applied in the GUI-owned `mpv` launch path.
- Legacy Syncplay `mpv` UI settings now apply in both explicit-IPC attach mode and GUI-owned startup, including chat input/output and timeout-backed OSD behavior.
- GUI chat/system notifications now forward into attached `mpv` via the legacy OSD/chat path, and `mpv` chat input now routes back into the GUI session runtime.
- GUI-owned `mpv` lifecycle is managed across startup, save/reload/reset, on-demand reopen, and unexpected process exit reporting.
- Saved host/port settings can drive a real GUI connect/disconnect flow, including saved-config startup auto-connect.
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
- The GUI chat box now mirrors Python slash-command handling, including local command dispatch before chat send, literal `//` escaping, echoed command lines, and runtime-backed seek/undo/offset/playlist/readiness/controller/chat aliases via the shared `syncplay-client-app` planner.
- Configuration dialog parity now covers the remaining `mpv`-scope legacy settings, including per-player arguments, loop/autosave/force-GUI toggles, multiline room/trusted-domain/media-directory editing on the main configuration surface, chat appearance controls, and OSD timeout/slowdown settings.
- Runtime notifications, update notices, and language-sensitive public-server/update-check requests now honor the selected GUI language, with a non-English semantic smoke scenario proving the path.

## Remaining Python Client Parity Tasks

### P1. Additional player backend parity

Current status: the Rust workspace only has a first-class `mpv` backend. The Python client supports `mpv`, `mpvnet`, `MPC-HC`, `MPC-BE`, `VLC`, `MPlayer`, `IINA`, and `Memento`.

Work to assign:

- Decide whether "client parity" still means Python's full player matrix or an explicit `mpv`-only milestone.
- If full parity remains the target, split ports by platform:
  - Windows: `mpvnet`, `MPC-HC`, `MPC-BE`
  - macOS: `IINA`
  - cross-platform/legacy: `VLC`, `MPlayer`, `Memento`

## Practical Assignment Order

1. Keep the Python-style config-confirm startup handoff treated as done for the `mpv`-first GUI scope.
2. Revisit additional player backends only if the product target expands beyond the current `mpv`-first milestone.

## Outside Strict Python-Client Feature Parity

These still matter, but they should not replace the feature tasks above in a client-parity tracker:

- End-to-end release packaging, installers, signing, and changelog flow.
- Cross-platform GUI validation beyond the current Windows-heavy evidence.
- Automated real-`mpv` smoke coverage in CI or a stricter scripted release gate.
- Ongoing refactor work for very large modules in `syncplay-gui`, `syncplay-cli`, and `syncplay-client-core`.
