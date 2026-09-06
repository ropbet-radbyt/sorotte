"""Workflow identity and execution invariants independent of display labels.

The reviewed index binds workflow/job/step IDs. It does not bless commands,
conditions, permissions, source refs or outcome checks; callers verify those
responsibilities separately. Conditions require an explicit reviewed match,
never a substring such as ``always()`` that can hide ``&& false``.
"""
from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
import re
import shlex

ROOT = Path(__file__).resolve().parents[1]
INDEX = ROOT / "coverage/ci-step-contracts.json"
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_-]*")


class ContractJobs(dict):
    """Carry the workflow scope through ordinary deepcopy-based mutations."""
    def __init__(self, jobs: dict, workflow: str):
        super().__init__(jobs)
        self.workflow = workflow


def workflow_name(path: str | Path) -> str:
    path = Path(path)
    name = path.relative_to(ROOT).as_posix() if path.is_absolute() else path.as_posix()
    if not re.fullmatch(r"\.github/workflows/[A-Za-z0-9_-]+\.ya?ml", name):
        raise AssertionError("step contracts require an explicit repository workflow path")
    return name


@lru_cache(maxsize=1)
def contracts() -> tuple[dict, ...]:
    if not INDEX.is_file():
        raise AssertionError("reviewed workflow step identity index is missing")
    value = json.loads(INDEX.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise AssertionError("workflow step identity index must be an object")
    entries = value.get("steps")
    if (type(value.get("schema_version")) is not int or value["schema_version"] != 1
            or not isinstance(entries, list) or not entries):
        raise AssertionError("unsupported or empty workflow step identity index")
    identities, labels = set(), set()
    for entry in entries:
        if (not isinstance(entry, dict)
                or set(entry) != {"workflow", "job_id", "step_id", "display_name"}
                or not all(isinstance(entry[key], str) for key in entry)
                or not IDENTIFIER.fullmatch(entry["job_id"])
                or not IDENTIFIER.fullmatch(entry["step_id"])
                or not isinstance(entry["display_name"], str) or not entry["display_name"].strip()):
            raise AssertionError("invalid reviewed step identity")
        workflow_name(entry["workflow"])
        identity = (entry["workflow"], entry["job_id"], entry["step_id"])
        label = (entry["workflow"], entry["job_id"], entry["display_name"])
        if identity in identities or label in labels:
            raise AssertionError("duplicate reviewed step identity or label")
        identities.add(identity)
        labels.add(label)
    return tuple(entries)


def _step_ids(job: dict) -> dict[str, dict]:
    result = {}
    steps = job.get("steps", [])
    if not isinstance(steps, list):
        raise AssertionError("workflow steps must be a sequence")
    for step in steps:
        if not isinstance(step, dict):
            raise AssertionError("workflow steps must be objects")
        if "id" not in step:
            continue
        identity = step["id"]
        if not isinstance(identity, str) or not IDENTIFIER.fullmatch(identity) or identity in result:
            raise AssertionError("workflow step IDs must be valid and unique within each job")
        result[identity] = step
    return result


def canonicalize_labels(jobs: dict, workflow: str | Path) -> ContractJobs:
    scope = workflow_name(workflow)
    indexed = [entry for entry in contracts() if entry["workflow"] == scope]
    if not indexed:
        raise AssertionError(f"{scope}: reviewed step identities are missing")
    scoped = ContractJobs(jobs, scope)
    present = {job_id: _step_ids(job) for job_id, job in scoped.items()}
    for entry in indexed:
        step = present.get(entry["job_id"], {}).get(entry["step_id"])
        if step is None:
            raise AssertionError(f"{scope}: required step ID {entry['job_id']}/{entry['step_id']} is missing")
        step["name"] = entry["display_name"]
    return scoped


def by_contract(jobs: dict, job_id: str, label: str) -> dict:
    scope = getattr(jobs, "workflow", None)
    if scope is None:
        raise AssertionError("required step lookup needs explicit workflow scope")
    entries = [entry for entry in contracts() if entry["workflow"] == scope
               and entry["job_id"] == job_id and entry["display_name"] == label]
    if len(entries) != 1 or job_id not in jobs:
        raise AssertionError(f"{scope}: required step contract {job_id}/{label!r} is missing or ambiguous")
    found = _step_ids(jobs[job_id]).get(entries[0]["step_id"])
    if found is None:
        raise AssertionError(f"{job_id}: required step ID for {label!r} is missing")
    return found


def condition(value: object) -> str:
    if not isinstance(value, str):
        raise AssertionError("execution conditions must be reviewed expressions")
    value = value.strip()
    if value.startswith("${{") and value.endswith("}}"):
        value = value[3:-2].strip()
    return " ".join(value.split())


def no_error_tolerance(value: dict) -> None:
    if "continue-on-error" in value:
        setting = value["continue-on-error"]
        if setting is not False and setting != "false":
            raise AssertionError("required execution cannot tolerate failure or use dynamic error tolerance")


def _commands(script: str) -> list[list[str]]:
    if not isinstance(script, str):
        raise AssertionError("required run commands must be shell text")
    script = re.sub(r"(?:\\|`)\r?\n", " ", script)
    result = []
    for line in script.splitlines():
        lexer = shlex.shlex(line, posix=True, punctuation_chars=";&|<>")
        lexer.whitespace_split = True
        try:
            tokens = list(lexer)
        except ValueError as error:
            raise AssertionError(f"invalid required shell command: {error}") from error
        if tokens:
            result.append(tokens)
    return result


def command_step(job: dict, prefix: str, *, allowed_if: str | None = None) -> dict:
    wanted = shlex.split(prefix)
    if not wanted:
        raise AssertionError("required command prefix cannot be empty")
    found = []
    for step in job.get("steps", []):
        commands = _commands(step.get("run", ""))
        for tokens in commands:
            offset = int(tokens[0] in {"python", "python3", "python.exe", "py"})
            if tokens[offset:offset + len(wanted)] == wanted:
                if len(commands) != 1 or any(re.fullmatch(r"[;&|<>]+", token) for token in tokens):
                    raise AssertionError("required command must be a single executable producer, without shell control-flow bypass")
                if any(token in {"-h", "--help", "--version"} for token in tokens[offset + len(wanted):]):
                    raise AssertionError("required command cannot substitute help or version output for execution")
                found.append(step)
    if len(found) != 1:
        raise AssertionError(f"expected one executable command producer for {prefix}")
    no_error_tolerance(found[0])
    actual = found[0].get("if")
    if (allowed_if is None and actual is not None) or (allowed_if is not None and condition(actual) != condition(allowed_if)):
        raise AssertionError("required producer execution condition changed")
    return found[0]


def required_graph(jobs: dict, final: str, expected: set[str], *,
                   allowed_if: str = "always()", dependency_conditions: dict[str, str] | None = None) -> None:
    supported_aggregates = {"always()", "always() && github.event_name != 'schedule'", "always() && github.event_name == 'schedule'"}
    if condition(allowed_if) not in supported_aggregates:
        raise AssertionError("aggregate condition is outside reviewed event restrictions")
    if final not in jobs or not expected:
        raise AssertionError("required aggregate or producer inventory is missing")
    job = jobs[final]
    if condition(job.get("if")) != condition(allowed_if):
        raise AssertionError("aggregate must run after failures and skips for every reviewed event")
    no_error_tolerance(job)
    needs = job.get("needs", [])
    if isinstance(needs, str):
        needs = [needs]
    if (not isinstance(needs, list) or not all(isinstance(item, str) for item in needs)
            or len(needs) != len(set(needs)) or set(needs) != expected):
        raise AssertionError("required dependency graph changed or contains duplicate producers")
    reviewed = dependency_conditions or {}
    if reviewed.keys() - expected:
        raise AssertionError("condition was specified for an unrequired producer")
    for dependency in expected:
        if dependency not in jobs:
            raise AssertionError("required producer is missing")
        producer = jobs[dependency]
        no_error_tolerance(producer)
        actual = producer.get("if")
        wanted = reviewed.get(dependency)
        if (wanted is None and actual is not None) or (wanted is not None and condition(actual) != condition(wanted)):
            raise AssertionError(f"required producer {dependency} has an unreviewed skip condition")
