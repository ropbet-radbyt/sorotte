from __future__ import annotations

import contextlib
import copy
import dataclasses
import json
import os
import pathlib
import shutil
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import mutation_campaign as campaign
import mutation_ci as ci
import mutation_process
try:
    from . import test_mutation_ci as fixtures
except ImportError:
    import test_mutation_ci as fixtures

_REAL_IMMUTABLE_SOURCE = campaign.require_immutable_source

class MutationCampaignTests(unittest.TestCase):
    git = fixtures.MutationRunnerTests.git

    def setUp(self):
        fixtures.MutationRunnerTests.setUp(self)
        (self.repo / campaign.EXECUTION_POLICY).write_text('''schema_version=1
target_mutants_per_chunk=1
max_chunks_per_shard=8
reference_run="https://github.com/example/project/actions/runs/1"
reference_sha="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
[minimum_chunks]
[historical_job_seconds]
''')
        self.inventory = []
        for index, replacement in enumerate(("false", "false", "true")):
            mutant = copy.deepcopy(self.fixture.mutant)
            mutant["span"]["start"]["column"] += index
            mutant["name"] = f"{mutant['file']}:2:{5+index}: replace demo -> bool with {replacement}"
            mutant["replacement"] = replacement
            self.inventory.append(mutant)
        self.selected = {"base": "a" * 40, "head": self.git("rev-parse", "HEAD"), "shards": ["demo"], "full": False}
        self.mocks = contextlib.ExitStack()
        self.mocks.enter_context(mock.patch.object(campaign, "tool_inputs", return_value={"cargo_mutants": "27.1.0"}))
        self.listing = self.mocks.enter_context(mock.patch.object(campaign, "list_mutants", side_effect=lambda *_: copy.deepcopy(self.inventory)))
        self.run_mock = self.mocks.enter_context(mock.patch.object(ci, "run_process", side_effect=self.fake_run))
        self.mocks.enter_context(mock.patch.object(ci, "verify_tool"))
        self.immutable_source = self.mocks.enter_context(mock.patch.object(campaign, "require_immutable_source"))
        self.mocks.enter_context(contextlib.redirect_stdout(__import__("io").StringIO()))
        self.mocks.enter_context(contextlib.redirect_stderr(__import__("io").StringIO()))
        self.artifacts = self.repo / "target" / "artifacts"
        self.unviable = False

    def tearDown(self):
        self.mocks.close()
        fixtures.MutationRunnerTests.tearDown(self)

    def fake_run(self, argv, *, cwd, **kwargs):
        if argv[:2] == ["rustc", "--version"]:
            return subprocess.CompletedProcess(argv, 0, "rustc 1.97.1\n", "")
        if argv[:2] == ["cargo", "test"]:
            return subprocess.CompletedProcess(argv, 0, "demo::tests::case: test\n", "")
        index, count = map(int, argv[argv.index("--shard") + 1].split("/"))
        subset = self.inventory[index::count]
        if "--list" in argv:
            return subprocess.CompletedProcess(argv, 0, json.dumps(subset), "")
        output = pathlib.Path(argv[argv.index("--output") + 1])
        self.assertEqual(len(subset), 1)
        fixture = copy.deepcopy(self.fixture)
        fixture.mutant = copy.deepcopy(subset[0])
        fixture.inventory = copy.deepcopy(subset)
        fixture.outcomes["outcomes"][1]["scenario"] = {"Mutant": ci.without_diff(subset[0])}
        if self.unviable and subset[0]["replacement"] == "false":
            fixture.outcomes["caught"] = 0
            fixture.outcomes["unviable"] = 1
            fixture.outcomes["outcomes"][1]["summary"] = "Unviable"
            fixture.outcomes["outcomes"][1]["phase_results"] = [fixture.phase("Build", {"Failure": 101}, no_run=True)]
        fixture.write(output / ci.MUTANTS_DIRECTORY)
        if self.unviable and subset[0]["replacement"] == "false":
            (output / ci.MUTANTS_DIRECTORY / "caught.txt").write_text("")
            (output / ci.MUTANTS_DIRECTORY / "unviable.txt").write_text(subset[0]["name"] + "\n")
        result = subprocess.CompletedProcess(argv, 0, "completed", "")
        result.execution = {"status": "completed", "returncode": 0, "command": list(argv),
                            "cleanup": {"status": "passed"}, "scratch_cleanup": {"status": "passed"}}
        return result

    def produce(self):
        prepared = campaign.prepare(self.repo, self.selected)
        for item in campaign.matrix(prepared):
            code = campaign.run_chunk(self.repo, prepared, self.selected, item["chunk"], self.artifacts / item["chunk"])
            self.assertEqual(code, 0)
        return prepared

    def reports(self):
        return sorted(self.artifacts.rglob("mutation-*.json"))

    def mutate_report(self, callback):
        path = self.reports()[0]
        report = json.loads(path.read_text())
        callback(report)
        ci.atomic_write_json(path, report)

    def test_complete_exact_campaign_rechecks_raw_artifacts_and_lists_tests_once(self):
        prepared = self.produce()
        self.run_mock.reset_mock()
        receipt = campaign.verify(self.repo, prepared, self.selected, self.artifacts)
        self.assertEqual(receipt["shards"]["demo"]["caught"], 3)
        self.assertEqual(receipt["chunks"], 3)
        self.assertEqual(receipt["fresh_test_listing_executions"], 1)
        self.assertEqual(sum(call.args[0][:2] == ["cargo", "test"] for call in self.run_mock.call_args_list), 1)
        self.assertGreaterEqual(self.listing.call_count, 2)  # independent prepare and finalizer

    def test_balanced_partition_preserves_large_inventory_with_no_empty_chunks(self):
        policy = {"target_mutants_per_chunk": 48, "max_chunks_per_shard": 8, "minimum_chunks": {"server": 4}}
        inventory = [{"name": str(index)} for index in range(216)]
        chunks = campaign.partition("server", inventory, policy)
        counts = [len(chunk["inventory"]) for chunk in chunks]
        self.assertEqual(counts, [44, 43, 43, 43, 43])
        campaign.verify_union(inventory, [chunk["inventory"] for chunk in chunks])
        self.assertEqual(len(campaign.partition("server", inventory[:1], policy)), 1)

    def test_missing_and_overlapping_chunks_fail_even_if_digest_is_recomputed(self):
        prepared = campaign.prepare(self.repo, self.selected)
        for corrupt in (lambda chunks: chunks.pop(), lambda chunks: chunks.append(chunks[0]),
                        lambda chunks: chunks[0]["inventory"].append(chunks[1]["inventory"][0])):
            candidate = copy.deepcopy(prepared)
            corrupt(candidate["shards"]["demo"]["chunks"])
            candidate["sha256"] = campaign.campaign_digest(candidate)
            with self.assertRaisesRegex(ci.MutationCiError, "partition"):
                campaign.validate_campaign(self.repo, candidate, self.selected)

    def test_stale_inventory_is_rejected_by_fresh_generator(self):
        prepared = campaign.prepare(self.repo, self.selected)
        self.inventory.pop()
        with self.assertRaisesRegex(ci.MutationCiError, "independent fresh inventory"):
            campaign.validate_campaign(self.repo, prepared, self.selected, fresh_inventory=True)

    def test_source_and_test_input_changes_invalidate_every_chunk(self):
        prepared = campaign.prepare(self.repo, self.selected)
        (self.repo / "crates/demo/src/tests.rs").write_text("new assertion")
        with self.assertRaisesRegex(ci.MutationCiError, "source/test/policy inputs are stale"):
            campaign.validate_campaign(self.repo, prepared, self.selected)

    def test_uncommitted_deleted_tests_are_not_omitted_from_source_authority(self):
        # Exercise the real Git check separately from synthetic producer fixtures.
        with mock.patch.object(campaign.selection, "git", side_effect=[(self.selected["head"] + "\n").encode(), b" D crates/demo/src/tests.rs\n"]):
            with self.assertRaisesRegex(ci.MutationCiError, "committed source/test/policy"):
                _REAL_IMMUTABLE_SOURCE(self.repo, self.selected["head"])

    def test_wrong_tool_input_cannot_reuse_prepared_inventory(self):
        prepared = campaign.prepare(self.repo, self.selected)
        with mock.patch.object(campaign, "tool_inputs", return_value={"cargo_mutants": "27.2.0"}):
            with self.assertRaisesRegex(ci.MutationCiError, "compiler/tool/environment"):
                campaign.validate_campaign(self.repo, prepared, self.selected)

    def test_tool_change_during_execution_replaces_success_with_incomplete_evidence(self):
        prepared = campaign.prepare(self.repo, self.selected)
        chunk = campaign.matrix(prepared)[0]["chunk"]
        with mock.patch.object(campaign, "tool_inputs", side_effect=[prepared["tool_inputs"], {"cargo_mutants": "27.2.0"}]):
            code = campaign.run_chunk(self.repo, prepared, self.selected, chunk, self.artifacts / chunk)
        self.assertEqual(code, 1)
        report = json.loads(self.reports()[0].read_text())
        self.assertFalse(report["complete"])
        self.assertEqual(report["status"], "error")
        self.assertIn("changed during execution", report["errors"][0])

    def test_missing_duplicate_and_failed_chunk_reports_fail(self):
        prepared = self.produce()
        path = self.reports()[0]
        duplicate = path.parent / "duplicate" / path.name
        duplicate.parent.mkdir()
        shutil.copyfile(path, duplicate)
        with self.assertRaisesRegex(ci.MutationCiError, "duplicate"):
            campaign.verify(self.repo, prepared, self.selected, self.artifacts)
        duplicate.unlink()
        original = path.read_bytes()
        path.unlink()
        with self.assertRaisesRegex(ci.MutationCiError, "exact and complete"):
            campaign.verify(self.repo, prepared, self.selected, self.artifacts)
        path.write_bytes(original)
        self.mutate_report(lambda report: report.update(status="timeout", complete=False))
        with self.assertRaisesRegex(ci.MutationCiError, "stale, incomplete"):
            campaign.verify(self.repo, prepared, self.selected, self.artifacts)

    def test_raw_baseline_missing_and_log_tampering_cannot_be_hidden_by_passed_summary(self):
        prepared = self.produce()
        raw = self.reports()[0].parent / "results/mutants.out/outcomes.json"
        value = json.loads(raw.read_text())
        value["outcomes"] = value["outcomes"][1:]
        ci.atomic_write_json(raw, value)
        with self.assertRaisesRegex(ci.MutationCiError, "exactly one baseline"):
            campaign.verify(self.repo, prepared, self.selected, self.artifacts)

    def test_cleanup_failure_and_summary_tampering_cannot_pass(self):
        prepared = self.produce()
        path = self.reports()[0]
        original = path.read_bytes()
        for corrupt in (lambda report: report["execution"]["cleanup"].update(status="failed"),
                        lambda report: report["execution"]["scratch_cleanup"].update(status="failed"),
                        lambda report: report["summary"].update(caught=100)):
            path.write_bytes(original)
            self.mutate_report(corrupt)
            with self.assertRaises(ci.MutationCiError):
                campaign.verify(self.repo, prepared, self.selected, self.artifacts)

    def test_unviable_counts_are_reconciled_globally_including_all_unviable_chunks(self):
        self.unviable = True
        policy = fixtures.MutationPolicyTests.policy_text(self, accepted=True) + "expected_count = 2\n"
        (self.repo / "coverage/mutation-policy.toml").write_text(policy)
        prepared = self.produce()
        receipt = campaign.verify(self.repo, prepared, self.selected, self.artifacts)
        self.assertEqual(receipt["shards"]["demo"]["unviable"], 2)
        self.assertEqual(receipt["shards"]["demo"]["viable_mutants"], 1)
        (self.repo / "coverage/mutation-policy.toml").write_text(policy.replace("expected_count = 2", "expected_count = 1"))
        self.artifacts = self.repo / "target/artifacts-wrong-count"
        wrong_plan = self.produce()
        with self.assertRaisesRegex(ci.MutationCiError, "reviewed unviable counts"):
            campaign.verify(self.repo, wrong_plan, self.selected, self.artifacts)

    def test_unchanged_retry_never_erases_a_completed_failed_mutation_attempt(self):
        prepared = self.produce()
        original = self.reports()[0]
        old = json.loads(original.read_text())
        old.update(status="failed", complete=True)
        ci.atomic_write_json(original, old)
        retry_path = original.parent / "retry" / original.name
        latest = copy.deepcopy(old)
        latest.update(attempt=2, status="passed")
        ci.atomic_write_json(retry_path, latest)
        with self.assertRaisesRegex(ci.MutationCiError, "unchanged retry"):
            campaign.verify(self.repo, prepared, self.selected, self.artifacts)

    def test_interrupted_attempt_is_preserved_when_a_fresh_attempt_completes(self):
        prepared = self.produce()
        original = self.reports()[0]
        retry_directory = self.artifacts / "explicit-retry"
        shutil.copytree(original.parent, retry_directory)
        previous = json.loads(original.read_text())
        previous.update(status="cancelled", complete=False, errors=["runner cancellation"])
        ci.atomic_write_json(original, previous)
        retry = retry_directory / original.name
        current = json.loads(retry.read_text())
        current["attempt"] = 2
        ci.atomic_write_json(retry, current)
        receipt = campaign.verify(self.repo, prepared, self.selected, self.artifacts)
        self.assertEqual(receipt["shards"]["demo"]["total_mutants"], 3)
        self.assertEqual(receipt["previous_attempts"][0]["status"], "cancelled")
        self.assertEqual(receipt["previous_attempts"][0]["errors"], ["runner cancellation"])
        self.assertEqual(json.loads(original.read_text())["status"], "cancelled")

    def test_boolean_exit_code_is_not_a_successful_producer(self):
        prepared = self.produce()
        self.mutate_report(lambda report: report.update(producer_exit_code=False))
        with self.assertRaisesRegex(ci.MutationCiError, "must be an integer"):
            campaign.verify(self.repo, prepared, self.selected, self.artifacts)

    def test_no_applicable_work_is_a_source_bound_validated_receipt(self):
        selected = {**self.selected, "shards": []}
        prepared = campaign.prepare(self.repo, selected)
        self.run_mock.reset_mock()
        receipt = campaign.verify(self.repo, prepared, selected, self.artifacts)
        self.assertEqual(receipt["disposition"], "no-applicable-shards")
        self.assertEqual(receipt["head"], selected["head"])
        self.run_mock.assert_not_called()
        self.assertTrue(receipt["complete"])

    def test_cache_reuses_only_identical_scope_and_input_bindings(self):
        policy = ci.load_policy(self.repo, self.repo / "coverage/mutation-policy.toml")
        shard = policy.shards[0]
        cache = ci.TestInventoryCache()
        bindings = [{"digest": "same"}]
        cache.listing(self.repo, shard, bindings)
        cache.listing(self.repo, dataclasses.replace(shard, identifier="same-scope"), bindings)
        self.assertEqual(cache.executions, 1)
        cache.listing(self.repo, shard, [{"digest": "changed"}])
        self.assertEqual(cache.executions, 2)
        cache.listing(self.repo, dataclasses.replace(shard, test_target="lib", test_filter="demo::tests::"), bindings)
        self.assertEqual(cache.executions, 3)

    def test_whole_shard_verifier_never_accepts_a_single_chunk(self):
        self.produce()
        self.assertEqual(ci.verify_report(__import__("argparse").Namespace(repo_root=str(self.repo), policy="coverage/mutation-policy.toml",
                                                                          shard="demo", report=str(self.reports()[0]))), 2)

    def test_matrix_failure_cannot_turn_into_empty_or_passed_required_receipt(self):
        output = self.repo / "target/required.json"
        with mock.patch.object(campaign.selection, "plan", return_value={**self.selected, "shards": []}):
            for result in ("failure", "cancelled", "success", ""):
                code = campaign.main(["verify", "--repo-root", str(self.repo), "--base", self.selected["base"],
                                      "--head", self.selected["head"], "--mutation-result", result,
                                      "--output", str(output)])
                self.assertEqual(code, 1)
                self.assertFalse(json.loads(output.read_text())["complete"])


if __name__ == "__main__":
    unittest.main()
