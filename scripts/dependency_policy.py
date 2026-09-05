#!/usr/bin/env python3
"""Pinned dependency checks with explicit unavailable evidence and input binding."""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tomllib

from artifact_input import strict_json_load, strict_json_loads, sha256_file

POLICY = "coverage/dependency-policy.toml"
DATABASE = "https://github.com/RustSec/advisory-db"
MAX_OUTPUT = 64 * 1024 * 1024


class DependencyError(ValueError):
    pass


def strict_json(text: str):
    return strict_json_loads(text, max_bytes=MAX_OUTPUT, expected_type=dict, label="dependency evidence")


def digest(path: pathlib.Path) -> str:
    return sha256_file(path)


def run(argv: list[str], root: pathlib.Path, *, env: dict | None = None) -> subprocess.CompletedProcess:
    result = subprocess.run(argv, cwd=root, capture_output=True, text=True, encoding="utf-8", errors="strict", timeout=900, env=env, check=False)
    if len(result.stdout.encode()) + len(result.stderr.encode()) > MAX_OUTPUT:
        raise DependencyError("scanner output exceeds limit")
    return result


def checked(argv: list[str], root: pathlib.Path, **kwargs) -> str:
    result = run(argv, root, **kwargs)
    if result.returncode:
        raise DependencyError(f"{argv[0]} failed ({result.returncode}): {result.stderr[-2000:]}")
    return result.stdout


def git(root: pathlib.Path, *args: str) -> str:
    return checked(["git", "-c", f"safe.directory={root.as_posix()}", "-C", str(root), *args], root).strip()


def load_policy(root: pathlib.Path, *, today: dt.date | None = None) -> dict:
    data = tomllib.loads((root / POLICY).read_text(encoding="utf-8"))
    if set(data) != {"schema_version", "cargo_deny_version", "pip_audit_version", "advisory_database", "exception"} or type(data["schema_version"]) is not int or data["schema_version"] != 1 or data["advisory_database"] != DATABASE:
        raise DependencyError("unsupported dependency policy")
    for field in ("cargo_deny_version", "pip_audit_version"):
        if not isinstance(data[field], str) or not re.fullmatch(r"\d+\.\d+\.\d+", data[field]):
            raise DependencyError("checker must be an exact pinned version")
    seen = set()
    if not isinstance(data["exception"], list):
        raise DependencyError("exceptions must be a list")
    for item in data["exception"]:
        if not isinstance(item, dict) or set(item) != {"id", "ecosystem", "rationale", "owner", "expires"} or not all(isinstance(value, str) and value.strip() for value in item.values()):
            raise DependencyError("exception requires identity, ecosystem, rationale, owner and expiry")
        if item["ecosystem"] not in {"rust", "python"} or not re.fullmatch(r"(?:RUSTSEC|PYSEC|CVE)-\d{4}-\d+|GHSA-[a-z0-9-]+", item["id"]):
            raise DependencyError("invalid exception identity")
        if item["id"] in seen:
            raise DependencyError("duplicate dependency exception")
        seen.add(item["id"])
        if dt.date.fromisoformat(item["expires"]) <= (today or dt.datetime.now(dt.timezone.utc).date()):
            raise DependencyError(f"expired dependency exception {item['id']}")
    return data


def inputs(root: pathlib.Path) -> list[dict]:
    paths = {root / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "deny.toml", POLICY, "scripts/dependency_policy.py", "scripts/artifact_input.py", "coverage/native-components.toml")}
    paths.update((root / "crates").glob("*/Cargo.toml"))
    paths.update((root / "requirements").glob("*.txt"))
    for extra in (".cargo/config.toml", "Dockerfile.server", "scripts/package-gui-release.ps1", "scripts/package-server-release.ps1"):
        if (root / extra).is_file():
            paths.add(root / extra)
    return [{"path": path.relative_to(root).as_posix(), "sha256": digest(path)} for path in sorted(paths)]


def summarize_deny(text: str, returncode: int) -> dict:
    records = [strict_json(line) for line in text.splitlines() if line.strip()]
    summaries = [record.get("fields") for record in records if record.get("type") == "summary"]
    if len(summaries) != 1 or not isinstance(summaries[0], dict) or set(summaries[0]) != {"advisories", "sources"}:
        raise DependencyError("scanner did not produce exactly one complete advisory/source summary")
    for check in summaries[0].values():
        if not isinstance(check, dict) or set(check) != {"errors", "helps", "notes", "warnings"} or any(type(value) is not int or value < 0 for value in check.values()):
            raise DependencyError("invalid scanner summary counters")
    errors = sum(check["errors"] for check in summaries[0].values())
    if (returncode == 0) != (errors == 0):
        raise DependencyError("scanner exit status contradicts its summary")
    findings = []
    for record in records:
        if record.get("type") == "diagnostic":
            fields = record["fields"]
            advisory = fields.get("advisory", {})
            findings.append({"id": advisory.get("id"), "package": advisory.get("package"), "code": fields.get("code"), "severity": fields.get("severity"), "message": fields.get("message"), "packages": [item.get("span") for item in fields.get("labels", [])]})
    return {"status": "passed" if errors == 0 else "failed", "summary": summaries[0], "findings": findings}


def rust_scan(root: pathlib.Path, out: pathlib.Path, tool: str, policy: dict) -> dict:
    version = checked([tool, "--version"], root).strip()
    if version != f"cargo-deny {policy['cargo_deny_version']}":
        raise DependencyError(f"unexpected cargo-deny identity: {version}")
    checked(["cargo", "fetch", "--locked"], root)
    db = out / "advisory-dbs"
    env = os.environ.copy()
    env["SOROTTE_ADVISORY_DB"] = str(db)
    config_text = (root / "deny.toml").read_text(encoding="utf-8")
    config = tomllib.loads(config_text)
    if "ignore" in config.get("advisories", {}):
        raise DependencyError("advisory exceptions must use the reviewed expiring policy")
    ignored = [item["id"] for item in policy["exception"] if item["ecosystem"] == "rust"]
    config_text = config_text.replace("[advisories]", "[advisories]\nignore = " + json.dumps(ignored))
    config_path = out / "effective-deny.toml"
    config_path.write_text(config_text, encoding="utf-8")
    # A successful online fetch is mandatory for this run. Offline cache hits
    # never masquerade as current advisory evidence.
    fetch_command = [tool, "--config", str(config_path), "fetch", "db"]
    fetched = run(fetch_command, root, env=env)
    (out / "rustsec-fetch.txt").write_text(fetched.stdout + fetched.stderr, encoding="utf-8")
    if fetched.returncode:
        raise DependencyError("required RustSec fetch unavailable (see rustsec-fetch.txt)")
    repos = [path for path in db.iterdir() if path.is_dir() and (path / ".git").exists()]
    if len(repos) != 1:
        raise DependencyError("advisory database identity is ambiguous")
    database = {"url": git(repos[0], "remote", "get-url", "origin"), "revision": git(repos[0], "rev-parse", "HEAD"), "commit_time": git(repos[0], "show", "-s", "--format=%cI", "HEAD"), "fetched_at": dt.datetime.now(dt.timezone.utc).isoformat()}
    if database["url"].removesuffix(".git") != DATABASE or not re.fullmatch(r"[0-9a-f]{40}", database["revision"]) or git(repos[0], "status", "--porcelain"):
        raise DependencyError("RustSec database identity or checkout is invalid")
    command = [tool, "--format", "json", "--config", str(config_path), "--locked", "--offline", "check", "advisories", "sources"]
    result = run(command, root, env=env)
    raw = result.stdout + result.stderr
    (out / "cargo-deny.jsonl").write_text(raw, encoding="utf-8")
    summary = summarize_deny(raw, result.returncode)
    if git(repos[0], "rev-parse", "HEAD") != database["revision"]:
        raise DependencyError("advisory database changed during scan")
    executable = shutil.which(tool)
    if executable is None:
        raise DependencyError("scanner executable identity unavailable")
    return {**summary, "checker": version, "checker_sha256": digest(pathlib.Path(executable)), "database": database, "command": command, "raw_sha256": digest(out / "cargo-deny.jsonl"), "effective_config_sha256": digest(config_path)}


def python_scan(root: pathlib.Path, out: pathlib.Path, python: str, policy: dict) -> dict:
    version = checked([python, "-m", "pip_audit", "--version"], root).strip()
    if version != f"pip-audit {policy['pip_audit_version']}":
        raise DependencyError(f"unexpected pip-audit identity: {version}")
    command = [python, "-m", "pip_audit", "--format", "json", "--progress-spinner", "off", "--strict", "--disable-pip", "--no-deps"]
    # Resolve transitive verification dependencies first with pip's dry-run
    # report. The exact resolved versions are then queried without installing
    # production requirements or executing their build scripts in this process.
    requirements = sorted((root / "requirements").glob("*.txt"))
    resolve_command = [python, "-m", "pip", "install", "--dry-run", "--ignore-installed", "--only-binary=:all:", "--report", str(out / "python-resolve.json")]
    for path in requirements:
        resolve_command += ["-r", str(path)]
    checked(resolve_command, root)
    resolved = strict_json_load(out / "python-resolve.json", max_bytes=MAX_OUTPUT, expected_type=dict, label="resolved Python dependencies")
    locked = out / "python-resolved.txt"
    locked.write_text("".join(f"{item['metadata']['name']}=={item['metadata']['version']}\n" for item in resolved["install"]), encoding="utf-8")
    command += ["-r", str(locked)]
    for item in policy["exception"]:
        if item["ecosystem"] == "python":
            command += ["--ignore-vuln", item["id"]]
    result = run(command, root)
    (out / "pip-audit.json").write_text(result.stdout, encoding="utf-8")
    (out / "pip-audit.stderr.txt").write_text(result.stderr, encoding="utf-8")
    data = strict_json(result.stdout)
    if not isinstance(data, dict) or not isinstance(data.get("dependencies"), list) or any("skip_reason" in item for item in data["dependencies"]):
        raise DependencyError("Python advisory evidence missing or incomplete")
    findings = [dict(package=item["name"], version=item["version"], **finding) for item in data["dependencies"] for finding in item["vulns"]]
    if result.returncode not in (0, 1) or (result.returncode == 0) != (not findings):
        raise DependencyError("Python scanner status contradicts findings")
    # PyPI's vulnerability API is live and exposes no immutable DB revision.
    # Preserve exact response bytes/time; never claim a RustSec-style DB commit.
    return {"status": "failed" if findings else "passed", "checker": version, "database": {"service": "https://pypi.org/pypi/{package}/{version}/json", "revision": None, "queried_at": dt.datetime.now(dt.timezone.utc).isoformat()}, "response_sha256": digest(out / "pip-audit.json"), "resolution_sha256": digest(out / "python-resolve.json"), "findings": findings, "command": command}


def inventory(root: pathlib.Path, package: str, target: str, payload: pathlib.Path, output: pathlib.Path) -> dict:
    before = inputs(root)
    tree_command = ["cargo", "tree", "--locked", "-p", package, "--target", target, "--edges", "normal,build", "--prefix", "none", "--format", "{p}"]
    tree = checked(tree_command, root)
    selected = set()
    for line in tree.splitlines():
        match = re.match(r"([a-zA-Z0-9_-]+) v([^ ]+)", line)
        if not match:
            raise DependencyError(f"unrecognized resolved dependency: {line}")
        selected.add(match.groups())
    metadata = strict_json(checked(["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target], root))
    packages, notices = [], ["Sorotte third-party dependency inventory", "", "Declared SPDX expressions and upstream notice files; this inventory does not make a legal license-compliance conclusion.", ""]
    for item in metadata["packages"]:
        if (item["name"], item["version"]) not in selected:
            continue
        package_dir = pathlib.Path(item["manifest_path"]).parent
        notice_files = sorted({path for pattern in ("LICENSE*", "COPYING*", "NOTICE*", "license*", "copying*") for path in package_dir.glob(pattern) if path.is_file()})
        licenses = [{"path": path.name, "sha256": digest(path)} for path in notice_files]
        packages.append({"name": item["name"], "version": item["version"], "source": item["source"], "license": item["license"], "repository": item["repository"], "notice_files": licenses})
        notices += [f"{item['name']} {item['version']}", f"Declared license: {item['license'] or 'unspecified'}", f"Source: {item['repository'] or item['source'] or 'Sorotte workspace'}"]
        for path in notice_files:
            if path.stat().st_size > 1024 * 1024:
                raise DependencyError("upstream notice exceeds byte limit")
            notices += [f"--- {path.name} ---", path.read_text(encoding="utf-8", errors="replace")]
        notices.append("")
    if len(packages) != len(selected):
        raise DependencyError("dependency source/version identity is ambiguous")
    files = [{"path": path.relative_to(payload).as_posix(), "sha256": digest(path)} for path in sorted(payload.rglob("*")) if path.is_file() and path.name not in {"DEPENDENCIES.json", "THIRD-PARTY-NOTICES.txt", "manifest.json", "sorotte-install.json"}]
    if not files or inputs(root) != before:
        raise DependencyError("release payload missing or dependency inputs changed")
    result = {"schema": "sorotte-dependency-inventory-v1", "package": package, "target": target, "features": "default", "dependency_kinds": ["normal", "build"], "source_sha": git(root, "rev-parse", "HEAD"), "inputs": before, "payload": files, "resolution_command": tree_command, "resolution_sha256": hashlib.sha256(tree.encode()).hexdigest(), "packages": sorted(packages, key=lambda value: (value['name'], value['version'])), "native_components": tomllib.loads((root / "coverage/native-components.toml").read_text(encoding="utf-8"))}
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    (output.parent / "THIRD-PARTY-NOTICES.txt").write_text("\n".join(notices), encoding="utf-8")
    return result


def validate_inventory(value: dict, *, payload_hashes: dict[str, str], expected_package: str, expected_source_sha: str) -> None:
    """Independent package oracle: bind inventory to actual archive member hashes.

    Callers parse bytes with artifact_input, then pass the hashes they computed
    independently from the archive. The inventory excludes its own two files
    and the outer package/install manifest to avoid circular digests.
    """
    keys = {"schema", "package", "target", "features", "dependency_kinds", "source_sha", "inputs", "payload", "resolution_command", "resolution_sha256", "packages", "native_components"}
    if not isinstance(value, dict) or set(value) != keys or value["schema"] != "sorotte-dependency-inventory-v1" or value["package"] != expected_package or value["source_sha"] != expected_source_sha:
        raise DependencyError("release dependency inventory identity mismatch")
    if not isinstance(value["target"], str) or value["target"] not in {"x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"} or value["features"] != "default" or value["dependency_kinds"] != ["normal", "build"]:
        raise DependencyError("release dependency inventory target/features mismatch")
    def mappings(records):
        if not isinstance(records, list) or not records:
            raise DependencyError("release dependency inventory is empty")
        result = {}
        for record in records:
            if not isinstance(record, dict) or set(record) != {"path", "sha256"} or not isinstance(record["path"], str) or not record["path"] or "\\" in record["path"] or pathlib.PurePosixPath(record["path"]).is_absolute() or ".." in pathlib.PurePosixPath(record["path"]).parts or record["path"] in result or not isinstance(record["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", record["sha256"]):
                raise DependencyError("invalid or duplicate dependency inventory binding")
            result[record["path"]] = record["sha256"]
        return result
    declared_payload = mappings(value["payload"])
    expected_payload = {path: sha for path, sha in payload_hashes.items() if path not in {"DEPENDENCIES.json", "THIRD-PARTY-NOTICES.txt", "manifest.json", "sorotte-install.json"}}
    if declared_payload != expected_payload:
        raise DependencyError("dependency inventory does not bind the actual package payload")
    bound_inputs = mappings(value["inputs"])
    if not {"Cargo.toml", "Cargo.lock", f"crates/{expected_package}/Cargo.toml", POLICY, "coverage/native-components.toml"} <= bound_inputs.keys():
        raise DependencyError("dependency inventory omits a release graph input")
    if value["resolution_command"] != ["cargo", "tree", "--locked", "-p", expected_package, "--target", value["target"], "--edges", "normal,build", "--prefix", "none", "--format", "{p}"] or not isinstance(value["resolution_sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", value["resolution_sha256"]):
        raise DependencyError("dependency inventory resolution contract mismatch")
    packages = value["packages"]
    if not isinstance(packages, list) or not packages:
        raise DependencyError("resolved dependency inventory is empty")
    seen = set()
    for package in packages:
        if not isinstance(package, dict) or set(package) != {"name", "version", "source", "license", "repository", "notice_files"} or not isinstance(package["name"], str) or not isinstance(package["version"], str) or (package["name"], package["version"]) in seen:
            raise DependencyError("invalid or duplicate resolved dependency")
        seen.add((package["name"], package["version"]))
        if package["source"] is not None and package["source"] != "registry+https://github.com/rust-lang/crates.io-index":
            raise DependencyError("unapproved dependency source in release inventory")
        if not package["name"] or not package["version"] or any(package[key] is not None and not isinstance(package[key], str) for key in ("license", "repository")) or not isinstance(package["notice_files"], list):
            raise DependencyError("invalid resolved dependency metadata")
        if package["notice_files"]:
            mappings(package["notice_files"])
    if not any(name == expected_package for name, _ in seen):
        raise DependencyError("release root package absent from dependency inventory")
    native = value["native_components"]
    if not isinstance(native, dict) or set(native) != {"schema_version", "component"} or type(native["schema_version"]) is not int or native["schema_version"] != 1 or not isinstance(native["component"], list):
        raise DependencyError("invalid native component inventory")
    names = set()
    for item in native["component"]:
        if not isinstance(item, dict) or set(item) != {"name", "delivery", "identity", "upstream"} or not all(isinstance(text, str) and text.strip() for text in item.values()) or item["name"] in names or not item["upstream"].startswith("https://"):
            raise DependencyError("invalid or duplicate native component")
        names.add(item["name"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("scan", "inventory"))
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path("."))
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--cargo-deny", default="cargo-deny")
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--ecosystem", choices=("rust", "python", "all"), default="all")
    parser.add_argument("--package", choices=("sorotte-gui", "sorotte-server"))
    parser.add_argument("--target")
    parser.add_argument("--payload", type=pathlib.Path)
    args = parser.parse_args()
    root, output = args.repo_root.resolve(), args.output.resolve()
    report = {"schema": "sorotte-dependency-check-v1", "status": "unavailable", "checks": {}, "errors": []}
    try:
        policy = load_policy(root)
        if args.command == "inventory":
            if not args.package or not args.target or not args.payload:
                raise DependencyError("inventory requires package, target and payload")
            inventory(root, args.package, args.target, args.payload.resolve(), output)
            return 0
        output.parent.mkdir(parents=True, exist_ok=True)
        report.update({"inputs": inputs(root), "source_sha": git(root, "rev-parse", "HEAD"), "exceptions": policy["exception"]})
        for ecosystem in (["rust", "python"] if args.ecosystem == "all" else [args.ecosystem]):
            try:
                report["checks"][ecosystem] = rust_scan(root, output.parent, args.cargo_deny, policy) if ecosystem == "rust" else python_scan(root, output.parent, args.python, policy)
            except (DependencyError, OSError, ValueError, subprocess.SubprocessError) as error:
                report["checks"][ecosystem] = {"status": "unavailable", "error": str(error)}
        if inputs(root) != report["inputs"]:
            raise DependencyError("dependency inputs changed during the scan")
        statuses = {check["status"] for check in report["checks"].values()}
        report["status"] = "unavailable" if "unavailable" in statuses else "failed" if "failed" in statuses else "passed"
    except (DependencyError, OSError, ValueError, subprocess.SubprocessError) as error:
        report["errors"].append(str(error))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"dependency evidence: {report['status']} ({output})")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
