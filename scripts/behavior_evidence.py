#!/usr/bin/env python3
"""Fail-closed behavior catalog runner and CI evidence aggregator.

The catalog contains data, never shell fragments. This program constructs every
subprocess argv itself, records exact proof identities, verifies the checked-out
Git revision, and refuses to combine evidence from another commit, workflow
run, repository, or catalog. A shard from an earlier attempt of the same
workflow run is accepted so failed-job reruns can reuse successful evidence.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import sys
import tempfile
import time
import tomllib
from collections.abc import Iterable, Mapping, Sequence
from typing import Any


EVIDENCE_KIND = "sorotte-behavior-evidence-shard"
AGGREGATE_KIND = "sorotte-behavior-evidence-aggregate"
SCHEMA_VERSION = 1
MAX_EVIDENCE_BYTES = 1_048_576
IDENTIFIER = re.compile(r"^[a-z][a-z0-9-]*$")
BEHAVIOR_ID = re.compile(r"^[A-Z][A-Z0-9]*-[A-Z][A-Z0-9]*-[0-9]{3}$")
PROOF_ID = re.compile(r"^[A-Z][A-Z0-9-]*-[0-9]{3}\.[a-z][a-z0-9-]*$")
RUST_SELECTOR = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+$")
SHA = re.compile(r"^[0-9a-f]{40}$")
REPOSITORY = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
RUN_ID = re.compile(r"^[A-Za-z0-9_.-]+$")
ALLOWED_PROOF_STATUSES = {"passed", "failed", "ignored", "skipped", "error"}


class CatalogError(ValueError):
    pass


class EvidenceError(ValueError):
    pass


def is_json_integer(value: Any) -> bool:
    """Return true only for JSON integer values, never booleans."""
    return type(value) is int


@dataclasses.dataclass(frozen=True)
class ProcessResult:
    argv: tuple[str, ...]
    return_code: int
    stdout: str
    stderr: str
    duration_ms: int
    timed_out: bool = False

    @property
    def combined_output(self) -> str:
        return f"{self.stdout}\n{self.stderr}"


def utc_now() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def reject_duplicate_json_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise EvidenceError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def load_json_text(text: str) -> Any:
    return json.loads(text, object_pairs_hook=reject_duplicate_json_keys)


def exact_keys(
    value: Mapping[str, Any],
    *,
    allowed: set[str],
    required: set[str],
    context: str,
) -> None:
    unknown = set(value) - allowed
    missing = required - set(value)
    if unknown:
        raise CatalogError(f"{context} has unknown keys: {sorted(unknown)}")
    if missing:
        raise CatalogError(f"{context} is missing keys: {sorted(missing)}")


def require_string(value: Any, context: str) -> str:
    if not isinstance(value, str) or not value.strip() or value != value.strip():
        raise CatalogError(f"{context} must be a non-empty trimmed string")
    return value


def require_string_list(value: Any, context: str, *, nonempty: bool = True) -> list[str]:
    if not isinstance(value, list) or (nonempty and not value):
        raise CatalogError(f"{context} must be a {'non-empty ' if nonempty else ''}list")
    result = [require_string(item, f"{context}[]") for item in value]
    if len(set(result)) != len(result):
        raise CatalogError(f"{context} contains duplicates")
    return result


def safe_source(repo_root: pathlib.Path, source: Any, context: str) -> pathlib.Path:
    relative = pathlib.PurePosixPath(require_string(source, context))
    if relative.is_absolute() or ".." in relative.parts or "\\" in str(relative):
        raise CatalogError(f"{context} must be a normalized repository-relative POSIX path")
    candidate = (repo_root / pathlib.Path(*relative.parts)).resolve()
    root = repo_root.resolve()
    try:
        candidate.relative_to(root)
    except ValueError as error:
        raise CatalogError(f"{context} escapes the repository") from error
    if not candidate.is_file():
        raise CatalogError(f"{context} does not exist: {relative}")
    return candidate


def catalog_digest(catalog_path: pathlib.Path) -> str:
    return f"sha256:{hashlib.sha256(catalog_path.read_bytes()).hexdigest()}"


def load_catalog(catalog_path: pathlib.Path) -> dict[str, Any]:
    try:
        with catalog_path.open("rb") as handle:
            value = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise CatalogError(f"cannot read catalog {catalog_path}: {error}") from error
    if not isinstance(value, dict):
        raise CatalogError("catalog root must be a table")
    return value


def validate_catalog(
    catalog: Mapping[str, Any],
    *,
    repo_root: pathlib.Path,
) -> dict[str, Any]:
    exact_keys(
        catalog,
        allowed={"schema_version", "policy", "lanes", "behavior"},
        required={"schema_version", "policy", "lanes", "behavior"},
        context="catalog",
    )
    if (
        not is_json_integer(catalog["schema_version"])
        or catalog["schema_version"] != SCHEMA_VERSION
    ):
        raise CatalogError(f"unsupported catalog schema {catalog['schema_version']!r}")

    policy = catalog["policy"]
    if not isinstance(policy, dict):
        raise CatalogError("policy must be a table")
    exact_keys(
        policy,
        allowed={
            "allowed_namespaces",
            "allowed_risks",
            "critical_minimum_proofs",
            "required_jobs",
        },
        required={
            "allowed_namespaces",
            "allowed_risks",
            "critical_minimum_proofs",
            "required_jobs",
        },
        context="policy",
    )
    namespaces = require_string_list(policy["allowed_namespaces"], "policy.allowed_namespaces")
    risks = require_string_list(policy["allowed_risks"], "policy.allowed_risks")
    minimum = policy["critical_minimum_proofs"]
    if not is_json_integer(minimum) or minimum < 1:
        raise CatalogError("policy.critical_minimum_proofs must be a positive integer")
    required_jobs = require_string_list(policy["required_jobs"], "policy.required_jobs")
    if any(not IDENTIFIER.fullmatch(job) for job in required_jobs):
        raise CatalogError("policy.required_jobs contains an invalid job identifier")

    lanes = catalog["lanes"]
    if not isinstance(lanes, dict) or not lanes:
        raise CatalogError("lanes must be a non-empty table")
    normalized_lanes: dict[str, dict[str, Any]] = {}
    for lane_id, lane in lanes.items():
        if not isinstance(lane_id, str) or not IDENTIFIER.fullmatch(lane_id):
            raise CatalogError(f"invalid lane identifier {lane_id!r}")
        if not isinstance(lane, dict):
            raise CatalogError(f"lane {lane_id} must be a table")
        runner = lane.get("runner")
        allowed = {
            "runner",
            "required_on",
            "operating_systems",
            "timeout_seconds",
        }
        required = set(allowed)
        if runner == "gui-semantic-suite":
            allowed |= {"complete_inventory", "semantic_suite"}
            required |= {"complete_inventory", "semantic_suite"}
        exact_keys(lane, allowed=allowed, required=required, context=f"lane {lane_id}")
        if runner not in {"rust-exact", "gui-semantic-suite"}:
            raise CatalogError(f"lane {lane_id} has unsupported runner {runner!r}")
        required_on = require_string_list(lane["required_on"], f"lane {lane_id}.required_on")
        if not set(required_on) <= {"push", "pull_request", "workflow_dispatch", "schedule"}:
            raise CatalogError(f"lane {lane_id} has unsupported trigger")
        operating_systems = require_string_list(
            lane["operating_systems"], f"lane {lane_id}.operating_systems"
        )
        if not set(operating_systems) <= {"linux", "windows", "macos"}:
            raise CatalogError(f"lane {lane_id} has unsupported operating system")
        timeout = lane["timeout_seconds"]
        if not is_json_integer(timeout) or not 1 <= timeout <= 3600:
            raise CatalogError(f"lane {lane_id}.timeout_seconds must be 1..3600")
        normalized = dict(lane)
        if runner == "gui-semantic-suite":
            if lane["complete_inventory"] is not True:
                raise CatalogError(f"lane {lane_id} must use complete_inventory=true")
            suite = lane["semantic_suite"]
            if not isinstance(suite, dict):
                raise CatalogError(f"lane {lane_id}.semantic_suite must be a table")
            exact_keys(
                suite,
                allowed={"package", "binary", "features", "scenarios"},
                required={"package", "binary", "features", "scenarios"},
                context=f"lane {lane_id}.semantic_suite",
            )
            require_string(suite["package"], f"lane {lane_id}.semantic_suite.package")
            require_string(suite["binary"], f"lane {lane_id}.semantic_suite.binary")
            require_string_list(suite["features"], f"lane {lane_id}.semantic_suite.features")
            require_string_list(suite["scenarios"], f"lane {lane_id}.semantic_suite.scenarios")
        normalized_lanes[lane_id] = normalized

    behaviors = catalog["behavior"]
    if not isinstance(behaviors, list) or not behaviors:
        raise CatalogError("behavior must be a non-empty array of tables")
    behavior_ids: set[str] = set()
    proof_ids: set[str] = set()
    normalized_behaviors: list[dict[str, Any]] = []
    for index, behavior in enumerate(behaviors):
        context = f"behavior[{index}]"
        if not isinstance(behavior, dict):
            raise CatalogError(f"{context} must be a table")
        exact_keys(
            behavior,
            allowed={"id", "title", "risk", "owners", "invariants", "proof"},
            required={"id", "title", "risk", "owners", "invariants", "proof"},
            context=context,
        )
        behavior_id = require_string(behavior["id"], f"{context}.id")
        if not BEHAVIOR_ID.fullmatch(behavior_id):
            raise CatalogError(f"{context}.id has invalid shape: {behavior_id!r}")
        if behavior_id in behavior_ids:
            raise CatalogError(f"duplicate behavior id {behavior_id}")
        behavior_ids.add(behavior_id)
        namespace = behavior_id.split("-", 1)[0]
        if namespace not in namespaces:
            raise CatalogError(f"{behavior_id} uses disallowed namespace {namespace}")
        require_string(behavior["title"], f"{context}.title")
        risk = require_string(behavior["risk"], f"{context}.risk")
        if risk not in risks:
            raise CatalogError(f"{behavior_id} uses disallowed risk {risk}")
        require_string_list(behavior["owners"], f"{context}.owners")
        require_string_list(behavior["invariants"], f"{context}.invariants")
        proofs = behavior["proof"]
        if not isinstance(proofs, list) or not proofs:
            raise CatalogError(f"{behavior_id} needs at least one proof")
        if risk == "critical" and len(proofs) < minimum:
            raise CatalogError(f"{behavior_id} needs at least {minimum} proofs")
        normalized_proofs: list[dict[str, Any]] = []
        for proof_index, proof in enumerate(proofs):
            proof_context = f"{behavior_id}.proof[{proof_index}]"
            if not isinstance(proof, dict):
                raise CatalogError(f"{proof_context} must be a table")
            common = {
                "id",
                "kind",
                "oracle",
                "source",
                "operating_systems",
                "required_lanes",
            }
            kind = proof.get("kind")
            if kind == "rust-test":
                specific = {"package", "target_kind", "test", "feature_mode"}
            elif kind == "semantic-scenario":
                specific = {"scenario"}
            else:
                raise CatalogError(f"{proof_context} has unsupported kind {kind!r}")
            exact_keys(
                proof,
                allowed=common | specific,
                required=common | specific,
                context=proof_context,
            )
            proof_id = require_string(proof["id"], f"{proof_context}.id")
            if not PROOF_ID.fullmatch(proof_id) or not proof_id.startswith(f"{behavior_id}."):
                raise CatalogError(f"{proof_context}.id does not belong to {behavior_id}")
            if proof_id in proof_ids:
                raise CatalogError(f"duplicate proof id {proof_id}")
            proof_ids.add(proof_id)
            require_string(proof["oracle"], f"{proof_context}.oracle")
            source_path = safe_source(repo_root, proof["source"], f"{proof_context}.source")
            proof_operating_systems = require_string_list(
                proof["operating_systems"], f"{proof_context}.operating_systems"
            )
            lane_ids = require_string_list(proof["required_lanes"], f"{proof_context}.required_lanes")
            if len(lane_ids) != 1:
                raise CatalogError(f"{proof_context} must belong to exactly one required lane")
            lane_id = lane_ids[0]
            lane = normalized_lanes.get(lane_id)
            if lane is None:
                raise CatalogError(f"{proof_context} references unknown lane {lane_id}")
            if not set(proof_operating_systems) <= set(lane["operating_systems"]):
                raise CatalogError(f"{proof_context} operating systems exceed lane {lane_id}")
            if kind == "rust-test":
                if lane["runner"] != "rust-exact":
                    raise CatalogError(f"{proof_context} must use a rust-exact lane")
                require_string(proof["package"], f"{proof_context}.package")
                if proof["target_kind"] != "lib":
                    raise CatalogError(f"{proof_context}.target_kind must be lib")
                if proof["feature_mode"] != "all-features":
                    raise CatalogError(f"{proof_context}.feature_mode must be all-features")
                selector = require_string(proof["test"], f"{proof_context}.test")
                if not RUST_SELECTOR.fullmatch(selector):
                    raise CatalogError(f"{proof_context}.test is not an exact Rust test selector")
                leaf = selector.rsplit("::", 1)[1]
                source_text = source_path.read_text(encoding="utf-8")
                matches = list(
                    re.finditer(
                        rf"(?m)^\s*(?:async\s+)?fn\s+{re.escape(leaf)}\s*\(",
                        source_text,
                    )
                )
                if len(matches) != 1:
                    raise CatalogError(
                        f"{proof_context}.source must define {leaf} exactly once; found {len(matches)}"
                    )
                preceding = source_text[max(0, matches[0].start() - 256) : matches[0].start()]
                if re.search(r"#\s*\[\s*ignore(?:\s*=|\s*\])", preceding):
                    raise CatalogError(f"{proof_context} points at an ignored test")
            else:
                if lane["runner"] != "gui-semantic-suite":
                    raise CatalogError(f"{proof_context} must use a gui-semantic-suite lane")
                scenario = require_string(proof["scenario"], f"{proof_context}.scenario")
                inventory = lane["semantic_suite"]["scenarios"]
                if scenario not in inventory:
                    raise CatalogError(f"{proof_context} scenario is outside lane inventory")
                if source_path.name != f"{scenario}.txt":
                    raise CatalogError(f"{proof_context}.source must be {scenario}.txt")
            normalized_proofs.append(dict(proof))
        normalized_behavior = dict(behavior)
        normalized_behavior["proof"] = normalized_proofs
        normalized_behaviors.append(normalized_behavior)

    return {
        "schema_version": SCHEMA_VERSION,
        "policy": dict(policy),
        "lanes": normalized_lanes,
        "behavior": normalized_behaviors,
    }


def proofs_for_lane(catalog: Mapping[str, Any], lane_id: str) -> list[tuple[str, dict[str, Any]]]:
    proofs: list[tuple[str, dict[str, Any]]] = []
    for behavior in catalog["behavior"]:
        for proof in behavior["proof"]:
            if lane_id in proof["required_lanes"]:
                proofs.append((behavior["id"], proof))
    return sorted(proofs, key=lambda item: item[1]["id"])


def rust_discovery_argv(package: str) -> tuple[str, ...]:
    return (
        "cargo",
        "test",
        "--locked",
        "-p",
        package,
        "--all-features",
        "--lib",
        "--",
        "--list",
        "--format",
        "terse",
    )


def rust_execution_argv(proof: Mapping[str, Any]) -> tuple[str, ...]:
    return (
        "cargo",
        "test",
        "--locked",
        "-p",
        proof["package"],
        "--all-features",
        "--lib",
        proof["test"],
        "--",
        "--exact",
        "--nocapture",
    )


def semantic_cargo_prefix(lane: Mapping[str, Any]) -> tuple[str, ...]:
    suite = lane["semantic_suite"]
    return (
        "cargo",
        "run",
        "--quiet",
        "--locked",
        "-p",
        suite["package"],
        "--features",
        ",".join(suite["features"]),
        "--bin",
        suite["binary"],
        "--",
    )


def semantic_inventory_argv(lane: Mapping[str, Any]) -> tuple[str, ...]:
    return (*semantic_cargo_prefix(lane), "--list", "--json")


def semantic_execution_argv(lane: Mapping[str, Any]) -> tuple[str, ...]:
    argv = [*semantic_cargo_prefix(lane), "--json"]
    for scenario in lane["semantic_suite"]["scenarios"]:
        argv.extend(("--scenario", scenario))
    return tuple(argv)


def run_process(
    argv: Sequence[str],
    *,
    cwd: pathlib.Path,
    timeout_seconds: int,
    env: Mapping[str, str] | None = None,
) -> ProcessResult:
    started = time.monotonic()
    print(f"+ {subprocess.list2cmdline(list(argv))}", flush=True)
    try:
        completed = subprocess.run(
            list(argv),
            cwd=cwd,
            env=dict(env) if env is not None else None,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout_seconds,
            shell=False,
            check=False,
        )
        result = ProcessResult(
            tuple(argv),
            completed.returncode,
            completed.stdout,
            completed.stderr,
            round((time.monotonic() - started) * 1000),
        )
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout.decode("utf-8", "replace") if isinstance(error.stdout, bytes) else error.stdout
        stderr = error.stderr.decode("utf-8", "replace") if isinstance(error.stderr, bytes) else error.stderr
        result = ProcessResult(
            tuple(argv),
            124,
            stdout or "",
            stderr or "",
            round((time.monotonic() - started) * 1000),
            timed_out=True,
        )
    if result.stdout:
        print(result.stdout, end="" if result.stdout.endswith("\n") else "\n", flush=True)
    if result.stderr:
        print(result.stderr, end="" if result.stderr.endswith("\n") else "\n", file=sys.stderr, flush=True)
    return result


def discover_libtests(output: str) -> list[str]:
    names: list[str] = []
    for line in output.splitlines():
        if line.endswith(": test"):
            names.append(line[: -len(": test")].strip())
    return names


def parse_libtest_execution(result: ProcessResult, selector: str) -> tuple[str, dict[str, int]]:
    output = result.combined_output
    exact_lines = re.findall(rf"(?m)^test {re.escape(selector)} \.\.\. (ok|ignored|FAILED)$", output)
    summaries = [
        tuple(map(int, match))
        for match in re.findall(
            r"test result: (?:ok|FAILED)\. ([0-9]+) passed; ([0-9]+) failed; "
            r"([0-9]+) ignored;",
            output,
        )
    ]
    observed = {"passed": 0, "failed": 0, "ignored": 0}
    if summaries:
        observed = {
            "passed": sum(item[0] for item in summaries),
            "failed": sum(item[1] for item in summaries),
            "ignored": sum(item[2] for item in summaries),
        }
    if result.timed_out:
        return "error", observed
    if exact_lines == ["ignored"] or observed["ignored"]:
        return "ignored", observed
    if (
        result.return_code == 0
        and exact_lines == ["ok"]
        and observed == {"passed": 1, "failed": 0, "ignored": 0}
    ):
        return "passed", observed
    return "failed", observed


def extract_json_document(output: str) -> Mapping[str, Any]:
    for line in reversed([line.strip() for line in output.splitlines() if line.strip()]):
        if line.startswith("{") and line.endswith("}"):
            value = load_json_text(line)
            if isinstance(value, dict):
                return value
    raise EvidenceError("subprocess output contained no JSON object")


def parse_semantic_inventory(value: Mapping[str, Any]) -> list[str]:
    if set(value) != {"result", "scenarios"} or value.get("result") != "ok":
        raise EvidenceError("semantic inventory has an unexpected schema or result")
    scenarios = value.get("scenarios")
    if not isinstance(scenarios, list) or not all(isinstance(item, str) for item in scenarios):
        raise EvidenceError("semantic inventory scenarios must be strings")
    if len(scenarios) != len(set(scenarios)):
        raise EvidenceError("semantic inventory contains duplicate scenarios")
    return list(scenarios)


def parse_semantic_summary(value: Mapping[str, Any]) -> tuple[list[str], list[str]]:
    required = {"result", "total", "passed", "failed", "reports", "errors"}
    if set(value) != required:
        raise EvidenceError("semantic summary has an unexpected schema")
    reports = value["reports"]
    errors = value["errors"]
    if not isinstance(reports, list) or not isinstance(errors, list):
        raise EvidenceError("semantic reports and errors must be arrays")
    report_names: list[str] = []
    for report in reports:
        if not isinstance(report, dict) or report.get("result") != "ok":
            raise EvidenceError("semantic report is malformed or not successful")
        name = report.get("scenario")
        if not isinstance(name, str):
            raise EvidenceError("semantic report is missing a scenario name")
        report_names.append(name)
    error_names: list[str] = []
    for error in errors:
        if not isinstance(error, dict) or not isinstance(error.get("scenario"), str):
            raise EvidenceError("semantic error is malformed")
        error_names.append(error["scenario"])
    if len(report_names) != len(set(report_names)):
        raise EvidenceError("semantic summary contains duplicate successful scenarios")
    if len(error_names) != len(set(error_names)):
        raise EvidenceError("semantic summary contains duplicate failed scenarios")
    total = value["total"]
    passed = value["passed"]
    failed = value["failed"]
    if (
        not all(is_json_integer(item) for item in (total, passed, failed))
        or total != len(reports) + len(errors)
        or passed != len(reports)
        or failed != len(errors)
        or total != passed + failed
    ):
        raise EvidenceError("semantic summary counts are inconsistent")
    if value["result"] != ("ok" if failed == 0 else "error"):
        raise EvidenceError("semantic summary result contradicts its failures")
    return report_names, error_names


def tool_versions(repo_root: pathlib.Path) -> dict[str, str]:
    versions = {"python": platform.python_version()}
    for tool in ("git", "rustc", "cargo"):
        try:
            result = subprocess.run(
                [tool, "--version"],
                cwd=repo_root,
                capture_output=True,
                text=True,
                timeout=15,
                shell=False,
                check=False,
            )
            versions[tool] = result.stdout.strip() if result.returncode == 0 else "unavailable"
        except (OSError, subprocess.TimeoutExpired):
            versions[tool] = "unavailable"
    return versions


def verify_git_head(repo_root: pathlib.Path, expected_sha: str) -> None:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--verify", "HEAD"],
            cwd=repo_root,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=15,
            shell=False,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise EvidenceError(f"cannot verify checked-out Git revision: {error}") from error
    observed = result.stdout.strip()
    if result.returncode != 0 or not SHA.fullmatch(observed):
        detail = result.stderr.strip() or observed or f"exit {result.returncode}"
        raise EvidenceError(f"cannot verify checked-out Git revision: {detail}")
    if observed != expected_sha:
        raise EvidenceError(
            f"checked-out Git revision {observed} does not match evidence SHA {expected_sha}"
        )


def verify_clean_worktree(repo_root: pathlib.Path) -> None:
    try:
        result = subprocess.run(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"],
            cwd=repo_root,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=15,
            shell=False,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise EvidenceError(f"cannot verify clean Git worktree: {error}") from error
    if result.returncode != 0:
        detail = result.stderr.strip() or f"exit {result.returncode}"
        raise EvidenceError(f"cannot verify clean Git worktree: {detail}")
    changes = [line for line in result.stdout.splitlines() if line.strip()]
    if changes:
        preview = ", ".join(changes[:10])
        suffix = "" if len(changes) <= 10 else f", and {len(changes) - 10} more"
        raise EvidenceError(
            f"evidence requires a clean Git worktree; observed {preview}{suffix}"
        )


def atomic_write_json(path: pathlib.Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    rendered = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(rendered, encoding="utf-8", newline="\n")
    os.replace(temporary, path)


def validate_metadata(sha: str, repository: str, run_id: str, run_attempt: int, operating_system: str) -> None:
    if not SHA.fullmatch(sha):
        raise EvidenceError("--sha must be a 40-character lowercase commit SHA")
    if not REPOSITORY.fullmatch(repository):
        raise EvidenceError("--repository must have owner/name form")
    if not RUN_ID.fullmatch(run_id):
        raise EvidenceError("--run-id contains unsupported characters")
    if not is_json_integer(run_attempt) or run_attempt < 1:
        raise EvidenceError("--run-attempt must be positive")
    if operating_system not in {"linux", "windows", "macos"}:
        raise EvidenceError("--os must be linux, windows, or macos")


def base_shard(
    *,
    digest: str,
    lane_id: str,
    sha: str,
    repository: str,
    run_id: str,
    run_attempt: int,
    operating_system: str,
    repo_root: pathlib.Path,
) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": EVIDENCE_KIND,
        "catalog_sha256": digest,
        "repository": repository,
        "sha": sha,
        "run_id": run_id,
        "run_attempt": run_attempt,
        "lane": lane_id,
        "operating_system": operating_system,
        "status": "error",
        "started_at": utc_now(),
        "finished_at": None,
        "tool_versions": tool_versions(repo_root),
        "proofs": [],
        "inventory_cases": [],
        "errors": [],
    }


def run_rust_lane(
    catalog: Mapping[str, Any],
    lane_id: str,
    shard: dict[str, Any],
    *,
    repo_root: pathlib.Path,
) -> None:
    lane = catalog["lanes"][lane_id]
    timeout = lane["timeout_seconds"]
    proofs = proofs_for_lane(catalog, lane_id)
    package_inventory: dict[str, tuple[ProcessResult, list[str]]] = {}
    for _, proof in proofs:
        package = proof["package"]
        if package in package_inventory:
            continue
        argv = rust_discovery_argv(package)
        result = run_process(argv, cwd=repo_root, timeout_seconds=timeout)
        package_inventory[package] = (result, discover_libtests(result.combined_output))

    for behavior_id, proof in proofs:
        selector = proof["test"]
        discovery_result, discovered = package_inventory[proof["package"]]
        discovered_count = discovered.count(selector)
        argv = rust_execution_argv(proof)
        result = run_process(argv, cwd=repo_root, timeout_seconds=timeout)
        status, observed = parse_libtest_execution(result, selector)
        if discovery_result.return_code != 0 or discovered_count != 1:
            status = "error" if discovery_result.return_code != 0 else "failed"
        entry = {
            "behavior_id": behavior_id,
            "proof_id": proof["id"],
            "kind": "rust-test",
            "selector": selector,
            "status": status,
            "duration_ms": result.duration_ms,
            "return_code": result.return_code,
            "observed": {
                **observed,
                "discovered_exactly": discovered_count,
            },
            "discovery_command": list(discovery_result.argv),
            "command": list(result.argv),
        }
        shard["proofs"].append(entry)
        if status != "passed":
            shard["errors"].append(f"{proof['id']}: exact Rust proof status is {status}")


def run_semantic_lane(
    catalog: Mapping[str, Any],
    lane_id: str,
    shard: dict[str, Any],
    *,
    repo_root: pathlib.Path,
) -> None:
    lane = catalog["lanes"][lane_id]
    timeout = lane["timeout_seconds"]
    suite = lane["semantic_suite"]
    env = dict(os.environ)
    env["CARGO_TERM_COLOR"] = "never"
    with tempfile.TemporaryDirectory(prefix="sorotte-semantic-evidence-") as temporary:
        env["SOROTTE_CLIENT_CONFIG_PATH"] = str(pathlib.Path(temporary) / "sorotte.ini")
        inventory_result = run_process(
            semantic_inventory_argv(lane),
            cwd=repo_root,
            timeout_seconds=timeout,
            env=env,
        )
        inventory: list[str] = []
        try:
            inventory = parse_semantic_inventory(
                extract_json_document(inventory_result.combined_output)
            )
        except EvidenceError as error:
            shard["errors"].append(f"semantic inventory: {error}")
        expected = suite["scenarios"]
        if inventory_result.return_code != 0:
            shard["errors"].append(
                f"semantic inventory command exited {inventory_result.return_code}"
            )
        if set(inventory) != set(expected) or len(inventory) != len(expected):
            shard["errors"].append(
                f"semantic inventory mismatch: expected {expected!r}, observed {inventory!r}"
            )
        suite_result = run_process(
            semantic_execution_argv(lane),
            cwd=repo_root,
            timeout_seconds=timeout,
            env=env,
        )

    successful: list[str] = []
    failed: list[str] = []
    try:
        successful, failed = parse_semantic_summary(
            extract_json_document(suite_result.combined_output)
        )
    except EvidenceError as error:
        shard["errors"].append(f"semantic summary: {error}")
    if suite_result.return_code != 0:
        shard["errors"].append(f"semantic suite command exited {suite_result.return_code}")
    if set(successful) != set(expected) or len(successful) != len(expected) or failed:
        shard["errors"].append(
            f"semantic execution mismatch: passed={successful!r}, failed={failed!r}"
        )
    shard["inventory_cases"] = sorted(
        (
            {
                "scenario": scenario,
                "status": "passed" if scenario in successful else "failed",
            }
            for scenario in expected
        ),
        key=lambda case: case["scenario"],
    )
    for behavior_id, proof in proofs_for_lane(catalog, lane_id):
        scenario = proof["scenario"]
        status = "passed" if scenario in successful and scenario not in failed else "failed"
        shard["proofs"].append(
            {
                "behavior_id": behavior_id,
                "proof_id": proof["id"],
                "kind": "semantic-scenario",
                "selector": scenario,
                "status": status,
                "duration_ms": suite_result.duration_ms,
                "return_code": suite_result.return_code,
                "observed": {"passed": 1 if status == "passed" else 0},
                "inventory_command": list(inventory_result.argv),
                "command": list(suite_result.argv),
            }
        )
        if status != "passed":
            shard["errors"].append(f"{proof['id']}: semantic proof status is {status}")


def run_lane(args: argparse.Namespace) -> int:
    repo_root = pathlib.Path(args.repo_root).resolve()
    catalog_path = pathlib.Path(args.catalog).resolve()
    output = pathlib.Path(args.output).resolve()
    shard: dict[str, Any] | None = None
    try:
        validate_metadata(args.sha, args.repository, args.run_id, args.run_attempt, args.os)
        verify_git_head(repo_root, args.sha)
        verify_clean_worktree(repo_root)
        catalog = validate_catalog(load_catalog(catalog_path), repo_root=repo_root)
        lane = catalog["lanes"].get(args.lane)
        if lane is None:
            raise EvidenceError(f"unknown lane {args.lane!r}")
        if args.os not in lane["operating_systems"]:
            raise EvidenceError(f"lane {args.lane} does not support {args.os}")
        shard = base_shard(
            digest=catalog_digest(catalog_path),
            lane_id=args.lane,
            sha=args.sha,
            repository=args.repository,
            run_id=args.run_id,
            run_attempt=args.run_attempt,
            operating_system=args.os,
            repo_root=repo_root,
        )
        if lane["runner"] == "rust-exact":
            run_rust_lane(catalog, args.lane, shard, repo_root=repo_root)
        else:
            run_semantic_lane(catalog, args.lane, shard, repo_root=repo_root)
        # Proof code is untrusted with respect to provenance: tests and semantic
        # scenarios can mutate files or invoke Git. Re-bind the completed shard
        # to the claimed commit and clean source tree before it can pass.
        verify_git_head(repo_root, args.sha)
        verify_clean_worktree(repo_root)
        shard["proofs"] = sorted(shard["proofs"], key=lambda proof: proof["proof_id"])
        shard["status"] = "passed" if not shard["errors"] else "failed"
    except (CatalogError, EvidenceError, OSError) as error:
        if shard is None:
            shard = {
                "schema_version": SCHEMA_VERSION,
                "kind": EVIDENCE_KIND,
                "catalog_sha256": (
                    catalog_digest(catalog_path) if catalog_path.is_file() else "unavailable"
                ),
                "repository": args.repository,
                "sha": args.sha,
                "run_id": args.run_id,
                "run_attempt": args.run_attempt,
                "lane": args.lane,
                "operating_system": args.os,
                "status": "error",
                "started_at": utc_now(),
                "tool_versions": {"python": platform.python_version()},
                "proofs": [],
                "inventory_cases": [],
                "errors": [],
            }
        shard["errors"].append(str(error))
        shard["status"] = "error"
    shard["finished_at"] = utc_now()
    atomic_write_json(output, shard)
    print(f"wrote {shard['status']} evidence to {output}")
    return 0 if shard["status"] == "passed" else 1


def read_evidence(path: pathlib.Path) -> Mapping[str, Any]:
    if path.is_symlink():
        raise EvidenceError(f"refusing symlink evidence input {path}")
    try:
        size = path.stat().st_size
    except OSError as error:
        raise EvidenceError(f"cannot inspect evidence {path}: {error}") from error
    if size > MAX_EVIDENCE_BYTES:
        raise EvidenceError(f"evidence input exceeds 1 MiB: {path}")
    try:
        value = load_json_text(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise EvidenceError(f"cannot read evidence {path}: {error}") from error
    if not isinstance(value, dict):
        raise EvidenceError(f"evidence {path} must contain a JSON object")
    return value


def proof_evidence_errors(
    entry: Mapping[str, Any],
    *,
    lane_id: str,
    behavior_id: str,
    proof: Mapping[str, Any],
    lane: Mapping[str, Any],
) -> list[str]:
    proof_id = proof["id"]
    common = {
        "behavior_id",
        "proof_id",
        "kind",
        "selector",
        "status",
        "duration_ms",
        "return_code",
        "observed",
        "command",
    }
    runner_key = "discovery_command" if proof["kind"] == "rust-test" else "inventory_command"
    expected_keys = common | {runner_key}
    errors: list[str] = []
    if set(entry) != expected_keys:
        errors.append(
            f"proof {proof_id} fields differ: "
            f"expected={sorted(expected_keys)}, observed={sorted(entry)}"
        )
    expected_scalar = {
        "behavior_id": behavior_id,
        "proof_id": proof_id,
        "kind": proof["kind"],
        "selector": proof.get("test", proof.get("scenario")),
    }
    for field, expected in expected_scalar.items():
        if entry.get(field) != expected:
            errors.append(
                f"proof {proof_id} field {field} is {entry.get(field)!r}; expected {expected!r}"
            )
    status = entry.get("status")
    if not isinstance(status, str) or status not in ALLOWED_PROOF_STATUSES:
        errors.append(f"proof {proof_id} has invalid status {status!r}")
    if (
        not is_json_integer(entry.get("duration_ms"))
        or entry.get("duration_ms", -1) < 0
    ):
        errors.append(f"proof {proof_id} duration_ms must be a non-negative integer")
    if not is_json_integer(entry.get("return_code")):
        errors.append(f"proof {proof_id} return_code must be an integer")
    observed = entry.get("observed")
    if not isinstance(observed, dict):
        errors.append(f"proof {proof_id} observed must be an object")
        observed = {}

    if proof["kind"] == "rust-test":
        expected_discovery = list(rust_discovery_argv(proof["package"]))
        expected_command = list(rust_execution_argv(proof))
        if entry.get("discovery_command") != expected_discovery:
            errors.append(f"proof {proof_id} discovery command differs from catalog policy")
        if entry.get("command") != expected_command:
            errors.append(f"proof {proof_id} execution command differs from catalog policy")
        observed_keys = {"passed", "failed", "ignored", "discovered_exactly"}
        if set(observed) != observed_keys or not all(
            is_json_integer(observed.get(field)) for field in observed_keys
        ):
            errors.append(f"proof {proof_id} has malformed Rust observations")
        if status == "passed" and (
            entry.get("return_code") != 0
            or observed
            != {
                "passed": 1,
                "failed": 0,
                "ignored": 0,
                "discovered_exactly": 1,
            }
        ):
            errors.append(f"proof {proof_id} passing status contradicts Rust observations")
    else:
        if entry.get("inventory_command") != list(semantic_inventory_argv(lane)):
            errors.append(f"proof {proof_id} inventory command differs from catalog policy")
        if entry.get("command") != list(semantic_execution_argv(lane)):
            errors.append(f"proof {proof_id} execution command differs from catalog policy")
        if set(observed) != {"passed"} or not is_json_integer(observed.get("passed")):
            errors.append(f"proof {proof_id} has malformed semantic observations")
        if status == "passed" and (
            entry.get("return_code") != 0 or observed != {"passed": 1}
        ):
            errors.append(f"proof {proof_id} passing status contradicts semantic observations")
    return errors


def aggregate_evidence(
    catalog: Mapping[str, Any],
    shards: Sequence[Mapping[str, Any]],
    *,
    digest: str,
    expected_sha: str,
    expected_repository: str,
    expected_run_id: str,
    expected_run_attempt: int,
    job_results: Mapping[str, str],
) -> dict[str, Any]:
    errors: list[str] = []
    expected_attempt_is_valid = (
        is_json_integer(expected_run_attempt) and expected_run_attempt >= 1
    )
    if not expected_attempt_is_valid:
        errors.append(
            f"expected run attempt must be a positive integer, got {expected_run_attempt!r}"
        )
    required_jobs = set(catalog["policy"]["required_jobs"])
    supplied_jobs = set(job_results)
    missing_jobs = required_jobs - supplied_jobs
    unexpected_jobs = supplied_jobs - required_jobs
    if missing_jobs:
        errors.append(f"missing required job results: {sorted(missing_jobs)}")
    if unexpected_jobs:
        errors.append(f"unexpected job results: {sorted(unexpected_jobs)}")
    for job, result in sorted(job_results.items()):
        if result not in {"success", "failure", "cancelled", "skipped"}:
            errors.append(f"required job {job} has invalid conclusion {result!r}")
        elif result != "success":
            errors.append(f"required job {job} concluded {result}, not success")

    expected_lanes = set(catalog["lanes"])
    expected_proofs: dict[str, dict[str, Any]] = {}
    for behavior in catalog["behavior"]:
        for proof in behavior["proof"]:
            lane_id = proof["required_lanes"][0]
            expected_proofs[proof["id"]] = {
                "lane": lane_id,
                "behavior_id": behavior["id"],
                "proof": proof,
            }
    by_lane: dict[str, list[Mapping[str, Any]]] = {lane: [] for lane in expected_lanes}
    for shard in shards:
        if not isinstance(shard, Mapping):
            errors.append("evidence shard must be an object")
            continue
        lane = shard.get("lane")
        if not isinstance(lane, str) or lane not in expected_lanes:
            errors.append(f"unexpected evidence lane {lane!r}")
            continue
        by_lane[lane].append(shard)
    for lane, lane_shards in sorted(by_lane.items()):
        if len(lane_shards) != 1:
            errors.append(f"lane {lane} has {len(lane_shards)} shards; expected exactly one")

    observed_proofs: dict[str, str] = {}
    inventory_by_lane: dict[str, list[str]] = {}
    shard_keys = {
        "schema_version",
        "kind",
        "catalog_sha256",
        "repository",
        "sha",
        "run_id",
        "run_attempt",
        "lane",
        "operating_system",
        "status",
        "started_at",
        "finished_at",
        "tool_versions",
        "proofs",
        "inventory_cases",
        "errors",
    }
    for lane, lane_shards in sorted(by_lane.items()):
        if len(lane_shards) != 1:
            continue
        shard = lane_shards[0]
        if set(shard) != shard_keys:
            errors.append(
                f"lane {lane} shard fields differ: "
                f"expected={sorted(shard_keys)}, observed={sorted(shard)}"
            )
        expected_fields = {
            "schema_version": SCHEMA_VERSION,
            "kind": EVIDENCE_KIND,
            "catalog_sha256": digest,
            "repository": expected_repository,
            "sha": expected_sha,
            "run_id": expected_run_id,
            "lane": lane,
        }
        for field, expected in expected_fields.items():
            if shard.get(field) != expected:
                errors.append(
                    f"lane {lane} field {field} is {shard.get(field)!r}; expected {expected!r}"
                )
        if not is_json_integer(shard.get("schema_version")):
            errors.append(f"lane {lane} schema_version must be an integer")
        shard_attempt = shard.get("run_attempt")
        if not is_json_integer(shard_attempt) or shard_attempt < 1:
            errors.append(
                f"lane {lane} run_attempt must be a positive integer, got {shard_attempt!r}"
            )
        elif expected_attempt_is_valid and shard_attempt > expected_run_attempt:
            errors.append(
                f"lane {lane} run_attempt {shard_attempt} is newer than "
                f"current attempt {expected_run_attempt}"
            )
        if shard.get("operating_system") not in catalog["lanes"][lane]["operating_systems"]:
            errors.append(
                f"lane {lane} operating system {shard.get('operating_system')!r} is not allowed"
            )
        if not isinstance(shard.get("started_at"), str) or not isinstance(
            shard.get("finished_at"), str
        ):
            errors.append(f"lane {lane} timestamps must be strings")
        tools = shard.get("tool_versions")
        if not isinstance(tools, dict) or not all(
            isinstance(key, str) and isinstance(value, str) for key, value in tools.items()
        ):
            errors.append(f"lane {lane} tool_versions must map strings to strings")
        shard_errors = shard.get("errors")
        if not isinstance(shard_errors, list) or not all(
            isinstance(error, str) for error in shard_errors
        ):
            errors.append(f"lane {lane} errors must be an array of strings")
        elif shard.get("status") == "passed" and shard_errors:
            errors.append(f"lane {lane} claims passed with recorded errors")
        if shard.get("status") != "passed":
            errors.append(f"lane {lane} status is {shard.get('status')!r}")
        proofs = shard.get("proofs")
        if not isinstance(proofs, list):
            errors.append(f"lane {lane} proofs must be an array")
            proofs = []
        for proof in proofs:
            if not isinstance(proof, dict) or not isinstance(proof.get("proof_id"), str):
                errors.append(f"lane {lane} contains malformed proof evidence")
                continue
            proof_id = proof["proof_id"]
            if proof_id in observed_proofs:
                errors.append(f"duplicate proof evidence {proof_id}")
            expected = expected_proofs.get(proof_id)
            if expected is None:
                errors.append(f"unexpected proof evidence {proof_id}")
            else:
                if expected["lane"] != lane:
                    errors.append(
                        f"proof {proof_id} appeared in lane {lane}; "
                        f"expected {expected['lane']}"
                    )
                operating_system = shard.get("operating_system")
                if operating_system not in expected["proof"]["operating_systems"]:
                    errors.append(
                        f"proof {proof_id} does not support evidence operating system "
                        f"{operating_system!r}"
                    )
                errors.extend(
                    proof_evidence_errors(
                        proof,
                        lane_id=lane,
                        behavior_id=expected["behavior_id"],
                        proof=expected["proof"],
                        lane=catalog["lanes"][lane],
                    )
                )
            status = proof.get("status")
            if not isinstance(status, str) or status not in ALLOWED_PROOF_STATUSES:
                errors.append(f"proof {proof_id} has invalid status {status!r}")
                status = "error"
            observed_proofs[proof_id] = status
        cases = shard.get("inventory_cases")
        if not isinstance(cases, list):
            errors.append(f"lane {lane} inventory_cases must be an array")
            cases = []
        case_names: list[str] = []
        for case in cases:
            if not isinstance(case, dict) or set(case) != {"scenario", "status"}:
                errors.append(f"lane {lane} contains malformed inventory case")
                continue
            if not isinstance(case["scenario"], str):
                errors.append(f"lane {lane} contains a non-string inventory scenario")
                continue
            if case["status"] != "passed":
                errors.append(f"inventory scenario {case['scenario']!r} is {case['status']!r}")
            case_names.append(case["scenario"])
        if len(case_names) != len(set(case_names)):
            errors.append(f"lane {lane} contains duplicate inventory cases")
        inventory_by_lane[lane] = case_names

    missing = set(expected_proofs) - set(observed_proofs)
    unexpected = set(observed_proofs) - set(expected_proofs)
    if missing:
        errors.append(f"missing proof evidence: {sorted(missing)}")
    if unexpected:
        errors.append(f"unexpected proof evidence: {sorted(unexpected)}")
    for proof_id in sorted(set(expected_proofs) & set(observed_proofs)):
        if observed_proofs[proof_id] != "passed":
            errors.append(f"required proof {proof_id} is {observed_proofs[proof_id]}")

    for lane_id, lane in sorted(catalog["lanes"].items()):
        if lane["runner"] != "gui-semantic-suite":
            if inventory_by_lane.get(lane_id):
                errors.append(f"non-semantic lane {lane_id} reported inventory cases")
            continue
        expected_cases = lane["semantic_suite"]["scenarios"]
        observed_cases = inventory_by_lane.get(lane_id, [])
        if set(observed_cases) != set(expected_cases) or len(observed_cases) != len(expected_cases):
            errors.append(
                f"lane {lane_id} semantic inventory differs: "
                f"expected={expected_cases!r}, observed={observed_cases!r}"
            )

    behavior_status: list[dict[str, Any]] = []
    for behavior in sorted(catalog["behavior"], key=lambda item: item["id"]):
        statuses = [observed_proofs.get(proof["id"], "missing") for proof in behavior["proof"]]
        behavior_status.append(
            {
                "behavior_id": behavior["id"],
                "risk": behavior["risk"],
                "status": "passed" if statuses and all(item == "passed" for item in statuses) else "failed",
                "proofs": [proof["id"] for proof in behavior["proof"]],
            }
        )
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": AGGREGATE_KIND,
        "catalog_sha256": digest,
        "repository": expected_repository,
        "sha": expected_sha,
        "run_id": expected_run_id,
        "run_attempt": expected_run_attempt,
        "status": "passed" if not errors else "failed",
        "behaviors": behavior_status,
        "lanes": sorted(expected_lanes),
        "job_results": dict(sorted(job_results.items())),
        "errors": sorted(set(errors)),
    }


def parse_job_results(values: Iterable[str]) -> dict[str, str]:
    values = list(values)
    if not values:
        raise EvidenceError("at least one --job-result is required")
    results: dict[str, str] = {}
    for value in values:
        if "=" not in value:
            raise EvidenceError(f"job result must be NAME=CONCLUSION: {value!r}")
        name, conclusion = value.split("=", 1)
        if not IDENTIFIER.fullmatch(name) or conclusion not in {
            "success",
            "failure",
            "cancelled",
            "skipped",
        }:
            raise EvidenceError(f"invalid job result {value!r}")
        if name in results:
            raise EvidenceError(f"duplicate job result {name}")
        results[name] = conclusion
    return results


def aggregate_command(args: argparse.Namespace) -> int:
    repo_root = pathlib.Path(args.repo_root).resolve()
    catalog_path = pathlib.Path(args.catalog).resolve()
    output = pathlib.Path(args.output).resolve()
    try:
        validate_metadata(
            args.expected_sha,
            args.expected_repository,
            args.expected_run_id,
            args.expected_run_attempt,
            "linux",
        )
        verify_git_head(repo_root, args.expected_sha)
        verify_clean_worktree(repo_root)
        catalog = validate_catalog(load_catalog(catalog_path), repo_root=repo_root)
        job_results = parse_job_results(args.job_result)
        shards = [read_evidence(pathlib.Path(value).resolve()) for value in args.input]
        aggregate = aggregate_evidence(
            catalog,
            shards,
            digest=catalog_digest(catalog_path),
            expected_sha=args.expected_sha,
            expected_repository=args.expected_repository,
            expected_run_id=args.expected_run_id,
            expected_run_attempt=args.expected_run_attempt,
            job_results=job_results,
        )
    except Exception as error:
        try:
            failed_digest = (
                catalog_digest(catalog_path) if catalog_path.is_file() else "unavailable"
            )
        except OSError:
            failed_digest = "unavailable"
        aggregate = {
            "schema_version": SCHEMA_VERSION,
            "kind": AGGREGATE_KIND,
            "catalog_sha256": failed_digest,
            "repository": args.expected_repository,
            "sha": args.expected_sha,
            "run_id": args.expected_run_id,
            "run_attempt": args.expected_run_attempt,
            "status": "error",
            "behaviors": [],
            "lanes": [],
            "job_results": {},
            "errors": [str(error)],
        }
    atomic_write_json(output, aggregate)
    print(json.dumps(aggregate, indent=2, sort_keys=True))
    return 0 if aggregate["status"] == "passed" else 1


def validate_command(args: argparse.Namespace) -> int:
    catalog_path = pathlib.Path(args.catalog).resolve()
    repo_root = pathlib.Path(args.repo_root).resolve()
    catalog = validate_catalog(load_catalog(catalog_path), repo_root=repo_root)
    proof_count = sum(len(behavior["proof"]) for behavior in catalog["behavior"])
    print(
        f"valid behavior catalog: {len(catalog['behavior'])} behaviors, "
        f"{proof_count} proofs, {len(catalog['lanes'])} lanes, "
        f"{catalog_digest(catalog_path)}"
    )
    return 0


def build_parser() -> argparse.ArgumentParser:
    repo_root = pathlib.Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    validate = subparsers.add_parser("validate", help="validate catalog structure and sources")
    validate.add_argument("--catalog", default=str(repo_root / "coverage" / "behaviors.toml"))
    validate.add_argument("--repo-root", default=str(repo_root))
    validate.set_defaults(handler=validate_command)

    lane = subparsers.add_parser("run-lane", help="run every proof in one catalog lane")
    lane.add_argument("--catalog", default=str(repo_root / "coverage" / "behaviors.toml"))
    lane.add_argument("--repo-root", default=str(repo_root))
    lane.add_argument("--lane", required=True)
    lane.add_argument("--sha", required=True)
    lane.add_argument("--repository", required=True)
    lane.add_argument("--run-id", required=True)
    lane.add_argument("--run-attempt", required=True, type=int)
    lane.add_argument("--os", required=True)
    lane.add_argument("--output", required=True)
    lane.set_defaults(handler=run_lane)

    aggregate = subparsers.add_parser("aggregate", help="combine exact lane evidence")
    aggregate.add_argument("--catalog", default=str(repo_root / "coverage" / "behaviors.toml"))
    aggregate.add_argument("--repo-root", default=str(repo_root))
    aggregate.add_argument("--expected-sha", required=True)
    aggregate.add_argument("--expected-repository", required=True)
    aggregate.add_argument("--expected-run-id", required=True)
    aggregate.add_argument("--expected-run-attempt", required=True, type=int)
    aggregate.add_argument("--job-result", action="append", required=True)
    aggregate.add_argument("--input", action="append", required=True)
    aggregate.add_argument("--output", required=True)
    aggregate.set_defaults(handler=aggregate_command)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.handler(args)
    except CatalogError as error:
        print(f"catalog error: {error}", file=sys.stderr)
        return 2
    except EvidenceError as error:
        print(f"evidence error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
