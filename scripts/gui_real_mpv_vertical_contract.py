#!/usr/bin/env python3
"""Fail-closed contract validation for the native GUI-to-real-mpv vertical lane."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-gui-real-mpv-vertical"
SUMMARY_KIND = "sorotte-gui-real-mpv-vertical-contract"
SESSION_EXCHANGE_KIND = "sorotte-gui-real-mpv-loopback-exchange"
MENU_INTERACTIONS_KIND = "sorotte-gui-real-mpv-menu-interactions"
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


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


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
) -> None:
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

    file_loaded_index = next(
        (
            index
            for index, item in enumerate(observations)
            if item.get("event") == "file-loaded"
            and item.get("pid") == expected_mpv_pid
            and normalized_resolved_path(item.get("path", "")) == expected_media
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


def validate_report(
    report: dict[str, Any],
    *,
    artifact_root: Path,
    expected_gui: Path,
    expected_gui_sha256: str,
    expected_mpv: Path,
    expected_mpv_sha256: str,
    producer_exit_code: int,
) -> dict[str, Any]:
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

    isolation = report.get("isolation")
    require(isinstance(isolation, dict), "isolation contract missing")
    require(
        normalized_resolved_path(isolation.get("artifact_root", "")) == artifact_root,
        "artifact root mismatch",
    )
    require(
        isolation.get("network_mode") == "os-assigned-ipv4-loopback-session",
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
        isolation.get("media_source") == "generated-local-pcm-wav",
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
    ipc_prefix = rf"\\.\pipe\sorotte-gui-mpv-{mpv['parent_pid']}-"
    require(
        str(isolation.get("ipc_endpoint", "")).startswith(ipc_prefix),
        "managed mpv IPC endpoint was not bound to the GUI process",
    )

    assertions = report.get("assertions")
    require(isinstance(assertions, list), "assertions must be a list")
    require(
        assertions == list(REQUIRED_ASSERTIONS),
        "assertion inventory or order drifted",
    )

    artifacts = report.get("artifacts")
    require(isinstance(artifacts, dict), "artifact manifest missing")
    require(
        set(artifacts) == set(REQUIRED_ARTIFACTS),
        f"artifact inventory mismatch: {sorted(artifacts)}",
    )
    resolved_artifacts: dict[str, Path] = {}
    for label in REQUIRED_ARTIFACTS:
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
    require(client_hello == EXPECTED_CLIENT_HELLO, "client Hello exchange drifted")
    require(server_hello == EXPECTED_SERVER_HELLO, "server Hello exchange drifted")

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
    require(
        [row.get("action_automation_id") for row in interaction_rows]
        == ["menu.open_media", "menu.exit"],
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
    require(state.get("assertions") == assertions, "state/report assertions diverged")

    validate_observations(
        resolved_artifacts["mpv_observation"], expected_media, mpv["pid"]
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

    return {
        "schema_version": SCHEMA_VERSION,
        "kind": SUMMARY_KIND,
        "result": "passed",
        "capability": "executed",
        "assertion_count": len(REQUIRED_ASSERTIONS),
        "artifact_count": len(REQUIRED_ARTIFACTS),
        "gui_sha256": expected_gui_sha256,
        "mpv_sha256": expected_mpv_sha256,
    }


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
        )
    except (OSError, ValueError) as error:
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
