#!/usr/bin/env python3
"""Keep named production contracts critical when their Rust modules split.

The catalog names responsibility roots, independently of coverage path rules.
External Rust modules below each root are followed (including #[path]), and
every resulting production file must retain a critical rule and its owner.
Target-specific modules are all included; test modules are excluded.
"""
from __future__ import annotations

import argparse
import pathlib
import re
import sys
import tomllib

from diff_coverage import DiffCoverageError, is_test_only_rust_path, load_critical_path_policy


class BoundaryError(ValueError):
    pass


MODULE = re.compile(
    r'(?P<attrs>(?:^[ \t]*#\[[^\n]+\][ \t]*\n)*)'
    r'^[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<name>[A-Za-z_][A-Za-z_0-9]*)\s*;',
    re.MULTILINE,
)
PATH = re.compile(r'#\[path\s*=\s*"([^"]+)"\]')


def without_test_modules(text: str) -> str:
    # Balance braces on a copy with Rust strings/comments masked. The original
    # attributes remain available for #[path] resolution after test removal.
    literal = re.compile(r'r(\#*)".*?"\1|"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/', re.DOTALL)
    masked = literal.sub(lambda match: re.sub(r"[^\n]", " ", match[0]), text)
    start = re.compile(r'#\[cfg\(test\)\]\s*(?:#\[[^\n]+\]\s*)*(?:pub\s+)?mod\s+\w+\s*\{')
    spans = []
    for match in start.finditer(text):
        depth, offset = 1, match.end()
        while depth and offset < len(masked):
            depth += (masked[offset] == "{") - (masked[offset] == "}")
            offset += 1
        if depth:
            raise BoundaryError("cannot resolve test module braces")
        spans.append((match.start(), offset))
    for begin, end in reversed(spans):
        text = text[:begin] + re.sub(r"[^\n]", " ", text[begin:end]) + text[end:]
    return text


def production(path: pathlib.Path, root: pathlib.Path) -> bool:
    relative = path.relative_to(root).as_posix()
    return path.suffix == ".rs" and not is_test_only_rust_path(relative)


def owned_modules(root: pathlib.Path, initial: str) -> set[str]:
    pending = [root / initial]
    found: set[str] = set()
    while pending:
        path = pending.pop()
        if path.is_symlink():
            raise BoundaryError(f"critical boundary contains a symlink: {path}")
        path = path.resolve()
        if not path.is_relative_to(root):
            raise BoundaryError(f"critical module escapes repository: {path}")
        if path.is_dir():
            pending.extend(path.rglob("*.rs"))
            continue
        if not path.is_file():
            raise BoundaryError(f"critical responsibility root/module is missing: {path}")
        if not production(path, root):
            continue
        relative = path.relative_to(root).as_posix()
        if relative in found:
            continue
        found.add(relative)
        text = without_test_modules(path.read_text(encoding="utf-8"))
        # Ignore block comments so commented-out module declarations are not roots.
        text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
        for match in MODULE.finditer(text):
            attrs, name = match["attrs"], match["name"]
            if re.search(r"cfg\(\s*test\s*\)", attrs) or name in {"tests", "test_support", "testing"} or name.endswith("_tests"):
                continue
            explicit = PATH.search(attrs)
            if explicit:
                candidates = [path.parent / explicit[1]]
            else:
                parent = path.parent if path.name in {"lib.rs", "main.rs", "mod.rs"} else path.with_suffix("")
                candidates = [parent / f"{name}.rs", parent / name / "mod.rs"]
            existing = [candidate for candidate in candidates if candidate.is_file()]
            if len(existing) != 1:
                raise BoundaryError(f"cannot resolve exactly one production module {name!r} in {relative}")
            pending.extend(existing)
    return found


def validate(root: pathlib.Path, catalog: pathlib.Path | None = None) -> dict[str, list[str]]:
    root = root.resolve()
    catalog = catalog or root / "coverage/critical-boundaries.toml"
    raw = catalog.read_bytes()
    if len(raw) > 128 * 1024:
        raise BoundaryError("critical boundary catalog exceeds byte limit")
    data = tomllib.loads(raw.decode("utf-8"))
    if set(data) != {"schema_version", "boundary"} or type(data["schema_version"]) is not int or data["schema_version"] != 1:
        raise BoundaryError("unsupported critical boundary catalog")
    policy = load_critical_path_policy(repo_root=root, policy_path=None)
    result: dict[str, list[str]] = {}
    for boundary in data["boundary"]:
        if set(boundary) != {"id", "owner", "contract", "roots"}:
            raise BoundaryError("boundary must name id, owner, contract, roots")
        identifier = boundary["id"]
        if identifier in result or not all(isinstance(boundary[key], str) and boundary[key].strip() for key in ("id", "owner", "contract")):
            raise BoundaryError("invalid or duplicate critical boundary")
        roots = boundary["roots"]
        if not isinstance(roots, list) or not roots or len(set(roots)) != len(roots):
            raise BoundaryError(f"{identifier}: roots must be unique and nonempty")
        files: set[str] = set()
        for initial in roots:
            if not isinstance(initial, str) or "\\" in initial or pathlib.PurePosixPath(initial).is_absolute() or ".." in pathlib.PurePosixPath(initial).parts:
                raise BoundaryError(f"{identifier}: invalid repository root")
            files.update(owned_modules(root, initial))
        if not files:
            raise BoundaryError(f"{identifier}: no production modules")
        for source in sorted(files):
            rule = policy.match(source)
            if rule is None or rule.owner != boundary["owner"]:
                raise BoundaryError(f"{identifier}: {source} lost critical coverage owned by {boundary['owner']}")
        result[identifier] = sorted(files)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path("."))
    args = parser.parse_args()
    try:
        result = validate(args.repo_root)
    except (BoundaryError, DiffCoverageError, OSError, ValueError) as error:
        print(f"critical boundary inventory failed: {error}", file=sys.stderr)
        return 1
    print(f"Validated {len(result)} responsibility boundaries and {len(set().union(*map(set, result.values())))} critical production modules")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
