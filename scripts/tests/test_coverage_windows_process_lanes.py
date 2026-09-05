from __future__ import annotations

import copy
import json
import os
import pathlib
import shlex
import stat
import sys
import tempfile
import unittest
from types import SimpleNamespace


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import coverage_profile_lanes as common  # noqa: E402
import coverage_windows_process_lanes as lanes  # noqa: E402


def command_result(
    stdout: bytes,
    *,
    stderr: bytes = b"",
    returncode: int = 0,
    command: tuple[str, ...] = ("fixture",),
) -> common.CommandResult:
    return common.CommandResult(
        command=command,
        returncode=returncode,
        stdout=stdout,
        stderr=stderr,
        duration_seconds=0.01,
    )


class WindowsProcessCoverageLaneTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        (self.root / "Cargo.toml").write_text(
            "[workspace]\n",
            encoding="utf-8",
        )
        self.target = self.root / pathlib.Path(lanes.TARGET_DIR)
        self.target.mkdir(parents=True)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_show_env_requests_stable_posix_shell_output(self) -> None:
        self.assertEqual(
            lanes.SHOW_ENV_COMMAND,
            ("cargo", "llvm-cov", "show-env", "--sh"),
        )

    def show_env_output(
        self,
        *,
        shell_quote: bool = True,
        **overrides: str,
    ) -> bytes:
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
            "__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES": ",".join(
                sorted(lanes.REQUIRED_INSTRUMENTED_CRATES)
            ),
            "RUSTC_WRAPPER": str(wrapper),
            "CARGO_LLVM_COV": "1",
            "CARGO_LLVM_COV_SHOW_ENV": "1",
            "CARGO_LLVM_COV_TARGET_DIR": str(self.target),
            "CARGO_LLVM_COV_BUILD_DIR": str(self.target),
        }
        values.update(overrides)
        render = shlex.quote if shell_quote else str
        return (
            "\n".join(
                f"export {key}={render(value)}"
                for key, value in values.items()
            )
            + "\n"
        ).encode("utf-8")

    @staticmethod
    def log_metadata(lane: str, stream: str) -> dict[str, object]:
        return {
            "path": (
                "target/verification/coverage-windows-process-logs/"
                f"{lane}.{stream}.log"
            ),
            "size_bytes": 0,
            "sha256": common.sha256(b""),
        }

    def source_identity(self) -> dict[str, object]:
        tracked_digest = common.sha256(b"")
        untracked: list[dict[str, object]] = []
        return {
            "head_command": list(lanes.HEAD_COMMAND),
            "head_commit": "a" * 40,
            "tracked_diff_command": list(lanes.TRACKED_DIFF_COMMAND),
            "tracked_diff_size_bytes": 0,
            "tracked_diff_sha256": tracked_digest,
            "untracked_command": list(lanes.UNTRACKED_COMMAND),
            "untracked_file_count": 0,
            "untracked_files": untracked,
            "is_clean": True,
            "working_tree_sha256": lanes.working_tree_digest(
                tracked_digest,
                untracked,
            ),
        }

    def lane_entry(
        self,
        lane: str,
        index: int,
    ) -> dict[str, object]:
        if lane in lanes.PROFILE_LANES:
            deltas = [
                {
                    "path": f"{lanes.TARGET_DIR}/{lane}.profraw",
                    "size_bytes": 3,
                    "sha256": common.sha256(b"raw"),
                }
            ]
            before, after = index, index + 1
        else:
            deltas = []
            before = after = len(lanes.PROFILE_LANES)
        return {
            "status": "passed",
            "command": list(lanes.LANE_COMMANDS[lane]),
            "instrumentation": lanes.LANE_INSTRUMENTATION[lane],
            "environment_overrides": list(
                lanes.LANE_ENVIRONMENT_OVERRIDES[lane]
            ),
            "exit_code": 0,
            "duration_seconds": 0.01,
            "stdout": self.log_metadata(lane, "stdout"),
            "stderr": self.log_metadata(lane, "stderr"),
            "profile_count_before": before,
            "profile_count_after": after,
            "profile_delta_count": len(deltas),
            "profile_deltas": deltas,
            "profile_removed_count": 0,
            "oracle": lanes.expected_oracle(lane),
            "errors": [],
        }

    def valid_report(self) -> dict[str, object]:
        return {
            "schema_version": lanes.SCHEMA_VERSION,
            "kind": lanes.REPORT_KIND,
            "status": "passed",
            "source_identity": self.source_identity(),
            "producer": {
                "cargo_llvm_cov": {
                    "version": lanes.PINNED_CARGO_LLVM_COV_VERSION,
                    "command": list(lanes.VERSION_COMMAND),
                },
                "rustc": {
                    "command": list(lanes.RUSTC_COMMAND),
                    "release": lanes.PINNED_RUST_RELEASE,
                    "commit_hash": lanes.PINNED_RUST_COMMIT,
                    "commit_date": "2026-07-14",
                    "host": lanes.PINNED_RUST_HOST,
                    "llvm_version": lanes.PINNED_LLVM_VERSION,
                },
                "platform": {
                    "system": "Windows",
                    "architecture": "x86_64",
                    "rust_host": lanes.PINNED_RUST_HOST,
                    "compatibility_domain": lanes.COMPATIBILITY_DOMAIN,
                },
            },
            "coverage_boundary": {
                "interactive_native_included": False,
                "interactive_native_reason": lanes.NATIVE_EXCLUSION_REASON,
                "profile_merge_scope": lanes.COMPATIBILITY_DOMAIN,
            },
            "instrumentation_environment": {
                "keys": sorted(common.EXPECTED_SHOW_ENV_KEYS),
                "profile_pattern": (
                    f"{lanes.TARGET_DIR}/fixture-%p-%32m.profraw"
                ),
                "target_dir": lanes.TARGET_DIR,
                "build_dir": lanes.TARGET_DIR,
                "crate_count": len(lanes.REQUIRED_INSTRUMENTED_CRATES),
                "required_crates": sorted(
                    lanes.REQUIRED_INSTRUMENTED_CRATES
                ),
            },
            "profile_reset": {
                "kind": "fresh-profile-reset",
                "profile_root": lanes.TARGET_DIR,
                "removed_raw_profile_count": 2,
                "removed_merged_profile_count": 1,
                "remaining_raw_profile_count": 0,
                "remaining_merged_profile_count": 0,
            },
            "lane_order": list(lanes.LANE_ORDER),
            "lanes": {
                lane: self.lane_entry(lane, index)
                for index, lane in enumerate(lanes.LANE_ORDER)
            },
            "errors": [],
        }

    @staticmethod
    def libtest_output(lane: str) -> bytes:
        tests = sorted(lanes.EXPECTED_TESTS[lane])
        test_noun = "test" if len(tests) == 1 else "tests"
        lines = [f"running {len(tests)} {test_noun}"]
        lines.extend(f"test {test} ... ok" for test in tests)
        lines.append(
            f"test result: ok. {len(tests)} passed; 0 failed; 0 ignored; "
            f"0 measured; {lanes.EXPECTED_FILTERED_OUT[lane]} filtered out; "
            "finished in 0.10s"
        )
        return ("\n".join(lines) + "\n").encode("utf-8")

    def test_rustc_identity_is_exactly_pinned(self) -> None:
        output = b"\n".join(
            [
                b"rustc 1.97.1 (8bab26f4f 2026-07-14)",
                b"binary: rustc",
                f"commit-hash: {lanes.PINNED_RUST_COMMIT}".encode(),
                b"commit-date: 2026-07-14",
                f"host: {lanes.PINNED_RUST_HOST}".encode(),
                f"release: {lanes.PINNED_RUST_RELEASE}".encode(),
                f"LLVM version: {lanes.PINNED_LLVM_VERSION}".encode(),
                b"",
            ]
        )
        identity = lanes.parse_rustc_identity(command_result(output))
        self.assertEqual(identity["host"], lanes.PINNED_RUST_HOST)
        self.assertEqual(identity["commit_hash"], lanes.PINNED_RUST_COMMIT)

        stale = output.replace(b"release: 1.97.1", b"release: 1.97.0")
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "release must be 1.97.1",
        ):
            lanes.parse_rustc_identity(command_result(stale))

    def test_untracked_source_status_rejects_links_and_windows_reparse_points(
        self,
    ) -> None:
        regular = SimpleNamespace(st_mode=stat.S_IFREG, st_file_attributes=0)
        symbolic_link = SimpleNamespace(
            st_mode=stat.S_IFLNK,
            st_file_attributes=0,
        )
        reparse_point = SimpleNamespace(
            st_mode=stat.S_IFREG,
            st_file_attributes=getattr(
                stat,
                "FILE_ATTRIBUTE_REPARSE_POINT",
                0x400,
            ),
        )

        self.assertFalse(lanes.is_link_or_reparse_status(regular))
        self.assertTrue(lanes.is_link_or_reparse_status(symbolic_link))
        self.assertTrue(lanes.is_link_or_reparse_status(reparse_point))

    def test_show_env_requires_isolation_and_all_process_crates(self) -> None:
        environment, summary, root = lanes.parse_show_env(
            command_result(self.show_env_output()),
            repo_root=self.root,
        )
        self.assertEqual(root, self.target)
        self.assertEqual(
            set(summary["required_crates"]),
            lanes.REQUIRED_INSTRUMENTED_CRATES,
        )
        self.assertEqual(set(environment), common.EXPECTED_SHOW_ENV_KEYS)

        outside = self.root / "target" / "wrong"
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "isolated",
        ):
            lanes.parse_show_env(
                command_result(
                    self.show_env_output(
                        CARGO_LLVM_COV_TARGET_DIR=str(outside),
                        CARGO_LLVM_COV_BUILD_DIR=str(outside),
                    )
                ),
                repo_root=self.root,
            )

        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "required instrumented crates",
        ):
            lanes.parse_show_env(
                command_result(
                    self.show_env_output(
                        __CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES=(
                            "sorotte_gui_updater"
                        )
                    )
                ),
                repo_root=self.root,
            )

    def test_show_env_accepts_quoted_dynamic_merge_pool(self) -> None:
        profile_pattern = self.target / "fixture-%p-%4m.profraw"
        environment, summary, root = lanes.parse_show_env(
            command_result(
                self.show_env_output(
                    shell_quote=True,
                    LLVM_PROFILE_FILE=str(profile_pattern),
                )
            ),
            repo_root=self.root,
        )
        self.assertEqual(
            environment["LLVM_PROFILE_FILE"],
            str(profile_pattern),
        )
        self.assertEqual(
            summary["profile_pattern"],
            f"{lanes.TARGET_DIR}/fixture-%p-%4m.profraw",
        )
        self.assertEqual(root, self.target)

    def test_exact_libtest_oracle_accepts_every_lane(self) -> None:
        total = 0
        for lane in lanes.PROFILE_LANES:
            with self.subTest(lane=lane):
                oracle = lanes.libtest_oracle(
                    lane,
                    self.libtest_output(lane),
                    b"",
                )
                self.assertGreater(oracle["passed"], 0)
                self.assertEqual(
                    oracle["tests"],
                    sorted(lanes.EXPECTED_TESTS[lane]),
                )
                total += oracle["passed"]
        self.assertEqual(total, 65)

    def test_mpv_lane_filtered_counts_share_reviewed_inventory_size(self) -> None:
        for lane in ("mpv-named-pipe", "mpv-external-process"):
            with self.subTest(lane=lane):
                self.assertEqual(
                    len(lanes.EXPECTED_TESTS[lane])
                    + lanes.EXPECTED_FILTERED_OUT[lane],
                    lanes.MPV_LIBTEST_INVENTORY_SIZE,
                )
        self.assertEqual(lanes.MPV_LIBTEST_INVENTORY_SIZE, 458)

    def test_libtest_oracle_requires_rust_singular_one_test_grammar(self) -> None:
        lane = "server-platform-signal"
        output = self.libtest_output(lane)
        self.assertIn(b"running 1 test\n", output)
        lanes.libtest_oracle(lane, output, b"")

        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "exactly one non-zero running count",
        ):
            lanes.libtest_oracle(
                lane,
                output.replace(b"running 1 test", b"running 1 tests"),
                b"",
            )

    def test_libtest_oracle_rejects_zero_partial_extra_and_skip(self) -> None:
        lane = "mpv-external-process"
        complete = self.libtest_output(lane)
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "exact test inventory drifted",
        ):
            lanes.libtest_oracle(
                lane,
                complete.replace(
                    lanes.EXPECTED_TESTS[lane][0].encode(),
                    b"tests::missing",
                ),
                b"",
            )
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "exact test inventory drifted",
        ):
            lanes.libtest_oracle(
                lane,
                complete.replace(
                    b"test result:",
                    b"test tests::unexpected ... ok\ntest result:",
                ),
                b"",
            )
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "executed zero tests",
        ):
            lanes.libtest_oracle(lane, b"running 0 tests\n", b"")
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "skipped-oracle markers",
        ):
            lanes.libtest_oracle(
                lane,
                complete,
                b"assertion skipped due to missing dependency",
            )

    def test_libtest_oracle_rejects_filtered_count_drift(self) -> None:
        lane = "media-tool-process"
        output = self.libtest_output(lane)
        filtered = lanes.EXPECTED_FILTERED_OUT[lane]
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "summary or filtered count drifted",
        ):
            lanes.libtest_oracle(
                lane,
                output.replace(
                    f"{filtered} filtered out".encode(),
                    f"{filtered + 1} filtered out".encode(),
                ),
                b"",
            )

    def test_complete_report_validates_without_mutation(self) -> None:
        report = self.valid_report()
        before = copy.deepcopy(report)
        self.assertIs(lanes.validate_report_document(report), report)
        self.assertEqual(report, before)

    def test_report_rejects_missing_profile_delta_and_discontinuity(self) -> None:
        report = self.valid_report()
        entry = report["lanes"]["mpv-named-pipe"]
        entry["profile_delta_count"] = 0
        entry["profile_deltas"] = []
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "produced no fresh instrumented profile",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["lanes"]["mpv-named-pipe"]["profile_count_before"] = 0
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "not continuous",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["lanes"]["merge-check"]["profile_delta_count"] = False
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "must be an integer",
        ):
            lanes.validate_report_document(report)

    def test_report_rejects_native_overclaim_and_cross_domain_profile(self) -> None:
        report = self.valid_report()
        report["coverage_boundary"]["interactive_native_included"] = True
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "overclaims native UI",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        delta = report["lanes"]["mpv-named-pipe"]["profile_deltas"][0]
        delta["path"] = "target/linux.profraw"
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "escaped the isolated target",
        ):
            lanes.validate_report_document(report)

    def test_report_rejects_command_or_exact_oracle_drift(self) -> None:
        report = self.valid_report()
        report["lanes"]["media-tool-process"]["command"] = ["cargo", "test"]
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "command drifted",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["lanes"]["mpv-external-process"]["oracle"]["passed"] = 0
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "exact required result",
        ):
            lanes.validate_report_document(report)

    def test_source_identity_binds_dirty_content_and_inventory(self) -> None:
        report = self.valid_report()
        source = report["source_identity"]
        source["untracked_file_count"] = 1
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "count does not match",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["source_identity"]["tracked_diff_sha256"] = "b" * 64
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "working-tree digest",
        ):
            lanes.validate_report_document(report)

    def test_strict_loader_rejects_failed_duplicate_or_unknown_report(
        self,
    ) -> None:
        report = self.valid_report()
        report["status"] = "failed"
        report["errors"] = ["lane failed"]
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "status is 'failed'",
        ):
            lanes.validate_report_document(report)

        report = self.valid_report()
        report["unreviewed"] = True
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "fields do not match schema",
        ):
            lanes.validate_report_document(report)

        path = self.root / "report.json"
        report = self.valid_report()
        raw = json.dumps(report).replace(
            f'"kind": "{lanes.REPORT_KIND}"',
            f'"kind": "{lanes.REPORT_KIND}", "kind": "{lanes.REPORT_KIND}"',
            1,
        )
        path.write_text(raw, encoding="utf-8")
        with self.assertRaisesRegex(
            common.CoverageProfileLaneError,
            "duplicate_key",
        ):
            lanes.strict_load_report(path)

    def test_lane_commands_cannot_invoke_interactive_native_smoke(self) -> None:
        joined = "\n".join(
            " ".join(command) for command in lanes.LANE_COMMANDS.values()
        ).lower()
        self.assertNotIn("gui-native-smoke", joined)
        self.assertNotIn("sorotte-gui-native-smoke", joined)
        for lane, command in lanes.LANE_COMMANDS.items():
            with self.subTest(lane=lane):
                self.assertIsInstance(command, tuple)
                self.assertEqual(command[0], "cargo")
                self.assertFalse(any("\n" in argument for argument in command))


if __name__ == "__main__":
    unittest.main()
