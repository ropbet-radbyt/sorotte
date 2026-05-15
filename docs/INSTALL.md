# Install Guide

This project is intended to be built from source. The server also has packaging and Docker workflows for release distribution.

## Prerequisites

- Rust `1.95.0` with `rustfmt` and `clippy`
- PowerShell for the provided scripts
- `mpv` for client playback
- Python with `twisted`, `pyopenssl`, and `service_identity` for compatibility and server release verification
- Docker if you want to build or run the server container

The Rust toolchain is pinned in `rust-toolchain.toml`, so `rustup` will select the expected toolchain automatically.

Install Python prerequisites when you need the server release gate or live legacy compatibility tests:

```powershell
python -m pip install -r requirements/legacy-python-interop.txt
```

## Build From Source

From the repository root:

```powershell
cargo build --release
```

Release binaries are written to `target/release/`:

- `syncplay-gui.exe` / `syncplay-gui`
- `syncplay-cli.exe` / `syncplay-cli`
- `syncplay-server.exe` / `syncplay-server`

For faster local iteration, omit `--release`.

## Run The GUI Client

```powershell
cargo run --release -p syncplay-gui --bin syncplay-gui
```

The GUI supports saved server/user/room settings and GUI-owned `mpv` startup. Configure the `mpv` path in the GUI if automatic discovery does not find it.

## Run The CLI Client

Managed `mpv` launch:

```powershell
$env:SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH = "1"
$env:SYNCPLAY_CLIENT_MPV_MANAGED_BIN = "C:\path\to\mpv.exe"
$env:SYNCPLAY_CLIENT_MPV_MANAGED_MEDIA = "C:\media\clip.mkv"
$env:SYNCPLAY_CLIENT_HOST = "127.0.0.1"
$env:SYNCPLAY_CLIENT_PORT = "8999"
$env:SYNCPLAY_CLIENT_NAME = "alice"
$env:SYNCPLAY_CLIENT_ROOM = "demo"
cargo run --release -p syncplay-cli -- --no-gui
```

Attach to an existing `mpv` instance:

```powershell
& "C:\path\to\mpv.exe" `
  --pause `
  --idle=yes `
  --input-ipc-server="\\.\pipe\syncplay-rs-mpv" `
  "C:\media\clip.mkv"

$env:SYNCPLAY_CLIENT_MPV_IPC_PATH = "\\.\pipe\syncplay-rs-mpv"
cargo run --release -p syncplay-cli -- --no-gui -a 127.0.0.1:8999 -n alice -r demo
```

On Unix-like systems, use a Unix socket path for `--input-ipc-server` and `SYNCPLAY_CLIENT_MPV_IPC_PATH`.

## Run The Server

```powershell
cargo run --release -p syncplay-server -- --port 8999
```

Common options:

```powershell
cargo run --release -p syncplay-server -- `
  --port 8999 `
  --password "change-me" `
  --rooms-db-file "syncplay-rooms.sqlite3"
```

See [Server Guide](SERVER_RELEASE.md) for persistence, TLS, MOTD, Docker, and release packaging.

## Package The Server

```powershell
powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
```

Artifacts are written under `target/server-release/artifacts/` and include a `.sha256` checksum sidecar.

## Docker Server

```powershell
docker build -f Dockerfile.server -t syncplay-rs-server:local .
docker run --rm -p 8999:8999/tcp syncplay-rs-server:local
```

For persistent rooms:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -v ${PWD}/syncplay-data:/data `
  syncplay-rs-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --rooms-db-file /data/rooms.sqlite3
```

## Validate The Build

Standard local validation:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

GUI validation:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
cargo build -p syncplay-gui --bin syncplay-gui
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000
```
