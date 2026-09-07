from __future__ import annotations

import copy
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import assurance_registry
import coverage_tool_canary
import mutation_process
import verification_ledger
import verification_tools
import verify


class ObservationTests(unittest.TestCase):
    def test_expected_child_fault_does_not_hide_the_owning_selftest_failure(self):
        result = subprocess.CompletedProcess([], 1, "error: intentional child fault\n", "FAIL: test_bad_map (test_coverage.Policy)\n")
        self.assertEqual(verify.primary_failure(result), "FAIL: test_bad_map (test_coverage.Policy)")

    def job(self, **changes):
        return dict(id=1, head_sha="a" * 40, run_id=5, run_attempt=1, status="completed", conclusion="success",
                    started_at="2026-09-06T01:00:00Z", completed_at="2026-09-06T01:02:00Z", steps=[]) | changes

    def test_parallel_job_minutes_are_distinct_from_span_and_retry_is_not_a_flake(self):
        jobs = [self.job(), self.job(id=2, conclusion="cancelled", run_attempt=2,
                steps=[dict(name="Setup dependencies", started_at="2026-09-06T01:00:00Z",
                            completed_at="2026-09-06T01:00:30Z", conclusion="failure")])]
        report = verification_ledger.metrics(jobs, "a" * 40)
        self.assertEqual((report["job_minutes"], report["observed_job_span_seconds"]), (4, 120))
        self.assertEqual((report["cancelled_job_minutes"], report["first_failed_step_seconds"]), (2, 30))
        self.assertEqual(report["setup_step_minutes"], 0.5)
        self.assertIsNone(report["genuine_flaky_cases"])

    def test_missing_source_duplicate_and_impossible_timing_cannot_be_counted(self):
        for jobs in ([self.job(head_sha="b" * 40)], [self.job(), self.job()],
                     [self.job(completed_at="2026-09-06T00:00:00Z")]):
            with self.assertRaises(ValueError): verification_ledger.metrics(jobs, "a" * 40)
        self.assertEqual(verification_ledger.metrics([self.job(status="queued")], "a" * 40)["incomplete_jobs"], [1])

    def test_classification_is_separate_and_bound_to_original_failed_attempt(self):
        with tempfile.TemporaryDirectory() as folder:
            receipt = Path(folder) / "attempt.json"
            receipt.write_text('{"status":"failed"}', encoding="utf-8")
            original = receipt.read_bytes()
            note = verification_ledger.annotate(receipt, "harness-defect", "The trace reproduces a missing object map.")
            self.assertEqual(note["receipt_sha256"], verification_tools.digest(receipt))
            self.assertEqual(receipt.read_bytes(), original)
            with self.assertRaises(ValueError): verification_ledger.annotate(receipt, "environment-unavailable", " ")

    def test_markdown_index_preserves_failure_replay_and_input_identity(self):
        text = verification_ledger.render(dict(source_sha="a" * 40, entries=[dict(path="attempt.json", sha256="b" * 64,
            receipt=dict(kind="verification-attempt", lane="behavior", status="failed", duration_seconds=12,
                         primary_failure="missing Hello | frame", cleanup={"status": "passed"},
                         identity={"working_source_sha256": "c" * 64}, replay_command=["cargo", "test"]))]))
        for value in ("missing Hello \\| frame", "cargo", "c" * 64, "passed", "unclassified"):
            self.assertIn(value, text)

    def test_successful_command_with_source_drift_remains_failed_and_preserves_cleanup(self):
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / "attempt"
            def process(*args, **kwargs):
                root = kwargs["log_root"]
                root.mkdir()
                (root / "process.json").write_text(json.dumps({"cleanup": {"status": "passed"}}), encoding="utf-8")
                return subprocess.CompletedProcess([], 0, "", "")
            with mock.patch.object(verify, "identity", side_effect=[{"source_sha": "a" * 40}, {"source_sha": "b" * 40}]), mock.patch.object(mutation_process, "run", side_effect=process):
                result = verify.run_lane("static", output, 60)
            self.assertEqual(result["status"], "failed")
            self.assertIn("drift", result["primary_failure"])
            self.assertEqual(result["cleanup"]["status"], "passed")
            with self.assertRaisesRegex(ValueError, "already exists"): verify.run_lane("static", output, 60)


class AssuranceAndCoverageTests(unittest.TestCase):
    def registry(self):
        return {"schema_version": 1, "capabilities": [dict(id="display", owner="gui", command="manual-profile",
                environment="isolated desktop", cadence="monthly", execution="manual", max_age_days=35, evidence=None)]}

    def test_unavailable_stale_and_current_proofs_are_distinct(self):
        now = datetime(2026, 9, 6, tzinfo=timezone.utc)
        registry = self.registry()
        self.assertEqual(assurance_registry.evaluate(registry, now)[0]["freshness"], "unavailable")
        proof = dict(source_sha="a" * 40, completed_at="2026-09-01T00:00:00Z", artifact_sha256="b" * 64, url="https://github.com/owner/repo/releases/download/proof.zip")
        registry["capabilities"][0]["evidence"] = proof
        self.assertEqual(assurance_registry.evaluate(registry, now)[0]["freshness"], "current")
        proof["completed_at"] = "2026-01-01T00:00:00Z"
        self.assertEqual(assurance_registry.evaluate(registry, now)[0]["freshness"], "stale")
        for field, value in (("artifact_sha256", "not-a-hash"), ("completed_at", "2026-10-01T00:00:00Z"), ("completed_at", "2026-09-01T00:00:00")):
            broken = copy.deepcopy(registry)
            broken["capabilities"][0]["evidence"][field] = value
            with self.assertRaises(ValueError): assurance_registry.evaluate(broken, now)

    def test_independent_source_view_rejects_missing_zeroed_or_duplicate_worker(self):
        lines = [f"{number}| {int(name != 'CANARY_MISS')}| value // {name}" for number, name in enumerate(
                 ["CANARY_UNIT", "CANARY_MISS", "CANARY_CHILD", "CANARY_STANDALONE", "CANARY_MULTILINE"], 1)]
        self.assertEqual(len(coverage_tool_canary.source_view_observations("\n".join(lines))), 5)
        for broken in (lines[:-1], lines + [lines[-1]], [line.replace("| 1|", "| 0|") for line in lines]):
            with self.assertRaises(ValueError): coverage_tool_canary.source_view_observations("\n".join(broken))


if __name__ == "__main__": unittest.main()
