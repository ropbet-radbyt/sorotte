#!/usr/bin/env python3
"""Exercise the installed coverage producer, subprocess runtime and object registration."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

from verification_tools import ROOT, digest, identity, pins
from diff_coverage import lexical_non_coverable_lines

LIB = '''pub fn ordinary() -> u8 {
    11 // CANARY_UNIT
}
pub fn missed() -> u8 {
    99 // CANARY_MISS
}
#[test]
fn ordinary_test() { assert_eq!(ordinary(), 11); }
pub fn multiline(flag: bool) -> u8 {
    if flag
        && [1, 2, 3]
            .iter()
            .any(|value| *value == 2)
    {
        44 // CANARY_MULTILINE
    } else {
        0
    }
}
#[test]
fn multiline_test() { assert_eq!(multiline(true), 44); }
fn tuple_pattern(first: Option<u8>, second: Option<u8>) -> u8 {
    let (Some(first), Some(second)) = (
        first, // PATTERN_TUPLE_FIRST_INPUT
        second, // PATTERN_TUPLE_SECOND_INPUT
    ) else { // STRUCTURAL_TUPLE_LET_ELSE
        return 0; // PATTERN_TUPLE_REJECT
    };
    first + second // PATTERN_TUPLE_ACCEPT
}
struct Payload { value: u8 }
fn nested_pattern(candidate: Option<Payload>) -> u8 {
    let Some(Payload { // STRUCTURAL_NESTED_PATTERN
        value,
    }) = candidate // PATTERN_NESTED_INPUT
    else {
        return 0; // PATTERN_NESTED_REJECT
    };
    value // PATTERN_NESTED_ACCEPT
}
#[test]
fn pattern_acceptance_and_rejection() {
    assert_eq!(tuple_pattern(Some(3), Some(4)), 7);
    assert_eq!(tuple_pattern(None, Some(4)), 0);
    assert_eq!(tuple_pattern(Some(3), None), 0);
    assert_eq!(nested_pattern(Some(Payload { value: 9 })), 9);
    assert_eq!(nested_pattern(None), 0);
}
'''
WORKER = '''fn child() -> u8 {
    22 // CANARY_CHILD
}
fn standalone() -> u8 {
    33 // CANARY_STANDALONE
}
fn main() {
    if std::env::args().any(|arg| arg == "--child") {
        assert_eq!(child(), 22);
    } else {
        assert_eq!(standalone(), 33);
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--child").status().unwrap().success());
    }
}
'''

PATTERN_EXECUTABLE_MARKERS = frozenset({
    "PATTERN_TUPLE_FIRST_INPUT", "PATTERN_TUPLE_SECOND_INPUT",
    "PATTERN_TUPLE_ACCEPT", "PATTERN_TUPLE_REJECT",
    "PATTERN_NESTED_INPUT", "PATTERN_NESTED_ACCEPT", "PATTERN_NESTED_REJECT",
})
PATTERN_STRUCTURAL_MARKERS = frozenset({
    "STRUCTURAL_TUPLE_LET_ELSE", "STRUCTURAL_NESTED_PATTERN",
})


def observations(report: dict, source: dict[str, str]) -> dict:
    found = {}
    seen = set()
    for unit in report["data"]:
        for entry in unit["files"]:
            filename = Path(entry["filename"]).name
            if filename not in source:
                continue
            if filename in seen:
                raise ValueError(f"duplicate coverage object/source: {filename}")
            seen.add(filename)
            locations = [tuple(segment[:2]) for segment in entry["segments"]]
            if len(locations) != len(set(locations)):
                raise ValueError("duplicate coverage segment location")
            for number, line in enumerate(source[filename].splitlines(), 1):
                if "// CANARY_" not in line:
                    continue
                marker = line.split("// ")[1]
                counts = [segment[2] for segment in entry["segments"]
                          if segment[0] == number and segment[3] is True and segment[4] is True]
                if counts:
                    found[marker] = max(counts)
    expected = {"CANARY_UNIT", "CANARY_CHILD", "CANARY_STANDALONE", "CANARY_MISS", "CANARY_MULTILINE"}
    if set(found) != expected:
        raise ValueError(f"coverage canary object/line inventory incomplete: {found}")
    if found["CANARY_MISS"] != 0 or any(found[name] <= 0 for name in expected - {"CANARY_MISS"}):
        raise ValueError(f"known coverage hit/miss contract failed: {found}")
    return found


def source_view_observations(text: str) -> dict:
    """Independent llvm-cov show consumer; does not reuse the JSON segment parser."""
    found = {}
    for line in text.splitlines():
        match = re.fullmatch(r"\s*\d+\|\s*(\d+)\|.*// (CANARY_[A-Z_]+)\s*", line)
        if match:
            if match[2] in found:
                raise ValueError("duplicate canary line in source view")
            found[match[2]] = int(match[1])
    if set(found) != {"CANARY_UNIT", "CANARY_CHILD", "CANARY_STANDALONE", "CANARY_MISS", "CANARY_MULTILINE"}:
        raise ValueError("source view is missing a required canary line")
    if found["CANARY_MISS"] != 0 or any(value <= 0 for key, value in found.items() if key != "CANARY_MISS"):
        raise ValueError("source view lost known positive/negative line coverage")
    return found


def pattern_observations(report: dict, text: str, source: dict[str, str]) -> dict:
    """Require real pattern execution and independently absent wrapper regions.

    A lexical exemption never substitutes for one of the required executable
    markers. Both LLVM representations must retain every initializer and both
    accept/reject branches before structural classification can pass.
    """
    expected = PATTERN_EXECUTABLE_MARKERS | PATTERN_STRUCTURAL_MARKERS
    locations = {}
    structural_lines = {}
    for filename, content in source.items():
        lines = content.splitlines()
        structural_lines[filename] = lexical_non_coverable_lines(lines, source=filename)
        for number, line in enumerate(lines, 1):
            match = re.search(r"// ((?:PATTERN|STRUCTURAL)_[A-Z_]+)\s*$", line)
            if match:
                if match[1] in locations:
                    raise ValueError("duplicate coverage pattern source marker")
                locations[match[1]] = (filename, number)
    if set(locations) != expected:
        raise ValueError("coverage pattern source marker inventory incomplete")
    entries = {}
    for unit in report["data"]:
        for entry in unit["files"]:
            filename = Path(entry["filename"]).name
            if filename in source:
                if filename in entries:
                    raise ValueError("duplicate coverage pattern source object")
                entries[filename] = entry
    text_counts = {}
    for line in text.splitlines():
        match = re.fullmatch(
            r"\s*(\d+)\|\s*(\d*)\|.*// ((?:PATTERN|STRUCTURAL)_[A-Z_]+)\s*", line
        )
        if match:
            marker = match[3]
            if marker in text_counts:
                raise ValueError("duplicate coverage pattern source-view marker")
            if marker not in locations or int(match[1]) != locations[marker][1]:
                raise ValueError("coverage pattern source-view line identity mismatch")
            text_counts[marker] = int(match[2]) if match[2] else None
    if set(text_counts) != expected:
        raise ValueError("coverage pattern source-view inventory incomplete")
    executable = {}
    structural = {}
    for marker, (filename, number) in locations.items():
        entry = entries.get(filename)
        if entry is None:
            raise ValueError("coverage pattern source object missing")
        counts = [segment[2] for segment in entry["segments"]
                  if segment[0] == number and segment[3] is True and segment[4] is True]
        if marker in PATTERN_EXECUTABLE_MARKERS:
            if not counts or max(counts) <= 0 or text_counts[marker] != max(counts):
                raise ValueError(f"coverage pattern executable mapping lost or disagrees: {marker}")
            executable[marker] = max(counts)
        else:
            if counts or text_counts[marker] is not None:
                raise ValueError(f"coverage pattern wrapper unexpectedly instrumented: {marker}")
            if number not in structural_lines[filename]:
                raise ValueError(f"coverage pattern wrapper misclassified as executable: {marker}")
            structural[marker] = {"line": number, "llvm_json": "absent", "llvm_text": "blank",
                                  "classification": "non-coverable"}
    return {"executable": executable, "structural": structural}


def run(output: Path) -> dict:
    if output.exists():
        raise ValueError("canary output must be fresh; previous evidence is immutable")
    output.mkdir(parents=True)
    started = time.monotonic()
    record = {"schema_version": 1, "kind": "coverage-tool-canary", "identity": identity(),
              "status": "incomplete", "commands": [], "variants": {}}
    receipt = output / "receipt.json"
    def save():
        receipt.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    save()
    try:
        version = subprocess.check_output(["cargo", "llvm-cov", "--version"], text=True).strip()
        if version != f"cargo-llvm-cov {pins()['tools']['cargo-llvm-cov']}":
            raise ValueError(f"wrong coverage tool: {version}")
        for variant, newline in (("lf", "\n"), ("crlf", "\r\n")):
            fixture = output / variant
            (fixture / "src/bin").mkdir(parents=True)
            (fixture / "Cargo.toml").write_text('[package]\nname="sorotte-coverage-canary"\nversion="0.0.0"\nedition="2024"\n[workspace]\n', encoding="utf-8")
            (fixture / "Cargo.lock").write_text('version = 4\n[[package]]\nname = "sorotte-coverage-canary"\nversion = "0.0.0"\n', encoding="utf-8")
            for path, text in ((fixture / "src/lib.rs", LIB), (fixture / "src/bin/worker.rs", WORKER)):
                path.write_bytes(text.replace("\n", newline).encode())
            inputs = {str(path.relative_to(fixture)): digest(path) for path in
                      (fixture / "src/lib.rs", fixture / "src/bin/worker.rs", fixture / "Cargo.toml", fixture / "Cargo.lock")}
            env = dict(os.environ)
            for key in list(env):
                if key.startswith(("LLVM_", "CARGO_LLVM_COV", "__CARGO_LLVM_COV")) or key in ("RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC_WRAPPER", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"):
                    env.pop(key, None)
            for command in (
                ["cargo", "llvm-cov", "--locked", "--lib", "--no-report"],
                ["cargo", "llvm-cov", "run", "--locked", "--no-clean", "--bin", "worker", "--output-path", "worker-coverage.txt"],
                ["cargo", "llvm-cov", "report", "--json", "--output-path", "coverage.json"],
                ["cargo", "llvm-cov", "report", "--text", "--output-path", "source-view.txt"],
            ):
                record["commands"].append({"cwd_variant": variant, "argv": command, "status": "running"})
                save()
                log = fixture / f"command-{len(record['commands'])}.log"
                with log.open("wb") as stream:
                    result = subprocess.run(command, cwd=fixture, env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=180)
                record["commands"][-1].update(status="passed" if result.returncode == 0 else "failed", log_sha256=digest(log))
                if result.returncode:
                    raise ValueError(f"coverage tool canary failed: {log}")
            report = json.loads((fixture / "coverage.json").read_text(encoding="utf-8"))
            record["variants"][variant] = observations(report, {"lib.rs": LIB, "worker.rs": WORKER})
            view = source_view_observations((fixture / "source-view.txt").read_text(encoding="utf-8"))
            if view != record["variants"][variant] or any(digest(fixture / path) != expected for path, expected in inputs.items()):
                raise ValueError("coverage views disagree or source inputs changed")
            record.setdefault("artifacts", {})[variant] = {"inputs": inputs,
                "json_sha256": digest(fixture / "coverage.json"), "source_view_sha256": digest(fixture / "source-view.txt")}
            record.setdefault("pattern_variants", {})[variant] = pattern_observations(
                report, (fixture / "source-view.txt").read_text(encoding="utf-8"),
                {"lib.rs": LIB, "worker.rs": WORKER},
            )
        record["status"] = "passed"
        return record
    except BaseException as error:
        record.update(status="failed", error=str(error))
        raise
    finally:
        record["duration_seconds"] = round(time.monotonic() - started, 3)
        save()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        run(args.output.resolve())
        print(f"coverage producer canary passed: {args.output}")
        return 0
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(error, file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
