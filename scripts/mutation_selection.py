#!/usr/bin/env python3
"""Select source-bound mutation evidence from immutable base/head inputs."""
from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import tomllib

import mutation_ci

CATALOG = "coverage/mutation-selection.toml"
POLICY = "coverage/mutation-policy.toml"
SHA = re.compile(r"[0-9a-f]{40}")


class SelectionError(ValueError):
    pass


def git(root: pathlib.Path, *args: str) -> bytes:
    completed = subprocess.run(["git", "-c", f"safe.directory={root.as_posix()}", "-C", str(root), *args], capture_output=True, timeout=60, check=False)
    if completed.returncode:
        raise SelectionError(completed.stderr.decode("utf-8", errors="replace"))
    return completed.stdout


def read_revision(root: pathlib.Path, revision: str, path: str) -> bytes | None:
    exists = git(root, "ls-tree", revision, "--", path)
    if not exists:
        return None
    raw = git(root, "show", f"{revision}:{path}")
    if len(raw) > 1024 * 1024:
        raise SelectionError(f"oversized selection input: {path}")
    return raw


def checkout_matches_blob(blob: bytes | None, checkout: bytes) -> bool:
    # Git's normal Windows checkout conversion is not a policy edit. Permit
    # only CRLF/LF transport differences, never arbitrary clean-filter changes.
    return blob is not None and blob.replace(b"\r\n", b"\n") == checkout.replace(b"\r\n", b"\n")


def selected_shards(policy: dict, catalog: dict, changed: list[str], *, full: bool) -> set[str]:
    changed = [path for path in changed if not path.endswith(".md")]
    if set(catalog) != {"schema_version", "global_inputs", "dependency"} or type(catalog["schema_version"]) is not int or catalog["schema_version"] != 1:
        raise SelectionError("invalid mutation selection catalog")
    shards = {value["id"]: value for value in policy["shard"]}
    if len(shards) != len(policy["shard"]):
        raise SelectionError("duplicate mutation shard")
    required = {item for group in policy["required_report_set"] for item in group["shards"]}
    if not required <= shards.keys():
        raise SelectionError("unknown mandatory mutation shard")
    def matches(patterns: list[str]) -> bool:
        if not isinstance(patterns, list) or not patterns or any(not isinstance(pattern, str) or not pattern or ".." in pathlib.PurePosixPath(pattern).parts or "\\" in pattern for pattern in patterns):
            raise SelectionError("invalid mutation dependency paths")
        return any(fnmatch.fnmatchcase(path, pattern) for path in changed for pattern in patterns)
    global_changed = matches(catalog["global_inputs"])
    selected = set(shards) if full or global_changed else set()
    for identifier, shard in shards.items():
        # Includes extracted files, selectors/tests, build.rs and feature manifests.
        if any(path.startswith(f"crates/{shard['package']}/") for path in changed):
            selected.add(identifier)
    for dependency in catalog["dependency"]:
        if set(dependency) != {"paths", "shards"} or not isinstance(dependency["shards"], list) or not dependency["shards"] or not set(dependency["shards"]) <= shards.keys():
            raise SelectionError("invalid or unknown mutation dependency shard")
        if matches(dependency["paths"]):
            selected.update(dependency["shards"])
    # Preserve the historical report set for every relevant production/tool change.
    # A docs-only selection remains empty even if this script is run manually.
    if selected or any(path.startswith("crates/") and not path.endswith(".md") for path in changed):
        selected.update(required)
    return selected


def plan(root: pathlib.Path, base: str, head: str, *, full: bool = False) -> dict:
    if not SHA.fullmatch(base) or not SHA.fullmatch(head):
        raise SelectionError("selection requires full immutable base/head SHAs")
    if git(root, "rev-parse", "HEAD").decode().strip() != head:
        raise SelectionError("selection head must equal checked-out HEAD")
    changed = sorted(set(git(root, "diff", "--name-only", "--no-renames", "-z", base, head).decode("utf-8").strip("\0").split("\0")) - {""})
    current = mutation_ci.load_policy(root, root / POLICY)
    head_policy, head_catalog = (root / POLICY).read_bytes(), (root / CATALOG).read_bytes()
    if not checkout_matches_blob(read_revision(root, head, POLICY), head_policy) or not checkout_matches_blob(read_revision(root, head, CATALOG), head_catalog):
        raise SelectionError("selection policies must match immutable checked-out head")
    union: set[str] = set()
    inputs = []
    for revision, origin in ((base, "base"), (head, "head")):
        raw_policy = read_revision(root, revision, POLICY)
        raw_catalog = read_revision(root, revision, CATALOG)
        if raw_policy is None:
            raise SelectionError("mutation policy missing at immutable revision")
        # Initial rollout has no base selection catalog. Its mandatory set is
        # still retained; head catalog adds risk-based selection immediately.
        policy_data = tomllib.loads(raw_policy.decode("utf-8"))
        catalog_data = tomllib.loads((raw_catalog or head_catalog).decode("utf-8"))
        if raw_catalog is None:
            catalog_data = {"schema_version": 1, "global_inputs": ["Cargo.lock", "Cargo.toml"], "dependency": []}
        union.update(selected_shards(policy_data, catalog_data, changed, full=full))
        inputs.append({"origin": origin, "revision": revision, "policy_sha256": hashlib.sha256(raw_policy).hexdigest(), "catalog_sha256": hashlib.sha256(raw_catalog).hexdigest() if raw_catalog else None})
    known = {shard.identifier for shard in current.shards}
    if not union <= known:
        raise SelectionError(f"head removed base-selected shards: {sorted(union - known)}")
    return {"schema_version": 1, "base": base, "head": head, "full": full, "changed": changed, "shards": sorted(union), "inputs": inputs}


def verify_selected(root: pathlib.Path, selection: dict, artifacts: pathlib.Path) -> None:
    expected = set(selection["shards"])
    discovered: dict[str, list[pathlib.Path]] = {}
    for path in artifacts.rglob("mutation-*.json"):
        if path.is_symlink() or not path.resolve().is_relative_to(artifacts.resolve()):
            raise SelectionError("mutation artifact path escapes artifact root")
        shard = path.name.removeprefix("mutation-").removesuffix(".json")
        discovered.setdefault(shard, []).append(path)
    if set(discovered) != expected or any(len(paths) != 1 for paths in discovered.values()):
        raise SelectionError(f"selected mutation reports must be unique and complete; expected {sorted(expected)}, found { {key: len(paths) for key, paths in discovered.items()} }")
    inventory_cache = mutation_ci.TestInventoryCache()
    for shard in sorted(expected):
        if mutation_ci.verify_report(argparse.Namespace(repo_root=str(root), policy=POLICY, shard=shard, report=str(discovered[shard][0])), inventory_cache=inventory_cache) != 0:
            raise SelectionError(f"selected mutation evidence failed: {shard}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("select", "verify"))
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path("."))
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--full", action="store_true")
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--artifacts", type=pathlib.Path)
    parser.add_argument("--github-output", type=pathlib.Path)
    args = parser.parse_args()
    try:
        root = args.repo_root.resolve()
        selection = plan(root, args.base, args.head, full=args.full)
        if args.command == "verify":
            if args.artifacts is None:
                raise SelectionError("--artifacts required for verification")
            verify_selected(root, selection, args.artifacts)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(selection, indent=2) + "\n", encoding="utf-8")
        if args.github_output:
            with args.github_output.open("a", encoding="utf-8") as stream:
                stream.write("shards=" + json.dumps(selection["shards"], separators=(",", ":")) + "\n")
        print(json.dumps(selection, separators=(",", ":")))
        return 0
    except (SelectionError, mutation_ci.MutationCiError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"mutation selection failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
