#!/usr/bin/env python3
"""Create fail-closed, phase-aware evidence for changed-line coverage CI.

The base resolver deliberately implements the event contract instead of
guessing:

* pull requests use the single merge base of the PR base tip and verified head;
* branch pushes use exactly ``github.event.before``;
* updated tag pushes use exactly ``github.event.before``;
* newly created tags, whose ``before`` value is all zeroes, use the single
  merge base of the verified tag commit and the fetched remote default branch;
* manual runs require an explicit full base commit SHA.

Both commands write machine-readable JSON on success and policy failure.  The
``finalize`` command is intended to run under ``if: always()`` after the base,
profile, LLVM JSON, LLVM native text, canonical line-map, and diff-policy
phases were allowed to continue on error.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
from collections.abc import Mapping, Sequence
from typing import Any

import coverage_profile_lanes


BASE_SCHEMA_VERSION = 1
PHASE_SCHEMA_VERSION = 2
BASE_REPORT_KIND = "sorotte-coverage-base"
PHASE_REPORT_KIND = "sorotte-coverage-ci-evidence"
DIFF_REPORT_KIND = "sorotte-diff-coverage"
LINE_MAP_REPORT_KIND = "sorotte-llvm-line-map"
FULL_SHA = re.compile(r"[0-9a-fA-F]{40}")
ZERO_SHA = "0" * 40
MAX_JSON_BYTES = 32 * 1024 * 1024
MAX_LLVM_JSON_BYTES = 256 * 1024 * 1024
MAX_LLVM_TEXT_BYTES = 256 * 1024 * 1024
MAX_LINE_MAP_BYTES = 128 * 1024 * 1024
MAX_COVERAGE_MAPS = 8
KNOWN_OUTCOMES = {"success", "failure", "skipped", "cancelled"}
PINNED_LINE_MAP_PRODUCER = {
    "llvm_export_type": "llvm.coverage.json.export",
    "llvm_export_version": "3.1.0",
    "cargo_llvm_cov_version": "0.8.4",
    "manifest_path": "Cargo.toml",
}


class CoverageCiGuardError(Exception):
    """An expected fail-closed coverage evidence error."""


def atomic_write_json(path: pathlib.Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(value, indent=2, sort_keys=True) + "\n"
    temporary: pathlib.Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            dir=path.parent,
            prefix=f".{path.name}.",
            suffix=".tmp",
            delete=False,
        ) as handle:
            handle.write(payload)
            temporary = pathlib.Path(handle.name)
        os.replace(temporary, path)
    except OSError as error:
        raise CoverageCiGuardError(
            f"cannot write JSON report {path}: {error}"
        ) from error
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


def run_git(repo_root: pathlib.Path, argv: Sequence[str], *, description: str) -> str:
    try:
        process = subprocess.run(
            ["git", "-C", str(repo_root), *argv],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="strict",
        )
    except (OSError, UnicodeError) as error:
        raise CoverageCiGuardError(f"cannot {description}: {error}") from error
    if process.returncode != 0:
        detail = process.stderr.strip() or process.stdout.strip() or "no diagnostics"
        raise CoverageCiGuardError(f"cannot {description}: {detail}")
    return process.stdout


def require_full_sha(value: str, *, label: str) -> str:
    if not FULL_SHA.fullmatch(value):
        raise CoverageCiGuardError(f"{label} must be exactly 40 hexadecimal characters")
    if value == ZERO_SHA:
        raise CoverageCiGuardError(f"{label} must not be the all-zero SHA")
    return value.lower()


def resolve_commit(repo_root: pathlib.Path, value: str, *, label: str) -> str:
    requested = require_full_sha(value, label=label)
    output = run_git(
        repo_root,
        ["rev-parse", "--verify", "--end-of-options", f"{requested}^{{commit}}"],
        description=f"resolve {label}",
    )
    lines = output.splitlines()
    if len(lines) != 1 or not FULL_SHA.fullmatch(lines[0]):
        raise CoverageCiGuardError(f"{label} did not resolve to exactly one commit")
    return lines[0].lower()


def resolve_remote_branch(
    repo_root: pathlib.Path, branch: str, *, label: str
) -> str:
    checked = run_git(
        repo_root,
        ["check-ref-format", "--branch", branch],
        description=f"validate {label}",
    ).strip()
    if checked != branch:
        raise CoverageCiGuardError(f"{label} did not validate exactly")
    ref = f"refs/remotes/origin/{branch}"
    output = run_git(
        repo_root,
        ["rev-parse", "--verify", "--end-of-options", f"{ref}^{{commit}}"],
        description=f"resolve {label}",
    )
    lines = output.splitlines()
    if len(lines) != 1 or not FULL_SHA.fullmatch(lines[0]):
        raise CoverageCiGuardError(f"{label} did not resolve to exactly one commit")
    return lines[0].lower()


def unique_merge_base(
    repo_root: pathlib.Path,
    left: str,
    right: str,
    *,
    label: str,
) -> list[str]:
    output = run_git(
        repo_root,
        ["merge-base", "--all", left, right],
        description=f"compute {label}",
    )
    merge_bases = [
        require_full_sha(line, label=label) for line in output.splitlines() if line
    ]
    if len(merge_bases) != 1:
        raise CoverageCiGuardError(
            f"{label} must resolve exactly once; Git returned {len(merge_bases)}"
        )
    return merge_bases


def append_github_env(path: pathlib.Path, values: Mapping[str, str]) -> None:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a", encoding="utf-8", newline="\n") as handle:
            for key, value in values.items():
                handle.write(f"{key}={value}\n")
    except OSError as error:
        raise CoverageCiGuardError(
            f"cannot append resolved base to GitHub environment file: {error}"
        ) from error


def base_report_template(args: argparse.Namespace) -> dict[str, Any]:
    requested = {
        "pull_request": args.pull_request_base,
        "push": args.push_before,
        "workflow_dispatch": args.dispatch_base,
    }.get(args.event_name)
    return {
        "schema_version": BASE_SCHEMA_VERSION,
        "kind": BASE_REPORT_KIND,
        "status": "error",
        "event_name": args.event_name,
        "mode": None,
        "verification_sha_input": args.verification_sha,
        "verification_sha": None,
        "pull_request_base_sha_input": args.pull_request_base,
        "push_before_sha_input": args.push_before,
        "push_ref_type": args.push_ref_type if args.event_name == "push" else None,
        "default_branch_name": (
            args.default_branch
            if args.event_name == "push" and args.push_ref_type == "tag"
            else None
        ),
        "default_branch_ref": None,
        "default_branch_sha": None,
        "dispatch_base_sha_input": args.dispatch_base,
        "requested_base_sha": requested,
        "effective_base_sha": None,
        "merge_bases": [],
        "errors": [],
    }


def resolve_base(args: argparse.Namespace) -> int:
    report = base_report_template(args)
    output = pathlib.Path(args.output)
    try:
        repo_root = pathlib.Path(args.repo_root).resolve()
        if not repo_root.is_dir():
            raise CoverageCiGuardError(
                f"repository root is not a directory: {repo_root}"
            )
        verification = resolve_commit(
            repo_root,
            args.verification_sha,
            label="verification SHA",
        )
        checkout = run_git(
            repo_root,
            ["rev-parse", "--verify", "HEAD^{commit}"],
            description="resolve checkout HEAD",
        ).strip()
        if checkout.lower() != verification:
            raise CoverageCiGuardError(
                f"checkout HEAD {checkout} does not match verification SHA {verification}"
            )
        report["verification_sha"] = verification

        if args.event_name == "pull_request":
            requested = resolve_commit(
                repo_root,
                args.pull_request_base,
                label="pull-request base SHA",
            )
            merge_bases = unique_merge_base(
                repo_root,
                requested,
                verification,
                label="pull-request merge base",
            )
            report["merge_bases"] = merge_bases
            mode = "pull-request-merge-base"
            effective = merge_bases[0]
        elif args.event_name == "push":
            if args.push_ref_type == "tag":
                if args.push_before == ZERO_SHA:
                    requested = resolve_remote_branch(
                        repo_root,
                        args.default_branch,
                        label="remote default branch",
                    )
                    default_branch_ref = (
                        f"refs/remotes/origin/{args.default_branch}"
                    )
                    merge_bases = unique_merge_base(
                        repo_root,
                        requested,
                        verification,
                        label="tag/default-branch merge base",
                    )
                    report.update(
                        {
                            "default_branch_ref": default_branch_ref,
                            "default_branch_sha": requested,
                            "merge_bases": merge_bases,
                        }
                    )
                    mode = "tag-default-branch-merge-base"
                    effective = merge_bases[0]
                else:
                    requested = resolve_commit(
                        repo_root,
                        args.push_before,
                        label="tag push before SHA",
                    )
                    mode = "tag-push-before"
                    effective = requested
            elif args.push_ref_type == "branch":
                requested = resolve_commit(
                    repo_root,
                    args.push_before,
                    label="push before SHA",
                )
                mode = "push-before"
                effective = requested
            else:
                raise CoverageCiGuardError(
                    "push ref type must be exactly branch or tag"
                )
        elif args.event_name == "workflow_dispatch":
            requested = resolve_commit(
                repo_root,
                args.dispatch_base,
                label="workflow-dispatch base SHA",
            )
            mode = "workflow-dispatch-explicit"
            effective = requested
        else:
            raise CoverageCiGuardError(
                "event name must be pull_request, push, or workflow_dispatch"
            )

        report.update(
            {
                "status": "passed",
                "mode": mode,
                "requested_base_sha": requested,
                "effective_base_sha": effective,
            }
        )
        append_github_env(
            pathlib.Path(args.github_env),
            {
                "COVERAGE_BASE_SHA": effective,
                "COVERAGE_REQUESTED_BASE_SHA": requested,
                "COVERAGE_BASE_MODE": mode,
            },
        )
    except CoverageCiGuardError as error:
        report["status"] = "error"
        report["errors"].append(str(error))
    except Exception as error:  # pragma: no cover - last-resort CI diagnostics
        report["status"] = "error"
        report["errors"].append(
            f"unexpected {type(error).__name__} while resolving coverage base: {error}"
        )

    try:
        atomic_write_json(output, report)
    except CoverageCiGuardError as error:
        print(f"coverage base resolution failed: {error}", file=sys.stderr)
        return 2
    if report["status"] != "passed":
        print(
            "coverage base resolution failed: " + " | ".join(report["errors"]),
            file=sys.stderr,
        )
        return 2
    print(
        f"coverage base: {report['effective_base_sha']} ({report['mode']})"
    )
    return 0


def duplicate_rejecting_json_object(
    pairs: list[tuple[str, Any]],
) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise CoverageCiGuardError(
                f"JSON evidence duplicates object key {key!r}"
            )
        value[key] = item
    return value


def reject_json_numeric_constant(value: str) -> None:
    raise CoverageCiGuardError(
        f"JSON evidence contains unsupported numeric constant {value}"
    )


def read_json(
    path: pathlib.Path,
    *,
    description: str,
    limit: int = MAX_JSON_BYTES,
) -> dict[str, Any]:
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise CoverageCiGuardError(f"cannot read {description} {path}: {error}") from error
    if len(raw) > limit:
        raise CoverageCiGuardError(
            f"{description} exceeds the {limit}-byte safety limit"
        )
    try:
        value = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=duplicate_rejecting_json_object,
            parse_constant=reject_json_numeric_constant,
        )
    except CoverageCiGuardError:
        raise
    except (UnicodeError, json.JSONDecodeError) as error:
        raise CoverageCiGuardError(f"{description} is not valid UTF-8 JSON: {error}") from error
    if not isinstance(value, dict):
        raise CoverageCiGuardError(f"{description} must contain a JSON object")
    return value


def file_metadata(path: pathlib.Path, *, limit: int, description: str) -> dict[str, Any]:
    try:
        stat = path.stat()
    except OSError as error:
        raise CoverageCiGuardError(f"cannot inspect {description} {path}: {error}") from error
    if not path.is_file():
        raise CoverageCiGuardError(f"{description} is not a regular file: {path}")
    if stat.st_size <= 0:
        raise CoverageCiGuardError(f"{description} is empty: {path}")
    if stat.st_size > limit:
        raise CoverageCiGuardError(
            f"{description} exceeds the {limit}-byte safety limit"
        )
    digest = hashlib.sha256()
    try:
        with path.open("rb") as handle:
            for block in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as error:
        raise CoverageCiGuardError(f"cannot hash {description} {path}: {error}") from error
    return {
        "path": str(path),
        "size_bytes": stat.st_size,
        "sha256": digest.hexdigest(),
    }


def outcome_phase(outcome: str) -> dict[str, Any]:
    return {
        "outcome": outcome,
        "status": (
            "pending-validation"
            if outcome == "success"
            else "blocked"
            if outcome == "skipped"
            else "failed"
        ),
        "errors": [],
    }


def validate_outcome(outcome: str, *, phase: str) -> None:
    if outcome not in KNOWN_OUTCOMES:
        raise CoverageCiGuardError(
            f"{phase} outcome {outcome!r} is not a recognized GitHub step outcome"
        )
    if outcome != "success":
        raise CoverageCiGuardError(f"{phase} step outcome was {outcome}")


def finalize(args: argparse.Namespace) -> int:
    phase_order = [
        "resolve-base",
        "coverage-profiles",
        "llvm-json",
        "llvm-text",
        "line-map",
        "diff-policy",
    ]
    report: dict[str, Any] = {
        "schema_version": PHASE_SCHEMA_VERSION,
        "kind": PHASE_REPORT_KIND,
        "status": "failed",
        "phase_order": phase_order,
        "phases": {
            "resolve-base": outcome_phase(args.base_outcome),
            "coverage-profiles": outcome_phase(args.profiles_outcome),
            "llvm-json": outcome_phase(args.llvm_json_outcome),
            "llvm-text": outcome_phase(args.llvm_text_outcome),
            "line-map": outcome_phase(args.line_map_outcome),
            "diff-policy": outcome_phase(args.policy_outcome),
        },
        "errors": [],
    }
    phases = report["phases"]
    retained_line_maps: list[dict[str, Any]] = []

    def fail(phase_name: str, error: CoverageCiGuardError) -> None:
        message = str(error)
        phases[phase_name]["errors"].append(message)
        report["errors"].append(f"{phase_name}: {message}")
        if phases[phase_name]["status"] in {"pending-validation", "passed"}:
            phases[phase_name]["status"] = "failed"

    def pass_if_clean(phase_name: str) -> None:
        if not phases[phase_name]["errors"]:
            phases[phase_name]["status"] = "passed"

    def validate_file_phase(
        *,
        phase_name: str,
        outcome: str,
        outcome_description: str,
        path: str,
        limit: int,
        artifact_description: str,
    ) -> None:
        try:
            validate_outcome(outcome, phase=outcome_description)
        except CoverageCiGuardError as error:
            fail(phase_name, error)
        if outcome in {"success", "failure"}:
            try:
                phases[phase_name].update(
                    file_metadata(
                        pathlib.Path(path),
                        limit=limit,
                        description=artifact_description,
                    )
                )
            except CoverageCiGuardError as error:
                fail(phase_name, error)
        pass_if_clean(phase_name)

    def failed_report_detail(value: Mapping[str, Any]) -> str:
        errors = value.get("errors")
        if isinstance(errors, list) and errors:
            return " | ".join(str(item) for item in errors)
        return f"status is {value.get('status')!r}"

    def line_map_summary(value: Mapping[str, Any]) -> dict[str, Any]:
        return {
            field: value.get(field)
            for field in (
                "schema_version",
                "kind",
                "status",
                "line_model",
                "inputs",
                "producer",
                "summary",
                "errors",
            )
        }

    def validate_line_map_document(
        value: Mapping[str, Any], *, description: str
    ) -> Mapping[str, Any]:
        if (
            value.get("kind") != LINE_MAP_REPORT_KIND
            or value.get("schema_version") != 1
        ):
            raise CoverageCiGuardError(
                f"{description} report has an unsupported schema"
            )
        if value.get("status") != "passed":
            raise CoverageCiGuardError(
                f"{description} conversion did not pass: "
                + failed_report_detail(value)
            )
        if value.get("line_model") != "unique-physical-source-lines":
            raise CoverageCiGuardError(
                f"{description} report has an unsupported line model"
            )
        if value.get("producer") != PINNED_LINE_MAP_PRODUCER:
            raise CoverageCiGuardError(
                f"{description} report has unpinned producer metadata"
            )
        inputs = value.get("inputs")
        if not isinstance(inputs, Mapping):
            raise CoverageCiGuardError(f"{description} report has no producer inputs")
        for input_name in ("llvm_json", "llvm_text"):
            if not isinstance(inputs.get(input_name), Mapping):
                raise CoverageCiGuardError(
                    f"{description} report has no {input_name} reference"
                )
        return inputs

    try:
        validate_outcome(args.base_outcome, phase="base resolution")
    except CoverageCiGuardError as error:
        fail("resolve-base", error)
    if args.base_outcome in {"success", "failure"}:
        try:
            base = read_json(pathlib.Path(args.base_report), description="base report")
            phases["resolve-base"]["report"] = base
            if base.get("kind") != BASE_REPORT_KIND or base.get("schema_version") != 1:
                raise CoverageCiGuardError("base report has an unsupported schema")
            if base.get("status") != "passed":
                base_errors = base.get("errors")
                detail = (
                    " | ".join(str(item) for item in base_errors)
                    if isinstance(base_errors, list) and base_errors
                    else f"status is {base.get('status')!r}"
                )
                raise CoverageCiGuardError(f"base resolution did not pass: {detail}")
            effective = base.get("effective_base_sha")
            require_full_sha(
                effective if isinstance(effective, str) else "",
                label="effective base SHA",
            )
        except CoverageCiGuardError as error:
            fail("resolve-base", error)
    pass_if_clean("resolve-base")

    try:
        validate_outcome(
            args.profiles_outcome,
            phase="coverage profile generation",
        )
    except CoverageCiGuardError as error:
        fail("coverage-profiles", error)
    if args.profiles_outcome in {"success", "failure"}:
        try:
            profile_path = pathlib.Path(args.profile_lanes)
            phases["coverage-profiles"].update(
                file_metadata(
                    profile_path,
                    limit=coverage_profile_lanes.MAX_REPORT_BYTES,
                    description="coverage profile lane report",
                )
            )
            profile_report = read_json(
                profile_path,
                description="coverage profile lane report",
                limit=coverage_profile_lanes.MAX_REPORT_BYTES,
            )
            lanes = profile_report.get("lanes")
            lane_summaries: dict[str, Any] = {}
            if isinstance(lanes, Mapping):
                for lane_name, lane in lanes.items():
                    if isinstance(lane_name, str) and isinstance(lane, Mapping):
                        lane_summaries[lane_name] = {
                            field: lane.get(field)
                            for field in (
                                "status",
                                "command",
                                "instrumentation",
                                "environment_overrides",
                                "exit_code",
                                "profile_count_before",
                                "profile_count_after",
                                "profile_delta_count",
                                "profile_removed_count",
                                "oracle",
                                "errors",
                            )
                        }
            phases["coverage-profiles"]["report"] = {
                field: profile_report.get(field)
                for field in (
                    "schema_version",
                    "kind",
                    "status",
                    "producer",
                    "legacy_reference",
                    "instrumentation_environment",
                    "profile_reset",
                    "lane_order",
                    "errors",
                )
            }
            phases["coverage-profiles"]["report"]["lanes"] = lane_summaries
            try:
                coverage_profile_lanes.validate_report_document(profile_report)
            except coverage_profile_lanes.CoverageProfileLaneError as error:
                raise CoverageCiGuardError(
                    f"coverage profile lane report did not pass: {error}"
                ) from error
        except CoverageCiGuardError as error:
            fail("coverage-profiles", error)
    pass_if_clean("coverage-profiles")

    validate_file_phase(
        phase_name="llvm-json",
        outcome=args.llvm_json_outcome,
        outcome_description="LLVM JSON export",
        path=args.llvm_json,
        limit=MAX_LLVM_JSON_BYTES,
        artifact_description="LLVM JSON artifact",
    )
    validate_file_phase(
        phase_name="llvm-text",
        outcome=args.llvm_text_outcome,
        outcome_description="LLVM native text export",
        path=args.llvm_text,
        limit=MAX_LLVM_TEXT_BYTES,
        artifact_description="LLVM native text artifact",
    )
    validate_file_phase(
        phase_name="line-map",
        outcome=args.line_map_outcome,
        outcome_description="canonical line-map conversion",
        path=args.line_map,
        limit=MAX_LINE_MAP_BYTES,
        artifact_description="canonical line-map artifact",
    )

    if args.line_map_outcome in {"success", "failure"}:
        try:
            line_map = read_json(
                pathlib.Path(args.line_map),
                description="canonical line-map report",
                limit=MAX_LINE_MAP_BYTES,
            )
            phases["line-map"]["report"] = line_map_summary(line_map)
            inputs = validate_line_map_document(
                line_map, description="canonical line-map"
            )
            references = {
                "llvm_json": phases["llvm-json"],
                "llvm_text": phases["llvm-text"],
            }
            for input_name, artifact_phase in references.items():
                reference = inputs.get(input_name)
                if not isinstance(reference, dict):
                    raise CoverageCiGuardError(
                        f"canonical line-map report has no {input_name} reference"
                    )
                expected_size = artifact_phase.get("size_bytes")
                expected_digest = artifact_phase.get("sha256")
                if (
                    reference.get("size_bytes") != expected_size
                    or reference.get("sha256") != f"sha256:{expected_digest}"
                ):
                    raise CoverageCiGuardError(
                        f"canonical line-map {input_name} reference does not "
                        "match the retained producer artifact"
                    )
            retained_line_maps.append(
                {
                    "path": args.line_map,
                    "sha256": phases["line-map"].get("sha256"),
                    "document": line_map,
                }
            )
        except CoverageCiGuardError as error:
            fail("line-map", error)

        supplemental_paths = list(args.supplemental_line_map)
        if len(supplemental_paths) + 1 > MAX_COVERAGE_MAPS:
            fail(
                "line-map",
                CoverageCiGuardError(
                    f"coverage evidence supplies more than {MAX_COVERAGE_MAPS} "
                    "canonical line maps"
                ),
            )
        else:
            phases["line-map"]["supplemental_maps"] = []
            seen_paths = {os.path.normcase(os.path.abspath(args.line_map))}
            seen_digests = {
                item["sha256"]
                for item in retained_line_maps
                if isinstance(item.get("sha256"), str)
            }
            for raw_path in supplemental_paths:
                try:
                    path = pathlib.Path(raw_path)
                    normalized_path = os.path.normcase(os.path.abspath(raw_path))
                    if normalized_path in seen_paths:
                        raise CoverageCiGuardError(
                            "supplemental line-map path duplicates a retained map"
                        )
                    seen_paths.add(normalized_path)
                    metadata = file_metadata(
                        path,
                        limit=MAX_LINE_MAP_BYTES,
                        description="supplemental canonical line-map artifact",
                    )
                    digest = metadata.get("sha256")
                    if digest in seen_digests:
                        raise CoverageCiGuardError(
                            "supplemental line-map content duplicates a retained map"
                        )
                    if isinstance(digest, str):
                        seen_digests.add(digest)
                    supplemental = read_json(
                        path,
                        description="supplemental canonical line-map report",
                        limit=MAX_LINE_MAP_BYTES,
                    )
                    validate_line_map_document(
                        supplemental,
                        description="supplemental canonical line-map",
                    )
                    entry = {
                        "path": raw_path,
                        **metadata,
                        "report": line_map_summary(supplemental),
                    }
                    phases["line-map"]["supplemental_maps"].append(entry)
                    retained_line_maps.append(
                        {
                            "path": raw_path,
                            "sha256": digest,
                            "document": supplemental,
                        }
                    )
                except CoverageCiGuardError as error:
                    fail("line-map", error)
    pass_if_clean("line-map")

    try:
        validate_outcome(args.policy_outcome, phase="diff policy")
    except CoverageCiGuardError as error:
        fail("diff-policy", error)
    if args.policy_outcome in {"success", "failure"}:
        try:
            policy = read_json(
                pathlib.Path(args.policy_report),
                description="diff-coverage report",
            )
            phases["diff-policy"]["report"] = policy
            if (
                policy.get("kind") != DIFF_REPORT_KIND
                or policy.get("schema_version") != 1
            ):
                raise CoverageCiGuardError(
                    "diff-coverage report has an unsupported schema"
                )
            if policy.get("status") != "passed":
                raise CoverageCiGuardError(
                    "diff-coverage policy did not pass: "
                    + failed_report_detail(policy)
                )
            policy_inputs = policy.get("inputs")
            if not isinstance(policy_inputs, dict):
                raise CoverageCiGuardError(
                    "diff-coverage report has no input attestation"
                )
            if args.supplemental_line_map:
                expected_maps = [
                    {
                        "path": item["path"],
                        "sha256": f"sha256:{item['sha256']}",
                        "line_model": item["document"].get("line_model"),
                        "producer": item["document"].get("producer"),
                        "producer_inputs": item["document"].get("inputs"),
                    }
                    for item in retained_line_maps
                ]
                if (
                    len(expected_maps) != len(args.supplemental_line_map) + 1
                    or policy_inputs.get("coverage_kind")
                    != "llvm-physical-line-map-union"
                    or policy_inputs.get("coverage_line_model")
                    != "unique-physical-source-lines"
                    or policy_inputs.get("coverage_maps") != expected_maps
                ):
                    raise CoverageCiGuardError(
                        "diff-coverage report is not bound to the complete "
                        "retained canonical line-map artifact set"
                    )
            else:
                expected_line_map_digest = phases["line-map"].get("sha256")
                if (
                    policy_inputs.get("coverage_kind")
                    != "llvm-physical-line-map"
                    or policy_inputs.get("coverage_map_sha256")
                    != f"sha256:{expected_line_map_digest}"
                ):
                    raise CoverageCiGuardError(
                        "diff-coverage report is not bound to the retained "
                        "canonical line-map artifact"
                    )
        except CoverageCiGuardError as error:
            fail("diff-policy", error)
    pass_if_clean("diff-policy")

    if all(phases[name]["status"] == "passed" for name in phase_order):
        report["status"] = "passed"

    output = pathlib.Path(args.output)
    try:
        atomic_write_json(output, report)
    except CoverageCiGuardError as error:
        print(f"coverage evidence finalization failed: {error}", file=sys.stderr)
        return 2
    if report["status"] != "passed":
        print(
            "coverage evidence finalization failed: " + " | ".join(report["errors"]),
            file=sys.stderr,
        )
        return 1
    print(f"complete changed-line coverage evidence: {output}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    base = subcommands.add_parser("resolve-base")
    base.add_argument("--repo-root", required=True)
    base.add_argument("--event-name", required=True)
    base.add_argument("--verification-sha", required=True)
    base.add_argument("--pull-request-base", default="")
    base.add_argument("--push-before", default="")
    base.add_argument("--push-ref-type", default="branch")
    base.add_argument("--default-branch", default="")
    base.add_argument("--dispatch-base", default="")
    base.add_argument("--github-env", required=True)
    base.add_argument("--output", required=True)
    base.set_defaults(func=resolve_base)

    phase = subcommands.add_parser("finalize")
    phase.add_argument("--base-outcome", required=True)
    phase.add_argument("--profiles-outcome", required=True)
    phase.add_argument("--llvm-json-outcome", required=True)
    phase.add_argument("--llvm-text-outcome", required=True)
    phase.add_argument("--line-map-outcome", required=True)
    phase.add_argument("--policy-outcome", required=True)
    phase.add_argument("--base-report", required=True)
    phase.add_argument("--llvm-json", required=True)
    phase.add_argument("--llvm-text", required=True)
    phase.add_argument("--line-map", required=True)
    phase.add_argument("--supplemental-line-map", action="append", default=[])
    phase.add_argument("--policy-report", required=True)
    phase.add_argument("--profile-lanes", required=True)
    phase.add_argument("--output", required=True)
    phase.set_defaults(func=finalize)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
