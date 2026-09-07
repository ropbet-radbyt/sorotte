from __future__ import annotations
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify
import merge_gate
import coverage_tool_canary
import test_inventory
import verification_tools


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.policy = json.loads(verify.POLICY.read_text(encoding="utf-8"))

    def selected(self, paths):
        return {lane["id"] for lane in verify.select(paths, self.policy) if lane["selected"]}

    def test_documentation_preserves_static_gate_without_expensive_execution(self):
        self.assertEqual(self.selected(["docs/notes.md"]), {"static"})

    def test_unknown_input_and_policy_changes_fail_closed_to_all_obligations(self):
        all_lanes = {lane["id"] for lane in self.policy["lanes"]}
        for path in ("surprise-input", "coverage/verification-lanes.json", ".github/workflows/rust-ci.yml", "scripts/verify.py"):
            self.assertEqual(self.selected([path]), all_lanes)

    def test_protocol_change_keeps_fuzz_and_mutation(self):
        self.assertTrue({"fuzz", "mutation", "coverage", "behavior"} <= self.selected(["crates/sorotte-protocol/src/state.rs"]))

    def test_gate_rejects_missing_duplicate_and_failed_producers(self):
        for results in (["a=success"], ["a=success", "a=success"], ["a=success", "b=skipped"], ["a=success", "b=cancelled"]):
            with self.assertRaises(ValueError):
                verify.gate("fuzz", True, results, ["a", "b"], None)

    def test_noop_requires_immutable_recomputed_plan(self):
        jobs = self.policy["gate_producers"]["fuzz"]
        with self.assertRaisesRegex(ValueError, "no-op requires"):
            verify.gate("fuzz", False, [f"{job}=skipped" for job in jobs], jobs, None)

    def test_noop_rejects_forged_baseline_even_when_self_consistent(self):
        jobs = self.policy["gate_producers"]["fuzz"]
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "plan.json"
            path.write_text(json.dumps({"base_sha": "a" * 40, "source_sha": "a" * 40}), encoding="utf-8")
            with mock.patch.object(verify, "git", side_effect=["a" * 40, "b" * 40]):
                with self.assertRaisesRegex(ValueError, "external event base/source"):
                    verify.gate("fuzz", False, [f"{job}=skipped" for job in jobs], jobs, path, "b" * 40, "a" * 40)

    def test_noop_rejects_fictitious_producer_inventory(self):
        with self.assertRaisesRegex(ValueError, "producer inventory"):
            verify.gate("fuzz", False, ["fake-producer=skipped"], ["fake-producer"], Path("unused"), "b" * 40, "a" * 40)

    def test_ledger_refuses_wrong_source_and_is_not_an_attestation(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "receipt.json"
            path.write_text(json.dumps({"source_sha": "a" * 40, "status": "passed"}), encoding="utf-8")
            with self.assertRaises(ValueError):
                verify.ledger([path], "b" * 40)
            self.assertIn("Index only", verify.ledger([path], "a" * 40)["note"])

    def test_build_cache_namespaces_separate_profiles_features_and_instrumentation(self):
        with mock.patch.object(verification_tools, "identity", return_value={"cargo_lock_sha256": "abc"}):
            base = dict(features=[], profile="debug", instrumentation="ordinary", target="windows")
            original = verification_tools.build_key(**base)
            for change in ({"features": ["all"]}, {"profile": "release"}, {"instrumentation": "llvm"}, {"target": "linux"}):
                self.assertNotEqual(original, verification_tools.build_key(**(base | change)))


class FakeAPI:
    repository = "owner/repo"
    sha = "a" * 40
    workflow = ".github/workflows/rust-ci.yml"
    def __init__(self):
        self.protection = {"required_status_checks": {"strict": True, "contexts": ["merge-required"]},
                           "enforce_admins": {"enabled": True}, "allow_force_pushes": {"enabled": False},
                           "allow_deletions": {"enabled": False}}
        self.branch = {"protected": True, "commit": {"sha": self.sha}}
        self.run = {"head_sha": self.sha, "path": self.workflow, "repository": {"full_name": self.repository},
                    "head_repository": {"full_name": self.repository}, "event": "push", "head_branch": "main",
                    "check_suite_id": 9, "status": "completed", "conclusion": "success", "id": 42,
                    "run_attempt": 2, "html_url": f"https://github.com/{self.repository}/actions/runs/42"}
        self.check = {"name": "merge-required", "head_sha": self.sha, "status": "completed", "conclusion": "success",
                      "app": {"slug": "github-actions"}, "details_url": self.run["html_url"] + "/job/44",
                      "check_suite": {"id": 9}, "id": 12, "completed_at": "2026-09-06T01:00:00Z"}
        self.checks = [self.check]
    def get(self, path):
        return {"branches/main": self.branch, "branches/main/protection": self.protection,
                "actions/runs/42": self.run}[path]
    def pages(self, path, key):
        assert "filter=latest" in path
        return self.checks


class ReleaseAuthorityTests(unittest.TestCase):
    def authorize(self, api):
        return merge_gate.authorize(api, api.sha, {"merge-required": api.workflow})

    def test_exact_trusted_protected_main_records_run_attempt(self):
        receipt = self.authorize(FakeAPI())
        self.assertEqual(receipt["producers"][0]["run_attempt"], 2)

    def test_skipped_cancelled_or_older_green_cannot_authorize(self):
        for value in ("skipped", "cancelled", "failure", None):
            api = FakeAPI()
            api.check["conclusion"] = value
            with self.assertRaises(merge_gate.GateError): self.authorize(api)

    def test_foreign_or_wrong_workflow_producers_cannot_authorize(self):
        for key, value in (("path", ".github/workflows/forged.yml"), ("head_sha", "b" * 40),
                           ("event", "pull_request"), ("head_branch", "other"), ("check_suite_id", 999),
                           ("head_repository", {"full_name": "attacker/fork"}), ("conclusion", "failure")):
            api = FakeAPI()
            api.run[key] = value
            with self.subTest(key=key), self.assertRaises(merge_gate.GateError): self.authorize(api)

    def test_missing_duplicate_and_non_actions_checks_fail(self):
        for variant in ("missing", "duplicate", "foreign-app", "foreign-url"):
            api = FakeAPI()
            if variant == "missing": api.checks = []
            if variant == "duplicate": api.checks.append(copy.deepcopy(api.check))
            if variant == "foreign-app": api.check["app"]["slug"] = "external-ci"
            if variant == "foreign-url": api.check["details_url"] = "https://github.com/other/repo/actions/runs/42"
            with self.subTest(variant=variant), self.assertRaises(merge_gate.GateError): self.authorize(api)

    def test_branch_authority_cannot_be_bypassed(self):
        for variant in ("wrong-source", "unprotected", "missing-required", "non-strict", "admin-bypass", "force", "delete"):
            api = FakeAPI()
            if variant == "wrong-source": api.branch["commit"]["sha"] = "b" * 40
            if variant == "unprotected": api.branch["protected"] = False
            if variant == "missing-required": api.protection["required_status_checks"]["contexts"] = []
            if variant == "non-strict": api.protection["required_status_checks"]["strict"] = False
            if variant == "admin-bypass": api.protection["enforce_admins"]["enabled"] = False
            if variant == "force": api.protection["allow_force_pushes"]["enabled"] = True
            if variant == "delete": api.protection["allow_deletions"]["enabled"] = True
            with self.subTest(variant=variant), self.assertRaises(merge_gate.GateError): self.authorize(api)


class InventoryAndCoverageTests(unittest.TestCase):
    def test_proposed_removals_are_visible_without_rewriting_authority(self):
        self.assertEqual(test_inventory.difference(["required", "old"], ["required", "new"]),
                         {"added": ["new"], "removed": ["old"]})

    def test_ambiguous_binary_inventory_fails(self):
        with self.assertRaises(ValueError):
            test_inventory.flatten({"rust-suites": {"a": {"status": "listed", "testcases": {"same": {}}},
                                                    "b": {"status": "listed", "testcases": {"same": {}}}}})

    def test_unregistered_worker_or_covered_negative_line_fails(self):
        sources = {"lib.rs": coverage_tool_canary.LIB, "worker.rs": coverage_tool_canary.WORKER}
        files = []
        for name, source in sources.items():
            segments = [[i, 5, int("CANARY_MISS" not in line), True, True, False]
                        for i, line in enumerate(source.splitlines(), 1) if "CANARY_" in line]
            files.append({"filename": name, "segments": segments})
        data = {"data": [{"files": files}]}
        self.assertEqual(len(coverage_tool_canary.observations(data, sources)), 5)
        missing = copy.deepcopy(data)
        missing["data"][0]["files"].pop()
        with self.assertRaises(ValueError): coverage_tool_canary.observations(missing, sources)
        data["data"][0]["files"][0]["segments"][1][2] = 1
        with self.assertRaises(ValueError): coverage_tool_canary.observations(data, sources)


if __name__ == "__main__": unittest.main()
