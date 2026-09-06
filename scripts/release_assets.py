#!/usr/bin/env python3
"""Attach immutable release files and independently compare anonymous public bytes."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import artifact_input


class AssetError(ValueError):
    pass


def inventory(directory: Path) -> dict[str, dict]:
    files = {}
    for path in sorted(directory.iterdir()):
        if not path.is_file() or path.is_symlink() or not re.fullmatch(r"[A-Za-z0-9._-]+", path.name):
            raise AssetError("release inventory contains an unsafe or non-regular file")
        files[path.name] = {"sha256": artifact_input.sha256_file(path), "size": path.stat().st_size}
    if not files:
        raise AssetError("release artifact inventory is empty")
    return files


def public_digest(repository: str, tag: str, name: str, expected_size: int) -> dict:
    url = f"https://github.com/{repository}/releases/download/{urllib.parse.quote(tag, safe='')}/{urllib.parse.quote(name, safe='')}"
    request = urllib.request.Request(url, headers={"User-Agent": "sorotte-public-release-verifier/1"})
    result = hashlib.sha256()
    size = 0
    # Deliberately no GitHub token, gh download, or Actions artifact endpoint.
    with urllib.request.urlopen(request, timeout=90) as response:
        while chunk := response.read(1024 * 1024):
            size += len(chunk)
            if size > expected_size:
                raise AssetError(f"public release asset exceeds qualified size: {name}")
            result.update(chunk)
    return {"sha256": result.hexdigest(), "size": size}


def verify_public(repository: str, tag: str, directory: Path) -> dict:
    expected = inventory(directory)
    for name, identity in expected.items():
        if public_digest(repository, tag, name, identity["size"]) != identity:
            raise AssetError(f"anonymous public release asset differs from qualified bytes: {name}")
    return {"schema_version": 1, "kind": "sorotte-public-release-assets", "result": "passed", "repository": repository, "tag": tag, "files": expected}


def attach(repository: str, tag: str, directory: Path) -> None:
    expected = inventory(directory)
    result = subprocess.run(["gh", "release", "view", tag, "--repo", repository, "--json", "assets"], capture_output=True, text=True)
    if result.returncode:
        with tempfile.TemporaryDirectory(prefix="sorotte-release-notes-") as temporary:
            notes = Path(temporary) / "notes.md"
            notes.write_text(f"Sorotte {tag}. Archives are verified against the approved release qualification.\n", encoding="utf-8")
            subprocess.run(["gh", "release", "create", tag, "--repo", repository, "--title", tag, "--notes-file", str(notes), "--verify-tag"], check=False)
        # Concurrent GUI/server attachment can create the same release. Re-read
        # the authority; never ignore a failed create without an existing release.
        result = subprocess.run(["gh", "release", "view", tag, "--repo", repository, "--json", "assets"], capture_output=True, text=True, check=True)
    remote = json.loads(result.stdout)
    names = {asset["name"] for asset in remote["assets"]}
    missing = []
    for name, identity in expected.items():
        if name in names:
            if public_digest(repository, tag, name, identity["size"]) != identity:
                raise AssetError(f"immutable release asset already exists with different bytes: {name}")
        else:
            missing.append(str(directory / name))
    if missing:
        subprocess.run(["gh", "release", "upload", tag, "--repo", repository, *missing], check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("attach", "verify-public"))
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--artifacts-dir", required=True, type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository) or not re.fullmatch(r"(?:server-)?v[0-9][A-Za-z0-9._-]*", args.tag):
        parser.error("public verification requires an exact repository and stable version tag")
    try:
        if args.command == "attach":
            attach(args.repository, args.tag, args.artifacts_dir)
        else:
            value = verify_public(args.repository, args.tag, args.artifacts_dir)
            if args.report is None:
                raise AssetError("public verification requires a retained report")
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    except (ValueError, OSError, urllib.error.URLError, subprocess.SubprocessError) as error:
        print(f"release asset operation failed: {error}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
