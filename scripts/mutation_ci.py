#!/usr/bin/env python3
"""Run and attest a bounded, fail-closed cargo-mutants shard.

The wrapper owns the mutation command instead of accepting arbitrary shell
fragments from policy. It pins the producer, disables repository-local
cargo-mutants configuration, inventories the selected tests and mutations
before execution, binds the run to source and test-suite hashes before and
after execution, and reconciles every outcome with both mutation inventories
and cargo-mutants' status files.

Mutation survivors and timeouts are policy failures, not parser failures.
Malformed, incomplete, stale, or contradictory producer artifacts are rejected
as errors. Reports are written on every wrapper-controlled exit.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import decimal
import hashlib
import json
import math
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import tomllib
from collections.abc import Mapping, Sequence
from typing import Any


SCHEMA_VERSION = 3
REPORT_KIND = "sorotte-mutation-evidence"
DEFAULT_POLICY = "coverage/mutation-policy.toml"
MUTANTS_DIRECTORY = "mutants.out"
MAX_POLICY_BYTES = 256 * 1024
MAX_JSON_BYTES = 32 * 1024 * 1024
MAX_TEXT_BYTES = 64 * 1024 * 1024
MAX_SOURCE_BYTES = 16 * 1024 * 1024
IDENTIFIER = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
PACKAGE = re.compile(r"^[a-z][a-z0-9-]{0,127}$")
VERSION = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
TEST_FILTER = re.compile(
    r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+(?:::)?$"
)
MAX_MUTANT_FILTER_BYTES = 512
PERCENT = re.compile(
    r"^(?:100(?:\.0{1,2})?|(?:[0-9]|[1-9][0-9])(?:\.[0-9]{1,2})?)$"
)
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
KNOWN_SUMMARIES = {
    "Success",
    "CaughtMutant",
    "MissedMutant",
    "Timeout",
    "Unviable",
}
STATUS_FILES = {
    "CaughtMutant": "caught.txt",
    "MissedMutant": "missed.txt",
    "Timeout": "timeout.txt",
    "Unviable": "unviable.txt",
}
TOP_LEVEL_FUNCTION = "<top-level>"
TOP_LEVEL_RETURN_TYPE = "<none>"


class MutationCiError(ValueError):
    """An invalid policy, unsafe input, or malformed producer artifact."""


@dataclasses.dataclass(frozen=True)
class AcceptedUnviable:
    identifier: str
    shard: str
    file: str
    function: str
    return_type: str
    genre: str
    replacement: str
    reason: str
    review_by: dt.date
    expected_count: int = 1

    def identity(self) -> tuple[str, str, str, str, str]:
        return (
            self.file,
            self.function,
            self.return_type,
            self.genre,
            self.replacement,
        )


@dataclasses.dataclass(frozen=True)
class ShardPolicy:
    identifier: str
    owner: str
    package: str
    files: tuple[str, ...]
    mutant_filter: str
    test_target: str
    test_filter: str
    jobs: int
    timeout_seconds: int
    build_timeout_seconds: int
    minimum_viable_kill_percent: decimal.Decimal
    max_missed: int
    max_timeouts: int
    require_baseline: bool


@dataclasses.dataclass(frozen=True)
class MutationPolicy:
    cargo_mutants_version: str
    shards: tuple[ShardPolicy, ...]
    accepted_unviable: tuple[AcceptedUnviable, ...]

    def shard(self, identifier: str) -> ShardPolicy:
        matches = [shard for shard in self.shards if shard.identifier == identifier]
        if len(matches) != 1:
            raise MutationCiError(
                f"mutation shard {identifier!r} must exist exactly once"
            )
        return matches[0]

    def accepted_for(self, identifier: str) -> tuple[AcceptedUnviable, ...]:
        return tuple(
            entry for entry in self.accepted_unviable if entry.shard == identifier
        )


def require_exact_keys(
    value: Mapping[str, Any],
    expected: set[str],
    *,
    label: str,
) -> None:
    actual = set(value)
    if actual != expected:
        unknown = sorted(actual - expected)
        missing = sorted(expected - actual)
        raise MutationCiError(
            f"{label} fields do not match schema; "
            f"unknown={unknown}, missing={missing}"
        )


def require_mapping(value: Any, *, label: str) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise MutationCiError(f"{label} must be an object")
    return value


def require_list(value: Any, *, label: str) -> list[Any]:
    if not isinstance(value, list):
        raise MutationCiError(f"{label} must be an array")
    return value


def require_string(value: Any, *, label: str, allow_empty: bool = False) -> str:
    if not isinstance(value, str) or (not allow_empty and not value):
        qualifier = "a string" if allow_empty else "a non-empty string"
        raise MutationCiError(f"{label} must be {qualifier}")
    if "\x00" in value:
        raise MutationCiError(f"{label} must not contain NUL")
    return value


def require_int(
    value: Any,
    *,
    label: str,
    minimum: int = 0,
    maximum: int | None = None,
) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise MutationCiError(f"{label} must be an integer")
    if value < minimum or (maximum is not None and value > maximum):
        bounds = f">= {minimum}"
        if maximum is not None:
            bounds += f" and <= {maximum}"
        raise MutationCiError(f"{label} must be {bounds}")
    return value


def require_bool(value: Any, *, label: str) -> bool:
    if not isinstance(value, bool):
        raise MutationCiError(f"{label} must be a boolean")
    return value


def require_identifier(value: Any, *, label: str) -> str:
    text = require_string(value, label=label)
    if not IDENTIFIER.fullmatch(text):
        raise MutationCiError(f"{label} is not a safe identifier")
    return text


def require_percent(value: Any, *, label: str) -> decimal.Decimal:
    text = require_string(value, label=label)
    if not PERCENT.fullmatch(text):
        raise MutationCiError(f"{label} must be a percentage string from 0 to 100")
    return decimal.Decimal(text)


def require_timestamp(value: Any, *, label: str) -> dt.datetime:
    text = require_string(value, label=label)
    try:
        parsed = dt.datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError as error:
        raise MutationCiError(f"{label} must be an ISO-8601 timestamp") from error
    if parsed.tzinfo is None or parsed.utcoffset() != dt.timedelta(0):
        raise MutationCiError(f"{label} must identify an instant in UTC")
    return parsed


def bounded_bytes(path: pathlib.Path, *, maximum: int, label: str) -> bytes:
    try:
        size = path.stat().st_size
    except OSError as error:
        raise MutationCiError(f"cannot stat {label} {path}: {error}") from error
    if size > maximum:
        raise MutationCiError(
            f"{label} exceeds the {maximum}-byte safety limit: {size}"
        )
    try:
        return path.read_bytes()
    except OSError as error:
        raise MutationCiError(f"cannot read {label} {path}: {error}") from error


def reject_duplicate_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise MutationCiError(f"JSON object contains duplicate key {key!r}")
        value[key] = item
    return value


def parse_json_bytes(data: bytes, *, label: str) -> Any:
    try:
        text = data.decode("utf-8", errors="strict")
        return json.loads(text, object_pairs_hook=reject_duplicate_pairs)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise MutationCiError(f"{label} is not strict UTF-8 JSON: {error}") from error


def load_json(path: pathlib.Path, *, label: str) -> tuple[Any, bytes]:
    data = bounded_bytes(path, maximum=MAX_JSON_BYTES, label=label)
    return parse_json_bytes(data, label=label), data


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical_digest(value: Any) -> str:
    data = json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return sha256(data)


def atomic_write_json(path: pathlib.Path, value: Mapping[str, Any] | list[Any]) -> None:
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
        raise MutationCiError(f"cannot write JSON {path}: {error}") from error
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


def atomic_write_text(path: pathlib.Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
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
            handle.write(value)
            temporary = pathlib.Path(handle.name)
        os.replace(temporary, path)
    except OSError as error:
        raise MutationCiError(f"cannot write text {path}: {error}") from error
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


def safe_repo_path(repo_root: pathlib.Path, value: Any, *, label: str) -> str:
    text = require_string(value, label=label)
    pure = pathlib.PurePosixPath(text)
    if (
        "\\" in text
        or pure.is_absolute()
        or not pure.parts
        or any(part in {"", ".", ".."} for part in pure.parts)
        or any(character in text for character in "*?[]")
    ):
        raise MutationCiError(f"{label} must be a normalized literal relative path")
    candidate = repo_root.joinpath(*pure.parts)
    try:
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(repo_root)
    except (OSError, ValueError) as error:
        raise MutationCiError(f"{label} escapes or is missing from the repository") from error
    if not resolved.is_file() or resolved.suffix != ".rs":
        raise MutationCiError(f"{label} must identify an existing Rust source file")
    return pure.as_posix()


def nearest_package_name(repo_root: pathlib.Path, source: str) -> str:
    candidate = repo_root.joinpath(*pathlib.PurePosixPath(source).parts).parent
    while candidate != repo_root:
        manifest = candidate / "Cargo.toml"
        if manifest.is_file():
            data = bounded_bytes(
                manifest,
                maximum=MAX_POLICY_BYTES,
                label="package manifest",
            )
            try:
                parsed = tomllib.loads(data.decode("utf-8", errors="strict"))
            except (UnicodeError, tomllib.TOMLDecodeError) as error:
                raise MutationCiError(
                    f"cannot parse package manifest {manifest}: {error}"
                ) from error
            package = parsed.get("package")
            if not isinstance(package, Mapping):
                raise MutationCiError(
                    f"nearest manifest for {source} has no package table"
                )
            return require_string(package.get("name"), label=f"{source} package name")
        candidate = candidate.parent
    raise MutationCiError(f"no package manifest found for {source}")


def load_policy(
    repo_root: pathlib.Path,
    policy_path: pathlib.Path,
    *,
    today: dt.date | None = None,
) -> MutationPolicy:
    data = bounded_bytes(policy_path, maximum=MAX_POLICY_BYTES, label="mutation policy")
    try:
        parsed = tomllib.loads(data.decode("utf-8", errors="strict"))
    except (UnicodeError, tomllib.TOMLDecodeError) as error:
        raise MutationCiError(f"cannot parse mutation policy: {error}") from error
    root = require_mapping(parsed, label="mutation policy")
    require_exact_keys(
        root,
        {
            "schema_version",
            "cargo_mutants_version",
            "shard",
            "accepted_unviable",
        },
        label="mutation policy",
    )
    if require_int(root["schema_version"], label="schema_version") != SCHEMA_VERSION:
        raise MutationCiError(
            f"unsupported mutation policy schema {root['schema_version']!r}"
        )
    version = require_string(
        root["cargo_mutants_version"],
        label="cargo_mutants_version",
    )
    if not VERSION.fullmatch(version):
        raise MutationCiError("cargo_mutants_version must be an exact semver triplet")

    shard_values = require_list(root["shard"], label="shard")
    if not shard_values:
        raise MutationCiError("mutation policy must declare at least one shard")
    shards: list[ShardPolicy] = []
    shard_ids: set[str] = set()
    globally_owned_files: set[str] = set()
    for index, item in enumerate(shard_values):
        table = require_mapping(item, label=f"shard[{index}]")
        require_exact_keys(
            table,
            {
                "id",
                "owner",
                "package",
                "files",
                "mutant_filter",
                "test_target",
                "test_filter",
                "jobs",
                "timeout_seconds",
                "build_timeout_seconds",
                "minimum_viable_kill_percent",
                "max_missed",
                "max_timeouts",
                "require_baseline",
            },
            label=f"shard[{index}]",
        )
        identifier = require_identifier(table["id"], label=f"shard[{index}].id")
        if identifier in shard_ids:
            raise MutationCiError(f"duplicate mutation shard id {identifier!r}")
        shard_ids.add(identifier)
        owner = require_identifier(table["owner"], label=f"{identifier}.owner")
        package = require_string(table["package"], label=f"{identifier}.package")
        if not PACKAGE.fullmatch(package):
            raise MutationCiError(f"{identifier}.package is not a safe package name")
        raw_files = require_list(table["files"], label=f"{identifier}.files")
        if not raw_files:
            raise MutationCiError(f"{identifier}.files must not be empty")
        files = tuple(
            safe_repo_path(repo_root, value, label=f"{identifier}.files[{offset}]")
            for offset, value in enumerate(raw_files)
        )
        if len(set(files)) != len(files):
            raise MutationCiError(f"{identifier}.files contains duplicates")
        overlaps = globally_owned_files.intersection(files)
        if overlaps:
            raise MutationCiError(
                f"mutation source files belong to more than one shard: {sorted(overlaps)}"
            )
        globally_owned_files.update(files)
        for source in files:
            actual_package = nearest_package_name(repo_root, source)
            if actual_package != package:
                raise MutationCiError(
                    f"{source} belongs to package {actual_package!r}, not {package!r}"
                )
        mutant_filter = require_string(
            table["mutant_filter"],
            label=f"{identifier}.mutant_filter",
            allow_empty=True,
        )
        if len(mutant_filter.encode("utf-8")) > MAX_MUTANT_FILTER_BYTES:
            raise MutationCiError(
                f"{identifier}.mutant_filter exceeds {MAX_MUTANT_FILTER_BYTES} bytes"
            )
        if "\r" in mutant_filter or "\n" in mutant_filter:
            raise MutationCiError(
                f"{identifier}.mutant_filter must remain on one line"
            )
        try:
            re.compile(mutant_filter)
        except re.error as error:
            raise MutationCiError(
                f"{identifier}.mutant_filter is not a valid regular expression: {error}"
            ) from error
        test_target = require_string(
            table["test_target"],
            label=f"{identifier}.test_target",
        )
        if test_target not in {"package", "lib"}:
            raise MutationCiError(
                f"{identifier}.test_target must be either 'package' or 'lib'"
            )
        test_filter = require_string(
            table["test_filter"],
            label=f"{identifier}.test_filter",
            allow_empty=True,
        )
        if test_filter and test_target != "lib":
            raise MutationCiError(
                f"{identifier}.test_filter requires test_target = 'lib'"
            )
        if test_filter and not TEST_FILTER.fullmatch(test_filter):
            raise MutationCiError(
                f"{identifier}.test_filter must be a Rust test selector prefix"
            )
        require_baseline = require_bool(
            table["require_baseline"],
            label=f"{identifier}.require_baseline",
        )
        if not require_baseline:
            raise MutationCiError(
                f"{identifier}.require_baseline cannot disable the baseline"
            )
        shards.append(
            ShardPolicy(
                identifier=identifier,
                owner=owner,
                package=package,
                files=files,
                mutant_filter=mutant_filter,
                test_target=test_target,
                test_filter=test_filter,
                jobs=require_int(
                    table["jobs"],
                    label=f"{identifier}.jobs",
                    minimum=1,
                    maximum=16,
                ),
                timeout_seconds=require_int(
                    table["timeout_seconds"],
                    label=f"{identifier}.timeout_seconds",
                    minimum=1,
                    maximum=3600,
                ),
                build_timeout_seconds=require_int(
                    table["build_timeout_seconds"],
                    label=f"{identifier}.build_timeout_seconds",
                    minimum=1,
                    maximum=3600,
                ),
                minimum_viable_kill_percent=require_percent(
                    table["minimum_viable_kill_percent"],
                    label=f"{identifier}.minimum_viable_kill_percent",
                ),
                max_missed=require_int(
                    table["max_missed"],
                    label=f"{identifier}.max_missed",
                    maximum=100_000,
                ),
                max_timeouts=require_int(
                    table["max_timeouts"],
                    label=f"{identifier}.max_timeouts",
                    maximum=100_000,
                ),
                require_baseline=require_baseline,
            )
        )

    accepted_values = require_list(
        root["accepted_unviable"],
        label="accepted_unviable",
    )
    accepted: list[AcceptedUnviable] = []
    accepted_ids: set[str] = set()
    accepted_identities: set[tuple[str, str, str, str, str]] = set()
    effective_today = today or dt.date.today()
    for index, item in enumerate(accepted_values):
        table = require_mapping(item, label=f"accepted_unviable[{index}]")
        required_keys = {
            "id",
            "shard",
            "file",
            "function",
            "return_type",
            "genre",
            "replacement",
            "reason",
            "review_by",
        }
        actual_keys = set(table)
        unknown_keys = actual_keys - required_keys - {"expected_count"}
        missing_keys = required_keys - actual_keys
        if unknown_keys or missing_keys:
            raise MutationCiError(
                f"accepted_unviable[{index}] fields do not match schema: unexpected keys "
                f"{sorted(unknown_keys)!r} and missing keys {sorted(missing_keys)!r}"
            )
        identifier = require_identifier(
            table["id"],
            label=f"accepted_unviable[{index}].id",
        )
        if identifier in accepted_ids:
            raise MutationCiError(f"duplicate accepted-unviable id {identifier!r}")
        accepted_ids.add(identifier)
        shard_id = require_identifier(
            table["shard"],
            label=f"{identifier}.shard",
        )
        if shard_id not in shard_ids:
            raise MutationCiError(
                f"{identifier} refers to unknown mutation shard {shard_id!r}"
            )
        source = safe_repo_path(repo_root, table["file"], label=f"{identifier}.file")
        shard = next(value for value in shards if value.identifier == shard_id)
        if source not in shard.files:
            raise MutationCiError(
                f"{identifier}.file is not owned by shard {shard_id!r}"
            )
        reason = require_string(table["reason"], label=f"{identifier}.reason")
        if len(reason) < 40:
            raise MutationCiError(f"{identifier}.reason must explain the exception")
        review_text = require_string(
            table["review_by"],
            label=f"{identifier}.review_by",
        )
        try:
            review_by = dt.date.fromisoformat(review_text)
        except ValueError as error:
            raise MutationCiError(
                f"{identifier}.review_by must be an ISO calendar date"
            ) from error
        if review_by < effective_today:
            raise MutationCiError(
                f"{identifier} accepted-unviable review expired on {review_by}"
            )
        entry = AcceptedUnviable(
            identifier=identifier,
            shard=shard_id,
            file=source,
            function=require_string(
                table["function"],
                label=f"{identifier}.function",
            ),
            return_type=require_string(
                table["return_type"],
                label=f"{identifier}.return_type",
                allow_empty=True,
            ),
            genre=require_string(table["genre"], label=f"{identifier}.genre"),
            replacement=require_string(
                table["replacement"],
                label=f"{identifier}.replacement",
                allow_empty=True,
            ),
            expected_count=require_int(
                table.get("expected_count", 1),
                label=f"{identifier}.expected_count",
                minimum=1,
                maximum=100_000,
            ),
            reason=reason,
            review_by=review_by,
        )
        if entry.identity() in accepted_identities:
            raise MutationCiError(
                f"duplicate accepted-unviable identity for {identifier!r}"
            )
        accepted_identities.add(entry.identity())
        accepted.append(entry)
    return MutationPolicy(
        cargo_mutants_version=version,
        shards=tuple(shards),
        accepted_unviable=tuple(accepted),
    )


def source_bindings(
    repo_root: pathlib.Path,
    files: Sequence[str],
) -> list[dict[str, Any]]:
    bindings: list[dict[str, Any]] = []
    for source in files:
        path = repo_root.joinpath(*pathlib.PurePosixPath(source).parts)
        data = bounded_bytes(path, maximum=MAX_SOURCE_BYTES, label=f"source {source}")
        bindings.append(
            {
                "path": source,
                "bytes": len(data),
                "sha256": sha256(data),
            }
        )
    return bindings


def verification_input_files(
    repo_root: pathlib.Path,
    policy_path: pathlib.Path,
) -> list[str]:
    """Return every repository input that can change a mutation test result.

    Mutation shards are deliberately small, but their selected tests compile
    against the whole workspace. Binding only the mutated production file lets
    a changed test, dependency, fixture, toolchain, or wrapper reuse stale
    evidence. The digest therefore covers workspace sources and fixtures plus
    the Cargo/toolchain/policy inputs that define the run.
    """

    candidates: set[pathlib.Path] = set()
    for relative in (
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".cargo/config.toml",
        "scripts/mutation_ci.py",
        "coverage/mutation-report-set.json",
    ):
        candidate = repo_root / relative
        if candidate.is_file() and not candidate.is_symlink():
            candidates.add(candidate)
    candidates.add(policy_path)
    for relative_root in ("crates", "fixtures"):
        root = repo_root / relative_root
        if not root.is_dir():
            continue
        for candidate in root.rglob("*"):
            if candidate.is_file() and not candidate.is_symlink():
                candidates.add(candidate)

    files = sorted(
        candidate.relative_to(repo_root).as_posix()
        for candidate in candidates
    )
    if not files:
        raise MutationCiError("mutation verification input set must not be empty")
    return files


def verification_input_bindings(
    repo_root: pathlib.Path,
    policy_path: pathlib.Path,
) -> list[dict[str, Any]]:
    return source_bindings(repo_root, verification_input_files(repo_root, policy_path))


def test_inventory_binding(selected_tests: Sequence[str]) -> dict[str, Any]:
    tests = list(selected_tests)
    return {
        "count": len(tests),
        "canonical_sha256": canonical_digest(tests),
        "tests": tests,
    }


def git_binding(repo_root: pathlib.Path, files: Sequence[str]) -> dict[str, Any]:
    def run(*argv: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(repo_root), *argv],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="strict",
        )

    try:
        head_process = run("rev-parse", "--verify", "HEAD^{commit}")
        status_process = run(
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            *files,
        )
    except (OSError, UnicodeError) as error:
        raise MutationCiError(f"cannot bind mutation run to Git checkout: {error}") from error
    if head_process.returncode != 0:
        detail = head_process.stderr.strip() or head_process.stdout.strip()
        raise MutationCiError(f"cannot resolve mutation checkout HEAD: {detail}")
    head_lines = head_process.stdout.splitlines()
    if len(head_lines) != 1 or not FULL_SHA.fullmatch(head_lines[0]):
        raise MutationCiError("mutation checkout HEAD did not resolve exactly once")
    if status_process.returncode != 0:
        detail = status_process.stderr.strip() or status_process.stdout.strip()
        raise MutationCiError(f"cannot inspect mutation source status: {detail}")
    return {
        "head_sha": head_lines[0],
        "configured_sources_dirty": bool(status_process.stdout),
    }


def require_point(value: Any, *, label: str) -> dict[str, int]:
    point = require_mapping(value, label=label)
    require_exact_keys(point, {"line", "column"}, label=label)
    return {
        "line": require_int(point["line"], label=f"{label}.line", minimum=1),
        "column": require_int(
            point["column"],
            label=f"{label}.column",
            minimum=1,
        ),
    }


def require_span(value: Any, *, label: str) -> dict[str, dict[str, int]]:
    span = require_mapping(value, label=label)
    require_exact_keys(span, {"start", "end"}, label=label)
    start = require_point(span["start"], label=f"{label}.start")
    end = require_point(span["end"], label=f"{label}.end")
    if (end["line"], end["column"]) < (start["line"], start["column"]):
        raise MutationCiError(f"{label} ends before it starts")
    return {"start": start, "end": end}


def require_function(value: Any, *, label: str) -> dict[str, Any]:
    function = require_mapping(value, label=label)
    require_exact_keys(
        function,
        {"function_name", "return_type", "span"},
        label=label,
    )
    return {
        "function_name": require_string(
            function["function_name"],
            label=f"{label}.function_name",
        ),
        "return_type": require_string(
            function["return_type"],
            label=f"{label}.return_type",
            allow_empty=True,
        ),
        "span": require_span(function["span"], label=f"{label}.span"),
    }


def require_mutant(
    value: Any,
    *,
    label: str,
    shard: ShardPolicy,
    includes_diff: bool,
) -> dict[str, Any]:
    mutant = require_mapping(value, label=label)
    expected = {
        "file",
        "function",
        "genre",
        "name",
        "package",
        "replacement",
        "span",
    }
    if includes_diff:
        expected.add("diff")
    require_exact_keys(mutant, expected, label=label)
    source = require_string(mutant["file"], label=f"{label}.file")
    if source not in shard.files:
        raise MutationCiError(f"{label}.file is outside shard {shard.identifier!r}")
    package = require_string(mutant["package"], label=f"{label}.package")
    if package != shard.package:
        raise MutationCiError(
            f"{label}.package {package!r} does not match {shard.package!r}"
        )
    span = require_span(mutant["span"], label=f"{label}.span")
    name = require_string(mutant["name"], label=f"{label}.name")
    prefix = f"{source}:{span['start']['line']}:{span['start']['column']}: "
    if not name.startswith(prefix):
        raise MutationCiError(f"{label}.name is not bound to its source span")
    if shard.mutant_filter and re.search(shard.mutant_filter, name) is None:
        raise MutationCiError(
            f"{label}.name is outside shard mutant_filter {shard.mutant_filter!r}"
        )
    function = (
        None
        if mutant["function"] is None
        else require_function(
            mutant["function"],
            label=f"{label}.function",
        )
    )
    normalized: dict[str, Any] = {
        "file": source,
        "function": function,
        "genre": require_string(mutant["genre"], label=f"{label}.genre"),
        "name": name,
        "package": package,
        "replacement": require_string(
            mutant["replacement"],
            label=f"{label}.replacement",
            allow_empty=True,
        ),
        "span": span,
    }
    if includes_diff:
        diff = require_string(mutant["diff"], label=f"{label}.diff")
        if not diff.startswith(f"--- {source}\n"):
            raise MutationCiError(f"{label}.diff is not bound to its source file")
        normalized["diff"] = diff
    return normalized


def parse_inventory(value: Any, *, shard: ShardPolicy, label: str) -> list[dict[str, Any]]:
    raw = require_list(value, label=label)
    if not raw:
        raise MutationCiError(f"{label} must contain at least one mutant")
    inventory = [
        require_mutant(
            item,
            label=f"{label}[{index}]",
            shard=shard,
            includes_diff=True,
        )
        for index, item in enumerate(raw)
    ]
    names = [mutant["name"] for mutant in inventory]
    if len(set(names)) != len(names):
        raise MutationCiError(f"{label} contains duplicate mutant names")
    return inventory


def parse_test_inventory(stdout: str, *, shard: ShardPolicy) -> list[str]:
    if len(stdout.encode("utf-8")) > MAX_TEXT_BYTES:
        raise MutationCiError("cargo test inventory output is too large")
    tests: list[str] = []
    for line_number, line in enumerate(stdout.splitlines(), start=1):
        if not line.endswith(": test"):
            raise MutationCiError(
                f"cargo test inventory line {line_number} is not a test selector"
            )
        selector = line.removesuffix(": test")
        if not selector:
            raise MutationCiError(
                f"cargo test inventory line {line_number} has an empty selector"
            )
        if shard.test_filter and not selector.startswith(shard.test_filter):
            raise MutationCiError(
                "cargo test inventory escaped the configured test selector prefix: "
                f"{selector!r}"
            )
        tests.append(selector)
    if not tests:
        raise MutationCiError("cargo test inventory must contain at least one test")
    return tests


def without_diff(mutant: Mapping[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in mutant.items() if key != "diff"}


def mutant_identity(mutant: Mapping[str, Any]) -> tuple[str, str, str, str, str]:
    function_value = mutant["function"]
    if function_value is None:
        function_name = TOP_LEVEL_FUNCTION
        return_type = TOP_LEVEL_RETURN_TYPE
    else:
        function = require_mapping(function_value, label="mutant.function")
        function_name = str(function["function_name"])
        return_type = str(function["return_type"])
    return (
        str(mutant["file"]),
        function_name,
        return_type,
        str(mutant["genre"]),
        str(mutant["replacement"]),
    )


def safe_artifact_path(
    results_dir: pathlib.Path,
    value: Any,
    *,
    label: str,
) -> tuple[str, pathlib.Path]:
    text = require_string(value, label=label)
    pure = pathlib.PurePosixPath(text)
    if (
        "\\" in text
        or pure.is_absolute()
        or not pure.parts
        or any(part in {"", ".", ".."} for part in pure.parts)
    ):
        raise MutationCiError(f"{label} must be a safe relative artifact path")
    path = results_dir.joinpath(*pure.parts)
    try:
        resolved = path.resolve(strict=True)
        resolved.relative_to(results_dir)
    except (OSError, ValueError) as error:
        raise MutationCiError(f"{label} escapes or is missing from results") from error
    if not resolved.is_file():
        raise MutationCiError(f"{label} must identify a result file")
    return pure.as_posix(), resolved


def process_status(value: Any, *, label: str) -> str:
    if isinstance(value, str) and value in {"Success", "Timeout"}:
        return value
    mapping = require_mapping(value, label=label)
    require_exact_keys(mapping, {"Failure"}, label=label)
    code = require_int(
        mapping["Failure"],
        label=f"{label}.Failure",
        minimum=1,
        maximum=255,
    )
    return f"Failure({code})"


def require_phase_results(
    value: Any,
    *,
    label: str,
    shard: ShardPolicy,
) -> list[dict[str, Any]]:
    raw = require_list(value, label=label)
    if not raw:
        raise MutationCiError(f"{label} must not be empty")
    phases: list[dict[str, Any]] = []
    for index, item in enumerate(raw):
        phase = require_mapping(item, label=f"{label}[{index}]")
        require_exact_keys(
            phase,
            {"phase", "duration", "process_status", "argv"},
            label=f"{label}[{index}]",
        )
        phase_name = require_string(
            phase["phase"],
            label=f"{label}[{index}].phase",
        )
        if phase_name not in {"Build", "Test"}:
            raise MutationCiError(f"{label}[{index}].phase is unknown")
        duration = phase["duration"]
        if (
            isinstance(duration, bool)
            or not isinstance(duration, (int, float))
            or not math.isfinite(float(duration))
            or duration < 0
        ):
            raise MutationCiError(
                f"{label}[{index}].duration must be finite and non-negative"
            )
        argv = require_list(phase["argv"], label=f"{label}[{index}].argv")
        if len(argv) < 3 or not all(isinstance(argument, str) for argument in argv):
            raise MutationCiError(f"{label}[{index}].argv is malformed")
        cargo_name = pathlib.PurePath(argv[0]).name.casefold()
        if cargo_name not in {"cargo", "cargo.exe"} or argv[1] != "test":
            raise MutationCiError(f"{label}[{index}].argv is not cargo test")
        package_arguments = [
            argument
            for argument in argv[2:]
            if argument.startswith("--package=")
        ]
        if (
            len(package_arguments) != 1
            or not package_arguments[0].startswith(f"--package={shard.package}@")
        ):
            raise MutationCiError(
                f"{label}[{index}].argv is not bound to package {shard.package!r}"
            )
        if "--locked" not in argv:
            raise MutationCiError(f"{label}[{index}].argv does not enforce Cargo.lock")
        if "--all-features" not in argv:
            raise MutationCiError(
                f"{label}[{index}].argv does not exercise all package features"
            )
        if phase_name == "Build" and "--no-run" not in argv:
            raise MutationCiError(f"{label}[{index}].argv is not a build phase")
        if phase_name == "Test" and "--no-run" in argv:
            raise MutationCiError(f"{label}[{index}].argv is not a test phase")
        expected_arguments = [
            "test",
            "--verbose",
            "--all-features",
            "--locked",
        ]
        if phase_name == "Build":
            expected_arguments.append("--no-run")
        else:
            expected_arguments.extend(cargo_test_scope_arguments(shard))
        actual_arguments = list(argv[1:])
        actual_arguments.remove(package_arguments[0])
        if sorted(actual_arguments) != sorted(expected_arguments):
            raise MutationCiError(
                f"{label}[{index}].argv does not exactly match the configured "
                f"{shard.test_target!r} test scope"
            )
        phases.append(
            {
                "phase": phase_name,
                "duration": float(duration),
                "status": process_status(
                    phase["process_status"],
                    label=f"{label}[{index}].process_status",
                ),
                "argv": list(argv),
            }
        )
    names = [phase["phase"] for phase in phases]
    if names not in (["Build"], ["Build", "Test"]):
        raise MutationCiError(f"{label} has an impossible phase sequence {names}")
    return phases


def require_phase_coherence(
    summary: str,
    phases: Sequence[Mapping[str, Any]],
    *,
    label: str,
) -> None:
    statuses = [str(phase["status"]) for phase in phases]
    if summary == "Success" and statuses != ["Success", "Success"]:
        raise MutationCiError(f"{label} successful baseline has failing phases")
    if summary == "CaughtMutant" and (
        len(statuses) != 2
        or statuses[0] != "Success"
        or not statuses[1].startswith("Failure(")
    ):
        raise MutationCiError(f"{label} caught mutant has incoherent phases")
    if summary == "MissedMutant" and statuses != ["Success", "Success"]:
        raise MutationCiError(f"{label} missed mutant has incoherent phases")
    if summary == "Unviable" and (
        len(statuses) != 1 or not statuses[0].startswith("Failure(")
    ):
        raise MutationCiError(f"{label} unviable mutant has incoherent phases")
    if summary == "Timeout" and (
        len(statuses) != 2
        or statuses[0] != "Success"
        or statuses[1] != "Timeout"
    ):
        raise MutationCiError(f"{label} timed-out mutant has incoherent phases")


def artifact_metadata(path: pathlib.Path, *, label: str) -> dict[str, Any]:
    data = bounded_bytes(path, maximum=MAX_TEXT_BYTES, label=label)
    return {"bytes": len(data), "sha256": sha256(data)}


def parse_status_file(path: pathlib.Path, *, label: str) -> list[str]:
    data = bounded_bytes(path, maximum=MAX_TEXT_BYTES, label=label)
    try:
        text = data.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise MutationCiError(f"{label} is not UTF-8: {error}") from error
    lines = text.splitlines()
    if any(not line for line in lines):
        raise MutationCiError(f"{label} contains a blank status entry")
    if len(set(lines)) != len(lines):
        raise MutationCiError(f"{label} contains duplicate status entries")
    return lines


def parse_lock(
    results_dir: pathlib.Path,
    *,
    expected_version: str,
) -> tuple[dict[str, Any], bytes]:
    value, data = load_json(results_dir / "lock.json", label="cargo-mutants lock")
    lock = require_mapping(value, label="cargo-mutants lock")
    require_exact_keys(
        lock,
        {"cargo_mutants_version", "start_time", "hostname", "username"},
        label="cargo-mutants lock",
    )
    version = require_string(
        lock["cargo_mutants_version"],
        label="lock.cargo_mutants_version",
    )
    if version != expected_version:
        raise MutationCiError(
            f"lock producer version {version!r} does not match {expected_version!r}"
        )
    require_timestamp(lock["start_time"], label="lock.start_time")
    require_string(lock["hostname"], label="lock.hostname", allow_empty=True)
    require_string(lock["username"], label="lock.username", allow_empty=True)
    return {"cargo_mutants_version": version}, data


def evaluate_results(
    *,
    results_dir: pathlib.Path,
    shard: ShardPolicy,
    accepted: Sequence[AcceptedUnviable],
    expected_version: str,
    producer_exit_code: int,
    pre_inventory: Sequence[Mapping[str, Any]],
    source_before: Sequence[Mapping[str, Any]],
    source_after: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    if list(source_before) != list(source_after):
        raise MutationCiError("configured mutation sources changed during the run")
    inventory_value, inventory_bytes = load_json(
        results_dir / "mutants.json",
        label="cargo-mutants inventory",
    )
    inventory = parse_inventory(
        inventory_value,
        shard=shard,
        label="cargo-mutants inventory",
    )
    if list(pre_inventory) != inventory:
        raise MutationCiError(
            "pre-run mutation inventory does not exactly match result inventory"
        )
    inventory_by_name = {mutant["name"]: mutant for mutant in inventory}

    root_value, outcomes_bytes = load_json(
        results_dir / "outcomes.json",
        label="cargo-mutants outcomes",
    )
    root = require_mapping(root_value, label="cargo-mutants outcomes")
    require_exact_keys(
        root,
        {
            "outcomes",
            "total_mutants",
            "missed",
            "caught",
            "timeout",
            "unviable",
            "success",
            "start_time",
            "end_time",
            "cargo_mutants_version",
        },
        label="cargo-mutants outcomes",
    )
    root_version = require_string(
        root["cargo_mutants_version"],
        label="outcomes.cargo_mutants_version",
    )
    if root_version != expected_version:
        raise MutationCiError(
            f"outcome producer version {root_version!r} does not match "
            f"{expected_version!r}"
        )
    start_time = require_timestamp(root["start_time"], label="outcomes.start_time")
    end_time = require_timestamp(root["end_time"], label="outcomes.end_time")
    if end_time < start_time:
        raise MutationCiError("outcomes.end_time precedes outcomes.start_time")
    if require_int(root["success"], label="outcomes.success") != 0:
        raise MutationCiError("outcomes.success must remain zero for mutation shards")

    raw_outcomes = require_list(root["outcomes"], label="outcomes.outcomes")
    artifact_paths: set[str] = set()
    artifact_details: dict[str, dict[str, Any]] = {}
    mutant_outcomes: dict[str, str] = {}
    baseline_count = 0
    for index, item in enumerate(raw_outcomes):
        outcome = require_mapping(item, label=f"outcomes[{index}]")
        require_exact_keys(
            outcome,
            {"scenario", "summary", "log_path", "diff_path", "phase_results"},
            label=f"outcomes[{index}]",
        )
        summary = require_string(
            outcome["summary"],
            label=f"outcomes[{index}].summary",
        )
        if summary not in KNOWN_SUMMARIES:
            raise MutationCiError(f"outcomes[{index}].summary is unknown")
        phases = require_phase_results(
            outcome["phase_results"],
            label=f"outcomes[{index}].phase_results",
            shard=shard,
        )
        require_phase_coherence(
            summary,
            phases,
            label=f"outcomes[{index}]",
        )
        log_relative, log_path = safe_artifact_path(
            results_dir,
            outcome["log_path"],
            label=f"outcomes[{index}].log_path",
        )
        if log_relative in artifact_paths:
            raise MutationCiError(f"duplicate outcome artifact {log_relative!r}")
        artifact_paths.add(log_relative)
        artifact_details[log_relative] = artifact_metadata(
            log_path,
            label=f"outcome log {log_relative}",
        )

        scenario = outcome["scenario"]
        if scenario == "Baseline":
            baseline_count += 1
            if summary != "Success":
                raise MutationCiError("baseline outcome did not succeed")
            if outcome["diff_path"] is not None:
                raise MutationCiError("baseline outcome must not have a diff")
            continue
        scenario_mapping = require_mapping(
            scenario,
            label=f"outcomes[{index}].scenario",
        )
        require_exact_keys(
            scenario_mapping,
            {"Mutant"},
            label=f"outcomes[{index}].scenario",
        )
        mutant = require_mutant(
            scenario_mapping["Mutant"],
            label=f"outcomes[{index}].scenario.Mutant",
            shard=shard,
            includes_diff=False,
        )
        name = mutant["name"]
        expected = inventory_by_name.get(name)
        if expected is None or mutant != without_diff(expected):
            raise MutationCiError(
                f"outcome mutant {name!r} does not exactly match the inventory"
            )
        if name in mutant_outcomes:
            raise MutationCiError(f"duplicate outcome for mutant {name!r}")
        mutant_outcomes[name] = summary
        if summary == "Success":
            raise MutationCiError("Success is valid only for the baseline")
        diff_relative, diff_path = safe_artifact_path(
            results_dir,
            outcome["diff_path"],
            label=f"outcomes[{index}].diff_path",
        )
        if diff_relative in artifact_paths:
            raise MutationCiError(f"duplicate outcome artifact {diff_relative!r}")
        artifact_paths.add(diff_relative)
        artifact_details[diff_relative] = artifact_metadata(
            diff_path,
            label=f"outcome diff {diff_relative}",
        )
    if baseline_count != 1:
        raise MutationCiError(
            f"outcomes must contain exactly one baseline; found {baseline_count}"
        )
    missing_outcomes = sorted(set(inventory_by_name) - set(mutant_outcomes))
    unknown_outcomes = sorted(set(mutant_outcomes) - set(inventory_by_name))
    if missing_outcomes or unknown_outcomes:
        raise MutationCiError(
            "mutation outcomes are incomplete; "
            f"missing={missing_outcomes}, unknown={unknown_outcomes}"
        )

    observed_by_summary = {
        summary: sorted(
            name for name, observed in mutant_outcomes.items() if observed == summary
        )
        for summary in STATUS_FILES
    }
    status_metadata: dict[str, dict[str, Any]] = {}
    for summary, filename in STATUS_FILES.items():
        path = results_dir / filename
        lines = parse_status_file(path, label=filename)
        if sorted(lines) != observed_by_summary[summary]:
            raise MutationCiError(
                f"{filename} does not exactly match structured {summary} outcomes"
            )
        status_metadata[filename] = artifact_metadata(path, label=filename)

    counts = {
        "caught": len(observed_by_summary["CaughtMutant"]),
        "missed": len(observed_by_summary["MissedMutant"]),
        "timeout": len(observed_by_summary["Timeout"]),
        "unviable": len(observed_by_summary["Unviable"]),
    }
    root_counts = {
        "total_mutants": require_int(
            root["total_mutants"],
            label="outcomes.total_mutants",
        ),
        "caught": require_int(root["caught"], label="outcomes.caught"),
        "missed": require_int(root["missed"], label="outcomes.missed"),
        "timeout": require_int(root["timeout"], label="outcomes.timeout"),
        "unviable": require_int(root["unviable"], label="outcomes.unviable"),
    }
    expected_root_counts = {"total_mutants": len(inventory), **counts}
    if root_counts != expected_root_counts:
        raise MutationCiError(
            f"outcome summary counts contradict detail: "
            f"summary={root_counts}, detail={expected_root_counts}"
        )

    lock_public, lock_bytes = parse_lock(
        results_dir,
        expected_version=expected_version,
    )
    accepted_by_identity = {entry.identity(): entry for entry in accepted}
    accepted_matches: list[dict[str, Any]] = []
    observed_unviable_identities: set[tuple[str, str, str, str, str]] = set()
    observed_unviable_counts: dict[tuple[str, str, str, str, str], int] = {}
    for name in observed_by_summary["Unviable"]:
        mutant = inventory_by_name[name]
        identity = mutant_identity(mutant)
        observed_unviable_identities.add(identity)
        observed_unviable_counts[identity] = observed_unviable_counts.get(identity, 0) + 1
        entry = accepted_by_identity.get(identity)
        if entry is None:
            continue
        accepted_matches.append(
            {
                "id": entry.identifier,
                "actual_mutant": name,
                "expected_count": entry.expected_count,
                "reason": entry.reason,
                "review_by": entry.review_by.isoformat(),
            }
        )
    accepted_identities = set(accepted_by_identity)
    unexpected_unviable = observed_unviable_identities - accepted_identities
    stale_unviable = accepted_identities - observed_unviable_identities

    viable = counts["caught"] + counts["missed"] + counts["timeout"]
    if viable == 0:
        kill_percent = decimal.Decimal("0")
    else:
        kill_percent = (
            decimal.Decimal(counts["caught"])
            * decimal.Decimal(100)
            / decimal.Decimal(viable)
        )
    failures: list[str] = []
    if shard.require_baseline and baseline_count != 1:
        failures.append("required unmutated baseline is missing")
    if counts["missed"] > shard.max_missed:
        failures.append(
            f"missed mutants {counts['missed']} exceed maximum {shard.max_missed}"
        )
    if counts["timeout"] > shard.max_timeouts:
        failures.append(
            f"timed-out mutants {counts['timeout']} exceed maximum "
            f"{shard.max_timeouts}"
        )
    for identity, entry in accepted_by_identity.items():
        observed_count = observed_unviable_counts.get(identity, 0)
        if observed_count != entry.expected_count:
            failures.append(
                f"accepted-unviable {entry.identifier!r} expected "
                f"{entry.expected_count} occurrence(s), observed {observed_count}"
            )
    if viable == 0:
        failures.append("mutation shard has no viable mutants")
    elif kill_percent < shard.minimum_viable_kill_percent:
        failures.append(
            f"viable kill percentage {kill_percent:.2f}% is below "
            f"{shard.minimum_viable_kill_percent:.2f}%"
        )
    if unexpected_unviable:
        failures.append(
            f"{len(unexpected_unviable)} unviable mutant(s) are not accepted"
        )
    if stale_unviable:
        failures.append(
            f"{len(stale_unviable)} accepted-unviable entry or entries are stale"
        )
    expected_zero_exit = counts["missed"] == 0 and counts["timeout"] == 0
    if expected_zero_exit and producer_exit_code != 0:
        raise MutationCiError(
            "cargo-mutants exited nonzero despite complete caught/unviable outcomes"
        )
    if not expected_zero_exit and producer_exit_code == 0:
        raise MutationCiError(
            "cargo-mutants exited zero despite missed or timed-out outcomes"
        )

    artifacts = {
        "mutants.json": {
            "bytes": len(inventory_bytes),
            "sha256": sha256(inventory_bytes),
        },
        "outcomes.json": {
            "bytes": len(outcomes_bytes),
            "sha256": sha256(outcomes_bytes),
        },
        "lock.json": {
            "bytes": len(lock_bytes),
            "sha256": sha256(lock_bytes),
            **lock_public,
        },
        **status_metadata,
        "outcome_files": dict(sorted(artifact_details.items())),
    }
    return {
        "status": "failed" if failures else "passed",
        "errors": failures,
        "summary": {
            **counts,
            "total_mutants": len(inventory),
            "viable_mutants": viable,
            "viable_kill_percent": f"{kill_percent:.2f}",
            "baseline": "passed",
        },
        "inventory": {
            "count": len(inventory),
            "canonical_sha256": canonical_digest(inventory),
            "pre_run_canonical_sha256": canonical_digest(list(pre_inventory)),
        },
        "accepted_unviable": sorted(
            accepted_matches,
            key=lambda item: item["id"],
        ),
        "survivors": observed_by_summary["MissedMutant"],
        "timeouts": observed_by_summary["Timeout"],
        "unexpected_unviable": sorted(
            name
            for name in observed_by_summary["Unviable"]
            if mutant_identity(inventory_by_name[name]) in unexpected_unviable
        ),
        "stale_accepted_unviable": sorted(
            entry.identifier
            for identity, entry in accepted_by_identity.items()
            if identity in stale_unviable
        ),
        "artifacts": artifacts,
    }


def cargo_test_scope_arguments(shard: ShardPolicy) -> list[str]:
    arguments: list[str] = []
    if shard.test_target == "lib":
        arguments.append("--lib")
    if shard.test_filter:
        arguments.append(shard.test_filter)
    return arguments


def cargo_test_inventory_command(shard: ShardPolicy) -> list[str]:
    return [
        "cargo",
        "test",
        "--package",
        shard.package,
        "--locked",
        "--all-features",
        *cargo_test_scope_arguments(shard),
        "--",
        "--list",
        "--format",
        "terse",
    ]


def cargo_mutants_base_command(shard: ShardPolicy) -> list[str]:
    command = [
        "cargo",
        "mutants",
        "--package",
        shard.package,
    ]
    for source in shard.files:
        command.extend(["--file", source])
    if shard.mutant_filter:
        command.extend(["--re", shard.mutant_filter])
    command.extend(
        [
            "--no-config",
            "--colors",
            "never",
            "--no-times",
            "--no-shuffle",
            "--all-features",
            "--cargo-arg=--locked",
        ]
    )
    for argument in cargo_test_scope_arguments(shard):
        command.append(f"--cargo-test-arg={argument}")
    command.extend(
        [
            "--jobs",
            str(shard.jobs),
            "--timeout",
            str(shard.timeout_seconds),
            "--build-timeout",
            str(shard.build_timeout_seconds),
        ]
    )
    return command


def run_process(
    argv: Sequence[str],
    *,
    cwd: pathlib.Path,
) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            list(argv),
            cwd=cwd,
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="strict",
        )
    except (OSError, UnicodeError) as error:
        raise MutationCiError(f"cannot run {argv[0]!r}: {error}") from error


def verify_tool(repo_root: pathlib.Path, expected_version: str) -> None:
    process = run_process(["cargo", "mutants", "--version"], cwd=repo_root)
    expected = f"cargo-mutants {expected_version}"
    if (
        process.returncode != 0
        or process.stdout.strip() != expected
        or process.stderr
    ):
        raise MutationCiError(
            f"cargo-mutants must report exactly {expected!r}; "
            f"exit={process.returncode}, stdout={process.stdout.strip()!r}, "
            f"stderr={process.stderr.strip()!r}"
        )


def report_template(
    *,
    mode: str,
    policy_path: pathlib.Path,
    shard_id: str,
) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": REPORT_KIND,
        "status": "error",
        "mode": mode,
        "policy_path": policy_path.as_posix(),
        "shard": shard_id,
        "owner": None,
        "package": None,
        "test_scope": None,
        "test_inventory": None,
        "cargo_mutants_version": None,
        "command": None,
        "producer_exit_code": None,
        "git": None,
        "source_bindings": {"before": [], "after": []},
        "verification_input_bindings": {"before": [], "after": []},
        "inventory": None,
        "summary": None,
        "accepted_unviable": [],
        "survivors": [],
        "timeouts": [],
        "unexpected_unviable": [],
        "stale_accepted_unviable": [],
        "artifacts": None,
        "errors": [],
    }


def resolve_policy_path(repo_root: pathlib.Path, value: str) -> pathlib.Path:
    candidate = pathlib.Path(value)
    if not candidate.is_absolute():
        candidate = repo_root / candidate
    try:
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(repo_root)
    except (OSError, ValueError) as error:
        raise MutationCiError(
            "mutation policy must be an existing file inside the repository"
        ) from error
    if not resolved.is_file():
        raise MutationCiError("mutation policy must be a file")
    return resolved


def resolve_output_path(repo_root: pathlib.Path, value: str, *, label: str) -> pathlib.Path:
    candidate = pathlib.Path(value)
    if not candidate.is_absolute():
        candidate = repo_root / candidate
    resolved = candidate.resolve(strict=False)
    target_root = (repo_root / "target").resolve(strict=False)
    try:
        relative = resolved.relative_to(target_root)
    except ValueError as error:
        raise MutationCiError(f"{label} must remain below the repository target directory") from error
    if relative == pathlib.Path("."):
        raise MutationCiError(f"{label} must name a path below the repository target directory")
    return resolved


def run_shard(args: argparse.Namespace) -> int:
    repo_root = pathlib.Path(args.repo_root).resolve()
    output_path = resolve_output_path(repo_root, args.output, label="report output")
    policy_display = pathlib.PurePosixPath(args.policy).as_posix()
    report = report_template(
        mode="run",
        policy_path=pathlib.Path(policy_display),
        shard_id=args.shard,
    )
    try:
        if not (repo_root / "Cargo.toml").is_file():
            raise MutationCiError("repository root must contain Cargo.toml")
        policy_path = resolve_policy_path(repo_root, args.policy)
        policy = load_policy(repo_root, policy_path)
        shard = policy.shard(args.shard)
        accepted = policy.accepted_for(shard.identifier)
        report.update(
            {
                "owner": shard.owner,
                "package": shard.package,
                "test_scope": {
                    "target": shard.test_target,
                    "filter": shard.test_filter,
                },
                "cargo_mutants_version": policy.cargo_mutants_version,
                "policy_path": policy_path.relative_to(repo_root).as_posix(),
                "git": git_binding(repo_root, shard.files),
            }
        )
        verify_tool(repo_root, policy.cargo_mutants_version)
        results_root = resolve_output_path(
            repo_root,
            args.results_root,
            label="mutation results root",
        )
        results_dir = results_root / MUTANTS_DIRECTORY
        if results_dir.exists():
            raise MutationCiError(
                f"mutation results directory already exists: {results_dir}"
            )
        results_root.mkdir(parents=True, exist_ok=True)
        source_before = source_bindings(repo_root, shard.files)
        report["source_bindings"]["before"] = source_before
        verification_inputs_before = verification_input_bindings(repo_root, policy_path)
        report["verification_input_bindings"]["before"] = verification_inputs_before

        test_inventory_command = cargo_test_inventory_command(shard)
        test_listing = run_process(test_inventory_command, cwd=repo_root)
        atomic_write_text(
            results_root / "test-inventory.stdout.txt",
            test_listing.stdout,
        )
        atomic_write_text(
            results_root / "test-inventory.stderr.txt",
            test_listing.stderr,
        )
        if test_listing.returncode != 0:
            raise MutationCiError(
                f"cargo test inventory failed with exit {test_listing.returncode}"
            )
        selected_tests = parse_test_inventory(test_listing.stdout, shard=shard)
        atomic_write_json(results_root / "test-inventory.json", selected_tests)
        report["test_inventory"] = test_inventory_binding(selected_tests)

        base_command = cargo_mutants_base_command(shard)
        list_command = [*base_command, "--list", "--json"]
        listing = run_process(list_command, cwd=repo_root)
        atomic_write_text(results_root / "inventory.stderr.txt", listing.stderr)
        if listing.returncode != 0:
            raise MutationCiError(
                f"cargo-mutants inventory failed with exit {listing.returncode}"
            )
        if listing.stderr:
            raise MutationCiError("cargo-mutants inventory wrote unexpected stderr")
        if len(listing.stdout.encode("utf-8")) > MAX_JSON_BYTES:
            raise MutationCiError("cargo-mutants inventory output is too large")
        pre_inventory = parse_inventory(
            parse_json_bytes(
                listing.stdout.encode("utf-8"),
                label="pre-run cargo-mutants inventory",
            ),
            shard=shard,
            label="pre-run cargo-mutants inventory",
        )
        atomic_write_json(results_root / "inventory.list.json", pre_inventory)

        mutation_command = [*base_command, "--output", str(results_root)]
        report["command"] = [
            *base_command,
            "--output",
            pathlib.PurePosixPath(args.results_root).as_posix(),
        ]
        producer = run_process(mutation_command, cwd=repo_root)
        report["producer_exit_code"] = producer.returncode
        atomic_write_text(results_root / "producer.stdout.txt", producer.stdout)
        atomic_write_text(results_root / "producer.stderr.txt", producer.stderr)
        if producer.stdout:
            print(producer.stdout, end="")
        if producer.stderr:
            print(producer.stderr, end="", file=sys.stderr)

        source_after = source_bindings(repo_root, shard.files)
        report["source_bindings"]["after"] = source_after
        verification_inputs_after = verification_input_bindings(repo_root, policy_path)
        report["verification_input_bindings"]["after"] = verification_inputs_after
        if verification_inputs_after != verification_inputs_before:
            raise MutationCiError("mutation verification inputs changed during the run")
        evaluation = evaluate_results(
            results_dir=results_dir,
            shard=shard,
            accepted=accepted,
            expected_version=policy.cargo_mutants_version,
            producer_exit_code=producer.returncode,
            pre_inventory=pre_inventory,
            source_before=source_before,
            source_after=source_after,
        )
        report.update(evaluation)
        atomic_write_json(output_path, report)
        if report["status"] == "passed":
            print(
                f"mutation shard {shard.identifier}: "
                f"{report['summary']['caught']}/"
                f"{report['summary']['viable_mutants']} viable mutants caught "
                f"({report['summary']['viable_kill_percent']}%)"
            )
            return 0
        for error in report["errors"]:
            print(f"mutation policy failure: {error}", file=sys.stderr)
        return 1
    except MutationCiError as error:
        report["status"] = "error"
        report["errors"] = [str(error)]
        try:
            atomic_write_json(output_path, report)
        except MutationCiError as write_error:
            print(f"mutation evidence error: {write_error}", file=sys.stderr)
        print(f"mutation evidence error: {error}", file=sys.stderr)
        return 2


def validate_policy(args: argparse.Namespace) -> int:
    repo_root = pathlib.Path(args.repo_root).resolve()
    try:
        if not (repo_root / "Cargo.toml").is_file():
            raise MutationCiError("repository root must contain Cargo.toml")
        policy_path = resolve_policy_path(repo_root, args.policy)
        policy = load_policy(repo_root, policy_path)
        if args.shard:
            policy.shard(args.shard)
        print(
            f"mutation policy valid: {len(policy.shards)} shard(s), "
            f"{len(policy.accepted_unviable)} accepted unviable mutation(s)"
        )
        return 0
    except MutationCiError as error:
        print(f"mutation policy error: {error}", file=sys.stderr)
        return 2


def verify_report(args: argparse.Namespace) -> int:
    """Reject mutation evidence that is stale for the current source tree."""

    repo_root = pathlib.Path(args.repo_root).resolve()
    try:
        if not (repo_root / "Cargo.toml").is_file():
            raise MutationCiError("repository root must contain Cargo.toml")
        policy_path = resolve_policy_path(repo_root, args.policy)
        policy = load_policy(repo_root, policy_path)
        shard = policy.shard(args.shard)
        report_path = resolve_output_path(
            repo_root,
            args.report,
            label="mutation evidence report",
        )
        if not report_path.is_file():
            raise MutationCiError("mutation evidence report must be an existing file")
        report_value, _ = load_json(report_path, label="mutation evidence report")
        report = require_mapping(report_value, label="mutation evidence report")
        if report.get("schema_version") != SCHEMA_VERSION:
            raise MutationCiError("mutation evidence report has the wrong schema version")
        if report.get("kind") != REPORT_KIND:
            raise MutationCiError("mutation evidence report has the wrong kind")
        if report.get("status") != "passed":
            raise MutationCiError("mutation evidence report did not pass")
        if report.get("shard") != shard.identifier:
            raise MutationCiError("mutation evidence report names a different shard")

        bindings = require_mapping(
            report.get("source_bindings"),
            label="mutation evidence source_bindings",
        )
        current_bindings = source_bindings(repo_root, shard.files)
        if bindings.get("before") != current_bindings:
            raise MutationCiError(
                "mutation evidence source bindings are stale before execution"
            )
        if bindings.get("after") != current_bindings:
            raise MutationCiError(
                "mutation evidence source bindings are stale after execution"
            )

        verification_bindings = require_mapping(
            report.get("verification_input_bindings"),
            label="mutation evidence verification_input_bindings",
        )
        current_verification_bindings = verification_input_bindings(repo_root, policy_path)
        if verification_bindings.get("before") != current_verification_bindings:
            raise MutationCiError(
                "mutation evidence verification inputs are stale before execution"
            )
        if verification_bindings.get("after") != current_verification_bindings:
            raise MutationCiError(
                "mutation evidence verification inputs are stale after execution"
            )

        test_listing = run_process(cargo_test_inventory_command(shard), cwd=repo_root)
        if test_listing.returncode != 0:
            raise MutationCiError(
                "current cargo test inventory failed with exit "
                f"{test_listing.returncode}"
            )
        current_tests = parse_test_inventory(test_listing.stdout, shard=shard)
        if report.get("test_inventory") != test_inventory_binding(current_tests):
            raise MutationCiError("mutation evidence test inventory is stale")

        summary = require_mapping(
            report.get("summary"),
            label="mutation evidence summary",
        )
        if summary.get("missed") != 0 or summary.get("timeout") != 0:
            raise MutationCiError("mutation evidence contains a survivor or timeout")
        if summary.get("caught") != summary.get("viable_mutants"):
            raise MutationCiError("mutation evidence does not catch every viable mutant")
        print(
            f"mutation evidence current: {shard.identifier} "
            f"({summary['caught']}/{summary['viable_mutants']} viable mutants caught)"
        )
        return 0
    except MutationCiError as error:
        print(f"mutation evidence error: {error}", file=sys.stderr)
        return 2


def verify_report_set(args: argparse.Namespace) -> int:
    """Verify one unambiguous current report for every manifest-selected shard."""

    repo_root = pathlib.Path(args.repo_root).resolve()
    try:
        manifest_path = resolve_policy_path(repo_root, args.manifest)
        manifest_value, _ = load_json(manifest_path, label="mutation report-set manifest")
        manifest = require_mapping(manifest_value, label="mutation report-set manifest")
        expected_keys = {"schema_version", "kind", "policy", "reports"}
        if set(manifest) != expected_keys:
            raise MutationCiError(
                "mutation report-set manifest fields do not match schema: "
                f"expected {sorted(expected_keys)!r}, got {sorted(manifest)!r}"
            )
        if manifest.get("schema_version") != 1:
            raise MutationCiError("mutation report-set manifest has the wrong schema version")
        if manifest.get("kind") != "sorotte-mutation-report-set":
            raise MutationCiError("mutation report-set manifest has the wrong kind")
        policy_value = require_string(
            manifest.get("policy"),
            label="mutation report-set manifest.policy",
        )
        if pathlib.PurePosixPath(policy_value).as_posix() != pathlib.PurePosixPath(
            args.policy
        ).as_posix():
            raise MutationCiError("mutation report-set manifest names a different policy")
        policy_path = resolve_policy_path(repo_root, args.policy)
        policy = load_policy(repo_root, policy_path)
        reports = require_mapping(
            manifest.get("reports"),
            label="mutation report-set manifest.reports",
        )
        if not reports:
            raise MutationCiError("mutation report-set manifest must select at least one report")

        verified = 0
        for shard_id, report_value in sorted(reports.items()):
            if not isinstance(shard_id, str) or not IDENTIFIER.fullmatch(shard_id):
                raise MutationCiError("mutation report-set manifest has an invalid shard id")
            policy.shard(shard_id)
            report_path = require_string(
                report_value,
                label=f"mutation report-set manifest.reports.{shard_id}",
            )
            result = verify_report(
                argparse.Namespace(
                    repo_root=str(repo_root),
                    policy=args.policy,
                    shard=shard_id,
                    report=report_path,
                )
            )
            if result != 0:
                raise MutationCiError(
                    f"mutation report-set member {shard_id!r} did not verify"
                )
            verified += 1
        print(f"mutation report set current: {verified} uniquely selected shard report(s)")
        return 0
    except MutationCiError as error:
        print(f"mutation report-set error: {error}", file=sys.stderr)
        return 2


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description=__doc__)
    subparsers = value.add_subparsers(dest="command", required=True)

    validate = subparsers.add_parser("validate")
    validate.add_argument("--repo-root", default=".")
    validate.add_argument("--policy", default=DEFAULT_POLICY)
    validate.add_argument("--shard")

    run = subparsers.add_parser("run")
    run.add_argument("--repo-root", default=".")
    run.add_argument("--policy", default=DEFAULT_POLICY)
    run.add_argument("--shard", required=True)
    run.add_argument("--results-root", required=True)
    run.add_argument("--output", required=True)

    verify = subparsers.add_parser("verify-report")
    verify.add_argument("--repo-root", default=".")
    verify.add_argument("--policy", default=DEFAULT_POLICY)
    verify.add_argument("--shard", required=True)
    verify.add_argument("--report", required=True)
    verify_set = subparsers.add_parser("verify-report-set")
    verify_set.add_argument("--repo-root", default=".")
    verify_set.add_argument("--policy", default=DEFAULT_POLICY)
    verify_set.add_argument("--manifest", required=True)
    return value


def main(argv: Sequence[str] | None = None) -> int:
    args = parser().parse_args(argv)
    if args.command == "validate":
        return validate_policy(args)
    if args.command == "run":
        return run_shard(args)
    if args.command == "verify-report":
        return verify_report(args)
    if args.command == "verify-report-set":
        return verify_report_set(args)
    raise AssertionError(f"unknown command {args.command!r}")


if __name__ == "__main__":
    raise SystemExit(main())
