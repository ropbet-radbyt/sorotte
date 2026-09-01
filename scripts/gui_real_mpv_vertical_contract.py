#!/usr/bin/env python3
"""Fail-closed contract validation for the native GUI-to-real-mpv vertical lane."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-gui-real-mpv-vertical"
SUMMARY_KIND = "sorotte-gui-real-mpv-vertical-contract"
SESSION_EXCHANGE_KIND = "sorotte-gui-real-mpv-loopback-exchange"
MENU_INTERACTIONS_KIND = "sorotte-gui-real-mpv-menu-interactions"
RECOVERY_KIND = "sorotte-gui-real-mpv-owned-process-recovery"
HTTP_FAULT_KIND = "sorotte-gui-real-mpv-faulting-http-recovery"
HTTP_FAULT_ROUTE = "/generated-fault.au"
MEDIA_FAILURE_KIND = "sorotte-gui-real-mpv-media-failure-recovery"
MEDIA_FAILURE_ROUTE = "/hard-media-failure.au"
HTTP_FAULT_DURATION_SECONDS = 45
HTTP_FAULT_DISCONNECT_AFTER_BYTES = 720_000
HTTP_STALL_KIND = "sorotte-gui-real-mpv-stalled-http"
HTTP_STALL_ROUTE = "/generated-stall.au"
HTTP_STALL_DURATION_SECONDS = 45
HTTP_STALL_PREFIX_BYTES = 720_000
HTTP_STALL_PREFIX_BYTES_PER_SECOND = 350_000
HTTP_STALL_MINIMUM_DURATION_MS = 25_000
HTTP_STALL_MAXIMUM_RECOVERY_WAIT_MS = 50_000
HTTP_STALL_AU_HEADER_BYTES = 24
HTTP_STALL_PCM_BYTES_PER_SECOND = 48_000 * 2
HTTP_STALL_EXPECTED_PREFIX_PLAYABLE_SECONDS = (
    HTTP_STALL_PREFIX_BYTES - HTTP_STALL_AU_HEADER_BYTES
) / HTTP_STALL_PCM_BYTES_PER_SECOND
HTTP_STALL_POSITION_TOLERANCE_SECONDS = 0.25
SESSION_CAPABILITIES = ("chat", "readiness", "sharedPlaylists")
SESSION_EXCHANGE_KEYS = {
    "schema_version",
    "kind",
    "result",
    "bound_endpoint",
    "connected_peer_endpoint",
    "listener_ipv4_loopback",
    "peer_ipv4_loopback",
    "client_hello",
    "server_hello",
    "advertised_capabilities",
    "playlist_change_request",
    "playlist_change_echo",
    "playlist_index_request",
    "playlist_index_echo",
    "initial_authoritative_playstate",
    "playstate_exchanges",
    "server_thread_released",
    "socket_released",
    "error",
}
PLAYSTATE_EXCHANGE_KEYS = {
    "action",
    "mutation_kind",
    "expected_paused",
    "request",
    "authoritative_echo",
}
EXPECTED_CLIENT_HELLO = {
    "Hello": {
        "username": "real-mpv-user",
        "room": {"name": "real-mpv-room"},
        "version": "1.2.255",
        "realversion": "1.7.5",
        "features": {
            "chat": True,
            "featureList": True,
            "managedRooms": True,
            "mediaMatch": True,
            "persistentRooms": True,
            "readiness": True,
            "setOthersReadiness": True,
            "sharedPlaylists": True,
            "sorotteParticipantStatusV1": True,
            "sorottePlaybackBarrierV1": True,
            "sorottePlexPlaylistUris": True,
            "sorotteReadinessV2": True,
            "uiMode": "GUI",
        },
    }
}
EXPECTED_HTTP_FAULT_CLIENT_HELLO = {
    "Hello": {
        **EXPECTED_CLIENT_HELLO["Hello"],
        "features": {
            **EXPECTED_CLIENT_HELLO["Hello"]["features"],
            "sharedPlaylists": True,
        },
    }
}
EXPECTED_SERVER_HELLO = {
    "Hello": {
        "username": "real-mpv-user",
        "room": {"name": "real-mpv-room"},
        "version": "1.7.5",
        "features": {
            "chat": True,
            "readiness": True,
            "sharedPlaylists": True,
        },
    }
}
REQUIRED_ASSERTIONS = (
    "supported-mpv-version-and-digest",
    "isolated-config-and-generated-local-media",
    "actual-native-gui-window",
    "loopback-session-bound-to-local-gui",
    "native-file-menu-open-media",
    "gui-owned-exact-mpv-loaded-generated-media",
    "gui-projected-real-mpv-transport-ready",
    "gui-play-command-observed-by-real-mpv",
    "gui-projected-playing-after-real-mpv-observation",
    "gui-pause-command-observed-by-real-mpv",
    "gui-projected-paused-after-real-mpv-observation",
    "native-success-screenshot",
    "gui-exit-reaped-owned-mpv",
)
RECOVERY_REQUIRED_ASSERTIONS = (
    *REQUIRED_ASSERTIONS[:-2],
    "exact-attested-owned-mpv-terminated",
    "automatic-relaunch-distinct-owned-exact-mpv",
    "gui-remained-on-active-room-during-automatic-relaunch",
    "replacement-mpv-loaded-generated-media",
    "gui-play-command-observed-by-replacement-mpv",
    "gui-pause-command-observed-by-replacement-mpv",
    "replacement-transport-recovered-with-old-mpv-fenced",
    "native-success-screenshot",
    "gui-exit-reaped-replacement-owned-mpv",
)
HTTP_FAULT_REQUIRED_ASSERTIONS = (
    "supported-mpv-version-and-digest",
    "isolated-config-and-generated-local-media",
    "strict-loopback-faulting-http-ready",
    "actual-native-gui-window",
    "loopback-session-bound-to-local-gui",
    "native-file-menu-open-media",
    "gui-owned-exact-mpv-loaded-generated-media",
    "gui-projected-real-mpv-transport-ready",
    "gui-play-command-observed-by-real-mpv",
    "gui-projected-playing-after-real-mpv-observation",
    "one-malformed-http-premature-eof-observed",
    "same-owned-mpv-reloaded-stable-network-media",
    "recovered-playback-advanced-past-fault",
    "gui-pause-command-observed-by-real-mpv",
    "gui-projected-paused-after-real-mpv-observation",
    "fault-evidence-retained-before-cleanup",
    "authoritative-http-404-produced-media-failure",
    "same-owned-mpv-recovered-from-hard-media-failure",
    "hard-media-failure-evidence-retained",
    "native-success-screenshot",
    "gui-exit-reaped-owned-mpv-and-released-fault-servers",
)
HTTP_STALL_REQUIRED_ASSERTIONS = (
    "supported-mpv-version-and-digest",
    "isolated-config-and-generated-local-media",
    "strict-loopback-stalled-http-ready",
    "actual-native-gui-window",
    "loopback-session-bound-to-local-gui",
    "native-file-menu-open-media",
    "gui-owned-exact-mpv-loaded-generated-media",
    "gui-projected-real-mpv-transport-ready",
    "gui-play-command-observed-by-real-mpv",
    "gui-projected-playing-after-real-mpv-observation",
    "sustained-valid-http-cache-stall-observed",
    "same-owned-mpv-reloaded-after-bounded-cache-stall",
    "recovered-playback-advanced-past-stall",
    "gui-pause-command-observed-by-real-mpv",
    "gui-projected-paused-after-real-mpv-observation",
    "stall-evidence-retained-before-cleanup",
    "native-success-screenshot",
    "gui-exit-reaped-owned-mpv-and-released-stall-server",
)
REQUIRED_ARTIFACTS = (
    "config",
    "generated_media",
    "observation_script",
    "mpv_observation",
    "mpv_log",
    "gui_lifecycle",
    "shared_lifecycle",
    "session_exchange",
    "menu_interactions",
    "success_screenshot",
    "state",
)
RECOVERY_REQUIRED_ARTIFACTS = (
    *REQUIRED_ARTIFACTS,
    "owned_mpv_recovery",
    "automatic_relaunch_screenshot",
    "recovery_screenshot",
)
HTTP_FAULT_REQUIRED_ARTIFACTS = (
    *REQUIRED_ARTIFACTS,
    "faulting_http_recovery",
    "hard_media_failure",
)
HTTP_STALL_REQUIRED_ARTIFACTS = (
    *REQUIRED_ARTIFACTS,
    "stalled_http",
)
RECOVERY_KEYS = {
    "schema_version",
    "kind",
    "result",
    "fault",
    "recovery_mode",
    "automatic_relaunch_timeout_ms",
    "initial_pid",
    "initial_parent_pid",
    "initial_process_image_path",
    "initial_sha256",
    "initial_ipc_endpoint",
    "initial_process_terminated",
    "automatic_relaunch_observation_index",
    "automatic_relaunch_observation_event",
    "gui_room_remained_active",
    "manual_retry_invoked",
    "recovered_pid",
    "recovered_parent_pid",
    "recovered_process_image_path",
    "recovered_sha256",
    "recovered_ipc_endpoint",
    "distinct_pid",
    "distinct_ipc_endpoint",
    "post_termination_observation_index",
    "recovered_file_loaded_index",
    "recovered_playing_index",
    "recovered_paused_index",
    "initial_process_still_terminated_after_recovery",
    "initial_process_still_terminated_after_gui_exit",
    "recovered_process_terminated_after_gui_exit",
    "error",
}
HTTP_FAULT_KEYS = {
    "schema_version",
    "kind",
    "result",
    "fault",
    "recovery_mode",
    "listener_endpoint",
    "listener_ipv4_loopback",
    "media_url",
    "route",
    "generated_media_bytes",
    "generated_media_sha256",
    "duration_seconds",
    "minimum_body_bytes_before_fault",
    "request_count",
    "premature_disconnect_count",
    "complete_response_count",
    "requests",
    "initial_file_loaded_index",
    "pre_fault_progress_index",
    "fault_triggered_after_progress",
    "premature_eof_index",
    "recovered_file_loaded_index",
    "recovered_progress_index",
    "recovered_paused_index",
    "initial_pid",
    "recovered_pid",
    "parent_pid",
    "process_image_path",
    "process_sha256",
    "initial_ipc_endpoint",
    "recovered_ipc_endpoint",
    "stable_process_identity",
    "stable_ipc_endpoint",
    "stable_media_url",
    "stable_duration",
    "pre_fault_position_seconds",
    "premature_eof_position_seconds",
    "recovered_position_seconds",
    "manual_retry_invoked",
    "foreign_pid_observations_after_fault",
    "evidence_retained_before_cleanup",
    "server_thread_released",
    "socket_released",
    "owned_mpv_terminated_after_gui_exit",
    "error",
}
HTTP_REQUEST_KEYS = {
    "ordinal",
    "method",
    "path",
    "peer_endpoint",
    "peer_ipv4_loopback",
    "range_header",
    "status_code",
    "content_length_header",
    "transfer_encoding",
    "transmitted_body_bytes",
    "framing_fault_injected",
    "disconnected_early",
    "write_error",
}
MEDIA_FAILURE_KEYS = {
    "schema_version",
    "kind",
    "result",
    "failure_mode",
    "recovery_mode",
    "listener_endpoint",
    "listener_ipv4_loopback",
    "media_url",
    "route",
    "request_count",
    "requests",
    "failure_end_file_index",
    "failure_reason",
    "media_fail_event_id",
    "media_fail_emitter",
    "media_fail_process_role",
    "restored_file_loaded_index",
    "media_playable_event_id",
    "media_playable_emitter",
    "media_playable_process_role",
    "initial_pid",
    "failure_pid",
    "recovered_pid",
    "parent_pid",
    "process_image_path",
    "process_sha256",
    "initial_ipc_endpoint",
    "failure_ipc_endpoint",
    "recovered_ipc_endpoint",
    "same_process_identity",
    "same_ipc_endpoint",
    "restored_media_path",
    "restored_media_sha256",
    "manual_retry_invoked",
    "evidence_retained_before_cleanup",
    "server_thread_released",
    "socket_released",
    "owned_mpv_terminated_after_gui_exit",
    "error",
}
HTTP_STALL_KEYS = {
    "schema_version",
    "kind",
    "result",
    "schedule",
    "expected_outcome",
    "listener_endpoint",
    "listener_ipv4_loopback",
    "media_url",
    "route",
    "generated_media_bytes",
    "generated_media_sha256",
    "duration_seconds",
    "prefix_body_bytes",
    "prefix_bytes_per_second",
    "expected_prefix_playable_seconds",
    "cache_stall_position_tolerance_seconds",
    "minimum_stall_duration_ms",
    "maximum_recovery_wait_ms",
    "request_count",
    "stalled_response_count",
    "complete_response_count",
    "requests",
    "initial_file_loaded_index",
    "pre_stall_progress_index",
    "cache_stall_index",
    "recovered_file_loaded_index",
    "recovered_progress_index",
    "recovered_paused_index",
    "initial_pid",
    "recovered_pid",
    "parent_pid",
    "process_image_path",
    "process_sha256",
    "initial_ipc_endpoint",
    "recovered_ipc_endpoint",
    "stable_process_identity",
    "stable_ipc_endpoint",
    "stable_media_url",
    "stable_duration",
    "pre_stall_position_seconds",
    "cache_stall_position_seconds",
    "recovered_position_seconds",
    "eof_observations_before_recovery",
    "end_file_observations_before_recovery",
    "manual_retry_invoked",
    "foreign_pid_observations_after_stall",
    "evidence_retained_before_cleanup",
    "server_thread_released",
    "socket_released",
    "owned_mpv_terminated_after_gui_exit",
    "error",
}
HTTP_STALL_REQUEST_KEYS = {
    "ordinal",
    "method",
    "path",
    "peer_endpoint",
    "peer_ipv4_loopback",
    "range_header",
    "status_code",
    "content_length_header",
    "transfer_encoding",
    "transmitted_body_bytes",
    "stall_injected",
    "stalled_for_ms",
    "server_response_retained_at_recovery_get",
    "connection_released",
    "response_completed",
    "write_error",
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def lifecycle_summary_binding(path: Path) -> tuple[str, list[str]]:
    summary = load_json(path, "shared lifecycle summary")
    require(
        summary.get("schema_version") == 1
        and summary.get("kind") == "sorotte-playback-lifecycle-evidence-validation"
        and summary.get("result") == "passed",
        "shared lifecycle summary is not a validated pass",
    )
    transitions = summary.get("transitions")
    require(
        isinstance(transitions, dict) and bool(transitions),
        "shared lifecycle summary has no transition inventory",
    )
    for transition_id, count in transitions.items():
        require(
            isinstance(transition_id, str)
            and transition_id
            and is_json_integer(count)
            and count > 0,
            "shared lifecycle summary transition inventory is malformed",
        )
    return sha256_file(path), sorted(transitions)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def is_json_integer(value: Any) -> bool:
    return type(value) is int


def is_json_number(value: Any) -> bool:
    return type(value) in (int, float) and math.isfinite(float(value))


def normalized_resolved_path(value: Any) -> Path:
    text = str(value)
    if os.name == "nt" and text.startswith("\\\\?\\UNC\\"):
        text = "\\\\" + text[len("\\\\?\\UNC\\") :]
    elif os.name == "nt" and text.startswith("\\\\?\\"):
        text = text[len("\\\\?\\") :]
    path = Path(text)
    require(path.is_absolute(), f"path must be absolute: {value!r}")
    return path.resolve()


def resolved_child(root: Path, relative_path: str, label: str) -> Path:
    require(relative_path != "", f"{label} path must not be empty")
    candidate = (root / relative_path).resolve()
    require(candidate.is_relative_to(root), f"{label} escaped artifact root: {relative_path}")
    return candidate


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"failed to load {label} {path}: {error}") from error
    require(isinstance(payload, dict), f"{label} must be a JSON object")
    return payload


def validate_ipv4_loopback_endpoint(value: Any, label: str) -> tuple[str, int]:
    host, separator, port = str(value).rpartition(":")
    require(
        separator == ":" and host == "127.0.0.1",
        f"{label} was not bound to IPv4 loopback",
    )
    require(port.isdigit() and 0 < int(port) <= 65535, f"{label} port was invalid")
    return host, int(port)


def validate_binary_identity(
    identity: Any,
    expected_path: Path,
    expected_sha256: str,
    label: str,
) -> None:
    require(isinstance(identity, dict), f"{label} identity must be an object")
    reported_path = normalized_resolved_path(identity.get("path", ""))
    require(reported_path == expected_path, f"{label} path mismatch")
    require(identity.get("sha256") == expected_sha256, f"{label} digest mismatch")
    require(identity.get("bytes") == expected_path.stat().st_size, f"{label} size mismatch")
    require(sha256_file(expected_path) == expected_sha256, f"{label} bytes changed")


def validate_playstate_exchange(
    row: Any,
    *,
    expected_action: str,
    expected_paused: bool,
    expected_mutation_kind: str,
) -> None:
    require(isinstance(row, dict), "session playstate exchange must be an object")
    require(
        set(row) == PLAYSTATE_EXCHANGE_KEYS,
        "session playstate exchange field inventory drifted",
    )
    require(row.get("action") == expected_action, "session playstate action order drifted")
    require(
        row.get("mutation_kind") == expected_mutation_kind,
        f"{expected_action} mutation kind drifted",
    )
    require(
        row.get("expected_paused") is expected_paused,
        f"{expected_action} expected pause level drifted",
    )
    try:
        request = json.loads(str(row.get("request", "")))
        echo = json.loads(str(row.get("authoritative_echo", "")))
    except json.JSONDecodeError as error:
        raise ValueError(
            f"{expected_action} request/echo evidence was invalid JSON: {error}"
        ) from error

    require(
        isinstance(request, dict) and set(request) == {"State"},
        f"{expected_action} request envelope drifted",
    )
    request_state = request.get("State")
    expected_state_keys = (
        {"playstate"} if expected_mutation_kind == "seek" else {"playstate", "ping"}
    )
    require(
        isinstance(request_state, dict) and set(request_state) == expected_state_keys,
        f"{expected_action} request State schema drifted",
    )
    request_playstate = request_state.get("playstate")
    request_ping = request_state.get("ping")
    expected_playstate_keys = (
        {"position", "paused", "doSeek"}
        if expected_mutation_kind == "seek"
        else {"position", "paused"}
    )
    require(
        isinstance(request_playstate, dict)
        and set(request_playstate) == expected_playstate_keys,
        f"{expected_action} request playstate schema drifted",
    )
    require(
        request_playstate.get("paused") is expected_paused
        and request_playstate.get("doSeek", False)
        is (expected_mutation_kind == "seek")
        and is_json_number(request_playstate.get("position"))
        and float(request_playstate["position"]) >= 0.0,
        f"{expected_action} request pause or position was invalid",
    )
    if expected_mutation_kind == "pause":
        require(
            isinstance(request_ping, dict)
            and set(request_ping) == {"clientLatencyCalculation", "clientRtt"}
            and is_json_number(request_ping.get("clientLatencyCalculation"))
            and is_json_number(request_ping.get("clientRtt"))
            and float(request_ping["clientRtt"]) >= 0.0,
            f"{expected_action} request ping schema drifted",
        )

    require(
        isinstance(echo, dict) and set(echo) == {"State"},
        f"{expected_action} authoritative echo envelope drifted",
    )
    echo_state = echo.get("State")
    require(
        isinstance(echo_state, dict) and set(echo_state) == {"playstate"},
        f"{expected_action} authoritative echo State schema drifted",
    )
    echo_playstate = echo_state.get("playstate")
    require(
        isinstance(echo_playstate, dict)
        and set(echo_playstate) == {"doSeek", "paused", "position", "setBy"},
        f"{expected_action} authoritative echo playstate schema drifted",
    )
    require(
        echo_playstate.get("paused") is expected_paused
        and echo_playstate.get("doSeek") is (expected_mutation_kind == "seek")
        and echo_playstate.get("setBy") == "real-mpv-user"
        and is_json_number(echo_playstate.get("position"))
        and float(echo_playstate["position"])
        == float(request_playstate["position"]),
        f"{expected_action} authoritative echo did not authenticate the exact mutation",
    )


def validate_observations(
    path: Path,
    expected_media: Path,
    expected_mpv_pid: int,
    expected_media_url: str | None = None,
) -> list[dict[str, Any]]:
    observations: list[dict[str, Any]] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if not line.strip():
            continue
        try:
            payload = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(
                f"mpv observation line {line_number} was invalid JSON: {error}"
            ) from error
        require(isinstance(payload, dict), "mpv observation must be an object")
        observations.append(payload)
    require(observations, "mpv observation artifact was empty")

    def media_path_matches(value: Any) -> bool:
        if expected_media_url is not None:
            return value == expected_media_url
        return normalized_resolved_path(value) == expected_media

    file_loaded_index = next(
        (
            index
            for index, item in enumerate(observations)
            if item.get("event") == "file-loaded"
            and item.get("pid") == expected_mpv_pid
            and media_path_matches(item.get("path", ""))
        ),
        None,
    )
    require(file_loaded_index is not None, "missing exact real-mpv file-loaded observation")
    playing_index = next(
        (
            index
            for index, item in enumerate(observations)
            if index > file_loaded_index
            and item.get("event") == "pause"
            and item.get("pid") == expected_mpv_pid
            and item.get("pause") is False
        ),
        None,
    )
    require(playing_index is not None, "missing real-mpv pause=false observation after load")
    paused_index = next(
        (
            index
            for index, item in enumerate(observations)
            if index > playing_index
            and item.get("event") == "pause"
            and item.get("pid") == expected_mpv_pid
            and item.get("pause") is True
        ),
        None,
    )
    require(paused_index is not None, "missing real-mpv pause=true observation after Play")
    return observations


def validate_http_fault_evidence(
    evidence: Any,
    *,
    artifact_evidence: dict[str, Any],
    observations: list[dict[str, Any]],
    expected_media: Path,
    expected_media_url: str,
    expected_mpv: Path,
    expected_mpv_sha256: str,
    mpv: dict[str, Any],
    ipc_endpoint: str,
) -> None:
    require(isinstance(evidence, dict), "HTTP fault contract missing")
    require(set(evidence) == HTTP_FAULT_KEYS, "HTTP fault field inventory drifted")
    require(artifact_evidence == evidence, "report/artifact HTTP fault evidence diverged")
    require(evidence.get("schema_version") == SCHEMA_VERSION, "HTTP fault schema mismatch")
    require(evidence.get("kind") == HTTP_FAULT_KIND, "HTTP fault kind mismatch")
    require(evidence.get("result") == "passed", "HTTP fault recovery did not pass")
    require(
        evidence.get("fault")
        == "first-response-malformed-chunk-after-observed-progress-and-playable-prefix-once",
        "HTTP fault shape drifted",
    )
    require(
        evidence.get("fault_triggered_after_progress") is True,
        "HTTP fault was not causally released after observed playback progress",
    )
    require(
        evidence.get("recovery_mode")
        == "same-generation-automatic-network-stream-reload",
        "HTTP recovery mode drifted",
    )
    listener_endpoint = evidence.get("listener_endpoint")
    validate_ipv4_loopback_endpoint(listener_endpoint, "recorded HTTP listener")
    require(
        evidence.get("listener_ipv4_loopback") is True,
        "HTTP listener loopback attestation missing",
    )
    require(
        evidence.get("media_url") == expected_media_url
        == f"http://{listener_endpoint}{HTTP_FAULT_ROUTE}",
        "native Open Media did not deliver the exact strict loopback URL",
    )
    require(evidence.get("route") == HTTP_FAULT_ROUTE, "HTTP route drifted")
    require(
        evidence.get("generated_media_bytes") == expected_media.stat().st_size,
        "HTTP generated-media size mismatch",
    )
    require(
        evidence.get("generated_media_sha256") == sha256_file(expected_media),
        "HTTP generated-media digest mismatch",
    )
    require(
        evidence.get("duration_seconds") == HTTP_FAULT_DURATION_SECONDS,
        "HTTP media duration contract drifted",
    )
    require(
        evidence.get("minimum_body_bytes_before_fault")
        == HTTP_FAULT_DISCONNECT_AFTER_BYTES,
        "HTTP minimum playable-prefix boundary drifted",
    )
    require(
        evidence.get("initial_pid") == mpv["pid"]
        and evidence.get("recovered_pid") == mpv["pid"]
        and evidence.get("parent_pid") == mpv["parent_pid"],
        "HTTP recovery changed the attested GUI-owned mpv process",
    )
    require(
        normalized_resolved_path(evidence.get("process_image_path", "")) == expected_mpv
        and evidence.get("process_sha256") == expected_mpv_sha256,
        "HTTP recovery process image identity drifted",
    )
    require(
        evidence.get("initial_ipc_endpoint") == ipc_endpoint
        and evidence.get("recovered_ipc_endpoint") == ipc_endpoint,
        "HTTP recovery changed the managed mpv IPC endpoint",
    )
    require(
        all(
            evidence.get(key) is True
            for key in (
                "stable_process_identity",
                "stable_ipc_endpoint",
                "stable_media_url",
                "stable_duration",
                "evidence_retained_before_cleanup",
                "server_thread_released",
                "socket_released",
                "owned_mpv_terminated_after_gui_exit",
            )
        ),
        "HTTP recovery identity, retention, release, or cleanup attestation was incomplete",
    )
    require(
        evidence.get("manual_retry_invoked") is False,
        "HTTP recovery unexpectedly used a manual retry",
    )
    require(
        evidence.get("foreign_pid_observations_after_fault") == 0,
        "stale or foreign mpv generation was observed after the HTTP fault",
    )
    require(evidence.get("error") is None, "HTTP fault evidence retained an error")

    indices = [
        evidence.get("initial_file_loaded_index"),
        evidence.get("pre_fault_progress_index"),
        evidence.get("premature_eof_index"),
        evidence.get("recovered_file_loaded_index"),
        evidence.get("recovered_progress_index"),
        evidence.get("recovered_paused_index"),
    ]
    require(
        all(is_json_integer(index) and index >= 0 for index in indices),
        "HTTP recovery observation indices were invalid",
    )
    (
        initial_index,
        pre_fault_progress_index,
        end_index,
        recovered_index,
        progress_index,
        paused_index,
    ) = indices
    require(
        initial_index
        < pre_fault_progress_index
        < end_index
        < recovered_index
        < progress_index
        < paused_index
        < len(observations),
        "HTTP recovery observation ordering or bounds drifted",
    )
    require(
        all(
            (item.get("pid") in (None, mpv["pid"]))
            and (item.get("ipc_endpoint") in (None, ipc_endpoint))
            for item in observations[end_index : paused_index + 1]
        ),
        "stale or foreign mpv generation appeared after the HTTP fault boundary",
    )
    initial = observations[initial_index]
    require(
        initial.get("event") == "file-loaded"
        and initial.get("pid") == mpv["pid"]
        and initial.get("ipc_endpoint") == ipc_endpoint
        and initial.get("path") == expected_media_url
        and initial.get("filename") == HTTP_FAULT_ROUTE.rsplit("/", 1)[-1]
        and abs(float(initial.get("duration")) - HTTP_FAULT_DURATION_SECONDS) <= 0.05,
        "initial native HTTP file-loaded identity drifted or used a cache path",
    )
    premature_eof = observations[end_index]
    require(
        premature_eof.get("event") == "eof-reached"
        and premature_eof.get("eof_reached") is True
        and premature_eof.get("pid") == mpv["pid"]
        and premature_eof.get("ipc_endpoint") == ipc_endpoint
        and premature_eof.get("path") == expected_media_url
        and abs(float(premature_eof.get("duration")) - HTTP_FAULT_DURATION_SECONDS)
        <= 0.05,
        "malformed HTTP response did not cause the expected keep-open premature EOF",
    )
    recovered = observations[recovered_index]
    require(
        recovered.get("event") == "file-loaded"
        and recovered.get("pid") == mpv["pid"]
        and recovered.get("ipc_endpoint") == ipc_endpoint
        and recovered.get("path") == expected_media_url
        and recovered.get("filename") == HTTP_FAULT_ROUTE.rsplit("/", 1)[-1]
        and abs(float(recovered.get("duration")) - HTTP_FAULT_DURATION_SECONDS) <= 0.05,
        "recovered native HTTP media identity drifted or used a cache path",
    )
    pre_fault_position = evidence.get("pre_fault_position_seconds")
    premature_eof_position = evidence.get("premature_eof_position_seconds")
    recovered_position = evidence.get("recovered_position_seconds")
    require(
        is_json_number(pre_fault_position)
        and pre_fault_position >= 0.5
        and is_json_number(premature_eof_position)
        and premature_eof_position >= pre_fault_position
        and HTTP_FAULT_DURATION_SECONDS - premature_eof_position > 15.0
        and is_json_number(recovered_position)
        and recovered_position >= pre_fault_position + 0.5,
        "HTTP recovery did not retain bounded positive playback progress",
    )
    require(
        is_json_number(premature_eof.get("position"))
        and float(premature_eof["position"]) == float(premature_eof_position),
        "HTTP premature EOF position observation mismatch",
    )
    pre_fault_progress = observations[pre_fault_progress_index]
    require(
        pre_fault_progress.get("event") == "time-pos"
        and pre_fault_progress.get("pid") == mpv["pid"]
        and pre_fault_progress.get("ipc_endpoint") == ipc_endpoint
        and pre_fault_progress.get("path") == expected_media_url
        and is_json_number(pre_fault_progress.get("position"))
        and float(pre_fault_progress["position"]) == float(pre_fault_position),
        "HTTP pre-fault progress observation mismatch",
    )
    recovered_progress = observations[progress_index]
    require(
        recovered_progress.get("event") == "time-pos"
        and recovered_progress.get("pid") == mpv["pid"]
        and recovered_progress.get("ipc_endpoint") == ipc_endpoint
        and recovered_progress.get("path") == expected_media_url
        and is_json_number(recovered_progress.get("position"))
        and float(recovered_progress["position"]) == float(recovered_position),
        "HTTP recovered progress observation mismatch",
    )
    recovered_pause = observations[paused_index]
    require(
        recovered_pause.get("event") == "pause"
        and recovered_pause.get("pause") is True
        and recovered_pause.get("pid") == mpv["pid"]
        and recovered_pause.get("ipc_endpoint") == ipc_endpoint
        and recovered_pause.get("path") == expected_media_url,
        "HTTP recovered pause observation mismatch",
    )
    requests = evidence.get("requests")
    require(isinstance(requests, list) and requests, "HTTP request accounting was empty")
    require(
        evidence.get("request_count") == len(requests),
        "HTTP request count diverged from retained rows",
    )
    for index, request in enumerate(requests, start=1):
        require(isinstance(request, dict), "HTTP request row must be an object")
        require(set(request) == HTTP_REQUEST_KEYS, "HTTP request field inventory drifted")
        require(request.get("ordinal") == index, "HTTP request ordinal drifted")
        require(request.get("path") == HTTP_FAULT_ROUTE, "HTTP request path drifted")
        require(request.get("method") in {"HEAD", "GET"}, "HTTP method was unaccounted")
        validate_ipv4_loopback_endpoint(
            request.get("peer_endpoint"), "HTTP request peer endpoint"
        )
        require(
            request.get("peer_ipv4_loopback") is True,
            "HTTP request ownership accounting drifted",
        )
        require(
            request.get("range_header") is None
            or isinstance(request.get("range_header"), str),
            "HTTP Range accounting was malformed",
        )
        require(
            request.get("write_error") is None,
            "HTTP response write did not complete cleanly",
        )
        if request["method"] == "HEAD":
            require(
                request.get("status_code") == 200
                and request.get("content_length_header")
                == evidence["generated_media_bytes"]
                and request.get("transfer_encoding") is None
                and request.get("transmitted_body_bytes") == 0
                and request.get("framing_fault_injected") is False
                and request.get("disconnected_early") is False,
                "HTTP HEAD probe unexpectedly carried the controlled body fault",
            )
        else:
            require(
                request.get("status_code") == 200
                and request.get("range_header") == "bytes=0-",
                "HTTP media GET did not use the exact byte-zero non-seekable contract",
            )

    media_gets = [request for request in requests if request["method"] == "GET"]
    require(
        len(media_gets) == 2,
        "HTTP recovery did not use exactly one malformed chunked GET and one complete GET",
    )
    first_get, recovered_get = media_gets
    require(
        first_get.get("disconnected_early") is True
        and first_get.get("content_length_header") is None
        and first_get.get("transfer_encoding") == "chunked"
        and first_get.get("transmitted_body_bytes")
        >= HTTP_FAULT_DISCONNECT_AFTER_BYTES
        < evidence["generated_media_bytes"]
        and first_get.get("framing_fault_injected") is True,
        "first HTTP GET was not the exact malformed chunked response",
    )
    require(
        recovered_get.get("disconnected_early") is False
        and recovered_get.get("content_length_header")
        == evidence["generated_media_bytes"]
        and recovered_get.get("transfer_encoding") is None
        and recovered_get.get("framing_fault_injected") is False
        and recovered_get.get("transmitted_body_bytes")
        == recovered_get.get("content_length_header"),
        "second HTTP GET was not a complete recovery response",
    )
    require(
        evidence.get("premature_disconnect_count") == 1
        and evidence.get("complete_response_count") == 1,
        "HTTP response aggregate accounting drifted",
    )


def validate_media_failure_evidence(
    evidence: Any,
    *,
    artifact_evidence: dict[str, Any],
    observations: list[dict[str, Any]],
    expected_media: Path,
    expected_mpv: Path,
    expected_mpv_sha256: str,
    mpv: dict[str, Any],
    ipc_endpoint: str,
) -> None:
    require(isinstance(evidence, dict), "hard media-failure contract missing")
    require(
        set(evidence) == MEDIA_FAILURE_KEYS,
        "hard media-failure field inventory drifted",
    )
    require(
        artifact_evidence == evidence,
        "report/artifact hard media-failure evidence diverged",
    )
    require(
        evidence.get("schema_version") == SCHEMA_VERSION,
        "hard media-failure schema mismatch",
    )
    require(evidence.get("kind") == MEDIA_FAILURE_KIND, "hard media-failure kind mismatch")
    require(evidence.get("result") == "passed", "hard media-failure recovery did not pass")
    require(
        evidence.get("failure_mode") == "authoritative-loopback-http-404",
        "hard media-failure mode drifted",
    )
    require(
        evidence.get("recovery_mode") == "authoritative-local-media-restore",
        "hard media-failure recovery mode drifted",
    )
    listener_endpoint = evidence.get("listener_endpoint")
    validate_ipv4_loopback_endpoint(listener_endpoint, "hard media-failure HTTP listener")
    require(
        evidence.get("listener_ipv4_loopback") is True,
        "hard media-failure listener loopback attestation missing",
    )
    media_url = f"http://{listener_endpoint}{MEDIA_FAILURE_ROUTE}"
    require(evidence.get("media_url") == media_url, "hard media-failure URL drifted")
    require(evidence.get("route") == MEDIA_FAILURE_ROUTE, "hard media-failure route drifted")

    requests = evidence.get("requests")
    require(isinstance(requests, list) and len(requests) >= 1, "hard media-failure request evidence missing")
    require(
        evidence.get("request_count") == len(requests),
        "hard media-failure request count drifted",
    )
    require(
        any(request.get("method") == "GET" for request in requests if isinstance(request, dict)),
        "hard media-failure evidence omitted a media GET",
    )
    for index, request in enumerate(requests):
        require(isinstance(request, dict), f"hard media-failure request {index} was not an object")
        require(set(request) == HTTP_REQUEST_KEYS, "hard media-failure request fields drifted")
        require(request.get("ordinal") == index + 1, "hard media-failure request ordering drifted")
        require(request.get("method") in ("GET", "HEAD"), "hard media-failure method drifted")
        require(request.get("path") == MEDIA_FAILURE_ROUTE, "hard media-failure request route drifted")
        validate_ipv4_loopback_endpoint(
            request.get("peer_endpoint"), "hard media-failure request peer"
        )
        require(
            request.get("peer_ipv4_loopback") is True
            and request.get("status_code") == 404
            and request.get("content_length_header") == 0
            and request.get("transfer_encoding") is None
            and request.get("transmitted_body_bytes") == 0
            and request.get("framing_fault_injected") is False
            and request.get("disconnected_early") is False
            and request.get("write_error") is None,
            "hard media-failure request did not retain the exact bodyless 404 contract",
        )
        require(
            request.get("range_header") is None
            or isinstance(request.get("range_header"), str),
            "hard media-failure Range accounting was malformed",
        )

    failure_index = evidence.get("failure_end_file_index")
    restored_index = evidence.get("restored_file_loaded_index")
    require(
        is_json_integer(failure_index)
        and is_json_integer(restored_index)
        and 0 <= failure_index < restored_index < len(observations),
        "hard media-failure observation ordering or bounds drifted",
    )
    failure = observations[failure_index]
    restored = observations[restored_index]
    require(
        failure.get("event") == "end-file"
        and failure.get("reason") == evidence.get("failure_reason") == "error"
        and failure.get("pid") == mpv["pid"]
        and failure.get("ipc_endpoint") == ipc_endpoint
        and failure.get("path") in (None, media_url),
        "hard media-failure did not retain the same-process end-file error boundary",
    )
    require(
        restored.get("event") == "file-loaded"
        and restored.get("pid") == mpv["pid"]
        and restored.get("ipc_endpoint") == ipc_endpoint
        and normalized_resolved_path(restored.get("path", "")) == expected_media,
        "hard media-failure recovery did not load the generated local media",
    )
    require(
        all(
            observation.get("pid") in (None, mpv["pid"])
            and observation.get("ipc_endpoint") in (None, ipc_endpoint)
            for observation in observations[failure_index : restored_index + 1]
        ),
        "stale or foreign mpv generation appeared across hard media-failure recovery",
    )
    require(
        evidence.get("initial_pid") == mpv["pid"]
        and evidence.get("failure_pid") == mpv["pid"]
        and evidence.get("recovered_pid") == mpv["pid"]
        and evidence.get("parent_pid") == mpv["parent_pid"],
        "hard media-failure recovery changed the attested GUI-owned process",
    )
    require(
        normalized_resolved_path(evidence.get("process_image_path", "")) == expected_mpv
        and evidence.get("process_sha256") == expected_mpv_sha256,
        "hard media-failure process image identity drifted",
    )
    require(
        evidence.get("initial_ipc_endpoint") == ipc_endpoint
        and evidence.get("failure_ipc_endpoint") == ipc_endpoint
        and evidence.get("recovered_ipc_endpoint") == ipc_endpoint,
        "hard media-failure recovery changed the managed mpv IPC endpoint",
    )
    require(
        normalized_resolved_path(evidence.get("restored_media_path", ""))
        == expected_media
        and evidence.get("restored_media_sha256") == sha256_file(expected_media),
        "hard media-failure restored-media identity drifted",
    )
    require(
        all(
            evidence.get(key) is True
            for key in (
                "same_process_identity",
                "same_ipc_endpoint",
                "evidence_retained_before_cleanup",
                "server_thread_released",
                "socket_released",
                "owned_mpv_terminated_after_gui_exit",
            )
        ),
        "hard media-failure identity, retention, release, or cleanup attestation was incomplete",
    )
    require(
        evidence.get("manual_retry_invoked") is False,
        "hard media-failure recovery unexpectedly used a manual retry",
    )
    require(evidence.get("error") is None, "hard media-failure evidence retained an error")
    require(
        isinstance(evidence.get("media_fail_event_id"), str)
        and evidence["media_fail_event_id"]
        and evidence.get("media_fail_emitter") == "gui-real-mpv"
        and evidence.get("media_fail_process_role") == "client",
        "MEDIA-FAIL-001 lifecycle attribution drifted",
    )
    require(
        isinstance(evidence.get("media_playable_event_id"), str)
        and evidence["media_playable_event_id"]
        and evidence["media_playable_event_id"] != evidence["media_fail_event_id"]
        and evidence.get("media_playable_emitter") == "gui-real-mpv"
        and evidence.get("media_playable_process_role") == "client",
        "MEDIA-PLAYABLE-001 lifecycle attribution drifted",
    )


def validate_http_stall_evidence(
    evidence: Any,
    *,
    artifact_evidence: dict[str, Any],
    observations: list[dict[str, Any]],
    expected_media: Path,
    expected_media_url: str,
    expected_mpv: Path,
    expected_mpv_sha256: str,
    mpv: dict[str, Any],
    ipc_endpoint: str,
) -> None:
    require(isinstance(evidence, dict), "HTTP stall contract missing")
    require(set(evidence) == HTTP_STALL_KEYS, "HTTP stall field inventory drifted")
    require(artifact_evidence == evidence, "report/artifact HTTP stall evidence diverged")
    require(evidence.get("schema_version") == SCHEMA_VERSION, "HTTP stall schema mismatch")
    require(evidence.get("kind") == HTTP_STALL_KIND, "HTTP stall kind mismatch")
    require(evidence.get("result") == "passed", "HTTP stall campaign did not pass")
    require(
        evidence.get("schedule")
        == "first-response-valid-prefix-then-open-byte-silence",
        "HTTP stall schedule drifted",
    )
    require(
        evidence.get("expected_outcome")
        == "one-bounded-same-generation-reload-after-sustained-cache-pause",
        "HTTP stall expected outcome drifted",
    )
    listener_endpoint = evidence.get("listener_endpoint")
    validate_ipv4_loopback_endpoint(listener_endpoint, "recorded stalled HTTP listener")
    require(
        evidence.get("listener_ipv4_loopback") is True,
        "HTTP stall listener loopback attestation missing",
    )
    require(
        evidence.get("media_url") == expected_media_url
        == f"http://{listener_endpoint}{HTTP_STALL_ROUTE}",
        "native Open Media did not deliver the exact stalled loopback URL",
    )
    require(evidence.get("route") == HTTP_STALL_ROUTE, "HTTP stall route drifted")
    require(
        is_json_integer(evidence.get("generated_media_bytes"))
        and evidence["generated_media_bytes"] == expected_media.stat().st_size,
        "HTTP stall generated-media size mismatch",
    )
    require(
        evidence.get("generated_media_sha256") == sha256_file(expected_media),
        "HTTP stall generated-media digest mismatch",
    )
    require(
        is_json_integer(evidence.get("duration_seconds"))
        and evidence["duration_seconds"] == HTTP_STALL_DURATION_SECONDS,
        "HTTP stall media duration contract drifted",
    )
    require(
        is_json_integer(evidence.get("prefix_body_bytes"))
        and evidence["prefix_body_bytes"] == HTTP_STALL_PREFIX_BYTES
        and evidence["prefix_body_bytes"] < evidence["generated_media_bytes"],
        "HTTP stall playable-prefix boundary drifted",
    )
    require(
        is_json_integer(evidence.get("prefix_bytes_per_second"))
        and evidence["prefix_bytes_per_second"] == HTTP_STALL_PREFIX_BYTES_PER_SECOND,
        "HTTP stall prefix pacing drifted",
    )
    require(
        is_json_number(evidence.get("expected_prefix_playable_seconds"))
        and math.isclose(
            float(evidence["expected_prefix_playable_seconds"]),
            HTTP_STALL_EXPECTED_PREFIX_PLAYABLE_SECONDS,
            abs_tol=1e-9,
        )
        and is_json_number(evidence.get("cache_stall_position_tolerance_seconds"))
        and math.isclose(
            float(evidence["cache_stall_position_tolerance_seconds"]),
            HTTP_STALL_POSITION_TOLERANCE_SECONDS,
            abs_tol=1e-9,
        ),
        "HTTP stall deterministic playable-prefix position oracle drifted",
    )
    require(
        is_json_integer(evidence.get("minimum_stall_duration_ms"))
        and evidence["minimum_stall_duration_ms"] == HTTP_STALL_MINIMUM_DURATION_MS
        and is_json_integer(evidence.get("maximum_recovery_wait_ms"))
        and evidence["maximum_recovery_wait_ms"]
        == HTTP_STALL_MAXIMUM_RECOVERY_WAIT_MS,
        "HTTP stall independent finite bounds drifted",
    )
    require(
        evidence.get("initial_pid") == mpv["pid"]
        and evidence.get("recovered_pid") == mpv["pid"]
        and evidence.get("parent_pid") == mpv["parent_pid"],
        "HTTP stall changed the attested GUI-owned mpv process",
    )
    require(
        normalized_resolved_path(evidence.get("process_image_path", "")) == expected_mpv
        and evidence.get("process_sha256") == expected_mpv_sha256,
        "HTTP stall process image identity drifted",
    )
    require(
        evidence.get("initial_ipc_endpoint") == ipc_endpoint
        and evidence.get("recovered_ipc_endpoint") == ipc_endpoint,
        "HTTP stall changed the managed mpv IPC endpoint",
    )
    require(
        all(
            evidence.get(key) is True
            for key in (
                "stable_process_identity",
                "stable_ipc_endpoint",
                "stable_media_url",
                "stable_duration",
                "evidence_retained_before_cleanup",
                "server_thread_released",
                "socket_released",
                "owned_mpv_terminated_after_gui_exit",
            )
        ),
        "HTTP stall identity, retention, release, or cleanup attestation was incomplete",
    )
    require(
        evidence.get("manual_retry_invoked") is False,
        "HTTP stall unexpectedly used a manual retry",
    )
    require(
        is_json_integer(evidence.get("foreign_pid_observations_after_stall"))
        and evidence["foreign_pid_observations_after_stall"] == 0,
        "stale or foreign mpv generation was observed after the HTTP stall",
    )
    require(
        is_json_integer(evidence.get("eof_observations_before_recovery"))
        and evidence["eof_observations_before_recovery"] == 0,
        "HTTP stall unexpectedly crossed an EOF boundary before recovery",
    )
    require(evidence.get("error") is None, "HTTP stall evidence retained an error")

    indices = [
        evidence.get("initial_file_loaded_index"),
        evidence.get("pre_stall_progress_index"),
        evidence.get("cache_stall_index"),
        evidence.get("recovered_file_loaded_index"),
        evidence.get("recovered_progress_index"),
        evidence.get("recovered_paused_index"),
    ]
    require(
        all(is_json_integer(index) and index >= 0 for index in indices),
        "HTTP stall observation indices were invalid",
    )
    (
        initial_index,
        pre_stall_progress_index,
        cache_stall_index,
        recovered_index,
        progress_index,
        paused_index,
    ) = indices
    require(
        initial_index
        < pre_stall_progress_index
        < cache_stall_index
        < recovered_index
        < progress_index
        < paused_index
        < len(observations),
        "HTTP stall observation ordering or bounds drifted",
    )
    require(
        all(
            item.get("pid") == mpv["pid"]
            and item.get("ipc_endpoint") == ipc_endpoint
            for item in observations[cache_stall_index : paused_index + 1]
        ),
        "unidentified, stale, or foreign mpv generation appeared after the HTTP stall boundary",
    )
    initial = observations[initial_index]
    require(
        initial.get("event") == "file-loaded"
        and initial.get("pid") == mpv["pid"]
        and initial.get("ipc_endpoint") == ipc_endpoint
        and initial.get("path") == expected_media_url
        and initial.get("filename") == HTTP_STALL_ROUTE.rsplit("/", 1)[-1]
        and is_json_number(initial.get("duration"))
        and abs(float(initial["duration"]) - HTTP_STALL_DURATION_SECONDS) <= 0.05,
        "initial native stalled HTTP file-loaded identity drifted or used a cache path",
    )
    pre_stall_position = evidence.get("pre_stall_position_seconds")
    cache_stall_position = evidence.get("cache_stall_position_seconds")
    recovered_position = evidence.get("recovered_position_seconds")
    require(
        is_json_number(pre_stall_position)
        and float(pre_stall_position) >= 0.5
        and is_json_number(cache_stall_position)
        and float(cache_stall_position) >= float(pre_stall_position)
        and abs(
            float(cache_stall_position) - HTTP_STALL_EXPECTED_PREFIX_PLAYABLE_SECONDS
        )
        <= HTTP_STALL_POSITION_TOLERANCE_SECONDS
        and is_json_number(recovered_position)
        and float(recovered_position) >= float(cache_stall_position) + 0.5,
        "HTTP stall did not retain bounded positive playback progress",
    )
    pre_stall_progress = observations[pre_stall_progress_index]
    require(
        pre_stall_progress.get("event") == "time-pos"
        and pre_stall_progress.get("pid") == mpv["pid"]
        and pre_stall_progress.get("ipc_endpoint") == ipc_endpoint
        and pre_stall_progress.get("path") == expected_media_url
        and is_json_number(pre_stall_progress.get("position"))
        and float(pre_stall_progress["position"]) == float(pre_stall_position),
        "HTTP pre-stall progress observation mismatch",
    )
    cache_stall = observations[cache_stall_index]
    require(
        cache_stall.get("event") == "paused-for-cache"
        and cache_stall.get("paused_for_cache") is True
        and cache_stall.get("pid") == mpv["pid"]
        and cache_stall.get("ipc_endpoint") == ipc_endpoint
        and cache_stall.get("path") == expected_media_url
        and is_json_number(cache_stall.get("duration"))
        and abs(float(cache_stall["duration"]) - HTTP_STALL_DURATION_SECONDS) <= 0.05
        and is_json_number(cache_stall.get("position"))
        and float(cache_stall["position"]) == float(cache_stall_position)
        and cache_stall.get("eof_reached") is not True,
        "valid open HTTP response did not produce the exact cache-stall observation",
    )
    require(
        not any(
            item.get("event") == "eof-reached" and item.get("eof_reached") is True
            for item in observations[initial_index:recovered_index]
        ),
        "HTTP stall trace observed EOF before the recovery load",
    )
    end_file_observations = [
        item
        for item in observations[cache_stall_index + 1 : recovered_index]
        if item.get("event") == "end-file"
    ]
    recovery_lifecycle_observations = [
        (index, item)
        for index, item in enumerate(observations)
        if cache_stall_index < index <= recovered_index
        and item.get("event") in {"end-file", "file-loaded"}
    ]
    require(
        len(end_file_observations) == 1
        and len(recovery_lifecycle_observations) == 2
        and recovery_lifecycle_observations[0][1].get("event") == "end-file"
        and recovery_lifecycle_observations[0][1].get("pid") == mpv["pid"]
        and recovery_lifecycle_observations[0][1].get("ipc_endpoint")
        == ipc_endpoint
        and recovery_lifecycle_observations[0][1].get("reason") == "stop"
        and recovery_lifecycle_observations[1][0] == recovered_index
        and recovery_lifecycle_observations[1][1].get("event") == "file-loaded"
        and recovery_lifecycle_observations[1][1].get("pid") == mpv["pid"]
        and recovery_lifecycle_observations[1][1].get("ipc_endpoint")
        == ipc_endpoint,
        "HTTP stall trace contained an unidentified or intervening lifecycle row instead of exactly one same-process end-file stop followed by recovery",
    )
    require(
        is_json_integer(evidence.get("end_file_observations_before_recovery"))
        and evidence["end_file_observations_before_recovery"]
        == len(end_file_observations),
        "HTTP stall end-file boundary accounting drifted",
    )
    recovered = observations[recovered_index]
    require(
        recovered.get("event") == "file-loaded"
        and recovered.get("pid") == mpv["pid"]
        and recovered.get("ipc_endpoint") == ipc_endpoint
        and recovered.get("path") == expected_media_url
        and recovered.get("filename") == HTTP_STALL_ROUTE.rsplit("/", 1)[-1]
        and is_json_number(recovered.get("duration"))
        and abs(float(recovered["duration"]) - HTTP_STALL_DURATION_SECONDS) <= 0.05,
        "recovered stalled HTTP media identity drifted or used a cache path",
    )
    recovered_progress = observations[progress_index]
    require(
        recovered_progress.get("event") == "time-pos"
        and recovered_progress.get("pid") == mpv["pid"]
        and recovered_progress.get("ipc_endpoint") == ipc_endpoint
        and recovered_progress.get("path") == expected_media_url
        and is_json_number(recovered_progress.get("position"))
        and float(recovered_progress["position"]) == float(recovered_position),
        "HTTP stalled recovery progress observation mismatch",
    )
    recovered_pause = observations[paused_index]
    require(
        recovered_pause.get("event") == "pause"
        and recovered_pause.get("pause") is True
        and recovered_pause.get("pid") == mpv["pid"]
        and recovered_pause.get("ipc_endpoint") == ipc_endpoint
        and recovered_pause.get("path") == expected_media_url,
        "HTTP stalled recovery pause observation mismatch",
    )

    requests = evidence.get("requests")
    require(
        isinstance(requests, list) and requests,
        "stalled HTTP request accounting was empty",
    )
    require(
        is_json_integer(evidence.get("request_count"))
        and evidence["request_count"] == len(requests),
        "stalled HTTP request count diverged from retained rows",
    )
    for index, request in enumerate(requests, start=1):
        require(isinstance(request, dict), "stalled HTTP request row must be an object")
        require(
            set(request) == HTTP_STALL_REQUEST_KEYS,
            "stalled HTTP request field inventory drifted",
        )
        require(
            is_json_integer(request.get("ordinal")) and request["ordinal"] == index,
            "stalled HTTP request ordinal drifted",
        )
        require(request.get("path") == HTTP_STALL_ROUTE, "stalled HTTP request path drifted")
        require(
            request.get("method") in {"HEAD", "GET"},
            "stalled HTTP method was unaccounted",
        )
        validate_ipv4_loopback_endpoint(
            request.get("peer_endpoint"), "stalled HTTP request peer endpoint"
        )
        require(
            request.get("peer_ipv4_loopback") is True,
            "stalled HTTP request ownership accounting drifted",
        )
        require(
            request.get("range_header") is None
            or isinstance(request.get("range_header"), str),
            "stalled HTTP Range accounting was malformed",
        )
        require(
            is_json_integer(request.get("status_code"))
            and request["status_code"] == 200
            and is_json_integer(request.get("content_length_header"))
            and request["content_length_header"] == evidence["generated_media_bytes"]
            and request.get("transfer_encoding") is None
            and is_json_integer(request.get("transmitted_body_bytes"))
            and request.get("write_error") is None,
            "stalled HTTP response framing or write accounting drifted",
        )
        if request["method"] == "HEAD":
            require(
                request.get("transmitted_body_bytes") == 0
                and request.get("stall_injected") is False
                and request.get("stalled_for_ms") is None
                and request.get("server_response_retained_at_recovery_get") is False
                and request.get("connection_released") is True
                and request.get("response_completed") is True,
                "stalled HTTP HEAD probe unexpectedly carried a body or stall",
            )
        else:
            require(
                request.get("range_header") == "bytes=0-",
                "stalled HTTP media GET did not use the exact byte-zero contract",
            )

    media_gets = [request for request in requests if request["method"] == "GET"]
    require(
        len(media_gets) == 2,
        "HTTP stall did not use exactly one open stalled GET and one complete GET",
    )
    first_get, recovered_get = media_gets
    require(
        first_get.get("transmitted_body_bytes") == HTTP_STALL_PREFIX_BYTES
        and first_get.get("stall_injected") is True
        and is_json_integer(first_get.get("stalled_for_ms"))
        and HTTP_STALL_MINIMUM_DURATION_MS
        <= first_get["stalled_for_ms"]
        <= HTTP_STALL_MAXIMUM_RECOVERY_WAIT_MS
        and first_get.get("server_response_retained_at_recovery_get") is True
        and first_get.get("connection_released") is True
        and first_get.get("response_completed") is False,
        "first HTTP GET was not the exact bounded open byte-silent response",
    )
    require(
        recovered_get.get("transmitted_body_bytes")
        == evidence["generated_media_bytes"]
        and recovered_get.get("stall_injected") is False
        and recovered_get.get("stalled_for_ms") is None
        and recovered_get.get("server_response_retained_at_recovery_get") is False
        and recovered_get.get("connection_released") is True
        and recovered_get.get("response_completed") is True,
        "second HTTP GET was not a complete stalled-read recovery response",
    )
    require(
        is_json_integer(evidence.get("stalled_response_count"))
        and evidence["stalled_response_count"] == 1
        and is_json_integer(evidence.get("complete_response_count"))
        and evidence["complete_response_count"] == 1,
        "stalled HTTP response aggregate accounting drifted",
    )


def validate_report(
    report: dict[str, Any],
    *,
    artifact_root: Path,
    expected_gui: Path,
    expected_gui_sha256: str,
    expected_mpv: Path,
    expected_mpv_sha256: str,
    producer_exit_code: int,
    expect_recovery: bool = False,
    expect_http_fault: bool = False,
    expect_http_stall: bool = False,
    lifecycle_summary_path: Path | None = None,
) -> dict[str, Any]:
    require(
        sum((expect_recovery, expect_http_fault, expect_http_stall)) <= 1,
        "process recovery, faulting HTTP, and stalled HTTP contracts are mutually exclusive",
    )
    expect_http = expect_http_fault or expect_http_stall
    artifact_root = normalized_resolved_path(artifact_root)
    expected_gui = normalized_resolved_path(expected_gui)
    expected_mpv = normalized_resolved_path(expected_mpv)
    require(producer_exit_code == 0, f"producer exited {producer_exit_code}")
    require(report.get("schema_version") == SCHEMA_VERSION, "report schema mismatch")
    require(report.get("kind") == REPORT_KIND, "report kind mismatch")
    require(report.get("result") == "passed", "real-mpv result was not passed")
    require(report.get("capability") == "executed", "real-mpv capability was not executed")

    validate_binary_identity(
        report.get("gui"), expected_gui, expected_gui_sha256, "GUI binary"
    )
    mpv = report.get("mpv")
    validate_binary_identity(mpv, expected_mpv, expected_mpv_sha256, "mpv binary")
    require(isinstance(mpv, dict), "mpv identity must be an object")
    require(mpv.get("minimum_supported_version") == "0.41.0", "minimum mpv drift")
    require(
        isinstance(mpv.get("version"), str) and mpv["version"].startswith("mpv v"),
        "mpv version identity missing",
    )
    require(isinstance(mpv.get("pid"), int) and mpv["pid"] > 0, "mpv PID missing")
    require(
        isinstance(mpv.get("parent_pid"), int) and mpv["parent_pid"] > 0,
        "mpv parent PID missing",
    )
    require(
        normalized_resolved_path(mpv.get("process_image_path", "")) == expected_mpv,
        "running mpv image path mismatch",
    )
    recovered_mpv = report.get("recovered_mpv")
    if expect_recovery:
        validate_binary_identity(
            recovered_mpv,
            expected_mpv,
            expected_mpv_sha256,
            "recovered mpv binary",
        )
        require(isinstance(recovered_mpv, dict), "recovered mpv identity must be an object")
        require(
            recovered_mpv.get("minimum_supported_version") == "0.41.0",
            "recovered minimum mpv drift",
        )
        require(
            recovered_mpv.get("version") == mpv["version"],
            "recovered mpv version identity drifted",
        )
        require(
            isinstance(recovered_mpv.get("pid"), int)
            and recovered_mpv["pid"] > 0
            and recovered_mpv["pid"] != mpv["pid"],
            "recovered mpv PID was not a distinct positive process",
        )
        require(
            recovered_mpv.get("parent_pid") == mpv["parent_pid"],
            "recovered mpv was not owned by the same GUI",
        )
        require(
            normalized_resolved_path(recovered_mpv.get("process_image_path", ""))
            == expected_mpv,
            "recovered running mpv image path mismatch",
        )
    else:
        require("recovered_mpv" not in report, "baseline report unexpectedly included recovery")
        require("recovery" not in report, "baseline report unexpectedly included recovery state")
    if expect_http_fault:
        require(isinstance(report.get("http_fault"), dict), "HTTP fault contract missing")
        require(
            isinstance(report.get("media_failure"), dict),
            "hard media-failure contract missing",
        )
    else:
        require(
            "http_fault" not in report,
            "non-HTTP report unexpectedly included faulting HTTP state",
        )
        require(
            "media_failure" not in report,
            "non-HTTP report unexpectedly included hard media-failure state",
        )
    if expect_http_stall:
        require(isinstance(report.get("http_stall"), dict), "HTTP stall contract missing")
    else:
        require(
            "http_stall" not in report,
            "non-stall report unexpectedly included stalled HTTP state",
        )

    isolation = report.get("isolation")
    require(isinstance(isolation, dict), "isolation contract missing")
    require(
        normalized_resolved_path(isolation.get("artifact_root", "")) == artifact_root,
        "artifact root mismatch",
    )
    require(
        isolation.get("network_mode")
        == (
            "os-assigned-ipv4-loopback-session-and-http"
            if expect_http
            else "os-assigned-ipv4-loopback-session"
        ),
        "network mode drift",
    )
    validate_ipv4_loopback_endpoint(
        isolation.get("session_endpoint"), "session listener endpoint"
    )
    validate_ipv4_loopback_endpoint(
        isolation.get("session_peer_endpoint"), "session connected peer endpoint"
    )
    require(
        isolation.get("session_advertised_capabilities") == list(SESSION_CAPABILITIES),
        "session advertised capability inventory drifted",
    )
    require(
        isolation.get("media_source")
        == (
            (
                "generated-pcm-au-over-faulting-loopback-http"
                if expect_http_fault
                else "generated-pcm-au-over-stalled-loopback-http"
            )
            if expect_http
            else "generated-local-pcm-wav"
        ),
        "generated media contract drift",
    )
    require(isolation.get("mpv_config") == "isolated --no-config", "mpv config drift")
    for key in (
        "config_path",
        "appdata_root",
        "media_path",
        "observation_script_path",
        "observation_path",
        "mpv_log_path",
        "lifecycle_path",
        "shared_lifecycle_path",
        "session_exchange_path",
        "menu_interactions_path",
    ):
        candidate = normalized_resolved_path(isolation.get(key, ""))
        require(candidate.is_relative_to(artifact_root), f"{key} escaped artifact root")
    expected_media = normalized_resolved_path(isolation["media_path"])
    expected_media_url: str | None = None
    if expect_http:
        expected_media_url = str(isolation.get("media_url", ""))
        http_endpoint = isolation.get("http_endpoint")
        validate_ipv4_loopback_endpoint(http_endpoint, "HTTP listener endpoint")
        expected_http_route = (
            HTTP_FAULT_ROUTE if expect_http_fault else HTTP_STALL_ROUTE
        )
        require(
            expected_media_url == f"http://{http_endpoint}{expected_http_route}",
            "HTTP media URL was not the exact strict loopback route",
        )
        http_evidence_path = normalized_resolved_path(
            isolation.get("http_evidence_path", "")
        )
        require(
            http_evidence_path.is_relative_to(artifact_root),
            "HTTP evidence path escaped artifact root",
        )
    else:
        require("media_url" not in isolation, "baseline isolation included media URL")
        require("http_endpoint" not in isolation, "baseline isolation included HTTP endpoint")
        require(
            "http_evidence_path" not in isolation,
            "baseline isolation included HTTP evidence path",
        )
    ipc_prefix = rf"\\.\pipe\sorotte-gui-mpv-{mpv['parent_pid']}-"
    require(
        str(isolation.get("ipc_endpoint", "")).startswith(ipc_prefix),
        "managed mpv IPC endpoint was not bound to the GUI process",
    )

    required_assertions = (
        RECOVERY_REQUIRED_ASSERTIONS
        if expect_recovery
        else (
            HTTP_FAULT_REQUIRED_ASSERTIONS
            if expect_http_fault
            else (
                HTTP_STALL_REQUIRED_ASSERTIONS
                if expect_http_stall
                else REQUIRED_ASSERTIONS
            )
        )
    )
    assertions = report.get("assertions")
    require(isinstance(assertions, list), "assertions must be a list")
    require(
        assertions == list(required_assertions),
        "assertion inventory or order drifted",
    )

    required_artifacts = (
        RECOVERY_REQUIRED_ARTIFACTS
        if expect_recovery
        else (
            HTTP_FAULT_REQUIRED_ARTIFACTS
            if expect_http_fault
            else (
                HTTP_STALL_REQUIRED_ARTIFACTS
                if expect_http_stall
                else REQUIRED_ARTIFACTS
            )
        )
    )
    artifacts = report.get("artifacts")
    require(isinstance(artifacts, dict), "artifact manifest missing")
    require(
        set(artifacts) == set(required_artifacts),
        f"artifact inventory mismatch: {sorted(artifacts)}",
    )
    resolved_artifacts: dict[str, Path] = {}
    for label in required_artifacts:
        identity = artifacts[label]
        require(isinstance(identity, dict), f"{label} artifact identity must be an object")
        path = resolved_child(artifact_root, str(identity.get("path", "")), label)
        require(path.is_file(), f"{label} artifact is missing: {path}")
        require(identity.get("bytes") == path.stat().st_size, f"{label} size mismatch")
        require(identity.get("sha256") == sha256_file(path), f"{label} digest mismatch")
        resolved_artifacts[label] = path

    require(
        resolved_artifacts["success_screenshot"].read_bytes().startswith(b"\x89PNG\r\n\x1a\n"),
        "success screenshot was not a PNG",
    )
    if expect_recovery:
        for label in ("automatic_relaunch_screenshot", "recovery_screenshot"):
            require(
                resolved_artifacts[label]
                .read_bytes()
                .startswith(b"\x89PNG\r\n\x1a\n"),
                f"{label} was not a PNG",
            )
    require(resolved_artifacts["mpv_log"].stat().st_size > 0, "mpv log was empty")
    require(
        resolved_artifacts["generated_media"].resolve() == expected_media,
        "generated media artifact mismatch",
    )
    require(
        resolved_artifacts["session_exchange"].resolve()
        == normalized_resolved_path(isolation["session_exchange_path"]),
        "session exchange artifact mismatch",
    )
    require(
        resolved_artifacts["menu_interactions"].resolve()
        == normalized_resolved_path(isolation["menu_interactions_path"]),
        "menu interaction artifact mismatch",
    )
    if expect_http_fault:
        require(
            resolved_artifacts["faulting_http_recovery"].resolve()
            == normalized_resolved_path(isolation["http_evidence_path"]),
            "faulting HTTP evidence artifact mismatch",
        )
    if expect_http_stall:
        require(
            resolved_artifacts["stalled_http"].resolve()
            == normalized_resolved_path(isolation["http_evidence_path"]),
            "stalled HTTP evidence artifact mismatch",
        )

    session_exchange = load_json(
        resolved_artifacts["session_exchange"], "real-mpv session exchange"
    )
    require(
        set(session_exchange) == SESSION_EXCHANGE_KEYS,
        "session exchange field inventory drifted",
    )
    require(
        session_exchange.get("schema_version") == SCHEMA_VERSION,
        "session exchange schema mismatch",
    )
    require(
        session_exchange.get("kind") == SESSION_EXCHANGE_KIND,
        "session exchange kind mismatch",
    )
    require(session_exchange.get("result") == "released", "session was not released")
    require(
        session_exchange.get("bound_endpoint") == isolation["session_endpoint"],
        "session exchange listener endpoint mismatch",
    )
    require(
        session_exchange.get("connected_peer_endpoint")
        == isolation["session_peer_endpoint"],
        "session exchange peer endpoint mismatch",
    )
    validate_ipv4_loopback_endpoint(
        session_exchange.get("bound_endpoint"), "recorded session listener"
    )
    validate_ipv4_loopback_endpoint(
        session_exchange.get("connected_peer_endpoint"),
        "recorded session connected peer",
    )
    require(
        session_exchange.get("listener_ipv4_loopback") is True,
        "session listener loopback attestation missing",
    )
    require(
        session_exchange.get("peer_ipv4_loopback") is True,
        "session peer loopback attestation missing",
    )
    require(
        session_exchange.get("advertised_capabilities") == list(SESSION_CAPABILITIES),
        "session exchange advertised capability inventory drifted",
    )
    require(
        session_exchange.get("server_thread_released") is True,
        "session server thread was not released",
    )
    require(
        session_exchange.get("socket_released") is True,
        "session socket was not released",
    )
    require(session_exchange.get("error") is None, "session exchange retained an error")
    try:
        client_hello = json.loads(str(session_exchange.get("client_hello", "")))
        server_hello = json.loads(str(session_exchange.get("server_hello", "")))
    except json.JSONDecodeError as error:
        raise ValueError(f"session Hello exchange was invalid JSON: {error}") from error
    expected_client_hello = (
        EXPECTED_HTTP_FAULT_CLIENT_HELLO if expect_http else EXPECTED_CLIENT_HELLO
    )
    require(client_hello == expected_client_hello, "client Hello exchange drifted")
    require(server_hello == EXPECTED_SERVER_HELLO, "server Hello exchange drifted")
    expected_playlist_target = isolation.get("media_url") if expect_http else Path(
        str(isolation.get("media_path", ""))
    ).name
    require(
        isinstance(expected_playlist_target, str) and bool(expected_playlist_target),
        "session playlist target was unavailable",
    )
    try:
        playlist_change_request = json.loads(
            str(session_exchange.get("playlist_change_request", ""))
        )
        playlist_change_echo = json.loads(
            str(session_exchange.get("playlist_change_echo", ""))
        )
        playlist_index_request = json.loads(
            str(session_exchange.get("playlist_index_request", ""))
        )
        playlist_index_echo = json.loads(
            str(session_exchange.get("playlist_index_echo", ""))
        )
    except json.JSONDecodeError as error:
        raise ValueError(
            f"session playlist request/echo evidence was invalid JSON: {error}"
        ) from error
    require(
        playlist_change_request
        == {
            "Set": {
                "playlistChange": {
                    "files": [expected_playlist_target],
                }
            }
        },
        "session playlistChange request drifted from the exact closed request schema",
    )
    require(
        playlist_change_echo
        == {
            "Set": {
                "playlistChange": {
                    "files": [expected_playlist_target],
                    "user": "real-mpv-user",
                }
            }
        },
        "session authoritative playlistChange echo drifted",
    )
    require(
        playlist_index_request
        == {
            "Set": {
                "playlistIndex": {
                    "index": 0,
                }
            }
        },
        "session playlistIndex request drifted from the exact closed request schema",
    )
    require(
        playlist_index_echo
        == {
            "Set": {
                "playlistIndex": {
                    "index": 0,
                    "user": "real-mpv-user",
                }
            }
        },
        "session authoritative playlistIndex echo drifted",
    )
    try:
        initial_authoritative_playstate = json.loads(
            str(session_exchange.get("initial_authoritative_playstate", ""))
        )
    except json.JSONDecodeError as error:
        raise ValueError(
            f"initial authoritative playstate evidence was invalid JSON: {error}"
        ) from error
    require(
        initial_authoritative_playstate
        == {
            "State": {
                "playstate": {
                    "doSeek": False,
                    "paused": True,
                    "position": 0.0,
                    "setBy": "real-mpv-user",
                }
            }
        },
        "initial authoritative paused playstate drifted",
    )
    expected_playstate_exchanges = [
        ("GUI Play canonical transport", False),
        (
            "GUI Pause after HTTP fault canonical transport"
            if expect_http_fault
            else "GUI Pause after HTTP stall canonical transport"
            if expect_http_stall
            else "GUI Pause canonical transport",
            True,
        ),
    ]
    if expect_recovery:
        expected_playstate_exchanges.extend(
            [
                ("GUI Play on replacement mpv canonical transport", False),
                ("GUI Pause on replacement mpv canonical transport", True),
            ]
        )
    playstate_exchanges = session_exchange.get("playstate_exchanges")
    require(
        isinstance(playstate_exchanges, list),
        "session playstate exchange inventory must be a list",
    )
    exchange_index = 0
    for expected_action, expected_paused in expected_playstate_exchanges:
        while exchange_index < len(playstate_exchanges):
            row = playstate_exchanges[exchange_index]
            exchange_index += 1
            require(
                isinstance(row, dict),
                "session playstate exchange must be an object",
            )
            mutation_kind = row.get("mutation_kind")
            if mutation_kind == "seek":
                observed_paused = row.get("expected_paused")
                require(
                    isinstance(observed_paused, bool),
                    f"{expected_action} interleaved seek pause level was invalid",
                )
                validate_playstate_exchange(
                    row,
                    expected_action=expected_action,
                    expected_paused=observed_paused,
                    expected_mutation_kind="seek",
                )
                continue
            validate_playstate_exchange(
                row,
                expected_action=expected_action,
                expected_paused=expected_paused,
                expected_mutation_kind="pause",
            )
            break
        else:
            raise ValueError("session canonical Play/Pause exchange inventory drifted")
    require(
        exchange_index == len(playstate_exchanges),
        "session canonical Play/Pause exchange inventory drifted",
    )

    menu_interactions = load_json(
        resolved_artifacts["menu_interactions"], "real-mpv menu interactions"
    )
    require(
        menu_interactions.get("schema_version") == SCHEMA_VERSION,
        "menu interaction schema mismatch",
    )
    require(
        menu_interactions.get("kind") == MENU_INTERACTIONS_KIND,
        "menu interaction kind mismatch",
    )
    require(menu_interactions.get("result") == "passed", "menu interactions did not pass")
    require(menu_interactions.get("error") is None, "menu interactions retained an error")
    interaction_rows = menu_interactions.get("interactions")
    require(isinstance(interaction_rows, list), "menu interactions must be an array")
    expected_menu_actions = (
        ["menu.open_media", "menu.open_media", "menu.exit"]
        if expect_recovery
        else ["menu.open_media", "menu.exit"]
    )
    require(
        [row.get("action_automation_id") for row in interaction_rows]
        == expected_menu_actions,
        "menu action inventory or order drifted",
    )
    for row in interaction_rows:
        require(isinstance(row, dict), "menu interaction row must be an object")
        require(
            row.get("section_automation_id") == "menu.section.file",
            "menu interaction section drifted",
        )
        require(
            row.get("leaf_delivery") == "single-exact-physical-click-no-retry"
            and row.get("leaf_delivered") is True,
            "menu leaf was not delivered by one exact physical click",
        )
        require(row.get("error") is None, "menu interaction retained an error")
        strategy = row.get("section_open_strategy")
        require(
            strategy
            in {
                "physical-section-open",
                "uia-section-open-after-two-hidden-snapshots",
            },
            "menu section-open strategy drifted",
        )
        snapshots = row.get("pre_fallback_snapshots")
        require(isinstance(snapshots, list), "menu fallback snapshots must be an array")
        if strategy == "physical-section-open":
            require(snapshots == [], "physical section open unexpectedly retained fallback state")
        else:
            require(
                len(snapshots) == 2
                and all(
                    isinstance(snapshot, dict) and snapshot.get("visible_nodes") == 0
                    for snapshot in snapshots
                ),
                "UIA section fallback lacked two confirmed-hidden snapshots",
            )
            opened_snapshot = row.get("opened_snapshot")
            require(
                isinstance(opened_snapshot, dict)
                and opened_snapshot.get("visible_enabled_nodes") == 1,
                "UIA section fallback did not retain its exact enabled leaf",
            )

    state = load_json(resolved_artifacts["state"], "real-mpv state")
    require(state.get("schema_version") == SCHEMA_VERSION, "state schema mismatch")
    require(state.get("kind") == REPORT_KIND, "state kind mismatch")
    require(state.get("result") == "passed", "state did not finish passed")
    require(state.get("stage") == "complete", "state did not reach complete")
    require(state.get("gui_pid") == mpv["parent_pid"], "GUI ownership PID mismatch")
    require(state.get("mpv_pid") == mpv["pid"], "mpv state PID mismatch")
    if expect_recovery:
        require(
            state.get("recovered_mpv_pid") == recovered_mpv["pid"],
            "recovered mpv state PID mismatch",
        )
    else:
        require(
            "recovered_mpv_pid" not in state,
            "baseline state unexpectedly included recovery PID",
        )
    require(state.get("assertions") == assertions, "state/report assertions diverged")

    observations = validate_observations(
        resolved_artifacts["mpv_observation"],
        expected_media,
        mpv["pid"],
        expected_media_url,
    )
    if expect_recovery:
        recovery = report.get("recovery")
        require(isinstance(recovery, dict), "recovery contract missing")
        require(set(recovery) == RECOVERY_KEYS, "recovery field inventory drifted")
        artifact_recovery = load_json(
            resolved_artifacts["owned_mpv_recovery"], "owned-mpv recovery"
        )
        require(artifact_recovery == recovery, "report/artifact recovery evidence diverged")
        require(
            recovery.get("schema_version") == SCHEMA_VERSION,
            "recovery schema mismatch",
        )
        require(recovery.get("kind") == RECOVERY_KIND, "recovery kind mismatch")
        require(recovery.get("result") == "passed", "recovery did not finish passed")
        require(
            recovery.get("fault") == "terminate-exact-attested-gui-owned-mpv",
            "recovery fault contract drifted",
        )
        require(
            recovery.get("recovery_mode")
            == "active-session-automatic-managed-mpv-relaunch",
            "automatic recovery mode drifted",
        )
        require(
            isinstance(recovery.get("automatic_relaunch_timeout_ms"), int)
            and recovery["automatic_relaunch_timeout_ms"] > 0
            and recovery["automatic_relaunch_timeout_ms"] <= 12_000,
            "automatic relaunch timeout was not positive and bounded",
        )
        require(recovery.get("error") is None, "recovery retained an error")
        require(recovery.get("initial_pid") == mpv["pid"], "recovery initial PID mismatch")
        require(
            recovery.get("initial_parent_pid") == mpv["parent_pid"],
            "recovery initial parent mismatch",
        )
        require(
            normalized_resolved_path(recovery.get("initial_process_image_path", ""))
            == expected_mpv,
            "recovery initial image mismatch",
        )
        require(
            recovery.get("initial_sha256") == expected_mpv_sha256,
            "recovery initial digest mismatch",
        )
        require(
            recovery.get("initial_ipc_endpoint") == isolation["ipc_endpoint"],
            "recovery initial IPC mismatch",
        )
        require(
            recovery.get("initial_process_terminated") is True,
            "initial owned mpv was not confirmed terminated before automatic relaunch",
        )
        require(
            recovery.get("automatic_relaunch_observation_event") == "pause",
            "automatic relaunch observation event drifted",
        )
        require(
            recovery.get("gui_room_remained_active") is True
            and recovery.get("manual_retry_invoked") is False,
            "automatic recovery unexpectedly required manual retry or left the active room",
        )
        require(
            recovery.get("recovered_pid") == recovered_mpv["pid"]
            and recovery.get("recovered_parent_pid") == recovered_mpv["parent_pid"],
            "recovery replacement ownership mismatch",
        )
        require(
            normalized_resolved_path(recovery.get("recovered_process_image_path", ""))
            == expected_mpv
            and recovery.get("recovered_sha256") == expected_mpv_sha256,
            "recovery replacement image identity mismatch",
        )
        recovered_ipc = str(recovery.get("recovered_ipc_endpoint", ""))
        require(
            recovered_ipc.startswith(ipc_prefix)
            and recovered_ipc != isolation["ipc_endpoint"],
            "recovery replacement IPC was not distinct and GUI-owned",
        )
        require(
            recovery.get("distinct_pid") is True
            and recovery.get("distinct_ipc_endpoint") is True,
            "recovery replacement identity was not explicitly distinct",
        )
        require(
            recovery.get("initial_process_still_terminated_after_recovery") is True
            and recovery.get("initial_process_still_terminated_after_gui_exit") is True
            and recovery.get("recovered_process_terminated_after_gui_exit") is True,
            "recovery final process cleanup was incomplete",
        )
        observation_indices = [
            recovery.get("post_termination_observation_index"),
            recovery.get("automatic_relaunch_observation_index"),
            recovery.get("recovered_file_loaded_index"),
            recovery.get("recovered_playing_index"),
            recovery.get("recovered_paused_index"),
        ]
        require(
            all(isinstance(index, int) and index >= 0 for index in observation_indices),
            "recovery observation indices were invalid",
        )
        boundary, relaunch_index, loaded_index, playing_index, paused_index = (
            observation_indices
        )
        require(
            boundary
            <= relaunch_index
            < loaded_index
            < playing_index
            < paused_index
            < len(observations),
            "recovery observation ordering or bounds drifted",
        )
        require(
            all(
                item.get("pid") == recovered_mpv["pid"]
                for item in observations[boundary:]
            ),
            "stale or foreign mpv generation observed after termination boundary",
        )
        require(
            observations[relaunch_index].get("event") == "pause"
            and observations[relaunch_index].get("pause") is True
            and observations[relaunch_index].get("ipc_endpoint") == recovered_ipc,
            "automatic replacement start observation mismatch",
        )
        recovered_loaded = observations[loaded_index]
        require(
            recovered_loaded.get("event") == "file-loaded"
            and normalized_resolved_path(recovered_loaded.get("path", ""))
            == expected_media
            and recovered_loaded.get("ipc_endpoint") == recovered_ipc,
            "replacement mpv file-loaded observation mismatch",
        )
        require(
            observations[playing_index].get("event") == "pause"
            and observations[playing_index].get("pause") is False,
            "replacement mpv playing observation mismatch",
        )
        require(
            observations[paused_index].get("event") == "pause"
            and observations[paused_index].get("pause") is True,
            "replacement mpv paused observation mismatch",
        )
    if expect_http_fault:
        http_fault = report.get("http_fault")
        artifact_http_fault = load_json(
            resolved_artifacts["faulting_http_recovery"],
            "faulting HTTP recovery",
        )
        validate_http_fault_evidence(
            http_fault,
            artifact_evidence=artifact_http_fault,
            observations=observations,
            expected_media=expected_media,
            expected_media_url=str(expected_media_url),
            expected_mpv=expected_mpv,
            expected_mpv_sha256=expected_mpv_sha256,
            mpv=mpv,
            ipc_endpoint=str(isolation["ipc_endpoint"]),
        )
        media_failure = report.get("media_failure")
        artifact_media_failure = load_json(
            resolved_artifacts["hard_media_failure"],
            "hard media-failure recovery",
        )
        validate_media_failure_evidence(
            media_failure,
            artifact_evidence=artifact_media_failure,
            observations=observations,
            expected_media=expected_media,
            expected_mpv=expected_mpv,
            expected_mpv_sha256=expected_mpv_sha256,
            mpv=mpv,
            ipc_endpoint=str(isolation["ipc_endpoint"]),
        )
    if expect_http_stall:
        http_stall = report.get("http_stall")
        artifact_http_stall = load_json(
            resolved_artifacts["stalled_http"],
            "stalled HTTP",
        )
        validate_http_stall_evidence(
            http_stall,
            artifact_evidence=artifact_http_stall,
            observations=observations,
            expected_media=expected_media,
            expected_media_url=str(expected_media_url),
            expected_mpv=expected_mpv,
            expected_mpv_sha256=expected_mpv_sha256,
            mpv=mpv,
            ipc_endpoint=str(isolation["ipc_endpoint"]),
        )
    config_text = resolved_artifacts["config"].read_text(encoding="utf-8")
    expected_mpv_spellings = {str(expected_mpv)}
    if os.name == "nt":
        expected_mpv_spellings.add("\\\\?\\" + str(expected_mpv))
    require(
        any(spelling in config_text for spelling in expected_mpv_spellings),
        "isolated config omitted exact mpv path",
    )
    require("host =" not in config_text, "isolated real-mpv config unexpectedly defined a host")

    summary = {
        "schema_version": SCHEMA_VERSION,
        "kind": SUMMARY_KIND,
        "result": "passed",
        "capability": "executed",
        "assertion_count": len(required_assertions),
        "artifact_count": len(required_artifacts),
        "gui_sha256": expected_gui_sha256,
        "mpv_sha256": expected_mpv_sha256,
    }
    if expect_recovery:
        summary["recovery_exercised"] = True
    if expect_http_fault:
        summary["http_fault_exercised"] = True
        summary["media_failure_recovery_exercised"] = True
    if expect_http_stall:
        summary["http_stall_exercised"] = True
    if lifecycle_summary_path is not None:
        lifecycle_digest, transition_coverage = lifecycle_summary_binding(
            lifecycle_summary_path
        )
        summary["lifecycle_summary_sha256"] = lifecycle_digest
        summary["lifecycle_transition_coverage"] = transition_coverage
    return summary


def write_summary(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--artifact-dir", type=Path, required=True)
    parser.add_argument("--expected-gui", type=Path, required=True)
    parser.add_argument("--expected-gui-sha256", required=True)
    parser.add_argument("--expected-mpv", type=Path, required=True)
    parser.add_argument("--expected-mpv-sha256", required=True)
    parser.add_argument("--producer-exit-code", type=int, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--lifecycle-summary", type=Path)
    capability = parser.add_mutually_exclusive_group()
    capability.add_argument("--expect-recovery", action="store_true")
    capability.add_argument("--expect-http-fault", action="store_true")
    capability.add_argument("--expect-http-stall", action="store_true")
    args = parser.parse_args()

    try:
        report = load_json(args.report, "real-mpv report")
        summary = validate_report(
            report,
            artifact_root=args.artifact_dir,
            expected_gui=args.expected_gui,
            expected_gui_sha256=args.expected_gui_sha256,
            expected_mpv=args.expected_mpv,
            expected_mpv_sha256=args.expected_mpv_sha256,
            producer_exit_code=args.producer_exit_code,
            expect_recovery=args.expect_recovery,
            expect_http_fault=args.expect_http_fault,
            expect_http_stall=args.expect_http_stall,
            lifecycle_summary_path=args.lifecycle_summary,
        )
    except (OSError, TypeError, ValueError) as error:
        summary = {
            "schema_version": SCHEMA_VERSION,
            "kind": SUMMARY_KIND,
            "result": "error",
            "error": str(error),
        }
        write_summary(args.summary, summary)
        print(json.dumps(summary, sort_keys=True))
        return 1

    write_summary(args.summary, summary)
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
