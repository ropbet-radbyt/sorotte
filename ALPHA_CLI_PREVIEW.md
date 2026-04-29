# Syncplay Rust CLI Alpha (Windows / mpv)

This document captures the current alpha packaging/run flow that was validated locally.

Use `PROJECT_STATUS.md` for the current repo audit/checklist. This file is the packaging/run guide for the Windows/`mpv` CLI alpha path.

## Scope

- `syncplay-cli.exe` (headless CLI client, `mpv` integration)
- `syncplay-server.exe` is a Python-compatible Rust server executable with a separate release verification and packaging flow in `docs/SERVER_RELEASE.md`

This is a developer-preview / CLI-alpha workflow, not a GUI release.

## Validated Gates (local validation notes, 2026-02-24)

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- real-`mpv` smoke matrix (serial) for:
  - adapter smoke
  - managed launch
  - unmanaged external launch
  - explicit-IPC startup
  - explicit-IPC reconnect validation
  - explicit-IPC inbound rewind
  - explicit-IPC ping forward-delay A/B (`serverRtt` present/absent)
- release gate spot-check:
  - `cargo build --release`
  - `target/release/syncplay-cli.exe --version`
  - release-profile real-`mpv` smokes (managed + explicit-IPC startup + explicit-IPC reconnect)

Audit note: the latest documentation audit re-ran `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`; real-`mpv` and release-profile checks remain manual/local validations.

## Release Artifacts (current build)

Built by `cargo build --release`:

- `target/release/syncplay-cli.exe`
- `target/release/syncplay-server.exe`
- optional debug symbols:
  - `target/release/syncplay_cli.pdb`
  - `target/release/syncplay_server.pdb`

Recommended alpha zip contents (Windows):

- `syncplay-cli.exe`
- `syncplay_cli.pdb` / `syncplay_server.pdb` (optional, useful for debugging)
- `README.md`
- `ALPHA_CLI_PREVIEW.md`

Do not package `target/release/deps/`.
Package release-ready server artifacts with `scripts/package-server-release.ps1`; this CLI alpha zip remains focused on the headless client workflow.

## Prerequisites

- Windows
- `mpv` with JSON IPC support
- A Syncplay-compatible server endpoint (existing server, or the Python `syncplay-server` from the sibling `syncplay/` repo in this workspace)

## Quick Start (Managed `mpv`)

Validated behavior: CLI launches `mpv`, waits for JSON IPC, auto-attaches, and connects.

PowerShell example:

```powershell
$env:SYNCPLAY_CLIENT_CONNECT = "1"
$env:SYNCPLAY_CLIENT_HOST = "127.0.0.1"
$env:SYNCPLAY_CLIENT_PORT = "8999"
$env:SYNCPLAY_CLIENT_NAME = "alice"
$env:SYNCPLAY_CLIENT_ROOM = "demo"

$env:SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH = "1"
$env:SYNCPLAY_CLIENT_MPV_MANAGED_BIN = "C:\path\to\mpv.exe"   # optional if repo-local/default discovery works
$env:SYNCPLAY_CLIENT_MPV_MANAGED_MEDIA = "C:\media\clip.mkv"  # optional preload file

.\target\release\syncplay-cli.exe --no-gui
```

Optional managed-launch tuning envs:

- `SYNCPLAY_CLIENT_MPV_MANAGED_IPC_PATH`
- `SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS`
- `SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS`

## Quick Start (Attach to Existing `mpv` via Explicit IPC)

Validated behavior: CLI attaches to an already-running `mpv` instance using JSON IPC.

1. Start `mpv` with an IPC pipe:

```powershell
& "C:\path\to\mpv.exe" `
  --pause `
  --force-window=no `
  --idle=yes `
  --input-ipc-server="\\.\pipe\syncplay-alpha-mpv" `
  "C:\media\clip.mkv"
```

2. Start `syncplay-cli` and point it at that pipe:

```powershell
$env:SYNCPLAY_CLIENT_CONNECT = "1"
$env:SYNCPLAY_CLIENT_HOST = "127.0.0.1"
$env:SYNCPLAY_CLIENT_PORT = "8999"
$env:SYNCPLAY_CLIENT_NAME = "alice"
$env:SYNCPLAY_CLIENT_ROOM = "demo"
$env:SYNCPLAY_CLIENT_MPV_IPC_PATH = "\\.\pipe\syncplay-alpha-mpv"

.\target\release\syncplay-cli.exe --no-gui
```

### Explicit-IPC startup file / `_args` subset (best effort)

When `SYNCPLAY_CLIENT_MPV_IPC_PATH` is set, the CLI can also apply a startup file and a limited `_args` subset to the attached `mpv` via legacy startup parsing.

Example:

```powershell
$env:SYNCPLAY_CLIENT_CONNECT = "1"
$env:SYNCPLAY_CLIENT_MPV_IPC_PATH = "\\.\pipe\syncplay-alpha-mpv"

.\target\release\syncplay-cli.exe `
  --no-gui `
  -a 127.0.0.1:8999 `
  -n alice `
  -r demo `
  "C:\media\clip.mkv" `
  -- --start=12 --pause --speed=1.05
```

Supported explicit-IPC `_args` subset:

- `--pause`
- `--no-pause`
- `--pause=<bool>`
- `--start <seconds-or-timecode>` / `--start=<seconds-or-timecode>` (for example `12.5`, `01:02`, `01:02:03.5`)
- `--speed <rate>` / `--speed=<rate>`
- `--volume <value>` / `--volume=<value>`
- `--mute`
- `--no-mute`
- `--mute=<bool>`
- `--deinterlace`
- `--no-deinterlace`
- `--deinterlace=<bool>`
- `--keepaspect`
- `--no-keepaspect`
- `--keepaspect=<bool>`
- `--keepaspect-window`
- `--no-keepaspect-window`
- `--keepaspect-window=<bool>`
- `--sub-visibility`
- `--no-sub-visibility`
- `--sub-visibility=<bool>`
- `--osd-bar`
- `--no-osd-bar`
- `--osd-bar=<bool>`
- `--fs` / `--fullscreen`
- `--no-fs` / `--no-fullscreen`
- `--fs=<bool>` / `--fullscreen=<bool>`
- `--ontop`
- `--no-ontop`
- `--ontop=<bool>`
- `--border`
- `--no-border`
- `--border=<bool>`
- `--force-window`
- `--no-force-window`
- `--force-window=<bool>`
- `--keep-open`
- `--no-keep-open`
- `--keep-open=<bool>`
- `--keep-open-pause`
- `--no-keep-open-pause`
- `--keep-open-pause=<bool>`
- `--cursor-autohide-fs-only`
- `--no-cursor-autohide-fs-only`
- `--cursor-autohide-fs-only=<bool>`
- `--stop-screensaver`
- `--no-stop-screensaver`
- `--stop-screensaver=<bool>`
- `--window-maximized`
- `--no-window-maximized`
- `--window-maximized=<bool>`
- `--window-minimized`
- `--no-window-minimized`
- `--window-minimized=<bool>`

Unsupported `_args` are ignored in explicit-IPC mode (with startup summary/warning diagnostics).

## Optional: Local Server for Manual Testing (Rust server or Python reference server)

Default local server port is `8999`.

Rust server:

```powershell
.\target\release\syncplay-server.exe --port 8999
```

Current Rust server parity note: the executable supports the Python-compatible server CLI/listener/MOTD/password/persistence/TLS path and is covered by the strict release gate in `scripts/server-release-verify.ps1`.

Python reference server fallback:

```powershell
python ..\syncplay\syncplayServer.py --port 8999
```

Then point the client to `127.0.0.1:8999` as shown above.

## Known Limitations (Alpha)

- CLI/headless only; no GUI runtime parity.
- Many GUI-related `syncplay.ini` keys are preserved for compatibility but not applied by `syncplay-cli`.
- Full Qt `QSettings` GUI behavior parity is out of scope; `--clear-gui-data` is best-effort for known legacy stores.
- Explicit-IPC `_args` support is intentionally limited to the subset above.
- Non-managed external-player startup is best-effort launch compatibility only (no non-`mpv` adapter integration).
- Server release artifacts are produced separately from this CLI alpha guide and are checksumed but unsigned in the current milestone.
- Real-`mpv` smokes are Windows-oriented and rely on local `mpv` + media availability.

## Diagnostics / Troubleshooting

- Print raw player telemetry:
  - `SYNCPLAY_CLIENT_LOG_PLAYER_TELEMETRY=1`
- Print drift diagnostics:
  - `SYNCPLAY_CLIENT_LOG_PLAYER_DRIFT_DIAGNOSTICS=1`
- Print reconnect correction diagnostics:
  - `SYNCPLAY_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS=1`
  - `SYNCPLAY_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS_JSON=1`

For local memory/soak checks:

- `scripts/watch-syncplay-memory.ps1`
