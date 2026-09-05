#!/usr/bin/env python3
"""Validate and merge Sorotte's privacy-safe cross-process lifecycle ledger."""

from __future__ import annotations

import argparse
import json
import os
import re
import time
import tomllib
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence

import artifact_input


SCHEMA_VERSION = 1
MAX_EVIDENCE_BYTES = 128 * 1024 * 1024
MAX_EVIDENCE_RECORD_BYTES = 64 * 1024
MAX_EVIDENCE_RECORDS = 200_000
SCHEMA_KIND = "sorotte-playback-lifecycle-evidence"
VALIDATION_KIND = "sorotte-playback-lifecycle-evidence-validation"
TOKEN = re.compile(r"^[A-Za-z0-9._:-]{1,128}$")
DIGEST = re.compile(r"^[0-9a-f]{64}$")
EVENT_ID = re.compile(r"^(?P<emitter>[A-Za-z0-9._:-]{1,128})\.(?P<sequence>[0-9]{8})$")

ROLES = {"server", "client", "gui", "player", "proxy", "harness", "oracle"}
TARGET_KINDS = {
    "none",
    "process-boundary",
    "protocol-message",
    "server-state",
    "player-command",
    "player-state",
    "gui-projection",
    "fault-boundary",
}
TRIGGERS = {
    "startup",
    "shutdown",
    "local-input",
    "remote-event",
    "player-event",
    "timer",
    "fault",
    "recovery",
    "internal",
}
DISPOSITIONS = {
    "observed",
    "submitted",
    "accepted",
    "committed",
    "applied",
    "rejected",
    "superseded",
    "failed",
    "timed-out",
}

COMMON_KEYS = {
    "schema_version",
    "kind",
    "record_type",
    "event_id",
    "run_id",
    "monotonic_ns",
    "emitter",
}
INVENTORY_KEYS = COMMON_KEYS | {
    "binary_role",
    "component_roles",
    "product_name",
    "product_version",
    "product_digest",
}
TRANSITION_KEYS = COMMON_KEYS | {
    "process_role",
    "subject",
    "machine",
    "transition",
    "causal_predecessors",
    "identities",
    "target_kind",
    "trigger",
    "authority_before",
    "authority_after",
    "expected_effect",
    "observed_effect",
    "disposition",
    "deadline_ms",
    "deadline_expired",
}


class EvidenceError(ValueError):
    pass


def _token(field: str, value: Any) -> str:
    if not isinstance(value, str) or TOKEN.fullmatch(value) is None:
        raise EvidenceError(f"{field} must be a bounded privacy-safe token")
    return value


def _positive_integer(field: str, value: Any) -> int:
    if not artifact_input.is_json_integer(value) or value <= 0:
        raise EvidenceError(f"{field} must be a positive integer")
    return value


def _non_negative_integer(field: str, value: Any) -> int:
    if not artifact_input.is_json_integer(value) or value < 0:
        raise EvidenceError(f"{field} must be a non-negative integer")
    return value


def sha256_file(path: Path) -> str:
    return artifact_input.sha256_file(path)


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        raise EvidenceError("declared lifecycle evidence file is missing")
    try:
        records = artifact_input.strict_jsonl_load(
            path, max_bytes=MAX_EVIDENCE_BYTES, max_record_bytes=MAX_EVIDENCE_RECORD_BYTES,
            max_records=MAX_EVIDENCE_RECORDS, label="lifecycle evidence",
        )
    except artifact_input.ArtifactInputError as error:
        raise EvidenceError(str(error)) from error
    if not records:
        raise EvidenceError("lifecycle evidence file is empty")
    return records


def load_model_transitions(path: Path) -> dict[str, str]:
    try:
        raw = artifact_input.read_bounded(path, max_bytes=4 * 1024 * 1024, label="lifecycle model")
        document = tomllib.loads(raw.decode("utf-8", errors="strict"))
    except (artifact_input.ArtifactInputError, UnicodeError) as error:
        raise EvidenceError(str(error)) from error
    transitions: dict[str, str] = {}
    for machine in document.get("machine", []):
        machine_id = _token("model machine id", machine.get("id"))
        for transition in machine.get("transition", []):
            transition_id = _token("model transition id", transition.get("id"))
            if transition_id in transitions:
                raise EvidenceError(f"duplicate model transition {transition_id}")
            transitions[transition_id] = machine_id
    if not transitions:
        raise EvidenceError("lifecycle model contains no transitions")
    return transitions


def _validate_common(record: Mapping[str, Any], expected_type: str) -> tuple[str, int]:
    if not artifact_input.is_json_integer(record.get("schema_version")) or record.get("schema_version") != SCHEMA_VERSION:
        raise EvidenceError("evidence has an unsupported schema version")
    if record.get("kind") != SCHEMA_KIND:
        raise EvidenceError("evidence has the wrong kind")
    if record.get("record_type") != expected_type:
        raise EvidenceError(f"expected {expected_type} evidence record")
    event_id = _token("event_id", record.get("event_id"))
    run_id = _token("run_id", record.get("run_id"))
    del run_id
    emitter = _token("emitter", record.get("emitter"))
    match = EVENT_ID.fullmatch(event_id)
    if match is None or match.group("emitter") != emitter:
        raise EvidenceError("event_id does not belong to its emitter")
    monotonic_ns = _non_negative_integer("monotonic_ns", record.get("monotonic_ns"))
    return event_id, monotonic_ns


@dataclass(frozen=True)
class Inventory:
    emitter: str
    binary_role: str
    component_roles: frozenset[str]
    product_version: str
    product_digest: str


def validate_inventory(record: Mapping[str, Any]) -> Inventory:
    if set(record) != INVENTORY_KEYS:
        raise EvidenceError("process inventory does not use the exact closed schema")
    _validate_common(record, "process-inventory")
    binary_role = _token("binary_role", record.get("binary_role"))
    roles = record.get("component_roles")
    if (
        not isinstance(roles, list)
        or not roles
        or len(roles) != len(set(roles))
        or any(role not in ROLES for role in roles)
    ):
        raise EvidenceError("component_roles must be a non-empty unique role list")
    if binary_role not in ROLES or binary_role not in roles:
        raise EvidenceError("binary_role must be one of its declared component roles")
    if record.get("product_name") != "sorotte":
        raise EvidenceError("process inventory has the wrong product name")
    product_version = _token("product_version", record.get("product_version"))
    product_digest = record.get("product_digest")
    if not isinstance(product_digest, str) or DIGEST.fullmatch(product_digest) is None:
        raise EvidenceError("product_digest must be lowercase SHA-256")
    return Inventory(
        emitter=_token("emitter", record.get("emitter")),
        binary_role=binary_role,
        component_roles=frozenset(roles),
        product_version=product_version,
        product_digest=product_digest,
    )


def validate_transition(
    record: Mapping[str, Any],
    inventory: Inventory,
    model_transitions: Mapping[str, str],
) -> None:
    if set(record) != TRANSITION_KEYS:
        raise EvidenceError("transition evidence does not use the exact closed schema")
    _validate_common(record, "transition")
    process_role = _token("process_role", record.get("process_role"))
    if process_role not in inventory.component_roles:
        raise EvidenceError("transition role is absent from its process inventory")
    for field in (
        "subject",
        "machine",
        "transition",
        "authority_before",
        "authority_after",
        "expected_effect",
        "observed_effect",
    ):
        _token(field, record.get(field))
    transition = record["transition"]
    machine = record["machine"]
    if model_transitions.get(transition) != machine:
        raise EvidenceError(
            f"transition {transition} on {machine} is absent from the model or assigned to the wrong machine at {record['event_id']}"
        )
    predecessors = record.get("causal_predecessors")
    if (
        not isinstance(predecessors, list)
        or not predecessors
        or len(predecessors) != len(set(predecessors))
        or len(predecessors) > 16
    ):
        raise EvidenceError("transition must declare a bounded unique causal predecessor list")
    for predecessor in predecessors:
        _token("causal_predecessor", predecessor)
    identities = record.get("identities")
    if not isinstance(identities, dict) or len(identities) > 16:
        raise EvidenceError("identities must be a bounded object")
    for name, value in identities.items():
        _token("identity_name", name)
        _positive_integer(f"identity {name}", value)
    if record.get("target_kind") not in TARGET_KINDS:
        raise EvidenceError("transition has an unknown target_kind")
    if record.get("trigger") not in TRIGGERS:
        raise EvidenceError("transition has an unknown trigger")
    if record.get("disposition") not in DISPOSITIONS:
        raise EvidenceError("transition has an unknown disposition")
    deadline_ms = record.get("deadline_ms")
    if deadline_ms is not None:
        _non_negative_integer("deadline_ms", deadline_ms)
    if not isinstance(record.get("deadline_expired"), bool):
        raise EvidenceError("deadline_expired must be boolean")
    if record["deadline_expired"] and deadline_ms is None:
        raise EvidenceError("expired transition has no deadline")


def _atomic_write_json(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def _atomic_write_jsonl(path: Path, records: Iterable[Mapping[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with temporary.open("x", encoding="utf-8", newline="\n") as output:
        for record in records:
            output.write(json.dumps(record, sort_keys=True, ensure_ascii=False) + "\n")
    os.replace(temporary, path)


def validate_and_merge(
    inputs: Sequence[Path],
    *,
    model_path: Path,
    output_path: Path | None = None,
    summary_path: Path | None = None,
    required_inventories: Mapping[str, frozenset[str]] | None = None,
    required_roles: frozenset[str] = frozenset(),
    expected_digests: Mapping[str, str] | None = None,
    minimum_cross_process_edges: int = 0,
) -> dict[str, Any]:
    if not inputs:
        raise EvidenceError("at least one lifecycle evidence input is required")
    _non_negative_integer("minimum_cross_process_edges", minimum_cross_process_edges)
    model_transitions = load_model_transitions(model_path)
    all_records: list[dict[str, Any]] = []
    inventories: dict[str, Inventory] = {}
    event_ids: set[str] = set()
    run_ids: set[str] = set()
    role_counts: Counter[str] = Counter()
    transition_counts: Counter[str] = Counter()

    for path in inputs:
        records = read_jsonl(path)
        inventory = validate_inventory(records[0])
        if inventory.emitter in inventories:
            raise EvidenceError(f"duplicate process inventory for emitter {inventory.emitter}")
        inventories[inventory.emitter] = inventory
        previous_event_id: str | None = None
        previous_monotonic_ns = 0
        for sequence, record in enumerate(records, 1):
            expected_type = "process-inventory" if sequence == 1 else "transition"
            event_id, monotonic_ns = _validate_common(record, expected_type)
            match = EVENT_ID.fullmatch(event_id)
            assert match is not None
            if int(match.group("sequence")) != sequence:
                raise EvidenceError("emitter event sequence is not contiguous")
            if event_id in event_ids:
                raise EvidenceError(f"duplicate lifecycle event id {event_id}")
            if monotonic_ns < previous_monotonic_ns:
                raise EvidenceError("emitter monotonic clock moved backwards")
            run_ids.add(_token("run_id", record.get("run_id")))
            if record.get("emitter") != inventory.emitter:
                raise EvidenceError("evidence file contains multiple emitters")
            if sequence > 1:
                validate_transition(record, inventory, model_transitions)
                if previous_event_id not in record["causal_predecessors"]:
                    raise EvidenceError("transition does not retain its immediate local cause")
                role_counts[record["process_role"]] += 1
                transition_counts[record["transition"]] += 1
            event_ids.add(event_id)
            all_records.append(record)
            previous_event_id = event_id
            previous_monotonic_ns = monotonic_ns

    if len(run_ids) != 1:
        raise EvidenceError("merged evidence contains multiple run ids")
    for record in all_records:
        if record["record_type"] == "transition":
            unknown = set(record["causal_predecessors"]) - event_ids
            if unknown:
                raise EvidenceError("transition references an unknown causal predecessor")

    cross_process_edges = {
        (predecessor, record["event_id"])
        for record in all_records
        if record["record_type"] == "transition"
        for predecessor in record["causal_predecessors"]
        if EVENT_ID.fullmatch(predecessor).group("emitter") != record["emitter"]  # type: ignore[union-attr]
    }
    if len(cross_process_edges) < minimum_cross_process_edges:
        raise EvidenceError(
            "merged evidence has too few cross-process causal edges: "
            f"expected at least {minimum_cross_process_edges}, observed {len(cross_process_edges)}"
        )

    for emitter, expected_roles in (required_inventories or {}).items():
        inventory = inventories.get(emitter)
        if inventory is None:
            raise EvidenceError(f"required process inventory {emitter} is missing")
        if inventory.component_roles != expected_roles:
            raise EvidenceError(f"process inventory {emitter} has the wrong component roles")
    missing_roles = required_roles - set(role_counts)
    if missing_roles:
        raise EvidenceError(f"required product roles emitted no transitions: {sorted(missing_roles)}")
    for emitter, expected_digest in (expected_digests or {}).items():
        if DIGEST.fullmatch(expected_digest) is None:
            raise EvidenceError("expected digest is not lowercase SHA-256")
        inventory = inventories.get(emitter)
        if inventory is None or inventory.product_digest != expected_digest:
            raise EvidenceError(f"process inventory {emitter} does not match its exact artifact")

    ordered_records = sorted(
        all_records,
        key=lambda record: (
            record["emitter"],
            int(EVENT_ID.fullmatch(record["event_id"]).group("sequence")),  # type: ignore[union-attr]
        ),
    )
    if output_path is not None:
        if output_path.exists():
            raise EvidenceError("merged lifecycle evidence output already exists")
        _atomic_write_jsonl(output_path, ordered_records)

    versions = sorted({inventory.product_version for inventory in inventories.values()})
    summary: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "kind": VALIDATION_KIND,
        "result": "passed",
        "run_id": next(iter(run_ids)),
        "process_count": len(inventories),
        "event_count": len(all_records),
        "transition_count": sum(transition_counts.values()),
        "cross_process_edge_count": len(cross_process_edges),
        "emitters": sorted(inventories),
        "component_roles": sorted(
            {role for inventory in inventories.values() for role in inventory.component_roles}
        ),
        "emitted_roles": dict(sorted(role_counts.items())),
        "transitions": dict(sorted(transition_counts.items())),
        "product_versions": versions,
    }
    if output_path is not None:
        summary["merged_sha256"] = sha256_file(output_path)
    if summary_path is not None:
        if summary_path.exists():
            raise EvidenceError("lifecycle evidence summary output already exists")
        _atomic_write_json(summary_path, summary)
    return summary


class EvidenceWriter:
    """Small Python emitter used by the external harness, proxy, and oracle."""

    def __init__(
        self,
        path: Path,
        *,
        run_id: str,
        emitter: str,
        binary_role: str,
        component_roles: Sequence[str],
        product_version: str,
        product_digest: str,
    ) -> None:
        self.path = path
        self.run_id = _token("run_id", run_id)
        self.emitter = _token("emitter", emitter)
        roles = list(component_roles)
        if not roles or len(roles) != len(set(roles)) or any(role not in ROLES for role in roles):
            raise EvidenceError("component_roles must be a unique non-empty role list")
        if binary_role not in roles:
            raise EvidenceError("binary_role must be declared as a component role")
        _token("product_version", product_version)
        if DIGEST.fullmatch(product_digest) is None:
            raise EvidenceError("product_digest must be lowercase SHA-256")
        path.parent.mkdir(parents=True, exist_ok=True)
        self._output = path.open("x", encoding="utf-8", newline="\n")
        self._origin = time.monotonic_ns()
        self._sequence = 1
        self._last_event_id = self._event_id()
        self._roles = frozenset(roles)
        self._write(
            {
                "schema_version": SCHEMA_VERSION,
                "kind": SCHEMA_KIND,
                "record_type": "process-inventory",
                "event_id": self._last_event_id,
                "run_id": self.run_id,
                "monotonic_ns": 0,
                "emitter": self.emitter,
                "binary_role": binary_role,
                "component_roles": roles,
                "product_name": "sorotte",
                "product_version": product_version,
                "product_digest": product_digest,
            }
        )
        self._sequence += 1

    def _event_id(self) -> str:
        return f"{self.emitter}.{self._sequence:08d}"

    def _write(self, record: Mapping[str, Any]) -> None:
        self._output.write(json.dumps(record, sort_keys=True, ensure_ascii=False) + "\n")
        self._output.flush()

    def emit(
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
        causal_predecessors: Sequence[str] = (),
        deadline_ms: int | None = None,
        deadline_expired: bool = False,
    ) -> str:
        if process_role not in self._roles:
            raise EvidenceError("transition role is absent from writer inventory")
        for field, value in (
            ("subject", subject),
            ("machine", machine),
            ("transition", transition),
            ("authority_before", authority_before),
            ("authority_after", authority_after),
            ("expected_effect", expected_effect),
            ("observed_effect", observed_effect),
        ):
            _token(field, value)
        if target_kind not in TARGET_KINDS or trigger not in TRIGGERS or disposition not in DISPOSITIONS:
            raise EvidenceError("transition enum value is unknown")
        safe_identities = dict(identities or {})
        for name, value in safe_identities.items():
            _token("identity_name", name)
            _positive_integer(f"identity {name}", value)
        predecessors = list(causal_predecessors)
        if self._last_event_id not in predecessors:
            predecessors.insert(0, self._last_event_id)
        for predecessor in predecessors:
            _token("causal_predecessor", predecessor)
        if deadline_ms is not None:
            _non_negative_integer("deadline_ms", deadline_ms)
        if deadline_expired and deadline_ms is None:
            raise EvidenceError("expired transition has no deadline")
        event_id = self._event_id()
        self._write(
            {
                "schema_version": SCHEMA_VERSION,
                "kind": SCHEMA_KIND,
                "record_type": "transition",
                "event_id": event_id,
                "run_id": self.run_id,
                "monotonic_ns": max(0, time.monotonic_ns() - self._origin),
                "emitter": self.emitter,
                "process_role": process_role,
                "subject": subject,
                "machine": machine,
                "transition": transition,
                "causal_predecessors": predecessors,
                "identities": safe_identities,
                "target_kind": target_kind,
                "trigger": trigger,
                "authority_before": authority_before,
                "authority_after": authority_after,
                "expected_effect": expected_effect,
                "observed_effect": observed_effect,
                "disposition": disposition,
                "deadline_ms": deadline_ms,
                "deadline_expired": deadline_expired,
            }
        )
        self._last_event_id = event_id
        self._sequence += 1
        return event_id

    def close(self) -> None:
        self._output.flush()
        self._output.close()


def _mapping(values: Sequence[str], *, roles: bool = False) -> dict[str, Any]:
    parsed: dict[str, Any] = {}
    for value in values:
        emitter, separator, raw = value.partition("=")
        if not separator:
            raise EvidenceError("mapping arguments must use emitter=value")
        emitter = _token("emitter", emitter)
        parsed[emitter] = frozenset(raw.split(",")) if roles else raw
    return parsed


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--input", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--require-inventory", action="append", default=[])
    parser.add_argument("--require-role", action="append", default=[])
    parser.add_argument("--expected-digest", action="append", default=[])
    parser.add_argument("--minimum-cross-process-edges", type=int, default=0)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        summary = validate_and_merge(
            args.input,
            model_path=args.model,
            output_path=args.output,
            summary_path=args.summary,
            required_inventories=_mapping(args.require_inventory, roles=True),
            required_roles=frozenset(args.require_role),
            expected_digests=_mapping(args.expected_digest),
            minimum_cross_process_edges=args.minimum_cross_process_edges,
        )
    except (EvidenceError, OSError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(json.dumps({"kind": VALIDATION_KIND, "result": "failed", "message": str(error)}))
        return 1
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
