"""Strict, bounded serialization primitives for independent verification oracles.

Domain schemas and supported versions deliberately stay with each consumer.
No error includes malformed input values or duplicated field contents.
"""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
from typing import Any, BinaryIO

DEFAULT_MAX_BYTES = 32 * 1024 * 1024
DEFAULT_MAX_RECORD_BYTES = 64 * 1024
DEFAULT_MAX_RECORDS = 200_000


class ArtifactInputError(ValueError):
    def __init__(self, category: str, message: str) -> None:
        self.category = category
        super().__init__(f"{category}: {message}")


def read_stream_bounded(source: BinaryIO, *, max_bytes: int, label: str = "artifact") -> bytes:
    """Bound actual reads, including a file that grows after a metadata check."""
    require_int(max_bytes, label="byte limit", minimum=0)
    chunks: list[bytes] = []
    remaining = max_bytes
    while True:
        block = source.read(min(64 * 1024, remaining + 1))
        if not block:
            return b"".join(chunks)
        if len(block) > remaining:
            raise ArtifactInputError("byte_limit", f"{label} exceeds the {max_bytes}-byte safety limit")
        chunks.append(block)
        remaining -= len(block)


def read_bounded(path: Path, *, max_bytes: int = DEFAULT_MAX_BYTES, label: str = "artifact") -> bytes:
    try:
        # This check avoids reading obviously oversized regular artifacts. It is
        # only an optimization; the opened stream has the same actual-byte cap.
        if path.stat().st_size > max_bytes:
            raise ArtifactInputError("byte_limit", f"{label} exceeds the {max_bytes}-byte safety limit")
        with path.open("rb") as source:
            return read_stream_bounded(source, max_bytes=max_bytes, label=label)
    except OSError as error:
        raise ArtifactInputError("io", f"cannot read {label}") from error


def is_json_integer(value: Any) -> bool:
    return type(value) is int


def require_int(
    value: Any,
    *,
    label: str,
    minimum: int | None = None,
    maximum: int | None = None,
) -> int:
    if not is_json_integer(value):
        raise ArtifactInputError("integer", f"{label} must be an integer, never a boolean")
    if minimum is not None and value < minimum:
        raise ArtifactInputError("integer", f"{label} must be >= {minimum}")
    if maximum is not None and value > maximum:
        raise ArtifactInputError("integer", f"{label} must be <= {maximum}")
    return value


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ArtifactInputError("duplicate_key", "duplicate JSON key in object")
        value[key] = item
    return value


def _constant(_value: str) -> None:
    raise ArtifactInputError("nonfinite", "JSON numbers must be finite")


def _float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed):
        _constant(value)
    return parsed


def strict_json_loads(
    data: str | bytes,
    *,
    max_bytes: int = DEFAULT_MAX_BYTES,
    expected_type: type | None = None,
    label: str = "artifact",
) -> Any:
    try:
        if len(data) > max_bytes:
            raise ArtifactInputError("byte_limit", f"{label} exceeds the {max_bytes}-byte safety limit")
        raw = data.encode("utf-8", errors="strict") if isinstance(data, str) else data
        if len(raw) > max_bytes:
            raise ArtifactInputError("byte_limit", f"{label} exceeds the {max_bytes}-byte safety limit")
        text = raw.decode("utf-8", errors="strict")
    except UnicodeError as error:
        raise ArtifactInputError("utf8", f"{label} is not valid UTF-8") from error
    try:
        value = json.loads(
            text,
            object_pairs_hook=_unique_object,
            parse_constant=_constant,
            parse_float=_float,
        )
    except ArtifactInputError:
        raise
    except (ValueError, RecursionError) as error:
        raise ArtifactInputError("json", f"{label} is not one complete JSON value (malformed JSON or trailing data)") from error
    if expected_type is not None and type(value) is not expected_type:
        raise ArtifactInputError("type", f"{label} must contain a JSON {expected_type.__name__}")
    return value


def strict_json_load(
    path: Path,
    *,
    max_bytes: int = DEFAULT_MAX_BYTES,
    expected_type: type | None = None,
    label: str = "artifact",
) -> Any:
    return strict_json_loads(
        read_bounded(path, max_bytes=max_bytes, label=label),
        max_bytes=max_bytes,
        expected_type=expected_type,
        label=label,
    )


def strict_jsonl_load(
    path: Path,
    *,
    max_bytes: int = DEFAULT_MAX_BYTES,
    max_record_bytes: int = DEFAULT_MAX_RECORD_BYTES,
    max_records: int = DEFAULT_MAX_RECORDS,
    label: str = "artifact",
    allow_blank_lines: bool = True,
) -> list[dict[str, Any]]:
    require_int(max_bytes, label="byte limit", minimum=0)
    require_int(max_record_bytes, label="record byte limit", minimum=0)
    require_int(max_records, label="record count limit", minimum=0)
    records: list[dict[str, Any]] = []
    remaining = max_bytes
    try:
        if path.stat().st_size > max_bytes:
            raise ArtifactInputError("byte_limit", f"{label} exceeds the {max_bytes}-byte safety limit")
        with path.open("rb") as source:
            line_number = 0
            while True:
                line = source.readline(min(max_record_bytes, remaining) + 1)
                if not line:
                    break
                line_number += 1
                if len(line) > remaining:
                    raise ArtifactInputError("byte_limit", f"{label} exceeds the {max_bytes}-byte safety limit")
                remaining -= len(line)
                if len(line) > max_record_bytes:
                    raise ArtifactInputError("record_bytes", f"{label} line {line_number} exceeds {max_record_bytes} bytes")
                # Blank physical lines count against the record budget, too.
                if line_number > max_records:
                    raise ArtifactInputError("record_limit", f"{label} exceeds {max_records} records")
                if line.strip(b" \t\r\n"):
                    records.append(strict_json_loads(
                        line,
                        max_bytes=max_record_bytes,
                        expected_type=dict,
                        label=f"{label} line {line_number}",
                    ))
                elif not allow_blank_lines:
                    raise ArtifactInputError("json", f"{label} line {line_number} is blank")
    except OSError as error:
        raise ArtifactInputError("io", f"cannot read {label}") from error
    return records


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()
