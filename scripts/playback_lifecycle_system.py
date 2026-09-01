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
import struct
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence

import playback_lifecycle_evidence as lifecycle_evidence
import playback_lifecycle_faults as lifecycle_faults


SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-playback-lifecycle-system"
TRACE_KIND = "sorotte-playback-lifecycle-causal-trace"
PLAYER_TRACE_KIND = "sorotte-real-mpv-observation"
MISSING_PREREQUISITE_EXIT = 125
DEFAULT_TIMEOUT_SECONDS = 135.0
DEFAULT_CLIENT_RUNTIME_SECONDS = 65.0
FIRST_MEDIA_DURATION_SECONDS = 30.0
INTENDED_EOF_LEAD_SECONDS = 2.5
ROOM_VERSION = "1.7.5"
MINIMUM_MPV_VERSION = (0, 41, 0)
MINIMUM_CROSS_PROCESS_EDGES = 6
WRITE_FAILURE_FRAME_PAYLOAD_BYTES = 256 * 1024
LIFECYCLE_WRITE_BARRIER_MODE = "leased-oversized-frame"
LIFECYCLE_WRITE_BARRIER_READY_SUFFIX = ".leased-frame-ready"
LIFECYCLE_WRITE_BARRIER_RELEASE_SUFFIX = ".leased-frame-release"

ORACLE_CONVERGENCE_CHECKS = frozenset(
    {
        "room-switch-rejoin-preserved-authority",
        "initial-two-clients-loaded-paused",
        "play-committed-and-applied",
        "pause-committed-and-applied",
        "seek-committed-and-applied",
        "late-joiner-caught-up",
        "participant-status-snapshot-and-cadence",
        "participant-status-single-loss-self-healed",
        "participant-status-delayed-and-stale",
        "participant-status-fresh-recovery-advisory",
        "untrusted-selection-rejected-and-restored",
        "same-index-replacement-fresh-authority",
        "empty-playlist-clears-selected-media",
        "playlist-restore-reloads-selected-media",
        "partitioned-follower-caught-up",
        "post-reconnect-room-stable",
        "scheduled-write-failure-recovered",
        "natural-eof-advanced-once",
        "next-item-loaded-everywhere",
        "final-item-canonical-terminal-bounded",
        "participant-status-withdrawal",
    }
)

ROLE_USERNAMES = {
    "observer": "life-observer",
    "controller": "life-control",
    "follower": "life-follower",
    "late": "life-late",
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
    "member_role",
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
    "selection_present",
    "playlist_size",
    "set_by",
    "do_seek",
    "transport_revision",
    "status_revision",
    "status_mode",
    "participants",
    "participant_health",
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
_CONTAINED_PLAYER_FAILURE = "warning: external player step '"


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def contained_player_failure_counts(
    process_logs: Mapping[str, Path],
) -> dict[str, int]:
    """Count contained player-step failures without retaining log contents."""

    failures: dict[str, int] = {}
    for role, path in process_logs.items():
        count = sum(
            _CONTAINED_PLAYER_FAILURE in line
            for line in path.read_text(encoding="utf-8", errors="replace").splitlines()
        )
        if count:
            failures[role] = count
    return failures


def participant_status_authority_withdrawn(
    event: Mapping[str, Any], role: str
) -> bool:
    """Return whether a live status snapshot no longer grants fresh authority."""

    if event.get("event") != "participant-status-snapshot":
        return False
    if role not in set(event.get("participants", [])):
        return True
    views = event.get("participant_views", {})
    view = views.get(role) if isinstance(views, dict) else None
    return isinstance(view, dict) and view.get("availability") in {
        "delayed",
        "stale",
        "awaitingReport",
        "unavailable",
    }


def bounded_playback_frame_delivery(
    records: Sequence[Mapping[str, Any]],
    *,
    after: int,
    minimum_frame_bytes: int,
) -> Mapping[str, Any] | None:
    """Find the staged playback frame large enough to own the fault boundary."""

    return next(
        (
            record
            for record in records[after:]
            if record.get("transition") == "TX-DELIVERY-001"
            and isinstance(record.get("identities"), Mapping)
            and record["identities"].get("frame-bytes", 0) >= minimum_frame_bytes
        ),
        None,
    )


def exact_leased_frame_failure(
    records: Sequence[Mapping[str, Any]],
    *,
    after: int,
    frame_receipt: int,
) -> Mapping[str, Any] | None:
    """Return exact lease failure, rejecting a locally completed target frame."""

    terminal_records = [
        record
        for record in records[after:]
        if isinstance(record.get("identities"), Mapping)
        and record["identities"].get("frame-receipt") == frame_receipt
        and record.get("transition") in {"TX-WRITTEN-001", "TX-FAIL-001"}
    ]
    if any(record.get("transition") == "TX-WRITTEN-001" for record in terminal_records):
        raise ValueError(
            "the exact leased playback frame completed before the scheduled reset"
        )
    return next(
        (
            record
            for record in terminal_records
            if record.get("transition") == "TX-FAIL-001"
        ),
        None,
    )


def lifecycle_write_barrier_paths(evidence_path: Path) -> tuple[Path, Path]:
    return (
        evidence_path.with_name(
            evidence_path.name + LIFECYCLE_WRITE_BARRIER_READY_SUFFIX
        ),
        evidence_path.with_name(
            evidence_path.name + LIFECYCLE_WRITE_BARRIER_RELEASE_SUFFIX
        ),
    )


def validate_terminal_playlist_boundary(
    canonical_events: Iterable[Mapping[str, Any]],
    player_records: Mapping[str, Sequence[Mapping[str, Any]]],
    *,
    terminal_duration_seconds: float,
    initial_transport_revision: int,
) -> None:
    """Prove bounded canonical and physical state at a no-loop final EOF."""

    if not math.isfinite(terminal_duration_seconds) or terminal_duration_seconds <= 0.0:
        raise ValueError("the final playlist duration must be finite and positive")
    if initial_transport_revision <= 0:
        raise ValueError("the initial transport revision must be positive")

    canonical_events = list(canonical_events)
    index_mutations = [
        event for event in canonical_events if event.get("event") == "playlist-index"
    ]
    if index_mutations:
        raise ValueError("the final playlist item mutated canonical selection with looping disabled")

    playstates = [
        event for event in canonical_events if event.get("event") == "playstate"
    ]
    terminal_states = [event for event in playstates if event.get("paused") is True]
    if not terminal_states:
        raise ValueError("the final playlist item never committed canonical terminal pause")
    if len(terminal_states) < 2:
        raise ValueError("canonical terminal pause was not observed to remain stable")

    terminal_state = terminal_states[0]
    terminal_position = _safe_number(terminal_state.get("position_seconds"))
    lower_bound = max(0.0, terminal_duration_seconds - 1.5)
    upper_bound = terminal_duration_seconds + 0.75
    if terminal_position is None or not lower_bound <= terminal_position <= upper_bound:
        raise ValueError("canonical terminal position was not bounded by media duration")
    terminal_revision = terminal_state.get("transport_revision")
    if terminal_revision != initial_transport_revision + 1:
        raise ValueError("canonical terminal pause did not commit exactly one transport revision")
    if terminal_state.get("do_seek") is True:
        raise ValueError("canonical terminal pause incorrectly became a seek")

    first_terminal_index = playstates.index(terminal_state)
    for state in playstates[first_terminal_index:]:
        position = _safe_number(state.get("position_seconds"))
        if state.get("paused") is not True:
            raise ValueError("canonical playback resumed after the no-loop terminal pause")
        if state.get("transport_revision") != terminal_revision:
            raise ValueError("canonical terminal authority mutated more than once")
        if position is None or not lower_bound <= position <= upper_bound:
            raise ValueError("canonical terminal position drifted beyond media duration")
        if abs(position - terminal_position) > 0.25:
            raise ValueError("canonical terminal position continued projecting after pause")

    total_eof_records = 0
    for role, records in player_records.items():
        eof_indices = [
            index
            for index, record in enumerate(records)
            if record.get("event") == "end-file"
            and record.get("reason") == "eof"
            and record.get("media_slot") == "media-2"
        ]
        if len(eof_indices) > 1:
            raise ValueError(
                f"the {role} player emitted {len(eof_indices)} final-item EOF records instead of one"
            )
        if any(record.get("event") == "file-loaded" for record in records):
            raise ValueError(f"the {role} player reloaded media at the no-loop final boundary")
        if eof_indices:
            total_eof_records += 1
            continue

        resumed_indices = [
            index
            for index, record in enumerate(records)
            if record.get("media_slot") == "media-2" and record.get("paused") is False
        ]
        resumed_index = resumed_indices[0] if resumed_indices else -1
        physically_paused_near_terminal = any(
            index > resumed_index
            and record.get("media_slot") == "media-2"
            and record.get("paused") is True
            and (position := _safe_number(record.get("position_seconds"))) is not None
            and lower_bound <= position <= upper_bound
            for index, record in enumerate(records)
        )
        if not physically_paused_near_terminal:
            raise ValueError(
                f"the {role} player neither reached EOF nor paused near the terminal position"
            )

    if total_eof_records == 0:
        raise ValueError("no real player produced the natural EOF that authorized terminal pause")


def validate_natural_eof_successor_boundary(
    canonical_events: Iterable[Mapping[str, Any]],
    player_records: Mapping[str, Sequence[Mapping[str, Any]]],
    *,
    previous_transport_revision: int,
    expected_playlist_index: int = 1,
    predecessor_media_slot: str = "media-1",
    successor_media_slot: str = "media-2",
) -> int:
    """Prove that completed-media authority cannot cross a playlist advance."""

    if previous_transport_revision <= 0:
        raise ValueError("the predecessor transport revision must be positive")

    canonical_events = list(canonical_events)
    index_mutations = [
        (index, event)
        for index, event in enumerate(canonical_events)
        if event.get("event") == "playlist-index"
    ]
    if len(index_mutations) != 1 or index_mutations[0][1].get(
        "playlist_index"
    ) != expected_playlist_index:
        raise ValueError("natural EOF did not produce exactly one bounded playlist advance")

    selection_offset = index_mutations[0][0]
    successor_playstates = [
        event
        for event in canonical_events[selection_offset + 1 :]
        if event.get("event") == "playstate"
    ]
    if len(successor_playstates) < 2:
        raise ValueError("successor transport authority was not observed to remain stable")

    successor = successor_playstates[0]
    successor_revision = successor.get("transport_revision")
    if (
        not isinstance(successor_revision, int)
        or isinstance(successor_revision, bool)
        or successor_revision <= previous_transport_revision
    ):
        raise ValueError("the successor did not receive a fresh transport revision")

    for state in successor_playstates:
        position = _safe_number(state.get("position_seconds"))
        if state.get("paused") is not True:
            raise ValueError("canonical successor playback escaped its selection fence")
        if state.get("do_seek") is True:
            raise ValueError("the successor origin was incorrectly published as a seek")
        if state.get("transport_revision") != successor_revision:
            raise ValueError("canonical successor authority mutated during stabilization")
        if position is None or abs(position) > 0.25:
            raise ValueError("completed-media position crossed into canonical successor state")

    total_eof_records = 0
    for role, records in player_records.items():
        total_eof_records += sum(
            record.get("event") == "end-file"
            and record.get("reason") == "eof"
            and record.get("media_slot") == predecessor_media_slot
            for record in records
        )
        load_indices = [
            index
            for index, record in enumerate(records)
            if record.get("event") == "file-loaded"
            and record.get("media_slot") == successor_media_slot
        ]
        if len(load_indices) != 1:
            raise ValueError(
                f"the {role} player loaded the successor {len(load_indices)} times instead of one"
            )
        load_index = load_indices[0]
        convergence_index = next(
            (
                index
                for index, record in enumerate(records[load_index:], start=load_index)
                if record.get("media_slot") == successor_media_slot
                and record.get("paused") is True
                and (position := _safe_number(record.get("position_seconds")))
                is not None
                and abs(position) <= 0.75
            ),
            None,
        )
        if convergence_index is None:
            raise ValueError(f"the {role} player never converged at the successor origin")
        for record in records[convergence_index:]:
            if record.get("media_slot") != successor_media_slot:
                continue
            position = _safe_number(record.get("position_seconds"))
            if record.get("paused") is False:
                raise ValueError(f"the {role} successor resumed before start authority")
            if position is not None and abs(position) > 1.25:
                raise ValueError(
                    f"completed-media position crossed into the {role} successor"
                )

    if total_eof_records == 0:
        raise ValueError("no real player produced the natural EOF that authorized playlist advance")

    return successor_revision


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
    prerequisites = report.get("prerequisites")
    candidate_attestation = (
        prerequisites.get("candidate_attestation")
        if isinstance(prerequisites, dict)
        else None
    )
    if report.get("result") == "passed" and (
        not isinstance(candidate_attestation, dict)
        or candidate_attestation.get("verified") is not True
        or candidate_attestation.get("mode") != "verified-clean-checkout"
    ):
        raise ValueError("passed evidence lacks a verified clean candidate attestation")
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

    if report.get("result") == "passed":
        if not isinstance(artifacts, dict):
            raise ValueError("passed system report has no artifact inventory")
        process_names = artifacts.get("lifecycle_process_ledgers")
        expected_process_names = {
            "lifecycle-server.jsonl",
            "lifecycle-client-controller.jsonl",
            "lifecycle-client-follower.jsonl",
            "lifecycle-client-late.jsonl",
            "lifecycle-harness.jsonl",
        }
        if (
            not isinstance(process_names, list)
            or len(process_names) != len(expected_process_names)
            or set(process_names) != expected_process_names
        ):
            raise ValueError("passed system report has an incomplete lifecycle ledger inventory")
        if artifacts.get("lifecycle_evidence") != "lifecycle-evidence.jsonl":
            raise ValueError("passed system report has no merged lifecycle evidence")
        if (
            artifacts.get("lifecycle_evidence_summary")
            != "lifecycle-evidence-summary.json"
        ):
            raise ValueError("passed system report has no lifecycle evidence summary")
        process_paths = [artifact_dir / name for name in process_names]
        for path in process_paths:
            if not path.is_file():
                raise ValueError(f"declared lifecycle ledger is missing: {path.name}")
        merged_path = artifact_dir / "lifecycle-evidence.jsonl"
        summary_path = artifact_dir / "lifecycle-evidence-summary.json"
        if not merged_path.is_file() or not summary_path.is_file():
            raise ValueError("validated lifecycle evidence output is missing")
        server_prerequisite = prerequisites.get("server")
        client_prerequisite = prerequisites.get("client")
        if not isinstance(server_prerequisite, dict) or not isinstance(
            client_prerequisite, dict
        ):
            raise ValueError("passed evidence lacks packaged server and client attestations")
        with tempfile.TemporaryDirectory() as validation_directory:
            regenerated_merged = Path(validation_directory) / "lifecycle-evidence.jsonl"
            regenerated_summary = lifecycle_evidence.validate_and_merge(
                process_paths,
                model_path=Path(__file__).resolve().parents[1]
                / "coverage"
                / "playback-lifecycle.toml",
                output_path=regenerated_merged,
                required_inventories={
                    "server": frozenset({"server"}),
                    "client-controller": frozenset({"client", "player"}),
                    "client-follower": frozenset({"client", "player"}),
                    "client-late": frozenset({"client", "player"}),
                    "system-harness": frozenset({"harness", "proxy", "oracle"}),
                },
                required_roles=frozenset(
                    {"server", "client", "player", "proxy", "harness", "oracle"}
                ),
                expected_digests={
                    "server": server_prerequisite.get("sha256"),
                    "client-controller": client_prerequisite.get("sha256"),
                    "client-follower": client_prerequisite.get("sha256"),
                    "client-late": client_prerequisite.get("sha256"),
                    "system-harness": sha256_file(Path(__file__).resolve()),
                },
                minimum_cross_process_edges=MINIMUM_CROSS_PROCESS_EDGES,
            )
            if sha256_file(regenerated_merged) != sha256_file(merged_path):
                raise ValueError("merged lifecycle evidence is not reproducible")
        declared_summary = json.loads(summary_path.read_text(encoding="utf-8"))
        if declared_summary != regenerated_summary:
            raise ValueError("lifecycle evidence summary does not match validated ledgers")
        if report.get("lifecycle_summary") != declared_summary:
            raise ValueError("system report lifecycle summary does not match validated ledgers")
        assert_privacy_safe_tree(declared_summary)
        selected.extend([*process_paths, merged_path, summary_path])

        if artifacts.get("fault_schedule") != "fault-schedule.json" or artifacts.get(
            "fault_replay"
        ) != "fault-replay.jsonl":
            raise ValueError("passed system report has no deterministic fault replay evidence")
        fault_schedule_path = artifact_dir / "fault-schedule.json"
        fault_replay_path = artifact_dir / "fault-replay.jsonl"
        schedule = lifecycle_faults.FaultSchedule.read(fault_schedule_path)
        replay = lifecycle_faults.read_replay_trace(fault_replay_path)
        declared_fault = report.get("fault_schedule")
        if not isinstance(declared_fault, dict):
            raise ValueError("passed system report has no fault schedule attestation")
        if (
            declared_fault.get("id") != schedule.schedule_id
            or declared_fault.get("staged_sha256") != sha256_file(fault_schedule_path)
            or declared_fault.get("replay_sha256") != sha256_file(fault_replay_path)
            or declared_fault.get("step_count") != len(schedule.steps)
        ):
            raise ValueError("fault schedule attestation does not match staged evidence")
        replay_ids = [record["step_id"] for record in replay]
        if (
            len(replay_ids) != len(set(replay_ids))
            or set(replay_ids) != {step.id for step in schedule.steps}
            or any(record["outcome"] != "applied" for record in replay)
        ):
            raise ValueError("fault replay is incomplete, duplicated, or unsuccessful")
        selected.extend([fault_schedule_path, fault_replay_path])

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
        "candidate_attestation": candidate_attestation,
        "files": sorted(files, key=lambda item: item["name"]),
    }
    atomic_write_json(output_dir / "evidence-manifest.json", manifest)
    return manifest


def generate_av_fixture(
    ffmpeg_path: Path,
    path: Path,
    duration_seconds: float,
    *,
    color: str,
    sample_rate: int = 48_000,
) -> dict[str, Any]:
    """Generate a deterministic, seekable A/V fixture without music-mode semantics."""

    if not math.isfinite(duration_seconds) or duration_seconds <= 0.0:
        raise ValueError("duration_seconds must be finite and positive")
    if not re.fullmatch(r"[a-z]+", color):
        raise ValueError("fixture color must be a lowercase named color")
    if sample_rate <= 0:
        raise ValueError("sample_rate must be positive")

    duration = f"{duration_seconds:.6f}".rstrip("0").rstrip(".")
    path.parent.mkdir(parents=True, exist_ok=True)
    command = [
        str(ffmpeg_path),
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-f",
        "lavfi",
        "-i",
        f"color=c={color}:s=320x180:r=10:d={duration}",
        "-f",
        "lavfi",
        "-i",
        f"anullsrc=r={sample_rate}:cl=mono:d={duration}",
        "-map",
        "0:v:0",
        "-map",
        "1:a:0",
        "-c:v",
        "ffv1",
        "-level",
        "3",
        "-pix_fmt",
        "yuv420p",
        "-threads:v",
        "1",
        "-c:a",
        "pcm_s16le",
        "-shortest",
        "-fflags",
        "+bitexact",
        "-flags:v",
        "+bitexact",
        "-map_metadata",
        "-1",
        "-map_chapters",
        "-1",
        "-f",
        "matroska",
        "-y",
        str(path),
    ]
    result = subprocess.run(
        command,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=max(15.0, min(60.0, duration_seconds * 3.0)),
        check=False,
    )
    if result.returncode != 0 or not path.is_file() or path.stat().st_size == 0:
        raise ValueError("FFmpeg failed to generate a deterministic A/V fixture")
    return {
        "sha256": sha256_file(path),
        "duration_seconds": duration_seconds,
        "container": "matroska",
        "video_codec": "ffv1",
        "width_pixels": 320,
        "height_pixels": 180,
        "frame_rate_hz": 10,
        "audio_codec": "pcm_s16le",
        "sample_rate_hz": sample_rate,
        "channels": 1,
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
local last_media_slot = nil

local function current_media_slot()
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

local function emit(event_name, reason, retain_terminal_media)
    sequence = sequence + 1
    local observed_media_slot = current_media_slot()
    if observed_media_slot ~= nil then
        last_media_slot = observed_media_slot
    end
    local emitted_media_slot = observed_media_slot
    if retain_terminal_media and emitted_media_slot == nil then
        emitted_media_slot = last_media_slot
    end
    local record = {{
        schema_version = {SCHEMA_VERSION},
        kind = {_lua_quote(PLAYER_TRACE_KIND)},
        sequence = sequence,
        observed_at_ms = math.floor(mp.get_time() * 1000),
        role = role,
        event = event_name,
        media_slot = emitted_media_slot,
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
mp.register_event("end-file", function(event) emit("end-file", event.reason, true) end)
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
        user_updates = set_payload.get("user")
        if isinstance(user_updates, dict):
            for username, update in user_updates.items():
                if isinstance(update, dict) and isinstance(update.get("room"), dict):
                    events.append(
                        {
                            "event": "room-membership-update",
                            "member_role": _role(username, role_by_username),
                        }
                    )
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
            if value is None or (
                isinstance(value, int) and not isinstance(value, bool)
            ):
                events.append(
                    {
                        "event": "playlist-index",
                        "selection_present": value is not None,
                        "playlist_index": value if value is not None else None,
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
                    "transport_revision": playstate.get("sorotteTransportRevision")
                    if isinstance(playstate.get("sorotteTransportRevision"), int)
                    and not isinstance(playstate.get("sorotteTransportRevision"), bool)
                    else None,
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


def project_client_protocol_message(message: Mapping[str, Any]) -> list[dict[str, Any]]:
    """Project client frames without retaining identity, media, or extension payloads."""

    state = message.get("State")
    if not isinstance(state, dict):
        return []
    playstate = state.get("playstate")
    if not isinstance(playstate, dict):
        return []
    revision = playstate.get("sorotteTransportRevision")
    return [
        {
            "event": "client-playstate",
            "paused": playstate.get("paused")
            if isinstance(playstate.get("paused"), bool)
            else None,
            "position_seconds": _safe_number(playstate.get("position")),
            "do_seek": playstate.get("doSeek")
            if isinstance(playstate.get("doSeek"), bool)
            else None,
            "transport_revision": revision
            if isinstance(revision, int) and not isinstance(revision, bool)
            else None,
        }
    ]


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
                    "features": {
                        "sorotteParticipantStatusV1": True,
                        "sorottePlaybackBarrierV1": True,
                    },
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
        trace_fields = dict(fields)
        participant_views = trace_fields.pop("participant_views", None)
        if isinstance(participant_views, dict):
            availability_values = {
                "fresh",
                "delayed",
                "stale",
                "awaitingReport",
                "unsupported",
                "unavailable",
            }
            connection_values = {
                "unavailable",
                "starting",
                "connected",
                "disconnected",
                "failed",
            }
            phase_values = {
                "empty",
                "loading",
                "prebuffering",
                "readyPaused",
                "playing",
                "rebuffering",
                "seeking",
                "ended",
                "failed",
                "unknown",
            }
            participant_health = []
            for role in sorted(set(ROLE_USERNAMES) & set(participant_views)):
                view = participant_views.get(role)
                if not isinstance(view, dict):
                    continue
                availability = view.get("availability")
                connection = view.get("playerConnection")
                phase = view.get("phase")
                participant_health.append(
                    ":".join(
                        (
                            role,
                            availability if availability in availability_values else "unknown",
                            connection if connection in connection_values else "unknown",
                            phase if phase in phase_values else "unknown",
                        )
                    )
                )
            if participant_health:
                trace_fields["participant_health"] = participant_health
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
    """Owned loopback TCP proxy driven by a replayable bounded fault schedule."""

    _FRAGMENT_SIZES = (1, 2, 5, 13, 29)

    def __init__(
        self,
        *,
        upstream_host: str,
        upstream_port: int,
        role: str,
        ledger: TraceLedger,
        fault_cursor: lifecycle_faults.FaultScheduleCursor | None = None,
    ) -> None:
        self.upstream_host = upstream_host
        self.upstream_port = upstream_port
        self.role = role
        self.ledger = ledger
        self.fault_cursor = fault_cursor
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
        self._completed_connection_count = 0
        self._fragment_count = 0
        self._forwarded_bytes = 0
        self._participant_status_report_count = 0
        self._participant_status_forwarded_count = 0
        self._participant_status_dropped_count = 0
        self._drop_next_participant_status_reports = 0
        self._block_participant_status_reports = False
        self._upstream_send_lock = threading.Lock()
        self._channel_allowed = {
            1: threading.Event(),
            2: threading.Event(),
        }
        for event in self._channel_allowed.values():
            event.set()
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
    def completed_connection_count(self) -> int:
        with self._lock:
            return self._completed_connection_count

    @property
    def fragment_count(self) -> int:
        with self._lock:
            return self._fragment_count

    @property
    def forwarded_bytes(self) -> int:
        with self._lock:
            return self._forwarded_bytes

    @property
    def participant_status_report_count(self) -> int:
        with self._lock:
            return self._participant_status_report_count

    @property
    def participant_status_forwarded_count(self) -> int:
        with self._lock:
            return self._participant_status_forwarded_count

    @property
    def participant_status_dropped_count(self) -> int:
        with self._lock:
            return self._participant_status_dropped_count

    @staticmethod
    def _participant_status_report_sequence(message: Mapping[str, Any]) -> int | None:
        state = message.get("State")
        if not isinstance(state, dict):
            return None
        extension = state.get("sorotteParticipantStatusV1")
        if not isinstance(extension, dict):
            return None
        report = extension.get("report")
        if not isinstance(report, dict):
            return None
        sequence = report.get("reportSequence")
        return (
            sequence
            if isinstance(sequence, int) and not isinstance(sequence, bool)
            else None
        )

    def drop_next_participant_status_reports(self, count: int = 1) -> None:
        if count <= 0:
            raise ValueError("participant status drop count must be positive")
        with self._lock:
            self._drop_next_participant_status_reports += count
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event="participant-status-drop-armed",
            detail="bounded advisory report loss",
        )

    def set_participant_status_blocked(self, blocked: bool) -> None:
        with self._lock:
            self._block_participant_status_reports = blocked
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event=(
                "participant-status-blocked"
                if blocked
                else "participant-status-unblocked"
            ),
            detail="advisory report lane only",
        )

    def _participant_status_frame_should_be_dropped(
        self, message: Mapping[str, Any]
    ) -> tuple[bool, int | None]:
        sequence = self._participant_status_report_sequence(message)
        if sequence is None:
            return False, None
        with self._lock:
            self._participant_status_report_count += 1
            should_drop = self._block_participant_status_reports
            if self._drop_next_participant_status_reports > 0:
                self._drop_next_participant_status_reports -= 1
                should_drop = True
            if should_drop:
                self._participant_status_dropped_count += 1
            else:
                self._participant_status_forwarded_count += 1
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event=(
                "participant-status-report-dropped"
                if should_drop
                else "participant-status-report-forwarded"
            ),
            source_sequence=sequence,
            detail="advisory report frame",
        )
        return should_drop, sequence

    @staticmethod
    def _without_participant_status_report(
        message: Mapping[str, Any],
    ) -> dict[str, Any] | None:
        state = message.get("State")
        if not isinstance(state, dict):
            return dict(message)
        forwarded_state = dict(state)
        forwarded_state.pop("sorotteParticipantStatusV1", None)
        forwarded_message = dict(message)
        if forwarded_state:
            forwarded_message["State"] = forwarded_state
        else:
            forwarded_message.pop("State", None)
        return forwarded_message or None

    def _fragmented_send(
        self,
        destination: socket.socket,
        data: bytes,
        *,
        scheduled_fragment_bytes: int | None = None,
    ) -> None:
        offset = 0
        fragment_index = 0
        while offset < len(data) and not self._stop.is_set():
            size = (
                scheduled_fragment_bytes
                if scheduled_fragment_bytes is not None
                else self._FRAGMENT_SIZES[fragment_index % len(self._FRAGMENT_SIZES)]
            )
            fragment_index += 1
            destination.sendall(data[offset : offset + size])
            sent = min(size, len(data) - offset)
            with self._lock:
                self._fragment_count += 1
                self._forwarded_bytes += sent
            offset += size

    @staticmethod
    def _reset_socket(value: socket.socket | None) -> None:
        if value is None:
            return
        try:
            linger = struct.pack("HH", 1, 0) if os.name == "nt" else struct.pack("ii", 1, 0)
            value.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, linger)
        except OSError:
            pass
        try:
            value.close()
        except OSError:
            pass

    def _execute_fault_step(
        self,
        step: lifecycle_faults.FaultStep,
        source: socket.socket | None,
        destination: socket.socket | None,
    ) -> None:
        if step.action in {"delay", "worker-stall"}:
            self._stop.wait(step.value / 1000.0)
        elif step.action == "backpressure":
            with self._upstream_send_lock:
                self._stop.wait(step.value / 1000.0)
        elif step.action == "fragment":
            pass
        elif step.action == "half-close":
            target = destination or source
            if target is None:
                raise lifecycle_faults.FaultScheduleError(
                    "half-close step has no active transport"
                )
            try:
                target.shutdown(socket.SHUT_WR)
            except OSError:
                pass
        elif step.action == "reset":
            if source is None and destination is None:
                raise lifecycle_faults.FaultScheduleError(
                    "reset step has no active transport"
                )
            self._reset_socket(source)
            self._reset_socket(destination)
        elif step.action == "channel-hold":
            self._channel_allowed.setdefault(step.value, threading.Event()).clear()
        elif step.action == "channel-release":
            self._channel_allowed.setdefault(step.value, threading.Event()).set()
        else:
            raise lifecycle_faults.FaultScheduleError(
                f"protocol proxy cannot execute {step.action}"
            )
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event="fault-step-applied",
            detail=f"{step.action}:{step.boundary}",
        )

    def _fault_checkpoint(
        self,
        boundary: str,
        source: socket.socket | None,
        destination: socket.socket | None,
    ) -> lifecycle_faults.FaultStep | None:
        if self.fault_cursor is None:
            return None
        return self.fault_cursor.checkpoint(
            boundary,
            lambda step: self._execute_fault_step(step, source, destination),
        )

    def _pump(
        self,
        source: socket.socket,
        destination: socket.socket,
        finished: threading.Event,
        observe_client_protocol: bool = False,
    ) -> None:
        line_buffer = bytearray()
        worker_boundary = "client-worker" if observe_client_protocol else "server-worker"
        frame_boundary = (
            "client-to-server-frame"
            if observe_client_protocol
            else "server-to-client-frame"
        )
        channel = self._channel_allowed[1 if observe_client_protocol else 2]
        try:
            while not self._stop.is_set() and not finished.is_set():
                while not self._stop.is_set() and not channel.wait(0.05):
                    pass
                if self._stop.is_set():
                    return
                self._fault_checkpoint(worker_boundary, source, destination)
                data = source.recv(16 * 1024)
                if not data:
                    return
                if observe_client_protocol:
                    line_buffer.extend(data)
                    while b"\n" in line_buffer:
                        line, _, tail = line_buffer.partition(b"\n")
                        line_buffer = bytearray(tail)
                        framed_line = bytes(line) + b"\n"
                        try:
                            message = json.loads(line.decode("utf-8"))
                        except (UnicodeDecodeError, json.JSONDecodeError):
                            fault = self._fault_checkpoint(
                                frame_boundary, source, destination
                            )
                            with self._upstream_send_lock:
                                self._fragmented_send(
                                    destination,
                                    framed_line,
                                    scheduled_fragment_bytes=(
                                        fault.value
                                        if fault is not None
                                        and fault.action == "fragment"
                                        else None
                                    ),
                                )
                            continue
                        if not isinstance(message, dict):
                            fault = self._fault_checkpoint(
                                frame_boundary, source, destination
                            )
                            with self._upstream_send_lock:
                                self._fragmented_send(
                                    destination,
                                    framed_line,
                                    scheduled_fragment_bytes=(
                                        fault.value
                                        if fault is not None
                                        and fault.action == "fragment"
                                        else None
                                    ),
                                )
                            continue
                        for event in project_client_protocol_message(message):
                            self.ledger.emit(
                                source="client-protocol",
                                role=self.role,
                                **event,
                            )
                        should_drop, _sequence = (
                            self._participant_status_frame_should_be_dropped(message)
                        )
                        if should_drop:
                            forwarded_message = self._without_participant_status_report(
                                message
                            )
                            if forwarded_message is None:
                                continue
                            framed_line = (
                                json.dumps(
                                    forwarded_message,
                                    ensure_ascii=False,
                                    separators=(",", ":"),
                                ).encode("utf-8")
                                + b"\n"
                            )
                        fault = self._fault_checkpoint(
                            frame_boundary, source, destination
                        )
                        with self._upstream_send_lock:
                            self._fragmented_send(
                                destination,
                                framed_line,
                                scheduled_fragment_bytes=(
                                    fault.value
                                    if fault is not None and fault.action == "fragment"
                                    else None
                                ),
                            )
                    if len(line_buffer) > 1024 * 1024:
                        # A malformed unterminated frame is never useful
                        # evidence and must not create an unbounded raw buffer.
                        line_buffer.clear()
                    continue
                fault = self._fault_checkpoint(frame_boundary, source, destination)
                self._fragmented_send(
                    destination,
                    data,
                    scheduled_fragment_bytes=(
                        fault.value
                        if fault is not None and fault.action == "fragment"
                        else None
                    ),
                )
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
                args=(client, upstream, finished, True),
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
            self._completed_connection_count += 1

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
        fault = self._fault_checkpoint("harness-partition", client, upstream)
        if fault is None:
            self._close_socket(client)
            self._close_socket(upstream)
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event="proxy-cut-and-hold",
            detail="active transport closed; replacement held",
        )

    def resume(self) -> None:
        with self._lock:
            client = self._active_client
            upstream = self._active_upstream
        self._fault_checkpoint("harness-reconnect", client, upstream)
        self._upstream_allowed.set()
        self.ledger.emit(
            source="protocol-fault-proxy",
            role=self.role,
            event="proxy-resumed",
            detail="replacement transport released",
        )

    def apply_scheduled_backpressure(self) -> None:
        with self._lock:
            client = self._active_client
            upstream = self._active_upstream
        fault = self._fault_checkpoint("harness-backpressure", client, upstream)
        if fault is None:
            raise lifecycle_faults.FaultScheduleError(
                "fault schedule has no harness backpressure step"
            )

    def apply_scheduled_write_failure_hold(self) -> None:
        with self._lock:
            client = self._active_client
            upstream = self._active_upstream
        fault = self._fault_checkpoint("harness-partition", client, upstream)
        if fault is None or fault.action != "channel-hold":
            raise lifecycle_faults.FaultScheduleError(
                "fault schedule has no client-write channel hold step"
            )

    def apply_scheduled_write_failure_reset(self) -> None:
        with self._lock:
            client = self._active_client
            upstream = self._active_upstream
        fault = self._fault_checkpoint("harness-partition", client, upstream)
        if fault is None or fault.action != "reset":
            raise lifecycle_faults.FaultScheduleError(
                "fault schedule has no client-write reset step"
            )

    def apply_scheduled_write_failure_release(self) -> None:
        with self._lock:
            client = self._active_client
            upstream = self._active_upstream
        fault = self._fault_checkpoint("harness-reconnect", client, upstream)
        if fault is None or fault.action != "channel-release":
            raise lifecycle_faults.FaultScheduleError(
                "fault schedule has no client-write channel release step"
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
    ffmpeg_path: Path
    artifact_dir: Path
    candidate_sha: str | None
    fault_schedule_path: Path | None = None
    allow_unverified_candidate: bool = False
    loop_at_end_of_playlist: bool = False
    timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS
    client_runtime_seconds: float = DEFAULT_CLIENT_RUNTIME_SECONDS
    correlation_id: str = field(default_factory=lambda: uuid.uuid4().hex)

    def __post_init__(self) -> None:
        self.repo_root = Path(__file__).resolve().parents[1]
        self.artifact_dir = self.artifact_dir.resolve()
        self.report_path = self.artifact_dir / "report.json"
        self.trace_path = self.artifact_dir / "causal-trace.jsonl"
        self.lifecycle_model_path = self.repo_root / "coverage" / "playback-lifecycle.toml"
        self.lifecycle_merged_path = self.artifact_dir / "lifecycle-evidence.jsonl"
        self.lifecycle_summary_path = (
            self.artifact_dir / "lifecycle-evidence-summary.json"
        )
        self.lifecycle_process_paths = {
            "server": self.artifact_dir / "lifecycle-server.jsonl",
            "client-controller": self.artifact_dir
            / "lifecycle-client-controller.jsonl",
            "client-follower": self.artifact_dir / "lifecycle-client-follower.jsonl",
            "client-late": self.artifact_dir / "lifecycle-client-late.jsonl",
            "system-harness": self.artifact_dir / "lifecycle-harness.jsonl",
        }
        if self.fault_schedule_path is None:
            self.fault_schedule_path = (
                self.repo_root
                / "fixtures"
                / "playback-lifecycle"
                / "system-fault-schedule-v1.json"
            )
        else:
            self.fault_schedule_path = self.fault_schedule_path.resolve()
        self.staged_fault_schedule_path = self.artifact_dir / "fault-schedule.json"
        self.fault_replay_path = self.artifact_dir / "fault-replay.jsonl"
        with (self.repo_root / "Cargo.toml").open("rb") as workspace_manifest:
            self.product_version = str(
                tomllib.load(workspace_manifest)["workspace"]["package"]["version"]
            )
        self.started_at = utc_now()
        self.deadline = time.monotonic() + self.timeout_seconds
        self.stage = "initialization"
        self.ledger: TraceLedger | None = None
        self.lifecycle_writer: lifecycle_evidence.EvidenceWriter | None = None
        self.lifecycle_validation: dict[str, Any] | None = None
        self.fault_schedule: lifecycle_faults.FaultSchedule | None = None
        self.fault_cursor: lifecycle_faults.FaultScheduleCursor | None = None
        self.fault_ledger: lifecycle_faults.FaultReplayLedger | None = None
        self.fault_schedule_source_digest: str | None = None
        self.fault_schedule_validated = False
        self.server: ProcessCapture | None = None
        self.server_port: int | None = None
        self.observer: ProtocolObserver | None = None
        self.clients: dict[str, ClientProcess] = {}
        self.proxies: dict[str, ProtocolFaultProxy] = {}
        self._started_process_log_roles: list[str] = []
        self.checks: list[dict[str, Any]] = []
        self.prerequisites: dict[str, Any] = {}
        self.fixtures: dict[str, Any] = {}
        self._terminal_boundary_evidence: tuple[
            int,
            dict[str, int],
            float,
            int,
        ] | None = None
        self._loop_boundary_evidence: tuple[
            int,
            dict[str, int],
            int,
        ] | None = None
        self.room = f"lifecycle-{self.correlation_id[:12]}"
        self._known_sensitive_values: list[object] = [
            self.server_path,
            self.client_path,
            self.mpv_path,
            self.ffmpeg_path,
            self.artifact_dir,
            self.fault_schedule_path,
        ]

    def _emit(self, *, source: str = "harness", role: str = "orchestrator", event: str, **fields: Any) -> None:
        if self.ledger is not None:
            self.ledger.emit(source=source, role=role, event=event, **fields)

    def _lifecycle_product_tails(self) -> tuple[str, ...]:
        tails: list[str] = []
        for emitter, path in self.lifecycle_process_paths.items():
            if emitter == "system-harness" or not path.is_file():
                continue
            records = lifecycle_evidence.read_jsonl(path)
            if records:
                event_id = records[-1].get("event_id")
                if isinstance(event_id, str):
                    tails.append(event_id)
        return tuple(tails)

    def _lifecycle_emit(
        self,
        *,
        process_role: str,
        subject: str,
        machine: str,
        transition: str,
        target_kind: str,
        trigger: str,
        authority_before: str,
        authority_after: str,
        expected_effect: str,
        observed_effect: str,
        disposition: str,
        identities: Mapping[str, int] | None = None,
        include_product_tails: bool = False,
    ) -> str | None:
        if self.lifecycle_writer is None:
            return None
        predecessors = self._lifecycle_product_tails() if include_product_tails else ()
        return self.lifecycle_writer.emit(
            process_role=process_role,
            subject=subject,
            machine=machine,
            transition=transition,
            target_kind=target_kind,
            trigger=trigger,
            authority_before=authority_before,
            authority_after=authority_after,
            expected_effect=expected_effect,
            observed_effect=observed_effect,
            disposition=disposition,
            identities=identities,
            causal_predecessors=predecessors,
        )

    def _start_lifecycle_writer(self) -> None:
        self.lifecycle_writer = lifecycle_evidence.EvidenceWriter(
            self.lifecycle_process_paths["system-harness"],
            run_id=self.correlation_id,
            emitter="system-harness",
            binary_role="harness",
            component_roles=("harness", "proxy", "oracle"),
            product_version=self.product_version,
            product_digest=sha256_file(Path(__file__).resolve()),
        )
        self._lifecycle_emit(
            process_role="harness",
            subject="system-harness",
            machine="application",
            transition="APP-LAUNCH-001",
            target_kind="process-boundary",
            trigger="startup",
            authority_before="unowned",
            authority_after="initializing",
            expected_effect="harness-starting",
            observed_effect="harness-starting",
            disposition="accepted",
        )
        self._lifecycle_emit(
            process_role="harness",
            subject="system-harness",
            machine="application",
            transition="APP-RUN-001",
            target_kind="process-boundary",
            trigger="startup",
            authority_before="initializing",
            authority_after="running",
            expected_effect="harness-running",
            observed_effect="harness-running",
            disposition="applied",
        )

    def _stop_lifecycle_writer(self) -> None:
        if self.lifecycle_writer is None:
            return
        self._lifecycle_emit(
            process_role="harness",
            subject="system-harness",
            machine="application",
            transition="APP-STOP-001",
            target_kind="process-boundary",
            trigger="shutdown",
            authority_before="running",
            authority_after="stopping",
            expected_effect="owned-resources-draining",
            observed_effect="owned-resources-drained",
            disposition="applied",
            include_product_tails=True,
        )
        self._lifecycle_emit(
            process_role="harness",
            subject="system-harness",
            machine="application",
            transition="APP-TERM-001",
            target_kind="process-boundary",
            trigger="shutdown",
            authority_before="stopping",
            authority_after="terminated",
            expected_effect="evidence-flushed",
            observed_effect="evidence-flushed",
            disposition="applied",
        )
        self.lifecycle_writer.close()
        self.lifecycle_writer = None

    def _validate_lifecycle_evidence(self) -> None:
        self.stage = "shared-causal-ledger"
        client_digest = self.prerequisites["client"]["sha256"]
        summary = lifecycle_evidence.validate_and_merge(
            list(self.lifecycle_process_paths.values()),
            model_path=self.lifecycle_model_path,
            output_path=self.lifecycle_merged_path,
            summary_path=self.lifecycle_summary_path,
            required_inventories={
                "server": frozenset({"server"}),
                "client-controller": frozenset({"client", "player"}),
                "client-follower": frozenset({"client", "player"}),
                "client-late": frozenset({"client", "player"}),
                "system-harness": frozenset({"harness", "proxy", "oracle"}),
            },
            required_roles=frozenset(
                {"server", "client", "player", "proxy", "harness", "oracle"}
            ),
            expected_digests={
                "server": self.prerequisites["server"]["sha256"],
                "client-controller": client_digest,
                "client-follower": client_digest,
                "client-late": client_digest,
                "system-harness": sha256_file(Path(__file__).resolve()),
            },
            minimum_cross_process_edges=MINIMUM_CROSS_PROCESS_EDGES,
        )
        self.lifecycle_validation = summary
        self._pass(
            "shared-causal-ledger-validated",
            "all runtime roles produced one privacy-safe exact-artifact ledger with cross-process causes",
        )

    def _start_fault_schedule(self) -> None:
        assert self.fault_schedule_path is not None
        self.fault_schedule = lifecycle_faults.FaultSchedule.read(self.fault_schedule_path)
        self.fault_schedule_source_digest = lifecycle_faults.sha256_file(
            self.fault_schedule_path
        )
        self.fault_schedule.write_atomic(self.staged_fault_schedule_path)
        self.fault_ledger = lifecycle_faults.FaultReplayLedger(
            self.fault_replay_path,
            self.fault_schedule.schedule_id,
        )
        self.fault_cursor = lifecycle_faults.FaultScheduleCursor(
            self.fault_schedule,
            ledger=self.fault_ledger,
        )

    def _finish_fault_schedule(self) -> None:
        if self.fault_cursor is None or self.fault_schedule is None:
            raise HarnessFailure(
                "fault-schedule-validation", "fault schedule was not initialized"
            )
        self.fault_cursor.assert_consumed()
        if self.fault_ledger is not None:
            self.fault_ledger.close()
            self.fault_ledger = None
        records = lifecycle_faults.read_replay_trace(self.fault_replay_path)
        if len(records) != len(self.fault_schedule.steps):
            raise HarnessFailure(
                "fault-schedule-validation",
                "fault replay did not retain exactly one result per scheduled step",
            )
        if any(record["outcome"] != "applied" for record in records):
            raise HarnessFailure(
                "fault-schedule-validation", "a scheduled fault did not apply"
            )
        self._pass(
            "fault-schedule-replayed-completely",
            "the committed delay, fragmentation, backpressure, half-close, reset, and worker-stall schedule was consumed exactly once",
        )
        self.fault_schedule_validated = True

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
        if check_id in ORACLE_CONVERGENCE_CHECKS:
            self._lifecycle_emit(
                process_role="oracle",
                subject=check_id,
                machine="canonical-transaction",
                transition="TX-CONVERGE-001",
                target_kind="server-state",
                trigger="internal",
                authority_before="peer-application-pending",
                authority_after="converged",
                expected_effect="capable-peers-converged",
                observed_effect="external-oracle-converged",
                disposition="observed",
                identities={"check_sequence": len(self.checks)},
                include_product_tails=True,
            )

    def _not_applicable(self, check_id: str, detail: str) -> None:
        self.checks.append({"id": check_id, "status": "not-applicable", "detail": detail})
        self._emit(event="check-not-applicable", check_id=check_id, detail=detail)

    def _resolve_candidate_sha(self) -> str:
        git_prefix = ["git", "-c", f"safe.directory={self.repo_root}"]
        try:
            head_result = subprocess.run(
                [*git_prefix, "rev-parse", "--verify", "HEAD^{commit}"],
                cwd=self.repo_root,
                capture_output=True,
                text=True,
                timeout=5.0,
                check=False,
            )
            status_result = subprocess.run(
                [*git_prefix, "status", "--porcelain", "--untracked-files=all"],
                cwd=self.repo_root,
                capture_output=True,
                text=True,
                timeout=5.0,
                check=False,
            )
        except (OSError, subprocess.SubprocessError):
            head_result = None
            status_result = None

        checkout_sha = (
            head_result.stdout.strip().lower()
            if head_result is not None and head_result.returncode == 0
            else None
        )
        if not isinstance(checkout_sha, str) or not re.fullmatch(
            r"[0-9a-f]{40}", checkout_sha
        ):
            checkout_sha = None
        checkout_dirty = (
            bool(status_result.stdout.strip())
            if status_result is not None and status_result.returncode == 0
            else None
        )

        value = (
            self.candidate_sha.strip().lower()
            if self.candidate_sha is not None
            else checkout_sha or ""
        )
        if not re.fullmatch(r"[0-9a-f]{40}", value):
            raise MissingPrerequisite("candidate-attestation", "a full 40-character candidate SHA is required")

        verified = checkout_sha == value and checkout_dirty is False
        self.prerequisites["candidate_attestation"] = {
            "verified": verified,
            "mode": "verified-clean-checkout" if verified else "development-unverified",
            "checkout_sha": checkout_sha,
            "dirty": checkout_dirty,
        }
        if not verified and not self.allow_unverified_candidate:
            raise MissingPrerequisite(
                "candidate-attestation",
                "candidate SHA must match a clean source checkout; use the explicit development override only for non-publishable diagnostics",
            )
        return value

    @staticmethod
    def _resolve_executable(requested: Path) -> Path | None:
        if requested.is_file():
            return requested.resolve()
        located = shutil.which(str(requested))
        return Path(located).resolve() if located else None

    def _version(self, path: Path, *, argument: str = "--version") -> str:
        result = subprocess.run(
            [str(path), argument],
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
            ("ffmpeg", self.ffmpeg_path),
        ):
            executable = self._resolve_executable(requested)
            if executable is None:
                raise MissingPrerequisite(self.stage, f"the declared {label} executable is unavailable")
            resolved[label] = executable
            self._known_sensitive_values.append(executable)
            version = self._version(
                executable,
                argument="-version" if label == "ffmpeg" else "--version",
            )
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
        self.ffmpeg_path = resolved["ffmpeg"]
        self._pass("prerequisites-attested", "candidate SHA and executable digests were captured")

    def _create_fixtures(self) -> tuple[Path, Path]:
        self.stage = "fixture-generation"
        fixture_dir = self.artifact_dir / "generated-media"
        first = fixture_dir / f"lifecycle-media-one-{self.correlation_id[:8]}.mkv"
        second = fixture_dir / f"lifecycle-media-two-{self.correlation_id[:8]}.mkv"
        self._known_sensitive_values.extend((first, second))
        self.fixtures = {
            "media-1": generate_av_fixture(
                self.ffmpeg_path,
                first,
                FIRST_MEDIA_DURATION_SECONDS,
                color="red",
            ),
            "media-2": generate_av_fixture(self.ffmpeg_path, second, 14.0, color="blue"),
        }
        self._emit(event="fixtures-generated", detail="two deterministic FFmpeg A/V fixtures")
        return first.resolve(), second.resolve()

    def _start_server(self) -> None:
        self.stage = "server-startup"
        port_ready = threading.Event()

        def inspect_stderr(line: str) -> None:
            match = re.search(r"sorotte-server listening on 127\.0\.0\.1:(\d+)", line)
            if match:
                self.server_port = int(match.group(1))
                port_ready.set()

        environment = {
            key: value
            for key, value in os.environ.items()
            if not key.upper().startswith("SOROTTE_")
        }
        environment.update(
            {
                "SOROTTE_LIFECYCLE_EVIDENCE_PATH": str(
                    self.lifecycle_process_paths["server"]
                ),
                "SOROTTE_LIFECYCLE_RUN_ID": self.correlation_id,
                "SOROTTE_LIFECYCLE_EMITTER": "server",
            }
        )
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
            env=environment,
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
        self.stage = "room-switch-rejoin"
        switch_cursor = self.observer.cursor()
        self.observer.send(
            {"Set": {"room": {"name": f"alternate-{self.correlation_id[:8]}"}}}
        )
        switched = self._observer_event(
            switch_cursor,
            lambda event: event.get("event") == "room-membership-update"
            and event.get("member_role") == "observer",
        )
        self.observer.send({"Set": {"room": {"name": self.room}}})
        self._observer_event(
            switched.sequence,
            lambda event: event.get("event") == "room-membership-update"
            and event.get("member_role") == "observer",
        )
        self._pass(
            "room-switch-rejoin-preserved-authority",
            "the authenticated observer switched rooms and rejoined before installing canonical playback authority",
        )

        self.stage = "canonical-seed"
        cursor = self.observer.cursor()
        self.observer.send(
            {"Set": {"playlistChange": {"files": [str(first), str(second)]}}}
        )
        self.observer.send({"Set": {"playlistIndex": {"index": 0}}})
        self.observer.send(
            {
                "State": {
                    "playstate": {
                        "position": 0.0,
                        "paused": True,
                        "doSeek": False,
                    }
                }
            }
        )
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

        self.stage = "canonical-reject"
        reject_cursor = self.observer.cursor()
        self.observer.send({"Set": {"playlistIndex": {"index": 999}}})
        self._observer_event(
            reject_cursor,
            lambda event: event.get("event") == "playlist-index"
            and event.get("playlist_index") == 0,
        )
        if any(
            event.fields.get("event") == "playlist-index"
            and event.fields.get("playlist_index") == 999
            for event in self.observer.events_after(reject_cursor)
        ):
            raise HarnessFailure(
                self.stage,
                "the rejected out-of-range selection mutated canonical authority",
            )
        self._pass(
            "canonical-playlist-reject-preserved-authority",
            "an invalid authenticated selection was rejected and the exact canonical row remained selected",
        )

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
                "SOROTTE_CLIENT_LOOP_AT_END_OF_PLAYLIST": (
                    "1" if self.loop_at_end_of_playlist else "0"
                ),
                "SOROTTE_CLIENT_LOOP_SINGLE_FILES": "0",
                "SOROTTE_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS": "1",
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
                "SOROTTE_LIFECYCLE_EVIDENCE_PATH": str(
                    self.lifecycle_process_paths[f"client-{role}"]
                ),
                "SOROTTE_LIFECYCLE_RUN_ID": self.correlation_id,
                "SOROTTE_LIFECYCLE_EMITTER": f"client-{role}",
            }
        )
        if role == "follower":
            environment["SOROTTE_LIFECYCLE_WRITE_BARRIER"] = (
                LIFECYCLE_WRITE_BARRIER_MODE
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
                fault_cursor=self.fault_cursor,
            )
            self.proxies[role] = proxy
            client_server_port = proxy.port
            self._lifecycle_emit(
                process_role="proxy",
                subject="protocol-fault-proxy",
                machine="session",
                transition="SESSION-CONNECT-001",
                target_kind="fault-boundary",
                trigger="internal",
                authority_before="disconnected",
                authority_after="connecting",
                expected_effect="proxy-listening",
                observed_effect="proxy-listening",
                disposition="applied",
                identities={"proxy_generation": 1},
                include_product_tails=True,
            )
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

    def _wait_player_progress(
        self,
        roles: Iterable[str],
        cursors: Mapping[str, int],
        *,
        media_slot: str,
        minimum_delta: float,
        timeout: float = 8.0,
    ) -> None:
        """Prove that each real player advances from its own observed baseline.

        Players do not necessarily apply a canonical Play frame at the same
        instant. An absolute target can therefore already be behind the first
        player by the time the last player has started. Capture each trace's
        latest position at its cursor and require a later, still-playing sample
        to advance by the requested delta.
        """
        for role in roles:
            cursor = cursors.get(role, 0)
            records = read_jsonl(self.clients[role].player_trace)
            baseline = next(
                (
                    float(record["position_seconds"])
                    for record in reversed(records[:cursor])
                    if record.get("media_slot") == media_slot
                    and _safe_number(record.get("position_seconds")) is not None
                ),
                None,
            )
            if baseline is None:
                raise HarnessFailure(
                    self.stage,
                    f"the {role} player had no position baseline for {media_slot}",
                )
            self._player_record(
                role,
                cursor,
                lambda record, baseline=baseline: (
                    record.get("media_slot") == media_slot
                    and record.get("paused") is False
                    and _safe_number(record.get("position_seconds")) is not None
                    and float(record["position_seconds"])
                    >= baseline + minimum_delta
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
        self._wait_player_progress(
            ("controller", "follower"),
            progress_cursors,
            media_slot="media-1",
            minimum_delta=0.5,
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

    def _verify_scheduled_transport_fault_recovery(self) -> None:
        assert self.observer is not None
        proxy = self.proxies.get("follower")
        if proxy is None:
            raise HarnessFailure(
                "scheduled-transport-recovery", "the follower fault proxy is missing"
            )
        self.stage = "scheduled-backpressure"
        proxy.apply_scheduled_backpressure()

        self.stage = "scheduled-half-close"
        previous_connections = proxy.upstream_connection_count
        proxy.cut_and_hold()
        self._lifecycle_emit(
            process_role="proxy",
            subject="follower-session",
            machine="session",
            transition="SESSION-LOSS-001",
            target_kind="fault-boundary",
            trigger="fault",
            authority_before="active",
            authority_after="reconnecting",
            expected_effect="old-connection-fenced",
            observed_effect="half-close-applied",
            disposition="applied",
            identities={"connection_generation": previous_connections},
            include_product_tails=True,
        )
        proxy.resume()
        self._wait(
            "the half-closed follower to establish a replacement connection",
            lambda: proxy.upstream_connection_count > previous_connections,
            timeout=8.0,
        )

        self.stage = "scheduled-half-close-convergence"
        observer_cursor = self.observer.cursor()
        follower_cursor = self._player_cursor("follower")
        self._command("controller", "seek 6.0")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=6.0,
            require_seek=True,
        )
        self._wait_player_state(
            ("follower",),
            {"follower": follower_cursor},
            media_slot="media-1",
            paused=True,
            position=6.0,
            timeout=8.0,
        )
        observer_cursor = self.observer.cursor()
        follower_cursor = self._player_cursor("follower")
        self._command("controller", "seek 7.0")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=7.0,
            require_seek=True,
        )
        self._wait_player_state(
            ("follower",),
            {"follower": follower_cursor},
            media_slot="media-1",
            paused=True,
            position=7.0,
            timeout=8.0,
        )
        self._pass(
            "scheduled-half-close-reconnected",
            "bounded backpressure and half-close forced a production reconnect that applied newer canonical seek authority",
        )

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

    def _verify_participant_status_loss_delay_stale_recovery(self) -> None:
        assert self.observer is not None
        proxy = self.proxies.get("follower")
        if proxy is None:
            raise HarnessFailure(
                "participant-status-faults", "the follower fault proxy is missing"
            )

        canonical_events = [
            event
            for event in self.observer.events_after(0)
            if event.fields.get("event") == "playstate"
            and isinstance(event.fields.get("transport_revision"), int)
        ]
        if not canonical_events:
            raise HarnessFailure(
                "participant-status-faults",
                "canonical transport authority was unavailable before status faults",
            )
        canonical_baseline = canonical_events[-1].fields
        baseline_revision = canonical_baseline["transport_revision"]
        baseline_position = _safe_number(canonical_baseline.get("position_seconds"))
        if canonical_baseline.get("paused") is not True or baseline_position is None:
            raise HarnessFailure(
                "participant-status-faults",
                "status faults require a finite paused canonical baseline",
            )

        def follower_availability(event: dict[str, Any], expected: str) -> bool:
            if event.get("event") != "participant-status-snapshot":
                return False
            views = event.get("participant_views", {})
            follower = views.get("follower") if isinstance(views, dict) else None
            return isinstance(follower, dict) and follower.get("availability") == expected

        self.stage = "participant-status-single-loss"
        dropped_before = proxy.participant_status_dropped_count
        proxy.drop_next_participant_status_reports()
        self._wait(
            "one real participant-status report to be dropped",
            lambda: proxy.participant_status_dropped_count > dropped_before,
            timeout=4.0,
        )
        forwarded_after_drop = proxy.participant_status_forwarded_count
        self._wait(
            "the next periodic participant-status report to self-heal the loss",
            lambda: proxy.participant_status_forwarded_count > forwarded_after_drop,
            timeout=5.0,
        )
        status_cursor = self.observer.cursor()
        self._observer_event(
            status_cursor,
            lambda event: follower_availability(event, "fresh"),
            timeout=5.0,
        )
        self._pass(
            "participant-status-single-loss-self-healed",
            "one dropped advisory report was repaired by a later complete periodic report",
        )

        self.stage = "participant-status-delay-and-stale"
        status_cursor = self.observer.cursor()
        dropped_before = proxy.participant_status_dropped_count
        proxy.set_participant_status_blocked(True)
        self._wait(
            "the advisory-only block to suppress a real report",
            lambda: proxy.participant_status_dropped_count > dropped_before,
            timeout=4.0,
        )
        delayed = self._observer_event(
            status_cursor,
            lambda event: follower_availability(event, "delayed"),
            timeout=7.0,
        )
        self._observer_event(
            delayed.sequence,
            lambda event: follower_availability(event, "stale"),
            # The server's stale boundary is ten seconds from the last accepted
            # report, while snapshots are emitted on an independent periodic
            # cadence. Leave one complete slow-host cadence beyond that
            # boundary instead of assuming the delayed observation occurred at
            # its earliest possible instant.
            timeout=12.0,
        )
        self._pass(
            "participant-status-delayed-and-stale",
            "the live member aged through server-derived delayed and stale classifications while only advisory reports were suppressed",
        )

        self.stage = "participant-status-fresh-recovery"
        recovery_cursor = self.observer.cursor()
        forwarded_before = proxy.participant_status_forwarded_count
        proxy.set_participant_status_blocked(False)
        self._wait(
            "a fresh real participant-status report after the advisory block",
            lambda: proxy.participant_status_forwarded_count > forwarded_before,
            timeout=5.0,
        )
        self._observer_event(
            recovery_cursor,
            lambda event: follower_availability(event, "fresh"),
            timeout=5.0,
        )

        for event in self.observer.events_after(status_cursor):
            if event.fields.get("event") == "playlist-index" and event.fields.get(
                "playlist_index"
            ) != 0:
                raise HarnessFailure(
                    self.stage,
                    "participant-status faults changed canonical playlist selection",
                )
            if event.fields.get("event") != "playstate":
                continue
            position = _safe_number(event.fields.get("position_seconds"))
            if (
                event.fields.get("paused") is not True
                or event.fields.get("transport_revision") != baseline_revision
                or position is None
                or abs(position - baseline_position) > 0.25
            ):
                raise HarnessFailure(
                    self.stage,
                    "participant-status faults changed canonical transport authority",
                )
        self._pass(
            "participant-status-fresh-recovery-advisory",
            "a fresh report restored detail without changing paused transport or playlist authority",
        )

    def _verify_same_index_selected_entry_replacement(
        self, first: Path, second: Path
    ) -> None:
        assert self.observer is not None
        roles = ("controller", "follower", "late")

        def latest_transport_revision() -> int:
            revisions = [
                event.fields["transport_revision"]
                for event in self.observer.events_after(0)
                if event.fields.get("event") == "playstate"
                and isinstance(event.fields.get("transport_revision"), int)
            ]
            if not revisions:
                raise HarnessFailure(
                    self.stage,
                    "canonical transport authority was unavailable before replacement",
                )
            return revisions[-1]

        def replace_selected_entry(
            files: list[Path], media_slot: str, prior_revision: int, label: str
        ) -> int:
            observer_cursor = self.observer.cursor()
            player_cursors = {role: self._player_cursor(role) for role in roles}
            self.observer.send(
                {"Set": {"playlistChange": {"files": [str(path) for path in files]}}}
            )
            contents = self._observer_event(
                observer_cursor,
                lambda event: event.get("event") == "playlist-change"
                and event.get("playlist_size") == 2,
            )
            selection = self._observer_event(
                contents.sequence,
                lambda event: event.get("event") == "playlist-index"
                and event.get("selection_present") is True
                and event.get("playlist_index") == 0
                and event.get("set_by") == "observer",
            )
            transport = self._observer_event(
                selection.sequence,
                lambda event: event.get("event") == "playstate"
                and event.get("paused") is True
                and abs(float(event.get("position_seconds", math.inf))) <= 0.25
                and event.get("set_by") == "observer",
            )
            revision = transport.fields.get("transport_revision")
            if not isinstance(revision, int) or revision <= prior_revision:
                raise HarnessFailure(
                    self.stage,
                    f"{label} did not establish fresh transport authority",
                )
            self._wait_player_state(
                roles,
                player_cursors,
                media_slot=media_slot,
                paused=True,
                position=0.0,
                tolerance=0.75,
                timeout=10.0,
            )
            return revision

        self.stage = "same-index-selected-entry-replacement"
        replacement_revision = replace_selected_entry(
            [second, second],
            "media-2",
            latest_transport_revision(),
            "same-index replacement",
        )
        self._pass(
            "same-index-replacement-fresh-authority",
            "changing the selected entry under row zero announced a fresh selection and reset every real player",
        )

        self.stage = "same-index-selected-entry-restore"
        replace_selected_entry(
            [first, second],
            "media-1",
            replacement_revision,
            "same-index restoration",
        )
        self._pass(
            "same-index-restore-fresh-authority",
            "restoring the first selected entry repeated the fresh paused-zero physical generation",
        )

    def _verify_untrusted_selection_rejected_and_restored(
        self, first: Path, second: Path
    ) -> None:
        assert self.observer is not None
        roles = ("controller", "follower", "late")
        lifecycle_cursors = {
            role: len(
                lifecycle_evidence.read_jsonl(
                    self.lifecycle_process_paths[f"client-{role}"]
                )
            )
            for role in roles
        }
        player_cursors = {role: self._player_cursor(role) for role in roles}

        self.stage = "untrusted-selection-rejection"
        observer_cursor = self.observer.cursor()
        self.observer.send(
            {
                "Set": {
                    "playlistChange": {
                        "files": [
                            "https://untrusted.invalid/private/video.mkv",
                            str(second),
                        ]
                    }
                }
            }
        )
        untrusted_change = self._observer_event(
            observer_cursor,
            lambda event: event.get("event") == "playlist-change"
            and event.get("playlist_size") == 2,
        )
        self._observer_event(
            untrusted_change.sequence,
            lambda event: event.get("event") == "playlist-index"
            and event.get("playlist_index") == 0,
        )
        for role in roles:
            path = self.lifecycle_process_paths[f"client-{role}"]
            cursor = lifecycle_cursors[role]
            self._wait(
                f"{role} trusted-domain rejection evidence",
                lambda path=path, cursor=cursor: next(
                    (
                        record
                        for record in lifecycle_evidence.read_jsonl(path)[cursor:]
                        if record.get("transition") == "MEDIA-UNTRUSTED-001"
                    ),
                    None,
                ),
                timeout=6.0,
            )
        for role in roles:
            leaked = [
                record
                for record in read_jsonl(self.clients[role].player_trace)[
                    player_cursors[role] :
                ]
                if record.get("media_slot") == "other"
                and record.get("event") in {"media-changed", "file-loaded"}
            ]
            if leaked:
                raise HarnessFailure(
                    self.stage,
                    f"the {role} real player attempted the rejected untrusted target",
                )

        self.stage = "untrusted-selection-restore"
        observer_cursor = self.observer.cursor()
        restore_cursors = {role: self._player_cursor(role) for role in roles}
        restore_lifecycle_cursors = {
            role: len(
                lifecycle_evidence.read_jsonl(
                    self.lifecycle_process_paths[f"client-{role}"]
                )
            )
            for role in roles
        }
        self.observer.send(
            {"Set": {"playlistChange": {"files": [str(first), str(second)]}}}
        )
        restored_change = self._observer_event(
            observer_cursor,
            lambda event: event.get("event") == "playlist-change"
            and event.get("playlist_size") == 2,
        )
        self._observer_event(
            restored_change.sequence,
            lambda event: event.get("event") == "playlist-index"
            and event.get("playlist_index") == 0,
        )
        for role in roles:
            path = self.lifecycle_process_paths[f"client-{role}"]
            cursor = restore_lifecycle_cursors[role]
            self._wait(
                f"{role} trusted media restoration evidence",
                lambda path=path, cursor=cursor: next(
                    (
                        record
                        for record in lifecycle_evidence.read_jsonl(path)[cursor:]
                        if record.get("transition") == "MEDIA-PLAYABLE-001"
                    ),
                    None,
                ),
                timeout=6.0,
            )
        seek_cursor = self.observer.cursor()
        self._command("controller", "seek 1.0")
        self._canonical_playstate(
            seek_cursor,
            paused=True,
            set_by="controller",
            position=1.0,
            require_seek=True,
        )
        self._wait_player_state(
            roles,
            restore_cursors,
            media_slot="media-1",
            paused=True,
            position=1.0,
            tolerance=0.75,
            timeout=10.0,
        )
        self._pass(
            "untrusted-selection-rejected-and-restored",
            "every packaged client rejected the remote target locally without opening it, then converged after trusted canonical media returned",
        )

    def _verify_scheduled_write_failure_recovery(self) -> None:
        assert self.observer is not None
        roles = ("controller", "follower", "late")
        proxy = self.proxies.get("follower")
        if proxy is None:
            raise HarnessFailure(
                "scheduled-write-failure", "the follower fault proxy is missing"
            )
        lifecycle_path = self.lifecycle_process_paths["client-follower"]
        lifecycle_cursor = len(lifecycle_evidence.read_jsonl(lifecycle_path))
        previous_connections = proxy.upstream_connection_count
        previous_completed_connections = proxy.completed_connection_count
        observer_cursor = self.observer.cursor()
        barrier_ready_path, barrier_release_path = lifecycle_write_barrier_paths(
            lifecycle_path
        )

        self.stage = "scheduled-write-failure-hold"
        proxy.apply_scheduled_write_failure_hold()
        large_target = (
            "fault-payload-" + ("x" * WRITE_FAILURE_FRAME_PAYLOAD_BYTES) + ".mkv"
        )
        self.clients["follower"].process.write_line(f"queue {large_target}")
        self._emit(
            event="local-command-issued",
            role="follower",
            detail="bounded-large-playlist-frame",
        )
        leased_record = self._wait(
            "the follower to lease the bounded large playback frame",
            lambda: bounded_playback_frame_delivery(
                lifecycle_evidence.read_jsonl(lifecycle_path),
                after=lifecycle_cursor,
                minimum_frame_bytes=WRITE_FAILURE_FRAME_PAYLOAD_BYTES,
            ),
            timeout=6.0,
        )
        leased_receipt = leased_record["identities"].get("frame-receipt")
        if not isinstance(leased_receipt, int) or leased_receipt <= 0:
            raise HarnessFailure(
                self.stage,
                "the bounded large playback frame has no valid lease receipt",
            )
        self._wait(
            "the exact leased playback frame to enter the deterministic write barrier",
            barrier_ready_path.is_file,
            timeout=3.0,
        )
        self.stage = "scheduled-write-failure-reset"
        proxy.apply_scheduled_write_failure_reset()
        self._wait(
            "the reset relay generation to terminate before releasing the leased frame",
            lambda: proxy.completed_connection_count > previous_completed_connections,
            timeout=3.0,
        )
        try:
            with barrier_release_path.open("x", encoding="utf-8") as release:
                release.write("reset\n")
                release.flush()
                os.fsync(release.fileno())
        except OSError as error:
            raise HarnessFailure(
                self.stage,
                f"failed to release the leased-frame write barrier: {redact_sensitive_text(error)}",
            ) from error
        self._emit(
            event="leased-frame-write-released",
            role="follower",
            detail="after scheduled transport reset",
        )

        def exact_frame_failure() -> dict[str, Any] | None:
            try:
                result = exact_leased_frame_failure(
                    lifecycle_evidence.read_jsonl(lifecycle_path),
                    after=lifecycle_cursor,
                    frame_receipt=leased_receipt,
                )
            except ValueError as error:
                raise HarnessFailure(
                    self.stage,
                    str(error),
                ) from error
            return dict(result) if result is not None else None

        self._wait(
            "the exact leased playback frame to terminate as a write failure",
            exact_frame_failure,
            timeout=6.0,
        )

        # Replace the failed oversized local mutation before the next transport
        # is released, so the retry can only carry the bounded prior snapshot.
        self.clients["follower"].process.write_line("undo")
        self._emit(
            event="local-command-issued",
            role="follower",
            detail="playlist-undo-after-write-failure",
        )
        settle_cursors = {role: self._player_cursor(role) for role in roles}
        proxy.apply_scheduled_write_failure_release()
        self._wait(
            "the follower to reconnect after the deterministic write failure",
            lambda: proxy.upstream_connection_count > previous_connections,
            timeout=8.0,
        )

        # The undo is the first authoritative follower mutation on the new
        # connection. Its canonical same-index selection resets playstate to
        # the beginning, and every client must finish applying that remote
        # correction before a fresh local command can be meaningfully tested.
        restored_selection = self._observer_event(
            observer_cursor,
            lambda event: event.get("event") == "playlist-index"
            and event.get("playlist_index") == 0
            and event.get("set_by") == "follower",
            timeout=8.0,
        )
        self._canonical_playstate(
            restored_selection.sequence,
            paused=True,
            set_by="controller",
            position=0.0,
            require_seek=False,
            timeout=8.0,
        )
        self._wait_player_state(
            roles,
            settle_cursors,
            media_slot="media-1",
            paused=True,
            position=0.0,
            timeout=8.0,
        )
        leaked_playlist = next(
            (
                event
                for event in self.observer.events_after(observer_cursor)
                if event.fields.get("event") == "playlist-change"
                and event.fields.get("playlist_size") != 2
            ),
            None,
        )
        if leaked_playlist is not None:
            raise HarnessFailure(
                self.stage,
                "the failed oversized playlist mutation reached canonical authority",
            )

        self.stage = "scheduled-write-failure-convergence"
        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "play")
        self._canonical_playstate(
            observer_cursor,
            paused=False,
            set_by="controller",
        )
        self._wait_player_state(
            roles,
            player_cursors,
            media_slot="media-1",
            paused=False,
            timeout=8.0,
        )

        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "pause")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
        )
        self._wait_player_state(
            roles,
            player_cursors,
            media_slot="media-1",
            paused=True,
            timeout=8.0,
        )

        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "seek 3.0")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=3.0,
            require_seek=True,
        )
        self._wait_player_state(
            roles,
            player_cursors,
            media_slot="media-1",
            paused=True,
            position=3.0,
            timeout=8.0,
        )
        try:
            barrier_ready_path.unlink()
            barrier_release_path.unlink()
        except OSError as error:
            raise HarnessFailure(
                self.stage,
                f"failed to retire lifecycle write barrier markers: {redact_sensitive_text(error)}",
            ) from error
        self._pass(
            "scheduled-write-failure-recovered",
            "a leased playback frame failed at the socket boundary, never reached authority, and every reconnected client applied fresh play, pause, and seek commands",
        )

    def _verify_empty_playlist_clear_and_restore(self, first: Path, second: Path) -> None:
        assert self.observer is not None
        roles = ("controller", "follower", "late")

        self.stage = "canonical-empty-playlist"
        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        self.observer.send({"Set": {"playlistChange": {"files": []}}})
        empty_change = self._observer_event(
            observer_cursor,
            lambda event: event.get("event") == "playlist-change"
            and event.get("playlist_size") == 0,
        )
        self._observer_event(
            empty_change.sequence,
            lambda event: event.get("event") == "playlist-index"
            and event.get("selection_present") is False,
        )
        for role in roles:
            self._player_record(
                role,
                player_cursors[role],
                lambda record: record.get("event") == "media-changed"
                and record.get("media_slot") is None,
                timeout=8.0,
            )
        self._pass(
            "empty-playlist-clears-selected-media",
            "an actual empty canonical playlist retired selection and cleared every real player",
        )

        self.stage = "canonical-playlist-restore"
        canonical_playstates = [
            event
            for event in self.observer.events_after(0)
            if event.fields.get("event") == "playstate"
            and isinstance(event.fields.get("transport_revision"), int)
        ]
        if not canonical_playstates:
            raise HarnessFailure(
                self.stage,
                "the selected-media transport revision was unavailable before restore",
            )
        base_transport_revision = canonical_playstates[-1].fields["transport_revision"]
        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        self.observer.send(
            {"Set": {"playlistChange": {"files": [str(first), str(second)]}}}
        )
        self._observer_event(
            observer_cursor,
            lambda event: event.get("event") == "playlist-change"
            and event.get("playlist_size") == 2,
        )
        next_selection_attempt = 0.0

        def restored_selection() -> ObserverEvent | None:
            nonlocal next_selection_attempt
            committed = next(
                (
                    event
                    for event in self.observer.events_after(observer_cursor)
                    if event.fields.get("event") == "playlist-index"
                    and event.fields.get("playlist_index") == 0
                    and event.fields.get("set_by") == "controller"
                ),
                None,
            )
            if committed is not None:
                return committed
            now = time.monotonic()
            if now >= next_selection_attempt:
                self._command("controller", "select 1")
                next_selection_attempt = now + 0.25
            return None

        restored_index = self._wait(
            "the production controller to commit the restored selection",
            restored_selection,
            timeout=5.0,
        )
        restored_transport = self._observer_event(
            restored_index.sequence,
            lambda event: event.get("event") == "playstate"
            and event.get("paused") is True
            and abs(float(event.get("position_seconds", math.inf))) <= 0.25
            and event.get("set_by") == "controller",
            timeout=5.0,
        )
        if (
            restored_transport.fields.get("transport_revision", 0)
            <= base_transport_revision
        ):
            raise HarnessFailure(
                self.stage,
                "the restored selection did not establish fresh transport authority",
            )
        self._wait_player_state(
            roles,
            player_cursors,
            media_slot="media-1",
            paused=True,
            position=0.0,
            tolerance=0.75,
            timeout=10.0,
        )
        self._pass(
            "playlist-restore-reloads-selected-media",
            "the restored canonical selection established a fresh paused-at-zero physical generation everywhere",
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
        upstream_before_cut = proxy.upstream_connection_count
        proxy.cut_and_hold()
        self._wait(
            "the production follower to reconnect into the held proxy",
            lambda: proxy.accepted_count > accepted_before_cut,
            timeout=5.0,
        )
        if proxy.upstream_connection_count != upstream_before_cut:
            raise HarnessFailure(
                self.stage,
                "the held follower replacement unexpectedly reached the server",
            )
        self._observer_event(
            status_cursor,
            lambda event: participant_status_authority_withdrawn(event, "follower"),
            timeout=6.0,
        )
        self._pass(
            "partition-withdraws-follower-status",
            "a deterministic transport cut withdrew fresh follower status authority while its replacement upstream remained held",
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

        # Earlier scenarios deliberately leave the first item playing while a
        # participant is partitioned. Give that media ample headroom so slow
        # CI cannot trigger an accidental EOF, then approach the boundary here
        # under explicit canonical authority.
        first_duration_seconds = float(self.fixtures["media-1"]["duration_seconds"])
        eof_position = max(0.0, first_duration_seconds - INTENDED_EOF_LEAD_SECONDS)
        self.stage = "pre-eof-positioning"
        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", f"seek {eof_position:.3f}")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=eof_position,
            require_seek=True,
        )
        self._wait_player_state(
            roles,
            player_cursors,
            media_slot="media-1",
            paused=True,
            position=eof_position,
            timeout=8.0,
        )

        self.stage = "resume-authority"
        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        eof_cursors = dict(player_cursors)
        self._command("controller", "play")
        resume_commit = self._canonical_playstate(
            observer_cursor,
            paused=False,
            set_by="controller",
        )
        self._wait_player_state(roles, player_cursors, media_slot="media-1", paused=False)
        self._pass("resume-committed-and-applied", "all three real players resumed from the canonical seek point")

        self.stage = "natural-eof-playlist-advance"
        advance = self._observer_event(
            observer_cursor,
            lambda event: event.get("event") == "playlist-index"
            and event.get("playlist_index") == 1,
            timeout=10.0,
        )
        successor_commit = self._observer_event(
            advance.sequence,
            lambda event: event.get("event") == "playstate"
            and event.get("paused") is True
            and event.get("set_by") == "controller"
            and event.get("do_seek") is False
            and _safe_number(event.get("position_seconds")) is not None
            and abs(float(event["position_seconds"])) <= 0.25
            and isinstance(event.get("transport_revision"), int)
            and not isinstance(event.get("transport_revision"), bool),
            timeout=5.0,
        )
        resume_transport_revision = resume_commit.fields.get("transport_revision")
        successor_transport_revision = successor_commit.fields.get("transport_revision")
        if (
            not isinstance(resume_transport_revision, int)
            or isinstance(resume_transport_revision, bool)
            or successor_transport_revision <= resume_transport_revision
        ):
            raise HarnessFailure(
                self.stage,
                "the successor item did not receive a fresh transport authority revision",
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

        successor_loaded_cursors: dict[str, int] = {}
        for role in roles:
            loaded = self._player_record(
                role,
                eof_cursors[role],
                lambda record: record.get("event") == "file-loaded"
                and record.get("media_slot") == "media-2",
                timeout=10.0,
            )
            loaded_sequence = loaded.get("sequence")
            if not isinstance(loaded_sequence, int) or isinstance(loaded_sequence, bool):
                raise HarnessFailure(
                    self.stage,
                    f"the {role} successor load evidence had no sequence fence",
                )
            successor_loaded_cursors[role] = max(0, loaded_sequence - 1)

        for role in roles:
            converged = self._player_record(
                role,
                successor_loaded_cursors[role],
                lambda record: record.get("media_slot") == "media-2"
                and record.get("paused") is True
                and _safe_number(record.get("position_seconds")) is not None
                and abs(float(record["position_seconds"])) <= 0.75,
                timeout=8.0,
            )
            converged_sequence = converged.get("sequence")
            if not isinstance(converged_sequence, int) or isinstance(
                converged_sequence, bool
            ):
                raise HarnessFailure(
                    self.stage,
                    f"the {role} successor convergence evidence had no sequence fence",
                )

        def successor_authority_observed_twice() -> bool:
            return (
                len(
                    [
                        event
                        for event in self.observer.events_after(successor_commit.sequence - 1)
                        if event.fields.get("event") == "playstate"
                        and event.fields.get("paused") is True
                        and event.fields.get("transport_revision")
                        == successor_transport_revision
                        and _safe_number(event.fields.get("position_seconds"))
                        is not None
                        and abs(float(event.fields["position_seconds"])) <= 0.25
                    ]
                )
                >= 2
            )

        self._wait(
            "successor transport authority to remain stable across a server refresh",
            successor_authority_observed_twice,
            timeout=3.0,
        )

        try:
            verified_successor_revision = validate_natural_eof_successor_boundary(
                (event.fields for event in self.observer.events_after(observer_cursor)),
                {
                    role: read_jsonl(self.clients[role].player_trace)[
                        eof_cursors[role] :
                    ]
                    for role in roles
                },
                previous_transport_revision=resume_transport_revision,
            )
        except ValueError as error:
            raise HarnessFailure(self.stage, str(error)) from error
        if verified_successor_revision != successor_transport_revision:
            raise HarnessFailure(
                self.stage,
                "successor transport evidence changed while it was being verified",
            )
        self._pass(
            "natural-eof-advanced-once",
            "real mpv EOF produced exactly one canonical transition from item zero to item one",
        )
        self._pass(
            "next-item-loaded-everywhere",
            "every managed real mpv loaded the server-selected second item",
        )
        self._pass(
            "natural-eof-successor-authority-reset",
            "the server retired completed-media transport state and every real mpv remained at the paused successor origin",
        )

    def _verify_terminal_playlist_boundary(self) -> None:
        assert self.observer is not None
        roles = ("controller", "follower", "late")

        self.stage = "final-item-near-tail-seek"
        observer_cursor = self.observer.cursor()
        seek_cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "seek 11.0")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=11.0,
            require_seek=True,
        )
        self._wait_player_state(
            roles,
            seek_cursors,
            media_slot="media-2",
            paused=True,
            position=11.0,
            tolerance=1.25,
        )
        self._pass(
            "final-item-seek-committed-and-applied",
            "canonical near-tail seek reached every real player on the final item",
        )

        self.stage = "final-item-natural-eof"
        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "play")
        play_commit = self._canonical_playstate(
            observer_cursor,
            paused=False,
            set_by="controller",
        )
        initial_transport_revision = play_commit.fields.get("transport_revision")
        if not isinstance(initial_transport_revision, int) or isinstance(
            initial_transport_revision, bool
        ):
            raise HarnessFailure(
                self.stage,
                "the final-item Play commit did not carry transport authority",
            )
        self._wait_player_state(
            roles,
            player_cursors,
            media_slot="media-2",
            paused=False,
            position=11.0,
            tolerance=1.5,
        )
        self._pass(
            "final-item-resume-committed-and-applied",
            "all three real players resumed the final item from canonical near-tail state",
        )

        terminal_duration_seconds = float(self.fixtures["media-2"]["duration_seconds"])
        terminal_commit = self._observer_event(
            play_commit.sequence,
            lambda event: event.get("event") == "playstate"
            and event.get("paused") is True
            and event.get("set_by") in roles
            and event.get("do_seek") is False
            and event.get("transport_revision") == initial_transport_revision + 1
            and _safe_number(event.get("position_seconds")) is not None
            and abs(float(event["position_seconds"]) - terminal_duration_seconds) <= 0.75,
            timeout=8.0,
        )

        def terminal_authority_observed_twice() -> bool:
            return (
                len(
                    [
                        event
                        for event in self.observer.events_after(terminal_commit.sequence - 1)
                        if event.fields.get("event") == "playstate"
                        and event.fields.get("paused") is True
                        and event.fields.get("transport_revision")
                        == terminal_commit.fields.get("transport_revision")
                    ]
                )
                >= 2
            )

        self._wait(
            "canonical terminal pause to remain stable across a server refresh",
            terminal_authority_observed_twice,
            timeout=3.0,
        )

        stabilization_end = min(self.deadline, time.monotonic() + 0.5)
        while time.monotonic() < stabilization_end:
            time.sleep(0.05)
        validate_terminal_playlist_boundary(
            (event.fields for event in self.observer.events_after(observer_cursor)),
            {
                role: read_jsonl(self.clients[role].player_trace)[player_cursors[role] :]
                for role in roles
            },
            terminal_duration_seconds=terminal_duration_seconds,
            initial_transport_revision=initial_transport_revision,
        )
        self._terminal_boundary_evidence = (
            observer_cursor,
            player_cursors,
            terminal_duration_seconds,
            initial_transport_revision,
        )
        self._pass(
            "final-item-canonical-terminal-bounded",
            "natural EOF committed one stable terminal pause while selection remained on the final item",
        )

    def _verify_loop_playlist_boundary(self) -> None:
        assert self.observer is not None
        roles = ("controller", "follower", "late")

        self.stage = "loop-final-item-near-tail-seek"
        observer_cursor = self.observer.cursor()
        seek_cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "seek 11.0")
        self._canonical_playstate(
            observer_cursor,
            paused=True,
            set_by="controller",
            position=11.0,
            require_seek=True,
        )
        self._wait_player_state(
            roles,
            seek_cursors,
            media_slot="media-2",
            paused=True,
            position=11.0,
            tolerance=1.25,
        )
        self._pass(
            "loop-final-item-seek-committed-and-applied",
            "canonical near-tail seek reached every loop-enabled real player",
        )

        self.stage = "loop-final-item-natural-eof"
        observer_cursor = self.observer.cursor()
        player_cursors = {role: self._player_cursor(role) for role in roles}
        self._command("controller", "play")
        play_commit = self._canonical_playstate(
            observer_cursor,
            paused=False,
            set_by="controller",
        )
        initial_transport_revision = play_commit.fields.get("transport_revision")
        if not isinstance(initial_transport_revision, int) or isinstance(
            initial_transport_revision, bool
        ):
            raise HarnessFailure(
                self.stage,
                "the loop final-item Play commit did not carry transport authority",
            )
        self._wait_player_state(
            roles,
            player_cursors,
            media_slot="media-2",
            paused=False,
            position=11.0,
            tolerance=1.5,
        )
        self._pass(
            "loop-final-item-resume-committed-and-applied",
            "all three loop-enabled real players resumed the final item near its tail",
        )

        advance = self._observer_event(
            play_commit.sequence,
            lambda event: event.get("event") == "playlist-index"
            and event.get("playlist_index") == 0,
            timeout=8.0,
        )
        successor_commit = self._observer_event(
            advance.sequence,
            lambda event: event.get("event") == "playstate"
            and event.get("paused") is True
            and event.get("set_by") in roles
            and event.get("do_seek") is False
            and _safe_number(event.get("position_seconds")) is not None
            and abs(float(event["position_seconds"])) <= 0.25
            and isinstance(event.get("transport_revision"), int)
            and not isinstance(event.get("transport_revision"), bool),
            timeout=5.0,
        )
        successor_transport_revision = successor_commit.fields.get(
            "transport_revision"
        )
        if successor_transport_revision <= initial_transport_revision:
            raise HarnessFailure(
                self.stage,
                "the loop successor did not receive fresh transport authority",
            )

        def any_final_item_eof() -> bool:
            return any(
                any(
                    record.get("event") == "end-file"
                    and record.get("reason") == "eof"
                    and record.get("media_slot") == "media-2"
                    for record in read_jsonl(self.clients[role].player_trace)[
                        player_cursors[role] :
                    ]
                )
                for role in roles
            )

        self._wait("a loop-authorizing natural real-mpv EOF", any_final_item_eof, timeout=4.0)

        for role in roles:
            loaded = self._player_record(
                role,
                player_cursors[role],
                lambda record: record.get("event") == "file-loaded"
                and record.get("media_slot") == "media-1",
                timeout=10.0,
            )
            loaded_sequence = loaded.get("sequence")
            if not isinstance(loaded_sequence, int) or isinstance(
                loaded_sequence, bool
            ):
                raise HarnessFailure(
                    self.stage,
                    f"the {role} loop successor load had no sequence fence",
                )
            self._player_record(
                role,
                max(0, loaded_sequence - 1),
                lambda record: record.get("media_slot") == "media-1"
                and record.get("paused") is True
                and _safe_number(record.get("position_seconds")) is not None
                and abs(float(record["position_seconds"])) <= 0.75,
                timeout=8.0,
            )

        def loop_successor_authority_observed_twice() -> bool:
            return (
                len(
                    [
                        event
                        for event in self.observer.events_after(
                            successor_commit.sequence - 1
                        )
                        if event.fields.get("event") == "playstate"
                        and event.fields.get("paused") is True
                        and event.fields.get("transport_revision")
                        == successor_transport_revision
                        and _safe_number(event.fields.get("position_seconds"))
                        is not None
                        and abs(float(event.fields["position_seconds"])) <= 0.25
                    ]
                )
                >= 2
            )

        self._wait(
            "loop successor authority to remain stable across a server refresh",
            loop_successor_authority_observed_twice,
            timeout=3.0,
        )
        try:
            verified_successor_revision = validate_natural_eof_successor_boundary(
                (event.fields for event in self.observer.events_after(observer_cursor)),
                {
                    role: read_jsonl(self.clients[role].player_trace)[
                        player_cursors[role] :
                    ]
                    for role in roles
                },
                previous_transport_revision=initial_transport_revision,
                expected_playlist_index=0,
                predecessor_media_slot="media-2",
                successor_media_slot="media-1",
            )
        except ValueError as error:
            raise HarnessFailure(self.stage, str(error)) from error
        if verified_successor_revision != successor_transport_revision:
            raise HarnessFailure(
                self.stage,
                "loop successor transport evidence changed during verification",
            )
        self._loop_boundary_evidence = (
            observer_cursor,
            player_cursors,
            initial_transport_revision,
        )
        self._pass(
            "loop-final-item-advanced-once",
            "one natural final-item EOF committed exactly one canonical loop to item zero",
        )
        self._pass(
            "loop-successor-loaded-everywhere",
            "every loop-enabled managed real mpv loaded item zero at paused origin",
        )
        self._pass(
            "loop-successor-authority-reset",
            "completed final-item state could not cross the fresh loop successor revision",
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

        self.stage = "contained-player-failure-audit"
        contained_failures = contained_player_failure_counts(
            {
                role: self.artifact_dir / f"client-{role}.stderr.log"
                for role in self.clients
            }
        )
        if contained_failures:
            raise HarnessFailure(
                self.stage,
                f"packaged clients reported contained external player failures: {contained_failures}",
            )
        self._pass(
            "no-contained-player-failures",
            "no packaged client recovered past a failed external-player lifecycle step",
        )

        if self.loop_at_end_of_playlist:
            self.stage = "loop-successor-through-client-exit"
            if self._loop_boundary_evidence is None:
                raise HarnessFailure(
                    self.stage,
                    "loop boundary evidence was not retained for full-runtime verification",
                )
            (
                loop_observer_cursor,
                loop_player_cursors,
                initial_transport_revision,
            ) = self._loop_boundary_evidence
            validate_natural_eof_successor_boundary(
                (
                    event.fields
                    for event in self.observer.events_after(loop_observer_cursor)
                ),
                {
                    role: read_jsonl(self.clients[role].player_trace)[
                        loop_player_cursors[role] :
                    ]
                    for role in self.clients
                },
                previous_transport_revision=initial_transport_revision,
                expected_playlist_index=0,
                predecessor_media_slot="media-2",
                successor_media_slot="media-1",
            )
            self._pass(
                "loop-successor-stable-through-client-exit",
                "canonical loop selection, paused origin, and every physical successor remained bounded for the rest of the run",
            )
        else:
            self.stage = "final-item-terminal-through-client-exit"
            if self._terminal_boundary_evidence is None:
                raise HarnessFailure(
                    self.stage,
                    "terminal boundary evidence was not retained for full-runtime verification",
                )
            (
                terminal_observer_cursor,
                terminal_player_cursors,
                terminal_duration_seconds,
                initial_transport_revision,
            ) = self._terminal_boundary_evidence
            validate_terminal_playlist_boundary(
                (
                    event.fields
                    for event in self.observer.events_after(terminal_observer_cursor)
                ),
                {
                    role: read_jsonl(self.clients[role].player_trace)[
                        terminal_player_cursors[role] :
                    ]
                    for role in self.clients
                },
                terminal_duration_seconds=terminal_duration_seconds,
                initial_transport_revision=initial_transport_revision,
            )
            self._pass(
                "final-item-terminal-stable-through-client-exit",
                "canonical pause, terminal position, selection, and physical players remained bounded for the rest of the run",
            )

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
        artifacts: dict[str, Any] = {
            "causal_trace": self.trace_path.name,
            "player_traces": [f"player-{role}.jsonl" for role in sorted(self.clients)],
            "process_logs": [
                name for name in potential_process_logs if (self.artifact_dir / name).is_file()
            ],
        }
        if self.lifecycle_validation is not None:
            artifacts.update(
                {
                    "lifecycle_evidence": self.lifecycle_merged_path.name,
                    "lifecycle_evidence_summary": self.lifecycle_summary_path.name,
                    "lifecycle_process_ledgers": [
                        path.name for path in self.lifecycle_process_paths.values()
                    ],
                }
            )
        if self.fault_schedule_validated:
            artifacts.update(
                {
                    "fault_schedule": self.staged_fault_schedule_path.name,
                    "fault_replay": self.fault_replay_path.name,
                }
            )
        fault_schedule = None
        if self.fault_schedule is not None:
            fault_schedule = {
                "id": self.fault_schedule.schedule_id,
                "source_sha256": self.fault_schedule_source_digest,
                "staged_sha256": sha256_file(self.staged_fault_schedule_path),
                "replay_sha256": (
                    sha256_file(self.fault_replay_path)
                    if self.fault_schedule_validated
                    else None
                ),
                "step_count": len(self.fault_schedule.steps),
                "actions": sorted({step.action for step in self.fault_schedule.steps}),
            }
        return {
            "schema_version": SCHEMA_VERSION,
            "kind": REPORT_KIND,
            "result": result,
            "capability": "actual-server-multi-client-real-mpv",
            "playlist_policy": (
                "loop-at-end" if self.loop_at_end_of_playlist else "terminal-at-end"
            ),
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
            "fault_schedule": fault_schedule,
            "checks": self.checks,
            "artifacts": artifacts,
            "lifecycle_summary": self.lifecycle_validation,
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
        if self.lifecycle_writer is not None:
            try:
                self._stop_lifecycle_writer()
            except (OSError, ValueError):
                try:
                    self.lifecycle_writer.close()
                except OSError:
                    pass
                self.lifecycle_writer = None
        if self.fault_ledger is not None:
            try:
                self.fault_ledger.close()
            except OSError:
                pass
            self.fault_ledger = None

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
        try:
            self._start_fault_schedule()
            self._start_lifecycle_writer()
            self._emit(event="verification-started", detail="packaged multi-process lifecycle")
            self.preflight()
            first, second = self._create_fixtures()
            self._start_server()
            self._connect_observer_and_seed(first, second)
            self._verify_initial_players(first, second)
            self._verify_play_pause_seek()
            self._verify_scheduled_transport_fault_recovery()
            self._verify_late_join_and_status(first, second)
            self._verify_participant_status_loss_delay_stale_recovery()
            self._verify_untrusted_selection_rejected_and_restored(first, second)
            self._verify_same_index_selected_entry_replacement(first, second)
            self._verify_empty_playlist_clear_and_restore(first, second)
            self._verify_partitioned_follower_catches_up_to_missed_start()
            self._verify_scheduled_write_failure_recovery()
            self._verify_resume_eof_and_playlist()
            if self.loop_at_end_of_playlist:
                self._verify_loop_playlist_boundary()
            else:
                self._verify_terminal_playlist_boundary()
            self._verify_clean_shutdown_and_withdrawal()
            self._finish_fault_schedule()
            self._stop_lifecycle_writer()
            self._validate_lifecycle_evidence()
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
    run.add_argument("--ffmpeg", type=Path, required=True, help="exact FFmpeg fixture generator")
    run.add_argument("--artifact-dir", type=Path, required=True)
    run.add_argument(
        "--fault-schedule",
        type=Path,
        help="closed deterministic schedule to replay (defaults to the committed system schedule)",
    )
    run.add_argument("--candidate-sha", help="full git SHA represented by the candidate binaries")
    run.add_argument(
        "--allow-unverified-candidate",
        action="store_true",
        help="development only: run with a dirty or mismatched checkout and mark the evidence non-publishable",
    )
    run.add_argument(
        "--loop-at-end-of-playlist",
        action="store_true",
        help="require natural final-item EOF to commit one canonical last-to-first loop",
    )
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
            ffmpeg_path=args.ffmpeg,
            artifact_dir=args.artifact_dir,
            candidate_sha=args.candidate_sha,
            fault_schedule_path=args.fault_schedule,
            allow_unverified_candidate=args.allow_unverified_candidate,
            loop_at_end_of_playlist=args.loop_at_end_of_playlist,
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
