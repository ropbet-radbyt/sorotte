#!/usr/bin/env python3
"""Bundle immutable release candidates and attest playback lifecycle suites."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any, Mapping, Sequence

import playback_lifecycle_model as lifecycle_model


SCHEMA_VERSION = 1
BUNDLE_KIND = "sorotte-playback-release-candidate-bundle"
PLATFORM_KIND = "sorotte-playback-release-platform-gate"
COMPLETE_KIND = "sorotte-playback-release-complete-gate"
SHA40 = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^[0-9a-f]{64}$")
SAFE_ROLE = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
PLATFORM_ROLES = {
    "linux-x86_64": frozenset({"server", "client"}),
    "windows-x86_64": frozenset({"server", "client", "gui", "gui-updater"}),
}
FAULT_ACTIONS = frozenset(
    {
        "backpressure",
        "channel-hold",
        "channel-release",
        "delay",
        "fragment",
        "half-close",
        "reset",
        "worker-stall",
    }
)
BASE_CHECKS = frozenset(
    {
        "prerequisites-attested",
        "room-switch-rejoin-preserved-authority",
        "canonical-playlist-reject-preserved-authority",
        "initial-two-clients-loaded-paused",
        "play-committed-and-applied",
        "pause-committed-and-applied",
        "seek-committed-and-applied",
        "scheduled-half-close-reconnected",
        "late-joiner-caught-up",
        "participant-status-snapshot-and-cadence",
        "participant-status-single-loss-self-healed",
        "participant-status-delayed-and-stale",
        "participant-status-fresh-recovery-advisory",
        "untrusted-selection-rejected-and-restored",
        "same-index-replacement-fresh-authority",
        "empty-playlist-clears-selected-media",
        "playlist-restore-reloads-selected-media",
        "partitioned-follower-caught-up",
        "post-reconnect-room-stable",
        "scheduled-write-failure-recovered",
        "natural-eof-advanced-once",
        "next-item-loaded-everywhere",
        "natural-eof-successor-authority-reset",
        "final-item-canonical-terminal-bounded",
        "no-contained-player-failures",
        "fault-schedule-replayed-completely",
        "shared-causal-ledger-validated",
        "server-drained-cleanly",
    }
)
LOOP_CHECKS = frozenset(
    {
        "loop-final-item-seek-committed-and-applied",
        "loop-final-item-resume-committed-and-applied",
        "loop-final-item-advanced-once",
        "loop-successor-loaded-everywhere",
        "loop-successor-authority-reset",
        "loop-successor-stable-through-client-exit",
    }
)
STATUS_CHECKS = frozenset(
    {
        "actual-release-server-listening",
        "actual-release-cli-connected",
        "real-supported-mpv-loaded-generated-media",
        "server-accepted-fresh-production-status",
        "exact-release-gui-observed-named-peer",
        "username-bound-native-status-node-fresh",
        "exact-gui-and-mpv-digests-attested",
        "native-status-screenshot-captured",
        "bounded-graceful-product-shutdown",
        "closed-product-lifecycle-ledger-validated",
    }
)
START_PHASES = (
    "inactive",
    "waitingForIntent",
    "waitingForTechnicalReadiness",
    "readyToCommit",
    "committed",
    "degraded",
)
START_TRANSITIONS = (
    "GATE-PREPARE-001",
    "GATE-PLAYABILITY-001",
    "GATE-READY-001",
    "GATE-COMMIT-001",
    "GATE-DEGRADE-001",
    "GATE-CLEAR-001",
)
START_SCENARIOS = (
    "late-join-slow-resolution",
    "partition-reconnect-before-commit",
    "reconnect-between-commit-and-started",
    "timeout-degraded-late-join",
    "sleep-resume-degraded-snapshot",
)
VERTICAL_MODES = {
    "baseline": None,
    "faulting-http": "http_fault_exercised",
    "stalled-http": "http_stall_exercised",
    "owned-process-recovery": "recovery_exercised",
}


class ReleaseGateError(ValueError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ReleaseGateError(f"{label} is not readable JSON") from error
    if not isinstance(value, dict):
        raise ReleaseGateError(f"{label} must be an object")
    return value


def atomic_json(path: Path, value: Mapping[str, Any]) -> None:
    if path.exists():
        raise ReleaseGateError(f"output must be create-new: {path.name}")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with temporary.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(value, output, sort_keys=True, ensure_ascii=False, indent=2)
        output.write("\n")
    os.replace(temporary, path)


def require_candidate_sha(value: str) -> str:
    lowered = value.lower()
    if SHA40.fullmatch(lowered) is None:
        raise ReleaseGateError("candidate SHA must be exactly 40 lowercase hexadecimal characters")
    return lowered


def require_platform(value: str) -> str:
    if value not in PLATFORM_ROLES:
        raise ReleaseGateError(f"unsupported release candidate platform: {value}")
    return value


def parse_assignments(values: Sequence[str], label: str) -> dict[str, Path]:
    parsed: dict[str, Path] = {}
    for value in values:
        role, separator, raw_path = value.partition("=")
        if not separator or SAFE_ROLE.fullmatch(role) is None or not raw_path:
            raise ReleaseGateError(f"{label} must use role=path")
        if role in parsed:
            raise ReleaseGateError(f"duplicate {label} role: {role}")
        parsed[role] = Path(raw_path).resolve()
    return parsed


def workspace_version(repo_root: Path) -> str:
    with (repo_root / "Cargo.toml").open("rb") as source:
        value = tomllib.load(source)["workspace"]["package"]["version"]
    if not isinstance(value, str) or not value:
        raise ReleaseGateError("workspace version is invalid")
    return value


def exact_clean_head(repo_root: Path, candidate_sha: str) -> None:
    head = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip().lower()
    if head != candidate_sha:
        raise ReleaseGateError("checked-out HEAD differs from the release candidate SHA")
    dirty = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all"],
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if dirty:
        raise ReleaseGateError("release candidate source is not clean")


def validate_bundle(value: Mapping[str, Any], directory: Path) -> dict[str, Any]:
    if set(value) != {
        "schema_version",
        "kind",
        "result",
        "candidate_sha",
        "platform",
        "product_version",
        "files",
    }:
        raise ReleaseGateError("candidate bundle manifest does not use its closed schema")
    if value.get("schema_version") != SCHEMA_VERSION or value.get("kind") != BUNDLE_KIND:
        raise ReleaseGateError("candidate bundle manifest has the wrong identity")
    if value.get("result") != "passed":
        raise ReleaseGateError("candidate bundle manifest is not a pass")
    candidate_sha = require_candidate_sha(str(value.get("candidate_sha", "")))
    platform = require_platform(str(value.get("platform", "")))
    files = value.get("files")
    if not isinstance(files, dict) or set(files) != PLATFORM_ROLES[platform]:
        raise ReleaseGateError("candidate bundle role inventory is incomplete")
    expected_names: set[str] = {"candidate-manifest.json"}
    for role, identity in files.items():
        if not isinstance(identity, dict) or set(identity) != {"file_name", "size", "sha256"}:
            raise ReleaseGateError(f"candidate identity for {role} is malformed")
        file_name = identity.get("file_name")
        size = identity.get("size")
        digest = identity.get("sha256")
        if not isinstance(file_name, str) or Path(file_name).name != file_name:
            raise ReleaseGateError(f"candidate file name for {role} is unsafe")
        if file_name in expected_names:
            raise ReleaseGateError("candidate bundle contains duplicate file names")
        if not isinstance(size, int) or isinstance(size, bool) or size <= 0:
            raise ReleaseGateError(f"candidate size for {role} is invalid")
        if not isinstance(digest, str) or DIGEST.fullmatch(digest) is None:
            raise ReleaseGateError(f"candidate digest for {role} is invalid")
        path = directory / file_name
        if not path.is_file() or path.is_symlink():
            raise ReleaseGateError(f"candidate file for {role} is missing or not regular")
        if path.stat().st_size != size or sha256_file(path) != digest:
            raise ReleaseGateError(f"candidate file for {role} differs from its manifest")
        expected_names.add(file_name)
    actual_names = {path.name for path in directory.iterdir()}
    if actual_names != expected_names:
        raise ReleaseGateError("candidate bundle directory inventory is not closed")
    result = dict(value)
    result["candidate_sha"] = candidate_sha
    result["platform"] = platform
    return result


def bundle(args: argparse.Namespace) -> dict[str, Any]:
    repo_root = Path(args.repo_root).resolve()
    candidate_sha = require_candidate_sha(args.candidate_sha)
    platform = require_platform(args.platform)
    exact_clean_head(repo_root, candidate_sha)
    sources = parse_assignments(args.artifact, "artifact")
    if set(sources) != PLATFORM_ROLES[platform]:
        raise ReleaseGateError("bundle sources do not match the platform role inventory")
    output_dir = Path(args.output_dir).resolve()
    if output_dir.exists() and any(output_dir.iterdir()):
        raise ReleaseGateError("candidate bundle output directory must be empty")
    output_dir.mkdir(parents=True, exist_ok=True)
    identities: dict[str, dict[str, Any]] = {}
    used_names: set[str] = set()
    for role, source in sorted(sources.items()):
        if not source.is_file() or source.is_symlink():
            raise ReleaseGateError(f"candidate source for {role} is missing or not regular")
        name = source.name
        if name in used_names or name == "candidate-manifest.json":
            raise ReleaseGateError("candidate source file names collide")
        used_names.add(name)
        destination = output_dir / name
        with source.open("rb") as input_stream, destination.open("xb") as output_stream:
            shutil.copyfileobj(input_stream, output_stream, 1024 * 1024)
        identities[role] = {
            "file_name": name,
            "size": destination.stat().st_size,
            "sha256": sha256_file(destination),
        }
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "kind": BUNDLE_KIND,
        "result": "passed",
        "candidate_sha": candidate_sha,
        "platform": platform,
        "product_version": workspace_version(repo_root),
        "files": identities,
    }
    atomic_json(output_dir / "candidate-manifest.json", manifest)
    return validate_bundle(manifest, output_dir)


def read_bundle(directory: Path, candidate_sha: str, platform: str) -> dict[str, Any]:
    manifest = validate_bundle(
        load_json(directory / "candidate-manifest.json", "candidate manifest"), directory
    )
    if manifest["candidate_sha"] != require_candidate_sha(candidate_sha):
        raise ReleaseGateError("candidate manifest SHA does not match the requested SHA")
    if manifest["platform"] != require_platform(platform):
        raise ReleaseGateError("candidate manifest platform does not match")
    return manifest


def verify_files(bundle_manifest: Mapping[str, Any], assignments: Mapping[str, Path]) -> None:
    files = bundle_manifest["files"]
    if set(assignments) != set(files):
        raise ReleaseGateError("verification paths do not match the bundle role inventory")
    for role, path in assignments.items():
        identity = files[role]
        if not path.is_file() or path.is_symlink():
            raise ReleaseGateError(f"verification target for {role} is missing")
        if path.stat().st_size != identity["size"] or sha256_file(path) != identity["sha256"]:
            raise ReleaseGateError(f"verification target for {role} differs from tested candidate")


def checks_by_id(report: Mapping[str, Any]) -> dict[str, str]:
    checks = report.get("checks")
    if not isinstance(checks, list):
        raise ReleaseGateError("system report checks are missing")
    projected: dict[str, str] = {}
    for check in checks:
        if not isinstance(check, dict) or not isinstance(check.get("id"), str):
            raise ReleaseGateError("system report check is malformed")
        check_id = check["id"]
        if check_id in projected:
            raise ReleaseGateError("system report duplicated a check")
        status = check.get("status")
        if status not in {"passed", "not-applicable"}:
            raise ReleaseGateError(f"system report check {check_id} did not pass")
        projected[check_id] = status
    return projected


def lifecycle_transition_ids(summary: Any, label: str) -> set[str]:
    if (
        not isinstance(summary, dict)
        or summary.get("schema_version") != 1
        or summary.get("kind")
        != "sorotte-playback-lifecycle-evidence-validation"
        or summary.get("result") != "passed"
    ):
        raise ReleaseGateError(f"{label} lifecycle summary is not a validated pass")
    transitions = summary.get("transitions")
    if not isinstance(transitions, dict) or not transitions:
        raise ReleaseGateError(f"{label} lifecycle summary has no transition inventory")
    result: set[str] = set()
    for transition_id, count in transitions.items():
        if (
            not isinstance(transition_id, str)
            or not transition_id
            or not isinstance(count, int)
            or isinstance(count, bool)
            or count <= 0
        ):
            raise ReleaseGateError(
                f"{label} lifecycle summary transition inventory is malformed"
            )
        result.add(transition_id)
    return result


def validate_system_report(
    path: Path,
    *,
    bundle_manifest: Mapping[str, Any],
    loop: bool,
) -> dict[str, Any]:
    report = load_json(path, "playback lifecycle system report")
    if report.get("schema_version") != 1 or report.get("kind") != "sorotte-playback-lifecycle-system":
        raise ReleaseGateError("system report has the wrong identity")
    if report.get("result") != "passed" or report.get("candidate_sha") != bundle_manifest["candidate_sha"]:
        raise ReleaseGateError("system report is not bound to the passing candidate")
    attestation = report.get("prerequisites", {}).get("candidate_attestation", {})
    if not isinstance(attestation, dict) or attestation.get("verified") is not True:
        raise ReleaseGateError("system report did not verify its exact clean candidate")
    if attestation.get("checkout_sha") != bundle_manifest["candidate_sha"]:
        raise ReleaseGateError("system report checkout differs from candidate")
    prerequisites = report.get("prerequisites")
    if not isinstance(prerequisites, dict):
        raise ReleaseGateError("system prerequisites are missing")
    files = bundle_manifest["files"]
    if prerequisites.get("server", {}).get("sha256") != files["server"]["sha256"]:
        raise ReleaseGateError("system server digest differs from candidate bundle")
    if prerequisites.get("client", {}).get("sha256") != files["client"]["sha256"]:
        raise ReleaseGateError("system client digest differs from candidate bundle")
    required_checks = BASE_CHECKS | (LOOP_CHECKS if loop else frozenset())
    missing = required_checks - set(checks_by_id(report))
    if missing:
        raise ReleaseGateError(f"system report omitted required checks: {sorted(missing)}")
    if (report.get("playlist_policy") == "loop-at-end") != loop:
        raise ReleaseGateError("system report playlist policy does not match its release lane")
    schedule = report.get("fault_schedule")
    if not isinstance(schedule, dict) or frozenset(schedule.get("actions", [])) != FAULT_ACTIONS:
        raise ReleaseGateError("system report fault action inventory is incomplete")
    if schedule.get("step_count") != 12:
        raise ReleaseGateError("system report did not replay the complete deterministic schedule")
    lifecycle_transition_ids(report.get("lifecycle_summary"), "system")
    return report


def validate_start_report(path: Path, bundle_manifest: Mapping[str, Any]) -> dict[str, Any]:
    report = load_json(path, "start-gate system report")
    if report.get("schema_version") != 1 or report.get("kind") != "sorotte-playback-start-gate-system":
        raise ReleaseGateError("start-gate report has the wrong identity")
    if (
        report.get("result") != "passed"
        or report.get("candidate_sha") != bundle_manifest["candidate_sha"]
        or report.get("candidate_binding") != "exact-clean-head"
    ):
        raise ReleaseGateError("start-gate report is not bound to the exact candidate")
    if report.get("server", {}).get("sha256") != bundle_manifest["files"]["server"]["sha256"]:
        raise ReleaseGateError("start-gate server digest differs from candidate bundle")
    if report.get("phase_coverage") != list(START_PHASES):
        raise ReleaseGateError("start-gate phase coverage is incomplete")
    if report.get("transition_coverage") != list(START_TRANSITIONS):
        raise ReleaseGateError("start-gate transition coverage is incomplete")
    if report.get("scenario_coverage") != list(START_SCENARIOS):
        raise ReleaseGateError("start-gate generated scenario coverage is incomplete")
    summary = report.get("lifecycle_summary")
    transitions = lifecycle_transition_ids(summary, "start-gate")
    if summary.get("cross_process_edge_count", 0) < len(START_TRANSITIONS):
        raise ReleaseGateError("start-gate causal lifecycle has too few cross-process edges")
    if not set(START_TRANSITIONS) <= transitions:
        raise ReleaseGateError("start-gate lifecycle summary omitted a required transition")
    return report


def closed_gap_ids(model_path: Path) -> list[str]:
    with model_path.open("rb") as source:
        model = tomllib.load(source)
    gaps = model.get("gap")
    if not isinstance(gaps, list) or not gaps:
        raise ReleaseGateError("lifecycle model has no gap registry")
    open_ids = [gap.get("id") for gap in gaps if gap.get("status") != "closed"]
    if open_ids:
        raise ReleaseGateError(f"lifecycle model retains open gaps: {open_ids}")
    return [str(gap["id"]) for gap in gaps]


def model_transition_inventory(model_path: Path) -> tuple[set[str], set[str]]:
    with model_path.open("rb") as source:
        model = tomllib.load(source)
    machines = model.get("machine")
    if not isinstance(machines, list) or not machines:
        raise ReleaseGateError("lifecycle model has no machine registry")
    all_transitions: set[str] = set()
    required_system: set[str] = set()
    for machine in machines:
        if not isinstance(machine, dict):
            raise ReleaseGateError("lifecycle model contains a malformed machine")
        transitions = machine.get("transition", [])
        if not isinstance(transitions, list):
            raise ReleaseGateError("lifecycle model contains a malformed transition registry")
        for transition in transitions:
            if not isinstance(transition, dict) or not isinstance(
                transition.get("id"), str
            ):
                raise ReleaseGateError("lifecycle model contains a malformed transition")
            transition_id = transition["id"]
            if transition_id in all_transitions:
                raise ReleaseGateError("lifecycle model contains a duplicate transition")
            all_transitions.add(transition_id)
            required_tiers = transition.get("required_tiers")
            covered_tiers = transition.get("covered_tiers")
            if not isinstance(required_tiers, list) or not isinstance(
                covered_tiers, list
            ):
                raise ReleaseGateError("lifecycle model transition tiers are malformed")
            if "system" in required_tiers:
                required_system.add(transition_id)
    if not required_system:
        raise ReleaseGateError("lifecycle model requires no system transitions")
    return all_transitions, required_system


def model_system_suite_inventory(
    model_path: Path,
    *,
    all_transitions: set[str],
    required_system: set[str],
) -> tuple[Path, dict[str, dict[str, Any]]]:
    with model_path.open("rb") as source:
        model = tomllib.load(source)
    raw_path = model.get("system_coverage")
    if not isinstance(raw_path, str) or not raw_path:
        raise ReleaseGateError("lifecycle model has no system coverage registry")
    relative = Path(raw_path)
    if relative.is_absolute() or ".." in relative.parts:
        raise ReleaseGateError("lifecycle system coverage path is unsafe")
    repo_root = model_path.parent.parent.resolve()
    registry_path = (repo_root / relative).resolve()
    try:
        registry_path.relative_to(repo_root)
    except ValueError as error:
        raise ReleaseGateError("lifecycle system coverage path escapes the repository") from error
    try:
        with registry_path.open("rb") as source:
            raw_registry = tomllib.load(source)
        suites_by_id = lifecycle_model.system_coverage_suites(
            raw_registry, expected_model_id=str(model.get("model_id", ""))
        )
    except (OSError, tomllib.TOMLDecodeError, lifecycle_model.ModelError) as error:
        raise ReleaseGateError(f"lifecycle system coverage registry is invalid: {error}") from error
    suites_by_source: dict[str, dict[str, Any]] = {}
    for suite_id, suite in suites_by_id.items():
        source = suite["source"]
        suites_by_source[source] = {"id": suite_id, **suite}
    assigned = {
        transition_id
        for suite in suites_by_source.values()
        for transition_id in suite["transitions"]
    }
    unknown = assigned - all_transitions
    if unknown:
        raise ReleaseGateError(
            f"lifecycle system coverage registry has unknown transitions: {sorted(unknown)}"
        )
    if assigned != required_system:
        raise ReleaseGateError(
            "lifecycle system coverage registry does not exactly cover required transitions: "
            f"missing={sorted(required_system - assigned)} "
            f"unexpected={sorted(assigned - required_system)}"
        )
    return registry_path, suites_by_source


def require_suite_coverage(
    source: str,
    summary: Mapping[str, Any],
    *,
    platform: str,
    suites: Mapping[str, Mapping[str, Any]],
) -> set[str]:
    suite = suites.get(source)
    if suite is None or suite.get("platform") != platform:
        raise ReleaseGateError(f"system coverage registry has no {platform} source {source}")
    observed = lifecycle_transition_ids(summary, source)
    required = set(suite["transitions"])
    missing = required - observed
    if missing:
        raise ReleaseGateError(
            f"{source} lifecycle summary omitted assigned transitions: {sorted(missing)}"
        )
    return required


def attest_linux(args: argparse.Namespace) -> dict[str, Any]:
    bundle_dir = Path(args.bundle_dir).resolve()
    manifest = read_bundle(bundle_dir, args.candidate_sha, "linux-x86_64")
    base_path = Path(args.system_report).resolve()
    loop_path = Path(args.loop_report).resolve()
    start_path = Path(args.start_report).resolve()
    base = validate_system_report(base_path, bundle_manifest=manifest, loop=False)
    loop = validate_system_report(loop_path, bundle_manifest=manifest, loop=True)
    start = validate_start_report(start_path, manifest)
    model_path = Path(args.model).resolve()
    gaps = closed_gap_ids(model_path)
    all_transitions, required_system = model_transition_inventory(model_path)
    system_registry_path, system_suites = model_system_suite_inventory(
        model_path,
        all_transitions=all_transitions,
        required_system=required_system,
    )
    system_coverage: set[str] = set()
    for source, summary in (
        ("ordinary-terminal-system", base["lifecycle_summary"]),
        ("loop-system", loop["lifecycle_summary"]),
        ("start-gate-system", start["lifecycle_summary"]),
    ):
        system_coverage.update(
            require_suite_coverage(
                source,
                summary,
                platform="linux-x86_64",
                suites=system_suites,
            )
        )
    report = {
        "schema_version": SCHEMA_VERSION,
        "kind": PLATFORM_KIND,
        "result": "passed",
        "candidate_sha": manifest["candidate_sha"],
        "platform": "linux-x86_64",
        "candidate_manifest_sha256": sha256_file(bundle_dir / "candidate-manifest.json"),
        "candidate_files": manifest["files"],
        "closed_gaps": gaps,
        "model_sha256": sha256_file(model_path),
        "system_coverage_sha256": sha256_file(system_registry_path),
        "required_system_transitions": sorted(required_system),
        "system_transition_coverage": sorted(system_coverage),
        "suite_reports": {
            "ordinary-terminal-system": sha256_file(base_path),
            "loop-system": sha256_file(loop_path),
            "start-gate-system": sha256_file(start_path),
        },
        "claims": [
            "actual-server-three-packaged-clients-real-players",
            "deterministic-cross-boundary-fault-replay",
            "ordinary-loop-and-final-playlist-boundaries",
            "all-start-gate-phases-and-recovery-scenarios",
            "closed-cross-process-causal-evidence",
            "exact-clean-candidate-digests",
        ],
    }
    atomic_json(Path(args.output).resolve(), report)
    return report


def validate_status_report(
    path: Path, bundle_manifest: Mapping[str, Any], mpv_digest: str
) -> dict[str, Any]:
    report = load_json(path, "participant status system report")
    if report.get("schema_version") != 1 or report.get("kind") != "sorotte-playback-status-system":
        raise ReleaseGateError("status system report has the wrong identity")
    if report.get("result") != "passed" or report.get("candidate_sha") != bundle_manifest["candidate_sha"]:
        raise ReleaseGateError("status system report is not bound to the candidate")
    checks = report.get("checks")
    if not isinstance(checks, list) or set(checks) != STATUS_CHECKS:
        raise ReleaseGateError("status system report check inventory is incomplete")
    prerequisites = report.get("prerequisites")
    if not isinstance(prerequisites, dict):
        raise ReleaseGateError("status prerequisites are missing")
    for role in ("server", "client", "gui"):
        if prerequisites.get(role, {}).get("sha256") != bundle_manifest["files"][role]["sha256"]:
            raise ReleaseGateError(f"status {role} digest differs from candidate bundle")
    if prerequisites.get("mpv", {}).get("sha256") != mpv_digest:
        raise ReleaseGateError("status mpv digest differs from pinned supported player")
    projection = report.get("projection")
    if (
        not isinstance(projection, dict)
        or projection.get("visible") is not True
        or not isinstance(projection.get("status_label"), str)
        or not projection["status_label"].endswith("fresh")
    ):
        raise ReleaseGateError("status system did not prove a visible fresh named-row projection")
    summary = report.get("lifecycle_summary")
    lifecycle_transition_ids(summary, "status product")
    return report


def parse_named_paths(values: Sequence[str], expected: set[str]) -> dict[str, Path]:
    parsed = parse_assignments(values, "summary")
    if set(parsed) != expected:
        raise ReleaseGateError("named summary inventory is incomplete")
    return parsed


def attest_windows(args: argparse.Namespace) -> dict[str, Any]:
    bundle_dir = Path(args.bundle_dir).resolve()
    manifest = read_bundle(bundle_dir, args.candidate_sha, "windows-x86_64")
    mpv_path = Path(args.mpv).resolve()
    ffmpeg_path = Path(args.ffmpeg).resolve()
    mpv_digest = sha256_file(mpv_path)
    ffmpeg_digest = sha256_file(ffmpeg_path)
    if mpv_digest != args.expected_mpv_sha256:
        raise ReleaseGateError("mpv binary digest differs from the pinned release identity")
    if ffmpeg_digest != args.expected_ffmpeg_sha256:
        raise ReleaseGateError("FFmpeg binary digest differs from the pinned release identity")
    status_path = Path(args.status_report).resolve()
    status = validate_status_report(status_path, manifest, mpv_digest)
    summaries = parse_named_paths(args.vertical_summary, set(VERTICAL_MODES))
    summary_digests: dict[str, str] = {}
    lifecycle_by_source: dict[str, Mapping[str, Any]] = {
        "participant-status-system": status["lifecycle_summary"]
    }
    for mode, path in sorted(summaries.items()):
        summary = load_json(path, f"{mode} exact-GUI summary")
        if (
            summary.get("schema_version") != 1
            or summary.get("kind") != "sorotte-gui-real-mpv-vertical-contract"
            or summary.get("result") != "passed"
            or summary.get("capability") != "executed"
        ):
            raise ReleaseGateError(f"{mode} exact-GUI summary is not an executed pass")
        if summary.get("gui_sha256") != manifest["files"]["gui"]["sha256"]:
            raise ReleaseGateError(f"{mode} GUI digest differs from candidate bundle")
        if summary.get("mpv_sha256") != mpv_digest:
            raise ReleaseGateError(f"{mode} mpv digest differs from pinned player")
        required_flag = VERTICAL_MODES[mode]
        if required_flag is not None and summary.get(required_flag) is not True:
            raise ReleaseGateError(f"{mode} exact-GUI summary omitted its exercised fault")
        minimum_assertions = 13 if mode == "baseline" else (20 if mode == "owned-process-recovery" else 18)
        if summary.get("assertion_count", 0) < minimum_assertions:
            raise ReleaseGateError(f"{mode} exact-GUI summary has too few assertions")
        lifecycle_path = path.with_name("shared-lifecycle-summary.json")
        lifecycle_summary = load_json(
            lifecycle_path, f"{mode} exact-GUI lifecycle summary"
        )
        transition_ids = lifecycle_transition_ids(
            lifecycle_summary, f"{mode} exact-GUI"
        )
        if summary.get("lifecycle_summary_sha256") != sha256_file(lifecycle_path):
            raise ReleaseGateError(
                f"{mode} exact-GUI lifecycle summary digest is not bound"
            )
        if summary.get("lifecycle_transition_coverage") != sorted(transition_ids):
            raise ReleaseGateError(
                f"{mode} exact-GUI lifecycle transition inventory is not bound"
            )
        lifecycle_by_source[f"exact-gui-{mode}"] = lifecycle_summary
        summary_digests[mode] = sha256_file(path)
    model_path = Path(args.model).resolve()
    gaps = closed_gap_ids(model_path)
    all_transitions, required_system = model_transition_inventory(model_path)
    system_registry_path, system_suites = model_system_suite_inventory(
        model_path,
        all_transitions=all_transitions,
        required_system=required_system,
    )
    system_coverage: set[str] = set()
    for source, lifecycle_summary in lifecycle_by_source.items():
        system_coverage.update(
            require_suite_coverage(
                source,
                lifecycle_summary,
                platform="windows-x86_64",
                suites=system_suites,
            )
        )
    report = {
        "schema_version": SCHEMA_VERSION,
        "kind": PLATFORM_KIND,
        "result": "passed",
        "candidate_sha": manifest["candidate_sha"],
        "platform": "windows-x86_64",
        "candidate_manifest_sha256": sha256_file(bundle_dir / "candidate-manifest.json"),
        "candidate_files": manifest["files"],
        "closed_gaps": gaps,
        "model_sha256": sha256_file(model_path),
        "system_coverage_sha256": sha256_file(system_registry_path),
        "required_system_transitions": sorted(required_system),
        "system_transition_coverage": sorted(system_coverage),
        "tool_digests": {"mpv": mpv_digest, "ffmpeg": ffmpeg_digest},
        "suite_reports": {
            "participant-status-system": sha256_file(status_path),
            **{f"exact-gui-{mode}": digest for mode, digest in summary_digests.items()},
        },
        "claims": [
            "exact-packaged-gui-real-player-baseline",
            "cache-stall-and-transport-failure-terminal-schedules",
            "owned-player-process-and-ipc-recovery",
            "second-client-native-participant-status-projection",
            "closed-product-role-causal-evidence",
            "exact-clean-candidate-digests",
        ],
    }
    atomic_json(Path(args.output).resolve(), report)
    return report


def validate_platform_gate(
    path: Path, *, candidate_sha: str, platform: str
) -> dict[str, Any]:
    report = load_json(path, f"{platform} platform gate")
    required = {
        "schema_version",
        "kind",
        "result",
        "candidate_sha",
        "platform",
        "candidate_manifest_sha256",
        "candidate_files",
        "closed_gaps",
        "model_sha256",
        "system_coverage_sha256",
        "required_system_transitions",
        "system_transition_coverage",
        "suite_reports",
        "claims",
    }
    if platform == "windows-x86_64":
        required.add("tool_digests")
    if set(report) != required:
        raise ReleaseGateError(f"{platform} gate does not use its closed schema")
    if (
        report.get("schema_version") != SCHEMA_VERSION
        or report.get("kind") != PLATFORM_KIND
        or report.get("result") != "passed"
        or report.get("candidate_sha") != candidate_sha
        or report.get("platform") != platform
    ):
        raise ReleaseGateError(f"{platform} gate is not an exact passing candidate attestation")
    if not isinstance(report.get("closed_gaps"), list) or not report["closed_gaps"]:
        raise ReleaseGateError(f"{platform} gate has no closed gap inventory")
    if not isinstance(report.get("suite_reports"), dict) or not report["suite_reports"]:
        raise ReleaseGateError(f"{platform} gate has no suite report inventory")
    if not isinstance(report.get("model_sha256"), str) or DIGEST.fullmatch(
        report["model_sha256"]
    ) is None:
        raise ReleaseGateError(f"{platform} gate has no lifecycle model identity")
    if not isinstance(report.get("system_coverage_sha256"), str) or DIGEST.fullmatch(
        report["system_coverage_sha256"]
    ) is None:
        raise ReleaseGateError(
            f"{platform} gate has no lifecycle system coverage identity"
        )
    required_system = report.get("required_system_transitions")
    coverage = report.get("system_transition_coverage")
    if (
        not isinstance(required_system, list)
        or not required_system
        or not all(isinstance(item, str) and item for item in required_system)
        or required_system != sorted(set(required_system))
    ):
        raise ReleaseGateError(f"{platform} gate has an invalid required transition inventory")
    if (
        not isinstance(coverage, list)
        or not all(isinstance(item, str) and item for item in coverage)
        or coverage != sorted(set(coverage))
        or not set(coverage) <= set(required_system)
    ):
        raise ReleaseGateError(f"{platform} gate has invalid system transition coverage")
    return report


def attest_complete(args: argparse.Namespace) -> dict[str, Any]:
    candidate_sha = require_candidate_sha(args.candidate_sha)
    model_path = Path(args.model).resolve()
    all_transitions, required_from_model = model_transition_inventory(model_path)
    system_registry_path, _ = model_system_suite_inventory(
        model_path,
        all_transitions=all_transitions,
        required_system=required_from_model,
    )
    linux_path = Path(args.linux_gate).resolve()
    windows_path = Path(args.windows_gate).resolve()
    linux = validate_platform_gate(
        linux_path, candidate_sha=candidate_sha, platform="linux-x86_64"
    )
    windows = validate_platform_gate(
        windows_path, candidate_sha=candidate_sha, platform="windows-x86_64"
    )
    if linux["closed_gaps"] != windows["closed_gaps"]:
        raise ReleaseGateError("platform gates disagree on the closed gap inventory")
    if linux["model_sha256"] != sha256_file(model_path):
        raise ReleaseGateError("platform lifecycle model identity differs from checkout")
    if linux["system_coverage_sha256"] != sha256_file(system_registry_path):
        raise ReleaseGateError(
            "platform lifecycle system coverage identity differs from checkout"
        )
    if linux["model_sha256"] != windows["model_sha256"]:
        raise ReleaseGateError("platform gates disagree on the lifecycle model identity")
    if linux["system_coverage_sha256"] != windows["system_coverage_sha256"]:
        raise ReleaseGateError(
            "platform gates disagree on the lifecycle system coverage identity"
        )
    if linux["required_system_transitions"] != windows["required_system_transitions"]:
        raise ReleaseGateError("platform gates disagree on required system transitions")
    if set(linux["required_system_transitions"]) != required_from_model:
        raise ReleaseGateError(
            "platform required system transitions differ from the checked-out model"
        )
    required_system = set(linux["required_system_transitions"])
    observed_system = set(linux["system_transition_coverage"]) | set(
        windows["system_transition_coverage"]
    )
    if observed_system != required_system:
        raise ReleaseGateError(
            "platform lifecycle evidence does not cover every required system transition: "
            f"missing={sorted(required_system - observed_system)} "
            f"unexpected={sorted(observed_system - required_system)}"
        )
    report = {
        "schema_version": SCHEMA_VERSION,
        "kind": COMPLETE_KIND,
        "result": "passed",
        "candidate_sha": candidate_sha,
        "closed_gaps": linux["closed_gaps"],
        "model_sha256": linux["model_sha256"],
        "system_coverage_sha256": linux["system_coverage_sha256"],
        "required_system_transitions": sorted(required_system),
        "system_transition_coverage": sorted(observed_system),
        "platform_gate_sha256": {
            "linux-x86_64": sha256_file(linux_path),
            "windows-x86_64": sha256_file(windows_path),
        },
        "candidate_manifest_sha256": {
            "linux-x86_64": linux["candidate_manifest_sha256"],
            "windows-x86_64": windows["candidate_manifest_sha256"],
        },
        "claims": [
            "all-declared-lifecycle-gaps-closed",
            "linux-and-windows-candidate-artifacts-consumed",
            "published-binaries-must-match-tested-manifests",
        ],
    }
    atomic_json(Path(args.output).resolve(), report)
    return report


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    bundle_parser = subcommands.add_parser("bundle")
    bundle_parser.add_argument("--repo-root", default=Path(__file__).resolve().parents[1])
    bundle_parser.add_argument("--candidate-sha", required=True)
    bundle_parser.add_argument("--platform", required=True, choices=sorted(PLATFORM_ROLES))
    bundle_parser.add_argument("--artifact", action="append", required=True)
    bundle_parser.add_argument("--output-dir", required=True)

    verify = subcommands.add_parser("verify-bundle")
    verify.add_argument("--bundle-dir", required=True)
    verify.add_argument("--candidate-sha", required=True)
    verify.add_argument("--platform", required=True, choices=sorted(PLATFORM_ROLES))
    verify.add_argument("--artifact", action="append", default=[])

    linux = subcommands.add_parser("attest-linux")
    linux.add_argument("--bundle-dir", required=True)
    linux.add_argument("--candidate-sha", required=True)
    linux.add_argument("--system-report", required=True)
    linux.add_argument("--loop-report", required=True)
    linux.add_argument("--start-report", required=True)
    linux.add_argument("--model", required=True)
    linux.add_argument("--output", required=True)

    windows = subcommands.add_parser("attest-windows")
    windows.add_argument("--bundle-dir", required=True)
    windows.add_argument("--candidate-sha", required=True)
    windows.add_argument("--status-report", required=True)
    windows.add_argument("--vertical-summary", action="append", required=True)
    windows.add_argument("--mpv", required=True)
    windows.add_argument("--ffmpeg", required=True)
    windows.add_argument("--expected-mpv-sha256", required=True)
    windows.add_argument("--expected-ffmpeg-sha256", required=True)
    windows.add_argument("--model", required=True)
    windows.add_argument("--output", required=True)

    complete = subcommands.add_parser("attest-complete")
    complete.add_argument("--candidate-sha", required=True)
    complete.add_argument("--linux-gate", required=True)
    complete.add_argument("--windows-gate", required=True)
    complete.add_argument("--model", required=True)
    complete.add_argument("--output", required=True)
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        if args.command == "bundle":
            report = bundle(args)
        elif args.command == "verify-bundle":
            manifest = read_bundle(
                Path(args.bundle_dir).resolve(), args.candidate_sha, args.platform
            )
            assignments = parse_assignments(args.artifact, "artifact")
            if assignments:
                verify_files(manifest, assignments)
            report = manifest
        elif args.command == "attest-linux":
            report = attest_linux(args)
        elif args.command == "attest-windows":
            report = attest_windows(args)
        else:
            report = attest_complete(args)
    except (ReleaseGateError, OSError, subprocess.SubprocessError, KeyError, TypeError) as error:
        print(f"playback release gate failed: {error}", file=sys.stderr)
        return 1
    print(
        json.dumps(
            {
                "kind": report["kind"],
                "result": report["result"],
                "candidate_sha": report["candidate_sha"],
                **({"platform": report["platform"]} if "platform" in report else {}),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
