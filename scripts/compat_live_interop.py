#!/usr/bin/env python3
"""Run and attest the complete live Python compatibility matrix.

The Rust compatibility tests intentionally treat the Python reference
implementation as optional during ordinary developer runs.  This wrapper is
the only CI/release entry point for the required-live contract: it verifies the
pinned oracle and prerequisite identities, discovers the complete test and
ignored inventories without accepting selectors, executes the matrix once,
and fails if any test silently takes an optional skip path.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time
import tomllib
from collections.abc import Mapping, Sequence
from typing import Any


SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-compat-live-interop"
REQUIRED_ENVIRONMENT_VARIABLE = "SYNCPLAY_REQUIRE_LIVE_INTEROP"
PINNED_LEGACY_SYNCPLAY_SHA = "d1c5f85af377c960c5a940707c4d01bc84fd9c3f"
PINNED_LEGACY_SYNCPLAY_REPOSITORY = "Syncplay/syncplay"
SUPPORTED_PYTHON_MINIMUM = (3, 11)
SUPPORTED_PYTHON_MAXIMUM_EXCLUSIVE = (3, 14)
PINNED_PACKAGES = {
    "cryptography": ("cryptography", "50.0.1"),
    "pyopenssl": ("pyopenssl", "26.4.0"),
    "service-identity": ("service_identity", "24.2.0"),
    "twisted": ("twisted", "26.4.0"),
}
FIXTURE_ROOT_COUNTS = {
    "fixtures/protocol": 24,
    "fixtures/scenarios": 62,
    "fixtures/tls": 3,
}
PROBE_PATHS = (
    "crates/sorotte-compat/scripts/python_handshake_probe.py",
    "crates/sorotte-compat/scripts/python_live_peer_probe.py",
)
EXPECTED_IGNORED_TESTS = {
    "tests::fixture_tests::capture_live_reference_controlled_room_trace_fixtures": (
        "requires Twisted and writes fixture files from a live legacy server session"
    ),
    "tests::fixture_tests::capture_live_reference_state_latency_metrics_trace_fixture": (
        "requires Twisted and writes fixture files from a live legacy server session"
    ),
    "tests::fixture_tests::capture_permanent_rooms_file_trace_fixtures": (
        "writes permanent-rooms-file python/legacy trace fixtures"
    ),
    "tests::fixture_tests::capture_persistent_rooms_lifecycle_trace_fixtures": (
        "writes persistent-room lifecycle python/legacy trace fixtures"
    ),
    "tests::fixture_tests::capture_persistent_rooms_timeout_list_updates_trace_fixtures": (
        "writes persistent timeout-list-updates python/legacy trace fixtures"
    ),
    "tests::fixture_tests::capture_python_fanout_trace_fixtures": (
        "writes python fanout trace fixtures from current probe behavior"
    ),
    "tests::fixture_tests::capture_python_state_latency_metrics_trace_fixture": (
        "writes python fanout trace fixtures from current probe behavior"
    ),
}
REQUIRED_LIVE_SENTINELS = frozenset(
    {
        "tests::controlled_room_fanout_tests::legacy_server_fanout_roundtrip_matches_server_runtime_on_controlled_room_permissions_scenario",
        "tests::legacy_client_contract_tests::legacy_client_chat_send_contract_matches_client_core_behavior",
        "tests::legacy_tls_tests::legacy_server_live_tls_upgrade_roundtrip_supports_post_upgrade_hello_over_same_socket",
        "tests::python_protocol_tests::generated_json_framing_matches_pinned_python_oracle",
        "tests::python_protocol_tests::python_interop_roundtrip_returns_server_hello",
        "tests::state_fanout_tests::python_state_tests::python_fanout_roundtrip_matches_server_runtime_on_fanout_scenario",
    }
)
try:
    from test_inventory import reviewed as reviewed_tests
except ModuleNotFoundError:
    from scripts.test_inventory import reviewed as reviewed_tests

EXPECTED_DISCOVERED_TESTS = len(reviewed_tests("compat"))
MAX_REPORT_BYTES = 8 * 1024 * 1024
MAX_LOG_BYTES = 64 * 1024 * 1024
COMMAND_TIMEOUT_SECONDS = 15 * 60
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
TEST_LIST_LINE = re.compile(r"^(?P<name>[^:\r\n](?:.*[^:\r\n])?): test$")
TEST_RESULT_LINE = re.compile(
    r"^test (?P<name>.+) \.\.\. (?P<status>ok|FAILED|ignored)"
    r"(?:, (?P<reason>.+))?$"
)
CAPTURE_HEADER = re.compile(r"^---- (?P<name>.+) stdout ----$")
REQUIREMENT_LINE = re.compile(
    r"^(?P<name>[A-Za-z0-9][A-Za-z0-9_.-]*)==(?P<version>[A-Za-z0-9][A-Za-z0-9_.+-]*)$"
)

BASE_CARGO_COMMAND = (
    "cargo",
    "test",
    "--locked",
    "-p",
    "sorotte-compat",
    "--all-features",
)
LIST_COMMAND = BASE_CARGO_COMMAND + ("--", "--list", "--format", "terse")
IGNORED_LIST_COMMAND = BASE_CARGO_COMMAND + (
    "--",
    "--ignored",
    "--list",
    "--format",
    "terse",
)
TEST_COMMAND = BASE_CARGO_COMMAND + (
    "--",
    "--test-threads=1",
    "--show-output",
)
SKIP_REASON_CODES = frozenset(
    {
        "assertion-disabled",
        "missing-fixture",
        "missing-local-prerequisite",
        "missing-oracle-root",
        "missing-prerequisite",
        "missing-python",
        "missing-python-package",
    }
)


class InteropContractError(RuntimeError):
    """The required-live producer or evidence contract is invalid."""


class PrerequisiteUnavailable(InteropContractError):
    """A prerequisite is unavailable and may be skipped only in optional mode."""

    def __init__(self, code: str, reason: str) -> None:
        if code not in SKIP_REASON_CODES:
            raise ValueError(f"unknown prerequisite reason code: {code}")
        super().__init__(reason)
        self.code = code
        self.reason = reason


@dataclasses.dataclass(frozen=True)
class CommandResult:
    command: tuple[str, ...]
    returncode: int
    stdout: bytes
    stderr: bytes
    duration_seconds: float


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def normalize_package_name(value: str) -> str:
    return re.sub(r"[-_.]+", "-", value).lower()


def reject_duplicate_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise InteropContractError(f"JSON object contains duplicate key {key!r}")
        value[key] = item
    return value


def reject_json_constant(value: str) -> None:
    raise InteropContractError(
        f"JSON contains non-standard numeric constant {value!r}"
    )


def strict_parse_json(data: bytes, *, label: str) -> Any:
    try:
        text = data.decode("utf-8", errors="strict")
        return json.loads(
            text,
            object_pairs_hook=reject_duplicate_pairs,
            parse_constant=reject_json_constant,
        )
    except (UnicodeError, json.JSONDecodeError) as error:
        raise InteropContractError(f"{label} is not strict UTF-8 JSON: {error}") from error


def require_object(value: Any, *, label: str) -> Mapping[str, Any]:
    if not isinstance(value, dict):
        raise InteropContractError(f"{label} must be an object")
    return value


def require_list(value: Any, *, label: str) -> list[Any]:
    if not isinstance(value, list):
        raise InteropContractError(f"{label} must be an array")
    return value


def require_string(value: Any, *, label: str) -> str:
    if not isinstance(value, str):
        raise InteropContractError(f"{label} must be a string")
    return value


def require_nonnegative_int(value: Any, *, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise InteropContractError(f"{label} must be a non-negative integer")
    return value


def require_exact_keys(
    value: Mapping[str, Any], expected: set[str], *, label: str
) -> None:
    actual = set(value)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise InteropContractError(
            f"{label} keys differ: missing={missing}, extra={extra}"
        )


def resolve_within(
    repo_root: pathlib.Path, path: pathlib.Path, *, label: str
) -> pathlib.Path:
    resolved = path.resolve() if path.is_absolute() else (repo_root / path).resolve()
    try:
        resolved.relative_to(repo_root)
    except ValueError as error:
        raise InteropContractError(
            f"{label} must remain inside the repository"
        ) from error
    return resolved


def repo_relative(repo_root: pathlib.Path, path: pathlib.Path) -> str:
    return path.resolve().relative_to(repo_root).as_posix()


def run_command(
    command: Sequence[str],
    *,
    cwd: pathlib.Path,
    environment: Mapping[str, str],
    timeout_seconds: int = COMMAND_TIMEOUT_SECONDS,
) -> CommandResult:
    started = time.monotonic()
    try:
        process = subprocess.run(
            list(command),
            cwd=cwd,
            env=dict(environment),
            capture_output=True,
            check=False,
            timeout=timeout_seconds,
        )
    except FileNotFoundError as error:
        raise PrerequisiteUnavailable(
            "missing-prerequisite",
            f"command executable is unavailable: {command[0]}",
        ) from error
    except subprocess.TimeoutExpired as error:
        raise InteropContractError(
            f"command exceeded the {timeout_seconds}-second bound: {command[0]}"
        ) from error
    return CommandResult(
        command=tuple(command),
        returncode=process.returncode,
        stdout=process.stdout,
        stderr=process.stderr,
        duration_seconds=time.monotonic() - started,
    )


def require_success(result: CommandResult, *, label: str) -> CommandResult:
    if result.returncode != 0:
        raise InteropContractError(
            f"{label} failed with exit code {result.returncode}"
        )
    return result


def git_text(
    repo_root: pathlib.Path,
    arguments: Sequence[str],
    *,
    environment: Mapping[str, str],
    label: str,
) -> str:
    result = require_success(
        run_command(
            ("git", *arguments),
            cwd=repo_root,
            environment=environment,
            timeout_seconds=60,
        ),
        label=label,
    )
    try:
        return result.stdout.decode("utf-8", errors="strict").strip()
    except UnicodeError as error:
        raise InteropContractError(f"{label} output is not UTF-8") from error


def verify_source(
    repo_root: pathlib.Path, environment: Mapping[str, str]
) -> dict[str, str]:
    commit = git_text(
        repo_root,
        ("rev-parse", "HEAD^{commit}"),
        environment=environment,
        label="source revision query",
    )
    if not FULL_SHA.fullmatch(commit):
        raise InteropContractError(f"source revision is not a full commit SHA: {commit!r}")
    expected = environment.get("GITHUB_SHA", commit)
    if not FULL_SHA.fullmatch(expected):
        raise InteropContractError("GITHUB_SHA is not a full lowercase commit SHA")
    if commit != expected:
        raise InteropContractError(
            f"source revision {commit} does not match expected revision {expected}"
        )
    return {"commit_sha": commit, "expected_commit_sha": expected}


def verify_oracle(
    repo_root: pathlib.Path, environment: Mapping[str, str]
) -> dict[str, str]:
    configured = environment.get("SYNCPLAY_LEGACY_ROOT", "").strip()
    if not configured:
        raise PrerequisiteUnavailable(
            "missing-oracle-root",
            "SYNCPLAY_LEGACY_ROOT does not identify the pinned local oracle",
        )
    oracle = resolve_within(
        repo_root,
        pathlib.Path(configured),
        label="legacy Syncplay oracle",
    )
    if not oracle.is_dir() or not (oracle / "syncplayServer.py").is_file():
        raise PrerequisiteUnavailable(
            "missing-oracle-root",
            "configured legacy Syncplay oracle is missing syncplayServer.py",
        )
    observed = git_text(
        repo_root,
        ("-C", str(oracle), "rev-parse", "HEAD^{commit}"),
        environment=environment,
        label="legacy Syncplay oracle revision query",
    )
    if observed != PINNED_LEGACY_SYNCPLAY_SHA:
        raise InteropContractError(
            "legacy Syncplay oracle must be pinned to "
            f"{PINNED_LEGACY_SYNCPLAY_SHA}, received {observed!r}"
        )
    return {
        "path": repo_relative(repo_root, oracle),
        "repository": PINNED_LEGACY_SYNCPLAY_REPOSITORY,
        "expected_commit_sha": PINNED_LEGACY_SYNCPLAY_SHA,
        "observed_commit_sha": observed,
    }


def parse_pinned_requirements(data: bytes) -> dict[str, tuple[str, str]]:
    try:
        text = data.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise InteropContractError("legacy Python requirements are not UTF-8") from error
    packages: dict[str, tuple[str, str]] = {}
    constraints_seen = False
    for line_number, raw_line in enumerate(text.splitlines(), start=1):
        line = raw_line.split("#", 1)[0].strip()
        if not line:
            continue
        if line == "-c verification-constraints.txt":
            if constraints_seen:
                raise InteropContractError("duplicate legacy Python constraints input")
            constraints_seen = True
            continue
        match = REQUIREMENT_LINE.fullmatch(line)
        if match is None:
            raise InteropContractError(
                "legacy Python requirement line "
                f"{line_number} must be an exact name==version pin"
            )
        display_name = match.group("name")
        normalized = normalize_package_name(display_name)
        if normalized in packages:
            raise InteropContractError(
                f"duplicate legacy Python requirement {display_name!r}"
            )
        packages[normalized] = (display_name, match.group("version"))
    if packages != PINNED_PACKAGES:
        raise InteropContractError(
            "legacy Python requirement pins differ from the closed prerequisite set"
        )
    return packages


def verify_python_constraints(repo_root: pathlib.Path, requirement_bytes: bytes) -> None:
    """Bind the additive constraint input without changing historical report v1."""
    if not any(line.split("#", 1)[0].strip() == "-c verification-constraints.txt"
               for line in requirement_bytes.decode("utf-8").splitlines()):
        return
    constraints = repo_root / "requirements" / "verification-constraints.txt"
    manifest = repo_root / "coverage" / "verification-tools.toml"
    try:
        if constraints.is_symlink() or manifest.is_symlink():
            raise ValueError("indirect constraint input")
        policy = tomllib.loads(manifest.read_text(encoding="utf-8"))["python-resolution"]
        if policy["constraints"] != "requirements/verification-constraints.txt":
            raise ValueError("constraint path differs from the reviewed input")
        observed = sha256(constraints.read_text(encoding="utf-8").encode("utf-8"))
        if observed != policy["constraints-lf-sha256"]:
            raise ValueError("constraints differ from the reviewed digest")
    except (OSError, UnicodeError, ValueError, KeyError, TypeError) as error:
        raise InteropContractError(f"legacy Python constraints are unavailable or changed: {error}") from error


def verify_python(
    repo_root: pathlib.Path, environment: Mapping[str, str]
) -> tuple[dict[str, Any], dict[str, Any]]:
    requirements_path = repo_root / "requirements" / "legacy-python-interop.txt"
    try:
        requirement_bytes = requirements_path.read_bytes()
    except FileNotFoundError as error:
        raise PrerequisiteUnavailable(
            "missing-python-package",
            "pinned legacy Python requirements file is missing",
        ) from error
    packages = parse_pinned_requirements(requirement_bytes)
    verify_python_constraints(repo_root, requirement_bytes)

    python_command = environment.get("SYNCPLAY_PYTHON_BIN", sys.executable).strip()
    if not python_command:
        raise PrerequisiteUnavailable(
            "missing-python",
            "SYNCPLAY_PYTHON_BIN is empty",
        )
    probe = r"""
import importlib.metadata
import json
import os
import platform
import sys

names = json.loads(sys.argv[1])
packages = {}
for name in names:
    try:
        packages[name] = importlib.metadata.version(name)
    except importlib.metadata.PackageNotFoundError:
        packages[name] = None
print(json.dumps({
    "executable": os.path.realpath(sys.executable),
    "implementation": platform.python_implementation(),
    "packages": packages,
    "version": platform.python_version(),
    "version_info": list(sys.version_info[:3]),
}, sort_keys=True))
"""
    try:
        result = subprocess.run(
            [python_command, "-c", probe, json.dumps(sorted(packages))],
            cwd=repo_root,
            env=dict(environment),
            capture_output=True,
            check=False,
            timeout=60,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired) as error:
        raise PrerequisiteUnavailable(
            "missing-python",
            "configured Python interpreter is unavailable",
        ) from error
    if result.returncode != 0:
        raise PrerequisiteUnavailable(
            "missing-python",
            f"configured Python identity probe exited with code {result.returncode}",
        )
    document = require_object(
        strict_parse_json(result.stdout, label="Python identity probe"),
        label="Python identity probe",
    )
    require_exact_keys(
        document,
        {"executable", "implementation", "packages", "version", "version_info"},
        label="Python identity probe",
    )
    implementation = require_string(
        document["implementation"], label="Python implementation"
    )
    if implementation != "CPython":
        raise InteropContractError(
            f"live interoperability requires CPython, received {implementation!r}"
        )
    version_info = require_list(document["version_info"], label="Python version_info")
    if (
        len(version_info) != 3
        or any(not isinstance(item, int) or isinstance(item, bool) for item in version_info)
    ):
        raise InteropContractError("Python version_info must contain three integers")
    family = tuple(version_info[:2])
    if not (
        SUPPORTED_PYTHON_MINIMUM
        <= family
        < SUPPORTED_PYTHON_MAXIMUM_EXCLUSIVE
    ):
        raise InteropContractError(
            "Python version must be >=3.11 and <3.14; "
            f"received {require_string(document['version'], label='Python version')}"
        )
    observed_packages = require_object(
        document["packages"], label="Python package identities"
    )
    if set(observed_packages) != set(packages):
        raise InteropContractError("Python identity probe package inventory drifted")
    package_records = []
    for normalized, (display_name, expected_version) in sorted(packages.items()):
        observed = observed_packages[normalized]
        if observed is None:
            raise PrerequisiteUnavailable(
                "missing-python-package",
                f"required Python package {display_name} is not installed",
            )
        observed_version = require_string(
            observed, label=f"Python package {display_name} version"
        )
        if observed_version != expected_version:
            raise InteropContractError(
                f"Python package {display_name} must be {expected_version}, "
                f"received {observed_version}"
            )
        package_records.append(
            {
                "name": display_name,
                "expected_version": expected_version,
                "observed_version": observed_version,
            }
        )

    python_record = {
        "command": python_command,
        "executable": require_string(
            document["executable"], label="Python executable"
        ),
        "implementation": implementation,
        "version": require_string(document["version"], label="Python version"),
        "version_info": version_info,
        "supported_family": ">=3.11,<3.14",
        "packages": package_records,
    }
    requirements_record = {
        "path": repo_relative(repo_root, requirements_path),
        "sha256": sha256(requirement_bytes),
        "packages": [
            {"name": display, "version": version}
            for _, (display, version) in sorted(packages.items())
        ],
    }
    return python_record, requirements_record


def file_record(repo_root: pathlib.Path, path: pathlib.Path) -> dict[str, Any]:
    try:
        data = path.read_bytes()
    except FileNotFoundError as error:
        raise PrerequisiteUnavailable(
            "missing-fixture",
            f"required tracked file is missing: {repo_relative(repo_root, path)}",
        ) from error
    return {
        "path": repo_relative(repo_root, path),
        "sha256": sha256(data),
        "size_bytes": len(data),
    }


def verify_fixtures(
    repo_root: pathlib.Path, environment: Mapping[str, str]
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    result = require_success(
        run_command(
            (
                "git",
                "ls-files",
                "-z",
                "--",
                *FIXTURE_ROOT_COUNTS,
            ),
            cwd=repo_root,
            environment=environment,
            timeout_seconds=60,
        ),
        label="tracked compatibility fixture inventory",
    )
    try:
        tracked = [
            item.decode("utf-8", errors="strict")
            for item in result.stdout.split(b"\0")
            if item
        ]
    except UnicodeError as error:
        raise InteropContractError("tracked fixture path is not UTF-8") from error
    if tracked != sorted(tracked) or len(tracked) != len(set(tracked)):
        raise InteropContractError(
            "tracked compatibility fixture inventory is not unique and sorted"
        )
    counts: dict[str, int] = {}
    for root, expected_count in FIXTURE_ROOT_COUNTS.items():
        prefix = root + "/"
        count = sum(path.startswith(prefix) for path in tracked)
        if count != expected_count:
            raise InteropContractError(
                f"tracked fixture root {root} must contain exactly "
                f"{expected_count} files, received {count}"
            )
        counts[root] = count
    records = [file_record(repo_root, repo_root / path) for path in tracked]
    digest = hashlib.sha256()
    for record in records:
        digest.update(record["path"].encode("utf-8"))
        digest.update(b"\0")
        digest.update(record["sha256"].encode("ascii"))
        digest.update(b"\0")
        digest.update(str(record["size_bytes"]).encode("ascii"))
        digest.update(b"\n")
    probes = [file_record(repo_root, repo_root / path) for path in PROBE_PATHS]
    return (
        {
            "roots": list(FIXTURE_ROOT_COUNTS),
            "counts": counts,
            "file_count": len(records),
            "manifest_sha256": digest.hexdigest(),
            "files": records,
        },
        probes,
    )


def parse_test_list(data: bytes, *, label: str) -> list[str]:
    try:
        text = data.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise InteropContractError(f"{label} is not UTF-8") from error
    tests = [
        match.group("name")
        for line in text.splitlines()
        if (match := TEST_LIST_LINE.fullmatch(line.strip())) is not None
    ]
    if not tests:
        raise InteropContractError(f"{label} contains no tests")
    if tests != sorted(tests) or len(tests) != len(set(tests)):
        raise InteropContractError(f"{label} must be unique and sorted")
    return tests


def verify_inventory(
    list_result: CommandResult, ignored_list_result: CommandResult
) -> dict[str, Any]:
    require_success(list_result, label="complete compatibility test listing")
    require_success(ignored_list_result, label="ignored compatibility test listing")
    listed = parse_test_list(list_result.stdout, label="complete test listing")
    ignored = parse_test_list(
        ignored_list_result.stdout, label="ignored test listing"
    )
    if len(listed) != EXPECTED_DISCOVERED_TESTS:
        raise InteropContractError(
            "complete compatibility inventory differs from the source-bound "
            f"expectation: {len(listed)} != {EXPECTED_DISCOVERED_TESTS}"
        )
    missing_sentinels = sorted(REQUIRED_LIVE_SENTINELS - set(listed))
    if missing_sentinels:
        raise InteropContractError(
            f"complete compatibility inventory omits live sentinels {missing_sentinels}"
        )
    if listed != reviewed_tests("compat"):
        raise InteropContractError("complete compatibility inventory changed required test identities; run test_inventory.py propose and review its diff")
    if ignored != sorted(EXPECTED_IGNORED_TESTS):
        raise InteropContractError(
            "ignored compatibility inventory differs from the exact fixture-generator set"
        )
    return {
        "listed_count": len(listed),
        "listed_tests": listed,
        "ignored_count": len(ignored),
        "ignored_tests": [
            {"test": name, "reason": EXPECTED_IGNORED_TESTS[name]}
            for name in ignored
        ],
    }


def classify_skip_reason(line: str) -> tuple[str, str] | None:
    normalized = " ".join(line.strip().split())
    lowered = normalized.lower()
    if "skipped" not in lowered:
        return None
    if "parity assertion skipped" in lowered:
        return "assertion-disabled", normalized
    if "skipped due to missing local prerequisites" in lowered:
        return "missing-local-prerequisite", normalized
    if "skipped due to missing prerequisites" in lowered:
        return "missing-prerequisite", normalized
    raise InteropContractError(
        f"compatibility output contains an unclassified skip reason: {normalized}"
    )


def captured_skip_events(data: bytes, *, stream: str) -> list[dict[str, Any]]:
    try:
        text = data.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise InteropContractError(f"compatibility {stream} is not UTF-8") from error
    current_test: str | None = None
    events: list[dict[str, Any]] = []
    for line in text.splitlines():
        header = CAPTURE_HEADER.fullmatch(line.strip())
        if header is not None:
            current_test = header.group("name")
            continue
        classified = classify_skip_reason(line)
        if classified is None:
            continue
        code, reason = classified
        if current_test is None:
            raise InteropContractError(
                f"{stream} skip reason is not bound to a captured test section"
            )
        events.append(
            {
                "scope": "test",
                "test": current_test,
                "code": code,
                "reason": reason,
            }
        )
    return events


def parse_test_results(data: bytes) -> dict[str, tuple[str, str | None]]:
    try:
        text = data.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise InteropContractError("compatibility stdout is not UTF-8") from error
    results: dict[str, tuple[str, str | None]] = {}
    for line in text.splitlines():
        match = TEST_RESULT_LINE.fullmatch(line.strip())
        if match is None:
            continue
        name = match.group("name")
        if name in results:
            raise InteropContractError(
                f"compatibility result contains duplicate test {name!r}"
            )
        status = match.group("status")
        reason = match.group("reason")
        if status == "ignored":
            expected_reason = EXPECTED_IGNORED_TESTS.get(name)
            if reason != expected_reason:
                raise InteropContractError(
                    f"ignored test {name!r} reason differs from the closed inventory"
                )
        elif reason is not None:
            raise InteropContractError(
                f"non-ignored test {name!r} has an unexpected result suffix"
            )
        results[name] = (status, reason)
    if not results:
        raise InteropContractError("compatibility output contains no test results")
    return results


def account_execution(
    inventory: Mapping[str, Any],
    result: CommandResult,
) -> dict[str, Any]:
    results = parse_test_results(result.stdout)
    listed = set(inventory["listed_tests"])
    if set(results) != listed:
        raise InteropContractError(
            "executed compatibility inventory differs from the complete listing: "
            f"missing={sorted(listed - set(results))}, "
            f"extra={sorted(set(results) - listed)}"
        )
    expected_ignored = set(EXPECTED_IGNORED_TESTS)
    observed_ignored = {
        name for name, (status, _) in results.items() if status == "ignored"
    }
    if observed_ignored != expected_ignored:
        raise InteropContractError(
            "executed ignored inventory differs from the exact fixture-generator set"
        )
    skip_events = captured_skip_events(result.stdout, stream="stdout")
    skip_events.extend(captured_skip_events(result.stderr, stream="stderr"))
    skip_tests = [event["test"] for event in skip_events]
    if len(skip_tests) != len(set(skip_tests)):
        raise InteropContractError(
            "a compatibility test emitted more than one skip classification"
        )
    if not set(skip_tests).issubset(listed - expected_ignored):
        raise InteropContractError(
            "skip accounting references an unknown or ignored compatibility test"
        )
    failed = sorted(
        name for name, (status, _) in results.items() if status == "FAILED"
    )
    raw_passed = {
        name for name, (status, _) in results.items() if status == "ok"
    }
    if not set(skip_tests).issubset(raw_passed):
        raise InteropContractError(
            "skip accounting must reference libtest-success early returns"
        )
    passed = sorted(raw_passed - set(skip_tests))
    executed_count = len(passed) + len(failed)
    accounting = {
        "complete": True,
        "executed_count": executed_count,
        "passed_count": len(passed),
        "failed_count": len(failed),
        "skipped_count": len(skip_events),
        "ignored_count": len(observed_ignored),
        "executed_tests": sorted([*passed, *failed]),
        "failed_tests": failed,
        "skipped": skip_events,
    }
    return accounting


def execution_failures(
    accounting: Mapping[str, Any],
    result: CommandResult,
    *,
    required: bool,
) -> list[str]:
    errors: list[str] = []
    if result.returncode != 0:
        errors.append(
            f"complete compatibility matrix exited with code {result.returncode}"
        )
    if accounting["failed_tests"]:
        errors.append(
            "complete compatibility matrix failed tests "
            f"{accounting['failed_tests']}"
        )
    if required and accounting["skipped"]:
        errors.append(
            "required-live compatibility matrix took optional skip paths: "
            + ", ".join(
                f"{event['test']} ({event['code']})"
                for event in accounting["skipped"]
            )
        )
    return errors


def build_execution_environment(
    *,
    repo_root: pathlib.Path,
    environment: Mapping[str, str],
    oracle: Mapping[str, Any],
    python_record: Mapping[str, Any],
    required: bool,
) -> dict[str, str]:
    execution_environment = dict(environment)
    oracle_path = resolve_within(
        repo_root,
        pathlib.Path(require_string(oracle["path"], label="oracle path")),
        label="legacy Syncplay oracle execution path",
    )
    execution_environment["SYNCPLAY_LEGACY_ROOT"] = str(oracle_path)
    execution_environment["SYNCPLAY_PYTHON_BIN"] = require_string(
        python_record["executable"],
        label="Python executable",
    )
    if required:
        execution_environment["SYNCPLAY_REQUIRE_LIVE_INTEROP"] = "1"
        execution_environment["SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY"] = "1"
        execution_environment["SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY"] = "1"
    return execution_environment


def artifact_record(
    repo_root: pathlib.Path, path: pathlib.Path, data: bytes
) -> dict[str, Any]:
    if len(data) > MAX_LOG_BYTES:
        raise InteropContractError(
            f"compatibility log exceeds {MAX_LOG_BYTES} bytes"
        )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return {
        "path": repo_relative(repo_root, path),
        "sha256": sha256(data),
        "size_bytes": len(data),
    }


def failed_report(*, mode: str) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": REPORT_KIND,
        "mode": mode,
        "status": "failed",
        "source": None,
        "oracle": None,
        "prerequisites": None,
        "fixtures": None,
        "inventory": None,
        "accounting": {
            "complete": False,
            "executed_count": 0,
            "passed_count": 0,
            "failed_count": 0,
            "skipped_count": 0,
            "ignored_count": 0,
            "executed_tests": [],
            "failed_tests": [],
            "skipped": [],
        },
        "execution": None,
        "errors": [],
    }


def validate_file_record(value: Any, *, label: str) -> Mapping[str, Any]:
    record = require_object(value, label=label)
    require_exact_keys(record, {"path", "sha256", "size_bytes"}, label=label)
    require_string(record["path"], label=f"{label} path")
    digest = require_string(record["sha256"], label=f"{label} sha256")
    if not SHA256.fullmatch(digest):
        raise InteropContractError(f"{label} sha256 must be lowercase hex")
    require_nonnegative_int(record["size_bytes"], label=f"{label} size_bytes")
    return record


def validate_report_document(value: Any) -> Mapping[str, Any]:
    report = require_object(value, label="compatibility report")
    require_exact_keys(
        report,
        {
            "schema_version",
            "kind",
            "mode",
            "status",
            "source",
            "oracle",
            "prerequisites",
            "fixtures",
            "inventory",
            "accounting",
            "execution",
            "errors",
        },
        label="compatibility report",
    )
    if report["schema_version"] != SCHEMA_VERSION or report["kind"] != REPORT_KIND:
        raise InteropContractError("compatibility report identity is invalid")
    if report["mode"] not in {"optional", "required"}:
        raise InteropContractError("compatibility report mode is invalid")
    if report["status"] not in {"passed", "failed"}:
        raise InteropContractError("compatibility report status is invalid")
    errors = require_list(report["errors"], label="compatibility errors")
    if any(not isinstance(error, str) or not error for error in errors):
        raise InteropContractError("compatibility errors must be non-empty strings")
    if report["status"] == "passed" and errors:
        raise InteropContractError("passed compatibility report cannot contain errors")
    if report["status"] == "failed" and not errors:
        raise InteropContractError("failed compatibility report must contain errors")

    source = report["source"]
    if source is not None:
        source = require_object(source, label="source")
        require_exact_keys(
            source, {"commit_sha", "expected_commit_sha"}, label="source"
        )
        for key in ("commit_sha", "expected_commit_sha"):
            revision = require_string(source[key], label=f"source {key}")
            if not FULL_SHA.fullmatch(revision):
                raise InteropContractError(f"source {key} is not a full SHA")
        if source["commit_sha"] != source["expected_commit_sha"]:
            raise InteropContractError("source revisions do not match")

    oracle = report["oracle"]
    if oracle is not None:
        oracle = require_object(oracle, label="oracle")
        require_exact_keys(
            oracle,
            {
                "path",
                "repository",
                "expected_commit_sha",
                "observed_commit_sha",
            },
            label="oracle",
        )
        if oracle["repository"] != PINNED_LEGACY_SYNCPLAY_REPOSITORY:
            raise InteropContractError("oracle repository identity drifted")
        if (
            oracle["expected_commit_sha"] != PINNED_LEGACY_SYNCPLAY_SHA
            or oracle["observed_commit_sha"] != PINNED_LEGACY_SYNCPLAY_SHA
        ):
            raise InteropContractError("oracle revision identity drifted")
        require_string(oracle["path"], label="oracle path")

    prerequisites = report["prerequisites"]
    if prerequisites is not None:
        prerequisites = require_object(prerequisites, label="prerequisites")
        require_exact_keys(
            prerequisites, {"python", "requirements", "probes"}, label="prerequisites"
        )
        python = require_object(prerequisites["python"], label="Python prerequisite")
        require_exact_keys(
            python,
            {
                "command",
                "executable",
                "implementation",
                "version",
                "version_info",
                "supported_family",
                "packages",
            },
            label="Python prerequisite",
        )
        for key in ("command", "executable", "implementation", "version", "supported_family"):
            require_string(python[key], label=f"Python {key}")
        if python["implementation"] != "CPython":
            raise InteropContractError("Python implementation identity drifted")
        if python["supported_family"] != ">=3.11,<3.14":
            raise InteropContractError("Python supported-family policy drifted")
        version_info = require_list(python["version_info"], label="Python version_info")
        if (
            len(version_info) != 3
            or any(
                not isinstance(item, int) or isinstance(item, bool)
                for item in version_info
            )
        ):
            raise InteropContractError(
                "Python version_info must have three integer entries"
            )
        family = tuple(version_info[:2])
        if not (
            SUPPORTED_PYTHON_MINIMUM
            <= family
            < SUPPORTED_PYTHON_MAXIMUM_EXCLUSIVE
        ):
            raise InteropContractError("Python version family is outside policy")
        if python["version"] != ".".join(str(item) for item in version_info):
            raise InteropContractError("Python version identity is contradictory")
        packages = require_list(python["packages"], label="Python packages")
        if len(packages) != len(PINNED_PACKAGES):
            raise InteropContractError("Python package count drifted")
        for index, item in enumerate(packages):
            package = require_object(item, label=f"Python package {index}")
            require_exact_keys(
                package,
                {"name", "expected_version", "observed_version"},
                label=f"Python package {index}",
            )
            if package["expected_version"] != package["observed_version"]:
                raise InteropContractError("Python package version identity differs")
        expected_packages = [
            {
                "name": display,
                "expected_version": version,
                "observed_version": version,
            }
            for _, (display, version) in sorted(PINNED_PACKAGES.items())
        ]
        if packages != expected_packages:
            raise InteropContractError("Python package identities drifted")
        requirements = require_object(
            prerequisites["requirements"], label="requirements"
        )
        require_exact_keys(
            requirements, {"path", "sha256", "packages"}, label="requirements"
        )
        if requirements["path"] != "requirements/legacy-python-interop.txt":
            raise InteropContractError("requirements path identity drifted")
        if not SHA256.fullmatch(
            require_string(requirements["sha256"], label="requirements sha256")
        ):
            raise InteropContractError("requirements sha256 is invalid")
        requirement_packages = require_list(
            requirements["packages"], label="requirements packages"
        )
        expected_requirement_packages = [
            {"name": display, "version": version}
            for _, (display, version) in sorted(PINNED_PACKAGES.items())
        ]
        if requirement_packages != expected_requirement_packages:
            raise InteropContractError("requirement package identities drifted")
        probes = require_list(prerequisites["probes"], label="probe identities")
        if len(probes) != len(PROBE_PATHS):
            raise InteropContractError("probe identity count drifted")
        for index, item in enumerate(probes):
            validate_file_record(item, label=f"probe identity {index}")
        if [item["path"] for item in probes] != list(PROBE_PATHS):
            raise InteropContractError("probe path identities drifted")

    fixtures = report["fixtures"]
    if fixtures is not None:
        fixtures = require_object(fixtures, label="fixtures")
        require_exact_keys(
            fixtures,
            {"roots", "counts", "file_count", "manifest_sha256", "files"},
            label="fixtures",
        )
        if fixtures["roots"] != list(FIXTURE_ROOT_COUNTS):
            raise InteropContractError("fixture root inventory drifted")
        if fixtures["counts"] != FIXTURE_ROOT_COUNTS:
            raise InteropContractError("fixture root counts drifted")
        files = require_list(fixtures["files"], label="fixture files")
        if require_nonnegative_int(
            fixtures["file_count"], label="fixture file_count"
        ) != len(files):
            raise InteropContractError("fixture file_count differs from files")
        if fixtures["file_count"] != sum(FIXTURE_ROOT_COUNTS.values()):
            raise InteropContractError("fixture file_count differs from root counts")
        digest = require_string(
            fixtures["manifest_sha256"], label="fixture manifest sha256"
        )
        if not SHA256.fullmatch(digest):
            raise InteropContractError("fixture manifest sha256 is invalid")
        for index, item in enumerate(files):
            validate_file_record(item, label=f"fixture file {index}")
        fixture_paths = [item["path"] for item in files]
        if fixture_paths != sorted(set(fixture_paths)):
            raise InteropContractError("fixture file paths must be unique and sorted")
        for root, expected_count in FIXTURE_ROOT_COUNTS.items():
            observed_count = sum(
                path.startswith(root + "/") for path in fixture_paths
            )
            if observed_count != expected_count:
                raise InteropContractError(
                    f"fixture files do not match root count for {root}"
                )
        recomputed_manifest = hashlib.sha256()
        for item in files:
            recomputed_manifest.update(item["path"].encode("utf-8"))
            recomputed_manifest.update(b"\0")
            recomputed_manifest.update(item["sha256"].encode("ascii"))
            recomputed_manifest.update(b"\0")
            recomputed_manifest.update(str(item["size_bytes"]).encode("ascii"))
            recomputed_manifest.update(b"\n")
        if digest != recomputed_manifest.hexdigest():
            raise InteropContractError("fixture manifest sha256 is contradictory")

    inventory = report["inventory"]
    if inventory is not None:
        inventory = require_object(inventory, label="inventory")
        require_exact_keys(
            inventory,
            {"listed_count", "listed_tests", "ignored_count", "ignored_tests"},
            label="inventory",
        )
        listed = require_list(inventory["listed_tests"], label="listed_tests")
        if require_nonnegative_int(
            inventory["listed_count"], label="listed_count"
        ) != len(listed):
            raise InteropContractError("listed_count differs from listed_tests")
        if len(listed) != EXPECTED_DISCOVERED_TESTS:
            raise InteropContractError(
                "listed compatibility inventory differs from the source-bound "
                f"expectation: {len(listed)} != {EXPECTED_DISCOVERED_TESTS}"
            )
        for index, name in enumerate(listed):
            require_string(name, label=f"listed test {index}")
        if listed != sorted(set(listed)):
            raise InteropContractError("listed_tests must be unique and sorted")
        missing_sentinels = sorted(REQUIRED_LIVE_SENTINELS - set(listed))
        if missing_sentinels:
            raise InteropContractError(
                "listed compatibility inventory omits required live sentinels "
                f"{missing_sentinels}"
            )
        if listed != reviewed_tests("compat"):
            raise InteropContractError("listed compatibility inventory changed reviewed required identities")
        ignored = require_list(inventory["ignored_tests"], label="ignored_tests")
        if require_nonnegative_int(
            inventory["ignored_count"], label="ignored_count"
        ) != len(ignored):
            raise InteropContractError("ignored_count differs from ignored_tests")
        for index, item in enumerate(ignored):
            ignored_item = require_object(item, label=f"ignored test {index}")
            require_exact_keys(
                ignored_item, {"test", "reason"}, label=f"ignored test {index}"
            )
        expected_ignored = [
            {"test": name, "reason": EXPECTED_IGNORED_TESTS[name]}
            for name in sorted(EXPECTED_IGNORED_TESTS)
        ]
        if ignored != expected_ignored:
            raise InteropContractError("ignored test inventory drifted")
        if not {
            item["test"] for item in expected_ignored
        }.issubset(listed):
            raise InteropContractError(
                "ignored compatibility tests must belong to the complete inventory"
            )

    accounting = require_object(report["accounting"], label="accounting")
    require_exact_keys(
        accounting,
        {
            "complete",
            "executed_count",
            "passed_count",
            "failed_count",
            "skipped_count",
            "ignored_count",
            "executed_tests",
            "failed_tests",
            "skipped",
        },
        label="accounting",
    )
    if not isinstance(accounting["complete"], bool):
        raise InteropContractError("accounting complete must be a boolean")
    counts = {
        key: require_nonnegative_int(accounting[key], label=f"accounting {key}")
        for key in (
            "executed_count",
            "passed_count",
            "failed_count",
            "skipped_count",
            "ignored_count",
        )
    }
    executed_tests = require_list(
        accounting["executed_tests"], label="executed_tests"
    )
    failed_tests = require_list(accounting["failed_tests"], label="failed_tests")
    skipped = require_list(accounting["skipped"], label="skipped")
    if counts["executed_count"] != len(executed_tests):
        raise InteropContractError("executed_count differs from executed_tests")
    if counts["failed_count"] != len(failed_tests):
        raise InteropContractError("failed_count differs from failed_tests")
    if counts["skipped_count"] != len(skipped):
        raise InteropContractError("skipped_count differs from skipped")
    if counts["executed_count"] != counts["passed_count"] + counts["failed_count"]:
        raise InteropContractError("executed accounting is contradictory")
    if executed_tests != sorted(set(executed_tests)):
        raise InteropContractError("executed_tests must be unique and sorted")
    if failed_tests != sorted(set(failed_tests)):
        raise InteropContractError("failed_tests must be unique and sorted")
    if not set(failed_tests).issubset(executed_tests):
        raise InteropContractError("failed_tests must be a subset of executed_tests")
    for index, item in enumerate(skipped):
        event = require_object(item, label=f"skip event {index}")
        require_exact_keys(
            event, {"scope", "test", "code", "reason"}, label=f"skip event {index}"
        )
        if event["scope"] not in {"preflight", "test"}:
            raise InteropContractError("skip event scope is invalid")
        if event["test"] is not None:
            require_string(event["test"], label="skip event test")
        if event["code"] not in SKIP_REASON_CODES:
            raise InteropContractError("skip event code is invalid")
        require_string(event["reason"], label="skip event reason")
    if report["mode"] == "required" and report["status"] == "passed" and skipped:
        raise InteropContractError("required passed report cannot contain skips")
    if inventory is not None and accounting["complete"]:
        if counts["ignored_count"] != inventory["ignored_count"]:
            raise InteropContractError("ignored accounting differs from inventory")
        total = counts["executed_count"] + counts["skipped_count"] + counts["ignored_count"]
        if total != inventory["listed_count"]:
            raise InteropContractError(
                "executed, skipped, and ignored accounting is not exhaustive"
            )
        skip_tests = [
            event["test"]
            for event in skipped
            if event["scope"] == "test"
        ]
        if len(skip_tests) != len(set(skip_tests)):
            raise InteropContractError("test skip inventory contains duplicates")
        if not set(executed_tests).isdisjoint(skip_tests):
            raise InteropContractError(
                "executed and skipped test inventories overlap"
            )
        ignored_names = {item["test"] for item in inventory["ignored_tests"]}
        if (
            set(executed_tests)
            | set(skip_tests)
            | ignored_names
            != set(inventory["listed_tests"])
        ):
            raise InteropContractError(
                "executed, skipped, and ignored test inventories are not exhaustive"
            )
    if report["status"] == "passed" and inventory is not None and not accounting["complete"]:
        raise InteropContractError(
            "passed executed report must contain complete accounting"
        )

    execution = report["execution"]
    if execution is not None:
        execution = require_object(execution, label="execution")
        require_exact_keys(
            execution,
            {"command", "returncode", "duration_seconds", "stdout", "stderr"},
            label="execution",
        )
        if execution["command"] != list(TEST_COMMAND):
            raise InteropContractError(
                "execution command must be the complete selector-free matrix"
            )
        if not isinstance(execution["returncode"], int):
            raise InteropContractError("execution returncode must be an integer")
        if (
            not isinstance(execution["duration_seconds"], (int, float))
            or isinstance(execution["duration_seconds"], bool)
            or execution["duration_seconds"] < 0
        ):
            raise InteropContractError("execution duration_seconds is invalid")
        validate_file_record(execution["stdout"], label="execution stdout")
        validate_file_record(execution["stderr"], label="execution stderr")
        if report["status"] == "passed" and execution["returncode"] != 0:
            raise InteropContractError(
                "passed compatibility report must have a zero execution return code"
            )
    if report["mode"] == "required" and report["status"] == "passed":
        required_evidence = (
            "source",
            "oracle",
            "prerequisites",
            "fixtures",
            "inventory",
            "execution",
        )
        missing_evidence = [
            field for field in required_evidence if report[field] is None
        ]
        if missing_evidence:
            raise InteropContractError(
                "required passed report omits successful execution evidence "
                f"{missing_evidence}"
            )
        expected_passed = EXPECTED_DISCOVERED_TESTS - len(
            EXPECTED_IGNORED_TESTS
        )
        expected_counts = {
            "executed_count": expected_passed,
            "passed_count": expected_passed,
            "failed_count": 0,
            "skipped_count": 0,
            "ignored_count": len(EXPECTED_IGNORED_TESTS),
        }
        if not accounting["complete"] or counts != expected_counts:
            raise InteropContractError(
                "required passed report does not contain the exact successful "
                "compatibility accounting"
            )
    return report


def atomic_write_json(path: pathlib.Path, value: Mapping[str, Any]) -> None:
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
        raise InteropContractError(
            f"compatibility report exceeds {MAX_REPORT_BYTES} bytes"
        )
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_bytes(data)
    os.replace(temporary, path)


def strict_load_report(path: pathlib.Path) -> Mapping[str, Any]:
    try:
        data = path.read_bytes()
    except OSError as error:
        raise InteropContractError(f"cannot read compatibility report: {error}") from error
    if len(data) > MAX_REPORT_BYTES:
        raise InteropContractError(
            f"compatibility report exceeds {MAX_REPORT_BYTES} bytes"
        )
    return validate_report_document(
        strict_parse_json(data, label="compatibility report")
    )


def collect_report(
    *,
    repo_root: pathlib.Path,
    output: pathlib.Path,
    environment: Mapping[str, str],
) -> tuple[int, Mapping[str, Any]]:
    required = environment.get(REQUIRED_ENVIRONMENT_VARIABLE) == "1"
    mode = "required" if required else "optional"
    report = failed_report(mode=mode)
    try:
        if not (repo_root / "Cargo.toml").is_file():
            raise InteropContractError("repository root does not contain Cargo.toml")
        report["source"] = verify_source(repo_root, environment)
        report["oracle"] = verify_oracle(repo_root, environment)
        python_record, requirements_record = verify_python(repo_root, environment)
        fixtures, probes = verify_fixtures(repo_root, environment)
        report["prerequisites"] = {
            "python": python_record,
            "requirements": requirements_record,
            "probes": probes,
        }
        report["fixtures"] = fixtures

        execution_environment = build_execution_environment(
            repo_root=repo_root,
            environment=environment,
            oracle=report["oracle"],
            python_record=python_record,
            required=required,
        )

        list_result = run_command(
            LIST_COMMAND,
            cwd=repo_root,
            environment=execution_environment,
        )
        ignored_list_result = run_command(
            IGNORED_LIST_COMMAND,
            cwd=repo_root,
            environment=execution_environment,
        )
        report["inventory"] = verify_inventory(list_result, ignored_list_result)

        result = run_command(
            TEST_COMMAND,
            cwd=repo_root,
            environment=execution_environment,
        )
        stdout_path = output.with_name(output.stem + ".stdout.log")
        stderr_path = output.with_name(output.stem + ".stderr.log")
        stdout_record = artifact_record(
            repo_root, stdout_path, result.stdout
        )
        stderr_record = artifact_record(
            repo_root, stderr_path, result.stderr
        )
        report["execution"] = {
            "command": list(TEST_COMMAND),
            "returncode": result.returncode,
            "duration_seconds": round(result.duration_seconds, 6),
            "stdout": stdout_record,
            "stderr": stderr_record,
        }
        report["accounting"] = account_execution(report["inventory"], result)
        failures = execution_failures(
            report["accounting"], result, required=required
        )
        if failures:
            raise InteropContractError("; ".join(failures))
        report["status"] = "passed"
    except PrerequisiteUnavailable as error:
        report["accounting"]["skipped_count"] = 1
        report["accounting"]["skipped"] = [
            {
                "scope": "preflight",
                "test": None,
                "code": error.code,
                "reason": error.reason,
            }
        ]
        if required:
            report["errors"].append(
                f"required live prerequisite unavailable: {error.code}: {error.reason}"
            )
        else:
            report["status"] = "passed"
    except (InteropContractError, OSError) as error:
        report["errors"].append(str(error))

    try:
        validate_report_document(report)
    except InteropContractError as validation_error:
        report["status"] = "failed"
        report["errors"].append(
            f"generated report failed closed-schema validation: {validation_error}"
        )
    atomic_write_json(output, report)
    return (0 if report["status"] == "passed" else 1), report


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Run the complete pinned Python compatibility matrix"
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--repo-root", default=".")
    run_parser.add_argument(
        "--output",
        default="target/verification/compat-live-interop.json",
    )
    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("--report", required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command == "validate":
        try:
            strict_load_report(pathlib.Path(args.report))
        except InteropContractError as error:
            print(f"compatibility report validation failed: {error}", file=sys.stderr)
            return 1
        print(f"compatibility report passed: {args.report}")
        return 0

    repo_root = pathlib.Path(args.repo_root).resolve()
    try:
        output = resolve_within(
            repo_root,
            pathlib.Path(args.output),
            label="compatibility report output",
        )
        exit_code, report = collect_report(
            repo_root=repo_root,
            output=output,
            environment=dict(os.environ),
        )
    except (InteropContractError, OSError) as error:
        print(f"compatibility collection failed before report creation: {error}", file=sys.stderr)
        return 2
    if exit_code == 0:
        print(
            "compatibility matrix passed: "
            f"executed={report['accounting']['executed_count']} "
            f"skipped={report['accounting']['skipped_count']} "
            f"ignored={report['accounting']['ignored_count']} "
            f"report={output}"
        )
    else:
        print(
            f"compatibility matrix failed closed: report={output}",
            file=sys.stderr,
        )
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
