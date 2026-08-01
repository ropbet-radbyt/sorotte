#!/usr/bin/env python3
"""Run the required nextest policy and preserve fail-on-flaky evidence."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET
from collections.abc import Sequence
from typing import Any, TextIO


PINNED_NEXTEST_VERSION = "0.9.137"
CI_PROFILE = "ci"
RETRY_COUNT = 1
NEXTEST_COMMAND = (
    "cargo",
    "nextest",
    "run",
    "--locked",
    "--workspace",
    "--all-features",
    "--profile",
    CI_PROFILE,
    "--retries",
    str(RETRY_COUNT),
    "--no-fail-fast",
    "--status-level",
    "leak",
    "--final-status-level",
    "fail",
    "--flaky-result",
    "fail",
)
EXPECTED_CI_PROFILE: dict[str, Any] = {
    "retries": RETRY_COUNT,
    "flaky-result": "fail",
    "fail-fast": False,
    "status-level": "leak",
    "final-status-level": "fail",
}
EXPECTED_LEAK_TIMEOUT: dict[str, Any] = {
    "period": "500ms",
    "result": "fail",
}
EXPECTED_JUNIT: dict[str, Any] = {
    "path": "junit.xml",
    "store-success-output": True,
    "store-failure-output": True,
    "flaky-fail-status": "failure",
}
FAILING_JUNIT_ELEMENTS = (
    "failure",
    "error",
    "flakyFailure",
    "flakyError",
    "rerunFailure",
    "rerunError",
)


class PolicyError(RuntimeError):
    """Raised when fail-on-flaky policy is missing or weakened."""


def _require_mapping(value: Any, description: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise PolicyError(f"{description} must be a TOML table")
    return value


def _require_exact_keys(
    value: dict[str, Any],
    expected: set[str],
    description: str,
) -> None:
    actual = set(value)
    if actual != expected:
        missing = sorted(expected - actual)
        unexpected = sorted(actual - expected)
        details: list[str] = []
        if missing:
            details.append(f"missing {missing}")
        if unexpected:
            details.append(f"unreviewed {unexpected}")
        raise PolicyError(
            f"{description} fields must be exactly {sorted(expected)}: "
            + ", ".join(details)
        )


def _matches_exact(actual: Any, expected: Any) -> bool:
    # bool is a subclass of int in Python. Configuration policy must not let a
    # TOML integer masquerade as a boolean (or a float masquerade as retries).
    return type(actual) is type(expected) and actual == expected


def validate_config(path: pathlib.Path) -> None:
    """Fail closed unless the checked-in CI profile encodes the full policy."""

    try:
        with path.open("rb") as config_file:
            config = tomllib.load(config_file)
    except FileNotFoundError as error:
        raise PolicyError(f"missing nextest configuration: {path}") from error
    except tomllib.TOMLDecodeError as error:
        raise PolicyError(f"invalid nextest configuration {path}: {error}") from error

    _require_exact_keys(config, {"profile"}, "nextest configuration")
    profiles = _require_mapping(config.get("profile"), "profile")
    _require_exact_keys(profiles, {CI_PROFILE}, "profile")
    ci_profile = _require_mapping(profiles.get(CI_PROFILE), f"profile.{CI_PROFILE}")
    _require_exact_keys(
        ci_profile,
        set(EXPECTED_CI_PROFILE) | {"leak-timeout", "junit"},
        f"profile.{CI_PROFILE}",
    )
    for key, expected in EXPECTED_CI_PROFILE.items():
        actual = ci_profile.get(key)
        if not _matches_exact(actual, expected):
            raise PolicyError(
                f"profile.{CI_PROFILE}.{key} must be {expected!r}, "
                f"received {actual!r}"
            )

    leak_timeout = _require_mapping(
        ci_profile.get("leak-timeout"),
        f"profile.{CI_PROFILE}.leak-timeout",
    )
    if set(leak_timeout) != set(EXPECTED_LEAK_TIMEOUT):
        raise PolicyError(
            f"profile.{CI_PROFILE}.leak-timeout fields must be exactly "
            f"{sorted(EXPECTED_LEAK_TIMEOUT)}, received {sorted(leak_timeout)}"
        )
    for key, expected in EXPECTED_LEAK_TIMEOUT.items():
        actual = leak_timeout.get(key)
        if not _matches_exact(actual, expected):
            raise PolicyError(
                f"profile.{CI_PROFILE}.leak-timeout.{key} must be "
                f"{expected!r}, received {actual!r}"
            )

    junit = _require_mapping(
        ci_profile.get("junit"),
        f"profile.{CI_PROFILE}.junit",
    )
    _require_exact_keys(
        junit,
        set(EXPECTED_JUNIT),
        f"profile.{CI_PROFILE}.junit",
    )
    for key, expected in EXPECTED_JUNIT.items():
        actual = junit.get(key)
        if not _matches_exact(actual, expected):
            raise PolicyError(
                f"profile.{CI_PROFILE}.junit.{key} must be {expected!r}, "
                f"received {actual!r}"
            )


def _local_name(tag: str) -> str:
    return tag.rsplit("}", maxsplit=1)[-1]


def inspect_junit(path: pathlib.Path) -> tuple[dict[str, Any], list[str]]:
    """Summarize nextest JUnit and reject every failing or flaky attempt."""

    summary: dict[str, Any] = {
        "path": path.as_posix(),
        "present": path.is_file(),
        "testcases": 0,
        "elements": {name: 0 for name in FAILING_JUNIT_ELEMENTS},
    }
    if not path.is_file():
        return summary, [f"nextest did not produce required JUnit report: {path}"]

    try:
        root = ET.parse(path).getroot()
    except ET.ParseError as error:
        return summary, [f"nextest JUnit report is malformed: {error}"]

    for element in root.iter():
        local_name = _local_name(element.tag)
        if local_name == "testcase":
            summary["testcases"] += 1
        if local_name in summary["elements"]:
            summary["elements"][local_name] += 1

    violations: list[str] = []
    flaky_count = sum(
        summary["elements"][name] for name in ("flakyFailure", "flakyError")
    )
    rerun_failure_count = sum(
        summary["elements"][name] for name in ("rerunFailure", "rerunError")
    )
    final_failure_count = sum(
        summary["elements"][name] for name in ("failure", "error")
    )
    if flaky_count:
        violations.append(
            f"JUnit records {flaky_count} pass-after-fail attempt(s); "
            "flaky tests fail required CI"
        )
    if rerun_failure_count:
        violations.append(
            f"JUnit records {rerun_failure_count} unsuccessful retry attempt(s)"
        )
    if final_failure_count:
        violations.append(
            f"JUnit records {final_failure_count} final test failure(s)"
        )
    if summary["testcases"] == 0:
        violations.append(
            "JUnit records zero testcases; an empty suite cannot satisfy "
            "required workspace evidence"
        )
    return summary, violations


def assess_run(
    producer_exit_code: int | None,
    junit_path: pathlib.Path,
) -> tuple[dict[str, Any], list[str]]:
    summary, violations = inspect_junit(junit_path)
    if producer_exit_code is None:
        violations.insert(0, "nextest did not start")
    elif producer_exit_code != 0:
        violations.insert(0, f"nextest exited with status {producer_exit_code}")
    return summary, violations


def _write_line(message: str, log_file: TextIO) -> None:
    print(message, flush=True)
    log_file.write(f"{message}\n")
    log_file.flush()


def _target_root(repo_root: pathlib.Path) -> pathlib.Path:
    configured = os.environ.get("CARGO_TARGET_DIR")
    if not configured:
        return repo_root / "target"
    target = pathlib.Path(configured)
    return target if target.is_absolute() else repo_root / target


def _write_policy_report(
    path: pathlib.Path,
    *,
    repo_root: pathlib.Path,
    producer_exit_code: int | None,
    version_output: str | None,
    junit_summary: dict[str, Any],
    violations: Sequence[str],
) -> None:
    report = {
        "schema_version": 1,
        "policy": {
            "cargo_nextest_version": PINNED_NEXTEST_VERSION,
            "profile": CI_PROFILE,
            "retries": RETRY_COUNT,
            "flaky_result": "fail",
            "leak_timeout": EXPECTED_LEAK_TIMEOUT,
        },
        "command": list(NEXTEST_COMMAND),
        "producer_exit_code": producer_exit_code,
        "version_output": version_output,
        "junit": junit_summary,
        "outcome": "passed" if not violations else "failed",
        "violations": list(violations),
    }
    path.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    try:
        display_path = path.relative_to(repo_root)
    except ValueError:
        display_path = path
    print(f"nextest policy evidence: {display_path}", flush=True)


def _check_version(log_file: TextIO) -> tuple[str | None, list[str]]:
    completed = subprocess.run(
        ["cargo", "nextest", "--version"],
        check=False,
        capture_output=True,
        text=True,
        errors="replace",
    )
    output = "\n".join(
        part.strip() for part in (completed.stdout, completed.stderr) if part.strip()
    )
    if output:
        _write_line(f"$ cargo nextest --version\n{output}", log_file)
    if completed.returncode != 0:
        return output or None, [
            f"cargo nextest --version exited with status {completed.returncode}"
        ]
    match = re.search(r"\bcargo-nextest\s+(\d+\.\d+\.\d+)\b", output)
    if match is None:
        return output or None, [f"could not parse cargo-nextest version: {output!r}"]
    actual = match.group(1)
    if actual != PINNED_NEXTEST_VERSION:
        return output, [
            f"cargo-nextest must be {PINNED_NEXTEST_VERSION}, received {actual}"
        ]
    return output, []


def run_required_suite(repo_root: pathlib.Path) -> int:
    repo_root = repo_root.resolve()
    artifact_dir = _target_root(repo_root) / "nextest" / CI_PROFILE
    junit_path = artifact_dir / EXPECTED_JUNIT["path"]
    log_path = artifact_dir / "console.log"
    policy_path = artifact_dir / "policy.json"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    for stale_path in (junit_path, policy_path):
        stale_path.unlink(missing_ok=True)

    producer_exit_code: int | None = None
    version_output: str | None = None
    violations: list[str] = []
    with log_path.open("w", encoding="utf-8", errors="replace") as log_file:
        try:
            validate_config(repo_root / ".config" / "nextest.toml")
        except PolicyError as error:
            violations.append(str(error))

        if not violations:
            try:
                version_output, version_violations = _check_version(log_file)
                violations.extend(version_violations)
            except OSError as error:
                violations.append(f"could not execute cargo nextest: {error}")

        if not violations:
            _write_line(f"$ {' '.join(NEXTEST_COMMAND)}", log_file)
            try:
                process = subprocess.Popen(
                    NEXTEST_COMMAND,
                    cwd=repo_root,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    errors="replace",
                    bufsize=1,
                )
                assert process.stdout is not None
                for line in process.stdout:
                    sys.stdout.write(line)
                    sys.stdout.flush()
                    log_file.write(line)
                    log_file.flush()
                producer_exit_code = process.wait()
            except OSError as error:
                violations.append(f"could not execute cargo nextest: {error}")

        junit_summary, run_violations = assess_run(
            producer_exit_code,
            junit_path,
        )
        violations.extend(run_violations)
        for violation in violations:
            _write_line(f"policy violation: {violation}", log_file)

    _write_policy_report(
        policy_path,
        repo_root=repo_root,
        producer_exit_code=producer_exit_code,
        version_output=version_output,
        junit_summary=junit_summary,
        violations=violations,
    )
    return 0 if not violations else 1


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    validate_parser = subparsers.add_parser(
        "validate",
        help="validate the checked-in fail-on-flaky profile",
    )
    validate_parser.add_argument(
        "--config",
        type=pathlib.Path,
        default=pathlib.Path(".config/nextest.toml"),
    )

    run_parser = subparsers.add_parser(
        "run",
        help="run the pinned required workspace suite and preserve its evidence",
    )
    run_parser.add_argument(
        "--repo-root",
        type=pathlib.Path,
        default=pathlib.Path("."),
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    if args.command == "validate":
        try:
            validate_config(args.config)
        except PolicyError as error:
            print(f"nextest policy invalid: {error}", file=sys.stderr)
            return 1
        print(f"nextest policy valid: {args.config}")
        return 0
    if args.command == "run":
        return run_required_suite(args.repo_root)
    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
