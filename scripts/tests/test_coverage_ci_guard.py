from __future__ import annotations

import contextlib
import io
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import coverage_ci_guard as guard  # noqa: E402


class CoverageBaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "-b", "main")
        self.git("config", "user.email", "coverage-tests@example.invalid")
        self.git("config", "user.name", "Coverage Tests")
        self.base = self.commit("base")
        self.git("switch", "-c", "feature")
        self.head = self.commit("feature")
        self.git("switch", "main")
        self.base_tip = self.commit("base-tip")
        self.git("update-ref", "refs/remotes/origin/main", self.base_tip)
        self.git("switch", "feature")
        self.output = self.root / "coverage-base.json"
        self.github_env = self.root / "github-env"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def git(self, *argv: str) -> str:
        process = subprocess.run(
            ["git", "-C", str(self.repo), *argv],
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        return process.stdout.strip()

    def commit(self, value: str) -> str:
        source = self.repo / "value.txt"
        source.write_text(value + "\n", encoding="utf-8")
        self.git("add", "value.txt")
        self.git("commit", "-m", value)
        return self.git("rev-parse", "HEAD")

    def invoke(
        self,
        *,
        event: str,
        verification: str | None = None,
        pull_request_base: str = "",
        push_before: str = "",
        push_ref_type: str = "branch",
        default_branch: str = "",
        dispatch_base: str = "",
    ) -> tuple[int, dict[str, object]]:
        argv = [
            "resolve-base",
            "--repo-root",
            str(self.repo),
            "--event-name",
            event,
            "--verification-sha",
            verification or self.head,
            "--pull-request-base",
            pull_request_base,
            "--push-before",
            push_before,
            "--push-ref-type",
            push_ref_type,
            "--default-branch",
            default_branch,
            "--dispatch-base",
            dispatch_base,
            "--github-env",
            str(self.github_env),
            "--output",
            str(self.output),
        ]
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
            io.StringIO()
        ):
            result = guard.main(argv)
        return result, json.loads(self.output.read_text(encoding="utf-8"))

    def test_pull_request_records_single_merge_base_not_base_tip(self) -> None:
        result, report = self.invoke(
            event="pull_request",
            pull_request_base=self.base_tip,
        )
        self.assertEqual(result, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["mode"], "pull-request-merge-base")
        self.assertEqual(report["requested_base_sha"], self.base_tip)
        self.assertEqual(report["effective_base_sha"], self.base)
        self.assertEqual(report["merge_bases"], [self.base])
        environment = self.github_env.read_text(encoding="utf-8").splitlines()
        self.assertIn(f"COVERAGE_BASE_SHA={self.base}", environment)
        self.assertIn(
            f"COVERAGE_REQUESTED_BASE_SHA={self.base_tip}",
            environment,
        )

    def test_push_uses_exact_event_before_without_merge_base(self) -> None:
        result, report = self.invoke(event="push", push_before=self.base_tip)
        self.assertEqual(result, 0)
        self.assertEqual(report["mode"], "push-before")
        self.assertEqual(report["requested_base_sha"], self.base_tip)
        self.assertEqual(report["effective_base_sha"], self.base_tip)
        self.assertEqual(report["merge_bases"], [])

    def test_tag_push_uses_default_branch_merge_base_when_before_is_zero(self) -> None:
        result, report = self.invoke(
            event="push",
            push_before="0" * 40,
            push_ref_type="tag",
            default_branch="main",
        )
        self.assertEqual(result, 0)
        self.assertEqual(report["mode"], "tag-default-branch-merge-base")
        self.assertEqual(report["push_before_sha_input"], "0" * 40)
        self.assertEqual(report["push_ref_type"], "tag")
        self.assertEqual(report["default_branch_name"], "main")
        self.assertEqual(
            report["default_branch_ref"],
            "refs/remotes/origin/main",
        )
        self.assertEqual(report["default_branch_sha"], self.base_tip)
        self.assertEqual(report["requested_base_sha"], self.base_tip)
        self.assertEqual(report["effective_base_sha"], self.base)
        self.assertEqual(report["merge_bases"], [self.base])

    def test_updated_tag_uses_exact_event_before_without_default_branch(self) -> None:
        result, report = self.invoke(
            event="push",
            push_before=self.base_tip,
            push_ref_type="tag",
            default_branch="main",
        )
        self.assertEqual(result, 0)
        self.assertEqual(report["mode"], "tag-push-before")
        self.assertEqual(report["push_before_sha_input"], self.base_tip)
        self.assertEqual(report["push_ref_type"], "tag")
        self.assertEqual(report["default_branch_name"], "main")
        self.assertIsNone(report["default_branch_ref"])
        self.assertIsNone(report["default_branch_sha"])
        self.assertEqual(report["requested_base_sha"], self.base_tip)
        self.assertEqual(report["effective_base_sha"], self.base_tip)
        self.assertEqual(report["merge_bases"], [])

    def test_tag_push_rejects_missing_before_instead_of_guessing(self) -> None:
        result, report = self.invoke(
            event="push",
            push_ref_type="tag",
            default_branch="main",
        )
        self.assertEqual(result, 2)
        self.assertEqual(report["status"], "error")
        self.assertIn("tag push before SHA", report["errors"][0])
        self.assertIsNone(report["effective_base_sha"])

    def test_workflow_dispatch_requires_and_uses_explicit_base(self) -> None:
        result, report = self.invoke(
            event="workflow_dispatch",
            dispatch_base=self.base,
        )
        self.assertEqual(result, 0)
        self.assertEqual(report["mode"], "workflow-dispatch-explicit")
        self.assertEqual(report["effective_base_sha"], self.base)

        result, report = self.invoke(event="workflow_dispatch")
        self.assertEqual(result, 2)
        self.assertEqual(report["status"], "error")
        self.assertRegex(report["errors"][0], r"exactly 40 hexadecimal")

    def test_zero_push_before_is_rejected_and_never_falls_back_to_parent(self) -> None:
        result, report = self.invoke(event="push", push_before="0" * 40)
        self.assertEqual(result, 2)
        self.assertEqual(report["status"], "error")
        self.assertIsNone(report["effective_base_sha"])
        self.assertIn("all-zero SHA", report["errors"][0])
        self.assertFalse(self.github_env.exists())

    def test_checkout_must_be_exact_verification_revision(self) -> None:
        result, report = self.invoke(
            event="push",
            verification=self.base,
            push_before=self.base,
        )
        self.assertEqual(result, 2)
        self.assertEqual(report["status"], "error")
        self.assertIn("does not match verification SHA", report["errors"][0])

    def test_github_environment_write_failure_fails_closed_in_json(self) -> None:
        self.github_env.mkdir()
        result, report = self.invoke(event="push", push_before=self.base)
        self.assertEqual(result, 2)
        self.assertEqual(report["status"], "error")
        self.assertIn("cannot append", report["errors"][0])

    def test_unknown_event_writes_error_report(self) -> None:
        result, report = self.invoke(event="schedule")
        self.assertEqual(result, 2)
        self.assertEqual(report["kind"], guard.BASE_REPORT_KIND)
        self.assertEqual(report["status"], "error")
        self.assertIn("event name must be", report["errors"][0])


class CoverageFinalizerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.base = self.root / "base.json"
        self.llvm_json = self.root / "coverage.json"
        self.llvm_text = self.root / "coverage.txt"
        self.line_map = self.root / "line-map.json"
        self.policy = self.root / "policy.json"
        self.output = self.root / "phases.json"
        self.base.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "kind": guard.BASE_REPORT_KIND,
                    "status": "passed",
                    "effective_base_sha": "1" * 40,
                }
            ),
            encoding="utf-8",
        )
        self.llvm_json.write_text('{"native":"llvm-json"}\n', encoding="utf-8")
        self.llvm_text.write_text("native llvm source view\n", encoding="utf-8")
        self.write_line_map()
        self.write_policy()

    def write_line_map(
        self,
        *,
        status: str = "passed",
        errors: list[str] | None = None,
    ) -> None:
        self.line_map.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "kind": guard.LINE_MAP_REPORT_KIND,
                    "status": status,
                    "line_model": "unique-physical-source-lines",
                    "inputs": {
                        "llvm_json": {
                            "path": str(self.llvm_json),
                            "size_bytes": self.llvm_json.stat().st_size,
                            "sha256": "sha256:"
                            + guard.hashlib.sha256(
                                self.llvm_json.read_bytes()
                            ).hexdigest(),
                        },
                        "llvm_text": {
                            "path": str(self.llvm_text),
                            "size_bytes": self.llvm_text.stat().st_size,
                            "sha256": "sha256:"
                            + guard.hashlib.sha256(
                                self.llvm_text.read_bytes()
                            ).hexdigest(),
                        },
                    },
                    "producer": {
                        "llvm_export_type": "llvm.coverage.json.export",
                        "llvm_export_version": "3.1.0",
                        "cargo_llvm_cov_version": "0.8.4",
                        "manifest_path": "Cargo.toml",
                    },
                    "errors": errors or [],
                }
            ),
            encoding="utf-8",
        )

    def write_policy(
        self,
        *,
        status: str = "passed",
        errors: list[str] | None = None,
        line_map_digest: str | None = None,
    ) -> None:
        digest = line_map_digest or guard.hashlib.sha256(
            self.line_map.read_bytes()
        ).hexdigest()
        self.policy.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "kind": guard.DIFF_REPORT_KIND,
                    "status": status,
                    "inputs": {
                        "coverage_kind": "llvm-physical-line-map",
                        "coverage_map_sha256": f"sha256:{digest}",
                    },
                    "errors": errors or [],
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def invoke(
        self,
        *,
        base_outcome: str = "success",
        profiles_outcome: str = "success",
        llvm_json_outcome: str = "success",
        llvm_text_outcome: str = "success",
        line_map_outcome: str = "success",
        policy_outcome: str = "success",
    ) -> tuple[int, dict[str, object]]:
        argv = [
            "finalize",
            "--base-outcome",
            base_outcome,
            "--profiles-outcome",
            profiles_outcome,
            "--llvm-json-outcome",
            llvm_json_outcome,
            "--llvm-text-outcome",
            llvm_text_outcome,
            "--line-map-outcome",
            line_map_outcome,
            "--policy-outcome",
            policy_outcome,
            "--base-report",
            str(self.base),
            "--llvm-json",
            str(self.llvm_json),
            "--llvm-text",
            str(self.llvm_text),
            "--line-map",
            str(self.line_map),
            "--policy-report",
            str(self.policy),
            "--output",
            str(self.output),
        ]
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
            io.StringIO()
        ):
            result = guard.main(argv)
        return result, json.loads(self.output.read_text(encoding="utf-8"))

    def test_success_requires_all_six_bound_phases(self) -> None:
        result, report = self.invoke()
        self.assertEqual(result, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(
            [
                report["phases"][name]["status"]
                for name in report["phase_order"]
            ],
            ["passed", "passed", "passed", "passed", "passed", "passed"],
        )
        self.assertEqual(
            report["phases"]["llvm-json"]["sha256"],
            guard.hashlib.sha256(self.llvm_json.read_bytes()).hexdigest(),
        )
        self.assertEqual(
            report["phases"]["llvm-text"]["sha256"],
            guard.hashlib.sha256(self.llvm_text.read_bytes()).hexdigest(),
        )
        self.assertEqual(
            report["phases"]["line-map"]["report"]["line_model"],
            "unique-physical-source-lines",
        )

    def test_base_failure_still_writes_phase_aware_diagnostics(self) -> None:
        self.base.unlink()
        self.llvm_json.unlink()
        self.llvm_text.unlink()
        self.line_map.unlink()
        self.policy.unlink()
        result, report = self.invoke(
            base_outcome="failure",
            profiles_outcome="skipped",
            llvm_json_outcome="skipped",
            llvm_text_outcome="skipped",
            line_map_outcome="skipped",
            policy_outcome="skipped",
        )
        self.assertEqual(result, 1)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["phases"]["resolve-base"]["status"], "failed")
        self.assertEqual(report["phases"]["coverage-profiles"]["status"], "blocked")
        self.assertEqual(report["phases"]["llvm-json"]["status"], "blocked")
        self.assertEqual(report["phases"]["llvm-text"]["status"], "blocked")
        self.assertEqual(report["phases"]["line-map"]["status"], "blocked")
        self.assertEqual(report["phases"]["diff-policy"]["status"], "blocked")
        self.assertTrue(
            any("cannot read base report" in error for error in report["errors"])
        )

    def test_json_export_failure_is_recorded_even_when_consumers_are_skipped(
        self,
    ) -> None:
        self.llvm_json.unlink()
        result, report = self.invoke(
            llvm_json_outcome="failure",
            line_map_outcome="skipped",
            policy_outcome="skipped",
        )
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"]["resolve-base"]["status"], "passed")
        self.assertEqual(report["phases"]["coverage-profiles"]["status"], "passed")
        self.assertEqual(report["phases"]["llvm-json"]["status"], "failed")
        self.assertEqual(report["phases"]["llvm-text"]["status"], "passed")
        self.assertEqual(report["phases"]["line-map"]["status"], "blocked")
        self.assertEqual(report["phases"]["diff-policy"]["status"], "blocked")

    def test_line_map_must_bind_both_retained_native_artifacts(self) -> None:
        self.llvm_text.write_text("tampered after conversion\n", encoding="utf-8")
        self.write_policy()
        result, report = self.invoke()
        self.assertEqual(result, 1)
        phase = report["phases"]["line-map"]
        self.assertEqual(phase["status"], "failed")
        self.assertTrue(
            any("does not match" in error for error in phase["errors"])
        )

    def test_line_map_failure_report_is_retained(self) -> None:
        self.write_line_map(status="error", errors=["source content drifted"])
        result, report = self.invoke(
            line_map_outcome="failure",
            policy_outcome="skipped",
        )
        self.assertEqual(result, 1)
        phase = report["phases"]["line-map"]
        self.assertEqual(phase["status"], "failed")
        self.assertEqual(phase["report"]["errors"], ["source content drifted"])

    def test_policy_failure_embeds_normal_report_diagnostics(self) -> None:
        self.write_policy(
            status="failed",
            errors=["changed-line coverage 75.00% is below required 80.00%"],
        )
        result, report = self.invoke(policy_outcome="failure")
        self.assertEqual(result, 1)
        phase = report["phases"]["diff-policy"]
        self.assertEqual(phase["status"], "failed")
        self.assertEqual(phase["report"]["status"], "failed")
        self.assertIn("step outcome was failure", phase["errors"][0])

    def test_policy_report_must_bind_retained_line_map_digest(self) -> None:
        self.write_policy(line_map_digest="f" * 64)
        result, report = self.invoke()
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"]["diff-policy"]["status"], "failed")
        self.assertTrue(
            any(
                "not bound" in error
                for error in report["phases"]["diff-policy"]["errors"]
            )
        )

    def test_success_outcome_with_malformed_policy_fails_closed(self) -> None:
        self.policy.write_text("{", encoding="utf-8")
        result, report = self.invoke()
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"]["diff-policy"]["status"], "failed")
        self.assertIn("not valid UTF-8 JSON", report["errors"][-1])

    def test_duplicate_keys_and_nonstandard_numbers_fail_closed(self) -> None:
        raw = self.policy.read_text(encoding="utf-8")
        self.policy.write_text(
            raw.replace(
                '"kind": "sorotte-diff-coverage"',
                '"kind": "sorotte-diff-coverage", '
                '"kind": "sorotte-diff-coverage"',
            ),
            encoding="utf-8",
        )
        result, report = self.invoke()
        self.assertEqual(result, 1)
        self.assertTrue(
            any("duplicates object key" in error for error in report["errors"])
        )

        self.write_policy()
        raw = self.policy.read_text(encoding="utf-8").removesuffix("}")
        self.policy.write_text(raw + ', "nonstandard": NaN}', encoding="utf-8")
        result, report = self.invoke()
        self.assertEqual(result, 1)
        self.assertTrue(
            any("numeric constant" in error for error in report["errors"])
        )


if __name__ == "__main__":
    unittest.main()
