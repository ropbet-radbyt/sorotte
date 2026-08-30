#!/usr/bin/env python3
"""Execute and verify independent Sorotte playback-lifecycle schedules."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import random
import re
import sys
from collections import deque
from dataclasses import dataclass, field
from typing import Any, Callable, Iterable, Mapping, Sequence

import playback_lifecycle_model as lifecycle_model


SAFE_TOKEN = re.compile(r"[a-z0-9][a-z0-9.-]{0,127}")
IDENTITY_KEYS = {
    "process_run",
    "connection_generation",
    "membership_epoch",
    "attachment_epoch",
    "media_generation",
    "playlist_revision",
    "playlist_index_revision",
    "load_attempt",
    "command_sequence",
    "frame_receipt",
    "report_sequence",
}
PROCESS_ROLES = {"oracle", "client", "server", "player", "proxy", "harness"}
TARGET_KINDS = {
    "none",
    "generated-fixture",
    "local-file",
    "network-stream",
    "plex",
    "unknown",
}
TRIGGERS = {
    "model-witness",
    "startup",
    "shutdown",
    "user-intent",
    "player-observation",
    "server-snapshot",
    "canonical-mutation",
    "natural-completion",
    "playlist-authority",
    "media-resolution",
    "readiness",
    "network-loss",
    "retry",
    "recovery",
    "timer",
    "status",
    "harness-fault",
}
DISPOSITIONS = {
    "applied",
    "ignored-stale",
    "duplicate-noop",
    "rejected",
    "failed",
    "superseded",
}

BASE_IDENTITY_REQUIREMENTS = {
    "application": {"process_run"},
    "player-attachment": {"attachment_epoch"},
    "session": {"connection_generation"},
    "room-membership": {"connection_generation", "membership_epoch"},
    "playlist-selection": {"playlist_revision"},
    "media-resolution": {"playlist_revision", "media_generation"},
    "load-attempt": {"attachment_epoch", "media_generation", "load_attempt"},
    "local-transport": {"attachment_epoch", "media_generation", "load_attempt"},
    "canonical-transaction": {"command_sequence"},
    "start-gate": {"membership_epoch"},
    "participant-status": {"membership_epoch"},
}

TRANSITION_IDENTITY_REQUIREMENTS = {
    "PLAYLIST-SNAPSHOT-SELECTED-001": {"playlist_index_revision"},
    "PLAYLIST-SELECT-001": {"playlist_index_revision"},
    "PLAYLIST-EXHAUST-001": {"playlist_index_revision"},
    "TX-WRITTEN-001": {"frame_receipt"},
    "TX-COMMIT-001": {"frame_receipt"},
    "TX-FANOUT-001": {"frame_receipt"},
    "TX-CONVERGE-001": {"frame_receipt"},
    "STATUS-FRESH-001": {"report_sequence"},
    "STATUS-DELAY-001": {"report_sequence"},
    "STATUS-STALE-001": {"report_sequence"},
    "STATUS-WITHDRAW-001": {"report_sequence"},
}

ADVANCING_IDENTITIES = {
    "APP-LAUNCH-001": "process_run",
    "PLAYER-LAUNCH-001": "attachment_epoch",
    "PLAYER-RELAUNCH-001": "attachment_epoch",
    "SESSION-CONNECT-001": "connection_generation",
    "ROOM-JOIN-001": "membership_epoch",
    "ROOM-SWITCH-001": "membership_epoch",
    "PLAYLIST-MUTATE-001": "playlist_revision",
    "PLAYLIST-SELECT-001": "playlist_index_revision",
    "MEDIA-SELECT-001": "media_generation",
    "LOAD-SUBMIT-001": "load_attempt",
    "LOAD-RECOVERY-SUBMIT-001": "load_attempt",
    "TX-BEGIN-001": "command_sequence",
    "TX-WRITTEN-001": "frame_receipt",
    "STATUS-FRESH-001": "report_sequence",
}

ROLE_BY_AUTHORITY = {
    "application-process": {"client", "server", "oracle", "harness"},
    "client-intent": {"client", "oracle", "harness"},
    "client-media-resolver": {"client", "oracle", "harness"},
    "client-or-server-transaction": {"client", "server", "oracle", "harness"},
    "client-session": {"client", "oracle", "harness"},
    "client-transaction": {"client", "oracle", "harness"},
    "player-lifecycle": {"client", "player", "oracle", "harness"},
    "player-observation": {"player", "client", "oracle", "harness"},
    "player-owner": {"client", "player", "oracle", "harness"},
    "protocol-transport": {"client", "server", "proxy", "oracle", "harness"},
    "server-playlist": {"server", "oracle", "harness"},
    "server-readiness": {"server", "oracle", "harness"},
    "server-room": {"server", "oracle", "harness"},
    "server-session": {"server", "oracle", "harness"},
    "server-status": {"server", "oracle", "harness"},
    "verification-oracle": {"oracle", "harness"},
}

EVENT_KEYS = {
    "schema_version",
    "event_id",
    "run_id",
    "monotonic_ns",
    "emitter",
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


class OracleError(ValueError):
    """Raised when a schedule or observed ledger violates the lifecycle contract."""


@dataclass(frozen=True)
class MachineSpec:
    id: str
    mode: str
    initial_state: str
    states: frozenset[str]
    state_kinds: Mapping[str, str]
    terminal_states: frozenset[str]
    outgoing: Mapping[str, frozenset[str]]


@dataclass(frozen=True)
class TransitionSpec:
    id: str
    machine: str
    sources: frozenset[str]
    destination: str
    authority: str
    risk: str
    invariants: frozenset[str]


@dataclass(frozen=True)
class LifecycleSpec:
    model_id: str
    machines: Mapping[str, MachineSpec]
    transitions: Mapping[str, TransitionSpec]
    invariants: frozenset[str]


def require_exact_keys(
    value: Mapping[str, Any],
    *,
    required: set[str],
    allowed: set[str],
    context: str,
) -> None:
    missing = required - set(value)
    unexpected = set(value) - allowed
    if missing:
        raise OracleError(f"{context} is missing keys {sorted(missing)}")
    if unexpected:
        raise OracleError(f"{context} has unexpected keys {sorted(unexpected)}")


def require_token(value: Any, context: str) -> str:
    if not isinstance(value, str) or not SAFE_TOKEN.fullmatch(value):
        raise OracleError(f"{context} must be a privacy-safe lowercase token")
    return value


def require_bool(value: Any, context: str) -> bool:
    if not isinstance(value, bool):
        raise OracleError(f"{context} must be a boolean")
    return value


def require_positive_integer(value: Any, context: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise OracleError(f"{context} must be a positive integer")
    return value


def require_token_list(value: Any, context: str, *, allow_empty: bool = False) -> list[str]:
    if not isinstance(value, list) or (not value and not allow_empty):
        qualifier = "a token array" if allow_empty else "a non-empty token array"
        raise OracleError(f"{context} must be {qualifier}")
    result = [require_token(item, f"{context}[{index}]") for index, item in enumerate(value)]
    if len(result) != len(set(result)):
        raise OracleError(f"{context} must not contain duplicates")
    return result


def require_identities(value: Any, context: str) -> dict[str, int]:
    if not isinstance(value, dict):
        raise OracleError(f"{context} must be a table")
    unexpected = set(value) - IDENTITY_KEYS
    if unexpected:
        raise OracleError(f"{context} contains unknown identities {sorted(unexpected)}")
    return {
        key: require_positive_integer(item, f"{context}.{key}")
        for key, item in value.items()
    }


def load_spec(model_path: pathlib.Path, *, repo_root: pathlib.Path) -> LifecycleSpec:
    raw = lifecycle_model.load_toml(model_path.resolve(), "lifecycle model")
    summary = lifecycle_model.validate_model(raw, repo_root=repo_root.resolve())
    machines: dict[str, MachineSpec] = {}
    transitions: dict[str, TransitionSpec] = {}
    for machine in raw["machine"]:
        machine_id = machine["id"]
        machine_transitions = [
            transition
            for transition in machine["transition"]
        ]
        outgoing: dict[str, set[str]] = {
            state["id"]: set() for state in machine["state"]
        }
        for transition in machine_transitions:
            for source in transition["from"]:
                outgoing[source].add(transition["to"])
        machines[machine_id] = MachineSpec(
            id=machine_id,
            mode=machine["mode"],
            initial_state=machine["initial_state"],
            states=frozenset(state["id"] for state in machine["state"]),
            state_kinds={state["id"]: state["kind"] for state in machine["state"]},
            terminal_states=frozenset(machine["terminal_states"]),
            outgoing={state: frozenset(destinations) for state, destinations in outgoing.items()},
        )
        for transition in machine_transitions:
            transition_id = transition["id"]
            transitions[transition_id] = TransitionSpec(
                id=transition_id,
                machine=machine_id,
                sources=frozenset(transition["from"]),
                destination=transition["to"],
                authority=transition["authority"],
                risk=transition["risk"],
                invariants=frozenset(transition["invariants"]),
            )
    if len(transitions) != summary["transition_count"]:
        raise OracleError("validated transition inventory changed while loading the oracle")
    return LifecycleSpec(
        model_id=summary["model_id"],
        machines=machines,
        transitions=transitions,
        invariants=frozenset(invariant["id"] for invariant in raw["invariant"]),
    )


def required_identities(transition: TransitionSpec) -> set[str]:
    return BASE_IDENTITY_REQUIREMENTS[transition.machine] | TRANSITION_IDENTITY_REQUIREMENTS.get(
        transition.id, set()
    )


def identity_scope_key(
    subject: str,
    identity: str,
    identities: Mapping[str, int],
) -> tuple[Any, ...]:
    if identity == "load_attempt":
        return (
            subject,
            identity,
            identities.get("attachment_epoch"),
            identities.get("media_generation"),
        )
    if identity in {"command_sequence", "frame_receipt"}:
        return (subject, identity, identities.get("connection_generation"))
    if identity == "report_sequence":
        return (subject, identity, identities.get("membership_epoch"))
    if identity == "playlist_index_revision":
        return (subject, identity, identities.get("playlist_revision"))
    return (subject, identity)


class CausalOracle:
    def __init__(self, spec: LifecycleSpec) -> None:
        self.spec = spec
        self.states: dict[tuple[str, str], str] = {}
        self.events: dict[str, dict[str, Any]] = {}
        self.last_monotonic_ns: dict[str, int] = {}
        self.maximum_identities: dict[tuple[Any, ...], int] = {}
        self.first_failure_event: str | None = None
        self.checked_invariants: set[str] = set()
        self.invariant_evaluation_count = 0
        self.semantic_results: set[tuple[Any, ...]] = set()

    def current_state(self, subject: str, machine: str) -> str:
        key = (subject, machine)
        if key not in self.states:
            self.states[key] = self.spec.machines[machine].initial_state
        return self.states[key]

    def _can_reach(
        self,
        machine_id: str,
        start: str,
        predicate: Callable[[str], bool],
    ) -> bool:
        machine = self.spec.machines[machine_id]
        pending = deque([start])
        visited: set[str] = set()
        while pending:
            state = pending.popleft()
            if state in visited:
                continue
            visited.add(state)
            if predicate(state):
                return True
            pending.extend(machine.outgoing[state] - visited)
        return False

    @staticmethod
    def _semantic_once_keys(
        event: Mapping[str, Any],
        transition: TransitionSpec,
    ) -> tuple[tuple[Any, ...], ...]:
        identities = event["identities"]
        subject = event["subject"]
        keys: list[tuple[Any, ...]] = []
        if transition.id == "TX-COMMIT-001":
            keys.append(("canonical-commit", subject, identities["command_sequence"]))
        if transition.id in {
            "TX-CONVERGE-001",
            "TX-REJECT-001",
            "TX-FAIL-001",
            "TX-SUPERSEDE-001",
        }:
            keys.append(("transaction-result", subject, identities["command_sequence"]))
        if (
            transition.id == "PLAYLIST-SELECT-001"
            and event["trigger"] == "natural-completion"
        ):
            keys.append(
                (
                    "natural-progression",
                    subject,
                    identities["playlist_revision"],
                    identities["playlist_index_revision"],
                    identities["media_generation"],
                    identities["load_attempt"],
                )
            )
        return tuple(keys)

    def _validate_assigned_invariants(
        self,
        event: Mapping[str, Any],
        transition: TransitionSpec,
        *,
        current: str,
        stale_keys: set[str],
        allowed_roles: set[str],
    ) -> tuple[tuple[Any, ...], ...]:
        """Evaluate every invariant named by this transition before committing it."""

        disposition = event["disposition"]
        applied = disposition == "applied"
        machine = self.spec.machines[transition.machine]
        target_state = transition.destination if applied else current
        once_keys = self._semantic_once_keys(event, transition) if applied else ()

        for invariant in sorted(transition.invariants):
            if invariant == "LIFE-AUTH-001":
                if event["process_role"] not in allowed_roles:
                    raise OracleError(
                        f"{event['event_id']} lacks {transition.authority} authority"
                    )
                if applied and event["observed_effect"] != transition.destination:
                    raise OracleError(
                        f"{event['event_id']} canonical effect was not observed atomically"
                    )
            elif invariant == "LIFE-EPOCH-001":
                if applied and stale_keys:
                    raise OracleError(
                        f"{event['event_id']} violated epoch fencing for {sorted(stale_keys)}"
                    )
            elif invariant == "LIFE-IDENT-001":
                identities = event["identities"]
                for identity in required_identities(transition):
                    scope = identity_scope_key(event["subject"], identity, identities)
                    if scope[:2] != (event["subject"], identity):
                        raise OracleError(
                            f"{event['event_id']} conflated identity domain {identity}"
                        )
            elif invariant == "LIFE-DELIVERY-001":
                if (
                    event["trigger"] == "canonical-mutation"
                    and transition.id.startswith("TRANSPORT-")
                    and transition.id
                    in {
                        "TRANSPORT-PLAY-001",
                        "TRANSPORT-PAUSE-001",
                        "TRANSPORT-SEEK-001",
                        "TRANSPORT-LOAD-001",
                    }
                    and not self._has_matching_ancestor(
                        event,
                        {"TX-WRITTEN-001"},
                        {"command_sequence", "frame_receipt"},
                    )
                ):
                    raise OracleError(
                        f"{event['event_id']} violated exact-frame delivery ordering"
                    )
            elif invariant == "LIFE-ONCE-001":
                duplicate_keys = set(once_keys) & self.semantic_results
                if duplicate_keys:
                    raise OracleError(
                        f"{event['event_id']} duplicated semantic result {sorted(duplicate_keys)!r}"
                    )
            elif invariant == "LIFE-EOF-001":
                if event["trigger"] == "natural-completion" and not self._has_matching_ancestor(
                    event,
                    {"TRANSPORT-END-001"},
                    {"media_generation", "load_attempt"},
                ):
                    raise OracleError(
                        f"{event['event_id']} natural completion was not correlated"
                    )
            elif invariant == "LIFE-STATUS-001":
                if set(event) != EVENT_KEYS:
                    raise OracleError(
                        f"{event['event_id']} status projection escaped the safe schema"
                    )
            elif invariant == "LIFE-SNAPSHOT-001":
                if applied and (
                    event["authority_after"] != transition.destination
                    or event["observed_effect"] != transition.destination
                ):
                    raise OracleError(
                        f"{event['event_id']} installed a torn authoritative snapshot"
                    )
            elif invariant == "LIFE-EXIT-001":
                if machine.state_kinds[target_state] == "transient" and not self._can_reach(
                    transition.machine,
                    target_state,
                    lambda state: machine.state_kinds[state] != "transient",
                ):
                    raise OracleError(
                        f"{event['event_id']} entered a transient state with no bounded exit"
                    )
            elif invariant in {
                "LIFE-CONVERGE-001",
                "LIFE-REJOIN-001",
                "LIFE-RECOVERY-001",
            }:
                if not self._can_reach(
                    transition.machine,
                    target_state,
                    lambda state: machine.state_kinds[state] in {"stable", "terminal"},
                ):
                    raise OracleError(
                        f"{event['event_id']} has no declared convergence or recovery outcome"
                    )
            elif invariant == "LIFE-SHUTDOWN-001":
                destinations = (
                    machine.terminal_states
                    if machine.mode == "finite"
                    else frozenset({machine.initial_state})
                )
                if not self._can_reach(
                    transition.machine,
                    target_state,
                    lambda state: state in destinations,
                ):
                    raise OracleError(
                        f"{event['event_id']} has no declared ownership-release path"
                    )
            elif invariant == "LIFE-TRACE-001":
                if any(predecessor not in self.events for predecessor in event["causal_predecessors"]):
                    raise OracleError(
                        f"{event['event_id']} trace has an unresolved causal edge"
                    )
            elif invariant == "LIFE-FAILURE-001":
                if (
                    self.first_failure_event is not None
                    and self.first_failure_event not in self.events
                ):
                    raise OracleError("first lifecycle failure was not preserved")
            else:
                raise OracleError(
                    f"{event['event_id']} references unmapped invariant {invariant}"
                )
        return once_keys

    def _causal_ancestors(self, predecessor_ids: Iterable[str]) -> set[str]:
        ancestors: set[str] = set()
        pending = list(predecessor_ids)
        while pending:
            event_id = pending.pop()
            if event_id in ancestors:
                continue
            ancestors.add(event_id)
            pending.extend(self.events[event_id]["causal_predecessors"])
        return ancestors

    def _has_matching_ancestor(
        self,
        event: Mapping[str, Any],
        transition_ids: set[str],
        identity_keys: set[str],
    ) -> bool:
        identities = event["identities"]
        if not identity_keys <= set(identities):
            return False
        for ancestor_id in self._causal_ancestors(event["causal_predecessors"]):
            ancestor = self.events[ancestor_id]
            if ancestor["transition"] not in transition_ids:
                continue
            if all(ancestor["identities"].get(key) == identities[key] for key in identity_keys):
                return True
        return False

    def _validate_causal_rules(
        self,
        event: Mapping[str, Any],
        transition: TransitionSpec,
    ) -> None:
        if event["trigger"] == "natural-completion":
            if transition.id != "PLAYLIST-SELECT-001":
                raise OracleError(
                    f"{event['event_id']} natural-completion may only request playlist selection"
                )
            if not self._has_matching_ancestor(
                event,
                {"TRANSPORT-END-001"},
                {"media_generation", "load_attempt"},
            ):
                raise OracleError(
                    f"{event['event_id']} natural completion lacks a correlated transport end"
                )

        if event["trigger"] == "canonical-mutation" and transition.id in {
            "TRANSPORT-PLAY-001",
            "TRANSPORT-PAUSE-001",
            "TRANSPORT-SEEK-001",
            "TRANSPORT-LOAD-001",
        }:
            if not self._has_matching_ancestor(
                event,
                {"TX-WRITTEN-001"},
                {"command_sequence", "frame_receipt"},
            ):
                raise OracleError(
                    f"{event['event_id']} dependent player effect precedes its exact frame receipt"
                )

    def apply_event(self, raw_event: Mapping[str, Any], *, context: str = "event") -> None:
        if not isinstance(raw_event, dict):
            raise OracleError(f"{context} must be an object")
        require_exact_keys(
            raw_event,
            required=EVENT_KEYS,
            allowed=EVENT_KEYS,
            context=context,
        )
        if raw_event["schema_version"] != 1:
            raise OracleError(f"{context}.schema_version must be 1")
        event_id = require_token(raw_event["event_id"], f"{context}.event_id")
        if event_id in self.events:
            raise OracleError(f"duplicate event id {event_id}")
        require_token(raw_event["run_id"], f"{context}.run_id")
        monotonic_ns = require_positive_integer(
            raw_event["monotonic_ns"], f"{context}.monotonic_ns"
        )
        emitter = require_token(raw_event["emitter"], f"{context}.emitter")
        role = require_token(raw_event["process_role"], f"{context}.process_role")
        if role not in PROCESS_ROLES:
            raise OracleError(f"{context}.process_role is not allowed: {role}")
        subject = require_token(raw_event["subject"], f"{context}.subject")
        machine_id = require_token(raw_event["machine"], f"{context}.machine")
        if machine_id not in self.spec.machines:
            raise OracleError(f"{context}.machine is unknown: {machine_id}")
        transition_id = lifecycle_model.require_string(
            raw_event["transition"], f"{context}.transition"
        )
        transition = self.spec.transitions.get(transition_id)
        if transition is None:
            raise OracleError(f"{context}.transition is unknown: {transition_id}")
        if transition.machine != machine_id:
            raise OracleError(
                f"{event_id} transition {transition_id} belongs to {transition.machine}, not {machine_id}"
            )
        allowed_roles = ROLE_BY_AUTHORITY.get(transition.authority)
        if allowed_roles is None:
            raise OracleError(f"unmapped lifecycle authority {transition.authority}")
        if role not in allowed_roles:
            raise OracleError(
                f"{event_id} role {role} cannot claim {transition.authority} authority"
            )
        predecessors = require_token_list(
            raw_event["causal_predecessors"],
            f"{context}.causal_predecessors",
            allow_empty=True,
        )
        unknown_predecessors = set(predecessors) - set(self.events)
        if unknown_predecessors:
            raise OracleError(
                f"{event_id} references unseen causal predecessors {sorted(unknown_predecessors)}"
            )
        identities = require_identities(raw_event["identities"], f"{context}.identities")
        missing_identities = required_identities(transition) - set(identities)
        if missing_identities:
            raise OracleError(
                f"{event_id} is missing required identities {sorted(missing_identities)}"
            )
        target_kind = require_token(raw_event["target_kind"], f"{context}.target_kind")
        if target_kind not in TARGET_KINDS:
            raise OracleError(f"{context}.target_kind is not allowed: {target_kind}")
        trigger = require_token(raw_event["trigger"], f"{context}.trigger")
        if trigger not in TRIGGERS:
            raise OracleError(f"{context}.trigger is not allowed: {trigger}")
        before = require_token(raw_event["authority_before"], f"{context}.authority_before")
        after = require_token(raw_event["authority_after"], f"{context}.authority_after")
        expected = require_token(raw_event["expected_effect"], f"{context}.expected_effect")
        observed = require_token(raw_event["observed_effect"], f"{context}.observed_effect")
        disposition = require_token(raw_event["disposition"], f"{context}.disposition")
        if disposition not in DISPOSITIONS:
            raise OracleError(f"{context}.disposition is not allowed: {disposition}")
        require_positive_integer(raw_event["deadline_ms"], f"{context}.deadline_ms")
        deadline_expired = require_bool(
            raw_event["deadline_expired"], f"{context}.deadline_expired"
        )

        last_ns = self.last_monotonic_ns.get(emitter)
        if last_ns is not None and monotonic_ns <= last_ns:
            raise OracleError(f"{event_id} monotonic time did not advance for emitter {emitter}")

        current = self.current_state(subject, machine_id)
        if before != current:
            raise OracleError(
                f"{event_id} authority_before is {before}, expected current state {current}"
            )
        if expected != transition.destination:
            raise OracleError(
                f"{event_id} expected_effect is {expected}, contract requires {transition.destination}"
            )

        stale_keys = {
            key
            for key, value in identities.items()
            if (
                maximum := self.maximum_identities.get(
                    identity_scope_key(subject, key, identities)
                )
            )
            is not None
            and value < maximum
        }
        advancing_key = ADVANCING_IDENTITIES.get(transition_id)

        if disposition == "applied":
            if stale_keys:
                raise OracleError(
                    f"{event_id} applied stale identities {sorted(stale_keys)}"
                )
            if current not in transition.sources:
                raise OracleError(
                    f"{event_id} cannot apply {transition_id} from {current}"
                )
            if advancing_key is not None:
                previous = self.maximum_identities.get(
                    identity_scope_key(subject, advancing_key, identities)
                )
                if previous is not None and identities[advancing_key] <= previous:
                    raise OracleError(
                        f"{event_id} must advance {advancing_key} beyond {previous}"
                    )
            if after != transition.destination or observed != transition.destination:
                raise OracleError(
                    f"{event_id} applied transition must observe {transition.destination}"
                )
        else:
            if after != current or observed != current:
                raise OracleError(f"{event_id} non-applied transition changed authority")
            if disposition == "ignored-stale" and not stale_keys:
                raise OracleError(f"{event_id} claims ignored-stale without stale identity")
            if disposition != "ignored-stale" and stale_keys:
                raise OracleError(
                    f"{event_id} stale identities require ignored-stale disposition"
                )
            if deadline_expired and disposition != "failed":
                raise OracleError(
                    f"{event_id} expired deadline must have failed disposition"
                )

        if deadline_expired and disposition == "applied":
            raise OracleError(f"{event_id} cannot apply after its deadline expired")

        event = dict(raw_event)
        event["causal_predecessors"] = predecessors
        event["identities"] = identities
        self._validate_causal_rules(event, transition)
        once_keys = self._validate_assigned_invariants(
            event,
            transition,
            current=current,
            stale_keys=stale_keys,
            allowed_roles=allowed_roles,
        )

        self.events[event_id] = event
        self.last_monotonic_ns[emitter] = monotonic_ns
        if disposition == "applied":
            self.states[(subject, machine_id)] = transition.destination
            for key, value in identities.items():
                identity_key = identity_scope_key(subject, key, identities)
                self.maximum_identities[identity_key] = max(
                    value,
                    self.maximum_identities.get(identity_key, value),
                )
        elif disposition == "failed" and self.first_failure_event is None:
            self.first_failure_event = event_id
        self.semantic_results.update(once_keys)
        self.checked_invariants.update(transition.invariants)
        self.invariant_evaluation_count += len(transition.invariants)

    def summary(self) -> dict[str, Any]:
        covered = sorted(
            {
                event["transition"]
                for event in self.events.values()
                if event["disposition"] == "applied"
            }
        )
        return {
            "schema_version": 1,
            "model_id": self.spec.model_id,
            "status": "failed" if self.first_failure_event is not None else "passed",
            "first_failure_event": self.first_failure_event,
            "event_count": len(self.events),
            "covered_transition_count": len(covered),
            "covered_transitions": covered,
            "checked_invariant_count": len(self.checked_invariants),
            "checked_invariants": sorted(self.checked_invariants),
            "invariant_evaluation_count": self.invariant_evaluation_count,
            "final_states": [
                {"subject": subject, "machine": machine, "state": state}
                for (subject, machine), state in sorted(self.states.items())
            ],
        }


def step_to_event(
    step: Mapping[str, Any],
    *,
    index: int,
    run_id: str,
    oracle: CausalOracle,
) -> dict[str, Any]:
    required = {"id", "transition", "subject", "causes", "identities"}
    allowed = required | {
        "emitter",
        "process_role",
        "target_kind",
        "trigger",
        "disposition",
        "deadline_ms",
        "deadline_expired",
        "observed_state",
    }
    if not isinstance(step, dict):
        raise OracleError(f"step[{index}] must be a table")
    require_exact_keys(step, required=required, allowed=allowed, context=f"step[{index}]")
    event_id = require_token(step["id"], f"step[{index}].id")
    transition_id = lifecycle_model.require_string(
        step["transition"], f"step[{index}].transition"
    )
    transition = oracle.spec.transitions.get(transition_id)
    if transition is None:
        raise OracleError(f"step[{index}].transition is unknown: {transition_id}")
    subject = require_token(step["subject"], f"step[{index}].subject")
    before = oracle.current_state(subject, transition.machine)
    disposition = step.get("disposition", "applied")
    expected = transition.destination
    observed = step.get(
        "observed_state",
        expected if disposition == "applied" else before,
    )
    return {
        "schema_version": 1,
        "event_id": event_id,
        "run_id": run_id,
        "monotonic_ns": (index + 1) * 1_000_000,
        "emitter": step.get("emitter", subject),
        "process_role": step.get("process_role", "oracle"),
        "subject": subject,
        "machine": transition.machine,
        "transition": transition_id,
        "causal_predecessors": step["causes"],
        "identities": step["identities"],
        "target_kind": step.get("target_kind", "none"),
        "trigger": step.get("trigger", "model-witness"),
        "authority_before": before,
        "authority_after": observed,
        "expected_effect": expected,
        "observed_effect": observed,
        "disposition": disposition,
        "deadline_ms": step.get("deadline_ms", 5_000),
        "deadline_expired": step.get("deadline_expired", False),
    }


def load_schedule(path: pathlib.Path) -> dict[str, Any]:
    raw = lifecycle_model.load_toml(path.resolve(), "lifecycle schedule")
    require_exact_keys(
        raw,
        required={"schema_version", "schedule_id", "title", "model", "step", "expect"},
        allowed={"schema_version", "schedule_id", "title", "model", "step", "expect"},
        context="schedule",
    )
    if raw["schema_version"] != 1:
        raise OracleError("schedule.schema_version must be 1")
    require_token(raw["schedule_id"], "schedule.schedule_id")
    lifecycle_model.require_string(raw["title"], "schedule.title")
    lifecycle_model.require_string(raw["model"], "schedule.model")
    if not isinstance(raw["step"], list) or not raw["step"]:
        raise OracleError("schedule.step must be a non-empty array of tables")
    if not isinstance(raw["expect"], list) or not raw["expect"]:
        raise OracleError("schedule.expect must be a non-empty array of tables")
    return raw


def execute_schedule(
    schedule: Mapping[str, Any],
    *,
    repo_root: pathlib.Path,
) -> tuple[CausalOracle, list[dict[str, Any]]]:
    model_path = lifecycle_model.safe_repo_path(
        repo_root.resolve(), schedule["model"], "schedule.model"
    )
    spec = load_spec(model_path, repo_root=repo_root)
    oracle = CausalOracle(spec)
    run_id = require_token(schedule["schedule_id"], "schedule.schedule_id")
    events: list[dict[str, Any]] = []
    for index, step in enumerate(schedule["step"]):
        event = step_to_event(step, index=index, run_id=run_id, oracle=oracle)
        oracle.apply_event(event, context=f"step[{index}]")
        events.append(event)

    seen_expectations: set[tuple[str, str]] = set()
    for index, expectation in enumerate(schedule["expect"]):
        context = f"expect[{index}]"
        if not isinstance(expectation, dict):
            raise OracleError(f"{context} must be a table")
        require_exact_keys(
            expectation,
            required={"subject", "machine", "state"},
            allowed={"subject", "machine", "state"},
            context=context,
        )
        subject = require_token(expectation["subject"], f"{context}.subject")
        machine = require_token(expectation["machine"], f"{context}.machine")
        if machine not in spec.machines:
            raise OracleError(f"{context}.machine is unknown: {machine}")
        state = require_token(expectation["state"], f"{context}.state")
        if state not in spec.machines[machine].states:
            raise OracleError(f"{context}.state is unknown for {machine}: {state}")
        key = (subject, machine)
        if key in seen_expectations:
            raise OracleError(f"duplicate final-state expectation for {subject}/{machine}")
        seen_expectations.add(key)
        actual = oracle.current_state(subject, machine)
        if actual != state:
            raise OracleError(
                f"{context} expected {subject}/{machine}={state}, observed {actual}"
            )
    return oracle, events


def execute_schedule_suite(
    schedule_dir: pathlib.Path,
    *,
    repo_root: pathlib.Path,
) -> dict[str, Any]:
    try:
        schedule_paths = sorted(schedule_dir.resolve().glob("*.toml"))
    except OSError as error:
        raise OracleError(f"failed to enumerate schedule suite {schedule_dir}: {error}") from error
    if not schedule_paths:
        raise OracleError(f"schedule suite is empty: {schedule_dir}")
    covered: set[str] = set()
    schedules: list[dict[str, Any]] = []
    event_count = 0
    for schedule_path in schedule_paths:
        schedule = load_schedule(schedule_path)
        lifecycle_oracle, events = execute_schedule(schedule, repo_root=repo_root)
        summary = lifecycle_oracle.summary()
        if summary["status"] != "passed":
            raise OracleError(
                f"schedule {schedule['schedule_id']} retained failure "
                f"{summary['first_failure_event']}"
            )
        covered.update(summary["covered_transitions"])
        event_count += len(events)
        schedules.append(
            {
                "schedule_id": schedule["schedule_id"],
                "event_count": len(events),
                "covered_transition_count": summary["covered_transition_count"],
            }
        )
    return {
        "schema_version": 1,
        "status": "passed",
        "schedule_count": len(schedules),
        "event_count": event_count,
        "covered_transition_count": len(covered),
        "covered_transitions": sorted(covered),
        "schedules": schedules,
    }


def strict_json_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise OracleError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def load_ledger(path: pathlib.Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise OracleError(f"failed to read lifecycle ledger {path}: {error}") from error
    if not lines:
        raise OracleError("lifecycle ledger must not be empty")
    for index, line in enumerate(lines, start=1):
        if not line.strip():
            raise OracleError(f"lifecycle ledger line {index} is blank")
        try:
            value = json.loads(
                line,
                object_pairs_hook=strict_json_object,
                parse_constant=lambda constant: (_ for _ in ()).throw(
                    OracleError(f"nonstandard JSON number {constant}")
                ),
            )
        except (json.JSONDecodeError, OracleError) as error:
            raise OracleError(f"invalid lifecycle ledger line {index}: {error}") from error
        if not isinstance(value, dict):
            raise OracleError(f"lifecycle ledger line {index} must be an object")
        events.append(value)
    return events


def verify_ledger(events: Sequence[Mapping[str, Any]], spec: LifecycleSpec) -> CausalOracle:
    oracle = CausalOracle(spec)
    run_id: str | None = None
    for index, event in enumerate(events):
        current_run = event.get("run_id") if isinstance(event, dict) else None
        if run_id is None and isinstance(current_run, str):
            run_id = current_run
        elif current_run != run_id:
            raise OracleError(f"ledger event[{index}] belongs to a different run")
        oracle.apply_event(event, context=f"ledger event[{index}]")
    return oracle


def write_ledger(path: pathlib.Path, events: Sequence[Mapping[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = "".join(
        json.dumps(event, sort_keys=True, separators=(",", ":")) + "\n" for event in events
    )
    path.write_text(payload, encoding="utf-8", newline="\n")


def shortest_transition_witnesses(spec: LifecycleSpec) -> dict[str, list[str]]:
    by_machine: dict[str, list[TransitionSpec]] = {machine: [] for machine in spec.machines}
    for transition in spec.transitions.values():
        by_machine[transition.machine].append(transition)
    result: dict[str, list[str]] = {}
    for machine_id, machine in spec.machines.items():
        outgoing: dict[str, list[TransitionSpec]] = {state: [] for state in machine.states}
        for transition in sorted(by_machine[machine_id], key=lambda item: item.id):
            for source in transition.sources:
                outgoing[source].append(transition)
        paths: dict[str, list[str]] = {machine.initial_state: []}
        pending = deque([machine.initial_state])
        while pending:
            state = pending.popleft()
            for transition in outgoing[state]:
                if transition.destination in paths:
                    continue
                paths[transition.destination] = paths[state] + [transition.id]
                pending.append(transition.destination)
        for transition in by_machine[machine_id]:
            for source in sorted(transition.sources):
                if source not in paths:
                    raise OracleError(f"no executable witness for {transition.id} from {source}")
                result[f"{transition.id}@{source}"] = paths[source] + [transition.id]
    witnessed_transitions = {key.split("@", maxsplit=1)[0] for key in result}
    if witnessed_transitions != set(spec.transitions):
        raise OracleError("transition witnesses do not cover the complete model")
    return dict(sorted(result.items()))


def execute_transition_witnesses(
    spec: LifecycleSpec,
) -> dict[str, Any]:
    witnesses = shortest_transition_witnesses(spec)
    observed_sources: set[str] = set()
    maximum_steps = 0
    for witness_index, (witness_id, path) in enumerate(witnesses.items(), start=1):
        oracle = CausalOracle(spec)
        subject = f"witness-{witness_index}"
        identity_values = {key: 1 for key in IDENTITY_KEYS}
        previous_event: str | None = None
        final_source: str | None = None
        for step_index, transition_id in enumerate(path, start=1):
            transition = spec.transitions[transition_id]
            identities = {
                key: identity_values[key] for key in required_identities(transition)
            }
            advancing_key = ADVANCING_IDENTITIES.get(transition_id)
            if advancing_key is not None:
                previous_identity = oracle.maximum_identities.get(
                    identity_scope_key(subject, advancing_key, identities)
                )
                if previous_identity is not None:
                    identities[advancing_key] = previous_identity + 1
                    identity_values[advancing_key] = previous_identity + 1
            current = oracle.current_state(subject, transition.machine)
            event_id = f"w{witness_index}-s{step_index}"
            event = {
                "schema_version": 1,
                "event_id": event_id,
                "run_id": f"witness-{witness_index}",
                "monotonic_ns": step_index * 1_000_000,
                "emitter": subject,
                "process_role": "oracle",
                "subject": subject,
                "machine": transition.machine,
                "transition": transition_id,
                "causal_predecessors": (
                    [] if previous_event is None else [previous_event]
                ),
                "identities": identities,
                "target_kind": "none",
                "trigger": "model-witness",
                "authority_before": current,
                "authority_after": transition.destination,
                "expected_effect": transition.destination,
                "observed_effect": transition.destination,
                "disposition": "applied",
                "deadline_ms": 5_000,
                "deadline_expired": False,
            }
            oracle.apply_event(event, context=f"{witness_id}[{step_index - 1}]")
            previous_event = event_id
            final_source = current
        expected_source = witness_id.split("@", maxsplit=1)[1]
        if final_source != expected_source:
            raise OracleError(
                f"witness {witness_id} reached {final_source}, expected {expected_source}"
            )
        observed_sources.add(witness_id)
        maximum_steps = max(maximum_steps, len(path))
    if observed_sources != set(witnesses):
        raise OracleError("executed witnesses lost transition-source coverage")
    return {
        "schema_version": 1,
        "model_id": spec.model_id,
        "transition_count": len(spec.transitions),
        "transition_source_count": len(witnesses),
        "witness_count": len(witnesses),
        "maximum_witness_steps": maximum_steps,
        "witnesses": witnesses,
    }


def shrink_sequence(
    values: Sequence[Any],
    still_fails: Callable[[list[Any]], bool],
) -> list[Any]:
    """Deterministically delta-debug a failing sequence while preserving order."""

    current = list(values)
    if not current or not still_fails(current):
        raise OracleError("shrink input must reproduce the target failure")
    granularity = 2
    while len(current) >= 2:
        chunk_size = (len(current) + granularity - 1) // granularity
        reduced = False
        for start in range(0, len(current), chunk_size):
            candidate = current[:start] + current[start + chunk_size :]
            if candidate and still_fails(candidate):
                current = candidate
                granularity = max(2, granularity - 1)
                reduced = True
                break
        if reduced:
            continue
        if granularity >= len(current):
            break
        granularity = min(len(current), granularity * 2)
    return current


@dataclass
class IdentityAllocator:
    """Allocate monotonic values without conflating independent identity domains."""

    values: dict[tuple[str, str], int] = field(default_factory=dict)

    def for_transition(
        self,
        subject: str,
        transition: TransitionSpec,
        *,
        extra: Mapping[str, int] | None = None,
    ) -> dict[str, int]:
        identities: dict[str, int] = {}
        advancing = ADVANCING_IDENTITIES.get(transition.id)
        for identity in sorted(required_identities(transition)):
            if identity == advancing:
                continue
            key = (subject, identity)
            value = max(1, self.values.get(key, 0))
            identities[identity] = value
            self.values[key] = value

        if advancing is not None:
            key = (subject, advancing)
            value = self.values.get(key, 0) + 1
            identities[advancing] = value
            self.values[key] = value

        if extra is not None:
            for identity, value in extra.items():
                if identity not in IDENTITY_KEYS:
                    raise OracleError(f"explorer received unknown identity {identity}")
                require_positive_integer(value, f"explorer identity {identity}")
                identities[identity] = value
                key = (subject, identity)
                self.values[key] = max(value, self.values.get(key, value))
        return identities


@dataclass
class ExplorerEventBuilder:
    spec: LifecycleSpec
    oracle: CausalOracle
    run_id: str
    emitter: str
    event_prefix: str
    allocator: IdentityAllocator = field(default_factory=IdentityAllocator)
    events: list[dict[str, Any]] = field(default_factory=list)

    def make_event(
        self,
        transition_id: str,
        *,
        subject: str,
        process_role: str = "oracle",
        trigger: str = "model-witness",
        target_kind: str = "none",
        causes: Sequence[str] | None = None,
        extra_identities: Mapping[str, int] | None = None,
    ) -> dict[str, Any]:
        transition = self.spec.transitions[transition_id]
        before = self.oracle.current_state(subject, transition.machine)
        sequence = len(self.events) + 1
        predecessors = (
            list(causes)
            if causes is not None
            else ([] if not self.events else [self.events[-1]["event_id"]])
        )
        return {
            "schema_version": 1,
            "event_id": f"{self.event_prefix}-e{sequence}",
            "run_id": self.run_id,
            "monotonic_ns": sequence * 1_000_000,
            "emitter": self.emitter,
            "process_role": process_role,
            "subject": subject,
            "machine": transition.machine,
            "transition": transition_id,
            "causal_predecessors": predecessors,
            "identities": self.allocator.for_transition(
                subject,
                transition,
                extra=extra_identities,
            ),
            "target_kind": target_kind,
            "trigger": trigger,
            "authority_before": before,
            "authority_after": transition.destination,
            "expected_effect": transition.destination,
            "observed_effect": transition.destination,
            "disposition": "applied",
            "deadline_ms": 5_000,
            "deadline_expired": False,
        }

    def apply_event(self, event: Mapping[str, Any], *, context: str) -> dict[str, Any]:
        materialized = copy.deepcopy(dict(event))
        self.events.append(materialized)
        self.oracle.apply_event(materialized, context=context)
        return materialized

    def emit(self, transition_id: str, *, subject: str) -> dict[str, Any]:
        return self.apply_event(
            self.make_event(transition_id, subject=subject),
            context=f"{self.event_prefix}[{len(self.events)}]",
        )


def transition_source_coverage_ledger(
    spec: LifecycleSpec,
    *,
    run_id: str,
) -> tuple[list[dict[str, Any]], set[str]]:
    witnesses = shortest_transition_witnesses(spec)
    oracle = CausalOracle(spec)
    all_events: list[dict[str, Any]] = []
    observed_sources: set[str] = set()
    for witness_index, (witness_id, path) in enumerate(witnesses.items(), start=1):
        builder = ExplorerEventBuilder(
            spec=spec,
            oracle=oracle,
            run_id=run_id,
            emitter=f"witness-{witness_index}",
            event_prefix=f"w{witness_index}",
        )
        subject = f"witness-{witness_index}"
        final_source: str | None = None
        for transition_id in path:
            transition = spec.transitions[transition_id]
            final_source = oracle.current_state(subject, transition.machine)
            builder.emit(transition_id, subject=subject)
        expected_source = witness_id.split("@", maxsplit=1)[1]
        if final_source != expected_source:
            raise OracleError(
                f"coverage witness {witness_id} reached {final_source}, expected {expected_source}"
            )
        observed_sources.add(witness_id)
        all_events.extend(builder.events)
    if observed_sources != set(witnesses):
        raise OracleError("state-aware explorer lost transition-source coverage")
    return all_events, observed_sources


def exploration_subjects(case_index: int) -> dict[str, tuple[str, ...]]:
    client_a = f"case-{case_index}-client-a"
    client_b = f"case-{case_index}-client-b"
    room = f"case-{case_index}-room"
    return {
        "application": (client_a, client_b, f"case-{case_index}-server"),
        "player-attachment": (client_a, client_b),
        "session": (client_a, client_b),
        "room-membership": (client_a, client_b),
        "playlist-selection": (room,),
        "media-resolution": (client_a, client_b),
        "load-attempt": (client_a, client_b),
        "local-transport": (client_a, client_b),
        "canonical-transaction": (
            f"case-{case_index}-transaction-a",
            f"case-{case_index}-transaction-b",
        ),
        "start-gate": (room,),
        "participant-status": (client_a, client_b),
    }


def legal_exploration_choices(
    spec: LifecycleSpec,
    oracle: CausalOracle,
    subjects: Mapping[str, Sequence[str]],
) -> list[tuple[str, str, str]]:
    choices: list[tuple[str, str, str]] = []
    for machine_id, machine_subjects in sorted(subjects.items()):
        for subject in machine_subjects:
            current = oracle.current_state(subject, machine_id)
            for transition in sorted(spec.transitions.values(), key=lambda item: item.id):
                if transition.machine == machine_id and current in transition.sources:
                    choices.append((subject, machine_id, transition.id))
    return choices


def _base_probe_event(
    spec: LifecycleSpec,
    oracle: CausalOracle,
    transition_id: str,
    *,
    event_id: str,
    subject: str,
    identities: Mapping[str, int],
    monotonic_ns: int = 1_000_000,
    role: str = "oracle",
    trigger: str = "model-witness",
    causes: Sequence[str] = (),
) -> dict[str, Any]:
    transition = spec.transitions[transition_id]
    before = oracle.current_state(subject, transition.machine)
    return {
        "schema_version": 1,
        "event_id": event_id,
        "run_id": "invalid-probe",
        "monotonic_ns": monotonic_ns,
        "emitter": "probe-oracle",
        "process_role": role,
        "subject": subject,
        "machine": transition.machine,
        "transition": transition_id,
        "causal_predecessors": list(causes),
        "identities": dict(identities),
        "target_kind": "none",
        "trigger": trigger,
        "authority_before": before,
        "authority_after": transition.destination,
        "expected_effect": transition.destination,
        "observed_effect": transition.destination,
        "disposition": "applied",
        "deadline_ms": 5_000,
        "deadline_expired": False,
    }


def run_invalid_history_probes(spec: LifecycleSpec) -> list[str]:
    """Prove representative invalid histories fail closed at the oracle boundary."""

    rejected: list[str] = []

    def expect_rejection(
        name: str,
        lifecycle_oracle: CausalOracle,
        event: Mapping[str, Any],
        expected: str,
    ) -> None:
        try:
            lifecycle_oracle.apply_event(event, context=f"invalid probe {name}")
        except OracleError as error:
            if expected not in str(error):
                raise OracleError(
                    f"invalid probe {name} failed at the wrong boundary: {error}"
                ) from error
            rejected.append(name)
            return
        raise OracleError(f"invalid probe {name} was accepted")

    lifecycle_oracle = CausalOracle(spec)
    authority = _base_probe_event(
        spec,
        lifecycle_oracle,
        "PLAYLIST-SNAPSHOT-EMPTY-001",
        event_id="authority-role",
        subject="probe-room",
        identities={"playlist_revision": 1},
        role="client",
    )
    expect_rejection(
        "authority-role",
        lifecycle_oracle,
        authority,
        "cannot claim server-playlist authority",
    )

    lifecycle_oracle = CausalOracle(spec)
    missing_identity = _base_probe_event(
        spec,
        lifecycle_oracle,
        "APP-LAUNCH-001",
        event_id="missing-identity",
        subject="probe-client",
        identities={},
    )
    expect_rejection(
        "missing-identity",
        lifecycle_oracle,
        missing_identity,
        "missing required identities",
    )

    lifecycle_oracle = CausalOracle(spec)
    unknown_cause = _base_probe_event(
        spec,
        lifecycle_oracle,
        "APP-LAUNCH-001",
        event_id="unknown-cause",
        subject="probe-client",
        identities={"process_run": 1},
        causes=("unseen-event",),
    )
    expect_rejection(
        "unknown-cause",
        lifecycle_oracle,
        unknown_cause,
        "unseen causal predecessors",
    )

    lifecycle_oracle = CausalOracle(spec)
    expired = _base_probe_event(
        spec,
        lifecycle_oracle,
        "APP-LAUNCH-001",
        event_id="expired-applied",
        subject="probe-client",
        identities={"process_run": 1},
    )
    expired["deadline_expired"] = True
    expect_rejection(
        "expired-applied",
        lifecycle_oracle,
        expired,
        "cannot apply after its deadline expired",
    )

    lifecycle_oracle = CausalOracle(spec)
    raw_field = _base_probe_event(
        spec,
        lifecycle_oracle,
        "APP-LAUNCH-001",
        event_id="privacy-schema",
        subject="probe-client",
        identities={"process_run": 1},
    )
    raw_field["raw_path"] = "forbidden"
    expect_rejection(
        "privacy-schema",
        lifecycle_oracle,
        raw_field,
        "unexpected keys",
    )

    lifecycle_oracle = CausalOracle(spec)
    duplicate = _base_probe_event(
        spec,
        lifecycle_oracle,
        "APP-LAUNCH-001",
        event_id="duplicate-event",
        subject="probe-client",
        identities={"process_run": 1},
    )
    lifecycle_oracle.apply_event(duplicate, context="invalid probe duplicate setup")
    expect_rejection(
        "duplicate-event",
        lifecycle_oracle,
        duplicate,
        "duplicate event id",
    )

    lifecycle_oracle = CausalOracle(spec)
    selected_one = _base_probe_event(
        spec,
        lifecycle_oracle,
        "MEDIA-SELECT-001",
        event_id="stale-select-one",
        subject="probe-client",
        identities={"playlist_revision": 1, "media_generation": 1},
    )
    lifecycle_oracle.apply_event(selected_one)
    clear = _base_probe_event(
        spec,
        lifecycle_oracle,
        "MEDIA-CLEAR-001",
        event_id="stale-clear",
        subject="probe-client",
        identities={"playlist_revision": 1, "media_generation": 1},
        monotonic_ns=2_000_000,
        causes=("stale-select-one",),
    )
    lifecycle_oracle.apply_event(clear)
    selected_two = _base_probe_event(
        spec,
        lifecycle_oracle,
        "MEDIA-SELECT-001",
        event_id="stale-select-two",
        subject="probe-client",
        identities={"playlist_revision": 1, "media_generation": 2},
        monotonic_ns=3_000_000,
        causes=("stale-clear",),
    )
    lifecycle_oracle.apply_event(selected_two)
    stale = _base_probe_event(
        spec,
        lifecycle_oracle,
        "MEDIA-RESOLVE-001",
        event_id="stale-apply",
        subject="probe-client",
        identities={"playlist_revision": 1, "media_generation": 1},
        monotonic_ns=4_000_000,
        causes=("stale-select-two",),
    )
    expect_rejection(
        "stale-apply",
        lifecycle_oracle,
        stale,
        "applied stale identities",
    )

    lifecycle_oracle = CausalOracle(spec)
    snapshot = _base_probe_event(
        spec,
        lifecycle_oracle,
        "PLAYLIST-SNAPSHOT-POPULATED-001",
        event_id="natural-snapshot",
        subject="probe-room",
        identities={"playlist_revision": 1},
        role="server",
    )
    lifecycle_oracle.apply_event(snapshot)
    natural = _base_probe_event(
        spec,
        lifecycle_oracle,
        "PLAYLIST-SELECT-001",
        event_id="uncorrelated-natural",
        subject="probe-room",
        identities={
            "playlist_revision": 1,
            "playlist_index_revision": 1,
            "media_generation": 1,
            "load_attempt": 1,
        },
        monotonic_ns=2_000_000,
        role="client",
        trigger="natural-completion",
        causes=("natural-snapshot",),
    )
    expect_rejection(
        "uncorrelated-natural",
        lifecycle_oracle,
        natural,
        "lacks a correlated transport end",
    )

    lifecycle_oracle = CausalOracle(spec)
    transport_ids = {
        "attachment_epoch": 1,
        "media_generation": 1,
        "load_attempt": 1,
    }
    loaded = _base_probe_event(
        spec,
        lifecycle_oracle,
        "TRANSPORT-LOAD-001",
        event_id="delivery-load",
        subject="probe-client",
        identities=transport_ids,
        role="player",
        trigger="player-observation",
    )
    lifecycle_oracle.apply_event(loaded)
    playing = _base_probe_event(
        spec,
        lifecycle_oracle,
        "TRANSPORT-PLAY-001",
        event_id="delivery-play",
        subject="probe-client",
        identities=transport_ids,
        monotonic_ns=2_000_000,
        role="player",
        trigger="player-observation",
        causes=("delivery-load",),
    )
    lifecycle_oracle.apply_event(playing)
    premature = _base_probe_event(
        spec,
        lifecycle_oracle,
        "TRANSPORT-PAUSE-001",
        event_id="premature-dependent-effect",
        subject="probe-client",
        identities={**transport_ids, "command_sequence": 1, "frame_receipt": 1},
        monotonic_ns=3_000_000,
        role="client",
        trigger="canonical-mutation",
        causes=("delivery-play",),
    )
    expect_rejection(
        "premature-dependent-effect",
        lifecycle_oracle,
        premature,
        "precedes its exact frame receipt",
    )

    return sorted(rejected)


FAILURE_SIGNATURES = (
    ("authority-role", " cannot claim "),
    ("schema", "unexpected keys"),
    ("identity", "identit"),
    ("causal-predecessor", "causal predecessor"),
    ("delivery", "frame receipt"),
    ("natural-completion", "natural completion"),
    ("deadline", "deadline"),
    ("state", "cannot apply"),
    ("clock", "monotonic time"),
    ("duplicate", "duplicate event id"),
)


def failure_signature(error: OracleError) -> str:
    message = f" {error} ".lower()
    for signature, marker in FAILURE_SIGNATURES:
        if marker in message:
            return signature
    return "oracle-contract"


def normalize_failure_ledger(
    values: Sequence[Mapping[str, Any]],
    spec: LifecycleSpec,
) -> list[dict[str, Any]]:
    """Repair removable sequencing metadata without repairing the actual fault."""

    normalized: list[dict[str, Any]] = []
    states: dict[tuple[str, str], str] = {}
    previous_event: str | None = None
    emitter_clocks: dict[str, int] = {}
    for raw in values:
        event = copy.deepcopy(dict(raw))
        machine_id = event.get("machine")
        transition_id = event.get("transition")
        subject = event.get("subject")
        if (
            isinstance(machine_id, str)
            and machine_id in spec.machines
            and isinstance(transition_id, str)
            and transition_id in spec.transitions
            and isinstance(subject, str)
        ):
            current = states.get(
                (subject, machine_id),
                spec.machines[machine_id].initial_state,
            )
            event["authority_before"] = current
            destination = spec.transitions[transition_id].destination
            event["expected_effect"] = destination
            if event.get("disposition") == "applied":
                event["authority_after"] = destination
                event["observed_effect"] = destination
                states[(subject, machine_id)] = destination
            else:
                event["authority_after"] = current
                event["observed_effect"] = current
        event["causal_predecessors"] = (
            [] if previous_event is None else [previous_event]
        )
        emitter = event.get("emitter")
        if isinstance(emitter, str):
            next_clock = emitter_clocks.get(emitter, 0) + 1_000_000
            event["monotonic_ns"] = next_clock
            emitter_clocks[emitter] = next_clock
        event_id = event.get("event_id")
        previous_event = event_id if isinstance(event_id, str) else None
        normalized.append(event)
    return normalized


def shrink_failing_ledger(
    events: Sequence[Mapping[str, Any]],
    spec: LifecycleSpec,
    signature: str,
) -> list[dict[str, Any]]:
    def still_fails(candidate: list[Mapping[str, Any]]) -> bool:
        normalized = normalize_failure_ledger(candidate, spec)
        try:
            verify_ledger(normalized, spec)
        except OracleError as error:
            return failure_signature(error) == signature
        return False

    minimized = shrink_sequence(list(events), still_fails)
    return normalize_failure_ledger(minimized, spec)


def persist_minimized_failure(
    events: Sequence[Mapping[str, Any]],
    *,
    spec: LifecycleSpec,
    seed: int,
    case_index: int,
    signature: str,
    failure_dir: pathlib.Path,
) -> tuple[pathlib.Path, pathlib.Path, int]:
    minimized = shrink_failing_ledger(events, spec, signature)
    failure_dir.mkdir(parents=True, exist_ok=True)
    stem = f"seed-{seed}-case-{case_index}-{signature}"
    ledger_path = failure_dir / f"{stem}.jsonl"
    metadata_path = failure_dir / f"{stem}.json"
    if ledger_path.exists() or metadata_path.exists():
        raise OracleError(
            f"refusing to overwrite existing minimized failure artifact {stem}"
        )
    write_ledger(ledger_path, minimized)
    metadata = {
        "schema_version": 1,
        "result": "failed",
        "model_id": spec.model_id,
        "seed": seed,
        "case_index": case_index,
        "failure_signature": signature,
        "original_event_count": len(events),
        "minimized_event_count": len(minimized),
        "ledger": ledger_path.name,
        "replay": (
            "python scripts/playback_lifecycle_oracle.py verify-ledger "
            f"--ledger <failure-dir>/{ledger_path.name}"
        ),
    }
    metadata_path.write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    return ledger_path, metadata_path, len(minimized)


def event_stream_digest(events: Sequence[Mapping[str, Any]]) -> str:
    digest = hashlib.sha256()
    for event in events:
        digest.update(
            json.dumps(event, sort_keys=True, separators=(",", ":")).encode("utf-8")
        )
        digest.update(b"\n")
    return digest.hexdigest()


ExplorerMutator = Callable[[int, int, dict[str, Any]], dict[str, Any]]


def explore_lifecycle(
    spec: LifecycleSpec,
    *,
    seed: int,
    case_count: int,
    steps_per_case: int,
    failure_dir: pathlib.Path,
    ledger_path: pathlib.Path | None = None,
    event_mutator: ExplorerMutator | None = None,
) -> dict[str, Any]:
    if seed < 0:
        raise OracleError("exploration seed must be non-negative")
    if case_count <= 0 or case_count > 100_000:
        raise OracleError("exploration case count must be between 1 and 100000")
    if steps_per_case <= 0 or steps_per_case > 100_000:
        raise OracleError("exploration step count must be between 1 and 100000")

    run_id = f"explore-{seed}"
    coverage_events, observed_sources = transition_source_coverage_ledger(
        spec,
        run_id=run_id,
    )
    all_events = list(coverage_events)
    rng = random.Random(seed)
    case_digests: list[dict[str, Any]] = []

    for case_index in range(1, case_count + 1):
        lifecycle_oracle = CausalOracle(spec)
        builder = ExplorerEventBuilder(
            spec=spec,
            oracle=lifecycle_oracle,
            run_id=run_id,
            emitter=f"case-{case_index}-oracle",
            event_prefix=f"r{case_index}",
        )
        subjects = exploration_subjects(case_index)
        for step_index in range(1, steps_per_case + 1):
            choices = legal_exploration_choices(spec, lifecycle_oracle, subjects)
            if not choices:
                raise OracleError(f"exploration case {case_index} has no legal transition")
            subject, _machine, transition_id = choices[rng.randrange(len(choices))]
            event = builder.make_event(transition_id, subject=subject)
            if event_mutator is not None:
                event = event_mutator(case_index, step_index, copy.deepcopy(event))
            try:
                builder.apply_event(
                    event,
                    context=f"exploration case {case_index} step {step_index}",
                )
            except OracleError as error:
                signature = failure_signature(error)
                ledger, metadata, minimized_count = persist_minimized_failure(
                    builder.events,
                    spec=spec,
                    seed=seed,
                    case_index=case_index,
                    signature=signature,
                    failure_dir=failure_dir,
                )
                raise OracleError(
                    f"exploration case {case_index} step {step_index} failed "
                    f"({signature}); minimized {len(builder.events)} events to "
                    f"{minimized_count} in {ledger.name} with {metadata.name}"
                ) from error
        all_events.extend(builder.events)
        case_digests.append(
            {
                "case_index": case_index,
                "event_count": len(builder.events),
                "sha256": event_stream_digest(builder.events),
            }
        )

    invalid_probes = run_invalid_history_probes(spec)
    combined = verify_ledger(all_events, spec)
    summary = combined.summary()
    if combined.first_failure_event is not None:
        raise OracleError(
            f"exploration retained unexpected failure {combined.first_failure_event}"
        )
    if set(summary["covered_transitions"]) != set(spec.transitions):
        raise OracleError("exploration did not cover every lifecycle transition")
    if combined.checked_invariants != set(spec.invariants):
        missing = sorted(set(spec.invariants) - combined.checked_invariants)
        raise OracleError(f"exploration did not evaluate invariants {missing}")

    if ledger_path is not None:
        write_ledger(ledger_path, all_events)
    return {
        "schema_version": 1,
        "status": "passed",
        "model_id": spec.model_id,
        "seed": seed,
        "case_count": case_count,
        "steps_per_case": steps_per_case,
        "coverage_event_count": len(coverage_events),
        "random_walk_event_count": case_count * steps_per_case,
        "event_count": len(all_events),
        "transition_count": len(spec.transitions),
        "transition_source_count": len(observed_sources),
        "checked_invariant_count": len(combined.checked_invariants),
        "checked_invariants": sorted(combined.checked_invariants),
        "invariant_evaluation_count": combined.invariant_evaluation_count,
        "invalid_history_probe_count": len(invalid_probes),
        "invalid_history_probes": invalid_probes,
        "event_stream_sha256": event_stream_digest(all_events),
        "cases": case_digests,
    }


def build_parser(repo_root: pathlib.Path) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    schedule = subparsers.add_parser("run-schedule", help="execute a TOML replay schedule")
    schedule.add_argument("--schedule", type=pathlib.Path, required=True)
    schedule.add_argument("--repo-root", type=pathlib.Path, default=repo_root)
    schedule.add_argument("--ledger", type=pathlib.Path)

    suite = subparsers.add_parser(
        "run-suite", help="execute every committed TOML replay schedule"
    )
    suite.add_argument(
        "--schedule-dir",
        type=pathlib.Path,
        default=repo_root / "fixtures" / "playback-lifecycle",
    )
    suite.add_argument("--repo-root", type=pathlib.Path, default=repo_root)

    ledger = subparsers.add_parser("verify-ledger", help="verify an observed JSONL ledger")
    ledger.add_argument("--ledger", type=pathlib.Path, required=True)
    ledger.add_argument(
        "--model",
        type=pathlib.Path,
        default=repo_root / "coverage" / "playback-lifecycle.toml",
    )
    ledger.add_argument("--repo-root", type=pathlib.Path, default=repo_root)

    witnesses = subparsers.add_parser(
        "witness-summary", help="prove every declared transition has an executable model path"
    )
    witnesses.add_argument(
        "--model",
        type=pathlib.Path,
        default=repo_root / "coverage" / "playback-lifecycle.toml",
    )
    witnesses.add_argument("--repo-root", type=pathlib.Path, default=repo_root)
    witnesses.add_argument(
        "--compact",
        action="store_true",
        help="omit individual witness paths from the JSON summary",
    )

    explore = subparsers.add_parser(
        "explore",
        help="run deterministic state-aware composed lifecycle exploration",
    )
    explore.add_argument(
        "--model",
        type=pathlib.Path,
        default=repo_root / "coverage" / "playback-lifecycle.toml",
    )
    explore.add_argument("--repo-root", type=pathlib.Path, default=repo_root)
    explore.add_argument(
        "--seed",
        type=lambda value: int(value, 0),
        default=0x50A077E20260831,
    )
    explore.add_argument("--cases", type=int, default=32)
    explore.add_argument("--steps", type=int, default=128)
    explore.add_argument(
        "--failure-dir",
        type=pathlib.Path,
        default=repo_root / "target" / "verification" / "playback-lifecycle-model-failures",
    )
    explore.add_argument(
        "--ledger",
        type=pathlib.Path,
        help="optionally persist the complete passing privacy-safe event ledger",
    )
    explore.add_argument(
        "--compact",
        action="store_true",
        help="omit per-case event-stream digests from the JSON summary",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    repo_root = pathlib.Path(__file__).resolve().parent.parent
    args = build_parser(repo_root).parse_args(argv)
    try:
        if args.command == "run-schedule":
            schedule = load_schedule(args.schedule)
            oracle, events = execute_schedule(schedule, repo_root=args.repo_root.resolve())
            if args.ledger is not None:
                write_ledger(args.ledger, events)
            summary = oracle.summary()
        elif args.command == "run-suite":
            summary = execute_schedule_suite(
                args.schedule_dir,
                repo_root=args.repo_root.resolve(),
            )
        elif args.command == "verify-ledger":
            spec = load_spec(args.model, repo_root=args.repo_root.resolve())
            oracle = verify_ledger(load_ledger(args.ledger), spec)
            summary = oracle.summary()
        elif args.command == "witness-summary":
            spec = load_spec(args.model, repo_root=args.repo_root.resolve())
            summary = execute_transition_witnesses(spec)
            if args.compact:
                summary = {
                    key: value for key, value in summary.items() if key != "witnesses"
                }
        else:
            spec = load_spec(args.model, repo_root=args.repo_root.resolve())
            summary = explore_lifecycle(
                spec,
                seed=args.seed,
                case_count=args.cases,
                steps_per_case=args.steps,
                failure_dir=args.failure_dir,
                ledger_path=args.ledger,
            )
            if args.compact:
                summary = {key: value for key, value in summary.items() if key != "cases"}
    except (OracleError, lifecycle_model.ModelError) as error:
        print(f"playback lifecycle oracle failed: {error}", file=sys.stderr)
        return 2

    print(json.dumps(summary, indent=2, sort_keys=True))
    return 3 if summary.get("status") == "failed" else 0


if __name__ == "__main__":
    raise SystemExit(main())
