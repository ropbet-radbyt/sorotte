# Server Guide

This guide covers operating, verifying, packaging, and publishing the Rust `sorotte-server` binary.

## Run The Server

From source:

```powershell
cargo run --release -p sorotte-server -- --port 8999
```

From a built binary:

```powershell
.\target\release\sorotte-server.exe --port 8999
```

Default behavior listens on TCP port `8999`.

## Options And Environment

Show the current server help:

```powershell
cargo run --quiet -p sorotte-server -- --help
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

- `SOROTTE_SERVER_PORT`
- `SOROTTE_PASSWORD`
- `SOROTTE_SERVER_PASSWORD`
- `SOROTTE_SALT`
- `SOROTTE_SERVER_SALT`
- `SOROTTE_SERVER_MOTD_TEMPLATE`
- `SOROTTE_SERVER_ROOMS_DB_FILE`
- `SOROTTE_SERVER_PERMANENT_ROOMS_FILE`
- `SOROTTE_SERVER_STATS_DB_FILE`
- `SOROTTE_SERVER_TLS_CERT_PATH`
- `SOROTTE_SERVER_PERSISTENT_ROOMS`

## Persistence

Enable persistent rooms with SQLite:

```powershell
.\target\release\sorotte-server.exe `
  --port 8999 `
  --rooms-db-file .\sorotte-rooms.sqlite3
```

Load permanent room names from a file:

```powershell
.\target\release\sorotte-server.exe `
  --port 8999 `
  --permanent-rooms-file .\permanent-rooms.txt
```

Each permanent-room file line is treated as a room name.

Enable stats snapshots:

```powershell
.\target\release\sorotte-server.exe `
  --port 8999 `
  --stats-db-file .\sorotte-stats.sqlite3
```

## Passwords, MOTD, And Room Isolation

Server password:

```powershell
.\target\release\sorotte-server.exe --port 8999 --password "change-me"
```

MOTD template from a file:

```powershell
.\target\release\sorotte-server.exe --port 8999 --motd-file .\motd.txt
```

MOTD templates support the legacy Syncplay variables handled by the Rust server, including server/client/user/room fields.

Room isolation:

```powershell
.\target\release\sorotte-server.exe --port 8999 --isolate-rooms
```

## TLS

The `--tls` option points at a TLS bundle root. The recommended layout uses an
atomically replaced selector and immutable generations:

```text
tls/
  current.json
  generations/
    20260730T120000Z-1234-AbCdEf/
      cert.pem
      chain.pem
      privkey.pem
```

`current.json` uses schema `sorotte-tls-bundle-v1`, names the selected
generation, and records the exact byte length and lowercase SHA-256 digest of
all three members. The server reads only that generation, authenticates every
member, and verifies that the selector did not change during capture. A
publisher must fully write a new generation and atomically rename a temporary
selector to `current.json`; it must never modify a selected generation in
place. [`copy-swag-sorotte-certs.sh`](../scripts/copy-swag-sorotte-certs.sh)
implements this contract for SWAG/Let's Encrypt lineages.

The schema is deliberately closed. A generation ID is at most 128 ASCII bytes,
uses only letters, digits, `-`, and `_`, and begins/ends with a letter or
digit. `current.json` is limited to 16 KiB and each member to 4 MiB. The atomic
bundle root, `generations` directory, selected generation directory, selector,
and members must be ordinary directories/files rather than symbolic links or
Windows reparse points.

For compatibility, a directory containing loose `cert.pem`, `chain.pem`, and
`privkey.pem` files is still accepted when `current.json` is absent. The server
requires two matching captures before installing loose files, but no reader can
prove that three independently replaced paths came from one certificate
generation. Use loose files only when they are static or publication is
externally serialized.

Example:

```powershell
.\target\release\sorotte-server.exe --port 8999 --tls .\tls
```

## Docker

Build the image:

```powershell
docker build -f Dockerfile.server -t sorotte-server:local .
```

Run with defaults:

```powershell
docker run --rm -p 8999:8999/tcp sorotte-server:local
```

The image runs as a non-root `sorotte` user, exposes TCP port `8999`, and declares `/data` and `/tls` volumes. The default command binds IPv4 on `0.0.0.0`.

Persistent rooms:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -v ${PWD}/sorotte-data:/data `
  sorotte-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --rooms-db-file /data/rooms.sqlite3
```

Password and MOTD:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -e SOROTTE_PASSWORD=change-me `
  -v ${PWD}/sorotte-data:/data `
  sorotte-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --motd-file /data/motd.txt
```

TLS:

```powershell
docker run --rm `
  -p 8999:8999/tcp `
  -v ${PWD}/sorotte-data:/data `
  -v ${PWD}/sorotte-tls:/tls:ro `
  sorotte-server:local `
  --port 8999 --ipv4-only --interface-ipv4 0.0.0.0 --tls /tls
```

## Unraid And NAS Container UIs

Use the same generic Docker settings:

```text
Repository: ghcr.io/ropbet-radbyt/sorotte-server:latest
Container Port: 8999
Host Port: 8999
Protocol: TCP
/data -> /mnt/user/appdata/sorotte-server/data
/tls  -> /mnt/user/appdata/sorotte-server/tls
```

Optional environment variables:

```text
SOROTTE_SERVER_ROOMS_DB_FILE=/data/rooms.sqlite3
SOROTTE_PASSWORD=<optional server password>
SOROTTE_SERVER_TLS_CERT_PATH=/tls
```

Put extra server flags in the container command or post-arguments field.

For SWAG deployments where SWAG is the public entrypoint and Syncplay TLS is required, see `docs/SWAG_SOROTTE.md`.

## Verification

Run the strict release gate from the workspace root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1
```

The gate bootstraps the pinned Syncplay `v1.7.5` oracle into `.interop-cache/syncplay-legacy` when `SYNCPLAY_LEGACY_ROOT` is not set, then runs the normal cargo checks plus the strict `sorotte-server` binary release matrix. Python prerequisites are required:

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

The script builds only `sorotte-server` in release mode and writes artifacts under `target/server-release/artifacts`.

Supported artifact names:

- `sorotte-server-<version>-windows-x86_64.zip`
- `sorotte-server-<version>-linux-x86_64.tar.gz`
- `sorotte-server-<version>-windows-x86_64-symbols.zip` when Windows PDB symbols are available

Each artifact has a `.sha256` sidecar file. Windows packages do not include PDB files; `sorotte_server.pdb` is published only in the separate symbols archive when the release build produced one.

Server release packages include:

- `sorotte-server` or `sorotte-server.exe`
- `README.md`
- `SERVER_RELEASE.md`
- `LICENSE`

Windows symbols archives include:

- `sorotte_server.pdb`

Packages intentionally exclude `target/release/deps`.

## Publishing To GHCR

Release-tag runs build and push immutable source tags:

- `ghcr.io/ropbet-radbyt/sorotte-server:<git-tag>`
- `ghcr.io/ropbet-radbyt/sorotte-server:sha-<short-sha>`

The mutable `ghcr.io/ropbet-radbyt/sorotte-server:latest` tag is promoted only
by an explicit manual workflow run with `push_latest` set to `true`. Re-running
an older tag workflow cannot move `latest` backward.

To publish manually:

1. Push the workflow to GitHub.
2. Open the repository in GitHub.
3. Go to `Actions`.
4. Run `publish sorotte-server container`; set `push_latest` to `true` only
   when intentionally promoting that selected revision.
5. After the first successful push, open the package page for `sorotte-server`.
6. Go to `Package settings`.
7. Change visibility to `Public` if the image should be anonymously pullable.

GitHub Container Registry packages are private on first publish. Public container packages can be pulled anonymously after package visibility is changed.

## Signing

Artifacts are checksumed but unsigned. Add signing only after the project has a stable signing key and artifact publication process.
