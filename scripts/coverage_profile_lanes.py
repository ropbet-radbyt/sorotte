#!/usr/bin/env python3
"""Collect and attest compatible Sorotte coverage execution profiles.

The ordinary cargo-llvm-cov command covers workspace test binaries, but it does
not execute the GUI semantic suite or strict live compatibility paths. This
wrapper owns those commands, applies cargo-llvm-cov's machine-readable
``show-env`` contract to the external Cargo processes, requires a fresh raw
profile delta from every execution lane, removes and attests stale generated
profile inputs before execution, validates each lane's behavioral oracle, and
finally asks cargo-llvm-cov to merge only the current run's profiles.

The resulting JSON is intentionally strict. It lets CI distinguish
"instrumented and executed" from a green command that produced no coverage,
ran only part of a suite, or silently skipped an external prerequisite.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import math
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time
from collections.abc import Mapping, Sequence
from typing import Any, Callable


SCHEMA_VERSION = 2
REPORT_KIND = "sorotte-coverage-profile-lanes"
PINNED_CARGO_LLVM_COV_VERSION = "0.8.4"
PINNED_LEGACY_SYNCPLAY_SHA = "d1c5f85af377c960c5a940707c4d01bc84fd9c3f"
MAX_REPORT_BYTES = 8 * 1024 * 1024
MAX_LOG_BYTES = 128 * 1024 * 1024
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
ENVIRONMENT_KEY = re.compile(r"^[A-Z_][A-Z0-9_]*$")

VERSION_COMMAND = ("cargo", "llvm-cov", "--version")
SHOW_ENV_COMMAND = ("cargo", "llvm-cov", "show-env")
WORKSPACE_COMMAND = (
    "cargo",
    "llvm-cov",
    "--locked",
    "--workspace",
    "--all-features",
    "--no-report",
)
SEMANTIC_COMMAND = (
    "cargo",
    "run",
    "--quiet",
    "--locked",
    "-p",
    "sorotte-gui",
    "--features",
    "gui-semantic-smoke,live-python-interop",
    "--bin",
    "sorotte-gui-semantic-suite",
    "--",
    "--json",
)
COMPAT_COMMAND = (
    "cargo",
    "test",
    "--locked",
    "-p",
    "sorotte-compat",
    "--all-features",
    "legacy_server_live_tls_",
    "--",
    "--nocapture",
)
MERGE_COMMAND = ("cargo", "llvm-cov", "report", "--summary-only")

LANE_ORDER = (
    "workspace-all-features",
    "gui-semantic",
    "compat-live-tls",
    "merge-check",
)
PROFILE_LANES = frozenset(LANE_ORDER[:3])
EXPECTED_SHOW_ENV_KEYS = frozenset(
    {
        "LLVM_PROFILE_FILE",
        "__CARGO_LLVM_COV_RUSTC_WRAPPER",
        "__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS",
        "__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES",
        "RUSTC_WRAPPER",
        "CARGO_LLVM_COV",
        "CARGO_LLVM_COV_SHOW_ENV",
        "CARGO_LLVM_COV_TARGET_DIR",
        "CARGO_LLVM_COV_BUILD_DIR",
    }
)
REQUIRED_INSTRUMENTED_CRATES = frozenset(
    {
        "sorotte_gui_semantic_suite",
        "sorotte_compat_tests",
    }
)
EXPECTED_SEMANTIC_SCENARIOS = (
    "configuration-surface-flow",
    "core-shell-smoke-flow",
    "localized-runtime-flow",
    "runtime-chat-flow",
    "runtime-transport-churn-flow",
    "drag-and-drop-ingest-flow",
    "playlist-workflow-flow",
    "player-setup-flow",
    "persistence-reset-flow",
    "detached-runtime-ownership-flow",
    "live-python-peer-connect-flow",
    "live-python-peer-controlled-room-flow",
    "seek-preparation-flow",
    "readiness-v2-flow",
)
EXPECTED_COMPAT_TESTS = (
    "legacy_server_live_tls_upgrade_roundtrip_supports_post_upgrade_hello_over_same_socket",
    "legacy_server_live_tls_send_is_denied_for_logged_client",
    "legacy_server_live_tls_rotation_invalidates_subsequent_send",
    "legacy_server_live_tls_rotation_recovers_after_bundle_restored",
)
EXPECTED_COMPAT_FILTERED_OUT = 140
COMPAT_SKIP_MARKERS = (
    "assertion skipped",
    "test skipped",
    "skipped due to missing",
    "parity assertion skipped",
)
LANE_COMMANDS = {
    "workspace-all-features": WORKSPACE_COMMAND,
    "gui-semantic": SEMANTIC_COMMAND,
    "compat-live-tls": COMPAT_COMMAND,
    "merge-check": MERGE_COMMAND,
}
LANE_INSTRUMENTATION = {
    "workspace-all-features": "cargo-llvm-cov",
    "gui-semantic": "show-env",
    "compat-live-tls": "show-env",
    "merge-check": "cargo-llvm-cov-report",
}
LANE_ENVIRONMENT_OVERRIDES = {
    "workspace-all-features": (),
    "gui-semantic": (
        "CARGO_TARGET_DIR",
        "SOROTTE_CLIENT_CONFIG_PATH",
    ),
    "compat-live-tls": (
        "CARGO_TARGET_DIR",
        "SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY",
        "SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY",
    ),
    "merge-check": (),
}


class CoverageProfileLaneError(RuntimeError):
    """A producer, instrumentation, oracle, or attestation contract failed."""


@dataclasses.dataclass(frozen=True)
class CommandResult:
    command: tuple[str, ...]
    returncode: int
    stdout: bytes
    stderr: bytes
    duration_seconds: float


@dataclasses.dataclass(frozen=True)
class ProfileState:
    size_bytes: int
    modified_ns: int
    sha256: str


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def reject_duplicate_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise CoverageProfileLaneError(
                f"JSON object contains duplicate key {key!r}"
            )
        value[key] = item
    return value


def reject_json_constant(value: str) -> None:
    raise CoverageProfileLaneError(
        f"JSON contains non-standard numeric constant {value!r}"
    )


def parse_json(data: bytes, *, label: str) -> Any:
    try:
        text = data.decode("utf-8", errors="strict")
        return json.loads(
            text,
            object_pairs_hook=reject_duplicate_pairs,
            parse_constant=reject_json_constant,
        )
    except UnicodeError as error:
        raise CoverageProfileLaneError(
            f"{label} is not valid UTF-8: {error}"
        ) from error
    except json.JSONDecodeError as error:
        raise CoverageProfileLaneError(
            f"{label} is not valid JSON: {error}"
        ) from error


def require_mapping(value: Any, *, label: str) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise CoverageProfileLaneError(f"{label} must be an object")
    return value


def require_list(value: Any, *, label: str) -> list[Any]:
    if not isinstance(value, list):
        raise CoverageProfileLaneError(f"{label} must be an array")
    return value


def require_exact_keys(
    value: Mapping[str, Any],
    expected: set[str] | frozenset[str],
    *,
    label: str,
) -> None:
    actual = set(value)
    if actual != set(expected):
        raise CoverageProfileLaneError(
            f"{label} fields do not match schema; "
            f"unknown={sorted(actual - set(expected))}, "
            f"missing={sorted(set(expected) - actual)}"
        )


def require_string(value: Any, *, label: str) -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise CoverageProfileLaneError(f"{label} must be a non-empty safe string")
    return value


def require_int(value: Any, *, label: str, minimum: int = 0) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        raise CoverageProfileLaneError(
            f"{label} must be an integer greater than or equal to {minimum}"
        )
    return value


def require_number(value: Any, *, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise CoverageProfileLaneError(f"{label} must be a finite number")
    converted = float(value)
    if converted < 0 or not math.isfinite(converted):
        raise CoverageProfileLaneError(
            f"{label} must be a finite non-negative number"
        )
    return converted


def resolve_within(
    root: pathlib.Path,
    value: pathlib.Path,
    *,
    label: str,
) -> pathlib.Path:
    resolved_root = root.resolve()
    candidate = value if value.is_absolute() else resolved_root / value
    resolved = candidate.resolve()
    if not resolved.is_relative_to(resolved_root):
        raise CoverageProfileLaneError(
            f"{label} must remain inside repository root {resolved_root}: {resolved}"
        )
    return resolved


def repo_relative(repo_root: pathlib.Path, path: pathlib.Path) -> str:
    return path.resolve().relative_to(repo_root.resolve()).as_posix()


def parse_producer_version(result: CommandResult) -> str:
    if result.returncode != 0:
        raise CoverageProfileLaneError(
            f"cargo-llvm-cov version command exited with {result.returncode}"
        )
    try:
        output = result.stdout.decode("utf-8", errors="strict").strip()
    except UnicodeError as error:
        raise CoverageProfileLaneError(
            f"cargo-llvm-cov version output is not UTF-8: {error}"
        ) from error
    match = re.fullmatch(r"cargo-llvm-cov ([0-9]+\.[0-9]+\.[0-9]+)", output)
    if match is None:
        raise CoverageProfileLaneError(
            f"unrecognized cargo-llvm-cov version output {output!r}"
        )
    version = match.group(1)
    if version != PINNED_CARGO_LLVM_COV_VERSION:
        raise CoverageProfileLaneError(
            "cargo-llvm-cov version must be "
            f"{PINNED_CARGO_LLVM_COV_VERSION}, received {version}"
        )
    return version


def parse_show_env(
    result: CommandResult,
    *,
    repo_root: pathlib.Path,
) -> tuple[dict[str, str], dict[str, Any], pathlib.Path]:
    if result.returncode != 0:
        raise CoverageProfileLaneError(
            f"cargo llvm-cov show-env exited with {result.returncode}"
        )
    try:
        output = result.stdout.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise CoverageProfileLaneError(
            f"cargo llvm-cov show-env output is not UTF-8: {error}"
        ) from error

    environment: dict[str, str] = {}
    for line_number, raw_line in enumerate(output.splitlines(), start=1):
        if not raw_line or "=" not in raw_line:
            raise CoverageProfileLaneError(
                f"show-env line {line_number} is not KEY=VALUE"
            )
        key, value = raw_line.split("=", maxsplit=1)
        if not ENVIRONMENT_KEY.fullmatch(key):
            raise CoverageProfileLaneError(
                f"show-env line {line_number} has unsafe key {key!r}"
            )
        if key in environment:
            raise CoverageProfileLaneError(
                f"show-env contains duplicate key {key!r}"
            )
        if not value or "\x00" in value:
            raise CoverageProfileLaneError(
                f"show-env value for {key!r} is empty or unsafe"
            )
        environment[key] = value

    require_exact_keys(
        environment,
        EXPECTED_SHOW_ENV_KEYS,
        label="cargo llvm-cov show-env",
    )
    if environment["CARGO_LLVM_COV"] != "1":
        raise CoverageProfileLaneError("show-env must set CARGO_LLVM_COV=1")
    if environment["CARGO_LLVM_COV_SHOW_ENV"] != "1":
        raise CoverageProfileLaneError(
            "show-env must set CARGO_LLVM_COV_SHOW_ENV=1"
        )

    wrapper_name = pathlib.Path(environment["RUSTC_WRAPPER"]).name.lower()
    if wrapper_name not in {"cargo-llvm-cov", "cargo-llvm-cov.exe"}:
        raise CoverageProfileLaneError(
            f"show-env RUSTC_WRAPPER is not cargo-llvm-cov: {wrapper_name!r}"
        )
    rustflags = environment["__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS"].split(
        "\x1f"
    )
    if "instrument-coverage" not in rustflags:
        raise CoverageProfileLaneError(
            "show-env rustflags do not enable instrument-coverage"
        )

    target_dir = resolve_within(
        repo_root,
        pathlib.Path(environment["CARGO_LLVM_COV_TARGET_DIR"]),
        label="cargo-llvm-cov target directory",
    )
    build_dir = resolve_within(
        repo_root,
        pathlib.Path(environment["CARGO_LLVM_COV_BUILD_DIR"]),
        label="cargo-llvm-cov build directory",
    )
    if build_dir != target_dir:
        raise CoverageProfileLaneError(
            "cargo-llvm-cov target and build directories must match"
        )

    profile_pattern = pathlib.Path(environment["LLVM_PROFILE_FILE"])
    profile_root = resolve_within(
        repo_root,
        profile_pattern.parent,
        label="LLVM profile directory",
    )
    if profile_root != target_dir:
        raise CoverageProfileLaneError(
            "LLVM profile pattern must write directly into the coverage target"
        )
    pattern_name = profile_pattern.name
    if (
        not pattern_name.endswith(".profraw")
        or "%p" not in pattern_name
        or "%32m" not in pattern_name
    ):
        raise CoverageProfileLaneError(
            "LLVM profile pattern must contain %p and %32m and end in .profraw"
        )

    crate_names = [
        name
        for name in environment[
            "__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES"
        ].split(",")
        if name
    ]
    missing_crates = sorted(REQUIRED_INSTRUMENTED_CRATES - set(crate_names))
    if missing_crates:
        raise CoverageProfileLaneError(
            f"show-env omits required instrumented crates {missing_crates}"
        )

    summary = {
        "keys": sorted(environment),
        "profile_pattern": (
            profile_root.relative_to(repo_root.resolve()) / pattern_name
        ).as_posix(),
        "target_dir": repo_relative(repo_root, target_dir),
        "build_dir": repo_relative(repo_root, build_dir),
        "crate_count": len(crate_names),
        "required_crates": sorted(REQUIRED_INSTRUMENTED_CRATES),
    }
    return environment, summary, profile_root


def profile_inventory(
    profile_root: pathlib.Path,
    *,
    repo_root: pathlib.Path,
) -> dict[str, ProfileState]:
    root = resolve_within(
        repo_root,
        profile_root,
        label="LLVM profile inventory root",
    )
    if not root.exists():
        return {}
    inventory: dict[str, ProfileState] = {}
    # cargo-llvm-cov's owned workspace-test profiles live under
    # target/llvm-cov-target, while processes launched with show-env write to
    # the profile pattern directly under target. The merged producer consumes
    # both, so freshness evidence must inventory the complete target tree.
    for path in sorted(root.rglob("*.profraw")):
        if path.is_symlink() or not path.is_file():
            raise CoverageProfileLaneError(
                f"LLVM profile must be a regular non-symlink file: {path}"
            )
        stat = path.stat()
        data = path.read_bytes()
        relative = repo_relative(repo_root, path)
        inventory[relative] = ProfileState(
            size_bytes=len(data),
            modified_ns=stat.st_mtime_ns,
            sha256=sha256(data),
        )
    return inventory


def reset_profile_artifacts(
    profile_root: pathlib.Path,
    *,
    repo_root: pathlib.Path,
) -> dict[str, Any]:
    root = resolve_within(
        repo_root,
        profile_root,
        label="LLVM profile reset root",
    )
    relative_root = root.relative_to(repo_root.resolve())
    if not relative_root.parts or relative_root.parts[0] != "target":
        raise CoverageProfileLaneError(
            "LLVM profile reset root must be inside the repository target "
            f"directory, received {relative_root.as_posix()!r}"
        )

    artifacts: dict[str, list[pathlib.Path]] = {
        "raw": [],
        "merged": [],
    }
    if root.exists():
        for kind, pattern in (
            ("raw", "*.profraw"),
            ("merged", "*.profdata"),
        ):
            for path in sorted(root.rglob(pattern)):
                if path.is_symlink() or not path.is_file():
                    raise CoverageProfileLaneError(
                        "LLVM profile reset target must be a regular "
                        f"non-symlink file: {path}"
                    )
                artifacts[kind].append(path)

    for path in artifacts["raw"] + artifacts["merged"]:
        path.unlink()

    remaining_raw = list(root.rglob("*.profraw")) if root.exists() else []
    remaining_merged = list(root.rglob("*.profdata")) if root.exists() else []
    if remaining_raw or remaining_merged:
        raise CoverageProfileLaneError(
            "LLVM profile reset left generated profile artifacts behind"
        )
    return {
        "kind": "fresh-profile-reset",
        "profile_root": repo_relative(repo_root, root),
        "removed_raw_profile_count": len(artifacts["raw"]),
        "removed_merged_profile_count": len(artifacts["merged"]),
        "remaining_raw_profile_count": 0,
        "remaining_merged_profile_count": 0,
    }


def profile_delta(
    before: Mapping[str, ProfileState],
    after: Mapping[str, ProfileState],
    *,
    repo_root: pathlib.Path,
) -> list[dict[str, Any]]:
    changed = [
        path
        for path, state in after.items()
        if before.get(path) != state
    ]
    delta: list[dict[str, Any]] = []
    for relative in sorted(changed):
        state = after[relative]
        if state.size_bytes == 0:
            raise CoverageProfileLaneError(
                f"LLVM profile is empty: {relative}"
            )
        delta.append(
            {
                "path": relative,
                "size_bytes": state.size_bytes,
                "sha256": state.sha256,
            }
        )
    return delta


def semantic_oracle(stdout: bytes) -> dict[str, Any]:
    document = require_mapping(
        parse_json(stdout, label="semantic suite output"),
        label="semantic suite output",
    )
    require_exact_keys(
        document,
        {"result", "total", "passed", "failed", "reports", "errors"},
        label="semantic suite output",
    )
    if (
        document.get("result") != "ok"
        or document.get("total") != len(EXPECTED_SEMANTIC_SCENARIOS)
        or document.get("passed") != len(EXPECTED_SEMANTIC_SCENARIOS)
        or document.get("failed") != 0
    ):
        raise CoverageProfileLaneError(
            "semantic suite did not report exactly 14 passed scenarios"
        )
    errors = require_list(document.get("errors"), label="semantic errors")
    if errors:
        raise CoverageProfileLaneError("semantic suite reported errors")
    reports = require_list(document.get("reports"), label="semantic reports")
    scenarios: list[str] = []
    for index, raw_report in enumerate(reports):
        report = require_mapping(
            raw_report,
            label=f"semantic report {index}",
        )
        require_exact_keys(
            report,
            {"result", "scenario", "view", "modal", "pending", "widgets"},
            label=f"semantic report {index}",
        )
        if report.get("result") != "ok":
            raise CoverageProfileLaneError(
                f"semantic report {index} did not pass"
            )
        scenarios.append(
            require_string(
                report.get("scenario"),
                label=f"semantic report {index} scenario",
            )
        )
        require_string(
            report.get("view"),
            label=f"semantic report {index} view",
        )
        require_string(
            report.get("modal"),
            label=f"semantic report {index} modal",
        )
        require_string(
            report.get("pending"),
            label=f"semantic report {index} pending",
        )
        require_int(
            report.get("widgets"),
            label=f"semantic report {index} widget count",
        )
    if tuple(scenarios) != EXPECTED_SEMANTIC_SCENARIOS:
        raise CoverageProfileLaneError(
            "semantic scenario inventory is missing, duplicated, or reordered"
        )
    return {
        "kind": "semantic-suite-json",
        "total": len(scenarios),
        "passed": len(scenarios),
        "failed": 0,
        "scenarios": scenarios,
    }


def compatibility_oracle(stdout: bytes, stderr: bytes) -> dict[str, Any]:
    combined = stdout + b"\n" + stderr
    try:
        text = combined.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise CoverageProfileLaneError(
            f"compatibility output is not valid UTF-8: {error}"
        ) from error
    lowered = text.lower()
    found_markers = [marker for marker in COMPAT_SKIP_MARKERS if marker in lowered]
    if found_markers:
        raise CoverageProfileLaneError(
            f"compatibility output contains skipped-oracle markers {found_markers}"
        )
    observed_tests: list[str] = []
    for test_name in EXPECTED_COMPAT_TESTS:
        pattern = re.compile(
            rf"(?m)^test .*::{re.escape(test_name)} \.\.\. ok$"
        )
        matches = pattern.findall(text)
        if len(matches) != 1:
            raise CoverageProfileLaneError(
                f"strict live TLS test {test_name!r} did not pass exactly once"
            )
        observed_tests.append(test_name)
    summary = re.compile(
        rf"test result: ok\. {len(EXPECTED_COMPAT_TESTS)} passed; "
        rf"0 failed; 0 ignored; 0 measured; "
        rf"{EXPECTED_COMPAT_FILTERED_OUT} filtered out"
    )
    matches = summary.findall(text)
    if len(matches) != 1:
        raise CoverageProfileLaneError(
            "compatibility suite did not report exactly "
            f"{len(EXPECTED_COMPAT_TESTS)} strict live TLS tests"
        )
    return {
        "kind": "libtest-exact-live-tls",
        "passed": len(EXPECTED_COMPAT_TESTS),
        "failed": 0,
        "ignored": 0,
        "filtered_out": EXPECTED_COMPAT_FILTERED_OUT,
        "tests": observed_tests,
        "skip_markers": [],
    }


def merge_oracle(stdout: bytes, stderr: bytes) -> dict[str, Any]:
    try:
        text = (stdout + b"\n" + stderr).decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise CoverageProfileLaneError(
            f"cargo llvm-cov merge output is not valid UTF-8: {error}"
        ) from error
    if not re.search(r"(?m)^TOTAL\s+", text):
        raise CoverageProfileLaneError(
            "cargo llvm-cov merge check did not emit a TOTAL summary"
        )
    return {
        "kind": "llvm-profile-merge",
        "summary_detected": True,
    }


def run_command(
    command: Sequence[str],
    *,
    cwd: pathlib.Path,
    environment: Mapping[str, str],
) -> CommandResult:
    command_tuple = tuple(command)
    print(f"coverage profile lane: {' '.join(command_tuple)}", flush=True)
    started = time.monotonic()
    process = subprocess.run(
        list(command_tuple),
        cwd=cwd,
        env=dict(environment),
        capture_output=True,
        check=False,
    )
    duration = time.monotonic() - started
    if process.stdout:
        sys.stdout.buffer.write(process.stdout)
        sys.stdout.buffer.flush()
    if process.stderr:
        sys.stderr.buffer.write(process.stderr)
        sys.stderr.buffer.flush()
    print(
        f"coverage profile lane exit={process.returncode} "
        f"duration={duration:.3f}s",
        flush=True,
    )
    return CommandResult(
        command=command_tuple,
        returncode=process.returncode,
        stdout=process.stdout,
        stderr=process.stderr,
        duration_seconds=duration,
    )


def file_metadata(
    path: pathlib.Path,
    *,
    repo_root: pathlib.Path,
) -> dict[str, Any]:
    data = path.read_bytes()
    if len(data) > MAX_LOG_BYTES:
        raise CoverageProfileLaneError(
            f"coverage lane log exceeds {MAX_LOG_BYTES} bytes: {path}"
        )
    return {
        "path": repo_relative(repo_root, path),
        "size_bytes": len(data),
        "sha256": sha256(data),
    }


def write_logs(
    result: CommandResult,
    *,
    lane: str,
    log_root: pathlib.Path,
    repo_root: pathlib.Path,
) -> tuple[dict[str, Any], dict[str, Any]]:
    log_root.mkdir(parents=True, exist_ok=True)
    stdout_path = log_root / f"{lane}.stdout.log"
    stderr_path = log_root / f"{lane}.stderr.log"
    stdout_path.write_bytes(result.stdout)
    stderr_path.write_bytes(result.stderr)
    return (
        file_metadata(stdout_path, repo_root=repo_root),
        file_metadata(stderr_path, repo_root=repo_root),
    )


def lane_record(
    *,
    lane: str,
    result: CommandResult,
    before: Mapping[str, ProfileState],
    after: Mapping[str, ProfileState],
    deltas: list[dict[str, Any]],
    removed_count: int,
    stdout_metadata: Mapping[str, Any],
    stderr_metadata: Mapping[str, Any],
    oracle: Mapping[str, Any] | None,
    errors: Sequence[str],
) -> dict[str, Any]:
    return {
        "status": "passed" if not errors else "failed",
        "command": list(result.command),
        "instrumentation": LANE_INSTRUMENTATION[lane],
        "environment_overrides": list(LANE_ENVIRONMENT_OVERRIDES[lane]),
        "exit_code": result.returncode,
        "duration_seconds": round(result.duration_seconds, 3),
        "stdout": dict(stdout_metadata),
        "stderr": dict(stderr_metadata),
        "profile_count_before": len(before),
        "profile_count_after": len(after),
        "profile_delta_count": len(deltas),
        "profile_deltas": deltas,
        "profile_removed_count": removed_count,
        "oracle": dict(oracle) if oracle is not None else None,
        "errors": list(errors),
    }


def execute_lane(
    *,
    lane: str,
    repo_root: pathlib.Path,
    profile_root: pathlib.Path,
    environment: Mapping[str, str],
    log_root: pathlib.Path,
    oracle_builder: Callable[[bytes, bytes], dict[str, Any]],
    require_profile_delta: bool,
) -> tuple[dict[str, Any], dict[str, ProfileState]]:
    before = profile_inventory(profile_root, repo_root=repo_root)
    result = run_command(
        LANE_COMMANDS[lane],
        cwd=repo_root,
        environment=environment,
    )
    stdout_metadata, stderr_metadata = write_logs(
        result,
        lane=lane,
        log_root=log_root,
        repo_root=repo_root,
    )
    after = profile_inventory(profile_root, repo_root=repo_root)
    errors: list[str] = []
    removed_profiles = sorted(set(before) - set(after))
    if removed_profiles:
        errors.append(
            f"lane removed {len(removed_profiles)} existing LLVM raw profiles"
        )
    deltas: list[dict[str, Any]] = []
    if result.returncode != 0:
        errors.append(f"command exited with status {result.returncode}")
    try:
        deltas = profile_delta(
            before,
            after,
            repo_root=repo_root,
        )
    except CoverageProfileLaneError as error:
        errors.append(str(error))
    if require_profile_delta and not deltas:
        errors.append("lane produced no fresh or changed LLVM raw profile")
    oracle: dict[str, Any] | None = None
    if result.returncode == 0:
        try:
            oracle = oracle_builder(result.stdout, result.stderr)
        except CoverageProfileLaneError as error:
            errors.append(str(error))
    return (
        lane_record(
            lane=lane,
            result=result,
            before=before,
            after=after,
            deltas=deltas,
            removed_count=len(removed_profiles),
            stdout_metadata=stdout_metadata,
            stderr_metadata=stderr_metadata,
            oracle=oracle,
            errors=errors,
        ),
        after,
    )


def validate_file_metadata(value: Any, *, label: str) -> None:
    metadata = require_mapping(value, label=label)
    require_exact_keys(
        metadata,
        {"path", "size_bytes", "sha256"},
        label=label,
    )
    require_string(metadata.get("path"), label=f"{label} path")
    require_int(metadata.get("size_bytes"), label=f"{label} size")
    digest = require_string(metadata.get("sha256"), label=f"{label} SHA-256")
    if not SHA256.fullmatch(digest):
        raise CoverageProfileLaneError(
            f"{label} SHA-256 must be 64 lowercase hexadecimal characters"
        )


def validate_profile_delta(value: Any, *, label: str) -> None:
    delta = require_mapping(value, label=label)
    require_exact_keys(
        delta,
        {"path", "size_bytes", "sha256"},
        label=label,
    )
    path = require_string(delta.get("path"), label=f"{label} path")
    if not path.endswith(".profraw") or pathlib.PurePosixPath(path).is_absolute():
        raise CoverageProfileLaneError(
            f"{label} path must be a relative .profraw path"
        )
    require_int(delta.get("size_bytes"), label=f"{label} size", minimum=1)
    digest = require_string(delta.get("sha256"), label=f"{label} SHA-256")
    if not SHA256.fullmatch(digest):
        raise CoverageProfileLaneError(
            f"{label} SHA-256 must be 64 lowercase hexadecimal characters"
        )


def validate_oracle(lane: str, value: Any) -> None:
    oracle = require_mapping(value, label=f"{lane} oracle")
    if lane == "workspace-all-features":
        require_exact_keys(
            oracle,
            {"kind"},
            label=f"{lane} oracle",
        )
        expected: Mapping[str, Any] = {"kind": "exit-and-profile-delta"}
    elif lane == "gui-semantic":
        require_exact_keys(
            oracle,
            {"kind", "total", "passed", "failed", "scenarios"},
            label=f"{lane} oracle",
        )
        expected = {
            "kind": "semantic-suite-json",
            "total": len(EXPECTED_SEMANTIC_SCENARIOS),
            "passed": len(EXPECTED_SEMANTIC_SCENARIOS),
            "failed": 0,
            "scenarios": list(EXPECTED_SEMANTIC_SCENARIOS),
        }
    elif lane == "compat-live-tls":
        require_exact_keys(
            oracle,
            {
                "kind",
                "passed",
                "failed",
                "ignored",
                "filtered_out",
                "tests",
                "skip_markers",
            },
            label=f"{lane} oracle",
        )
        expected = {
            "kind": "libtest-exact-live-tls",
            "passed": len(EXPECTED_COMPAT_TESTS),
            "failed": 0,
            "ignored": 0,
            "filtered_out": EXPECTED_COMPAT_FILTERED_OUT,
            "tests": list(EXPECTED_COMPAT_TESTS),
            "skip_markers": [],
        }
    else:
        require_exact_keys(
            oracle,
            {"kind", "summary_detected"},
            label=f"{lane} oracle",
        )
        expected = {
            "kind": "llvm-profile-merge",
            "summary_detected": True,
        }
    if dict(oracle) != dict(expected):
        raise CoverageProfileLaneError(
            f"{lane} oracle does not match the required complete result"
        )


def validate_report_document(document: Any) -> Mapping[str, Any]:
    report = require_mapping(document, label="coverage profile report")
    require_exact_keys(
        report,
        {
            "schema_version",
            "kind",
            "status",
            "producer",
            "legacy_reference",
            "instrumentation_environment",
            "profile_reset",
            "lane_order",
            "lanes",
            "errors",
        },
        label="coverage profile report",
    )
    if (
        report.get("schema_version") != SCHEMA_VERSION
        or report.get("kind") != REPORT_KIND
    ):
        raise CoverageProfileLaneError(
            "coverage profile report has an unsupported schema"
        )
    if report.get("status") != "passed":
        raise CoverageProfileLaneError(
            f"coverage profile report status is {report.get('status')!r}"
        )
    if require_list(report.get("errors"), label="report errors"):
        raise CoverageProfileLaneError(
            "passed coverage profile report must contain no errors"
        )

    producer = require_mapping(report.get("producer"), label="producer")
    require_exact_keys(
        producer,
        {"cargo_llvm_cov_version", "version_command"},
        label="producer",
    )
    if producer != {
        "cargo_llvm_cov_version": PINNED_CARGO_LLVM_COV_VERSION,
        "version_command": list(VERSION_COMMAND),
    }:
        raise CoverageProfileLaneError(
            "coverage profile report has unpinned producer metadata"
        )

    reference = require_mapping(
        report.get("legacy_reference"),
        label="legacy reference",
    )
    require_exact_keys(
        reference,
        {"path", "commit_sha"},
        label="legacy reference",
    )
    require_string(reference.get("path"), label="legacy reference path")
    if reference.get("commit_sha") != PINNED_LEGACY_SYNCPLAY_SHA:
        raise CoverageProfileLaneError(
            "coverage profile report uses an unpinned legacy reference"
        )

    instrumentation = require_mapping(
        report.get("instrumentation_environment"),
        label="instrumentation environment",
    )
    require_exact_keys(
        instrumentation,
        {
            "keys",
            "profile_pattern",
            "target_dir",
            "build_dir",
            "crate_count",
            "required_crates",
        },
        label="instrumentation environment",
    )
    if instrumentation.get("keys") != sorted(EXPECTED_SHOW_ENV_KEYS):
        raise CoverageProfileLaneError(
            "instrumentation environment key inventory drifted"
        )
    require_string(
        instrumentation.get("profile_pattern"),
        label="instrumentation profile pattern",
    )
    require_string(
        instrumentation.get("target_dir"),
        label="instrumentation target directory",
    )
    require_string(
        instrumentation.get("build_dir"),
        label="instrumentation build directory",
    )
    require_int(
        instrumentation.get("crate_count"),
        label="instrumentation crate count",
        minimum=len(REQUIRED_INSTRUMENTED_CRATES),
    )
    if instrumentation.get("required_crates") != sorted(
        REQUIRED_INSTRUMENTED_CRATES
    ):
        raise CoverageProfileLaneError(
            "instrumentation required-crate inventory drifted"
        )

    profile_reset = require_mapping(
        report.get("profile_reset"),
        label="profile reset",
    )
    require_exact_keys(
        profile_reset,
        {
            "kind",
            "profile_root",
            "removed_raw_profile_count",
            "removed_merged_profile_count",
            "remaining_raw_profile_count",
            "remaining_merged_profile_count",
        },
        label="profile reset",
    )
    if profile_reset.get("kind") != "fresh-profile-reset":
        raise CoverageProfileLaneError("coverage profile reset kind drifted")
    profile_root = require_string(
        profile_reset.get("profile_root"),
        label="profile reset root",
    )
    if pathlib.PurePosixPath(profile_root).parts[:1] != ("target",):
        raise CoverageProfileLaneError(
            "coverage profile reset root is not inside target"
        )
    for field in (
        "removed_raw_profile_count",
        "removed_merged_profile_count",
    ):
        require_int(
            profile_reset.get(field),
            label=f"profile reset {field}",
        )
    for field in (
        "remaining_raw_profile_count",
        "remaining_merged_profile_count",
    ):
        value = require_int(
            profile_reset.get(field),
            label=f"profile reset {field}",
        )
        if value != 0:
            raise CoverageProfileLaneError(
                "coverage profile reset retained stale profile artifacts"
            )

    if report.get("lane_order") != list(LANE_ORDER):
        raise CoverageProfileLaneError(
            "coverage profile lane order is incomplete or reordered"
        )
    lanes = require_mapping(report.get("lanes"), label="coverage lanes")
    require_exact_keys(lanes, set(LANE_ORDER), label="coverage lanes")
    previous_profile_count = 0
    for lane in LANE_ORDER:
        entry = require_mapping(lanes.get(lane), label=f"{lane} lane")
        require_exact_keys(
            entry,
            {
                "status",
                "command",
                "instrumentation",
                "environment_overrides",
                "exit_code",
                "duration_seconds",
                "stdout",
                "stderr",
                "profile_count_before",
                "profile_count_after",
                "profile_delta_count",
                "profile_deltas",
                "profile_removed_count",
                "oracle",
                "errors",
            },
            label=f"{lane} lane",
        )
        if entry.get("status") != "passed" or entry.get("exit_code") != 0:
            raise CoverageProfileLaneError(f"{lane} lane did not pass")
        if entry.get("command") != list(LANE_COMMANDS[lane]):
            raise CoverageProfileLaneError(f"{lane} command drifted")
        if entry.get("instrumentation") != LANE_INSTRUMENTATION[lane]:
            raise CoverageProfileLaneError(
                f"{lane} instrumentation contract drifted"
            )
        if entry.get("environment_overrides") != list(
            LANE_ENVIRONMENT_OVERRIDES[lane]
        ):
            raise CoverageProfileLaneError(
                f"{lane} environment override inventory drifted"
            )
        require_number(
            entry.get("duration_seconds"),
            label=f"{lane} duration",
        )
        validate_file_metadata(entry.get("stdout"), label=f"{lane} stdout")
        validate_file_metadata(entry.get("stderr"), label=f"{lane} stderr")
        profile_count_before = require_int(
            entry.get("profile_count_before"),
            label=f"{lane} profiles before",
        )
        profile_count_after = require_int(
            entry.get("profile_count_after"),
            label=f"{lane} profiles after",
        )
        if profile_count_before != previous_profile_count:
            raise CoverageProfileLaneError(
                f"{lane} profile inventory is not continuous with the "
                "preceding lane"
            )
        deltas = require_list(
            entry.get("profile_deltas"),
            label=f"{lane} profile deltas",
        )
        delta_count = require_int(
            entry.get("profile_delta_count"),
            label=f"{lane} profile delta count",
        )
        if delta_count != len(deltas):
            raise CoverageProfileLaneError(
                f"{lane} profile delta count does not match its inventory"
            )
        removed_count = require_int(
            entry.get("profile_removed_count"),
            label=f"{lane} removed profile count",
        )
        if removed_count != 0:
            raise CoverageProfileLaneError(
                f"{lane} removed profiles created by a preceding lane"
            )
        if lane in PROFILE_LANES and delta_count == 0:
            raise CoverageProfileLaneError(
                f"{lane} produced no instrumented profile delta"
            )
        if lane in PROFILE_LANES and profile_count_after == 0:
            raise CoverageProfileLaneError(
                f"{lane} retained no instrumented profiles"
            )
        if lane == "merge-check" and (
            profile_count_after != profile_count_before or delta_count != 0
        ):
            raise CoverageProfileLaneError(
                "merge check unexpectedly changed the raw profile inventory"
            )
        for index, delta in enumerate(deltas):
            validate_profile_delta(
                delta,
                label=f"{lane} profile delta {index}",
            )
        if require_list(entry.get("errors"), label=f"{lane} errors"):
            raise CoverageProfileLaneError(
                f"passed {lane} lane must contain no errors"
            )
        validate_oracle(lane, entry.get("oracle"))
        previous_profile_count = profile_count_after
    return report


def strict_load_report(path: pathlib.Path) -> Mapping[str, Any]:
    try:
        size = path.stat().st_size
    except OSError as error:
        raise CoverageProfileLaneError(
            f"cannot stat coverage profile report {path}: {error}"
        ) from error
    if size > MAX_REPORT_BYTES:
        raise CoverageProfileLaneError(
            f"coverage profile report exceeds {MAX_REPORT_BYTES} bytes"
        )
    try:
        data = path.read_bytes()
    except OSError as error:
        raise CoverageProfileLaneError(
            f"cannot read coverage profile report {path}: {error}"
        ) from error
    return validate_report_document(parse_json(data, label="coverage profile report"))


def atomic_write_json(path: pathlib.Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = (
        json.dumps(
            value,
            indent=2,
            sort_keys=True,
            ensure_ascii=False,
            allow_nan=False,
        )
        + "\n"
    ).encode("utf-8")
    if len(data) > MAX_REPORT_BYTES:
        raise CoverageProfileLaneError(
            f"coverage profile report exceeds {MAX_REPORT_BYTES} bytes"
        )
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_bytes(data)
    os.replace(temporary, path)


def verify_legacy_reference(
    *,
    repo_root: pathlib.Path,
    environment: Mapping[str, str],
) -> dict[str, str]:
    configured = environment.get("SYNCPLAY_LEGACY_ROOT")
    if not configured:
        raise CoverageProfileLaneError(
            "SYNCPLAY_LEGACY_ROOT must identify the pinned legacy checkout"
        )
    path = resolve_within(
        repo_root,
        pathlib.Path(configured),
        label="legacy Syncplay checkout",
    )
    process = subprocess.run(
        ["git", "-C", str(path), "rev-parse", "HEAD"],
        cwd=repo_root,
        env=dict(environment),
        capture_output=True,
        check=False,
    )
    if process.returncode != 0:
        raise CoverageProfileLaneError(
            "legacy Syncplay checkout is missing or is not a Git worktree"
        )
    try:
        commit = process.stdout.decode("ascii", errors="strict").strip()
    except UnicodeError as error:
        raise CoverageProfileLaneError(
            f"legacy Syncplay revision is not ASCII: {error}"
        ) from error
    if commit != PINNED_LEGACY_SYNCPLAY_SHA or not FULL_SHA.fullmatch(commit):
        raise CoverageProfileLaneError(
            "legacy Syncplay checkout must be pinned to "
            f"{PINNED_LEGACY_SYNCPLAY_SHA}, received {commit!r}"
        )
    return {
        "path": repo_relative(repo_root, path),
        "commit_sha": commit,
    }


def failed_report() -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": REPORT_KIND,
        "status": "failed",
        "producer": None,
        "legacy_reference": None,
        "instrumentation_environment": None,
        "profile_reset": None,
        "lane_order": list(LANE_ORDER),
        "lanes": {},
        "errors": [],
    }


def run_collection(args: argparse.Namespace) -> int:
    repo_root = pathlib.Path(args.repo_root).resolve()
    output = resolve_within(
        repo_root,
        pathlib.Path(args.output),
        label="coverage profile report output",
    )
    log_root = output.parent / "coverage-profile-logs"
    environment = dict(os.environ)
    report = failed_report()

    try:
        if not (repo_root / "Cargo.toml").is_file():
            raise CoverageProfileLaneError(
                f"repository root has no Cargo.toml: {repo_root}"
            )
        report["legacy_reference"] = verify_legacy_reference(
            repo_root=repo_root,
            environment=environment,
        )
        version_result = run_command(
            VERSION_COMMAND,
            cwd=repo_root,
            environment=environment,
        )
        version = parse_producer_version(version_result)
        report["producer"] = {
            "cargo_llvm_cov_version": version,
            "version_command": list(VERSION_COMMAND),
        }
        show_env_result = run_command(
            SHOW_ENV_COMMAND,
            cwd=repo_root,
            environment=environment,
        )
        instrumentation_env, instrumentation_summary, profile_root = parse_show_env(
            show_env_result,
            repo_root=repo_root,
        )
        report["instrumentation_environment"] = instrumentation_summary
        report["profile_reset"] = reset_profile_artifacts(
            profile_root,
            repo_root=repo_root,
        )

        workspace_record, _ = execute_lane(
            lane="workspace-all-features",
            repo_root=repo_root,
            profile_root=profile_root,
            environment=environment,
            log_root=log_root,
            oracle_builder=lambda _stdout, _stderr: {
                "kind": "exit-and-profile-delta"
            },
            require_profile_delta=True,
        )
        report["lanes"]["workspace-all-features"] = workspace_record
        if workspace_record["status"] != "passed":
            raise CoverageProfileLaneError(
                "workspace all-feature coverage lane failed"
            )

        instrumented_environment = dict(environment)
        instrumented_environment.update(instrumentation_env)
        # RUSTC_WRAPPER is not part of Cargo's ordinary artifact freshness
        # fingerprint. Reusing target/debug can therefore run a previously
        # built, uninstrumented binary even though show-env is present. Pin all
        # external lanes to cargo-llvm-cov's isolated build directory, which
        # the workspace lane populated with the same instrumentation contract.
        instrumented_environment["CARGO_TARGET_DIR"] = str(
            profile_root / "llvm-cov-target"
        )
        with tempfile.TemporaryDirectory(
            prefix="sorotte-coverage-semantic-"
        ) as temporary:
            semantic_environment = dict(instrumented_environment)
            semantic_environment["SOROTTE_CLIENT_CONFIG_PATH"] = str(
                pathlib.Path(temporary) / "sorotte.ini"
            )
            semantic_record, _ = execute_lane(
                lane="gui-semantic",
                repo_root=repo_root,
                profile_root=profile_root,
                environment=semantic_environment,
                log_root=log_root,
                oracle_builder=lambda stdout, _stderr: semantic_oracle(stdout),
                require_profile_delta=True,
            )
        report["lanes"]["gui-semantic"] = semantic_record
        if semantic_record["status"] != "passed":
            raise CoverageProfileLaneError("GUI semantic coverage lane failed")

        compatibility_environment = dict(instrumented_environment)
        compatibility_environment.update(
            {
                "SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY": "1",
                "SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY": "1",
            }
        )
        compat_record, _ = execute_lane(
            lane="compat-live-tls",
            repo_root=repo_root,
            profile_root=profile_root,
            environment=compatibility_environment,
            log_root=log_root,
            oracle_builder=compatibility_oracle,
            require_profile_delta=True,
        )
        report["lanes"]["compat-live-tls"] = compat_record
        if compat_record["status"] != "passed":
            raise CoverageProfileLaneError(
                "strict live TLS compatibility coverage lane failed"
            )

        merge_record, _ = execute_lane(
            lane="merge-check",
            repo_root=repo_root,
            profile_root=profile_root,
            environment=environment,
            log_root=log_root,
            oracle_builder=merge_oracle,
            require_profile_delta=False,
        )
        report["lanes"]["merge-check"] = merge_record
        if merge_record["status"] != "passed":
            raise CoverageProfileLaneError(
                "LLVM profile compatibility merge check failed"
            )

        report["status"] = "passed"
        validate_report_document(report)
    except (CoverageProfileLaneError, OSError) as error:
        report["status"] = "failed"
        report["errors"].append(str(error))
        try:
            atomic_write_json(output, report)
        except (CoverageProfileLaneError, OSError) as write_error:
            print(
                f"coverage profile collection failed and report could not be "
                f"written: {write_error}",
                file=sys.stderr,
            )
            return 2
        print(
            f"coverage profile collection failed: {error}",
            file=sys.stderr,
        )
        return 1

    atomic_write_json(output, report)
    print(f"merged coverage profile lanes passed: {output}")
    return 0


def validate_command(args: argparse.Namespace) -> int:
    try:
        strict_load_report(pathlib.Path(args.report))
    except CoverageProfileLaneError as error:
        print(f"coverage profile report validation failed: {error}", file=sys.stderr)
        return 1
    print(f"coverage profile report passed: {args.report}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    run = subcommands.add_parser("run")
    run.add_argument("--repo-root", required=True)
    run.add_argument("--output", required=True)
    run.set_defaults(func=run_collection)

    validate = subcommands.add_parser("validate")
    validate.add_argument("--report", required=True)
    validate.set_defaults(func=validate_command)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
