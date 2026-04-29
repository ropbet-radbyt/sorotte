# Rust Server Release Guide

This guide covers release readiness for the Rust `syncplay-server` binary.

## Verification

Run the strict release gate from the workspace root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1
```

The gate bootstraps the pinned Syncplay `v1.7.5` oracle into `.interop-cache/syncplay-legacy` when `SYNCPLAY_LEGACY_ROOT` is not set, then runs the normal cargo checks plus the strict ignored server binary matrix. Python prerequisites are required:

```powershell
python -m pip install twisted pyopenssl service_identity
```

Reports are written to `target/server-release-verify/server-release-report.json` and `target/server-release-verify/server-release-report.md`.

## Packaging

After verification passes, create the server package:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
```

The packaging script builds only `syncplay-server` in release mode and writes artifacts under `target/server-release/artifacts`.

Supported artifact names:

- `syncplay-server-<version>-windows-x86_64.zip`
- `syncplay-server-<version>-linux-x86_64.tar.gz`

Each artifact has a `.sha256` sidecar file. Windows packages include `syncplay_server.pdb` when the release build produced one.

## Contents

Server release packages include:

- `syncplay-server` or `syncplay-server.exe`
- `README.md`
- `SERVER_RELEASE.md`
- `LICENSE`
- optional Windows debug symbols

Packages intentionally exclude `target/release/deps`.

## Container Image

Build the generic server image from the workspace root:

```powershell
docker build -f Dockerfile.server -t syncplay-rs-server:local .
```

Run the server with Docker defaults:

```powershell
docker run --rm -p 8999:8999/tcp syncplay-rs-server:local
```

The image runs as a non-root `syncplay` user, exposes TCP port `8999`, and declares `/data` and `/tls` as volumes. The default command binds IPv4 on `0.0.0.0`, which is the usual container bridge/networking shape. Override the command with normal `syncplay-server` flags when needed.

Persistent rooms:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -v ${PWD}/syncplay-data:/data `
  syncplay-rs-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --rooms-db-file /data/rooms.sqlite3
```

Server password and MOTD through environment/file mounts:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -e SYNCPLAY_PASSWORD=change-me `
  -v ${PWD}/syncplay-data:/data `
  syncplay-rs-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --motd-file /data/motd.txt
```

TLS expects `/tls/cert.pem`, `/tls/chain.pem`, and `/tls/privkey.pem`:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -v ${PWD}/syncplay-data:/data `
  -v ${PWD}/syncplay-tls:/tls:ro `
  syncplay-rs-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --tls /tls
```

For NAS/container UIs such as Unraid, use the same generic settings: map TCP container port `8999`, mount an appdata-style directory to `/data`, optionally mount certificates to `/tls`, and put any extra server flags in the container command/post-arguments field.

## Publishing To GHCR

The repository can remain private while the container package is made public in GitHub Container Registry. The publish workflow builds and pushes:

- `ghcr.io/ropbet-radbyt/syncplay-rs-server:latest`
- `ghcr.io/ropbet-radbyt/syncplay-rs-server:<git-tag>` for tag-triggered releases
- `ghcr.io/ropbet-radbyt/syncplay-rs-server:sha-<short-sha>`

To publish manually:

1. Push these workflow changes to GitHub.
2. In GitHub, open the private repository.
3. Go to `Actions`.
4. Run `publish syncplay-server container`.
5. After the first successful push, open the package page for `syncplay-rs-server`.
6. Go to `Package settings`.
7. Change visibility to `Public`.

GitHub Container Registry packages are private on first publish. Public container packages can be pulled anonymously, so Unraid does not need GitHub credentials after the package visibility is changed.

Use this image in Unraid:

```text
Repository: ghcr.io/ropbet-radbyt/syncplay-rs-server:latest
Container Port: 8999
Host Port: 8999
Protocol: TCP
```

Recommended paths and variables:

```text
/data -> /mnt/user/appdata/syncplay-rs-server/data
/tls  -> /mnt/user/appdata/syncplay-rs-server/tls
SYNCPLAY_SERVER_ROOMS_DB_FILE=/data/rooms.sqlite3
SYNCPLAY_PASSWORD=<optional server password>
SYNCPLAY_SERVER_TLS_CERT_PATH=/tls
```

## Signing

Artifacts are checksumed but unsigned in this milestone. Signing should be added later as release infrastructure, after the project has a stable signing key and artifact publication process.
