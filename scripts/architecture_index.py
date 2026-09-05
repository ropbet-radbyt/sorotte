#!/usr/bin/env python3
"""Validate current ownership/proof references and render their compact index.

This checks references, not test results. Only execution reports can attest a
candidate; an implementation or a valid index never implies hosted proof.
"""
from __future__ import annotations

import argparse
import ast
import pathlib
import re
import sys
import tomllib

CATALOG = "coverage/current-architecture.toml"
DOCUMENT = "docs/CURRENT_ARCHITECTURE.md"
SCOPE = "docs/audits/sorotte-post-v0.2.8-audit-2026-09-05.md"


class IndexError(ValueError):
    pass


def source(root: pathlib.Path, value: str) -> pathlib.Path:
    if not isinstance(value, str) or not value or "\\" in value:
        raise IndexError("invalid repository reference")
    relative = pathlib.PurePosixPath(value)
    if relative.is_absolute() or ".." in relative.parts or ":" in value:
        raise IndexError("reference escapes repository")
    path = (root / value).resolve()
    if not path.is_relative_to(root) or not path.is_file():
        raise IndexError(f"missing repository reference: {value}")
    return path


def read(path: pathlib.Path) -> str:
    with path.open("rb") as stream:
        raw = stream.read(2 * 1024 * 1024 + 1)
    if len(raw) > 2 * 1024 * 1024:
        raise IndexError(f"index input exceeds byte limit: {path.name}")
    return raw.decode("utf-8")


def nonempty_strings(values, label: str) -> None:
    if not isinstance(values, list) or not values or any(not isinstance(value, str) or not value.strip() for value in values):
        raise IndexError(f"{label} must contain nonempty strings")


def catalog_ids(value) -> set[str]:
    if isinstance(value, dict):
        result = {value["id"]} if isinstance(value.get("id"), str) else set()
        return result.union(*(catalog_ids(item) for item in value.values()))
    if isinstance(value, list):
        return set().union(*(catalog_ids(item) for item in value))
    return set()


def validate_proof(root: pathlib.Path, proof: dict) -> None:
    if not isinstance(proof, dict) or set(proof) != {"source", "symbol", "kind", "command"}:
        raise IndexError("proof must name source, symbol, kind, and command")
    if any(not isinstance(value, str) or not value.strip() for value in proof.values()):
        raise IndexError("proof fields must be nonempty strings")
    path = source(root, proof["source"])
    text = read(path)
    symbol = proof["symbol"]
    if proof["kind"] == "rust-test":
        match = re.search(r"\b(?:async\s+)?fn\s+" + re.escape(symbol) + r"\s*\(", text)
        if path.suffix != ".rs" or not match:
            raise IndexError(f"missing Rust proof symbol: {proof['source']}::{symbol}")
        attributes = text[max(0, match.start() - 500):match.start()]
        # Attribute region ends at the preceding item body. This admits ordinary,
        # async, ignored, and proptest tests but cannot relabel an arbitrary helper.
        attributes = attributes.rsplit("}", 1)[-1]
        if not re.search(r"#\[(?:\w+::)?test(?:\([^\n]*\))?\]", attributes):
            raise IndexError(f"Rust proof is not a test: {symbol}")
        if "#[ignore" in attributes and "--ignored" not in proof["command"]:
            raise IndexError(f"ignored proof command would not execute: {symbol}")
    elif proof["kind"] == "python-test":
        if path.suffix != ".py" or not symbol.startswith("test_") or not any(
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == symbol
            for node in ast.walk(ast.parse(text))
        ):
            raise IndexError(f"missing Python proof symbol: {proof['source']}::{symbol}")
    else:
        raise IndexError("unsupported proof kind")


def validate(root: pathlib.Path, data: dict | None = None) -> dict:
    root = root.resolve()
    data = data if data is not None else tomllib.loads(read(source(root, CATALOG)))
    if not isinstance(data, dict) or set(data) != {"schema_version", "release", "crate", "boundary"} or type(data["schema_version"]) is not int or data["schema_version"] != 1:
        raise IndexError("unsupported current architecture catalog")
    release = data["release"]
    if set(release) != {"version", "base_sha", "fixing_sha", "hosted", "note"} or not re.fullmatch(r"[0-9a-f]{40}", release["base_sha"]):
        raise IndexError("release must distinguish base, fixing, and hosted evidence")
    if release["fixing_sha"] != "pending" and not re.fullmatch(r"[0-9a-f]{40}", release["fixing_sha"]):
        raise IndexError("fixing SHA must be an actual commit or explicitly pending")
    workspace = tomllib.loads(read(source(root, "Cargo.toml")))["workspace"]
    if release["version"] != workspace["package"]["version"] or release["hosted"] not in {"pending", "recorded"}:
        raise IndexError("release version or hosted evidence state is invalid")
    crates = set()
    for crate in data["crate"]:
        if set(crate) != {"name", "responsibility"} or not all(isinstance(value, str) and value.strip() for value in crate.values()) or crate["name"] in crates:
            raise IndexError("invalid or duplicate crate responsibility")
        source(root, f"crates/{crate['name']}/Cargo.toml")
        crates.add(crate["name"])
    if crates != {path.removeprefix("crates/") for path in workspace["members"]}:
        raise IndexError("crate inventory does not match workspace members")
    identifiers, tasks = set(), set()
    for boundary in data["boundary"]:
        keys = {"id", "tasks", "contract", "owners", "normative", "proof", "catalog", "environment", "capability", "local_evidence", "remaining"}
        if not isinstance(boundary, dict) or set(boundary) != keys:
            raise IndexError("boundary must name ownership, normative contract, proof, environment, and evidence")
        for field in ("id", "contract", "capability", "local_evidence", "remaining"):
            if not isinstance(boundary[field], str) or not boundary[field].strip():
                raise IndexError(f"missing boundary {field}")
        if boundary["id"] in identifiers or boundary["capability"] not in {"implemented", "in-progress"}:
            raise IndexError("duplicate boundary or invalid capability state")
        identifiers.add(boundary["id"])
        for field in ("tasks", "owners", "normative", "environment"):
            nonempty_strings(boundary[field], field)
        tasks.update(boundary["tasks"])
        for reference in boundary["owners"] + boundary["normative"]:
            source(root, reference)
        if not isinstance(boundary["proof"], list) or not boundary["proof"]:
            raise IndexError("boundary must have an executable proof")
        for proof in boundary["proof"]:
            validate_proof(root, proof)
        if not isinstance(boundary["catalog"], list):
            raise IndexError("catalog references must be explicit")
        for reference in boundary["catalog"]:
            if set(reference) != {"path", "id"} or reference["id"] not in catalog_ids(tomllib.loads(read(source(root, reference["path"])))):
                raise IndexError(f"missing referenced catalog identity: {reference}")
    expected_tasks = set(re.findall(r"^### (A\d{2})\b", read(source(root, SCOPE)), re.MULTILINE))
    if tasks != expected_tasks:
        raise IndexError(f"audit task map is incomplete or unknown: {sorted(tasks ^ expected_tasks)}")
    return data


def link(path: str, label: str | None = None) -> str:
    return f"[{label or path}](../{path})"


def render(data: dict) -> str:
    release = data["release"]
    lines = ["# Current architecture and verification", "", "Generated from " + link(CATALOG) + ". Update that catalog, then run `python scripts/architecture_index.py --write`.", "", f"Release {release['version']}; landed base `{release['base_sha']}`; fixing commit **{release['fixing_sha']}**; hosted evidence **{release['hosted']}**. {release['note']}", "", "## Authority flow", "", "```mermaid", "flowchart LR", "  Intent[GUI / CLI local intent] --> Client[Client-core coordination]", "  Client --> Wire[Protocol and bounded transport]", "  Wire --> Server[Server room authority]", "  Server --> Wire", "  Wire --> Client", "  Client --> API[Player API commands]", "  API --> MPV[mpv adapter and owned process]", "  MPV --> Observation[Ordered physical observations]", "  Observation --> Client", "  Client --> Projection[GUI / CLI presentation]", "```", "", "Server state owns shared room order and canonical playback. Client-core maps that authority to ordered player commands and observations; it does not treat advisory status as playback authority. The mpv adapter owns physical process/IPC state. GUI and CLI own presentation and user intent. Settings transactions, network capacity, and evidence finalization have separate owners below.", "", "## Crate responsibilities", "", "| Crate | Responsibility |", "|---|---|"]
    for crate in data["crate"]:
        lines.append(f"| {link('crates/' + crate['name'] + '/Cargo.toml', crate['name'])} | {crate['responsibility']} |")
    lines += ["", "## Current invariants and executable proof", "", "Each local result identifies an implementation run. It is not a claim about a later modified candidate, another operating system, or hosted CI. Pending fixing commits stay explicit until a commit exists."]
    for boundary in data["boundary"]:
        lines += ["", f"### {boundary['id']} ({', '.join(boundary['tasks'])})", "", boundary["contract"], "", "- Owners: " + ", ".join(link(path) for path in boundary["owners"]) + ".", "- Normative: " + ", ".join(link(path) for path in boundary["normative"]) + "."]
        for proof in boundary["proof"]:
            lines.append(f"- Proof: {link(proof['source'], proof['symbol'])}; `{proof['command']}`.")
        if boundary["catalog"]:
            lines.append("- Catalogs: " + ", ".join(link(item["path"], item["id"]) for item in boundary["catalog"]) + ".")
        lines += ["- Environment: " + "; ".join(boundary["environment"]) + ".", f"- Capability: **{boundary['capability']}**. Local evidence: {boundary['local_evidence']}", "- Remaining proof: " + boundary["remaining"]]
    lines += ["", "## Historical material", "", "The chronological " + link("docs/TEST_COVERAGE_FINDINGS.md", "coverage findings") + ", " + link("docs/TEST_COVERAGE_STRATEGY.md", "coverage strategy") + ", and " + link("coverage/README.md", "coverage ledger") + " retain earlier decisions and evidence. Their old counts and remaining-work notes describe their recorded revisions. Use this current map and " + link("docs/DEVELOPMENT.md", "DEVELOPMENT") + " to locate today's owner and required execution command.", ""]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path("."))
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    try:
        root = args.repo_root.resolve()
        expected = render(validate(root))
        path = root / DOCUMENT
        if args.write:
            path.write_text(expected, encoding="utf-8", newline="\n")
        elif not path.is_file() or read(path).replace("\r\n", "\n") != expected:
            raise IndexError("current architecture document is stale; run --write")
        print("Current architecture references and generated index are valid")
        return 0
    except (IndexError, OSError, ValueError, SyntaxError) as error:
        print(f"architecture index failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
