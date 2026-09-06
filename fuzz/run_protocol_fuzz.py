from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import io
import json
import os
import pathlib
import platform
import re
import shutil
import subprocess
import sys
import time
from typing import Any, Sequence

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "scripts"))
from fuzz_regressions import validate as validate_corpus_manifest
from mutation_process import ProcessError, run as run_owned_process

TARGET_NAME = "protocol_line"
FRAMED_SESSION_TARGET_NAME = "framed_session"
MPV_FRAMED_TRANSCRIPT_TARGET_NAME = "mpv_framed_transcript"
MAX_TOTAL_SECONDS = 900
MAX_INPUT_BYTES = 65_536
PER_INPUT_TIMEOUT_SECONDS = 5
RSS_LIMIT_MB = 2_048
MINIMIZE_TIMEOUT_SECONDS = 120
REPORT_SCHEMA = "sorotte-protocol-fuzz-v1"
FRAMED_SESSION_REPORT_SCHEMA = "sorotte-framed-session-fuzz-v1"
MPV_FRAMED_TRANSCRIPT_REPORT_SCHEMA = "sorotte-mpv-framed-transcript-fuzz-v1"
EXPECTED_CARGO_FUZZ_VERSION = "cargo-fuzz 0.13.2"
SOURCE_SHA_PATTERN = re.compile(r"^[0-9a-f]{40}$")
SHARED_BOUND_SOURCE_PATHS = (
    ".gitattributes",
    ".github/workflows/rust-fuzz.yml",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "coverage/behaviors.toml",
    "coverage/known-defects.toml",
    "coverage/fuzz-corpora.json",
    "coverage/verification-tools.toml",
    "coverage/verification-lanes.json",
    "coverage/test-inventories.json",
    "scripts/fuzz_regressions.py",
    "scripts/fuzz_tool_canary.py",
    "scripts/verification_tools.py",
    "scripts/verify.py",
    "scripts/test_inventory.py",
    "scripts/mutation_process.py",
    "fuzz/Cargo.toml",
    "fuzz/Cargo.lock",
    "fuzz/run_protocol_fuzz.py",
    "requirements/ci-policy.txt",
    "scripts/known_defect_policy.py",
    "scripts/tests/test_known_defect_policy.py",
    "scripts/tests/test_protocol_fuzz_policy.py",
)
SHARED_BOUND_SOURCE_DIRECTORIES = (".cargo", "crates", "fuzz/fuzz_targets")
BOUND_FIXED_SOURCE_PATHS = SHARED_BOUND_SOURCE_PATHS + (
    "crates/sorotte-protocol/Cargo.toml",
    "fuzz/fuzz_targets/protocol_line.rs",
)
PROTOCOL_SOURCE_DIRECTORY = "crates/sorotte-protocol/src"
FRAMED_SESSION_BOUND_FIXED_SOURCE_PATHS = SHARED_BOUND_SOURCE_PATHS + (
    "fuzz/fuzz_targets/framed_session.rs",
)
FRAMED_SESSION_SOURCE_DIRECTORY = "crates"
MPV_FRAMED_TRANSCRIPT_BOUND_FIXED_SOURCE_PATHS = SHARED_BOUND_SOURCE_PATHS + (
    "fuzz/fuzz_targets/mpv_framed_transcript.rs",
)
MPV_FRAMED_TRANSCRIPT_SOURCE_DIRECTORIES = (
    "crates/sorotte-player-api",
    "crates/sorotte-player-mpv",
)
SUPPORTED_TARGETS = (
    TARGET_NAME,
    FRAMED_SESSION_TARGET_NAME,
    MPV_FRAMED_TRANSCRIPT_TARGET_NAME,
)
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
        source_directories = (PROTOCOL_SOURCE_DIRECTORY,)
        source_label = "protocol"
        source_filter = lambda path: path.suffix == ".rs"
    elif target_name == FRAMED_SESSION_TARGET_NAME:
        fixed_paths = FRAMED_SESSION_BOUND_FIXED_SOURCE_PATHS
        source_directories = (FRAMED_SESSION_SOURCE_DIRECTORY,)
        source_label = "workspace"
        source_filter = lambda path: path.is_file()
    elif target_name == MPV_FRAMED_TRANSCRIPT_TARGET_NAME:
        fixed_paths = MPV_FRAMED_TRANSCRIPT_BOUND_FIXED_SOURCE_PATHS
        source_directories = MPV_FRAMED_TRANSCRIPT_SOURCE_DIRECTORIES
        source_label = "player mpv and player API"
        source_filter = lambda path: path.is_file()
    else:
        raise ValueError(f"unsupported fuzz target: {target_name}")

    # Cargo builds this package's complete dependency graph even when only one
    # fuzz binary is requested. Bind every local crate and Cargo configuration.
    source_directories = SHARED_BOUND_SOURCE_DIRECTORIES
    source_filter = lambda path: path.is_file()
    paths = {repository_root / path for path in fixed_paths}
    for source_directory in source_directories:
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
        paths.update(sources)

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


def require_committed_source(
    repository_root: pathlib.Path, source_sha: str, manifest: dict[str, Any]
) -> None:
    """Compare the actual build inputs with exact Git blobs, including inventory."""
    git = ["git", "-c", f"safe.directory={repository_root.as_posix()}"]
    if checked_output([*git, "rev-parse", "HEAD"], repository_root) != source_sha:
        raise ValueError("fuzz source SHA differs from the checked-out commit")
    roots = sorted(set(SHARED_BOUND_SOURCE_PATHS + SHARED_BOUND_SOURCE_DIRECTORIES))
    tracked = subprocess.run(
        [*git, "ls-tree", "-r", "-z", "--name-only", source_sha, "--", *roots],
        cwd=repository_root, check=True, capture_output=True, timeout=30,
    ).stdout
    names = [entry["path"] for entry in manifest["files"]]
    if set(tracked.decode("utf-8").rstrip("\0").split("\0")) != set(names):
        raise ValueError("fuzz source inventory differs from the committed inputs")
    if any("\n" in name or "\r" in name for name in names):
        raise ValueError("fuzz source path cannot contain a newline")
    payload = "".join(f"{source_sha}:{name}\n" for name in names).encode("utf-8")
    result = subprocess.run(
        [*git, "cat-file", "--batch"], input=payload, cwd=repository_root,
        check=True, capture_output=True, timeout=30,
    )
    stream = io.BytesIO(result.stdout)
    for entry in manifest["files"]:
        header = stream.readline().decode("ascii").strip().split()
        if len(header) != 3 or header[1] != "blob":
            raise ValueError(f"fuzz committed source is not a blob: {entry['path']}")
        body = stream.read(int(header[2]))
        if stream.read(1) != b"\n":
            raise ValueError("malformed committed fuzz source response")
        if len(body) != entry["bytes"] or sha256_bytes(body) != entry["sha256"]:
            raise ValueError(f"fuzz source bytes differ from the commit: {entry['path']}")
    if stream.read():
        raise ValueError("unexpected trailing committed fuzz source response")


def locked_metadata_command(toolchain: str) -> list[str]:
    # cargo-fuzz 0.13.2 has no --locked flag or Cargo-argument passthrough.
    # Cargo metadata performs locked resolution without compiling the target.
    return ["cargo", f"+{toolchain}", "metadata", "--locked", "--manifest-path",
            "fuzz/Cargo.toml", "--format-version", "1"]


def build_command(toolchain: str, target: str) -> list[str]:
    if target not in SUPPORTED_TARGETS:
        raise ValueError(f"unsupported fuzz target: {target}")
    return [*cargo_fuzz_prefix(toolchain), "build", "--fuzz-dir", "fuzz",
            "--sanitizer", "address", target]


def prepare_target(
    repository_root: pathlib.Path, toolchain: str, target: str,
    source_sha: str, source_before: dict[str, Any], output: pathlib.Path,
    report: dict[str, Any], report_path: pathlib.Path,
) -> None:
    """Keep resolution and prebuild inside the same committed-input boundary."""
    require_committed_source(repository_root, source_sha, source_before)
    metadata = locked_metadata_command(toolchain)
    preparation = report["preparation"]
    preparation["metadata"] = {"command": metadata, "status": "running"}
    atomic_write_json(report_path, report)
    started = time.monotonic()
    print("[fuzz preparation] resolving committed dependencies with Cargo --locked", flush=True)
    try:
        with (output / "metadata.json").open("wb") as stdout, \
                (output / "metadata.stderr").open("wb") as stderr:
            result = subprocess.run(metadata, cwd=repository_root, stdout=stdout,
                                    stderr=stderr, timeout=180, check=False)
        preparation["metadata"].update(status="completed", exit_code=result.returncode)
        if result.returncode:
            diagnostic = (output / "metadata.stderr").read_text(encoding="utf-8", errors="replace")
            raise ValueError(f"locked fuzz dependency resolution failed: {diagnostic[-4000:]}")
    except BaseException as error:
        preparation["metadata"].update(
            status="timed_out" if isinstance(error, subprocess.TimeoutExpired) else "failed",
            error=f"{type(error).__name__}: {error}",
        )
        raise
    finally:
        preparation["metadata"]["duration_seconds"] = round(time.monotonic() - started, 3)
        for name in ("metadata.json", "metadata.stderr"):
            if (output / name).is_file():
                preparation["metadata"][name + "_sha256"] = sha256_file(output / name)
        atomic_write_json(report_path, report)
    after_metadata = bound_source_manifest(repository_root, target)
    if after_metadata != source_before:
        raise ValueError("fuzz source changed during locked dependency resolution")
    require_committed_source(repository_root, source_sha, after_metadata)
    command = build_command(toolchain, target)
    preparation["build"] = {"command": command, "status": "running"}
    atomic_write_json(report_path, report)
    try:
        result = run_owned_process(command, cwd=repository_root, timeout_seconds=600,
                                   log_root=output / "build", label="fuzz-prebuild")
        preparation["build"].update(status="completed", exit_code=result.returncode)
        if result.returncode:
            raise ValueError(f"fuzz target build failed with exit {result.returncode}; see build/console.log")
    except BaseException as error:
        preparation["build"].update(status="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        atomic_write_json(report_path, report)
    after_build = bound_source_manifest(repository_root, target)
    report["source_bindings"]["after_build"] = after_build
    if after_build != source_before:
        raise ValueError("fuzz source changed during target build")
    require_committed_source(repository_root, source_sha, after_build)


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
        # libFuzzer's crash minimizer sets its own limit to the input size.
        # Supplying max_len here attempts to initialize that limit twice.
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
    parser.add_argument("--expected-seed-count", type=int, help="legacy explicit count; must match the reviewed manifest")
    parser.add_argument("--corpus-manifest", type=pathlib.Path, default=pathlib.Path("coverage/fuzz-corpora.json"))
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
    report_schema = {
        TARGET_NAME: REPORT_SCHEMA,
        FRAMED_SESSION_TARGET_NAME: FRAMED_SESSION_REPORT_SCHEMA,
        MPV_FRAMED_TRANSCRIPT_TARGET_NAME: MPV_FRAMED_TRANSCRIPT_REPORT_SCHEMA,
    }[args.target]
    if SOURCE_SHA_PATTERN.fullmatch(args.source_sha) is None:
        raise ValueError(
            "source SHA must be exactly 40 lowercase hexadecimal characters"
        )
    if not 1 <= args.seconds <= MAX_TOTAL_SECONDS:
        raise ValueError(
            f"seconds must be between 1 and {MAX_TOTAL_SECONDS}, got {args.seconds}"
        )
    repository_root = pathlib.Path(__file__).resolve().parents[1]
    manifest_path = (repository_root / args.corpus_manifest).resolve()
    if manifest_path != repository_root / "coverage/fuzz-corpora.json":
        raise ValueError("fuzz corpus authority must be the reviewed repository manifest")
    manifest = validate_corpus_manifest(root=repository_root, manifest=manifest_path)
    target_entry = next(item for item in manifest["targets"] if item["id"] == args.target)
    expected_count = len(target_entry["files"])
    if args.expected_seed_count is not None and args.expected_seed_count != expected_count:
        raise ValueError("explicit seed count differs from reviewed corpus identity")
    args.expected_seed_count = expected_count
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
    if seed_source != (repository_root / target_entry["directory"]).resolve():
        raise ValueError("target seed directory differs from reviewed corpus authority")
    actual_files = []
    for seed in sorted(seed_source.iterdir()):
        if seed.is_symlink() or not seed.is_file():
            raise ValueError("corpus input must be a direct regular file")
        actual_files.append({"name": seed.name, "bytes": seed.stat().st_size, "sha256": sha256_file(seed)})
    if actual_files != target_entry["files"]:
        raise ValueError("seed corpus bytes differ from the reviewed manifest")

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
            "after_build": None,
            "after": None,
            "stable": None,
        },
        "preparation": {},
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
        require_committed_source(repository_root, args.source_sha, source_binding_before)
        tools = tool_identities(args.toolchain, repository_root)
        report["tools"] = tools
        prepare_target(repository_root, args.toolchain, args.target, args.source_sha,
                       source_binding_before, output_root, report, report_path)
        command = fuzz_command(
            args.toolchain,
            corpus,
            artifacts,
            args.seconds,
            args.target,
        )
        report["command"] = command
    except (OSError, ValueError, subprocess.SubprocessError, ProcessError) as error:
        try:
            report["source_bindings"]["after"] = bound_source_manifest(repository_root, args.target)
            report["source_bindings"]["stable"] = (
                report["source_bindings"]["before"] == report["source_bindings"]["after"]
            )
        except (OSError, ValueError) as binding_error:
            report["source_bindings"]["stable"] = False
            report["evidence_errors"].append(f"post-preparation source binding failed: {binding_error}")
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
        require_committed_source(repository_root, args.source_sha, source_binding_after)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
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
