"""Exercise malformed artifacts through the same CLIs that attest CI evidence."""

from __future__ import annotations

import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import behavior_evidence
import coverage_ci_guard
import coverage_profile_lanes
import diff_coverage
import llvm_cov_line_map
import mutation_ci
import playback_lifecycle_evidence
import playback_lifecycle_oracle
import playback_release_gate
from scripts.tests import test_behavior_evidence as behavior_fixtures
from scripts.tests import test_coverage_ci_guard as guard_fixtures
from scripts.tests import test_coverage_profile_lanes as profile_fixtures
from scripts.tests import test_coverage_windows_process_lanes as windows_profile_fixtures
from scripts.tests import test_diff_coverage_map as map_fixtures
from scripts.tests import test_mutation_ci as mutation_fixtures
from scripts.tests import test_llvm_cov_line_map as llvm_fixtures
from scripts.tests import test_playback_release_gate as release_fixtures
from scripts.tests import test_playback_lifecycle_oracle as oracle_fixtures
from scripts.tests import test_gui_release_artifact as gui_fixtures
from scripts.tests import test_server_release_artifact as server_fixtures
from scripts.tests.artifact_malformed import malformed_json_cases, oversized_file

REPO = pathlib.Path(__file__).resolve().parents[2]
SCRIPTS = REPO / "scripts"


class ArtifactEntrypointTests(unittest.TestCase):
    def fixture(self, cls):
        value = cls()
        value.setUp()
        self.addCleanup(value.tearDown)
        return value

    def run_cli(self, script: str, args: list[str]):
        return subprocess.run(
            [sys.executable, str(SCRIPTS / script), *map(str, args)],
            cwd=REPO, capture_output=True, text=True, encoding="utf-8", timeout=30,
        )

    def assert_matrix(self, script: str, args: list[str], path: pathlib.Path, valid: bytes, *, max_bytes: int, expect_healthy: bool = True, integer_field: str = "schema_version"):
        if expect_healthy:
            path.write_bytes(valid)
            result = self.run_cli(script, args)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name, raw, category in malformed_json_cases(valid, integer_field=integer_field):
            with self.subTest(entrypoint=script, mutation=name):
                path.write_bytes(raw)
                result = self.run_cli(script, args)
                self.assertNotEqual(result.returncode, 0)
                diagnostic = result.stdout + result.stderr
                self.assertRegex(diagnostic, category)
                self.assertNotIn("Traceback", diagnostic)
        with self.subTest(entrypoint=script, mutation="oversized"):
            oversized_file(path, max_bytes=max_bytes)
            result = self.run_cli(script, args)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("byte_limit", result.stdout + result.stderr)
        path.write_bytes(valid)

    def test_release_bundle_cli_rejects_each_malformed_artifact_before_attestation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            manifest = release_fixtures.materialize_bundle(root, "linux-x86_64")
            args = ["verify-bundle", "--bundle-dir", root, "--candidate-sha", release_fixtures.SHA, "--platform", "linux-x86_64"]
            self.assert_matrix("playback_release_gate.py", args, root / "candidate-manifest.json", json.dumps(manifest).encode(), max_bytes=playback_release_gate.MAX_REPORT_BYTES)
            # Reproduce the audit's failed-then-passed duplicate status exactly.
            raw = json.dumps(manifest).replace('"result": "passed"', '"result":"failed","result":"passed"').encode()
            (root / "candidate-manifest.json").write_bytes(raw)
            result = self.run_cli("playback_release_gate.py", args)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("duplicate_key", result.stderr)

    def test_lifecycle_cli_rejects_each_malformed_record_and_record_budgets(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "events.jsonl"
            writer = playback_lifecycle_evidence.EvidenceWriter(
                path, run_id="run-001", emitter="client-a", binary_role="client", component_roles=("client",), product_version="0.2.8", product_digest="a" * 64,
            )
            writer.close()
            args = ["--model", REPO / "coverage/playback-lifecycle.toml", "--input", path]
            valid = path.read_bytes()
            self.assert_matrix("playback_lifecycle_evidence.py", args, path, valid, max_bytes=playback_lifecycle_evidence.MAX_EVIDENCE_BYTES)
            for body, category in [
                (b" " * playback_lifecycle_evidence.MAX_EVIDENCE_RECORD_BYTES + b"\n", "record_bytes"),
                (b"\n" * playback_lifecycle_evidence.MAX_EVIDENCE_RECORDS + valid, "record_limit"),
            ]:
                path.write_bytes(body)
                result = self.run_cli("playback_lifecycle_evidence.py", args)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(category, result.stdout)

    def test_independent_causal_oracle_cli_rejects_matrix_without_changing_its_model(self) -> None:
        fixture = self.fixture(oracle_fixtures.PlaybackLifecycleOracleTests)
        event = fixture.apply("APP-LAUNCH-001", subject="client-a", identities={"process_run": 1}, trigger="startup", role="client")
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "ledger.jsonl"
            self.assert_matrix("playback_lifecycle_oracle.py", ["verify-ledger", "--ledger", path], path, json.dumps(event).encode() + b"\n", max_bytes=playback_lifecycle_oracle.MAX_LEDGER_BYTES)

    def test_diff_coverage_cli_retains_its_source_oracle_and_rejects_matrix(self) -> None:
        fixture = self.fixture(map_fixtures.DiffCoverageMapTests)
        args = ["--repo-root", fixture.repo, "--coverage-map", fixture.map_path, "--diff", fixture.diff_path, "--minimum", "50", "--json-out", fixture.repo / "result.json"]
        self.assert_matrix("diff_coverage.py", args, fixture.map_path, json.dumps(fixture.map_document()).encode(), max_bytes=diff_coverage.MAX_COVERAGE_MAP_BYTES)

    def test_llvm_line_map_cli_rejects_matrix_before_emitting_source_evidence(self) -> None:
        fixture = self.fixture(llvm_fixtures.LlvmCovLineMapTests)
        fixture.write_artifacts()
        args = ["--repo-root", fixture.repo, "--llvm-json", fixture.json_path, "--llvm-text", fixture.text_path, "--output", fixture.output_path]
        self.assert_matrix("llvm_cov_line_map.py", args, fixture.json_path, fixture.json_path.read_bytes(), max_bytes=llvm_cov_line_map.MAX_LLVM_JSON_BYTES, integer_field="count")

    def test_profile_report_cli_rejects_matrix_with_its_full_valid_schema(self) -> None:
        fixture = self.fixture(profile_fixtures.CoverageProfileLaneTests)
        path = fixture.root / "report.json"
        self.assert_matrix("coverage_profile_lanes.py", ["validate", "--report", path], path, json.dumps(fixture.valid_report()).encode(), max_bytes=coverage_profile_lanes.MAX_REPORT_BYTES)

    def test_windows_profile_cli_rejects_matrix_with_its_full_valid_schema(self) -> None:
        fixture = self.fixture(windows_profile_fixtures.WindowsProcessCoverageLaneTests)
        path = fixture.root / "report.json"
        self.assert_matrix("coverage_windows_process_lanes.py", ["validate", "--report", path], path, json.dumps(fixture.valid_report()).encode(), max_bytes=coverage_profile_lanes.MAX_REPORT_BYTES)

    def test_coverage_finalizer_cli_rejects_malformed_policy_before_pass(self) -> None:
        fixture = self.fixture(guard_fixtures.CoverageFinalizerTests)
        args = ["finalize"]
        for phase in ("base", "profiles", "llvm-json", "llvm-text", "line-map", "policy"):
            args += [f"--{phase}-outcome", "success"]
        for flag, path in [("base-report", fixture.base), ("llvm-json", fixture.llvm_json), ("llvm-text", fixture.llvm_text), ("line-map", fixture.line_map), ("policy-report", fixture.policy), ("profile-lanes", fixture.profile_lanes), ("output", fixture.output)]:
            args += [f"--{flag}", path]
        self.assert_matrix("coverage_ci_guard.py", args, fixture.policy, fixture.policy.read_bytes(), max_bytes=coverage_ci_guard.MAX_JSON_BYTES)

    def test_mutation_cli_rejects_matrix_before_source_and_proof_attestation(self) -> None:
        fixture = self.fixture(mutation_fixtures.MutationRunnerTests)
        path = fixture.repo / "target" / "report.json"
        path.parent.mkdir()
        valid = json.dumps({"schema_version": mutation_ci.SCHEMA_VERSION, "kind": mutation_ci.REPORT_KIND, "status": "passed", "shard": "demo"}).encode()
        # Full healthy mutation attestations require producer execution; their
        # existing runner tests exercise that path and remain independent here.
        self.assert_matrix("mutation_ci.py", ["verify-report", "--repo-root", fixture.repo, "--policy", "coverage/mutation-policy.toml", "--shard", "demo", "--report", path], path, valid, max_bytes=mutation_ci.MAX_JSON_BYTES, expect_healthy=False)

    def test_behavior_aggregate_cli_rejects_matrix_after_exact_clean_source_check(self) -> None:
        fixture = self.fixture(behavior_fixtures.BehaviorEvidenceTests)
        catalog = fixture.repo / "catalog.toml"
        catalog.write_text('''schema_version = 1
[policy]
allowed_namespaces = ["PL"]
allowed_risks = ["critical", "high"]
critical_minimum_proofs = 2
required_jobs = ["checks", "contract"]
[lanes.contract]
runner = "rust-exact"
required_on = ["pull_request"]
operating_systems = ["linux"]
timeout_seconds = 60
[[behavior]]
id = "PL-TEST-001"
title = "Exact contract remains true"
risk = "high"
owners = ["player"]
invariants = ["the exact assertion passes"]
[[behavior.proof]]
id = "PL-TEST-001.exact"
kind = "rust-test"
oracle = "state-invariant"
package = "example"
target_kind = "lib"
test = "tests::exact_contract"
source = "src/lib.rs"
feature_mode = "all-features"
operating_systems = ["linux"]
required_lanes = ["contract"]
''', encoding="utf-8")
        # Commit only a disposable synthetic test repository to exercise the
        # real clean-HEAD check; the project's checkout is never modified.
        def git(*args):
            return subprocess.run(["git", "-C", str(fixture.repo), *args], check=True, capture_output=True, text=True).stdout.strip()
        git("init", "-b", "main")
        git("config", "user.name", "Artifact Test")
        git("config", "user.email", "artifact@example.invalid")
        git("add", ".")
        git("commit", "-m", "fixture")
        sha = git("rev-parse", "HEAD")
        shard = fixture.shard()
        shard["sha"] = sha
        shard["catalog_sha256"] = "sha256:" + hashlib.sha256(catalog.read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "shard.json"
            output = pathlib.Path(temporary) / "aggregate.json"
            args = ["aggregate", "--repo-root", fixture.repo, "--catalog", catalog, "--expected-sha", sha, "--expected-repository", "owner/repo", "--expected-run-id", "42", "--expected-run-attempt", "1", "--job-result", "checks=success", "--job-result", "contract=success", "--input", path, "--output", output]
            self.assert_matrix("behavior_evidence.py", args, path, json.dumps(shard).encode(), max_bytes=behavior_evidence.MAX_EVIDENCE_BYTES)

    def test_package_verifier_clis_reject_manifest_matrix_after_archive_hash_verification(self) -> None:
        for gui in (False, True):
            fixture = gui_fixtures if gui else server_fixtures
            with tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                builder = fixture.GuiArtifactBuilder(root) if gui else fixture.ArtifactBuilder(root)
                builder.write()
                script = "verify_gui_release_artifact.py" if gui else "verify_server_release_artifact.py"
                args = ["--artifacts-dir", builder.artifacts_dir, "--expected-source-sha", fixture.SOURCE_SHA, "--report", root / "result.json", "--skip-runtime-smoke"]
                if gui:
                    args += ["--expected-channel", "dev"]
                    path = builder.artifacts_dir / "sorotte-update-manifest.json"
                    valid = path.read_bytes()
                else:
                    valid = json.dumps(builder.manifest()).encode()
                healthy = self.run_cli(script, args)
                self.assertEqual(healthy.returncode, 0, healthy.stderr)
                cases = list(malformed_json_cases(valid, integer_field="schema" if gui else "schemaVersion"))
                cases.append(("oversized", b" " * (1024 * 1024 + 1), "byte_limit"))
                for name, raw, category in cases:
                    with self.subTest(entrypoint=script, mutation=name):
                        if gui:
                            path.write_bytes(raw)
                        else:
                            # Rebuild the outer archive and checksum so every
                            # mutation reaches the authenticated manifest loader.
                            builder.write(manifest=raw)
                        result = self.run_cli(script, args)
                        self.assertNotEqual(result.returncode, 0)
                        self.assertRegex(result.stdout + result.stderr, category)
                        report = json.loads((root / "result.json").read_text(encoding="utf-8"))
                        self.assertEqual(report["status"], "failed")


if __name__ == "__main__":
    unittest.main()
