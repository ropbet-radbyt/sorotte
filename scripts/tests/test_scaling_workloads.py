from __future__ import annotations

import copy
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

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
            baseline, output = root / "baseline.json", root / "report.json"
            for contents in ('{"schema":"bad","schema":"sorotte-scaling-report-v1"}', '{"median":NaN}', '{} trailing'):
                baseline.write_text(contents, encoding="utf-8")
                result = subprocess.run([sys.executable, str(Path(scaling.__file__)), "--name", "candidate", "--output", str(output),
                                         "--baseline", str(baseline), "--baseline-name", "baseline", "--skip-build"], capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Traceback", result.stderr)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
