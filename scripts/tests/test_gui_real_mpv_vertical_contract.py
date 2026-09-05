from __future__ import annotations

import copy
import hashlib
import json
import os
import pathlib
import sys
import tempfile
import unittest
from typing import Any
from unittest import mock

from scripts import gui_real_mpv_vertical_contract as contract


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WRAPPER_PATH = REPO_ROOT / "scripts" / "gui-real-mpv-vertical.ps1"


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def identity(path: pathlib.Path, *, relative_to: pathlib.Path | None = None) -> dict[str, Any]:
    reported_path = path if relative_to is None else path.relative_to(relative_to)
    return {
        "path": str(reported_path),
        "bytes": path.stat().st_size,
        "sha256": sha256(path),
    }


def write_json(path: pathlib.Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def pcm_au_bytes(duration_seconds: int) -> bytes:
    sample_rate = 48_000
    channels = 1
    data_bytes = sample_rate * duration_seconds * channels * 2
    return b"".join(
        (
            b".snd",
            (24).to_bytes(4, byteorder="big"),
            data_bytes.to_bytes(4, byteorder="big"),
            (3).to_bytes(4, byteorder="big"),
            sample_rate.to_bytes(4, byteorder="big"),
            channels.to_bytes(4, byteorder="big"),
            bytes(data_bytes),
        )
    )


def playstate_exchange(
    action: str, paused: bool, position: float
) -> dict[str, Any]:
    request = {
        "State": {
            "playstate": {"position": position, "paused": paused},
            "ping": {"clientLatencyCalculation": 1234.5, "clientRtt": 0.0},
        }
    }
    authoritative_echo = {
        "State": {
            "playstate": {
                "doSeek": False,
                "paused": paused,
                "position": position,
                "setBy": "real-mpv-user",
            }
        }
    }
    return {
        "action": action,
        "mutation_kind": "pause",
        "expected_paused": paused,
        "request": json.dumps(request, separators=(",", ":")),
        "authoritative_echo": json.dumps(
            authoritative_echo, separators=(",", ":")
        ),
    }


def seek_exchange(action: str, paused: bool, position: float) -> dict[str, Any]:
    request = {
        "State": {
            "playstate": {
                "position": position,
                "paused": paused,
                "doSeek": True,
            }
        }
    }
    authoritative_echo = {
        "State": {
            "playstate": {
                "doSeek": True,
                "paused": paused,
                "position": position,
                "setBy": "real-mpv-user",
            }
        }
    }
    return {
        "action": action,
        "mutation_kind": "seek",
        "expected_paused": paused,
        "request": json.dumps(request, separators=(",", ":")),
        "authoritative_echo": json.dumps(
            authoritative_echo, separators=(",", ":")
        ),
    }


def build_valid_fixture(root: pathlib.Path) -> tuple[dict[str, Any], dict[str, Any]]:
    root.mkdir()
    gui = root / "sorotte-gui.exe"
    mpv = root / "mpv.exe"
    gui.write_bytes(b"fresh-sorotte-gui")
    mpv.write_bytes(b"supported-real-mpv")

    gui_pid = 4242
    mpv_pid = 4343
    config = root / "sorotte-real-mpv.ini"
    media = root / "generated-silence.wav"
    observer = root / "observe-real-mpv.lua"
    observations = root / "mpv-observation.jsonl"
    mpv_log = root / "mpv.log"
    lifecycle = root / "gui-lifecycle.jsonl"
    shared_lifecycle = root / "shared-lifecycle-evidence.jsonl"
    session_exchange = root / "session-exchange.json"
    menu_interactions = root / "menu-interactions.json"
    screenshot = root / "success-real-mpv.png"
    state = root / "real-mpv-state.json"
    appdata = root / "appdata"
    appdata.mkdir()

    config.write_text(f"playerPath = {mpv}\nshowOsd = false\n", encoding="utf-8")
    media.write_bytes(b"RIFF\x00\x00\x00\x00WAVEgenerated-local-pcm")
    observer.write_text("-- isolated observer\n", encoding="utf-8")
    observation_rows = [
        {
            "event": "file-loaded",
            "pid": mpv_pid,
            "path": str(media),
            "pause": True,
        },
        {"event": "pause", "pid": mpv_pid, "pause": False},
        {"event": "pause", "pid": mpv_pid, "pause": True},
    ]
    observations.write_text(
        "".join(json.dumps(row) + "\n" for row in observation_rows),
        encoding="utf-8",
    )
    mpv_log.write_text("[status] generated-silence.wav loaded\n", encoding="utf-8")
    lifecycle.write_text('{"event":"app-drop-complete"}\n', encoding="utf-8")
    shared_lifecycle.write_text('{"fixture":"shared-lifecycle"}\n', encoding="utf-8")
    write_json(
        session_exchange,
        {
            "schema_version": 1,
            "kind": contract.SESSION_EXCHANGE_KIND,
            "result": "released",
            "bound_endpoint": "127.0.0.1:45678",
            "connected_peer_endpoint": "127.0.0.1:51234",
            "listener_ipv4_loopback": True,
            "peer_ipv4_loopback": True,
            "client_hello": json.dumps(contract.EXPECTED_CLIENT_HELLO),
            "server_hello": json.dumps(contract.EXPECTED_SERVER_HELLO),
            "advertised_capabilities": list(contract.SESSION_CAPABILITIES),
            "playlist_change_request": json.dumps(
                {"Set": {"playlistChange": {"files": [media.name]}}},
                separators=(",", ":"),
            ),
            "playlist_change_echo": json.dumps(
                {
                    "Set": {
                        "playlistChange": {
                            "files": [media.name],
                            "user": "real-mpv-user",
                        }
                    }
                },
                separators=(",", ":"),
            ),
            "playlist_index_request": json.dumps(
                {"Set": {"playlistIndex": {"index": 0}}},
                separators=(",", ":"),
            ),
            "playlist_index_echo": json.dumps(
                {
                    "Set": {
                        "playlistIndex": {
                            "index": 0,
                            "user": "real-mpv-user",
                        }
                    }
                },
                separators=(",", ":"),
            ),
            "initial_authoritative_playstate": json.dumps(
                {
                    "State": {
                        "playstate": {
                            "doSeek": False,
                            "paused": True,
                            "position": 0.0,
                            "setBy": "real-mpv-user",
                        }
                    }
                },
                separators=(",", ":"),
            ),
            "playstate_exchanges": [
                playstate_exchange("GUI Play canonical transport", False, 0.0),
                playstate_exchange("GUI Pause canonical transport", True, 1.0),
            ],
            "server_thread_released": True,
            "socket_released": True,
            "error": None,
        },
    )
    write_json(
        menu_interactions,
        {
            "schema_version": 1,
            "kind": contract.MENU_INTERACTIONS_KIND,
            "result": "passed",
            "interactions": [
                {
                    "section_automation_id": "menu.section.file",
                    "action_automation_id": action,
                    "section_open_strategy": "physical-section-open",
                    "pre_fallback_snapshots": [],
                    "opened_snapshot": None,
                    "leaf_delivery": "single-exact-physical-click-no-retry",
                    "leaf_delivered": True,
                    "error": None,
                }
                for action in ("menu.open_media", "menu.exit")
            ],
            "error": None,
        },
    )
    screenshot.write_bytes(b"\x89PNG\r\n\x1a\nfixture")
    assertions = list(contract.REQUIRED_ASSERTIONS)
    write_json(
        state,
        {
            "schema_version": 1,
            "kind": contract.REPORT_KIND,
            "result": "passed",
            "stage": "complete",
            "artifact_root": str(root),
            "gui_pid": gui_pid,
            "mpv_pid": mpv_pid,
            "assertions": assertions,
        },
    )

    artifact_paths = {
        "config": config,
        "generated_media": media,
        "observation_script": observer,
        "mpv_observation": observations,
        "mpv_log": mpv_log,
        "gui_lifecycle": lifecycle,
        "shared_lifecycle": shared_lifecycle,
        "session_exchange": session_exchange,
        "menu_interactions": menu_interactions,
        "success_screenshot": screenshot,
        "state": state,
    }
    ipc_endpoint = rf"\\.\pipe\sorotte-gui-mpv-{gui_pid}-fixture"
    report = {
        "schema_version": 1,
        "kind": contract.REPORT_KIND,
        "result": "passed",
        "capability": "executed",
        "gui": identity(gui),
        "mpv": {
            **identity(mpv),
            "version": "mpv v0.41.0-fixture",
            "minimum_supported_version": "0.41.0",
            "pid": mpv_pid,
            "parent_pid": gui_pid,
            "process_image_path": str(mpv),
        },
        "isolation": {
            "artifact_root": str(root),
            "config_path": str(config),
            "appdata_root": str(appdata),
            "media_path": str(media),
            "observation_script_path": str(observer),
            "observation_path": str(observations),
            "mpv_log_path": str(mpv_log),
            "lifecycle_path": str(lifecycle),
            "shared_lifecycle_path": str(shared_lifecycle),
            "session_exchange_path": str(session_exchange),
            "menu_interactions_path": str(menu_interactions),
            "ipc_endpoint": ipc_endpoint,
            "session_endpoint": "127.0.0.1:45678",
            "session_peer_endpoint": "127.0.0.1:51234",
            "session_advertised_capabilities": list(contract.SESSION_CAPABILITIES),
            "network_mode": "os-assigned-ipv4-loopback-session",
            "media_source": "generated-local-pcm-wav",
            "mpv_config": "isolated --no-config",
        },
        "assertions": assertions,
        "artifacts": {
            label: identity(path, relative_to=root)
            for label, path in artifact_paths.items()
        },
        "duration_ms": 123,
    }
    arguments = {
        "artifact_root": root,
        "expected_gui": gui,
        "expected_gui_sha256": sha256(gui),
        "expected_mpv": mpv,
        "expected_mpv_sha256": sha256(mpv),
        "producer_exit_code": 0,
    }
    return report, arguments


def extend_with_owned_mpv_recovery(
    report: dict[str, Any], arguments: dict[str, Any]
) -> None:
    root = pathlib.Path(arguments["artifact_root"])
    observations = root / "mpv-observation.jsonl"
    session_exchange_path = root / "session-exchange.json"
    menu_path = root / "menu-interactions.json"
    state_path = root / "real-mpv-state.json"
    recovery_path = root / "owned-mpv-recovery.json"
    automatic_relaunch_screenshot = root / "owned-mpv-automatic-relaunch.png"
    recovery_screenshot = root / "owned-mpv-recovered.png"
    media = root / "generated-silence.wav"
    initial_pid = report["mpv"]["pid"]
    recovered_pid = initial_pid + 100
    initial_ipc = report["isolation"]["ipc_endpoint"]
    recovered_ipc = initial_ipc + "-replacement"

    rows = [
        json.loads(line)
        for line in observations.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    boundary = len(rows)
    rows.extend(
        [
            {
                "event": "pause",
                "pid": recovered_pid,
                "pause": True,
                "ipc_endpoint": recovered_ipc,
            },
            {
                "event": "file-loaded",
                "pid": recovered_pid,
                "path": str(media),
                "filename": media.name,
                "duration": 12.0,
                "pause": True,
                "ipc_endpoint": recovered_ipc,
            },
            {
                "event": "pause",
                "pid": recovered_pid,
                "pause": False,
                "ipc_endpoint": recovered_ipc,
            },
            {
                "event": "pause",
                "pid": recovered_pid,
                "pause": True,
                "ipc_endpoint": recovered_ipc,
            },
        ]
    )
    observations.write_text(
        "".join(json.dumps(row) + "\n" for row in rows),
        encoding="utf-8",
    )

    session_exchange = json.loads(session_exchange_path.read_text(encoding="utf-8"))
    session_exchange["playstate_exchanges"].extend(
        [
            playstate_exchange(
                "GUI Play on replacement mpv canonical transport", False, 0.0
            ),
            playstate_exchange(
                "GUI Pause on replacement mpv canonical transport", True, 1.0
            ),
        ]
    )
    write_json(session_exchange_path, session_exchange)

    menu = json.loads(menu_path.read_text(encoding="utf-8"))
    menu["interactions"].insert(1, copy.deepcopy(menu["interactions"][0]))
    write_json(menu_path, menu)

    state = json.loads(state_path.read_text(encoding="utf-8"))
    state["recovered_mpv_pid"] = recovered_pid
    state["assertions"] = list(contract.RECOVERY_REQUIRED_ASSERTIONS)
    write_json(state_path, state)

    recovered_mpv = copy.deepcopy(report["mpv"])
    recovered_mpv["pid"] = recovered_pid
    recovery = {
        "schema_version": 1,
        "kind": contract.RECOVERY_KIND,
        "result": "passed",
        "fault": "terminate-exact-attested-gui-owned-mpv",
        "recovery_mode": "active-session-automatic-managed-mpv-relaunch",
        "automatic_relaunch_timeout_ms": 12_000,
        "initial_pid": initial_pid,
        "initial_parent_pid": report["mpv"]["parent_pid"],
        "initial_process_image_path": report["mpv"]["process_image_path"],
        "initial_sha256": report["mpv"]["sha256"],
        "initial_ipc_endpoint": initial_ipc,
        "initial_process_terminated": True,
        "automatic_relaunch_observation_index": boundary,
        "automatic_relaunch_observation_event": "pause",
        "gui_room_remained_active": True,
        "manual_retry_invoked": False,
        "recovered_pid": recovered_pid,
        "recovered_parent_pid": report["mpv"]["parent_pid"],
        "recovered_process_image_path": report["mpv"]["process_image_path"],
        "recovered_sha256": report["mpv"]["sha256"],
        "recovered_ipc_endpoint": recovered_ipc,
        "distinct_pid": True,
        "distinct_ipc_endpoint": True,
        "post_termination_observation_index": boundary,
        "recovered_file_loaded_index": boundary + 1,
        "recovered_playing_index": boundary + 2,
        "recovered_paused_index": boundary + 3,
        "initial_process_still_terminated_after_recovery": True,
        "initial_process_still_terminated_after_gui_exit": True,
        "recovered_process_terminated_after_gui_exit": True,
        "error": None,
    }
    write_json(recovery_path, recovery)
    automatic_relaunch_screenshot.write_bytes(
        b"\x89PNG\r\n\x1a\nowned-mpv-automatic-relaunch"
    )
    recovery_screenshot.write_bytes(b"\x89PNG\r\n\x1a\nowned-mpv-recovered")

    report["assertions"] = list(contract.RECOVERY_REQUIRED_ASSERTIONS)
    report["recovered_mpv"] = recovered_mpv
    report["recovery"] = recovery
    for label, path in {
        "mpv_observation": observations,
        "session_exchange": session_exchange_path,
        "menu_interactions": menu_path,
        "state": state_path,
        "owned_mpv_recovery": recovery_path,
        "automatic_relaunch_screenshot": automatic_relaunch_screenshot,
        "recovery_screenshot": recovery_screenshot,
    }.items():
        report["artifacts"][label] = identity(path, relative_to=root)


def extend_with_faulting_http_recovery(
    report: dict[str, Any], arguments: dict[str, Any]
) -> None:
    root = pathlib.Path(arguments["artifact_root"])
    observations_path = root / "mpv-observation.jsonl"
    state_path = root / "real-mpv-state.json"
    evidence_path = root / "faulting-http-recovery.json"
    media_path = root / "generated-silence.au"
    media_path.write_bytes(pcm_au_bytes(contract.HTTP_FAULT_DURATION_SECONDS))
    endpoint = "127.0.0.1:46800"
    media_url = f"http://{endpoint}{contract.HTTP_FAULT_ROUTE}"
    pid = report["mpv"]["pid"]
    ipc_endpoint = report["isolation"]["ipc_endpoint"]
    session_exchange_path = root / "session-exchange.json"
    session_exchange = json.loads(session_exchange_path.read_text(encoding="utf-8"))
    session_exchange.update(
        {
            "client_hello": json.dumps(contract.EXPECTED_HTTP_FAULT_CLIENT_HELLO),
            "playlist_change_request": json.dumps(
                {"Set": {"playlistChange": {"files": [media_url]}}},
                separators=(",", ":"),
            ),
            "playlist_change_echo": json.dumps(
                {
                    "Set": {
                        "playlistChange": {
                            "files": [media_url],
                            "user": "real-mpv-user",
                        }
                    }
                },
                separators=(",", ":"),
            ),
            "playlist_index_request": json.dumps(
                {"Set": {"playlistIndex": {"index": 0}}},
                separators=(",", ":"),
            ),
            "playlist_index_echo": json.dumps(
                {
                    "Set": {
                        "playlistIndex": {
                            "index": 0,
                            "user": "real-mpv-user",
                        }
                    }
                },
                separators=(",", ":"),
            ),
        }
    )
    session_exchange["playstate_exchanges"][1] = playstate_exchange(
        "GUI Pause after HTTP fault canonical transport", True, 2.0
    )
    write_json(session_exchange_path, session_exchange)

    observations = [
        {
            "event": "file-loaded",
            "pid": pid,
            "path": media_url,
            "filename": "generated-fault.au",
            "duration": 45.0,
            "position": 0.0,
            "pause": True,
            "ipc_endpoint": ipc_endpoint,
            "reason": None,
        },
        {
            "event": "pause",
            "pid": pid,
            "path": media_url,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
        },
        {
            "event": "time-pos",
            "pid": pid,
            "path": media_url,
            "position": 1.0,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
        },
        {
            "event": "eof-reached",
            "pid": pid,
            "path": media_url,
            "duration": 45.0,
            "position": 7.5,
            "pause": True,
            "eof_reached": True,
            "ipc_endpoint": ipc_endpoint,
        },
        {
            "event": "file-loaded",
            "pid": pid,
            "path": media_url,
            "filename": "generated-fault.au",
            "duration": 45.0,
            "position": 1.0,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
            "reason": None,
        },
        {
            "event": "time-pos",
            "pid": pid,
            "path": media_url,
            "position": 2.0,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
        },
        {
            "event": "pause",
            "pid": pid,
            "path": media_url,
            "position": 2.0,
            "pause": True,
            "ipc_endpoint": ipc_endpoint,
        },
    ]
    observations_path.write_text(
        "".join(json.dumps(row) + "\n" for row in observations),
        encoding="utf-8",
    )

    requests = [
        {
            "ordinal": 1,
            "method": "HEAD",
            "path": contract.HTTP_FAULT_ROUTE,
            "peer_endpoint": "127.0.0.1:52001",
            "peer_ipv4_loopback": True,
            "range_header": None,
            "status_code": 200,
            "content_length_header": media_path.stat().st_size,
            "transfer_encoding": None,
            "transmitted_body_bytes": 0,
            "framing_fault_injected": False,
            "disconnected_early": False,
            "write_error": None,
        },
        {
            "ordinal": 2,
            "method": "GET",
            "path": contract.HTTP_FAULT_ROUTE,
            "peer_endpoint": "127.0.0.1:52002",
            "peer_ipv4_loopback": True,
            "range_header": "bytes=0-",
            "status_code": 200,
            "content_length_header": None,
            "transfer_encoding": "chunked",
            "transmitted_body_bytes": contract.HTTP_FAULT_DISCONNECT_AFTER_BYTES,
            "framing_fault_injected": True,
            "disconnected_early": True,
            "write_error": None,
        },
        {
            "ordinal": 3,
            "method": "GET",
            "path": contract.HTTP_FAULT_ROUTE,
            "peer_endpoint": "127.0.0.1:52003",
            "peer_ipv4_loopback": True,
            "range_header": "bytes=0-",
            "status_code": 200,
            "content_length_header": media_path.stat().st_size,
            "transfer_encoding": None,
            "transmitted_body_bytes": media_path.stat().st_size,
            "framing_fault_injected": False,
            "disconnected_early": False,
            "write_error": None,
        },
    ]
    evidence = {
        "schema_version": 1,
        "kind": contract.HTTP_FAULT_KIND,
        "result": "passed",
        "fault": "first-response-malformed-chunk-after-observed-progress-and-playable-prefix-once",
        "recovery_mode": "same-generation-automatic-network-stream-reload",
        "listener_endpoint": endpoint,
        "listener_ipv4_loopback": True,
        "media_url": media_url,
        "route": contract.HTTP_FAULT_ROUTE,
        "generated_media_bytes": media_path.stat().st_size,
        "generated_media_sha256": sha256(media_path),
        "duration_seconds": contract.HTTP_FAULT_DURATION_SECONDS,
        "minimum_body_bytes_before_fault": contract.HTTP_FAULT_DISCONNECT_AFTER_BYTES,
        "request_count": len(requests),
        "premature_disconnect_count": 1,
        "complete_response_count": 1,
        "requests": requests,
        "initial_file_loaded_index": 0,
        "pre_fault_progress_index": 2,
        "fault_triggered_after_progress": True,
        "premature_eof_index": 3,
        "recovered_file_loaded_index": 4,
        "recovered_progress_index": 5,
        "recovered_paused_index": 6,
        "initial_pid": pid,
        "recovered_pid": pid,
        "parent_pid": report["mpv"]["parent_pid"],
        "process_image_path": report["mpv"]["process_image_path"],
        "process_sha256": report["mpv"]["sha256"],
        "initial_ipc_endpoint": ipc_endpoint,
        "recovered_ipc_endpoint": ipc_endpoint,
        "stable_process_identity": True,
        "stable_ipc_endpoint": True,
        "stable_media_url": True,
        "stable_duration": True,
        "pre_fault_position_seconds": 1.0,
        "premature_eof_position_seconds": 7.5,
        "recovered_position_seconds": 2.0,
        "manual_retry_invoked": False,
        "foreign_pid_observations_after_fault": 0,
        "evidence_retained_before_cleanup": True,
        "server_thread_released": True,
        "socket_released": True,
        "owned_mpv_terminated_after_gui_exit": True,
        "error": None,
    }
    write_json(evidence_path, evidence)

    hard_failure_path = root / "hard-media-failure.json"
    hard_failure_endpoint = "127.0.0.1:46802"
    hard_failure_url = (
        f"http://{hard_failure_endpoint}{contract.MEDIA_FAILURE_ROUTE}"
    )
    failure_index = len(observations)
    observations.extend(
        [
            {
                "event": "end-file",
                "pid": pid,
                "path": hard_failure_url,
                "ipc_endpoint": ipc_endpoint,
                "reason": "error",
            },
            {
                "event": "file-loaded",
                "pid": pid,
                "path": str(media_path),
                "filename": media_path.name,
                "duration": float(contract.HTTP_FAULT_DURATION_SECONDS),
                "position": 0.0,
                "pause": True,
                "ipc_endpoint": ipc_endpoint,
            },
        ]
    )
    observations_path.write_text(
        "".join(json.dumps(row) + "\n" for row in observations),
        encoding="utf-8",
    )
    hard_failure_requests = [
        {
            "ordinal": 1,
            "method": "GET",
            "path": contract.MEDIA_FAILURE_ROUTE,
            "peer_endpoint": "127.0.0.1:52004",
            "peer_ipv4_loopback": True,
            "range_header": "bytes=0-",
            "status_code": 404,
            "content_length_header": 0,
            "transfer_encoding": None,
            "transmitted_body_bytes": 0,
            "framing_fault_injected": False,
            "disconnected_early": False,
            "write_error": None,
        }
    ]
    media_failure = {
        "schema_version": 1,
        "kind": contract.MEDIA_FAILURE_KIND,
        "result": "passed",
        "failure_mode": "authoritative-loopback-http-404",
        "recovery_mode": "authoritative-local-media-restore",
        "listener_endpoint": hard_failure_endpoint,
        "listener_ipv4_loopback": True,
        "media_url": hard_failure_url,
        "route": contract.MEDIA_FAILURE_ROUTE,
        "request_count": len(hard_failure_requests),
        "requests": hard_failure_requests,
        "failure_end_file_index": failure_index,
        "failure_reason": "error",
        "media_fail_event_id": "gui-real-mpv.00000020",
        "media_fail_emitter": "gui-real-mpv",
        "media_fail_process_role": "client",
        "restored_file_loaded_index": failure_index + 1,
        "media_playable_event_id": "gui-real-mpv.00000024",
        "media_playable_emitter": "gui-real-mpv",
        "media_playable_process_role": "client",
        "initial_pid": pid,
        "failure_pid": pid,
        "recovered_pid": pid,
        "parent_pid": report["mpv"]["parent_pid"],
        "process_image_path": report["mpv"]["process_image_path"],
        "process_sha256": report["mpv"]["sha256"],
        "initial_ipc_endpoint": ipc_endpoint,
        "failure_ipc_endpoint": ipc_endpoint,
        "recovered_ipc_endpoint": ipc_endpoint,
        "same_process_identity": True,
        "same_ipc_endpoint": True,
        "restored_media_path": str(media_path),
        "restored_media_sha256": sha256(media_path),
        "manual_retry_invoked": False,
        "evidence_retained_before_cleanup": True,
        "server_thread_released": True,
        "socket_released": True,
        "owned_mpv_terminated_after_gui_exit": True,
        "error": None,
    }
    write_json(hard_failure_path, media_failure)

    state = json.loads(state_path.read_text(encoding="utf-8"))
    state["assertions"] = list(contract.HTTP_FAULT_REQUIRED_ASSERTIONS)
    write_json(state_path, state)

    report["assertions"] = list(contract.HTTP_FAULT_REQUIRED_ASSERTIONS)
    report["http_fault"] = evidence
    report["media_failure"] = media_failure
    report["isolation"].update(
        {
            "network_mode": "os-assigned-ipv4-loopback-session-and-http",
            "media_source": "generated-pcm-au-over-faulting-loopback-http",
            "media_path": str(media_path),
            "media_url": media_url,
            "http_endpoint": endpoint,
            "http_evidence_path": str(evidence_path),
        }
    )
    for label, path in {
        "generated_media": media_path,
        "mpv_observation": observations_path,
        "session_exchange": session_exchange_path,
        "state": state_path,
        "faulting_http_recovery": evidence_path,
        "hard_media_failure": hard_failure_path,
    }.items():
        report["artifacts"][label] = identity(path, relative_to=root)


def extend_with_stalled_http(
    report: dict[str, Any], arguments: dict[str, Any]
) -> None:
    root = pathlib.Path(arguments["artifact_root"])
    observations_path = root / "mpv-observation.jsonl"
    state_path = root / "real-mpv-state.json"
    evidence_path = root / "stalled-http.json"
    media_path = root / "generated-silence.au"
    media_path.write_bytes(pcm_au_bytes(contract.HTTP_STALL_DURATION_SECONDS))
    endpoint = "127.0.0.1:46801"
    media_url = f"http://{endpoint}{contract.HTTP_STALL_ROUTE}"
    pid = report["mpv"]["pid"]
    ipc_endpoint = report["isolation"]["ipc_endpoint"]
    session_exchange_path = root / "session-exchange.json"
    session_exchange = json.loads(session_exchange_path.read_text(encoding="utf-8"))
    session_exchange.update(
        {
            "client_hello": json.dumps(contract.EXPECTED_HTTP_FAULT_CLIENT_HELLO),
            "playlist_change_request": json.dumps(
                {"Set": {"playlistChange": {"files": [media_url]}}},
                separators=(",", ":"),
            ),
            "playlist_change_echo": json.dumps(
                {
                    "Set": {
                        "playlistChange": {
                            "files": [media_url],
                            "user": "real-mpv-user",
                        }
                    }
                },
                separators=(",", ":"),
            ),
            "playlist_index_request": json.dumps(
                {"Set": {"playlistIndex": {"index": 0}}},
                separators=(",", ":"),
            ),
            "playlist_index_echo": json.dumps(
                {
                    "Set": {
                        "playlistIndex": {
                            "index": 0,
                            "user": "real-mpv-user",
                        }
                    }
                },
                separators=(",", ":"),
            ),
        }
    )
    session_exchange["playstate_exchanges"][1] = playstate_exchange(
        "GUI Pause after HTTP stall canonical transport", True, 8.2
    )
    write_json(session_exchange_path, session_exchange)

    observations = [
        {
            "event": "file-loaded",
            "pid": pid,
            "path": media_url,
            "filename": "generated-stall.au",
            "duration": 45.0,
            "position": 0.0,
            "pause": True,
            "ipc_endpoint": ipc_endpoint,
            "reason": None,
        },
        {
            "event": "pause",
            "pid": pid,
            "path": media_url,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
        },
        {
            "event": "time-pos",
            "pid": pid,
            "path": media_url,
            "position": 1.0,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
        },
        {
            "event": "paused-for-cache",
            "pid": pid,
            "path": media_url,
            "duration": 45.0,
            "position": 7.5,
            "pause": False,
            "paused_for_cache": True,
            "ipc_endpoint": ipc_endpoint,
        },
        {
            "event": "end-file",
            "pid": pid,
            "path": media_url,
            "duration": 45.0,
            "position": 7.5,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
            "reason": "stop",
        },
        {
            "event": "file-loaded",
            "pid": pid,
            "path": media_url,
            "filename": "generated-stall.au",
            "duration": 45.0,
            "position": 7.5,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
            "reason": None,
        },
        {
            "event": "time-pos",
            "pid": pid,
            "path": media_url,
            "position": 8.2,
            "pause": False,
            "ipc_endpoint": ipc_endpoint,
        },
        {
            "event": "pause",
            "pid": pid,
            "path": media_url,
            "position": 8.2,
            "pause": True,
            "ipc_endpoint": ipc_endpoint,
        },
    ]
    observations_path.write_text(
        "".join(json.dumps(row) + "\n" for row in observations),
        encoding="utf-8",
    )

    request_common = {
        "path": contract.HTTP_STALL_ROUTE,
        "peer_ipv4_loopback": True,
        "status_code": 200,
        "content_length_header": media_path.stat().st_size,
        "transfer_encoding": None,
        "write_error": None,
    }
    requests = [
        {
            **request_common,
            "ordinal": 1,
            "method": "HEAD",
            "peer_endpoint": "127.0.0.1:52101",
            "range_header": None,
            "transmitted_body_bytes": 0,
            "stall_injected": False,
            "stalled_for_ms": None,
            "server_response_retained_at_recovery_get": False,
            "connection_released": True,
            "response_completed": True,
        },
        {
            **request_common,
            "ordinal": 2,
            "method": "GET",
            "peer_endpoint": "127.0.0.1:52102",
            "range_header": "bytes=0-",
            "transmitted_body_bytes": contract.HTTP_STALL_PREFIX_BYTES,
            "stall_injected": True,
            "stalled_for_ms": 30_000,
            "server_response_retained_at_recovery_get": True,
            "connection_released": True,
            "response_completed": False,
        },
        {
            **request_common,
            "ordinal": 3,
            "method": "GET",
            "peer_endpoint": "127.0.0.1:52103",
            "range_header": "bytes=0-",
            "transmitted_body_bytes": media_path.stat().st_size,
            "stall_injected": False,
            "stalled_for_ms": None,
            "server_response_retained_at_recovery_get": False,
            "connection_released": True,
            "response_completed": True,
        },
    ]
    evidence = {
        "schema_version": 1,
        "kind": contract.HTTP_STALL_KIND,
        "result": "passed",
        "schedule": "first-response-valid-prefix-then-open-byte-silence",
        "expected_outcome": (
            "one-bounded-same-generation-reload-after-sustained-cache-pause"
        ),
        "listener_endpoint": endpoint,
        "listener_ipv4_loopback": True,
        "media_url": media_url,
        "route": contract.HTTP_STALL_ROUTE,
        "generated_media_bytes": media_path.stat().st_size,
        "generated_media_sha256": sha256(media_path),
        "duration_seconds": contract.HTTP_STALL_DURATION_SECONDS,
        "prefix_body_bytes": contract.HTTP_STALL_PREFIX_BYTES,
        "prefix_bytes_per_second": contract.HTTP_STALL_PREFIX_BYTES_PER_SECOND,
        "expected_prefix_playable_seconds": (
            contract.HTTP_STALL_EXPECTED_PREFIX_PLAYABLE_SECONDS
        ),
        "cache_stall_position_tolerance_seconds": (
            contract.HTTP_STALL_POSITION_TOLERANCE_SECONDS
        ),
        "minimum_stall_duration_ms": contract.HTTP_STALL_MINIMUM_DURATION_MS,
        "maximum_recovery_wait_ms": contract.HTTP_STALL_MAXIMUM_RECOVERY_WAIT_MS,
        "request_count": len(requests),
        "stalled_response_count": 1,
        "complete_response_count": 1,
        "requests": requests,
        "initial_file_loaded_index": 0,
        "pre_stall_progress_index": 2,
        "cache_stall_index": 3,
        "recovered_file_loaded_index": 5,
        "recovered_progress_index": 6,
        "recovered_paused_index": 7,
        "initial_pid": pid,
        "recovered_pid": pid,
        "parent_pid": report["mpv"]["parent_pid"],
        "process_image_path": report["mpv"]["process_image_path"],
        "process_sha256": report["mpv"]["sha256"],
        "initial_ipc_endpoint": ipc_endpoint,
        "recovered_ipc_endpoint": ipc_endpoint,
        "stable_process_identity": True,
        "stable_ipc_endpoint": True,
        "stable_media_url": True,
        "stable_duration": True,
        "pre_stall_position_seconds": 1.0,
        "cache_stall_position_seconds": 7.5,
        "recovered_position_seconds": 8.2,
        "eof_observations_before_recovery": 0,
        "end_file_observations_before_recovery": 1,
        "manual_retry_invoked": False,
        "foreign_pid_observations_after_stall": 0,
        "evidence_retained_before_cleanup": True,
        "server_thread_released": True,
        "socket_released": True,
        "owned_mpv_terminated_after_gui_exit": True,
        "error": None,
    }
    write_json(evidence_path, evidence)

    state = json.loads(state_path.read_text(encoding="utf-8"))
    state["assertions"] = list(contract.HTTP_STALL_REQUIRED_ASSERTIONS)
    write_json(state_path, state)

    report["assertions"] = list(contract.HTTP_STALL_REQUIRED_ASSERTIONS)
    report["http_stall"] = evidence
    report["isolation"].update(
        {
            "network_mode": "os-assigned-ipv4-loopback-session-and-http",
            "media_source": "generated-pcm-au-over-stalled-loopback-http",
            "media_path": str(media_path),
            "media_url": media_url,
            "http_endpoint": endpoint,
            "http_evidence_path": str(evidence_path),
        }
    )
    for label, path in {
        "generated_media": media_path,
        "mpv_observation": observations_path,
        "session_exchange": session_exchange_path,
        "state": state_path,
        "stalled_http": evidence_path,
    }.items():
        report["artifacts"][label] = identity(path, relative_to=root)


class RealMpvVerticalContractTests(unittest.TestCase):
    def test_accepts_complete_owned_isolated_vertical_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            summary = contract.validate_report(report, **arguments)

        self.assertEqual(summary["result"], "passed")
        self.assertEqual(summary["assertion_count"], len(contract.REQUIRED_ASSERTIONS))
        self.assertEqual(summary["artifact_count"], len(contract.REQUIRED_ARTIFACTS))
        self.assertNotIn("recovery_exercised", summary)

    def test_accepts_authenticated_seek_interleaved_before_canonical_play(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(
                pathlib.Path(temporary) / "artifacts"
            )
            root = pathlib.Path(arguments["artifact_root"])
            session_path = root / "session-exchange.json"
            session = json.loads(session_path.read_text(encoding="utf-8"))
            session["playstate_exchanges"].insert(
                0,
                seek_exchange("GUI Play canonical transport", True, 0.0),
            )
            write_json(session_path, session)
            report["artifacts"]["session_exchange"] = identity(
                session_path, relative_to=root
            )

            summary = contract.validate_report(report, **arguments)

        self.assertEqual(summary["result"], "passed")

    def test_local_media_contract_requires_exact_canonical_playlist_echo(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(
                pathlib.Path(temporary) / "artifacts"
            )
            root = pathlib.Path(arguments["artifact_root"])
            session_path = root / "session-exchange.json"
            session = json.loads(session_path.read_text(encoding="utf-8"))
            request = json.loads(session["playlist_change_request"])
            request["Set"]["playlistChange"]["files"] = [str(root / "other.wav")]
            session["playlist_change_request"] = json.dumps(request)
            write_json(session_path, session)
            report["artifacts"]["session_exchange"] = identity(
                session_path, relative_to=root
            )

            with self.assertRaisesRegex(
                ValueError, "playlistChange request drifted from the exact closed request schema"
            ):
                contract.validate_report(report, **arguments)

    def test_session_transport_contract_rejects_incomplete_or_forged_playstate_proof(
        self,
    ) -> None:
        def missing_pause(session: dict[str, Any]) -> None:
            session["playstate_exchanges"].pop()

        def reversed_edges(session: dict[str, Any]) -> None:
            session["playstate_exchanges"].reverse()

        def forged_echo(session: dict[str, Any]) -> None:
            echo = json.loads(
                session["playstate_exchanges"][0]["authoritative_echo"]
            )
            echo["State"]["playstate"]["setBy"] = "mallory"
            session["playstate_exchanges"][0]["authoritative_echo"] = json.dumps(
                echo, separators=(",", ":")
            )

        def expanded_request_schema(session: dict[str, Any]) -> None:
            request = json.loads(session["playstate_exchanges"][0]["request"])
            request["State"]["playstate"]["doSeek"] = False
            session["playstate_exchanges"][0]["request"] = json.dumps(
                request, separators=(",", ":")
            )

        def unpaused_initial_authority(session: dict[str, Any]) -> None:
            initial = json.loads(session["initial_authoritative_playstate"])
            initial["State"]["playstate"]["paused"] = False
            session["initial_authoritative_playstate"] = json.dumps(
                initial, separators=(",", ":")
            )

        cases = [
            (missing_pause, "canonical Play/Pause exchange inventory drifted"),
            (reversed_edges, "playstate action order drifted"),
            (forged_echo, "did not authenticate the exact mutation"),
            (expanded_request_schema, "request playstate schema drifted"),
            (unpaused_initial_authority, "initial authoritative paused playstate drifted"),
        ]
        for index, (mutate, expected_error) in enumerate(cases):
            with self.subTest(case=index), tempfile.TemporaryDirectory() as temporary:
                report, arguments = build_valid_fixture(
                    pathlib.Path(temporary) / "artifacts"
                )
                root = pathlib.Path(arguments["artifact_root"])
                session_path = root / "session-exchange.json"
                session = json.loads(session_path.read_text(encoding="utf-8"))
                mutate(session)
                write_json(session_path, session)
                report["artifacts"]["session_exchange"] = identity(
                    session_path, relative_to=root
                )

                with self.assertRaisesRegex(ValueError, expected_error):
                    contract.validate_report(report, **arguments)

    def test_accepts_separate_exact_owned_mpv_recovery_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            extend_with_owned_mpv_recovery(report, arguments)
            summary = contract.validate_report(
                report,
                **arguments,
                expect_recovery=True,
            )

        self.assertEqual(summary["result"], "passed")
        self.assertTrue(summary["recovery_exercised"])
        self.assertEqual(
            summary["assertion_count"], len(contract.RECOVERY_REQUIRED_ASSERTIONS)
        )
        self.assertEqual(
            summary["artifact_count"], len(contract.RECOVERY_REQUIRED_ARTIFACTS)
        )

    def test_accepts_native_faulting_http_same_process_recovery_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            extend_with_faulting_http_recovery(report, arguments)
            summary = contract.validate_report(
                report,
                **arguments,
                expect_http_fault=True,
            )

        self.assertEqual(summary["result"], "passed")
        self.assertTrue(summary["http_fault_exercised"])
        self.assertTrue(summary["media_failure_recovery_exercised"])
        self.assertEqual(
            summary["assertion_count"], len(contract.HTTP_FAULT_REQUIRED_ASSERTIONS)
        )
        self.assertEqual(
            summary["artifact_count"], len(contract.HTTP_FAULT_REQUIRED_ARTIFACTS)
        )

    def test_accepts_native_stalled_http_same_process_recovery_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            extend_with_stalled_http(report, arguments)
            summary = contract.validate_report(
                report,
                **arguments,
                expect_http_stall=True,
            )

        self.assertEqual(summary["result"], "passed")
        self.assertTrue(summary["http_stall_exercised"])
        self.assertEqual(
            summary["assertion_count"], len(contract.HTTP_STALL_REQUIRED_ASSERTIONS)
        )
        self.assertEqual(
            summary["artifact_count"], len(contract.HTTP_STALL_REQUIRED_ARTIFACTS)
        )

    def test_hard_media_failure_contract_rejects_weakened_causality_or_identity(
        self,
    ) -> None:
        mutations = (
            (
                "non-404 response",
                lambda evidence: evidence["requests"][0].__setitem__(
                    "status_code", 200
                ),
                "exact bodyless 404 contract",
            ),
            (
                "different recovery process",
                lambda evidence: evidence.__setitem__(
                    "recovered_pid", evidence["initial_pid"] + 1
                ),
                "changed the attested GUI-owned process",
            ),
            (
                "reused lifecycle event",
                lambda evidence: evidence.__setitem__(
                    "media_playable_event_id", evidence["media_fail_event_id"]
                ),
                "MEDIA-PLAYABLE-001 lifecycle attribution drifted",
            ),
            (
                "manual recovery",
                lambda evidence: evidence.__setitem__("manual_retry_invoked", True),
                "manual retry",
            ),
            (
                "unreleased socket",
                lambda evidence: evidence.__setitem__("socket_released", False),
                "release",
            ),
        )
        for label, mutate, error_pattern in mutations:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                report, arguments = build_valid_fixture(
                    pathlib.Path(temporary) / "artifacts"
                )
                extend_with_faulting_http_recovery(report, arguments)
                mutate(report["media_failure"])
                root = pathlib.Path(arguments["artifact_root"])
                evidence_path = root / "hard-media-failure.json"
                write_json(evidence_path, report["media_failure"])
                report["artifacts"]["hard_media_failure"] = identity(
                    evidence_path, relative_to=root
                )
                with self.assertRaisesRegex(ValueError, error_pattern):
                    contract.validate_report(
                        report,
                        **arguments,
                        expect_http_fault=True,
                    )

    def test_capability_modes_are_pairwise_mutually_exclusive(self) -> None:
        mode_pairs = (
            {"expect_recovery": True, "expect_http_fault": True},
            {"expect_recovery": True, "expect_http_stall": True},
            {"expect_http_fault": True, "expect_http_stall": True},
        )
        for modes in mode_pairs:
            with self.subTest(modes=modes), tempfile.TemporaryDirectory() as temporary:
                report, arguments = build_valid_fixture(
                    pathlib.Path(temporary) / "artifacts"
                )
                with self.assertRaisesRegex(ValueError, "mutually exclusive"):
                    contract.validate_report(report, **arguments, **modes)

    def test_stalled_http_contract_rejects_invalid_boundary_trace_and_cleanup(
        self,
    ) -> None:
        def insert_premature_eof(
            report: dict[str, Any], rows: list[dict[str, Any]]
        ) -> None:
            eof = copy.deepcopy(rows[3])
            eof.update({"event": "eof-reached", "eof_reached": True})
            rows.insert(4, eof)
            report["http_stall"].update(
                {
                    "recovered_file_loaded_index": 6,
                    "recovered_progress_index": 7,
                    "recovered_paused_index": 8,
                }
            )

        def remove_replacement_end_file(
            report: dict[str, Any], rows: list[dict[str, Any]]
        ) -> None:
            rows.pop(4)
            report["http_stall"].update(
                {
                    "recovered_file_loaded_index": 4,
                    "recovered_progress_index": 5,
                    "recovered_paused_index": 6,
                    "end_file_observations_before_recovery": 0,
                }
            )

        def insert_intervening_file_loaded(
            report: dict[str, Any], rows: list[dict[str, Any]]
        ) -> None:
            rows.insert(5, copy.deepcopy(rows[5]))
            report["http_stall"].update(
                {
                    "recovered_file_loaded_index": 6,
                    "recovered_progress_index": 7,
                    "recovered_paused_index": 8,
                }
            )

        mutations = (
            (
                "too-short server-side silence",
                lambda report, rows: report["http_stall"]["requests"][1].__setitem__(
                    "stalled_for_ms", contract.HTTP_STALL_MINIMUM_DURATION_MS - 1
                ),
                "exact bounded open byte-silent response",
            ),
            (
                "boolean stall duration",
                lambda report, rows: report["http_stall"]["requests"][1].__setitem__(
                    "stalled_for_ms", True
                ),
                "exact bounded open byte-silent response",
            ),
            (
                "framing corruption substituted for valid response",
                lambda report, rows: report["http_stall"]["requests"][1].__setitem__(
                    "transfer_encoding", "chunked"
                ),
                "framing or write accounting drifted",
            ),
            (
                "server response released before recovery request",
                lambda report, rows: report["http_stall"]["requests"][1].__setitem__(
                    "server_response_retained_at_recovery_get", False
                ),
                "exact bounded open byte-silent response",
            ),
            (
                "stalled connection not released at cleanup",
                lambda report, rows: report["http_stall"]["requests"][1].__setitem__(
                    "connection_released", False
                ),
                "exact bounded open byte-silent response",
            ),
            (
                "extra media GET",
                lambda report, rows: report["http_stall"]["requests"].append(
                    {
                        **copy.deepcopy(report["http_stall"]["requests"][-1]),
                        "ordinal": 4,
                        "peer_endpoint": "127.0.0.1:52104",
                    }
                ),
                "exactly one open stalled GET and one complete GET",
            ),
            (
                "premature EOF",
                insert_premature_eof,
                "observed EOF before the recovery load",
            ),
            (
                "missing cache pause",
                lambda report, rows: rows[3].__setitem__("paused_for_cache", False),
                "exact cache-stall observation",
            ),
            (
                "cache pause duration drift",
                lambda report, rows: rows[3].__setitem__("duration", 44.0),
                "exact cache-stall observation",
            ),
            (
                "cache pause embedded EOF",
                lambda report, rows: rows[3].__setitem__("eof_reached", True),
                "exact cache-stall observation",
            ),
            (
                "cache pause below deterministic prefix boundary",
                lambda report, rows: (
                    report["http_stall"].__setitem__(
                        "cache_stall_position_seconds",
                        contract.HTTP_STALL_EXPECTED_PREFIX_PLAYABLE_SECONDS
                        - contract.HTTP_STALL_POSITION_TOLERANCE_SECONDS
                        - 0.01,
                    ),
                    rows[3].__setitem__(
                        "position",
                        contract.HTTP_STALL_EXPECTED_PREFIX_PLAYABLE_SECONDS
                        - contract.HTTP_STALL_POSITION_TOLERANCE_SECONDS
                        - 0.01,
                    ),
                ),
                "bounded positive playback progress",
            ),
            (
                "cache pause above deterministic prefix boundary",
                lambda report, rows: (
                    report["http_stall"].__setitem__(
                        "cache_stall_position_seconds",
                        contract.HTTP_STALL_EXPECTED_PREFIX_PLAYABLE_SECONDS
                        + contract.HTTP_STALL_POSITION_TOLERANCE_SECONDS
                        + 0.01,
                    ),
                    rows[3].__setitem__(
                        "position",
                        contract.HTTP_STALL_EXPECTED_PREFIX_PLAYABLE_SECONDS
                        + contract.HTTP_STALL_POSITION_TOLERANCE_SECONDS
                        + 0.01,
                    ),
                ),
                "bounded positive playback progress",
            ),
            (
                "unexpected end-file reason",
                lambda report, rows: rows[4].__setitem__("reason", "error"),
                "exactly one same-process end-file stop",
            ),
            (
                "missing replacement end-file",
                remove_replacement_end_file,
                "exactly one same-process end-file stop",
            ),
            (
                "intervening same-process file-loaded",
                insert_intervening_file_loaded,
                "unidentified or intervening lifecycle row",
            ),
            (
                "recovery did not pass stall position",
                lambda report, rows: (
                    report["http_stall"].__setitem__(
                        "recovered_position_seconds", 7.9
                    ),
                    rows[6].__setitem__("position", 7.9),
                ),
                "bounded positive playback progress",
            ),
            (
                "foreign generation",
                lambda report, rows: rows[6].__setitem__(
                    "pid", report["mpv"]["pid"] + 1
                ),
                "stale, or foreign mpv generation",
            ),
            (
                "unidentified generation",
                lambda report, rows: rows[6].__setitem__("pid", None),
                "unidentified, stale, or foreign mpv generation",
            ),
            (
                "incomplete server cleanup",
                lambda report, rows: report["http_stall"].__setitem__(
                    "server_thread_released", False
                ),
                "release, or cleanup attestation was incomplete",
            ),
            (
                "boolean prefix size",
                lambda report, rows: report["http_stall"].__setitem__(
                    "prefix_body_bytes", True
                ),
                "playable-prefix boundary drifted",
            ),
        )
        for label, mutate, error_pattern in mutations:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                report, arguments = build_valid_fixture(
                    pathlib.Path(temporary) / "artifacts"
                )
                extend_with_stalled_http(report, arguments)
                root = pathlib.Path(arguments["artifact_root"])
                observations_path = root / "mpv-observation.jsonl"
                rows = [
                    json.loads(line)
                    for line in observations_path.read_text(encoding="utf-8").splitlines()
                ]
                mutate(report, rows)
                observations_path.write_text(
                    "".join(json.dumps(row) + "\n" for row in rows),
                    encoding="utf-8",
                )
                evidence_path = root / "stalled-http.json"
                report["http_stall"]["request_count"] = len(
                    report["http_stall"]["requests"]
                )
                write_json(evidence_path, report["http_stall"])
                report["artifacts"]["mpv_observation"] = identity(
                    observations_path, relative_to=root
                )
                report["artifacts"]["stalled_http"] = identity(
                    evidence_path, relative_to=root
                )
                with self.assertRaisesRegex(ValueError, error_pattern):
                    contract.validate_report(
                        report,
                        **arguments,
                        expect_http_stall=True,
                    )

    def test_http_fault_contract_rejects_cache_path_extra_get_and_foreign_generation(
        self,
    ) -> None:
        mutations = (
            (
                "fault released before progress",
                lambda report, rows: report["http_fault"].__setitem__(
                    "fault_triggered_after_progress", False
                ),
                "causally released after observed playback progress",
            ),
            (
                "boolean observation index",
                lambda report, rows: report["http_fault"].__setitem__(
                    "pre_fault_progress_index", True
                ),
                "observation indices were invalid",
            ),
            (
                "boolean retained position",
                lambda report, rows: report["http_fault"].__setitem__(
                    "pre_fault_position_seconds", True
                ),
                "bounded positive playback progress",
            ),
            (
                "boolean observed position",
                lambda report, rows: rows[2].__setitem__("position", True),
                "pre-fault progress observation mismatch",
            ),
            (
                "self-attested pre-fault position",
                lambda report, rows: rows[2].__setitem__("position", 1.25),
                "pre-fault progress observation mismatch",
            ),
            (
                "missing keep-open EOF",
                lambda report, rows: rows[3].__setitem__("eof_reached", False),
                "expected keep-open premature EOF",
            ),
            (
                "self-attested premature EOF position",
                lambda report, rows: rows[3].__setitem__("position", 8.0),
                "premature EOF position observation mismatch",
            ),
            (
                "downloaded cache path",
                lambda report, rows: rows[4].__setitem__(
                    "path",
                    str(
                        pathlib.Path(report["isolation"]["artifact_root"])
                        / "downloaded-cache.wav"
                    ),
                ),
                "cache path",
            ),
            (
                "extra libav retry",
                lambda report, rows: report["http_fault"]["requests"].append(
                    {
                        **copy.deepcopy(report["http_fault"]["requests"][-1]),
                        "ordinal": 4,
                        "peer_endpoint": "127.0.0.1:52004",
                    }
                ),
                "exactly one malformed chunked GET and one complete GET",
            ),
            (
                "non-range media response",
                lambda report, rows: report["http_fault"]["requests"][1].update(
                    {"range_header": None, "status_code": 200}
                ),
                "byte-zero non-seekable contract",
            ),
            (
                "foreign generation",
                lambda report, rows: rows[5].__setitem__(
                    "pid", report["mpv"]["pid"] + 1
                ),
                "stale or foreign mpv generation",
            ),
            (
                "recovered progress URL drift",
                lambda report, rows: rows[5].__setitem__(
                    "path", "http://127.0.0.1:46800/not-generated.wav"
                ),
                "recovered progress observation mismatch",
            ),
            (
                "recovered pause URL drift",
                lambda report, rows: rows[6].__setitem__(
                    "path", "http://127.0.0.1:46800/not-generated.wav"
                ),
                "recovered pause observation mismatch",
            ),
        )
        for label, mutate, error_pattern in mutations:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                report, arguments = build_valid_fixture(
                    pathlib.Path(temporary) / "artifacts"
                )
                extend_with_faulting_http_recovery(report, arguments)
                root = pathlib.Path(arguments["artifact_root"])
                observations_path = root / "mpv-observation.jsonl"
                rows = [
                    json.loads(line)
                    for line in observations_path.read_text(encoding="utf-8").splitlines()
                ]
                mutate(report, rows)
                observations_path.write_text(
                    "".join(json.dumps(row) + "\n" for row in rows),
                    encoding="utf-8",
                )
                evidence_path = root / "faulting-http-recovery.json"
                report["http_fault"]["request_count"] = len(
                    report["http_fault"]["requests"]
                )
                write_json(evidence_path, report["http_fault"])
                report["artifacts"]["mpv_observation"] = identity(
                    observations_path, relative_to=root
                )
                report["artifacts"]["faulting_http_recovery"] = identity(
                    evidence_path, relative_to=root
                )
                with self.assertRaisesRegex(ValueError, error_pattern):
                    contract.validate_report(
                        report,
                        **arguments,
                        expect_http_fault=True,
                    )

    def test_http_fault_contract_rejects_tampered_session_playlist_echo(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(
                pathlib.Path(temporary) / "artifacts"
            )
            extend_with_faulting_http_recovery(report, arguments)
            root = pathlib.Path(arguments["artifact_root"])
            session_path = root / "session-exchange.json"
            session = json.loads(session_path.read_text(encoding="utf-8"))
            session["playlist_change_echo"] = json.dumps(
                {
                    "Set": {
                        "playlistChange": {
                            "files": ["http://127.0.0.1:46800/not-generated.wav"],
                            "user": "real-mpv-user",
                        }
                    }
                }
            )
            write_json(session_path, session)
            report["artifacts"]["session_exchange"] = identity(
                session_path, relative_to=root
            )

            with self.assertRaisesRegex(
                ValueError, "authoritative playlistChange echo drifted"
            ):
                contract.validate_report(
                    report,
                    **arguments,
                    expect_http_fault=True,
                )

    def test_http_fault_contract_rejects_extra_field_session_playlist_request(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(
                pathlib.Path(temporary) / "artifacts"
            )
            extend_with_faulting_http_recovery(report, arguments)
            root = pathlib.Path(arguments["artifact_root"])
            session_path = root / "session-exchange.json"
            session = json.loads(session_path.read_text(encoding="utf-8"))
            request = json.loads(session["playlist_change_request"])
            request["Set"]["playlistChange"]["unexpected"] = True
            session["playlist_change_request"] = json.dumps(request)
            write_json(session_path, session)
            report["artifacts"]["session_exchange"] = identity(
                session_path, relative_to=root
            )

            with self.assertRaisesRegex(ValueError, "exact closed request schema"):
                contract.validate_report(
                    report,
                    **arguments,
                    expect_http_fault=True,
                )

    def test_http_fault_contract_rejects_extra_field_session_playlist_index_request(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(
                pathlib.Path(temporary) / "artifacts"
            )
            extend_with_faulting_http_recovery(report, arguments)
            root = pathlib.Path(arguments["artifact_root"])
            session_path = root / "session-exchange.json"
            session = json.loads(session_path.read_text(encoding="utf-8"))
            request = json.loads(session["playlist_index_request"])
            request["Set"]["playlistIndex"]["unexpected"] = True
            session["playlist_index_request"] = json.dumps(request)
            write_json(session_path, session)
            report["artifacts"]["session_exchange"] = identity(
                session_path, relative_to=root
            )

            with self.assertRaisesRegex(
                ValueError, "playlistIndex request drifted from the exact closed request schema"
            ):
                contract.validate_report(
                    report,
                    **arguments,
                    expect_http_fault=True,
                )

    def test_main_retains_error_summary_for_malformed_http_numeric_type(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary) / "artifacts"
            report, arguments = build_valid_fixture(root)
            extend_with_faulting_http_recovery(report, arguments)
            observations_path = root / "mpv-observation.jsonl"
            rows = [
                json.loads(line)
                for line in observations_path.read_text(encoding="utf-8").splitlines()
            ]
            rows[4]["duration"] = {"malformed": True}
            observations_path.write_text(
                "".join(json.dumps(row) + "\n" for row in rows),
                encoding="utf-8",
            )
            report["artifacts"]["mpv_observation"] = identity(
                observations_path, relative_to=root
            )
            report_path = root / "harness-report.json"
            summary_path = root / "contract-summary.json"
            write_json(report_path, report)
            argv = [
                "gui_real_mpv_vertical_contract.py",
                "--report",
                str(report_path),
                "--artifact-dir",
                str(root),
                "--expected-gui",
                str(arguments["expected_gui"]),
                "--expected-gui-sha256",
                arguments["expected_gui_sha256"],
                "--expected-mpv",
                str(arguments["expected_mpv"]),
                "--expected-mpv-sha256",
                arguments["expected_mpv_sha256"],
                "--producer-exit-code",
                str(arguments["producer_exit_code"]),
                "--summary",
                str(summary_path),
                "--expect-http-fault",
            ]
            with mock.patch.object(sys, "argv", argv):
                self.assertEqual(contract.main(), 1)
            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            self.assertEqual(summary["result"], "error")
            self.assertIn("float()", summary["error"])

    def test_http_fault_contract_rejects_non_loopback_and_incomplete_release(self) -> None:
        mutations = (
            (
                "external listener",
                lambda report: report["http_fault"].__setitem__(
                    "listener_endpoint", "192.0.2.1:46800"
                ),
                "not bound to IPv4 loopback",
            ),
            (
                "thread retained",
                lambda report: report["http_fault"].__setitem__(
                    "server_thread_released", False
                ),
                "release",
            ),
            (
                "second short response",
                lambda report: report["http_fault"]["requests"][2].update(
                    {
                        "transmitted_body_bytes": 100,
                        "disconnected_early": True,
                    }
                ),
                "complete recovery response",
            ),
            (
                "partial response write",
                lambda report: report["http_fault"]["requests"][1].__setitem__(
                    "write_error", "connection reset"
                ),
                "write did not complete cleanly",
            ),
        )
        for label, mutate, error_pattern in mutations:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                report, arguments = build_valid_fixture(
                    pathlib.Path(temporary) / "artifacts"
                )
                extend_with_faulting_http_recovery(report, arguments)
                mutate(report)
                root = pathlib.Path(arguments["artifact_root"])
                evidence_path = root / "faulting-http-recovery.json"
                write_json(evidence_path, report["http_fault"])
                report["artifacts"]["faulting_http_recovery"] = identity(
                    evidence_path, relative_to=root
                )
                with self.assertRaisesRegex(ValueError, error_pattern):
                    contract.validate_report(
                        report,
                        **arguments,
                        expect_http_fault=True,
                    )

    def test_recovery_contract_rejects_reused_identity_and_incomplete_cleanup(self) -> None:
        mutations = (
            (
                "reused PID",
                lambda report: report["recovered_mpv"].__setitem__(
                    "pid", report["mpv"]["pid"]
                ),
                "distinct positive process",
            ),
            (
                "reused IPC",
                lambda report: report["recovery"].__setitem__(
                    "recovered_ipc_endpoint", report["recovery"]["initial_ipc_endpoint"]
                ),
                "distinct and GUI-owned",
            ),
            (
                "incomplete cleanup",
                lambda report: report["recovery"].__setitem__(
                    "recovered_process_terminated_after_gui_exit", False
                ),
                "final process cleanup was incomplete",
            ),
            (
                "manual retry substituted for automatic recovery",
                lambda report: report["recovery"].__setitem__(
                    "manual_retry_invoked", True
                ),
                "unexpectedly required manual retry",
            ),
        )
        for label, mutate, error_pattern in mutations:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                report, arguments = build_valid_fixture(
                    pathlib.Path(temporary) / "artifacts"
                )
                extend_with_owned_mpv_recovery(report, arguments)
                mutate(report)
                recovery_path = (
                    pathlib.Path(arguments["artifact_root"]) / "owned-mpv-recovery.json"
                )
                write_json(recovery_path, report["recovery"])
                report["artifacts"]["owned_mpv_recovery"] = identity(
                    recovery_path,
                    relative_to=pathlib.Path(arguments["artifact_root"]),
                )
                with self.assertRaisesRegex(ValueError, error_pattern):
                    contract.validate_report(
                        report,
                        **arguments,
                        expect_recovery=True,
                    )

    def test_recovery_contract_requires_media_open_after_relaunch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary) / "artifacts"
            report, arguments = build_valid_fixture(root)
            extend_with_owned_mpv_recovery(report, arguments)
            menu_path = root / "menu-interactions.json"
            menu = json.loads(menu_path.read_text(encoding="utf-8"))
            menu["interactions"].pop(1)
            write_json(menu_path, menu)
            report["artifacts"]["menu_interactions"] = identity(menu_path, relative_to=root)
            with self.assertRaisesRegex(ValueError, "menu action inventory or order drifted"):
                contract.validate_report(report, **arguments, expect_recovery=True)

    def test_recovery_contract_rejects_old_generation_after_termination_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            extend_with_owned_mpv_recovery(report, arguments)
            observations = (
                pathlib.Path(arguments["artifact_root"]) / "mpv-observation.jsonl"
            )
            rows = [
                json.loads(line)
                for line in observations.read_text(encoding="utf-8").splitlines()
            ]
            boundary = report["recovery"]["post_termination_observation_index"]
            rows.insert(
                boundary,
                {
                    "event": "pause",
                    "pid": report["mpv"]["pid"],
                    "pause": True,
                    "ipc_endpoint": report["recovery"]["initial_ipc_endpoint"],
                },
            )
            observations.write_text(
                "".join(json.dumps(row) + "\n" for row in rows),
                encoding="utf-8",
            )
            report["artifacts"]["mpv_observation"] = identity(
                observations,
                relative_to=pathlib.Path(arguments["artifact_root"]),
            )
            with self.assertRaisesRegex(ValueError, "stale or foreign mpv generation"):
                contract.validate_report(
                    report,
                    **arguments,
                    expect_recovery=True,
                )

    def test_rejects_nonzero_producer_and_tampered_binary_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            failed_producer = dict(arguments, producer_exit_code=9)
            with self.assertRaisesRegex(ValueError, "producer exited 9"):
                contract.validate_report(report, **failed_producer)

            tampered = copy.deepcopy(report)
            tampered["mpv"]["sha256"] = "0" * 64
            with self.assertRaisesRegex(ValueError, "mpv binary digest mismatch"):
                contract.validate_report(tampered, **arguments)

    def test_rejects_client_identity_capability_and_server_version_drift(self) -> None:
        mutations = (
            (
                "client identity",
                "client_hello",
                contract.EXPECTED_CLIENT_HELLO,
                lambda hello: hello["Hello"].__setitem__(
                    "username", "mutated-real-mpv-user"
                ),
                "client Hello exchange drifted",
            ),
            (
                "client capability",
                "client_hello",
                contract.EXPECTED_CLIENT_HELLO,
                lambda hello: hello["Hello"]["features"].__setitem__(
                    "sharedPlaylists", False
                ),
                "client Hello exchange drifted",
            ),
            (
                "large-frame capability",
                "client_hello",
                contract.EXPECTED_CLIENT_HELLO,
                lambda hello: hello["Hello"]["features"].pop("sorotteLargeProtocolFramesV1"),
                "client Hello exchange drifted",
            ),
            (
                "server version",
                "server_hello",
                contract.EXPECTED_SERVER_HELLO,
                lambda hello: hello["Hello"].__setitem__("version", "1.7.4"),
                "server Hello exchange drifted",
            ),
        )
        for label, exchange_key, expected, mutate, error_pattern in mutations:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                report, arguments = build_valid_fixture(
                    pathlib.Path(temporary) / "artifacts"
                )
                exchange_path = (
                    pathlib.Path(arguments["artifact_root"]) / "session-exchange.json"
                )
                exchange = json.loads(exchange_path.read_text(encoding="utf-8"))
                mutated_hello = copy.deepcopy(expected)
                mutate(mutated_hello)
                exchange[exchange_key] = json.dumps(mutated_hello)
                write_json(exchange_path, exchange)
                report["artifacts"]["session_exchange"] = identity(
                    exchange_path,
                    relative_to=pathlib.Path(arguments["artifact_root"]),
                )

                with self.assertRaisesRegex(ValueError, error_pattern):
                    contract.validate_report(report, **arguments)

    def test_rejects_extra_and_reordered_assertions(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")

            extra = copy.deepcopy(report)
            extra["assertions"].append("unexpected-extra-assertion")
            with self.assertRaisesRegex(ValueError, "assertion inventory or order drifted"):
                contract.validate_report(extra, **arguments)

            reordered = copy.deepcopy(report)
            reordered["assertions"][0], reordered["assertions"][1] = (
                reordered["assertions"][1],
                reordered["assertions"][0],
            )
            with self.assertRaisesRegex(ValueError, "assertion inventory or order drifted"):
                contract.validate_report(reordered, **arguments)

    @unittest.skipUnless(os.name == "nt", "Windows extended path equivalence")
    def test_accepts_only_equivalent_absolute_windows_extended_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")

            def extended(path: pathlib.Path) -> str:
                return "\\\\?\\" + str(path)

            equivalent = copy.deepcopy(report)
            equivalent["gui"]["path"] = extended(arguments["expected_gui"])
            equivalent["mpv"]["path"] = extended(arguments["expected_mpv"])
            equivalent["mpv"]["process_image_path"] = extended(arguments["expected_mpv"])
            for key in (
                "artifact_root",
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
                equivalent["isolation"][key] = extended(
                    pathlib.Path(equivalent["isolation"][key])
                )
            self.assertEqual(
                contract.validate_report(equivalent, **arguments)["result"], "passed"
            )

            different = copy.deepcopy(equivalent)
            different["gui"]["path"] = extended(
                pathlib.Path(arguments["artifact_root"]) / "different-gui.exe"
            )
            with self.assertRaisesRegex(ValueError, "GUI binary path mismatch"):
                contract.validate_report(different, **arguments)

            relative = copy.deepcopy(report)
            relative["gui"]["path"] = "sorotte-gui.exe"
            with self.assertRaisesRegex(ValueError, "path must be absolute"):
                contract.validate_report(relative, **arguments)

    def test_rejects_unowned_ipc_and_out_of_order_real_state(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            unowned = copy.deepcopy(report)
            unowned["isolation"]["ipc_endpoint"] = (
                r"\\.\pipe\sorotte-gui-mpv-9999-unowned"
            )
            with self.assertRaisesRegex(ValueError, "not bound to the GUI process"):
                contract.validate_report(unowned, **arguments)

            observations = (
                pathlib.Path(arguments["artifact_root"]) / "mpv-observation.jsonl"
            )
            observations.write_text(
                "\n".join(
                    [
                        json.dumps({"event": "pause", "pid": 4343, "pause": False}),
                        json.dumps(
                            {
                                "event": "file-loaded",
                                "pid": 4343,
                                "path": str(
                                    pathlib.Path(arguments["artifact_root"])
                                    / "generated-silence.wav"
                                ),
                                "pause": True,
                            }
                        ),
                        json.dumps({"event": "pause", "pid": 4343, "pause": True}),
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            report["artifacts"]["mpv_observation"] = identity(
                observations, relative_to=pathlib.Path(arguments["artifact_root"])
            )
            with self.assertRaisesRegex(ValueError, "pause=false observation after load"):
                contract.validate_report(report, **arguments)

    def test_rejects_non_loopback_peer_and_unreleased_session_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            external_peer = copy.deepcopy(report)
            external_peer["isolation"]["session_peer_endpoint"] = "192.0.2.1:51234"
            with self.assertRaisesRegex(ValueError, "not bound to IPv4 loopback"):
                contract.validate_report(external_peer, **arguments)

            exchange_path = (
                pathlib.Path(arguments["artifact_root"]) / "session-exchange.json"
            )
            exchange = json.loads(exchange_path.read_text(encoding="utf-8"))
            exchange["result"] = "running"
            exchange["server_thread_released"] = False
            exchange["socket_released"] = False
            write_json(exchange_path, exchange)
            report["artifacts"]["session_exchange"] = identity(
                exchange_path, relative_to=pathlib.Path(arguments["artifact_root"])
            )
            with self.assertRaisesRegex(ValueError, "session was not released"):
                contract.validate_report(report, **arguments)

    def test_accepts_only_two_snapshot_menu_section_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report, arguments = build_valid_fixture(pathlib.Path(temporary) / "artifacts")
            menu_path = pathlib.Path(arguments["artifact_root"]) / "menu-interactions.json"
            menu = json.loads(menu_path.read_text(encoding="utf-8"))
            menu["interactions"][0]["section_open_strategy"] = (
                "uia-section-open-after-two-hidden-snapshots"
            )
            menu["interactions"][0]["pre_fallback_snapshots"] = [
                {"visible_nodes": 0},
                {"visible_nodes": 0},
            ]
            menu["interactions"][0]["opened_snapshot"] = {
                "visible_enabled_nodes": 1
            }
            write_json(menu_path, menu)
            report["artifacts"]["menu_interactions"] = identity(
                menu_path, relative_to=pathlib.Path(arguments["artifact_root"])
            )
            self.assertEqual(
                contract.validate_report(report, **arguments)["result"], "passed"
            )

            menu["interactions"][0]["pre_fallback_snapshots"].pop()
            write_json(menu_path, menu)
            report["artifacts"]["menu_interactions"] = identity(
                menu_path, relative_to=pathlib.Path(arguments["artifact_root"])
            )
            with self.assertRaisesRegex(ValueError, "two confirmed-hidden snapshots"):
                contract.validate_report(report, **arguments)

    def test_wrapper_retains_fail_closed_preflight_and_fresh_build_contract(self) -> None:
        text = WRAPPER_PATH.read_text(encoding="utf-8")
        required_fragments = [
            "fresh real-mpv artifact directory already exists",
            "required mpv binary does not exist",
            "missing-prerequisite",
            "HARNESS_PRELAUNCH_FAILURE",
            "exit 125",
            '@("build", "--quiet", "--locked", "-p", "sorotte-gui"',
            '"--real-mpv-vertical"',
            "--exercise-owned-mpv-recovery",
            "--exercise-faulting-http-recovery",
            "--exercise-stalled-http",
            "--expect-recovery",
            "--expect-http-fault",
            "--expect-http-stall",
            "gui-real-mpv-owned-process-recovery",
            "gui-real-mpv-faulting-http-recovery",
            "gui-real-mpv-stalled-http",
            "requires -TimeoutMs of at least 50000",
            "--expected-gui-sha256",
            "--expected-mpv-sha256",
            "--producer-exit-code",
            "--lifecycle-summary",
            "gui_binary_sha256_before",
            "gui_binary_sha256_after",
        ]
        missing = [fragment for fragment in required_fragments if fragment not in text]
        self.assertEqual(missing, [])

    def test_lifecycle_summary_binding_rejects_tampered_transition_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "shared-lifecycle-summary.json"
            payload = {
                "schema_version": 1,
                "kind": "sorotte-playback-lifecycle-evidence-validation",
                "result": "passed",
                "transitions": {
                    "APP-LAUNCH-001": 1,
                    "TRANSPORT-PLAY-001": 2,
                },
            }
            write_json(path, payload)
            digest, transitions = contract.lifecycle_summary_binding(path)
            self.assertEqual(digest, sha256(path))
            self.assertEqual(
                transitions, ["APP-LAUNCH-001", "TRANSPORT-PLAY-001"]
            )

            payload["transitions"]["TRANSPORT-PLAY-001"] = 0
            write_json(path, payload)
            with self.assertRaisesRegex(ValueError, "transition inventory is malformed"):
                contract.lifecycle_summary_binding(path)


if __name__ == "__main__":
    unittest.main()
