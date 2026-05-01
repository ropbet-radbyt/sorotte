# Client Guide

`syncplay-rs` provides two client entrypoints:

- `syncplay-gui`: desktop client for normal interactive use
- `syncplay-cli`: headless client for terminal, script, and automation workflows

Both clients currently target `mpv`.

## GUI Workflow

Start the GUI:

```powershell
cargo run --release -p syncplay-gui --bin syncplay-gui
```

Use the configuration window to set:

- server host and port
- username
- room
- optional server password
- `mpv` path and per-player arguments
- playback, readiness, playlist, chat, OSD, language, and trusted-domain settings

After connecting, the main window supports room/user/file browsing, chat, readiness, shared playlists, media open/import, controlled rooms, public-server browsing, media search, drag/drop ingest, and runtime-backed connect/disconnect flows.

## mpv Setup

The GUI and CLI can use a discovered `mpv` binary, a configured player path, or an explicit path supplied by environment/config.

Recommended Windows layout for local development:

```text
mpv\mpv.exe
syncplay-rs\
```

You can also set the path directly in the GUI or with CLI environment variables:

```powershell
$env:SYNCPLAY_CLIENT_MPV_MANAGED_BIN = "C:\path\to\mpv.exe"
```

## CLI Managed mpv

Managed mode starts `mpv`, creates or uses a JSON IPC endpoint, attaches to it, and then joins the Syncplay server.

```powershell
$env:SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH = "1"
$env:SYNCPLAY_CLIENT_MPV_MANAGED_BIN = "C:\path\to\mpv.exe"
$env:SYNCPLAY_CLIENT_MPV_MANAGED_MEDIA = "C:\media\clip.mkv"
$env:SYNCPLAY_CLIENT_HOST = "127.0.0.1"
$env:SYNCPLAY_CLIENT_PORT = "8999"
$env:SYNCPLAY_CLIENT_NAME = "alice"
$env:SYNCPLAY_CLIENT_ROOM = "demo"
.\target\release\syncplay-cli.exe --no-gui
```

Useful managed-mode variables:

- `SYNCPLAY_CLIENT_MPV_MANAGED_BIN`: `mpv` binary path
- `SYNCPLAY_CLIENT_MPV_MANAGED_MEDIA`: optional media file to preload
- `SYNCPLAY_CLIENT_MPV_MANAGED_IPC_PATH`: optional IPC socket or pipe path
- `SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS`: IPC startup timeout
- `SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS`: IPC polling interval

## CLI Explicit mpv IPC

Start `mpv` yourself with JSON IPC enabled:

```powershell
& "C:\path\to\mpv.exe" `
  --pause `
  --force-window=no `
  --idle=yes `
  --input-ipc-server="\\.\pipe\syncplay-rs-mpv" `
  "C:\media\clip.mkv"
```

Attach the CLI:

```powershell
$env:SYNCPLAY_CLIENT_MPV_IPC_PATH = "\\.\pipe\syncplay-rs-mpv"
.\target\release\syncplay-cli.exe --no-gui -a 127.0.0.1:8999 -n alice -r demo
```

`SYNCPLAY_MPV_IPC_PATH` is also accepted as a fallback for compatibility.

## CLI Arguments

Common startup options:

```powershell
.\target\release\syncplay-cli.exe `
  --no-gui `
  -a 127.0.0.1:8999 `
  -n alice `
  -r demo `
  "C:\media\clip.mkv" `
  -- --start=12 --pause
```

Useful options:

- `-a, --host <hostname[:port]>`
- `-n, --name <username>`
- `-r, --room [room]`
- `-p, --password [password]`
- `--player-path <path>`
- `--load-playlist-from-file <path>`
- `--language <language>`
- `--clear-gui-data`
- `--no-store`
- `-v, --version`
- `-h, --help`

Explicit-IPC mode applies a practical subset of startup player options directly to the attached `mpv` instance, including pause, start position, speed, volume, mute, subtitle visibility, fullscreen/window flags, and generic `--name=value` / `--profile` attach commands.

## Rooms, Playlists, And Controlled Rooms

Room names are normal Syncplay room names. Controlled-room suffix parsing and controller password overrides are supported for Python-compatible flows.

Shared playlist support includes connect-time playlist load in the CLI and GUI playlist import/edit/open/shuffle/undo workflows. URL playlist entries can be restricted with trusted-domain settings.

## Diagnostics

Useful CLI diagnostics:

```powershell
$env:SYNCPLAY_CLIENT_LOG_PLAYER_TELEMETRY = "1"
$env:SYNCPLAY_CLIENT_LOG_PLAYER_DRIFT_DIAGNOSTICS = "1"
$env:SYNCPLAY_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS = "1"
```

JSON reconnect diagnostics:

```powershell
$env:SYNCPLAY_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS_JSON = "1"
```

Local memory checks:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/watch-syncplay-memory.ps1
```

## Troubleshooting

- If the GUI cannot launch `mpv`, set the player path explicitly in configuration.
- If managed CLI mode cannot attach to `mpv`, increase `SYNCPLAY_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS`.
- If explicit IPC mode cannot connect, confirm the IPC path matches the `mpv --input-ipc-server` path and that the player is still running.
- If server connection fails, test with a local server: `cargo run --release -p syncplay-server -- --port 8999`.
- If stored settings cause unwanted startup behavior, use `--no-store` or `--clear-gui-data`.

## Player Scope

`mpv` is the supported player backend in this Rust implementation. Non-`mpv` players are intentionally outside the current supported client scope.
