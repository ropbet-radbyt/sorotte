# Client Guide

`sorotte` provides two client entrypoints:

- `sorotte-gui`: desktop client for normal interactive use
- `sorotte-cli`: headless client for terminal, script, and automation workflows

Both clients currently target `mpv` 0.41.0 or newer. Older mpv releases are outside the supported compatibility floor.

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
- storage location for `sorotte.ini` and colocated GUI data

After connecting, the main window supports room/user/file browsing, chat, readiness, shared playlists, media open/import, controlled rooms, public-server browsing, media search, drag/drop ingest, and runtime-backed connect/disconnect flows.

Readiness-capable Sorotte rooms keep the user's Ready/Not Ready intent separate from loading, buffering, seeking, and recovery. Player Play/Pause gestures count as deliberate readiness changes, while automatic player corrections do not. See [Readiness and automatic start](READINESS.md) for the full behavior and CLI commands.

## Configuration Storage

Sorotte uses `sorotte.ini` in the install folder as a locator to find the storage root for `sorotte.ini` and appdata-style GUI files. On first startup, Sorotte creates this locator if it is missing and points it at the platform Sorotte folder:

| Platform | Default config file |
| --- | --- |
| Windows | `%APPDATA%\Sorotte\sorotte.ini` |
| Linux/BSD | `${XDG_CONFIG_HOME:-$HOME/.config}/sorotte/sorotte.ini` |
| macOS | `$HOME/Library/Application Support/Sorotte/sorotte.ini` |

If install-folder `sorotte.ini` already exists, Sorotte leaves it untouched at startup. The effective storage root is also used for GUI state `.ini` files, cache files, Plex/media-search cache, stream-helper tools, and update staging. Precedence is:

1. CLI `--config-path <file>` or `--config-root <dir>`
2. `SOROTTE_CLIENT_CONFIG_PATH=<file>`
3. `SOROTTE_CLIENT_CONFIG_ROOT=<dir>`
4. install-folder `sorotte.ini`
5. legacy GUI-saved custom root pointer in the platform default Sorotte folder
6. platform default appdata root

`SOROTTE_CLIENT_CONFIG_PATH` is a full-file override and points directly at `sorotte.ini` or another config file. `SOROTTE_CLIENT_CONFIG_ROOT` is a folder override and Sorotte uses `<dir>\sorotte.ini`.

In the GUI, open `Interface & System` -> `Storage Location`. `Browse` selects a root and leaves the normal `Save` button available; saving writes the current configuration into the selected root, updates install-folder `sorotte.ini`, copies known Sorotte state/cache/tool files from the old root on a best-effort basis, and leaves the old folder untouched. `Use Default` selects the platform default root and saving writes that root into install-folder `sorotte.ini`. Persistent GUI changes are disabled while a CLI or environment override is active, because that external override wins on the next launch.

When the selected storage root is the install folder itself, install-folder `sorotte.ini` is both the locator and the normal settings file. Sorotte writes `configRoot = .` into its `[settings]` section instead of an absolute path, so the install folder can be moved as a portable bundle while preserving the rest of the settings. If the selected root is inside the install folder, Sorotte writes a relative path such as `configRoot = data` or `configRoot = config\settings`, keeping files like `MainWindow.ini` out of the install root while preserving portability.

## Client TLS Policy And Deadlines

Set the connection policy in `sorotte.ini`:

```ini
[server_data]
tlsPolicy = RequireTls
```

Accepted values are `RequireTls`, `PreferTls`, and `Plaintext`. `RequireTls` rejects a declined, malformed, interrupted, or certificate-invalid STARTTLS upgrade before sending the client Hello or credentials. `PreferTls` allows an explicit plaintext fallback and displays a security warning. `Plaintext` skips STARTTLS. When `tlsPolicy` is absent, saved server or controlled-room credentials default to `RequireTls`; connections without credentials default to `PreferTls`. The CLI environment override is `SOROTTE_CLIENT_TLS_POLICY` with the same values.

The CLI accepts these positive, seconds-based deadline overrides (decimal values are allowed):

- `SOROTTE_CLIENT_CONNECT_TIMEOUT_SECONDS` (default `8`): TCP connect
- `SOROTTE_CLIENT_STARTTLS_TIMEOUT_SECONDS` (default `8`): STARTTLS response
- `SOROTTE_CLIENT_TLS_HANDSHAKE_TIMEOUT_SECONDS` (default `8`): TLS handshake
- `SOROTTE_CLIENT_INITIAL_HELLO_TIMEOUT_SECONDS` (default `10`): client Hello/startup writes and the server's initial Hello response

Each deadline failure enters the normal reconnect policy; after retries are exhausted, the final phase error is returned.

## mpv Setup

The GUI and CLI can use a discovered `mpv` binary at version 0.41.0 or newer, a configured player path, or an explicit path supplied by environment/config.

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
- `--config-path <file>`
- `--config-root <dir>`
- `--clear-gui-data`
- `--no-store`
- `-v, --version`
- `-h, --help`

Explicit-IPC mode applies a practical subset of startup player options directly to the attached `mpv` instance, including pause, start position, speed, volume, mute, subtitle visibility, fullscreen/window flags, and generic `--name=value` / `--profile` attach commands.

## Streaming, Buffering, and Recovery

The GUI Streaming section and the `[client_settings]` section of `sorotte.ini` support typed quality and cache controls. The default episode-cache settings are:

```ini
[client_settings]
streamingQualityPreset = 720p
streamingBufferTarget = 60
streamingReadAhead = 7200
streamingMemoryCacheMiB = 256
streamingDiskCacheEnabled = true
streamingRecoveryPolicy = balanced
streamingStartPolicy = immediate
```

The GUI also provides **Private Room**, **Large Controlled Room**, and **Public Room** synchronization profiles. Profiles update the unsaved draft and are persisted only when **Save** is used; manually changing a profile-owned value produces a **Custom** profile. **Public Room** is the application default.

Quality and buffering values are attached as per-file options to network media that Sorotte opens in managed and attached `mpv` instances. Local files retain the player's own cache defaults and user configuration, and mpv restores those values after a stream ends. Matching per-player advanced arguments take precedence for streamed media; the GUI shows the effective value for generated streaming options.

Sorotte prevents cache-release seek loops with a generation-aware coordinator that retains room intent, blocks competing drift correction during recovery, and requires observed forward playback before accepting play. A network seek outside the observed cache uses one frozen target and one primary seek while data is fetched; advancing room timestamps do not restart it. The UI and opt-in CLI diagnostics distinguish seeking, fetching, buffer refill, ready, catching up, and explicit degradation without presenting refill percentage as media download progress or claiming an ETA. Local-file seeking remains unchanged.

Configured recovery uses bounded gentle catch-up, hard-seek and retry budgets, and explicit degradation. Sorotte clients also drive the feature-negotiated `sorottePlaybackBarrierV1` start barrier and authenticated controlled-room buffering policies. Quality downgrade remains advisory; Sorotte never changes it automatically. The transport contract for a future user-confirmed YouTube quality retry reloads only the local transport while preserving the logical room identity and frozen target; Plex can offer the equivalent only when backed by an actual transcoder-quality API. Mid-play room-wide seek barriers are future protocol work; current seek preparation is client-only.

For finite network media, an mpv `end-file` event well before the known duration is treated as a transport EOF rather than successful completion. Sorotte retries the same local transport at the last observed position with a bounded immediate-attempt budget while preserving the media generation; observed forward progress rearms that budget. On mpv builds with the curl backend, Sorotte also prefers negotiated HTTP/2 for network files when curl protocol selection is still `auto`, avoiding false EOFs caused by exhausted HTTP/3 connection-drain retries. An explicit user `curl-http-version` choice remains authoritative.

See the [Stream Synchronization Guide](STREAM_SYNCHRONIZATION.md) for the exact profile values, every setting, `mpv` mappings, source-specific guidance, wire lifecycle, diagnostics, tests, and the implemented-versus-planned boundary.

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

`mpv` 0.41.0 or newer is the supported player backend in this Rust implementation. Older mpv releases and non-`mpv` players are intentionally outside the current supported client scope.
