#!/usr/bin/env python3
"""Fail-closed policy for Rust tests carrying an ``#[ignore]`` attribute.

An ignored test is neither coverage nor a harmless implementation detail. This
tool discovers ignored Rust tests from source and requires an exact
machine-readable disposition for every one. The registry is data only: this
program never executes commands supplied by it.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import json
import pathlib
import re
import sys
import tomllib
from collections import Counter
from collections.abc import Mapping, Sequence
from typing import Any


SCHEMA_VERSION = 1
EXPECTED_TIERS = [
    "manual",
    "maintenance",
    "nightly",
    "pull-request",
    "quarantined",
    "subprocess-fixture",
]
EXPECTED_OWNERS = [
    "cli",
    "compatibility",
    "gui",
    "player-mpv",
    "simulation",
]
EXPECTED_OPERATING_SYSTEMS = ["linux", "macos", "windows"]

IGNORE_ATTRIBUTE = re.compile(
    r'^\s*#\[\s*ignore\s*=\s*"((?:\\.|[^"\\])*)"\s*\]\s*$'
)
IGNORE_CANDIDATE = re.compile(
    r"^\s*#\[\s*(?:ignore\b|cfg_attr\([^]]*\bignore\b)"
)
TEST_ATTRIBUTE = re.compile(r"^\s*#\[\s*(?:tokio::)?test(?:\([^]]*\))?\s*\]\s*$")
ATTRIBUTE = re.compile(r"^\s*#\[[^]]+\]\s*$")
FUNCTION = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
IDENTIFIER = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
TEST_SELECTOR = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+$")
POLICY_ID = re.compile(r"^IGN-[A-Z0-9]+-\d{3}$")
JOB_ID = re.compile(r"^[a-z0-9][a-z0-9-]*$")
CHAR_LITERAL = re.compile(
    r"""(?:b)?'(?:\\(?:[nrt0\\'"]|x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]{1,6}\})|[^\\'\r\n])'"""
)


class IgnoredTestPolicyError(ValueError):
    """The source inventory or its policy registry is invalid."""


@dataclasses.dataclass(frozen=True, order=True)
class IgnoredTest:
    source: str
    test: str
    source_reason: str
    line: int

    @property
    def key(self) -> tuple[str, str]:
        return (self.source, self.test)


def _strict_table(value: Any, context: str) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise IgnoredTestPolicyError(f"{context} must be a TOML table")
    return value


def _strict_string(value: Any, context: str, *, minimum: int = 1) -> str:
    if not isinstance(value, str) or len(value.strip()) < minimum:
        raise IgnoredTestPolicyError(
            f"{context} must be a nonblank string of at least {minimum} characters"
        )
    return value


def _strict_string_list(
    value: Any,
    context: str,
    *,
    allowed: set[str] | None = None,
    nonempty: bool = True,
) -> list[str]:
    if (
        not isinstance(value, list)
        or (nonempty and not value)
        or any(not isinstance(item, str) or not item.strip() for item in value)
    ):
        qualifier = "nonempty " if nonempty else ""
        raise IgnoredTestPolicyError(f"{context} must be a {qualifier}string array")
    if len(value) != len(set(value)):
        raise IgnoredTestPolicyError(f"{context} contains duplicate values")
    if allowed is not None:
        unexpected = sorted(set(value) - allowed)
        if unexpected:
            raise IgnoredTestPolicyError(
                f"{context} contains unsupported values: {unexpected}"
            )
    return value


def _require_exact_keys(
    table: Mapping[str, Any], expected: set[str], context: str
) -> None:
    actual = set(table)
    if actual != expected:
        missing = sorted(expected - actual)
        unknown = sorted(actual - expected)
        raise IgnoredTestPolicyError(
            f"{context} fields differ; missing={missing}, unknown={unknown}"
        )


def load_registry(path: pathlib.Path) -> Mapping[str, Any]:
    try:
        with path.open("rb") as handle:
            value = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise IgnoredTestPolicyError(
            f"cannot read ignored-test registry {path}: {error}"
        ) from error
    return _strict_table(value, "registry")


def _decode_rust_string(raw: str, context: str) -> str:
    try:
        value = json.loads(f'"{raw}"')
    except json.JSONDecodeError as error:
        raise IgnoredTestPolicyError(
            f"{context} uses an unsupported Rust string escape: {error}"
        ) from error
    return _strict_string(value, context)


def _mask_rust_comments_and_strings(text: str) -> str:
    """Preserve positions while hiding comments and string contents.

    This is intentionally a small lexical pass, not a Rust parser. Its job is
    to ensure a multiline or conditional ignore attribute cannot evade the
    stricter supported-form parser below.
    """

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
                raise IgnoredTestPolicyError("unterminated Rust block comment")
            hide(start, index)
            continue

        character_match = CHAR_LITERAL.match(text, index)
        if character_match:
            hide(index, character_match.end())
            index = character_match.end()
            continue

        raw_match = re.match(r'(?:br|cr|r)(#{0,255})"', text[index:])
        if raw_match:
            start = index
            hashes = raw_match.group(1)
            index += raw_match.end()
            closing = f'"{hashes}'
            end = text.find(closing, index)
            if end < 0:
                raise IgnoredTestPolicyError("unterminated Rust raw string")
            index = end + len(closing)
            hide(start, index)
            continue

        string_prefix = (
            2 if text.startswith(('b"', 'c"'), index) else 1
        )
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
                raise IgnoredTestPolicyError("unterminated Rust string")
            hide(start, index)
            continue
        index += 1
    return "".join(masked)


def _ignore_attribute_start_lines(text: str) -> set[int]:
    masked = _mask_rust_comments_and_strings(text)
    starts: set[int] = set()
    index = 0
    while index < len(masked) - 1:
        if masked[index : index + 2] != "#[":
            index += 1
            continue
        start = index
        depth = 1
        index += 2
        while index < len(masked) and depth:
            if masked[index] == "[":
                depth += 1
            elif masked[index] == "]":
                depth -= 1
            index += 1
        if depth:
            line = text.count("\n", 0, start) + 1
            raise IgnoredTestPolicyError(
                f"unterminated Rust attribute beginning at line {line}"
            )
        attribute = masked[start:index]
        if re.search(r"\bignore\b", attribute):
            starts.add(text.count("\n", 0, start) + 1)
    return starts


def _might_contain_ignore_attribute(text: str) -> bool:
    """Cheap conservative prefilter before the lexical attribute pass."""

    index = 0
    while True:
        start = text.find("#[", index)
        if start < 0:
            return False
        end = text.find("]", start + 2)
        if end < 0:
            return True
        if re.search(r"\bignore\b", text[start : end + 1]):
            return True
        index = end + 1


def _attached_test_function(
    lines: Sequence[str],
    ignore_index: int,
    *,
    source: str,
) -> str:
    saw_test = False
    preceding = ignore_index - 1
    while preceding >= 0 and ATTRIBUTE.fullmatch(lines[preceding]):
        saw_test = saw_test or bool(TEST_ATTRIBUTE.fullmatch(lines[preceding]))
        preceding -= 1
    if not saw_test:
        raise IgnoredTestPolicyError(
            f"{source}:{ignore_index + 1} ignore attribute is not attached to "
            "a #[test] or #[tokio::test]"
        )

    for following in range(ignore_index + 1, min(len(lines), ignore_index + 12)):
        text = lines[following]
        if not text.strip() or ATTRIBUTE.fullmatch(text):
            continue
        match = FUNCTION.match(text)
        if match:
            return match.group(1)
        raise IgnoredTestPolicyError(
            f"{source}:{ignore_index + 1} ignore attribute is not immediately "
            "attached to a test function"
        )
    raise IgnoredTestPolicyError(
        f"{source}:{ignore_index + 1} ignore attribute has no following test function"
    )


def discover_ignored_tests(repo_root: pathlib.Path) -> list[IgnoredTest]:
    crates_root = repo_root / "crates"
    if not crates_root.is_dir():
        raise IgnoredTestPolicyError(f"missing Rust crates directory {crates_root}")

    discovered: list[IgnoredTest] = []
    for source_path in sorted(crates_root.rglob("*.rs")):
        if source_path.is_symlink():
            raise IgnoredTestPolicyError(
                f"refusing symlinked Rust source {source_path.relative_to(repo_root)}"
            )
        try:
            source_text = source_path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            raise IgnoredTestPolicyError(
                f"cannot read Rust source {source_path}: {error}"
            ) from error
        lines = source_text.splitlines()
        source = source_path.relative_to(repo_root).as_posix()
        handled_attribute_lines: set[int] = set()
        for index, line in enumerate(lines):
            if not IGNORE_CANDIDATE.match(line):
                continue
            match = IGNORE_ATTRIBUTE.fullmatch(line)
            if not match:
                raise IgnoredTestPolicyError(
                    f"{source}:{index + 1} ignore attributes must use the single-line "
                    '#[ignore = "reason"] form'
                )
            reason = _decode_rust_string(
                match.group(1), f"{source}:{index + 1} ignore reason"
            )
            test = _attached_test_function(lines, index, source=source)
            discovered.append(
                IgnoredTest(
                    source=source,
                    test=test,
                    source_reason=reason,
                    line=index + 1,
                )
            )
            handled_attribute_lines.add(index + 1)
        try:
            ignored_attribute_lines = (
                _ignore_attribute_start_lines(source_text)
                if _might_contain_ignore_attribute(source_text)
                else set()
            )
        except IgnoredTestPolicyError as error:
            raise IgnoredTestPolicyError(f"{source}: {error}") from error
        unsupported = sorted(ignored_attribute_lines - handled_attribute_lines)
        if unsupported:
            raise IgnoredTestPolicyError(
                f"{source}:{unsupported[0]} uses an unsupported multiline or "
                "conditional ignore attribute; use #[ignore = \"reason\"]"
            )

    keys = [item.key for item in discovered]
    duplicates = sorted(key for key, count in Counter(keys).items() if count > 1)
    if duplicates:
        raise IgnoredTestPolicyError(
            f"duplicate ignored test identities discovered: {duplicates}"
        )
    return sorted(discovered)


def _validate_policy_header(registry: Mapping[str, Any]) -> None:
    _require_exact_keys(
        registry,
        {"schema_version", "policy", "ignored_test"},
        "registry",
    )
    version = registry["schema_version"]
    if isinstance(version, bool) or not isinstance(version, int) or version != SCHEMA_VERSION:
        raise IgnoredTestPolicyError(
            f"unsupported ignored-test registry schema {version!r}"
        )
    policy = _strict_table(registry["policy"], "registry.policy")
    _require_exact_keys(
        policy,
        {"allowed_tiers", "allowed_owners", "allowed_operating_systems"},
        "registry.policy",
    )
    if policy["allowed_tiers"] != EXPECTED_TIERS:
        raise IgnoredTestPolicyError(
            f"registry.policy.allowed_tiers must equal {EXPECTED_TIERS!r}"
        )
    if policy["allowed_owners"] != EXPECTED_OWNERS:
        raise IgnoredTestPolicyError(
            f"registry.policy.allowed_owners must equal {EXPECTED_OWNERS!r}"
        )
    if policy["allowed_operating_systems"] != EXPECTED_OPERATING_SYSTEMS:
        raise IgnoredTestPolicyError(
            "registry.policy.allowed_operating_systems must equal "
            f"{EXPECTED_OPERATING_SYSTEMS!r}"
        )


def _validate_entry(
    raw: Any,
    *,
    index: int,
    as_of: dt.date,
) -> tuple[tuple[str, str], Mapping[str, Any]]:
    entry = _strict_table(raw, f"ignored_test[{index}]")
    tier = _strict_string(entry.get("tier"), f"ignored_test[{index}].tier")
    if tier not in EXPECTED_TIERS:
        raise IgnoredTestPolicyError(
            f"ignored_test[{index}].tier is unsupported: {tier!r}"
        )

    common = {
        "id",
        "source",
        "test",
        "source_reason",
        "tier",
        "owner",
        "rationale",
        "prerequisites",
        "operating_systems",
    }
    tier_fields = {
        "pull-request": {"required_job"},
        "nightly": {"required_job"},
        "manual": set(),
        "maintenance": {"mutates_fixtures"},
        "quarantined": {"tracking", "review_by"},
        "subprocess-fixture": {"required_job", "parent_test"},
    }
    _require_exact_keys(entry, common | tier_fields[tier], f"ignored_test[{index}]")

    policy_id = _strict_string(entry["id"], f"ignored_test[{index}].id")
    if not POLICY_ID.fullmatch(policy_id):
        raise IgnoredTestPolicyError(
            f"ignored_test[{index}].id has invalid format: {policy_id!r}"
        )
    source = _strict_string(entry["source"], f"{policy_id}.source")
    source_path = pathlib.PurePosixPath(source)
    if (
        source_path.is_absolute()
        or ".." in source_path.parts
        or not source.startswith("crates/")
        or source_path.suffix != ".rs"
    ):
        raise IgnoredTestPolicyError(
            f"{policy_id}.source must be a normalized crates/**/*.rs path"
        )
    test = _strict_string(entry["test"], f"{policy_id}.test")
    if not IDENTIFIER.fullmatch(test):
        raise IgnoredTestPolicyError(f"{policy_id}.test is not a Rust identifier")
    _strict_string(entry["source_reason"], f"{policy_id}.source_reason")
    owner = _strict_string(entry["owner"], f"{policy_id}.owner")
    if owner not in EXPECTED_OWNERS:
        raise IgnoredTestPolicyError(f"{policy_id}.owner is unsupported: {owner!r}")
    _strict_string(entry["rationale"], f"{policy_id}.rationale", minimum=20)
    _strict_string_list(entry["prerequisites"], f"{policy_id}.prerequisites")
    _strict_string_list(
        entry["operating_systems"],
        f"{policy_id}.operating_systems",
        allowed=set(EXPECTED_OPERATING_SYSTEMS),
    )

    if tier in {"pull-request", "nightly", "subprocess-fixture"}:
        required_job = _strict_string(entry["required_job"], f"{policy_id}.required_job")
        if not JOB_ID.fullmatch(required_job):
            raise IgnoredTestPolicyError(
                f"{policy_id}.required_job is not a normalized CI job ID"
            )
        if tier == "subprocess-fixture":
            parent = _strict_string(entry["parent_test"], f"{policy_id}.parent_test")
            if not TEST_SELECTOR.fullmatch(parent) or parent.split("::")[-1] == test:
                raise IgnoredTestPolicyError(
                    f"{policy_id}.parent_test must select a distinct ordinary Rust test"
                )
    elif tier == "maintenance":
        if entry["mutates_fixtures"] is not True:
            raise IgnoredTestPolicyError(
                f"{policy_id}.mutates_fixtures must be true for maintenance tests"
            )
    elif tier == "quarantined":
        _strict_string(entry["tracking"], f"{policy_id}.tracking", minimum=8)
        review_text = _strict_string(entry["review_by"], f"{policy_id}.review_by")
        try:
            review_by = dt.date.fromisoformat(review_text)
        except ValueError as error:
            raise IgnoredTestPolicyError(
                f"{policy_id}.review_by must be an ISO date"
            ) from error
        if review_by < as_of:
            raise IgnoredTestPolicyError(
                f"{policy_id} quarantine review expired on {review_by.isoformat()}"
            )

    return (source, test), entry


def validate_registry(
    registry: Mapping[str, Any],
    discovered: Sequence[IgnoredTest],
    *,
    as_of: dt.date | None = None,
    repo_root: pathlib.Path | None = None,
) -> Counter[str]:
    _validate_policy_header(registry)
    raw_entries = registry["ignored_test"]
    if not isinstance(raw_entries, list):
        raise IgnoredTestPolicyError("registry.ignored_test must be an array of tables")

    evaluation_date = as_of or dt.date.today()
    entries: dict[tuple[str, str], Mapping[str, Any]] = {}
    ids: set[str] = set()
    for index, raw in enumerate(raw_entries):
        key, entry = _validate_entry(raw, index=index, as_of=evaluation_date)
        policy_id = entry["id"]
        if policy_id in ids:
            raise IgnoredTestPolicyError(f"duplicate ignored-test ID {policy_id!r}")
        if key in entries:
            raise IgnoredTestPolicyError(f"duplicate ignored-test identity {key!r}")
        ids.add(policy_id)
        entries[key] = entry

    discovered_by_key = {item.key: item for item in discovered}
    missing = sorted(set(discovered_by_key) - set(entries))
    extra = sorted(set(entries) - set(discovered_by_key))
    if missing or extra:
        raise IgnoredTestPolicyError(
            f"ignored-test registry does not match source; missing={missing}, extra={extra}"
        )
    for key, source_test in discovered_by_key.items():
        registered_reason = entries[key]["source_reason"]
        if registered_reason != source_test.source_reason:
            raise IgnoredTestPolicyError(
                f"{key!r} source reason changed at line {source_test.line}; "
                f"registry={registered_reason!r}, source={source_test.source_reason!r}"
            )
        entry = entries[key]
        if entry["tier"] == "subprocess-fixture":
            if repo_root is None:
                raise IgnoredTestPolicyError("subprocess fixtures require a source root")
            source_path = repo_root / entry["source"]
            if source_path.is_symlink() or not source_path.resolve().is_relative_to(
                repo_root.resolve()
            ):
                raise IgnoredTestPolicyError("fixture parent source escapes the source root")
            masked = _mask_rust_comments_and_strings(
                source_path.read_text(encoding="utf-8")
            )
            parent = entry["parent_test"].split("::")[-1]
            lines = masked.splitlines()
            matches = [
                index for index, line in enumerate(lines)
                if (match := FUNCTION.match(line)) and match.group(1) == parent
            ]
            if len(matches) != 1 or (entry["source"], parent) in discovered_by_key:
                raise IgnoredTestPolicyError(
                    f"{entry['id']}.parent_test must define one nonignored test in its source"
                )
            preceding = matches[0] - 1
            attributes = []
            while preceding >= 0 and ATTRIBUTE.fullmatch(lines[preceding]):
                attributes.append(lines[preceding])
                preceding -= 1
            if not any(TEST_ATTRIBUTE.fullmatch(attribute) for attribute in attributes):
                raise IgnoredTestPolicyError(
                    f"{entry['id']}.parent_test is not an ordinary test"
                )

    return Counter(entry["tier"] for entry in entries.values())


def validate_command(args: argparse.Namespace) -> int:
    repo_root = pathlib.Path(args.repo_root).resolve()
    registry_path = pathlib.Path(args.registry).resolve()
    try:
        discovered = discover_ignored_tests(repo_root)
        tiers = validate_registry(load_registry(registry_path), discovered, repo_root=repo_root)
    except IgnoredTestPolicyError as error:
        print(f"ignored-test policy failed: {error}", file=sys.stderr)
        return 1
    rendered_tiers = ", ".join(f"{tier}={tiers[tier]}" for tier in sorted(tiers))
    print(
        f"valid ignored-test policy: {len(discovered)} tests"
        + (f" ({rendered_tiers})" if rendered_tiers else "")
    )
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    validate = subcommands.add_parser(
        "validate", help="compare ignored Rust tests with the policy registry"
    )
    validate.add_argument("--repo-root", default=".")
    validate.add_argument(
        "--registry",
        default="coverage/ignored-tests.toml",
    )
    validate.set_defaults(func=validate_command)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
