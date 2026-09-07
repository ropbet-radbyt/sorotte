from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify


SHA = "a" * 40
OTHER = "b" * 40


class LedgerSourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def index(self, report, expected=SHA):
        path = self.root / "receipt.json"
        original = (json.dumps(report, indent=2) + "\n").encode()
        path.write_bytes(original)
        result = verify.ledger([path], expected)
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(result["entries"][0]["receipt"], report)
        self.assertEqual(result["entries"][0]["sha256"], hashlib.sha256(original).hexdigest())
        self.assertIn("Index only", result["note"])
        return result

    def test_live_interop_fixture_passes_existing_validator_and_index_cli(self):
        from scripts.tests.test_compat_live_interop import valid_report, interop
        report = valid_report()
        interop.validate_report_document(report)
        sha = report["source"]["commit_sha"]
        self.index(report, sha)
        output = self.root / "index.json"
        process = subprocess.run([sys.executable, str(verify.ROOT / "scripts/verify.py"), "ledger",
            "--source-sha", sha, "--receipt", str(self.root / "receipt.json"), "--output", str(output)],
            capture_output=True, text=True, timeout=15)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertTrue(output.with_suffix(".md").is_file())

    def test_existing_behavior_shard_and_actual_aggregate_are_indexable(self):
        from scripts.tests.test_behavior_evidence import BehaviorEvidenceTests, evidence, SHA as behavior_sha, DIGEST
        fixture = BehaviorEvidenceTests()
        fixture.setUp()
        self.addCleanup(fixture.tearDown)
        shard = fixture.shard()
        aggregate = evidence.aggregate_evidence(fixture.normalized(), [shard], digest=DIGEST,
            expected_sha=behavior_sha, expected_repository="owner/repo", expected_run_id="42",
            expected_run_attempt=1, job_results={"checks": "success", "contract": "success"})
        self.assertEqual(aggregate["status"], "passed")
        for report in (shard, aggregate):
            self.index(report, behavior_sha)

    def test_existing_windows_coverage_fixture_is_indexable(self):
        from scripts.tests.test_coverage_windows_process_lanes import WindowsProcessCoverageLaneTests, lanes
        report = WindowsProcessCoverageLaneTests().valid_report()
        lanes.validate_report_document(report)
        self.index(report)

    def test_coverage_aggregate_requires_agreeing_embedded_candidate_subjects(self):
        base = {"schema_version": 1, "kind": "sorotte-coverage-base", "verification_sha": SHA,
                "verification_sha_input": SHA, "resolved_base_sha": OTHER}
        diff = {"schema_version": 1, "kind": "sorotte-diff-coverage", "inputs": {"head_sha": SHA, "base_sha": OTHER}}
        report = {"schema_version": 2, "kind": "sorotte-coverage-ci-evidence", "status": "failed",
                  "phases": {"resolve-base": {"report": base}, "diff-policy": {"report": diff}}}
        for value in (base, diff, report):
            self.index(value)
        report["phases"]["diff-policy"]["report"]["inputs"]["head_sha"] = OTHER
        with self.assertRaisesRegex(ValueError, "conflicting"):
            self.index(report)

    def test_mutation_evidence_template_and_campaign_subjects(self):
        import mutation_ci
        report = mutation_ci.report_template(mode="full", policy_path=Path("coverage/mutation-policy.toml"), shard_id="fixture")
        report["git"] = {"head_sha": SHA, "configured_sources_dirty": False}
        self.index(report)
        selected = {"schema_version": 1, "base": OTHER, "head": SHA, "full": False, "changed": [], "shards": [], "inputs": {}}
        self.index(selected)
        for kind in ("sorotte-mutation-campaign", "sorotte-mutation-required"):
            self.index({"schema_version": 1, "kind": kind, "head": SHA, "selection": selected})
            self.index({"schema_version": 1, "kind": kind, "status": "failed", "head": SHA})
            with self.assertRaisesRegex(ValueError, "conflicting"):
                self.index({"schema_version": 1, "kind": kind, "head": OTHER, "selection": selected})

    def test_existing_package_verifier_outputs_and_failures_are_indexable(self):
        from scripts.tests.test_gui_release_artifact import GuiArtifactBuilder, artifact as gui, SOURCE_SHA
        from scripts.tests.test_server_release_artifact import ArtifactBuilder, artifact as server
        for name, builder_type in (("gui", GuiArtifactBuilder), ("server", ArtifactBuilder)):
            directory = self.root / name
            directory.mkdir()
            builder = builder_type(directory)
            builder.write()
            report = builder.verify()
            self.assertEqual(report["status"], "verified")
            self.index(report, SOURCE_SHA)
        self.index(gui.failure_report(SOURCE_SHA, "dev", ValueError("fixture failure")), SOURCE_SHA)
        self.index(server.failure_report(SOURCE_SHA, ValueError("fixture failure")), SOURCE_SHA)

    def test_existing_lifecycle_bundle_and_system_fixtures_are_indexable(self):
        from scripts.tests.test_playback_release_gate import materialize_bundle, system_report
        bundle = materialize_bundle(self.root, "linux-x86_64")
        self.index(bundle)
        report = system_report(bundle, loop=False)
        self.index(report)
        report["prerequisites"]["candidate_attestation"]["checkout_sha"] = OTHER
        with self.assertRaisesRegex(ValueError, "conflicting"):
            self.index(report)
        bundle["build_inputs"]["candidate_sha"] = OTHER
        with self.assertRaisesRegex(ValueError, "conflicting"):
            self.index(bundle)

    def test_server_stage_subject_does_not_confuse_the_legacy_oracle(self):
        self.index({"status": "FAIL", "stage": "Behavior", "sourceSha": SHA,
                    "steps": [], "legacyOracle": {"sha": OTHER}})

    def test_matching_standard_source_aliases_are_retained(self):
        report = {"source_sha": SHA, "candidate_sha": SHA, "identity": {"source_sha": SHA}, "status": "failed"}
        claims = self.index(report)["entries"][0]["source_claims"]
        self.assertEqual(set(claims), {"source_sha", "candidate_sha", "identity.source_sha"})

    def test_conflicting_standard_and_schema_specific_sources_are_rejected(self):
        for report in (
            {"source_sha": SHA, "candidate_sha": OTHER},
            {"source_sha": SHA, "identity": {"source_sha": OTHER}},
            {"source_sha": SHA, "schema_version": 1, "kind": "sorotte-behavior-evidence-shard", "sha": OTHER},
            {"schema_version": 1, "kind": "sorotte-compat-live-interop", "source": {"commit_sha": SHA, "expected_commit_sha": OTHER}},
            {"source_sha": SHA, "schemaVersion": 1, "status": "verified", "package": {"name": "sorotte-server", "sourceSha": OTHER}},
        ):
            with self.subTest(report=report), self.assertRaisesRegex(ValueError, "conflicting"):
                self.index(report)

    def test_missing_malformed_and_unknown_sha_only_sources_are_rejected(self):
        reports = [{}, [], {"sha": SHA}, {"kind": "unrecognized", "sha": SHA},
                   {"source_sha": SHA, "identity": None},
                   {"schema_version": 1, "kind": "sorotte-compat-live-interop", "source": {"commit_sha": SHA}},
                   {"schema_version": 2, "kind": "sorotte-behavior-evidence-shard", "sha": SHA}]
        reports.extend({"source_sha": value} for value in (None, 7, True, "HEAD", "A" * 40, "a" * 39, "a" * 64, "a" * 40 + "\n"))
        for report in reports:
            with self.subTest(report=report), self.assertRaises(ValueError):
                self.index(report)

    def test_raw_linux_profiles_and_line_maps_cannot_invent_candidate_attribution(self):
        import coverage_profile_lanes
        for report in (coverage_profile_lanes.failed_report(),
                       {"schema_version": 1, "kind": "sorotte-llvm-line-map", "files": [{"source_sha256": "a" * 64}]}):
            with self.assertRaisesRegex(ValueError, "source-bound parent"):
                self.index(report)

    def test_duplicate_json_source_keys_and_wrong_candidate_are_rejected(self):
        path = self.root / "duplicate.json"
        path.write_text('{"source_sha":"' + OTHER + '","source_sha":"' + SHA + '"}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate receipt JSON key"):
            verify.ledger([path], SHA)
        with self.assertRaisesRegex(ValueError, "source mismatch"):
            self.index({"candidate_sha": OTHER})
        with self.assertRaisesRegex(ValueError, "full lowercase"):
            verify.ledger([], "HEAD")

    def test_powershell_utf8_bom_preserves_physical_bytes_and_duplicate_key_rejection(self):
        report = {"status": "FAIL", "stage": "Behavior", "sourceSha": SHA,
                  "steps": [], "legacyOracle": {"sha": OTHER}}
        path = self.root / "powershell-stage.json"
        raw = b"\xef\xbb\xbf" + (json.dumps(report, indent=2).replace("\n", "\r\n") + "\r\n").encode("utf-8")
        path.write_bytes(raw)
        entry = verify.ledger([path], SHA)["entries"][0]
        self.assertEqual(entry["receipt"], report)
        self.assertEqual(entry["sha256"], hashlib.sha256(raw).hexdigest())
        self.assertNotEqual(entry["sha256"], hashlib.sha256(raw[3:]).hexdigest())
        self.assertEqual(path.read_bytes(), raw)
        path.write_bytes(b'\xef\xbb\xbf{"source_sha":"' + OTHER.encode() + b'","source_sha":"' + SHA.encode() + b'"}')
        with self.assertRaisesRegex(ValueError, "duplicate receipt JSON key"):
            verify.ledger([path], SHA)


if __name__ == "__main__":
    unittest.main()
