from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import subprocess
import sys
from typing import Any, Sequence


TARGET_NAME = "protocol_line"
FRAMED_SESSION_TARGET_NAME = "framed_session"
MAX_TOTAL_SECONDS = 900
MAX_INPUT_BYTES = 65_536
PER_INPUT_TIMEOUT_SECONDS = 5
RSS_LIMIT_MB = 2_048
MINIMIZE_TIMEOUT_SECONDS = 120
REPORT_SCHEMA = "sorotte-protocol-fuzz-v1"
FRAMED_SESSION_REPORT_SCHEMA = "sorotte-framed-session-fuzz-v1"
EXPECTED_CARGO_FUZZ_VERSION = "cargo-fuzz 0.13.2"
SOURCE_SHA_PATTERN = re.compile(r"^[0-9a-f]{40}$")
BOUND_FIXED_SOURCE_PATHS = (
    ".github/workflows/rust-fuzz.yml",
    "Cargo.toml",
    "rust-toolchain.toml",
    "coverage/behaviors.toml",
    "coverage/known-defects.toml",
    "crates/sorotte-protocol/Cargo.toml",
    "fuzz/Cargo.toml",
    "fuzz/Cargo.lock",
    "fuzz/fuzz_targets/protocol_line.rs",
    "fuzz/run_protocol_fuzz.py",
    "scripts/known_defect_policy.py",
    "scripts/tests/test_known_defect_policy.py",
    "scripts/tests/test_protocol_fuzz_policy.py",
)
PROTOCOL_SOURCE_DIRECTORY = "crates/sorotte-protocol/src"
FRAMED_SESSION_BOUND_FIXED_SOURCE_PATHS = (
    ".github/workflows/rust-fuzz.yml",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "coverage/behaviors.toml",
    "coverage/known-defects.toml",
    "fuzz/Cargo.toml",
    "fuzz/Cargo.lock",
    "fuzz/fuzz_targets/framed_session.rs",
    "fuzz/run_protocol_fuzz.py",
    "requirements/ci-policy.txt",
    "scripts/known_defect_policy.py",
    "scripts/tests/test_known_defect_policy.py",
    "scripts/tests/test_protocol_fuzz_policy.py",
)
FRAMED_SESSION_SOURCE_DIRECTORY = "crates"
SUPPORTED_TARGETS = (TARGET_NAME, FRAMED_SESSION_TARGET_NAME)
REQUIRED_FINAL_STATISTICS = (
    "number_of_executed_units",
    "average_exec_per_sec",
    "new_units_added",
    "slowest_unit_time_sec",
    "peak_rss_mb",
)
FINAL_STATISTIC_PATTERN = re.compile(r"^stat::([a-zA-Z0-9_]+):\s*(.*?)\s*$")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(64 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def sha256_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def manifest_digest(entries: list[dict[str, Any]]) -> str:
    encoded = json.dumps(
        entries,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return sha256_bytes(encoded)


def file_manifest_entry(
    path: pathlib.Path,
    repository_root: pathlib.Path,
) -> dict[str, Any]:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"bound source must be a direct regular file: {path}")
    resolved = path.resolve()
    require_relative_to(resolved, repository_root, "bound source")
    return {
        "path": resolved.relative_to(repository_root).as_posix(),
        "bytes": resolved.stat().st_size,
        "sha256": sha256_file(resolved),
    }


def bound_source_manifest(
    repository_root: pathlib.Path,
    target_name: str = TARGET_NAME,
) -> dict[str, Any]:
    repository_root = repository_root.resolve()
    if target_name == TARGET_NAME:
        fixed_paths = BOUND_FIXED_SOURCE_PATHS
        source_directory = PROTOCOL_SOURCE_DIRECTORY
        source_label = "protocol"
        source_filter = lambda path: path.suffix == ".rs"
    elif target_name == FRAMED_SESSION_TARGET_NAME:
        fixed_paths = FRAMED_SESSION_BOUND_FIXED_SOURCE_PATHS
        source_directory = FRAMED_SESSION_SOURCE_DIRECTORY
        source_label = "workspace"
        source_filter = lambda path: path.is_file()
    else:
        raise ValueError(f"unsupported fuzz target: {target_name}")

    paths = [repository_root / path for path in fixed_paths]
    source_root = repository_root / source_directory
    if source_root.is_symlink() or not source_root.is_dir():
        raise ValueError(
            f"{source_label} source directory must be a direct directory: "
            f"{source_root}"
        )
    sources = sorted(
        (path for path in source_root.rglob("*") if source_filter(path)),
        key=lambda path: path.relative_to(repository_root).as_posix(),
    )
    if not sources:
        raise ValueError(
            f"{source_label} source binding must contain at least one source file"
        )
    paths.extend(sources)

    entries = [
        file_manifest_entry(path, repository_root)
        for path in sorted(
            paths,
            key=lambda path: path.relative_to(repository_root).as_posix(),
        )
    ]
    names = [entry["path"] for entry in entries]
    if len(names) != len(set(names)):
        raise ValueError("bound source inventory contains duplicate paths")
    return {
        "file_count": len(entries),
        "total_bytes": sum(entry["bytes"] for entry in entries),
        "aggregate_sha256": manifest_digest(entries),
        "files": entries,
    }


def direct_file_manifest(directory: pathlib.Path) -> list[dict[str, Any]]:
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError(f"manifest directory must be a direct directory: {directory}")
    entries = sorted(directory.iterdir(), key=lambda entry: entry.name)
    manifest = []
    for entry in entries:
        if entry.is_symlink() or not entry.is_file():
            raise ValueError(f"manifest entry must be a direct regular file: {entry}")
        manifest.append(
            {
                "name": entry.name,
                "bytes": entry.stat().st_size,
                "sha256": sha256_file(entry),
            }
        )
    return manifest


def manifest_summary(entries: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "file_count": len(entries),
        "total_bytes": sum(entry["bytes"] for entry in entries),
        "aggregate_sha256": manifest_digest(entries),
        "files": entries,
    }


def parse_final_statistics(log_text: str) -> dict[str, int | float | str]:
    statistics: dict[str, int | float | str] = {}
    for line in log_text.splitlines():
        match = FINAL_STATISTIC_PATTERN.match(line.strip())
        if match is None:
            continue
        name, raw_value = match.groups()
        if name in statistics:
            raise ValueError(f"duplicate libFuzzer final statistic: {name}")
        try:
            value: int | float | str = int(raw_value)
        except ValueError:
            try:
                value = float(raw_value)
            except ValueError:
                value = raw_value
        statistics[name] = value
    return statistics


def validate_final_statistics(statistics: dict[str, int | float | str]) -> None:
    missing = sorted(set(REQUIRED_FINAL_STATISTICS) - set(statistics))
    if missing:
        raise ValueError(
            "libFuzzer final statistics are incomplete: " + ", ".join(missing)
        )
    executed = statistics["number_of_executed_units"]
    if not isinstance(executed, int) or executed <= 0:
        raise ValueError("libFuzzer must execute at least one unit")


def classify_status(
    *,
    exit_code: int,
    timed_out: bool,
    source_stable: bool,
    seed_source_stable: bool,
    evidence_errors: Sequence[str],
) -> str:
    if not source_stable or not seed_source_stable:
        return "source_drift"
    if evidence_errors:
        return "evidence_failed"
    if timed_out:
        return "timed_out"
    return "passed" if exit_code == 0 else "failed"


def atomic_write_json(path: pathlib.Path, payload: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def checked_output(command: Sequence[str], cwd: pathlib.Path) -> str:
    return subprocess.run(
        list(command),
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    ).stdout.strip()


def require_relative_to(path: pathlib.Path, root: pathlib.Path, label: str) -> None:
    try:
        path.relative_to(root)
    except ValueError as error:
        raise ValueError(f"{label} must remain under {root}") from error


def copy_seed_corpus(
    source: pathlib.Path,
    destination: pathlib.Path,
    expected_count: int,
) -> list[dict[str, Any]]:
    if not source.is_dir():
        raise ValueError(f"seed corpus is not a directory: {source}")

    entries = sorted(source.iterdir(), key=lambda entry: entry.name)
    if len(entries) != expected_count:
        raise ValueError(
            f"seed corpus must contain exactly {expected_count} direct files; "
            f"found {len(entries)}"
        )
    for entry in entries:
        if entry.is_symlink() or not entry.is_file():
            raise ValueError(f"seed corpus entry must be a direct regular file: {entry}")

    destination.mkdir(parents=True)
    manifest = []
    for source_file in entries:
        destination_file = destination / source_file.name
        shutil.copy2(source_file, destination_file)
        manifest.append(
            {
                "name": source_file.name,
                "bytes": destination_file.stat().st_size,
                "sha256": sha256_file(destination_file),
            }
        )
    return manifest


def cargo_fuzz_prefix(toolchain: str) -> list[str]:
    if not toolchain or any(character.isspace() for character in toolchain):
        raise ValueError("toolchain must be a non-empty rustup toolchain name")
    return ["cargo", f"+{toolchain}", "fuzz"]


def fuzz_command(
    toolchain: str,
    corpus: pathlib.Path,
    artifact_directory: pathlib.Path,
    seconds: int,
    target_name: str = TARGET_NAME,
) -> list[str]:
    if target_name not in SUPPORTED_TARGETS:
        raise ValueError(f"unsupported fuzz target: {target_name}")
    artifact_prefix = str(artifact_directory.resolve()) + os.sep
    return [
        *cargo_fuzz_prefix(toolchain),
        "run",
        "--fuzz-dir",
        "fuzz",
        "--sanitizer",
        "address",
        "--jobs",
        "1",
        target_name,
        str(corpus.resolve()),
        "--",
        f"-max_total_time={seconds}",
        f"-max_len={MAX_INPUT_BYTES}",
        f"-timeout={PER_INPUT_TIMEOUT_SECONDS}",
        f"-rss_limit_mb={RSS_LIMIT_MB}",
        f"-artifact_prefix={artifact_prefix}",
        "-print_final_stats=1",
    ]


def minimization_command(
    toolchain: str,
    artifact: pathlib.Path,
    minimized: pathlib.Path,
    target_name: str = TARGET_NAME,
) -> list[str]:
    if target_name not in SUPPORTED_TARGETS:
        raise ValueError(f"unsupported fuzz target: {target_name}")
    return [
        *cargo_fuzz_prefix(toolchain),
        "tmin",
        "--fuzz-dir",
        "fuzz",
        "--sanitizer",
        "address",
        target_name,
        str(artifact.resolve()),
        "--",
        f"-max_len={MAX_INPUT_BYTES}",
        f"-timeout={PER_INPUT_TIMEOUT_SECONDS}",
        f"-exact_artifact_path={minimized.resolve()}",
    ]


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a bounded, source-bound Sorotte local parser target."
    )
    parser.add_argument(
        "--target",
        choices=SUPPORTED_TARGETS,
        default=TARGET_NAME,
    )
    parser.add_argument("--toolchain", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--seconds", type=int, required=True)
    parser.add_argument("--seed-corpus", type=pathlib.Path, required=True)
    parser.add_argument("--expected-seed-count", type=int, required=True)
    parser.add_argument("--output-root", type=pathlib.Path, required=True)
    return parser.parse_args(argv)


def tool_identities(toolchain: str, repository_root: pathlib.Path) -> dict[str, str]:
    cargo_fuzz_version = checked_output(
        ["cargo", "fuzz", "--version"],
        repository_root,
    )
    if cargo_fuzz_version != EXPECTED_CARGO_FUZZ_VERSION:
        raise ValueError(
            "cargo-fuzz version mismatch: "
            f"expected {EXPECTED_CARGO_FUZZ_VERSION!r}, got {cargo_fuzz_version!r}"
        )
    return {
        "cargo_fuzz": cargo_fuzz_version,
        "cargo": checked_output(
            ["cargo", f"+{toolchain}", "--version"],
            repository_root,
        ),
        "rustc": checked_output(
            ["rustc", f"+{toolchain}", "-vV"],
            repository_root,
        ),
        "python": sys.version.replace("\n", " "),
        "platform": platform.platform(),
    }


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    report_schema = (
        REPORT_SCHEMA
        if args.target == TARGET_NAME
        else FRAMED_SESSION_REPORT_SCHEMA
    )
    if SOURCE_SHA_PATTERN.fullmatch(args.source_sha) is None:
        raise ValueError(
            "source SHA must be exactly 40 lowercase hexadecimal characters"
        )
    if not 1 <= args.seconds <= MAX_TOTAL_SECONDS:
        raise ValueError(
            f"seconds must be between 1 and {MAX_TOTAL_SECONDS}, got {args.seconds}"
        )
    if not 1 <= args.expected_seed_count <= 1_000:
        raise ValueError("expected seed count must be between 1 and 1000")

    repository_root = pathlib.Path(__file__).resolve().parents[1]
    target_root = (repository_root / "target").resolve()
    output_root = (
        args.output_root
        if args.output_root.is_absolute()
        else repository_root / args.output_root
    ).resolve()
    require_relative_to(output_root, target_root, "output root")
    if output_root.exists():
        raise ValueError(f"output root already exists; refusing stale evidence: {output_root}")

    seed_source = (
        args.seed_corpus
        if args.seed_corpus.is_absolute()
        else repository_root / args.seed_corpus
    ).resolve()
    require_relative_to(seed_source, repository_root, "seed corpus")

    corpus = output_root / "corpus"
    artifacts = output_root / "artifacts"
    minimized_directory = output_root / "minimized"
    output_root.mkdir(parents=True)
    artifacts.mkdir()
    minimized_directory.mkdir()
    started_at = utc_now()
    report: dict[str, Any] = {
        "schema": report_schema,
        "target": args.target,
        "source_sha": args.source_sha,
        "started_at": started_at,
        "finished_at": None,
        "status": "preparing",
        "toolchain": args.toolchain,
        "tools": None,
        "sanitizer": "address",
        "limits": {
            "max_total_seconds": args.seconds,
            "max_input_bytes": MAX_INPUT_BYTES,
            "per_input_timeout_seconds": PER_INPUT_TIMEOUT_SECONDS,
            "rss_limit_mb": RSS_LIMIT_MB,
        },
        "seed_corpus": {
            "source": str(seed_source.relative_to(repository_root)),
            "expected_files": args.expected_seed_count,
            "files": [],
            "source_after": [],
            "source_stable": None,
        },
        "source_bindings": {
            "before": None,
            "after": None,
            "stable": None,
        },
        "command": None,
        "fuzzer_exit_code": None,
        "statistics": {},
        "final_corpus": None,
        "artifacts": None,
        "minimization": [],
        "evidence_errors": [],
        "setup_error": None,
    }
    report_path = output_root / "run-report.json"
    atomic_write_json(report_path, report)

    try:
        seeds = copy_seed_corpus(seed_source, corpus, args.expected_seed_count)
        report["seed_corpus"]["files"] = seeds
        source_binding_before = bound_source_manifest(repository_root, args.target)
        report["source_bindings"]["before"] = source_binding_before
        tools = tool_identities(args.toolchain, repository_root)
        report["tools"] = tools
        command = fuzz_command(
            args.toolchain,
            corpus,
            artifacts,
            args.seconds,
            args.target,
        )
        report["command"] = command
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        report["finished_at"] = utc_now()
        report["status"] = "setup_failed"
        report["setup_error"] = f"{type(error).__name__}: {error}"
        atomic_write_json(report_path, report)
        return 2

    report["status"] = "running"
    atomic_write_json(report_path, report)

    log_path = output_root / "fuzz.log"
    timed_out = False
    evidence_errors: list[str] = []
    try:
        with log_path.open("w", encoding="utf-8") as log:
            try:
                completed = subprocess.run(
                    command,
                    cwd=repository_root,
                    check=False,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    text=True,
                    timeout=args.seconds + 120,
                )
                exit_code = completed.returncode
            except subprocess.TimeoutExpired:
                timed_out = True
                exit_code = 124
    except OSError as error:
        exit_code = 127
        evidence_errors.append(f"fuzzer launch failed: {type(error).__name__}: {error}")

    try:
        log_text = log_path.read_text(encoding="utf-8", errors="replace")
        statistics = parse_final_statistics(log_text)
        if exit_code == 0:
            validate_final_statistics(statistics)
    except (OSError, ValueError) as error:
        statistics = {}
        evidence_errors.append(
            f"final statistics invalid: {type(error).__name__}: {error}"
        )
    report["statistics"] = statistics

    try:
        final_corpus_entries = direct_file_manifest(corpus)
        report["final_corpus"] = manifest_summary(final_corpus_entries)
    except ValueError as error:
        report["final_corpus"] = None
        evidence_errors.append(f"final corpus invalid: {error}")

    artifact_paths = sorted(
        path for path in artifacts.iterdir() if path.is_file() and not path.is_symlink()
    )
    try:
        report["artifacts"] = manifest_summary(direct_file_manifest(artifacts))
    except ValueError as error:
        report["artifacts"] = None
        evidence_errors.append(f"artifact inventory invalid: {error}")

    if exit_code != 0:
        for artifact in artifact_paths:
            minimized = minimized_directory / f"minimized-{artifact.name}"
            minimize_log = minimized_directory / f"{artifact.name}.log"
            minimize_command = minimization_command(
                args.toolchain,
                artifact,
                minimized,
                args.target,
            )
            try:
                with minimize_log.open("w", encoding="utf-8") as log:
                    try:
                        result = subprocess.run(
                            minimize_command,
                            cwd=repository_root,
                            check=False,
                            stdout=log,
                            stderr=subprocess.STDOUT,
                            text=True,
                            timeout=MINIMIZE_TIMEOUT_SECONDS,
                        )
                        minimize_exit_code = result.returncode
                    except subprocess.TimeoutExpired:
                        minimize_exit_code = 124
            except OSError as error:
                minimize_exit_code = 127
                evidence_errors.append(
                    f"minimization launch failed for {artifact.name}: "
                    f"{type(error).__name__}: {error}"
                )
            report["minimization"].append(
                {
                    "source": artifact.name,
                    "command": minimize_command,
                    "exit_code": minimize_exit_code,
                    "output": (
                        {
                            "name": minimized.name,
                            "bytes": minimized.stat().st_size,
                            "sha256": sha256_file(minimized),
                        }
                        if minimized.is_file()
                        else None
                    ),
                }
            )

    try:
        source_binding_after = bound_source_manifest(repository_root, args.target)
        source_stable = source_binding_before == source_binding_after
    except (OSError, ValueError) as error:
        source_binding_after = None
        source_stable = False
        evidence_errors.append(
            f"post-run source binding failed: {type(error).__name__}: {error}"
        )
    report["source_bindings"]["after"] = source_binding_after
    report["source_bindings"]["stable"] = source_stable

    try:
        seed_source_after = direct_file_manifest(seed_source)
        seed_source_stable = seeds == seed_source_after
    except ValueError as error:
        seed_source_after = []
        seed_source_stable = False
        evidence_errors.append(f"post-run seed source binding failed: {error}")
    report["seed_corpus"]["source_after"] = seed_source_after
    report["seed_corpus"]["source_stable"] = seed_source_stable

    report["finished_at"] = utc_now()
    report["fuzzer_exit_code"] = exit_code
    report["evidence_errors"] = evidence_errors
    report["status"] = classify_status(
        exit_code=exit_code,
        timed_out=timed_out,
        source_stable=source_stable,
        seed_source_stable=seed_source_stable,
        evidence_errors=evidence_errors,
    )
    atomic_write_json(report_path, report)
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Sorotte fuzz runner failed closed: {error}", file=sys.stderr)
        raise SystemExit(2) from error
