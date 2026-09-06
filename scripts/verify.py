#!/usr/bin/env python3
"""Supported verification front door. Static preflight does not compile Rust."""
from __future__ import annotations

import argparse
import ast
from datetime import datetime, timezone
import fnmatch
import json
import hashlib
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import tomllib

from verification_tools import ROOT, digest, git, identity, pins, validate_pin_projections, verify_legacy

POLICY = ROOT / "coverage/verification-lanes.json"
DISPOSITIONS = {"product-defect", "harness-defect", "environment-unavailable", "assertion-gap", "unclassified"}
STATIC_VALIDATORS = (
    ("critical-boundaries", "scripts/critical_boundaries.py", "--repo-root", "."),
    ("architecture-index", "scripts/architecture_index.py", "--repo-root", "."),
    ("behavior-catalog", "scripts/behavior_evidence.py", "validate", "--catalog", "coverage/behaviors.toml"),
    ("lifecycle-model", "scripts/playback_lifecycle_model.py", "validate", "--model", "coverage/playback-lifecycle.toml"),
    ("ignored-tests", "scripts/ignored_test_policy.py", "validate", "--registry", "coverage/ignored-tests.toml"),
    ("known-defects", "scripts/known_defect_policy.py", "validate", "--registry", "coverage/known-defects.toml", "--catalog", "coverage/behaviors.toml"),
    ("nextest-policy", "scripts/nextest_ci.py", "validate"),
    ("mutation-targets", "scripts/mutation_campaign.py", "validate"),
    ("fuzz-corpus", "scripts/fuzz_regressions.py", "validate"),
)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def write(path: Path | None, value: dict) -> None:
    text = json.dumps(value, indent=2, sort_keys=True) + "\n"
    if path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    else:
        print(text, end="")


def matches(path: str, patterns: list[str]) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)


def select(paths: list[str], policy: dict) -> list[dict]:
    if len({lane["id"] for lane in policy["lanes"]}) != len(policy["lanes"]):
        raise ValueError("duplicate lane identity")
    broad = any(matches(path, policy["apparatus_patterns"]) for path in paths)
    unknown = any(not matches(path, policy["documentation_patterns"] + policy["apparatus_patterns"])
                  and not any(matches(path, lane.get("patterns", [])) for lane in policy["lanes"])
                  for path in paths)
    return [dict(lane, selected=bool(lane.get("always") or broad or unknown or
                                   any(matches(path, lane.get("patterns", [])) for path in paths)),
                 reason="apparatus changed" if broad else "unclassified input" if unknown else "path policy")
            for lane in policy["lanes"]]


def plan(base: str, head: str) -> dict:
    base = git("rev-parse", "--verify", f"{base}^{{commit}}")
    head = git("rev-parse", "--verify", f"{head}^{{commit}}")
    paths = git("diff", "--name-only", "--no-renames", "-z", base, head).split("\0")
    paths = sorted(path for path in paths if path)
    policy = json.loads(POLICY.read_text(encoding="utf-8"))
    lanes = select(paths, policy)
    # A changed policy cannot remove old obligations. Missing old policy means broad qualification.
    try:
        previous = json.loads(git("show", f"{base}:coverage/verification-lanes.json"))
        previous_lanes = {lane["id"]: lane for lane in select(paths, previous)}
    except (subprocess.CalledProcessError, ValueError, KeyError):
        previous_lanes = {}
        if "coverage/verification-lanes.json" in paths:
            for lane in lanes:
                lane.update(selected=True, reason="new policy requires full qualification")
    for lane in lanes:
        if previous_lanes.get(lane["id"], {}).get("selected"):
            lane.update(selected=True, reason="base or candidate policy")
    if previous_lanes.keys() - {lane["id"] for lane in lanes}:
        raise ValueError("removed lane requires explicit migration; base obligations remain required")
    return {"schema_version": 1, "kind": "verification-plan", "base_sha": base,
            "source_sha": head, "policy_sha256": digest(POLICY), "paths": paths, "lanes": lanes,
            "required_checks": policy["required_checks"], "created_at": now()}


def preflight(phase: str, requested_tools: list[str], legacy: Path | None) -> dict:
    started = time.monotonic()
    checks = []
    def check(name, action):
        check_started = time.monotonic()
        try:
            detail = action()
            checks.append({"id": name, "status": "passed", "detail": detail})
        except Exception as error:
            checks.append({"id": name, "status": "failed", "detail": str(error)})
        checks[-1]["duration_seconds"] = round(time.monotonic() - check_started, 3)
    def syntax():
        count = 0
        for folder in ("scripts", "fuzz"):
            for path in (ROOT / folder).rglob("*.py"):
                if "__pycache__" not in path.parts:
                    ast.parse(path.read_text(encoding="utf-8-sig"), filename=str(path))
                    count += 1
        return f"{count} Python sources parsed without imports or bytecode writes"
    def manifests():
        paths = [ROOT / "Cargo.toml", ROOT / "rust-toolchain.toml", *sorted((ROOT / "coverage").glob("*.toml"))]
        for path in paths:
            tomllib.loads(path.read_text(encoding="utf-8"))
        policy = json.loads(POLICY.read_text(encoding="utf-8"))
        if policy.get("schema_version") != 1 or not policy.get("required_checks") or not policy.get("gate_producers"):
            raise ValueError("lane policy requires reviewed checks and producers")
        for lane in policy["lanes"]:
            if not all(lane.get(key) for key in ("id", "owner", "command")):
                raise ValueError("every lane needs an identity, owner and replay command")
        select([], policy)
        for workflow in policy["required_checks"].values():
            if not (ROOT / workflow).is_file():
                raise ValueError(f"missing required workflow: {workflow}")
        from native_harness_canary import validate_inventory
        validate_inventory(json.loads((ROOT / "coverage/native-harness-canaries.json").read_text(encoding="utf-8")))
        from assurance_registry import evaluate
        evaluate(json.loads((ROOT / "coverage/assurance-capabilities.json").read_text(encoding="utf-8")), datetime.now(timezone.utc))
        return f"{len(paths)} TOML manifests, lane policy, native and assurance registries validated"
    def writable_temp():
        try:
            Path(tempfile.gettempdir()).resolve().relative_to(ROOT)
        except ValueError:
            pass
        else:
            raise ValueError("TEMP is inside the checkout; semver immutable exports require an external writable temp directory")
        with tempfile.TemporaryDirectory(prefix="sorotte-preflight-") as folder:
            path = Path(folder) / "rename-source"
            path.write_bytes(b"canary")
            path.rename(path.with_name("rename-destination"))
        return "create, write, rename and cleanup passed"
    def loopback():
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            with socket.create_connection(listener.getsockname(), timeout=2) as client:
                peer, _ = listener.accept()
                with peer:
                    peer.settimeout(2)
                    client.sendall(b"ok")
                    if peer.recv(2) != b"ok":
                        raise ValueError("loopback payload mismatch")
        return "owned IPv4 socket exchange and cleanup passed"
    def python_version():
        minimum = tuple(map(int, pins()["tools"]["python-min"].split(".")))
        maximum = tuple(map(int, pins()["tools"]["python-max-exclusive"].split(".")))
        if not minimum <= sys.version_info[:2] < maximum:
            raise ValueError(f"Python {platform.python_version()} outside reviewed range")
        return platform.python_version()
    check("python-version", python_version)
    check("python-syntax", syntax)
    check("manifests", manifests)
    check("tool-projections", validate_pin_projections)
    check("temporary-files", writable_temp)
    check("loopback", loopback)
    def process_control():
        process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(20)"],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            if os.name == "nt":
                killed = subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
                if killed.returncode:
                    raise ValueError("Windows owned process-tree control unavailable: taskkill was denied; run verification with normal process permissions")
            else:
                process.terminate()
            process.wait(timeout=5)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
        return "owned child termination and wait passed"
    check("process-control", process_control)
    for name, *arguments in STATIC_VALIDATORS:
        def validate(arguments=arguments):
            command = [sys.executable, *arguments]
            result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True,
                                    encoding="utf-8", errors="replace", timeout=45)
            if result.returncode:
                raise ValueError(f"replay: {' '.join(command)}; {(result.stderr or result.stdout).strip()[-4000:]}")
            return result.stdout.strip()[-2000:]
        check(name, validate)
    if legacy:
        check("legacy-reference", lambda: verify_legacy(legacy))
    if phase == "tools":
        def imports():
            import importlib.metadata
            import yaml
            wanted = pins()["python"]["PyYAML"]
            if importlib.metadata.version("PyYAML") != wanted:
                raise ValueError(f"install reviewed requirements/ci-policy.txt (PyYAML {wanted})")
            return "reviewed PyYAML import passed"
        check("policy-imports", imports)
        for tool in requested_tools or ["rust", "cargo-nextest"]:
            def probe(tool=tool):
                wanted = pins()["tools"][tool]
                command = ["rustc", "--version"] if tool == "rust" else ["cargo", tool.removeprefix("cargo-"), "--version"]
                actual = subprocess.check_output(command, text=True, encoding="utf-8", stderr=subprocess.STDOUT, timeout=30).strip()
                if wanted not in actual.split():
                    raise ValueError(f"expected {tool} {wanted}, got {actual}")
                return actual
            check(tool, probe)
    return {"schema_version": 1, "kind": "preflight", "phase": phase,
            "identity": identity(), "checks": checks, "created_at": now(),
            "replay_command": [sys.executable, "scripts/verify.py", "preflight", "--phase", phase,
                               *[item for tool in requested_tools for item in ("--tool", tool)],
                               *(["--legacy", str(legacy)] if legacy else [])],
            "duration_seconds": round(time.monotonic() - started, 3),
            "status": "passed" if all(c["status"] == "passed" for c in checks) else "failed"}


def receipt_source_claims(data: dict) -> dict[str, str]:
    """Locate reviewed subject fields, never infer a source from an arbitrary hash.

    This checks attribution only. Producer-specific gates still own status,
    coverage completeness, input freshness and execution authority validation.
    """
    if not isinstance(data, dict):
        raise ValueError("receipt must be a JSON object")
    claims = {}

    def claim(path: tuple[str, ...], *, required: bool = False) -> None:
        value = data
        label = ".".join(path)
        for key in path:
            if not isinstance(value, dict):
                raise ValueError(f"malformed receipt source container: {label}")
            if key not in value:
                if required:
                    raise ValueError(f"receipt source is missing: {label}")
                return
            value = value[key]
        if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{40}", value):
            raise ValueError(f"malformed receipt source SHA: {label}")
        claims[label] = value

    for path in (("source_sha",), ("candidate_sha",), ("identity", "source_sha")):
        claim(path)
    kind = data.get("kind")
    schemas = {
        "sorotte-behavior-evidence-shard": (1, (("sha",),)),
        "sorotte-behavior-evidence-aggregate": (1, (("sha",),)),
        "sorotte-compat-live-interop": (1, (("source", "commit_sha"), ("source", "expected_commit_sha"))),
        "sorotte-mutation-evidence": (3, (("git", "head_sha"),)),
        "sorotte-windows-process-coverage-lanes": (1, (("source_identity", "head_commit"),)),
        "sorotte-coverage-base": (1, (("verification_sha",), ("verification_sha_input",))),
        "sorotte-diff-coverage": (1, (("inputs", "head_sha"),)),
    }
    if isinstance(kind, str) and kind in schemas:
        version, fields = schemas[kind]
        if type(data.get("schema_version")) is not int or data["schema_version"] != version:
            raise ValueError(f"unsupported source attribution schema: {kind}")
        for path in fields:
            claim(path, required=True)
    elif kind in ("sorotte-mutation-campaign", "sorotte-mutation-required"):
        if type(data.get("schema_version")) is not int or data["schema_version"] != 1:
            raise ValueError("unsupported mutation subject schema")
        # Failed preparation emits head before a selection is available; complete
        # campaigns and finalizers bind the head again inside their selection.
        if kind == "sorotte-mutation-required" or "head" in data:
            claim(("head",), required=True)
        if "selection" in data or "head" not in data:
            claim(("selection", "head"), required=True)
    elif kind == "sorotte-coverage-ci-evidence":
        if type(data.get("schema_version")) is not int or data["schema_version"] != 2:
            raise ValueError("unsupported coverage aggregate subject schema")
        phases = data.get("phases")
        if not isinstance(phases, dict):
            raise ValueError("coverage receipt phases are missing")
        for phase, expected_kind in (("resolve-base", "sorotte-coverage-base"), ("diff-policy", "sorotte-diff-coverage")):
            item = phases.get(phase, {})
            if not isinstance(item, dict):
                raise ValueError("coverage receipt phase is malformed")
            nested = item.get("report")
            if nested is not None:
                if not isinstance(nested, dict) or nested.get("kind") != expected_kind:
                    raise ValueError("coverage receipt contains an unrecognized source report")
                for label, value in receipt_source_claims(nested).items():
                    claims[f"phases.{phase}.report.{label}"] = value
    elif kind is None and type(data.get("schema_version")) is int and data["schema_version"] == 1 and set(data) == {
        "schema_version", "base", "head", "full", "changed", "shards", "inputs"
    }:
        # The current mutation-selection producer has no kind discriminator.
        claim(("head",), required=True)

    if kind == "sorotte-playback-release-candidate-bundle":
        claim(("build_inputs", "candidate_sha"), required=True)
    if kind == "sorotte-playback-lifecycle-system":
        claim(("prerequisites", "candidate_sha"))
        claim(("prerequisites", "candidate_attestation", "checkout_sha"))

    # Existing archive verifiers use camelCase and no kind discriminator.
    if kind is None and type(data.get("schemaVersion")) is int and data["schemaVersion"] == 1:
        package = data.get("package")
        if isinstance(package, dict) and package.get("name") in ("sorotte-gui", "sorotte-server"):
            claim(("package", "sourceSha"), required=True)
            claim(("expectedSourceSha",))
        elif data.get("status") == "failed" and isinstance(data.get("error"), str):
            allowed = {"schemaVersion", "status", "expectedSourceSha", "expectedChannel", "error", "source_sha", "candidate_sha", "identity"}
            if set(data) <= allowed:
                claim(("expectedSourceSha",), required=True)
    if (kind is None and data.get("status") in ("PASS", "FAIL")
            and data.get("stage") in ("All", "Prepare", "Behavior")
            and isinstance(data.get("steps"), list) and isinstance(data.get("legacyOracle"), dict)):
        claim(("sourceSha",), required=True)
    if not claims:
        raise ValueError("receipt has no recognized source attribution; use its source-bound parent receipt")
    if len(set(claims.values())) != 1:
        raise ValueError(f"conflicting receipt source claims: {claims}")
    return claims


def ledger(paths: list[Path], expected: str) -> dict:
    if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{40}", expected):
        raise ValueError("ledger requires a full lowercase source SHA")
    def unique_keys(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate receipt JSON key: {key}")
            result[key] = value
        return result
    entries = []
    for path in paths:
        raw = path.read_bytes()
        data = json.loads(raw.decode("utf-8-sig"), object_pairs_hook=unique_keys)
        claims = receipt_source_claims(data)
        if set(claims.values()) != {expected}:
            raise ValueError(f"receipt source mismatch or missing: {path}")
        entries.append({"path": str(path), "sha256": hashlib.sha256(raw).hexdigest(), "source_claims": claims, "receipt": data})
    if not entries:
        raise ValueError("a ledger cannot attest an empty receipt set")
    return {"schema_version": 1, "kind": "receipt-index", "source_sha": expected,
            "created_at": now(), "entries": entries,
            "note": "Index only; each gate independently validates its obligations and producer authority."}


def gate(lane: str, selected: bool, results: list[str], expected_jobs: list[str], plan_path: Path | None,
         expected_base: str | None = None, expected_source: str | None = None) -> dict:
    pairs = [value.split("=", 1) for value in results]
    if any(len(pair) != 2 for pair in pairs) or len(dict(pairs)) != len(pairs):
        raise ValueError("job outcomes must be unique name=result pairs")
    outcomes = dict(pairs)
    policy = json.loads(POLICY.read_text(encoding="utf-8"))
    required_producers = policy["gate_producers"].get(lane)
    if not required_producers or set(expected_jobs) != set(required_producers) or set(outcomes) != set(required_producers):
        raise ValueError("required producer inventory is missing or different")
    source = git("rev-parse", "HEAD")
    if expected_source is not None and source != expected_source:
        raise ValueError("gate checkout differs from externally supplied candidate SHA")
    if plan_path:
        if not expected_base or not expected_source:
            raise ValueError("selection requires external event base and source authority")
        planned = json.loads(plan_path.read_text(encoding="utf-8"))
        if planned["base_sha"] != git("rev-parse", "--verify", f"{expected_base}^{{commit}}") or planned["source_sha"] != expected_source:
            raise ValueError("selection receipt does not match external event base/source")
        # Recompute with the original immutable subjects and both policy versions.
        fresh = plan(planned["base_sha"], planned["source_sha"])
        for key in ("base_sha", "source_sha", "policy_sha256", "paths", "lanes", "required_checks"):
            if fresh[key] != planned[key]:
                raise ValueError(f"selection receipt drift: {key}")
        if fresh["source_sha"] != git("rev-parse", "HEAD"):
            raise ValueError("selection is for another source")
        applicable = next(item["selected"] for item in fresh["lanes"] if item["id"] == lane)
        if applicable and not selected:
            raise ValueError("selected obligation cannot become a no-op")
    elif not selected:
        raise ValueError("no-op requires a verified immutable change plan")
    expected = "success" if selected else "skipped"
    if any(value != expected for value in outcomes.values()):
        raise ValueError(f"{lane} requires {expected} producers: {outcomes}")
    return {"schema_version": 1, "kind": "required-gate", "source_sha": git("rev-parse", "HEAD"),
            "lane": lane, "status": "passed", "selected": selected, "job_results": outcomes,
            "selection_sha256": digest(plan_path) if plan_path else None,
            "policy_sha256": digest(POLICY), "created_at": now()}


def primary_failure(result: subprocess.CompletedProcess) -> str:
    lines = [line.strip() for line in (result.stdout + "\n" + result.stderr).splitlines()]
    # Negative self-tests intentionally print child errors. Prefer the owning
    # suite's failed test over those expected diagnostics when discovery fails.
    for prefixes in (("FAIL:", "ERROR:"), ("FAIL ", "policy violation:"), ("error:", "thread '")):
        found = next((line for line in lines if line.startswith(prefixes)), None)
        if found:
            return found
    return f"producer exit {result.returncode}"


def run_lane(lane: str, output: Path, deadline: int) -> dict:
    from mutation_process import run as owned_run
    if output.exists():
        raise ValueError("attempt directory already exists; preserve it and choose a fresh output")
    commands = {
        "static": [sys.executable, "-m", "unittest", "discover", "-s", "scripts/tests", "-p", "test_*.py"],
        "behavior": [sys.executable, "scripts/nextest_ci.py", "run", "--repo-root", "."],
        "regression": [sys.executable, "scripts/fuzz_regressions.py", "replay"],
        "inventory": [sys.executable, "scripts/test_inventory.py", "check", "--output", str(output / "inventory.json")],
        "coverage-canary": [sys.executable, "scripts/coverage_tool_canary.py", "--output", str(output / "canary")],
    }
    command = commands[lane]
    output.mkdir(parents=True)
    record = {"schema_version": 1, "kind": "verification-attempt", "identity": identity(),
              "lane": lane, "status": "incomplete", "command": command, "created_at": now(),
              "disposition": "unclassified", "primary_failure": None, "operator_interventions": [],
              "replay_command": [sys.executable, "scripts/verify.py", "run", "--lane", lane, "--output", "FRESH_ATTEMPT_DIRECTORY"]}
    receipt = output / "receipt.json"
    write(receipt, record)
    started = time.monotonic()
    environment = dict(os.environ)
    # Hosted Windows supplies TEMP through an 8.3 alias. Canonicalize only
    # the child's temporary paths so fixtures and resolved evidence paths use
    # the same spelling, without changing the invoking shell or global state.
    for name in ("TMPDIR", "TEMP", "TMP"):
        if environment.get(name):
            environment[name] = str(Path(environment[name]).resolve())
    # Process-local Git trust only. Nested tests and wrappers inherit the exact
    # checkout exception; neither global configuration nor other worktrees change.
    count = int(environment.get("GIT_CONFIG_COUNT", "0"))
    environment.update(GIT_CONFIG_COUNT=str(count + 1))
    environment[f"GIT_CONFIG_KEY_{count}"] = "safe.directory"
    environment[f"GIT_CONFIG_VALUE_{count}"] = ROOT.as_posix()
    try:
        result = owned_run(command, cwd=ROOT, env=environment, timeout_seconds=deadline,
                           log_root=output / "process", label=lane)
        record["status"] = "passed" if result.returncode == 0 else "failed"
        if result.returncode:
            record["primary_failure"] = primary_failure(result)
        if record["identity"] != identity():
            record.update(status="failed", primary_failure="source or input drift during execution")
    except BaseException as error:
        record.update(status="failed", primary_failure=str(error))
        raise
    finally:
        record["duration_seconds"] = round(time.monotonic() - started, 3)
        process_receipt = output / "process/process.json"
        record["process_receipt_sha256"] = digest(process_receipt) if process_receipt.exists() else None
        record["cleanup"] = json.loads(process_receipt.read_text(encoding="utf-8"))["cleanup"] if process_receipt.exists() else {"status": "unavailable"}
        write(receipt, record)
    return record


def gate_attempt(args: argparse.Namespace) -> dict:
    """Retain a gate decision even when producer or selection validation fails."""
    replay = [sys.executable, "scripts/verify.py", "gate", "--lane", args.lane,
              "--selected", args.selected, "--source-sha", args.source_sha]
    for option, value in (("--plan", args.plan), ("--base-sha", args.base_sha)):
        if value is not None:
            replay.extend((option, str(value)))
    for option, values in (("--expected-job", args.expected_job), ("--job-result", args.job_result)):
        for value in values:
            replay.extend((option, value))
    replay.extend(("--output", "FRESH_GATE_ATTEMPT.json"))
    record = {"schema_version": 1, "kind": "required-gate", "source_sha": args.source_sha,
              "base_sha": args.base_sha, "lane": args.lane, "selected": args.selected == "true",
              "status": "incomplete", "created_at": now(), "primary_failure": None,
              "requested_job_results": args.job_result, "expected_jobs": args.expected_job,
              "selection_path": str(args.plan) if args.plan else None,
              "observed_checkout_sha": None, "replay_command": replay,
              "run_id": os.environ.get("GITHUB_RUN_ID"), "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
              "disposition": "unclassified", "cleanup": {"status": "not-applicable"}}
    started = time.monotonic()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Do not erase a prior failure when a caller reuses its attempt path.
    with args.output.open("x", encoding="utf-8") as output:
        output.write(json.dumps(record, indent=2, sort_keys=True) + "\n")
    try:
        record["observed_checkout_sha"] = git("rev-parse", "HEAD")
        record.update(gate(args.lane, args.selected == "true", args.job_result, args.expected_job,
                           args.plan, args.base_sha, args.source_sha))
        return record
    except BaseException as error:
        record.update(status="cancelled" if isinstance(error, KeyboardInterrupt) else "failed",
                      primary_failure=str(error) or type(error).__name__)
        raise
    finally:
        record["duration_seconds"] = round(time.monotonic() - started, 3)
        write(args.output, record)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    pre = sub.add_parser("preflight", help="cheap syntax, schema and environment checks; no Rust compilation")
    pre.add_argument("--phase", choices=("static", "tools"), default="static")
    pre.add_argument("--tool", action="append", default=[])
    pre.add_argument("--legacy", type=Path)
    pre.add_argument("--output", type=Path)
    planned = sub.add_parser("plan", help="review obligations for immutable base and candidate")
    planned.add_argument("--base", required=True)
    planned.add_argument("--head", default="HEAD")
    planned.add_argument("--output", type=Path)
    selected_parser = sub.add_parser("selected")
    selected_parser.add_argument("--plan", type=Path, required=True)
    selected_parser.add_argument("--lane", required=True)
    selected_parser.add_argument("--force", action="store_true")
    selected_parser.add_argument("--github-output", type=Path, required=True)
    gate_parser = sub.add_parser("gate")
    gate_parser.add_argument("--lane", required=True)
    gate_parser.add_argument("--selected", choices=("true", "false"), default="true")
    gate_parser.add_argument("--plan", type=Path)
    gate_parser.add_argument("--base-sha")
    gate_parser.add_argument("--source-sha", required=True)
    gate_parser.add_argument("--job-result", action="append", required=True)
    gate_parser.add_argument("--expected-job", action="append", required=True)
    gate_parser.add_argument("--output", type=Path, required=True)
    index = sub.add_parser("ledger", help="index existing source-bound receipts without promoting their authority")
    index.add_argument("--source-sha", required=True)
    index.add_argument("--receipt", type=Path, action="append", required=True)
    index.add_argument("--output", type=Path, required=True)
    execute = sub.add_parser("run", help="stream one supported lane and preserve every attempt")
    execute.add_argument("--lane", choices=("static", "behavior", "regression", "inventory", "coverage-canary"), required=True)
    execute.add_argument("--output", type=Path, required=True)
    execute.add_argument("--deadline-seconds", type=int, default=1800)
    args = parser.parse_args()
    try:
        if args.command == "gate":
            return int(gate_attempt(args)["status"] != "passed")
        if args.command == "run":
            if not 1 <= args.deadline_seconds <= 7200:
                raise ValueError("lane deadline must be between 1 and 7200 seconds")
            return int(run_lane(args.lane, args.output.resolve(), args.deadline_seconds)["status"] != "passed")
        if args.command == "selected":
            data = json.loads(args.plan.read_text(encoding="utf-8"))
            chosen = next(lane for lane in data["lanes"] if lane["id"] == args.lane)
            with args.github_output.open("a", encoding="utf-8") as output:
                output.write(f"selected={str(bool(args.force or chosen['selected'])).lower()}\n")
            return 0
        result = (preflight(args.phase, args.tool, args.legacy) if args.command == "preflight" else
                  plan(args.base, args.head) if args.command == "plan" else
                  ledger(args.receipt, args.source_sha))
        write(args.output, result)
        if args.command == "ledger":
            from verification_ledger import render
            args.output.with_suffix(".md").write_text(render(result), encoding="utf-8")
        return 1 if result.get("status") == "failed" else 0
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as error:
        print(f"verification failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
