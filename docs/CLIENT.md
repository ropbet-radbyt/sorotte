# Client Guide

`sorotte` provides two client entrypoints:

- `sorotte-gui`: desktop client for normal interactive use
- `sorotte-cli`: headless client for terminal, script, and automation workflows

Both clients currently target `mpv`.

## GUI Workflow

Start the GUI:

```powershell
cargo run --release -p sorotte-gui --bin sorotte-gui
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
sorotte\
```

You can also set the path directly in the GUI or with CLI environment variables:

```powershell
$env:SOROTTE_CLIENT_MPV_MANAGED_BIN = "C:\path\to\mpv.exe"
```

## CLI Managed mpv

Managed mode starts `mpv`, creates or uses a JSON IPC endpoint, attaches to it, and then joins the Sorotte or Syncplay-compatible server.

```powershell
$env:SOROTTE_CLIENT_MPV_MANAGED_LAUNCH = "1"
$env:SOROTTE_CLIENT_MPV_MANAGED_BIN = "C:\path\to\mpv.exe"
$env:SOROTTE_CLIENT_MPV_MANAGED_MEDIA = "C:\media\clip.mkv"
$env:SOROTTE_CLIENT_HOST = "127.0.0.1"
$env:SOROTTE_CLIENT_PORT = "8999"
$env:SOROTTE_CLIENT_NAME = "alice"
$env:SOROTTE_CLIENT_ROOM = "demo"
.\target\release\sorotte-cli.exe --no-gui
```

Useful managed-mode variables:

- `SOROTTE_CLIENT_MPV_MANAGED_BIN`: `mpv` binary path
- `SOROTTE_CLIENT_MPV_MANAGED_MEDIA`: optional media file to preload
- `SOROTTE_CLIENT_MPV_MANAGED_IPC_PATH`: optional IPC socket or pipe path
- `SOROTTE_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS`: IPC startup timeout
- `SOROTTE_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS`: IPC polling interval

## CLI Explicit mpv IPC

Start `mpv` yourself with JSON IPC enabled:

```powershell
& "C:\path\to\mpv.exe" `
  --pause `
  --force-window=no `
  --idle=yes `
  --input-ipc-server="\\.\pipe\sorotte-mpv" `
  "C:\media\clip.mkv"
```

Attach the CLI:

```powershell
$env:SOROTTE_CLIENT_MPV_IPC_PATH = "\\.\pipe\sorotte-mpv"
.\target\release\sorotte-cli.exe --no-gui -a 127.0.0.1:8999 -n alice -r demo
```

`SOROTTE_MPV_IPC_PATH` is also accepted as a fallback for compatibility.

## CLI Arguments

Common startup options:

```powershell
.\target\release\sorotte-cli.exe `
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
$env:SOROTTE_CLIENT_LOG_PLAYER_TELEMETRY = "1"
$env:SOROTTE_CLIENT_LOG_PLAYER_DRIFT_DIAGNOSTICS = "1"
$env:SOROTTE_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS = "1"
```

JSON reconnect diagnostics:

```powershell
$env:SOROTTE_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS_JSON = "1"
```

Local memory checks:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/watch-sorotte-memory.ps1
```

## Troubleshooting

- If the GUI cannot launch `mpv`, set the player path explicitly in configuration.
- If managed CLI mode cannot attach to `mpv`, increase `SOROTTE_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS`.
- If explicit IPC mode cannot connect, confirm the IPC path matches the `mpv --input-ipc-server` path and that the player is still running.
- If server connection fails, test with a local server: `cargo run --release -p sorotte-server -- --port 8999`.
- If stored settings cause unwanted startup behavior, use `--no-store` or `--clear-gui-data`.

## Player Scope

`mpv` is the supported player backend in this Rust implementation. Non-`mpv` players are intentionally outside the current supported client scope.
