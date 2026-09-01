#!/usr/bin/env python3
"""Closed, deterministic fault schedules for playback lifecycle verification."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import threading
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence


SCHEMA_VERSION = 1
SCHEDULE_KIND = "sorotte-playback-lifecycle-fault-schedule"
TRACE_KIND = "sorotte-playback-lifecycle-fault-replay"
TOKEN = re.compile(r"^[A-Za-z0-9._:-]{1,128}$")
SCHEDULE_KEYS = {"schema_version", "kind", "schedule_id", "seed", "steps"}
STEP_KEYS = {"id", "boundary", "action", "occurrence", "value"}
TRACE_KEYS = {
    "schema_version",
    "kind",
    "schedule_id",
    "sequence",
    "step_id",
    "boundary",
    "action",
    "occurrence",
    "value",
    "outcome",
}

BOUNDARIES = {
    "client-to-server-frame",
    "server-to-client-frame",
    "client-worker",
    "server-worker",
    "player-ipc-command",
    "player-ipc-response",
    "player-process",
    "harness-backpressure",
    "harness-partition",
    "harness-reconnect",
}

ACTION_BOUNDARIES = {
    "delay": BOUNDARIES,
    "fragment": {"client-to-server-frame", "server-to-client-frame"},
    "backpressure": {
        "client-to-server-frame",
        "server-to-client-frame",
        "harness-backpressure",
    },
    "half-close": {
        "client-to-server-frame",
        "server-to-client-frame",
        "harness-partition",
    },
    "reset": {
        "client-to-server-frame",
        "server-to-client-frame",
        "harness-partition",
    },
    "worker-stall": {"client-worker", "server-worker"},
    "channel-hold": {"client-worker", "server-worker", "harness-partition"},
    "channel-release": {"client-worker", "server-worker", "harness-reconnect"},
    "ipc-withhold": {"player-ipc-response"},
    "ipc-partial": {"player-ipc-response"},
    "ipc-reset": {"player-ipc-command", "player-ipc-response"},
    "ipc-process-exit": {"player-process"},
}

# Duration actions are bounded so a malformed schedule cannot turn a required
# verification lane into an unbounded sleeper. Other values are byte counts or
# stable non-zero action ordinals.
ACTION_VALUE_BOUNDS = {
    "delay": (1, 5_000),
    "fragment": (1, 65_536),
    "backpressure": (1, 5_000),
    "half-close": (1, 1),
    "reset": (1, 1),
    "worker-stall": (1, 5_000),
    "channel-hold": (1, 16),
    "channel-release": (1, 16),
    "ipc-withhold": (1, 5_000),
    "ipc-partial": (1, 65_536),
    "ipc-reset": (1, 1),
    "ipc-process-exit": (1, 1),
}


class FaultScheduleError(ValueError):
    pass


def _token(field: str, value: Any) -> str:
    if not isinstance(value, str) or TOKEN.fullmatch(value) is None:
        raise FaultScheduleError(f"{field} must be a bounded privacy-safe token")
    return value


def _positive_integer(field: str, value: Any) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise FaultScheduleError(f"{field} must be a positive integer")
    return value


def _non_negative_integer(field: str, value: Any) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise FaultScheduleError(f"{field} must be a non-negative integer")
    return value


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


@dataclass(frozen=True)
class FaultStep:
    id: str
    boundary: str
    action: str
    occurrence: int
    value: int

    @classmethod
    def parse(cls, raw: Mapping[str, Any]) -> "FaultStep":
        if set(raw) != STEP_KEYS:
            raise FaultScheduleError("fault step does not use the exact closed schema")
        step = cls(
            id=_token("step id", raw.get("id")),
            boundary=_token("boundary", raw.get("boundary")),
            action=_token("action", raw.get("action")),
            occurrence=_positive_integer("occurrence", raw.get("occurrence")),
            value=_positive_integer("value", raw.get("value")),
        )
        if step.boundary not in BOUNDARIES:
            raise FaultScheduleError(f"unknown fault boundary {step.boundary}")
        allowed_boundaries = ACTION_BOUNDARIES.get(step.action)
        if allowed_boundaries is None:
            raise FaultScheduleError(f"unknown fault action {step.action}")
        if step.boundary not in allowed_boundaries:
            raise FaultScheduleError(
                f"fault action {step.action} is invalid at {step.boundary}"
            )
        minimum, maximum = ACTION_VALUE_BOUNDS[step.action]
        if not minimum <= step.value <= maximum:
            raise FaultScheduleError(
                f"fault action {step.action} value must be in [{minimum}, {maximum}]"
            )
        return step

    def as_json(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "boundary": self.boundary,
            "action": self.action,
            "occurrence": self.occurrence,
            "value": self.value,
        }


@dataclass(frozen=True)
class FaultSchedule:
    schedule_id: str
    seed: int
    steps: tuple[FaultStep, ...]

    @classmethod
    def parse(cls, raw: Mapping[str, Any]) -> "FaultSchedule":
        if set(raw) != SCHEDULE_KEYS:
            raise FaultScheduleError("fault schedule does not use the exact closed schema")
        if raw.get("schema_version") != SCHEMA_VERSION or raw.get("kind") != SCHEDULE_KIND:
            raise FaultScheduleError("fault schedule schema version or kind is unsupported")
        raw_steps = raw.get("steps")
        if not isinstance(raw_steps, list) or not raw_steps:
            raise FaultScheduleError("fault schedule must contain at least one step")
        if len(raw_steps) > 256:
            raise FaultScheduleError("fault schedule exceeds the bounded step count")
        parsed_steps: list[FaultStep] = []
        for step in raw_steps:
            if not isinstance(step, dict):
                raise FaultScheduleError("fault step must be an object")
            parsed_steps.append(FaultStep.parse(step))
        steps = tuple(parsed_steps)
        ids = [step.id for step in steps]
        if len(ids) != len(set(ids)):
            raise FaultScheduleError("fault step ids must be unique")
        per_boundary_occurrences: dict[str, int] = {}
        for step in steps:
            previous = per_boundary_occurrences.get(step.boundary, 0)
            if step.occurrence <= previous:
                raise FaultScheduleError(
                    "fault step occurrences must increase within each boundary"
                )
            per_boundary_occurrences[step.boundary] = step.occurrence
        return cls(
            schedule_id=_token("schedule_id", raw.get("schedule_id")),
            seed=_non_negative_integer("seed", raw.get("seed")),
            steps=steps,
        )

    @classmethod
    def read(cls, path: Path) -> "FaultSchedule":
        raw = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(raw, dict):
            raise FaultScheduleError("fault schedule root must be an object")
        return cls.parse(raw)

    def as_json(self) -> dict[str, Any]:
        return {
            "schema_version": SCHEMA_VERSION,
            "kind": SCHEDULE_KIND,
            "schedule_id": self.schedule_id,
            "seed": self.seed,
            "steps": [step.as_json() for step in self.steps],
        }

    def write_atomic(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
        temporary.write_text(
            json.dumps(self.as_json(), indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, path)


class FaultReplayLedger:
    def __init__(self, path: Path, schedule_id: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        self.path = path
        self.schedule_id = _token("schedule_id", schedule_id)
        self._output = path.open("x", encoding="utf-8", newline="\n")
        self._lock = threading.Lock()
        self._sequence = 0

    def record(self, step: FaultStep, outcome: str) -> None:
        outcome = _token("outcome", outcome)
        with self._lock:
            self._sequence += 1
            record = {
                "schema_version": SCHEMA_VERSION,
                "kind": TRACE_KIND,
                "schedule_id": self.schedule_id,
                "sequence": self._sequence,
                "step_id": step.id,
                "boundary": step.boundary,
                "action": step.action,
                "occurrence": step.occurrence,
                "value": step.value,
                "outcome": outcome,
            }
            self._output.write(json.dumps(record, sort_keys=True) + "\n")
            self._output.flush()

    def close(self) -> None:
        self._output.flush()
        self._output.close()


class FaultScheduleCursor:
    """Thread-safe replay cursor triggered by named runtime boundaries."""

    def __init__(
        self,
        schedule: FaultSchedule,
        *,
        ledger: FaultReplayLedger | None = None,
    ) -> None:
        self.schedule = schedule
        self.ledger = ledger
        self._lock = threading.Lock()
        self._boundary_counts: dict[str, int] = {}
        self._next_step = 0

    def checkpoint(
        self,
        boundary: str,
        executor: Callable[[FaultStep], None],
    ) -> FaultStep | None:
        if boundary not in BOUNDARIES:
            raise FaultScheduleError(f"unknown runtime fault boundary {boundary}")
        with self._lock:
            occurrence = self._boundary_counts.get(boundary, 0) + 1
            self._boundary_counts[boundary] = occurrence
            if self._next_step >= len(self.schedule.steps):
                return None
            step = self.schedule.steps[self._next_step]
            if step.boundary != boundary or step.occurrence != occurrence:
                return None
            # Advance before executing so a callback that observes another
            # boundary cannot consume this same step twice.
            self._next_step += 1
        try:
            executor(step)
        except BaseException:
            if self.ledger is not None:
                self.ledger.record(step, "failed")
            raise
        if self.ledger is not None:
            self.ledger.record(step, "applied")
        return step

    @property
    def consumed_count(self) -> int:
        with self._lock:
            return self._next_step

    def assert_consumed(self) -> None:
        with self._lock:
            if self._next_step != len(self.schedule.steps):
                pending = self.schedule.steps[self._next_step]
                raise FaultScheduleError(
                    f"fault schedule stopped before step {pending.id} at {pending.boundary}"
                )


def shrink_fault_schedule(
    schedule: FaultSchedule,
    reproduces_failure: Callable[[FaultSchedule], bool],
) -> FaultSchedule:
    """Deterministically delta-debug a reproducing schedule."""

    if not reproduces_failure(schedule):
        raise FaultScheduleError("the supplied schedule does not reproduce the failure")
    steps = list(schedule.steps)
    granularity = 2
    while len(steps) >= 2:
        chunk_size = (len(steps) + granularity - 1) // granularity
        reduced = False
        for start in range(0, len(steps), chunk_size):
            candidate_steps = steps[:start] + steps[start + chunk_size :]
            if not candidate_steps:
                continue
            candidate = replace(schedule, steps=tuple(candidate_steps))
            if reproduces_failure(candidate):
                steps = candidate_steps
                granularity = max(2, granularity - 1)
                reduced = True
                break
        if reduced:
            continue
        if granularity >= len(steps):
            break
        granularity = min(len(steps), granularity * 2)
    return replace(schedule, steps=tuple(steps))


def read_replay_trace(path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for expected_sequence, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        raw = json.loads(line)
        if not isinstance(raw, dict) or set(raw) != TRACE_KEYS:
            raise FaultScheduleError("fault replay trace does not use the exact closed schema")
        if raw.get("schema_version") != SCHEMA_VERSION or raw.get("kind") != TRACE_KIND:
            raise FaultScheduleError("fault replay trace schema version or kind is unsupported")
        if raw.get("sequence") != expected_sequence:
            raise FaultScheduleError("fault replay trace sequence is not contiguous")
        _token("schedule_id", raw.get("schedule_id"))
        FaultStep.parse(
            {
                "id": raw.get("step_id"),
                "boundary": raw.get("boundary"),
                "action": raw.get("action"),
                "occurrence": raw.get("occurrence"),
                "value": raw.get("value"),
            }
        )
        _token("outcome", raw.get("outcome"))
        records.append(raw)
    return records


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("schedule", type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        schedule = FaultSchedule.read(args.schedule)
    except (FaultScheduleError, OSError, json.JSONDecodeError) as error:
        print(json.dumps({"kind": SCHEDULE_KIND, "result": "failed", "message": str(error)}))
        return 1
    print(
        json.dumps(
            {
                "kind": SCHEDULE_KIND,
                "result": "passed",
                "schedule_id": schedule.schedule_id,
                "step_count": len(schedule.steps),
                "sha256": sha256_file(args.schedule),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
