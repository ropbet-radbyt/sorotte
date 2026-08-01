#!/usr/bin/env python3
"""Fail-closed validation for the Windows GUI native-smoke report.

Every selected scenario is required. The boundary validates both observable
completion markers and structured native capability outcomes, rejects every
skip marker, and refuses schema drift or contradictory evidence.
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
    "input_mode",
    "binary",
    "pid",
    "window_title",
    "menu_source",
    "menu_labels",
    "menu_automation_ids",
    "menu_contract",
    "accessible_name_count",
    "accessibility_contract",
    "interaction_steps",
    "interaction_contract",
    "capability_outcomes",
    "closed",
    "duration_ms",
}
REQUIRED_MENU_LABELS = {"File", "Playback", "Advanced", "Window", "Help"}
REQUIRED_MENU_AUTOMATION_IDS = {
    "menu.section.file",
    "menu.section.playback",
    "menu.section.advanced",
    "menu.section.window",
    "menu.section.help",
}
REQUIRED_MENU_SOURCE = "uia-accesskit"
STRICT_PHYSICAL_INPUT_MODE = "strict-physical"
UIA_ONLY_INPUT_MODE = "uia-only"
INPUT_MODES = (STRICT_PHYSICAL_INPUT_MODE, UIA_ONLY_INPUT_MODE)

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
        "open-media-file-detached-disabled",
        "about-opens-and-closes-modal",
        "menu-input-stress-25",
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
    "menu-open-media": (
        "menu-open-media-enabled",
        "menu-open-media-invoked-by-automation-id",
        "menu-open-media-runtime-observed",
    ),
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
GLOBAL_REQUIRED_STEPS = ("file-exit", "file-exit-lifecycle-observed")
FORBIDDEN_STEP_PATTERN = re.compile(r"(?:^|[-_])skipped(?::|[-_]|$)", re.IGNORECASE)
CAPABILITY_CONTRACTS: dict[str, tuple[str, tuple[str, ...]]] = {
    "native.menu.inventory": (
        "uia-accesskit",
        (
            "menu.section.file",
            "menu.section.playback",
            "menu.section.advanced",
            "menu.section.window",
            "menu.section.help",
        ),
    ),
    "native.menu.open-media.detached": (
        "uia-accesskit",
        (
            "menu.open_media=disabled",
            "open-media-file-detached-disabled",
        ),
    ),
    "native.menu.open-media.attached": (
        "uia-accesskit+deterministic-test-player",
        (
            "menu.open_media=enabled",
            "menu-open-media-invoked-by-automation-id",
            "player.open_file=observed",
        ),
    ),
    "native.menu.physical-input": (
        "uia-hit-test+win32-sendinput",
        (
            "menu-input-stress-25",
            "menu-input-single-delivery",
        ),
    ),
    "native.shutdown.file-exit": (
        "accesskit+eframe+lifecycle-jsonl",
        (
            "exit-action-applied",
            "viewport-close-requested",
            "runtime-stop-requested",
            "runtime-worker-stopped",
            "app-drop-complete",
        ),
    ),
}
GLOBAL_REQUIRED_CAPABILITIES = (
    "native.menu.inventory",
    "native.shutdown.file-exit",
)
SCENARIO_REQUIRED_CAPABILITIES: dict[str, tuple[str, ...]] = {
    "baseline": (
        "native.menu.open-media.detached",
        "native.menu.physical-input",
    ),
    "menu-open-media": ("native.menu.open-media.attached",),
}
UIA_ONLY_REQUIRED_STEPS = (
    "uia-only-menu-inventory",
    "uia-only-file-exit",
    "uia-only-file-exit-lifecycle-observed",
)
UIA_ONLY_CAPABILITY_CONTRACTS: dict[
    str, tuple[str, str, tuple[str, ...]]
] = {
    "native.menu.inventory": (
        "required-pass",
        "uia-accesskit",
        (
            "menu.section.file",
            "menu.section.playback",
            "menu.section.advanced",
            "menu.section.window",
            "menu.section.help",
        ),
    ),
    "native.shutdown.file-exit": (
        "required-pass",
        "uia-accesskit+eframe+lifecycle-jsonl",
        (
            "exit-action-applied",
            "viewport-close-requested",
            "runtime-stop-requested",
            "runtime-worker-stopped",
            "app-drop-complete",
        ),
    ),
    "native.menu.physical-input": (
        "optional-skip",
        "local-uia-mode",
        (
            "reason=local-uia-mode",
            "win32-sendinput=disabled",
            "desktop-input-attempt-count=0",
        ),
    ),
    "native.input.focused-keyboard": (
        "optional-skip",
        "local-uia-mode",
        (
            "reason=local-uia-mode",
            "focused-keyboard-fallback=disabled",
            "win32-sendinput=disabled",
            "desktop-input-attempt-count=0",
        ),
    ),
}


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
    input_mode: str = STRICT_PHYSICAL_INPUT_MODE,
    allowed_stderr_patterns: Iterable[str] = (),
    expected_binary: pathlib.Path | None = None,
    expected_binary_sha256: str | None = None,
    producer_exit_code: int = 0,
) -> Mapping[str, Any]:
    if input_mode not in INPUT_MODES:
        raise NativeSmokeContractError(f"unknown native input mode: {input_mode!r}")
    scenario_inventory = tuple(scenarios)
    if input_mode == STRICT_PHYSICAL_INPUT_MODE:
        required_scenarios = normalize_scenarios(scenario_inventory)
    else:
        if scenario_inventory:
            raise NativeSmokeContractError(
                "uia-only native smoke does not accept strict scenario evidence"
            )
        required_scenarios = ()
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

    if report.get("input_mode") != input_mode:
        errors.append(
            "native report input mode differs from the requested validator mode: "
            f"expected={input_mode!r}, got={report.get('input_mode')!r}"
        )

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

    if report.get("menu_source") != REQUIRED_MENU_SOURCE:
        errors.append(
            "native menu source must prove the UIA/AccessKit path "
            f"({REQUIRED_MENU_SOURCE!r}), got {report.get('menu_source')!r}"
        )

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

    menu_automation_ids = report.get("menu_automation_ids")
    if not isinstance(menu_automation_ids, list) or not all(
        isinstance(automation_id, str) for automation_id in menu_automation_ids
    ):
        errors.append("native report menu_automation_ids must be an array of strings")
    else:
        observed_menu_automation_ids = set(menu_automation_ids)
        missing_menu_ids = sorted(
            REQUIRED_MENU_AUTOMATION_IDS - observed_menu_automation_ids
        )
        extra_menu_ids = sorted(
            observed_menu_automation_ids - REQUIRED_MENU_AUTOMATION_IDS
        )
        duplicate_menu_ids = sorted(
            automation_id
            for automation_id in observed_menu_automation_ids
            if menu_automation_ids.count(automation_id) > 1
        )
        if missing_menu_ids:
            errors.append(
                "native menu is missing required automation IDs: "
                + ", ".join(missing_menu_ids)
            )
        if extra_menu_ids:
            errors.append(
                "native menu contains unreviewed automation IDs: "
                + ", ".join(extra_menu_ids)
            )
        if duplicate_menu_ids:
            errors.append(
                "native menu contains duplicate automation IDs: "
                + ", ".join(duplicate_menu_ids)
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

        if input_mode == STRICT_PHYSICAL_INPUT_MODE:
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
        elif steps != list(UIA_ONLY_REQUIRED_STEPS):
            errors.append(
                "uia-only native smoke must report the exact local interaction inventory: "
                f"expected={list(UIA_ONLY_REQUIRED_STEPS)!r}, got={steps!r}"
            )

    expected_interaction_contract = (
        "verified"
        if input_mode == STRICT_PHYSICAL_INPUT_MODE
        else "local-uia-only-non-authoritative"
    )
    if report.get("interaction_contract") != expected_interaction_contract:
        errors.append(
            "interaction contract does not match the requested input mode: "
            f"expected={expected_interaction_contract!r}, "
            f"got {report.get('interaction_contract')!r}"
        )

    if input_mode == STRICT_PHYSICAL_INPUT_MODE:
        required_capability_ids = list(GLOBAL_REQUIRED_CAPABILITIES)
        for scenario in required_scenarios:
            required_capability_ids.extend(
                SCENARIO_REQUIRED_CAPABILITIES.get(scenario, ())
            )
        expected_capability_contracts = {
            capability_id: (
                "required-pass",
                CAPABILITY_CONTRACTS[capability_id][0],
                CAPABILITY_CONTRACTS[capability_id][1],
            )
            for capability_id in required_capability_ids
        }
    else:
        required_capability_ids = list(UIA_ONLY_CAPABILITY_CONTRACTS)
        expected_capability_contracts = UIA_ONLY_CAPABILITY_CONTRACTS
    capability_outcomes = report.get("capability_outcomes")
    observed_capabilities: dict[str, Mapping[str, Any]] = {}
    if not isinstance(capability_outcomes, list):
        errors.append("capability_outcomes must be an array")
    else:
        for index, capability in enumerate(capability_outcomes):
            if not isinstance(capability, Mapping):
                errors.append(
                    f"capability_outcomes[{index}] must be a structured object"
                )
                continue
            expected_keys = {"capability_id", "outcome", "source", "evidence"}
            actual_capability_keys = set(capability)
            if actual_capability_keys != expected_keys:
                errors.append(
                    f"capability_outcomes[{index}] has unreviewed schema: "
                    f"expected={sorted(expected_keys)!r}, "
                    f"observed={sorted(actual_capability_keys)!r}"
                )
                continue
            capability_id = capability.get("capability_id")
            if not isinstance(capability_id, str) or not capability_id:
                errors.append(
                    f"capability_outcomes[{index}].capability_id must be non-empty"
                )
                continue
            if capability_id in observed_capabilities:
                errors.append(f"duplicate native capability outcome: {capability_id}")
                continue
            observed_capabilities[capability_id] = capability

        required_capability_set = set(required_capability_ids)
        for capability_id in required_capability_ids:
            if capability_id not in observed_capabilities:
                errors.append(f"missing required capability {capability_id!r}")
        extra_capability_ids = sorted(
            set(observed_capabilities) - required_capability_set
        )
        if extra_capability_ids:
            errors.append(
                "native report has unreviewed capability outcomes: "
                + ", ".join(extra_capability_ids)
            )

        for capability_id in sorted(
            required_capability_set & set(observed_capabilities)
        ):
            capability = observed_capabilities[capability_id]
            expected_outcome, expected_source, expected_evidence = (
                expected_capability_contracts[capability_id]
            )
            if capability.get("outcome") != expected_outcome:
                errors.append(
                    f"native capability {capability_id!r} must have outcome "
                    f"{expected_outcome!r}, got {capability.get('outcome')!r}"
                )
            if capability.get("source") != expected_source:
                errors.append(
                    f"native capability {capability_id!r} must have source "
                    f"{expected_source!r}, got {capability.get('source')!r}"
                )
            evidence = capability.get("evidence")
            if evidence != list(expected_evidence):
                errors.append(
                    f"native capability {capability_id!r} must have exact evidence "
                    f"{list(expected_evidence)!r}, got {evidence!r}"
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
    input_mode: str,
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
        "input_mode": input_mode,
        "authoritative": input_mode == STRICT_PHYSICAL_INPUT_MODE,
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
    parser.add_argument(
        "--input-mode",
        choices=INPUT_MODES,
        default=STRICT_PHYSICAL_INPUT_MODE,
    )
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
            or options.input_mode != STRICT_PHYSICAL_INPUT_MODE
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
            or options.input_mode != STRICT_PHYSICAL_INPUT_MODE
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
        scenarios = (
            normalize_scenarios(options.scenario)
            if options.input_mode == STRICT_PHYSICAL_INPUT_MODE
            else tuple(options.scenario)
        )
        validate_native_smoke(
            report_text,
            stderr_text,
            scenarios,
            input_mode=options.input_mode,
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
                input_mode=options.input_mode,
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
            status=(
                "required-pass"
                if options.input_mode == STRICT_PHYSICAL_INPUT_MODE
                else "local-pass"
            ),
            input_mode=options.input_mode,
            scenarios=scenarios,
            report_text=report_text,
            errors=(),
            expected_binary=options.expected_binary,
            expected_binary_sha256=options.expected_binary_sha256,
            producer_exit_code=options.producer_exit_code,
        ),
    )
    if options.input_mode == STRICT_PHYSICAL_INPUT_MODE:
        message = "native smoke evidence accepted for required scenarios: " + ", ".join(
            scenarios
        )
    else:
        message = "native UIA-only evidence accepted as local non-authoritative development evidence"
    print(message, file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
