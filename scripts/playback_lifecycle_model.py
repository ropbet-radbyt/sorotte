#!/usr/bin/env python3
"""Validate Sorotte's machine-readable whole-playback lifecycle contract."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
import tomllib
from collections import defaultdict
from typing import Any, Iterable, Mapping


SLUG = re.compile(r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*")
TRANSITION_ID = re.compile(r"[A-Z][A-Z0-9]*(?:-[A-Z0-9]+)+")
INVARIANT_ID = re.compile(r"LIFE-[A-Z0-9]+-[0-9]{3}")
GAP_ID = re.compile(r"GAP-[A-Z0-9]+-[0-9]{3}")


class ModelError(ValueError):
    """Raised when the lifecycle model is incomplete or internally inconsistent."""


def load_toml(path: pathlib.Path, label: str) -> dict[str, Any]:
    try:
        with path.open("rb") as handle:
            value = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ModelError(f"failed to load {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise ModelError(f"{label} must be a TOML table")
    return value


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
        raise ModelError(f"{context} is missing keys {sorted(missing)}")
    if unexpected:
        raise ModelError(f"{context} has unexpected keys {sorted(unexpected)}")


def require_string(value: Any, context: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ModelError(f"{context} must be a non-empty string")
    return value


def require_string_list(value: Any, context: str, *, allow_empty: bool = False) -> list[str]:
    if not isinstance(value, list) or (not value and not allow_empty):
        qualifier = "a string array" if allow_empty else "a non-empty string array"
        raise ModelError(f"{context} must be {qualifier}")
    result: list[str] = []
    for index, item in enumerate(value):
        result.append(require_string(item, f"{context}[{index}]"))
    if len(result) != len(set(result)):
        raise ModelError(f"{context} must not contain duplicates")
    return result


def require_table_list(value: Any, context: str) -> list[dict[str, Any]]:
    if not isinstance(value, list) or not value:
        raise ModelError(f"{context} must be a non-empty array of tables")
    result: list[dict[str, Any]] = []
    for index, item in enumerate(value):
        if not isinstance(item, dict):
            raise ModelError(f"{context}[{index}] must be a table")
        result.append(item)
    return result


def safe_repo_path(repo_root: pathlib.Path, value: Any, context: str) -> pathlib.Path:
    relative = pathlib.PurePosixPath(require_string(value, context))
    if relative.is_absolute() or ".." in relative.parts:
        raise ModelError(f"{context} must stay within the repository")
    candidate = (repo_root / pathlib.Path(*relative.parts)).resolve()
    try:
        candidate.relative_to(repo_root)
    except ValueError as error:
        raise ModelError(f"{context} escapes the repository") from error
    if not candidate.is_file():
        raise ModelError(f"{context} does not identify a file: {relative}")
    return candidate


def reachable_states(initial: str, adjacency: Mapping[str, set[str]]) -> set[str]:
    visited: set[str] = set()
    pending = [initial]
    while pending:
        state = pending.pop()
        if state in visited:
            continue
        visited.add(state)
        pending.extend(sorted(adjacency.get(state, set()) - visited))
    return visited


def states_reaching(targets: Iterable[str], reverse: Mapping[str, set[str]]) -> set[str]:
    visited: set[str] = set()
    pending = list(targets)
    while pending:
        state = pending.pop()
        if state in visited:
            continue
        visited.add(state)
        pending.extend(sorted(reverse.get(state, set()) - visited))
    return visited


def behavior_ids(catalog: Mapping[str, Any]) -> set[str]:
    entries = catalog.get("behavior")
    if not isinstance(entries, list) or not entries:
        raise ModelError("behavior catalog must contain behavior tables")
    result: set[str] = set()
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            raise ModelError(f"behavior catalog behavior[{index}] must be a table")
        behavior_id = require_string(entry.get("id"), f"behavior catalog behavior[{index}].id")
        if behavior_id in result:
            raise ModelError(f"behavior catalog contains duplicate behavior {behavior_id}")
        result.add(behavior_id)
    return result


def validate_model(
    model: Mapping[str, Any],
    *,
    repo_root: pathlib.Path,
    require_closed: bool = False,
) -> dict[str, Any]:
    require_exact_keys(
        model,
        required={
            "schema_version",
            "model_id",
            "title",
            "contract",
            "behavior_catalog",
            "policy",
            "invariant",
            "gap",
            "machine",
        },
        allowed={
            "schema_version",
            "model_id",
            "title",
            "contract",
            "behavior_catalog",
            "policy",
            "invariant",
            "gap",
            "machine",
        },
        context="lifecycle model",
    )
    if model["schema_version"] != 1:
        raise ModelError("lifecycle model schema_version must be 1")
    model_id = require_string(model["model_id"], "model_id")
    if not SLUG.fullmatch(model_id):
        raise ModelError("model_id must be a lowercase hyphenated identifier")
    require_string(model["title"], "title")
    safe_repo_path(repo_root, model["contract"], "contract")
    catalog_path = safe_repo_path(repo_root, model["behavior_catalog"], "behavior_catalog")
    catalog_behaviors = behavior_ids(load_toml(catalog_path, "behavior catalog"))

    policy = model["policy"]
    if not isinstance(policy, dict):
        raise ModelError("policy must be a table")
    require_exact_keys(
        policy,
        required={
            "allowed_risks",
            "required_tiers",
            "allowed_state_kinds",
            "allowed_machine_modes",
            "allowed_gap_statuses",
            "allowed_invariant_kinds",
        },
        allowed={
            "allowed_risks",
            "required_tiers",
            "allowed_state_kinds",
            "allowed_machine_modes",
            "allowed_gap_statuses",
            "allowed_invariant_kinds",
        },
        context="policy",
    )
    allowed_risks = set(require_string_list(policy["allowed_risks"], "policy.allowed_risks"))
    required_tiers = set(require_string_list(policy["required_tiers"], "policy.required_tiers"))
    state_kinds = set(
        require_string_list(policy["allowed_state_kinds"], "policy.allowed_state_kinds")
    )
    machine_modes = set(
        require_string_list(policy["allowed_machine_modes"], "policy.allowed_machine_modes")
    )
    gap_statuses = set(
        require_string_list(policy["allowed_gap_statuses"], "policy.allowed_gap_statuses")
    )
    invariant_kinds = set(
        require_string_list(
            policy["allowed_invariant_kinds"], "policy.allowed_invariant_kinds"
        )
    )
    if required_tiers != {"model", "seam", "system"}:
        raise ModelError("policy.required_tiers must be exactly model, seam, and system")

    invariants: dict[str, dict[str, Any]] = {}
    for index, invariant in enumerate(require_table_list(model["invariant"], "invariant")):
        context = f"invariant[{index}]"
        require_exact_keys(
            invariant,
            required={"id", "kind", "statement"},
            allowed={"id", "kind", "statement"},
            context=context,
        )
        invariant_id = require_string(invariant["id"], f"{context}.id")
        if not INVARIANT_ID.fullmatch(invariant_id):
            raise ModelError(f"{context}.id has invalid format {invariant_id!r}")
        if invariant_id in invariants:
            raise ModelError(f"duplicate invariant id {invariant_id}")
        kind = require_string(invariant["kind"], f"{context}.kind")
        if kind not in invariant_kinds:
            raise ModelError(f"{context}.kind is not allowed: {kind}")
        require_string(invariant["statement"], f"{context}.statement")
        invariants[invariant_id] = invariant

    gaps: dict[str, dict[str, Any]] = {}
    open_gaps: set[str] = set()
    for index, gap in enumerate(require_table_list(model["gap"], "gap")):
        context = f"gap[{index}]"
        require_exact_keys(
            gap,
            required={"id", "title", "status", "risk", "owners", "summary", "closure"},
            allowed={"id", "title", "status", "risk", "owners", "summary", "closure"},
            context=context,
        )
        gap_id = require_string(gap["id"], f"{context}.id")
        if not GAP_ID.fullmatch(gap_id):
            raise ModelError(f"{context}.id has invalid format {gap_id!r}")
        if gap_id in gaps:
            raise ModelError(f"duplicate gap id {gap_id}")
        status = require_string(gap["status"], f"{context}.status")
        if status not in gap_statuses:
            raise ModelError(f"{context}.status is not allowed: {status}")
        risk = require_string(gap["risk"], f"{context}.risk")
        if risk not in allowed_risks:
            raise ModelError(f"{context}.risk is not allowed: {risk}")
        require_string(gap["title"], f"{context}.title")
        require_string_list(gap["owners"], f"{context}.owners")
        require_string(gap["summary"], f"{context}.summary")
        require_string(gap["closure"], f"{context}.closure")
        gaps[gap_id] = gap
        if status == "open":
            open_gaps.add(gap_id)

    if require_closed and open_gaps:
        raise ModelError(f"open lifecycle gaps remain: {sorted(open_gaps)}")

    machines = require_table_list(model["machine"], "machine")
    machine_ids: set[str] = set()
    transition_ids: set[str] = set()
    referenced_invariants: set[str] = set()
    referenced_gaps: set[str] = set()
    referenced_behaviors: set[str] = set()
    state_count = 0
    transition_count = 0
    transitions_missing_tiers: dict[str, list[str]] = {}

    for machine_index, machine in enumerate(machines):
        context = f"machine[{machine_index}]"
        require_exact_keys(
            machine,
            required={
                "id",
                "title",
                "mode",
                "initial_state",
                "terminal_states",
                "state",
                "transition",
            },
            allowed={
                "id",
                "title",
                "mode",
                "initial_state",
                "terminal_states",
                "state",
                "transition",
            },
            context=context,
        )
        machine_id = require_string(machine["id"], f"{context}.id")
        if not SLUG.fullmatch(machine_id):
            raise ModelError(f"{context}.id must be a lowercase hyphenated identifier")
        if machine_id in machine_ids:
            raise ModelError(f"duplicate machine id {machine_id}")
        machine_ids.add(machine_id)
        require_string(machine["title"], f"{context}.title")
        mode = require_string(machine["mode"], f"{context}.mode")
        if mode not in machine_modes:
            raise ModelError(f"{context}.mode is not allowed: {mode}")

        states: dict[str, dict[str, Any]] = {}
        for state_index, state in enumerate(require_table_list(machine["state"], f"{context}.state")):
            state_context = f"{context}.state[{state_index}]"
            require_exact_keys(
                state,
                required={"id", "title", "kind", "description"},
                allowed={"id", "title", "kind", "description"},
                context=state_context,
            )
            state_id = require_string(state["id"], f"{state_context}.id")
            if not SLUG.fullmatch(state_id):
                raise ModelError(f"{state_context}.id must be a lowercase hyphenated identifier")
            if state_id in states:
                raise ModelError(f"{machine_id} contains duplicate state {state_id}")
            require_string(state["title"], f"{state_context}.title")
            kind = require_string(state["kind"], f"{state_context}.kind")
            if kind not in state_kinds:
                raise ModelError(f"{state_context}.kind is not allowed: {kind}")
            require_string(state["description"], f"{state_context}.description")
            states[state_id] = state
        state_count += len(states)

        initial = require_string(machine["initial_state"], f"{context}.initial_state")
        if initial not in states:
            raise ModelError(f"{machine_id} initial state {initial!r} does not exist")
        terminals = require_string_list(
            machine["terminal_states"], f"{context}.terminal_states", allow_empty=True
        )
        unknown_terminals = set(terminals) - set(states)
        if unknown_terminals:
            raise ModelError(f"{machine_id} has unknown terminal states {sorted(unknown_terminals)}")
        if mode == "finite" and not terminals:
            raise ModelError(f"finite machine {machine_id} needs at least one terminal state")
        if mode == "cyclic" and terminals:
            raise ModelError(f"cyclic machine {machine_id} cannot declare terminal states")
        for terminal in terminals:
            if states[terminal]["kind"] != "terminal":
                raise ModelError(f"{machine_id} terminal state {terminal} must have kind terminal")
        undeclared_terminal_kinds = {
            state_id
            for state_id, state in states.items()
            if state["kind"] == "terminal" and state_id not in terminals
        }
        if undeclared_terminal_kinds:
            raise ModelError(
                f"{machine_id} has terminal-kind states not declared terminal: "
                f"{sorted(undeclared_terminal_kinds)}"
            )

        adjacency: dict[str, set[str]] = defaultdict(set)
        reverse: dict[str, set[str]] = defaultdict(set)
        outgoing_counts: dict[str, int] = defaultdict(int)
        for transition_index, transition in enumerate(
            require_table_list(machine["transition"], f"{context}.transition")
        ):
            transition_context = f"{context}.transition[{transition_index}]"
            require_exact_keys(
                transition,
                required={
                    "id",
                    "title",
                    "from",
                    "to",
                    "cause",
                    "authority",
                    "risk",
                    "invariants",
                    "evidence",
                    "covered_tiers",
                    "required_tiers",
                    "gaps",
                },
                allowed={
                    "id",
                    "title",
                    "from",
                    "to",
                    "cause",
                    "authority",
                    "risk",
                    "invariants",
                    "evidence",
                    "covered_tiers",
                    "required_tiers",
                    "gaps",
                },
                context=transition_context,
            )
            transition_id = require_string(transition["id"], f"{transition_context}.id")
            if not TRANSITION_ID.fullmatch(transition_id):
                raise ModelError(f"{transition_context}.id has invalid format {transition_id!r}")
            if transition_id in transition_ids:
                raise ModelError(f"duplicate transition id {transition_id}")
            transition_ids.add(transition_id)
            transition_count += 1
            require_string(transition["title"], f"{transition_context}.title")
            sources = require_string_list(transition["from"], f"{transition_context}.from")
            destination = require_string(transition["to"], f"{transition_context}.to")
            unknown_states = (set(sources) | {destination}) - set(states)
            if unknown_states:
                raise ModelError(
                    f"{transition_id} references unknown {machine_id} states "
                    f"{sorted(unknown_states)}"
                )
            require_string(transition["cause"], f"{transition_context}.cause")
            require_string(transition["authority"], f"{transition_context}.authority")
            risk = require_string(transition["risk"], f"{transition_context}.risk")
            if risk not in allowed_risks:
                raise ModelError(f"{transition_id} risk is not allowed: {risk}")

            invariant_refs = set(
                require_string_list(transition["invariants"], f"{transition_context}.invariants")
            )
            unknown_invariants = invariant_refs - set(invariants)
            if unknown_invariants:
                raise ModelError(
                    f"{transition_id} references unknown invariants {sorted(unknown_invariants)}"
                )
            referenced_invariants.update(invariant_refs)

            evidence_refs = set(
                require_string_list(
                    transition["evidence"], f"{transition_context}.evidence", allow_empty=True
                )
            )
            unknown_evidence = evidence_refs - catalog_behaviors
            if unknown_evidence:
                raise ModelError(
                    f"{transition_id} references unknown behaviors {sorted(unknown_evidence)}"
                )
            referenced_behaviors.update(evidence_refs)

            covered = set(
                require_string_list(
                    transition["covered_tiers"],
                    f"{transition_context}.covered_tiers",
                    allow_empty=True,
                )
            )
            required = set(
                require_string_list(
                    transition["required_tiers"], f"{transition_context}.required_tiers"
                )
            )
            if not required <= required_tiers:
                raise ModelError(
                    f"{transition_id} requires unknown proof tiers {sorted(required - required_tiers)}"
                )
            if not covered <= required:
                raise ModelError(
                    f"{transition_id} covers tiers it does not require {sorted(covered - required)}"
                )
            if risk == "critical" and required != required_tiers:
                raise ModelError(
                    f"critical transition {transition_id} must require model, seam, and system"
                )
            if covered and not evidence_refs:
                raise ModelError(f"{transition_id} claims covered tiers without evidence")
            if evidence_refs and not covered:
                raise ModelError(f"{transition_id} names evidence without covered tiers")

            gap_refs = set(
                require_string_list(
                    transition["gaps"], f"{transition_context}.gaps", allow_empty=True
                )
            )
            unknown_gaps = gap_refs - set(gaps)
            if unknown_gaps:
                raise ModelError(f"{transition_id} references unknown gaps {sorted(unknown_gaps)}")
            referenced_gaps.update(gap_refs)
            missing_tiers = required - covered
            if missing_tiers:
                open_refs = gap_refs & open_gaps
                if not open_refs:
                    raise ModelError(
                        f"{transition_id} is missing tiers {sorted(missing_tiers)} without an open gap"
                    )
                transitions_missing_tiers[transition_id] = sorted(missing_tiers)
            elif gap_refs & open_gaps:
                raise ModelError(
                    f"{transition_id} is fully covered but still references open gaps "
                    f"{sorted(gap_refs & open_gaps)}"
                )

            for source in sources:
                adjacency[source].add(destination)
                reverse[destination].add(source)
                outgoing_counts[source] += 1

        reachable = reachable_states(initial, adjacency)
        unreachable = set(states) - reachable
        if unreachable:
            raise ModelError(f"{machine_id} has unreachable states {sorted(unreachable)}")
        nonterminal_without_exit = {
            state_id for state_id in states if state_id not in terminals and not outgoing_counts[state_id]
        }
        if nonterminal_without_exit:
            raise ModelError(
                f"{machine_id} has nonterminal states without exits "
                f"{sorted(nonterminal_without_exit)}"
            )
        if mode == "finite":
            can_finish = states_reaching(terminals, reverse)
            cannot_finish = set(states) - can_finish
            if cannot_finish:
                raise ModelError(
                    f"finite machine {machine_id} has states with no terminal path "
                    f"{sorted(cannot_finish)}"
                )
        else:
            can_reset = states_reaching([initial], reverse)
            cannot_reset = set(states) - can_reset
            if cannot_reset:
                raise ModelError(
                    f"cyclic machine {machine_id} has states with no path to initial state "
                    f"{sorted(cannot_reset)}"
                )

    unused_invariants = set(invariants) - referenced_invariants
    if unused_invariants:
        raise ModelError(f"unreferenced lifecycle invariants: {sorted(unused_invariants)}")
    unused_gaps = set(gaps) - referenced_gaps
    if unused_gaps:
        raise ModelError(f"unreferenced lifecycle gaps: {sorted(unused_gaps)}")

    return {
        "schema_version": 1,
        "model_id": model_id,
        "machine_count": len(machine_ids),
        "state_count": state_count,
        "transition_count": transition_count,
        "invariant_count": len(invariants),
        "referenced_behavior_count": len(referenced_behaviors),
        "gap_count": len(gaps),
        "open_gaps": sorted(open_gaps),
        "transitions_missing_tiers": dict(sorted(transitions_missing_tiers.items())),
    }


def load_and_validate(
    model_path: pathlib.Path,
    *,
    repo_root: pathlib.Path,
    require_closed: bool = False,
) -> dict[str, Any]:
    resolved_root = repo_root.resolve()
    return validate_model(
        load_toml(model_path.resolve(), "lifecycle model"),
        repo_root=resolved_root,
        require_closed=require_closed,
    )


def build_parser(repo_root: pathlib.Path) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=("validate", "summary"),
        help="validate the model or emit its validated JSON summary",
    )
    parser.add_argument(
        "--model",
        type=pathlib.Path,
        default=repo_root / "coverage" / "playback-lifecycle.toml",
    )
    parser.add_argument("--repo-root", type=pathlib.Path, default=repo_root)
    parser.add_argument(
        "--require-closed",
        action="store_true",
        help="reject every open lifecycle gap",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    script_root = pathlib.Path(__file__).resolve().parent.parent
    parser = build_parser(script_root)
    args = parser.parse_args(argv)
    try:
        summary = load_and_validate(
            args.model,
            repo_root=args.repo_root,
            require_closed=args.require_closed,
        )
    except ModelError as error:
        print(f"playback lifecycle model invalid: {error}", file=sys.stderr)
        return 2

    if args.command == "summary":
        print(json.dumps(summary, indent=2, sort_keys=True))
    else:
        print(
            "valid playback lifecycle model: "
            f"{summary['machine_count']} machines, "
            f"{summary['state_count']} states, "
            f"{summary['transition_count']} transitions, "
            f"{summary['invariant_count']} invariants, "
            f"{summary['gap_count']} gaps "
            f"({len(summary['open_gaps'])} open)"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
