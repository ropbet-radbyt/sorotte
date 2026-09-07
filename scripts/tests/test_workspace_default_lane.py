from __future__ import annotations

from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify

try:
    from test_ci_policy import WORKFLOW_PATH, named_step, parse_workflow
except ModuleNotFoundError:
    from scripts.tests.test_ci_policy import WORKFLOW_PATH, named_step, parse_workflow


SHA = "a" * 40
SOURCE = {"source_sha": SHA, "working_source_sha256": "c" * 64}

# This executable models the existing workspace writer's protocol. These tests
# exercise real process supervision without compiling the product workspace.
WRITER = r'''
import argparse, json, pathlib, sys, time
parser = argparse.ArgumentParser()
parser.add_argument("command", choices=["workspace"])
for flag in ("repo-root", "candidate-sha", "platform", "features", "output"):
    parser.add_argument("--" + flag, required=True)
args = parser.parse_args()
output = pathlib.Path(args.output)
calls = output.parent / "calls.txt"
with calls.open("a", encoding="utf-8") as handle:
    handle.write("called\n")
(output.parent / "argv.json").write_text(json.dumps({"argv": sys.argv[1:], "cwd": str(pathlib.Path.cwd())}), encoding="utf-8")
mode = pathlib.Path("mode.txt").read_text(encoding="utf-8")
if mode == "fail":
    print("thread 'workspace_fixture' panicked: intentional producer failure", file=sys.stderr, flush=True)
    raise SystemExit(17)
if mode == "hang":
    print("workspace fixture reached deliberate hang", flush=True)
    time.sleep(600)
if mode == "omit":
    raise SystemExit(0)
value = {"schema_version": 1, "kind": "sorotte-release-workspace-receipt", "result": "passed",
         "candidate_sha": args.candidate_sha, "platform": args.platform, "features": args.features,
         "profile": "test", "instrumentation": "none", "command": ["cargo", "test", "--locked", "--workspace"],
         "source_files": {"Cargo.lock": "b" * 64}, "rustc": "fixture", "producer": {"run_id": "fixture"}}
output.write_text(json.dumps(value), encoding="utf-8")
'''


class DefaultWorkspaceLaneTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory(prefix="workspace lane "))).resolve()
        scripts = self.root / "scripts"
        scripts.mkdir()
        (scripts / "release_qualification.py").write_text(WRITER, encoding="utf-8")
        self.mode = self.root / "mode.txt"
        self.mode.write_text("pass", encoding="utf-8")
        self.output = self.root / "attempt"
        self.stack.enter_context(mock.patch.object(verify, "ROOT", self.root))
        self.identity = self.stack.enter_context(mock.patch.object(verify, "identity", return_value=SOURCE))
        self.stack.enter_context(mock.patch.dict(os.environ, {"RUST_TEST_THREADS": ""}))

    def record(self):
        return json.loads((self.output / "receipt.json").read_text(encoding="utf-8"))

    def assert_owned_process_closed(self, status):
        process = json.loads((self.output / "process/process.json").read_text(encoding="utf-8"))
        self.assertEqual(process["status"], status)
        self.assertEqual(process["cleanup"]["status"], "passed", process)
        self.assertEqual(self.record()["cleanup"], process["cleanup"])
        self.assertEqual((self.output / "calls.txt").read_text(encoding="utf-8"), "called\n")
        return process

    def test_real_supervisor_binds_checkout_subject_and_retains_default_writer_receipt(self):
        with mock.patch.dict(os.environ, {"GITHUB_SHA": "b" * 40, "VERIFICATION_SHA": "d" * 40}):
            record = verify.run_lane("workspace-default", self.output, 30)
        self.assertEqual(record["status"], "passed")
        self.assertEqual(record["identity"], SOURCE)
        self.assertEqual(record["command"][1:5], ["scripts/release_qualification.py", "workspace", "--repo-root", str(self.root)])
        self.assertEqual(record["command"][5:7], ["--candidate-sha", SHA])
        self.assertEqual(record["command"][-4:], ["--features", "default", "--output", str(self.output / "workspace.json")])
        self.assertEqual(record["workspace_receipt_sha256"], hashlib.sha256((self.output / "workspace.json").read_bytes()).hexdigest())
        argv = json.loads((self.output / "argv.json").read_text(encoding="utf-8"))
        self.assertEqual(argv["cwd"], str(self.root))
        self.assertEqual(argv["argv"], record["command"][2:])
        self.assertEqual(record["deadline_seconds"], 30)
        self.assertEqual(record["replay_command"][-2:], ["--deadline-seconds", "30"])
        self.assert_owned_process_closed("completed")
        before = (self.output / "receipt.json").read_bytes()
        with self.assertRaisesRegex(ValueError, "already exists"):
            verify.run_lane("workspace-default", self.output, 30)
        self.assertEqual((self.output / "receipt.json").read_bytes(), before)

    def test_failed_writer_is_not_retried_or_receipted_as_default_success(self):
        self.mode.write_text("fail", encoding="utf-8")
        record = verify.run_lane("workspace-default", self.output, 30)
        self.assertEqual(record["status"], "failed")
        self.assertIn("workspace_fixture", record["primary_failure"])
        self.assertNotIn("workspace_receipt_sha256", record)
        self.assertFalse((self.output / "workspace.json").exists())
        self.assertEqual(self.assert_owned_process_closed("completed")["returncode"], 17)

    def test_timeout_preserves_attempt_deadline_primary_failure_and_owned_cleanup(self):
        self.mode.write_text("hang", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "exceeded 10s deadline"):
            verify.run_lane("workspace-default", self.output, 10)
        record = self.record()
        self.assertEqual(record["status"], "failed")
        self.assertIn("exceeded 10s deadline", record["primary_failure"])
        self.assertGreater(record["duration_seconds"], 0)
        self.assertEqual(record["replay_command"][-2:], ["--deadline-seconds", "10"])
        self.assertNotIn("workspace_receipt_sha256", record)
        self.assert_owned_process_closed("timeout")

    def test_zero_exit_without_workspace_receipt_still_fails(self):
        self.mode.write_text("omit", encoding="utf-8")
        with self.assertRaises(FileNotFoundError):
            verify.run_lane("workspace-default", self.output, 30)
        self.assertEqual(self.record()["status"], "failed")
        self.assertIn("workspace.json", self.record()["primary_failure"])
        self.assert_owned_process_closed("completed")

    def test_source_drift_invalidates_otherwise_successful_writer(self):
        self.identity.side_effect = [SOURCE, {**SOURCE, "working_source_sha256": "f" * 64}]
        record = verify.run_lane("workspace-default", self.output, 30)
        self.assertEqual(record["status"], "failed")
        self.assertEqual(record["primary_failure"], "source or input drift during execution")
        self.assertTrue((self.output / "workspace.json").is_file())
        self.assert_owned_process_closed("completed")

    def test_platform_contract_does_not_mislabel_an_unsupported_runner(self):
        for system, machine, expected in (("Linux", "x86_64", "linux-x86_64"), ("Windows", "AMD64", "windows-x86_64"),
                                          ("Darwin", "x86_64", None), ("Linux", "aarch64", None)):
            with self.subTest(system=system, machine=machine), mock.patch.object(verify.platform, "system", return_value=system), \
                    mock.patch.object(verify.platform, "machine", return_value=machine):
                if expected:
                    self.assertEqual(verify.workspace_default_platform(), expected)
                else:
                    with self.assertRaisesRegex(ValueError, "Linux or Windows x86_64"):
                        verify.workspace_default_platform()

    def test_serial_override_fails_before_launch_and_preserves_reason(self):
        with mock.patch.dict(os.environ, {"RUST_TEST_THREADS": "1"}), self.assertRaisesRegex(ValueError, "RUST_TEST_THREADS"):
            verify.run_lane("workspace-default", self.output, 30)
        self.assertEqual(self.record()["status"], "failed")
        self.assertIn("RUST_TEST_THREADS", self.record()["primary_failure"])
        self.assertFalse((self.output / "calls.txt").exists())

    def test_wrong_source_mode_result_or_empty_inventory_cannot_certify_zero_exit(self):
        value = {"schema_version": 1, "kind": "sorotte-release-workspace-receipt", "result": "passed",
                 "candidate_sha": SHA, "platform": "linux-x86_64", "features": "default", "profile": "test",
                 "instrumentation": "none", "command": ["cargo", "test", "--locked", "--workspace"],
                 "source_files": {"Cargo.lock": "b" * 64}}
        path = self.root / "receipt.json"
        variants = [("schema_version", True), ("candidate_sha", "f" * 40), ("platform", "windows-x86_64"), ("features", "all"),
                    ("profile", "release"), ("instrumentation", "coverage"), ("result", "failed"),
                    ("command", ["cargo", "test", "--locked", "--workspace", "--", "--test-threads=1"]),
                    ("source_files", {})]
        for key, wrong in variants:
            with self.subTest(key=key):
                path.write_text(json.dumps({**value, key: wrong}), encoding="utf-8")
                with self.assertRaises(ValueError):
                    verify.validate_default_workspace_receipt(path, SHA, "linux-x86_64")


class DefaultWorkspaceGateExecutionTests(unittest.TestCase):
    def check_shell_gate(self, job, name, executable, flags, extension):
        jobs = parse_workflow(WORKFLOW_PATH.read_text(encoding="utf-8"))["jobs"]
        gate = named_step(jobs, job, name)
        with tempfile.TemporaryDirectory(prefix="workspace gate ") as temporary:
            script = Path(temporary) / ("gate" + extension)
            script.write_text(gate["run"], encoding="utf-8")
            for outcome in ("success", "failure", "skipped", "cancelled", ""):
                with self.subTest(job=job, outcome=outcome or "missing"):
                    environment = dict(os.environ, NEXTEST_OUTCOME="success", DOCTEST_OUTCOME="success")
                    environment.pop("DEFAULT_WORKSPACE_OUTCOME", None)
                    if outcome:
                        environment["DEFAULT_WORKSPACE_OUTCOME"] = outcome
                    result = subprocess.run([executable, *flags, str(script)], env=environment,
                                            capture_output=True, text=True, timeout=30)
                    self.assertEqual(result.returncode == 0, outcome == "success", result.stderr)

    def test_linux_gate_rejects_failed_skipped_and_missing_default_even_when_nextest_passes(self):
        if os.name == "nt":
            executable = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/bin/bash.exe"
            if not executable.is_file():
                self.skipTest("Git Bash is unavailable; Linux executes this gate on the hosted runner")
        else:
            executable = shutil.which("bash")
            if executable is None:
                self.skipTest("bash is unavailable")
        self.check_shell_gate("checks", "Enforce complete Linux test gate", str(executable),
                              ["--noprofile", "--norc", "-e", "-o", "pipefail"], ".sh")

    def test_windows_gate_rejects_failed_skipped_and_missing_default_even_when_nextest_passes(self):
        executable = shutil.which("pwsh")
        if executable is None:
            self.skipTest("PowerShell is unavailable; Windows executes this gate on the hosted runner")
        self.check_shell_gate("rust_windows_tests", "Enforce complete Windows test gate", executable,
                              ["-NoProfile", "-NonInteractive", "-File"], ".ps1")


if __name__ == "__main__":
    unittest.main()
