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

    menu = json.loads(menu_path.read_text(encoding="utf-8"))
    replacement_open = copy.deepcopy(menu["interactions"][0])
    menu["interactions"].insert(1, replacement_open)
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
            "event": "end-file",
            "pid": pid,
            "path": None,
            "reason": "eof",
            "pause": False,
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
            "advertised_body_bytes": media_path.stat().st_size,
            "transmitted_body_bytes": 0,
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
            "advertised_body_bytes": contract.HTTP_FAULT_DISCONNECT_AFTER_BYTES,
            "transmitted_body_bytes": contract.HTTP_FAULT_DISCONNECT_AFTER_BYTES,
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
            "advertised_body_bytes": media_path.stat().st_size,
            "transmitted_body_bytes": media_path.stat().st_size,
            "disconnected_early": False,
            "write_error": None,
        },
    ]
    evidence = {
        "schema_version": 1,
        "kind": contract.HTTP_FAULT_KIND,
        "result": "passed",
        "fault": "first-response-content-length-is-shorter-than-declared-au-media-once",
        "recovery_mode": "same-generation-automatic-network-stream-reload",
        "listener_endpoint": endpoint,
        "listener_ipv4_loopback": True,
        "media_url": media_url,
        "route": contract.HTTP_FAULT_ROUTE,
        "generated_media_bytes": media_path.stat().st_size,
        "generated_media_sha256": sha256(media_path),
        "duration_seconds": contract.HTTP_FAULT_DURATION_SECONDS,
        "disconnect_after_body_bytes": contract.HTTP_FAULT_DISCONNECT_AFTER_BYTES,
        "request_count": len(requests),
        "premature_disconnect_count": 1,
        "complete_response_count": 1,
        "requests": requests,
        "initial_file_loaded_index": 0,
        "pre_fault_progress_index": 2,
        "end_file_index": 3,
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

    state = json.loads(state_path.read_text(encoding="utf-8"))
    state["assertions"] = list(contract.HTTP_FAULT_REQUIRED_ASSERTIONS)
    write_json(state_path, state)

    report["assertions"] = list(contract.HTTP_FAULT_REQUIRED_ASSERTIONS)
    report["http_fault"] = evidence
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
        self.assertEqual(
            summary["assertion_count"], len(contract.HTTP_FAULT_REQUIRED_ASSERTIONS)
        )
        self.assertEqual(
            summary["artifact_count"], len(contract.HTTP_FAULT_REQUIRED_ARTIFACTS)
        )

    def test_http_fault_contract_rejects_cache_path_extra_get_and_foreign_generation(
        self,
    ) -> None:
        mutations = (
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
                "exactly one short GET and one complete GET",
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
                    "sharedPlaylists", True
                ),
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
            "--expect-recovery",
            "--expect-http-fault",
            "gui-real-mpv-owned-process-recovery",
            "gui-real-mpv-faulting-http-recovery",
            "--expected-gui-sha256",
            "--expected-mpv-sha256",
            "--producer-exit-code",
            "gui_binary_sha256_before",
            "gui_binary_sha256_after",
        ]
        missing = [fragment for fragment in required_fragments if fragment not in text]
        self.assertEqual(missing, [])


if __name__ == "__main__":
    unittest.main()
