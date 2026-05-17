# Server Guide

This guide covers operating, verifying, packaging, and publishing the Rust `syncplay-server` binary.

## Run The Server

From source:

```powershell
cargo run --release -p syncplay-server -- --port 8999
```

From a built binary:

```powershell
.\target\release\syncplay-server.exe --port 8999
```

Default behavior listens on TCP port `8999`.

## Options And Environment

Show the current server help:

```powershell
cargo run --quiet -p syncplay-server -- --help
```

Supported options include:

- `--port [port]`
- `--password [password]`
- `--salt [salt]`
- `--disable-ready`
- `--disable-chat`
- `--isolate-rooms`
- `--motd-file [file]`
- `--rooms-db-file [file]`
- `--permanent-rooms-file [file]`
- `--max-chat-message-length [n]`
- `--max-username-length [n]`
- `--stats-db-file [file]`
- `--tls [dir]`
- `--ipv4-only`
- `--ipv6-only`
- `--interface-ipv4 [ip]`
- `--interface-ipv6 [ip]`

Supported environment overrides:

- `SYNCPLAY_SERVER_PORT`
- `SYNCPLAY_PASSWORD`
- `SYNCPLAY_SERVER_PASSWORD`
- `SYNCPLAY_SALT`
- `SYNCPLAY_SERVER_SALT`
- `SYNCPLAY_SERVER_MOTD_TEMPLATE`
- `SYNCPLAY_SERVER_ROOMS_DB_FILE`
- `SYNCPLAY_SERVER_PERMANENT_ROOMS_FILE`
- `SYNCPLAY_SERVER_STATS_DB_FILE`
- `SYNCPLAY_SERVER_TLS_CERT_PATH`
- `SYNCPLAY_SERVER_PERSISTENT_ROOMS`

## Persistence

Enable persistent rooms with SQLite:

```powershell
.\target\release\syncplay-server.exe `
  --port 8999 `
  --rooms-db-file .\syncplay-rooms.sqlite3
```

Load permanent room names from a file:

```powershell
.\target\release\syncplay-server.exe `
  --port 8999 `
  --permanent-rooms-file .\permanent-rooms.txt
```

Each permanent-room file line is treated as a room name.

Enable stats snapshots:

```powershell
.\target\release\syncplay-server.exe `
  --port 8999 `
  --stats-db-file .\syncplay-stats.sqlite3
```

## Passwords, MOTD, And Room Isolation

Server password:

```powershell
.\target\release\syncplay-server.exe --port 8999 --password "change-me"
```

MOTD template from a file:

```powershell
.\target\release\syncplay-server.exe --port 8999 --motd-file .\motd.txt
```

MOTD templates support the legacy Syncplay variables handled by the Rust server, including server/client/user/room fields.

Room isolation:

```powershell
.\target\release\syncplay-server.exe --port 8999 --isolate-rooms
```

## TLS

The `--tls` option points at a directory containing:

- `cert.pem`
- `chain.pem`
- `privkey.pem`

Example:

```powershell
.\target\release\syncplay-server.exe --port 8999 --tls .\tls
```

## Docker

Build the image:

```powershell
docker build -f Dockerfile.server -t syncplay-rs-server:local .
```

Run with defaults:

```powershell
docker run --rm -p 8999:8999/tcp syncplay-rs-server:local
```

The image runs as a non-root `syncplay` user, exposes TCP port `8999`, and declares `/data` and `/tls` volumes. The default command binds IPv4 on `0.0.0.0`.

Persistent rooms:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -v ${PWD}/syncplay-data:/data `
  syncplay-rs-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --rooms-db-file /data/rooms.sqlite3
```

Password and MOTD:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -e SYNCPLAY_PASSWORD=change-me `
  -v ${PWD}/syncplay-data:/data `
  syncplay-rs-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --motd-file /data/motd.txt
```

TLS:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -v ${PWD}/syncplay-data:/data `
  -v ${PWD}/syncplay-tls:/tls:ro `
  syncplay-rs-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --tls /tls
```

## Unraid And NAS Container UIs

Use the same generic Docker settings:

```text
Repository: ghcr.io/ropbet-radbyt/syncplay-rs-server:latest
Container Port: 8999
Host Port: 8999
Protocol: TCP
/data -> /mnt/user/appdata/syncplay-rs-server/data
/tls  -> /mnt/user/appdata/syncplay-rs-server/tls
```

Optional environment variables:

```text
SYNCPLAY_SERVER_ROOMS_DB_FILE=/data/rooms.sqlite3
SYNCPLAY_PASSWORD=<optional server password>
SYNCPLAY_SERVER_TLS_CERT_PATH=/tls
```

Put extra server flags in the container command or post-arguments field.

For SWAG deployments where SWAG is the public entrypoint and Syncplay TLS is required, see `docs/SWAG_SYNCPLAY.md`.

## Verification

Run the strict release gate from the workspace root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1
```

The gate bootstraps the pinned Syncplay `v1.7.5` oracle into `.interop-cache/syncplay-legacy` when `SYNCPLAY_LEGACY_ROOT` is not set, then runs the normal cargo checks plus the strict `syncplay-server` binary release matrix. Python prerequisites are required:

```powershell
python -m pip install -r requirements/legacy-python-interop.txt
```

Reports are written to:

- `target/server-release-verify/server-release-report.json`
- `target/server-release-verify/server-release-report.md`

## Packaging

Create a server package:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
```

The script builds only `syncplay-server` in release mode and writes artifacts under `target/server-release/artifacts`.

Supported artifact names:

- `syncplay-server-<version>-windows-x86_64.zip`
- `syncplay-server-<version>-linux-x86_64.tar.gz`

Each artifact has a `.sha256` sidecar file. Windows packages include `syncplay_server.pdb` when the release build produced one.

Server release packages include:

- `syncplay-server` or `syncplay-server.exe`
- `README.md`
- `SERVER_RELEASE.md`
- `LICENSE`
- optional Windows debug symbols

Packages intentionally exclude `target/release/deps`.

## Publishing To GHCR

The publish workflow builds and pushes:

- `ghcr.io/ropbet-radbyt/syncplay-rs-server:latest`
- `ghcr.io/ropbet-radbyt/syncplay-rs-server:<git-tag>`
- `ghcr.io/ropbet-radbyt/syncplay-rs-server:sha-<short-sha>`

To publish manually:

1. Push the workflow to GitHub.
2. Open the repository in GitHub.
3. Go to `Actions`.
4. Run `publish syncplay-server container`.
5. After the first successful push, open the package page for `syncplay-rs-server`.
6. Go to `Package settings`.
7. Change visibility to `Public` if the image should be anonymously pullable.

GitHub Container Registry packages are private on first publish. Public container packages can be pulled anonymously after package visibility is changed.

## Signing

Artifacts are checksumed but unsigned. Add signing only after the project has a stable signing key and artifact publication process.
