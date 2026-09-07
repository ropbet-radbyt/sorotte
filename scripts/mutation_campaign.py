#!/usr/bin/env python3
"""Prepare, execute and independently verify exact-union mutation campaigns.

The plan partitions native cargo-mutants order using its pinned round-robin
sharding contract. Chunks keep the parent shard's complete test scope. Only a
fresh finalizer can establish the complete shard and accepted-unviable counts.
"""
from __future__ import annotations

import argparse
import collections
import json
import math
import os
import pathlib
import sys
import time
import tomllib
from typing import Any

import mutation_ci as ci
import mutation_process
import mutation_selection as selection


EXECUTION_POLICY = "coverage/mutation-execution.toml"
CAMPAIGN_KIND = "sorotte-mutation-campaign"


def require_immutable_source(root: pathlib.Path, head: str) -> None:
    if selection.git(root, "rev-parse", "HEAD").decode().strip() != head:
        raise ci.MutationCiError("mutation campaign checkout is not its immutable source subject")
    # Include deleted inputs too: hashing only currently existing files is
    # insufficient when an uncommitted deletion removes a test or fixture.
    scopes = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo", "crates", "fixtures", "resources",
              "scripts/mutation*.py", "scripts/artifact_input.py", "coverage/mutation-*",
              "scripts/verify.py", "scripts/verification_tools.py", "scripts/test_inventory.py",
              "coverage/verification-tools.toml", "coverage/verification-lanes.json", "coverage/test-inventories.json",
              ".github/workflows/rust-mutation.yml"]
    if selection.git(root, "status", "--porcelain=v1", "--untracked-files=all", "--", *scopes):
        raise ci.MutationCiError("mutation campaign requires committed source/test/policy inputs, including deleted or untracked inputs")


def load_execution_policy(root: pathlib.Path, policy: ci.MutationPolicy) -> dict:
    raw = ci.bounded_bytes(root / EXECUTION_POLICY, maximum=ci.MAX_POLICY_BYTES, label="mutation execution policy")
    value = tomllib.loads(raw.decode("utf-8"))
    ci.require_exact_keys(value, {"schema_version", "target_mutants_per_chunk", "max_chunks_per_shard",
                                 "minimum_chunks", "historical_job_seconds", "reference_run", "reference_sha"}, label="execution policy")
    if type(value["schema_version"]) is not int or value["schema_version"] != 1:
        raise ci.MutationCiError("unsupported mutation execution policy")
    ci.require_int(value["target_mutants_per_chunk"], label="target_mutants_per_chunk", minimum=1, maximum=1000)
    maximum = ci.require_int(value["max_chunks_per_shard"], label="max_chunks_per_shard", minimum=1, maximum=16)
    known = {shard.identifier for shard in policy.shards}
    for field, limit in (("minimum_chunks", maximum), ("historical_job_seconds", 86400)):
        for shard, count in ci.require_mapping(value[field], label=field).items():
            if shard not in known:
                raise ci.MutationCiError(f"execution policy names unknown shard: {shard}")
            ci.require_int(count, label=f"{field}.{shard}", minimum=1, maximum=limit)
    if not ci.FULL_SHA.fullmatch(value["reference_sha"]) or not value["reference_run"].startswith("https://github.com/"):
        raise ci.MutationCiError("execution timing reference must have an immutable source and run")
    return value


def partition(shard: str, inventory: list[dict], policy: dict) -> list[dict]:
    if not inventory:
        raise ci.MutationCiError("cannot partition an empty mutant inventory")
    count = min(len(inventory), policy["max_chunks_per_shard"],
                max(math.ceil(len(inventory) / policy["target_mutants_per_chunk"]),
                    policy["minimum_chunks"].get(shard, 1)))
    return [{"id": f"{shard}--{index + 1}-of-{count}", "shard": shard,
             "index": index, "count": count,
             "inventory": inventory[index::count],
             "inventory_sha256": ci.canonical_digest(inventory[index::count]),
             "full_inventory_sha256": ci.canonical_digest(inventory)} for index in range(count)]


def tool_inputs(root: pathlib.Path, expected: str) -> dict:
    ci.verify_tool(root, expected)
    versions = {}
    for tool in ("rustc", "cargo"):
        result = ci.run_process([tool, "--version", "--verbose"], cwd=root, timeout_seconds=60)
        if result.returncode or result.stderr or not result.stdout.startswith(tool + " "):
            raise ci.MutationCiError(f"cannot identify mutation {tool} compiler input")
        versions[tool] = result.stdout
    return {"cargo_mutants": expected, **versions, "platform": sys.platform,
            "build_environment": {key: os.environ.get(key) for key in (
                "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC", "RUSTDOC", "RUSTC_WRAPPER",
                "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_TARGET", "RUSTUP_TOOLCHAIN")}}


def list_mutants(root: pathlib.Path, shard: ci.ShardPolicy) -> list[dict]:
    result = ci.run_process([*ci.cargo_mutants_base_command(shard), "--list", "--json"], cwd=root)
    if result.returncode or result.stderr:
        raise ci.MutationCiError(f"cannot list immutable mutant inventory: {shard.identifier}: {result.stderr}")
    if len(result.stdout.encode("utf-8")) > ci.MAX_JSON_BYTES:
        raise ci.MutationCiError("mutant inventory exceeds size limit")
    return ci.parse_inventory(ci.parse_json_bytes(result.stdout.encode(), label="campaign inventory"),
                              shard=shard, label="campaign inventory")


def campaign_digest(campaign: dict) -> str:
    return ci.canonical_digest({key: value for key, value in campaign.items() if key != "sha256"})


def prepare(root: pathlib.Path, selected: dict) -> dict:
    require_immutable_source(root, selected["head"])
    policy = ci.load_policy(root, root / selection.POLICY)
    execution = load_execution_policy(root, policy)
    before = ci.verification_input_bindings(root, root / selection.POLICY)
    inputs = tool_inputs(root, policy.cargo_mutants_version) if selected["shards"] else None
    shards = {}
    for identifier in selected["shards"]:
        inventory = list_mutants(root, policy.shard(identifier))
        chunks = partition(identifier, inventory, execution)
        shards[identifier] = {"inventory": inventory, "chunks": chunks}
        print(f"mutation preparation: {identifier}: {len(inventory)} mutants in {len(chunks)} chunks", file=sys.stderr, flush=True)
    if before != ci.verification_input_bindings(root, root / selection.POLICY):
        raise ci.MutationCiError("mutation inputs changed during inventory preparation")
    require_immutable_source(root, selected["head"])
    campaign = {"schema_version": 1, "kind": CAMPAIGN_KIND, "selection": selected,
                "verification_inputs": before, "tool_inputs": inputs,
                "execution_policy": execution, "shards": shards}
    campaign["sha256"] = campaign_digest(campaign)
    if sum(len(value["chunks"]) for value in shards.values()) > 256:
        raise ci.MutationCiError("mutation campaign exceeds GitHub's 256-entry matrix; adjust execution partition")
    return campaign


def validate_campaign(root: pathlib.Path, campaign: dict, selected: dict, *, fresh_inventory: bool = False) -> ci.MutationPolicy:
    require_immutable_source(root, selected["head"])
    ci.require_exact_keys(campaign, {"schema_version", "kind", "selection", "verification_inputs", "tool_inputs",
                                   "execution_policy", "shards", "sha256"}, label="mutation campaign")
    if type(campaign["schema_version"]) is not int or campaign["schema_version"] != 1 or campaign["kind"] != CAMPAIGN_KIND:
        raise ci.MutationCiError("unsupported mutation campaign schema")
    if campaign["sha256"] != campaign_digest(campaign) or campaign["selection"] != selected:
        raise ci.MutationCiError("mutation campaign identity or immutable selection differs")
    policy = ci.load_policy(root, root / selection.POLICY)
    if campaign["verification_inputs"] != ci.verification_input_bindings(root, root / selection.POLICY):
        raise ci.MutationCiError("mutation campaign source/test/policy inputs are stale")
    execution = load_execution_policy(root, policy)
    if campaign["execution_policy"] != execution:
        raise ci.MutationCiError("mutation execution partition policy differs")
    expected_tools = tool_inputs(root, policy.cargo_mutants_version) if selected["shards"] else None
    if campaign["tool_inputs"] != expected_tools:
        raise ci.MutationCiError("mutation campaign compiler/tool/environment inputs differ")
    if set(campaign["shards"]) != set(selected["shards"]):
        raise ci.MutationCiError("mutation campaign selected shard set is incomplete")
    for identifier, item in campaign["shards"].items():
        ci.require_exact_keys(item, {"inventory", "chunks"}, label=f"campaign shard {identifier}")
        inventory = ci.parse_inventory(item["inventory"], shard=policy.shard(identifier), label="campaign inventory")
        if item["chunks"] != partition(identifier, inventory, execution):
            raise ci.MutationCiError("mutation chunk partition is incomplete, overlapping or reordered")
        if fresh_inventory and inventory != list_mutants(root, policy.shard(identifier)):
            raise ci.MutationCiError("mutation campaign inventory differs from independent fresh inventory")
    return policy


def matrix(campaign: dict) -> list[dict]:
    rows = [{"chunk": chunk["id"], "shard": identifier}
            for identifier, item in campaign["shards"].items() for chunk in item["chunks"]]
    # Start the historically expensive work first when runner concurrency is
    # below matrix size. This is a scheduling estimate, never an evidence gate.
    costs = campaign["execution_policy"]["historical_job_seconds"]
    return sorted(rows, key=lambda row: (
        -costs.get(row["shard"], len(campaign["shards"][row["shard"]]["inventory"]))
        / len(campaign["shards"][row["shard"]]["chunks"]), row["chunk"]))


def read_campaign(directory: pathlib.Path) -> dict:
    """A retried preparation may repeat the same plan, never a different one."""
    plans = []
    for path in directory.rglob("mutation-campaign.json"):
        if path.is_symlink() or not path.resolve().is_relative_to(directory.resolve()):
            raise ci.MutationCiError("mutation campaign artifact escapes input root")
        value, _ = ci.load_json(path, label="mutation campaign")
        plans.append(value)
    if not plans or any(value != plans[0] for value in plans[1:]):
        raise ci.MutationCiError("prepared mutation campaigns are missing or disagree across attempts")
    return plans[0]


def run_chunk(root: pathlib.Path, campaign: dict, selected: dict, identifier: str, attempt: pathlib.Path,
              *, deadline_seconds: int = 6600, attempt_number: int | None = None) -> int:
    policy = validate_campaign(root, campaign, selected)
    matches = [chunk for item in campaign["shards"].values() for chunk in item["chunks"] if chunk["id"] == identifier]
    if len(matches) != 1:
        raise ci.MutationCiError("requested mutation chunk must exist exactly once")
    chunk = {**matches[0], "campaign_sha256": campaign["sha256"]}
    report_path = attempt / f"mutation-{identifier}.json"
    result = ci.run_shard(argparse.Namespace(repo_root=str(root), policy=selection.POLICY,
                       shard=chunk["shard"], chunk=chunk, deadline_seconds=deadline_seconds,
                       attempt=attempt_number,
                       results_root=str(attempt / "results"), output=str(report_path)))
    if result == 0:
        report, _ = ci.load_json(report_path if report_path.is_absolute() else root / report_path, label="completed mutation chunk")
        try:
            current_tools = tool_inputs(root, policy.cargo_mutants_version)
            report["tool_inputs"] = {"before": campaign["tool_inputs"], "after": current_tools}
            if current_tools != campaign["tool_inputs"]:
                raise ci.MutationCiError("mutation compiler/tool/environment inputs changed during execution")
            require_immutable_source(root, selected["head"])
        except (ci.MutationCiError, selection.SelectionError, mutation_process.ProcessError, OSError) as error:
            report.update(status="error", complete=False, errors=[str(error)])
            result = 1
        ci.atomic_write_json(report_path if report_path.is_absolute() else root / report_path, report)
    return result


def verify_union(inventory: list[dict], chunks: list[list[dict]]) -> None:
    expected = {item["name"]: item for item in inventory}
    actual = {}
    for chunk in chunks:
        for mutant in chunk:
            name = mutant["name"]
            if name in actual:
                raise ci.MutationCiError(f"mutation campaign contains duplicate mutant: {name}")
            actual[name] = mutant
    if actual != expected:
        raise ci.MutationCiError("mutation campaign union is incomplete or contains foreign/stale mutants")


def verify(root: pathlib.Path, campaign: dict, selected: dict, artifacts: pathlib.Path) -> dict:
    started = time.monotonic()
    policy = validate_campaign(root, campaign, selected, fresh_inventory=True)
    reports: dict[str, pathlib.Path] = {}
    attempts: dict[str, dict[int, tuple[pathlib.Path, dict]]] = {}
    artifacts = artifacts.resolve()
    for path in artifacts.rglob("mutation-*.json"):
        if path.is_symlink() or not path.resolve().is_relative_to(artifacts):
            raise ci.MutationCiError("mutation artifact escapes its owned directory")
        identifier = path.name.removeprefix("mutation-").removesuffix(".json")
        report, _ = ci.load_json(path, label="mutation attempt report")
        attempt = ci.require_int(report.get("attempt"), label="mutation attempt", minimum=1)
        if attempt in attempts.setdefault(identifier, {}):
            raise ci.MutationCiError(f"duplicate mutation chunk report in attempt {attempt}: {identifier}")
        attempts[identifier][attempt] = (path, report)
    previous_attempts = []
    for identifier, candidates in attempts.items():
        latest = max(candidates)
        reports[identifier] = candidates[latest][0]
        for attempt, (path, report) in sorted(candidates.items()):
            if attempt == latest:
                continue
            if report.get("complete") is True and report.get("status") == "failed":
                raise ci.MutationCiError("a completed failed mutation attempt cannot be erased by an unchanged retry")
            previous_attempts.append({"chunk": identifier, "attempt": attempt, "status": report.get("status"),
                                      "errors": report.get("errors"), "report": path.relative_to(artifacts).as_posix()})
    if set(reports) != {item["chunk"] for item in matrix(campaign)}:
        raise ci.MutationCiError("mutation chunk report set must be exact and complete")
    cache = ci.TestInventoryCache()
    summaries = {}
    for identifier, item in campaign["shards"].items():
        shard = policy.shard(identifier)
        tests = cache.listing(root, shard, campaign["verification_inputs"])
        source = ci.source_bindings(root, shard.files)
        unviable: collections.Counter = collections.Counter()
        inventories = []
        counts = collections.Counter()
        timings = []
        for chunk in item["chunks"]:
            path = reports[chunk["id"]]
            report, _ = ci.load_json(path, label="mutation chunk report")
            expected_chunk = {key: value for key, value in chunk.items() if key != "inventory"}
            expected_chunk["campaign_sha256"] = campaign["sha256"]
            expected_command = [*ci.cargo_mutants_base_command(shard), "--shard", f"{chunk['index']}/{chunk['count']}",
                                "--sharding", "round-robin", "--caught", "--unviable", "--output"]
            if (type(report.get("schema_version")) is not int or report["schema_version"] != ci.SCHEMA_VERSION
                    or report.get("kind") != ci.REPORT_KIND or report.get("status") != "passed"
                    or report.get("complete") is not True or report.get("execution_chunk") != expected_chunk
                    or report.get("shard") != identifier or report.get("package") != shard.package
                    or report.get("owner") != shard.owner or report.get("cargo_mutants_version") != policy.cargo_mutants_version
                    or report.get("test_scope") != {"target": shard.test_target, "filter": shard.test_filter}
                    or not isinstance(report.get("command"), list) or report["command"][:-1] != expected_command
                    or report.get("git", {}).get("head_sha") != selected["head"]
                    or report.get("git", {}).get("configured_sources_dirty") is not False
                    or report.get("source_bindings") != {"before": source, "after": source}
                    or report.get("verification_input_bindings") != {"before": campaign["verification_inputs"], "after": campaign["verification_inputs"]}
                    or report.get("test_inventory") != ci.test_inventory_binding(tests)):
                raise ci.MutationCiError(f"mutation chunk report is stale, incomplete or names wrong inputs: {chunk['id']}")
            if report.get("tool_inputs") != {"before": campaign["tool_inputs"], "after": campaign["tool_inputs"]}:
                raise ci.MutationCiError("mutation chunk compiler/tool/environment inputs changed or are incomplete")
            execution = ci.require_mapping(report.get("execution"), label="chunk execution")
            producer_exit = ci.require_int(report.get("producer_exit_code"), label="chunk producer exit", minimum=0, maximum=255)
            if (execution.get("status") != "completed" or execution.get("cleanup", {}).get("status") != "passed"
                    or execution.get("scratch_cleanup", {}).get("status") != "passed"
                    or execution.get("returncode") != producer_exit
                    or not isinstance(execution.get("command"), list) or execution["command"][:-1] != expected_command):
                raise ci.MutationCiError("mutation chunk did not finish with verified owned cleanup")
            results = path.parent / "results" / ci.MUTANTS_DIRECTORY
            if results.is_symlink() or not results.resolve().is_relative_to(artifacts):
                raise ci.MutationCiError("mutation producer results escape artifact root")
            evaluation = ci.evaluate_results(results_dir=results, shard=shard, accepted=policy.accepted_for(identifier),
                       expected_version=policy.cargo_mutants_version, producer_exit_code=producer_exit,
                       pre_inventory=chunk["inventory"], source_before=source, source_after=source, partial_shard=True)
            if evaluation["status"] != "passed" or any(report.get(key) != value for key, value in evaluation.items()):
                raise ci.MutationCiError("mutation chunk report contradicts independently evaluated raw artifacts")
            inventories.append(chunk["inventory"])
            raw, _ = ci.load_json(results / "outcomes.json", label="verified timing outcomes")
            baseline = next(outcome for outcome in raw["outcomes"] if outcome["scenario"] == "Baseline")
            timings.append({"chunk": chunk["id"], "mutants": len(chunk["inventory"]),
                            "execution_seconds": execution.get("elapsed_seconds"),
                            "baseline_build_seconds": sum(phase["duration"] for phase in baseline["phase_results"] if phase["phase"] == "Build"),
                            "baseline_test_seconds": sum(phase["duration"] for phase in baseline["phase_results"] if phase["phase"] == "Test"),
                            "mutant_build_and_test_seconds": sum(phase["duration"] for outcome in raw["outcomes"] if outcome["scenario"] != "Baseline" for phase in outcome["phase_results"])})
            unviable.update(match["id"] for match in evaluation["accepted_unviable"])
            counts.update({key: evaluation["summary"][key] for key in ("caught", "unviable", "missed", "timeout", "total_mutants", "viable_mutants")})
        verify_union(item["inventory"], inventories)
        expected_unviable = {entry.identifier: entry.expected_count for entry in policy.accepted_for(identifier)}
        if dict(unviable) != expected_unviable:
            raise ci.MutationCiError(f"mutation campaign reviewed unviable counts are stale or incomplete: {identifier}")
        if counts["viable_mutants"] == 0 or counts["caught"] != counts["viable_mutants"] or counts["missed"] or counts["timeout"]:
            raise ci.MutationCiError("mutation campaign must catch every viable mutant with no survivors/timeouts")
        summaries[identifier] = {**counts, "baseline": "passed in every chunk", "chunks": len(item["chunks"]),
                                 "timings": timings,
                                 "historical_whole_job_seconds": campaign["execution_policy"]["historical_job_seconds"].get(identifier)}
    if campaign["verification_inputs"] != ci.verification_input_bindings(root, root / selection.POLICY):
        raise ci.MutationCiError("mutation source/test/policy inputs changed during finalization")
    require_immutable_source(root, selected["head"])
    if selected["shards"] and tool_inputs(root, policy.cargo_mutants_version) != campaign["tool_inputs"]:
        raise ci.MutationCiError("mutation compiler/tool/environment inputs changed during finalization")
    return {"schema_version": 1, "kind": "sorotte-mutation-required", "status": "passed", "complete": True,
            "disposition": "verified-campaign" if summaries else "no-applicable-shards",
            "base": selected["base"], "head": selected["head"], "selection": selected,
            "campaign_sha256": campaign["sha256"], "shards": summaries, "chunks": len(reports),
            "previous_attempts": previous_attempts,
            "fresh_test_listing_executions": cache.executions, "elapsed_seconds": round(time.monotonic() - started, 3)}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "run", "verify", "validate"))
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path("."))
    parser.add_argument("--base")
    parser.add_argument("--head")
    parser.add_argument("--full", action="store_true")
    parser.add_argument("--campaign", type=pathlib.Path)
    parser.add_argument("--campaign-dir", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--github-output", type=pathlib.Path)
    parser.add_argument("--artifacts", type=pathlib.Path)
    parser.add_argument("--chunk")
    parser.add_argument("--attempt-root", type=pathlib.Path)
    parser.add_argument("--attempt", type=int)
    parser.add_argument("--deadline-seconds", type=int, default=6600)
    parser.add_argument("--selection-result", default="success")
    parser.add_argument("--preparation-result", default="success")
    parser.add_argument("--mutation-result", default="success")
    args = parser.parse_args(argv)
    root = args.repo_root.resolve()
    try:
        if args.command == "validate":
            load_execution_policy(root, ci.load_policy(root, root / selection.POLICY))
            print("mutation execution policy valid")
            return 0
        selected = selection.plan(root, args.base or "", args.head or "", full=args.full)
        if args.command == "prepare":
            campaign = prepare(root, selected)
            if args.output is None:
                raise ci.MutationCiError("prepare requires --output")
            ci.atomic_write_json(args.output, campaign)
            if args.github_output:
                with args.github_output.open("a", encoding="utf-8") as stream:
                    stream.write("matrix=" + json.dumps(matrix(campaign), separators=(",", ":")) + "\n")
            return 0
        if args.command == "verify":
            if args.selection_result != "success" or args.preparation_result != "success":
                raise ci.MutationCiError("mutation selection/preparation producer did not succeed")
            expected_result = "success" if selected["shards"] else "skipped"
            if args.mutation_result != expected_result:
                raise ci.MutationCiError("mutation matrix producer was failed, cancelled, missing or unexpectedly skipped")
        if args.campaign is not None and args.campaign_dir is None:
            campaign, _ = ci.load_json(args.campaign, label="mutation campaign")
        elif args.campaign_dir is not None and args.campaign is None:
            campaign = read_campaign(args.campaign_dir)
        else:
            raise ci.MutationCiError("exactly one of --campaign or --campaign-dir is required")
        if args.command == "run":
            if not args.chunk or args.attempt_root is None:
                raise ci.MutationCiError("run requires --chunk and --attempt-root")
            if args.attempt is not None:
                ci.require_int(args.attempt, label="attempt", minimum=1)
            return run_chunk(root, campaign, selected, args.chunk, args.attempt_root,
                             deadline_seconds=args.deadline_seconds, attempt_number=args.attempt)
        if args.artifacts is None or args.output is None:
            raise ci.MutationCiError("verify requires --artifacts and --output")
        receipt = verify(root, campaign, selected, args.artifacts)
        ci.atomic_write_json(args.output, receipt)
        print(json.dumps(receipt, separators=(",", ":")))
        return 0
    except (ci.MutationCiError, selection.SelectionError, mutation_process.ProcessError, OSError, ValueError, TypeError, KeyError) as error:
        error_path = args.output
        if args.command == "run" and args.attempt_root is not None and args.chunk:
            error_path = args.attempt_root / f"mutation-{args.chunk}.json"
        if error_path is not None and not (args.command == "run" and error_path.exists()):
            ci.atomic_write_json(error_path, {"schema_version": 1, "kind": "sorotte-mutation-required" if args.command == "verify" else CAMPAIGN_KIND,
                                 "status": "failed", "complete": False, "base": args.base, "head": args.head,
                                 "attempt": int(os.environ.get("GITHUB_RUN_ATTEMPT", "1")), "errors": [str(error)]})
        print(f"mutation campaign failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
