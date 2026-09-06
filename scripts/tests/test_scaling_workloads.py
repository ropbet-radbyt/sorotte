from __future__ import annotations

import copy
from contextlib import ExitStack, redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import scaling_workloads as scaling


def sample():
    return {"schema": scaling.SAMPLE_SCHEMA, "correctness": "passed", "fixture_version": 2,
            "fixture": {"name": "normal", **{key: 1 for key in ("roster", "empty_rooms", "metadata_bytes", "playlist_items", "server_playlist_items", "inventory", "anchors_per_file", "gui_pumps", "churn_cycles")}},
            "server": {"playlist": {"accepted_items": 1, "accepted_recipients": 1}},
            "network": {"retained_connections": 0, "retained_network_workers": 0, "joined_network_workers": True, "queue_byte_limit": 100,
                        "checkpoints": [{"resources": {"active_connections": 0, "unauthenticated_connections": 0, "queued_bytes": 0, "address_buckets": 0, "peak_queued_bytes": 10}}]},
            "media": {"retained_staging_directories": 0, "inventory_count": 1, "fingerprint_count": 1},
            "recovery": {"maximum_retained_attempts": 1}, "gui": {"projection": {"pump_nanoseconds": [25]}}}


class ScalingTests(unittest.TestCase):
    def test_independent_resource_checks_reject_false_success_labels(self):
        scaling.validate_sample(sample(), "normal")
        mutations = [
            lambda v: v["network"].update(retained_connections=1),
            lambda v: v["server"]["playlist"].update(accepted_items=0),
            lambda v: v["server"]["playlist"].update(accepted_recipients=0),
            lambda v: v["network"].update(joined_network_workers=False),
            lambda v: v["network"]["checkpoints"][0]["resources"].update(queued_bytes=1),
            lambda v: v["network"]["checkpoints"][0]["resources"].update(peak_queued_bytes=101),
            lambda v: v["media"].update(retained_staging_directories=1),
            lambda v: v["recovery"].update(maximum_retained_attempts=3),
            lambda v: v["fixture"].update(roster=True),
            lambda v: v["gui"]["projection"].update(pump_nanoseconds=[]),
        ]
        for mutate in mutations:
            value = sample()
            mutate(value)
            with self.subTest(value=value), self.assertRaises(ValueError):
                scaling.validate_sample(value, "normal")

    def test_distributions_preserve_all_pumps_without_timing_gates(self):
        result = scaling.distribution([1, 3, 5, 10000])
        self.assertEqual(result["count"], 4)
        self.assertEqual(result["median"], 4)
        self.assertEqual(result["p95"], 10000)
        self.assertGreater(result["standard_deviation"], 0)
        self.assertEqual(scaling.numeric_metrics({"pumps": [2, 3], "ready": True}), {"pumps": [2, 3]})
        for invalid in ([], [True], [float("nan")], [float("inf")]):
            with self.assertRaises(scaling.ScalingError):
                scaling.distribution(invalid)

    def test_named_comparison_rejects_incomparable_fixtures_and_hardware(self):
        baseline = {"schema": scaling.SCHEMA, "name": "windows-dev", "source": {"sha": "a" * 40}, "profile": "dev",
                    "hardware": {"system": "Windows"}, "features": scaling.FEATURES,
                    "cases": {"normal": {"fixture": {"roster": 4}, "distributions": {"allocations": {"median": 10}}}}}
        current = copy.deepcopy(baseline)
        current["cases"]["normal"]["distributions"]["allocations"]["median"] = 20
        result = scaling.compare(current, baseline, "windows-dev")
        self.assertEqual(result["metrics"]["normal"]["allocations"]["delta_percent"], 100)
        self.assertIsNone(result["timing_thresholds"])
        for field, value in (("name", "another"), ("profile", "release"), ("hardware", {"system": "Linux"}), ("cases", {})):
            bad = copy.deepcopy(baseline)
            bad[field] = value
            with self.subTest(field=field), self.assertRaises(scaling.ScalingError):
                scaling.compare(current, bad, "windows-dev")
        bad = copy.deepcopy(baseline)
        bad["cases"]["normal"]["fixture"]["roster"] = 8
        with self.assertRaises(scaling.ScalingError):
            scaling.compare(current, bad, "windows-dev")

    def test_cli_refuses_ambiguous_baseline_before_build_or_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline = root / "baseline.json"
            for index, contents in enumerate(('{"schema":"bad","schema":"sorotte-scaling-report-v1"}', '{"median":NaN}', '{} trailing')):
                output = root / f"report-{index}.json"
                baseline.write_text(contents, encoding="utf-8")
                result = subprocess.run([sys.executable, str(Path(scaling.__file__)), "--name", "candidate", "--output", str(output),
                                         "--baseline", str(baseline), "--baseline-name", "baseline", "--skip-build"], capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Traceback", result.stderr)
                self.assertFalse(output.exists())


class ScalingAttemptTests(unittest.TestCase):
    def invoke(self, root, samples, *, identities=None, build=False, count=2):
        output = root / "report.json"
        binary = root / "target/debug/examples" / ("scaling_workloads.exe" if os.name == "nt" else "scaling_workloads")
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"fixture binary identity")
        identity = {"sha": "a" * 40, "dirty": True, "working_source_sha256": "b" * 64,
                    "inputs": {".cargo/config.toml": "c" * 64}}
        original_command = scaling.command

        def metadata(argv, **kwargs):
            if argv == ["rustc", "-vV"]:
                return "rustc fixture compiler identity"
            if argv[:2] == ["cargo", "build"]:
                return ""
            return original_command(argv, **kwargs)

        with ExitStack() as stack:
            stack.enter_context(mock.patch.object(scaling, "ROOT", root))
            stack.enter_context(mock.patch.object(scaling, "source_identity", side_effect=identities, return_value=identity))
            stack.enter_context(mock.patch.object(scaling, "hardware", return_value={"system": "test"}))
            stack.enter_context(mock.patch.object(scaling, "command", side_effect=metadata))
            sampled = stack.enter_context(mock.patch.object(scaling, "run_sample", side_effect=samples))
            stack.enter_context(redirect_stdout(io.StringIO()))
            stack.enter_context(redirect_stderr(io.StringIO()))
            result = scaling.main(["--name", "attempt-test", "--output", str(output), "--target-dir", str(root / "target"),
                                   "--samples", str(count), "--warmup", "0", "--cases", "normal", *([] if build else ["--skip-build"])])
        receipt = json.loads((root / "report.json.attempt/receipt.json").read_text())
        return result, receipt, sampled.call_count

    def test_success_preserves_report_schema_and_hashes_each_completed_observation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result, receipt, calls = self.invoke(root, [sample(), sample()])
            self.assertEqual((result, receipt["status"], calls), (0, "passed", 2))
            report = json.loads((root / "report.json").read_text())
            self.assertEqual(report["schema"], scaling.SCHEMA)
            self.assertEqual(len(report["cases"]["normal"]["raw_samples"]), 2)
            self.assertNotIn("inputs", report["source"])
            self.assertEqual(receipt["report"]["sha256"], scaling.sha256_file(root / "report.json"))
            self.assertEqual(receipt["source_before"], receipt["source_after"])
            self.assertEqual(receipt["binary"]["sha256_before"], receipt["binary"]["sha256_after"])
            for observation in receipt["observations"]:
                path = root / "report.json.attempt" / observation["path"]
                self.assertEqual(scaling.sha256_file(path), observation["sha256"])
                self.assertEqual(json.loads(path.read_text()), sample())
            self.assertIsNone(receipt["first_failure"])

    def test_later_real_process_failure_retains_prior_sample_and_replay_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            count = 0

            def samples(_binary, _case, **kwargs):
                nonlocal count
                count += 1
                if count == 1:
                    return sample()
                return scaling.command([sys.executable, "-c", "import sys; print('sample evidence'); print('real fixture failure', file=sys.stderr); sys.exit(7)"],
                                       root=root, timeout=5, attempt=kwargs["attempt"], label=kwargs["label"])

            result, receipt, calls = self.invoke(root, samples)
            self.assertEqual((result, receipt["status"], calls), (1, "failed", 2))
            self.assertEqual(len(receipt["observations"]), 1)
            self.assertFalse((root / "report.json").exists())
            failure = receipt["first_failure"]
            self.assertEqual(failure["phase"], "normal-sample-2")
            self.assertEqual(failure["command"][0], sys.executable)
            log = root / "report.json.attempt" / failure["diagnostics"]
            self.assertIn("sample evidence", (log / "stdout.txt").read_text())
            self.assertIn("real fixture failure", (log / "stderr.txt").read_text())
            self.assertEqual(receipt["commands"][-1]["execution"]["returncode"], 7)
            self.assertEqual(receipt["commands"][-1]["execution"]["cleanup"]["status"], "passed")
            self.assertGreater(receipt["elapsed_seconds"], 0)
            self.assertEqual(receipt["source_before"], receipt["source_after"])
            # The real failed attempt is consumable by the standard ledger and
            # human-readable view without a scaling-specific schema adapter.
            import verify
            import verification_ledger
            path = root / "report.json.attempt/receipt.json"
            index = verify.ledger([path], "a" * 40)
            rendered = verification_ledger.render(index)
            self.assertEqual(index["entries"][0]["source_claims"], {"identity.source_sha": "a" * 40})
            self.assertIn("real fixture failure", rendered)
            self.assertIn("FRESH_ATTEMPT_REPORT.json", rendered)
            self.assertIn("normal-sample-2", rendered)
            self.assertEqual(receipt["cleanup"]["status"], "passed")
            self.assertEqual(receipt["identity"]["working_source_sha256"], "b" * 64)
            with self.assertRaisesRegex(ValueError, "source mismatch"):
                verify.ledger([path], "d" * 40)

    def test_real_timeout_kills_owned_descendants_and_preserves_partial_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            escaped = root / "escaped-child.txt"
            started = root / "child-started.txt"
            child = f"import pathlib,time; pathlib.Path({str(started)!r}).write_text('started'); time.sleep(1.5); pathlib.Path({str(escaped)!r}).write_text('escaped')"
            parent = f"import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',{child!r}]); print('waiting on owned child', flush=True); time.sleep(20)"
            count = 0

            def samples(_binary, _case, **kwargs):
                nonlocal count
                count += 1
                if count == 1:
                    return sample()
                return scaling.command([sys.executable, "-c", parent], root=root, timeout=1,
                                       attempt=kwargs["attempt"], label=kwargs["label"])

            result, receipt, calls = self.invoke(root, samples)
            self.assertEqual((result, receipt["status"], calls), (1, "timeout", 2))
            self.assertTrue(started.is_file(), "the timeout must exercise an actually started descendant")
            time.sleep(1)
            self.assertFalse(escaped.exists())
            self.assertEqual(len(receipt["observations"]), 1)
            self.assertEqual(receipt["first_failure"]["phase"], "normal-sample-2")
            execution = receipt["commands"][-1]["execution"]
            self.assertEqual(execution["status"], "timeout")
            self.assertEqual(execution["cleanup"]["status"], "passed")
            self.assertIn(execution["cleanup"]["ownership"], ("job-object", "process-group"))
            self.assertGreaterEqual(receipt["elapsed_seconds"], 1)
            self.assertEqual(receipt["source_before"], receipt["source_after"])
            self.assertIsNotNone(receipt["first_failure"]["command"])
            self.assertFalse((root / "report.json").exists())

    def test_sample_output_is_bounded_before_json_parsing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def samples(_binary, _case, **kwargs):
                return scaling.command([sys.executable, "-c", "print('x' * 4096)"], root=root, timeout=5,
                                       attempt=kwargs["attempt"], label=kwargs["label"], max_bytes=128)

            result, receipt, calls = self.invoke(root, samples)
            self.assertEqual((result, receipt["status"], calls), (1, "failed", 1))
            self.assertEqual(receipt["commands"][-1]["execution"]["status"], "output-limit")
            self.assertLessEqual(receipt["commands"][-1]["execution"]["capture"]["stdout"]["bytes"], 128)
            self.assertFalse((root / "report.json").exists())

    def test_source_change_after_measurement_rejects_success_and_retains_every_sample(self):
        before = {"sha": "a" * 40, "dirty": True, "working_source_sha256": "b" * 64, "inputs": {".cargo/config.toml": "c" * 64}}
        after = {**before, "working_source_sha256": "d" * 64, "inputs": {".cargo/config.toml": "e" * 64}}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result, receipt, calls = self.invoke(root, [sample(), sample()], identities=[before, after])
            self.assertEqual((result, receipt["status"], calls), (1, "failed", 2))
            self.assertEqual(len(receipt["observations"]), 2)
            self.assertEqual(receipt["source_before"], before)
            self.assertEqual(receipt["source_after"], after)
            self.assertEqual(receipt["first_failure"]["phase"], "source-after-samples")
            self.assertFalse((root / "report.json").exists())

    def test_build_source_change_stops_before_any_measurement(self):
        before = {"sha": "a" * 40, "dirty": True, "working_source_sha256": "b" * 64}
        after = {**before, "sha": "c" * 40}
        with tempfile.TemporaryDirectory() as directory:
            result, receipt, calls = self.invoke(Path(directory), [], identities=[before, after, after], build=True)
            self.assertEqual((result, receipt["status"], calls), (1, "failed", 0))
            self.assertEqual(receipt["source_after_build"], after)
            self.assertEqual(receipt["first_failure"]["phase"], "source-after-build")

    def test_binary_change_and_cancellation_cannot_publish_a_report(self):
        for failure in ("binary-change", "cancel"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)

                def samples(binary, _case, **kwargs):
                    if failure == "cancel":
                        raise KeyboardInterrupt()
                    binary.write_bytes(b"substituted binary")
                    return sample()

                result, receipt, _ = self.invoke(root, samples, count=1)
                self.assertEqual(result, 1)
                self.assertEqual(receipt["status"], "cancelled" if failure == "cancel" else "failed")
                self.assertIsNotNone(receipt["source_before"])
                self.assertIsNotNone(receipt["source_after"])
                self.assertFalse((root / "report.json").exists())

    def test_a_previous_report_or_attempt_is_never_rewritten(self):
        for previous in ("report", "attempt"):
            with self.subTest(previous=previous), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output = root / "report.json"
                if previous == "report":
                    evidence = output
                else:
                    evidence = root / "report.json.attempt/receipt.json"
                    evidence.parent.mkdir()
                evidence.write_text("preserved earlier evidence")
                with mock.patch.object(scaling, "source_identity") as source, redirect_stderr(io.StringIO()):
                    result = scaling.main(["--name", "replay", "--output", str(output)])
                self.assertEqual(result, 1)
                source.assert_not_called()
                self.assertEqual(evidence.read_text(), "preserved earlier evidence")

    def test_cargo_configuration_and_actual_deleted_inputs_are_bound(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = root / ".cargo/config.toml"
            config.parent.mkdir()
            config.write_text('[build]\njobs = 2\n')
            (root / "Cargo.lock").write_text("version = 4\n")
            names = [".cargo/config.toml", "Cargo.lock", "crates/deleted.rs", "docs/changed.md"]

            def git(argv, **kwargs):
                if "ls-files" in argv:
                    return "\0".join(names) + "\0"
                if "rev-parse" in argv:
                    return "a" * 40
                return " M .cargo/config.toml"

            with mock.patch.object(scaling, "command", side_effect=git):
                before = scaling.source_identity(root, with_inputs=True)
                config.write_text('[build]\njobs = 4\n')
                after = scaling.source_identity(root, with_inputs=True)
            self.assertNotEqual(before["working_source_sha256"], after["working_source_sha256"])
            self.assertEqual(after["inputs"][".cargo/config.toml"], scaling.sha256_file(config))
            self.assertEqual(after["inputs"]["crates/deleted.rs"], "deleted")
            self.assertNotIn("docs/changed.md", after["inputs"])


if __name__ == "__main__":
    unittest.main()
