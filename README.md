# syncplay-rs

`syncplay-rs` is a Rust implementation of Syncplay with:

- `syncplay-gui`: desktop client for watching together with `mpv`
- `syncplay-cli`: headless client for `mpv` automation and terminal workflows
- `syncplay-server`: Syncplay-compatible server with persistence, TLS, MOTD, Docker, and release packaging support

The current supported client target is `mpv`. Other player backends from the Python Syncplay project are not part of this Rust release line yet.

## Quick Start

Build the workspace from source:

```powershell
cargo build --release
```

Run the GUI client:

```powershell
cargo run --release -p syncplay-gui --bin syncplay-gui
```

Run a local server:

```powershell
cargo run --release -p syncplay-server -- --port 8999
```

Run the CLI client with a managed `mpv` process:

```powershell
$env:SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH = "1"
$env:SYNCPLAY_CLIENT_HOST = "127.0.0.1"
$env:SYNCPLAY_CLIENT_PORT = "8999"
$env:SYNCPLAY_CLIENT_NAME = "alice"
$env:SYNCPLAY_CLIENT_ROOM = "demo"
cargo run --release -p syncplay-cli -- --no-gui
```

## Documentation

- [Install Guide](docs/INSTALL.md): prerequisites, source builds, release builds, and first runs
- [Client Guide](docs/CLIENT.md): GUI and CLI workflows for `mpv`
- [Server Guide](docs/SERVER_RELEASE.md): server operation, Docker, release verification, packaging, and publishing
- [Development Guide](docs/DEVELOPMENT.md): workspace layout, test matrix, compatibility workflow, and contribution rules
- [Repository Guidelines](AGENTS.md): short contributor and agent-facing repo rules

## Supported Today

- Rust GUI client with saved configuration, room browser, chat, readiness, playlists, controlled rooms, public-server browsing, update checks, media search, drag/drop ingest, and GUI-owned `mpv` startup.
- Rust CLI client with legacy Syncplay startup/config compatibility, stored settings, local commands, shared playlist actions, reconnect behavior, and managed or explicit-IPC `mpv` integration.
- Rust server with Python-compatible protocol behavior, room/state/chat/playlist fanout, controlled rooms, persistent/permanent rooms, password/salt handling, MOTD templates, TLS, IPv4/IPv6 listeners, and strict release verification.
- Docker image support for the server, including `/data` and `/tls` volumes.

## Current Limits

- `mpv` is the supported player backend. `mpv.net`, MPC-HC, MPC-BE, VLC, MPlayer, IINA, and Memento are not implemented as first-class Rust adapters yet.
- Source builds are the primary install path for the client applications. The server has local packaging scripts and a container publishing workflow.
- Real-player smoke tests depend on local `mpv` and media files, so they are not part of the default `cargo test --workspace` run.

## Repository Layout

- `crates/syncplay-gui`: desktop GUI client
- `crates/syncplay-cli`: headless client binary
- `crates/syncplay-server`: server library and executable
- `crates/syncplay-client-core`: shared client session/runtime logic
- `crates/syncplay-client-app`: shared client app compatibility and settings logic
- `crates/syncplay-player-api`: player abstraction
- `crates/syncplay-player-mpv`: `mpv` JSON IPC adapter
- `crates/syncplay-protocol`: typed Syncplay protocol models
- `crates/syncplay-core`: shared domain helpers
- `crates/syncplay-compat`: Python Syncplay compatibility and interop test support
- `crates/syncplay-sim`: deterministic simulation helpers
- `fixtures/`: protocol, scenario, and TLS fixtures
- `scripts/`: verification, packaging, GUI smoke, and local utility scripts

## Useful Commands

Run standard checks:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run GUI semantic smoke coverage:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
```

Run Windows native GUI smoke coverage:

```powershell
cargo build -p syncplay-gui --bin syncplay-gui
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000
```

Run the strict server release gate:

```powershell
python -m pip install twisted pyopenssl service_identity
powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1
```

Package the server binary:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
```

Build and run the server container:

```powershell
docker build -f Dockerfile.server -t syncplay-rs-server:local .
docker run --rm -p 8999:8999/tcp syncplay-rs-server:local
```

## License

Apache-2.0. See [LICENSE](LICENSE).
