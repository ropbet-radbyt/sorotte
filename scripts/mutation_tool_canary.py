#!/usr/bin/env python3
"""Exercise cargo-mutants 27.1.0's actual inventory/chunk/outcome contract.

This tiny, dependency-free crate is deliberately separate from mocked harness
self-tests and from the product mutation campaign. No source inventory or
mutation test obligation can be suppressed by this canary.
"""
from __future__ import annotations

import argparse
import dataclasses
import decimal
import json
import os
import pathlib
import tempfile
import time

import mutation_campaign as campaign
import mutation_ci as ci
import mutation_process


SOURCE = """pub fn invert(value: bool) -> bool { !value }
pub fn sum(left: u32, right: u32) -> u32 { left + right }
#[cfg(test)] mod tests {
    #[test] fn both_boolean_values() {
        assert!(super::invert(false)); assert!(!super::invert(true));
    }
    #[test] fn arithmetic_examples() {
        assert_eq!(super::sum(2, 3), 5); assert_eq!(super::sum(7, 4), 11);
    }
}
"""


def run(output: pathlib.Path) -> dict:
    started = time.monotonic()
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    environment = os.environ.copy()
    with tempfile.TemporaryDirectory(prefix="sm-canary-") as directory:
        root = pathlib.Path(directory)
        (root / "src").mkdir()
        (root / "src/lib.rs").write_text(SOURCE, encoding="utf-8")
        (root / "Cargo.toml").write_text('[package]\nname="mutation-canary"\nversion="0.1.0"\nedition="2024"\n[workspace]\n', encoding="utf-8")
        # Every experimental compile stays in this owned external fixture.
        for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR"):
            environment[key] = str(root / "target")
        result = mutation_process.run(["cargo", "generate-lockfile", "--offline"], cwd=root, env=environment,
                                       timeout_seconds=60, log_root=output / "lockfile")
        if result.returncode:
            raise ci.MutationCiError("canary lockfile preparation failed")
        ci.verify_tool(root, "27.1.0")
        shard = ci.ShardPolicy(identifier="canary", owner="verification", package="mutation-canary",
                               files=("src/lib.rs",), mutant_filter="", test_target="lib", test_filter="",
                               jobs=1, timeout_seconds=30, build_timeout_seconds=120,
                               minimum_viable_kill_percent=decimal.Decimal("100.00"), max_missed=0,
                               max_timeouts=0, require_baseline=True)
        inventory = campaign.list_mutants(root, shard)
        partition_policy = {"target_mutants_per_chunk": 3, "max_chunks_per_shard": 3, "minimum_chunks": {}}
        chunks = campaign.partition("canary", inventory, partition_policy)
        binding = ci.source_bindings(root, shard.files)
        summaries = []
        observed = []
        for chunk in chunks:
            command = [*ci.cargo_mutants_base_command(shard), "--shard", f"{chunk['index']}/{chunk['count']}",
                       "--sharding", "round-robin"]
            listing = ci.run_process([*command, "--list", "--json"], cwd=root,
                                     log_root=output / f"list-{chunk['index']}")
            native = ci.parse_inventory(json.loads(listing.stdout), shard=shard, label="native canary chunk")
            if native != chunk["inventory"]:
                raise ci.MutationCiError("pinned native shard indexing/order differs from the planned exact partition")
            results = output / f"chunk-{chunk['index']}"
            producer = ci.run_process([*command, "--caught", "--unviable", "--output", str(results)], cwd=root,
                                      timeout_seconds=180, log_root=output / f"run-{chunk['index']}")
            evaluation = ci.evaluate_results(results_dir=results / ci.MUTANTS_DIRECTORY, shard=shard, accepted=(),
                                             expected_version="27.1.0", producer_exit_code=producer.returncode,
                                             pre_inventory=native, source_before=binding,
                                             source_after=ci.source_bindings(root, shard.files), partial_shard=True)
            if evaluation["status"] != "passed":
                raise ci.MutationCiError(f"pinned native tool canary failed: {evaluation['errors']}")
            observed.append(native)
            summaries.append(evaluation["summary"])
        campaign.verify_union(inventory, observed)
        # Check actual survivor reporting and nonzero exits as well as green
        # fixtures. A mock cannot establish the pinned producer's failure ABI.
        weak_source = SOURCE.replace("assert!(super::invert(false)); assert!(!super::invert(true));", "let _ = super::invert(false);")
        (root / "src/lib.rs").write_text(weak_source, encoding="utf-8")
        (output / "deliberate-survivor-source.rs").write_text(weak_source, encoding="utf-8")
        weak_shard = dataclasses.replace(shard, mutant_filter="invert")
        weak_inventory = campaign.list_mutants(root, weak_shard)
        weak_binding = ci.source_bindings(root, weak_shard.files)
        weak_results = output / "deliberate-survivor"
        weak_producer = ci.run_process([*ci.cargo_mutants_base_command(weak_shard), "--output", str(weak_results)],
                                       cwd=root, timeout_seconds=180, log_root=output / "run-deliberate-survivor")
        rejection = ci.evaluate_results(results_dir=weak_results / ci.MUTANTS_DIRECTORY, shard=weak_shard, accepted=(),
                                        expected_version="27.1.0", producer_exit_code=weak_producer.returncode,
                                        pre_inventory=weak_inventory, source_before=weak_binding,
                                        source_after=ci.source_bindings(root, weak_shard.files))
        if weak_producer.returncode == 0 or rejection["status"] != "failed" or not rejection["summary"]["missed"]:
            raise ci.MutationCiError("pinned native survivor was not rejected by the required evaluator")
        ci.atomic_write_json(output / "deliberate-survivor-rejection.json", rejection)
    receipt = {"schema_version": 1, "kind": "sorotte-mutation-tool-canary", "status": "passed",
               "cargo_mutants_version": "27.1.0", "native_shard_index": "zero-based", "sharding": "round-robin",
               "inventory_sha256": ci.canonical_digest(inventory), "mutants": len(inventory), "chunks": summaries,
               "deliberate_survivor_rejected": True, "deliberate_survivors": rejection["summary"]["missed"],
               "scratch_cleanup": "passed", "elapsed_seconds": round(time.monotonic() - started, 3)}
    ci.atomic_write_json(output / "canary.json", receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(run(args.output), indent=2))
        return 0
    except (OSError, ValueError, mutation_process.ProcessError) as error:
        ci.atomic_write_json(args.output / "canary.json", {"schema_version": 1, "kind": "sorotte-mutation-tool-canary",
                                                        "status": "failed", "error": str(error)})
        print(f"mutation tool canary failed: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
