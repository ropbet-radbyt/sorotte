from __future__ import annotations

import copy
import importlib.util
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "playback_start_gate_system.py"
sys.path.insert(0, str(SCRIPT.parent))
SPEC = importlib.util.spec_from_file_location("playback_start_gate_system", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


def passing_report() -> dict[str, object]:
    return {
        "schema_version": MODULE.SCHEMA_VERSION,
        "kind": MODULE.REPORT_KIND,
        "result": "passed",
        "candidate_sha": "a" * 40,
        "candidate_binding": "exact-clean-head",
        "run_id": "start-test",
        "composition": ["actual-release-server", "external-phase-oracle"],
        "server": {"file_name": "sorotte-server.exe", "sha256": "b" * 64},
        "phase_coverage": list(MODULE.REQUIRED_PHASES),
        "transition_coverage": list(MODULE.REQUIRED_TRANSITIONS),
        "scenario_coverage": list(MODULE.REQUIRED_SCENARIOS),
        "checks": [f"check-{index}" for index in range(18)],
        "lifecycle_summary": {"result": "passed"},
        "artifacts": {
            "server_lifecycle": "server.jsonl",
            "oracle_lifecycle": "oracle.jsonl",
            "merged_lifecycle": "merged.jsonl",
            "lifecycle_summary": "summary.json",
            "server_stdout": "server.stdout.log",
            "server_stderr": "server.stderr.log",
        },
    }


class StartGateProjectionTests(unittest.TestCase):
    def test_projects_only_closed_readiness_and_barrier_extensions(self) -> None:
        message = {
            "Set": {
                "sorotteReadinessV2": {
                    "snapshot": {
                        "mediaGeneration": 4,
                        "startGatePhase": {
                            "phase": "committed",
                            "mediaGeneration": 4,
                            "readinessRevision": 8,
                            "playbackRevision": 9,
                        },
                    }
                },
                "sorottePlaybackBarrierV1": {
                    "commit": {"mediaGeneration": 4, "stateRevision": 9}
                },
            }
        }
        snapshot = MODULE.readiness_snapshot(message)
        self.assertIsNotNone(snapshot)
        assert snapshot is not None
        self.assertEqual(MODULE.snapshot_phase(snapshot), "committed")
        extension = MODULE.barrier_extension(message)
        self.assertIsNotNone(extension)
        assert extension is not None
        self.assertEqual(extension["commit"]["stateRevision"], 9)

    def test_ordered_walk_requires_every_non_degraded_start_phase(self) -> None:
        MODULE.validate_phase_walk(
            [
                "inactive",
                "waitingForIntent",
                "waitingForTechnicalReadiness",
                "readyToCommit",
                "committed",
                "inactive",
            ]
        )
        with self.assertRaisesRegex(MODULE.StartGateSystemError, "readyToCommit"):
            MODULE.validate_phase_walk(
                [
                    "inactive",
                    "waitingForIntent",
                    "waitingForTechnicalReadiness",
                    "committed",
                    "inactive",
                ]
            )


class StartGateReportTests(unittest.TestCase):
    def test_accepts_exact_closed_report(self) -> None:
        self.assertEqual(MODULE.validate_report(passing_report())["result"], "passed")

    def test_rejects_missing_phase_transition_or_scenario(self) -> None:
        for field in ("phase_coverage", "transition_coverage", "scenario_coverage"):
            with self.subTest(field=field):
                report = copy.deepcopy(passing_report())
                report[field] = report[field][:-1]
                with self.assertRaises(MODULE.StartGateSystemError):
                    MODULE.validate_report(report)

    def test_rejects_extra_schema_field_and_non_basename_artifact(self) -> None:
        extra = passing_report()
        extra["reconnect_token"] = "must-never-be-persisted"
        with self.assertRaisesRegex(MODULE.StartGateSystemError, "closed schema"):
            MODULE.validate_report(extra)

        nested = passing_report()
        nested["artifacts"]["server_stderr"] = "private/path/server.log"
        with self.assertRaisesRegex(MODULE.StartGateSystemError, "basenames"):
            MODULE.validate_report(nested)

    def test_rejects_unbound_candidate_and_duplicate_checks(self) -> None:
        report = passing_report()
        report["candidate_binding"] = "probably-this-build"
        with self.assertRaisesRegex(MODULE.StartGateSystemError, "binding"):
            MODULE.validate_report(report)

        duplicate = passing_report()
        duplicate["checks"] = ["same-check"] * 18
        with self.assertRaisesRegex(MODULE.StartGateSystemError, "check inventory"):
            MODULE.validate_report(duplicate)


if __name__ == "__main__":
    unittest.main()
