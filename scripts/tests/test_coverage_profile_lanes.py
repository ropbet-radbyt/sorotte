from __future__ import annotations

import copy
import json
import os
import pathlib
import sys
import tempfile
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import coverage_profile_lanes as lanes  # noqa: E402


def command_result(
    stdout: bytes,
    *,
    stderr: bytes = b"",
    returncode: int = 0,
    command: tuple[str, ...] = ("fixture",),
) -> lanes.CommandResult:
    return lanes.CommandResult(
        command=command,
        returncode=returncode,
        stdout=stdout,
        stderr=stderr,
        duration_seconds=0.01,
    )


class CoverageProfileLaneTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        (self.root / "Cargo.toml").write_text(
            "[workspace]\n",
            encoding="utf-8",
        )
        self.target = self.root / "target"
        self.target.mkdir()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def show_env_output(self, **overrides: str) -> bytes:
        wrapper = self.root / (
            "cargo-llvm-cov.exe" if os.name == "nt" else "cargo-llvm-cov"
        )
        values = {
            "LLVM_PROFILE_FILE": str(
                self.target / "fixture-%p-%32m.profraw"
            ),
            "__CARGO_LLVM_COV_RUSTC_WRAPPER": "1",
            "__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS": (
                "-C\x1finstrument-coverage\x1f--cfg=coverage"
            ),
            "__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES": (
                "sorotte_gui_semantic_suite,sorotte_compat_tests"
            ),
            "RUSTC_WRAPPER": str(wrapper),
            "CARGO_LLVM_COV": "1",
            "CARGO_LLVM_COV_SHOW_ENV": "1",
            "CARGO_LLVM_COV_TARGET_DIR": str(self.target),
            "CARGO_LLVM_COV_BUILD_DIR": str(self.target),
        }
        values.update(overrides)
        return (
            "\n".join(f"{key}={value}" for key, value in values.items()) + "\n"
        ).encode("utf-8")

    def semantic_document(self) -> dict[str, object]:
        reports = [
            {
                "result": "ok",
                "scenario": scenario,
                "view": "main",
                "modal": "none",
                "pending": "none",
                "widgets": 12,
            }
            for scenario in lanes.EXPECTED_SEMANTIC_SCENARIOS
        ]
        return {
            "result": "ok",
            "total": len(reports),
            "passed": len(reports),
            "failed": 0,
            "reports": reports,
            "errors": [],
        }

    @staticmethod
    def log_metadata(name: str) -> dict[str, object]:
        return {
            "path": f"target/verification/coverage-profile-logs/{name}.log",
            "size_bytes": 0,
            "sha256": lanes.sha256(b""),
        }

    def lane_entry(self, lane: str) -> dict[str, object]:
        if lane == "workspace-all-features":
            oracle: dict[str, object] = {
                "kind": "exit-and-profile-delta"
            }
        elif lane == "gui-semantic":
            oracle = {
                "kind": "semantic-suite-json",
                "total": len(lanes.EXPECTED_SEMANTIC_SCENARIOS),
                "passed": len(lanes.EXPECTED_SEMANTIC_SCENARIOS),
                "failed": 0,
                "scenarios": list(lanes.EXPECTED_SEMANTIC_SCENARIOS),
            }
        elif lane == "compat-live-tls":
            oracle = {
                "kind": "libtest-exact-live-tls",
                "passed": len(lanes.EXPECTED_COMPAT_TESTS),
                "failed": 0,
                "ignored": 0,
                "filtered_out": lanes.EXPECTED_COMPAT_FILTERED_OUT,
                "tests": list(lanes.EXPECTED_COMPAT_TESTS),
                "skip_markers": [],
            }
        else:
            oracle = {
                "kind": "llvm-profile-merge",
                "summary_detected": True,
            }
        deltas = (
            [
                {
                    "path": f"target/{lane}.profraw",
                    "size_bytes": 3,
                    "sha256": lanes.sha256(b"raw"),
                }
            ]
            if lane in lanes.PROFILE_LANES
            else []
        )
        profile_counts = {
            "workspace-all-features": (0, 1),
            "gui-semantic": (1, 2),
            "compat-live-tls": (2, 3),
            "merge-check": (3, 3),
        }
        before_count, after_count = profile_counts[lane]
        return {
            "status": "passed",
            "command": list(lanes.LANE_COMMANDS[lane]),
            "instrumentation": lanes.LANE_INSTRUMENTATION[lane],
            "environment_overrides": list(
                lanes.LANE_ENVIRONMENT_OVERRIDES[lane]
            ),
            "exit_code": 0,
            "duration_seconds": 0.01,
            "stdout": self.log_metadata(f"{lane}.stdout"),
            "stderr": self.log_metadata(f"{lane}.stderr"),
            "profile_count_before": before_count,
            "profile_count_after": after_count,
            "profile_delta_count": len(deltas),
            "profile_deltas": deltas,
            "profile_removed_count": 0,
            "oracle": oracle,
            "errors": [],
        }

    def valid_report(self) -> dict[str, object]:
        return {
            "schema_version": lanes.SCHEMA_VERSION,
            "kind": lanes.REPORT_KIND,
            "status": "passed",
            "producer": {
                "cargo_llvm_cov_version": (
                    lanes.PINNED_CARGO_LLVM_COV_VERSION
                ),
                "version_command": list(lanes.VERSION_COMMAND),
            },
            "legacy_reference": {
                "path": ".interop-cache/syncplay-legacy",
                "commit_sha": lanes.PINNED_LEGACY_SYNCPLAY_SHA,
            },
            "instrumentation_environment": {
                "keys": sorted(lanes.EXPECTED_SHOW_ENV_KEYS),
                "profile_pattern": "target/fixture-%p-%32m.profraw",
                "target_dir": "target",
                "build_dir": "target",
                "crate_count": 2,
                "required_crates": sorted(lanes.REQUIRED_INSTRUMENTED_CRATES),
            },
            "profile_reset": {
                "kind": "fresh-profile-reset",
                "profile_root": "target",
                "removed_raw_profile_count": 2,
                "removed_merged_profile_count": 1,
                "remaining_raw_profile_count": 0,
                "remaining_merged_profile_count": 0,
            },
            "lane_order": list(lanes.LANE_ORDER),
            "lanes": {
                lane: self.lane_entry(lane) for lane in lanes.LANE_ORDER
            },
            "errors": [],
        }

    def test_pinned_producer_version_is_exact(self) -> None:
        result = command_result(b"cargo-llvm-cov 0.8.4\n")
        self.assertEqual(lanes.parse_producer_version(result), "0.8.4")

        stale = command_result(b"cargo-llvm-cov 0.8.3\n")
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "must be 0.8.4",
        ):
            lanes.parse_producer_version(stale)

    def test_show_env_is_shell_neutral_and_source_bound(self) -> None:
        environment, summary, profile_root = lanes.parse_show_env(
            command_result(self.show_env_output()),
            repo_root=self.root,
        )
        self.assertEqual(set(environment), lanes.EXPECTED_SHOW_ENV_KEYS)
        self.assertEqual(profile_root, self.target)
        self.assertEqual(summary["target_dir"], "target")
        self.assertEqual(
            summary["required_crates"],
            sorted(lanes.REQUIRED_INSTRUMENTED_CRATES),
        )

    def test_show_env_rejects_missing_duplicate_and_unknown_keys(self) -> None:
        missing = b"\n".join(self.show_env_output().splitlines()[:-1]) + b"\n"
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "fields do not match schema",
        ):
            lanes.parse_show_env(
                command_result(missing),
                repo_root=self.root,
            )

        duplicate = self.show_env_output() + b"CARGO_LLVM_COV=1\n"
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "duplicate key",
        ):
            lanes.parse_show_env(
                command_result(duplicate),
                repo_root=self.root,
            )

        unknown = self.show_env_output() + b"UNREVIEWED_FLAG=1\n"
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "fields do not match schema",
        ):
            lanes.parse_show_env(
                command_result(unknown),
                repo_root=self.root,
            )

    def test_show_env_rejects_uninstrumented_or_escaping_profiles(self) -> None:
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "instrument-coverage",
        ):
            lanes.parse_show_env(
                command_result(
                    self.show_env_output(
                        __CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS="-C\x1fopt-level=0"
                    )
                ),
                repo_root=self.root,
            )

        outside = self.root.parent / "outside" / "fixture-%p-%32m.profraw"
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "inside repository root",
        ):
            lanes.parse_show_env(
                command_result(
                    self.show_env_output(LLVM_PROFILE_FILE=str(outside))
                ),
                repo_root=self.root,
            )

    def test_show_env_requires_semantic_and_compatibility_crates(self) -> None:
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "omits required instrumented crates",
        ):
            lanes.parse_show_env(
                command_result(
                    self.show_env_output(
                        __CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES="demo"
                    )
                ),
                repo_root=self.root,
            )

    def test_profile_delta_hashes_only_new_or_changed_profiles(self) -> None:
        unchanged = self.target / "unchanged.profraw"
        internal_root = self.target / "llvm-cov-target"
        internal_root.mkdir()
        changed = internal_root / "changed.profraw"
        new = self.target / "new.profraw"
        unchanged.write_bytes(b"same")
        changed.write_bytes(b"old")
        before = lanes.profile_inventory(
            self.target,
            repo_root=self.root,
        )

        changed.write_bytes(b"new-content")
        new.write_bytes(b"new")
        after = lanes.profile_inventory(
            self.target,
            repo_root=self.root,
        )
        delta = lanes.profile_delta(
            before,
            after,
            repo_root=self.root,
        )
        self.assertEqual(
            [item["path"] for item in delta],
            [
                "target/llvm-cov-target/changed.profraw",
                "target/new.profraw",
            ],
        )
        self.assertEqual(delta[0]["sha256"], lanes.sha256(b"new-content"))

    def test_profile_delta_detects_same_size_same_mtime_content_change(
        self,
    ) -> None:
        profile = self.target / "reused.profraw"
        profile.write_bytes(b"old")
        before = lanes.profile_inventory(
            self.target,
            repo_root=self.root,
        )
        original = profile.stat()

        profile.write_bytes(b"new")
        os.utime(
            profile,
            ns=(original.st_atime_ns, original.st_mtime_ns),
        )
        after = lanes.profile_inventory(
            self.target,
            repo_root=self.root,
        )

        delta = lanes.profile_delta(
            before,
            after,
            repo_root=self.root,
        )
        self.assertEqual(len(delta), 1)
        self.assertEqual(delta[0]["sha256"], lanes.sha256(b"new"))

    def test_profile_reset_removes_only_owned_coverage_artifacts(self) -> None:
        internal_root = self.target / "llvm-cov-target"
        internal_root.mkdir()
        (self.target / "external.profraw").write_bytes(b"external")
        (internal_root / "workspace.profraw").write_bytes(b"workspace")
        (internal_root / "workspace.profdata").write_bytes(b"merged")
        unrelated = internal_root / "keep.txt"
        unrelated.write_text("keep\n", encoding="utf-8")

        result = lanes.reset_profile_artifacts(
            self.target,
            repo_root=self.root,
        )

        self.assertEqual(
            result,
            {
                "kind": "fresh-profile-reset",
                "profile_root": "target",
                "removed_raw_profile_count": 2,
                "removed_merged_profile_count": 1,
                "remaining_raw_profile_count": 0,
                "remaining_merged_profile_count": 0,
            },
        )
        self.assertTrue(unrelated.is_file())
        self.assertEqual(
            lanes.profile_inventory(self.target, repo_root=self.root),
            {},
        )

    def test_profile_reset_rejects_repository_root_or_symlink(self) -> None:
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "inside the repository target directory",
        ):
            lanes.reset_profile_artifacts(
                self.root,
                repo_root=self.root,
            )

        if hasattr(os, "symlink"):
            target = self.target / "real.profraw"
            target.write_bytes(b"profile")
            link = self.target / "linked.profraw"
            try:
                link.symlink_to(target)
            except OSError:
                return
            with self.assertRaisesRegex(
                lanes.CoverageProfileLaneError,
                "non-symlink",
            ):
                lanes.reset_profile_artifacts(
                    self.target,
                    repo_root=self.root,
                )

    def test_empty_profile_fails_closed(self) -> None:
        empty = self.target / "empty.profraw"
        empty.write_bytes(b"")
        after = lanes.profile_inventory(
            self.target,
            repo_root=self.root,
        )
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "profile is empty",
        ):
            lanes.profile_delta({}, after, repo_root=self.root)

    def test_semantic_oracle_requires_exact_fourteen_scenario_inventory(
        self,
    ) -> None:
        document = self.semantic_document()
        oracle = lanes.semantic_oracle(
            (json.dumps(document) + "\n").encode("utf-8")
        )
        self.assertEqual(oracle["total"], 14)
        self.assertEqual(
            oracle["scenarios"],
            list(lanes.EXPECTED_SEMANTIC_SCENARIOS),
        )

        document["reports"] = list(reversed(document["reports"]))
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "missing, duplicated, or reordered",
        ):
            lanes.semantic_oracle(json.dumps(document).encode("utf-8"))

    def test_semantic_oracle_rejects_partial_or_malformed_success(self) -> None:
        document = self.semantic_document()
        document["passed"] = 13
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "exactly 14",
        ):
            lanes.semantic_oracle(json.dumps(document).encode("utf-8"))

        raw = json.dumps(self.semantic_document()).replace(
            '"result": "ok"',
            '"result": "ok", "result": "ok"',
            1,
        )
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "duplicate key",
        ):
            lanes.semantic_oracle(raw.encode("utf-8"))

    def test_compatibility_oracle_requires_complete_live_inventory(self) -> None:
        output = "running 4 tests\n"
        for test_name in lanes.EXPECTED_COMPAT_TESTS:
            output += f"test tests::legacy_tls_tests::{test_name} ... ok\n"
        output += (
            "test result: ok. 4 passed; 0 failed; 0 ignored; "
            "0 measured; 140 filtered out; finished in 1.00s\n"
        )
        oracle = lanes.compatibility_oracle(output.encode("utf-8"), b"")
        self.assertEqual(oracle["passed"], 4)
        self.assertEqual(oracle["ignored"], 0)
        self.assertEqual(
            oracle["tests"],
            list(lanes.EXPECTED_COMPAT_TESTS),
        )

    def test_compatibility_oracle_rejects_skip_and_count_drift(self) -> None:
        good_summary = "running 4 tests\n"
        for test_name in lanes.EXPECTED_COMPAT_TESTS:
            good_summary += (
                f"test tests::legacy_tls_tests::{test_name} ... ok\n"
            )
        good_summary += (
            "test result: ok. 4 passed; 0 failed; 0 ignored; "
            "0 measured; 140 filtered out\n"
        )
        good_summary_bytes = good_summary.encode("utf-8")
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "skipped-oracle markers",
        ):
            lanes.compatibility_oracle(
                good_summary_bytes,
                b"assertion skipped due to missing prerequisites",
            )

        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "strict live TLS test",
        ):
            lanes.compatibility_oracle(
                good_summary_bytes.replace(
                    lanes.EXPECTED_COMPAT_TESTS[0].encode("utf-8"),
                    b"missing-test",
                ),
                b"",
            )

    def test_merge_oracle_requires_llvm_total_summary(self) -> None:
        oracle = lanes.merge_oracle(
            b"Filename Regions Missed Regions Cover\nTOTAL 10 1 90.00%\n",
            b"",
        )
        self.assertTrue(oracle["summary_detected"])
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "TOTAL",
        ):
            lanes.merge_oracle(b"no summary\n", b"")

    def test_complete_report_validates(self) -> None:
        report = self.valid_report()
        self.assertIs(lanes.validate_report_document(report), report)

    def test_report_rejects_missing_profile_delta(self) -> None:
        report = self.valid_report()
        lane = report["lanes"]["gui-semantic"]
        lane["profile_delta_count"] = 0
        lane["profile_deltas"] = []
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "produced no instrumented profile delta",
        ):
            lanes.validate_report_document(report)

    def test_report_rejects_stale_reset_or_discontinuous_inventory(self) -> None:
        report = self.valid_report()
        report["profile_reset"]["remaining_raw_profile_count"] = 1
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "retained stale profile artifacts",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["lanes"]["gui-semantic"]["profile_removed_count"] = 1
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "removed profiles",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["lanes"]["gui-semantic"]["profile_count_before"] = 0
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "not continuous",
        ):
            lanes.validate_report_document(report)

    def test_report_rejects_command_or_oracle_drift(self) -> None:
        report = self.valid_report()
        report["lanes"]["compat-live-tls"]["command"] = ["cargo", "test"]
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "command drifted",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["lanes"]["gui-semantic"]["oracle"]["passed"] = 13
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "oracle does not match",
        ):
            lanes.validate_report_document(report)

    def test_report_rejects_unpinned_reference_and_unknown_fields(self) -> None:
        report = self.valid_report()
        report["legacy_reference"]["commit_sha"] = "f" * 40
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "unpinned legacy reference",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["unreviewed"] = True
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "fields do not match schema",
        ):
            lanes.validate_report_document(report)

    def test_strict_report_loader_rejects_duplicate_keys(self) -> None:
        path = self.root / "report.json"
        report = self.valid_report()
        raw = json.dumps(report)
        raw = raw.replace(
            f'"kind": "{lanes.REPORT_KIND}"',
            f'"kind": "{lanes.REPORT_KIND}", "kind": "{lanes.REPORT_KIND}"',
            1,
        )
        path.write_text(raw, encoding="utf-8")
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "duplicate key",
        ):
            lanes.strict_load_report(path)

    def test_failed_or_incomplete_report_never_validates(self) -> None:
        report = self.valid_report()
        report["status"] = "failed"
        report["errors"] = ["semantic lane failed"]
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "status is 'failed'",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        del report["lanes"]["merge-check"]
        with self.assertRaisesRegex(
            lanes.CoverageProfileLaneError,
            "fields do not match schema",
        ):
            lanes.validate_report_document(report)

    def test_lane_constants_do_not_accept_arbitrary_shell_fragments(self) -> None:
        for lane, command in lanes.LANE_COMMANDS.items():
            with self.subTest(lane=lane):
                self.assertIsInstance(command, tuple)
                self.assertEqual(command[0], "cargo")
                self.assertNotIn("shell", command)
                self.assertFalse(any("\n" in argument for argument in command))

    def test_report_validation_does_not_mutate_input(self) -> None:
        report = self.valid_report()
        before = copy.deepcopy(report)
        lanes.validate_report_document(report)
        self.assertEqual(report, before)


if __name__ == "__main__":
    unittest.main()
