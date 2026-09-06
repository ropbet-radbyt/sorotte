from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from scripts import native_failure_evidence as evidence


class NativeFailureEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "private"
        self.root.mkdir()
        self.output = Path(self.temp.name) / "safe"

    def export(self, **kwargs):
        return evidence.export(self.root, self.output, source_sha="a" * 40,
                               run_id="42", run_attempt=2, stage="native",
                               cleanup="unavailable", mode_outcomes={"recovery": "failure"},
                               **kwargs)

    def test_before_checkout_failure_is_explicitly_unavailable(self):
        self.root.rmdir()
        result = self.export()
        self.assertFalse(result["authoritative"])
        self.assertEqual(result["records"][0]["reason"], "no-structured-evidence-produced")
        self.assertEqual(result["run_attempt"], 2)

    def test_each_stage_preserves_attribution_without_claiming_qualification(self):
        for stage in ("preflight", "checkout", "tool-setup", "vertical", "recovery", "http-fault", "http-stall", "validation", "cancelled"):
            with self.subTest(stage=stage):
                result = evidence.export(self.root, self.output / stage, source_sha="b" * 40,
                                         run_id="123", run_attempt=3, stage=stage,
                                         cleanup="failed", mode_outcomes={stage: "failure"})
                self.assertEqual(result["source_sha"], "b" * 40)
                self.assertEqual(result["stage"], stage)
                self.assertEqual(result["cleanup"], "failed")

    def test_canary_credentials_and_private_paths_are_not_exported(self):
        value = {"event": "process-gone", "pid": 42, "ipc_endpoint": r"\\.\pipe\secret-pipe",
                 "error": r"failed C:\Users\private-name\media.wav token=some-secret-value",
                 "token": "unknown-format-token", "nested": {"authorization": "Basic unknown"},
                 "source": "https://example.invalid/video?token=private-token",
                 r"C:\Users\private-name\config.ini": "canarycredential",
                 "path": "/home/private-name/media.wav"}
        (self.root / "harness-report.json").write_text(json.dumps(value))
        (self.root / "registration.log").write_text("secret raw registration log")
        (self.root / "window.png").write_bytes(b"private screenshot")
        self.export(secrets=("canarycredential",))
        text = "".join(p.read_text() for p in self.output.iterdir())
        for private in ("private-name", "secret-pipe", "some-secret-value", "unknown-format-token",
                        "Basic unknown", "private-token", "canarycredential", "secret raw", "private screenshot"):
            self.assertNotIn(private, text)
        self.assertIn('"pid": 42', text)
        self.assertIn("process-gone", text)

    def test_malformed_trace_is_not_repaired_into_valid_evidence(self):
        (self.root / "mpv-observations.jsonl").write_text('{"event":"file-loaded"}\n{broken}\n')
        result = self.export()
        self.assertEqual(result["records"][0]["status"], "unavailable")
        self.assertEqual(list(self.output.glob("record-*.json")), [])

    def test_private_endpoint_identity_remains_comparable_across_records(self):
        (self.root / "mpv-observation.jsonl").write_text("\n".join(json.dumps({"pid": 42, "ipc_endpoint": endpoint}) for endpoint in ("secret-pipe-one", "secret-pipe-two", "secret-pipe-one")))
        self.export()
        records = json.loads((self.output / "record-000.json").read_text())
        self.assertEqual(records[0]["ipc_endpoint"], records[2]["ipc_endpoint"])
        self.assertNotEqual(records[0]["ipc_endpoint"], records[1]["ipc_endpoint"])

    def test_paths_with_spaces_and_urls_inside_unknown_fields_are_withheld(self):
        (self.root / "harness-report.json").write_text(json.dumps({"error": r"failed C:\Users\Private User\My Movie.mp4 --token secret", "detail": "https://private.invalid/media?token=canary"}))
        self.export()
        data = (self.output / "record-000.json").read_text()
        for text in ("Private User", "My Movie", "secret", "private.invalid", "canary"):
            self.assertNotIn(text, data)

    def test_last_causal_records_are_bounded_and_hash_bound(self):
        import hashlib
        (self.root / "mpv-observations.jsonl").write_text("".join(json.dumps({"sequence": i}) + "\n" for i in range(300)))
        result = self.export()
        row = result["records"][0]
        data = (self.output / row["file"]).read_bytes()
        self.assertEqual(row["record_count"], 300)
        self.assertEqual(row["sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(json.loads(data)[-1]["sequence"], 299)
        self.assertEqual(len(json.loads(data)), 256)

    def test_previous_attempt_is_never_overwritten(self):
        self.export()
        original = (self.output / "diagnostic.json").read_bytes()
        with self.assertRaises(FileExistsError):
            self.export()
        self.assertEqual((self.output / "diagnostic.json").read_bytes(), original)

    def test_output_and_input_links_are_not_followed(self):
        link = Path(self.temp.name) / "link"
        try:
            link.symlink_to(self.root, target_is_directory=True)
        except OSError:
            self.skipTest("symlink privilege unavailable")
        with self.assertRaises(ValueError):
            evidence.export(self.root, link / "output", source_sha="a" * 40,
                            run_id="1", run_attempt=1, stage="native", cleanup="pending", mode_outcomes={})
        with self.assertRaises(ValueError):
            evidence.discover(link)


if __name__ == "__main__":
    unittest.main()
