from __future__ import annotations

import pathlib
import sys
import tempfile
import textwrap
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts"))

import nextest_ci  # noqa: E402


VALID_CONFIG = """
[profile.ci]
retries = 1
flaky-result = "fail"
fail-fast = false
leak-timeout = { period = "500ms", result = "fail" }
status-level = "leak"
final-status-level = "fail"

[profile.ci.junit]
path = "junit.xml"
store-success-output = true
store-failure-output = true
flaky-fail-status = "failure"
"""


class NextestConfigPolicyTests(unittest.TestCase):
    def write_config(self, directory: pathlib.Path, contents: str) -> pathlib.Path:
        path = directory / "nextest.toml"
        path.write_text(textwrap.dedent(contents), encoding="utf-8")
        return path

    def test_repository_config_satisfies_fail_on_flaky_policy(self) -> None:
        nextest_ci.validate_config(REPO_ROOT / ".config" / "nextest.toml")

    def test_rejects_silently_green_flaky_results(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG.replace('flaky-result = "fail"', 'flaky-result = "pass"'),
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"flaky-result must be 'fail'",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_disabled_retries(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG.replace("retries = 1", "retries = 0"),
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"retries must be 1",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_junit_that_marks_flaky_failure_successful(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG.replace(
                    'flaky-fail-status = "failure"',
                    'flaky-fail-status = "success"',
                ),
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"flaky-fail-status must be 'failure'",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_toml_integer_disguised_as_boolean(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG.replace("fail-fast = false", "fail-fast = 0"),
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"fail-fast must be False",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_leak_timeout_that_allows_green_result(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG.replace(
                    'leak-timeout = { period = "500ms", result = "fail" }',
                    'leak-timeout = { period = "500ms", result = "pass" }',
                ),
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"leak-timeout.result must be 'fail'",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_hidden_leak_status_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG.replace(
                    'status-level = "leak"',
                    'status-level = "retry"',
                ),
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"status-level must be 'leak'",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_per_test_override_that_can_weaken_leak_policy(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG
                + """

                [[profile.ci.overrides]]
                filter = "all()"
                leak-timeout = { period = "60s", result = "pass" }
                """,
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"profile\.ci fields must be exactly.*unreviewed \['overrides'\]",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_ci_default_filter_that_silently_excludes_tests(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG.replace(
                    "fail-fast = false",
                    'fail-fast = false\ndefault-filter = "not test(updater)"',
                ),
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"profile\.ci fields must be exactly.*unreviewed \['default-filter'\]",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_inherited_default_profile_that_can_filter_ci(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_config(
                pathlib.Path(temporary),
                """
                [profile.default]
                default-filter = "not test(updater)"
                """
                + VALID_CONFIG,
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"profile fields must be exactly.*unreviewed \['default'\]",
            ):
                nextest_ci.validate_config(path)

    def test_rejects_unreviewed_root_and_junit_fields(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root_field = self.write_config(
                pathlib.Path(temporary),
                'nextest-version = "0.9.137"\n' + VALID_CONFIG,
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"nextest configuration fields must be exactly.*nextest-version",
            ):
                nextest_ci.validate_config(root_field)

            junit_field = self.write_config(
                pathlib.Path(temporary),
                VALID_CONFIG.replace(
                    'path = "junit.xml"',
                    'path = "junit.xml"\nreport-name = "weakened"',
                ),
            )
            with self.assertRaisesRegex(
                nextest_ci.PolicyError,
                r"profile\.ci\.junit fields must be exactly.*report-name",
            ):
                nextest_ci.validate_config(junit_field)

    def test_command_line_reasserts_policy_over_environment_and_overrides(self) -> None:
        self.assertEqual(nextest_ci.PINNED_NEXTEST_VERSION, "0.9.137")
        retry_index = nextest_ci.NEXTEST_COMMAND.index("--retries")
        self.assertEqual(nextest_ci.NEXTEST_COMMAND[retry_index + 1], "1")
        status_index = nextest_ci.NEXTEST_COMMAND.index("--status-level")
        self.assertEqual(nextest_ci.NEXTEST_COMMAND[status_index + 1], "leak")
        final_status_index = nextest_ci.NEXTEST_COMMAND.index(
            "--final-status-level"
        )
        self.assertEqual(
            nextest_ci.NEXTEST_COMMAND[final_status_index + 1],
            "fail",
        )
        self.assertIn("--no-fail-fast", nextest_ci.NEXTEST_COMMAND)
        self.assertEqual(
            nextest_ci.NEXTEST_COMMAND[-2:],
            ("--flaky-result", "fail"),
        )


class NextestJunitPolicyTests(unittest.TestCase):
    def write_junit(self, directory: pathlib.Path, contents: str) -> pathlib.Path:
        path = directory / "junit.xml"
        path.write_text(textwrap.dedent(contents), encoding="utf-8")
        return path

    def test_accepts_clean_successful_report(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_junit(
                pathlib.Path(temporary),
                """
                <testsuites tests="1" failures="0" errors="0">
                  <testsuite name="suite">
                    <testcase name="passes" />
                  </testsuite>
                </testsuites>
                """,
            )
            summary, violations = nextest_ci.assess_run(0, path)
        self.assertEqual(summary["testcases"], 1)
        self.assertEqual(violations, [])

    def test_pass_after_fail_is_rejected_even_if_producer_exits_zero(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_junit(
                pathlib.Path(temporary),
                """
                <testsuites tests="1" failures="1" errors="0">
                  <testsuite name="suite">
                    <testcase name="flaky">
                      <failure message="flaky test treated as failure" />
                      <flakyFailure message="first attempt failed" />
                      <system-out>first attempt and retry output</system-out>
                    </testcase>
                  </testsuite>
                </testsuites>
                """,
            )
            summary, violations = nextest_ci.assess_run(0, path)
        self.assertEqual(summary["elements"]["flakyFailure"], 1)
        self.assertTrue(any("pass-after-fail" in item for item in violations))
        self.assertTrue(any("final test failure" in item for item in violations))

    def test_leaked_process_error_is_rejected_even_if_producer_exits_zero(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_junit(
                pathlib.Path(temporary),
                """
                <testsuites tests="1" failures="0" errors="1">
                  <testsuite name="suite">
                    <testcase name="leaks">
                      <error
                        type="test exited with code 0, but leaked handles so was marked failed"
                      />
                    </testcase>
                  </testsuite>
                </testsuites>
                """,
            )
            summary, violations = nextest_ci.assess_run(0, path)
        self.assertEqual(summary["elements"]["error"], 1)
        self.assertTrue(any("final test failure" in item for item in violations))

    def test_nonzero_producer_is_rejected_even_with_clean_junit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_junit(
                pathlib.Path(temporary),
                "<testsuites><testsuite /></testsuites>",
            )
            _, violations = nextest_ci.assess_run(100, path)
        self.assertIn("nextest exited with status 100", violations)

    def test_zero_testcases_cannot_satisfy_required_workspace_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_junit(
                pathlib.Path(temporary),
                """
                <testsuites tests="0" failures="0" errors="0">
                  <testsuite name="empty" tests="0" />
                </testsuites>
                """,
            )
            summary, violations = nextest_ci.assess_run(0, path)
        self.assertEqual(summary["testcases"], 0)
        self.assertTrue(any("zero testcases" in item for item in violations))

    def test_missing_or_malformed_junit_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            _, missing = nextest_ci.assess_run(0, root / "missing.xml")
            malformed_path = self.write_junit(root, "<testsuites>")
            _, malformed = nextest_ci.assess_run(0, malformed_path)
        self.assertTrue(any("did not produce" in item for item in missing))
        self.assertTrue(any("malformed" in item for item in malformed))


if __name__ == "__main__":
    unittest.main()
