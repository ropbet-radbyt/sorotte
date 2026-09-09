"""Collect the exact pre-merge archive bytes for immutable publication."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import sys

import artifact_input
import candidate_authority
import release_assets


def collect(inputs: list[Path], output: Path, source: str, *, server_only: bool, premerge: bool = False) -> dict:
    candidate_authority.sha(source, "archive candidate")
    files, producers, names = {}, set(), set()
    for directory in inputs:
        proof = artifact_input.strict_json_load(directory / ".authority/download.json", expected_type=dict)
        if (proof.get("kind") != "sorotte-candidate-artifact-download" or proof.get("status") != "passed"
                or proof.get("candidate_sha") != source):
            raise ValueError("archive input lacks the exact candidate download identity")
        if premerge:
            if proof.get("main_sha") is not None:
                raise ValueError("pre-merge handoff cannot claim a completed integration")
        else:
            candidate_authority.sha(proof.get("main_sha"), "qualified integration")
        producers.add((proof["qualification_run_id"], proof["qualification_run_attempt"], proof["main_sha"]))
        name = proof["artifact"]["name"]
        if name in names:
            raise ValueError("duplicate qualified archive input")
        names.add(name)
        entries = [path for path in directory.iterdir() if path.name != ".authority"]
        if any(path.is_symlink() or not path.is_file() for path in entries):
            raise ValueError("publication archive input must contain only regular files")
        actual = {path.name: {"sha256": artifact_input.sha256_file(path), "size": path.stat().st_size} for path in entries}
        if actual != proof.get("files") or not actual:
            raise ValueError("downloaded archive bytes changed before publication")
        for path in entries:
            if path.name.casefold() in files:
                raise ValueError("qualified publication files collide")
            files[path.name.casefold()] = path
    if len(producers) != 1:
        raise ValueError("archive inputs do not share one original candidate and integration authority")
    run_id, attempt, main_sha = next(iter(producers))
    expected = {"sorotte-server-ubuntu-24.04", "sorotte-server-windows-2025", f"durable-qualification-{run_id}-{attempt}"}
    if not server_only:
        expected.add("sorotte-gui-windows-x86_64")
    if names != expected:
        raise ValueError("publication omitted or added a candidate archive group")
    output.mkdir(parents=True, exist_ok=False)
    for path in files.values():
        shutil.copyfile(path, output / path.name)
    inventory = release_assets.inventory(output)
    for name, identity in inventory.items():
        if name.endswith(".sha256"):
            checksum = (output / name).read_text(encoding="utf-8").strip().split()
            if (len(checksum) != 2 or checksum[1] != name.removesuffix(".sha256")
                    or checksum[1] not in inventory or checksum[0] != inventory[checksum[1]]["sha256"]):
                raise ValueError("qualified archive checksum does not match its retained bytes")
        elif name.endswith((".zip", ".tar.gz")) and name + ".sha256" not in inventory:
            raise ValueError("qualified archive is missing its checksum sidecar")
    return {"schema_version": 1, "kind": "sorotte-qualified-publication-files", "status": "passed",
            "candidate_sha": source, "main_sha": main_sha, "qualification_run_id": run_id,
            "qualification_run_attempt": attempt, "files": inventory, "rebuilt": False,
            "application_tests_executed": False}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--input", action="append", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--server-only", action="store_true")
    args = parser.parse_args(argv)
    try:
        value = collect(args.input, args.output_dir, args.source_sha, server_only=args.server_only)
        candidate_authority.write(args.report, value)
        print(json.dumps({"status": "passed", "files": len(value["files"]), "rebuilt": False}))
        return 0
    except (ValueError, OSError, KeyError) as error:
        print(f"qualified archive collection failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
