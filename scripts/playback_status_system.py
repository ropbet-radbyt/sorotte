#!/usr/bin/env python3
"""Exact packaged GUI participant-status composition proof.

This runner owns an actual release server, an actual release CLI backed by a
supported real mpv, and the native exact-GUI harness. It intentionally keeps
the native UI assertion in Rust while this process provides bounded process
orchestration and closed-schema evidence validation.
"""

from __future__ import annotations

import argparse
import array
import json
import math
import os
import re
import signal
import subprocess
import sys
import threading
import time
import uuid
import wave
from pathlib import Path
from typing import Any, Mapping, Sequence

import playback_lifecycle_evidence as lifecycle_evidence
from playback_lifecycle_system import (
    ProcessCapture,
    read_jsonl,
    render_mpv_observer_lua,
    sha256_file,
)

SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-playback-status-system"
NATIVE_REPORT_KIND = "sorotte-gui-participant-status-system"
ROOM_VERSION = "1.7.5"
SAFE_TOKEN = re.compile(r"[A-Za-z0-9._-]{1,64}")
DIGEST = re.compile(r"[0-9a-f]{64}")
CANDIDATE_SHA = re.compile(r"[0-9a-f]{40}")
REQUIRED_NATIVE_ASSERTIONS = frozenset(
    {
        "exact-gui-digest-matched",
        "configured-mpv-digest-matched",
        "actual-server-room-visible",
        "named-reporter-row-bound-to-status-node",
        "production-participant-status-fresh",
        "exact-gui-player-attached",
        "exact-gui-projection-ledger-recorded",
        "native-window-captured",
        "graceful-gui-shutdown",
    }
)
ACCEPTED_FRESH_STATUS_PREFIXES = frozenset(
    {
        "Player connected",
        "Player starting",
        "No media",
        "Loading",
        "Prebuffering",
        "Ready",
        "Playing",
        "Rebuffering",
        "Seeking",
        "Ended",
        "Playback failed",
    }
)


class StatusSystemError(RuntimeError):
    """Closed failure from the status composition proof."""


def require_safe_token(label: str, value: str) -> str:
    if SAFE_TOKEN.fullmatch(value) is None:
        raise StatusSystemError(f"{label} must be a 1-64 character privacy-safe token")
    return value


def require_digest(label: str, value: Any) -> str:
    if not isinstance(value, str) or DIGEST.fullmatch(value) is None:
        raise StatusSystemError(f"{label} must be lowercase SHA-256")
    return value


def require_mapping(label: str, value: Any) -> Mapping[str, Any]:
    if not isinstance(value, dict):
        raise StatusSystemError(f"{label} must be an object")
    return value


def accepted_fresh_status_label(value: Any) -> bool:
    if not isinstance(value, str) or len(value) > 192 or not value.endswith(" · fresh"):
        return False
    if "\r" in value or "\n" in value:
        return False
    return any(
        value == f"{prefix} · fresh" or value.startswith(f"{prefix} · ")
        for prefix in ACCEPTED_FRESH_STATUS_PREFIXES
    )


def validate_native_report(
    report: Mapping[str, Any],
    *,
    run_id: str,
    reporter_username: str,
    observer_username: str,
    room: str,
    gui_sha256: str,
    mpv_sha256: str,
) -> dict[str, Any]:
    allowed = {
        "schema_version",
        "kind",
        "result",
        "run_id",
        "endpoint",
        "room",
        "observer_username",
        "reporter_username",
        "gui_pid",
        "gui",
        "configured_mpv",
        "projection",
        "artifacts",
        "assertions",
    }
    if set(report) != allowed:
        raise StatusSystemError("native GUI report did not match its closed schema")
    expected_scalars = {
        "schema_version": SCHEMA_VERSION,
        "kind": NATIVE_REPORT_KIND,
        "result": "passed",
        "run_id": run_id,
        "room": room,
        "observer_username": observer_username,
        "reporter_username": reporter_username,
    }
    for field, expected in expected_scalars.items():
        if report.get(field) != expected:
            raise StatusSystemError(f"native GUI report field {field} did not match")
    if not isinstance(report.get("gui_pid"), int) or report["gui_pid"] <= 0:
        raise StatusSystemError("native GUI report omitted a positive GUI pid")
    endpoint = report.get("endpoint")
    if not isinstance(endpoint, str) or re.fullmatch(r"127\.0\.0\.1:[1-9][0-9]{0,4}", endpoint) is None:
        raise StatusSystemError("native GUI report endpoint was not strict IPv4 loopback")

    gui = require_mapping("native GUI identity", report.get("gui"))
    mpv = require_mapping("native mpv identity", report.get("configured_mpv"))
    if set(gui) != {"file_name", "sha256"} or set(mpv) != {"file_name", "sha256"}:
        raise StatusSystemError("native binary identity did not match its closed schema")
    if gui.get("sha256") != gui_sha256 or mpv.get("sha256") != mpv_sha256:
        raise StatusSystemError("native GUI or mpv digest diverged from the immutable candidate")
    for identity in (gui, mpv):
        if not isinstance(identity.get("file_name"), str) or Path(identity["file_name"]).name != identity["file_name"]:
            raise StatusSystemError("native binary identity exposed a path instead of a file name")

    projection = require_mapping("native projection", report.get("projection"))
    if set(projection) != {
        "reporter_username",
        "user_row_identity",
        "participant_index",
        "username_bounds",
        "status_automation_id",
        "status_bounds",
        "status_label",
        "binding_source",
        "vertical_gap_px",
        "visible",
    }:
        raise StatusSystemError("native projection did not match its closed schema")
    if projection.get("reporter_username") != reporter_username:
        raise StatusSystemError("native projection was not bound to the reporter username")
    user_id = projection.get("user_row_identity")
    participant_index = projection.get("participant_index")
    status_id = projection.get("status_automation_id")
    if not isinstance(user_id, str) or re.fullmatch(r"main-window:user:[0-9]+", user_id) is None:
        raise StatusSystemError("native projection omitted the exact participant row identity")
    if not isinstance(participant_index, int) or user_id != f"main-window:user:{participant_index}":
        raise StatusSystemError("native participant index diverged from the row identity")
    if status_id != f"{user_id}:participant-status":
        raise StatusSystemError("native status node was not causally bound to the reporter row")
    if not accepted_fresh_status_label(projection.get("status_label")):
        raise StatusSystemError("native status projection was not a closed fresh production state")
    if projection.get("binding_source") != "uia-spatial-row+status-index":
        raise StatusSystemError("native status projection omitted its binding source")
    username_bounds = projection.get("username_bounds")
    status_bounds = projection.get("status_bounds")
    if not all(
        isinstance(bounds, list)
        and len(bounds) == 4
        and all(isinstance(coordinate, int) for coordinate in bounds)
        for bounds in (username_bounds, status_bounds)
    ):
        raise StatusSystemError("native status projection omitted bounded UIA geometry")
    expected_gap = status_bounds[1] - username_bounds[3]
    if projection.get("vertical_gap_px") != expected_gap or not 0 <= expected_gap <= 96:
        raise StatusSystemError("native username and status nodes were not in one bounded row")
    horizontal_overlap = min(status_bounds[2], username_bounds[2]) - max(
        status_bounds[0], username_bounds[0]
    )
    if horizontal_overlap <= 0:
        raise StatusSystemError("native username and status nodes did not horizontally overlap")
    if projection.get("visible") is not True:
        raise StatusSystemError("native status projection was not visible")

    assertions = report.get("assertions")
    if not isinstance(assertions, list) or not all(isinstance(value, str) for value in assertions):
        raise StatusSystemError("native report assertions must be strings")
    if set(assertions) != REQUIRED_NATIVE_ASSERTIONS or len(assertions) != len(set(assertions)):
        raise StatusSystemError("native report assertion set was incomplete or duplicated")
    artifacts = require_mapping("native artifacts", report.get("artifacts"))
    if set(artifacts) != {"screenshot", "projection", "gui_lifecycle"}:
        raise StatusSystemError("native artifact inventory was incomplete")
    if any(
        not isinstance(value, str) or Path(value).name != value
        for value in artifacts.values()
    ):
        raise StatusSystemError("native artifact inventory exposed a path")
    return dict(report)


def atomic_write_json(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    if path.exists() or temporary.exists():
        raise StatusSystemError(f"report path must be create-new: {path}")
    with temporary.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(value, output, ensure_ascii=False, sort_keys=True, indent=2)
        output.write("\n")
    os.replace(temporary, path)


def clean_environment() -> dict[str, str]:
    return {
        key: value
        for key, value in os.environ.items()
        if not key.upper().startswith("SOROTTE_")
    }


def generate_reporter_wav(path: Path, duration_seconds: int = 30) -> dict[str, Any]:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise StatusSystemError(f"generated media path must be create-new: {path}")
    sample_rate = 48_000
    one_second = array.array(
        "h",
        (
            int(8_000 * math.sin(2.0 * math.pi * 440.0 * sample / sample_rate))
            for sample in range(sample_rate)
        ),
    )
    with wave.open(str(path), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(sample_rate)
        for _ in range(duration_seconds):
            output.writeframesraw(one_second.tobytes())
        output.writeframes(b"")
    return {
        "kind": "generated-pcm-wav",
        "duration_seconds": duration_seconds,
        "sample_rate_hz": sample_rate,
        "sha256": sha256_file(path),
    }


def wait_until(label: str, predicate: Any, timeout: float) -> Any:
    deadline = time.monotonic() + timeout
    last_error: BaseException | None = None
    while time.monotonic() < deadline:
        try:
            value = predicate()
            if value:
                return value
        except (OSError, ValueError, json.JSONDecodeError) as error:
            last_error = error
        time.sleep(0.05)
    suffix = f"; last error: {last_error}" if last_error is not None else ""
    raise StatusSystemError(f"timed out waiting for {label}{suffix}")


def terminate_process(process: ProcessCapture, *, graceful_break: bool) -> int:
    if process.process.poll() is None and graceful_break:
        try:
            if os.name == "nt":
                process.process.send_signal(signal.CTRL_BREAK_EVENT)
            else:
                os.killpg(process.process.pid, signal.SIGINT)
        except OSError:
            pass
    try:
        code = process.process.wait(timeout=10.0 if graceful_break else 1.0)
    except subprocess.TimeoutExpired:
        try:
            if os.name == "nt":
                process.process.terminate()
            else:
                os.killpg(process.process.pid, signal.SIGTERM)
            code = process.process.wait(timeout=3.0)
        except (OSError, subprocess.TimeoutExpired):
            if os.name == "nt":
                process.process.kill()
            else:
                os.killpg(process.process.pid, signal.SIGKILL)
            code = process.process.wait(timeout=3.0)
    process.join_capture()
    return code


def await_natural_process_exit(process: ProcessCapture, timeout: float) -> int:
    try:
        code = process.process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        return terminate_process(process, graceful_break=True)
    process.join_capture()
    return code


def parse_native_stdout(stdout: str) -> Mapping[str, Any]:
    lines = [line.strip() for line in stdout.splitlines() if line.strip()]
    if not lines:
        raise StatusSystemError("native GUI harness produced no JSON report")
    try:
        value = json.loads(lines[-1])
    except json.JSONDecodeError as error:
        raise StatusSystemError("native GUI harness did not end with a JSON report") from error
    return require_mapping("native GUI report", value)


def run(args: argparse.Namespace) -> dict[str, Any]:
    repo_root = Path(args.repo_root).resolve()
    artifact_dir = Path(args.artifact_dir).resolve()
    if artifact_dir.exists() and any(artifact_dir.iterdir()):
        raise StatusSystemError(f"artifact directory must be empty: {artifact_dir}")
    artifact_dir.mkdir(parents=True, exist_ok=True)
    run_id = require_safe_token("run id", args.run_id or f"status-{uuid.uuid4().hex[:20]}")
    reporter_username = require_safe_token("reporter username", args.reporter_username)
    observer_username = require_safe_token("observer username", args.observer_username)
    room = require_safe_token("room", args.room)
    if reporter_username == observer_username:
        raise StatusSystemError("reporter and observer usernames must be distinct")
    candidate_sha = args.candidate_sha.lower()
    if CANDIDATE_SHA.fullmatch(candidate_sha) is None:
        raise StatusSystemError("candidate SHA must be exactly 40 hexadecimal characters")

    paths = {
        label: Path(value).resolve()
        for label, value in {
            "server": args.server,
            "client": args.client,
            "gui": args.gui,
            "native_harness": args.native_harness,
            "mpv": args.mpv,
        }.items()
    }
    for label, path in paths.items():
        if not path.is_file():
            raise StatusSystemError(f"{label} executable does not exist: {path}")
    digests = {label: sha256_file(path) for label, path in paths.items()}
    for label, digest in digests.items():
        require_digest(f"{label} digest", digest)

    lifecycle_paths = {
        "server": artifact_dir / "server-product-lifecycle.jsonl",
        "client-status-reporter": artifact_dir / "reporter-product-lifecycle.jsonl",
        "gui-status-observer": artifact_dir / "gui-product-lifecycle.jsonl",
    }
    merged_lifecycle = artifact_dir / "status-product-lifecycle.merged.jsonl"
    lifecycle_summary_path = artifact_dir / "status-product-lifecycle.summary.json"
    native_report_path = artifact_dir / "participant-status-native-report.json"
    report_path = artifact_dir / "playback-status-system-report.json"
    media_path = artifact_dir / "generated-status-reporter.wav"
    player_trace = artifact_dir / "reporter-player.jsonl"
    player_script = artifact_dir / "reporter-player-observer.lua"
    native_artifact_dir = artifact_dir / "native"
    native_artifact_dir.mkdir()

    media_evidence = generate_reporter_wav(media_path)
    player_trace.write_text("", encoding="utf-8")
    player_script.write_text(
        render_mpv_observer_lua(
            role="status-reporter",
            trace_path=player_trace,
            first_media_name=media_path.name,
            second_media_name="unused-second-media",
        ),
        encoding="utf-8",
    )

    server: ProcessCapture | None = None
    reporter: ProcessCapture | None = None
    server_port: int | None = None
    native_report: dict[str, Any] | None = None
    checks: list[str] = []
    failure: BaseException | None = None

    try:
        port_ready = threading.Event()

        def inspect_server_stderr(line: str) -> None:
            nonlocal server_port
            match = re.search(r"sorotte-server listening on 127\.0\.0\.1:(\d+)", line)
            if match:
                server_port = int(match.group(1))
                port_ready.set()

        server_env = clean_environment()
        server_env.update(
            {
                "SOROTTE_LIFECYCLE_EVIDENCE_PATH": str(lifecycle_paths["server"]),
                "SOROTTE_LIFECYCLE_RUN_ID": run_id,
                "SOROTTE_LIFECYCLE_EMITTER": "server",
            }
        )
        server = ProcessCapture(
            role="server",
            args=[
                str(paths["server"]),
                "--port",
                "0",
                "--ipv4-only",
                "--interface-ipv4",
                "127.0.0.1",
                "--disable-ready",
            ],
            cwd=repo_root,
            env=server_env,
            artifact_dir=artifact_dir,
            stdin=False,
            stderr_callback=inspect_server_stderr,
        )
        wait_until("release server IPv4 listener", port_ready.is_set, 10.0)
        assert server_port is not None and 0 < server_port <= 65535
        checks.append("actual-release-server-listening")

        config_root = artifact_dir / "reporter-config"
        config_root.mkdir()
        ipc_path = rf"\\.\pipe\sorotte-status-{run_id[:24]}" if os.name == "nt" else str(
            artifact_dir / "reporter-mpv.sock"
        )
        reporter_env = clean_environment()
        reporter_env.update(
            {
                "SOROTTE_CLIENT_HOST": "127.0.0.1",
                "SOROTTE_CLIENT_PORT": str(server_port),
                "SOROTTE_CLIENT_USERNAME": reporter_username,
                "SOROTTE_CLIENT_ROOM": room,
                "SOROTTE_CLIENT_VERSION": ROOM_VERSION,
                "SOROTTE_CLIENT_MAX_RETRIES": "20",
                "SOROTTE_CLIENT_MAX_CONNECTED_RUNTIME_SECONDS": "15",
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
                "SOROTTE_CLIENT_MPV_MANAGED_BIN": str(paths["mpv"]),
                "SOROTTE_CLIENT_MPV_MANAGED_MEDIA": str(media_path),
                "SOROTTE_CLIENT_MPV_MANAGED_IPC_PATH": ipc_path,
                "SOROTTE_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS": "10000",
                "SOROTTE_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS": "25",
                "SOROTTE_CLIENT_LOG_PLAYER_TELEMETRY": "1",
                "SOROTTE_LIFECYCLE_EVIDENCE_PATH": str(
                    lifecycle_paths["client-status-reporter"]
                ),
                "SOROTTE_LIFECYCLE_RUN_ID": run_id,
                "SOROTTE_LIFECYCLE_EMITTER": "client-status-reporter",
            }
        )
        reporter = ProcessCapture(
            role="client-status-reporter",
            args=[
                str(paths["client"]),
                "--no-gui",
                "--no-store",
                "--",
                f"--script={player_script}",
                "--no-config",
                "--ao=null",
                "--vo=null",
                "--audio-display=no",
                "--keep-open=yes",
            ],
            cwd=repo_root,
            env=reporter_env,
            artifact_dir=artifact_dir,
            stdin=True,
        )

        def reporter_ready() -> bool:
            if reporter is not None and reporter.process.poll() is not None:
                raise StatusSystemError(
                    f"release reporter exited early with code {reporter.process.returncode}"
                )
            server_text = (
                lifecycle_paths["server"].read_text(encoding="utf-8", errors="replace")
                if lifecycle_paths["server"].exists()
                else ""
            )
            trace = read_jsonl(player_trace)
            return "STATUS-FRESH-001" in server_text and any(
                record.get("event") == "file-loaded" for record in trace
            )

        wait_until("real-mpv reporter fresh status", reporter_ready, 20.0)
        checks.extend(
            [
                "actual-release-cli-connected",
                "real-supported-mpv-loaded-generated-media",
                "server-accepted-fresh-production-status",
            ]
        )

        native_args = [
            str(paths["native_harness"]),
            "--participant-status-system",
            "--binary",
            str(paths["gui"]),
            "--mpv",
            str(paths["mpv"]),
            "--artifact-dir",
            str(native_artifact_dir),
            "--shared-lifecycle",
            str(lifecycle_paths["gui-status-observer"]),
            "--host",
            "127.0.0.1",
            "--port",
            str(server_port),
            "--run-id",
            run_id,
            "--observer-username",
            observer_username,
            "--reporter-username",
            reporter_username,
            "--room",
            room,
            "--expected-gui-sha256",
            digests["gui"],
            "--expected-mpv-sha256",
            digests["mpv"],
            "--timeout-ms",
            str(args.timeout_ms),
            "--json",
        ]
        native_env = clean_environment()
        native_stdout_path = artifact_dir / "native-harness.stdout.log"
        native_stderr_path = artifact_dir / "native-harness.stderr.log"
        native_popen_kwargs: dict[str, Any] = {
            "cwd": str(repo_root),
            "env": native_env,
        }
        if os.name == "nt":
            native_popen_kwargs["creationflags"] = getattr(
                subprocess, "CREATE_NEW_PROCESS_GROUP", 0
            )
        else:
            native_popen_kwargs["start_new_session"] = True
        with native_stdout_path.open("x", encoding="utf-8", newline="\n") as stdout_log, native_stderr_path.open(
            "x", encoding="utf-8", newline="\n"
        ) as stderr_log:
            native_process = subprocess.Popen(
                native_args,
                stdout=stdout_log,
                stderr=stderr_log,
                **native_popen_kwargs,
            )
            try:
                native_code = native_process.wait(
                    timeout=max(30.0, args.timeout_ms / 1000.0 + 20.0)
                )
            except subprocess.TimeoutExpired as error:
                native_process.kill()
                native_process.wait(timeout=5.0)
                raise StatusSystemError(
                    "native exact-GUI harness exceeded its absolute process deadline"
                ) from error
        native_stdout = native_stdout_path.read_text(
            encoding="utf-8", errors="replace"
        )
        if native_code != 0:
            raise StatusSystemError(
                f"native exact-GUI harness failed with code {native_code}"
            )
        native_report = validate_native_report(
            parse_native_stdout(native_stdout),
            run_id=run_id,
            reporter_username=reporter_username,
            observer_username=observer_username,
            room=room,
            gui_sha256=digests["gui"],
            mpv_sha256=digests["mpv"],
        )
        atomic_write_json(native_report_path, native_report)
        checks.extend(
            [
                "exact-release-gui-observed-named-peer",
                "username-bound-native-status-node-fresh",
                "exact-gui-and-mpv-digests-attested",
                "native-status-screenshot-captured",
            ]
        )
    except BaseException as error:
        failure = error
    finally:
        if reporter is None:
            reporter_code = None
        elif failure is None:
            reporter_code = await_natural_process_exit(reporter, 20.0)
        else:
            reporter_code = terminate_process(reporter, graceful_break=True)
        server_code = terminate_process(server, graceful_break=True) if server is not None else None

    if failure is not None:
        raise failure
    if reporter_code != 0:
        raise StatusSystemError(f"release reporter exited with code {reporter_code}")
    if server_code != 0:
        raise StatusSystemError(f"release server exited with code {server_code}")
    checks.append("bounded-graceful-product-shutdown")

    lifecycle_summary = lifecycle_evidence.validate_and_merge(
        list(lifecycle_paths.values()),
        model_path=repo_root / "coverage" / "playback-lifecycle.toml",
        output_path=merged_lifecycle,
        summary_path=lifecycle_summary_path,
        required_inventories={
            "server": frozenset({"server"}),
            "client-status-reporter": frozenset({"client", "player"}),
            "gui-status-observer": frozenset({"gui", "client", "player"}),
        },
        required_roles=frozenset({"server", "client", "player", "gui"}),
        expected_digests={
            "server": digests["server"],
            "client-status-reporter": digests["client"],
            "gui-status-observer": digests["gui"],
        },
        minimum_cross_process_edges=0,
    )
    checks.append("closed-product-lifecycle-ledger-validated")
    assert native_report is not None
    report = {
        "schema_version": SCHEMA_VERSION,
        "kind": REPORT_KIND,
        "result": "passed",
        "candidate_sha": candidate_sha,
        "run_id": run_id,
        "composition": [
            "actual-release-server",
            "actual-release-cli",
            "supported-real-mpv",
            "exact-release-gui",
            "windows-uia-accesskit",
        ],
        "prerequisites": {
            label: {"file_name": paths[label].name, "sha256": digests[label]}
            for label in sorted(paths)
        },
        "generated_media": media_evidence,
        "projection": native_report["projection"],
        "lifecycle_summary": lifecycle_summary,
        "checks": checks,
        "artifacts": {
            "native_report": native_report_path.name,
            "native_screenshot": str(
                Path("native") / native_report["artifacts"]["screenshot"]
            ).replace("\\", "/"),
            "native_projection": str(
                Path("native") / native_report["artifacts"]["projection"]
            ).replace("\\", "/"),
            "player_trace": player_trace.name,
            "lifecycle_evidence": merged_lifecycle.name,
            "lifecycle_summary": lifecycle_summary_path.name,
            "lifecycle_process_ledgers": [path.name for path in lifecycle_paths.values()],
        },
    }
    atomic_write_json(report_path, report)
    return report


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", default=Path(__file__).resolve().parents[1])
    parser.add_argument("--server", required=True)
    parser.add_argument("--client", required=True)
    parser.add_argument("--gui", required=True)
    parser.add_argument("--native-harness", required=True)
    parser.add_argument("--mpv", required=True)
    parser.add_argument("--artifact-dir", required=True)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--run-id")
    parser.add_argument("--reporter-username", default="status-reporter")
    parser.add_argument("--observer-username", default="status-observer")
    parser.add_argument("--room", default="status-system-room")
    parser.add_argument("--timeout-ms", type=int, default=30_000)
    args = parser.parse_args(argv)
    if not 1_000 <= args.timeout_ms <= 120_000:
        parser.error("--timeout-ms must be between 1000 and 120000")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        report = run(args)
    except BaseException as error:
        print(
            json.dumps(
                {
                    "kind": REPORT_KIND,
                    "result": "failed",
                    "error_type": type(error).__name__,
                    "error": str(error),
                },
                sort_keys=True,
            ),
            flush=True,
        )
        return 1
    print(
        json.dumps(
            {
                "kind": REPORT_KIND,
                "result": report["result"],
                "candidate_sha": report["candidate_sha"],
                "checks": len(report["checks"]),
                "status_label": report["projection"]["status_label"],
            },
            sort_keys=True,
        ),
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
