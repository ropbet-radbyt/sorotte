#!/usr/bin/env python3
"""Fail-closed validation for the Windows GUI native-smoke report.

The native runner predates structured capability outcomes.  Until it emits
those outcomes itself, this boundary treats every selected scenario as
required, rejects every skip marker, and requires an observable completion
marker for each scenario.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
from collections.abc import Iterable, Mapping
from typing import Any


SUMMARY_KIND = "sorotte-gui-native-smoke-contract"
REQUIRED_REPORT_KEYS = {
    "result",
    "binary",
    "pid",
    "window_title",
    "menu_labels",
    "menu_contract",
    "accessible_name_count",
    "accessibility_contract",
    "interaction_steps",
    "interaction_contract",
    "closed",
    "duration_ms",
}
REQUIRED_MENU_LABELS = {"File", "Playback", "Advanced", "Window", "Help"}

# Completion markers are deliberately behavior-facing rather than an assertion
# count.  A runner refactor that removes a scenario or turns it into a no-op
# must update this reviewed contract instead of silently preserving a green
# exit code.
SCENARIO_REQUIRED_STEPS: dict[str, tuple[str, ...]] = {
    "baseline": (
        "main-window-playback-controls-detached",
        "config-validation-visible",
        "config-save-persisted",
        "surface-public-servers",
        "public-server-refresh-complete",
        "surface-media-search",
        "media-search-browse",
        "surface-configuration",
        "config-reload-persisted",
        "open-media-prep-shared-playlists",
        "open-media-file",
        "about-opens-and-closes-modal",
    ),
    "relaunch": (
        "gui-state-restored",
        "clear-gui-data-completed",
        "clear-gui-data-relaunch-first-run",
        "config-migration-predictable",
    ),
    "drag-drop": (
        "drag-drop-window-media",
        "drag-drop-playlist-import",
    ),
    "loopback": ("loopback-chat-send",),
    "live-python": (
        "transport-python-peer-connect",
        "transport-python-peer-readiness",
        "transport-python-peer-playlist-peer-to-local",
        "transport-python-peer-disconnect",
        "transport-python-peer-reconnect-peer-to-local",
    ),
    "controlled-room": (
        "transport-python-peer-controlled-room-connect",
        "transport-python-peer-controlled-room-auth",
        "transport-python-peer-controlled-room-playlist-enabled",
    ),
    "detached-missing-media": (
        "detached-missing-media-target-staged",
        "detached-missing-media-search-success",
    ),
    "missing-media-continue": ("main-window-missing-media-continue-session",),
    "transport": ("transport-saved-config-startup",),
}
DEFAULT_REQUIRED_SCENARIOS = tuple(SCENARIO_REQUIRED_STEPS)
GLOBAL_REQUIRED_STEPS = ("file-exit",)
FORBIDDEN_STEP_PATTERN = re.compile(r"(?:^|[-_])skipped(?::|[-_]|$)", re.IGNORECASE)


class NativeSmokeContractError(ValueError):
    """The native-smoke output does not constitute required evidence."""


def _is_int(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _read_text(path: pathlib.Path) -> str:
    payload = path.read_bytes()
    for encoding in ("utf-8-sig", "utf-16"):
        try:
            return payload.decode(encoding)
        except UnicodeDecodeError:
            continue
    raise NativeSmokeContractError(f"{path} is not UTF-8 or UTF-16 text")


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise NativeSmokeContractError(
                f"native report contains duplicate JSON key {key!r}"
            )
        result[key] = value
    return result


def _parse_report(report_text: str) -> Mapping[str, Any]:
    try:
        report = json.loads(report_text, object_pairs_hook=_unique_object)
    except json.JSONDecodeError as error:
        raise NativeSmokeContractError(
            f"native report is not one complete JSON document: {error}"
        ) from error
    if not isinstance(report, dict):
        raise NativeSmokeContractError("native report root must be a JSON object")
    return report


def normalize_scenarios(scenarios: Iterable[str]) -> tuple[str, ...]:
    normalized = tuple(scenario.strip().lower() for scenario in scenarios)
    errors: list[str] = []
    if not normalized:
        errors.append("at least one required native scenario must be selected")
    if any(not scenario for scenario in normalized):
        errors.append("native scenario names must not be empty")

    duplicates = sorted(
        scenario for scenario in set(normalized) if normalized.count(scenario) > 1
    )
    if duplicates:
        errors.append(f"duplicate native scenarios: {', '.join(duplicates)}")

    unknown = sorted(set(normalized) - set(SCENARIO_REQUIRED_STEPS))
    if unknown:
        errors.append(f"unknown native scenarios: {', '.join(unknown)}")

    if errors:
        raise NativeSmokeContractError("; ".join(errors))
    return normalized


def validate_native_smoke(
    report_text: str,
    stderr_text: str,
    scenarios: Iterable[str],
    *,
    allowed_stderr_patterns: Iterable[str] = (),
    expected_binary: pathlib.Path | None = None,
    expected_binary_sha256: str | None = None,
    producer_exit_code: int = 0,
) -> Mapping[str, Any]:
    required_scenarios = normalize_scenarios(scenarios)
    report = _parse_report(report_text)
    errors: list[str] = []

    actual_keys = set(report)
    missing_keys = sorted(REQUIRED_REPORT_KEYS - actual_keys)
    extra_keys = sorted(actual_keys - REQUIRED_REPORT_KEYS)
    if missing_keys:
        errors.append(f"native report is missing keys: {', '.join(missing_keys)}")
    if extra_keys:
        errors.append(f"native report has unreviewed keys: {', '.join(extra_keys)}")

    if report.get("result") != "ok":
        errors.append(f"native report result must be 'ok', got {report.get('result')!r}")

    if not _is_int(producer_exit_code):
        errors.append("native producer exit code must be an integer")
    elif producer_exit_code != 0:
        errors.append(f"native producer exited with code {producer_exit_code}")

    binary = report.get("binary")
    if not isinstance(binary, str) or not binary.strip():
        errors.append("native report binary must be a non-empty string")
    elif expected_binary is not None:
        try:
            if not pathlib.Path(binary).samefile(expected_binary):
                errors.append(
                    "native report binary does not match the expected executable: "
                    f"reported={binary!r}, expected={str(expected_binary)!r}"
                )
        except OSError as error:
            errors.append(f"could not bind native report binary to expected path: {error}")

    if expected_binary_sha256 is not None:
        normalized_digest = expected_binary_sha256.lower()
        if not re.fullmatch(r"[0-9a-f]{64}", normalized_digest):
            errors.append("expected native binary SHA-256 must be 64 lowercase hex digits")
        elif expected_binary is None:
            errors.append("expected native binary path is required with its SHA-256")
        else:
            try:
                observed_digest = hashlib.sha256(expected_binary.read_bytes()).hexdigest()
            except OSError as error:
                errors.append(f"could not hash expected native binary: {error}")
            else:
                if observed_digest != normalized_digest:
                    errors.append(
                        "native binary SHA-256 changed before evidence validation: "
                        f"expected={normalized_digest}, observed={observed_digest}"
                    )

    pid = report.get("pid")
    if not _is_int(pid) or pid <= 0:
        errors.append("native report pid must be a positive integer")

    window_title = report.get("window_title")
    if not isinstance(window_title, str) or "Sorotte" not in window_title:
        errors.append("native report window_title must identify Sorotte")

    menu_labels = report.get("menu_labels")
    normalized_menu_labels: set[str] = set()
    if not isinstance(menu_labels, list) or not all(
        isinstance(label, str) for label in menu_labels
    ):
        errors.append("native report menu_labels must be an array of strings")
    else:
        normalized_menu_labels = {
            label.replace("&", "").strip() for label in menu_labels
        }
        missing_labels = sorted(REQUIRED_MENU_LABELS - normalized_menu_labels)
        if missing_labels:
            errors.append(
                f"native menu is missing required labels: {', '.join(missing_labels)}"
            )

    if report.get("menu_contract") != "verified":
        errors.append(
            "native menu contract must be required-pass ('verified'), "
            f"got {report.get('menu_contract')!r}"
        )

    accessible_name_count = report.get("accessible_name_count")
    if not _is_int(accessible_name_count) or accessible_name_count <= 0:
        errors.append("accessible_name_count must be a positive integer")

    if report.get("accessibility_contract") != "verified":
        errors.append(
            "accessibility contract must be required-pass ('verified'), "
            f"got {report.get('accessibility_contract')!r}"
        )

    interaction_steps = report.get("interaction_steps")
    steps: list[str] = []
    if not isinstance(interaction_steps, list) or not all(
        isinstance(step, str) and step for step in interaction_steps
    ):
        errors.append("interaction_steps must be an array of non-empty strings")
    else:
        steps = interaction_steps
        forbidden_steps = sorted(
            step for step in steps if FORBIDDEN_STEP_PATTERN.search(step)
        )
        if forbidden_steps:
            errors.append(
                "required native capabilities were skipped: "
                + ", ".join(forbidden_steps)
            )

        step_set = set(steps)
        for step in GLOBAL_REQUIRED_STEPS:
            if step not in step_set:
                errors.append(f"global native completion step is missing: {step}")
        for scenario in required_scenarios:
            for step in SCENARIO_REQUIRED_STEPS[scenario]:
                if step not in step_set:
                    errors.append(
                        f"scenario {scenario!r} is missing completion step {step!r}"
                    )

    if report.get("interaction_contract") != "verified":
        errors.append(
            "interaction contract must be required-pass ('verified'), "
            f"got {report.get('interaction_contract')!r}"
        )

    if report.get("closed") is not True:
        errors.append("native GUI must be closed at the end of a strict run")

    duration_ms = report.get("duration_ms")
    if not _is_int(duration_ms) or duration_ms <= 0:
        errors.append("duration_ms must be a positive integer")

    stderr_lines = [line for line in stderr_text.splitlines() if line.strip()]
    compiled_patterns: list[re.Pattern[str]] = []
    for pattern in allowed_stderr_patterns:
        try:
            compiled_patterns.append(re.compile(pattern))
        except re.error as error:
            raise NativeSmokeContractError(
                f"invalid stderr allowlist regex {pattern!r}: {error}"
            ) from error
    unexpected_stderr = [
        line
        for line in stderr_lines
        if not any(pattern.fullmatch(line) for pattern in compiled_patterns)
    ]
    if unexpected_stderr:
        errors.append(
            "native runner wrote unexpected stderr: " + " | ".join(unexpected_stderr)
        )

    if errors:
        raise NativeSmokeContractError("\n".join(errors))
    return report


def _summary(
    *,
    status: str,
    scenarios: Iterable[str],
    report_text: str,
    errors: Iterable[str],
    expected_binary: pathlib.Path | None,
    expected_binary_sha256: str | None,
    producer_exit_code: int | None,
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "kind": SUMMARY_KIND,
        "status": status,
        "required_scenarios": list(scenarios),
        "report_sha256": hashlib.sha256(report_text.encode("utf-8")).hexdigest(),
        "expected_binary": (
            str(expected_binary) if expected_binary is not None else None
        ),
        "expected_binary_sha256": expected_binary_sha256,
        "producer_exit_code": producer_exit_code,
        "errors": list(errors),
    }


def _write_summary(path: pathlib.Path | None, summary: Mapping[str, Any]) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Validate required Windows GUI native-smoke evidence"
    )
    parser.add_argument("--report", type=pathlib.Path)
    parser.add_argument("--stderr", type=pathlib.Path)
    parser.add_argument("--summary", type=pathlib.Path)
    parser.add_argument("--scenario", action="append", default=[])
    parser.add_argument("--allow-stderr-regex", action="append", default=[])
    parser.add_argument("--expected-binary", type=pathlib.Path)
    parser.add_argument("--expected-binary-sha256")
    parser.add_argument("--producer-exit-code", type=int)
    parser.add_argument("--print-default-scenarios", action="store_true")
    parser.add_argument("--check-scenarios", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    options = build_parser().parse_args(argv)
    if options.print_default_scenarios:
        if (
            options.report
            or options.stderr
            or options.summary
            or options.scenario
            or options.check_scenarios
            or options.expected_binary
            or options.expected_binary_sha256
            or options.producer_exit_code is not None
        ):
            print(
                "--print-default-scenarios cannot be combined with validation options",
                file=sys.stderr,
            )
            return 2
        print("\n".join(DEFAULT_REQUIRED_SCENARIOS))
        return 0

    if options.check_scenarios:
        if (
            options.report
            or options.stderr
            or options.summary
            or options.expected_binary
            or options.expected_binary_sha256
            or options.producer_exit_code is not None
        ):
            print(
                "--check-scenarios cannot be combined with report validation",
                file=sys.stderr,
            )
            return 2
        try:
            normalize_scenarios(options.scenario)
        except NativeSmokeContractError as error:
            print(f"native scenario inventory rejected: {error}", file=sys.stderr)
            return 1
        return 0

    report_text = ""
    scenarios: tuple[str, ...] = tuple(options.scenario)
    try:
        if options.report is None or options.stderr is None:
            raise NativeSmokeContractError(
                "--report and --stderr are required for validation"
            )
        if options.expected_binary is None:
            raise NativeSmokeContractError(
                "--expected-binary is required for validation"
            )
        if options.expected_binary_sha256 is None:
            raise NativeSmokeContractError(
                "--expected-binary-sha256 is required for validation"
            )
        if options.producer_exit_code is None:
            raise NativeSmokeContractError(
                "--producer-exit-code is required for validation"
            )
        report_text = _read_text(options.report)
        stderr_text = _read_text(options.stderr)
        scenarios = normalize_scenarios(options.scenario)
        validate_native_smoke(
            report_text,
            stderr_text,
            scenarios,
            allowed_stderr_patterns=options.allow_stderr_regex,
            expected_binary=options.expected_binary,
            expected_binary_sha256=options.expected_binary_sha256,
            producer_exit_code=options.producer_exit_code,
        )
    except (NativeSmokeContractError, OSError) as error:
        errors = str(error).splitlines() or [error.__class__.__name__]
        _write_summary(
            options.summary,
            _summary(
                status="failure",
                scenarios=scenarios,
                report_text=report_text,
                errors=errors,
                expected_binary=options.expected_binary,
                expected_binary_sha256=options.expected_binary_sha256,
                producer_exit_code=options.producer_exit_code,
            ),
        )
        print(f"native smoke evidence rejected: {error}", file=sys.stderr)
        return 1

    _write_summary(
        options.summary,
        _summary(
            status="required-pass",
            scenarios=scenarios,
            report_text=report_text,
            errors=(),
            expected_binary=options.expected_binary,
            expected_binary_sha256=options.expected_binary_sha256,
            producer_exit_code=options.producer_exit_code,
        ),
    )
    print(
        "native smoke evidence accepted for required scenarios: "
        + ", ".join(scenarios),
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
