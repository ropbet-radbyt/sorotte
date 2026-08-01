#!/usr/bin/env python3
"""Validate one source-pinned mpv version-matrix endpoint."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from typing import Sequence


MPV_SOURCE_SHA_RE = re.compile(r"[0-9a-f]{40}")
MPV_VERSION_RE = re.compile(
    r"(?:^|\s)v?(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)(?=-|\s|$)"
)


def parse_version_tuple(value: str, *, label: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", value)
    if match is None:
        raise ValueError(f"{label} must be an exact major.minor.patch tuple: {value!r}")
    return tuple(map(int, match.groups()))


def parse_mpv_version(first_line: str) -> tuple[int, int, int]:
    match = MPV_VERSION_RE.search(first_line)
    if match is None:
        raise ValueError(f"could not parse mpv version: {first_line}")
    return tuple(
        int(match.group(component)) for component in ("major", "minor", "patch")
    )


def validate_source_identity(
    *,
    identity: str,
    source_sha: str,
    minimum_source_sha: str,
    newest_source_sha: str,
) -> None:
    expected_sources = {
        "minimum": minimum_source_sha,
        "newest": newest_source_sha,
    }
    if identity not in expected_sources:
        raise ValueError(f"unknown mpv matrix identity: {identity}")
    for label, value in (
        ("selected", source_sha),
        ("minimum", minimum_source_sha),
        ("newest", newest_source_sha),
    ):
        if MPV_SOURCE_SHA_RE.fullmatch(value) is None:
            raise ValueError(f"{label} mpv source must be an exact lowercase SHA: {value}")
    if minimum_source_sha == newest_source_sha:
        raise ValueError("minimum and newest mpv matrix sources must be distinct")
    if source_sha != expected_sources[identity]:
        raise ValueError(f"mpv source identity drift for {identity}: {source_sha}")


def validate_observation(
    *,
    identity: str,
    source_sha: str,
    minimum_source_sha: str,
    newest_source_sha: str,
    minimum_version: str,
    first_line: str,
) -> dict[str, object]:
    validate_source_identity(
        identity=identity,
        source_sha=source_sha,
        minimum_source_sha=minimum_source_sha,
        newest_source_sha=newest_source_sha,
    )
    version = parse_mpv_version(first_line)
    minimum = parse_version_tuple(minimum_version, label="minimum mpv version")
    if version < minimum:
        raise ValueError(
            f"expected mpv {minimum_version} or newer, received: {first_line}"
        )
    if identity == "minimum" and version != minimum:
        raise ValueError(
            f"minimum mpv source no longer reports {minimum_version}: {first_line}"
        )
    return {
        "identity": identity,
        "kind": "sorotte-mpv-version-matrix-endpoint",
        "minimum_version": minimum_version,
        "schema_version": 1,
        "source_sha": source_sha,
        "version": ".".join(map(str, version)),
        "version_line": first_line,
    }


def inspect_binary(binary: str) -> str:
    completed = subprocess.run(
        [binary, "--version"],
        check=True,
        capture_output=True,
        text=True,
    )
    lines = completed.stdout.splitlines()
    if not lines:
        raise ValueError(f"mpv --version emitted no stdout: {binary}")
    return lines[0]


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    validate = subparsers.add_parser("validate")
    validate.add_argument("--identity", required=True)
    validate.add_argument("--source-sha", required=True)
    validate.add_argument("--minimum-source-sha", required=True)
    validate.add_argument("--newest-source-sha", required=True)
    validate.add_argument("--minimum-version", required=True)
    validate.add_argument("--binary", default="mpv")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        result = validate_observation(
            identity=args.identity,
            source_sha=args.source_sha,
            minimum_source_sha=args.minimum_source_sha,
            newest_source_sha=args.newest_source_sha,
            minimum_version=args.minimum_version,
            first_line=inspect_binary(args.binary),
        )
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        print(str(error), file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
