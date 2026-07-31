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
HTTP_FAULT_DURATION_SECONDS = 45
HTTP_FAULT_DISCONNECT_AFTER_BYTES = 720_000
SESSION_CAPABILITIES = ("chat", "readiness", "sharedPlaylists")
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
            "sharedPlaylists": False,
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
    "one-premature-http-eof-observed",
    "same-owned-mpv-reloaded-stable-network-media",
    "recovered-playback-advanced-past-fault",
    "gui-pause-command-observed-by-real-mpv",
    "gui-projected-paused-after-real-mpv-observation",
    "fault-evidence-retained-before-cleanup",
    "native-success-screenshot",
    "gui-exit-reaped-owned-mpv-and-released-fault-server",
)
REQUIRED_ARTIFACTS = (
    "config",
    "generated_media",
    "observation_script",
    "mpv_observation",
    "mpv_log",
    "gui_lifecycle",
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
    "disconnect_after_body_bytes",
    "request_count",
    "premature_disconnect_count",
    "complete_response_count",
    "requests",
    "initial_file_loaded_index",
    "pre_fault_progress_index",
    "end_file_index",
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
    "advertised_body_bytes",
    "transmitted_body_bytes",
    "disconnected_early",
    "write_error",
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


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
        == "first-response-content-length-is-shorter-than-declared-au-media-once",
        "HTTP fault shape drifted",
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
        evidence.get("disconnect_after_body_bytes")
        == HTTP_FAULT_DISCONNECT_AFTER_BYTES,
        "HTTP short-response boundary drifted",
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
        evidence.get("end_file_index"),
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
    terminal = observations[end_index]
    require(
        terminal.get("event") == "end-file"
        and terminal.get("reason") == "eof"
        and terminal.get("pid") == mpv["pid"]
        and terminal.get("ipc_endpoint") == ipc_endpoint,
        "controlled HTTP response did not cause the expected terminal EOF",
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
    recovered_position = evidence.get("recovered_position_seconds")
    require(
        is_json_number(pre_fault_position)
        and pre_fault_position >= 0.5
        and is_json_number(recovered_position)
        and recovered_position >= pre_fault_position + 0.5,
        "HTTP recovery did not retain bounded positive playback progress",
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
                and request.get("advertised_body_bytes")
                == evidence["generated_media_bytes"]
                and request.get("transmitted_body_bytes") == 0
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
        "HTTP recovery did not use exactly one short GET and one complete GET",
    )
    first_get, recovered_get = media_gets
    require(
        first_get.get("disconnected_early") is True
        and first_get.get("advertised_body_bytes")
        == HTTP_FAULT_DISCONNECT_AFTER_BYTES
        and first_get.get("transmitted_body_bytes")
        == HTTP_FAULT_DISCONNECT_AFTER_BYTES
        < evidence["generated_media_bytes"],
        "first HTTP GET was not the exact one-shot short response",
    )
    require(
        recovered_get.get("disconnected_early") is False
        and recovered_get.get("advertised_body_bytes")
        == evidence["generated_media_bytes"]
        and recovered_get.get("transmitted_body_bytes")
        == recovered_get.get("advertised_body_bytes"),
        "second HTTP GET was not a complete recovery response",
    )
    require(
        evidence.get("premature_disconnect_count") == 1
        and evidence.get("complete_response_count") == 1,
        "HTTP response aggregate accounting drifted",
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
) -> dict[str, Any]:
    require(
        not (expect_recovery and expect_http_fault),
        "process recovery and faulting HTTP contracts are mutually exclusive",
    )
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
    else:
        require(
            "http_fault" not in report,
            "non-HTTP report unexpectedly included faulting HTTP state",
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
            if expect_http_fault
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
            "generated-pcm-au-over-faulting-loopback-http"
            if expect_http_fault
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
        "session_exchange_path",
        "menu_interactions_path",
    ):
        candidate = normalized_resolved_path(isolation.get(key, ""))
        require(candidate.is_relative_to(artifact_root), f"{key} escaped artifact root")
    expected_media = normalized_resolved_path(isolation["media_path"])
    expected_media_url: str | None = None
    if expect_http_fault:
        expected_media_url = str(isolation.get("media_url", ""))
        http_endpoint = isolation.get("http_endpoint")
        validate_ipv4_loopback_endpoint(http_endpoint, "faulting HTTP listener endpoint")
        require(
            expected_media_url == f"http://{http_endpoint}{HTTP_FAULT_ROUTE}",
            "faulting HTTP media URL was not the exact strict loopback route",
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
            else REQUIRED_ASSERTIONS
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
            else REQUIRED_ARTIFACTS
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

    session_exchange = load_json(
        resolved_artifacts["session_exchange"], "real-mpv session exchange"
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
        EXPECTED_HTTP_FAULT_CLIENT_HELLO if expect_http_fault else EXPECTED_CLIENT_HELLO
    )
    require(client_hello == expected_client_hello, "client Hello exchange drifted")
    require(server_hello == EXPECTED_SERVER_HELLO, "server Hello exchange drifted")
    if expect_http_fault:
        expected_media_url = isolation.get("media_url")
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
                        "files": [expected_media_url],
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
                        "files": [expected_media_url],
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
    capability = parser.add_mutually_exclusive_group()
    capability.add_argument("--expect-recovery", action="store_true")
    capability.add_argument("--expect-http-fault", action="store_true")
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
