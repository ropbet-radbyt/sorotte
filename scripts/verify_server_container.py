#!/usr/bin/env python3
"""Build-consumer and publication verification for the sorotte-server image.

The local smoke subcommand operates only on a Docker daemon image that was
already built and loaded by the caller.  Publication is a separate explicit
subcommand so CI cannot accidentally replace the tested image with a rebuild.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import http.client
import json
import math
import os
import re
import shutil
import socket
import sqlite3
import ssl
import stat
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from typing import Any, Callable, Iterable, Sequence


SOURCE_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
IMAGE_NAME_RE = re.compile(
    r"^ghcr\.io/[a-z0-9]+(?:[._-][a-z0-9]+)*(?:/[a-z0-9]+(?:[._-][a-z0-9]+)*)+$"
)
TAG_RE = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$")
PUSH_DIGEST_RE = re.compile(r"\bdigest:\s*(sha256:[0-9a-f]{64})\b")
CONTAINER_ID_RE = re.compile(r"^[0-9a-f]{64}$")
MAX_JSON_BYTES = 128 * 1024 * 1024
MAX_PROTOCOL_BYTES = 1024 * 1024
COMMAND_TIMEOUT_SECONDS = 90
SERVER_START_TIMEOUT_SECONDS = 15
SERVER_STOP_TIMEOUT_SECONDS = 10
CONTAINER_LOG_CAPTURE_ATTEMPTS = 20
CONTAINER_LOG_CAPTURE_RETRY_SECONDS = 0.1
CONTAINER_STARTUP_LOG_MARKER = "sorotte-server listening on "
CONTAINER_SHUTDOWN_LOG_MARKER = "shutdown requested; draining client sessions"
CONTAINER_TEST_USERNAME_MAX_CHARACTERS = 16
REGISTRY_ATTEMPTS = 6
REGISTRY_RETRY_BASE_SECONDS = 0.5
REGISTRY_REQUEST_TIMEOUT_SECONDS = 15
EXPECTED_SOURCE_LABEL = "org.opencontainers.image.source"
EXPECTED_REVISION_LABEL = "org.opencontainers.image.revision"
EXPECTED_CREATED_LABEL = "org.opencontainers.image.created"
EXPECTED_LICENSE_LABEL = "org.opencontainers.image.licenses"
EXPECTED_ENTRYPOINT = ["sorotte-server"]
EXPECTED_DEFAULT_COMMAND = [
    "--port",
    "8999",
    "--ipv4-only",
    "--interface-ipv4",
    "0.0.0.0",
]


class VerificationError(RuntimeError):
    """Raised when the tested image or its publication evidence is invalid."""


def _reject_duplicate_key(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise VerificationError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _load_json_bytes(payload: bytes, description: str) -> Any:
    if len(payload) > MAX_JSON_BYTES:
        raise VerificationError(f"{description} exceeds the JSON size limit")
    try:
        return json.loads(payload, object_pairs_hook=_reject_duplicate_key)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError(f"{description} is not valid duplicate-free JSON") from error


def _load_json_file(path: Path, description: str) -> Any:
    _require_regular_file(path, description)
    if path.stat().st_size > MAX_JSON_BYTES:
        raise VerificationError(f"{description} exceeds the JSON size limit: {path}")
    return _load_json_bytes(path.read_bytes(), description)


def _require_regular_file(path: Path, description: str) -> None:
    try:
        mode = path.lstat().st_mode
    except FileNotFoundError as error:
        raise VerificationError(f"{description} is missing: {path}") from error
    if path.is_symlink() or not stat.S_ISREG(mode):
        raise VerificationError(f"{description} must be a regular file: {path}")


def _require_exact_keys(value: Any, keys: set[str], description: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise VerificationError(f"{description} must be a JSON object")
    observed = set(value)
    if observed != keys:
        raise VerificationError(
            f"{description} keys mismatch; missing={sorted(keys - observed)}, "
            f"extra={sorted(observed - keys)}"
        )
    return value


def _require_nonempty_string(value: Any, description: str) -> str:
    if not isinstance(value, str) or not value:
        raise VerificationError(f"{description} must be a non-empty string")
    return value


def _validate_source_sha(value: str, description: str = "source SHA") -> str:
    if not isinstance(value, str) or SOURCE_SHA_RE.fullmatch(value) is None:
        raise VerificationError(f"{description} must be 40 lowercase hexadecimal characters")
    return value


def _validate_digest(value: str, description: str = "digest") -> str:
    if not isinstance(value, str) or DIGEST_RE.fullmatch(value) is None:
        raise VerificationError(f"{description} must be a lowercase SHA-256 digest")
    return value


def _validate_image_name(value: str) -> str:
    if not isinstance(value, str) or IMAGE_NAME_RE.fullmatch(value) is None:
        raise VerificationError(
            "image name must be a canonical lowercase ghcr.io repository path"
        )
    return value


def _validate_source_url(value: str) -> str:
    if not isinstance(value, str):
        raise VerificationError("source URL must be a canonical https://github.com path")
    parsed = urllib.parse.urlsplit(value)
    if (
        parsed.scheme != "https"
        or parsed.netloc != "github.com"
        or not parsed.path.strip("/")
        or parsed.query
        or parsed.fragment
    ):
        raise VerificationError("source URL must be a canonical https://github.com path")
    return value.rstrip("/")


def _validate_publication_scope(image: str, source_url: str) -> tuple[str, str]:
    canonical_image = _validate_image_name(image)
    canonical_source = _validate_source_url(source_url)
    source_parts = urllib.parse.urlsplit(canonical_source).path.strip("/").split("/")
    if len(source_parts) != 2 or any(not part for part in source_parts):
        raise VerificationError("source URL must identify exactly one GitHub owner/repository")
    owner, repository = source_parts
    expected_image = f"ghcr.io/{owner.lower()}/sorotte-server"
    if canonical_image != expected_image:
        raise VerificationError(
            f"publication image must be the source owner's sorotte-server repository: "
            f"{expected_image}"
        )
    return owner, repository


def _validate_artifacts_root(repo_root: Path, artifacts_root: Path) -> None:
    resolved_repo = repo_root.resolve()
    resolved_artifacts = artifacts_root.resolve()
    target_root = (resolved_repo / "target").resolve()
    try:
        relative = resolved_artifacts.relative_to(target_root)
    except ValueError as error:
        raise VerificationError(
            "container smoke artifacts must stay under the repository target directory"
        ) from error
    if not relative.parts:
        raise VerificationError(
            "container smoke artifacts must use a dedicated child of target"
        )


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return f"sha256:{digest.hexdigest()}"


def _write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    os.replace(temporary, path)


def _run(
    command: Sequence[str],
    *,
    timeout: int = COMMAND_TIMEOUT_SECONDS,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            list(command),
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise VerificationError(f"command could not complete: {command[0]}: {error}") from error
    if check and result.returncode != 0:
        output = result.stdout.strip()
        raise VerificationError(
            f"command exited {result.returncode}: {' '.join(command)}\n{output}"
        )
    return result


def _docker_json(command: Sequence[str], description: str) -> Any:
    result = _run(["docker", *command])
    return _load_json_bytes(result.stdout.encode("utf-8"), description)


def inspect_local_image(
    image: str,
    *,
    expected_source_sha: str,
    expected_source_url: str,
) -> dict[str, Any]:
    _validate_source_sha(expected_source_sha)
    _validate_source_url(expected_source_url)
    inspected = _docker_json(["image", "inspect", image], "docker image inspection")
    if not isinstance(inspected, list) or len(inspected) != 1:
        raise VerificationError("docker image inspection must return exactly one image")
    item = inspected[0]
    if not isinstance(item, dict):
        raise VerificationError("docker image inspection entry must be an object")
    image_id = _validate_digest(item.get("Id"), "local image ID/config digest")
    if item.get("Os") != "linux" or item.get("Architecture") != "amd64":
        raise VerificationError(
            f"tested image must be linux/amd64, received "
            f"{item.get('Os')}/{item.get('Architecture')}"
        )
    config = item.get("Config")
    if not isinstance(config, dict):
        raise VerificationError("docker image inspection is missing Config")
    labels = config.get("Labels")
    if not isinstance(labels, dict):
        raise VerificationError("tested image must have OCI labels")
    expected_labels = {
        EXPECTED_SOURCE_LABEL: expected_source_url,
        EXPECTED_REVISION_LABEL: expected_source_sha,
        EXPECTED_LICENSE_LABEL: "Apache-2.0",
    }
    for key, expected in expected_labels.items():
        if labels.get(key) != expected:
            raise VerificationError(
                f"tested image label {key} mismatch: expected {expected!r}, "
                f"received {labels.get(key)!r}"
            )
    created = labels.get(EXPECTED_CREATED_LABEL)
    if not isinstance(created, str) or re.fullmatch(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", created
    ) is None:
        raise VerificationError("tested image must have a canonical UTC OCI created label")
    if config.get("User") not in {"sorotte", "10001", "10001:10001"}:
        raise VerificationError("tested image must run as the non-root sorotte user")
    if config.get("Entrypoint") != EXPECTED_ENTRYPOINT:
        raise VerificationError(
            f"tested image entrypoint mismatch: {config.get('Entrypoint')!r}"
        )
    if config.get("Cmd") != EXPECTED_DEFAULT_COMMAND:
        raise VerificationError(f"tested image default command mismatch: {config.get('Cmd')!r}")
    rootfs = item.get("RootFS")
    if not isinstance(rootfs, dict) or rootfs.get("Type") != "layers":
        raise VerificationError("tested image must expose a layer-backed RootFS")
    diff_ids = rootfs.get("Layers")
    if not isinstance(diff_ids, list) or not diff_ids:
        raise VerificationError("tested image must contain at least one RootFS diff ID")
    for index, digest in enumerate(diff_ids):
        _validate_digest(digest, f"local RootFS diff ID {index}")
    return {
        "architecture": "amd64",
        "created": created,
        "entrypoint": EXPECTED_ENTRYPOINT,
        "id": image_id,
        "os": "linux",
        "rootfsDiffIds": diff_ids,
        "source": expected_source_url,
        "sourceSha": expected_source_sha,
        "user": config["User"],
    }


def read_publication_tags(
    path: Path,
    *,
    expected_image: str,
    expected_source_sha: str,
) -> list[str]:
    _require_regular_file(path, "container tag inventory")
    if path.stat().st_size > 16 * 1024:
        raise VerificationError("container tag inventory exceeds 16 KiB")
    try:
        text = path.read_bytes().decode("ascii")
    except UnicodeDecodeError as error:
        raise VerificationError("container tag inventory must be ASCII") from error
    if "\r" in text or not text.endswith("\n"):
        raise VerificationError("container tag inventory must use canonical LF-terminated lines")
    image = _validate_image_name(expected_image)
    source_sha = _validate_source_sha(expected_source_sha)
    tags: list[str] = []
    seen: set[str] = set()
    for line in text.splitlines():
        if not line or line != line.strip():
            raise VerificationError("container tag inventory contains an empty or padded line")
        prefix = f"{image}:"
        if not line.startswith(prefix):
            raise VerificationError(f"container tag is outside expected repository: {line!r}")
        tag = line[len(prefix) :]
        if TAG_RE.fullmatch(tag) is None:
            raise VerificationError(f"container tag is not canonical: {tag!r}")
        if line in seen:
            raise VerificationError(f"container tag inventory contains a duplicate: {line}")
        seen.add(line)
        tags.append(line)
    if not tags:
        raise VerificationError("container tag inventory must not be empty")
    exact_sha_tag = f"{image}:sha-{source_sha}"
    if exact_sha_tag not in seen:
        raise VerificationError(f"container tag inventory is missing {exact_sha_tag}")
    return tags


def _container_running(name: str) -> bool:
    result = _run(
        ["docker", "container", "inspect", "--format", "{{.State.Running}}", name],
        check=False,
    )
    return result.returncode == 0 and result.stdout.strip() == "true"


def _published_loopback_port(name: str) -> int:
    result = _run(["docker", "port", name, "8999/tcp"])
    lines = [line.strip() for line in result.stdout.splitlines() if line.strip()]
    if len(lines) != 1:
        raise VerificationError(f"container must publish exactly one loopback port: {lines!r}")
    match = re.fullmatch(r"127\.0\.0\.1:(\d{1,5})", lines[0])
    if match is None:
        raise VerificationError(f"container port is not loopback-only: {lines[0]!r}")
    port = int(match.group(1))
    if not 1 <= port <= 65535:
        raise VerificationError(f"container published invalid port {port}")
    return port


def _wait_for_container_port(name: str) -> int:
    deadline = time.monotonic() + SERVER_START_TIMEOUT_SECONDS
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        if not _container_running(name):
            raise VerificationError("server container exited before accepting a connection")
        try:
            port = _published_loopback_port(name)
            with socket.create_connection(("127.0.0.1", port), timeout=0.25):
                return port
        except (OSError, VerificationError) as error:
            last_error = error
            time.sleep(0.05)
    raise VerificationError(f"server container did not listen before deadline: {last_error}")


class _ProtocolSession:
    def __init__(self, connection: socket.socket) -> None:
        self.connection = connection
        self.buffered = b""

    def send(self, message: dict[str, Any]) -> None:
        payload = json.dumps(message, separators=(",", ":")).encode("utf-8") + b"\r\n"
        self.connection.sendall(payload)

    def receive_until(
        self,
        matchers: dict[str, Callable[[dict[str, Any]], bool]],
        description: str,
    ) -> dict[str, dict[str, Any]]:
        pending = dict(matchers)
        matched: dict[str, dict[str, Any]] = {}
        deadline = time.monotonic() + SERVER_START_TIMEOUT_SECONDS
        messages_seen = 0
        while pending and time.monotonic() < deadline and messages_seen < 256:
            while b"\n" not in self.buffered:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                self.connection.settimeout(min(1.0, remaining))
                try:
                    chunk = self.connection.recv(65536)
                except socket.timeout:
                    continue
                if not chunk:
                    raise VerificationError(
                        f"server closed before completing {description}: "
                        f"{sorted(pending)}"
                    )
                self.buffered += chunk
                if len(self.buffered) > MAX_PROTOCOL_BYTES:
                    raise VerificationError("server protocol line exceeded 1 MiB")
            if b"\n" not in self.buffered:
                continue
            raw, self.buffered = self.buffered.split(b"\n", 1)
            message = _load_json_bytes(raw.rstrip(b"\r"), "server protocol line")
            if not isinstance(message, dict):
                raise VerificationError("server protocol line must be a JSON object")
            messages_seen += 1
            for name, matcher in list(pending.items()):
                if matcher(message):
                    matched[name] = message
                    del pending[name]
        if pending:
            raise VerificationError(
                f"server did not complete {description} before the bounded read limit: "
                f"{sorted(pending)}"
            )
        return matched


def _hello_message(
    username: str,
    room: str,
    *,
    persistent_features: bool = False,
) -> dict[str, Any]:
    if not username or len(username) > CONTAINER_TEST_USERNAME_MAX_CHARACTERS:
        raise VerificationError(
            "container smoke username must fit the server's default 16-character limit"
        )
    hello: dict[str, Any] = {
        "Hello": {
            "username": username,
            "room": {"name": room},
            "version": "1.7.5",
        }
    }
    if persistent_features:
        hello["Hello"]["features"] = {
            "featureList": True,
            "persistentRooms": True,
            "sharedPlaylists": True,
            "uiMode": "GUI",
        }
    return hello


def _protocol_hello(
    session: _ProtocolSession,
    *,
    username: str,
    room: str,
    persistent_features: bool = False,
) -> dict[str, Any]:
    session.send(
        _hello_message(
            username,
            room,
            persistent_features=persistent_features,
        )
    )
    return session.receive_until(
        {
            "hello": lambda message: (
                _pointer_equals(message, ["Hello", "username"], username)
                and _pointer_equals(message, ["Hello", "room", "name"], room)
            )
        },
        "protocol Hello",
    )["hello"]


def _pointer_equals(message: dict[str, Any], keys: Sequence[str], expected: Any) -> bool:
    value: Any = message
    for key in keys:
        if not isinstance(value, dict) or key not in value:
            return False
        value = value[key]
    return value == expected


def _verify_sqlite(path: Path, description: str) -> dict[str, Any]:
    _require_regular_file(path, description)
    connection: sqlite3.Connection | None = None
    try:
        connection = sqlite3.connect(f"file:{path.as_posix()}?mode=ro", uri=True)
        integrity = connection.execute("PRAGMA integrity_check").fetchone()
    except sqlite3.Error as error:
        raise VerificationError(f"{description} is not readable SQLite: {error}") from error
    finally:
        if connection is not None:
            connection.close()
    if integrity != ("ok",):
        raise VerificationError(f"{description} failed SQLite integrity_check: {integrity!r}")
    return {
        "integrityCheck": "ok",
        "path": path.name,
        "sha256": _sha256_file(path),
        "size": path.stat().st_size,
    }


def _verify_persisted_room_row(
    path: Path,
    *,
    room: str,
    playlist: list[str],
    playlist_index: int,
    position: float,
) -> dict[str, Any]:
    _require_regular_file(path, "plaintext persisted rooms database")
    connection: sqlite3.Connection | None = None
    try:
        connection = sqlite3.connect(f"file:{path.as_posix()}?mode=ro", uri=True)
        integrity = connection.execute("PRAGMA integrity_check").fetchone()
        rows = connection.execute(
            "SELECT name, playlist, playlistJson, playlistIndex, position, "
            "lastSavedUpdate, persistenceVersion, ownerBucket, createdAt "
            "FROM persistent_rooms ORDER BY name"
        ).fetchall()
    except sqlite3.Error as error:
        raise VerificationError(
            f"plaintext persisted room row is not readable SQLite: {error}"
        ) from error
    finally:
        if connection is not None:
            connection.close()
    if integrity != ("ok",):
        raise VerificationError(
            f"plaintext persisted rooms database failed integrity_check: {integrity!r}"
        )
    if len(rows) != 1:
        raise VerificationError(
            f"plaintext persistence must create exactly one raw room row, received {len(rows)}"
        )
    row = rows[0]
    expected_payload = (
        room,
        "\n".join(playlist),
        json.dumps(playlist, separators=(",", ":")),
        playlist_index,
        position,
    )
    if row[:5] != expected_payload:
        raise VerificationError(
            "plaintext persisted room payload mismatch: "
            f"expected {expected_payload!r}, received {row[:5]!r}"
        )
    last_saved, version, owner_bucket, created_at = row[5:]
    for name, value in {
        "lastSavedUpdate": last_saved,
        "createdAt": created_at,
    }.items():
        if (
            isinstance(value, bool)
            or not isinstance(value, (int, float))
            or not math.isfinite(value)
            or value <= 0
        ):
            raise VerificationError(f"plaintext persisted room {name} is invalid: {value!r}")
    if type(version) is not int or version <= 0:
        raise VerificationError(
            f"plaintext persisted room persistenceVersion is invalid: {version!r}"
        )
    if (
        not isinstance(owner_bucket, str)
        or re.fullmatch(r"quota:v1:[0-9a-f]{64}", owner_bucket) is None
    ):
        raise VerificationError(
            f"plaintext persisted room ownerBucket is invalid: {owner_bucket!r}"
        )
    return {
        "integrityCheck": "ok",
        "row": {
            "createdAt": created_at,
            "lastSavedUpdate": last_saved,
            "name": row[0],
            "ownerBucket": owner_bucket,
            "persistenceVersion": version,
            "playlist": row[1],
            "playlistIndex": row[3],
            "playlistJson": row[2],
            "position": row[4],
        },
    }


def _copy_tls_fixtures(repo_root: Path, destination: Path) -> None:
    destination.mkdir()
    mapping = {
        "test_privkey.pem": "privkey.pem",
        "test_cert.pem": "cert.pem",
        "test_chain.pem": "chain.pem",
    }
    for source_name, destination_name in mapping.items():
        source = repo_root / "fixtures" / "tls" / source_name
        _require_regular_file(source, f"TLS fixture {source_name}")
        shutil.copyfile(source, destination / destination_name)


def _stop_and_inspect_container(name: str) -> dict[str, Any]:
    _run(["docker", "kill", "--signal=SIGINT", name], timeout=SERVER_STOP_TIMEOUT_SECONDS)
    wait = _run(["docker", "wait", name], timeout=SERVER_STOP_TIMEOUT_SECONDS)
    try:
        exit_code = int(wait.stdout.strip())
    except ValueError as error:
        raise VerificationError(
            f"docker wait returned invalid exit code: {wait.stdout!r}"
        ) from error
    state = _docker_json(
        ["container", "inspect", "--format", "{{json .State}}", name],
        "stopped container state",
    )
    if not isinstance(state, dict):
        raise VerificationError("stopped container state must be an object")
    if (
        exit_code != 0
        or state.get("Status") != "exited"
        or state.get("Running") is not False
        or state.get("Dead") is not False
        or state.get("ExitCode") != 0
        or state.get("OOMKilled") is not False
        or state.get("Error") != ""
    ):
        raise VerificationError(f"server container did not stop cleanly: {state!r}")
    return {
        "error": state.get("Error", ""),
        "exitCode": 0,
        "oomKilled": False,
        "signal": "SIGINT",
    }


def _container_command(
    *,
    image: str,
    name: str,
    state_root: Path,
    tls_root: Path | None,
) -> list[str]:
    command = [
        "docker",
        "run",
        "--detach",
        "--name",
        name,
        "--publish",
        "127.0.0.1::8999/tcp",
        "--mount",
        f"type=bind,src={state_root.resolve()},dst=/data",
    ]
    if tls_root is not None:
        command.extend(
            [
                "--mount",
                f"type=bind,src={tls_root.resolve()},dst=/tls,readonly",
            ]
        )
    command.extend(
        [
            image,
            "--port",
            "8999",
            "--ipv4-only",
            "--interface-ipv4",
            "0.0.0.0",
            "--isolate-rooms",
            "--rooms-db-file",
            "/data/rooms.sqlite3",
            "--stats-db-file",
            "/data/stats.sqlite3",
        ]
    )
    if tls_root is not None:
        command.extend(["--tls", "/tls"])
    return command


def _start_container(
    *,
    image: str,
    name: str,
    state_root: Path,
    tls_root: Path | None,
) -> tuple[str, int]:
    started = _run(
        _container_command(
            image=image,
            name=name,
            state_root=state_root,
            tls_root=tls_root,
        )
    )
    container_id = started.stdout.strip()
    if CONTAINER_ID_RE.fullmatch(container_id) is None:
        raise VerificationError(f"docker run returned invalid container ID: {container_id!r}")
    return container_id, _wait_for_container_port(name)


def _drain_after_stop(
    *,
    name: str,
    connections: Sequence[socket.socket],
) -> dict[str, Any]:
    stop = _stop_and_inspect_container(name)
    for index, connection in enumerate(connections):
        drained = False
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            connection.settimeout(min(0.25, max(0.01, deadline - time.monotonic())))
            try:
                trailing = connection.recv(65536)
            except socket.timeout:
                continue
            except (ssl.SSLEOFError, ConnectionResetError):
                drained = True
                break
            if not trailing:
                drained = True
                break
        if not drained:
            raise VerificationError(
                f"live client session {index} did not receive EOF during shutdown"
            )
    return stop


def _write_container_log_and_remove(
    name: str,
    path: Path,
    *,
    require_shutdown_marker: bool,
) -> None:
    primary_error = sys.exc_info()[1]
    marker = (
        CONTAINER_SHUTDOWN_LOG_MARKER
        if require_shutdown_marker
        else CONTAINER_STARTUP_LOG_MARKER
    )
    marker_description = (
        "graceful shutdown barrier"
        if require_shutdown_marker
        else "startup listener marker"
    )
    logs: subprocess.CompletedProcess[str] | None = None
    cleanup_errors: list[str] = []
    try:
        for attempt in range(CONTAINER_LOG_CAPTURE_ATTEMPTS):
            logs = _run(["docker", "logs", name], check=False)
            path.write_text(logs.stdout, encoding="utf-8", newline="\n")
            if logs.returncode == 0 and marker in logs.stdout:
                break
            if attempt + 1 < CONTAINER_LOG_CAPTURE_ATTEMPTS:
                time.sleep(CONTAINER_LOG_CAPTURE_RETRY_SECONDS)
        else:
            command_detail = (
                f"; docker logs exited {logs.returncode}"
                if logs is not None and logs.returncode != 0
                else ""
            )
            raise VerificationError(
                f"{path.stem} container log did not retain the {marker_description}"
                f"{command_detail}"
            )
    except (OSError, VerificationError) as error:
        cleanup_errors.append(str(error))

    try:
        removed = _run(["docker", "rm", "--force", name], check=False)
        if removed.returncode != 0:
            cleanup_errors.append(
                f"docker rm exited {removed.returncode}: {removed.stdout.strip()}"
            )
    except VerificationError as error:
        cleanup_errors.append(str(error))

    if not cleanup_errors:
        return
    cleanup_detail = "; ".join(cleanup_errors)
    if primary_error is not None:
        raise VerificationError(
            f"{primary_error}; container diagnostics/cleanup also failed: "
            f"{cleanup_detail}"
        ) from primary_error
    raise VerificationError(cleanup_detail)


def _run_plaintext_persistence_scenario(
    *,
    image: str,
    scenario: str,
    state_root: Path,
    artifacts_root: Path,
) -> dict[str, Any]:
    room = "container-persistence-restore-7e91"
    writer_username = "writer-7e91"
    watcher_username = "watcher-4b27"
    restorer_username = "restore-8c13"
    playlist = [
        "container-persistence-alpha-7e91.mkv",
        "container-persistence-beta-4b27.mkv",
    ]
    playlist_index = 1
    position = 137.25
    accepted_state = {
        "playlist": playlist,
        "playlistIndex": playlist_index,
        "playstate": {"paused": True, "position": position},
    }
    state_root.mkdir()
    state_root.chmod(0o777)

    first_name = f"sorotte-container-verify-{scenario}-{uuid.uuid4().hex[:12]}"
    first_log = artifacts_root / f"{scenario}-container.log"
    first_connections: list[socket.socket] = []
    first_id: str | None = None
    first_stop: dict[str, Any] | None = None
    try:
        first_id, port = _start_container(
            image=image,
            name=first_name,
            state_root=state_root,
            tls_root=None,
        )
        writer_socket = socket.create_connection(
            ("127.0.0.1", port), timeout=SERVER_START_TIMEOUT_SECONDS
        )
        first_connections.append(writer_socket)
        writer = _ProtocolSession(writer_socket)
        _protocol_hello(
            writer,
            username=writer_username,
            room=room,
            persistent_features=True,
        )

        watcher_socket = socket.create_connection(
            ("127.0.0.1", port), timeout=SERVER_START_TIMEOUT_SECONDS
        )
        first_connections.append(watcher_socket)
        watcher = _ProtocolSession(watcher_socket)
        _protocol_hello(
            watcher,
            username=watcher_username,
            room=room,
            persistent_features=True,
        )
        writer.receive_until(
            {
                "watcherJoined": lambda message: _pointer_equals(
                    message,
                    ["Set", "user", watcher_username, "event", "joined"],
                    True,
                )
            },
            "same-room watcher join",
        )

        writer.send({"Set": {"playlistChange": {"files": playlist}}})
        watcher.receive_until(
            {
                "playlist": lambda message: _pointer_equals(
                    message, ["Set", "playlistChange", "files"], playlist
                )
            },
            "accepted playlist fanout",
        )
        writer.send({"Set": {"playlistIndex": {"index": playlist_index}}})
        watcher.receive_until(
            {
                "playlistIndex": lambda message: _pointer_equals(
                    message, ["Set", "playlistIndex", "index"], playlist_index
                )
            },
            "accepted playlist index fanout",
        )
        writer.send(
            {
                "State": {
                    "playstate": {
                        "doSeek": True,
                        "paused": True,
                        "position": position,
                    }
                }
            }
        )
        watcher.receive_until(
            {
                "playstate": lambda message: (
                    _pointer_equals(
                        message, ["State", "playstate", "position"], position
                    )
                    and _pointer_equals(
                        message, ["State", "playstate", "paused"], True
                    )
                )
            },
            "accepted playstate fanout",
        )
        first_stop = _drain_after_stop(
            name=first_name,
            connections=first_connections,
        )
    finally:
        for connection in first_connections:
            connection.close()
        _write_container_log_and_remove(
            first_name,
            first_log,
            require_shutdown_marker=first_stop is not None,
        )
    if first_id is None or first_stop is None:
        raise VerificationError("plaintext persistence write phase did not complete")

    rooms_path = state_root / "rooms.sqlite3"
    raw_after_write = _verify_persisted_room_row(
        rooms_path,
        room=room,
        playlist=playlist,
        playlist_index=playlist_index,
        position=position,
    )

    restart_name = f"sorotte-container-verify-{scenario}-restart-{uuid.uuid4().hex[:12]}"
    restart_log = artifacts_root / f"{scenario}-restart-container.log"
    restart_connections: list[socket.socket] = []
    restart_id: str | None = None
    restart_stop: dict[str, Any] | None = None
    try:
        restart_id, port = _start_container(
            image=image,
            name=restart_name,
            state_root=state_root,
            tls_root=None,
        )
        restorer_socket = socket.create_connection(
            ("127.0.0.1", port), timeout=SERVER_START_TIMEOUT_SECONDS
        )
        restart_connections.append(restorer_socket)
        restorer = _ProtocolSession(restorer_socket)
        restorer.send(
            _hello_message(
                restorer_username,
                room,
                persistent_features=True,
            )
        )
        restorer.receive_until(
            {
                "hello": lambda message: (
                    _pointer_equals(
                        message,
                        ["Hello", "username"],
                        restorer_username,
                    )
                    and _pointer_equals(message, ["Hello", "room", "name"], room)
                ),
                "playlist": lambda message: _pointer_equals(
                    message, ["Set", "playlistChange", "files"], playlist
                ),
                "playlistIndex": lambda message: _pointer_equals(
                    message, ["Set", "playlistIndex", "index"], playlist_index
                ),
                "playstate": lambda message: (
                    _pointer_equals(
                        message, ["State", "playstate", "position"], position
                    )
                    and _pointer_equals(
                        message, ["State", "playstate", "paused"], True
                    )
                ),
            },
            "complete restored room snapshot after restart",
        )
        restart_stop = _drain_after_stop(
            name=restart_name,
            connections=restart_connections,
        )
    finally:
        for connection in restart_connections:
            connection.close()
        _write_container_log_and_remove(
            restart_name,
            restart_log,
            require_shutdown_marker=restart_stop is not None,
        )
    if restart_id is None or restart_stop is None:
        raise VerificationError("plaintext persistence restart phase did not complete")

    raw_after_restart = _verify_persisted_room_row(
        rooms_path,
        room=room,
        playlist=playlist,
        playlist_index=playlist_index,
        position=position,
    )
    databases = [
        _verify_sqlite(rooms_path, f"{scenario} rooms database"),
        _verify_sqlite(state_root / "stats.sqlite3", f"{scenario} stats database"),
    ]
    return {
        "clientSessionDrained": True,
        "containerId": first_id,
        "databases": databases,
        "log": first_log.name,
        "persistence": {
            "accepted": accepted_state,
            "rawAfterRestart": raw_after_restart,
            "rawAfterWrite": raw_after_write,
            "restored": accepted_state,
            "room": room,
            "sameLoadedImage": True,
            "sameStateDirectory": True,
        },
        "protocolHello": True,
        "restart": {
            "clientSessionDrained": True,
            "containerId": restart_id,
            "log": restart_log.name,
            "protocolHello": True,
            "shutdown": restart_stop,
        },
        "scenario": scenario,
        "shutdown": first_stop,
        "tls": None,
    }


def _run_container_scenario(
    *,
    image: str,
    scenario: str,
    state_root: Path,
    tls_root: Path | None,
    artifacts_root: Path,
) -> dict[str, Any]:
    if tls_root is None:
        return _run_plaintext_persistence_scenario(
            image=image,
            scenario=scenario,
            state_root=state_root,
            artifacts_root=artifacts_root,
        )
    name = f"sorotte-container-verify-{scenario}-{uuid.uuid4().hex[:12]}"
    state_root.mkdir()
    state_root.chmod(0o777)
    log_path = artifacts_root / f"{scenario}-container.log"
    container_id: str | None = None
    stop: dict[str, Any] | None = None
    protocol: dict[str, Any] | None = None
    tls_evidence: dict[str, Any] | None = None
    client_session_drained = False
    connection: socket.socket | None = None
    try:
        container_id, port = _start_container(
            image=image,
            name=name,
            state_root=state_root,
            tls_root=tls_root,
        )
        raw_connection = socket.create_connection(
            ("127.0.0.1", port), timeout=SERVER_START_TIMEOUT_SECONDS
        )
        connection = raw_connection
        try:
            context = ssl.create_default_context(cafile=str(tls_root / "cert.pem"))
            connection = context.wrap_socket(raw_connection, server_hostname="localhost")
            peer = connection.getpeercert(binary_form=True)
            if not peer:
                raise VerificationError("TLS server did not present a certificate")
            tls_evidence = {
                "cipher": connection.cipher()[0],
                "peerCertificateSha256": f"sha256:{hashlib.sha256(peer).hexdigest()}",
                "version": connection.version(),
            }
            protocol = _protocol_hello(
                _ProtocolSession(connection),
                username="tls-client-7e91",
                room="container-tls-verification",
            )
            stop = _drain_after_stop(name=name, connections=[connection])
            client_session_drained = True
        finally:
            connection.close()
    finally:
        _write_container_log_and_remove(
            name,
            log_path,
            require_shutdown_marker=stop is not None,
        )
    if container_id is None or protocol is None or stop is None:
        raise VerificationError(f"{scenario} container scenario did not complete")
    databases = [
        _verify_sqlite(state_root / "rooms.sqlite3", f"{scenario} rooms database"),
        _verify_sqlite(state_root / "stats.sqlite3", f"{scenario} stats database"),
    ]
    return {
        "containerId": container_id,
        "clientSessionDrained": client_session_drained,
        "databases": databases,
        "log": log_path.name,
        "persistence": None,
        "protocolHello": True,
        "restart": None,
        "scenario": scenario,
        "shutdown": stop,
        "tls": tls_evidence,
    }


def smoke_loaded_image(
    *,
    image: str,
    expected_source_sha: str,
    expected_source_url: str,
    repo_root: Path,
    artifacts_root: Path,
) -> dict[str, Any]:
    if os.name != "posix":
        raise VerificationError(
            "container runtime smoke is CI-owned and requires a POSIX Docker host"
        )
    _validate_artifacts_root(repo_root, artifacts_root)
    if artifacts_root.exists():
        raise VerificationError(f"container smoke artifacts root must be fresh: {artifacts_root}")
    artifacts_root.mkdir(parents=True)
    local_image = inspect_local_image(
        image,
        expected_source_sha=expected_source_sha,
        expected_source_url=expected_source_url,
    )
    tls_root = artifacts_root / "tls"
    _copy_tls_fixtures(repo_root, tls_root)
    scenarios = [
        _run_container_scenario(
            image=image,
            scenario="plaintext-persistence",
            state_root=artifacts_root / "plaintext-state",
            tls_root=None,
            artifacts_root=artifacts_root,
        ),
        _run_container_scenario(
            image=image,
            scenario="tls-persistence",
            state_root=artifacts_root / "tls-state",
            tls_root=tls_root,
            artifacts_root=artifacts_root,
        ),
    ]
    return {
        "image": image,
        "localImage": local_image,
        "schemaVersion": 1,
        "scenarios": scenarios,
        "status": "passed",
    }


def verify_sbom(
    *,
    sbom_path: Path,
    runtime_report_path: Path,
) -> dict[str, Any]:
    sbom = _load_json_file(sbom_path, "SPDX SBOM")
    if not isinstance(sbom, dict):
        raise VerificationError("SPDX SBOM must be an object")
    if not isinstance(sbom.get("spdxVersion"), str) or not sbom["spdxVersion"].startswith(
        "SPDX-2."
    ):
        raise VerificationError("SBOM must use SPDX 2.x JSON")
    if sbom.get("dataLicense") != "CC0-1.0":
        raise VerificationError("SPDX SBOM dataLicense must be CC0-1.0")
    _require_nonempty_string(sbom.get("documentNamespace"), "SPDX documentNamespace")
    packages = sbom.get("packages")
    if not isinstance(packages, list) or not packages:
        raise VerificationError("SPDX SBOM must contain at least one package")
    creation = sbom.get("creationInfo")
    if not isinstance(creation, dict):
        raise VerificationError("SPDX SBOM must contain creationInfo")
    creators = creation.get("creators")
    if not isinstance(creators, list) or "Tool: syft-1.44.0" not in creators:
        raise VerificationError("SPDX SBOM must identify exactly Syft 1.44.0")
    runtime = parse_runtime_report(runtime_report_path)
    reinspected = inspect_local_image(
        runtime["image"],
        expected_source_sha=runtime["localImage"]["sourceSha"],
        expected_source_url=runtime["localImage"]["source"],
    )
    if reinspected != runtime["localImage"]:
        raise VerificationError(
            "exact local image tag changed between runtime consumption and SBOM verification"
        )
    return {
        "bindingMode": "pinned-syft-input-plus-daemon-reinspection",
        "image": runtime["image"],
        "localImageId": reinspected["id"],
        "packageCount": len(packages),
        "rootfsDiffIds": reinspected["rootfsDiffIds"],
        "sbomSha256": _sha256_file(sbom_path),
        "schemaVersion": 1,
        "source": reinspected["source"],
        "sourceSha": reinspected["sourceSha"],
        "spdxVersion": sbom["spdxVersion"],
        "status": "passed",
    }


def publish_tested_image(
    *,
    local_image: str,
    image_name: str,
    tags_path: Path,
    expected_source_sha: str,
    expected_source_url: str,
) -> dict[str, Any]:
    _validate_publication_scope(image_name, expected_source_url)
    tags = read_publication_tags(
        tags_path,
        expected_image=image_name,
        expected_source_sha=expected_source_sha,
    )
    local = inspect_local_image(
        local_image,
        expected_source_sha=expected_source_sha,
        expected_source_url=expected_source_url,
    )
    observed_digest: str | None = None
    pushes: list[dict[str, str]] = []
    for tag in tags:
        _run(["docker", "image", "tag", local_image, tag])
        result = _run(["docker", "image", "push", tag], timeout=15 * 60)
        digests = sorted(set(PUSH_DIGEST_RE.findall(result.stdout)))
        if len(digests) != 1:
            raise VerificationError(
                f"docker push for {tag} must report exactly one manifest digest: {digests}"
            )
        digest = _validate_digest(digests[0], f"registry manifest digest for {tag}")
        if observed_digest is None:
            observed_digest = digest
        elif digest != observed_digest:
            raise VerificationError(
                f"published tags diverged: expected {observed_digest}, received {digest} for {tag}"
            )
        pushes.append({"digest": digest, "tag": tag})
    if observed_digest is None:
        raise VerificationError("no image tag was published")
    return {
        "digest": observed_digest,
        "image": _validate_image_name(image_name),
        "localImageId": local["id"],
        "pushes": pushes,
        "schemaVersion": 1,
        "source": expected_source_url,
        "sourceSha": expected_source_sha,
        "status": "passed",
        "tags": tags,
    }


def _parse_ghcr_repository(image: str) -> str:
    _validate_image_name(image)
    return image.removeprefix("ghcr.io/")


def _anonymous_ghcr_token(repository: str) -> str:
    query = urllib.parse.urlencode(
        {"scope": f"repository:{repository}:pull", "service": "ghcr.io"}
    )
    request = urllib.request.Request(
        f"https://ghcr.io/token?{query}",
        headers={"Accept": "application/json", "User-Agent": "sorotte-container-verifier/1"},
    )
    try:
        with urllib.request.urlopen(
            request, timeout=REGISTRY_REQUEST_TIMEOUT_SECONDS
        ) as response:
            payload = response.read(MAX_JSON_BYTES + 1)
    except (OSError, urllib.error.URLError) as error:
        raise VerificationError(f"anonymous GHCR token request failed: {error}") from error
    token_response = _load_json_bytes(payload, "anonymous GHCR token response")
    if not isinstance(token_response, dict):
        raise VerificationError("anonymous GHCR token response must be an object")
    token = token_response.get("token") or token_response.get("access_token")
    return _require_nonempty_string(token, "anonymous GHCR pull token")


def _registry_get(
    repository: str,
    relative_path: str,
    *,
    token: str,
    accept: str,
) -> tuple[bytes, dict[str, str]]:
    url = f"https://ghcr.io/v2/{repository}/{relative_path}"
    request = urllib.request.Request(
        url,
        headers={
            "Accept": accept,
            "Authorization": f"Bearer {token}",
            "User-Agent": "sorotte-container-verifier/1",
        },
    )
    try:
        with urllib.request.urlopen(
            request, timeout=REGISTRY_REQUEST_TIMEOUT_SECONDS
        ) as response:
            payload = response.read(MAX_JSON_BYTES + 1)
            headers = {key.lower(): value for key, value in response.headers.items()}
    except urllib.error.HTTPError:
        raise
    except (OSError, urllib.error.URLError, http.client.HTTPException) as error:
        raise VerificationError(
            f"anonymous GHCR request failed for {relative_path}: {error}"
        ) from error
    if len(payload) > MAX_JSON_BYTES:
        raise VerificationError(f"anonymous GHCR response exceeds size limit: {relative_path}")
    return payload, headers


def _fetch_public_manifest(
    repository: str,
    reference: str,
    *,
    token: str,
    sleep: Callable[[float], None] = time.sleep,
) -> tuple[dict[str, Any], str, bytes]:
    accept = ", ".join(
        [
            "application/vnd.oci.image.manifest.v1+json",
            "application/vnd.docker.distribution.manifest.v2+json",
        ]
    )
    last_error: Exception | None = None
    for attempt in range(REGISTRY_ATTEMPTS):
        try:
            payload, headers = _registry_get(
                repository,
                f"manifests/{urllib.parse.quote(reference, safe=':')}",
                token=token,
                accept=accept,
            )
            digest = headers.get("docker-content-digest")
            _validate_digest(digest or "", "anonymous registry content digest")
            computed = f"sha256:{hashlib.sha256(payload).hexdigest()}"
            if computed != digest:
                raise VerificationError(
                    f"anonymous registry manifest bytes hash to {computed}, header names {digest}"
                )
            manifest = _load_json_bytes(payload, f"public manifest {reference}")
            if not isinstance(manifest, dict):
                raise VerificationError("public image manifest must be an object")
            if manifest.get("schemaVersion") != 2:
                raise VerificationError("public image manifest schemaVersion must be 2")
            if manifest.get("mediaType") not in {
                "application/vnd.oci.image.manifest.v1+json",
                "application/vnd.docker.distribution.manifest.v2+json",
            }:
                raise VerificationError(
                    f"public tag did not resolve to a single image manifest: "
                    f"{manifest.get('mediaType')!r}"
                )
            return manifest, digest, payload
        except urllib.error.HTTPError as error:
            last_error = error
            if error.code not in {404, 429, 500, 502, 503, 504}:
                raise VerificationError(
                    f"anonymous GHCR manifest request returned HTTP {error.code}; "
                    "the package may not be public"
                ) from error
        except VerificationError as error:
            last_error = error
        if attempt + 1 < REGISTRY_ATTEMPTS:
            sleep(min(8.0, REGISTRY_RETRY_BASE_SECONDS * (2**attempt)))
    raise VerificationError(
        f"public manifest {reference} did not converge after {REGISTRY_ATTEMPTS} attempts: "
        f"{last_error}"
    )


def _fetch_public_blob(
    repository: str,
    digest: str,
    *,
    token: str,
    accept: str,
    sleep: Callable[[float], None] = time.sleep,
) -> bytes:
    expected_digest = _validate_digest(digest, "public blob digest")
    last_error: Exception | None = None
    for attempt in range(REGISTRY_ATTEMPTS):
        try:
            payload, _headers = _registry_get(
                repository,
                f"blobs/{expected_digest}",
                token=token,
                accept=accept,
            )
            computed = f"sha256:{hashlib.sha256(payload).hexdigest()}"
            if computed != expected_digest:
                raise VerificationError(
                    f"public blob bytes hash to {computed}, descriptor names {expected_digest}"
                )
            return payload
        except urllib.error.HTTPError as error:
            last_error = error
            if error.code not in {404, 429, 500, 502, 503, 504}:
                raise VerificationError(
                    f"anonymous GHCR blob request returned HTTP {error.code}; "
                    "the package may not be public"
                ) from error
        except VerificationError as error:
            last_error = error
        if attempt + 1 < REGISTRY_ATTEMPTS:
            sleep(min(8.0, REGISTRY_RETRY_BASE_SECONDS * (2**attempt)))
    raise VerificationError(
        f"public blob {expected_digest} did not converge after "
        f"{REGISTRY_ATTEMPTS} attempts: {last_error}"
    )


def _manifest_descriptor(
    manifest: dict[str, Any], name: str, *, expected_media_prefix: str
) -> dict[str, Any]:
    descriptor = manifest.get(name)
    if not isinstance(descriptor, dict):
        raise VerificationError(f"public manifest is missing {name} descriptor")
    digest = _validate_digest(descriptor.get("digest"), f"public manifest {name} digest")
    size = descriptor.get("size")
    if type(size) is not int or size <= 0:
        raise VerificationError(f"public manifest {name} size must be positive")
    media_type = descriptor.get("mediaType")
    if not isinstance(media_type, str) or not media_type.startswith(expected_media_prefix):
        raise VerificationError(f"public manifest {name} has invalid mediaType {media_type!r}")
    return {"digest": digest, "mediaType": media_type, "size": size}


def _verify_public_config(
    repository: str,
    manifest: dict[str, Any],
    *,
    token: str,
    expected_local_image_id: str,
    expected_source_sha: str,
    expected_source_url: str,
    sleep: Callable[[float], None] = time.sleep,
) -> dict[str, Any]:
    descriptor = _manifest_descriptor(
        manifest, "config", expected_media_prefix="application/vnd."
    )
    if descriptor["mediaType"] not in {
        "application/vnd.oci.image.config.v1+json",
        "application/vnd.docker.container.image.v1+json",
    }:
        raise VerificationError(
            f"public manifest config mediaType is unsupported: {descriptor['mediaType']}"
        )
    if descriptor["digest"] != expected_local_image_id:
        raise VerificationError(
            "public manifest config digest does not equal the tested local image ID: "
            f"{descriptor['digest']} != {expected_local_image_id}"
        )
    payload = _fetch_public_blob(
        repository,
        descriptor["digest"],
        token=token,
        accept="application/vnd.oci.image.config.v1+json, "
        "application/vnd.docker.container.image.v1+json",
        sleep=sleep,
    )
    if len(payload) != descriptor["size"]:
        raise VerificationError(
            f"public image config size mismatch: {len(payload)} != {descriptor['size']}"
        )
    config_document = _load_json_bytes(payload, "public image config")
    if not isinstance(config_document, dict):
        raise VerificationError("public image config must be an object")
    if (
        config_document.get("os") != "linux"
        or config_document.get("architecture") != "amd64"
    ):
        raise VerificationError("public image config must be linux/amd64")
    config = config_document.get("config")
    if not isinstance(config, dict):
        raise VerificationError("public image config is missing runtime config")
    labels = config.get("Labels")
    if not isinstance(labels, dict):
        raise VerificationError("public image config is missing labels")
    for key, expected in {
        EXPECTED_SOURCE_LABEL: expected_source_url,
        EXPECTED_REVISION_LABEL: expected_source_sha,
    }.items():
        if labels.get(key) != expected:
            raise VerificationError(
                f"public image config label {key} mismatch: {labels.get(key)!r}"
            )
    if config.get("User") not in {"sorotte", "10001", "10001:10001"}:
        raise VerificationError("public image config does not run as the non-root user")
    if config.get("Entrypoint") != EXPECTED_ENTRYPOINT:
        raise VerificationError("public image config entrypoint drifted from the tested contract")
    if config.get("Cmd") != EXPECTED_DEFAULT_COMMAND:
        raise VerificationError(
            "public image config default command drifted from the tested contract"
        )
    rootfs = config_document.get("rootfs")
    if not isinstance(rootfs, dict) or not isinstance(rootfs.get("diff_ids"), list):
        raise VerificationError("public image config is missing rootfs diff IDs")
    diff_ids = rootfs["diff_ids"]
    if not diff_ids:
        raise VerificationError("public image config rootfs diff IDs must not be empty")
    for index, digest in enumerate(diff_ids):
        _validate_digest(digest, f"public image config diff ID {index}")
    layers = manifest.get("layers")
    if not isinstance(layers, list) or len(layers) != len(diff_ids):
        raise VerificationError(
            "public manifest layer inventory does not match tested config rootfs inventory"
        )
    verified_layers = []
    allowed_layer_media_types = {
        "application/vnd.oci.image.layer.v1.tar",
        "application/vnd.oci.image.layer.v1.tar+gzip",
        "application/vnd.oci.image.layer.v1.tar+zstd",
        "application/vnd.docker.image.rootfs.diff.tar.gzip",
    }
    for index, layer in enumerate(layers):
        if not isinstance(layer, dict):
            raise VerificationError(f"public manifest layer {index} must be an object")
        if layer.get("mediaType") not in allowed_layer_media_types:
            raise VerificationError(
                f"public manifest layer {index} has unsupported mediaType "
                f"{layer.get('mediaType')!r}"
            )
        verified_layers.append(
            {
                "digest": _validate_digest(
                    layer.get("digest"), f"public manifest layer {index} digest"
                ),
                "size": layer.get("size"),
            }
        )
        if type(layer.get("size")) is not int or layer["size"] <= 0:
            raise VerificationError(f"public manifest layer {index} size must be positive")
    return {
        "configDigest": descriptor["digest"],
        "layers": verified_layers,
        "rootfsDiffIds": diff_ids,
    }


def _decode_json_stream(path: Path, description: str) -> list[Any]:
    _require_regular_file(path, description)
    try:
        text = path.read_bytes().decode("utf-8")
    except UnicodeDecodeError as error:
        raise VerificationError(f"{description} must be UTF-8 JSON") from error
    decoder = json.JSONDecoder(object_pairs_hook=_reject_duplicate_key)
    values: list[Any] = []
    offset = 0
    while offset < len(text):
        while offset < len(text) and text[offset].isspace():
            offset += 1
        if offset == len(text):
            break
        try:
            value, offset = decoder.raw_decode(text, offset)
        except json.JSONDecodeError as error:
            raise VerificationError(f"{description} contains invalid JSON") from error
        values.append(value)
    if not values:
        raise VerificationError(f"{description} must not be empty")
    return values


def verify_cosign_signature_output(
    path: Path,
    *,
    expected_image: str,
    expected_digest: str,
    expected_annotations: dict[str, str] | None = None,
) -> int:
    values = _decode_json_stream(path, "cosign signature verification output")
    records: list[Any] = []
    for value in values:
        records.extend(value if isinstance(value, list) else [value])
    matches = 0
    for record in records:
        if not isinstance(record, dict):
            continue
        critical = record.get("critical")
        if not isinstance(critical, dict):
            continue
        image = critical.get("image")
        identity = critical.get("identity")
        if not isinstance(image, dict) or not isinstance(identity, dict):
            continue
        optional = record.get("optional")
        annotations_match = expected_annotations is None or (
            isinstance(optional, dict)
            and all(optional.get(key) == value for key, value in expected_annotations.items())
        )
        if (
            image.get("docker-manifest-digest") == expected_digest
            and identity.get("docker-reference") == expected_image
            and annotations_match
        ):
            matches += 1
    if matches == 0:
        raise VerificationError("cosign signature output does not bind the expected image digest")
    return matches


def verify_cosign_attestation_output(
    path: Path,
    *,
    expected_digest: str,
    expected_image: str | None = None,
    expected_predicate_path: Path | None = None,
) -> int:
    values = _decode_json_stream(path, "cosign attestation verification output")
    envelopes: list[Any] = []
    for value in values:
        envelopes.extend(value if isinstance(value, list) else [value])
    expected_hex = expected_digest.removeprefix("sha256:")
    expected_predicate = (
        _load_json_file(expected_predicate_path, "expected SPDX predicate")
        if expected_predicate_path is not None
        else None
    )
    matches = 0
    for envelope in envelopes:
        if not isinstance(envelope, dict) or not isinstance(envelope.get("payload"), str):
            continue
        try:
            payload = base64.b64decode(envelope["payload"], validate=True)
        except (ValueError, binascii.Error):
            continue
        statement = _load_json_bytes(payload, "cosign attestation statement")
        if not isinstance(statement, dict):
            continue
        predicate_type = statement.get("predicateType")
        if predicate_type not in {
            "https://spdx.dev/Document",
            "https://spdx.dev/Document/",
        }:
            continue
        subjects = statement.get("subject")
        if not isinstance(subjects, list):
            continue
        bound = any(
            isinstance(subject, dict)
            and (expected_image is None or subject.get("name") == expected_image)
            and isinstance(subject.get("digest"), dict)
            and subject["digest"].get("sha256") == expected_hex
            for subject in subjects
        )
        predicate = statement.get("predicate")
        if (
            bound
            and isinstance(predicate, dict)
            and isinstance(predicate.get("spdxVersion"), str)
            and predicate["spdxVersion"].startswith("SPDX-2.")
            and (expected_predicate is None or predicate == expected_predicate)
        ):
            matches += 1
    if matches == 0:
        raise VerificationError(
            "cosign attestation output does not bind an SPDX predicate to the expected digest"
        )
    return matches


def verify_publication(
    *,
    publish_report_path: Path,
    sbom_path: Path,
    sbom_report_path: Path,
    signature_path: Path,
    attestation_path: Path,
    expected_workflow_identity: str,
    expected_workflow_sha: str,
    sleep: Callable[[float], None] = time.sleep,
) -> dict[str, Any]:
    publish = parse_publish_report(publish_report_path)
    sbom = parse_sbom_report(sbom_report_path)
    if sbom["localImageId"] != publish["localImageId"] or sbom["image"] not in {
        publish["image"],
        f"sorotte-server:test-{publish['sourceSha']}",
    }:
        raise VerificationError("SBOM and publication reports do not bind the same tested image")
    workflow_sha = _validate_source_sha(expected_workflow_sha, "workflow source SHA")
    if not expected_workflow_identity.startswith("https://github.com/") or "@" not in (
        expected_workflow_identity
    ):
        raise VerificationError("workflow identity must be an exact GitHub workflow URI")
    signature_count = verify_cosign_signature_output(
        signature_path,
        expected_image=publish["image"],
        expected_digest=publish["digest"],
        expected_annotations={
            "sourceSha": publish["sourceSha"],
            "workflowSourceSha": workflow_sha,
        },
    )
    attestation_count = verify_cosign_attestation_output(
        attestation_path,
        expected_digest=publish["digest"],
        expected_image=publish["image"],
        expected_predicate_path=sbom_path,
    )
    repository = _parse_ghcr_repository(publish["image"])
    token = _anonymous_ghcr_token(repository)
    references = [*publish["tags"], f"{publish['image']}@{publish['digest']}"]
    public_references: list[dict[str, Any]] = []
    canonical_manifest: bytes | None = None
    config_evidence: dict[str, Any] | None = None
    for reference in references:
        if reference.startswith(f"{publish['image']}:"):
            registry_reference = reference.removeprefix(f"{publish['image']}:")
        else:
            registry_reference = reference.removeprefix(f"{publish['image']}@")
        manifest, digest, payload = _fetch_public_manifest(
            repository, registry_reference, token=token, sleep=sleep
        )
        if digest != publish["digest"]:
            raise VerificationError(
                f"public reference {reference} resolved to {digest}, expected {publish['digest']}"
            )
        if canonical_manifest is None:
            canonical_manifest = payload
            config_evidence = _verify_public_config(
                repository,
                manifest,
                token=token,
                expected_local_image_id=publish["localImageId"],
                expected_source_sha=publish["sourceSha"],
                expected_source_url=publish["source"],
                sleep=sleep,
            )
        elif payload != canonical_manifest:
            raise VerificationError(
                f"public reference {reference} returned divergent manifest bytes"
            )
        public_references.append({"digest": digest, "reference": reference})
    if config_evidence is None:
        raise VerificationError("no public image config was verified")
    return {
        "attestations": attestation_count,
        "digest": publish["digest"],
        "image": publish["image"],
        "localImageId": publish["localImageId"],
        "publicConfig": config_evidence,
        "publicReferences": public_references,
        "sbomSha256": sbom["sbomSha256"],
        "schemaVersion": 1,
        "signatures": signature_count,
        "sourceSha": publish["sourceSha"],
        "status": "passed",
        "verificationPolicy": {
            "certificateGithubWorkflowSha": publish["sourceSha"],
            "certificateIdentity": expected_workflow_identity,
            "certificateIssuer": "https://token.actions.githubusercontent.com",
            "workflowSourceSha": workflow_sha,
        },
    }


def parse_runtime_report(path: Path) -> dict[str, Any]:
    report = _require_exact_keys(
        _load_json_file(path, "container runtime report"),
        {"image", "localImage", "schemaVersion", "scenarios", "status"},
        "container runtime report",
    )
    if report["schemaVersion"] != 1 or report["status"] != "passed":
        raise VerificationError("container runtime report is not a passed schema v1 report")
    _require_nonempty_string(report["image"], "container runtime image")
    local = _require_exact_keys(
        report["localImage"],
        {
            "architecture",
            "created",
            "entrypoint",
            "id",
            "os",
            "rootfsDiffIds",
            "source",
            "sourceSha",
            "user",
        },
        "container runtime localImage",
    )
    _validate_digest(local["id"], "container runtime local image ID")
    _validate_source_sha(local["sourceSha"])
    _validate_source_url(local["source"])
    if (
        local["architecture"] != "amd64"
        or local["os"] != "linux"
        or local["entrypoint"] != EXPECTED_ENTRYPOINT
    ):
        raise VerificationError("container runtime local image platform/entrypoint drifted")
    if not isinstance(local["rootfsDiffIds"], list) or not local["rootfsDiffIds"]:
        raise VerificationError("container runtime local RootFS inventory is empty")
    for index, diff_id in enumerate(local["rootfsDiffIds"]):
        _validate_digest(diff_id, f"container runtime local diff ID {index}")
    scenarios = report["scenarios"]
    if (
        not isinstance(scenarios, list)
        or len(scenarios) != 2
        or {
            item.get("scenario") for item in scenarios if isinstance(item, dict)
        }
        != {"plaintext-persistence", "tls-persistence"}
    ):
        raise VerificationError("container runtime report is missing required scenarios")
    for scenario in scenarios:
        required = {
            "clientSessionDrained",
            "containerId",
            "databases",
            "log",
            "persistence",
            "protocolHello",
            "restart",
            "scenario",
            "shutdown",
            "tls",
        }
        _require_exact_keys(scenario, required, "container runtime scenario")
        if (
            scenario["protocolHello"] is not True
            or scenario["clientSessionDrained"] is not True
        ):
            raise VerificationError(
                "container runtime protocol Hello/session drain was not proven"
            )
        if (
            not isinstance(scenario["containerId"], str)
            or CONTAINER_ID_RE.fullmatch(scenario["containerId"]) is None
        ):
            raise VerificationError("container runtime scenario has an invalid container ID")
        _require_nonempty_string(scenario["log"], "container runtime scenario log")
        databases = scenario["databases"]
        if not isinstance(databases, list) or len(databases) != 2:
            raise VerificationError("container runtime scenario must prove two SQLite databases")
        for index, database in enumerate(databases):
            _require_exact_keys(
                database,
                {"integrityCheck", "path", "sha256", "size"},
                f"container database {index}",
            )
            _validate_digest(database["sha256"], f"container database {index} digest")
            if (
                database["integrityCheck"] != "ok"
                or type(database["size"]) is not int
                or database["size"] <= 0
            ):
                raise VerificationError("container database size must be positive")
        shutdown = scenario["shutdown"]
        _require_exact_keys(
            shutdown, {"error", "exitCode", "oomKilled", "signal"}, "container shutdown"
        )
        if (
            shutdown["exitCode"] != 0
            or shutdown["error"] != ""
            or shutdown["oomKilled"] is not False
            or shutdown["signal"] != "SIGINT"
        ):
            raise VerificationError("container runtime scenario did not shut down cleanly")
        if scenario["scenario"] == "tls-persistence" and not isinstance(
            scenario["tls"], dict
        ):
            raise VerificationError("TLS container scenario is missing TLS evidence")
        if scenario["scenario"] == "tls-persistence":
            _require_exact_keys(
                scenario["tls"],
                {"cipher", "peerCertificateSha256", "version"},
                "container TLS evidence",
            )
            _validate_digest(
                scenario["tls"]["peerCertificateSha256"], "TLS peer certificate digest"
            )
            if scenario["restart"] is not None or scenario["persistence"] is not None:
                raise VerificationError(
                    "TLS Hello/drain scenario must not claim a duplicate restart proof"
                )
        elif scenario["tls"] is not None:
            raise VerificationError("plaintext container scenario must not claim TLS evidence")
        else:
            persistence = _require_exact_keys(
                scenario["persistence"],
                {
                    "accepted",
                    "rawAfterRestart",
                    "rawAfterWrite",
                    "restored",
                    "room",
                    "sameLoadedImage",
                    "sameStateDirectory",
                },
                "plaintext persistence evidence",
            )
            if (
                persistence["sameLoadedImage"] is not True
                or persistence["sameStateDirectory"] is not True
            ):
                raise VerificationError(
                    "plaintext persistence restart did not reuse image and state directory"
                )
            _require_nonempty_string(
                persistence["room"], "plaintext persisted room name"
            )
            accepted = _require_exact_keys(
                persistence["accepted"],
                {"playlist", "playlistIndex", "playstate"},
                "accepted plaintext persisted state",
            )
            if persistence["restored"] != accepted:
                raise VerificationError(
                    "plaintext restored state does not exactly equal accepted state"
                )
            playlist = accepted["playlist"]
            if (
                not isinstance(playlist, list)
                or not playlist
                or not all(isinstance(item, str) and item for item in playlist)
            ):
                raise VerificationError("plaintext persisted playlist is invalid")
            if (
                type(accepted["playlistIndex"]) is not int
                or not 0 <= accepted["playlistIndex"] < len(playlist)
            ):
                raise VerificationError("plaintext persisted playlist index is invalid")
            playstate = _require_exact_keys(
                accepted["playstate"],
                {"paused", "position"},
                "accepted plaintext playstate",
            )
            if playstate["paused"] is not True or (
                isinstance(playstate["position"], bool)
                or not isinstance(playstate["position"], (int, float))
                or not math.isfinite(playstate["position"])
            ):
                raise VerificationError("plaintext persisted playstate is invalid")
            expected_raw_payload = {
                "name": persistence["room"],
                "playlist": "\n".join(playlist),
                "playlistIndex": accepted["playlistIndex"],
                "playlistJson": json.dumps(playlist, separators=(",", ":")),
                "position": playstate["position"],
            }
            for phase in ("rawAfterWrite", "rawAfterRestart"):
                raw = _require_exact_keys(
                    persistence[phase],
                    {"integrityCheck", "row"},
                    f"plaintext persistence {phase}",
                )
                if raw["integrityCheck"] != "ok":
                    raise VerificationError(
                        f"plaintext persistence {phase} lacks SQLite integrity proof"
                    )
                row = _require_exact_keys(
                    raw["row"],
                    {
                        "createdAt",
                        "lastSavedUpdate",
                        "name",
                        "ownerBucket",
                        "persistenceVersion",
                        "playlist",
                        "playlistIndex",
                        "playlistJson",
                        "position",
                    },
                    f"plaintext persistence {phase} raw row",
                )
                for key, expected in expected_raw_payload.items():
                    if row[key] != expected:
                        raise VerificationError(
                            f"plaintext persistence {phase} raw {key} diverged"
                        )
                if (
                    not isinstance(row["ownerBucket"], str)
                    or re.fullmatch(r"quota:v1:[0-9a-f]{64}", row["ownerBucket"])
                    is None
                    or type(row["persistenceVersion"]) is not int
                    or row["persistenceVersion"] <= 0
                ):
                    raise VerificationError(
                        f"plaintext persistence {phase} raw metadata is invalid"
                    )
                for field in ("createdAt", "lastSavedUpdate"):
                    value = row[field]
                    if (
                        isinstance(value, bool)
                        or not isinstance(value, (int, float))
                        or not math.isfinite(value)
                        or value <= 0
                    ):
                        raise VerificationError(
                            f"plaintext persistence {phase} raw {field} is invalid"
                        )
            restart = _require_exact_keys(
                scenario["restart"],
                {
                    "clientSessionDrained",
                    "containerId",
                    "log",
                    "protocolHello",
                    "shutdown",
                },
                "plaintext persistence restart",
            )
            if (
                restart["clientSessionDrained"] is not True
                or restart["protocolHello"] is not True
                or not isinstance(restart["containerId"], str)
                or CONTAINER_ID_RE.fullmatch(restart["containerId"]) is None
            ):
                raise VerificationError(
                    "plaintext persistence restart protocol/drain proof is incomplete"
                )
            _require_nonempty_string(
                restart["log"], "plaintext persistence restart log"
            )
            restart_shutdown = _require_exact_keys(
                restart["shutdown"],
                {"error", "exitCode", "oomKilled", "signal"},
                "plaintext persistence restart shutdown",
            )
            if (
                restart_shutdown["exitCode"] != 0
                or restart_shutdown["error"] != ""
                or restart_shutdown["oomKilled"] is not False
                or restart_shutdown["signal"] != "SIGINT"
            ):
                raise VerificationError(
                    "plaintext persistence restart did not shut down cleanly"
                )
    return report


def parse_sbom_report(path: Path) -> dict[str, Any]:
    report = _require_exact_keys(
        _load_json_file(path, "container SBOM report"),
        {
            "bindingMode",
            "image",
            "localImageId",
            "packageCount",
            "rootfsDiffIds",
            "sbomSha256",
            "schemaVersion",
            "source",
            "sourceSha",
            "spdxVersion",
            "status",
        },
        "container SBOM report",
    )
    if report["schemaVersion"] != 1 or report["status"] != "passed":
        raise VerificationError("container SBOM report is not a passed schema v1 report")
    _require_nonempty_string(report["image"], "container SBOM image")
    if report["bindingMode"] != "pinned-syft-input-plus-daemon-reinspection":
        raise VerificationError("container SBOM report used an unknown identity binding")
    _validate_digest(report["localImageId"], "SBOM local image ID")
    _validate_digest(report["sbomSha256"], "SBOM file digest")
    _validate_source_url(report["source"])
    _validate_source_sha(report["sourceSha"])
    if not isinstance(report["rootfsDiffIds"], list) or not report["rootfsDiffIds"]:
        raise VerificationError("SBOM local RootFS inventory is empty")
    for index, diff_id in enumerate(report["rootfsDiffIds"]):
        _validate_digest(diff_id, f"SBOM local RootFS diff ID {index}")
    if type(report["packageCount"]) is not int or report["packageCount"] <= 0:
        raise VerificationError("SBOM package count must be positive")
    return report


def parse_publish_report(path: Path) -> dict[str, Any]:
    report = _require_exact_keys(
        _load_json_file(path, "container publish report"),
        {
            "digest",
            "image",
            "localImageId",
            "pushes",
            "schemaVersion",
            "source",
            "sourceSha",
            "status",
            "tags",
        },
        "container publish report",
    )
    if report["schemaVersion"] != 1 or report["status"] != "passed":
        raise VerificationError("container publish report is not a passed schema v1 report")
    _validate_image_name(report["image"])
    _validate_digest(report["digest"], "published manifest digest")
    _validate_digest(report["localImageId"], "published local image ID")
    _validate_source_sha(report["sourceSha"])
    _validate_source_url(report["source"])
    _validate_publication_scope(report["image"], report["source"])
    if not isinstance(report["tags"], list) or not report["tags"]:
        raise VerificationError("container publish report tags must not be empty")
    seen_tags: set[str] = set()
    for tag_reference in report["tags"]:
        if not isinstance(tag_reference, str) or not tag_reference.startswith(
            f"{report['image']}:"
        ):
            raise VerificationError("container publish report contains a foreign tag")
        tag = tag_reference.removeprefix(f"{report['image']}:")
        if TAG_RE.fullmatch(tag) is None or tag_reference in seen_tags:
            raise VerificationError("container publish report contains a noncanonical tag")
        seen_tags.add(tag_reference)
    if f"{report['image']}:sha-{report['sourceSha']}" not in seen_tags:
        raise VerificationError("container publish report is missing its exact source SHA tag")
    if not isinstance(report["pushes"], list) or len(report["pushes"]) != len(
        report["tags"]
    ):
        raise VerificationError("container publish report push inventory is incomplete")
    for index, push in enumerate(report["pushes"]):
        _require_exact_keys(push, {"digest", "tag"}, f"container push {index}")
        if (
            push["digest"] != report["digest"]
            or push["tag"] != report["tags"][index]
        ):
            raise VerificationError("container push inventory diverges from report identity")
    return report


def parse_publication_report(path: Path) -> dict[str, Any]:
    report = _require_exact_keys(
        _load_json_file(path, "public container report"),
        {
            "attestations",
            "digest",
            "image",
            "localImageId",
            "publicConfig",
            "publicReferences",
            "sbomSha256",
            "schemaVersion",
            "signatures",
            "sourceSha",
            "status",
            "verificationPolicy",
        },
        "public container report",
    )
    if report["schemaVersion"] != 1 or report["status"] != "passed":
        raise VerificationError("public container report is not a passed schema v1 report")
    _validate_image_name(report["image"])
    _validate_digest(report["digest"], "public manifest digest")
    _validate_digest(report["localImageId"], "public local image ID")
    _validate_digest(report["sbomSha256"], "public SBOM digest")
    _validate_source_sha(report["sourceSha"])
    if type(report["signatures"]) is not int or report["signatures"] <= 0:
        raise VerificationError("public container report must prove at least one signature")
    if type(report["attestations"]) is not int or report["attestations"] <= 0:
        raise VerificationError("public container report must prove at least one attestation")
    if not isinstance(report["publicReferences"], list) or not report["publicReferences"]:
        raise VerificationError("public container report references must not be empty")
    for index, reference in enumerate(report["publicReferences"]):
        _require_exact_keys(
            reference, {"digest", "reference"}, f"public container reference {index}"
        )
        if reference["digest"] != report["digest"]:
            raise VerificationError("public container reference digest diverges")
    config = _require_exact_keys(
        report["publicConfig"],
        {"configDigest", "layers", "rootfsDiffIds"},
        "public container config evidence",
    )
    if config["configDigest"] != report["localImageId"]:
        raise VerificationError("public config digest does not equal tested local image ID")
    if not isinstance(config["layers"], list) or not isinstance(
        config["rootfsDiffIds"], list
    ):
        raise VerificationError("public config layer inventories must be arrays")
    if len(config["layers"]) != len(config["rootfsDiffIds"]) or not config["layers"]:
        raise VerificationError("public config layer inventories must be non-empty and paired")
    for index, layer in enumerate(config["layers"]):
        _require_exact_keys(layer, {"digest", "size"}, f"public config layer {index}")
        _validate_digest(layer["digest"], f"public config layer {index} digest")
        if type(layer["size"]) is not int or layer["size"] <= 0:
            raise VerificationError("public config layer size must be positive")
    for index, diff_id in enumerate(config["rootfsDiffIds"]):
        _validate_digest(diff_id, f"public config rootfs diff ID {index}")
    policy = _require_exact_keys(
        report["verificationPolicy"],
        {
            "certificateGithubWorkflowSha",
            "certificateIdentity",
            "certificateIssuer",
            "workflowSourceSha",
        },
        "public container verification policy",
    )
    _validate_source_sha(
        policy["certificateGithubWorkflowSha"], "certificate GitHub workflow SHA"
    )
    _validate_source_sha(policy["workflowSourceSha"], "workflow source SHA")
    if policy["certificateIssuer"] != "https://token.actions.githubusercontent.com":
        raise VerificationError("public container verification used an unexpected OIDC issuer")
    if (
        not isinstance(policy["certificateIdentity"], str)
        or not policy["certificateIdentity"].startswith("https://github.com/")
        or "@" not in policy["certificateIdentity"]
    ):
        raise VerificationError("public container certificate identity is not exact")
    return report


def enforce_final_gate(
    *,
    runtime_report_path: Path,
    sbom_path: Path,
    sbom_report_path: Path,
    publish_report_path: Path,
    signature_path: Path,
    attestation_path: Path,
    publication_report_path: Path,
) -> dict[str, Any]:
    runtime = parse_runtime_report(runtime_report_path)
    sbom = parse_sbom_report(sbom_report_path)
    publish = parse_publish_report(publish_report_path)
    public = parse_publication_report(publication_report_path)
    _require_regular_file(sbom_path, "SPDX SBOM")
    if _sha256_file(sbom_path) != sbom["sbomSha256"]:
        raise VerificationError("SPDX SBOM bytes changed after verification")
    verify_cosign_signature_output(
        signature_path,
        expected_image=publish["image"],
        expected_digest=publish["digest"],
        expected_annotations={
            "sourceSha": publish["sourceSha"],
            "workflowSourceSha": public["verificationPolicy"]["workflowSourceSha"],
        },
    )
    verify_cosign_attestation_output(
        attestation_path,
        expected_digest=publish["digest"],
        expected_image=publish["image"],
        expected_predicate_path=sbom_path,
    )
    identities = {
        runtime["localImage"]["id"],
        sbom["localImageId"],
        publish["localImageId"],
        public["localImageId"],
    }
    digests = {publish["digest"], public["digest"]}
    source_shas = {
        runtime["localImage"]["sourceSha"],
        sbom["sourceSha"],
        publish["sourceSha"],
        public["sourceSha"],
    }
    sbom_digests = {sbom["sbomSha256"], public["sbomSha256"]}
    if len(identities) != 1:
        raise VerificationError("final gate found divergent local image/config identities")
    if len(digests) != 1:
        raise VerificationError("final gate found divergent registry manifest identities")
    if len(source_shas) != 1:
        raise VerificationError("final gate found divergent source revisions")
    if len(sbom_digests) != 1:
        raise VerificationError("final gate found divergent SBOM identities")
    if (
        runtime["localImage"]["source"] != publish["source"]
        or runtime["localImage"]["source"] != sbom["source"]
        or runtime["image"] != sbom["image"]
        or publish["image"] != public["image"]
        or public["verificationPolicy"]["certificateGithubWorkflowSha"]
        != publish["sourceSha"]
    ):
        raise VerificationError("final gate found divergent source or image identity")
    if (
        runtime["localImage"]["rootfsDiffIds"]
        != sbom["rootfsDiffIds"]
        or runtime["localImage"]["rootfsDiffIds"]
        != public["publicConfig"]["rootfsDiffIds"]
    ):
        raise VerificationError(
            "final gate found divergent runtime/SBOM/public RootFS diff ID inventories"
        )
    public_refs = {
        item.get("reference")
        for item in public["publicReferences"]
        if isinstance(item, dict)
    }
    expected_refs = {*publish["tags"], f"{publish['image']}@{publish['digest']}"}
    if public_refs != expected_refs:
        raise VerificationError("final gate found incomplete public reference comparison")
    return {
        "localImageId": identities.pop(),
        "registryManifestDigest": digests.pop(),
        "sbomSha256": sbom_digests.pop(),
        "schemaVersion": 1,
        "sourceSha": source_shas.pop(),
        "status": "passed",
    }


def _command_smoke(args: argparse.Namespace) -> dict[str, Any]:
    return smoke_loaded_image(
        image=args.image,
        expected_source_sha=args.expected_source_sha,
        expected_source_url=args.expected_source_url,
        repo_root=args.repo_root.resolve(),
        artifacts_root=args.artifacts_dir.resolve(),
    )


def _command_sbom(args: argparse.Namespace) -> dict[str, Any]:
    return verify_sbom(
        sbom_path=args.sbom.resolve(),
        runtime_report_path=args.runtime_report.resolve(),
    )


def _command_publish(args: argparse.Namespace) -> dict[str, Any]:
    return publish_tested_image(
        local_image=args.local_image,
        image_name=args.image,
        tags_path=args.tags_file.resolve(),
        expected_source_sha=args.expected_source_sha,
        expected_source_url=args.expected_source_url,
    )


def _command_public(args: argparse.Namespace) -> dict[str, Any]:
    return verify_publication(
        publish_report_path=args.publish_report.resolve(),
        sbom_path=args.sbom.resolve(),
        sbom_report_path=args.sbom_report.resolve(),
        signature_path=args.signature_verification.resolve(),
        attestation_path=args.attestation_verification.resolve(),
        expected_workflow_identity=args.expected_workflow_identity,
        expected_workflow_sha=args.expected_workflow_sha,
    )


def _command_final(args: argparse.Namespace) -> dict[str, Any]:
    return enforce_final_gate(
        runtime_report_path=args.runtime_report.resolve(),
        sbom_path=args.sbom.resolve(),
        sbom_report_path=args.sbom_report.resolve(),
        publish_report_path=args.publish_report.resolve(),
        signature_path=args.signature_verification.resolve(),
        attestation_path=args.attestation_verification.resolve(),
        publication_report_path=args.publication_report.resolve(),
    )


def _command_digest(args: argparse.Namespace) -> None:
    report = parse_publish_report(args.publish_report.resolve())
    print(report["digest"])


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    smoke = subparsers.add_parser("smoke", help="consume the loaded image through Docker")
    smoke.add_argument("--image", required=True)
    smoke.add_argument("--expected-source-sha", required=True)
    smoke.add_argument("--expected-source-url", required=True)
    smoke.add_argument("--repo-root", type=Path, default=Path.cwd())
    smoke.add_argument("--artifacts-dir", type=Path, required=True)
    smoke.add_argument("--report", type=Path, required=True)
    smoke.set_defaults(handler=_command_smoke)

    sbom = subparsers.add_parser("verify-sbom", help="verify the exact local-image SBOM")
    sbom.add_argument("--sbom", type=Path, required=True)
    sbom.add_argument("--runtime-report", type=Path, required=True)
    sbom.add_argument("--report", type=Path, required=True)
    sbom.set_defaults(handler=_command_sbom)

    publish = subparsers.add_parser(
        "publish", help="tag and push only the already-tested daemon image"
    )
    publish.add_argument("--local-image", required=True)
    publish.add_argument("--image", required=True)
    publish.add_argument("--tags-file", type=Path, required=True)
    publish.add_argument("--expected-source-sha", required=True)
    publish.add_argument("--expected-source-url", required=True)
    publish.add_argument("--report", type=Path, required=True)
    publish.set_defaults(handler=_command_publish)

    digest = subparsers.add_parser(
        "publication-digest", help="print a validated publish-report digest"
    )
    digest.add_argument("--publish-report", type=Path, required=True)
    digest.set_defaults(handler=_command_digest)

    public = subparsers.add_parser(
        "verify-publication",
        help="verify signatures and anonymous GHCR identity after publication",
    )
    public.add_argument("--publish-report", type=Path, required=True)
    public.add_argument("--sbom", type=Path, required=True)
    public.add_argument("--sbom-report", type=Path, required=True)
    public.add_argument("--signature-verification", type=Path, required=True)
    public.add_argument("--attestation-verification", type=Path, required=True)
    public.add_argument("--expected-workflow-identity", required=True)
    public.add_argument("--expected-workflow-sha", required=True)
    public.add_argument("--report", type=Path, required=True)
    public.set_defaults(handler=_command_public)

    final = subparsers.add_parser(
        "final-gate", help="fail unless every container publication phase completed"
    )
    final.add_argument("--runtime-report", type=Path, required=True)
    final.add_argument("--sbom", type=Path, required=True)
    final.add_argument("--sbom-report", type=Path, required=True)
    final.add_argument("--publish-report", type=Path, required=True)
    final.add_argument("--signature-verification", type=Path, required=True)
    final.add_argument("--attestation-verification", type=Path, required=True)
    final.add_argument("--publication-report", type=Path, required=True)
    final.add_argument("--report", type=Path, required=True)
    final.set_defaults(handler=_command_final)
    return parser


def main(argv: Iterable[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        result = args.handler(args)
        if result is not None:
            _write_json(args.report.resolve(), result)
    except VerificationError as error:
        report_path = getattr(args, "report", None)
        if isinstance(report_path, Path):
            failure_path = report_path.resolve().with_name(
                f"{report_path.stem}-failure.json"
            )
            _write_json(
                failure_path,
                {
                    "command": args.command,
                    "error": str(error),
                    "schemaVersion": 1,
                    "status": "failed",
                },
            )
        print(f"server container verification failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
