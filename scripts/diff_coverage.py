#!/usr/bin/env python3
"""Fail-closed changed-line coverage policy for Rust source changes.

The required path intersects added lines from a Git unified diff with Sorotte's
source-bound LLVM physical-line map. A mapped line is authoritative when
present. A changed line absent from the map is exempt only when it is
conservatively recognizable as lexical structure, a comment, or whitespace;
every other missing line is unmapped and fails. This applies both to represented
source files and to wholly missing files, preventing cfg-gated code and omitted
crates from disappearing from the denominator.

Strict LCOV ingestion remains available as a diagnostic compatibility mode. It
continues to reject contradictory LF/LH summaries and is not used by the
required changed-line gate.

Obvious test-only Rust paths are reported but excluded from the production
percentage. The classification is path-based (tests/src/tests, tests.rs,
*_tests.rs, test_support.rs, benches, and examples). Complete inline
``#[cfg(test)] mod ... { ... }`` ranges are also reported and excluded using a
fail-closed lexical scanner that masks comments and Rust literals before
matching braces. Other cfg-gated items inside production files remain
production scope.

Production lines are evaluated as two disjoint classes. Ordinary production
uses ``--minimum``; paths declared by the repository-owned critical-path policy
use an independent, non-lowerable 90% ratchet. Policy files are strict TOML:
unknown fields, stale targets, overlaps, globs, and malformed paths fail closed.
Base/head runs validate the policy from each immutable revision against its own
tree and classify with their deduplicated union, preventing a change from
lowering its own critical denominator by deleting a rule.

Two input modes are supported:

* ``--diff PATCH`` consumes an explicit Git unified diff, rejects policy-file
  changes, and verifies every added/context line against the current source
  tree.
* ``--base REV --head REV`` resolves both revisions, requires the checkout to
  be the clean head revision for Rust sources, and obtains the diff from Git.

Malformed coverage, malformed/ambiguous patches, unsafe paths, stale source
bindings, missing instrumentation, and threshold failures all fail closed.
"""

from __future__ import annotations

import argparse
import bisect
import dataclasses
import decimal
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import tomllib
from collections.abc import Callable, Iterable, Mapping, Sequence
from typing import Any


SCHEMA_VERSION = 1
REPORT_KIND = "sorotte-diff-coverage"
MAX_LCOV_BYTES = 512 * 1024 * 1024
MAX_COVERAGE_MAP_BYTES = 128 * 1024 * 1024
MAX_COVERAGE_SOURCE_BYTES = 16 * 1024 * 1024
LLVM_LINE_MAP_SCHEMA_VERSION = 1
LLVM_LINE_MAP_KIND = "sorotte-llvm-line-map"
LLVM_LINE_MODEL = "unique-physical-source-lines"
LLVM_EXPORT_TYPE = "llvm.coverage.json.export"
LLVM_EXPORT_VERSION = "3.1.0"
CARGO_LLVM_COV_VERSION = "0.8.4"
MAX_DIFF_BYTES = 64 * 1024 * 1024
MAX_CRITICAL_POLICY_BYTES = 256 * 1024
DEFAULT_CRITICAL_POLICY_PATH = "coverage/diff-coverage-policy.toml"
CRITICAL_MINIMUM = decimal.Decimal("90.00")
CRITICAL_CATEGORIES = frozenset(
    {
        "authorization",
        "lifecycle",
        "persistence-arbitration",
        "privacy",
        "protocol-parsing",
        "updater-trust",
    }
)
POLICY_IDENTIFIER = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
HUNK_HEADER = re.compile(
    r"^@@ -(?P<old_start>[0-9]+)(?:,(?P<old_count>[0-9]+))?"
    r" \+(?P<new_start>[0-9]+)(?:,(?P<new_count>[0-9]+))? @@(?: .*)?$"
)
PERCENT = re.compile(r"^(?:100(?:\.0{1,2})?|(?:[0-9]|[1-9][0-9])(?:\.[0-9]{1,2})?)$")
PUNCTUATION_ONLY = re.compile(r"^[{}\[\](),;]+$")
FUNCTION_SIGNATURE_START = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:(?:async|const|unsafe)\s+)*"
    r"(?:extern(?:\s+\"[A-Za-z0-9_-]+\")?\s+)?"
    r"fn\s+[A-Za-z_][A-Za-z0-9_]*\b"
)
TYPE_BLOCK_START = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:unsafe\s+)?(?:struct|enum|union|trait|impl)\b"
)
IMPORT_START = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:use\b|extern\s+crate\b)"
)
RUST_CHAR_LITERAL = re.compile(
    r"""(?:b)?'(?:\\(?:[nrt0\\'"]|x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]{1,6}\})|[^\\'\r\n])'"""
)
EXACT_CFG_TEST_ATTRIBUTE = re.compile(
    r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]",
    re.DOTALL,
)
INLINE_MODULE_DECLARATION = re.compile(
    r"(?:(?:pub(?:\s*\([^)]*\))?)\s+)?mod\s+"
    r"(?P<name>(?:r#)?[A-Za-z_][A-Za-z0-9_]*)\b",
    re.DOTALL,
)


class DiffCoverageError(ValueError):
    """An invalid, unsafe, stale, or ambiguous coverage input."""


@dataclasses.dataclass(frozen=True)
class ChangedLine:
    number: int
    text: str


@dataclasses.dataclass(frozen=True)
class ChangedFile:
    old_path: str | None
    new_path: str | None
    change_kind: str
    added_lines: tuple[ChangedLine, ...]
    bound_new_lines: tuple[ChangedLine, ...]


@dataclasses.dataclass(frozen=True)
class SourceCoverage:
    path: str
    lines: Mapping[int, int]


@dataclasses.dataclass(frozen=True)
class DiffInput:
    text: str
    origin: str
    base_sha: str | None
    head_sha: str | None


@dataclasses.dataclass(frozen=True)
class CriticalPathRule:
    identifier: str
    category: str
    owner: str
    match_kind: str
    path: str
    policy_origins: tuple[str, ...] = ()

    def matches(self, candidate: str) -> bool:
        path = candidate.casefold() if os.name == "nt" else candidate
        configured = self.path.casefold() if os.name == "nt" else self.path
        if self.match_kind == "file":
            return path == configured
        return path.startswith(configured)

    def report(self) -> dict[str, Any]:
        return {
            "id": self.identifier,
            "category": self.category,
            "owner": self.owner,
            "match": self.match_kind,
            "path": self.path,
            "policy_origins": list(self.policy_origins),
        }


@dataclasses.dataclass(frozen=True)
class CriticalPolicyVersion:
    origin: str
    revision: str | None
    source: str
    present: bool
    sha256: str | None
    schema_version: int | None

    def report(self) -> dict[str, Any]:
        return {
            "origin": self.origin,
            "revision": self.revision,
            "source": self.source,
            "present": self.present,
            "sha256": self.sha256,
            "schema_version": self.schema_version,
        }


@dataclasses.dataclass(frozen=True)
class CriticalPathPolicy:
    source: str
    sha256: str
    schema_version: int
    minimum: decimal.Decimal
    rules: tuple[CriticalPathRule, ...]
    versions: tuple[CriticalPolicyVersion, ...]

    def match(self, candidate: str | None) -> CriticalPathRule | None:
        if (
            candidate is None
            or not candidate.endswith(".rs")
            or is_test_only_rust_path(candidate)
        ):
            return None
        matches = [rule for rule in self.rules if rule.matches(candidate)]
        if len(matches) > 1:
            # The loader rejects every theoretical overlap. Keep this defensive
            # boundary so a programmatically constructed policy still fails.
            raise DiffCoverageError(
                f"critical path {candidate!r} matches multiple policy rules"
            )
        return matches[0] if matches else None


def sha256_bytes(value: bytes) -> str:
    return f"sha256:{hashlib.sha256(value).hexdigest()}"


def read_bounded(path: pathlib.Path, *, limit: int, description: str) -> bytes:
    try:
        size = path.stat().st_size
    except OSError as error:
        raise DiffCoverageError(f"cannot stat {description} {path}: {error}") from error
    if size > limit:
        raise DiffCoverageError(
            f"{description} exceeds the {limit}-byte safety limit: {size} bytes"
        )
    try:
        return path.read_bytes()
    except OSError as error:
        raise DiffCoverageError(f"cannot read {description} {path}: {error}") from error


def decode_utf8(value: bytes, *, description: str) -> str:
    try:
        text = value.decode("utf-8")
    except UnicodeDecodeError as error:
        raise DiffCoverageError(f"{description} is not valid UTF-8: {error}") from error
    if "\x00" in text:
        raise DiffCoverageError(f"{description} contains a NUL byte")
    return text


def repository_relative_path(
    raw: str,
    *,
    context: str,
    allow_prefix: str | None = None,
) -> str:
    if not raw or raw != raw.strip() or "\x00" in raw or "\\" in raw:
        raise DiffCoverageError(
            f"{context} must be a non-empty normalized POSIX repository path"
        )
    if allow_prefix is not None:
        expected = f"{allow_prefix}/"
        if not raw.startswith(expected):
            raise DiffCoverageError(f"{context} must begin with {expected!r}: {raw!r}")
        raw = raw[len(expected) :]
    path = pathlib.PurePosixPath(raw)
    if path.is_absolute() or not path.parts or any(part in {"", ".", ".."} for part in path.parts):
        raise DiffCoverageError(
            f"{context} must be a normalized repository-relative path: {raw!r}"
        )
    normalized = path.as_posix()
    if normalized != raw:
        raise DiffCoverageError(f"{context} is not normalized: {raw!r}")
    return normalized


def decode_git_quoted_path(value: str, *, context: str) -> str:
    """Decode Git's double-quoted path format, including octal UTF-8 bytes."""

    if not value.startswith('"'):
        if "\t" in value or "\r" in value or "\n" in value:
            raise DiffCoverageError(f"{context} contains an ambiguous path separator")
        return value
    if len(value) < 2 or not value.endswith('"'):
        raise DiffCoverageError(f"{context} has an unterminated quoted path")

    encoded = bytearray()
    index = 1
    end = len(value) - 1
    escapes = {
        "\\": ord("\\"),
        '"': ord('"'),
        "a": 7,
        "b": 8,
        "t": 9,
        "n": 10,
        "v": 11,
        "f": 12,
        "r": 13,
    }
    while index < end:
        character = value[index]
        if character != "\\":
            encoded.extend(character.encode("utf-8"))
            index += 1
            continue
        index += 1
        if index >= end:
            raise DiffCoverageError(f"{context} ends with an incomplete escape")
        escaped = value[index]
        if escaped in escapes:
            encoded.append(escapes[escaped])
            index += 1
            continue
        if escaped in "01234567":
            digits = escaped
            index += 1
            while index < end and len(digits) < 3 and value[index] in "01234567":
                digits += value[index]
                index += 1
            encoded.append(int(digits, 8))
            continue
        raise DiffCoverageError(f"{context} contains unsupported escape \\{escaped}")
    try:
        return encoded.decode("utf-8")
    except UnicodeDecodeError as error:
        raise DiffCoverageError(
            f"{context} quoted bytes are not valid UTF-8: {error}"
        ) from error


def split_git_header_paths(value: str) -> tuple[str, str]:
    """Split the two path tokens in a ``diff --git`` header."""

    tokens: list[str] = []
    index = 0
    while index < len(value):
        while index < len(value) and value[index] == " ":
            index += 1
        if index >= len(value):
            break
        start = index
        if value[index] == '"':
            index += 1
            escaped = False
            while index < len(value):
                character = value[index]
                if character == '"' and not escaped:
                    index += 1
                    break
                if character == "\\" and not escaped:
                    escaped = True
                else:
                    escaped = False
                index += 1
            else:
                raise DiffCoverageError("diff --git header has an unterminated path")
        else:
            while index < len(value) and value[index] != " ":
                index += 1
        tokens.append(value[start:index])
    if len(tokens) != 2:
        raise DiffCoverageError(
            "diff --git header must contain exactly two unambiguous path tokens"
        )
    return (
        decode_git_quoted_path(tokens[0], context="diff --git old path"),
        decode_git_quoted_path(tokens[1], context="diff --git new path"),
    )


def parse_header_path(line: str, *, marker: str, context: str) -> str | None:
    prefix = f"{marker} "
    if not line.startswith(prefix):
        raise DiffCoverageError(f"{context} must start with {prefix!r}")
    value = decode_git_quoted_path(line[len(prefix) :], context=context)
    if value == "/dev/null":
        return None
    return repository_relative_path(
        value,
        context=context,
        allow_prefix="a" if marker == "---" else "b",
    )


def parse_rename_path(line: str, *, marker: str) -> str:
    prefix = f"{marker} "
    value = decode_git_quoted_path(line[len(prefix) :], context=marker)
    return repository_relative_path(value, context=marker)


def parse_nonnegative_int(value: str, *, context: str) -> int:
    if not value or not value.isascii() or not value.isdigit():
        raise DiffCoverageError(f"{context} must be a non-negative decimal integer")
    if len(value) > 20:
        raise DiffCoverageError(f"{context} exceeds the supported 20-digit range")
    return int(value)


def parse_hunks(
    lines: Sequence[str],
    *,
    start_index: int,
    context: str,
) -> tuple[tuple[ChangedLine, ...], tuple[ChangedLine, ...]]:
    added: list[ChangedLine] = []
    bound: list[ChangedLine] = []
    seen_new_lines: set[int] = set()
    previous_old_end = -1
    previous_new_end = -1
    index = start_index

    while index < len(lines):
        header = lines[index]
        match = HUNK_HEADER.fullmatch(header)
        if match is None:
            raise DiffCoverageError(f"{context} has content outside a valid hunk: {header!r}")
        old_start = parse_nonnegative_int(match["old_start"], context="old hunk start")
        old_count = (
            parse_nonnegative_int(match["old_count"], context="old hunk count")
            if match["old_count"] is not None
            else 1
        )
        new_start = parse_nonnegative_int(match["new_start"], context="new hunk start")
        new_count = (
            parse_nonnegative_int(match["new_count"], context="new hunk count")
            if match["new_count"] is not None
            else 1
        )
        if (old_count > 0 and old_start == 0) or (new_count > 0 and new_start == 0):
            raise DiffCoverageError(f"{context} has a positive hunk count starting at line zero")
        if old_start < previous_old_end or new_start < previous_new_end:
            raise DiffCoverageError(f"{context} has overlapping or out-of-order hunks")

        old_line = old_start
        new_line = new_start
        consumed_old = 0
        consumed_new = 0
        index += 1
        previous_was_content = False
        while index < len(lines) and not lines[index].startswith("@@ "):
            line = lines[index]
            if line == r"\ No newline at end of file":
                if not previous_was_content:
                    raise DiffCoverageError(
                        f"{context} has a misplaced no-newline marker"
                    )
                previous_was_content = False
                index += 1
                continue
            if not line:
                raise DiffCoverageError(
                    f"{context} has a hunk line without a unified-diff prefix"
                )
            marker = line[0]
            text = line[1:]
            previous_was_content = True
            if marker == " ":
                consumed_old += 1
                consumed_new += 1
                bound.append(ChangedLine(new_line, text))
                old_line += 1
                new_line += 1
            elif marker == "-":
                consumed_old += 1
                old_line += 1
            elif marker == "+":
                consumed_new += 1
                if new_line in seen_new_lines:
                    raise DiffCoverageError(
                        f"{context} adds new line {new_line} more than once"
                    )
                changed = ChangedLine(new_line, text)
                added.append(changed)
                bound.append(changed)
                seen_new_lines.add(new_line)
                new_line += 1
            else:
                raise DiffCoverageError(
                    f"{context} has invalid hunk prefix {marker!r}"
                )
            if consumed_old > old_count or consumed_new > new_count:
                raise DiffCoverageError(f"{context} hunk body exceeds its declared counts")
            index += 1

        if consumed_old != old_count or consumed_new != new_count:
            raise DiffCoverageError(
                f"{context} hunk count mismatch: declared -{old_count}/+{new_count}, "
                f"observed -{consumed_old}/+{consumed_new}"
            )
        previous_old_end = old_start + old_count
        previous_new_end = new_start + new_count

    return tuple(added), tuple(bound)


def parse_diff_segment(lines: Sequence[str], *, ordinal: int) -> ChangedFile:
    context = f"diff file {ordinal}"
    if not lines or not lines[0].startswith("diff --git "):
        raise DiffCoverageError(f"{context} is missing a diff --git header")
    header_old_raw, header_new_raw = split_git_header_paths(
        lines[0][len("diff --git ") :]
    )
    header_old = repository_relative_path(
        header_old_raw, context=f"{context} header old path", allow_prefix="a"
    )
    header_new = repository_relative_path(
        header_new_raw, context=f"{context} header new path", allow_prefix="b"
    )

    hunk_indices = [index for index, line in enumerate(lines) if line.startswith("@@ ")]
    preamble_end = hunk_indices[0] if hunk_indices else len(lines)
    preamble = lines[1:preamble_end]
    old_headers = [line for line in preamble if line.startswith("--- ")]
    new_headers = [line for line in preamble if line.startswith("+++ ")]
    if len(old_headers) != len(new_headers) or len(old_headers) > 1:
        raise DiffCoverageError(f"{context} must contain zero or one ---/+++ header pair")
    old_path = (
        parse_header_path(old_headers[0], marker="---", context=f"{context} old path")
        if old_headers
        else header_old
    )
    new_path = (
        parse_header_path(new_headers[0], marker="+++", context=f"{context} new path")
        if new_headers
        else header_new
    )
    if old_headers:
        old_header_index = preamble.index(old_headers[0])
        new_header_index = preamble.index(new_headers[0])
        if new_header_index != old_header_index + 1:
            raise DiffCoverageError(
                f"{context} must place +++ immediately after its --- content header"
            )
    if old_path is not None and old_path != header_old:
        raise DiffCoverageError(f"{context} old content path disagrees with its header")
    if new_path is not None and new_path != header_new:
        raise DiffCoverageError(f"{context} new content path disagrees with its header")

    rename_from_lines = [line for line in preamble if line.startswith("rename from ")]
    rename_to_lines = [line for line in preamble if line.startswith("rename to ")]
    if len(rename_from_lines) != len(rename_to_lines) or len(rename_from_lines) > 1:
        raise DiffCoverageError(f"{context} has incomplete or duplicate rename metadata")
    renamed = bool(rename_from_lines)
    if renamed:
        rename_from = parse_rename_path(rename_from_lines[0], marker="rename from")
        rename_to = parse_rename_path(rename_to_lines[0], marker="rename to")
        if rename_from != header_old or rename_to != header_new:
            raise DiffCoverageError(f"{context} rename metadata disagrees with its header")
        if old_path != rename_from or new_path != rename_to:
            raise DiffCoverageError(f"{context} rename paths disagree with content headers")

    copy_lines = [
        line for line in preamble if line.startswith(("copy from ", "copy to "))
    ]
    if copy_lines:
        raise DiffCoverageError(
            f"{context} uses copy metadata, which is intentionally unsupported; "
            "regenerate without copy detection so new Rust lines are explicit"
        )

    new_file = any(line.startswith("new file mode ") for line in preamble)
    deleted_file = any(line.startswith("deleted file mode ") for line in preamble)
    if new_file and deleted_file:
        raise DiffCoverageError(f"{context} cannot be both a new and deleted file")
    if new_file and old_headers and old_path is not None:
        raise DiffCoverageError(f"{context} new file must use /dev/null as its old path")
    if deleted_file and new_headers and new_path is not None:
        raise DiffCoverageError(f"{context} deleted file must use /dev/null as its new path")
    if new_file and not old_headers:
        old_path = None
    if deleted_file and not new_headers:
        new_path = None
    if (new_file or deleted_file) and header_old != header_new:
        raise DiffCoverageError(
            f"{context} new/deleted file header paths must identify the same file"
        )
    if (
        not new_file
        and not deleted_file
        and not renamed
        and header_old != header_new
    ):
        raise DiffCoverageError(
            f"{context} changes paths without explicit rename metadata"
        )

    binary = any(
        line == "GIT binary patch" or line.startswith("Binary files ")
        for line in preamble
    )
    rust_target = new_path is not None and new_path.endswith(".rs")
    if binary:
        if rust_target:
            raise DiffCoverageError(f"{context} reports binary content for a Rust source")
        return ChangedFile(old_path, new_path, "binary", (), ())

    if hunk_indices:
        first_hunk = hunk_indices[0]
        if not old_headers:
            raise DiffCoverageError(f"{context} has hunks without ---/+++ content headers")
        allowed_metadata_prefixes = (
            "old mode ",
            "new mode ",
            "new file mode ",
            "deleted file mode ",
            "similarity index ",
            "dissimilarity index ",
            "rename from ",
            "rename to ",
            "index ",
            "--- ",
            "+++ ",
        )
        for line in lines[1:first_hunk]:
            if not line.startswith(allowed_metadata_prefixes):
                raise DiffCoverageError(f"{context} has unsupported metadata: {line!r}")
        added, bound = parse_hunks(lines, start_index=first_hunk, context=context)
    else:
        allowed_no_hunk_prefixes = (
            "old mode ",
            "new mode ",
            "new file mode ",
            "deleted file mode ",
            "similarity index ",
            "dissimilarity index ",
            "rename from ",
            "rename to ",
            "index ",
        )
        for line in lines[1:]:
            if not line.startswith(allowed_no_hunk_prefixes):
                raise DiffCoverageError(f"{context} has unsupported metadata: {line!r}")
        added, bound = (), ()

    if new_path is None:
        change_kind = "deleted"
    elif old_path is None or new_file:
        change_kind = "added"
    elif renamed:
        change_kind = "renamed"
    else:
        change_kind = "modified"
    return ChangedFile(old_path, new_path, change_kind, added, bound)


def parse_unified_diff(text: str) -> list[ChangedFile]:
    if "\x00" in text:
        raise DiffCoverageError("unified diff contains a NUL byte")
    lines = text.splitlines()
    if not lines:
        return []
    if lines[0].startswith(("--- ", "@@ ", "Index: ")):
        raise DiffCoverageError("only Git unified diffs with diff --git headers are accepted")
    starts = [index for index, line in enumerate(lines) if line.startswith("diff --git ")]
    if not starts or starts[0] != 0:
        raise DiffCoverageError("unified diff has content before its first diff --git header")
    starts.append(len(lines))
    files = [
        parse_diff_segment(lines[starts[index] : starts[index + 1]], ordinal=index + 1)
        for index in range(len(starts) - 1)
    ]
    seen: set[str] = set()
    for changed_file in files:
        identity = changed_file.new_path or changed_file.old_path
        if identity is None:
            raise DiffCoverageError("diff file has neither an old nor a new path")
        key = identity.casefold() if os.name == "nt" else identity
        if key in seen:
            raise DiffCoverageError(f"unified diff contains duplicate target path {identity!r}")
        seen.add(key)
    return files


def is_test_only_rust_path(path: str) -> bool:
    """Classify only conventional, unambiguous test/benchmark/example paths."""

    pure = pathlib.PurePosixPath(path)
    parts = tuple(part.casefold() for part in pure.parts)
    if any(part in {"tests", "benches", "examples"} for part in parts[:-1]):
        return True
    name = parts[-1]
    return (
        name == "tests.rs"
        or name.endswith("_tests.rs")
        or name == "test_support.rs"
    )


def require_exact_toml_keys(
    value: object,
    expected: set[str],
    *,
    context: str,
) -> Mapping[str, Any]:
    if not isinstance(value, dict):
        raise DiffCoverageError(f"{context} must be a TOML table")
    actual = set(value)
    missing = sorted(expected - actual)
    unexpected = sorted(actual - expected)
    if missing or unexpected:
        details: list[str] = []
        if missing:
            details.append(f"missing {missing}")
        if unexpected:
            details.append(f"unexpected {unexpected}")
        raise DiffCoverageError(f"{context} has invalid keys: {', '.join(details)}")
    return value


def validate_critical_rule_target(
    rule: CriticalPathRule,
    *,
    repo_root: pathlib.Path,
) -> None:
    target_text = rule.path[:-1] if rule.match_kind == "directory" else rule.path
    target = (
        repo_root.joinpath(*pathlib.PurePosixPath(target_text).parts).resolve()
    )
    try:
        target.relative_to(repo_root)
    except ValueError as error:
        raise DiffCoverageError(
            f"critical path rule {rule.identifier!r} escapes the repository"
        ) from error

    if rule.match_kind == "file":
        if not target.is_file():
            raise DiffCoverageError(
                f"critical path rule {rule.identifier!r} target does not exist "
                f"as a file: {rule.path}"
            )
        return
    if not target.is_dir():
        raise DiffCoverageError(
            f"critical path rule {rule.identifier!r} target does not exist "
            f"as a directory: {rule.path}"
        )

    has_production_rust = False
    try:
        candidates = target.rglob("*.rs")
        for candidate in candidates:
            resolved = candidate.resolve()
            try:
                relative = resolved.relative_to(repo_root).as_posix()
            except ValueError as error:
                raise DiffCoverageError(
                    f"critical path rule {rule.identifier!r} contains a Rust "
                    "source that escapes the repository"
                ) from error
            if candidate.is_file() and not is_test_only_rust_path(relative):
                has_production_rust = True
                break
    except OSError as error:
        raise DiffCoverageError(
            f"cannot inspect critical path rule {rule.identifier!r}: {error}"
        ) from error
    if not has_production_rust:
        raise DiffCoverageError(
            f"critical path rule {rule.identifier!r} directory contains no "
            f"production Rust source: {rule.path}"
        )


def git_tree_entries(
    repo_root: pathlib.Path,
    revision: str,
    path: str,
) -> list[tuple[str, str, str, str]]:
    """Return exact raw Git-tree entries as (mode, type, object, path)."""

    output = run_git(
        repo_root,
        [
            "-c",
            "core.quotePath=false",
            "ls-tree",
            "-r",
            "-z",
            "--full-tree",
            revision,
            "--",
            path,
        ],
        description=f"inspect {path} in critical-policy revision {revision}",
    )
    entries: list[tuple[str, str, str, str]] = []
    for record in output.split("\x00"):
        if not record:
            continue
        try:
            metadata, entry_path = record.split("\t", 1)
            mode, object_type, object_id = metadata.split(" ", 2)
        except ValueError as error:
            raise DiffCoverageError(
                f"Git returned malformed tree metadata for {path!r} at {revision}"
            ) from error
        if (
            not mode
            or object_type not in {"blob", "commit"}
            or not re.fullmatch(r"[0-9a-f]{40,64}", object_id)
            or not entry_path
        ):
            raise DiffCoverageError(
                f"Git returned invalid tree metadata for {path!r} at {revision}"
            )
        entries.append((mode, object_type, object_id, entry_path))
    return entries


def validate_critical_rule_target_at_revision(
    rule: CriticalPathRule,
    *,
    repo_root: pathlib.Path,
    revision: str,
) -> None:
    """Validate a rule against its own immutable Git tree, not the checkout."""

    target_text = rule.path[:-1] if rule.match_kind == "directory" else rule.path
    entries = git_tree_entries(repo_root, revision, target_text)
    if rule.match_kind == "file":
        exact = [entry for entry in entries if entry[3] == target_text]
        if (
            len(exact) != 1
            or exact[0][1] != "blob"
            or exact[0][0] not in {"100644", "100755"}
        ):
            raise DiffCoverageError(
                f"critical path rule {rule.identifier!r} target does not exist "
                f"as a regular file at {revision}: {rule.path}"
            )
        return

    prefix = f"{target_text}/"
    descendants = [entry for entry in entries if entry[3].startswith(prefix)]
    if not descendants:
        raise DiffCoverageError(
            f"critical path rule {rule.identifier!r} target does not exist "
            f"as a directory at {revision}: {rule.path}"
        )
    for mode, object_type, _object_id, candidate in descendants:
        if not candidate.endswith(".rs"):
            continue
        if mode == "120000":
            raise DiffCoverageError(
                f"critical path rule {rule.identifier!r} contains a symlinked "
                f"Rust source at {revision}: {candidate}"
            )
        if (
            object_type == "blob"
            and mode in {"100644", "100755"}
            and not is_test_only_rust_path(candidate)
        ):
            return
    raise DiffCoverageError(
        f"critical path rule {rule.identifier!r} directory contains no "
        f"production Rust source at {revision}: {rule.path}"
    )


def critical_rules_overlap(
    first: CriticalPathRule,
    second: CriticalPathRule,
) -> bool:
    first_path = first.path.casefold()
    second_path = second.path.casefold()
    if first.match_kind == "file" and second.match_kind == "file":
        return first_path == second_path
    if first.match_kind == "directory" and second.match_kind == "directory":
        return first_path.startswith(second_path) or second_path.startswith(first_path)
    directory = first if first.match_kind == "directory" else second
    file_rule = second if first.match_kind == "directory" else first
    return file_rule.path.casefold().startswith(directory.path.casefold())


def parse_critical_rule(
    raw_rule: object,
    *,
    ordinal: int,
    origin: str,
    target_validator: Callable[[CriticalPathRule], None],
) -> CriticalPathRule:
    rule = require_exact_toml_keys(
        raw_rule,
        {"id", "category", "owner", "match", "path"},
        context=f"critical_path[{ordinal}]",
    )
    for key in ("id", "category", "owner", "match", "path"):
        if not isinstance(rule[key], str):
            raise DiffCoverageError(
                f"critical_path[{ordinal}].{key} must be a string"
            )

    identifier = rule["id"]
    category = rule["category"]
    owner = rule["owner"]
    match_kind = rule["match"]
    raw_path = rule["path"]
    if not POLICY_IDENTIFIER.fullmatch(identifier):
        raise DiffCoverageError(
            f"critical_path[{ordinal}].id must be a normalized policy identifier"
        )
    if category not in CRITICAL_CATEGORIES:
        raise DiffCoverageError(
            f"critical_path[{ordinal}].category must be one of "
            f"{sorted(CRITICAL_CATEGORIES)}"
        )
    if not POLICY_IDENTIFIER.fullmatch(owner):
        raise DiffCoverageError(
            f"critical_path[{ordinal}].owner must be a normalized policy identifier"
        )
    if match_kind not in {"file", "directory"}:
        raise DiffCoverageError(
            f"critical_path[{ordinal}].match must be 'file' or 'directory'"
        )
    if any(character in raw_path for character in "*?[]{}"):
        raise DiffCoverageError(
            f"critical_path[{ordinal}].path cannot contain glob syntax"
        )

    if match_kind == "directory":
        if not raw_path.endswith("/") or raw_path.endswith("//"):
            raise DiffCoverageError(
                f"critical_path[{ordinal}].path must end with exactly one '/' "
                "for a directory rule"
            )
        normalized = repository_relative_path(
            raw_path[:-1],
            context=f"critical_path[{ordinal}].path",
        )
        configured_path = f"{normalized}/"
    else:
        normalized = repository_relative_path(
            raw_path,
            context=f"critical_path[{ordinal}].path",
        )
        if not normalized.endswith(".rs"):
            raise DiffCoverageError(
                f"critical_path[{ordinal}].path must name a Rust source file"
            )
        if is_test_only_rust_path(normalized):
            raise DiffCoverageError(
                f"critical_path[{ordinal}].path cannot name a test-only Rust path"
            )
        configured_path = normalized
    if not configured_path.startswith("crates/"):
        raise DiffCoverageError(
            f"critical_path[{ordinal}].path must be inside crates/"
        )

    parsed = CriticalPathRule(
        identifier=identifier,
        category=category,
        owner=owner,
        match_kind=match_kind,
        path=configured_path,
        policy_origins=(origin,),
    )
    target_validator(parsed)
    return parsed


def resolve_critical_policy_source(
    *,
    repo_root: pathlib.Path,
    policy_path: pathlib.Path | None,
) -> tuple[str, pathlib.Path]:
    requested = (
        pathlib.Path(DEFAULT_CRITICAL_POLICY_PATH)
        if policy_path is None
        else policy_path
    )
    candidate = requested if requested.is_absolute() else repo_root / requested
    resolved = candidate.resolve()
    try:
        source = resolved.relative_to(repo_root).as_posix()
    except ValueError as error:
        raise DiffCoverageError(
            f"critical path policy must be inside the repository: {requested}"
        ) from error
    return source, resolved


def parse_critical_path_policy(
    raw: bytes,
    *,
    source: str,
    origin: str,
    revision: str | None,
    target_validator: Callable[[CriticalPathRule], None],
) -> CriticalPathPolicy:
    if len(raw) > MAX_CRITICAL_POLICY_BYTES:
        raise DiffCoverageError(
            "critical path policy exceeds the "
            f"{MAX_CRITICAL_POLICY_BYTES}-byte safety limit"
        )
    text = decode_utf8(raw, description="critical path policy")
    try:
        parsed = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise DiffCoverageError(
            f"critical path policy is not valid TOML: {error}"
        ) from error

    root = require_exact_toml_keys(
        parsed,
        {"schema_version", "policy", "critical_path"},
        context="critical path policy",
    )
    schema_version = root["schema_version"]
    if (
        isinstance(schema_version, bool)
        or not isinstance(schema_version, int)
        or schema_version != 1
    ):
        raise DiffCoverageError(
            f"unsupported critical path policy schema {schema_version!r}"
        )
    policy = require_exact_toml_keys(
        root["policy"],
        {"critical_minimum_percent"},
        context="critical path policy.policy",
    )
    minimum_text = policy["critical_minimum_percent"]
    if not isinstance(minimum_text, str) or minimum_text != "90.00":
        raise DiffCoverageError(
            "critical path policy critical_minimum_percent must be exactly "
            f"{CRITICAL_MINIMUM:.2f}"
        )
    raw_rules = root["critical_path"]
    if not isinstance(raw_rules, list) or not raw_rules:
        raise DiffCoverageError(
            "critical path policy.critical_path must be a non-empty array of tables"
        )
    if len(raw_rules) > 256:
        raise DiffCoverageError(
            "critical path policy has more than the supported 256 rules"
        )
    rules = tuple(
        parse_critical_rule(
            item,
            ordinal=index,
            origin=origin,
            target_validator=target_validator,
        )
        for index, item in enumerate(raw_rules)
    )
    identifiers: set[str] = set()
    for rule in rules:
        if rule.identifier in identifiers:
            raise DiffCoverageError(
                f"critical path policy duplicates rule id {rule.identifier!r}"
            )
        identifiers.add(rule.identifier)
    for index, first in enumerate(rules):
        for second in rules[index + 1 :]:
            if critical_rules_overlap(first, second):
                raise DiffCoverageError(
                    "critical path policy rules overlap: "
                    f"{first.identifier!r} ({first.path}) and "
                    f"{second.identifier!r} ({second.path})"
                )
    return CriticalPathPolicy(
        source=source,
        sha256=sha256_bytes(raw),
        schema_version=schema_version,
        minimum=CRITICAL_MINIMUM,
        rules=rules,
        versions=(
            CriticalPolicyVersion(
                origin=origin,
                revision=revision,
                source=source,
                present=True,
                sha256=sha256_bytes(raw),
                schema_version=schema_version,
            ),
        ),
    )


def load_critical_path_policy(
    *,
    repo_root: pathlib.Path,
    policy_path: pathlib.Path | None,
    origin: str = "head",
    revision: str | None = None,
) -> CriticalPathPolicy:
    source, resolved = resolve_critical_policy_source(
        repo_root=repo_root,
        policy_path=policy_path,
    )
    raw = read_bounded(
        resolved,
        limit=MAX_CRITICAL_POLICY_BYTES,
        description="critical path policy",
    )
    return parse_critical_path_policy(
        raw,
        source=source,
        origin=origin,
        revision=revision,
        target_validator=lambda rule: validate_critical_rule_target(
            rule,
            repo_root=repo_root,
        ),
    )


def load_critical_path_policy_at_revision(
    *,
    repo_root: pathlib.Path,
    source: str,
    revision: str,
    origin: str,
) -> tuple[CriticalPathPolicy | None, CriticalPolicyVersion]:
    entries = git_tree_entries(repo_root, revision, source)
    exact = [entry for entry in entries if entry[3] == source]
    if not exact:
        return (
            None,
            CriticalPolicyVersion(
                origin=origin,
                revision=revision,
                source=source,
                present=False,
                sha256=None,
                schema_version=None,
            ),
        )
    if (
        len(exact) != 1
        or exact[0][1] != "blob"
        or exact[0][0] not in {"100644", "100755"}
    ):
        raise DiffCoverageError(
            f"critical path policy is not a regular file at {revision}: {source}"
        )
    raw = run_git_bytes(
        repo_root,
        ["cat-file", "blob", f"{revision}:{source}"],
        description=f"read critical path policy {source} at {revision}",
    )
    policy = parse_critical_path_policy(
        raw,
        source=source,
        origin=origin,
        revision=revision,
        target_validator=lambda rule: validate_critical_rule_target_at_revision(
            rule,
            repo_root=repo_root,
            revision=revision,
        ),
    )
    return policy, policy.versions[0]


def critical_rule_identity(rule: CriticalPathRule) -> tuple[str, str, str, str, str]:
    return (
        rule.identifier,
        rule.category,
        rule.owner,
        rule.match_kind,
        rule.path,
    )


def merge_critical_path_policies(
    head_policy: CriticalPathPolicy,
    *,
    base_policy: CriticalPathPolicy | None,
    base_version: CriticalPolicyVersion,
) -> CriticalPathPolicy:
    """Use the base/head union so a change cannot weaken its own classification."""

    candidates = [
        *(base_policy.rules if base_policy is not None else ()),
        *head_policy.rules,
    ]
    merged_by_identity: dict[
        tuple[str, str, str, str, str],
        CriticalPathRule,
    ] = {}
    for rule in candidates:
        identity = critical_rule_identity(rule)
        existing = merged_by_identity.get(identity)
        if existing is None:
            merged_by_identity[identity] = rule
            continue
        origins = tuple(
            origin
            for origin in ("base", "head")
            if origin in {*existing.policy_origins, *rule.policy_origins}
        )
        merged_by_identity[identity] = dataclasses.replace(
            existing,
            policy_origins=origins,
        )

    rules = tuple(
        sorted(
            merged_by_identity.values(),
            key=lambda rule: (
                rule.path.casefold(),
                rule.match_kind,
                rule.identifier,
                rule.category,
                rule.owner,
            ),
        )
    )
    for index, first in enumerate(rules):
        for second in rules[index + 1 :]:
            if critical_rules_overlap(first, second):
                raise DiffCoverageError(
                    "base/head critical path policy union is ambiguous: "
                    f"{first.identifier!r} ({first.path}, "
                    f"origins={list(first.policy_origins)!r}) overlaps "
                    f"{second.identifier!r} ({second.path}, "
                    f"origins={list(second.policy_origins)!r})"
                )

    return CriticalPathPolicy(
        source=head_policy.source,
        sha256=head_policy.sha256,
        schema_version=head_policy.schema_version,
        minimum=head_policy.minimum,
        rules=rules,
        versions=(base_version, *head_policy.versions),
    )


def materialize_implicit_rust_additions(
    changed_files: Sequence[ChangedFile],
    *,
    repo_root: pathlib.Path,
    critical_policy: CriticalPathPolicy,
) -> list[ChangedFile]:
    """Materialize no-hunk introductions such as a non-Rust-to-Rust rename.

    Normal Git new-file diffs contain hunks. Empty files and a pure rename from
    another extension do not. Treating the latter as a zero-line Rust change
    would create a coverage bypass, so every target source line becomes added.
    """

    result: list[ChangedFile] = []
    for changed_file in changed_files:
        old_critical = critical_policy.match(changed_file.old_path)
        new_critical = critical_policy.match(changed_file.new_path)
        crosses_critical_boundary = (old_critical is None) != (
            new_critical is None
        )
        introduced = (
            changed_file.new_path is not None
            and changed_file.new_path.endswith(".rs")
            and (
                changed_file.old_path is None
                or not changed_file.old_path.endswith(".rs")
                or (
                    is_test_only_rust_path(changed_file.old_path)
                    and not is_test_only_rust_path(changed_file.new_path)
                )
                or crosses_critical_boundary
            )
            and not changed_file.added_lines
        )
        if not introduced:
            result.append(changed_file)
            continue
        path = repo_root.joinpath(*pathlib.PurePosixPath(changed_file.new_path).parts)
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError) as error:
            raise DiffCoverageError(
                f"cannot read introduced Rust source {changed_file.new_path}: {error}"
            ) from error
        additions = tuple(
            ChangedLine(number, text) for number, text in enumerate(lines, start=1)
        )
        result.append(
            dataclasses.replace(
                changed_file,
                added_lines=additions,
                bound_new_lines=additions,
            )
        )
    return result


def validate_source_bindings(
    changed_files: Iterable[ChangedFile], *, repo_root: pathlib.Path
) -> None:
    root = repo_root.resolve()
    for changed_file in changed_files:
        if (
            changed_file.new_path is None
            or not changed_file.new_path.endswith(".rs")
        ):
            continue
        path = (root / pathlib.Path(*pathlib.PurePosixPath(changed_file.new_path).parts)).resolve()
        try:
            path.relative_to(root)
        except ValueError as error:
            raise DiffCoverageError(
                f"changed source escapes the repository: {changed_file.new_path}"
            ) from error
        if not path.is_file():
            raise DiffCoverageError(
                f"changed Rust source does not exist at the diff target: {changed_file.new_path}"
            )
        if not changed_file.bound_new_lines:
            continue
        try:
            source_lines = path.read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError) as error:
            raise DiffCoverageError(
                f"cannot read changed Rust source {changed_file.new_path}: {error}"
            ) from error
        for binding in changed_file.bound_new_lines:
            if binding.number < 1 or binding.number > len(source_lines):
                raise DiffCoverageError(
                    f"diff line {changed_file.new_path}:{binding.number} is outside "
                    f"the {len(source_lines)}-line current source"
                )
            actual = source_lines[binding.number - 1]
            if actual != binding.text:
                raise DiffCoverageError(
                    f"diff does not match current source at "
                    f"{changed_file.new_path}:{binding.number}"
                )


def resolve_lcov_source(raw: str, *, repo_root: pathlib.Path) -> str | None:
    if not raw or raw != raw.strip() or "\x00" in raw:
        raise DiffCoverageError("LCOV SF must contain a non-empty trimmed path")
    root = repo_root.resolve()
    candidate_raw = raw.replace("\\", os.sep)
    candidate = pathlib.Path(candidate_raw)
    if not candidate.is_absolute():
        candidate = root / candidate
    resolved = candidate.resolve()
    try:
        relative = resolved.relative_to(root)
    except ValueError:
        return None
    if not resolved.is_file():
        raise DiffCoverageError(f"LCOV source inside the repository does not exist: {raw}")
    return relative.as_posix()


def parse_lcov_record(
    directives: Sequence[tuple[str, str, int]],
    *,
    repo_root: pathlib.Path,
) -> SourceCoverage | None:
    source_values = [
        (value, line_number)
        for key, value, line_number in directives
        if key == "SF"
    ]
    if len(source_values) != 1:
        raise DiffCoverageError("each LCOV record must contain exactly one SF directive")
    source = resolve_lcov_source(source_values[0][0], repo_root=repo_root)
    data: dict[int, int] = {}
    lf: int | None = None
    lh: int | None = None

    for key, value, line_number in directives:
        context = f"LCOV line {line_number} ({key})"
        if key in {"TN", "SF", "VER"}:
            continue
        if key == "DA":
            fields = value.split(",")
            if len(fields) not in {2, 3}:
                raise DiffCoverageError(f"{context} must have line,count[,checksum]")
            source_line = parse_nonnegative_int(fields[0], context=f"{context} line")
            hits = parse_nonnegative_int(fields[1], context=f"{context} count")
            if source_line == 0:
                raise DiffCoverageError(f"{context} source line must be positive")
            if source_line in data:
                raise DiffCoverageError(
                    f"{context} duplicates source line {source_line} in one record"
                )
            if len(fields) == 3 and (
                not re.fullmatch(r"[0-9A-Fa-f]{32}", fields[2])
            ):
                raise DiffCoverageError(f"{context} checksum must be a 32-digit MD5")
            data[source_line] = hits
        elif key in {"LF", "LH"}:
            parsed = parse_nonnegative_int(value, context=context)
            if key == "LF":
                if lf is not None:
                    raise DiffCoverageError(f"{context} duplicates LF")
                lf = parsed
            else:
                if lh is not None:
                    raise DiffCoverageError(f"{context} duplicates LH")
                lh = parsed
        elif key == "FN":
            fields = value.split(",", 1)
            if len(fields) != 2 or not fields[1]:
                raise DiffCoverageError(f"{context} must have line,name")
            if parse_nonnegative_int(fields[0], context=f"{context} line") == 0:
                raise DiffCoverageError(f"{context} function line must be positive")
        elif key == "FNDA":
            fields = value.split(",", 1)
            if len(fields) != 2 or not fields[1]:
                raise DiffCoverageError(f"{context} must have count,name")
            parse_nonnegative_int(fields[0], context=f"{context} count")
        elif key in {"FNF", "FNH", "BRF", "BRH"}:
            parse_nonnegative_int(value, context=context)
        elif key == "BRDA":
            fields = value.split(",")
            if len(fields) != 4:
                raise DiffCoverageError(f"{context} must have line,block,branch,taken")
            if parse_nonnegative_int(fields[0], context=f"{context} line") == 0:
                raise DiffCoverageError(f"{context} branch line must be positive")
            parse_nonnegative_int(fields[1], context=f"{context} block")
            parse_nonnegative_int(fields[2], context=f"{context} branch")
            if fields[3] != "-":
                parse_nonnegative_int(fields[3], context=f"{context} taken")
        else:
            raise DiffCoverageError(f"{context} is unsupported")

    if lf is None or lh is None:
        raise DiffCoverageError("each LCOV record must contain LF and LH summaries")
    expected_lf = len(data)
    expected_lh = sum(hits > 0 for hits in data.values())
    if lf != expected_lf or lh != expected_lh:
        raise DiffCoverageError(
            f"LCOV LF/LH summary mismatch: declared {lf}/{lh}, "
            f"computed {expected_lf}/{expected_lh}"
        )
    if source is None:
        return None
    source_path = repo_root.joinpath(*pathlib.PurePosixPath(source).parts)
    try:
        source_line_count = len(source_path.read_text(encoding="utf-8").splitlines())
    except (OSError, UnicodeDecodeError) as error:
        raise DiffCoverageError(f"cannot validate LCOV source {source}: {error}") from error
    out_of_range = sorted(line for line in data if line > source_line_count)
    if out_of_range:
        raise DiffCoverageError(
            f"LCOV source {source!r} maps line(s) beyond its current "
            f"{source_line_count}-line file: {out_of_range[:5]}"
        )
    return SourceCoverage(source, data)


def parse_lcov(text: str, *, repo_root: pathlib.Path) -> dict[str, SourceCoverage]:
    if "\x00" in text:
        raise DiffCoverageError("LCOV contains a NUL byte")
    records: list[list[tuple[str, str, int]]] = []
    current: list[tuple[str, str, int]] = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        if not line:
            continue
        if line == "end_of_record":
            if not current:
                raise DiffCoverageError(
                    f"LCOV line {line_number} ends an empty record"
                )
            records.append(current)
            current = []
            continue
        if ":" not in line:
            raise DiffCoverageError(
                f"LCOV line {line_number} is not a directive or end_of_record"
            )
        key, value = line.split(":", 1)
        if not re.fullmatch(r"[A-Z]+", key):
            raise DiffCoverageError(f"LCOV line {line_number} has an invalid directive")
        if key == "SF" and any(existing[0] == "SF" for existing in current):
            raise DiffCoverageError(
                f"LCOV line {line_number} starts a second source without end_of_record"
            )
        current.append((key, value, line_number))
    if current:
        raise DiffCoverageError("LCOV final record is missing end_of_record")
    if not records:
        raise DiffCoverageError("LCOV contains no source records")

    sources: dict[str, SourceCoverage] = {}
    identity: dict[str, str] = {}
    for directives in records:
        coverage = parse_lcov_record(directives, repo_root=repo_root)
        if coverage is None:
            continue
        key = coverage.path.casefold() if os.name == "nt" else coverage.path
        if key in identity:
            raise DiffCoverageError(
                f"LCOV contains duplicate source records for {coverage.path!r}"
            )
        identity[key] = coverage.path
        sources[coverage.path] = coverage
    return sources


def require_exact_json_keys(
    value: Mapping[str, Any],
    expected: set[str] | frozenset[str],
    *,
    context: str,
) -> None:
    actual = set(value)
    if actual != set(expected):
        missing = sorted(set(expected) - actual)
        unknown = sorted(actual - set(expected))
        details: list[str] = []
        if missing:
            details.append(f"missing {missing}")
        if unknown:
            details.append(f"unknown {unknown}")
        raise DiffCoverageError(
            f"{context} fields are invalid: {', '.join(details)}"
        )


def require_json_object(value: Any, *, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise DiffCoverageError(f"{context} must be a JSON object")
    return value


def require_json_array(value: Any, *, context: str) -> list[Any]:
    if not isinstance(value, list):
        raise DiffCoverageError(f"{context} must be a JSON array")
    return value


def require_json_string(value: Any, *, context: str) -> str:
    if (
        not isinstance(value, str)
        or not value
        or value != value.strip()
        or "\x00" in value
    ):
        raise DiffCoverageError(
            f"{context} must be a non-empty, trimmed string without NUL bytes"
        )
    return value


def require_json_nonnegative_integer(value: Any, *, context: str) -> int:
    if type(value) is not int or value < 0:
        raise DiffCoverageError(f"{context} must be a non-negative integer")
    return value


def duplicate_rejecting_json_object(
    pairs: list[tuple[str, Any]],
) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise DiffCoverageError(f"coverage map duplicates object key {key!r}")
        value[key] = item
    return value


def reject_json_numeric_constant(value: str) -> None:
    raise DiffCoverageError(
        f"coverage map contains unsupported numeric constant {value}"
    )


def coverage_percent_text(covered: int, count: int) -> str | None:
    if count == 0:
        return None
    with decimal.localcontext() as context:
        context.prec = 40
        value = (
            decimal.Decimal(covered)
            * decimal.Decimal(100)
            / decimal.Decimal(count)
        )
        return format(value.quantize(decimal.Decimal("0.000001")), "f")


def validate_coverage_percent(
    value: Any,
    *,
    covered: int,
    count: int,
    context: str,
) -> None:
    expected = coverage_percent_text(covered, count)
    if value != expected:
        raise DiffCoverageError(
            f"{context} must be the canonical percentage {expected!r}"
        )


def source_physical_lines(
    raw: bytes,
    *,
    source: str,
) -> list[str]:
    text = decode_utf8(raw, description=f"coverage-map source {source}")
    if re.search(r"\r(?!\n)", text):
        raise DiffCoverageError(
            f"coverage-map source {source} contains a bare carriage return"
        )
    normalized = text.replace("\r\n", "\n")
    if not normalized:
        return []
    lines = normalized.split("\n")
    if normalized.endswith("\n"):
        lines.pop()
    return lines


def validate_artifact_reference(value: Any, *, context: str) -> None:
    metadata = require_json_object(value, context=context)
    require_exact_json_keys(
        metadata,
        {"path", "size_bytes", "sha256"},
        context=context,
    )
    require_json_string(metadata["path"], context=f"{context}.path")
    size = require_json_nonnegative_integer(
        metadata["size_bytes"],
        context=f"{context}.size_bytes",
    )
    if size == 0:
        raise DiffCoverageError(f"{context}.size_bytes must be positive")
    digest = require_json_string(
        metadata["sha256"],
        context=f"{context}.sha256",
    )
    if re.fullmatch(r"sha256:[0-9a-f]{64}", digest) is None:
        raise DiffCoverageError(
            f"{context}.sha256 must be a lowercase SHA-256 digest"
        )


def parse_coverage_map(
    value: bytes,
    *,
    repo_root: pathlib.Path,
) -> tuple[dict[str, SourceCoverage], dict[str, Any]]:
    text = decode_utf8(value, description="coverage map")
    try:
        parsed = json.loads(
            text,
            object_pairs_hook=duplicate_rejecting_json_object,
            parse_constant=reject_json_numeric_constant,
        )
    except DiffCoverageError:
        raise
    except json.JSONDecodeError as error:
        raise DiffCoverageError(f"coverage map is malformed JSON: {error}") from error
    document = require_json_object(parsed, context="coverage map")
    for field in ("schema_version", "kind", "status", "errors"):
        if field not in document:
            raise DiffCoverageError(f"coverage map is missing required field {field!r}")
    if document["schema_version"] != LLVM_LINE_MAP_SCHEMA_VERSION:
        raise DiffCoverageError("coverage map has an unsupported schema version")
    if document["kind"] != LLVM_LINE_MAP_KIND:
        raise DiffCoverageError("coverage map has an unsupported kind")
    errors = require_json_array(document["errors"], context="coverage map errors")
    if document["status"] != "passed":
        detail = (
            " | ".join(str(item) for item in errors)
            if errors
            else f"status is {document['status']!r}"
        )
        raise DiffCoverageError(f"coverage map did not pass: {detail}")
    if errors:
        raise DiffCoverageError("passing coverage map must have no errors")
    require_exact_json_keys(
        document,
        {
            "schema_version",
            "kind",
            "status",
            "line_model",
            "inputs",
            "producer",
            "summary",
            "files",
            "errors",
        },
        context="coverage map",
    )
    if document["line_model"] != LLVM_LINE_MODEL:
        raise DiffCoverageError("coverage map uses an unsupported line model")

    inputs = require_json_object(document["inputs"], context="coverage map inputs")
    require_exact_json_keys(
        inputs,
        {"llvm_json", "llvm_text"},
        context="coverage map inputs",
    )
    validate_artifact_reference(
        inputs["llvm_json"],
        context="coverage map inputs.llvm_json",
    )
    validate_artifact_reference(
        inputs["llvm_text"],
        context="coverage map inputs.llvm_text",
    )

    producer = require_json_object(
        document["producer"],
        context="coverage map producer",
    )
    require_exact_json_keys(
        producer,
        {
            "llvm_export_type",
            "llvm_export_version",
            "cargo_llvm_cov_version",
            "manifest_path",
        },
        context="coverage map producer",
    )
    expected_producer = {
        "llvm_export_type": LLVM_EXPORT_TYPE,
        "llvm_export_version": LLVM_EXPORT_VERSION,
        "cargo_llvm_cov_version": CARGO_LLVM_COV_VERSION,
        "manifest_path": "Cargo.toml",
    }
    if producer != expected_producer:
        raise DiffCoverageError(
            "coverage map producer metadata does not match the pinned tool contract"
        )

    summary = require_json_object(
        document["summary"],
        context="coverage map summary",
    )
    summary_fields = {
        "file_count",
        "source_line_count",
        "instrumented_line_count",
        "covered_line_count",
        "uncovered_line_count",
        "physical_line_percent",
        "llvm_summary_line_count",
        "llvm_summary_covered_line_count",
        "llvm_summary_line_percent",
        "llvm_minus_physical_line_count",
        "llvm_minus_physical_covered_line_count",
    }
    require_exact_json_keys(summary, summary_fields, context="coverage map summary")
    nonnegative_summary_fields = {
        "file_count",
        "source_line_count",
        "instrumented_line_count",
        "covered_line_count",
        "uncovered_line_count",
        "llvm_summary_line_count",
        "llvm_summary_covered_line_count",
    }
    summary_counts = {
        field: require_json_nonnegative_integer(
            summary[field],
            context=f"coverage map summary.{field}",
        )
        for field in nonnegative_summary_fields
    }
    for field in (
        "llvm_minus_physical_line_count",
        "llvm_minus_physical_covered_line_count",
    ):
        if type(summary[field]) is not int:
            raise DiffCoverageError(
                f"coverage map summary.{field} must be an integer"
            )
    instrumented = summary_counts["instrumented_line_count"]
    covered = summary_counts["covered_line_count"]
    if instrumented == 0:
        raise DiffCoverageError("coverage map contains no instrumented lines")
    if covered > instrumented:
        raise DiffCoverageError(
            "coverage map covered line count exceeds instrumented line count"
        )
    if summary_counts["uncovered_line_count"] != instrumented - covered:
        raise DiffCoverageError("coverage map uncovered line count is inconsistent")
    if instrumented > summary_counts["source_line_count"]:
        raise DiffCoverageError(
            "coverage map instrumented line count exceeds physical source lines"
        )
    validate_coverage_percent(
        summary["physical_line_percent"],
        covered=covered,
        count=instrumented,
        context="coverage map summary.physical_line_percent",
    )
    llvm_count = summary_counts["llvm_summary_line_count"]
    llvm_covered = summary_counts["llvm_summary_covered_line_count"]
    if llvm_covered > llvm_count:
        raise DiffCoverageError(
            "coverage map LLVM covered line count exceeds LLVM line count"
        )
    validate_coverage_percent(
        summary["llvm_summary_line_percent"],
        covered=llvm_covered,
        count=llvm_count,
        context="coverage map summary.llvm_summary_line_percent",
    )
    if summary["llvm_minus_physical_line_count"] != llvm_count - instrumented:
        raise DiffCoverageError(
            "coverage map LLVM/physical line-count delta is inconsistent"
        )
    if (
        summary["llvm_minus_physical_covered_line_count"]
        != llvm_covered - covered
    ):
        raise DiffCoverageError(
            "coverage map LLVM/physical covered-line delta is inconsistent"
        )

    files = require_json_array(document["files"], context="coverage map files")
    if not files:
        raise DiffCoverageError("coverage map contains no files")
    sources: dict[str, SourceCoverage] = {}
    identities: dict[str, str] = {}
    aggregate_source_lines = 0
    aggregate_instrumented = 0
    aggregate_covered = 0
    file_fields = {
        "path",
        "source_sha256",
        "source_line_count",
        "instrumented_line_count",
        "covered_line_count",
        "lines",
    }
    for file_index, raw_file in enumerate(files):
        context = f"coverage map files[{file_index}]"
        file_value = require_json_object(raw_file, context=context)
        require_exact_json_keys(file_value, file_fields, context=context)
        path = repository_relative_path(
            require_json_string(file_value["path"], context=f"{context}.path"),
            context=f"{context}.path",
        )
        if not path.startswith("crates/") or not path.endswith(".rs"):
            raise DiffCoverageError(
                f"{context}.path must be a Rust source below crates/"
            )
        identity = path.casefold() if os.name == "nt" else path
        if identity in identities:
            raise DiffCoverageError(
                f"coverage map contains duplicate source file {path!r}"
            )
        identities[identity] = path

        source_path = repo_root.joinpath(*pathlib.PurePosixPath(path).parts)
        source_raw = read_bounded(
            source_path,
            limit=MAX_COVERAGE_SOURCE_BYTES,
            description=f"coverage-map source {path}",
        )
        digest = require_json_string(
            file_value["source_sha256"],
            context=f"{context}.source_sha256",
        )
        if re.fullmatch(r"sha256:[0-9a-f]{64}", digest) is None:
            raise DiffCoverageError(
                f"{context}.source_sha256 must be a lowercase SHA-256 digest"
            )
        if digest != sha256_bytes(source_raw):
            raise DiffCoverageError(
                f"coverage map source digest is stale for {path!r}"
            )
        source_lines = source_physical_lines(source_raw, source=path)
        source_line_count = require_json_nonnegative_integer(
            file_value["source_line_count"],
            context=f"{context}.source_line_count",
        )
        if source_line_count != len(source_lines):
            raise DiffCoverageError(
                f"coverage map physical source line count is stale for {path!r}"
            )
        file_instrumented = require_json_nonnegative_integer(
            file_value["instrumented_line_count"],
            context=f"{context}.instrumented_line_count",
        )
        file_covered = require_json_nonnegative_integer(
            file_value["covered_line_count"],
            context=f"{context}.covered_line_count",
        )
        if file_instrumented > source_line_count or file_covered > file_instrumented:
            raise DiffCoverageError(
                f"coverage map counts are impossible for {path!r}"
            )

        line_rows = require_json_array(file_value["lines"], context=f"{context}.lines")
        if len(line_rows) != file_instrumented:
            raise DiffCoverageError(
                f"coverage map line row count is inconsistent for {path!r}"
            )
        line_map: dict[int, int] = {}
        previous = 0
        for line_index, raw_line in enumerate(line_rows):
            line_context = f"{context}.lines[{line_index}]"
            row = require_json_array(raw_line, context=line_context)
            if len(row) != 2:
                raise DiffCoverageError(
                    f"{line_context} must contain line number and binary execution"
                )
            line_number = require_json_nonnegative_integer(
                row[0],
                context=f"{line_context}[0]",
            )
            if line_number <= previous or line_number > source_line_count:
                raise DiffCoverageError(
                    f"{line_context} is duplicate, out of order, or out of range"
                )
            if type(row[1]) is not int or row[1] not in {0, 1}:
                raise DiffCoverageError(
                    f"{line_context}[1] must be binary execution evidence"
                )
            line_map[line_number] = row[1]
            previous = line_number
        if sum(line_map.values()) != file_covered:
            raise DiffCoverageError(
                f"coverage map covered line count is inconsistent for {path!r}"
            )
        sources[path] = SourceCoverage(path, line_map)
        aggregate_source_lines += source_line_count
        aggregate_instrumented += file_instrumented
        aggregate_covered += file_covered

    aggregates = {
        "file_count": len(files),
        "source_line_count": aggregate_source_lines,
        "instrumented_line_count": aggregate_instrumented,
        "covered_line_count": aggregate_covered,
    }
    for field, actual in aggregates.items():
        if summary_counts[field] != actual:
            raise DiffCoverageError(
                f"coverage map summary.{field} does not equal its file records"
            )
    return sources, document


def mask_rust_comments_and_literals(text: str, *, source: str) -> str:
    """Replace non-code tokens with spaces while preserving every newline."""

    masked = list(text)

    def hide(start: int, end: int) -> None:
        for position in range(start, end):
            if masked[position] not in {"\r", "\n"}:
                masked[position] = " "

    index = 0
    length = len(text)
    while index < length:
        if text.startswith("//", index):
            end = text.find("\n", index + 2)
            end = length if end < 0 else end
            hide(index, end)
            index = end
            continue
        if text.startswith("/*", index):
            start = index
            depth = 1
            index += 2
            while index < length and depth:
                if text.startswith("/*", index):
                    depth += 1
                    index += 2
                elif text.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            if depth:
                line = text.count("\n", 0, start) + 1
                raise DiffCoverageError(
                    f"{source}:{line} has an unterminated Rust block comment"
                )
            hide(start, index)
            continue

        raw_match = re.match(r'(?:br|cr|r)(#{0,255})"', text[index:])
        if raw_match is not None and (
            index == 0
            or not (text[index - 1].isalnum() or text[index - 1] == "_")
        ):
            start = index
            hashes = raw_match.group(1)
            content_start = index + raw_match.end()
            closing = f'"{hashes}'
            closing_start = text.find(closing, content_start)
            if closing_start < 0:
                line = text.count("\n", 0, start) + 1
                raise DiffCoverageError(
                    f"{source}:{line} has an unterminated Rust raw string"
                )
            index = closing_start + len(closing)
            hide(start, index)
            continue

        string_prefix = 2 if text.startswith(('b"', 'c"'), index) else 1
        if text[index] == '"' or string_prefix == 2:
            start = index
            index += string_prefix
            escaped = False
            while index < length:
                character = text[index]
                if escaped:
                    escaped = False
                elif character == "\\":
                    escaped = True
                elif character == '"':
                    index += 1
                    break
                index += 1
            else:
                line = text.count("\n", 0, start) + 1
                raise DiffCoverageError(
                    f"{source}:{line} has an unterminated Rust string"
                )
            hide(start, index)
            continue

        character_match = RUST_CHAR_LITERAL.match(text, index)
        if character_match is not None:
            hide(index, character_match.end())
            index = character_match.end()
            continue
        index += 1
    return "".join(masked)


def rust_attribute_spans(masked: str, *, source: str) -> list[tuple[int, int]]:
    """Locate complete outer attributes in already-masked Rust source."""

    spans: list[tuple[int, int]] = []
    cursor = 0
    attribute_start = re.compile(r"#\s*\[")
    while match := attribute_start.search(masked, cursor):
        start = match.start()
        opening = masked.find("[", start, match.end())
        if opening < 0:  # pragma: no cover - guaranteed by attribute_start
            raise AssertionError("attribute regex did not contain an opening bracket")
        depth = 1
        position = opening + 1
        while position < len(masked) and depth:
            if masked[position] == "[":
                depth += 1
            elif masked[position] == "]":
                depth -= 1
            position += 1
        if depth:
            line = masked.count("\n", 0, start) + 1
            raise DiffCoverageError(
                f"{source}:{line} has an unterminated Rust attribute"
            )
        spans.append((start, position))
        cursor = position
    return spans


def inline_cfg_test_module_lines(
    source_lines: Sequence[str],
    *,
    source: str,
) -> set[int]:
    """Return complete inline ``#[cfg(test)] mod`` line ranges.

    Braces inside comments, ordinary/byte/C strings, raw strings, and character
    literals are masked first. An exact cfg(test) attribute followed by an
    external ``mod name;`` declaration has no inline range. Once an inline
    module declaration is recognized, a missing or ambiguous body fails closed.
    """

    text = "\n".join(source_lines)
    masked = mask_rust_comments_and_literals(text, source=source)
    spans = rust_attribute_spans(masked, source=source)
    spans_by_start = {start: (start, end) for start, end in spans}
    line_starts = [0]
    line_starts.extend(
        position + 1 for position, character in enumerate(masked) if character == "\n"
    )

    def line_number(position: int) -> int:
        return bisect.bisect_right(line_starts, position)

    excluded: set[int] = set()
    for attribute_start, attribute_end in spans:
        attribute = masked[attribute_start:attribute_end]
        if EXACT_CFG_TEST_ATTRIBUTE.fullmatch(attribute) is None:
            continue

        cursor = attribute_end
        while True:
            while cursor < len(masked) and masked[cursor].isspace():
                cursor += 1
            following_attribute = spans_by_start.get(cursor)
            if following_attribute is None:
                break
            cursor = following_attribute[1]

        declaration = INLINE_MODULE_DECLARATION.match(masked, cursor)
        if declaration is None:
            # cfg(test) is also legitimately used for functions, impl members,
            # imports, and other test seams. Those remain production-file scope.
            continue
        cursor = declaration.end()
        while cursor < len(masked) and masked[cursor].isspace():
            cursor += 1
        if cursor < len(masked) and masked[cursor] == ";":
            continue
        if cursor >= len(masked) or masked[cursor] != "{":
            line = line_number(attribute_start)
            raise DiffCoverageError(
                f"{source}:{line} has an ambiguous inline #[cfg(test)] module "
                "declaration; expected '{' or ';'"
            )

        depth = 1
        closing = cursor + 1
        while closing < len(masked) and depth:
            if masked[closing] == "{":
                depth += 1
            elif masked[closing] == "}":
                depth -= 1
            closing += 1
        if depth:
            line = line_number(attribute_start)
            raise DiffCoverageError(
                f"{source}:{line} has an unclosed inline #[cfg(test)] module"
            )
        start_line = line_number(attribute_start)
        end_line = line_number(closing - 1)
        excluded.update(range(start_line, end_line + 1))
    return excluded


def lexical_non_coverable_lines(source_lines: Sequence[str]) -> set[int]:
    """Return lines that are conservatively structural rather than executable.

    This is intentionally not a Rust parser. It exempts only whitespace,
    comments, attributes, imports, item/function signatures, and punctuation
    that cannot itself express runtime behavior. Everything else remains
    unmapped when LCOV has no DA entry, including statements inside cfg-gated
    bodies. Multi-line attributes, imports, and function signatures are tracked
    so ordinary formatting does not create false failures.
    """

    result: set[int] = set()
    in_block_comment = False
    attribute_depth = 0
    in_import = False
    in_signature = False
    for number, line in enumerate(source_lines, start=1):
        stripped = line.strip()
        if not stripped:
            result.add(number)
            continue
        if in_block_comment:
            result.add(number)
            if "*/" in stripped:
                tail = stripped.split("*/", 1)[1].strip()
                in_block_comment = False
                if tail:
                    result.discard(number)
            continue
        if stripped.startswith(("//", "///", "//!")):
            result.add(number)
            continue
        if stripped.startswith("/*"):
            if "*/" not in stripped[2:]:
                in_block_comment = True
                result.add(number)
            elif not stripped.split("*/", 1)[1].strip():
                result.add(number)
            continue
        if attribute_depth:
            result.add(number)
            attribute_depth += stripped.count("[") - stripped.count("]")
            if attribute_depth <= 0:
                attribute_depth = 0
                if "]" in stripped and stripped.rsplit("]", 1)[1].strip():
                    result.discard(number)
            continue
        if stripped.startswith(("#[", "#![")):
            attribute_depth = stripped.count("[") - stripped.count("]")
            tail = (
                stripped.rsplit("]", 1)[1].strip()
                if attribute_depth <= 0 and "]" in stripped
                else ""
            )
            if not tail:
                result.add(number)
            if attribute_depth < 0:
                attribute_depth = 0
            continue
        if in_import:
            result.add(number)
            if ";" in stripped:
                in_import = False
            continue
        if IMPORT_START.match(stripped):
            result.add(number)
            if ";" not in stripped:
                in_import = True
            continue
        if in_signature:
            # A line that opens the body and contains anything after the opening
            # brace may also contain an executable statement; keep it unmapped.
            if "{" in stripped:
                tail = stripped.split("{", 1)[1].strip()
                if not tail or tail == "}":
                    result.add(number)
                in_signature = False
            else:
                result.add(number)
                if ";" in stripped:
                    in_signature = False
            continue
        if FUNCTION_SIGNATURE_START.match(stripped):
            if "{" in stripped:
                tail = stripped.split("{", 1)[1].strip()
                if not tail or tail == "}":
                    result.add(number)
            elif ";" in stripped:
                result.add(number)
            else:
                result.add(number)
                in_signature = True
            continue
        if TYPE_BLOCK_START.match(stripped):
            # Item headers introduce compile-time structure. Runtime expressions
            # sharing the same line are intentionally not exempt.
            if "{" not in stripped or not stripped.split("{", 1)[1].strip():
                result.add(number)
            continue
        if PUNCTUATION_ONLY.fullmatch(stripped):
            result.add(number)
            continue
        if re.fullmatch(r"}\s*else\s*{", stripped):
            result.add(number)
            continue
        if re.fullmatch(
            r"(?:pub(?:\([^)]*\))?\s+)?(?:mod\s+[A-Za-z_][A-Za-z0-9_]*"
            r"|type\s+.+);",
            stripped,
        ):
            result.add(number)
    return result


def decimal_percent(covered: int, coverable: int) -> decimal.Decimal | None:
    if coverable == 0:
        return None
    with decimal.localcontext() as context:
        context.prec = 28
        return (decimal.Decimal(covered) * 100 / decimal.Decimal(coverable)).quantize(
            decimal.Decimal("0.01"), rounding=decimal.ROUND_DOWN
        )


def parse_minimum(value: str) -> decimal.Decimal:
    if not PERCENT.fullmatch(value):
        raise DiffCoverageError(
            "minimum percent must be a decimal number between 0 and 100 "
            "with at most two fractional digits"
        )
    return decimal.Decimal(value).quantize(decimal.Decimal("0.01"))


def analyze_coverage(
    changed_files: Sequence[ChangedFile],
    sources: Mapping[str, SourceCoverage],
    *,
    repo_root: pathlib.Path,
    minimum: decimal.Decimal,
    critical_policy: CriticalPathPolicy,
    coverage_map_label: str = "lcov",
) -> dict[str, Any]:
    file_reports: list[dict[str, Any]] = []
    totals = {
        "changed_rust_lines": 0,
        "production_changed_lines": 0,
        "excluded_test_lines": 0,
        "inline_test_lines": 0,
        "coverable_lines": 0,
        "covered_lines": 0,
        "uncovered_lines": 0,
        "non_coverable_lines": 0,
        "unmapped_lines": 0,
    }
    coverage_class_totals = {
        name: {
            "production_changed_lines": 0,
            "production_changed_files": 0,
            "inline_test_lines": 0,
            "coverable_lines": 0,
            "covered_lines": 0,
            "uncovered_lines": 0,
            "non_coverable_lines": 0,
            "unmapped_lines": 0,
        }
        for name in ("ordinary", "critical")
    }

    source_lookup = {
        (path.casefold() if os.name == "nt" else path): coverage
        for path, coverage in sources.items()
    }
    for changed_file in changed_files:
        path = changed_file.new_path
        if path is None or not path.endswith(".rs"):
            continue
        test_only = is_test_only_rust_path(path)
        if test_only:
            line_reports = [
                {
                    "line": changed_line.number,
                    "text": changed_line.text,
                    "status": "excluded-test",
                    "reason": "test-only-path",
                }
                for changed_line in changed_file.added_lines
            ]
            changed_count = len(changed_file.added_lines)
            totals["changed_rust_lines"] += changed_count
            totals["excluded_test_lines"] += changed_count
            file_reports.append(
                {
                    "path": path,
                    "old_path": changed_file.old_path,
                    "change_kind": changed_file.change_kind,
                    "scope": "test-only",
                    "coverage_class": "excluded",
                    "critical_path": None,
                    "summary": {
                        "changed": changed_count,
                        "production_changed": 0,
                        "excluded_test": changed_count,
                        "inline_test": 0,
                        "coverable": 0,
                        "covered": 0,
                        "uncovered": 0,
                        "non_coverable": 0,
                        "unmapped": 0,
                    },
                    "lines": line_reports,
                }
            )
            continue
        new_critical_rule = critical_policy.match(path)
        old_critical_rule = critical_policy.match(changed_file.old_path)
        if new_critical_rule is not None:
            critical_rule = new_critical_rule
            critical_match_origin = "new-path"
        elif old_critical_rule is not None:
            # Base/head mode retains the immutable base rule, so renaming a
            # critical file or directory out of its current rule cannot earn
            # the weaker ordinary threshold in the same change.
            critical_rule = old_critical_rule
            critical_match_origin = "old-path"
        else:
            critical_rule = None
            critical_match_origin = None
        coverage_class = "critical" if critical_rule is not None else "ordinary"
        key = path.casefold() if os.name == "nt" else path
        coverage = source_lookup.get(key)
        lexical: set[int] = set()
        inline_test: set[int] = set()
        if changed_file.added_lines:
            source_path = repo_root.joinpath(
                *pathlib.PurePosixPath(path).parts
            )
            try:
                source_lines = source_path.read_text(encoding="utf-8").splitlines()
            except (OSError, UnicodeDecodeError) as error:
                raise DiffCoverageError(
                    f"cannot read unmapped Rust source {path}: {error}"
                ) from error
            lexical = lexical_non_coverable_lines(source_lines)
            inline_test = inline_cfg_test_module_lines(
                source_lines,
                source=path,
            )

        line_reports: list[dict[str, Any]] = []
        file_counts = {
            "changed": len(changed_file.added_lines),
            "production_changed": 0,
            "excluded_test": 0,
            "inline_test": 0,
            "coverable": 0,
            "covered": 0,
            "uncovered": 0,
            "non_coverable": 0,
            "unmapped": 0,
        }
        for changed_line in changed_file.added_lines:
            item: dict[str, Any] = {
                "line": changed_line.number,
                "text": changed_line.text,
            }
            if changed_line.number in inline_test:
                item["status"] = "excluded-inline-test"
                item["reason"] = "cfg-test-inline-module"
                file_counts["inline_test"] += 1
                line_reports.append(item)
                continue

            file_counts["production_changed"] += 1
            if coverage is not None and changed_line.number in coverage.lines:
                hits = coverage.lines[changed_line.number]
                item["hits"] = hits
                item["status"] = "covered" if hits > 0 else "uncovered"
                file_counts["coverable"] += 1
                file_counts[item["status"]] += 1
            elif coverage is not None:
                if changed_line.number in lexical:
                    item["status"] = "non-coverable"
                    item["reason"] = (
                        f"lexical-structure-absent-from-{coverage_map_label}-map"
                    )
                    file_counts["non_coverable"] += 1
                else:
                    item["status"] = "unmapped"
                    item["reason"] = (
                        "executable-looking-line-absent-from-"
                        f"{coverage_map_label}-map"
                    )
                    file_counts["unmapped"] += 1
            elif changed_line.number in lexical:
                item["status"] = "non-coverable"
                item["reason"] = "lexical-structure-in-unmapped-file"
                file_counts["non_coverable"] += 1
            else:
                item["status"] = "unmapped"
                item["reason"] = f"source-file-absent-from-{coverage_map_label}"
                file_counts["unmapped"] += 1
            line_reports.append(item)

        totals["changed_rust_lines"] += file_counts["changed"]
        totals["production_changed_lines"] += file_counts["production_changed"]
        totals["inline_test_lines"] += file_counts["inline_test"]
        totals["coverable_lines"] += file_counts["coverable"]
        totals["covered_lines"] += file_counts["covered"]
        totals["uncovered_lines"] += file_counts["uncovered"]
        totals["non_coverable_lines"] += file_counts["non_coverable"]
        totals["unmapped_lines"] += file_counts["unmapped"]
        class_totals = coverage_class_totals[coverage_class]
        if file_counts["production_changed"]:
            class_totals["production_changed_files"] += 1
        class_totals["production_changed_lines"] += file_counts["production_changed"]
        class_totals["inline_test_lines"] += file_counts["inline_test"]
        class_totals["coverable_lines"] += file_counts["coverable"]
        class_totals["covered_lines"] += file_counts["covered"]
        class_totals["uncovered_lines"] += file_counts["uncovered"]
        class_totals["non_coverable_lines"] += file_counts["non_coverable"]
        class_totals["unmapped_lines"] += file_counts["unmapped"]
        critical_path_report: dict[str, Any] | None = None
        if critical_rule is not None and critical_match_origin is not None:
            critical_path_report = {
                **critical_rule.report(),
                "matched_on": critical_match_origin,
            }
        file_reports.append(
            {
                "path": path,
                "old_path": changed_file.old_path,
                "change_kind": changed_file.change_kind,
                "scope": "production",
                "coverage_class": coverage_class,
                "critical_path": critical_path_report,
                "summary": file_counts,
                "lines": line_reports,
            }
        )

    percent = decimal_percent(totals["covered_lines"], totals["coverable_lines"])
    coverage_classes: dict[str, Any] = {}
    policy_errors: list[str] = []
    for name, class_minimum in (
        ("ordinary", minimum),
        ("critical", critical_policy.minimum),
    ):
        class_totals = coverage_class_totals[name]
        class_percent = decimal_percent(
            class_totals["covered_lines"],
            class_totals["coverable_lines"],
        )
        class_errors: list[str] = []
        if class_totals["unmapped_lines"]:
            class_errors.append(
                f"{name} coverage has {class_totals['unmapped_lines']} changed "
                "Rust line(s) with no LCOV source mapping"
            )
        if class_percent is not None and class_percent < class_minimum:
            class_errors.append(
                f"{name} changed-line coverage {class_percent:.2f}% is below "
                f"required {class_minimum:.2f}%"
            )
        if class_errors:
            class_status = "failed"
        elif class_totals["coverable_lines"] == 0:
            class_status = "not-applicable"
        else:
            class_status = "passed"
        coverage_classes[name] = {
            "status": class_status,
            "minimum_percent": f"{class_minimum:.2f}",
            "summary": {
                **class_totals,
                "percent": (
                    None if class_percent is None else f"{class_percent:.2f}"
                ),
            },
            "errors": class_errors,
        }
        policy_errors.extend(class_errors)
    status = "failed" if policy_errors else "passed"
    return {
        "status": status,
        "policy": {
            "minimum_percent": f"{minimum:.2f}",
            "ordinary_minimum_percent": f"{minimum:.2f}",
            "critical_minimum_percent": f"{critical_policy.minimum:.2f}",
            "scope": "production-rust-paths-v3-inline-cfg-test",
            "critical_path_policy": {
                "schema_version": critical_policy.schema_version,
                "rules": [rule.report() for rule in critical_policy.rules],
                "versions": [
                    version.report() for version in critical_policy.versions
                ],
            },
        },
        "summary": {
            **totals,
            "changed_files": len(file_reports),
            "production_changed_files": sum(
                report["scope"] == "production"
                and report["summary"].get("production_changed", 0) > 0
                for report in file_reports
            ),
            "excluded_test_files": sum(
                report["scope"] == "test-only" for report in file_reports
            ),
            "inline_test_files": sum(
                report["summary"].get("inline_test", 0) > 0
                for report in file_reports
            ),
            "percent": None if percent is None else f"{percent:.2f}",
        },
        "coverage_classes": coverage_classes,
        "files": file_reports,
        "errors": policy_errors,
    }


def run_git(
    repo_root: pathlib.Path, argv: Sequence[str], *, description: str
) -> str:
    try:
        process = subprocess.run(
            ["git", "-C", str(repo_root), *argv],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="strict",
        )
    except (OSError, UnicodeError) as error:
        raise DiffCoverageError(f"cannot {description}: {error}") from error
    if process.returncode != 0:
        detail = process.stderr.strip() or process.stdout.strip() or "no diagnostics"
        raise DiffCoverageError(f"cannot {description}: {detail}")
    return process.stdout


def run_git_bytes(
    repo_root: pathlib.Path,
    argv: Sequence[str],
    *,
    description: str,
) -> bytes:
    try:
        process = subprocess.run(
            ["git", "-C", str(repo_root), *argv],
            check=False,
            capture_output=True,
            text=False,
        )
    except OSError as error:
        raise DiffCoverageError(f"cannot {description}: {error}") from error
    if process.returncode != 0:
        detail = process.stderr.decode("utf-8", "replace").strip()
        if not detail:
            detail = process.stdout.decode("utf-8", "replace").strip()
        raise DiffCoverageError(
            f"cannot {description}: {detail or 'no diagnostics'}"
        )
    return process.stdout


def resolve_commit(repo_root: pathlib.Path, revision: str, *, label: str) -> str:
    if not revision or revision != revision.strip() or revision.startswith("-"):
        raise DiffCoverageError(f"{label} revision must be a non-empty safe Git revision")
    output = run_git(
        repo_root,
        ["rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}"],
        description=f"resolve {label} revision",
    )
    lines = output.splitlines()
    if len(lines) != 1 or not FULL_SHA.fullmatch(lines[0]):
        raise DiffCoverageError(f"{label} revision did not resolve to exactly one commit")
    return lines[0]


def load_diff_input(
    *,
    repo_root: pathlib.Path,
    diff_path: pathlib.Path | None,
    base: str | None,
    head: str | None,
) -> DiffInput:
    if diff_path is not None:
        if base is not None or head is not None:
            raise DiffCoverageError("--diff cannot be combined with --base or --head")
        raw = read_bounded(diff_path, limit=MAX_DIFF_BYTES, description="unified diff")
        return DiffInput(
            decode_utf8(raw, description="unified diff"),
            str(diff_path),
            None,
            None,
        )
    if base is None or head is None:
        raise DiffCoverageError("provide either --diff or both --base and --head")

    base_sha = resolve_commit(repo_root, base, label="base")
    head_sha = resolve_commit(repo_root, head, label="head")
    checkout_sha = resolve_commit(repo_root, "HEAD", label="checkout HEAD")
    if checkout_sha != head_sha:
        raise DiffCoverageError(
            f"checkout HEAD {checkout_sha} does not match requested head {head_sha}"
        )
    dirty = run_git(
        repo_root,
        ["status", "--porcelain=v1", "--untracked-files=all", "--", "*.rs"],
        description="check Rust worktree cleanliness",
    )
    if dirty.strip():
        raise DiffCoverageError(
            "base/head mode requires a clean Rust source tree; Git reported: "
            + " | ".join(dirty.splitlines())
        )
    text = run_git(
        repo_root,
        [
            "-c",
            "core.quotePath=true",
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=0",
            "--find-renames",
            "--diff-filter=ACMR",
            base_sha,
            head_sha,
            "--",
            "*.rs",
        ],
        description="generate base/head Rust diff",
    )
    if len(text.encode("utf-8")) > MAX_DIFF_BYTES:
        raise DiffCoverageError(
            f"generated unified diff exceeds the {MAX_DIFF_BYTES}-byte safety limit"
        )
    return DiffInput(text, "git-base-head", base_sha, head_sha)


def atomic_write_json(path: pathlib.Path, value: Mapping[str, Any]) -> None:
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
        raise DiffCoverageError(f"cannot write JSON report {path}: {error}") from error
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


def build_report(
    *,
    repo_root: pathlib.Path,
    lcov_path: pathlib.Path | None,
    diff_path: pathlib.Path | None,
    base: str | None,
    head: str | None,
    minimum_text: str,
    critical_policy_path: pathlib.Path | None = None,
    coverage_map_path: pathlib.Path | None = None,
) -> dict[str, Any]:
    root = repo_root.resolve()
    if not root.is_dir():
        raise DiffCoverageError(f"repository root is not a directory: {root}")
    minimum = parse_minimum(minimum_text)
    if (lcov_path is None) == (coverage_map_path is None):
        raise DiffCoverageError(
            "exactly one of LCOV or the canonical coverage map must be supplied"
        )
    coverage_document: dict[str, Any] | None = None
    if coverage_map_path is not None:
        coverage_bytes = read_bounded(
            coverage_map_path,
            limit=MAX_COVERAGE_MAP_BYTES,
            description="coverage map",
        )
        sources, coverage_document = parse_coverage_map(
            coverage_bytes,
            repo_root=root,
        )
        coverage_map_label = "canonical-coverage"
        coverage_inputs: dict[str, Any] = {
            "coverage_kind": "llvm-physical-line-map",
            "coverage_map": str(coverage_map_path),
            "coverage_map_sha256": sha256_bytes(coverage_bytes),
            "coverage_line_model": coverage_document["line_model"],
            "coverage_producer": coverage_document["producer"],
            "coverage_producer_inputs": coverage_document["inputs"],
        }
    else:
        assert lcov_path is not None
        coverage_bytes = read_bounded(
            lcov_path,
            limit=MAX_LCOV_BYTES,
            description="LCOV",
        )
        lcov_text = decode_utf8(coverage_bytes, description="LCOV")
        sources = parse_lcov(lcov_text, repo_root=root)
        coverage_map_label = "lcov"
        coverage_inputs = {
            "coverage_kind": "legacy-lcov-diagnostic",
            "lcov": str(lcov_path),
            "lcov_sha256": sha256_bytes(coverage_bytes),
        }
    diff_input = load_diff_input(
        repo_root=root,
        diff_path=diff_path,
        base=base,
        head=head,
    )
    policy_source, _policy_file = resolve_critical_policy_source(
        repo_root=root,
        policy_path=critical_policy_path,
    )
    parsed_diff = parse_unified_diff(diff_input.text)
    if diff_input.base_sha is None:
        if any(
            changed_file.old_path == policy_source
            or changed_file.new_path == policy_source
            for changed_file in parsed_diff
        ):
            raise DiffCoverageError(
                "explicit --diff input changes the critical path policy; "
                "use base/head mode so classification is bound to both revisions"
            )
        critical_policy = load_critical_path_policy(
            repo_root=root,
            policy_path=critical_policy_path,
            origin="head",
            revision=None,
        )
    else:
        assert diff_input.head_sha is not None
        head_policy, _head_version = load_critical_path_policy_at_revision(
            repo_root=root,
            source=policy_source,
            revision=diff_input.head_sha,
            origin="head",
        )
        if head_policy is None:
            raise DiffCoverageError(
                f"head revision {diff_input.head_sha} has no critical path "
                f"policy at {policy_source}"
            )
        base_policy, base_version = load_critical_path_policy_at_revision(
            repo_root=root,
            source=policy_source,
            revision=diff_input.base_sha,
            origin="base",
        )
        critical_policy = merge_critical_path_policies(
            head_policy,
            base_policy=base_policy,
            base_version=base_version,
        )
    changed_files = materialize_implicit_rust_additions(
        parsed_diff,
        repo_root=root,
        critical_policy=critical_policy,
    )
    validate_source_bindings(changed_files, repo_root=root)
    result = analyze_coverage(
        changed_files,
        sources,
        repo_root=root,
        minimum=minimum,
        critical_policy=critical_policy,
        coverage_map_label=coverage_map_label,
    )
    result.update(
        {
            "schema_version": SCHEMA_VERSION,
            "kind": REPORT_KIND,
            "inputs": {
                "repository_root": str(root),
                **coverage_inputs,
                "diff_origin": diff_input.origin,
                "diff_sha256": sha256_bytes(diff_input.text.encode("utf-8")),
                "base_sha": diff_input.base_sha,
                "head_sha": diff_input.head_sha,
                "critical_path_policy": critical_policy.source,
                "critical_path_policy_sha256": critical_policy.sha256,
                "critical_path_policy_versions": [
                    version.report() for version in critical_policy.versions
                ],
            },
        }
    )
    return result


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path("."))
    coverage = parser.add_mutually_exclusive_group(required=True)
    coverage.add_argument(
        "--coverage-map",
        type=pathlib.Path,
        help="source-bound Sorotte LLVM physical-line map",
    )
    coverage.add_argument(
        "--lcov",
        type=pathlib.Path,
        help="strict legacy LCOV diagnostic input",
    )
    parser.add_argument("--diff", type=pathlib.Path)
    parser.add_argument("--base")
    parser.add_argument("--head")
    parser.add_argument(
        "--minimum",
        default="100",
        help=(
            "required ordinary changed executable-line coverage percentage "
            "(default: 100)"
        ),
    )
    parser.add_argument(
        "--critical-policy",
        type=pathlib.Path,
        help=(
            "repository-relative critical-path policy "
            f"(default: {DEFAULT_CRITICAL_POLICY_PATH})"
        ),
    )
    parser.add_argument("--json-out", type=pathlib.Path)
    return parser


def error_report(error: Exception) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": REPORT_KIND,
        "status": "error",
        "errors": [str(error)],
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argument_parser()
    args = parser.parse_args(argv)
    try:
        report = build_report(
            repo_root=args.repo_root,
            lcov_path=args.lcov,
            coverage_map_path=args.coverage_map,
            diff_path=args.diff,
            base=args.base,
            head=args.head,
            minimum_text=args.minimum,
            critical_policy_path=args.critical_policy,
        )
        if args.json_out is not None:
            atomic_write_json(args.json_out, report)
        summary = report["summary"]
        percent = summary["percent"] if summary["percent"] is not None else "n/a"
        print(
            f"diff coverage: {report['status']} "
            f"({summary['covered_lines']}/{summary['coverable_lines']}, "
            f"{percent}%; {summary['non_coverable_lines']} non-coverable, "
            f"{summary['unmapped_lines']} unmapped; "
            f"ordinary={report['coverage_classes']['ordinary']['status']}, "
            f"critical={report['coverage_classes']['critical']['status']})"
        )
        for error in report["errors"]:
            print(f"error: {error}", file=sys.stderr)
        return 0 if report["status"] == "passed" else 1
    except DiffCoverageError as error:
        report = error_report(error)
        if args.json_out is not None:
            try:
                atomic_write_json(args.json_out, report)
            except DiffCoverageError as write_error:
                print(f"diff coverage input error: {error}", file=sys.stderr)
                print(f"cannot write error report: {write_error}", file=sys.stderr)
                return 2
        print(f"diff coverage input error: {error}", file=sys.stderr)
        return 2
    except Exception as error:  # Defensive fail-closed boundary for CI artifacts.
        wrapped = DiffCoverageError(
            f"unexpected {type(error).__name__} while evaluating coverage: {error}"
        )
        report = error_report(wrapped)
        if args.json_out is not None:
            try:
                atomic_write_json(args.json_out, report)
            except DiffCoverageError as write_error:
                print(f"diff coverage input error: {wrapped}", file=sys.stderr)
                print(f"cannot write error report: {write_error}", file=sys.stderr)
                return 2
        print(f"diff coverage input error: {wrapped}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
