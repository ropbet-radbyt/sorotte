# Install Guide

This project is intended to be built from source. The server also has packaging and Docker workflows for release distribution.

## Prerequisites

- Rust `1.97.1` with `rustfmt` and `clippy`
- PowerShell for the provided scripts
- `mpv` 0.41.0 or newer for client playback
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

- `sorotte-gui.exe` / `sorotte-gui`
- `sorotte-cli.exe` / `sorotte-cli`
- `sorotte-server.exe` / `sorotte-server`

For faster local iteration, omit `--release`.

## Run The GUI Client

```powershell
cargo run --release -p sorotte-gui --bin sorotte-gui
```

The GUI supports saved server/user/room settings and GUI-owned `mpv` startup. Configure the `mpv` path in the GUI if automatic discovery does not find it.
Packaged Windows GUI builds check <https://github.com/ropbet-radbyt/sorotte/releases> for self-updates. Stable builds read the latest non-prerelease release; dev-channel builds read the moving `sorotte-gui-dev` prerelease.

## Run The CLI Client

Managed `mpv` launch:

```powershell
$env:SOROTTE_CLIENT_MPV_MANAGED_LAUNCH = "1"
$env:SOROTTE_CLIENT_MPV_MANAGED_BIN = "C:\path\to\mpv.exe"
$env:SOROTTE_CLIENT_MPV_MANAGED_MEDIA = "C:\media\clip.mkv"
$env:SOROTTE_CLIENT_HOST = "127.0.0.1"
$env:SOROTTE_CLIENT_PORT = "8999"
$env:SOROTTE_CLIENT_NAME = "alice"
$env:SOROTTE_CLIENT_ROOM = "demo"
cargo run --release -p sorotte-cli -- --no-gui
```

Attach to an existing `mpv` instance:

```powershell
& "C:\path\to\mpv.exe" `
  --pause `
  --idle=yes `
  --input-ipc-server="\\.\pipe\sorotte-mpv" `
  "C:\media\clip.mkv"

$env:SOROTTE_CLIENT_MPV_IPC_PATH = "\\.\pipe\sorotte-mpv"
cargo run --release -p sorotte-cli -- --no-gui -a 127.0.0.1:8999 -n alice -r demo
```

On Unix-like systems, use a Unix socket path for `--input-ipc-server` and `SOROTTE_CLIENT_MPV_IPC_PATH`.

## Run The Server

```powershell
cargo run --release -p sorotte-server -- --port 8999
```

Common options:

```powershell
cargo run --release -p sorotte-server -- `
  --port 8999 `
  --password "change-me" `
  --rooms-db-file "sorotte-rooms.sqlite3"
```

See [Server Guide](SERVER_RELEASE.md) for persistence, TLS, MOTD, Docker, and release packaging.

## Package The Server

```powershell
powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
```

Artifacts are written under `target/server-release/artifacts/` and include a `.sha256` checksum sidecar. Windows debug symbols are emitted as a separate `*-symbols.zip` sidecar when the release build produced a PDB.

## Package The GUI

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-gui-release.ps1 -Channel stable
```

Artifacts are written under `target/gui-release/artifacts/` and include the Windows package, a `.sha256` checksum sidecar, and `sorotte-update-manifest.json`. Windows debug symbols are emitted as a separate `*-symbols.zip` sidecar when the release build produced PDBs. The `sorotte-gui release` workflow publishes those files to GitHub Releases in the main `ropbet-radbyt/sorotte` repository.

## Docker Server

```powershell
docker build -f Dockerfile.server -t sorotte-server:local .
docker run --rm -p 8999:8999/tcp sorotte-server:local
```

For persistent rooms:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -v ${PWD}/sorotte-data:/data `
  sorotte-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --rooms-db-file /data/rooms.sqlite3
```

## Validate The Build

Standard local validation:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

GUI validation:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
cargo build -p sorotte-gui --bin sorotte-gui
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000
```
