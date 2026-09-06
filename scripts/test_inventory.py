#!/usr/bin/env python3
"""Propose/diff reviewed libtest names and ignored status without accepting drift."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
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


def validate(value: object, *, require_passed: bool = True) -> dict:
    if not isinstance(value, dict) or type(value.get("schema_version")) is not int or value["schema_version"] != 2:
        raise ValueError("inventory schema 2 with explicit ignored status is required")
    if require_passed and value.get("status") != "passed":
        raise ValueError("incomplete or failed inventory cannot be used as authority")
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
        version = subprocess.check_output(["cargo", "nextest", "--version"], cwd=ROOT, text=True, timeout=30).strip()
        expected = pins()["tools"]["cargo-nextest"]
        if not re.match(r"^cargo-nextest " + re.escape(expected) + r"(?:[ (]|$)", version):
            raise ValueError("nextest inventory runtime does not match the reviewed pin")
        value["cargo_nextest_version"] = version
        for name, scope in SCOPES.items():
            command = ["cargo", "nextest", "list", "--locked", "--run-ignored", "all", "--ignore-default-filter",
                       *scope, "--message-format", "json"]
            attempt = {"scope": name, "command": command, "status": "running"}
            value["attempts"].append(attempt)
            save()
            print(f"Listing reviewed scope {name}", flush=True)
            result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, check=True, timeout=1800)
            raw = result.stdout
            raw_path = listings / f"{name}.json"
            raw_path.write_bytes(raw)
            attempt["listing_path"] = str(raw_path)
            attempt["listing_sha256"] = hashlib.sha256(raw).hexdigest()
            collected = listing(json.loads(raw, object_pairs_hook=unique_object), name)
            value["scopes"][name] = {"cargo_scope": scope, **collected, "command": command,
                                      "listing_sha256": attempt["listing_sha256"]}
            attempt["status"] = "passed"
            save()
        value["identity_after"] = identity()
        if value["identity"] != value["identity_after"]:
            raise ValueError("input identity changed during inventory collection")
        value["status"] = "passed"
        validate(value)
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


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    propose = sub.add_parser("propose")
    propose.add_argument("--output", type=Path, required=True)
    diff = sub.add_parser("diff")
    diff.add_argument("--proposed", type=Path, required=True)
    check = sub.add_parser("check")
    check.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "propose":
            collect(args.output)
            return 0
        before = validate(load(REVIEWED))
        actual = collect(args.output) if args.command == "check" else validate(load(args.proposed))
        changed = False
        for name in SCOPES:
            delta = scope_difference(before["scopes"][name], actual["scopes"][name])
            print(json.dumps({"scope": name, **delta}))
            changed |= any(delta.values())
        return int(changed)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(error, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
