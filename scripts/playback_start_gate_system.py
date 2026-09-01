#!/usr/bin/env python3
"""Exercise every coordinated-start phase against an exact Sorotte server binary.

The harness uses independent raw-protocol peers as an external oracle.  It
generates late join, slow resolution, transport partition, reconnect,
commit-before-Started recovery, timeout degradation, and sleep/resume walks.
No reconnect token, media identity, endpoint, or local path is written to the
report or causal ledgers.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import signal
import socket
import subprocess
import threading
import time
import tomllib
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence

import playback_lifecycle_evidence as lifecycle_evidence


SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-playback-start-gate-system"
CANDIDATE_SHA = re.compile(r"^[0-9a-f]{40}$")
SAFE_TOKEN = re.compile(r"^[A-Za-z0-9._:-]{1,128}$")
REQUIRED_PHASES = (
    "inactive",
    "waitingForIntent",
    "waitingForTechnicalReadiness",
    "readyToCommit",
    "committed",
    "degraded",
)
REQUIRED_TRANSITIONS = (
    "GATE-PREPARE-001",
    "GATE-PLAYABILITY-001",
    "GATE-READY-001",
    "GATE-COMMIT-001",
    "GATE-DEGRADE-001",
    "GATE-CLEAR-001",
)
REQUIRED_SCENARIOS = (
    "late-join-slow-resolution",
    "partition-reconnect-before-commit",
    "reconnect-between-commit-and-started",
    "timeout-degraded-late-join",
    "sleep-resume-degraded-snapshot",
)
REPORT_KEYS = {
    "schema_version",
    "kind",
    "result",
    "candidate_sha",
    "candidate_binding",
    "run_id",
    "composition",
    "server",
    "phase_coverage",
    "transition_coverage",
    "scenario_coverage",
    "checks",
    "lifecycle_summary",
    "artifacts",
}


class StartGateSystemError(RuntimeError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def require_safe_token(label: str, value: Any) -> str:
    if not isinstance(value, str) or SAFE_TOKEN.fullmatch(value) is None:
        raise StartGateSystemError(f"{label} must be a bounded safe token")
    return value


def atomic_write_json(path: Path, value: Mapping[str, Any]) -> None:
    if path.exists():
        raise StartGateSystemError(f"create-new artifact already exists: {path.name}")
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with temporary.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(value, output, sort_keys=True, ensure_ascii=False, indent=2)
        output.write("\n")
    os.replace(temporary, path)


def clean_environment() -> dict[str, str]:
    return {
        key: value
        for key, value in os.environ.items()
        if not key.upper().startswith("SOROTTE_")
    }


def wait_until(label: str, predicate: Callable[[], Any], timeout: float = 8.0) -> Any:
    deadline = time.monotonic() + timeout
    last_error: BaseException | None = None
    while time.monotonic() < deadline:
        try:
            value = predicate()
            if value:
                return value
        except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
            last_error = error
        time.sleep(0.025)
    suffix = f" ({type(last_error).__name__})" if last_error is not None else ""
    raise StartGateSystemError(f"timed out waiting for {label}{suffix}")


def readiness_snapshot(message: Mapping[str, Any]) -> Mapping[str, Any] | None:
    payload = message.get("Set")
    if not isinstance(payload, Mapping):
        return None
    extension = payload.get("sorotteReadinessV2")
    if not isinstance(extension, Mapping):
        return None
    snapshot = extension.get("snapshot")
    return snapshot if isinstance(snapshot, Mapping) else None


def barrier_extension(message: Mapping[str, Any]) -> Mapping[str, Any] | None:
    payload = message.get("Set")
    if not isinstance(payload, Mapping):
        return None
    extension = payload.get("sorottePlaybackBarrierV1")
    return extension if isinstance(extension, Mapping) else None


def snapshot_phase(snapshot: Mapping[str, Any]) -> str | None:
    phase = snapshot.get("startGatePhase")
    if not isinstance(phase, Mapping):
        return None
    value = phase.get("phase")
    return value if isinstance(value, str) else None


def validate_phase_walk(phases: Sequence[str]) -> None:
    allowed = set(REQUIRED_PHASES)
    if not phases or any(phase not in allowed for phase in phases):
        raise StartGateSystemError("phase walk contains an unknown or empty phase")
    required_order = [
        "waitingForIntent",
        "waitingForTechnicalReadiness",
        "readyToCommit",
        "committed",
        "inactive",
    ]
    positions: list[int] = []
    cursor = 0
    for required in required_order:
        try:
            cursor = phases.index(required, cursor)
        except ValueError as error:
            raise StartGateSystemError(
                f"phase walk omitted ordered phase {required}"
            ) from error
        positions.append(cursor)
        cursor += 1
    if positions != sorted(set(positions)):
        raise StartGateSystemError("phase walk did not advance monotonically")


def validate_report(report: Mapping[str, Any]) -> dict[str, Any]:
    if set(report) != REPORT_KEYS:
        raise StartGateSystemError("start-gate report does not use its closed schema")
    if report.get("schema_version") != SCHEMA_VERSION or report.get("kind") != REPORT_KIND:
        raise StartGateSystemError("start-gate report has the wrong schema identity")
    if report.get("result") != "passed":
        raise StartGateSystemError("start-gate report is not a pass")
    candidate_sha = report.get("candidate_sha")
    if not isinstance(candidate_sha, str) or CANDIDATE_SHA.fullmatch(candidate_sha) is None:
        raise StartGateSystemError("start-gate report has an invalid candidate SHA")
    if report.get("candidate_binding") not in {"exact-clean-head", "development-unverified"}:
        raise StartGateSystemError("start-gate report has an unknown candidate binding")
    if report.get("phase_coverage") != list(REQUIRED_PHASES):
        raise StartGateSystemError("start-gate report phase coverage is incomplete")
    if report.get("transition_coverage") != list(REQUIRED_TRANSITIONS):
        raise StartGateSystemError("start-gate report transition coverage is incomplete")
    if report.get("scenario_coverage") != list(REQUIRED_SCENARIOS):
        raise StartGateSystemError("start-gate report scenario coverage is incomplete")
    checks = report.get("checks")
    if not isinstance(checks, list) or len(checks) != len(set(checks)) or len(checks) < 18:
        raise StartGateSystemError("start-gate report check inventory is incomplete")
    server = report.get("server")
    if not isinstance(server, Mapping) or set(server) != {"file_name", "sha256"}:
        raise StartGateSystemError("start-gate report server identity is malformed")
    digest = server.get("sha256")
    if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        raise StartGateSystemError("start-gate report server digest is invalid")
    artifacts = report.get("artifacts")
    if not isinstance(artifacts, Mapping) or set(artifacts) != {
        "server_lifecycle",
        "oracle_lifecycle",
        "merged_lifecycle",
        "lifecycle_summary",
        "server_stdout",
        "server_stderr",
    }:
        raise StartGateSystemError("start-gate report artifact inventory is malformed")
    for value in artifacts.values():
        if not isinstance(value, str) or Path(value).name != value:
            raise StartGateSystemError("start-gate report artifact names must be local basenames")
    return dict(report)


@dataclass(frozen=True)
class Received:
    sequence: int
    message: dict[str, Any]


class RawPeer:
    """A bounded in-memory production-wire peer that never persists raw frames."""

    def __init__(
        self,
        *,
        host: str,
        port: int,
        room: str,
        username: str,
        reconnect_token: str | None = None,
    ) -> None:
        self.host = host
        self.port = port
        self.room = require_safe_token("room", room)
        self.username = require_safe_token("username", username)
        self._presented_token = reconnect_token
        self._socket: socket.socket | None = None
        self._reader: threading.Thread | None = None
        self._send_lock = threading.Lock()
        self._message_lock = threading.Lock()
        self._messages: list[Received] = []
        self._sequence = 0
        self.error: str | None = None

    def connect(self) -> None:
        if self._socket is not None:
            raise StartGateSystemError("peer is already connected")
        self._socket = socket.create_connection((self.host, self.port), timeout=5.0)
        self._socket.settimeout(None)
        self._reader = threading.Thread(
            target=self._read_loop,
            name=f"start-gate-{self.username}",
            daemon=True,
        )
        self._reader.start()
        hello: dict[str, Any] = {
            "username": self.username,
            "room": {"name": self.room},
            "version": "1.7.5",
            "features": {
                "sorottePlaybackBarrierV1": True,
                "sorotteReadinessV2": True,
            },
        }
        if self._presented_token is not None:
            hello["sorotteReadinessReconnectToken"] = self._presented_token
        self.send({"Hello": hello})
        wait_until(f"{self.username} readiness handshake", lambda: self.latest_snapshot())

    def _read_loop(self) -> None:
        assert self._socket is not None
        try:
            with self._socket.makefile("rb") as source:
                for raw_line in source:
                    if not raw_line.strip():
                        continue
                    message = json.loads(raw_line.decode("utf-8"))
                    if not isinstance(message, dict):
                        raise ValueError("server frame was not an object")
                    with self._message_lock:
                        self._sequence += 1
                        self._messages.append(Received(self._sequence, message))
                    self._respond_to_liveness(message)
        except (OSError, ValueError, UnicodeError, json.JSONDecodeError) as error:
            if self._socket is not None:
                self.error = type(error).__name__

    def _respond_to_liveness(self, message: Mapping[str, Any]) -> None:
        state = message.get("State")
        if not isinstance(state, Mapping):
            return
        response: dict[str, Any] = {}
        ping = state.get("ping")
        if isinstance(ping, Mapping) and isinstance(ping.get("latencyCalculation"), (int, float)):
            response["ping"] = {
                "latencyCalculation": ping["latencyCalculation"],
                "clientLatencyCalculation": time.monotonic(),
                "clientRtt": 0.0,
            }
        ignoring = state.get("ignoringOnTheFly")
        if isinstance(ignoring, Mapping) and isinstance(ignoring.get("server"), int):
            response["ignoringOnTheFly"] = {"server": ignoring["server"]}
        if response:
            try:
                self.send({"State": response})
            except OSError:
                pass

    def send(self, message: Mapping[str, Any]) -> None:
        encoded = (json.dumps(message, separators=(",", ":")) + "\r\n").encode("utf-8")
        with self._send_lock:
            if self._socket is None:
                raise StartGateSystemError("peer is not connected")
            self._socket.sendall(encoded)

    def cursor(self) -> int:
        with self._message_lock:
            return self._sequence

    def messages_after(self, cursor: int = 0) -> list[Received]:
        with self._message_lock:
            return [record for record in self._messages if record.sequence > cursor]

    def wait_message(
        self,
        label: str,
        predicate: Callable[[Mapping[str, Any]], bool],
        *,
        after: int = 0,
        timeout: float = 8.0,
    ) -> Mapping[str, Any]:
        def match() -> Mapping[str, Any] | None:
            if self.error is not None:
                raise StartGateSystemError(f"{self.username} reader failed")
            for record in self.messages_after(after):
                if predicate(record.message):
                    return record.message
            return None

        return wait_until(label, match, timeout)

    def latest_snapshot(self) -> Mapping[str, Any] | None:
        for record in reversed(self.messages_after()):
            snapshot = readiness_snapshot(record.message)
            if snapshot is not None:
                return snapshot
        return None

    def wait_phase(self, phase: str, *, after: int = 0, timeout: float = 8.0) -> Mapping[str, Any]:
        message = self.wait_message(
            f"{self.username} phase {phase}",
            lambda value: (snapshot := readiness_snapshot(value)) is not None
            and snapshot_phase(snapshot) == phase,
            after=after,
            timeout=timeout,
        )
        snapshot = readiness_snapshot(message)
        assert snapshot is not None
        return snapshot

    def reconnect_token(self) -> str:
        for record in reversed(self.messages_after()):
            hello = record.message.get("Hello")
            if isinstance(hello, Mapping):
                token = hello.get("sorotteReadinessReconnectToken")
                if isinstance(token, str) and token:
                    return token
        raise StartGateSystemError("server did not issue a reconnect token")

    def membership_epoch(self) -> int:
        snapshot = self.latest_snapshot()
        if snapshot is None:
            raise StartGateSystemError("peer has no readiness snapshot")
        participants = snapshot.get("participants")
        participant = participants.get(self.username) if isinstance(participants, Mapping) else None
        epoch = participant.get("membershipEpoch") if isinstance(participant, Mapping) else None
        if not isinstance(epoch, int) or isinstance(epoch, bool) or epoch <= 0:
            raise StartGateSystemError("peer snapshot has no positive membership epoch")
        return epoch

    def close(self) -> None:
        sock = self._socket
        self._socket = None
        if sock is not None:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                sock.close()
            except OSError:
                pass
        if self._reader is not None:
            self._reader.join(timeout=1.0)


class ServerProcess:
    def __init__(self, executable: Path, repo_root: Path, artifact_dir: Path, env: Mapping[str, str]) -> None:
        self.stdout_path = artifact_dir / "start-gate-server.stdout.log"
        self.stderr_path = artifact_dir / "start-gate-server.stderr.log"
        self._stdout = self.stdout_path.open("x", encoding="utf-8", newline="\n")
        self._stderr = self.stderr_path.open("x", encoding="utf-8", newline="\n")
        creationflags = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) if os.name == "nt" else 0
        self.process = subprocess.Popen(
            [str(executable), "--port", "0", "--ipv4-only", "--interface-ipv4", "127.0.0.1"],
            cwd=repo_root,
            env=dict(env),
            stdin=subprocess.DEVNULL,
            stdout=self._stdout,
            stderr=self._stderr,
            text=True,
            creationflags=creationflags,
            start_new_session=os.name != "nt",
        )
        self.port: int | None = None

    def wait_ready(self) -> int:
        pattern = re.compile(r"sorotte-server listening on 127\.0\.0\.1:(\d+)")

        def inspect() -> int | None:
            if self.process.poll() is not None:
                raise StartGateSystemError(f"release server exited early with code {self.process.returncode}")
            self._stderr.flush()
            text = self.stderr_path.read_text(encoding="utf-8", errors="replace")
            match = pattern.search(text)
            return int(match.group(1)) if match else None

        self.port = wait_until("release server IPv4 listener", inspect, 10.0)
        return self.port

    def stop(self) -> int:
        if self.process.poll() is None:
            try:
                if os.name == "nt":
                    self.process.send_signal(signal.CTRL_BREAK_EVENT)
                else:
                    os.killpg(self.process.pid, signal.SIGINT)
            except OSError:
                pass
        try:
            code = self.process.wait(timeout=12.0)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                code = self.process.wait(timeout=3.0)
            except subprocess.TimeoutExpired:
                self.process.kill()
                code = self.process.wait(timeout=3.0)
        self._stdout.close()
        self._stderr.close()
        return code


class StartGateHarness:
    def __init__(self, args: argparse.Namespace) -> None:
        self.repo_root = Path(args.repo_root).resolve()
        self.server_path = Path(args.server).resolve()
        self.artifact_dir = Path(args.artifact_dir).resolve()
        self.candidate_sha = str(args.candidate_sha).lower()
        self.allow_unverified = bool(args.allow_unverified_candidate)
        self.run_id = require_safe_token(
            "run id", args.run_id or f"start-{uuid.uuid4().hex[:20]}"
        )
        self.server_digest = sha256_file(self.server_path)
        self.server_lifecycle = self.artifact_dir / "start-gate-server.lifecycle.jsonl"
        self.oracle_lifecycle = self.artifact_dir / "start-gate-oracle.lifecycle.jsonl"
        self.merged_lifecycle = self.artifact_dir / "start-gate-lifecycle.merged.jsonl"
        self.lifecycle_summary_path = self.artifact_dir / "start-gate-lifecycle.summary.json"
        self.report_path = self.artifact_dir / "playback-start-gate-system-report.json"
        self.server: ServerProcess | None = None
        self.peers: list[RawPeer] = []
        self.checks: list[str] = []
        self.observed_phases: set[str] = {"inactive"}
        self.scenarios: set[str] = set()
        self.started_at = utc_now()
        with (self.repo_root / "Cargo.toml").open("rb") as source:
            self.product_version = str(tomllib.load(source)["workspace"]["package"]["version"])

    def preflight(self) -> str:
        if not self.server_path.is_file():
            raise StartGateSystemError("release server executable does not exist")
        if CANDIDATE_SHA.fullmatch(self.candidate_sha) is None:
            raise StartGateSystemError("candidate SHA must be exactly 40 lowercase hex characters")
        head = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=self.repo_root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip().lower()
        dirty = bool(
            subprocess.run(
                ["git", "status", "--porcelain"],
                cwd=self.repo_root,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
        )
        exact = head == self.candidate_sha and not dirty
        if not exact and not self.allow_unverified:
            raise StartGateSystemError("candidate binding requires the exact clean Git HEAD")
        return "exact-clean-head" if exact else "development-unverified"

    def peer(self, room: str, username: str, token: str | None = None) -> RawPeer:
        if self.server is None or self.server.port is None:
            raise StartGateSystemError("server is not listening")
        peer = RawPeer(
            host="127.0.0.1",
            port=self.server.port,
            room=room,
            username=username,
            reconnect_token=token,
        )
        peer.connect()
        self.peers.append(peer)
        phase = snapshot_phase(peer.latest_snapshot() or {})
        if phase is not None:
            self.observed_phases.add(phase)
        return peer

    @staticmethod
    def send_intent(peer: RawPeer, operation: str, nonce: int) -> None:
        peer.send(
            {
                "Set": {
                    "sorotteReadinessV2": {
                        "intent": {
                            "operationId": operation,
                            "requestNonce": nonce,
                            "membershipEpoch": peer.membership_epoch(),
                            "desired": "ready",
                            "source": {"type": "directUser", "surface": "guiButton"},
                        }
                    }
                }
            }
        )

    @staticmethod
    def send_prepare(
        peer: RawPeer,
        *,
        request_id: str,
        nonce: int,
        logical_media_id: str,
        timeout_ms: int,
    ) -> None:
        peer.send(
            {
                "Set": {
                    "sorottePlaybackBarrierV1": {
                        "prepare": {
                            "mediaGeneration": 0,
                            "requestNonce": nonce,
                            "requestId": request_id,
                            "loadIntent": "newPlayback",
                            "logicalMediaId": logical_media_id,
                            "targetPosition": 0.0,
                            "policy": "allEligible",
                            "timeoutMs": timeout_ms,
                            "timeoutAction": "remainPaused",
                        }
                    }
                }
            }
        )

    @staticmethod
    def send_technical(peer: RawPeer, generation: int, sequence: int = 1) -> None:
        peer.send(
            {
                "State": {
                    "sorotteReadinessV2": {
                        "technical": {
                            "mediaGeneration": generation,
                            "membershipEpoch": peer.membership_epoch(),
                            "reportSequence": sequence,
                            "phase": "playable",
                        }
                    }
                }
            }
        )

    @staticmethod
    def send_target_ready(peer: RawPeer, generation: int) -> None:
        peer.send(
            {
                "State": {
                    "sorottePlaybackBarrierV1": {
                        "ready": {
                            "mediaGeneration": generation,
                            "loaded": True,
                            "seekable": True,
                            "bufferReady": True,
                        }
                    }
                }
            }
        )

    @staticmethod
    def send_started(peer: RawPeer, generation: int, revision: int) -> None:
        peer.send(
            {
                "State": {
                    "sorottePlaybackBarrierV1": {
                        "started": {
                            "mediaGeneration": generation,
                            "stateRevision": revision,
                            "observedPosition": 0.1,
                        }
                    }
                }
            }
        )

    @staticmethod
    def send_recovery(
        peer: RawPeer,
        *,
        request_id: str,
        original_nonce: int,
        recovery_nonce: int,
        logical_media_id: str,
    ) -> None:
        peer.send(
            {
                "Set": {
                    "sorottePlaybackBarrierV1": {
                        "recovery": {
                            "requestId": request_id,
                            "originalRequestNonce": original_nonce,
                            "recoveryNonce": recovery_nonce,
                            "logicalMediaId": logical_media_id,
                        }
                    }
                }
            }
        )

    def observe_phase(self, peer: RawPeer, phase: str, *, after: int = 0, timeout: float = 8.0) -> Mapping[str, Any]:
        snapshot = peer.wait_phase(phase, after=after, timeout=timeout)
        self.observed_phases.add(phase)
        return snapshot

    @staticmethod
    def generation(snapshot: Mapping[str, Any]) -> int:
        value = snapshot.get("mediaGeneration")
        if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
            raise StartGateSystemError("readiness snapshot has no positive media generation")
        return value

    @staticmethod
    def commit_from(peer: RawPeer, *, after: int = 0, timeout: float = 8.0) -> tuple[int, int]:
        message = peer.wait_message(
            "canonical start commit",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("commit"), Mapping),
            after=after,
            timeout=timeout,
        )
        extension = barrier_extension(message)
        assert extension is not None
        commit = extension.get("commit")
        assert isinstance(commit, Mapping)
        generation = commit.get("mediaGeneration")
        revision = commit.get("stateRevision")
        if not isinstance(generation, int) or not isinstance(revision, int):
            raise StartGateSystemError("commit omitted numeric generation or revision")
        return generation, revision

    def close_peer(self, peer: RawPeer) -> None:
        peer.close()
        if peer in self.peers:
            self.peers.remove(peer)

    def slow_resolution_walk(self, suffix: str) -> None:
        room = f"sg-slow-{suffix}"
        controller = self.peer(room, "slow-control")
        follower = self.peer(room, "slow-peer")
        phase_walk = [snapshot_phase(controller.latest_snapshot() or {}) or ""]
        prepare_cursor = controller.cursor()
        self.send_prepare(
            controller,
            request_id="slow-operation",
            nonce=1,
            logical_media_id="slow-media",
            timeout_ms=6000,
        )
        waiting = self.observe_phase(controller, "waitingForIntent", after=prepare_cursor)
        phase_walk.append("waitingForIntent")
        generation = self.generation(waiting)
        self.checks.append("new-generation-waited-for-explicit-intent")

        late = self.peer(room, "slow-late")
        late_snapshot = self.observe_phase(late, "waitingForIntent")
        if self.generation(late_snapshot) != generation:
            raise StartGateSystemError("late joiner received the wrong media generation")
        late.wait_message(
            "late joiner preparing barrier snapshot",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("status"), Mapping)
            and extension["status"].get("phase") == "preparing",
        )
        self.checks.append("late-joiner-received-current-preparing-snapshot")

        intent_cursor = controller.cursor()
        for index, peer in enumerate((controller, follower, late), 1):
            self.send_intent(peer, f"slow-ready-{index}", 1)
        self.observe_phase(controller, "waitingForTechnicalReadiness", after=intent_cursor)
        phase_walk.append("waitingForTechnicalReadiness")
        self.checks.append("all-intent-advanced-to-technical-wait")

        for peer in (controller, late):
            self.send_technical(peer, generation)
        time.sleep(0.15)
        if snapshot_phase(controller.latest_snapshot() or {}) != "waitingForTechnicalReadiness":
            raise StartGateSystemError("slow participant was bypassed before technical readiness")
        technical_cursor = controller.cursor()
        self.send_technical(follower, generation)
        self.observe_phase(controller, "readyToCommit", after=technical_cursor)
        phase_walk.append("readyToCommit")
        self.checks.append("slow-resolution-blocked-until-last-playable-report")

        commit_cursor = controller.cursor()
        for peer in (controller, follower, late):
            self.send_target_ready(peer, generation)
        committed_generation, revision = self.commit_from(controller, after=commit_cursor)
        if committed_generation != generation:
            raise StartGateSystemError("commit changed the active media generation")
        self.observe_phase(controller, "committed", after=commit_cursor)
        phase_walk.append("committed")
        self.checks.append("one-canonical-generation-revision-committed")

        complete_cursor = controller.cursor()
        for peer in (controller, follower, late):
            self.send_started(peer, generation, revision)
        controller.wait_message(
            "complete barrier status",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("status"), Mapping)
            and extension["status"].get("phase") == "complete",
            after=complete_cursor,
        )
        self.observe_phase(controller, "inactive", after=complete_cursor)
        phase_walk.append("inactive")
        validate_phase_walk(phase_walk)
        self.checks.extend(
            [
                "all-started-acks-completed-barrier",
                "completed-barrier-cleared-readiness-gate",
                "ordered-phase-walk-validated",
            ]
        )
        self.scenarios.add("late-join-slow-resolution")
        for peer in (late, follower, controller):
            self.close_peer(peer)

    def reconnect_walk(self, suffix: str) -> None:
        room = f"sg-rejoin-{suffix}"
        controller = self.peer(room, "rejoin-control")
        keeper = self.peer(room, "rejoin-peer")
        original_epoch = controller.membership_epoch()
        for index, peer in enumerate((controller, keeper), 1):
            self.send_intent(peer, f"rejoin-ready-{index}", 1)
        self.send_prepare(
            controller,
            request_id="rejoin-operation",
            nonce=1,
            logical_media_id="rejoin-media",
            timeout_ms=6000,
        )
        waiting = self.observe_phase(controller, "waitingForTechnicalReadiness")
        generation = self.generation(waiting)
        first_token = controller.reconnect_token()
        partition_cursor = keeper.cursor()
        self.close_peer(controller)
        keeper.wait_message(
            "pre-commit partition evidence",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("status"), Mapping)
            and isinstance(extension["status"].get("participants"), Mapping)
            and isinstance(extension["status"]["participants"].get("rejoin-control"), Mapping)
            and extension["status"]["participants"]["rejoin-control"].get("degradedReason")
            == "disconnected",
            after=partition_cursor,
        )
        self.checks.append("pre-commit-partition-explicitly-observed")

        recovered = self.peer(room, "rejoin-control", first_token)
        if recovered.membership_epoch() != original_epoch:
            raise StartGateSystemError("reconnect did not preserve readiness membership epoch")
        recovery_cursor = recovered.cursor()
        self.send_recovery(
            recovered,
            request_id="rejoin-operation",
            original_nonce=1,
            recovery_nonce=2,
            logical_media_id="rejoin-media",
        )
        recovered.wait_message(
            "pre-commit recovered barrier",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("recovery"), Mapping)
            and extension["recovery"].get("disposition") == "recovered"
            and isinstance(extension.get("prepare"), Mapping),
            after=recovery_cursor,
        )
        self.checks.extend(
            [
                "pre-commit-reconnect-preserved-membership-epoch",
                "pre-commit-recovery-restored-preparing-lifecycle",
            ]
        )
        self.scenarios.add("partition-reconnect-before-commit")

        commit_cursor = keeper.cursor()
        for peer in (recovered, keeper):
            self.send_technical(peer, generation)
        for peer in (recovered, keeper):
            self.send_target_ready(peer, generation)
        committed_generation, revision = self.commit_from(keeper, after=commit_cursor)
        if committed_generation != generation:
            raise StartGateSystemError("recovered prepare committed another generation")
        self.observe_phase(keeper, "committed", after=commit_cursor)

        second_token = recovered.reconnect_token()
        committed_cursor = keeper.cursor()
        self.close_peer(recovered)
        keeper.wait_message(
            "committed partition evidence",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("status"), Mapping)
            and extension["status"].get("phase") == "committed"
            and isinstance(extension["status"].get("participants"), Mapping)
            and isinstance(extension["status"]["participants"].get("rejoin-control"), Mapping)
            and extension["status"]["participants"]["rejoin-control"].get("degradedReason")
            == "disconnected",
            after=committed_cursor,
        )
        resumed = self.peer(room, "rejoin-control", second_token)
        resume_cursor = resumed.cursor()
        self.send_recovery(
            resumed,
            request_id="rejoin-operation",
            original_nonce=1,
            recovery_nonce=3,
            logical_media_id="rejoin-media",
        )
        resumed.wait_message(
            "committed lifecycle recovery",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("recovery"), Mapping)
            and extension["recovery"].get("disposition") == "recovered"
            and isinstance(extension.get("commit"), Mapping)
            and isinstance(extension.get("status"), Mapping)
            and extension["status"].get("phase") == "committed",
            after=resume_cursor,
        )
        self.observe_phase(resumed, "committed")
        self.checks.extend(
            [
                "commit-remained-canonical-during-partition",
                "sleep-resume-recovered-commit-before-started",
            ]
        )
        self.scenarios.add("reconnect-between-commit-and-started")

        completion_cursor = keeper.cursor()
        self.send_started(keeper, generation, revision)
        self.send_started(resumed, generation, revision)
        keeper.wait_message(
            "recovered lifecycle completion",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("status"), Mapping)
            and extension["status"].get("phase") == "complete",
            after=completion_cursor,
        )
        self.observe_phase(keeper, "inactive", after=completion_cursor)
        self.checks.append("recovered-started-acks-cleared-gate")
        for peer in (resumed, keeper):
            self.close_peer(peer)

    def degraded_walk(self, suffix: str) -> None:
        room = f"sg-timeout-{suffix}"
        controller = self.peer(room, "timeout-control")
        slow = self.peer(room, "timeout-peer")
        start_cursor = controller.cursor()
        self.send_prepare(
            controller,
            request_id="timeout-operation",
            nonce=1,
            logical_media_id="timeout-media",
            timeout_ms=1000,
        )
        waiting = self.observe_phase(controller, "waitingForIntent", after=start_cursor)
        generation = self.generation(waiting)
        degraded = self.observe_phase(controller, "degraded", after=start_cursor, timeout=5.0)
        phase = degraded.get("startGatePhase")
        if not isinstance(phase, Mapping) or phase.get("reason") != "timedOut":
            raise StartGateSystemError("degraded start gate did not expose timeout reason")
        controller.wait_message(
            "degraded remain-paused barrier",
            lambda value: (extension := barrier_extension(value)) is not None
            and isinstance(extension.get("status"), Mapping)
            and extension["status"].get("mediaGeneration") == generation
            and extension["status"].get("phase") == "degraded",
            after=start_cursor,
            timeout=5.0,
        )
        if any(
            isinstance(extension := barrier_extension(record.message), Mapping)
            and isinstance(extension.get("commit"), Mapping)
            for record in controller.messages_after(start_cursor)
        ):
            raise StartGateSystemError("remain-paused timeout manufactured a start commit")
        self.checks.extend(
            [
                "bounded-timeout-entered-degraded-phase",
                "timeout-reason-remained-explicit",
                "remain-paused-timeout-did-not-commit",
            ]
        )

        late = self.peer(room, "timeout-late")
        late_degraded = self.observe_phase(late, "degraded")
        if self.generation(late_degraded) != generation:
            raise StartGateSystemError("late degraded snapshot had the wrong generation")
        self.checks.append("late-joiner-received-degraded-snapshot")
        self.scenarios.add("timeout-degraded-late-join")

        token = late.reconnect_token()
        self.close_peer(late)
        time.sleep(0.15)
        resumed = self.peer(room, "timeout-late", token)
        resumed_snapshot = self.observe_phase(resumed, "degraded")
        resumed_phase = resumed_snapshot.get("startGatePhase")
        if not isinstance(resumed_phase, Mapping) or resumed_phase.get("reason") != "timedOut":
            raise StartGateSystemError("sleep/resume lost the terminal gate disposition")
        self.checks.extend(
            [
                "sleep-resume-restored-readiness-membership",
                "sleep-resume-received-explicit-terminal-snapshot",
            ]
        )
        self.scenarios.add("sleep-resume-degraded-snapshot")
        for peer in (resumed, slow, controller):
            self.close_peer(peer)

    def run(self) -> dict[str, Any]:
        if self.artifact_dir.exists() and any(self.artifact_dir.iterdir()):
            raise StartGateSystemError("artifact directory must be empty")
        self.artifact_dir.mkdir(parents=True, exist_ok=True)
        binding = self.preflight()
        environment = clean_environment()
        environment.update(
            {
                "SOROTTE_LIFECYCLE_EVIDENCE_PATH": str(self.server_lifecycle),
                "SOROTTE_LIFECYCLE_RUN_ID": self.run_id,
                "SOROTTE_LIFECYCLE_EMITTER": "server",
            }
        )
        self.server = ServerProcess(self.server_path, self.repo_root, self.artifact_dir, environment)
        server_code: int | None = None
        failure: BaseException | None = None
        try:
            self.server.wait_ready()
            self.checks.append("actual-release-server-listening-with-readiness-enabled")
            suffix = self.run_id[-8:].lower()
            self.slow_resolution_walk(suffix)
            self.reconnect_walk(suffix)
            self.degraded_walk(suffix)
            wait_until(
                "server clear transition flush",
                lambda: self.server_lifecycle.is_file()
                and "GATE-CLEAR-001" in self.server_lifecycle.read_text(
                    encoding="utf-8", errors="replace"
                ),
            )
        except BaseException as error:
            failure = error
        finally:
            for peer in list(reversed(self.peers)):
                self.close_peer(peer)
            if self.server is not None:
                server_code = self.server.stop()
        if failure is not None:
            raise failure
        if server_code != 0:
            raise StartGateSystemError(f"release server exited with code {server_code}")
        self.checks.append("bounded-graceful-server-shutdown")

        server_records = lifecycle_evidence.read_jsonl(self.server_lifecycle)
        inventory = lifecycle_evidence.validate_inventory(server_records[0])
        if inventory.product_digest != self.server_digest:
            raise StartGateSystemError("server lifecycle inventory digest did not match executable")
        by_transition: dict[str, list[Mapping[str, Any]]] = {
            transition: [] for transition in REQUIRED_TRANSITIONS
        }
        for record in server_records[1:]:
            transition = record.get("transition")
            if transition in by_transition:
                by_transition[transition].append(record)
        missing = [transition for transition, records in by_transition.items() if not records]
        if missing:
            raise StartGateSystemError(f"server lifecycle omitted required start-gate transitions: {missing}")
        self.checks.append("server-ledger-covered-every-start-gate-transition")

        writer = lifecycle_evidence.EvidenceWriter(
            self.oracle_lifecycle,
            run_id=self.run_id,
            emitter="start-gate-oracle",
            binary_role="harness",
            component_roles=("harness", "oracle"),
            product_version=self.product_version,
            product_digest=sha256_file(Path(__file__).resolve()),
        )
        try:
            for transition in REQUIRED_TRANSITIONS:
                server_record = by_transition[transition][0]
                writer.emit(
                    process_role="oracle",
                    subject="start-gate-walk",
                    machine="start-gate",
                    transition=transition,
                    target_kind="server-state",
                    trigger="internal",
                    authority_before=str(server_record["authority_before"]),
                    authority_after=str(server_record["authority_after"]),
                    expected_effect="phase-observed",
                    observed_effect="phase-observed",
                    disposition="observed",
                    identities={
                        key: value
                        for key, value in server_record.get("identities", {}).items()
                        if isinstance(value, int) and value > 0
                    },
                    causal_predecessors=(str(server_record["event_id"]),),
                )
        finally:
            writer.close()
        lifecycle_summary = lifecycle_evidence.validate_and_merge(
            [self.server_lifecycle, self.oracle_lifecycle],
            model_path=self.repo_root / "coverage" / "playback-lifecycle.toml",
            output_path=self.merged_lifecycle,
            summary_path=self.lifecycle_summary_path,
            required_inventories={
                "server": frozenset({"server"}),
                "start-gate-oracle": frozenset({"harness", "oracle"}),
            },
            required_roles=frozenset({"server", "oracle"}),
            expected_digests={"server": self.server_digest},
            minimum_cross_process_edges=len(REQUIRED_TRANSITIONS),
        )
        self.checks.append("closed-cross-process-causal-ledger-validated")

        if set(self.observed_phases) != set(REQUIRED_PHASES):
            raise StartGateSystemError(
                f"phase coverage incomplete: {sorted(set(REQUIRED_PHASES) - self.observed_phases)}"
            )
        if self.scenarios != set(REQUIRED_SCENARIOS):
            raise StartGateSystemError("generated scenario coverage is incomplete")
        self.checks.extend(
            [
                "closed-phase-grammar-complete",
                "generated-scenario-matrix-complete",
                "exact-server-digest-attested",
            ]
        )
        assert self.server is not None
        report = {
            "schema_version": SCHEMA_VERSION,
            "kind": REPORT_KIND,
            "result": "passed",
            "candidate_sha": self.candidate_sha,
            "candidate_binding": binding,
            "run_id": self.run_id,
            "composition": [
                "actual-release-server",
                "independent-production-wire-peers",
                "external-phase-oracle",
                "closed-cross-process-lifecycle-ledger",
            ],
            "server": {"file_name": self.server_path.name, "sha256": self.server_digest},
            "phase_coverage": list(REQUIRED_PHASES),
            "transition_coverage": list(REQUIRED_TRANSITIONS),
            "scenario_coverage": list(REQUIRED_SCENARIOS),
            "checks": self.checks,
            "lifecycle_summary": lifecycle_summary,
            "artifacts": {
                "server_lifecycle": self.server_lifecycle.name,
                "oracle_lifecycle": self.oracle_lifecycle.name,
                "merged_lifecycle": self.merged_lifecycle.name,
                "lifecycle_summary": self.lifecycle_summary_path.name,
                "server_stdout": self.server.stdout_path.name,
                "server_stderr": self.server.stderr_path.name,
            },
        }
        validate_report(report)
        atomic_write_json(self.report_path, report)
        return report


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", default=Path(__file__).resolve().parents[1])
    parser.add_argument("--server", required=True)
    parser.add_argument("--artifact-dir", required=True)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--run-id")
    parser.add_argument("--allow-unverified-candidate", action="store_true")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    try:
        report = StartGateHarness(parse_args(argv or tuple(os.sys.argv[1:]))).run()
    except (StartGateSystemError, lifecycle_evidence.EvidenceError, OSError, subprocess.SubprocessError) as error:
        print(json.dumps({"kind": REPORT_KIND, "result": "failed", "error": str(error)}), file=os.sys.stderr)
        return 1
    print(
        json.dumps(
            {
                "kind": report["kind"],
                "result": report["result"],
                "candidate_sha": report["candidate_sha"],
                "checks": len(report["checks"]),
                "phases": len(report["phase_coverage"]),
                "scenarios": len(report["scenario_coverage"]),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
