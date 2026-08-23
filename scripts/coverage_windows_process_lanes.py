#!/usr/bin/env python3
"""Collect fail-closed Windows/MSVC process-harness coverage profiles.

This producer deliberately covers non-interactive Windows behavior that the
ordinary Linux workspace profile cannot execute: updater replacement and
recovery, installed-updater self replacement, named-pipe faults, external mpv
process faults, and media-tool child-process faults. Every lane has an exact
libtest inventory, must add a fresh raw LLVM profile, and must remain merge
compatible with the other profiles from this Windows/MSVC producer.

Interactive GUI/UI Automation smoke remains a separate test signal. It is not
instrumented by this producer and the report schema makes that boundary
explicit so a green Windows coverage artifact cannot be mistaken for native UI
coverage.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import stat
import subprocess
import sys
import time
from collections.abc import Mapping, Sequence
from typing import Any, Callable

import coverage_profile_lanes as common


SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-windows-process-coverage-lanes"
PINNED_CARGO_LLVM_COV_VERSION = "0.8.4"
PINNED_RUST_RELEASE = "1.97.1"
PINNED_RUST_COMMIT = "8bab26f4f68e0e26f0bb7960be334d5b520ea452"
PINNED_RUST_HOST = "x86_64-pc-windows-msvc"
PINNED_LLVM_VERSION = "22.1.6"
TARGET_DIR = "target/llvm-cov-windows-process"
COMPATIBILITY_DOMAIN = "windows-x86_64-msvc"
NATIVE_EXCLUSION_REASON = (
    "interactive Windows UI Automation is an uninstrumented, separately "
    "attested smoke signal"
)
MAX_UNTRACKED_FILES = 1_000
MAX_UNTRACKED_FILE_BYTES = 32 * 1024 * 1024

VERSION_COMMAND = ("cargo", "llvm-cov", "--version")
RUSTC_COMMAND = ("rustc", "-vV")
SHOW_ENV_COMMAND = ("cargo", "llvm-cov", "show-env", "--sh")
HEAD_COMMAND = ("git", "rev-parse", "HEAD")
TRACKED_DIFF_COMMAND = ("git", "diff", "--binary", "--no-ext-diff", "HEAD", "--", ".")
UNTRACKED_COMMAND = (
    "git",
    "ls-files",
    "--others",
    "--exclude-standard",
    "-z",
)
MERGE_COMMAND = ("cargo", "llvm-cov", "report", "--summary-only")

UPDATER_TRANSACTION_COMMAND = (
    "cargo",
    "llvm-cov",
    "--locked",
    "-p",
    "sorotte-gui",
    "--all-features",
    "--bin",
    "sorotte-gui-updater",
    "--no-report",
    "--",
    "--nocapture",
)
UPDATER_INSTALLED_COMMAND = (
    "cargo",
    "test",
    "--locked",
    "-p",
    "sorotte-gui",
    "--all-features",
    "--test",
    "updater_self_replacement_windows",
    "--",
    "--nocapture",
)
MPV_NAMED_PIPE_COMMAND = (
    "cargo",
    "test",
    "--locked",
    "-p",
    "sorotte-player-mpv",
    "--all-features",
    "--lib",
    "windows_named_pipe_",
    "--",
    "--nocapture",
)
MPV_EXTERNAL_PROCESS_COMMAND = (
    "cargo",
    "test",
    "--locked",
    "-p",
    "sorotte-player-mpv",
    "--all-features",
    "--lib",
    "external_mpv_",
    "--",
    "--nocapture",
)
MEDIA_TOOL_PROCESS_COMMAND = (
    "cargo",
    "test",
    "--locked",
    "-p",
    "sorotte-gui",
    "--all-features",
    "--lib",
    "process_fault_tests::",
    "--",
    "--nocapture",
)

LANE_ORDER = (
    "updater-transaction-process",
    "updater-installed-self-replacement",
    "mpv-named-pipe",
    "mpv-external-process",
    "media-tool-process",
    "merge-check",
)
PROFILE_LANES = frozenset(LANE_ORDER[:-1])
LANE_COMMANDS = {
    "updater-transaction-process": UPDATER_TRANSACTION_COMMAND,
    "updater-installed-self-replacement": UPDATER_INSTALLED_COMMAND,
    "mpv-named-pipe": MPV_NAMED_PIPE_COMMAND,
    "mpv-external-process": MPV_EXTERNAL_PROCESS_COMMAND,
    "media-tool-process": MEDIA_TOOL_PROCESS_COMMAND,
    "merge-check": MERGE_COMMAND,
}
LANE_INSTRUMENTATION = {
    "updater-transaction-process": "cargo-llvm-cov",
    "updater-installed-self-replacement": "cargo-llvm-cov-show-env",
    "mpv-named-pipe": "cargo-llvm-cov-show-env",
    "mpv-external-process": "cargo-llvm-cov-show-env",
    "media-tool-process": "cargo-llvm-cov-show-env",
    "merge-check": "cargo-llvm-cov-report",
}
LANE_ENVIRONMENT_OVERRIDES = {
    "updater-transaction-process": ("CARGO_TARGET_DIR",),
    **{
        lane: ("CARGO_TARGET_DIR", *sorted(common.EXPECTED_SHOW_ENV_KEYS))
        for lane in PROFILE_LANES
        if lane != "updater-transaction-process"
    },
    "merge-check": ("CARGO_TARGET_DIR",),
}

EXPECTED_TESTS = {
    "updater-transaction-process": (
        "tests::authenticated_prepared_replacements_are_disposable_or_cleanable_by_mode",
        "tests::committed_cleanup_validates_targets_before_deleting_journal",
        "tests::committed_plan_removes_obsolete_files",
        "tests::deterministic_updater_storage_fault_matrix_recovers_complete_old_or_new_installs",
        "tests::elevated_execution_is_rejected_before_all_updater_modes",
        "tests::every_before_and_after_replacement_failure_boundary_rolls_back_the_matrix",
        "tests::extracted_manifest_revalidates_every_payload_digest_and_exact_file_set",
        "tests::failure_on_nth_replacement_rolls_back_every_prior_file",
        "tests::interrupted_atomic_replacement_keeps_executables_invokable_and_recovers",
        "tests::interrupted_prefix_recovery_is_idempotent_at_every_replacement_boundary",
        "tests::legacy_old_gui_invocation_bootstraps_v2_source_transactionally",
        "tests::locked_target_failure_rolls_back_prior_replacements",
        "tests::missing_backup_with_replacement_target_is_ambiguous_and_retains_journal",
        "tests::missing_original_target_and_backup_retains_recovery_journal",
        "tests::parse_args_accepts_exact_legacy_source_and_backup_pair",
        "tests::parse_args_accepts_package_digest_and_restart",
        "tests::parse_args_accepts_recovery_only_reentry",
        "tests::parse_args_requires_authenticated_package",
        "tests::preparation_failure_simulating_disk_exhaustion_leaves_install_unchanged",
        "tests::process_interruption_tests::real_process_termination_recovers_every_durable_transaction_boundary",
        "tests::process_interruption_tests::updater_process_fixture_entrypoint",
        "tests::recovery_rejects_links_for_every_artifact_in_both_journal_modes",
        "tests::relative_path_rejects_parent_and_absolute_components",
        "tests::reparse_or_symlink_package_paths_are_rejected",
        "tests::replacement_journal_artifact_fault_matrix_matches_the_reference_model",
        "tests::stale_bootstrap_cleanup_removes_only_owned_inactive_directories",
        "tests::tampered_later_replacement_does_not_block_rollback_of_prior_file",
        "tests::tampered_prepared_replacement_is_discarded_during_safe_rollback",
        "tests::target_update_lock_serializes_live_updaters_for_the_same_install",
        "tests::tc_updater_002_parent_directory_sync_failure_retains_authenticated_recovery",
        "tests::uncommitted_rollback_processes_entries_in_reverse_order",
        "tests::verified_package_snapshot_rejects_substitution_and_is_used_immutably",
        "tests::windows_parent_directory_sync_reports_reversible_share_denial",
    ),
    "updater-installed-self-replacement": (
        "running_installed_updater_can_replace_its_own_installed_path",
        "running_installed_updater_recovers_interrupted_replacement_and_restarts",
    ),
    "mpv-named-pipe": (
        "tests::ipc_named_pipe_fault_tests::windows_named_pipe_disconnected_adapter_retries_an_explicit_endpoint_when_it_appears",
        "tests::ipc_named_pipe_fault_tests::windows_named_pipe_fragmentation_and_coalescing_preserve_event_response_order",
        "tests::ipc_named_pipe_fault_tests::windows_named_pipe_replacement_client_recovers_on_the_same_pipe_name",
        "tests::ipc_named_pipe_fault_tests::windows_named_pipe_request_ids_wrap_without_losing_response_correlation",
        "tests::ipc_named_pipe_fault_tests::windows_named_pipe_response_correlation_matrix_is_terminal_and_at_most_once",
        "tests::ipc_named_pipe_fault_tests::windows_named_pipe_server_disconnect_before_request_fails_the_write_and_fences_reuse",
        "tests::ipc_named_pipe_fault_tests::windows_named_pipe_truncated_and_closed_responses_fail_boundedly_and_terminally",
        "tests::ipc_named_pipe_fault_tests::windows_named_pipe_withheld_response_honors_deadline_and_fences_later_writes",
        "tests::ipc_tests::windows_named_pipe_read_is_cancelled_at_command_deadline",
    ),
    "mpv-external-process": (
        "tests::ipc_process_fault_tests::external_mpv_hang_times_out_then_kill_reap_releases_process_and_pipe_handles",
        "tests::ipc_process_fault_tests::external_mpv_large_stdio_and_ipc_frame_do_not_block_command_completion",
        "tests::ipc_process_fault_tests::external_mpv_partial_response_and_early_exit_fail_terminally_and_boundedly",
    ),
    "media-tool-process": (
        "app::media_match_support::process_fault_tests::media_match_large_stdout_process_fixture",
        "app::media_match_support::process_fault_tests::media_match_parked_process_fixture",
        "app::media_match_support::process_fault_tests::process_fixture_requires_copied_image_and_exact_target",
        "app::media_match_support::process_fault_tests::timed_out_version_probe_reaps_process_and_releases_executable",
        "app::media_match_support::process_fault_tests::version_probe_drains_finite_output_larger_than_pipe_capacity",
        "app::media_match_support::process_fault_tests::version_probe_preserves_nonzero_exit_status",
        "app::media_match_support::process_fault_tests::version_probe_rejects_unusable_success_output",
        "app::media_match_support::process_fault_tests::version_probe_selects_first_nonempty_line_and_accepts_unterminated_final_line",
    ),
}
EXPECTED_FILTERED_OUT = {
    "updater-transaction-process": 0,
    "updater-installed-self-replacement": 0,
    "mpv-named-pipe": 422,
    "mpv-external-process": 428,
    "media-tool-process": 1152,
}
REQUIRED_INSTRUMENTED_CRATES = frozenset(
    {
        "sorotte_gui_tests",
        "sorotte_gui_updater",
        "sorotte_player_mpv_tests",
        "updater_self_replacement_windows",
    }
)
SKIP_MARKERS = (
    "assertion skipped",
    "test skipped",
    "skipped due to missing",
    "prerequisite unavailable",
)
TEST_LINE = re.compile(r"(?m)^test ([^\r\n]+) \.\.\. ok\r?$")


def capture(
    command: Sequence[str],
    *,
    repo_root: pathlib.Path,
    environment: Mapping[str, str],
) -> common.CommandResult:
    started = time.monotonic()
    process = subprocess.run(
        list(command),
        cwd=repo_root,
        env=dict(environment),
        capture_output=True,
        check=False,
    )
    return common.CommandResult(
        command=tuple(command),
        returncode=process.returncode,
        stdout=process.stdout,
        stderr=process.stderr,
        duration_seconds=time.monotonic() - started,
    )


def require_success(result: common.CommandResult, *, label: str) -> bytes:
    if result.returncode != 0:
        raise common.CoverageProfileLaneError(
            f"{label} exited with status {result.returncode}"
        )
    return result.stdout


def parse_rustc_identity(result: common.CommandResult) -> dict[str, Any]:
    raw = require_success(result, label="rustc identity command")
    try:
        text = raw.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise common.CoverageProfileLaneError(
            f"rustc identity is not UTF-8: {error}"
        ) from error
    lines = text.splitlines()
    if not lines or not re.fullmatch(r"rustc 1\.97\.1 \([^)]+\)", lines[0]):
        raise common.CoverageProfileLaneError("rustc version header drifted")
    values: dict[str, str] = {}
    for line in lines[1:]:
        if ": " not in line:
            raise common.CoverageProfileLaneError(
                f"rustc identity line is malformed: {line!r}"
            )
        key, value = line.split(": ", maxsplit=1)
        if key in values or not key or not value:
            raise common.CoverageProfileLaneError(
                f"rustc identity field is malformed or duplicated: {key!r}"
            )
        values[key] = value
    required = {
        "binary",
        "commit-hash",
        "commit-date",
        "host",
        "release",
        "LLVM version",
    }
    common.require_exact_keys(values, required, label="rustc identity")
    expected = {
        "release": PINNED_RUST_RELEASE,
        "commit-hash": PINNED_RUST_COMMIT,
        "host": PINNED_RUST_HOST,
        "LLVM version": PINNED_LLVM_VERSION,
    }
    for key, value in expected.items():
        if values[key] != value:
            raise common.CoverageProfileLaneError(
                f"rustc {key} must be {value}, received {values[key]!r}"
            )
    if values["binary"] != "rustc":
        raise common.CoverageProfileLaneError("rustc binary identity drifted")
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", values["commit-date"]):
        raise common.CoverageProfileLaneError("rustc commit date is malformed")
    return {
        "command": list(RUSTC_COMMAND),
        "release": values["release"],
        "commit_hash": values["commit-hash"],
        "commit_date": values["commit-date"],
        "host": values["host"],
        "llvm_version": values["LLVM version"],
    }


def platform_identity() -> dict[str, Any]:
    system = platform.system()
    machine = platform.machine().lower()
    normalized = {"amd64": "x86_64", "x86_64": "x86_64"}.get(machine)
    if os.name != "nt" or system != "Windows" or normalized != "x86_64":
        raise common.CoverageProfileLaneError(
            "Windows process coverage requires x86_64 Windows"
        )
    return {
        "system": "Windows",
        "architecture": normalized,
        "rust_host": PINNED_RUST_HOST,
        "compatibility_domain": COMPATIBILITY_DOMAIN,
    }


def working_tree_digest(
    tracked_diff_sha256: str,
    untracked_files: Sequence[Mapping[str, Any]],
) -> str:
    value = {
        "tracked_diff_sha256": tracked_diff_sha256,
        "untracked_files": [dict(item) for item in untracked_files],
    }
    encoded = json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")
    return common.sha256(encoded)


def is_link_or_reparse_status(status: os.stat_result) -> bool:
    reparse_flag = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)
    return stat.S_ISLNK(status.st_mode) or bool(
        getattr(status, "st_file_attributes", 0) & reparse_flag
    )


def source_identity(
    *,
    repo_root: pathlib.Path,
    environment: Mapping[str, str],
) -> dict[str, Any]:
    head_result = capture(
        HEAD_COMMAND,
        repo_root=repo_root,
        environment=environment,
    )
    try:
        head = require_success(
            head_result,
            label="source HEAD command",
        ).decode("ascii", errors="strict").strip()
    except UnicodeError as error:
        raise common.CoverageProfileLaneError(
            f"source HEAD is not ASCII: {error}"
        ) from error
    if not common.FULL_SHA.fullmatch(head):
        raise common.CoverageProfileLaneError("source HEAD is not a full Git SHA")

    diff_result = capture(
        TRACKED_DIFF_COMMAND,
        repo_root=repo_root,
        environment=environment,
    )
    tracked_diff = require_success(
        diff_result,
        label="tracked source diff command",
    )
    tracked_digest = common.sha256(tracked_diff)

    untracked_result = capture(
        UNTRACKED_COMMAND,
        repo_root=repo_root,
        environment=environment,
    )
    raw_untracked = require_success(
        untracked_result,
        label="untracked source inventory command",
    )
    try:
        raw_paths = [
            item.decode("utf-8", errors="strict")
            for item in raw_untracked.split(b"\0")
            if item
        ]
    except UnicodeError as error:
        raise common.CoverageProfileLaneError(
            f"untracked source path is not UTF-8: {error}"
        ) from error
    if raw_paths != sorted(raw_paths) or len(raw_paths) != len(set(raw_paths)):
        raise common.CoverageProfileLaneError(
            "untracked source inventory is not sorted and unique"
        )
    if len(raw_paths) > MAX_UNTRACKED_FILES:
        raise common.CoverageProfileLaneError(
            "untracked source inventory exceeds the bounded file count"
        )

    untracked_files: list[dict[str, Any]] = []
    for relative in raw_paths:
        pure = pathlib.PurePosixPath(relative)
        if pure.is_absolute() or ".." in pure.parts or "\x00" in relative:
            raise common.CoverageProfileLaneError(
                f"unsafe untracked source path {relative!r}"
            )
        unresolved_path = repo_root / pathlib.Path(relative)
        try:
            unresolved_status = unresolved_path.lstat()
        except OSError as error:
            raise common.CoverageProfileLaneError(
                f"cannot inspect untracked source path {relative!r}: {error}"
            ) from error
        if is_link_or_reparse_status(unresolved_status):
            raise common.CoverageProfileLaneError(
                f"untracked source must not be a link or reparse point: {relative}"
            )
        path = common.resolve_within(
            repo_root,
            pathlib.Path(relative),
            label="untracked source file",
        )
        if not path.is_file():
            raise common.CoverageProfileLaneError(
                f"untracked source must be a regular file: {relative}"
            )
        data = path.read_bytes()
        if len(data) > MAX_UNTRACKED_FILE_BYTES:
            raise common.CoverageProfileLaneError(
                f"untracked source exceeds size bound: {relative}"
            )
        untracked_files.append(
            {
                "path": relative,
                "size_bytes": len(data),
                "sha256": common.sha256(data),
            }
        )

    return {
        "head_command": list(HEAD_COMMAND),
        "head_commit": head,
        "tracked_diff_command": list(TRACKED_DIFF_COMMAND),
        "tracked_diff_size_bytes": len(tracked_diff),
        "tracked_diff_sha256": tracked_digest,
        "untracked_command": list(UNTRACKED_COMMAND),
        "untracked_file_count": len(untracked_files),
        "untracked_files": untracked_files,
        "is_clean": not tracked_diff and not untracked_files,
        "working_tree_sha256": working_tree_digest(
            tracked_digest,
            untracked_files,
        ),
    }


def parse_show_env(
    result: common.CommandResult,
    *,
    repo_root: pathlib.Path,
) -> tuple[dict[str, str], dict[str, Any], pathlib.Path]:
    raw = require_success(result, label="cargo llvm-cov show-env")
    try:
        text = raw.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise common.CoverageProfileLaneError(
            f"cargo llvm-cov show-env is not UTF-8: {error}"
        ) from error
    environment: dict[str, str] = {}
    for line_number, line in enumerate(text.splitlines(), start=1):
        if not line.startswith("export ") or "=" not in line:
            raise common.CoverageProfileLaneError(
                f"show-env line {line_number} is not export KEY=VALUE"
            )
        key, value = line.removeprefix("export ").split("=", maxsplit=1)
        if not common.ENVIRONMENT_KEY.fullmatch(key) or key in environment:
            raise common.CoverageProfileLaneError(
                f"show-env line {line_number} is unsafe or duplicated"
            )
        environment[key] = common.decode_show_env_value(value, key=key)
    common.require_exact_keys(
        environment,
        common.EXPECTED_SHOW_ENV_KEYS,
        label="cargo llvm-cov show-env",
    )
    if (
        environment["CARGO_LLVM_COV"] != "1"
        or environment["CARGO_LLVM_COV_SHOW_ENV"] != "1"
        or environment["__CARGO_LLVM_COV_RUSTC_WRAPPER"] != "1"
    ):
        raise common.CoverageProfileLaneError(
            "show-env instrumentation sentinels are not enabled"
        )
    wrapper = pathlib.Path(environment["RUSTC_WRAPPER"]).name.lower()
    if wrapper not in {"cargo-llvm-cov", "cargo-llvm-cov.exe"}:
        raise common.CoverageProfileLaneError(
            "show-env RUSTC_WRAPPER is not cargo-llvm-cov"
        )
    rustflags = environment[
        "__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS"
    ].split("\x1f")
    if "instrument-coverage" not in rustflags:
        raise common.CoverageProfileLaneError(
            "show-env rustflags do not enable instrument-coverage"
        )

    target_dir = common.resolve_within(
        repo_root,
        pathlib.Path(environment["CARGO_LLVM_COV_TARGET_DIR"]),
        label="cargo-llvm-cov target directory",
    )
    build_dir = common.resolve_within(
        repo_root,
        pathlib.Path(environment["CARGO_LLVM_COV_BUILD_DIR"]),
        label="cargo-llvm-cov build directory",
    )
    expected_target = common.resolve_within(
        repo_root,
        pathlib.Path(TARGET_DIR),
        label="Windows process coverage target",
    )
    if target_dir != expected_target or build_dir != expected_target:
        raise common.CoverageProfileLaneError(
            f"show-env must use the isolated {TARGET_DIR} directory"
        )
    profile_pattern = pathlib.Path(environment["LLVM_PROFILE_FILE"])
    profile_root = common.resolve_within(
        repo_root,
        profile_pattern.parent,
        label="LLVM profile directory",
    )
    if profile_root != expected_target:
        raise common.CoverageProfileLaneError(
            "LLVM raw profiles must write directly into the isolated target"
        )
    pattern_name = profile_pattern.name
    common.validate_profile_pattern_name(pattern_name)

    crate_names = [
        item
        for item in environment[
            "__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES"
        ].split(",")
        if item
    ]
    missing = sorted(REQUIRED_INSTRUMENTED_CRATES - set(crate_names))
    if missing:
        raise common.CoverageProfileLaneError(
            f"show-env omits required instrumented crates {missing}"
        )
    summary = {
        "keys": sorted(environment),
        "profile_pattern": (
            profile_root.relative_to(repo_root.resolve()) / pattern_name
        ).as_posix(),
        "target_dir": common.repo_relative(repo_root, target_dir),
        "build_dir": common.repo_relative(repo_root, build_dir),
        "crate_count": len(crate_names),
        "required_crates": sorted(REQUIRED_INSTRUMENTED_CRATES),
    }
    return environment, summary, profile_root


def libtest_oracle(
    lane: str,
    stdout: bytes,
    stderr: bytes,
) -> dict[str, Any]:
    try:
        text = (stdout + b"\n" + stderr).decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise common.CoverageProfileLaneError(
            f"{lane} output is not valid UTF-8: {error}"
        ) from error
    found_markers = [marker for marker in SKIP_MARKERS if marker in text.lower()]
    if found_markers:
        raise common.CoverageProfileLaneError(
            f"{lane} output contains skipped-oracle markers {found_markers}"
        )
    if re.search(r"(?m)^running 0 tests\r?$", text):
        raise common.CoverageProfileLaneError(
            f"{lane} selector executed zero tests"
        )
    expected = tuple(sorted(EXPECTED_TESTS[lane]))
    observed = TEST_LINE.findall(text)
    if len(observed) != len(set(observed)):
        raise common.CoverageProfileLaneError(
            f"{lane} output contains duplicate passing test records"
        )
    if tuple(sorted(observed)) != expected:
        missing = sorted(set(expected) - set(observed))
        extra = sorted(set(observed) - set(expected))
        raise common.CoverageProfileLaneError(
            f"{lane} exact test inventory drifted; "
            f"missing={missing}, extra={extra}"
        )
    count = len(expected)
    running = re.findall(rf"(?m)^running {count} tests\r?$", text)
    if len(running) != 1:
        raise common.CoverageProfileLaneError(
            f"{lane} did not report exactly one non-zero running count"
        )
    filtered = EXPECTED_FILTERED_OUT[lane]
    summary = re.findall(
        rf"test result: ok\. {count} passed; 0 failed; 0 ignored; "
        rf"0 measured; {filtered} filtered out",
        text,
    )
    if len(summary) != 1:
        raise common.CoverageProfileLaneError(
            f"{lane} libtest summary or filtered count drifted"
        )
    return {
        "kind": "libtest-exact-windows-process",
        "passed": count,
        "failed": 0,
        "ignored": 0,
        "filtered_out": filtered,
        "tests": list(expected),
        "skip_markers": [],
    }


def merge_oracle(stdout: bytes, stderr: bytes) -> dict[str, Any]:
    return common.merge_oracle(stdout, stderr)


def lane_record(
    *,
    lane: str,
    result: common.CommandResult,
    before: Mapping[str, common.ProfileState],
    after: Mapping[str, common.ProfileState],
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
) -> dict[str, Any]:
    before = common.profile_inventory(profile_root, repo_root=repo_root)
    result = common.run_command(
        LANE_COMMANDS[lane],
        cwd=repo_root,
        environment=environment,
    )
    stdout_metadata, stderr_metadata = common.write_logs(
        result,
        lane=lane,
        log_root=log_root,
        repo_root=repo_root,
    )
    after = common.profile_inventory(profile_root, repo_root=repo_root)
    errors: list[str] = []
    removed = sorted(set(before) - set(after))
    if removed:
        errors.append(f"lane removed {len(removed)} existing LLVM raw profiles")
    deltas: list[dict[str, Any]] = []
    if result.returncode != 0:
        errors.append(f"command exited with status {result.returncode}")
    try:
        deltas = common.profile_delta(before, after, repo_root=repo_root)
    except common.CoverageProfileLaneError as error:
        errors.append(str(error))
    if require_profile_delta and not deltas:
        errors.append("lane produced no fresh or changed LLVM raw profile")
    oracle: dict[str, Any] | None = None
    if result.returncode == 0:
        try:
            oracle = oracle_builder(result.stdout, result.stderr)
        except common.CoverageProfileLaneError as error:
            errors.append(str(error))
    return lane_record(
        lane=lane,
        result=result,
        before=before,
        after=after,
        deltas=deltas,
        removed_count=len(removed),
        stdout_metadata=stdout_metadata,
        stderr_metadata=stderr_metadata,
        oracle=oracle,
        errors=errors,
    )


def validate_digest(value: Any, *, label: str) -> str:
    digest = common.require_string(value, label=label)
    if not common.SHA256.fullmatch(digest):
        raise common.CoverageProfileLaneError(
            f"{label} must be 64 lowercase hexadecimal characters"
        )
    return digest


def validate_source_identity(value: Any) -> None:
    source = common.require_mapping(value, label="source identity")
    common.require_exact_keys(
        source,
        {
            "head_command",
            "head_commit",
            "tracked_diff_command",
            "tracked_diff_size_bytes",
            "tracked_diff_sha256",
            "untracked_command",
            "untracked_file_count",
            "untracked_files",
            "is_clean",
            "working_tree_sha256",
        },
        label="source identity",
    )
    if source.get("head_command") != list(HEAD_COMMAND):
        raise common.CoverageProfileLaneError("source HEAD command drifted")
    head = common.require_string(
        source.get("head_commit"),
        label="source HEAD commit",
    )
    if not common.FULL_SHA.fullmatch(head):
        raise common.CoverageProfileLaneError(
            "source HEAD commit must be a full Git SHA"
        )
    if source.get("tracked_diff_command") != list(TRACKED_DIFF_COMMAND):
        raise common.CoverageProfileLaneError("tracked diff command drifted")
    if source.get("untracked_command") != list(UNTRACKED_COMMAND):
        raise common.CoverageProfileLaneError(
            "untracked inventory command drifted"
        )
    tracked_size = common.require_int(
        source.get("tracked_diff_size_bytes"),
        label="tracked diff size",
    )
    tracked_digest = validate_digest(
        source.get("tracked_diff_sha256"),
        label="tracked diff SHA-256",
    )
    files = common.require_list(
        source.get("untracked_files"),
        label="untracked files",
    )
    if len(files) > MAX_UNTRACKED_FILES:
        raise common.CoverageProfileLaneError(
            "untracked source inventory exceeds the bounded file count"
        )
    validated_files: list[dict[str, Any]] = []
    previous = ""
    for index, raw in enumerate(files):
        item = common.require_mapping(raw, label=f"untracked file {index}")
        common.require_exact_keys(
            item,
            {"path", "size_bytes", "sha256"},
            label=f"untracked file {index}",
        )
        path = common.require_string(
            item.get("path"),
            label=f"untracked file {index} path",
        )
        pure = pathlib.PurePosixPath(path)
        if pure.is_absolute() or ".." in pure.parts or path <= previous:
            raise common.CoverageProfileLaneError(
                "untracked file paths must be safe, sorted, and unique"
            )
        previous = path
        size = common.require_int(
            item.get("size_bytes"),
            label=f"untracked file {index} size",
        )
        if size > MAX_UNTRACKED_FILE_BYTES:
            raise common.CoverageProfileLaneError(
                "untracked source exceeds size bound"
            )
        digest = validate_digest(
            item.get("sha256"),
            label=f"untracked file {index} SHA-256",
        )
        validated_files.append(
            {"path": path, "size_bytes": size, "sha256": digest}
        )
    if source.get("untracked_file_count") != len(validated_files):
        raise common.CoverageProfileLaneError(
            "untracked source count does not match its inventory"
        )
    is_clean = source.get("is_clean")
    if not isinstance(is_clean, bool) or is_clean != (
        tracked_size == 0 and not validated_files
    ):
        raise common.CoverageProfileLaneError(
            "source clean-state claim does not match its inventory"
        )
    expected_working_digest = working_tree_digest(
        tracked_digest,
        validated_files,
    )
    if source.get("working_tree_sha256") != expected_working_digest:
        raise common.CoverageProfileLaneError(
            "working-tree digest does not match source evidence"
        )


def validate_instrumentation(value: Any) -> None:
    instrumentation = common.require_mapping(
        value,
        label="instrumentation environment",
    )
    common.require_exact_keys(
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
    if instrumentation.get("keys") != sorted(common.EXPECTED_SHOW_ENV_KEYS):
        raise common.CoverageProfileLaneError(
            "instrumentation environment key inventory drifted"
        )
    if (
        instrumentation.get("target_dir") != TARGET_DIR
        or instrumentation.get("build_dir") != TARGET_DIR
    ):
        raise common.CoverageProfileLaneError(
            "instrumentation escaped the isolated Windows target"
        )
    pattern = common.require_string(
        instrumentation.get("profile_pattern"),
        label="instrumentation profile pattern",
    )
    if not pattern.startswith(f"{TARGET_DIR}/") or not pattern.endswith(
        ".profraw"
    ):
        raise common.CoverageProfileLaneError(
            "instrumentation profile pattern drifted"
        )
    common.require_int(
        instrumentation.get("crate_count"),
        label="instrumentation crate count",
        minimum=len(REQUIRED_INSTRUMENTED_CRATES),
    )
    if instrumentation.get("required_crates") != sorted(
        REQUIRED_INSTRUMENTED_CRATES
    ):
        raise common.CoverageProfileLaneError(
            "instrumentation required-crate inventory drifted"
        )


def expected_oracle(lane: str) -> dict[str, Any]:
    if lane == "merge-check":
        return {"kind": "llvm-profile-merge", "summary_detected": True}
    tests = sorted(EXPECTED_TESTS[lane])
    return {
        "kind": "libtest-exact-windows-process",
        "passed": len(tests),
        "failed": 0,
        "ignored": 0,
        "filtered_out": EXPECTED_FILTERED_OUT[lane],
        "tests": tests,
        "skip_markers": [],
    }


def validate_report_document(document: Any) -> Mapping[str, Any]:
    report = common.require_mapping(
        document,
        label="Windows process coverage report",
    )
    common.require_exact_keys(
        report,
        {
            "schema_version",
            "kind",
            "status",
            "source_identity",
            "producer",
            "coverage_boundary",
            "instrumentation_environment",
            "profile_reset",
            "lane_order",
            "lanes",
            "errors",
        },
        label="Windows process coverage report",
    )
    if (
        report.get("schema_version") != SCHEMA_VERSION
        or report.get("kind") != REPORT_KIND
    ):
        raise common.CoverageProfileLaneError(
            "Windows process coverage report has an unsupported schema"
        )
    if report.get("status") != "passed":
        raise common.CoverageProfileLaneError(
            f"Windows process coverage report status is "
            f"{report.get('status')!r}"
        )
    if common.require_list(report.get("errors"), label="report errors"):
        raise common.CoverageProfileLaneError(
            "passed Windows process coverage report must contain no errors"
        )

    validate_source_identity(report.get("source_identity"))
    producer = common.require_mapping(report.get("producer"), label="producer")
    common.require_exact_keys(
        producer,
        {"cargo_llvm_cov", "rustc", "platform"},
        label="producer",
    )
    cargo = common.require_mapping(
        producer.get("cargo_llvm_cov"),
        label="cargo-llvm-cov producer",
    )
    if cargo != {
        "version": PINNED_CARGO_LLVM_COV_VERSION,
        "command": list(VERSION_COMMAND),
    }:
        raise common.CoverageProfileLaneError(
            "coverage report has unpinned cargo-llvm-cov metadata"
        )
    rustc = common.require_mapping(producer.get("rustc"), label="rustc producer")
    common.require_exact_keys(
        rustc,
        {
            "command",
            "release",
            "commit_hash",
            "commit_date",
            "host",
            "llvm_version",
        },
        label="rustc producer",
    )
    if (
        rustc.get("command") != list(RUSTC_COMMAND)
        or rustc.get("release") != PINNED_RUST_RELEASE
        or rustc.get("commit_hash") != PINNED_RUST_COMMIT
        or rustc.get("host") != PINNED_RUST_HOST
        or rustc.get("llvm_version") != PINNED_LLVM_VERSION
        or not re.fullmatch(
            r"\d{4}-\d{2}-\d{2}",
            str(rustc.get("commit_date", "")),
        )
    ):
        raise common.CoverageProfileLaneError(
            "coverage report has unpinned rustc metadata"
        )
    if producer.get("platform") != {
        "system": "Windows",
        "architecture": "x86_64",
        "rust_host": PINNED_RUST_HOST,
        "compatibility_domain": COMPATIBILITY_DOMAIN,
    }:
        raise common.CoverageProfileLaneError(
            "coverage report has the wrong platform compatibility domain"
        )

    if report.get("coverage_boundary") != {
        "interactive_native_included": False,
        "interactive_native_reason": NATIVE_EXCLUSION_REASON,
        "profile_merge_scope": COMPATIBILITY_DOMAIN,
    }:
        raise common.CoverageProfileLaneError(
            "Windows coverage boundary is missing or overclaims native UI"
        )
    validate_instrumentation(report.get("instrumentation_environment"))

    reset = common.require_mapping(report.get("profile_reset"), label="profile reset")
    common.require_exact_keys(
        reset,
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
    if (
        reset.get("kind") != "fresh-profile-reset"
        or reset.get("profile_root") != TARGET_DIR
    ):
        raise common.CoverageProfileLaneError(
            "Windows profile reset did not target the isolated profile root"
        )
    for field in ("removed_raw_profile_count", "removed_merged_profile_count"):
        common.require_int(reset.get(field), label=f"profile reset {field}")
    for field in (
        "remaining_raw_profile_count",
        "remaining_merged_profile_count",
    ):
        if common.require_int(
            reset.get(field),
            label=f"profile reset {field}",
        ) != 0:
            raise common.CoverageProfileLaneError(
                "coverage profile reset retained stale profile artifacts"
            )

    if report.get("lane_order") != list(LANE_ORDER):
        raise common.CoverageProfileLaneError(
            "Windows process lane order is incomplete or reordered"
        )
    lanes = common.require_mapping(report.get("lanes"), label="coverage lanes")
    common.require_exact_keys(lanes, set(LANE_ORDER), label="coverage lanes")
    previous_count = 0
    for lane in LANE_ORDER:
        entry = common.require_mapping(lanes.get(lane), label=f"{lane} lane")
        common.require_exact_keys(
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
            raise common.CoverageProfileLaneError(f"{lane} lane did not pass")
        if entry.get("command") != list(LANE_COMMANDS[lane]):
            raise common.CoverageProfileLaneError(f"{lane} command drifted")
        if entry.get("instrumentation") != LANE_INSTRUMENTATION[lane]:
            raise common.CoverageProfileLaneError(
                f"{lane} instrumentation contract drifted"
            )
        if entry.get("environment_overrides") != list(
            LANE_ENVIRONMENT_OVERRIDES[lane]
        ):
            raise common.CoverageProfileLaneError(
                f"{lane} environment override inventory drifted"
            )
        common.require_number(
            entry.get("duration_seconds"),
            label=f"{lane} duration",
        )
        common.validate_file_metadata(entry.get("stdout"), label=f"{lane} stdout")
        common.validate_file_metadata(entry.get("stderr"), label=f"{lane} stderr")
        before = common.require_int(
            entry.get("profile_count_before"),
            label=f"{lane} profiles before",
        )
        after = common.require_int(
            entry.get("profile_count_after"),
            label=f"{lane} profiles after",
        )
        if before != previous_count:
            raise common.CoverageProfileLaneError(
                f"{lane} profile inventory is not continuous"
            )
        deltas = common.require_list(
            entry.get("profile_deltas"),
            label=f"{lane} profile deltas",
        )
        delta_count = common.require_int(
            entry.get("profile_delta_count"),
            label=f"{lane} profile delta count",
        )
        if delta_count != len(deltas):
            raise common.CoverageProfileLaneError(
                f"{lane} profile delta count does not match its inventory"
            )
        removed_count = common.require_int(
            entry.get("profile_removed_count"),
            label=f"{lane} removed profile count",
        )
        if removed_count != 0:
            raise common.CoverageProfileLaneError(
                f"{lane} removed profiles created by a preceding lane"
            )
        if lane in PROFILE_LANES and (not deltas or after == 0):
            raise common.CoverageProfileLaneError(
                f"{lane} produced no fresh instrumented profile"
            )
        if lane == "merge-check" and (deltas or after != before):
            raise common.CoverageProfileLaneError(
                "merge check unexpectedly changed the raw profile inventory"
            )
        for index, delta in enumerate(deltas):
            common.validate_profile_delta(
                delta,
                label=f"{lane} profile delta {index}",
            )
            path = common.require_string(
                common.require_mapping(
                    delta,
                    label=f"{lane} profile delta {index}",
                ).get("path"),
                label=f"{lane} profile delta {index} path",
            )
            if not path.startswith(f"{TARGET_DIR}/"):
                raise common.CoverageProfileLaneError(
                    f"{lane} profile escaped the isolated target"
                )
        if common.require_list(entry.get("errors"), label=f"{lane} errors"):
            raise common.CoverageProfileLaneError(
                f"passed {lane} lane must contain no errors"
            )
        if entry.get("oracle") != expected_oracle(lane):
            raise common.CoverageProfileLaneError(
                f"{lane} oracle does not match the exact required result"
            )
        previous_count = after
    return report


def strict_load_report(path: pathlib.Path) -> Mapping[str, Any]:
    try:
        data = path.read_bytes()
    except OSError as error:
        raise common.CoverageProfileLaneError(
            f"cannot read Windows process coverage report {path}: {error}"
        ) from error
    if len(data) > common.MAX_REPORT_BYTES:
        raise common.CoverageProfileLaneError(
            "Windows process coverage report exceeds size bound"
        )
    return validate_report_document(
        common.parse_json(data, label="Windows process coverage report")
    )


def failed_report() -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": REPORT_KIND,
        "status": "failed",
        "source_identity": None,
        "producer": None,
        "coverage_boundary": {
            "interactive_native_included": False,
            "interactive_native_reason": NATIVE_EXCLUSION_REASON,
            "profile_merge_scope": COMPATIBILITY_DOMAIN,
        },
        "instrumentation_environment": None,
        "profile_reset": None,
        "lane_order": list(LANE_ORDER),
        "lanes": {},
        "errors": [],
    }


def run_collection(args: argparse.Namespace) -> int:
    repo_root = pathlib.Path(args.repo_root).resolve()
    output = common.resolve_within(
        repo_root,
        pathlib.Path(args.output),
        label="Windows process coverage report output",
    )
    log_root = output.parent / "coverage-windows-process-logs"
    environment = dict(os.environ)
    report = failed_report()
    try:
        if not (repo_root / "Cargo.toml").is_file():
            raise common.CoverageProfileLaneError(
                f"repository root has no Cargo.toml: {repo_root}"
            )
        report["source_identity"] = source_identity(
            repo_root=repo_root,
            environment=environment,
        )
        version_result = common.run_command(
            VERSION_COMMAND,
            cwd=repo_root,
            environment=environment,
        )
        version = common.parse_producer_version(version_result)
        rustc = parse_rustc_identity(
            common.run_command(
                RUSTC_COMMAND,
                cwd=repo_root,
                environment=environment,
            )
        )
        report["producer"] = {
            "cargo_llvm_cov": {
                "version": version,
                "command": list(VERSION_COMMAND),
            },
            "rustc": rustc,
            "platform": platform_identity(),
        }

        target = common.resolve_within(
            repo_root,
            pathlib.Path(TARGET_DIR),
            label="Windows process coverage target",
        )
        target.mkdir(parents=True, exist_ok=True)
        coverage_environment = dict(environment)
        coverage_environment["CARGO_TARGET_DIR"] = str(target)
        show_env_result = common.run_command(
            SHOW_ENV_COMMAND,
            cwd=repo_root,
            environment=coverage_environment,
        )
        (
            instrumentation_environment,
            instrumentation_summary,
            profile_root,
        ) = parse_show_env(show_env_result, repo_root=repo_root)
        report["instrumentation_environment"] = instrumentation_summary
        report["profile_reset"] = common.reset_profile_artifacts(
            profile_root,
            repo_root=repo_root,
        )

        instrumented_environment = dict(coverage_environment)
        instrumented_environment.update(instrumentation_environment)
        instrumented_environment["CARGO_TARGET_DIR"] = str(
            profile_root / "llvm-cov-target"
        )
        for lane in LANE_ORDER:
            if lane == "merge-check":
                lane_environment = coverage_environment
                oracle_builder = merge_oracle
                require_delta = False
            elif lane == "updater-transaction-process":
                lane_environment = coverage_environment
                oracle_builder = (
                    lambda stdout, stderr, current=lane: libtest_oracle(
                        current,
                        stdout,
                        stderr,
                    )
                )
                require_delta = True
            else:
                lane_environment = instrumented_environment
                oracle_builder = (
                    lambda stdout, stderr, current=lane: libtest_oracle(
                        current,
                        stdout,
                        stderr,
                    )
                )
                require_delta = True
            record = execute_lane(
                lane=lane,
                repo_root=repo_root,
                profile_root=profile_root,
                environment=lane_environment,
                log_root=log_root,
                oracle_builder=oracle_builder,
                require_profile_delta=require_delta,
            )
            report["lanes"][lane] = record
            if record["status"] != "passed":
                raise common.CoverageProfileLaneError(
                    f"{lane} Windows process coverage lane failed"
                )

        report["status"] = "passed"
        validate_report_document(report)
    except (common.CoverageProfileLaneError, OSError) as error:
        report["status"] = "failed"
        report["errors"].append(str(error))
        try:
            common.atomic_write_json(output, report)
        except (common.CoverageProfileLaneError, OSError) as write_error:
            print(
                "Windows process coverage collection failed and its report "
                f"could not be written: {write_error}",
                file=sys.stderr,
            )
            return 2
        print(
            f"Windows process coverage collection failed: {error}",
            file=sys.stderr,
        )
        return 1

    common.atomic_write_json(output, report)
    print(f"Windows process coverage profiles passed: {output}")
    return 0


def validate_command(args: argparse.Namespace) -> int:
    try:
        strict_load_report(pathlib.Path(args.report))
    except common.CoverageProfileLaneError as error:
        print(
            f"Windows process coverage report validation failed: {error}",
            file=sys.stderr,
        )
        return 1
    print(f"Windows process coverage report passed: {args.report}")
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
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
