#!/usr/bin/env python3
"""Fail-closed registry for executable known-defect characterizations.

Known product defects are kept as narrowly matched ``should_panic`` tests until
the product fix converts them into positive regressions. A defect-free tree has
an explicit empty defect array. This validator makes any temporary defect state
explicit: every such test must have an owner, finding, expiry, exact panic
oracle, and selector, and no known-defect selector may be promoted into the
positive behavior catalog.
"""

from __future__ import annotations

import argparse
import ast
import dataclasses
import datetime as dt
import pathlib
import re
import sys
import tomllib
from collections.abc import Mapping, Sequence
from typing import Any


SCHEMA_VERSION = 1
DEFECT_ID = re.compile(r"^TC-[A-Z][A-Z0-9]*-[0-9]{3}$")
IDENTIFIER = re.compile(r"^[a-z][a-z0-9-]*$")
TEST_SELECTOR = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+$")
FUNCTION = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+"
    r"(?P<name>known_defect_[A-Za-z0-9_]+)\s*\("
)
EXPECTED_PANIC = re.compile(
    r"^#\[should_panic\s*\(\s*expected\s*=\s*"
    r"(?P<literal>\"(?:\\.|[^\"\\])*\")\s*\)\s*\]$"
)
FINDING_HEADING = re.compile(
    r"^## (?P<id>TC-[A-Z][A-Z0-9]*-[0-9]{3}): (?P<title>\S(?:.*\S)?)$"
)
ALLOWED_SEVERITIES = {"critical", "high", "medium", "low"}


class PolicyError(ValueError):
    pass


@dataclasses.dataclass(frozen=True)
class Characterization:
    source: str
    function: str
    expected_panic: str
    line: int


def exact_keys(
    value: Mapping[str, Any],
    *,
    allowed: set[str],
    required: set[str],
    context: str,
) -> None:
    unknown = set(value) - allowed
    missing = required - set(value)
    if unknown:
        raise PolicyError(f"{context} has unknown keys: {sorted(unknown)}")
    if missing:
        raise PolicyError(f"{context} is missing keys: {sorted(missing)}")


def require_string(value: Any, context: str) -> str:
    if not isinstance(value, str) or not value.strip() or value != value.strip():
        raise PolicyError(f"{context} must be a non-empty trimmed string")
    return value


def require_string_list(value: Any, context: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise PolicyError(f"{context} must be a non-empty list")
    result = [require_string(item, f"{context}[]") for item in value]
    if len(result) != len(set(result)):
        raise PolicyError(f"{context} contains duplicates")
    return result


def safe_file(repo_root: pathlib.Path, relative_value: Any, context: str) -> pathlib.Path:
    relative = pathlib.PurePosixPath(require_string(relative_value, context))
    if relative.is_absolute() or ".." in relative.parts or "\\" in str(relative):
        raise PolicyError(f"{context} must be a normalized repository-relative POSIX path")
    candidate = (repo_root / pathlib.Path(*relative.parts)).resolve()
    root = repo_root.resolve()
    try:
        candidate.relative_to(root)
    except ValueError as error:
        raise PolicyError(f"{context} escapes the repository") from error
    if not candidate.is_file():
        raise PolicyError(f"{context} does not exist: {relative}")
    return candidate


def parse_expected_panic(attributes: Sequence[str], context: str) -> str | None:
    matching = [EXPECTED_PANIC.fullmatch(attribute) for attribute in attributes]
    literals = [match.group("literal") for match in matching if match is not None]
    if not literals:
        return None
    if len(literals) != 1:
        raise PolicyError(f"{context} has multiple should_panic(expected=...) attributes")
    try:
        decoded = ast.literal_eval(literals[0])
    except (SyntaxError, ValueError) as error:
        raise PolicyError(f"{context} has an invalid expected panic string") from error
    return require_string(decoded, f"{context} expected panic")


def scan_characterizations(repo_root: pathlib.Path) -> dict[tuple[str, str], Characterization]:
    crates_root = repo_root / "crates"
    if not crates_root.is_dir():
        raise PolicyError(f"missing crates directory: {crates_root}")
    found: dict[tuple[str, str], Characterization] = {}
    for source_path in sorted(crates_root.rglob("*.rs")):
        relative = source_path.relative_to(repo_root).as_posix()
        attributes: list[str] = []
        attribute_parts: list[str] | None = None
        try:
            lines = source_path.read_text(encoding="utf-8").splitlines()
        except OSError as error:
            raise PolicyError(f"cannot read {relative}: {error}") from error
        for line_number, line in enumerate(lines, start=1):
            stripped = line.strip()
            if attribute_parts is not None:
                attribute_parts.append(stripped)
                if stripped.endswith("]"):
                    attributes.append(" ".join(attribute_parts))
                    attribute_parts = None
                continue
            if stripped.startswith("#["):
                if stripped.endswith("]"):
                    attributes.append(stripped)
                else:
                    attribute_parts = [stripped]
                continue
            match = FUNCTION.match(stripped)
            if match is not None:
                name = match.group("name")
                context = f"{relative}:{line_number} {name}"
                expected = parse_expected_panic(attributes, context)
                has_should_panic = any(
                    attribute.startswith("#[should_panic") for attribute in attributes
                )
                if has_should_panic and expected is None:
                    raise PolicyError(
                        f"{context} must use an exact should_panic(expected=...) oracle"
                    )
                if expected is not None:
                    if "#[test]" not in attributes:
                        raise PolicyError(
                            f"{context} is should_panic but is not an ordinary #[test]"
                        )
                    key = (relative, name)
                    if key in found:
                        raise PolicyError(f"duplicate known-defect function at {context}")
                    found[key] = Characterization(
                        source=relative,
                        function=name,
                        expected_panic=expected,
                        line=line_number,
                    )
            if stripped and not stripped.startswith("//"):
                attributes.clear()
        if attribute_parts is not None:
            raise PolicyError(f"{relative} ends with an unterminated Rust attribute")
    return found


def load_toml(path: pathlib.Path, context: str) -> dict[str, Any]:
    try:
        with path.open("rb") as handle:
            value = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise PolicyError(f"cannot read {context} {path}: {error}") from error
    if not isinstance(value, dict):
        raise PolicyError(f"{context} root must be a table")
    return value


def finding_headings(path: pathlib.Path) -> dict[str, tuple[str, int]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise PolicyError(f"cannot read finding {path}: {error}") from error
    headings: dict[str, tuple[str, int]] = {}
    for line_number, line in enumerate(lines, start=1):
        match = FINDING_HEADING.fullmatch(line)
        if match is None:
            continue
        defect_id = match.group("id")
        if defect_id in headings:
            _, first_line = headings[defect_id]
            raise PolicyError(
                f"duplicate finding heading {defect_id} at {path}:{line_number} "
                f"(first at line {first_line})"
            )
        headings[defect_id] = (match.group("title"), line_number)
    return headings


def package_name_for_source(repo_root: pathlib.Path, source: str) -> tuple[str, pathlib.Path]:
    parts = pathlib.PurePosixPath(source).parts
    if len(parts) < 3 or parts[0] != "crates":
        raise PolicyError(f"characterization source is outside a crate: {source}")
    crate_root = repo_root / "crates" / parts[1]
    manifest = safe_file(
        repo_root,
        (pathlib.PurePosixPath("crates") / parts[1] / "Cargo.toml").as_posix(),
        f"{source} package manifest",
    )
    manifest_value = load_toml(manifest, "Cargo manifest")
    package = manifest_value.get("package")
    if not isinstance(package, dict):
        raise PolicyError(f"{manifest} has no package table")
    name = require_string(package.get("name"), f"{manifest} package.name")
    return name, crate_root.resolve()


def positive_behavior_selectors(catalog_path: pathlib.Path) -> set[str]:
    catalog = load_toml(catalog_path, "behavior catalog")
    behaviors = catalog.get("behavior")
    if not isinstance(behaviors, list):
        raise PolicyError("behavior catalog must contain behavior tables")
    selectors: set[str] = set()
    for behavior_index, behavior in enumerate(behaviors):
        if not isinstance(behavior, dict):
            raise PolicyError(f"behavior[{behavior_index}] must be a table")
        proofs = behavior.get("proof")
        if not isinstance(proofs, list):
            raise PolicyError(f"behavior[{behavior_index}].proof must be a list")
        for proof in proofs:
            if isinstance(proof, dict) and isinstance(proof.get("test"), str):
                selectors.add(proof["test"])
    return selectors


def validate_registry(
    registry: Mapping[str, Any],
    *,
    repo_root: pathlib.Path,
    catalog_path: pathlib.Path,
    today: dt.date | None = None,
) -> tuple[int, int]:
    exact_keys(
        registry,
        allowed={"schema_version", "defect"},
        required={"schema_version", "defect"},
        context="registry",
    )
    if type(registry["schema_version"]) is not int or registry["schema_version"] != SCHEMA_VERSION:
        raise PolicyError(f"unsupported registry schema {registry['schema_version']!r}")
    defects = registry["defect"]
    if not isinstance(defects, list):
        raise PolicyError("registry.defect must be a list")

    actual = scan_characterizations(repo_root)
    positive_selectors = positive_behavior_selectors(catalog_path)
    registered: dict[tuple[str, str], tuple[str, str]] = {}
    defect_ids: set[str] = set()
    selector_owners: dict[str, str] = {}
    finding_cache: dict[pathlib.Path, dict[str, tuple[str, int]]] = {}
    effective_today = today or dt.datetime.now(dt.UTC).date()

    for defect_index, defect in enumerate(defects):
        context = f"defect[{defect_index}]"
        if not isinstance(defect, dict):
            raise PolicyError(f"{context} must be a table")
        exact_keys(
            defect,
            allowed={
                "id",
                "title",
                "severity",
                "owners",
                "finding",
                "expires",
                "characterization",
            },
            required={
                "id",
                "title",
                "severity",
                "owners",
                "finding",
                "expires",
                "characterization",
            },
            context=context,
        )
        defect_id = require_string(defect["id"], f"{context}.id")
        if not DEFECT_ID.fullmatch(defect_id):
            raise PolicyError(f"{context}.id has invalid shape: {defect_id!r}")
        if defect_id in defect_ids:
            raise PolicyError(f"duplicate defect id {defect_id}")
        defect_ids.add(defect_id)
        title = require_string(defect["title"], f"{context}.title")
        severity = require_string(defect["severity"], f"{context}.severity")
        if severity not in ALLOWED_SEVERITIES:
            raise PolicyError(f"{defect_id} has unsupported severity {severity!r}")
        owners = require_string_list(defect["owners"], f"{context}.owners")
        if any(not IDENTIFIER.fullmatch(owner) for owner in owners):
            raise PolicyError(f"{defect_id} has an invalid owner")

        finding_path = safe_file(repo_root, defect["finding"], f"{context}.finding")
        if finding_path not in finding_cache:
            finding_cache[finding_path] = finding_headings(finding_path)
        headings = finding_cache[finding_path]
        heading = headings.get(defect_id)
        if heading is None:
            raise PolicyError(f"{defect_id} finding has no exact markdown heading")
        heading_title, heading_line = heading
        if heading_title.casefold() != title.casefold():
            raise PolicyError(
                f"{defect_id} finding title drifted at {finding_path}:{heading_line}: "
                f"registry {title!r}, heading {heading_title!r}"
            )

        expiry_text = require_string(defect["expires"], f"{context}.expires")
        try:
            expiry = dt.date.fromisoformat(expiry_text)
        except ValueError as error:
            raise PolicyError(f"{defect_id} expiry must be YYYY-MM-DD") from error
        if expiry < effective_today:
            raise PolicyError(
                f"{defect_id} expired on {expiry.isoformat()} (today {effective_today.isoformat()})"
            )

        characterizations = defect["characterization"]
        if not isinstance(characterizations, list) or not characterizations:
            raise PolicyError(f"{defect_id}.characterization must be a non-empty list")
        for characterization_index, item in enumerate(characterizations):
            item_context = f"{defect_id}.characterization[{characterization_index}]"
            if not isinstance(item, dict):
                raise PolicyError(f"{item_context} must be a table")
            exact_keys(
                item,
                allowed={"package", "source", "test", "expected_panic"},
                required={"package", "source", "test", "expected_panic"},
                context=item_context,
            )
            package = require_string(item["package"], f"{item_context}.package")
            source = require_string(item["source"], f"{item_context}.source")
            source_path = safe_file(repo_root, source, f"{item_context}.source")
            package_name, crate_root = package_name_for_source(repo_root, source)
            if package != package_name:
                raise PolicyError(
                    f"{item_context}.package is {package!r}, expected {package_name!r}"
                )
            try:
                source_path.resolve().relative_to(crate_root)
            except ValueError as error:
                raise PolicyError(f"{item_context}.source is outside package {package}") from error

            selector = require_string(item["test"], f"{item_context}.test")
            if not TEST_SELECTOR.fullmatch(selector):
                raise PolicyError(f"{item_context}.test is not an exact Rust selector")
            function = selector.rsplit("::", 1)[-1]
            expected = require_string(
                item["expected_panic"], f"{item_context}.expected_panic"
            )
            key = (source, function)
            if key in registered:
                previous = registered[key][0]
                raise PolicyError(
                    f"{source}::{function} is registered by both {previous} and {defect_id}"
                )
            if selector in selector_owners:
                raise PolicyError(
                    f"selector {selector} is shared by {selector_owners[selector]} and {defect_id}"
                )
            if selector in positive_selectors:
                raise PolicyError(
                    f"known-defect selector {selector} is also a positive behavior proof"
                )
            registered[key] = (defect_id, expected)
            selector_owners[selector] = defect_id

    missing = sorted(set(actual) - set(registered))
    stale = sorted(set(registered) - set(actual))
    if missing:
        details = ", ".join(f"{source}::{function}" for source, function in missing)
        raise PolicyError(f"unregistered known-defect characterizations: {details}")
    if stale:
        details = ", ".join(f"{source}::{function}" for source, function in stale)
        raise PolicyError(f"registry entries without executable characterizations: {details}")
    for key, found in actual.items():
        defect_id, expected = registered[key]
        if expected != found.expected_panic:
            raise PolicyError(
                f"{defect_id} expected panic drifted at {found.source}:{found.line}: "
                f"registry {expected!r}, source {found.expected_panic!r}"
            )

    return len(defect_ids), len(registered)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    validate = subparsers.add_parser("validate", help="validate registry and source inventory")
    validate.add_argument("--registry", type=pathlib.Path, required=True)
    validate.add_argument("--catalog", type=pathlib.Path, required=True)
    validate.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path.cwd())
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        registry = load_toml(args.registry, "known-defect registry")
        defect_count, characterization_count = validate_registry(
            registry,
            repo_root=args.repo_root.resolve(),
            catalog_path=args.catalog.resolve(),
        )
    except PolicyError as error:
        print(f"known-defect policy error: {error}", file=sys.stderr)
        return 1
    print(
        "valid known-defect registry: "
        f"{defect_count} defects, {characterization_count} characterizations"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
