#!/usr/bin/env python3
"""Validate the reviewed byte inventory and replay the retained 0.2.9 crash cheaply."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import re
import stat
import subprocess
import sys
import time

try:
    from .verification_tools import ROOT, digest, identity
except ImportError:
    from verification_tools import ROOT, digest, identity

MANIFEST = ROOT / "coverage/fuzz-corpora.json"
TARGET_DIRECTORIES = {
    "protocol_line": "crates/sorotte-protocol/tests/corpus/protocol_parser",
    "framed_session": "crates/sorotte-cli/tests/corpus/framed_session",
    "mpv_framed_transcript": "crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript",
}
# A manifest edit cannot silently remove the historical defect or rewrite its input.
REQUIRED_REGRESSIONS = {
    "v0.2.9-zero-row-scope": {
        "package": "sorotte-client-core",
        "test": "session::tests::participant_status_tests::participant_status_replays_fuzzed_zero_transport_row_scope",
        "seed": "crates/sorotte-cli/tests/corpus/framed_session/participant-status-zero-row-scope.txt",
        "sha256": "b54f01dd1ad6e8295ce13dcadd6e0ec422dbe458bd46dca100521cdc4b11e071",
    },
}


def direct_path(root: Path, relative: str) -> Path:
    path = root
    for component in relative.split("/"):
        if component in ("", ".", "..") or "\\" in component or ":" in component:
            raise ValueError("corpus paths must be repository-relative direct paths")
        path /= component
        attributes = path.lstat()
        if stat.S_ISLNK(attributes.st_mode) or getattr(attributes, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0):
            raise ValueError("corpus must contain direct paths, not symlinks")
    path.resolve().relative_to(root.resolve())
    return path


def require_keys(value: object, keys: set[str], label: str) -> None:
    if not isinstance(value, dict) or set(value) != keys:
        raise ValueError(f"invalid {label} schema")


def unique_json_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key in corpus manifest: {key}")
        result[key] = value
    return result


def validate(*, root: Path = ROOT, manifest: Path | None = None) -> dict:
    reviewed = direct_path(root, "coverage/fuzz-corpora.json")
    if manifest is not None and manifest.absolute() != reviewed.absolute():
        raise ValueError("fuzz corpus authority must be the reviewed repository manifest")
    data = json.loads(reviewed.read_text(encoding="utf-8"), object_pairs_hook=unique_json_object)
    require_keys(data, {"schema_version", "targets", "regressions"}, "corpus manifest")
    if type(data["schema_version"]) is not int or data["schema_version"] != 1:
        raise ValueError("unsupported corpus manifest schema")
    if not isinstance(data["targets"], list) or len(data["targets"]) != len(TARGET_DIRECTORIES):
        raise ValueError("corpus manifest must include every supported target exactly once")
    seen_targets = set()
    reviewed_files = {}
    for target in data["targets"]:
        require_keys(target, {"id", "directory", "files"}, "corpus target")
        target_id = target["id"]
        if not isinstance(target_id, str) or target_id not in TARGET_DIRECTORIES or target_id in seen_targets:
            raise ValueError("unknown or duplicate corpus target")
        seen_targets.add(target_id)
        if target["directory"] != TARGET_DIRECTORIES[target_id]:
            raise ValueError("target seed directory differs from reviewed corpus authority")
        if not isinstance(target["files"], list) or not target["files"]:
            raise ValueError("each corpus must contain a nonempty reviewed inventory")
        previous_name = ""
        for entry in target["files"]:
            require_keys(entry, {"name", "bytes", "sha256"}, "corpus file")
            name = entry["name"]
            if not isinstance(name, str) or not name or name <= previous_name or "/" in name:
                raise ValueError("corpus filenames must be unique, sorted direct names")
            direct_path(root, target["directory"] + "/" + name)
            if type(entry["bytes"]) is not int or entry["bytes"] < 0:
                raise ValueError("corpus byte counts must be nonnegative integers")
            if not isinstance(entry["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]):
                raise ValueError("corpus hashes must be SHA256 identities")
            previous_name = name
            reviewed_files[target["directory"] + "/" + name] = entry
        directory = direct_path(root, target["directory"])
        files = sorted(directory.iterdir())
        if any(path.is_symlink() or not path.is_file() for path in files):
            raise ValueError("corpus must contain direct regular files only")
        actual = [{"name": path.name, "bytes": path.stat().st_size, "sha256": digest(path)} for path in files]
        if actual != target["files"]:
            raise ValueError(f"reviewed corpus identity changed: {target['id']}; preserve original crash bytes and review the manifest diff")
    if not isinstance(data["regressions"], list) or not data["regressions"]:
        raise ValueError("retained product regressions must not be empty")
    seen_regressions = set()
    for regression in data["regressions"]:
        require_keys(regression, {"id", "package", "test", "seed"}, "regression")
        for key, pattern in (("id", r"[a-z0-9][a-z0-9.-]*"), ("package", r"sorotte-[a-z0-9-]+"),
                             ("test", r"[a-zA-Z_][a-zA-Z0-9_]*(?:::[a-zA-Z_][a-zA-Z0-9_]*)+")):
            if not isinstance(regression[key], str) or not re.fullmatch(pattern, regression[key]):
                raise ValueError(f"invalid regression {key}")
        if regression["id"] in seen_regressions:
            raise ValueError("duplicate regression id")
        seen_regressions.add(regression["id"])
        if not isinstance(regression["seed"], str) or regression["seed"] not in reviewed_files:
            raise ValueError("regression seed must belong to the reviewed corpus inventory")
        required = REQUIRED_REGRESSIONS.get(regression["id"])
        if required and (any(regression[key] != required[key] for key in ("package", "test", "seed"))
                         or reviewed_files[regression["seed"]]["sha256"] != required["sha256"]):
            raise ValueError("required product regression identity or original crash bytes changed")
    if not set(REQUIRED_REGRESSIONS).issubset(seen_regressions):
        raise ValueError("required product regression is missing")
    return data


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("validate", "replay"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    result = {"schema_version": 1, "kind": "fuzz-regressions", "identity": None,
              "status": "incomplete", "commands": [], "attempts": []}
    started = time.monotonic()
    def save():
        if args.output:
            args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    if args.output:
        try:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            with args.output.open("x", encoding="utf-8") as stream:
                stream.write(json.dumps(result, indent=2) + "\n")
        except OSError as error:
            print(f"regression evidence must be fresh and writable: {error}", file=sys.stderr)
            return 1
    try:
        result["identity"] = identity()
        manifest = validate()
        result.update(corpus_manifest_sha256=digest(MANIFEST), regression_count=len(manifest["regressions"]),
                      corpus_files=sum(len(target["files"]) for target in manifest["targets"]))
        save()
        if args.command == "replay":
            for regression in manifest["regressions"]:
                command = ["cargo", "nextest", "run", "--locked", "-p", regression["package"], "--lib",
                           "--no-tests", "fail", "-E", f"test(={regression['test']})"]
                result["commands"].append(command)
                attempt = {"id": regression["id"], "status": "running"}
                result["attempts"].append(attempt)
                save()
                try:
                    subprocess.run(command, cwd=ROOT, check=True, timeout=300)
                    attempt["status"] = "passed"
                except BaseException as error:
                    attempt.update(status="timed_out" if isinstance(error, subprocess.TimeoutExpired)
                                   else "cancelled" if isinstance(error, KeyboardInterrupt) else "failed", error=str(error))
                    raise
        result["status"] = "passed"
        print(f"fuzz regression {args.command} passed ({result['corpus_files']} immutable seeds)")
        return 0
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        result.update(status="timed_out" if isinstance(error, subprocess.TimeoutExpired) else "failed", error=str(error))
        print(error, file=sys.stderr)
        return 1
    except KeyboardInterrupt as error:
        result.update(status="cancelled", error=str(error))
        return 130
    finally:
        result["duration_seconds"] = round(time.monotonic() - started, 3)
        save()


if __name__ == "__main__": raise SystemExit(main())
