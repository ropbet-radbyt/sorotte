#!/usr/bin/env python3
"""Build a source-bound physical-line map from native LLVM coverage artifacts.

``cargo llvm-cov`` exposes two useful but intentionally different views:

* LLVM's JSON export identifies the producer, schema, files, segments, and
  aggregate summaries.
* LLVM's native ``show`` text identifies the execution count attached to each
  physical source line.

The JSON line summary is not an identity map: monomorphized Rust code can make
its line-instance totals differ from the unique physical lines printed by
``llvm-cov show``.  This converter therefore keeps both measurements, uses the
native source view for changed-line policy, and never rewrites or attempts to
reconcile the LLVM summary.

Every source row must exactly match the current checkout.  The resulting JSON
also records a SHA-256 digest for every source file and both producer artifacts.
Unknown schemas, fields, text rows, count formats, paths, or source drift fail
closed.  On an expected input failure, ``--output`` receives a machine-readable
error report rather than a partial line map.
"""

from __future__ import annotations

import argparse
import decimal
import hashlib
import json
import math
import os
import pathlib
import re
import sys
import tempfile
from collections.abc import Mapping, Sequence
from typing import Any


SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-llvm-line-map"
SUPPORTED_LLVM_EXPORT_TYPE = "llvm.coverage.json.export"
SUPPORTED_LLVM_EXPORT_VERSION = "3.1.0"
SUPPORTED_CARGO_LLVM_COV_VERSION = "0.8.4"
MAX_LLVM_JSON_BYTES = 256 * 1024 * 1024
MAX_LLVM_TEXT_BYTES = 256 * 1024 * 1024
MAX_SOURCE_BYTES = 16 * 1024 * 1024

ROOT_KEYS = frozenset({"data", "type", "version", "cargo_llvm_cov"})
CARGO_KEYS = frozenset({"version", "manifest_path"})
DATA_KEYS = frozenset({"files", "totals"})
FILE_KEYS = frozenset(
    {
        "branches",
        "mcdc_records",
        "expansions",
        "filename",
        "segments",
        "summary",
    }
)
SUMMARY_KEYS = frozenset(
    {"branches", "functions", "instantiations", "lines", "mcdc", "regions"}
)
COUNT_METRIC_KEYS = frozenset({"count", "covered", "percent"})
COUNT_WITH_NOT_COVERED_KEYS = frozenset(
    {"count", "covered", "notcovered", "percent"}
)
METRICS_WITH_NOT_COVERED = frozenset({"branches", "mcdc", "regions"})

SOURCE_ROW = re.compile(r"^ *([1-9][0-9]*)\|([^|]*)\|(.*)$")
ABBREVIATED_COUNT_PATTERN = (
    r"(?:[1-9]\.[0-9]{2}|[1-9][0-9]\.[0-9]|[1-9][0-9]{2})[kMGTPE]"
)
POSITIVE_COUNT = re.compile(
    rf"^(?:[1-9][0-9]{{0,2}}|{ABBREVIATED_COUNT_PATTERN})$"
)
ANNOTATION_ROW = re.compile(
    rf"^(?: *\^(?:0|[1-9][0-9]{{0,2}}|{ABBREVIATED_COUNT_PATTERN}))+ *$"
)


class LlvmCovLineMapError(ValueError):
    """An invalid, unsupported, unsafe, or stale LLVM coverage artifact."""


def sha256_bytes(value: bytes) -> str:
    return f"sha256:{hashlib.sha256(value).hexdigest()}"


def exact_keys(value: Mapping[str, Any], expected: frozenset[str], *, context: str) -> None:
    actual = frozenset(value)
    if actual != expected:
        missing = sorted(expected - actual)
        unknown = sorted(actual - expected)
        details: list[str] = []
        if missing:
            details.append(f"missing {missing}")
        if unknown:
            details.append(f"unknown {unknown}")
        raise LlvmCovLineMapError(f"{context} fields are invalid: {', '.join(details)}")


def require_object(value: Any, *, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise LlvmCovLineMapError(f"{context} must be a JSON object")
    return value


def require_array(value: Any, *, context: str) -> list[Any]:
    if not isinstance(value, list):
        raise LlvmCovLineMapError(f"{context} must be a JSON array")
    return value


def require_string(value: Any, *, context: str) -> str:
    if not isinstance(value, str) or not value or value != value.strip() or "\x00" in value:
        raise LlvmCovLineMapError(
            f"{context} must be a non-empty, trimmed string without NUL bytes"
        )
    return value


def require_nonnegative_integer(value: Any, *, context: str) -> int:
    if type(value) is not int or value < 0:
        raise LlvmCovLineMapError(f"{context} must be a non-negative integer")
    return value


def require_positive_integer(value: Any, *, context: str) -> int:
    parsed = require_nonnegative_integer(value, context=context)
    if parsed == 0:
        raise LlvmCovLineMapError(f"{context} must be positive")
    return parsed


def read_bounded(path: pathlib.Path, *, limit: int, description: str) -> bytes:
    try:
        stat = path.stat()
    except OSError as error:
        raise LlvmCovLineMapError(
            f"cannot inspect {description} {path}: {error}"
        ) from error
    if not path.is_file():
        raise LlvmCovLineMapError(f"{description} is not a regular file: {path}")
    if stat.st_size <= 0:
        raise LlvmCovLineMapError(f"{description} is empty: {path}")
    if stat.st_size > limit:
        raise LlvmCovLineMapError(
            f"{description} exceeds the {limit}-byte safety limit: {stat.st_size} bytes"
        )
    try:
        return path.read_bytes()
    except OSError as error:
        raise LlvmCovLineMapError(f"cannot read {description} {path}: {error}") from error


def decode_utf8(value: bytes, *, description: str) -> str:
    try:
        text = value.decode("utf-8")
    except UnicodeDecodeError as error:
        raise LlvmCovLineMapError(f"{description} is not valid UTF-8: {error}") from error
    if "\x00" in text:
        raise LlvmCovLineMapError(f"{description} contains a NUL byte")
    return text


def normalized_lines(text: str, *, description: str) -> list[str]:
    if re.search(r"\r(?!\n)", text):
        raise LlvmCovLineMapError(f"{description} contains a bare carriage return")
    normalized = text.replace("\r\n", "\n")
    if not normalized:
        return []
    lines = normalized.split("\n")
    if normalized.endswith("\n"):
        lines.pop()
    return lines


def duplicate_rejecting_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise LlvmCovLineMapError(f"LLVM JSON duplicates object key {key!r}")
        value[key] = item
    return value


def reject_json_constant(value: str) -> None:
    raise LlvmCovLineMapError(f"LLVM JSON contains unsupported numeric constant {value}")


def parse_json(value: bytes) -> dict[str, Any]:
    text = decode_utf8(value, description="LLVM JSON")
    try:
        parsed = json.loads(
            text,
            object_pairs_hook=duplicate_rejecting_object,
            parse_constant=reject_json_constant,
        )
    except LlvmCovLineMapError:
        raise
    except json.JSONDecodeError as error:
        raise LlvmCovLineMapError(f"LLVM JSON is malformed: {error}") from error
    return require_object(parsed, context="LLVM JSON root")


def resolve_inside_repository(
    raw: str,
    *,
    repo_root: pathlib.Path,
    context: str,
    require_rust: bool,
) -> tuple[str, pathlib.Path]:
    path_text = require_string(raw, context=context)
    candidate = pathlib.Path(path_text)
    if not candidate.is_absolute():
        candidate = repo_root / candidate
    try:
        resolved = candidate.resolve(strict=True)
    except OSError as error:
        raise LlvmCovLineMapError(f"{context} does not resolve to a file: {raw}") from error
    try:
        relative = resolved.relative_to(repo_root)
    except ValueError as error:
        raise LlvmCovLineMapError(
            f"{context} resolves outside the repository: {raw}"
        ) from error
    if not resolved.is_file():
        raise LlvmCovLineMapError(f"{context} is not a regular file: {raw}")
    relative_text = relative.as_posix()
    if require_rust and (
        resolved.suffix != ".rs"
        or not relative_text.startswith("crates/")
    ):
        raise LlvmCovLineMapError(
            f"{context} must be a Rust source below crates/: {relative_text}"
        )
    return relative_text, resolved


def validate_percentage(
    value: Any,
    *,
    count: int,
    covered: int,
    context: str,
) -> None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise LlvmCovLineMapError(f"{context} percent must be a finite JSON number")
    parsed = float(value)
    if not math.isfinite(parsed) or parsed < 0.0 or parsed > 100.0:
        raise LlvmCovLineMapError(f"{context} percent must be between 0 and 100")
    expected = 0.0 if count == 0 else covered * 100.0 / count
    if not math.isclose(parsed, expected, rel_tol=1e-12, abs_tol=1e-12):
        raise LlvmCovLineMapError(
            f"{context} percent {parsed} disagrees with {covered}/{count}"
        )


def validate_summary(value: Any, *, context: str) -> dict[str, dict[str, Any]]:
    summary = require_object(value, context=context)
    exact_keys(summary, SUMMARY_KEYS, context=context)
    validated: dict[str, dict[str, Any]] = {}
    for metric_name in sorted(SUMMARY_KEYS):
        metric_context = f"{context}.{metric_name}"
        metric = require_object(summary[metric_name], context=metric_context)
        expected = (
            COUNT_WITH_NOT_COVERED_KEYS
            if metric_name in METRICS_WITH_NOT_COVERED
            else COUNT_METRIC_KEYS
        )
        exact_keys(metric, expected, context=metric_context)
        count = require_nonnegative_integer(
            metric["count"], context=f"{metric_context}.count"
        )
        covered = require_nonnegative_integer(
            metric["covered"], context=f"{metric_context}.covered"
        )
        if covered > count:
            raise LlvmCovLineMapError(
                f"{metric_context}.covered cannot exceed its count"
            )
        if "notcovered" in metric:
            not_covered = require_nonnegative_integer(
                metric["notcovered"], context=f"{metric_context}.notcovered"
            )
            if covered + not_covered != count:
                raise LlvmCovLineMapError(
                    f"{metric_context} covered/notcovered values do not sum to count"
                )
        validate_percentage(
            metric["percent"],
            count=count,
            covered=covered,
            context=metric_context,
        )
        validated[metric_name] = metric
    return validated


def validate_segments(value: Any, *, context: str) -> None:
    segments = require_array(value, context=context)
    if not segments:
        raise LlvmCovLineMapError(f"{context} must not be empty")
    previous: tuple[int, int] | None = None
    for index, raw_segment in enumerate(segments):
        segment_context = f"{context}[{index}]"
        segment = require_array(raw_segment, context=segment_context)
        if len(segment) != 6:
            raise LlvmCovLineMapError(
                f"{segment_context} must contain exactly six values"
            )
        line = require_positive_integer(segment[0], context=f"{segment_context}[0]")
        column = require_positive_integer(segment[1], context=f"{segment_context}[1]")
        require_nonnegative_integer(segment[2], context=f"{segment_context}[2]")
        for field in range(3, 6):
            if type(segment[field]) is not bool:
                raise LlvmCovLineMapError(
                    f"{segment_context}[{field}] must be a boolean"
                )
        position = (line, column)
        if previous is not None and position < previous:
            raise LlvmCovLineMapError(f"{context} is not ordered by line and column")
        previous = position


def source_bytes_and_lines(path: pathlib.Path, *, relative: str) -> tuple[bytes, list[str]]:
    raw = read_bounded(
        path,
        limit=MAX_SOURCE_BYTES,
        description=f"source file {relative}",
    )
    text = decode_utf8(raw, description=f"source file {relative}")
    return raw, normalized_lines(text, description=f"source file {relative}")


def validate_json_export(
    root: dict[str, Any],
    *,
    repo_root: pathlib.Path,
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]], str]:
    exact_keys(root, ROOT_KEYS, context="LLVM JSON root")
    if root["type"] != SUPPORTED_LLVM_EXPORT_TYPE:
        raise LlvmCovLineMapError(
            f"LLVM JSON type must be {SUPPORTED_LLVM_EXPORT_TYPE!r}"
        )
    if root["version"] != SUPPORTED_LLVM_EXPORT_VERSION:
        raise LlvmCovLineMapError(
            "LLVM JSON export version "
            f"{root['version']!r} is unsupported; expected "
            f"{SUPPORTED_LLVM_EXPORT_VERSION!r}"
        )

    cargo = require_object(root["cargo_llvm_cov"], context="cargo_llvm_cov")
    exact_keys(cargo, CARGO_KEYS, context="cargo_llvm_cov")
    if cargo["version"] != SUPPORTED_CARGO_LLVM_COV_VERSION:
        raise LlvmCovLineMapError(
            "cargo-llvm-cov version "
            f"{cargo['version']!r} is unsupported; expected "
            f"{SUPPORTED_CARGO_LLVM_COV_VERSION!r}"
        )
    manifest_relative, manifest_path = resolve_inside_repository(
        cargo["manifest_path"],
        repo_root=repo_root,
        context="cargo_llvm_cov.manifest_path",
        require_rust=False,
    )
    if manifest_relative != "Cargo.toml" or manifest_path != repo_root / "Cargo.toml":
        raise LlvmCovLineMapError(
            "cargo_llvm_cov.manifest_path must resolve to repository Cargo.toml"
        )

    data = require_array(root["data"], context="LLVM JSON data")
    if len(data) != 1:
        raise LlvmCovLineMapError("LLVM JSON data must contain exactly one export")
    export = require_object(data[0], context="LLVM JSON data[0]")
    exact_keys(export, DATA_KEYS, context="LLVM JSON data[0]")
    totals = validate_summary(export["totals"], context="LLVM JSON totals")

    files = require_array(export["files"], context="LLVM JSON files")
    if not files:
        raise LlvmCovLineMapError("LLVM JSON contains no files")
    identities: dict[str, str] = {}
    validated_files: list[dict[str, Any]] = []
    aggregate_line_count = 0
    aggregate_line_covered = 0
    for index, raw_file in enumerate(files):
        context = f"LLVM JSON files[{index}]"
        file_value = require_object(raw_file, context=context)
        exact_keys(file_value, FILE_KEYS, context=context)
        relative, path = resolve_inside_repository(
            file_value["filename"],
            repo_root=repo_root,
            context=f"{context}.filename",
            require_rust=True,
        )
        identity = relative.casefold() if os.name == "nt" else relative
        if identity in identities:
            raise LlvmCovLineMapError(
                f"LLVM JSON contains duplicate source file {relative!r}"
            )
        identities[identity] = relative

        summary = validate_summary(file_value["summary"], context=f"{context}.summary")
        validate_segments(file_value["segments"], context=f"{context}.segments")
        for empty_field in ("branches", "mcdc_records", "expansions"):
            records = require_array(
                file_value[empty_field],
                context=f"{context}.{empty_field}",
            )
            if records:
                raise LlvmCovLineMapError(
                    f"{context}.{empty_field} must be empty for the pinned "
                    "non-branch export"
                )
        line_summary = summary["lines"]
        aggregate_line_count += line_summary["count"]
        aggregate_line_covered += line_summary["covered"]
        source_raw, source_lines = source_bytes_and_lines(path, relative=relative)
        if not source_lines:
            raise LlvmCovLineMapError(
                f"LLVM JSON source file {relative!r} has no physical lines"
            )
        validated_files.append(
            {
                "path": relative,
                "resolved": path,
                "source_raw": source_raw,
                "source_lines": source_lines,
            }
        )

    root_lines = totals["lines"]
    if (
        aggregate_line_count != root_lines["count"]
        or aggregate_line_covered != root_lines["covered"]
    ):
        raise LlvmCovLineMapError(
            "LLVM JSON per-file line summaries do not equal the root line summary"
        )
    return validated_files, totals, manifest_relative


def parse_count_token(token_field: str, *, context: str) -> int | None:
    if token_field != token_field.strip(" "):
        token = token_field.strip(" ")
    else:
        token = token_field
    if "\t" in token_field:
        raise LlvmCovLineMapError(f"{context} count column contains a tab")
    if token == "":
        return None
    if token == "0":
        return 0
    if POSITIVE_COUNT.fullmatch(token):
        return 1
    raise LlvmCovLineMapError(
        f"{context} has unsupported execution-count token {token!r}"
    )


def parse_native_text(
    value: bytes,
    *,
    repo_root: pathlib.Path,
    files: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    text = decode_utf8(value, description="LLVM native text")
    rows = normalized_lines(text, description="LLVM native text")
    if not rows:
        raise LlvmCovLineMapError("LLVM native text contains no rows")

    cursor = 0
    mapped_files: list[dict[str, Any]] = []
    for file_index, file_value in enumerate(files):
        relative = file_value["path"]
        if cursor >= len(rows):
            raise LlvmCovLineMapError(
                f"LLVM native text is missing the header for {relative!r}"
            )
        header = rows[cursor]
        if not header.endswith(":"):
            raise LlvmCovLineMapError(
                f"LLVM native text row {cursor + 1} is not a source header"
            )
        header_relative, _header_path = resolve_inside_repository(
            header[:-1],
            repo_root=repo_root,
            context=f"LLVM native text row {cursor + 1} header",
            require_rust=True,
        )
        if header_relative != relative:
            raise LlvmCovLineMapError(
                f"LLVM native text file order mismatch: expected {relative!r}, "
                f"found {header_relative!r}"
            )
        cursor += 1

        physical_lines: list[list[int]] = []
        covered = 0
        source_lines = file_value["source_lines"]
        for expected_line, expected_source in enumerate(source_lines, start=1):
            if cursor >= len(rows):
                raise LlvmCovLineMapError(
                    f"LLVM native text is truncated at {relative}:{expected_line}"
                )
            match = SOURCE_ROW.fullmatch(rows[cursor])
            if match is None:
                raise LlvmCovLineMapError(
                    f"LLVM native text row {cursor + 1} is not source line "
                    f"{relative}:{expected_line}"
                )
            observed_line = int(match.group(1))
            if observed_line != expected_line:
                raise LlvmCovLineMapError(
                    f"LLVM native text row {cursor + 1} has source line "
                    f"{observed_line}; expected {expected_line}"
                )
            if match.group(3) != expected_source:
                raise LlvmCovLineMapError(
                    f"LLVM native text source content disagrees with the checkout "
                    f"at {relative}:{expected_line}"
                )
            execution = parse_count_token(
                match.group(2),
                context=f"LLVM native text {relative}:{expected_line}",
            )
            if execution is not None:
                physical_lines.append([expected_line, execution])
                covered += execution
            cursor += 1
            while cursor < len(rows) and ANNOTATION_ROW.fullmatch(rows[cursor]):
                cursor += 1

        if file_index + 1 < len(files):
            if cursor >= len(rows) or rows[cursor] != "":
                raise LlvmCovLineMapError(
                    f"LLVM native text must contain one blank separator after {relative!r}"
                )
            cursor += 1
            if cursor < len(rows) and rows[cursor] == "":
                raise LlvmCovLineMapError(
                    f"LLVM native text contains multiple separators after {relative!r}"
                )

        mapped_files.append(
            {
                "path": relative,
                "source_sha256": sha256_bytes(file_value["source_raw"]),
                "source_line_count": len(source_lines),
                "instrumented_line_count": len(physical_lines),
                "covered_line_count": covered,
                "lines": physical_lines,
            }
        )

    if cursor != len(rows):
        raise LlvmCovLineMapError(
            f"LLVM native text has unexpected content at row {cursor + 1}"
        )
    return mapped_files


def percent_text(covered: int, count: int) -> str | None:
    if count == 0:
        return None
    with decimal.localcontext() as context:
        context.prec = 40
        value = (
            decimal.Decimal(covered) * decimal.Decimal(100) / decimal.Decimal(count)
        )
        return format(value.quantize(decimal.Decimal("0.000001")), "f")


def artifact_metadata(path: pathlib.Path, value: bytes) -> dict[str, Any]:
    return {
        "path": str(path.resolve()),
        "size_bytes": len(value),
        "sha256": sha256_bytes(value),
    }


def build_report(
    *,
    repo_root: pathlib.Path,
    llvm_json_path: pathlib.Path,
    llvm_text_path: pathlib.Path,
) -> dict[str, Any]:
    root = repo_root.resolve()
    if not root.is_dir():
        raise LlvmCovLineMapError(f"repository root is not a directory: {root}")
    if not (root / "Cargo.toml").is_file():
        raise LlvmCovLineMapError(f"repository root has no Cargo.toml: {root}")

    json_bytes = read_bounded(
        llvm_json_path,
        limit=MAX_LLVM_JSON_BYTES,
        description="LLVM JSON",
    )
    text_bytes = read_bounded(
        llvm_text_path,
        limit=MAX_LLVM_TEXT_BYTES,
        description="LLVM native text",
    )
    json_root = parse_json(json_bytes)
    files, totals, manifest_relative = validate_json_export(
        json_root,
        repo_root=root,
    )
    mapped_files = parse_native_text(
        text_bytes,
        repo_root=root,
        files=files,
    )

    source_line_count = sum(item["source_line_count"] for item in mapped_files)
    instrumented = sum(item["instrumented_line_count"] for item in mapped_files)
    covered = sum(item["covered_line_count"] for item in mapped_files)
    llvm_count = totals["lines"]["count"]
    llvm_covered = totals["lines"]["covered"]
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": REPORT_KIND,
        "status": "passed",
        "line_model": "unique-physical-source-lines",
        "inputs": {
            "llvm_json": artifact_metadata(llvm_json_path, json_bytes),
            "llvm_text": artifact_metadata(llvm_text_path, text_bytes),
        },
        "producer": {
            "llvm_export_type": SUPPORTED_LLVM_EXPORT_TYPE,
            "llvm_export_version": SUPPORTED_LLVM_EXPORT_VERSION,
            "cargo_llvm_cov_version": SUPPORTED_CARGO_LLVM_COV_VERSION,
            "manifest_path": manifest_relative,
        },
        "summary": {
            "file_count": len(mapped_files),
            "source_line_count": source_line_count,
            "instrumented_line_count": instrumented,
            "covered_line_count": covered,
            "uncovered_line_count": instrumented - covered,
            "physical_line_percent": percent_text(covered, instrumented),
            "llvm_summary_line_count": llvm_count,
            "llvm_summary_covered_line_count": llvm_covered,
            "llvm_summary_line_percent": percent_text(llvm_covered, llvm_count),
            "llvm_minus_physical_line_count": llvm_count - instrumented,
            "llvm_minus_physical_covered_line_count": llvm_covered - covered,
        },
        "files": mapped_files,
        "errors": [],
    }


def atomic_write_json(path: pathlib.Path, value: Mapping[str, Any]) -> None:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise LlvmCovLineMapError(
            f"cannot create output directory for {path}: {error}"
        ) from error
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
        raise LlvmCovLineMapError(f"cannot write line-map report {path}: {error}") from error
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


def error_report(error: Exception) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": REPORT_KIND,
        "status": "error",
        "errors": [str(error)],
    }


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path("."))
    parser.add_argument("--llvm-json", type=pathlib.Path, required=True)
    parser.add_argument("--llvm-text", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = argument_parser().parse_args(argv)
    try:
        report = build_report(
            repo_root=args.repo_root,
            llvm_json_path=args.llvm_json,
            llvm_text_path=args.llvm_text,
        )
        atomic_write_json(args.output, report)
        summary = report["summary"]
        print(
            "LLVM physical line map: "
            f"{summary['covered_line_count']}/{summary['instrumented_line_count']} "
            f"({summary['physical_line_percent']}%); "
            "LLVM summary retained separately as "
            f"{summary['llvm_summary_covered_line_count']}/"
            f"{summary['llvm_summary_line_count']} "
            f"({summary['llvm_summary_line_percent']}%)"
        )
        return 0
    except LlvmCovLineMapError as error:
        try:
            atomic_write_json(args.output, error_report(error))
        except LlvmCovLineMapError as write_error:
            print(f"LLVM line-map input error: {error}", file=sys.stderr)
            print(f"cannot write error report: {write_error}", file=sys.stderr)
            return 2
        print(f"LLVM line-map input error: {error}", file=sys.stderr)
        return 2
    except Exception as error:  # Defensive fail-closed boundary for CI evidence.
        wrapped = LlvmCovLineMapError(
            f"unexpected {type(error).__name__} while building LLVM line map: {error}"
        )
        try:
            atomic_write_json(args.output, error_report(wrapped))
        except LlvmCovLineMapError as write_error:
            print(f"LLVM line-map input error: {wrapped}", file=sys.stderr)
            print(f"cannot write error report: {write_error}", file=sys.stderr)
            return 2
        print(f"LLVM line-map input error: {wrapped}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
