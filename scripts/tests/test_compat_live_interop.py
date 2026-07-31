from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import pathlib
import sys
import tempfile
import types
import unittest
from collections.abc import Mapping
from unittest import mock

import yaml


ROOT = pathlib.Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "compat_live_interop.py"
SPEC = importlib.util.spec_from_file_location("compat_live_interop", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
interop = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = interop
SPEC.loader.exec_module(interop)


def command_result(
    command: tuple[str, ...],
    stdout: bytes = b"",
    stderr: bytes = b"",
    returncode: int = 0,
) -> interop.CommandResult:
    return interop.CommandResult(command, returncode, stdout, stderr, 0.125)


def complete_test_inventory() -> list[str]:
    tests = set(interop.EXPECTED_IGNORED_TESTS)
    tests.update(interop.REQUIRED_LIVE_SENTINELS)
    index = 0
    while len(tests) < interop.EXPECTED_DISCOVERED_TESTS:
        tests.add(f"tests::policy_fixture_{index:03d}")
        index += 1
    return sorted(tests)


def inventory_document(tests: list[str] | None = None) -> dict[str, object]:
    tests = complete_test_inventory() if tests is None else sorted(tests)
    ignored = sorted(interop.EXPECTED_IGNORED_TESTS)
    return {
        "listed_count": len(tests),
        "listed_tests": tests,
        "ignored_count": len(ignored),
        "ignored_tests": [
            {"test": name, "reason": interop.EXPECTED_IGNORED_TESTS[name]}
            for name in ignored
        ],
    }


def result_output(
    tests: list[str],
    *,
    skipped: tuple[str, str] | None = None,
    failed: str | None = None,
) -> bytes:
    lines = []
    for name in tests:
        if name in interop.EXPECTED_IGNORED_TESTS:
            status = f"ignored, {interop.EXPECTED_IGNORED_TESTS[name]}"
        elif name == failed:
            status = "FAILED"
        else:
            status = "ok"
        lines.append(f"test {name} ... {status}")
    if skipped is not None:
        test, reason = skipped
        lines.extend(["", "successes:", "", f"---- {test} stdout ----", reason])
    return ("\n".join(lines) + "\n").encode()


def file_identity(path: str, digest: str = "a" * 64) -> dict[str, object]:
    return {"path": path, "sha256": digest, "size_bytes": 1}


def valid_report() -> dict[str, object]:
    ignored = sorted(interop.EXPECTED_IGNORED_TESTS)
    tests = complete_test_inventory()
    executed = sorted(set(tests) - set(ignored))
    fixture_files = sorted(
        (
            file_identity(f"{root}/policy-{index:03d}.json")
            for root, count in interop.FIXTURE_ROOT_COUNTS.items()
            for index in range(count)
        ),
        key=lambda item: item["path"],
    )
    fixture_manifest = hashlib.sha256()
    for item in fixture_files:
        fixture_manifest.update(item["path"].encode("utf-8"))
        fixture_manifest.update(b"\0")
        fixture_manifest.update(item["sha256"].encode("ascii"))
        fixture_manifest.update(b"\0")
        fixture_manifest.update(str(item["size_bytes"]).encode("ascii"))
        fixture_manifest.update(b"\n")
    return {
        "schema_version": interop.SCHEMA_VERSION,
        "kind": interop.REPORT_KIND,
        "mode": "required",
        "status": "passed",
        "source": {
            "commit_sha": "1" * 40,
            "expected_commit_sha": "1" * 40,
        },
        "oracle": {
            "path": ".interop-cache/syncplay-legacy",
            "repository": interop.PINNED_LEGACY_SYNCPLAY_REPOSITORY,
            "expected_commit_sha": interop.PINNED_LEGACY_SYNCPLAY_SHA,
            "observed_commit_sha": interop.PINNED_LEGACY_SYNCPLAY_SHA,
        },
        "prerequisites": {
            "python": {
                "command": "python",
                "executable": "/python",
                "implementation": "CPython",
                "version": "3.11.13",
                "version_info": [3, 11, 13],
                "supported_family": ">=3.11,<3.14",
                "packages": [
                    {
                        "name": display,
                        "expected_version": version,
                        "observed_version": version,
                    }
                    for _, (display, version) in sorted(
                        interop.PINNED_PACKAGES.items()
                    )
                ],
            },
            "requirements": {
                "path": "requirements/legacy-python-interop.txt",
                "sha256": "b" * 64,
                "packages": [
                    {"name": display, "version": version}
                    for _, (display, version) in sorted(
                        interop.PINNED_PACKAGES.items()
                    )
                ],
            },
            "probes": [
                file_identity(path, "c" * 64) for path in interop.PROBE_PATHS
            ],
        },
        "fixtures": {
            "roots": list(interop.FIXTURE_ROOT_COUNTS),
            "counts": dict(interop.FIXTURE_ROOT_COUNTS),
            "file_count": len(fixture_files),
            "manifest_sha256": fixture_manifest.hexdigest(),
            "files": fixture_files,
        },
        "inventory": {
            "listed_count": len(tests),
            "listed_tests": tests,
            "ignored_count": len(ignored),
            "ignored_tests": [
                {"test": name, "reason": interop.EXPECTED_IGNORED_TESTS[name]}
                for name in ignored
            ],
        },
        "accounting": {
            "complete": True,
            "executed_count": len(executed),
            "passed_count": len(executed),
            "failed_count": 0,
            "skipped_count": 0,
            "ignored_count": len(ignored),
            "executed_tests": executed,
            "failed_tests": [],
            "skipped": [],
        },
        "execution": {
            "command": list(interop.TEST_COMMAND),
            "returncode": 0,
            "duration_seconds": 1.25,
            "stdout": file_identity(
                "target/verification/compat-live-interop.stdout.log", "e" * 64
            ),
            "stderr": file_identity(
                "target/verification/compat-live-interop.stderr.log", "f" * 64
            ),
        },
        "errors": [],
    }


class RequirementPolicyTests(unittest.TestCase):
    def test_exact_requirement_pins_are_accepted(self) -> None:
        parsed = interop.parse_pinned_requirements(
            b"twisted==25.5.0\n"
            b"# compatibility comment\n"
            b"pyopenssl==25.3.0\n"
            b"service_identity==24.2.0\n"
        )
        self.assertEqual(parsed, interop.PINNED_PACKAGES)

    def test_ranges_duplicates_and_inventory_drift_are_rejected(self) -> None:
        mutations = (
            b"twisted>=25.5.0\npyopenssl==25.3.0\nservice_identity==24.2.0\n",
            b"twisted==25.5.0\nTwisted==25.5.0\npyopenssl==25.3.0\nservice_identity==24.2.0\n",
            b"twisted==25.5.0\npyopenssl==25.3.0\n",
        )
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(
                interop.InteropContractError
            ):
                interop.parse_pinned_requirements(mutation)

    def test_python_identity_is_exact_and_missing_process_or_package_fails_closed(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            requirements = root / "requirements"
            requirements.mkdir()
            requirements.joinpath("legacy-python-interop.txt").write_text(
                "twisted==25.5.0\n"
                "pyopenssl==25.3.0\n"
                "service_identity==24.2.0\n",
                encoding="utf-8",
            )
            identity = {
                "executable": "C:/Python313/python.exe",
                "implementation": "CPython",
                "packages": {
                    "pyopenssl": "25.3.0",
                    "service-identity": "24.2.0",
                    "twisted": "25.5.0",
                },
                "version": "3.13.5",
                "version_info": [3, 13, 5],
            }
            completed = types.SimpleNamespace(
                returncode=0,
                stdout=json.dumps(identity).encode(),
                stderr=b"",
            )
            with mock.patch.object(
                interop.subprocess, "run", return_value=completed
            ):
                python, pinned = interop.verify_python(
                    root, {"SYNCPLAY_PYTHON_BIN": "python"}
                )
            self.assertEqual(python["version"], "3.13.5")
            self.assertEqual(
                [package["observed_version"] for package in python["packages"]],
                ["25.3.0", "24.2.0", "25.5.0"],
            )
            self.assertRegex(pinned["sha256"], r"^[0-9a-f]{64}$")

            with mock.patch.object(
                interop.subprocess,
                "run",
                side_effect=FileNotFoundError("missing"),
            ), self.assertRaisesRegex(
                interop.PrerequisiteUnavailable, "interpreter is unavailable"
            ) as missing_python:
                interop.verify_python(
                    root, {"SYNCPLAY_PYTHON_BIN": "missing-python"}
                )
            self.assertEqual(missing_python.exception.code, "missing-python")

            identity["packages"]["twisted"] = None
            missing_package = types.SimpleNamespace(
                returncode=0,
                stdout=json.dumps(identity).encode(),
                stderr=b"",
            )
            with mock.patch.object(
                interop.subprocess, "run", return_value=missing_package
            ), self.assertRaisesRegex(
                interop.PrerequisiteUnavailable, "twisted is not installed"
            ) as missing_dependency:
                interop.verify_python(
                    root, {"SYNCPLAY_PYTHON_BIN": "python"}
                )
            self.assertEqual(
                missing_dependency.exception.code, "missing-python-package"
            )

    def test_missing_fixture_and_wrong_oracle_revision_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw)
            with self.assertRaises(interop.PrerequisiteUnavailable) as missing:
                interop.file_record(root, root / "fixtures" / "missing.json")
            self.assertEqual(missing.exception.code, "missing-fixture")

            oracle = root / ".interop-cache" / "syncplay-legacy"
            oracle.mkdir(parents=True)
            oracle.joinpath("syncplayServer.py").write_text("", encoding="utf-8")
            with mock.patch.object(
                interop, "git_text", return_value="2" * 40
            ), self.assertRaisesRegex(
                interop.InteropContractError, "must be pinned"
            ):
                interop.verify_oracle(
                    root, {"SYNCPLAY_LEGACY_ROOT": str(oracle)}
                )


class InventoryAndAccountingTests(unittest.TestCase):
    def test_generated_json_framing_differential_is_required_and_counted(
        self,
    ) -> None:
        self.assertIn(
            (
                "tests::python_protocol_tests::"
                "generated_json_framing_matches_pinned_python_oracle"
            ),
            interop.REQUIRED_LIVE_SENTINELS,
        )
        self.assertEqual(interop.EXPECTED_DISCOVERED_TESTS, 148)

    def test_complete_and_ignored_inventories_are_exact(self) -> None:
        tests = complete_test_inventory()
        all_output = "".join(f"{name}: test\n" for name in tests).encode()
        ignored_output = "".join(
            f"{name}: test\n" for name in sorted(interop.EXPECTED_IGNORED_TESTS)
        ).encode()
        inventory = interop.verify_inventory(
            command_result(interop.LIST_COMMAND, all_output),
            command_result(interop.IGNORED_LIST_COMMAND, ignored_output),
        )
        self.assertEqual(inventory["listed_count"], len(tests))
        self.assertEqual(
            [item["test"] for item in inventory["ignored_tests"]],
            sorted(interop.EXPECTED_IGNORED_TESTS),
        )

    def test_complete_inventory_rejects_count_drift_in_either_direction(
        self,
    ) -> None:
        tests = complete_test_inventory()
        ignored_output = "".join(
            f"{name}: test\n" for name in sorted(interop.EXPECTED_IGNORED_TESTS)
        ).encode()
        for drifted in (tests[:-1], [*tests, "tests::unexpected_extra_test"]):
            all_output = "".join(
                f"{name}: test\n" for name in sorted(drifted)
            ).encode()
            with self.subTest(discovered=len(drifted)), self.assertRaisesRegex(
                interop.InteropContractError,
                "differs from the source-bound expectation",
            ):
                interop.verify_inventory(
                    command_result(interop.LIST_COMMAND, all_output),
                    command_result(
                        interop.IGNORED_LIST_COMMAND,
                        ignored_output,
                    ),
                )

    def test_partial_or_unexpected_ignored_inventory_is_rejected(self) -> None:
        tests = complete_test_inventory()
        all_output = "".join(f"{name}: test\n" for name in tests).encode()
        partial = sorted(interop.EXPECTED_IGNORED_TESTS)[:-1]
        ignored_output = "".join(f"{name}: test\n" for name in partial).encode()
        with self.assertRaisesRegex(
            interop.InteropContractError, "ignored compatibility inventory"
        ):
            interop.verify_inventory(
                command_result(interop.LIST_COMMAND, all_output),
                command_result(interop.IGNORED_LIST_COMMAND, ignored_output),
            )

    def test_required_mode_accounts_complete_success(self) -> None:
        inventory = inventory_document()
        tests = inventory["listed_tests"]
        result = command_result(
            interop.TEST_COMMAND,
            result_output(tests),
        )
        accounting = interop.account_execution(inventory, result)
        self.assertEqual(accounting["skipped_count"], 0)
        self.assertEqual(
            accounting["executed_count"],
            len(tests) - len(interop.EXPECTED_IGNORED_TESTS),
        )
        self.assertEqual(
            interop.execution_failures(accounting, result, required=True), []
        )

    def test_mixed_failure_accounting_remains_sorted_for_closed_schema(self) -> None:
        inventory = inventory_document()
        tests = inventory["listed_tests"]
        failed = next(
            name
            for name in tests
            if name not in interop.EXPECTED_IGNORED_TESTS
        )
        result = command_result(
            interop.TEST_COMMAND,
            result_output(tests, failed=failed),
            returncode=101,
        )

        accounting = interop.account_execution(inventory, result)

        self.assertEqual(
            accounting["executed_tests"],
            sorted(accounting["executed_tests"]),
        )
        self.assertEqual(accounting["failed_tests"], [failed])
        self.assertRegex(
            interop.execution_failures(accounting, result, required=True)[0],
            "exited with code 101",
        )

    def test_skip_is_structured_optional_and_fails_required(self) -> None:
        inventory = inventory_document()
        tests = inventory["listed_tests"]
        skipped_test = sorted(interop.REQUIRED_LIVE_SENTINELS)[0]
        reason = (
            "python fanout interop test skipped due to missing local prerequisites"
        )
        result = command_result(
            interop.TEST_COMMAND,
            result_output(tests, skipped=(skipped_test, reason)),
        )
        accounting = interop.account_execution(inventory, result)
        self.assertEqual(
            accounting["skipped"],
            [
                {
                    "scope": "test",
                    "test": skipped_test,
                    "code": "missing-local-prerequisite",
                    "reason": reason,
                }
            ],
        )
        self.assertEqual(
            interop.execution_failures(accounting, result, required=False), []
        )
        self.assertRegex(
            interop.execution_failures(accounting, result, required=True)[0],
            "optional skip paths",
        )

    def test_unknown_skip_vocabulary_and_partial_execution_fail_closed(self) -> None:
        inventory = inventory_document()
        tests = inventory["listed_tests"]
        selected = sorted(interop.REQUIRED_LIVE_SENTINELS)[0]
        unknown = result_output(
            tests,
            skipped=(selected, "compatibility test skipped for surprising reason"),
        )
        with self.assertRaisesRegex(
            interop.InteropContractError, "unclassified skip reason"
        ):
            interop.account_execution(
                inventory, command_result(interop.TEST_COMMAND, unknown)
            )

        partial = result_output(tests[:-1])
        with self.assertRaisesRegex(
            interop.InteropContractError, "differs from the complete listing"
        ):
            interop.account_execution(
                inventory, command_result(interop.TEST_COMMAND, partial)
            )

    def test_closed_skip_vocabulary_covers_disabled_tls_and_missing_processes(
        self,
    ) -> None:
        cases = {
            (
                "legacy server TLS parity assertion skipped; set "
                "SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            ): "assertion-disabled",
            (
                "legacy live TLS roundtrip test skipped due to missing "
                "prerequisites: tls support is not enabled"
            ): "missing-prerequisite",
            (
                "python fanout interop test skipped due to missing local "
                "prerequisites"
            ): "missing-local-prerequisite",
        }
        for reason, expected_code in cases.items():
            with self.subTest(reason=reason):
                self.assertEqual(
                    interop.classify_skip_reason(reason),
                    (expected_code, reason),
                )


class ClosedSchemaTests(unittest.TestCase):
    def test_valid_closed_schema_report_is_accepted(self) -> None:
        report = valid_report()
        self.assertIs(interop.validate_report_document(report), report)

    def test_extra_missing_duplicate_and_contradictory_fields_fail(self) -> None:
        mutations = []
        extra = valid_report()
        extra["unexpected"] = True
        mutations.append(extra)
        missing = valid_report()
        del missing["accounting"]["ignored_count"]
        mutations.append(missing)
        contradictory = valid_report()
        contradictory["accounting"]["executed_count"] = 2
        mutations.append(contradictory)
        selector = valid_report()
        selector["execution"]["command"].insert(6, "legacy_server_")
        mutations.append(selector)
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(
                interop.InteropContractError
            ):
                interop.validate_report_document(mutation)

        duplicate = json.dumps(valid_report()).replace(
            '"schema_version": 1',
            '"schema_version": 1, "schema_version": 1',
            1,
        )
        with self.assertRaisesRegex(
            interop.InteropContractError, "duplicate key"
        ):
            interop.strict_parse_json(
                duplicate.encode(), label="duplicate report"
            )

    def test_persisted_report_requires_exact_inventory_and_live_sentinels(
        self,
    ) -> None:
        truncated = valid_report()
        removed = truncated["inventory"]["listed_tests"].pop()
        truncated["inventory"]["listed_count"] -= 1
        if removed in truncated["accounting"]["executed_tests"]:
            truncated["accounting"]["executed_tests"].remove(removed)
            truncated["accounting"]["executed_count"] -= 1
            truncated["accounting"]["passed_count"] -= 1
        with self.assertRaisesRegex(
            interop.InteropContractError,
            "source-bound expectation",
        ):
            interop.validate_report_document(truncated)

        missing_sentinel = valid_report()
        sentinel = sorted(interop.REQUIRED_LIVE_SENTINELS)[0]
        sentinel_index = missing_sentinel["inventory"]["listed_tests"].index(
            sentinel
        )
        replacement = "tests::replacement_non_sentinel"
        missing_sentinel["inventory"]["listed_tests"][sentinel_index] = replacement
        missing_sentinel["inventory"]["listed_tests"].sort()
        missing_sentinel["accounting"]["executed_tests"].remove(sentinel)
        missing_sentinel["accounting"]["executed_tests"].append(replacement)
        missing_sentinel["accounting"]["executed_tests"].sort()
        with self.assertRaisesRegex(
            interop.InteropContractError,
            "omits required live sentinels",
        ):
            interop.validate_report_document(missing_sentinel)

    def test_required_pass_requires_complete_success_evidence(self) -> None:
        missing_state = valid_report()
        missing_state["inventory"] = None
        missing_state["execution"] = None
        missing_state["accounting"] = copy.deepcopy(
            interop.failed_report(mode="required")["accounting"]
        )
        with self.assertRaisesRegex(
            interop.InteropContractError,
            "omits successful execution evidence",
        ):
            interop.validate_report_document(missing_state)

        for field in ("source", "oracle", "prerequisites", "fixtures"):
            report = valid_report()
            report[field] = None
            with self.subTest(field=field), self.assertRaisesRegex(
                interop.InteropContractError,
                "omits successful execution evidence",
            ):
                interop.validate_report_document(report)

        nonzero = valid_report()
        nonzero["execution"]["returncode"] = 101
        with self.assertRaisesRegex(
            interop.InteropContractError,
            "zero execution return code",
        ):
            interop.validate_report_document(nonzero)

    def test_required_pass_cannot_contain_a_skip(self) -> None:
        report = valid_report()
        skipped_test = report["accounting"]["executed_tests"].pop()
        report["accounting"]["executed_count"] -= 1
        report["accounting"]["passed_count"] -= 1
        report["accounting"]["skipped_count"] = 1
        report["accounting"]["skipped"] = [
            {
                "scope": "test",
                "test": skipped_test,
                "code": "missing-prerequisite",
                "reason": "required assertion skipped due to missing prerequisites",
            }
        ]
        with self.assertRaisesRegex(
            interop.InteropContractError, "required passed report"
        ):
            interop.validate_report_document(report)


class PreflightModeTests(unittest.TestCase):
    def test_execution_uses_absolute_attested_oracle_path(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw).resolve()
            oracle = {
                "path": ".interop-cache/syncplay-legacy",
            }
            python = {
                "executable": "C:/Python313/python.exe",
            }

            environment = interop.build_execution_environment(
                repo_root=root,
                environment={
                    "SYNCPLAY_LEGACY_ROOT": ".interop-cache/syncplay-legacy",
                },
                oracle=oracle,
                python_record=python,
                required=True,
            )

        self.assertEqual(
            pathlib.Path(environment["SYNCPLAY_LEGACY_ROOT"]),
            root / ".interop-cache" / "syncplay-legacy",
        )
        self.assertEqual(
            environment["SYNCPLAY_PYTHON_BIN"],
            "C:/Python313/python.exe",
        )
        self.assertEqual(environment[interop.REQUIRED_ENVIRONMENT_VARIABLE], "1")
        self.assertEqual(environment["SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY"], "1")
        self.assertEqual(environment["SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY"], "1")

    def test_missing_oracle_is_optional_only_outside_required_mode(self) -> None:
        source = {
            "commit_sha": "1" * 40,
            "expected_commit_sha": "1" * 40,
        }
        for required, expected_code, expected_status in (
            (False, 0, "passed"),
            (True, 1, "failed"),
        ):
            with self.subTest(required=required), tempfile.TemporaryDirectory() as raw:
                root = pathlib.Path(raw)
                (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
                output = root / "report.json"
                environment = (
                    {interop.REQUIRED_ENVIRONMENT_VARIABLE: "1"} if required else {}
                )
                with mock.patch.object(
                    interop, "verify_source", return_value=source
                ), mock.patch.object(
                    interop,
                    "verify_oracle",
                    side_effect=interop.PrerequisiteUnavailable(
                        "missing-oracle-root",
                        "SYNCPLAY_LEGACY_ROOT does not identify the pinned local oracle",
                    ),
                ):
                    code, report = interop.collect_report(
                        repo_root=root,
                        output=output,
                        environment=environment,
                    )
                self.assertEqual(code, expected_code)
                self.assertEqual(report["status"], expected_status)
                self.assertEqual(report["accounting"]["skipped_count"], 1)
                self.assertEqual(
                    report["accounting"]["skipped"][0]["code"],
                    "missing-oracle-root",
                )
                self.assertTrue(output.is_file())


class WorkflowPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = yaml.safe_load(
            (ROOT / ".github" / "workflows" / "rust-ci.yml").read_text(
                encoding="utf-8"
            )
        )

    @staticmethod
    def named_step(job: Mapping[str, object], name: str) -> Mapping[str, object]:
        matches = [step for step in job["steps"] if step.get("name") == name]
        if len(matches) != 1:
            raise AssertionError(f"expected exactly one step named {name!r}")
        return matches[0]

    def assert_required_job(self, job_name: str, *, nightly: bool) -> None:
        job = self.workflow["jobs"][job_name]
        checkout = self.named_step(job, "Checkout pinned legacy reference")
        self.assertEqual(
            checkout["with"],
            {
                "repository": "Syncplay/syncplay",
                "ref": interop.PINNED_LEGACY_SYNCPLAY_SHA,
                "path": ".interop-cache/syncplay-legacy",
                "persist-credentials": False,
            },
        )
        setup_python = self.named_step(job, "Setup Python")
        self.assertEqual(setup_python["with"]["python-version"], "3.11")
        install = self.named_step(job, "Install pinned live Python prerequisites")
        self.assertEqual(
            " ".join(install["run"].split()),
            "python -m pip install --disable-pip-version-check "
            "-r requirements/legacy-python-interop.txt",
        )
        run = self.named_step(job, "Strict complete live Python compatibility")
        self.assertEqual(
            run.get("env"),
            {interop.REQUIRED_ENVIRONMENT_VARIABLE: "1"},
        )
        self.assertEqual(
            " ".join(run["run"].split()),
            "python scripts/compat_live_interop.py run --repo-root . "
            "--output target/verification/compat-live-interop.json",
        )
        upload_name = (
            "Upload nightly live compatibility evidence"
            if nightly
            else "Upload live compatibility evidence"
        )
        upload = self.named_step(job, upload_name)
        self.assertEqual(upload.get("if"), "always()")
        self.assertEqual(upload["with"]["if-no-files-found"], "error")
        self.assertIn(
            "target/verification/compat-live-interop.json",
            upload["with"]["path"],
        )

    def test_pr_and_nightly_jobs_own_the_complete_required_contract(self) -> None:
        self.assert_required_job("compat-live-tls", nightly=False)
        self.assert_required_job("nightly-deep", nightly=True)

    def test_runner_commands_are_selector_free_and_coverage_is_fail_closed(self) -> None:
        self.assertEqual(interop.TEST_COMMAND[:6], interop.BASE_CARGO_COMMAND)
        self.assertNotIn("legacy_server_", interop.TEST_COMMAND)
        coverage = (
            ROOT / "scripts" / "coverage_profile_lanes.py"
        ).read_text(encoding="utf-8")
        self.assertIn('"SYNCPLAY_REQUIRE_LIVE_INTEROP": "1"', coverage)
        rust = (
            ROOT / "crates" / "sorotte-compat" / "src" / "legacy_process.rs"
        ).read_text(encoding="utf-8")
        self.assertIn("SYNCPLAY_REQUIRE_LIVE_INTEROP", rust)
        self.assertIn(
            "if required_live_interop_enabled() {\n        return false;\n    }",
            rust,
        )


if __name__ == "__main__":
    unittest.main()
