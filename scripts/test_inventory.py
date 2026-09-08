#!/usr/bin/env python3
"""Discover, review and refresh stable test inventories from retained listings."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

try:
    from verification_tools import ROOT, identity, pins
except ModuleNotFoundError:
    from scripts.verification_tools import ROOT, identity, pins

REVIEWED = ROOT / "coverage/test-inventories.json"
SCOPES = {
    "compat": ["-p", "sorotte-compat", "--all-features"],
    "mpv-lib": ["-p", "sorotte-player-mpv", "--all-features", "--lib"],
    "gui-lib": ["-p", "sorotte-gui", "--all-features", "--lib"],
    "server-bin": ["-p", "sorotte-server", "--all-features", "--bin", "sorotte-server"],
    "media-lib": ["-p", "sorotte-media-match", "--all-features", "--lib"],
    "client-app-lib": ["-p", "sorotte-client-app", "--all-features", "--lib"],
    "updater-bin": ["-p", "sorotte-gui", "--all-features", "--bin", "sorotte-gui-updater"],
}


def unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate inventory key: {key}")
        result[key] = value
    return result


def load(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)


def names(value: object, label: str, *, allow_empty: bool = False) -> list[str]:
    if not isinstance(value, list) or (not value and not allow_empty):
        raise ValueError(f"{label} must be an explicit {'possibly empty' if allow_empty else 'nonempty'} name inventory")
    if any(not isinstance(name, str) or not name or any(char.isspace() or ord(char) < 32 for char in name)
           for name in value) or value != sorted(set(value)):
        raise ValueError(f"{label} must contain unique sorted test names")
    return value


def validate_scopes(value: dict) -> dict:
    if not isinstance(value.get("scopes"), dict) or set(value["scopes"]) != set(SCOPES):
        raise ValueError("inventory must contain every reviewed scope exactly once")
    for scope, arguments in SCOPES.items():
        entry = value["scopes"][scope]
        if not isinstance(entry, dict) or entry.get("cargo_scope") != arguments:
            raise ValueError(f"scope changed without reviewed inventory migration: {scope}")
        tests = names(entry.get("tests"), f"{scope} tests")
        ignored = names(entry.get("ignored"), f"{scope} ignored tests", allow_empty=True)
        if not set(ignored) <= set(tests):
            raise ValueError(f"ignored identities outside the reviewed tests: {scope}")
    return value


def validate(value: object) -> dict:
    """The reviewed file declares expectations; it never claims a test run passed."""
    if (not isinstance(value, dict) or set(value) != {"schema_version", "kind", "scopes"}
            or type(value.get("schema_version")) is not int or value["schema_version"] != 3
            or value["kind"] != "reviewed-test-inventory"):
        raise ValueError("reviewed inventory schema 3 must contain only kind and scopes; keep discovery receipts separately")
    validate_scopes(value)
    for scope, entry in value["scopes"].items():
        if set(entry) != {"cargo_scope", "tests", "ignored"}:
            raise ValueError(f"reviewed scope {scope} must contain only cargo_scope, tests and ignored")
    return value


def validate_discovery(value: object) -> dict:
    if not isinstance(value, dict) or type(value.get("schema_version")) is not int or value["schema_version"] != 2:
        raise ValueError("discovery receipt schema 2 with explicit ignored status is required")
    if value.get("status") != "passed":
        raise ValueError("incomplete or failed discovery cannot update reviewed expectations")
    return validate_scopes(value)


def review_document(discovery: dict) -> dict:
    validate_discovery(discovery)
    return validate({"schema_version": 3, "kind": "reviewed-test-inventory", "scopes": {
        scope: {key: discovery["scopes"][scope][key] for key in ("cargo_scope", "tests", "ignored")}
        for scope in SCOPES}})


def reviewed(scope: str) -> list[str]:
    """Keep the legacy complete-name/total API, including ignored test identities."""
    if scope not in SCOPES:
        raise ValueError(f"unknown inventory scope: {scope}")
    return validate(load(REVIEWED))["scopes"][scope]["tests"]


def listing(data: object, scope: str | None = None) -> dict:
    if not isinstance(data, dict) or not isinstance(data.get("rust-suites"), dict):
        raise ValueError("nextest inventory must contain rust-suites")
    tests, ignored = [], []
    for suite in data["rust-suites"].values():
        if not isinstance(suite, dict) or suite.get("status") != "listed" or not isinstance(suite.get("testcases"), dict):
            raise ValueError("every nextest suite must be completely listed")
        if scope is not None:
            args = SCOPES[scope]
            if suite.get("package-name") != args[1]:
                raise ValueError(f"nextest listed a package outside {scope}")
            if "--lib" in args and suite.get("kind") != "lib":
                raise ValueError(f"nextest listed a non-library target for {scope}")
            if "--bin" in args and (suite.get("kind") != "bin" or suite.get("binary-name") != args[-1]):
                raise ValueError(f"nextest listed the wrong binary for {scope}")
        for name, testcase in suite["testcases"].items():
            if not isinstance(testcase, dict) or testcase.get("kind") != "test" or type(testcase.get("ignored")) is not bool:
                raise ValueError("nextest test identity must include an explicit boolean ignored status")
            if testcase.get("filter-match") != {"status": "matches"}:
                raise ValueError("filtered nextest inventory cannot replace the complete reviewed scope")
            tests.append(name)
            if testcase["ignored"]:
                ignored.append(name)
    if len(set(tests)) != len(tests):
        raise ValueError("ambiguous cross-binary test inventory")
    names(sorted(tests), "nextest tests")
    if type(data.get("test-count")) is not int or data["test-count"] != len(tests):
        raise ValueError("nextest declared test count differs from its complete inventory")
    return {"tests": sorted(tests), "ignored": sorted(ignored)}


def flatten(data: dict) -> list[str]:
    return listing(data)["tests"]


def difference(before: list[str], after: list[str]) -> dict:
    # Renames remain removals plus additions: equal totals cannot hide them.
    return {"added": sorted(set(after) - set(before)), "removed": sorted(set(before) - set(after))}


def scope_difference(before: dict, after: dict) -> dict:
    retained = set(before["tests"]) & set(after["tests"])
    return {**difference(before["tests"], after["tests"]),
            "newly_ignored": sorted((set(after["ignored"]) - set(before["ignored"])) & retained),
            "no_longer_ignored": sorted((set(before["ignored"]) - set(after["ignored"])) & retained)}


def discovery_command(scope: str) -> list[str]:
    return ["cargo", "nextest", "list", "--locked", "--run-ignored", "all", "--ignore-default-filter",
            *SCOPES[scope], "--message-format", "json"]


def verify_version(version: str):
    expected = pins()["tools"]["cargo-nextest"]
    if not isinstance(version, str) or not re.match(r"^cargo-nextest " + re.escape(expected) + r"(?:[ (]|$)", version):
        raise ValueError(f"inventory requires cargo-nextest {expected}; observed {version!r}. "
                         f"Install with: cargo install cargo-nextest --version {expected} --locked")


def collect(output: Path) -> dict:
    output = output.absolute()
    if output.resolve() == REVIEWED.resolve() or (output.exists() and REVIEWED.exists() and output.samefile(REVIEWED)):
        raise ValueError("inventory output cannot overwrite reviewed authority; inspect the proposal diff first")
    listings = output.with_suffix(".listings")
    if output.exists() or output.is_symlink() or listings.exists() or listings.is_symlink():
        raise ValueError("inventory output must be fresh; preserve the previous attempt")
    output.parent.mkdir(parents=True, exist_ok=True)
    listings.mkdir()
    value = {"schema_version": 2, "status": "incomplete", "identity": None, "scopes": {}, "attempts": []}
    with output.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(value, indent=2) + "\n")
    started = time.monotonic()
    def save(): output.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    try:
        value["identity"] = identity()
        value["reviewed_inventory_sha256"] = hashlib.sha256(REVIEWED.read_bytes()).hexdigest()
        version = subprocess.check_output(["cargo", "nextest", "--version"], cwd=ROOT, text=True, timeout=30).strip()
        verify_version(version)
        value["cargo_nextest_version"] = version
        for name, scope in SCOPES.items():
            command = discovery_command(name)
            attempt = {"scope": name, "command": command, "status": "running"}
            value["attempts"].append(attempt)
            save()
            print(f"Listing reviewed scope {name}", flush=True)
            try:
                result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        check=True, timeout=1800)
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
                (listings / f"{name}.stdout.log").write_bytes(error.stdout or b"")
                (listings / f"{name}.stderr.log").write_bytes(error.stderr or b"")
                attempt["diagnostic_directory"] = str(listings)
                raise
            raw = result.stdout
            raw_path = listings / f"{name}.json"
            raw_path.write_bytes(raw)
            attempt["listing_path"] = str(raw_path)
            attempt["listing_sha256"] = hashlib.sha256(raw).hexdigest()
            (listings / f"{name}.stderr.log").write_bytes(result.stderr or b"")
            attempt["returncode"] = result.returncode
            collected = listing(json.loads(raw, object_pairs_hook=unique_object), name)
            value["scopes"][name] = {"cargo_scope": scope, **collected, "command": command,
                                      "listing_sha256": attempt["listing_sha256"]}
            attempt["status"] = "passed"
            save()
        value["identity_after"] = identity()
        if value["identity"] != value["identity_after"]:
            raise ValueError("input identity changed during inventory collection")
        value["status"] = "passed"
        validate_discovery(value)
        return value
    except BaseException as error:
        value.update(status="timed_out" if isinstance(error, subprocess.TimeoutExpired)
                     else "cancelled" if isinstance(error, KeyboardInterrupt) else "failed", error=str(error))
        if value["attempts"] and value["attempts"][-1]["status"] == "running":
            value["attempts"][-1].update(status=value["status"], error=str(error))
        raise
    finally:
        value["duration_seconds"] = round(time.monotonic() - started, 3)
        save()


def differences(before: dict, after: dict, *, as_json: bool = False) -> bool:
    changed = False
    for scope in SCOPES:
        delta = scope_difference(before["scopes"][scope], after["scopes"][scope])
        changed |= any(delta.values())
        if as_json:
            print(json.dumps({"scope": scope, **delta}))
        elif any(delta.values()):
            print(f"{scope}:")
            for key, names in delta.items():
                for name in names:
                    print(f"  {key}: {name}")
    if not as_json and not changed:
        print("Inventory unchanged.")
    return changed


def verify_proposal(path: Path, value: dict, reviewed_bytes: bytes):
    """Verify fresh discovery inputs, commands and actual raw listings before apply."""
    validate_discovery(value)
    current = identity()
    if (value.get("identity") != current or value.get("identity_after") != current
            or value.get("reviewed_inventory_sha256") != hashlib.sha256(reviewed_bytes).hexdigest()):
        raise ValueError("discovery source or reviewed inventory changed; collect a fresh proposal before refresh")
    verify_version(value.get("cargo_nextest_version"))
    attempts = value.get("attempts")
    if not isinstance(attempts, list) or len(attempts) != len(SCOPES):
        raise ValueError("discovery must retain exactly one completed attempt for every scope")
    directory = path.with_suffix(".listings")
    if directory.is_symlink() or directory.resolve() != directory:
        raise ValueError("discovery listings must use direct paths")
    for scope, attempt in zip(SCOPES, attempts):
        entry = value["scopes"][scope]
        command = discovery_command(scope)
        if (not isinstance(attempt, dict) or attempt.get("scope") != scope or attempt.get("status") != "passed"
                or type(attempt.get("returncode")) is not int or attempt["returncode"] != 0
                or attempt.get("command") != command or entry.get("command") != command):
            raise ValueError(f"{scope}: discovery execution mode or completion differs")
        raw_path = directory / f"{scope}.json"
        if raw_path.is_symlink() or raw_path.resolve() != raw_path or attempt.get("listing_path") != str(raw_path):
            raise ValueError(f"{scope}: discovery listing path differs or is indirect")
        raw = raw_path.read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        if attempt.get("listing_sha256") != digest or entry.get("listing_sha256") != digest:
            raise ValueError(f"{scope}: discovery listing bytes changed")
        actual = listing(json.loads(raw, object_pairs_hook=unique_object), scope)
        if actual != {key: entry[key] for key in ("tests", "ignored")}:
            raise ValueError(f"{scope}: proposal names or ignored status differ from the raw listing")
    if identity() != current:
        raise ValueError("source changed while validating discovery; collect a fresh proposal")


def refresh(path: Path) -> bool:
    path = path.absolute()
    if path.is_symlink() or path.resolve() != path or REVIEWED.is_symlink() or REVIEWED.resolve() != REVIEWED.absolute():
        raise ValueError("inventory refresh requires direct paths")
    before = REVIEWED.read_bytes()
    baseline = validate(json.loads(before, object_pairs_hook=unique_object))
    proposal = load(path)
    verify_proposal(path, proposal, before)
    document = review_document(proposal)
    if not differences(baseline, document):
        return False
    after = (json.dumps(document, indent=2) + "\n").encode("utf-8")
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=REVIEWED.parent, prefix=".inventory-", suffix=".tmp", delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(after)
        if REVIEWED.read_bytes() != before:
            raise ValueError("reviewed inventory changed during refresh; review a fresh proposal")
        temporary.replace(REVIEWED)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    print("Updated coverage/test-inventories.json; discovery receipts remain unchanged.")
    return True


def configure_parser(parser: argparse.ArgumentParser):
    sub = parser.add_subparsers(dest="inventory_command", required=True)
    propose = sub.add_parser("propose")
    propose.add_argument("--output", type=Path, required=True)
    diff = sub.add_parser("diff")
    diff.add_argument("--proposed", type=Path, required=True)
    diff.add_argument("--json", action="store_true", help="emit one machine-readable row per scope")
    check = sub.add_parser("check")
    check.add_argument("--output", type=Path, required=True)
    check.add_argument("--json", action="store_true")
    apply = sub.add_parser("refresh", help="verify current-source discovery and update reviewed expectations")
    apply.add_argument("--proposed", type=Path, required=True)


def execute(args: argparse.Namespace) -> int:
    try:
        if args.inventory_command == "propose":
            collect(args.output)
            print(f"Review: python scripts/verify.py inventory diff --proposed \"{args.output}\"")
            return 0
        if args.inventory_command == "refresh":
            refresh(args.proposed)
            return 0
        before = validate(load(REVIEWED))
        actual = collect(args.output) if args.inventory_command == "check" else validate_discovery(load(args.proposed))
        changed = differences(before, actual, as_json=args.json)
        if changed:
            proposed = args.output if args.inventory_command == "check" else args.proposed
            print(f"After reviewing the changes: python scripts/verify.py inventory refresh --proposed \"{proposed}\"", file=sys.stderr)
        return int(changed)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(error, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    configure_parser(parser)
    return execute(parser.parse_args(argv))


if __name__ == "__main__":
    raise SystemExit(main())
