#!/usr/bin/env python3
"""Packaged, multi-process playback lifecycle verification.

This harness deliberately crosses every production process boundary:

* the exact ``sorotte-server`` binary owns canonical room state;
* three exact ``sorotte-cli`` binaries use the production network/stdin loops;
* each client owns a real managed mpv process;
* an independent raw-protocol observer records server authority without being
  used as the playback implementation under test; and
* a tiny mpv Lua observer records physical player state using generated media.

Exit code 125 means a declared prerequisite was unavailable.  That result is
never treated as a pass by the required CI lane.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid
import wave
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence


SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-playback-lifecycle-system"
TRACE_KIND = "sorotte-playback-lifecycle-causal-trace"
PLAYER_TRACE_KIND = "sorotte-real-mpv-observation"
MISSING_PREREQUISITE_EXIT = 125
DEFAULT_TIMEOUT_SECONDS = 75.0
DEFAULT_CLIENT_RUNTIME_SECONDS = 24.0
ROOM_VERSION = "1.7.5"
MINIMUM_MPV_VERSION = (0, 41, 0)
FAULT_SCHEDULE_ID = "follower-cut-miss-start-reconnect-v1"

ROLE_USERNAMES = {
    "observer": "lifecycle-observer",
    "controller": "lifecycle-controller",
    "follower": "lifecycle-follower",
    "late": "lifecycle-late",
}
USERNAME_ROLES = {value: key for key, value in ROLE_USERNAMES.items()}

SAFE_TRACE_FIELDS = {
    "schema_version",
    "kind",
    "sequence",
    "elapsed_ms",
    "correlation_id",
    "source",
    "role",
    "event",
    "source_sequence",
    "media_slot",
    "paused",
    "position_seconds",
    "duration_seconds",
    "paused_for_cache",
    "eof_reached",
    "reason",
    "playlist_index",
    "playlist_size",
    "set_by",
    "do_seek",
    "status_revision",
    "status_mode",
    "participants",
    "server_ignore_counter",
    "check_id",
    "detail",
}
SAFE_PLAYER_TRACE_FIELDS = {
    "schema_version",
    "kind",
    "sequence",
    "observed_at_ms",
    "role",
    "event",
    "media_slot",
    "paused",
    "position_seconds",
    "duration_seconds",
    "paused_for_cache",
    "eof_reached",
    "reason",
}

_SENSITIVE_KEY = re.compile(
    r"(?i)(?:token|password|secret|credential|authorization|cookie|uri|url|path)"
)
_URL_SECRET = re.compile(
    r"(?i)([?&](?:access_token|auth|key|password|secret|token|x-plex-token)=)[^&\s]+"
)
_INLINE_SECRET = re.compile(
    r"(?i)(\b(?:access_token|authorization|cookie|credential|password|secret|token|x-plex-token)\s*[:=]\s*)[^\s,;]+"
)
_WINDOWS_ABSOLUTE_PATH = re.compile(r"(?i)(?:[a-z]:[\\/]|\\\\)[^\s\"'<>]+")
_POSIX_ABSOLUTE_PATH = re.compile(r"(?<![A-Za-z0-9_.])/(?:[^/\s\"'<>]+/)*[^/\s\"'<>]+")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_mpv_version(version_line: str) -> tuple[int, int, int]:
    match = re.search(r"(?i)\bmpv\s+v?(\d+)\.(\d+)\.(\d+)", version_line)
    if match is None:
        raise ValueError("mpv version output did not contain a semantic version")
    return (int(match.group(1)), int(match.group(2)), int(match.group(3)))


def redact_sensitive_text(text: object, known_values: Iterable[object] = ()) -> str:
    """Return a bounded error string without URLs, credentials, or local paths."""

    redacted = str(text)
    replacements = sorted(
        {
            str(value)
            for value in known_values
            if value is not None and str(value).strip()
        },
        key=len,
        reverse=True,
    )
    for value in replacements:
        redacted = redacted.replace(value, "<redacted-path>")
    redacted = _URL_SECRET.sub(r"\1<redacted>", redacted)
    redacted = _INLINE_SECRET.sub(r"\1<redacted>", redacted)
    redacted = _WINDOWS_ABSOLUTE_PATH.sub("<redacted-path>", redacted)
    redacted = _POSIX_ABSOLUTE_PATH.sub("<redacted-path>", redacted)
    return redacted[:500]


def assert_privacy_safe_trace_record(record: Mapping[str, Any]) -> None:
    unexpected = set(record) - SAFE_TRACE_FIELDS
    if unexpected:
        raise ValueError(f"trace record contains non-whitelisted fields: {sorted(unexpected)}")
    for key, value in record.items():
        if _SENSITIVE_KEY.search(key) and key not in {"media_slot"}:
            raise ValueError(f"trace record contains a sensitive field name: {key}")
        values: Iterable[Any]
        if isinstance(value, (list, tuple, set)):
            values = value
        else:
            values = (value,)
        for item in values:
            if isinstance(item, str):
                if _URL_SECRET.search(item):
                    raise ValueError(f"trace record contains a credential-bearing value in {key}")
                if _INLINE_SECRET.search(item):
                    raise ValueError(f"trace record contains a credential-bearing value in {key}")
                if _WINDOWS_ABSOLUTE_PATH.search(item) or _POSIX_ABSOLUTE_PATH.search(item):
                    raise ValueError(f"trace record contains a local path in {key}")


def assert_privacy_safe_player_trace_record(record: Mapping[str, Any]) -> None:
    unexpected = set(record) - SAFE_PLAYER_TRACE_FIELDS
    if unexpected:
        raise ValueError(
            f"player trace record contains non-whitelisted fields: {sorted(unexpected)}"
        )
    if record.get("kind") != PLAYER_TRACE_KIND:
        raise ValueError("player trace record has the wrong kind")
    if record.get("role") not in {"controller", "follower", "late"}:
        raise ValueError("player trace record has an unknown role")
    if record.get("media_slot") not in {None, "media-1", "media-2", "other"}:
        raise ValueError("player trace record has an unknown media slot")
    for key, value in record.items():
        if _SENSITIVE_KEY.search(key) and key != "media_slot":
            raise ValueError(f"player trace record contains a sensitive field name: {key}")
        if isinstance(value, str):
            if _URL_SECRET.search(value) or _INLINE_SECRET.search(value):
                raise ValueError(
                    f"player trace record contains a credential-bearing value in {key}"
                )
            if _WINDOWS_ABSOLUTE_PATH.search(value) or _POSIX_ABSOLUTE_PATH.search(value):
                raise ValueError(f"player trace record contains a local path in {key}")


def assert_privacy_safe_tree(value: Any, *, location: str = "report") -> None:
    if isinstance(value, Mapping):
        for key, child in value.items():
            if not isinstance(key, str):
                raise ValueError(f"{location} contains a non-string object key")
            if _SENSITIVE_KEY.search(key):
                raise ValueError(f"{location} contains a sensitive field name: {key}")
            assert_privacy_safe_tree(child, location=f"{location}.{key}")
        return
    if isinstance(value, (list, tuple)):
        for index, child in enumerate(value):
            assert_privacy_safe_tree(child, location=f"{location}[{index}]")
        return
    if isinstance(value, str):
        if _URL_SECRET.search(value) or _INLINE_SECRET.search(value):
            raise ValueError(f"{location} contains a credential-bearing value")
        if _WINDOWS_ABSOLUTE_PATH.search(value) or _POSIX_ABSOLUTE_PATH.search(value):
            raise ValueError(f"{location} contains a local path")


def atomic_write_json(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def stage_privacy_safe_evidence(artifact_dir: Path, output_dir: Path) -> dict[str, Any]:
    """Validate and copy only the closed, publishable evidence surface."""

    artifact_dir = artifact_dir.resolve()
    output_dir = output_dir.resolve()
    if output_dir.exists() and any(output_dir.iterdir()):
        raise ValueError("safe evidence output directory must be absent or empty")
    report_path = artifact_dir / "report.json"
    if not report_path.is_file():
        raise ValueError("system report is missing")
    report = json.loads(report_path.read_text(encoding="utf-8"))
    if not isinstance(report, dict) or report.get("kind") != REPORT_KIND:
        raise ValueError("system report has the wrong shape or kind")
    if report.get("result") not in {"passed", "failed", "skipped"}:
        raise ValueError("system report has an unknown result")
    assert_privacy_safe_tree(report)

    selected: list[Path] = [report_path]
    trace_path = artifact_dir / "causal-trace.jsonl"
    if not trace_path.is_file():
        raise ValueError("causal trace is missing")
    for record in read_jsonl(trace_path):
        assert_privacy_safe_trace_record(record)
    selected.append(trace_path)

    artifacts = report.get("artifacts")
    declared_player_traces = artifacts.get("player_traces", []) if isinstance(artifacts, dict) else []
    if not isinstance(declared_player_traces, list):
        raise ValueError("system report player trace inventory is invalid")
    allowed_player_names = {
        "player-controller.jsonl",
        "player-follower.jsonl",
        "player-late.jsonl",
    }
    for name in declared_player_traces:
        if not isinstance(name, str) or name not in allowed_player_names:
            raise ValueError("system report declares an unexpected player trace")
        path = artifact_dir / name
        if not path.is_file():
            raise ValueError(f"declared player trace is missing: {name}")
        for record in read_jsonl(path):
            assert_privacy_safe_player_trace_record(record)
        selected.append(path)

    output_dir.mkdir(parents=True, exist_ok=True)
    files: list[dict[str, Any]] = []
    for source in selected:
        destination = output_dir / source.name
        shutil.copy2(source, destination)
        files.append(
            {
                "name": source.name,
                "sha256": sha256_file(destination),
                "size_bytes": destination.stat().st_size,
            }
        )
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "kind": "sorotte-playback-lifecycle-safe-evidence",
        "result": report["result"],
        "candidate_sha": report.get("candidate_sha"),
        "files": sorted(files, key=lambda item: item["name"]),
    }
    atomic_write_json(output_dir / "evidence-manifest.json", manifest)
    return manifest


def generate_pcm_wav(path: Path, duration_seconds: float, sample_rate: int = 48_000) -> dict[str, Any]:
    if not math.isfinite(duration_seconds) or duration_seconds <= 0.0:
        raise ValueError("duration_seconds must be finite and positive")
    frame_count = int(round(duration_seconds * sample_rate))
    path.parent.mkdir(parents=True, exist_ok=True)
    silence_chunk = b"\0\0" * min(sample_rate, frame_count)
    remaining = frame_count
    with wave.open(str(path), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(sample_rate)
        while remaining:
            frames = min(remaining, sample_rate)
            output.writeframesraw(silence_chunk[: frames * 2])
            remaining -= frames
        output.writeframes(b"")
    return {
        "sha256": sha256_file(path),
        "duration_seconds": frame_count / sample_rate,
        "sample_rate_hz": sample_rate,
        "channels": 1,
        "sample_width_bytes": 2,
    }


def _lua_quote(value: str) -> str:
    # JSON string syntax is valid for the path/name subset used by Lua here.
    return json.dumps(value, ensure_ascii=False)


def render_mpv_observer_lua(
    *, role: str, trace_path: Path, first_media_name: str, second_media_name: str
) -> str:
    """Build a path-redacting mpv observer.

    The generated script necessarily knows where to write, but emitted records
    contain only stable media slots and coarse playback properties.
    """

    return f'''local utils = require "mp.utils"
local trace_path = {_lua_quote(str(trace_path))}
local role = {_lua_quote(role)}
local first_media_name = {_lua_quote(first_media_name)}
local second_media_name = {_lua_quote(second_media_name)}
local sequence = 0

local function media_slot()
    local value = mp.get_property("path")
    if value == nil then
        return nil
    end
    if string.find(value, first_media_name, 1, true) ~= nil then
        return "media-1"
    end
    if string.find(value, second_media_name, 1, true) ~= nil then
        return "media-2"
    end
    return "other"
end

local function finite_number(value)
    if type(value) ~= "number" or value ~= value or value == math.huge or value == -math.huge then
        return nil
    end
    return value
end

local function emit(event_name, reason)
    sequence = sequence + 1
    local record = {{
        schema_version = {SCHEMA_VERSION},
        kind = {_lua_quote(PLAYER_TRACE_KIND)},
        sequence = sequence,
        observed_at_ms = math.floor(mp.get_time() * 1000),
        role = role,
        event = event_name,
        media_slot = media_slot(),
        paused = mp.get_property_native("pause"),
        position_seconds = finite_number(mp.get_property_native("time-pos")),
        duration_seconds = finite_number(mp.get_property_native("duration")),
        paused_for_cache = mp.get_property_native("paused-for-cache"),
        eof_reached = mp.get_property_native("eof-reached"),
        reason = reason,
    }}
    local output = io.open(trace_path, "a")
    if output ~= nil then
        output:write(utils.format_json(record), "\\n")
        output:flush()
        output:close()
    end
end

mp.register_event("file-loaded", function() emit("file-loaded", nil) end)
mp.register_event("end-file", function(event) emit("end-file", event.reason) end)
mp.register_event("shutdown", function() emit("shutdown", nil) end)
mp.observe_property("pause", "bool", function() emit("pause-changed", nil) end)
mp.observe_property("time-pos", "number", function() emit("position-changed", nil) end)
mp.observe_property("paused-for-cache", "bool", function() emit("cache-pause-changed", nil) end)
mp.observe_property("eof-reached", "bool", function() emit("eof-changed", nil) end)
mp.observe_property("path", "string", function() emit("media-changed", nil) end)
'''


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.splitlines()
    records: list[dict[str, Any]] = []
    for index, line in enumerate(lines):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            is_incomplete_tail = index == len(lines) - 1 and not text.endswith(("\n", "\r"))
            if is_incomplete_tail:
                continue
            raise
        if not isinstance(value, dict):
            raise ValueError(f"JSONL record {index + 1} is not an object")
        records.append(value)
    return records


class TraceLedger:
    def __init__(self, path: Path, correlation_id: str) -> None:
        self.path = path
        self.correlation_id = correlation_id
        self.started = time.monotonic()
        self._lock = threading.Lock()
        self._sequence = 0
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("", encoding="utf-8")

    def emit(self, *, source: str, role: str, event: str, **fields: Any) -> dict[str, Any]:
        with self._lock:
            self._sequence += 1
            record: dict[str, Any] = {
                "schema_version": SCHEMA_VERSION,
                "kind": TRACE_KIND,
                "sequence": self._sequence,
                "elapsed_ms": int((time.monotonic() - self.started) * 1000),
                "correlation_id": self.correlation_id,
                "source": source,
                "role": role,
                "event": event,
            }
            record.update({key: value for key, value in fields.items() if value is not None})
            assert_privacy_safe_trace_record(record)
            with self.path.open("a", encoding="utf-8", newline="\n") as output:
                output.write(json.dumps(record, sort_keys=True, ensure_ascii=False) + "\n")
            return record


def _safe_number(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    number = float(value)
    return number if math.isfinite(number) else None


def _role(value: Any, role_by_username: Mapping[str, str]) -> str | None:
    if not isinstance(value, str):
        return None
    return role_by_username.get(value, "other")


def project_protocol_message(
    message: Mapping[str, Any], role_by_username: Mapping[str, str] = USERNAME_ROLES
) -> tuple[list[dict[str, Any]], dict[str, Any] | None]:
    """Project one raw protocol message into safe observations and obligations."""

    events: list[dict[str, Any]] = []
    response_state: dict[str, Any] = {}
    hello = message.get("Hello")
    if isinstance(hello, dict):
        events.append({"event": "server-hello"})

    set_payload = message.get("Set")
    if isinstance(set_payload, dict):
        playlist_change = set_payload.get("playlistChange")
        if isinstance(playlist_change, dict) and isinstance(playlist_change.get("files"), list):
            events.append(
                {
                    "event": "playlist-change",
                    "playlist_size": len(playlist_change["files"]),
                    "set_by": _role(playlist_change.get("user"), role_by_username),
                }
            )
        playlist_index = set_payload.get("playlistIndex")
        if isinstance(playlist_index, dict):
            value = playlist_index.get("index")
            if isinstance(value, int) and not isinstance(value, bool):
                events.append(
                    {
                        "event": "playlist-index",
                        "playlist_index": value,
                        "set_by": _role(playlist_index.get("user"), role_by_username),
                    }
                )

    state = message.get("State")
    if isinstance(state, dict):
        playstate = state.get("playstate")
        if isinstance(playstate, dict):
            events.append(
                {
                    "event": "playstate",
                    "paused": playstate.get("paused")
                    if isinstance(playstate.get("paused"), bool)
                    else None,
                    "position_seconds": _safe_number(playstate.get("position")),
                    "do_seek": playstate.get("doSeek")
                    if isinstance(playstate.get("doSeek"), bool)
                    else None,
                    "set_by": _role(playstate.get("setBy"), role_by_username),
                }
            )

        participant_status = state.get("sorotteParticipantStatusV1")
        if isinstance(participant_status, dict):
            snapshot = participant_status.get("snapshot")
            if isinstance(snapshot, dict):
                participants = snapshot.get("participants")
                safe_views: dict[str, dict[str, Any]] = {}
                if isinstance(participants, dict):
                    for username, view in participants.items():
                        role = role_by_username.get(str(username), "other")
                        if isinstance(view, dict):
                            safe_views[role] = {
                                key: view.get(key)
                                for key in ("availability", "playerConnection", "phase")
                                if isinstance(view.get(key), str)
                            }
                        else:
                            safe_views[role] = {}
                revision = snapshot.get("revision")
                events.append(
                    {
                        "event": "participant-status-snapshot",
                        "status_revision": revision
                        if isinstance(revision, int) and not isinstance(revision, bool)
                        else None,
                        "status_mode": snapshot.get("mode", "full")
                        if isinstance(snapshot.get("mode", "full"), str)
                        else "unknown",
                        "participants": sorted(safe_views),
                        "participant_views": safe_views,
                    }
                )

        ping = state.get("ping")
        if isinstance(ping, dict):
            challenge = _safe_number(ping.get("latencyCalculation"))
            if challenge is not None:
                response_state["ping"] = {
                    "latencyCalculation": challenge,
                    "clientLatencyCalculation": time.monotonic(),
                    "clientRtt": 0.0,
                }

        ignoring = state.get("ignoringOnTheFly")
        if isinstance(ignoring, dict):
            server_counter = ignoring.get("server")
            if isinstance(server_counter, int) and not isinstance(server_counter, bool):
                response_state["ignoringOnTheFly"] = {"server": server_counter}
                events.append(
                    {
                        "event": "server-ignore-observed",
                        "server_ignore_counter": server_counter,
                    }
                )

    return events, ({"State": response_state} if response_state else None)


@dataclass
class ObserverEvent:
    sequence: int
    fields: dict[str, Any]


class ProtocolObserver:
    def __init__(self, *, host: str, port: int, room: str, ledger: TraceLedger) -> None:
        self.host = host
        self.port = port
        self.room = room
        self.ledger = ledger
        self.socket: socket.socket | None = None
        self._reader: threading.Thread | None = None
        self._send_lock = threading.Lock()
        self._event_lock = threading.Lock()
        self._events: list[ObserverEvent] = []
        self._sequence = 0
        self.error: str | None = None

    def connect(self) -> None:
        self.socket = socket.create_connection((self.host, self.port), timeout=5.0)
        self.socket.settimeout(None)
        self._reader = threading.Thread(target=self._read_loop, name="protocol-observer", daemon=True)
        self._reader.start()
        self.send(
            {
                "Hello": {
                    "username": ROLE_USERNAMES["observer"],
                    "room": {"name": self.room},
                    "version": ROOM_VERSION,
                    "features": {"sorotteParticipantStatusV1": True},
                }
            }
        )

    def send(self, message: Mapping[str, Any]) -> None:
        data = (json.dumps(message, separators=(",", ":")) + "\r\n").encode("utf-8")
        with self._send_lock:
            if self.socket is None:
                raise RuntimeError("protocol observer is not connected")
            self.socket.sendall(data)

    def _record(self, fields: dict[str, Any]) -> None:
        internal_fields = dict(fields)
        trace_fields = {key: value for key, value in fields.items() if key != "participant_views"}
        self.ledger.emit(source="server-protocol", role="server", **trace_fields)
        with self._event_lock:
            self._sequence += 1
            self._events.append(ObserverEvent(self._sequence, internal_fields))

    def _read_loop(self) -> None:
        assert self.socket is not None
        try:
            with self.socket.makefile("r", encoding="utf-8", errors="replace", newline="") as reader:
                for line in reader:
                    if not line.strip():
                        continue
                    value = json.loads(line)
                    if not isinstance(value, dict):
                        raise ValueError("server protocol line was not a JSON object")
                    events, response = project_protocol_message(value)
                    for event in events:
                        self._record(event)
                    if response is not None:
                        self.send(response)
        except (OSError, ValueError, json.JSONDecodeError) as error:
            if self.socket is not None:
                self.error = redact_sensitive_text(error)

    def cursor(self) -> int:
        with self._event_lock:
            return self._sequence

    def events_after(self, sequence: int = 0) -> list[ObserverEvent]:
        with self._event_lock:
            return [event for event in self._events if event.sequence > sequence]

    def close(self) -> None:
        current = self.socket
        self.socket = None
        if current is not None:
            try:
                current.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            current.close()
        if self._reader is not None:
            self._reader.join(timeout=2.0)


class ProtocolFaultProxy:
    """Owned loopback TCP proxy with a deterministic cut-and-hold barrier."""

    _FRAGMENT_SIZES = (1, 2, 5, 13, 29)

    def __init__(
        self,
        *,
        upstream_host: str,
        upstream_port: int,
        role: str,
        ledger: TraceLedger,
    ) -> None:
        self.upstream_host = upstream_host
        self.upstream_port = upstream_port
        self.role = role
        self.ledger = ledger
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(4)
        self.listener.settimeout(0.1)
        self.port = int(self.listener.getsockname()[1])
        self._stop = threading.Event()
        self._upstream_allowed = threading.Event()
        self._upstream_allowed.set()
        self._lock = threading.Lock()
        self._active_client: socket.socket | None = None
        self._active_upstream: socket.socket | None = None
        self._accepted_count = 0
        self._upstream_connection_count = 0
        self._fragment_count = 0
        self._forwarded_bytes = 0
        self.error: str | None = None
        self.thread = threading.Thread(
            target=self._run,
            name=f"{role}-protocol-fault-proxy",
            daemon=True,
        )
        self.thread.start()
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=role,
            event="proxy-listening",
            detail="ephemeral IPv4 loopback listener",
        )

    @staticmethod
    def _close_socket(value: socket.socket | None) -> None:
        if value is None:
            return
        try:
            value.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        try:
            value.close()
        except OSError:
            pass

    @property
    def accepted_count(self) -> int:
        with self._lock:
            return self._accepted_count

    @property
    def upstream_connection_count(self) -> int:
        with self._lock:
            return self._upstream_connection_count

    @property
    def fragment_count(self) -> int:
        with self._lock:
            return self._fragment_count

    @property
    def forwarded_bytes(self) -> int:
        with self._lock:
            return self._forwarded_bytes

    def _fragmented_send(self, destination: socket.socket, data: bytes) -> None:
        offset = 0
        fragment_index = 0
        while offset < len(data) and not self._stop.is_set():
            size = self._FRAGMENT_SIZES[fragment_index % len(self._FRAGMENT_SIZES)]
            fragment_index += 1
            destination.sendall(data[offset : offset + size])
            sent = min(size, len(data) - offset)
            with self._lock:
                self._fragment_count += 1
                self._forwarded_bytes += sent
            offset += size

    def _pump(
        self,
        source: socket.socket,
        destination: socket.socket,
        finished: threading.Event,
    ) -> None:
        try:
            while not self._stop.is_set() and not finished.is_set():
                data = source.recv(16 * 1024)
                if not data:
                    return
                self._fragmented_send(destination, data)
        except OSError:
            return
        finally:
            finished.set()
            self._close_socket(source)
            self._close_socket(destination)

    def _run_connection(self, client: socket.socket) -> None:
        while not self._stop.is_set() and not self._upstream_allowed.wait(0.05):
            pass
        if self._stop.is_set():
            self._close_socket(client)
            return
        upstream = socket.create_connection(
            (self.upstream_host, self.upstream_port), timeout=3.0
        )
        upstream.settimeout(None)
        client.settimeout(None)
        with self._lock:
            self._active_client = client
            self._active_upstream = upstream
            self._upstream_connection_count += 1
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event="proxy-upstream-connected",
            detail="fragmenting bidirectional relay",
        )
        finished = threading.Event()
        pumps = [
            threading.Thread(
                target=self._pump,
                args=(client, upstream, finished),
                name=f"{self.role}-proxy-client-upstream",
                daemon=True,
            ),
            threading.Thread(
                target=self._pump,
                args=(upstream, client, finished),
                name=f"{self.role}-proxy-upstream-client",
                daemon=True,
            ),
        ]
        for pump in pumps:
            pump.start()
        finished.wait()
        self._close_socket(client)
        self._close_socket(upstream)
        for pump in pumps:
            pump.join(timeout=1.0)
        with self._lock:
            if self._active_client is client:
                self._active_client = None
            if self._active_upstream is upstream:
                self._active_upstream = None

    def _run(self) -> None:
        try:
            while not self._stop.is_set():
                try:
                    client, _address = self.listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    if self._stop.is_set():
                        return
                    raise
                with self._lock:
                    self._accepted_count += 1
                self.ledger.emit(
                    source="protocol-fault-proxy",
                    role=self.role,
                    event="proxy-client-accepted",
                    detail="production client transport",
                )
                self._run_connection(client)
        except OSError as error:
            if not self._stop.is_set():
                self.error = redact_sensitive_text(error)

    def cut_and_hold(self) -> None:
        self._upstream_allowed.clear()
        with self._lock:
            client = self._active_client
            upstream = self._active_upstream
        self._close_socket(client)
        self._close_socket(upstream)
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event="proxy-cut-and-hold",
            detail="active transport closed; replacement held",
        )

    def resume(self) -> None:
        self._upstream_allowed.set()
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event="proxy-resumed",
            detail="replacement transport released",
        )

    def close(self) -> None:
        self._stop.set()
        self._upstream_allowed.set()
        self._close_socket(self.listener)
        with self._lock:
            client = self._active_client
            upstream = self._active_upstream
        self._close_socket(client)
        self._close_socket(upstream)
        self.thread.join(timeout=2.0)


class ProcessCapture:
    def __init__(
        self,
        *,
        role: str,
        args: Sequence[str],
        cwd: Path,
        env: Mapping[str, str] | None,
        artifact_dir: Path,
        stdin: bool,
        stderr_callback: Callable[[str], None] | None = None,
    ) -> None:
        self.role = role
        popen_kwargs: dict[str, Any] = {
            "cwd": str(cwd),
            "env": dict(env) if env is not None else None,
            "stdin": subprocess.PIPE if stdin else subprocess.DEVNULL,
            "stdout": subprocess.PIPE,
            "stderr": subprocess.PIPE,
            "text": True,
            "encoding": "utf-8",
            "errors": "replace",
            "bufsize": 1,
        }
        if os.name == "nt":
            popen_kwargs["creationflags"] = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
        else:
            popen_kwargs["start_new_session"] = True
        self.process = subprocess.Popen(list(args), **popen_kwargs)
        self._threads: list[threading.Thread] = []
        for stream_name, stream, callback in (
            ("stdout", self.process.stdout, None),
            ("stderr", self.process.stderr, stderr_callback),
        ):
            assert stream is not None
            log_path = artifact_dir / f"{role}.{stream_name}.log"
            thread = threading.Thread(
                target=self._drain,
                args=(stream, log_path, callback),
                name=f"{role}-{stream_name}",
                daemon=True,
            )
            thread.start()
            self._threads.append(thread)

    @staticmethod
    def _drain(
        stream: Any, path: Path, callback: Callable[[str], None] | None
    ) -> None:
        with path.open("w", encoding="utf-8", newline="\n") as output:
            for line in stream:
                output.write(line)
                output.flush()
                if callback is not None:
                    callback(line)

    def write_line(self, line: str) -> None:
        if self.process.stdin is None:
            raise RuntimeError(f"{self.role} has no stdin")
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()

    def join_capture(self) -> None:
        for thread in self._threads:
            thread.join(timeout=2.0)


class PlayerTraceMonitor:
    def __init__(self, *, role: str, path: Path, ledger: TraceLedger) -> None:
        self.role = role
        self.path = path
        self.ledger = ledger
        self._stop = threading.Event()
        self._count = 0
        self.error: str | None = None
        self.thread = threading.Thread(target=self._run, name=f"{role}-trace", daemon=True)
        self.thread.start()

    def _run(self) -> None:
        try:
            while not self._stop.is_set():
                records = read_jsonl(self.path)
                for record in records[self._count :]:
                    event = record.get("event")
                    if not isinstance(event, str):
                        event = "player-event"
                    reason = record.get("reason")
                    if not isinstance(reason, str) or reason not in {
                        "eof",
                        "stop",
                        "quit",
                        "error",
                        "redirect",
                    }:
                        reason = None
                    self.ledger.emit(
                        source="real-mpv",
                        role=self.role,
                        event=event,
                        source_sequence=record.get("sequence")
                        if isinstance(record.get("sequence"), int)
                        else None,
                        media_slot=record.get("media_slot")
                        if record.get("media_slot") in {"media-1", "media-2", "other"}
                        else None,
                        paused=record.get("paused")
                        if isinstance(record.get("paused"), bool)
                        else None,
                        position_seconds=_safe_number(record.get("position_seconds")),
                        duration_seconds=_safe_number(record.get("duration_seconds")),
                        paused_for_cache=record.get("paused_for_cache")
                        if isinstance(record.get("paused_for_cache"), bool)
                        else None,
                        eof_reached=record.get("eof_reached")
                        if isinstance(record.get("eof_reached"), bool)
                        else None,
                        reason=reason,
                    )
                self._count = len(records)
                self._stop.wait(0.05)
        except (OSError, ValueError, json.JSONDecodeError) as error:
            self.error = redact_sensitive_text(error, (self.path,))

    def close(self) -> None:
        self._stop.set()
        self.thread.join(timeout=2.0)


@dataclass
class ClientProcess:
    role: str
    process: ProcessCapture
    player_trace: Path
    ipc_path: str
    script_path: Path
    monitor: PlayerTraceMonitor


class HarnessFailure(RuntimeError):
    def __init__(self, stage: str, message: str) -> None:
        super().__init__(message)
        self.stage = stage


class MissingPrerequisite(HarnessFailure):
    pass


@dataclass
class PlaybackLifecycleHarness:
    server_path: Path
    client_path: Path
    mpv_path: Path
    artifact_dir: Path
    candidate_sha: str | None
    timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS
    client_runtime_seconds: float = DEFAULT_CLIENT_RUNTIME_SECONDS
    correlation_id: str = field(default_factory=lambda: uuid.uuid4().hex)

    def __post_init__(self) -> None:
        self.repo_root = Path(__file__).resolve().parents[1]
        self.artifact_dir = self.artifact_dir.resolve()
        self.report_path = self.artifact_dir / "report.json"
        self.trace_path = self.artifact_dir / "causal-trace.jsonl"
        self.started_at = utc_now()
        self.deadline = time.monotonic() + self.timeout_seconds
        self.stage = "initialization"
        self.ledger: TraceLedger | None = None
        self.server: ProcessCapture | None = None
        self.server_port: int | None = None
        self.observer: ProtocolObserver | None = None
        self.clients: dict[str, ClientProcess] = {}
        self.proxies: dict[str, ProtocolFaultProxy] = {}
        self._started_process_log_roles: list[str] = []
        self.checks: list[dict[str, Any]] = []
        self.prerequisites: dict[str, Any] = {}
        self.fixtures: dict[str, Any] = {}
        self.room = f"lifecycle-{self.correlation_id[:12]}"
        self._known_sensitive_values: list[object] = [
            self.server_path,
            self.client_path,
            self.mpv_path,
            self.artifact_dir,
        ]

    def _emit(self, *, source: str = "harness", role: str = "orchestrator", event: str, **fields: Any) -> None:
        if self.ledger is not None:
            self.ledger.emit(source=source, role=role, event=event, **fields)

    def _remaining(self) -> float:
        return max(0.0, self.deadline - time.monotonic())

    def _wait(
        self,
        description: str,
        predicate: Callable[[], Any],
        *,
        timeout: float = 8.0,
        allow_client_exit: bool = False,
    ) -> Any:
        deadline = min(self.deadline, time.monotonic() + timeout)
        while time.monotonic() < deadline:
            if self.server is not None and self.server.process.poll() is not None:
                raise HarnessFailure(self.stage, "the server process exited before verification completed")
            if not allow_client_exit:
                for client in self.clients.values():
                    code = client.process.process.poll()
                    if code is not None:
                        raise HarnessFailure(
                            self.stage,
                            f"the {client.role} client exited early with code {code}",
                        )
            if self.observer is not None and self.observer.error is not None:
                raise HarnessFailure(self.stage, "the independent protocol observer failed")
            for client in self.clients.values():
                if client.monitor.error is not None:
                    raise HarnessFailure(self.stage, f"the {client.role} mpv trace became invalid")
            for proxy in self.proxies.values():
                if proxy.error is not None:
                    raise HarnessFailure(self.stage, f"the {proxy.role} protocol proxy failed")
            value = predicate()
            if value:
                return value
            time.sleep(0.05)
        raise HarnessFailure(self.stage, f"timed out waiting for {description}")

    def _pass(self, check_id: str, detail: str) -> None:
        self.checks.append({"id": check_id, "status": "passed", "detail": detail})
        self._emit(event="check-passed", check_id=check_id, detail=detail)

    def _not_applicable(self, check_id: str, detail: str) -> None:
        self.checks.append({"id": check_id, "status": "not-applicable", "detail": detail})
        self._emit(event="check-not-applicable", check_id=check_id, detail=detail)

    def _resolve_candidate_sha(self) -> str:
        if self.candidate_sha is not None:
            value = self.candidate_sha.strip().lower()
        else:
            result = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=self.repo_root,
                capture_output=True,
                text=True,
                timeout=5.0,
                check=False,
            )
            value = result.stdout.strip().lower() if result.returncode == 0 else ""
        if not re.fullmatch(r"[0-9a-f]{40}", value):
            raise MissingPrerequisite("candidate-attestation", "a full 40-character candidate SHA is required")
        return value

    @staticmethod
    def _resolve_executable(requested: Path) -> Path | None:
        if requested.is_file():
            return requested.resolve()
        located = shutil.which(str(requested))
        return Path(located).resolve() if located else None

    def _version(self, path: Path) -> str:
        result = subprocess.run(
            [str(path), "--version"],
            cwd=self.repo_root,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=8.0,
            check=False,
        )
        if result.returncode != 0:
            raise MissingPrerequisite("binary-preflight", "a declared executable failed its version probe")
        lines = (result.stdout + "\n" + result.stderr).splitlines()
        return redact_sensitive_text(next((line for line in lines if line.strip()), "unknown"))

    def preflight(self) -> None:
        self.stage = "binary-preflight"
        candidate_sha = self._resolve_candidate_sha()
        self.candidate_sha = candidate_sha
        self.prerequisites["candidate_sha"] = candidate_sha
        resolved: dict[str, Path] = {}
        for label, requested in (
            ("server", self.server_path),
            ("client", self.client_path),
            ("mpv", self.mpv_path),
        ):
            executable = self._resolve_executable(requested)
            if executable is None:
                raise MissingPrerequisite(self.stage, f"the declared {label} executable is unavailable")
            resolved[label] = executable
            self._known_sensitive_values.append(executable)
            version = self._version(executable)
            if label == "mpv":
                try:
                    parsed_mpv_version = parse_mpv_version(version)
                except ValueError as error:
                    raise MissingPrerequisite(
                        self.stage, "the declared mpv version could not be verified"
                    ) from error
                if parsed_mpv_version < MINIMUM_MPV_VERSION:
                    raise MissingPrerequisite(
                        self.stage,
                        "the declared mpv executable is older than the supported minimum",
                    )
            self.prerequisites[label] = {
                "sha256": sha256_file(executable),
                "version": version,
            }
            if label == "mpv":
                self.prerequisites[label]["minimum_version"] = ".".join(
                    str(component) for component in MINIMUM_MPV_VERSION
                )
        self.server_path = resolved["server"]
        self.client_path = resolved["client"]
        self.mpv_path = resolved["mpv"]
        self._pass("prerequisites-attested", "candidate SHA and executable digests were captured")

    def _create_fixtures(self) -> tuple[Path, Path]:
        self.stage = "fixture-generation"
        fixture_dir = self.artifact_dir / "generated-media"
        first = fixture_dir / f"lifecycle-media-one-{self.correlation_id[:8]}.wav"
        second = fixture_dir / f"lifecycle-media-two-{self.correlation_id[:8]}.wav"
        self.fixtures = {
            "media-1": generate_pcm_wav(first, 10.0),
            "media-2": generate_pcm_wav(second, 14.0),
        }
        self._known_sensitive_values.extend((first, second))
        self._emit(event="fixtures-generated", detail="two deterministic PCM WAV fixtures")
        return first.resolve(), second.resolve()

    def _start_server(self) -> None:
        self.stage = "server-startup"
        port_ready = threading.Event()

        def inspect_stderr(line: str) -> None:
            match = re.search(r"sorotte-server listening on 127\.0\.0\.1:(\d+)", line)
            if match:
                self.server_port = int(match.group(1))
                port_ready.set()

        self.server = ProcessCapture(
            role="server",
            args=[
                str(self.server_path),
                "--port",
                "0",
                "--ipv4-only",
                "--interface-ipv4",
                "127.0.0.1",
                "--disable-ready",
            ],
            cwd=self.repo_root,
            env={
                key: value
                for key, value in os.environ.items()
                if not key.upper().startswith("SOROTTE_")
            },
            artifact_dir=self.artifact_dir,
            stdin=False,
            stderr_callback=inspect_stderr,
        )
        self._started_process_log_roles.append("server")
        self._wait("the server's ephemeral IPv4 listener", lambda: port_ready.is_set(), timeout=8.0)
        assert self.server_port is not None
        self._emit(event="server-listening", detail="ephemeral IPv4 loopback listener")

    def _observer_event(
        self,
        after: int,
        predicate: Callable[[dict[str, Any]], bool],
        timeout: float = 8.0,
        *,
        allow_client_exit: bool = False,
    ) -> ObserverEvent:
        assert self.observer is not None

        def match() -> ObserverEvent | None:
            return next(
                (event for event in self.observer.events_after(after) if predicate(event.fields)),
                None,
            )

        return self._wait(
            "canonical server evidence",
            match,
            timeout=timeout,
            allow_client_exit=allow_client_exit,
        )

    def _connect_observer_and_seed(self, first: Path, second: Path) -> None:
        self.stage = "canonical-seed"
        assert self.server_port is not None
        self.observer = ProtocolObserver(
            host="127.0.0.1",
            port=self.server_port,
            room=self.room,
            ledger=self.ledger,
        )
        self.observer.connect()
        self._observer_event(0, lambda event: event.get("event") == "server-hello")
        cursor = self.observer.cursor()
        self.observer.send(
            {"Set": {"playlistChange": {"files": [str(first), str(second)]}}}
        )
        self.observer.send({"Set": {"playlistIndex": {"index": 0}}})
        self._observer_event(
            cursor,
            lambda event: event.get("event") == "playlist-change"
            and event.get("playlist_size") == 2,
        )
        self._observer_event(
            cursor,
            lambda event: event.get("event") == "playlist-index"
            and event.get("playlist_index") == 0,
        )
        self._pass("canonical-playlist-seeded", "server committed the generated two-item playlist")

    def _ipc_path(self, role: str) -> str:
        suffix = self.correlation_id[:8]
        if os.name == "nt":
            return rf"\\.\pipe\sorotte-lifecycle-{suffix}-{role}"
        return str(Path(tempfile.gettempdir()) / f"sorotte-{suffix}-{role}.sock")

    def _client_environment(
        self, *, role: str, media: Path, ipc_path: str, server_port: int
    ) -> dict[str, str]:
        environment = {
            key: value
            for key, value in os.environ.items()
            if not key.upper().startswith("SOROTTE_")
        }
        config_root = self.artifact_dir / "client-config" / self.correlation_id / role
        config_root.mkdir(parents=True, exist_ok=True)
        environment.update(
            {
                "SOROTTE_CLIENT_HOST": "127.0.0.1",
                "SOROTTE_CLIENT_PORT": str(server_port),
                "SOROTTE_CLIENT_USERNAME": ROLE_USERNAMES[role],
                "SOROTTE_CLIENT_ROOM": self.room,
                "SOROTTE_CLIENT_VERSION": ROOM_VERSION,
                "SOROTTE_CLIENT_MAX_RETRIES": "20",
                "SOROTTE_CLIENT_MAX_CONNECTED_RUNTIME_SECONDS": str(self.client_runtime_seconds),
                "SOROTTE_CLIENT_READINESS_SUPPORTED": "0",
                "SOROTTE_CLIENT_READY_AT_START": "0",
                "SOROTTE_CLIENT_CAN_CONTROL": "1",
                "SOROTTE_CLIENT_SHARED_PLAYLIST_ENABLED": "1",
                "SOROTTE_CLIENT_PAUSE_ON_LEAVE": "0",
                "SOROTTE_CLIENT_LOOP_AT_END_OF_PLAYLIST": "0",
                "SOROTTE_CLIENT_LOOP_SINGLE_FILES": "0",
                "SOROTTE_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS": "0",
                "SOROTTE_CLIENT_UNPAUSE_ACTION": "always",
                "SOROTTE_CLIENT_FILENAME_PRIVACY_MODE": "raw",
                "SOROTTE_CLIENT_FILESIZE_PRIVACY_MODE": "raw",
                "SOROTTE_CLIENT_TLS_POLICY": "plaintext",
                "SOROTTE_CLIENT_STDIN": "1",
                "SOROTTE_CLIENT_CONFIG_ROOT": str(config_root),
                "SOROTTE_CLIENT_MPV_MANAGED_LAUNCH": "1",
                "SOROTTE_CLIENT_MPV_MANAGED_BIN": str(self.mpv_path),
                "SOROTTE_CLIENT_MPV_MANAGED_MEDIA": str(media),
                "SOROTTE_CLIENT_MPV_MANAGED_IPC_PATH": ipc_path,
                "SOROTTE_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS": "10000",
                "SOROTTE_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS": "25",
                "SOROTTE_CLIENT_LOG_PLAYER_TELEMETRY": "1",
            }
        )
        return environment

    def _start_client(self, role: str, first: Path, second: Path) -> ClientProcess:
        self.stage = f"{role}-startup"
        assert self.server_port is not None
        client_server_port = self.server_port
        if role == "follower":
            proxy = ProtocolFaultProxy(
                upstream_host="127.0.0.1",
                upstream_port=self.server_port,
                role=role,
                ledger=self.ledger,
            )
            self.proxies[role] = proxy
            client_server_port = proxy.port
        trace_path = self.artifact_dir / f"player-{role}.jsonl"
        script_path = self.artifact_dir / f"player-{role}-observer.lua"
        trace_path.write_text("", encoding="utf-8")
        script_path.write_text(
            render_mpv_observer_lua(
                role=role,
                trace_path=trace_path,
                first_media_name=first.name,
                second_media_name=second.name,
            ),
            encoding="utf-8",
        )
        ipc_path = self._ipc_path(role)
        ipc_cleanup = Path(ipc_path) if os.name != "nt" else None
        if ipc_cleanup is not None:
            try:
                ipc_cleanup.unlink()
            except FileNotFoundError:
                pass
        process = ProcessCapture(
            role=f"client-{role}",
            args=[
                str(self.client_path),
                "--no-gui",
                "--no-store",
                "--",
                f"--script={script_path}",
                "--no-config",
                "--ao=null",
                "--vo=null",
                "--audio-display=no",
                "--keep-open=no",
            ],
            cwd=self.repo_root,
            env=self._client_environment(
                role=role,
                media=first,
                ipc_path=ipc_path,
                server_port=client_server_port,
            ),
            artifact_dir=self.artifact_dir,
            stdin=True,
        )
        self._started_process_log_roles.append(f"client-{role}")
        monitor = PlayerTraceMonitor(role=role, path=trace_path, ledger=self.ledger)
        client = ClientProcess(role, process, trace_path, ipc_path, script_path, monitor)
        self.clients[role] = client
        self._emit(event="client-started", role=role, detail="production CLI with managed real mpv")
        return client

    def _player_cursor(self, role: str) -> int:
        return len(read_jsonl(self.clients[role].player_trace))

    def _player_record(
        self,
        role: str,
        after: int,
        predicate: Callable[[dict[str, Any]], bool],
        timeout: float = 8.0,
    ) -> dict[str, Any]:
        def match() -> dict[str, Any] | None:
            records = read_jsonl(self.clients[role].player_trace)
            return next((record for record in records[after:] if predicate(record)), None)

        return self._wait(f"{role} physical mpv evidence", match, timeout=timeout)

    def _wait_player_state(
        self,
        roles: Iterable[str],
        cursors: Mapping[str, int],
        *,
        media_slot: str,
        paused: bool | None = None,
        position: float | None = None,
        tolerance: float = 1.25,
        timeout: float = 8.0,
    ) -> None:
        for role in roles:
            self._player_record(
                role,
                cursors.get(role, 0),
                lambda record, paused=paused, position=position: (
                    record.get("media_slot") == media_slot
                    and (paused is None or record.get("paused") is paused)
                    and (
                        position is None
                        or (
                            _safe_number(record.get("position_seconds")) is not None
                            and abs(float(record["position_seconds"]) - position) <= tolerance
                        )
                    )
                ),
                timeout=timeout,
            )

    def _command(self, role: str, command: str) -> None:
        self.clients[role].process.write_line(command)
        safe_command = "seek" if command.startswith(("seek ", "s ")) else command
        self._emit(event="local-command-issued", role=role, detail=safe_command)

    def _canonical_playstate(
        self,
        cursor: int,
        *,
        paused: bool,
        set_by: str,
        position: float | None = None,
        require_seek: bool | None = None,
        timeout: float = 8.0,
    ) -> ObserverEvent:
        return self._observer_event(
            cursor,
            lambda event: (
                event.get("event") == "playstate"
                and event.get("paused") is paused
                and event.get("set_by") == set_by
                and (
                    position is None
                    or (
                        _safe_number(event.get("position_seconds")) is not None
                        and abs(float(event["position_seconds"]) - position) <= 0.75
                    )
                )
                and (require_seek is None or event.get("do_seek") is require_seek)
            ),
            timeout=timeout,
        )

    def _verify_initial_players(self, first: Path, second: Path) -> None:
        self.stage = "initial-two-client-convergence"
        self._start_client("controller", first, second)
        self._start_client("follower", first, second)
        for role in ("controller", "follower"):
            self._player_record(
                role,
                0,
                lambda record: record.get("event") == "file-loaded"
                and record.get("media_slot") == "media-1",
                timeout=10.0,
            )
        self._wait_player_state(
            ("controller", "follower"),
            {"controller": 0, "follower": 0},
            media_slot="media-1",
            paused=True,
            timeout=10.0,
        )
        self._pass(
            "initial-two-clients-loaded-paused",
            "two packaged clients loaded the selected item and honored canonical pause",
        )
        proxy = self.proxies.get("follower")
        if proxy is None:
            raise HarnessFailure(
                self.stage,
                "the follower protocol proxy was not created",
            )
        self._wait(
            "the follower protocol path to traverse deterministic fragmentation",
            lambda: proxy.fragment_count >= 2 and proxy.forwarded_bytes > 0,
            timeout=5.0,
        )
        self._pass(
            "follower-protocol-fragmentation-active",
            "the follower converged through the deterministic fragmenting protocol proxy",
        )

    def _verify_play_pause_seek(self) -> None:
        assert self.observer is not None
        self.stage = "play-authority"
        observer_cursor = self.observer.cursor()
        cursors = {role: self._player_cursor(role) for role in ("controller", "follower")}
        self._command("controller", "play")
        self._canonical_playstate(observer_cursor, paused=False, set_by="controller")
        self._wait_player_state(
            ("controller", "follower"), cursors, media_slot="media-1", paused=False
        )
        self._pass("play-committed-and-applied", "local play crossed server authority and reached both mpv processes")

        # Let the real clocks advance enough to distinguish an applied play
        # from a transient property echo.
        progress_cursors = {role: self._player_cursor(role) for role in ("controller", "follower")}
        self._wait_player_state(
            ("controller", "follower"),
            progress_cursors,
            media_slot="media-1",
            paused=False,
            position=0.6,
            tolerance=0.25,
            timeout=4.0,
        )

        self.stage = "pause-authority"
        observer_cursor = self.observer.cursor()
        cursors = {role: self._player_cursor(role) for role in ("controller", "follower")}
        self._command("controller", "pause")
        self._canonical_playstate(observer_cursor, paused=True, set_by="controller")
        self._wait_player_state(
            ("controller", "follower"), cursors, media_slot="media-1", paused=True
        )
        self._pass("pause-committed-and-applied", "local pause crossed server authority and reached both mpv processes")

        self.stage = "seek-authority"
        observer_cursor = self.observer.cursor()
        cursors = {role: self._player_cursor(role) for role in ("controller", "follower")}
        self._command("controller", "seek 7.0")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=7.0,
            require_seek=True,
        )
        self._wait_player_state(
            ("controller", "follower"),
            cursors,
            media_slot="media-1",
            paused=True,
            position=7.0,
        )
        self._pass("seek-committed-and-applied", "canonical seek reached both real player timelines while paused")

    def _verify_late_join_and_status(self, first: Path, second: Path) -> None:
        assert self.observer is not None
        self.stage = "late-join-catch-up"
        status_cursor = self.observer.cursor()
        self._start_client("late", first, second)
        self._player_record(
            "late",
            0,
            lambda record: record.get("event") == "file-loaded"
            and record.get("media_slot") == "media-1",
            timeout=10.0,
        )
        self._wait_player_state(
            ("late",),
            {"late": 0},
            media_slot="media-1",
            paused=True,
            position=7.0,
            tolerance=1.5,
            timeout=10.0,
        )
        self._pass(
            "late-joiner-caught-up",
            "a client with no prior deltas acquired canonical media, pause, and seek state",
        )

        self.stage = "participant-status-cadence"
        required = {"controller", "follower", "late"}

        def complete_status(event: dict[str, Any]) -> bool:
            if event.get("event") != "participant-status-snapshot":
                return False
            participants = set(event.get("participants", []))
            views = event.get("participant_views", {})
            return required <= participants and all(
                isinstance(views.get(role), dict)
                and views[role].get("playerConnection") == "connected"
                and views[role].get("availability") in {"fresh", "delayed"}
                for role in required
            )

        first_status = self._observer_event(status_cursor, complete_status, timeout=10.0)
        first_revision = first_status.fields.get("status_revision")
        if not isinstance(first_revision, int):
            raise HarnessFailure(self.stage, "participant status snapshot omitted its revision")
        second_status = self._observer_event(
            first_status.sequence,
            lambda event: complete_status(event)
            and isinstance(event.get("status_revision"), int)
            and event["status_revision"] > first_revision,
            timeout=8.0,
        )
        self._pass(
            "participant-status-snapshot-and-cadence",
            f"all real players appeared in advancing status revisions through {second_status.fields['status_revision']}",
        )

    def _verify_partitioned_follower_catches_up_to_missed_start(self) -> None:
        assert self.observer is not None
        proxy = self.proxies.get("follower")
        if proxy is None:
            raise HarnessFailure("partition-reconnect", "the follower fault proxy is missing")
        roles = ("controller", "follower", "late")

        self.stage = "partition-reconnect-preparation"
        observer_cursor = self.observer.cursor()
        cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "seek 2.0")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=2.0,
            require_seek=True,
        )
        self._wait_player_state(
            roles,
            cursors,
            media_slot="media-1",
            paused=True,
            position=2.0,
        )

        self.stage = "partition-follower"
        status_cursor = self.observer.cursor()
        follower_trace_cursor = self._player_cursor("follower")
        accepted_before_cut = proxy.accepted_count
        proxy.cut_and_hold()
        self._wait(
            "the production follower to reconnect into the held proxy",
            lambda: proxy.accepted_count > accepted_before_cut,
            timeout=5.0,
        )
        self._observer_event(
            status_cursor,
            lambda event: event.get("event") == "participant-status-snapshot"
            and "follower" not in set(event.get("participants", [])),
            timeout=6.0,
        )
        self._pass(
            "partition-withdraws-follower-status",
            "a deterministic transport cut removed the absent packaged follower from status authority",
        )

        self.stage = "start-while-follower-partitioned"
        play_cursor = self.observer.cursor()
        online_cursors = {
            role: self._player_cursor(role) for role in ("controller", "late")
        }
        self._command("controller", "play")
        play_commit = self._canonical_playstate(
            play_cursor, paused=False, set_by="controller"
        )
        self._wait_player_state(
            ("controller", "late"),
            online_cursors,
            media_slot="media-1",
            paused=False,
        )
        for role in ("controller", "late"):
            progress_cursor = self._player_cursor(role)
            self._player_record(
                role,
                progress_cursor,
                lambda record: record.get("media_slot") == "media-1"
                and record.get("paused") is False
                and _safe_number(record.get("position_seconds")) is not None
                and float(record["position_seconds"]) >= 2.6,
                timeout=4.0,
            )

        self.stage = "partitioned-follower-reconnect-catch-up"
        status_cursor = self.observer.cursor()
        upstream_before_resume = proxy.upstream_connection_count
        proxy.resume()
        self._wait(
            "the held follower replacement transport to reach the server",
            lambda: proxy.upstream_connection_count > upstream_before_resume,
            timeout=5.0,
        )

        def follower_connected_status(event: dict[str, Any]) -> bool:
            if event.get("event") != "participant-status-snapshot":
                return False
            views = event.get("participant_views", {})
            follower = views.get("follower") if isinstance(views, dict) else None
            return (
                "follower" in set(event.get("participants", []))
                and isinstance(follower, dict)
                and follower.get("playerConnection") == "connected"
            )

        self._observer_event(status_cursor, follower_connected_status, timeout=10.0)
        self._player_record(
            "follower",
            follower_trace_cursor,
            lambda record: record.get("media_slot") == "media-1"
            and record.get("paused") is False
            and _safe_number(record.get("position_seconds")) is not None
            and float(record["position_seconds"]) >= 2.5,
            timeout=10.0,
        )
        rogue_pause = next(
            (
                event
                for event in self.observer.events_after(play_commit.sequence)
                if event.fields.get("event") == "playstate"
                and event.fields.get("paused") is True
                and event.fields.get("set_by") == "follower"
            ),
            None,
        )
        if rogue_pause is not None:
            raise HarnessFailure(
                self.stage,
                "the reconnecting follower overwrote canonical playback with stale local pause",
            )
        self._pass(
            "partitioned-follower-caught-up",
            "the held follower missed Play, rejoined from authority, and resumed its existing real mpv without overwriting the room",
        )

        self.stage = "post-reconnect-stabilization"
        observer_cursor = self.observer.cursor()
        cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "pause")
        self._canonical_playstate(observer_cursor, paused=True, set_by="controller")
        self._wait_player_state(roles, cursors, media_slot="media-1", paused=True)

        observer_cursor = self.observer.cursor()
        cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "seek 7.0")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=7.0,
            require_seek=True,
        )
        self._wait_player_state(
            roles,
            cursors,
            media_slot="media-1",
            paused=True,
            position=7.0,
        )
        self._pass(
            "post-reconnect-room-stable",
            "all three players accepted fresh pause and seek authority after follower recovery",
        )

    def _verify_resume_eof_and_playlist(self) -> None:
        assert self.observer is not None
        roles = ("controller", "follower", "late")
        self.stage = "resume-authority"
        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        eof_cursors = dict(player_cursors)
        self._command("controller", "play")
        self._canonical_playstate(observer_cursor, paused=False, set_by="controller")
        self._wait_player_state(roles, player_cursors, media_slot="media-1", paused=False)
        self._pass("resume-committed-and-applied", "all three real players resumed from the canonical seek point")

        self.stage = "natural-eof-playlist-advance"
        advance = self._observer_event(
            observer_cursor,
            lambda event: event.get("event") == "playlist-index"
            and event.get("playlist_index") == 1,
            timeout=10.0,
        )
        eof_observed = False
        for role in roles:
            records = self._wait(
                "at least one natural real-mpv EOF",
                lambda role=role: read_jsonl(self.clients[role].player_trace),
                timeout=1.0,
            )
            if any(
                record.get("event") == "end-file"
                and record.get("reason") == "eof"
                and record.get("media_slot") == "media-1"
                for record in records[eof_cursors[role] :]
            ):
                eof_observed = True
        if not eof_observed:
            # One participant may advance first and stop the others; wait on
            # the union rather than requiring every player to reach EOF.
            def any_eof() -> bool:
                return any(
                    any(
                        record.get("event") == "end-file"
                        and record.get("reason") == "eof"
                        and record.get("media_slot") == "media-1"
                        for record in read_jsonl(self.clients[role].player_trace)[eof_cursors[role] :]
                    )
                    for role in roles
                )

            self._wait("a natural real-mpv EOF", any_eof, timeout=4.0)

        for role in roles:
            self._player_record(
                role,
                eof_cursors[role],
                lambda record: record.get("event") == "file-loaded"
                and record.get("media_slot") == "media-2",
                timeout=10.0,
            )

        stabilization_end = min(self.deadline, time.monotonic() + 1.0)
        while time.monotonic() < stabilization_end:
            time.sleep(0.05)
        index_events = [
            event
            for event in self.observer.events_after(observer_cursor)
            if event.fields.get("event") == "playlist-index"
        ]
        committed_ones = [
            event for event in index_events if event.fields.get("playlist_index") == 1
        ]
        invalid_advances = [
            event
            for event in index_events
            if isinstance(event.fields.get("playlist_index"), int)
            and event.fields["playlist_index"] > 1
        ]
        if len(committed_ones) != 1 or invalid_advances:
            raise HarnessFailure(
                self.stage,
                "natural EOF did not produce exactly one bounded canonical playlist advance",
            )
        if advance.sequence != committed_ones[0].sequence:
            raise HarnessFailure(self.stage, "playlist evidence changed while it was being verified")
        self._pass(
            "natural-eof-advanced-once",
            "real mpv EOF produced exactly one canonical transition from item zero to item one",
        )
        self._pass(
            "next-item-loaded-everywhere",
            "every managed real mpv loaded the server-selected second item",
        )

    def _verify_clean_shutdown_and_withdrawal(self) -> None:
        assert self.observer is not None
        self.stage = "client-bounded-shutdown"

        def all_clients_exited() -> bool:
            return all(client.process.process.poll() is not None for client in self.clients.values())

        self._wait(
            "all packaged clients to reach their bounded normal exit",
            all_clients_exited,
            timeout=max(3.0, min(self.client_runtime_seconds + 8.0, self._remaining())),
            allow_client_exit=True,
        )
        exit_codes = {
            role: client.process.process.returncode for role, client in self.clients.items()
        }
        if any(code != 0 for code in exit_codes.values()):
            raise HarnessFailure(self.stage, f"packaged clients did not all exit cleanly: {exit_codes}")
        for client in self.clients.values():
            client.process.join_capture()
        self._pass("clients-exited-cleanly", "all production CLI loops reached code zero at their bounded runtime")

        self.stage = "participant-status-withdrawal"
        withdrawal_cursor = self.observer.cursor()
        departed = {"controller", "follower", "late"}
        self._observer_event(
            withdrawal_cursor,
            lambda event: event.get("event") == "participant-status-snapshot"
            and not (departed & set(event.get("participants", []))),
            timeout=8.0,
            allow_client_exit=True,
        )
        self._pass("participant-status-withdrawal", "departed player rows were removed from the live room snapshot")

        if os.name == "nt":
            self._not_applicable(
                "managed-mpv-ipc-cleaned",
                "Windows named-pipe disappearance is not filesystem-observable",
            )
        else:
            self._wait(
                "managed mpv IPC sockets to be removed",
                lambda: all(not Path(client.ipc_path).exists() for client in self.clients.values()),
                timeout=5.0,
                allow_client_exit=True,
            )
            self._pass("managed-mpv-ipc-cleaned", "managed process guards removed every Unix IPC socket")

        self.stage = "server-bounded-shutdown"
        self.observer.close()
        self.observer = None
        assert self.server is not None
        if self.server.process.poll() is None:
            if os.name == "nt":
                self.server.process.send_signal(signal.CTRL_BREAK_EVENT)
            else:
                os.killpg(self.server.process.pid, signal.SIGINT)
        try:
            server_code = self.server.process.wait(timeout=min(10.0, max(1.0, self._remaining())))
        except subprocess.TimeoutExpired as error:
            raise HarnessFailure(self.stage, "server did not finish its explicit shutdown barrier") from error
        self.server.join_capture()
        if server_code != 0:
            raise HarnessFailure(self.stage, f"server exited with code {server_code}")
        port = self.server_port
        self.server = None
        if port is not None:
            def listener_closed() -> bool:
                probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                probe.settimeout(0.15)
                try:
                    return probe.connect_ex(("127.0.0.1", port)) != 0
                finally:
                    probe.close()

            self._wait("the server listener to close", listener_closed, timeout=3.0, allow_client_exit=True)
        self._pass("server-drained-cleanly", "the packaged server completed its signal-driven session and actor drain")

    def _base_report(self, result: str) -> dict[str, Any]:
        safe_candidate_sha = (
            self.candidate_sha.lower()
            if isinstance(self.candidate_sha, str)
            and re.fullmatch(r"[0-9a-fA-F]{40}", self.candidate_sha)
            else None
        )
        potential_process_logs = [
            f"{role}.{stream}.log"
            for role in self._started_process_log_roles
            for stream in ("stdout", "stderr")
        ]
        return {
            "schema_version": SCHEMA_VERSION,
            "kind": REPORT_KIND,
            "result": result,
            "capability": "actual-server-multi-client-real-mpv",
            "candidate_sha": safe_candidate_sha,
            "correlation_id": self.correlation_id,
            "started_at_utc": self.started_at,
            "finished_at_utc": utc_now(),
            "platform": {
                "os": sys.platform,
                "architecture": platform.machine().lower(),
            },
            "prerequisites": self.prerequisites,
            "fixtures": self.fixtures,
            "fault_schedule": {
                "id": FAULT_SCHEDULE_ID,
                "fragment_sizes": list(ProtocolFaultProxy._FRAGMENT_SIZES),
                "steps": [
                    "converge-through-fragmenting-proxy",
                    "seek-paused-baseline",
                    "cut-and-hold-follower",
                    "start-while-follower-absent",
                    "release-replacement-transport",
                    "verify-authoritative-catch-up",
                ],
            },
            "checks": self.checks,
            "artifacts": {
                "causal_trace": self.trace_path.name,
                "player_traces": [f"player-{role}.jsonl" for role in sorted(self.clients)],
                "process_logs": [
                    name for name in potential_process_logs if (self.artifact_dir / name).is_file()
                ],
            },
        }

    def _write_report(
        self, result: str, failure: BaseException | None = None, stage: str | None = None
    ) -> None:
        report = self._base_report(result)
        if failure is not None:
            report["failure"] = {
                "stage": stage or self.stage,
                "type": type(failure).__name__,
                "message": redact_sensitive_text(failure, self._known_sensitive_values),
            }
        atomic_write_json(self.report_path, report)

    def _print_summary(
        self, result: str, failure: BaseException | None = None, stage: str | None = None
    ) -> None:
        summary: dict[str, Any] = {
            "kind": REPORT_KIND,
            "result": result,
            "candidate_sha": self.candidate_sha
            if isinstance(self.candidate_sha, str)
            and re.fullmatch(r"[0-9a-fA-F]{40}", self.candidate_sha)
            else None,
            "passed_checks": sum(check["status"] == "passed" for check in self.checks),
        }
        if failure is not None:
            summary["failure"] = {
                "stage": stage or self.stage,
                "type": type(failure).__name__,
                "message": redact_sensitive_text(failure, self._known_sensitive_values),
            }
        print(json.dumps(summary, sort_keys=True), flush=True)

    def _terminate_owned_process(self, process: ProcessCapture) -> None:
        if process.process.poll() is not None:
            process.join_capture()
            return
        try:
            if os.name == "nt":
                process.process.terminate()
            else:
                os.killpg(process.process.pid, signal.SIGTERM)
            process.process.wait(timeout=3.0)
        except (OSError, subprocess.TimeoutExpired):
            try:
                if os.name == "nt":
                    process.process.kill()
                else:
                    os.killpg(process.process.pid, signal.SIGKILL)
                process.process.wait(timeout=2.0)
            except (OSError, subprocess.TimeoutExpired):
                pass
        process.join_capture()

    def cleanup(self) -> None:
        if self.observer is not None:
            self.observer.close()
            self.observer = None
        for client in self.clients.values():
            client.monitor.close()
        for client in self.clients.values():
            self._terminate_owned_process(client.process)
        for proxy in self.proxies.values():
            proxy.close()
        if self.server is not None:
            self._terminate_owned_process(self.server)
            self.server = None
        if os.name != "nt":
            for client in self.clients.values():
                try:
                    Path(client.ipc_path).unlink()
                except FileNotFoundError:
                    pass

    def run(self) -> int:
        if self.artifact_dir.exists() and any(self.artifact_dir.iterdir()):
            error = HarnessFailure(
                "artifact-preflight",
                "artifact directory must be absent or empty so an earlier failure cannot be overwritten",
            )
            self._print_summary("failed", error, error.stage)
            return 2
        self.artifact_dir.mkdir(parents=True, exist_ok=True)
        self.ledger = TraceLedger(self.trace_path, self.correlation_id)
        self._emit(event="verification-started", detail="packaged multi-process lifecycle")
        try:
            self.preflight()
            first, second = self._create_fixtures()
            self._start_server()
            self._connect_observer_and_seed(first, second)
            self._verify_initial_players(first, second)
            self._verify_play_pause_seek()
            self._verify_late_join_and_status(first, second)
            self._verify_partitioned_follower_catches_up_to_missed_start()
            self._verify_resume_eof_and_playlist()
            self._verify_clean_shutdown_and_withdrawal()
            self._emit(event="verification-passed", detail="all required lifecycle checks")
            self._write_report("passed")
            self._print_summary("passed")
            return 0
        except MissingPrerequisite as error:
            self._emit(event="verification-skipped", detail="missing declared prerequisite")
            self._write_report("skipped", error, error.stage)
            self._print_summary("skipped", error, error.stage)
            return MISSING_PREREQUISITE_EXIT
        except (HarnessFailure, OSError, ValueError, subprocess.SubprocessError) as error:
            stage = error.stage if isinstance(error, HarnessFailure) else self.stage
            self._emit(event="verification-failed", detail=stage)
            self._write_report("failed", error, stage)
            self._print_summary("failed", error, stage)
            return 1
        finally:
            self.cleanup()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    run = subparsers.add_parser("run", help="execute the packaged real-mpv lifecycle")
    run.add_argument("--server", type=Path, required=True, help="exact sorotte-server executable")
    run.add_argument("--client", type=Path, required=True, help="exact sorotte-cli executable")
    run.add_argument("--mpv", type=Path, required=True, help="exact supported mpv executable")
    run.add_argument("--artifact-dir", type=Path, required=True)
    run.add_argument("--candidate-sha", help="full git SHA represented by the candidate binaries")
    run.add_argument("--timeout-seconds", type=float, default=DEFAULT_TIMEOUT_SECONDS)
    run.add_argument(
        "--client-runtime-seconds", type=float, default=DEFAULT_CLIENT_RUNTIME_SECONDS
    )
    stage = subparsers.add_parser(
        "stage-safe-evidence",
        help="validate and stage only privacy-safe system evidence for publication",
    )
    stage.add_argument("--artifact-dir", type=Path, required=True)
    stage.add_argument("--output-dir", type=Path, required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command == "run":
        if not math.isfinite(args.timeout_seconds) or args.timeout_seconds <= 0.0:
            raise SystemExit("--timeout-seconds must be finite and positive")
        if (
            not math.isfinite(args.client_runtime_seconds)
            or args.client_runtime_seconds <= 8.0
        ):
            raise SystemExit("--client-runtime-seconds must be finite and greater than 8")
        harness = PlaybackLifecycleHarness(
            server_path=args.server,
            client_path=args.client,
            mpv_path=args.mpv,
            artifact_dir=args.artifact_dir,
            candidate_sha=args.candidate_sha,
            timeout_seconds=args.timeout_seconds,
            client_runtime_seconds=args.client_runtime_seconds,
        )
        return harness.run()
    if args.command == "stage-safe-evidence":
        try:
            manifest = stage_privacy_safe_evidence(args.artifact_dir, args.output_dir)
        except (OSError, ValueError, json.JSONDecodeError) as error:
            print(
                json.dumps(
                    {
                        "kind": "sorotte-playback-lifecycle-safe-evidence",
                        "result": "failed",
                        "message": redact_sensitive_text(
                            error, (args.artifact_dir, args.output_dir)
                        ),
                    },
                    sort_keys=True,
                ),
                flush=True,
            )
            return 1
        print(json.dumps(manifest, sort_keys=True), flush=True)
        return 0
    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
