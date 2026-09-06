from __future__ import annotations

import contextlib
import copy
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
from urllib.error import HTTPError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import merge_gate as gate

SHA = "a" * 40
WORKFLOW = ".github/workflows/rust-ci.yml"
REQUIRED = {"verification-required": WORKFLOW, "merge-required": WORKFLOW}


class FakeAPI:
    repository = "owner/repo"

    def __init__(self):
        self.branch = {"protected": True, "commit": {"sha": SHA}}
        self.protection = {
            "required_status_checks": {"strict": True, "contexts": list(REQUIRED)},
            "enforce_admins": {"enabled": True}, "allow_force_pushes": {"enabled": False},
            "allow_deletions": {"enabled": False},
        }
        self.runs = {42: {
            "id": 42, "head_sha": SHA, "path": WORKFLOW,
            "repository": {"full_name": self.repository}, "head_repository": {"full_name": self.repository},
            "event": "push", "head_branch": "main", "check_suite_id": 9,
            "status": "completed", "conclusion": "success", "run_attempt": 2,
            "html_url": f"https://github.com/{self.repository}/actions/runs/42",
        }}
        self.checks = [{
            "id": index + 1, "name": name, "head_sha": SHA, "status": "completed", "conclusion": "success",
            "app": {"slug": "github-actions"}, "check_suite": {"id": 9},
            "details_url": self.runs[42]["html_url"] + f"/job/{index + 10}",
            "completed_at": "2026-09-06T01:00:00Z",
        } for index, name in enumerate(REQUIRED)]
        self.snapshots = []
        self.calls = []
        self.branch_versions = []

    def get(self, path):
        self.calls.append(path)
        if path == "branches/main":
            value = self.branch_versions.pop(0) if self.branch_versions else self.branch
        elif path == gate.PROTECTION_PATH:
            value = self.protection
        else:
            value = self.runs[int(path.removeprefix("actions/runs/"))]
        return copy.deepcopy(value)

    def pages(self, path, key):
        assert path == f"commits/{SHA}/check-runs?filter=latest"
        assert key == "check_runs"
        self.calls.append(path)
        return copy.deepcopy(self.snapshots.pop(0) if self.snapshots else self.checks)


class TokenBoundaryTests(unittest.TestCase):
    def test_only_exact_classic_protection_endpoint_receives_app_token(self):
        api = gate.GitHub("owner/repo", "normal-token", "protection-token")
        paths = ["branches/main", gate.PROTECTION_PATH,
                 f"commits/{SHA}/check-runs?filter=latest", "actions/runs/42"]
        with mock.patch.object(gate, "urlopen", side_effect=lambda *_args, **_kwargs: io.StringIO("{}")) as http:
            for path in paths:
                api.get(path)
        observed = [(call.args[0].full_url, call.args[0].get_header("Authorization")) for call in http.call_args_list]
        self.assertEqual(observed, [(f"https://api.github.com/repos/owner/repo/{path}",
                                    "Bearer protection-token" if path == gate.PROTECTION_PATH else "Bearer normal-token")
                                   for path in paths])

    def test_missing_app_token_fails_before_http_and_removes_stale_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "old-authorization.json"
            output.write_text('{"status":"passed"}')
            with mock.patch.dict(os.environ, {"GH_TOKEN": "normal-token"}, clear=True), mock.patch.object(gate, "urlopen") as http, contextlib.redirect_stderr(io.StringIO()) as errors:
                result = gate.main(["authorize-release", "--candidate-sha", SHA, "--repository", "owner/repo", "--output", str(output)])
            self.assertEqual(result, 1)
            self.assertFalse(output.exists())
            http.assert_not_called()
            self.assertIn("SOROTTE_PROTECTION_APP_PRIVATE_KEY", errors.getvalue())
            self.assertIn("docs/PROTECTION_READER_SETUP.md", errors.getvalue())

    def test_missing_or_insufficient_protection_never_falls_back_to_normal_token(self):
        for status in (401, 403, 404):
            with self.subTest(status=status):
                api = gate.GitHub("owner/repo", "normal-token", "protection-token")
                failure = HTTPError("https://api.github.com/", status, "denied", {}, None)
                with mock.patch.object(gate, "urlopen", side_effect=failure) as http:
                    with self.assertRaisesRegex(gate.GateError, "Administration-read"):
                        api.get(gate.PROTECTION_PATH)
                self.assertEqual(http.call_count, 1)
                self.assertEqual(http.call_args.args[0].get_header("Authorization"), "Bearer protection-token")
        with mock.patch.object(gate, "urlopen") as http, self.assertRaisesRegex(gate.GateError, "SOROTTE_PROTECTION_TOKEN"):
            gate.GitHub("owner/repo", "normal-token").get(gate.PROTECTION_PATH)
        http.assert_not_called()

    def test_wait_cli_drops_app_token_and_emits_no_authorization_receipt(self):
        api = FakeAPI()
        with tempfile.TemporaryDirectory() as temporary:
            policy = Path(temporary) / "policy.json"
            policy.write_text(json.dumps({"required_checks": REQUIRED}))
            with mock.patch.dict(os.environ, {"GH_TOKEN": "normal-token", "SOROTTE_PROTECTION_TOKEN": "unused-app-token"}, clear=True), mock.patch.object(gate, "GitHub", return_value=api) as factory, mock.patch.object(gate, "POLICY", policy), contextlib.redirect_stdout(io.StringIO()) as output:
                result = gate.main(["wait-checks", "--candidate-sha", SHA, "--repository", "owner/repo"])
            self.assertEqual(result, 0)
            factory.assert_called_once_with("owner/repo", "normal-token", "")
            self.assertNotIn(gate.PROTECTION_PATH, api.calls)
            self.assertIn("protection authorization is still required", output.getvalue())
            self.assertEqual(list(Path(temporary).iterdir()), [policy])


class ReadinessTests(unittest.TestCase):
    def wait(self, api, *, seconds=30, poll=5):
        return gate.wait_checks(api, SHA, REQUIRED, wait_seconds=seconds, poll_seconds=poll)

    def test_pending_snapshot_skips_run_lookups_then_reuses_one_completed_run(self):
        api = FakeAPI()
        pending = copy.deepcopy(api.checks)
        pending[1].update(status="in_progress", conclusion=None)
        api.snapshots = [pending]
        def before_retry(_seconds):
            self.assertFalse(any(path.startswith("actions/runs/") for path in api.calls))
        with mock.patch.object(gate.time, "sleep", side_effect=before_retry) as sleep, contextlib.redirect_stdout(io.StringIO()):
            self.wait(api)
        sleep.assert_called_once()
        self.assertEqual(api.calls.count(f"commits/{SHA}/check-runs?filter=latest"), 2)
        self.assertEqual(api.calls.count("actions/runs/42"), 1)
        self.assertNotIn(gate.PROTECTION_PATH, api.calls)

    def test_missing_check_is_retryable_but_missing_policy_is_not(self):
        api = FakeAPI()
        api.snapshots = [[api.checks[0]]]
        with mock.patch.object(gate.time, "sleep") as sleep, contextlib.redirect_stdout(io.StringIO()):
            self.wait(api)
        sleep.assert_called_once()
        with self.assertRaisesRegex(gate.GateError, "policy"), mock.patch.object(gate.time, "sleep") as sleep:
            gate.wait_checks(api, SHA, {}, wait_seconds=30, poll_seconds=5)
        sleep.assert_not_called()

    def test_observed_failure_takes_precedence_over_another_missing_check(self):
        for conclusion in ("failure", "cancelled", "skipped", "timed_out", "neutral", "action_required", "stale", None):
            api = FakeAPI()
            api.checks = [api.checks[1]]
            api.checks[0]["conclusion"] = conclusion
            with self.subTest(conclusion=conclusion), mock.patch.object(gate.time, "sleep") as sleep:
                with self.assertRaises(gate.GateError) as failure:
                    self.wait(api)
                self.assertNotIsInstance(failure.exception, gate.PendingChecks)
                sleep.assert_not_called()
                self.assertFalse(any(path.startswith("actions/runs/") for path in api.calls))

    def test_pending_foreign_or_duplicate_authority_is_not_retried(self):
        for variant in ("source", "app", "url", "suite", "duplicate", "unknown-status", "conclusion-on-pending"):
            api = FakeAPI()
            api.checks[0].update(status="queued", conclusion=None)
            if variant == "source": api.checks[0]["head_sha"] = "b" * 40
            if variant == "app": api.checks[0]["app"] = {"slug": "someone-else"}
            if variant == "url": api.checks[0]["details_url"] = "https://github.com/other/repo/actions/runs/42"
            if variant == "suite": api.checks[0]["check_suite"] = {"id": True}
            if variant == "duplicate": api.checks.append(copy.deepcopy(api.checks[0]))
            if variant == "unknown-status": api.checks[0]["status"] = "unrecognized"
            if variant == "conclusion-on-pending": api.checks[0]["conclusion"] = "success"
            with self.subTest(variant=variant), mock.patch.object(gate.time, "sleep") as sleep:
                with self.assertRaises(gate.GateError) as failure:
                    self.wait(api)
                self.assertNotIsInstance(failure.exception, gate.PendingChecks)
                sleep.assert_not_called()

    def test_trusted_workflow_finishing_is_retryable_but_failed_or_foreign_run_is_not(self):
        api = FakeAPI()
        api.runs[42].update(status="in_progress", conclusion=None)
        with mock.patch.object(gate.time, "sleep", side_effect=lambda _seconds: api.runs[42].update(status="completed", conclusion="success")) as sleep, contextlib.redirect_stdout(io.StringIO()):
            self.wait(api)
        sleep.assert_called_once()
        for field, value in (("conclusion", "failure"), ("path", ".github/workflows/wrong.yml"),
                             ("head_branch", "other"), ("run_attempt", True), ("id", 43)):
            api = FakeAPI()
            api.runs[42][field] = value
            with self.subTest(field=field), mock.patch.object(gate.time, "sleep") as sleep:
                with self.assertRaises(gate.GateError) as failure:
                    self.wait(api)
                self.assertNotIsInstance(failure.exception, gate.PendingChecks)
                sleep.assert_not_called()

    def test_second_run_failure_is_not_hidden_by_first_pending_run(self):
        api = FakeAPI()
        api.runs[42].update(status="in_progress", conclusion=None)
        api.runs[43] = {**api.runs[42], "id": 43, "html_url": "https://github.com/owner/repo/actions/runs/43", "status": "completed", "conclusion": "failure"}
        api.checks[1]["details_url"] = api.runs[43]["html_url"] + "/job/20"
        with mock.patch.object(gate.time, "sleep") as sleep, self.assertRaisesRegex(gate.GateError, "failure"):
            self.wait(api)
        sleep.assert_not_called()

    def test_main_drift_and_api_errors_fail_without_retry(self):
        api = FakeAPI()
        api.branch_versions = [copy.deepcopy(api.branch), {"protected": True, "commit": {"sha": "b" * 40}}]
        with mock.patch.object(gate.time, "sleep") as sleep, self.assertRaisesRegex(gate.GateError, "exact current main"):
            self.wait(api)
        sleep.assert_not_called()
        api = FakeAPI()
        with mock.patch.object(api, "pages", side_effect=gate.GateError("GitHub authority lookup failed (403)")), mock.patch.object(gate.time, "sleep") as sleep, self.assertRaisesRegex(gate.GateError, "403"):
            self.wait(api)
        sleep.assert_not_called()

    def test_poll_delay_respects_deadline_and_times_out_without_old_green(self):
        api = FakeAPI()
        api.checks = []
        with mock.patch.object(gate.time, "monotonic", side_effect=[0, 0, 5]), mock.patch.object(gate.time, "sleep") as sleep, contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(gate.GateError, "timed out"):
            self.wait(api, seconds=5, poll=30)
        sleep.assert_called_once_with(5)
        self.assertFalse(any(path.startswith("actions/runs/") for path in api.calls))


class ReleaseAuthorizationTests(unittest.TestCase):
    def test_complete_receipt_records_exact_checks_and_one_lookup_per_workflow(self):
        api = FakeAPI()
        receipt = gate.authorize(api, SHA, REQUIRED)
        self.assertEqual(receipt["candidate_sha"], SHA)
        self.assertEqual(receipt["status"], "passed")
        self.assertEqual([item["name"] for item in receipt["producers"]], list(REQUIRED))
        self.assertTrue(all(item["run_id"] == 42 and item["run_attempt"] == 2 for item in receipt["producers"]))
        self.assertEqual(api.calls.count("actions/runs/42"), 1)
        self.assertEqual(api.calls.count(gate.PROTECTION_PATH), 2)

    def test_pending_authorization_does_not_wait_or_write_success(self):
        api = FakeAPI()
        api.checks = []
        with mock.patch.object(gate.time, "sleep") as sleep, self.assertRaises(gate.PendingChecks):
            gate.authorize(api, SHA, REQUIRED)
        sleep.assert_not_called()

    def test_omitted_or_non_boolean_protection_cannot_attest_absence_of_bypass(self):
        for flag in ("enforce_admins", "allow_force_pushes", "allow_deletions"):
            for value in (None, {}, {"enabled": 0}):
                api = FakeAPI()
                api.protection[flag] = value
                with self.subTest(flag=flag, value=value), self.assertRaises(gate.GateError):
                    gate.authorize(api, SHA, REQUIRED)

    def test_protection_change_during_authorization_requires_a_fresh_attempt(self):
        api = FakeAPI()
        original_get = api.get
        def changing(path):
            value = original_get(path)
            if path == "actions/runs/42":
                api.protection["required_status_checks"]["contexts"].append("new-required")
            return value
        with mock.patch.object(api, "get", side_effect=changing), self.assertRaisesRegex(gate.GateError, "changed during"):
            gate.authorize(api, SHA, REQUIRED)


if __name__ == "__main__":
    unittest.main()
