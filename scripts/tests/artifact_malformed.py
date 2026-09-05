"""Reusable single-boundary mutations of otherwise valid verification JSON."""

from __future__ import annotations

import json
import re
from collections.abc import Iterator


def malformed_json_cases(valid: bytes, *, integer_field: str = "schema_version") -> Iterator[tuple[str, bytes, str]]:
    key = json.dumps(integer_field).encode()
    pattern = re.escape(key) + rb'\s*:\s*(?:[0-9]+|"(?:\\.|[^"\\])*")'
    for name, number, category in (
        ("nan", b"NaN", "nonfinite"),
        ("infinity", b"Infinity", "nonfinite"),
        ("negative-infinity", b"-Infinity", "nonfinite"),
        ("overflowing-float", b"1e9999", "nonfinite"),
        ("boolean-integer", b"true", "schema|integer|version"),
    ):
        mutated, count = re.subn(pattern, key + b":" + number, valid, count=1)
        if count != 1:
            raise AssertionError(f"fixture lacks numeric {integer_field}")
        yield name, mutated, category
    duplicate = re.sub(pattern, lambda match: key + b":0," + match.group(), valid, count=1)
    yield "duplicate", duplicate, "duplicate_key"
    yield "invalid-utf8", b"\xff" + valid, "utf8"
    yield "trailing-garbage", valid.rstrip() + b" trailing-canary", "json"


def oversized_file(path, *, max_bytes: int) -> None:
    # Exercise admission without allocating the maximum in the test process.
    with path.open("wb") as output:
        output.seek(max_bytes)
        output.write(b"\n")
