from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verification_ledger
import verify


class GateAttemptTests(unittest.TestCase):
    def setUp(self):
        self.source = verify.git("rev-parse", "HEAD")
        self.jobs = json.loads(verify.POLICY.read_text(encoding="utf-8"))["gate_producers"]["fuzz"]

    def arguments(self, output: Path, *, result="success", **changes):
        return argparse.Namespace(lane="fuzz", selected="true", source_sha=self.source,
            base_sha=None, plan=None, expected_job=self.jobs,
            job_result=[f"{job}={result}" for job in self.jobs], output=output, **changes)

    def cli(self, output: Path, *, result="success", selected="true", extra=()):
        command = [sys.executable, str(verify.ROOT / "scripts/verify.py"), "gate", "--lane", "fuzz",
                   "--selected", selected, "--source-sha", self.source, "--output", str(output)]
        for job in self.jobs:
            command.extend(("--expected-job", job, "--job-result", f"{job}={result}"))
        return subprocess.run([*command, *extra], cwd=verify.ROOT, capture_output=True,
                              text=True, encoding="utf-8", timeout=30)

    def test_failed_producers_leave_source_bound_replayable_decision(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "decision.json"
            result = self.cli(path, result="failure")
            self.assertEqual(result.returncode, 1, result.stderr)
            receipt = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(receipt["status"], "failed")
            self.assertEqual(receipt["observed_checkout_sha"], self.source)
            self.assertIn("requires success producers", receipt["primary_failure"])
            self.assertEqual(receipt["replay_command"][-1], "FRESH_GATE_ATTEMPT.json")
            rendered = verification_ledger.render(verify.ledger([path], self.source))
            self.assertIn("requires success producers", rendered)
            self.assertIn("failed", rendered)

    def test_missing_selection_artifact_remains_a_primary_failure_receipt(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "decision.json"
            missing = Path(folder) / "missing-plan.json"
            result = self.cli(path, result="skipped", selected="false",
                              extra=("--plan", str(missing), "--base-sha", self.source))
            self.assertEqual(result.returncode, 1, result.stderr)
            receipt = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(receipt["status"], "failed")
            self.assertIn("missing-plan.json", receipt["primary_failure"])
            self.assertFalse(receipt["selected"])

    def test_a_later_success_cannot_overwrite_a_failed_attempt(self):
        with tempfile.TemporaryDirectory() as folder:
            first = Path(folder) / "first.json"
            self.assertEqual(self.cli(first, result="cancelled").returncode, 1)
            original = first.read_bytes()
            self.assertEqual(self.cli(first).returncode, 1)
            self.assertEqual(first.read_bytes(), original)
            second = Path(folder) / "second.json"
            result = self.cli(second)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(second.read_text(encoding="utf-8"))["status"], "passed")

    def test_interrupted_validation_leaves_an_incomplete_then_cancelled_record(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "interrupted.json"
            def interrupt(*args):
                self.assertEqual(json.loads(path.read_text(encoding="utf-8"))["status"], "incomplete")
                raise KeyboardInterrupt()
            with mock.patch.object(verify, "gate", side_effect=interrupt):
                with self.assertRaises(KeyboardInterrupt):
                    verify.gate_attempt(self.arguments(path))
            receipt = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(receipt["status"], "cancelled")
            self.assertEqual(receipt["primary_failure"], "KeyboardInterrupt")


if __name__ == "__main__":
    unittest.main()
