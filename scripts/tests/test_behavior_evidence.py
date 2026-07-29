from __future__ import annotations

import argparse
import contextlib
import copy
import io
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import behavior_evidence as evidence  # noqa: E402


SHA = "0123456789abcdef0123456789abcdef01234567"
DIGEST = f"sha256:{'a' * 64}"


class BehaviorEvidenceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temporary.name)
        source = self.repo / "src" / "lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text(
            "#[test]\nfn exact_contract() {\n    assert!(true);\n}\n",
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def base_catalog(self) -> dict:
        return {
            "schema_version": 1,
            "policy": {
                "allowed_namespaces": ["PL"],
                "allowed_risks": ["critical", "high"],
                "critical_minimum_proofs": 2,
                "required_jobs": ["checks", "contract"],
            },
            "lanes": {
                "contract": {
                    "runner": "rust-exact",
                    "required_on": ["pull_request"],
                    "operating_systems": ["linux"],
                    "timeout_seconds": 60,
                }
            },
            "behavior": [
                {
                    "id": "PL-TEST-001",
                    "title": "Exact contract remains true",
                    "risk": "high",
                    "owners": ["player"],
                    "invariants": ["the exact assertion passes"],
                    "proof": [
                        {
                            "id": "PL-TEST-001.exact",
                            "kind": "rust-test",
                            "oracle": "state-invariant",
                            "package": "example",
                            "target_kind": "lib",
                            "test": "tests::exact_contract",
                            "source": "src/lib.rs",
                            "feature_mode": "all-features",
                            "operating_systems": ["linux"],
                            "required_lanes": ["contract"],
                        }
                    ],
                }
            ],
        }

    def normalized(self) -> dict:
        return evidence.validate_catalog(self.base_catalog(), repo_root=self.repo)

    def shard(self, *, status: str = "passed", proof_status: str = "passed") -> dict:
        proof = self.normalized()["behavior"][0]["proof"][0]
        return {
            "schema_version": 1,
            "kind": evidence.EVIDENCE_KIND,
            "catalog_sha256": DIGEST,
            "repository": "owner/repo",
            "sha": SHA,
            "run_id": "42",
            "run_attempt": 1,
            "lane": "contract",
            "operating_system": "linux",
            "status": status,
            "started_at": "2026-07-28T00:00:00Z",
            "finished_at": "2026-07-28T00:00:01Z",
            "tool_versions": {},
            "proofs": [
                {
                    "behavior_id": "PL-TEST-001",
                    "proof_id": "PL-TEST-001.exact",
                    "kind": "rust-test",
                    "selector": "tests::exact_contract",
                    "status": proof_status,
                    "duration_ms": 1,
                    "return_code": 0 if proof_status == "passed" else 1,
                    "observed": {
                        "passed": 1 if proof_status == "passed" else 0,
                        "failed": 0 if proof_status == "passed" else 1,
                        "ignored": 1 if proof_status == "ignored" else 0,
                        "discovered_exactly": 1,
                    },
                    "discovery_command": list(
                        evidence.rust_discovery_argv(proof["package"])
                    ),
                    "command": list(evidence.rust_execution_argv(proof)),
                }
            ],
            "inventory_cases": [],
            "errors": [],
        }

    def aggregate(
        self,
        shards: list[dict],
        jobs: dict[str, str] | None = None,
        *,
        expected_attempt: int = 1,
    ) -> dict:
        return evidence.aggregate_evidence(
            self.normalized(),
            shards,
            digest=DIGEST,
            expected_sha=SHA,
            expected_repository="owner/repo",
            expected_run_id="42",
            expected_run_attempt=expected_attempt,
            job_results=(
                jobs
                if jobs is not None
                else {"checks": "success", "contract": "success"}
            ),
        )

    def test_valid_catalog_is_accepted(self) -> None:
        catalog = self.normalized()
        self.assertEqual(catalog["behavior"][0]["id"], "PL-TEST-001")

    def test_async_rust_test_source_is_accepted(self) -> None:
        (self.repo / "src" / "lib.rs").write_text(
            "#[tokio::test]\nasync fn exact_contract() {\n    assert!(true);\n}\n",
            encoding="utf-8",
        )

        catalog = self.normalized()

        self.assertEqual(
            catalog["behavior"][0]["proof"][0]["test"],
            "tests::exact_contract",
        )

    def test_unknown_catalog_key_is_rejected(self) -> None:
        catalog = self.base_catalog()
        catalog["surprise"] = True
        with self.assertRaisesRegex(evidence.CatalogError, "unknown keys"):
            evidence.validate_catalog(catalog, repo_root=self.repo)

    def test_boolean_catalog_integers_are_rejected(self) -> None:
        cases = (
            (
                "schema",
                lambda catalog: catalog.__setitem__("schema_version", True),
                "schema",
            ),
            (
                "critical minimum",
                lambda catalog: catalog["policy"].__setitem__(
                    "critical_minimum_proofs", True
                ),
                "positive integer",
            ),
            (
                "lane timeout",
                lambda catalog: catalog["lanes"]["contract"].__setitem__(
                    "timeout_seconds", True
                ),
                "1..3600",
            ),
        )
        for name, mutate, message in cases:
            with self.subTest(name=name):
                catalog = self.base_catalog()
                mutate(catalog)
                with self.assertRaisesRegex(evidence.CatalogError, message):
                    evidence.validate_catalog(catalog, repo_root=self.repo)

    def test_duplicate_behavior_id_is_rejected(self) -> None:
        catalog = self.base_catalog()
        catalog["behavior"].append(copy.deepcopy(catalog["behavior"][0]))
        with self.assertRaisesRegex(evidence.CatalogError, "duplicate behavior"):
            evidence.validate_catalog(catalog, repo_root=self.repo)

    def test_duplicate_proof_id_is_rejected(self) -> None:
        catalog = self.base_catalog()
        catalog["behavior"][0]["proof"].append(
            copy.deepcopy(catalog["behavior"][0]["proof"][0])
        )
        with self.assertRaisesRegex(evidence.CatalogError, "duplicate proof"):
            evidence.validate_catalog(catalog, repo_root=self.repo)

    def test_critical_behavior_needs_policy_minimum(self) -> None:
        catalog = self.base_catalog()
        catalog["behavior"][0]["risk"] = "critical"
        with self.assertRaisesRegex(evidence.CatalogError, "at least 2 proofs"):
            evidence.validate_catalog(catalog, repo_root=self.repo)

    def test_wildcard_rust_selector_is_rejected(self) -> None:
        catalog = self.base_catalog()
        catalog["behavior"][0]["proof"][0]["test"] = "tests::*"
        with self.assertRaisesRegex(evidence.CatalogError, "exact Rust test selector"):
            evidence.validate_catalog(catalog, repo_root=self.repo)

    def test_ignored_rust_test_is_rejected(self) -> None:
        (self.repo / "src" / "lib.rs").write_text(
            "#[test]\n#[ignore]\nfn exact_contract() {}\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(evidence.CatalogError, "ignored test"):
            self.normalized()

    def test_proof_operating_system_must_fit_lane(self) -> None:
        catalog = self.base_catalog()
        catalog["behavior"][0]["proof"][0]["operating_systems"] = ["windows"]
        with self.assertRaisesRegex(evidence.CatalogError, "exceed lane"):
            evidence.validate_catalog(catalog, repo_root=self.repo)

    def test_exact_libtest_discovery_is_countable(self) -> None:
        names = evidence.discover_libtests(
            "tests::first: test\ntests::exact_contract: test\n"
        )
        self.assertEqual(names.count("tests::exact_contract"), 1)
        self.assertEqual(names.count("tests::missing"), 0)

    def execution(self, line: str, summary: str, return_code: int = 0):
        return evidence.ProcessResult(
            ("cargo", "test"),
            return_code,
            f"running 1 test\n{line}\n{summary}\n",
            "",
            1,
        )

    def test_exact_one_pass_output_succeeds(self) -> None:
        result = self.execution(
            "test tests::exact_contract ... ok",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out",
        )
        status, counts = evidence.parse_libtest_execution(
            result, "tests::exact_contract"
        )
        self.assertEqual(status, "passed")
        self.assertEqual(counts, {"passed": 1, "failed": 0, "ignored": 0})

    def test_ignored_exact_test_is_not_proof(self) -> None:
        result = self.execution(
            "test tests::exact_contract ... ignored",
            "test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 1 filtered out",
        )
        self.assertEqual(
            evidence.parse_libtest_execution(result, "tests::exact_contract")[0],
            "ignored",
        )

    def test_zero_pass_or_contradictory_exit_is_not_proof(self) -> None:
        zero = self.execution(
            "",
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out",
        )
        contradictory = self.execution(
            "test tests::exact_contract ... ok",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out",
            return_code=1,
        )
        self.assertEqual(
            evidence.parse_libtest_execution(zero, "tests::exact_contract")[0],
            "failed",
        )
        self.assertEqual(
            evidence.parse_libtest_execution(
                contradictory, "tests::exact_contract"
            )[0],
            "failed",
        )

    def test_semantic_inventory_requires_unique_exact_names(self) -> None:
        self.assertEqual(
            evidence.parse_semantic_inventory(
                {"result": "ok", "scenarios": ["one", "two"]}
            ),
            ["one", "two"],
        )
        with self.assertRaisesRegex(evidence.EvidenceError, "duplicate"):
            evidence.parse_semantic_inventory(
                {"result": "ok", "scenarios": ["one", "one"]}
            )

    def test_semantic_summary_recomputes_counts(self) -> None:
        report = {
            "result": "ok",
            "total": 1,
            "passed": 1,
            "failed": 0,
            "reports": [{"result": "ok", "scenario": "one"}],
            "errors": [],
        }
        self.assertEqual(evidence.parse_semantic_summary(report), (["one"], []))
        report["passed"] = 2
        with self.assertRaisesRegex(evidence.EvidenceError, "counts"):
            evidence.parse_semantic_summary(report)

    def test_semantic_summary_rejects_boolean_counts(self) -> None:
        report = {
            "result": "ok",
            "total": True,
            "passed": True,
            "failed": False,
            "reports": [{"result": "ok", "scenario": "one"}],
            "errors": [],
        }
        with self.assertRaisesRegex(evidence.EvidenceError, "counts"):
            evidence.parse_semantic_summary(report)

    def test_valid_aggregate_passes(self) -> None:
        aggregate = self.aggregate([self.shard()])
        self.assertEqual(aggregate["status"], "passed")
        self.assertEqual(aggregate["behaviors"][0]["status"], "passed")

    def test_aggregate_rejects_missing_duplicate_and_stale_shards(self) -> None:
        self.assertEqual(self.aggregate([])["status"], "failed")
        self.assertEqual(
            self.aggregate([self.shard(), self.shard()])["status"], "failed"
        )
        stale = self.shard()
        stale["sha"] = "f" * 40
        self.assertEqual(self.aggregate([stale])["status"], "failed")

    def test_aggregate_rejects_catalog_mismatch_and_ignored_proof(self) -> None:
        mismatch = self.shard()
        mismatch["catalog_sha256"] = f"sha256:{'b' * 64}"
        self.assertEqual(self.aggregate([mismatch])["status"], "failed")
        self.assertEqual(
            self.aggregate([self.shard(proof_status="ignored")])["status"],
            "failed",
        )

    def test_aggregate_binds_behavior_selector_command_os_and_lane(self) -> None:
        forged = self.shard()
        forged["operating_system"] = "windows"
        forged["proofs"][0]["behavior_id"] = "PL-FORGED-999"
        forged["proofs"][0]["selector"] = "tests::some_other_test"
        forged["proofs"][0]["command"][-3] = "tests::some_other_test"
        aggregate = self.aggregate([forged])
        self.assertEqual(aggregate["status"], "failed")
        self.assertTrue(any("operating system" in error for error in aggregate["errors"]))
        self.assertTrue(any("behavior_id" in error for error in aggregate["errors"]))
        self.assertTrue(any("selector" in error for error in aggregate["errors"]))
        self.assertTrue(any("execution command" in error for error in aggregate["errors"]))

    def test_aggregate_binds_proof_to_its_declared_operating_systems(self) -> None:
        catalog = self.base_catalog()
        catalog["lanes"]["contract"]["operating_systems"] = ["linux", "windows"]
        catalog["behavior"][0]["proof"][0]["operating_systems"] = ["windows"]
        normalized = evidence.validate_catalog(catalog, repo_root=self.repo)
        aggregate = evidence.aggregate_evidence(
            normalized,
            [self.shard()],
            digest=DIGEST,
            expected_sha=SHA,
            expected_repository="owner/repo",
            expected_run_id="42",
            expected_run_attempt=1,
            job_results={"checks": "success", "contract": "success"},
        )
        self.assertEqual(aggregate["status"], "failed")
        self.assertTrue(
            any(
                "does not support evidence operating system" in error
                for error in aggregate["errors"]
            )
        )

    def test_aggregate_rejects_boolean_evidence_integers(self) -> None:
        forged = self.shard()
        forged["schema_version"] = True
        forged["run_attempt"] = True
        proof = forged["proofs"][0]
        proof["duration_ms"] = False
        proof["return_code"] = False
        proof["observed"] = {
            "passed": True,
            "failed": False,
            "ignored": False,
            "discovered_exactly": True,
        }
        aggregate = self.aggregate([forged])
        self.assertEqual(aggregate["status"], "failed")
        self.assertTrue(any("schema_version" in error for error in aggregate["errors"]))
        self.assertTrue(any("run_attempt" in error for error in aggregate["errors"]))
        self.assertTrue(any("duration_ms" in error for error in aggregate["errors"]))
        self.assertTrue(any("return_code" in error for error in aggregate["errors"]))
        self.assertTrue(any("observations" in error for error in aggregate["errors"]))

    def test_aggregate_accepts_prior_attempt_but_rejects_invalid_attempts(self) -> None:
        prior = self.shard()
        prior["run_attempt"] = 1
        self.assertEqual(
            self.aggregate([prior], expected_attempt=2)["status"],
            "passed",
        )
        for attempt in (True, 0, -1, 3):
            with self.subTest(attempt=attempt):
                shard = self.shard()
                shard["run_attempt"] = attempt
                aggregate = self.aggregate([shard], expected_attempt=2)
                self.assertEqual(aggregate["status"], "failed")
                self.assertTrue(
                    any("run_attempt" in error for error in aggregate["errors"])
                )

    def test_malformed_lane_fails_closed_and_cli_writes_aggregate(self) -> None:
        malformed = self.shard()
        malformed["lane"] = []
        aggregate = self.aggregate([malformed])
        self.assertEqual(aggregate["status"], "failed")
        self.assertTrue(any("unexpected evidence lane" in error for error in aggregate["errors"]))

        catalog_path = self.repo / "behaviors.toml"
        catalog_path.write_text("# supplied by test double\n", encoding="utf-8")
        input_path = self.repo / "malformed.json"
        input_path.write_text(json.dumps(malformed), encoding="utf-8")
        output_path = self.repo / "aggregate.json"
        args = argparse.Namespace(
            repo_root=str(self.repo),
            catalog=str(catalog_path),
            output=str(output_path),
            expected_sha=SHA,
            expected_repository="owner/repo",
            expected_run_id="42",
            expected_run_attempt=1,
            job_result=["checks=success", "contract=success"],
            input=[str(input_path)],
        )
        with (
            contextlib.redirect_stdout(io.StringIO()),
            mock.patch.object(evidence, "verify_git_head"),
            mock.patch.object(evidence, "verify_clean_worktree"),
            mock.patch.object(evidence, "load_catalog", return_value=self.base_catalog()),
            mock.patch.object(evidence, "catalog_digest", return_value=DIGEST),
        ):
            self.assertEqual(evidence.aggregate_command(args), 1)
        self.assertTrue(output_path.is_file())
        rendered = json.loads(output_path.read_text(encoding="utf-8"))
        self.assertEqual(rendered["status"], "failed")
        self.assertTrue(
            any("unexpected evidence lane" in error for error in rendered["errors"])
        )

    def test_unhashable_proof_status_fails_closed(self) -> None:
        for status in ([], {}):
            with self.subTest(status=status):
                malformed = self.shard()
                malformed["proofs"][0]["status"] = status
                aggregate = self.aggregate([malformed])
                self.assertEqual(aggregate["status"], "failed")
                self.assertTrue(
                    any("invalid status" in error for error in aggregate["errors"])
                )

    def test_aggregate_cli_writes_error_artifact_for_unexpected_exception(self) -> None:
        catalog_path = self.repo / "behaviors.toml"
        catalog_path.write_text("# supplied by test double\n", encoding="utf-8")
        input_path = self.repo / "evidence.json"
        input_path.write_text(json.dumps(self.shard()), encoding="utf-8")
        output_path = self.repo / "aggregate-error.json"
        args = argparse.Namespace(
            repo_root=str(self.repo),
            catalog=str(catalog_path),
            output=str(output_path),
            expected_sha=SHA,
            expected_repository="owner/repo",
            expected_run_id="42",
            expected_run_attempt=1,
            job_result=["checks=success", "contract=success"],
            input=[str(input_path)],
        )
        with (
            contextlib.redirect_stdout(io.StringIO()),
            mock.patch.object(evidence, "verify_git_head"),
            mock.patch.object(evidence, "verify_clean_worktree"),
            mock.patch.object(evidence, "load_catalog", return_value=self.base_catalog()),
            mock.patch.object(evidence, "catalog_digest", return_value=DIGEST),
            mock.patch.object(
                evidence,
                "aggregate_evidence",
                side_effect=TypeError("malformed aggregate input"),
            ),
        ):
            self.assertEqual(evidence.aggregate_command(args), 1)
        rendered = json.loads(output_path.read_text(encoding="utf-8"))
        self.assertEqual(rendered["status"], "error")
        self.assertEqual(rendered["errors"], ["malformed aggregate input"])

    def test_aggregate_rejects_missing_required_job_result(self) -> None:
        aggregate = self.aggregate([self.shard()], jobs={"checks": "success"})
        self.assertEqual(aggregate["status"], "failed")
        self.assertTrue(any("missing required job" in error for error in aggregate["errors"]))

    def test_aggregate_rejects_minimal_forged_proof_schema(self) -> None:
        forged = self.shard()
        forged["proofs"][0] = {
            "proof_id": "PL-TEST-001.exact",
            "status": "passed",
        }
        aggregate = self.aggregate([forged])
        self.assertEqual(aggregate["status"], "failed")
        self.assertTrue(any("fields differ" in error for error in aggregate["errors"]))

    def test_aggregate_rejects_proofs_swapped_between_lanes(self) -> None:
        (self.repo / "src" / "lib.rs").write_text(
            "#[test]\nfn exact_contract() {}\n"
            "#[test]\nfn second_contract() {}\n",
            encoding="utf-8",
        )
        catalog = self.base_catalog()
        catalog["lanes"]["other"] = copy.deepcopy(catalog["lanes"]["contract"])
        catalog["policy"]["required_jobs"].append("other")
        second_behavior = copy.deepcopy(catalog["behavior"][0])
        second_behavior["id"] = "PL-TEST-002"
        second_behavior["title"] = "Second exact contract remains true"
        second_proof = second_behavior["proof"][0]
        second_proof["id"] = "PL-TEST-002.exact"
        second_proof["test"] = "tests::second_contract"
        second_proof["required_lanes"] = ["other"]
        catalog["behavior"].append(second_behavior)
        normalized = evidence.validate_catalog(catalog, repo_root=self.repo)

        contract = self.shard()
        other = copy.deepcopy(contract)
        other["lane"] = "other"
        other_proof = other["proofs"][0]
        other_proof["behavior_id"] = "PL-TEST-002"
        other_proof["proof_id"] = "PL-TEST-002.exact"
        other_proof["selector"] = "tests::second_contract"
        second_definition = normalized["behavior"][1]["proof"][0]
        other_proof["command"] = list(evidence.rust_execution_argv(second_definition))

        contract["proofs"], other["proofs"] = other["proofs"], contract["proofs"]
        aggregate = evidence.aggregate_evidence(
            normalized,
            [contract, other],
            digest=DIGEST,
            expected_sha=SHA,
            expected_repository="owner/repo",
            expected_run_id="42",
            expected_run_attempt=1,
            job_results={
                "checks": "success",
                "contract": "success",
                "other": "success",
            },
        )
        self.assertEqual(aggregate["status"], "failed")
        self.assertTrue(any("appeared in lane" in error for error in aggregate["errors"]))

    def test_failed_dependency_cannot_be_forged_by_passing_shard(self) -> None:
        aggregate = self.aggregate(
            [self.shard()],
            jobs={"checks": "failure", "contract": "success"},
        )
        self.assertEqual(aggregate["status"], "failed")
        self.assertTrue(any("checks" in error for error in aggregate["errors"]))

    def test_aggregate_output_is_deterministic(self) -> None:
        first = self.aggregate([self.shard()])
        second = self.aggregate([copy.deepcopy(self.shard())])
        self.assertEqual(first, second)

    def test_duplicate_json_keys_fail_closed(self) -> None:
        with self.assertRaisesRegex(evidence.EvidenceError, "duplicate JSON key"):
            evidence.load_json_text('{"status":"passed","status":"failed"}')

    def test_git_head_must_match_evidence_sha(self) -> None:
        completed = subprocess.CompletedProcess(
            ["git", "rev-parse", "--verify", "HEAD"],
            0,
            stdout=f"{SHA}\n",
            stderr="",
        )
        with mock.patch.object(evidence.subprocess, "run", return_value=completed):
            evidence.verify_git_head(self.repo, SHA)
            with self.assertRaisesRegex(evidence.EvidenceError, "does not match"):
                evidence.verify_git_head(self.repo, "f" * 40)

    def test_git_head_verification_fails_closed_when_git_fails(self) -> None:
        completed = subprocess.CompletedProcess(
            ["git", "rev-parse", "--verify", "HEAD"],
            128,
            stdout="",
            stderr="fatal: not a git repository",
        )
        with (
            mock.patch.object(evidence.subprocess, "run", return_value=completed),
            self.assertRaisesRegex(evidence.EvidenceError, "cannot verify"),
        ):
            evidence.verify_git_head(self.repo, SHA)

    def test_dirty_worktree_is_rejected(self) -> None:
        clean = subprocess.CompletedProcess(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"],
            0,
            stdout="",
            stderr="",
        )
        dirty = subprocess.CompletedProcess(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"],
            0,
            stdout=" M src/lib.rs\n?? unexpected.txt\n",
            stderr="",
        )
        with mock.patch.object(evidence.subprocess, "run", return_value=clean):
            evidence.verify_clean_worktree(self.repo)
        with (
            mock.patch.object(evidence.subprocess, "run", return_value=dirty),
            self.assertRaisesRegex(evidence.EvidenceError, "clean Git worktree"),
        ):
            evidence.verify_clean_worktree(self.repo)

    def test_lane_rechecks_provenance_after_proof_execution(self) -> None:
        catalog_path = self.repo / "behaviors.toml"
        catalog_path.write_text("# supplied by test double\n", encoding="utf-8")
        output_path = self.repo / "lane.json"
        args = argparse.Namespace(
            repo_root=str(self.repo),
            catalog=str(catalog_path),
            output=str(output_path),
            sha=SHA,
            repository="owner/repo",
            run_id="42",
            run_attempt=1,
            os="linux",
            lane="contract",
        )
        head_check = mock.Mock()
        clean_check = mock.Mock(
            side_effect=[
                None,
                evidence.EvidenceError(
                    "evidence requires a clean Git worktree; unexpected entries: M src/lib.rs"
                ),
            ]
        )
        with (
            contextlib.redirect_stdout(io.StringIO()),
            mock.patch.object(evidence, "verify_git_head", head_check),
            mock.patch.object(evidence, "verify_clean_worktree", clean_check),
            mock.patch.object(evidence, "load_catalog", return_value=self.base_catalog()),
            mock.patch.object(evidence, "catalog_digest", return_value=DIGEST),
            mock.patch.object(evidence, "run_rust_lane"),
        ):
            self.assertEqual(evidence.run_lane(args), 1)

        self.assertEqual(head_check.call_count, 2)
        self.assertEqual(clean_check.call_count, 2)
        rendered = json.loads(output_path.read_text(encoding="utf-8"))
        self.assertEqual(rendered["status"], "error")
        self.assertTrue(
            any("clean Git worktree" in error for error in rendered["errors"])
        )


if __name__ == "__main__":
    unittest.main()
