#!/usr/bin/env python3
"""Verify registry archives against every supplied Cargo lock before Cargo uses them.

Only immutable downloaded .crate archives are reusable inputs. This receipt is
neither compiled-artifact qualification nor a cached advisory verdict.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys
import tomllib


def direct(path: Path, *, missing: bool = False) -> Path:
    absolute = path.expanduser().absolute()
    if ".." in absolute.parts:
        raise ValueError("cache paths must not contain parent traversal")
    for ancestor in (*reversed(absolute.parents), absolute):
        try:
            info = ancestor.lstat()
        except FileNotFoundError:
            if missing:
                continue
            raise
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0):
            raise ValueError(f"cache inputs must be direct paths, not links or reparse points: {ancestor}")
    return absolute


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def authorities(locks: list[Path]) -> tuple[dict[str, str], list[dict]]:
    if not locks:
        raise ValueError("at least one explicit Cargo lock authority is required")
    expected, identities, seen = {}, [], set()
    for path in locks:
        lock = direct(path)
        if lock in seen or not lock.is_file():
            raise ValueError("lock authorities must be unique direct regular files")
        seen.add(lock)
        raw = lock.read_bytes()
        data = tomllib.loads(raw.decode("utf-8"))
        if type(data.get("version")) is not int or data["version"] not in (3, 4) or not isinstance(data.get("package"), list) or not data["package"]:
            raise ValueError("unsupported or empty Cargo lock authority")
        for package in data["package"]:
            if not isinstance(package, dict):
                raise ValueError("invalid locked package")
            source, checksum = package.get("source"), package.get("checksum")
            if not isinstance(source, str) or not source.startswith("registry+"):
                if checksum is not None:
                    raise ValueError("only registry packages may authorize cached crate checksums")
                continue
            name, version = package.get("name"), package.get("version")
            if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*", name):
                raise ValueError("invalid registry package name")
            if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
                raise ValueError("invalid registry package version")
            if not isinstance(checksum, str) or not re.fullmatch(r"[0-9a-f]{64}", checksum):
                raise ValueError("every registry package requires a locked SHA256 checksum")
            filename = f"{name}-{version}.crate"
            if filename in expected and expected[filename] != checksum:
                raise ValueError("conflicting locked checksum authorities")
            expected[filename] = checksum
        identities.append({"path": str(lock), "sha256": hashlib.sha256(raw).hexdigest()})
    return expected, identities


def cache_path(cache_root: Path) -> Path:
    root = direct(cache_root, missing=True)
    if root.name != "cache" or root.parent.name != "registry":
        raise ValueError("cache root must name the registry/cache archive directory")
    if root.exists() and not root.is_dir():
        raise ValueError("registry cache root must be a directory")
    return root


def fingerprint(path: Path) -> tuple[int, int, int, int]:
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode):
        raise ValueError("cache archive must be a direct regular file")
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns


def verify(cache_root: Path, locks: list[Path]) -> dict:
    # Validate all authorities and all direct paths before attempting any repair.
    expected, identities = authorities(locks)
    root = cache_path(cache_root)
    candidates, files, unreferenced = [], [], 0
    if root.exists():
        for registry in sorted(root.iterdir()):
            direct(registry)
            if not registry.is_dir():
                raise ValueError("registry cache may contain direct registry directories only")
            for archive in sorted(registry.glob("*.crate")):
                direct(archive)
                if archive.name not in expected:
                    unreferenced += 1
                    continue
                snapshot = fingerprint(archive)
                actual = digest(archive)
                if snapshot != fingerprint(archive):
                    raise ValueError("cached archive changed while its digest was verified")
                candidates.append((archive, snapshot, actual))
    for lock in identities:
        if digest(direct(Path(lock["path"]))) != lock["sha256"]:
            raise ValueError("Cargo lock authority changed during cache verification")
    for archive, snapshot, actual in candidates:
        direct(archive)
        resolved = archive.resolve()
        relative = resolved.relative_to(root)
        if len(relative.parts) != 2 or snapshot != fingerprint(archive):
            raise ValueError("archive path or bytes changed before bounded cache repair")
        valid = actual == expected[archive.name]
        if not valid:
            # Delete exactly the revalidated archive; never purge a directory or
            # follow an unpacked-source/build/advisory path. Cargo redownloads it.
            archive.unlink()
        files.append({"archive": relative.as_posix(), "sha256": actual,
                      "expected_sha256": expected[archive.name],
                      "outcome": "verified" if valid else "removed-for-locked-redownload"})
    return {"schema_version": 1, "kind": "cargo-input-cache", "status": "passed", "files": files,
            "locks": identities, "locked_archive_count": len(expected), "unreferenced_archive_count": unreferenced,
            "scope": "downloaded registry archives only; no compiled artifact, source or advisory evidence reuse"}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-root", type=Path, default=Path("~/.cargo/registry/cache"))
    parser.add_argument("--lock", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        output = direct(args.output, missing=True)
        root = cache_path(args.cache_root)
        if output.is_relative_to(root) or output in [path.expanduser().absolute() for path in args.lock]:
            raise ValueError("cache receipt must be outside cached archives and lock authorities")
        output.parent.mkdir(parents=True, exist_ok=True)
        with output.open("x", encoding="utf-8") as stream:
            json.dump({"schema_version": 1, "kind": "cargo-input-cache", "status": "incomplete"}, stream)
    except (OSError, ValueError) as error:
        print(f"cache receipt must be fresh and safe: {error}", file=sys.stderr)
        return 1
    try:
        report = verify(root, args.lock)
        result = 0
    except (OSError, ValueError) as error:
        report = {"schema_version": 1, "kind": "cargo-input-cache", "status": "failed", "error": str(error)}
        print(error, file=sys.stderr)
        result = 1
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return result


if __name__ == "__main__":
    raise SystemExit(main())
