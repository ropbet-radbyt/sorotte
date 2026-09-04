import pathlib
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import playback_status_system as status_system


class PlaybackStatusSystemTests(unittest.TestCase):
    def native_report(self) -> dict:
        gui_digest = "a" * 64
        mpv_digest = "b" * 64
        return {
            "schema_version": 1,
            "kind": "sorotte-gui-participant-status-system",
            "result": "passed",
            "run_id": "run-1",
            "endpoint": "127.0.0.1:8999",
            "room": "status-room",
            "observer_username": "status-observer",
            "reporter_username": "status-reporter",
            "gui_pid": 42,
            "gui": {"file_name": "sorotte-gui.exe", "sha256": gui_digest},
            "configured_mpv": {"file_name": "mpv.exe", "sha256": mpv_digest},
            "projection": {
                "reporter_username": "status-reporter",
                "user_row_identity": "main-window:user:1",
                "participant_index": 1,
                "username_bounds": [340, 617, 471, 638],
                "status_automation_id": "main-window:user:1:participant-status",
                "status_bounds": [340, 663, 643, 678],
                "status_label": (
                    "Ready · paused · 00:00.0 · Offset unavailable · fresh"
                ),
                "binding_source": "uia-spatial-row+status-index",
                "vertical_gap_px": 25,
                "visible": True,
            },
            "artifacts": {
                "screenshot": "participant-status-system.png",
                "projection": "participant-status-projection.json",
                "gui_lifecycle": "gui-product-lifecycle.jsonl",
            },
            "assertions": sorted(status_system.REQUIRED_NATIVE_ASSERTIONS),
        }

    def validate(self, report: dict) -> dict:
        return status_system.validate_native_report(
            report,
            run_id="run-1",
            reporter_username="status-reporter",
            observer_username="status-observer",
            room="status-room",
            gui_sha256="a" * 64,
            mpv_sha256="b" * 64,
        )

    def test_accepts_closed_username_bound_fresh_projection(self) -> None:
        validated = self.validate(self.native_report())
        self.assertTrue(validated["projection"]["status_label"].endswith(" · fresh"))

    def test_rejects_status_node_from_a_different_user_row(self) -> None:
        report = self.native_report()
        report["projection"]["status_automation_id"] = (
            "main-window:user:2:participant-status"
        )
        with self.assertRaises(status_system.StatusSystemError):
            self.validate(report)

    def test_rejects_waiting_or_non_fresh_status(self) -> None:
        report = self.native_report()
        report["projection"]["status_label"] = "Waiting for first status report"
        with self.assertRaises(status_system.StatusSystemError):
            self.validate(report)

    def test_rejects_digest_drift_and_schema_expansion(self) -> None:
        digest_drift = self.native_report()
        digest_drift["gui"]["sha256"] = "c" * 64
        with self.assertRaises(status_system.StatusSystemError):
            self.validate(digest_drift)

        expanded = self.native_report()
        expanded["raw_server_address"] = "forbidden"
        with self.assertRaises(status_system.StatusSystemError):
            self.validate(expanded)

    def test_generated_fixture_is_deterministic_and_attested(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            first = Path(temporary) / "first.wav"
            second = Path(temporary) / "second.wav"
            first_evidence = status_system.generate_reporter_wav(first, duration_seconds=1)
            second_evidence = status_system.generate_reporter_wav(second, duration_seconds=1)
            self.assertEqual(first_evidence, second_evidence)
            self.assertEqual(first_evidence["duration_seconds"], 1)
            self.assertRegex(first_evidence["sha256"], r"^[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
