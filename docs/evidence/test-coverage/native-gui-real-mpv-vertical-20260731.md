# Native GUI to real mpv vertical evidence (2026-07-31)

## Scope and safety boundary

This slice adds an explicit local Windows native-GUI vertical for Sorotte's supported mpv integration. It does not target an external service. The only session transport is an existing native-smoke fixture bound to an OS-assigned `127.0.0.1` port; both the listener and connected peer are attested as nonzero IPv4-loopback endpoints. Media is a deterministically generated 12-second PCM WAV. The isolated Sorotte config, APPDATA root, generated media, mpv Lua observer, logs, screenshots, state, and manifests all live below one fresh `target/verification/gui-real-mpv-vertical/<run>` root.

The mpv IPC endpoint is a Windows named-pipe namespace path rather than a filesystem path. The lane requires the product-generated `\\.\pipe\sorotte-gui-mpv-<gui-pid>-...` prefix and binds it to the attested GUI PID in the report.

The lane is opt-in through `scripts/gui-real-mpv-vertical.ps1`. A missing explicit mpv fails before build or GUI launch, retains a structured bundle, and returns exit code 125.

## Executable contract

The wrapper rebuilds the GUI and native harness unless an explicit GUI binary is supplied. It records the GUI digest before and after the run and requires equality. The native harness then proves:

1. The exact mpv binary is supported and digest-bound.
2. The GUI is a real native window and owns foreground before physical input.
3. The mock listener, connected peer, exact client/server Hello frames, advertised `chat`, `readiness`, and `sharedPlaylists` capabilities, and release state are retained.
4. The exact File -> Open Media leaf is delivered once by a physical click.
5. The GUI owns the exact mpv process; its parent PID, process image, digest, managed IPC endpoint, generated-media path, filename, and 12-second duration match.
6. The GUI exposes enabled transport controls and a paused state.
7. GUI Play produces an ordered real-mpv `pause=false` observation and GUI `playing` projection.
8. GUI Pause produces a later real-mpv `pause=true` observation and GUI `paused` projection.
9. A success screenshot is retained.
10. The exact File -> Exit leaf is delivered once by a physical click; the complete lifecycle sequence is observed; the GUI and owned mpv terminate naturally; and the server thread/socket are joined and released.

Menu-section opening remains fail-closed. The normal path is a physical section click. If that click returns without exposing its leaf, the dedicated vertical retains two separate snapshots proving the leaf is still hidden before one UI Automation section-open fallback. The leaf itself is always one exact physical click and is never retried after delivery.

The strict Python validator rejects nonzero producer exits, any assertion inventory or order other than the canonical 13 entries, missing or extra artifacts, digest or ownership drift, non-loopback endpoints, any client/server Hello object other than the complete canonical identity, version, and feature maps, altered advertised capabilities, unreleased threads/sockets, unordered mpv state, incomplete lifecycle state, relative or different binary paths, and incomplete menu fallback evidence. Windows `\\?\X:\...` and `\\?\UNC\...` spellings are accepted only when they resolve to the same absolute path under Windows path semantics.

## Canonical local pass

Command:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\gui-real-mpv-vertical.ps1 -MpvPath "C:\Program Files\mpv\mpv.exe" -TimeoutMs 30000
```

Result: exit 0, producer `passed`, strict contract `passed`, 13 assertions, and 10 hashed contract artifacts.

Canonical bundle:

```text
target\verification\gui-real-mpv-vertical\20260730T220441781Z-41332
```

Bound identities:

- GUI SHA-256 before/after: `247df3f60f2dda546317ab2a8dd209a130bb887ac5ca1c5d6d996f1f9035037a`
- mpv: `mpv v0.41.0-877-ge5486b96d`
- mpv bytes: `121090048`
- mpv SHA-256: `2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0`
- Session listener/peer: `127.0.0.1:63930` / `127.0.0.1:63931`
- GUI PID / owned mpv PID: `1140` / `66728`
- Managed IPC: `\\.\pipe\sorotte-gui-mpv-1140-1785449096096`

Both menu sections used the normal `physical-section-open` strategy; both exact leaves record `single-exact-physical-click-no-retry`. The lifecycle contains, in order, `exit-action-applied`, `viewport-close-requested`, `runtime-stop-requested`, `runtime-worker-stopped`, and `app-drop-complete`. No Sorotte GUI or mpv process remained afterward.

This terminal pass ran after the complete all-feature workspace gate rebuilt the
GUI, so the evidence is bound to the final validated binary rather than the
earlier pre-gate build.

The earlier producer-complete bundle at `target\verification\gui-real-mpv-vertical\20260730T213424802Z-42412` was preserved unchanged. Its producer passed the same substantive vertical; the original validator rejected only equivalent Windows extended-length versus ordinary path spelling. After the documented-path normalization regression fix, that unchanged bundle revalidated with 13 assertions and 10 artifacts; the revalidation summary is outside the bundle at `target\verification\gui-real-mpv-vertical-revalidation-20260731.json`.

## Fail-closed prerequisite evidence

Command:

```powershell
& powershell -NoProfile -ExecutionPolicy Bypass -File scripts\gui-real-mpv-vertical.ps1 -MpvPath "C:\tmp\sorotte-test-coverage-design\target\verification\missing-real-mpv.exe" -TimeoutMs 30000
$LASTEXITCODE
```

Result: `125`; capability `missing-prerequisite`; stage `mpv-preflight`; GUI build, harness build, and runner were not started.

Bundle:

```text
target\verification\gui-real-mpv-vertical\20260730T213718625Z-61044
```

## Preserved diagnostic bundles

All unsuccessful exploratory bundles remain intact:

- `20260730T211217393Z-43104`: foreground ownership failure before menu input.
- `20260730T211348973Z-22180`: unchanged replay of the same foreground failure.
- `20260730T211745248Z-67240`: owned mpv file-loaded proof; disconnected UI has no filename accessibility contract.
- `20260730T212054568Z-46280`: generic in-process chat loopback correctly lacked shared-playlist control.
- `20260730T212807426Z-57304`: full media/play/pause proof; final Exit section did not open.
- `20260730T213011151Z-64300`: Open Media section did not open.

These bundles retain progressive state, failure screenshot/accessibility tree when a window existed, isolated config/media/Lua, mpv observation/log, lifecycle, wrapper metadata, producer report, and strict-contract summary.

## Focused validation

The slice was validated with:

```powershell
cargo fmt -p sorotte-gui -- --check
cargo test --locked -p sorotte-gui --features gui-native-smoke --bin sorotte-gui-native-smoke real_mpv -- --nocapture
cargo clippy --locked -p sorotte-gui --features gui-native-smoke --bin sorotte-gui-native-smoke -- -D warnings
python -m py_compile scripts\gui_real_mpv_vertical_contract.py scripts\tests\test_gui_real_mpv_vertical_contract.py
python -m unittest scripts.tests.test_gui_real_mpv_vertical_contract -v
```

Results at evidence capture: 6 Rust tests passed; focused Clippy passed with warnings denied; 9 Python tests passed.
