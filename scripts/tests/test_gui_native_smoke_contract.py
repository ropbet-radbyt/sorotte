from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))
import gui_native_smoke_contract as contract  # noqa: E402


class GuiNativeSmokeContractTests(unittest.TestCase):
    def complete_report(self, scenarios: tuple[str, ...] = ("baseline",)) -> dict:
        steps = list(contract.GLOBAL_REQUIRED_STEPS)
        for scenario in scenarios:
            steps.extend(contract.SCENARIO_REQUIRED_STEPS[scenario])
        capability_ids = list(contract.GLOBAL_REQUIRED_CAPABILITIES)
        for scenario in scenarios:
            capability_ids.extend(
                contract.SCENARIO_REQUIRED_CAPABILITIES.get(scenario, ())
            )
        capability_outcomes = []
        for capability_id in capability_ids:
            source, evidence = contract.CAPABILITY_CONTRACTS[capability_id]
            capability_outcomes.append(
                {
                    "capability_id": capability_id,
                    "outcome": "required-pass",
                    "source": source,
                    "evidence": list(evidence),
                }
            )
        return {
            "result": "ok",
            "input_mode": contract.STRICT_PHYSICAL_INPUT_MODE,
            "binary": r"C:\test\sorotte-gui.exe",
            "pid": 42,
            "window_title": "Sorotte",
            "menu_source": "uia-accesskit",
            "menu_labels": ["&File", "Playback", "Advanced", "Window", "Help"],
            "menu_automation_ids": [
                "menu.section.file",
                "menu.section.playback",
                "menu.section.advanced",
                "menu.section.window",
                "menu.section.help",
            ],
            "menu_contract": "verified",
            "accessible_name_count": 20,
            "accessibility_contract": "verified",
            "interaction_steps": steps,
            "interaction_contract": "verified",
            "capability_outcomes": capability_outcomes,
            "closed": True,
            "duration_ms": 1200,
        }

    def uia_only_report(self) -> dict:
        report = self.complete_report()
        report["input_mode"] = contract.UIA_ONLY_INPUT_MODE
        report["interaction_steps"] = list(contract.UIA_ONLY_REQUIRED_STEPS)
        report["interaction_contract"] = "local-uia-only-non-authoritative"
        report["capability_outcomes"] = [
            {
                "capability_id": capability_id,
                "outcome": outcome,
                "source": source,
                "evidence": list(evidence),
            }
            for capability_id, (
                outcome,
                source,
                evidence,
            ) in contract.UIA_ONLY_CAPABILITY_CONTRACTS.items()
        ]
        return report

    def validate(
        self,
        report: dict,
        scenarios: tuple[str, ...] = ("baseline",),
        stderr: str = "",
        input_mode: str = contract.STRICT_PHYSICAL_INPUT_MODE,
    ) -> None:
        contract.validate_native_smoke(
            json.dumps(report), stderr, scenarios, input_mode=input_mode
        )

    def prepare_binary(
        self, root: pathlib.Path, report: dict
    ) -> tuple[pathlib.Path, str]:
        binary = root / "sorotte-gui.exe"
        binary.write_bytes(b"exact native GUI fixture")
        report["binary"] = str(binary)
        return binary, hashlib.sha256(binary.read_bytes()).hexdigest()

    def provenance_cli_args(
        self, binary: pathlib.Path, digest: str, producer_exit_code: int = 0
    ) -> list[str]:
        return [
            "--expected-binary",
            str(binary),
            "--expected-binary-sha256",
            digest,
            "--producer-exit-code",
            str(producer_exit_code),
        ]

    def test_complete_required_report_passes(self) -> None:
        self.validate(self.complete_report())

    def test_complete_default_inventory_report_passes(self) -> None:
        report = self.complete_report(contract.DEFAULT_REQUIRED_SCENARIOS)
        self.validate(report, contract.DEFAULT_REQUIRED_SCENARIOS)

    def test_uia_only_local_report_passes_but_cannot_satisfy_strict_evidence(self) -> None:
        report = self.uia_only_report()
        self.validate(report, (), input_mode=contract.UIA_ONLY_INPUT_MODE)
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "input mode differs from the requested validator mode",
        ):
            self.validate(report)

    def test_uia_only_is_exact_and_rejects_strict_or_injected_input_evidence(self) -> None:
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "does not accept strict scenario evidence",
        ):
            self.validate(
                self.uia_only_report(),
                ("baseline",),
                input_mode=contract.UIA_ONLY_INPUT_MODE,
            )

        report = self.uia_only_report()
        next(
            outcome
            for outcome in report["capability_outcomes"]
            if outcome["capability_id"] == "native.menu.physical-input"
        )["outcome"] = "required-pass"
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "must have outcome 'optional-skip'",
        ):
            self.validate(report, (), input_mode=contract.UIA_ONLY_INPUT_MODE)

        report = self.uia_only_report()
        next(
            outcome
            for outcome in report["capability_outcomes"]
            if outcome["capability_id"] == "native.menu.physical-input"
        )["evidence"][2] = "desktop-input-attempt-count=1"
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "must have exact evidence",
        ):
            self.validate(report, (), input_mode=contract.UIA_ONLY_INPUT_MODE)

        report = self.uia_only_report()
        report["interaction_steps"].append("menu-input-stress-25")
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "exact local interaction inventory",
        ):
            self.validate(report, (), input_mode=contract.UIA_ONLY_INPUT_MODE)

    def test_default_inventory_covers_every_known_scenario(self) -> None:
        self.assertEqual(
            set(contract.DEFAULT_REQUIRED_SCENARIOS),
            set(contract.SCENARIO_REQUIRED_STEPS),
        )
        self.assertEqual(
            len(contract.DEFAULT_REQUIRED_SCENARIOS),
            len(contract.SCENARIO_REQUIRED_STEPS),
        )

    def test_empty_or_unknown_or_duplicate_scenario_fails(self) -> None:
        for scenarios in ((), ("not-a-scenario",), ("baseline", "baseline")):
            with self.subTest(scenarios=scenarios):
                with self.assertRaises(contract.NativeSmokeContractError):
                    self.validate(self.complete_report(), scenarios)

    def test_cli_rejects_unknown_scenario_without_report_execution(self) -> None:
        completed = subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "gui_native_smoke_contract.py"),
                "--check-scenarios",
                "--scenario",
                "baseline",
                "--scenario",
                "not-a-scenario",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(completed.returncode, 1)
        self.assertIn("unknown native scenarios", completed.stderr)

    def test_skipped_native_menu_fails_even_when_runner_says_ok(self) -> None:
        report = self.complete_report()
        report["menu_labels"] = []
        report["menu_contract"] = "skipped-no-native-menu"
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError, "menu contract must be required-pass"
        ):
            self.validate(report)

    def test_menu_inventory_requires_accesskit_source_and_exact_stable_ids(self) -> None:
        report = self.complete_report()
        report["menu_source"] = "win32-hmenu"
        report["menu_automation_ids"] = []
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "menu source must prove the UIA/AccessKit path",
        ):
            self.validate(report)

        report = self.complete_report()
        report["menu_automation_ids"][0] = "menu.section.files"
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "missing required automation IDs",
        ):
            self.validate(report)

        report = self.complete_report()
        report["menu_automation_ids"].append("menu.section.file")
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "duplicate automation IDs",
        ):
            self.validate(report)

    def test_any_required_skip_step_fails(self) -> None:
        report = self.complete_report()
        report["interaction_steps"].append(
            "open-media-file-skipped:no discovery method"
        )
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "required native capabilities were skipped",
        ):
            self.validate(report)

    def test_baseline_requires_disabled_open_media_evidence(self) -> None:
        report = self.complete_report()
        report["interaction_steps"].remove("open-media-file-detached-disabled")
        report["interaction_steps"].append(
            "open-media-file-detached-runtime-unavailable"
        )
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "missing completion step 'open-media-file-detached-disabled'",
        ):
            self.validate(report)

    def test_shortcut_only_does_not_substitute_for_menu_open_media(self) -> None:
        scenarios = ("menu-open-media",)
        report = self.complete_report(scenarios)
        report["interaction_steps"].remove(
            "menu-open-media-invoked-by-automation-id"
        )
        report["interaction_steps"].append("menu-open-media-invoked-by-ctrl-o")
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "missing completion step 'menu-open-media-invoked-by-automation-id'",
        ):
            self.validate(report, scenarios)

    def test_structured_capabilities_are_required_exact_and_fail_closed(self) -> None:
        scenarios = ("baseline", "menu-open-media")
        report = self.complete_report(scenarios)
        report["capability_outcomes"] = report["capability_outcomes"][1:]
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "missing required capability 'native.menu.inventory'",
        ):
            self.validate(report, scenarios)

        report = self.complete_report(scenarios)
        next(
            outcome
            for outcome in report["capability_outcomes"]
            if outcome["capability_id"] == "native.shutdown.file-exit"
        )["outcome"] = "skipped"
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "must have outcome 'required-pass'",
        ):
            self.validate(report, scenarios)

        report = self.complete_report(scenarios)
        next(
            outcome
            for outcome in report["capability_outcomes"]
            if outcome["capability_id"] == "native.menu.open-media.attached"
        )["source"] = "keyboard-shortcut"
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "must have source 'uia-accesskit\\+deterministic-test-player'",
        ):
            self.validate(report, scenarios)

        report = self.complete_report(scenarios)
        next(
            outcome
            for outcome in report["capability_outcomes"]
            if outcome["capability_id"] == "native.menu.physical-input"
        )["evidence"][1] = "menu-input-redelivered"
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "must have exact evidence",
        ):
            self.validate(report, scenarios)

        report = self.complete_report(scenarios)
        report["capability_outcomes"].append(
            {
                "capability_id": "native.menu.unreviewed",
                "outcome": "required-pass",
                "source": "uia-accesskit",
                "evidence": ["invented"],
            }
        )
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "unreviewed capability outcomes",
        ):
            self.validate(report, scenarios)

    def test_missing_scenario_completion_marker_fails(self) -> None:
        scenarios = ("transport",)
        report = self.complete_report(scenarios)
        report["interaction_steps"].remove("transport-saved-config-startup")
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError,
            "scenario 'transport' is missing completion step",
        ):
            self.validate(report, scenarios)

    def test_unexpected_stderr_fails(self) -> None:
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError, "unexpected stderr"
        ):
            self.validate(
                self.complete_report(),
                stderr="failed to lookup address information: syncplay.example\n",
            )

    def test_explicit_stderr_allowlist_is_line_scoped_and_full_match(self) -> None:
        report = self.complete_report()
        contract.validate_native_smoke(
            json.dumps(report),
            "known diagnostic 42\n",
            ("baseline",),
            allowed_stderr_patterns=(r"known diagnostic \d+",),
        )
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError, "unexpected stderr"
        ):
            contract.validate_native_smoke(
                json.dumps(report),
                "prefix known diagnostic 42 suffix\n",
                ("baseline",),
                allowed_stderr_patterns=(r"known diagnostic \d+",),
            )

    def test_false_integer_types_fail(self) -> None:
        for field in ("pid", "accessible_name_count", "duration_ms"):
            with self.subTest(field=field):
                report = self.complete_report()
                report[field] = True
                with self.assertRaises(contract.NativeSmokeContractError):
                    self.validate(report)

    def test_open_process_or_unreviewed_schema_fails(self) -> None:
        open_report = self.complete_report()
        open_report["closed"] = False
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError, "must be closed"
        ):
            self.validate(open_report)

        extended_report = self.complete_report()
        extended_report["silent_new_contract"] = "passed"
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError, "unreviewed keys"
        ):
            self.validate(extended_report)

    def test_error_or_concatenated_json_report_fails(self) -> None:
        with self.assertRaises(contract.NativeSmokeContractError):
            contract.validate_native_smoke(
                '{"result":"error","error":"failed"}', "", ("baseline",)
            )
        payload = json.dumps(self.complete_report())
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError, "one complete JSON document"
        ):
            contract.validate_native_smoke(
                f"{payload}\n{payload}", "", ("baseline",)
            )

    def test_duplicate_json_key_fails_closed(self) -> None:
        payload = json.dumps(self.complete_report())
        duplicated = payload.replace(
            '"result": "ok"',
            '"result": "error", "result": "ok"',
            1,
        )
        with self.assertRaisesRegex(
            contract.NativeSmokeContractError, "duplicate JSON key 'result'"
        ):
            contract.validate_native_smoke(duplicated, "", ("baseline",))

    def test_binary_path_hash_and_producer_exit_are_bound(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            report = self.complete_report()
            binary, digest = self.prepare_binary(root, report)
            payload = json.dumps(report)

            contract.validate_native_smoke(
                payload,
                "",
                ("baseline",),
                expected_binary=binary,
                expected_binary_sha256=digest,
                producer_exit_code=0,
            )

            with self.assertRaisesRegex(
                contract.NativeSmokeContractError, "producer exited with code 17"
            ):
                contract.validate_native_smoke(
                    payload,
                    "",
                    ("baseline",),
                    expected_binary=binary,
                    expected_binary_sha256=digest,
                    producer_exit_code=17,
                )

            other_binary = root / "other.exe"
            other_binary.write_bytes(binary.read_bytes())
            with self.assertRaisesRegex(
                contract.NativeSmokeContractError,
                "does not match the expected executable",
            ):
                contract.validate_native_smoke(
                    payload,
                    "",
                    ("baseline",),
                    expected_binary=other_binary,
                    expected_binary_sha256=digest,
                )

            with self.assertRaisesRegex(
                contract.NativeSmokeContractError, "SHA-256 changed"
            ):
                contract.validate_native_smoke(
                    payload,
                    "",
                    ("baseline",),
                    expected_binary=binary,
                    expected_binary_sha256="0" * 64,
                )

    def test_cli_always_writes_failure_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            report_path = root / "report.json"
            stderr_path = root / "stderr.log"
            summary_path = root / "summary.json"
            report = self.complete_report()
            report["menu_contract"] = "skipped-no-native-menu"
            binary, digest = self.prepare_binary(root, report)
            report_path.write_text(json.dumps(report), encoding="utf-8")
            stderr_path.write_text("", encoding="utf-8")

            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "gui_native_smoke_contract.py"),
                    "--report",
                    str(report_path),
                    "--stderr",
                    str(stderr_path),
                    "--summary",
                    str(summary_path),
                    "--scenario",
                    "baseline",
                ]
                + self.provenance_cli_args(binary, digest),
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.returncode, 1)
            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            self.assertEqual(summary["status"], "failure")
            self.assertEqual(summary["input_mode"], "strict-physical")
            self.assertTrue(summary["authoritative"])
            self.assertEqual(summary["required_scenarios"], ["baseline"])
            self.assertEqual(summary["producer_exit_code"], 0)
            self.assertEqual(summary["expected_binary_sha256"], digest)
            self.assertTrue(summary["errors"])

    def test_cli_nonzero_producer_cannot_write_required_pass_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            report_path = root / "report.json"
            stderr_path = root / "stderr.log"
            summary_path = root / "summary.json"
            report = self.complete_report()
            binary, digest = self.prepare_binary(root, report)
            report_path.write_text(json.dumps(report), encoding="utf-8")
            stderr_path.write_text("", encoding="utf-8")

            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "gui_native_smoke_contract.py"),
                    "--report",
                    str(report_path),
                    "--stderr",
                    str(stderr_path),
                    "--summary",
                    str(summary_path),
                    "--scenario",
                    "baseline",
                ]
                + self.provenance_cli_args(binary, digest, producer_exit_code=17),
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.returncode, 1)
            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            self.assertEqual(summary["status"], "failure")
            self.assertEqual(summary["producer_exit_code"], 17)
            self.assertIn(
                "native producer exited with code 17",
                summary["errors"],
            )

    def test_cli_accepts_utf16_powershell_redirection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            report_path = root / "report.json"
            stderr_path = root / "stderr.log"
            summary_path = root / "summary.json"
            report = self.complete_report()
            binary, digest = self.prepare_binary(root, report)
            report_path.write_text(
                json.dumps(report), encoding="utf-16"
            )
            stderr_path.write_text("", encoding="utf-16")

            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "gui_native_smoke_contract.py"),
                    "--report",
                    str(report_path),
                    "--stderr",
                    str(stderr_path),
                    "--summary",
                    str(summary_path),
                    "--scenario",
                    "baseline",
                ]
                + self.provenance_cli_args(binary, digest),
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            self.assertEqual(summary["status"], "required-pass")
            self.assertEqual(summary["input_mode"], "strict-physical")
            self.assertTrue(summary["authoritative"])
            self.assertEqual(summary["producer_exit_code"], 0)

    def test_cli_labels_uia_only_success_as_local_non_authoritative(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            report_path = root / "report.json"
            stderr_path = root / "stderr.log"
            summary_path = root / "summary.json"
            report = self.uia_only_report()
            binary, digest = self.prepare_binary(root, report)
            report_path.write_text(json.dumps(report), encoding="utf-8")
            stderr_path.write_text("", encoding="utf-8")

            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "gui_native_smoke_contract.py"),
                    "--input-mode",
                    "uia-only",
                    "--report",
                    str(report_path),
                    "--stderr",
                    str(stderr_path),
                    "--summary",
                    str(summary_path),
                ]
                + self.provenance_cli_args(binary, digest),
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            self.assertEqual(summary["status"], "local-pass")
            self.assertEqual(summary["input_mode"], "uia-only")
            self.assertFalse(summary["authoritative"])
            self.assertEqual(summary["required_scenarios"], [])

    def test_wrapper_is_fail_closed_and_uses_validator_inventory(self) -> None:
        wrapper = (SCRIPTS / "gui-native-smoke.ps1").read_text(encoding="utf-8")
        process_helper = (
            SCRIPTS / "gui-native-smoke-process.ps1"
        ).read_text(encoding="utf-8")
        self.assertIn("--print-default-scenarios", wrapper)
        self.assertIn('"--json"', wrapper)
        self.assertIn('[ValidateSet("StrictPhysical", "UiaOnly")]', wrapper)
        self.assertIn('"--input-mode", $inputModeArgument', wrapper)
        self.assertIn('"--input-mode", $inputModeArgument,', wrapper)
        self.assertIn('authoritative = $InputMode -eq "StrictPhysical"', wrapper)
        self.assertIn("native-report.json", wrapper)
        self.assertIn("native-stderr.log", wrapper)
        self.assertIn("contract-summary.json", wrapper)
        self.assertIn("build-stdout.log", wrapper)
        self.assertIn("build-stderr.log", wrapper)
        self.assertIn("harness-build-stdout.log", wrapper)
        self.assertIn("harness-build-stderr.log", wrapper)
        self.assertIn('"rebuilt-debug"', wrapper)
        self.assertIn('"--locked"', wrapper)
        self.assertIn('"--check-scenarios"', wrapper)
        self.assertIn("gui-native-smoke-process.ps1", wrapper)
        self.assertIn("[System.Diagnostics.ProcessStartInfo]::new()", process_helper)
        self.assertIn("$processStart.RedirectStandardError = $true", process_helper)
        self.assertIn("$process.WaitForExit($ProcessTimeoutMs)", process_helper)
        self.assertIn("/PID $($process.Id) /T /F", process_helper)
        self.assertIn("HARNESS_TIMEOUT:", process_helper)
        self.assertIn("exit_code = if ($timedOut) { 124 }", process_helper)
        self.assertIn("wall_clock_timeout_ms", wrapper)
        self.assertIn("timeout_grace_ms", wrapper)
        self.assertIn("SOROTTE_GUI_NATIVE_SMOKE_ARTIFACT_DIR", wrapper)
        self.assertIn("$processStart.EnvironmentVariables", process_helper)
        self.assertIn("-FilePath $nativeHarnessPath", wrapper)
        self.assertIn('"--expected-binary-sha256"', wrapper)
        self.assertIn('"--producer-exit-code"', wrapper)
        self.assertNotIn('"run",', wrapper)
        self.assertIn("if ($nativeExitCode -ne 0)", wrapper)
        self.assertIn("if ($validatorExitCode -ne 0)", wrapper)
        self.assertNotIn('"baseline"\n    $suiteArgs += "--scenario"', wrapper)

    def test_uia_only_sendinput_guard_is_central_and_fail_closed(self) -> None:
        native_bin_root = (
            ROOT
            / "crates"
            / "sorotte-gui"
            / "src"
            / "bin"
        )
        native_source_root = native_bin_root / "sorotte-gui-native-smoke"
        input_path = (
            native_source_root
            / "platform_driver"
            / "windows_input.rs"
        )
        input_source = input_path.read_text(encoding="utf-8")
        driver_source = (native_source_root / "platform_driver.rs").read_text(
            encoding="utf-8"
        )
        runner_source = (native_source_root / "native_smoke_runner.rs").read_text(
            encoding="utf-8"
        )
        control_source = (
            native_source_root
            / "platform_driver"
            / "windows_control_actions.rs"
        ).read_text(encoding="utf-8")

        native_rust_sources = [native_bin_root / "sorotte-gui-native-smoke.rs"]
        native_rust_sources.extend(sorted(native_source_root.rglob("*.rs")))
        sendinput_sites = [
            path.relative_to(ROOT).as_posix()
            for path in native_rust_sources
            for _ in re.finditer(r"\bSendInput\s*\(", path.read_text(encoding="utf-8"))
        ]
        self.assertEqual(
            sendinput_sites,
            [input_path.relative_to(ROOT).as_posix()],
        )
        guard_index = input_source.index("self.begin_desktop_input()?;")
        dispatch_index = input_source.index("SendInput(", guard_index)
        self.assertLess(guard_index, dispatch_index)
        cursor_guard_index = control_source.index("self.begin_desktop_input()")
        first_cursor_move_index = control_source.index("SetCursorPos(center_x, center_y)")
        self.assertLess(cursor_guard_index, first_cursor_move_index)
        self.assertIn("if self.input_mode == NativeInputMode::UiaOnly", driver_source)
        self.assertIn("desktop-wide Win32 input is disabled", driver_source)
        self.assertIn("if desktop_input_attempts != 0", runner_source)
        self.assertIn(
            '"desktop-input-attempt-count={desktop_input_attempts}"', runner_source
        )

    @unittest.skipUnless(os.name == "nt", "PowerShell watchdog is Windows-only")
    def test_process_watchdog_terminates_hung_process_and_persists_evidence(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            stdout_path = root / "stdout.log"
            stderr_path = root / "stderr.log"

            def quote(value: pathlib.Path | str) -> str:
                return str(value).replace("'", "''")

            command = (
                f". '{quote(SCRIPTS / 'gui-native-smoke-process.ps1')}'; "
                "$result = Invoke-CapturedProcess "
                "-FilePath (Get-Command powershell).Source "
                "-Arguments @('-NoProfile','-Command','Start-Sleep -Seconds 30') "
                f"-WorkingDirectory '{quote(root)}' "
                f"-StdoutPath '{quote(stdout_path)}' "
                f"-StderrPath '{quote(stderr_path)}' "
                "-ProcessTimeoutMs 200; "
                "$result | ConvertTo-Json -Compress"
            )
            completed = subprocess.run(
                ["powershell", "-NoProfile", "-Command", command],
                check=False,
                capture_output=True,
                text=True,
                timeout=20,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            result = json.loads(completed.stdout)
            self.assertEqual(result["exit_code"], 124)
            self.assertTrue(result["timed_out"])
            self.assertEqual(result["process_timeout_ms"], 200)
            self.assertLess(result["duration_ms"], 15000)
            timeout_evidence = stderr_path.read_text(encoding="utf-8")
            self.assertIn("HARNESS_TIMEOUT: process exceeded 200 ms", timeout_evidence)
            self.assertIn("tree_kill_exit=0", timeout_evidence)

    @unittest.skipUnless(os.name == "nt", "PowerShell process launch is Windows-only")
    def test_process_start_failure_is_structured_and_persists_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            stdout_path = root / "stdout.log"
            stderr_path = root / "stderr.log"

            def quote(value: pathlib.Path | str) -> str:
                return str(value).replace("'", "''")

            command = (
                f". '{quote(SCRIPTS / 'gui-native-smoke-process.ps1')}'; "
                "$result = Invoke-CapturedProcess "
                f"-FilePath '{quote(root / 'missing-process.exe')}' "
                "-Arguments @() "
                f"-WorkingDirectory '{quote(root)}' "
                f"-StdoutPath '{quote(stdout_path)}' "
                f"-StderrPath '{quote(stderr_path)}' "
                "-ProcessTimeoutMs 1000; "
                "$result | ConvertTo-Json -Compress"
            )
            completed = subprocess.run(
                ["powershell", "-NoProfile", "-Command", command],
                check=False,
                capture_output=True,
                text=True,
                timeout=20,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            result = json.loads(completed.stdout)
            self.assertEqual(result["exit_code"], 125)
            self.assertFalse(result["timed_out"])
            self.assertTrue(result["start_error"])
            self.assertEqual(stdout_path.read_text(encoding="utf-8"), "")
            self.assertIn(
                "HARNESS_START_FAILURE:",
                stderr_path.read_text(encoding="utf-8"),
            )

    @unittest.skipUnless(os.name == "nt", "PowerShell process launch is Windows-only")
    def test_process_wrapper_forwards_explicit_environment_variables(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            stdout_path = root / "stdout.log"
            stderr_path = root / "stderr.log"

            def quote(value: pathlib.Path | str) -> str:
                return str(value).replace("'", "''")

            command = (
                f". '{quote(SCRIPTS / 'gui-native-smoke-process.ps1')}'; "
                "$result = Invoke-CapturedProcess "
                "-FilePath (Get-Command powershell).Source "
                "-Arguments @('-NoProfile','-Command',"
                "'[Console]::Out.Write($env:SOROTTE_NATIVE_ENV_TEST)') "
                f"-WorkingDirectory '{quote(root)}' "
                f"-StdoutPath '{quote(stdout_path)}' "
                f"-StderrPath '{quote(stderr_path)}' "
                "-ProcessTimeoutMs 5000 "
                "-EnvironmentVariables @{ SOROTTE_NATIVE_ENV_TEST = 'forwarded' }; "
                "$result | ConvertTo-Json -Compress"
            )
            completed = subprocess.run(
                ["powershell", "-NoProfile", "-Command", command],
                check=False,
                capture_output=True,
                text=True,
                timeout=20,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            result = json.loads(completed.stdout)
            self.assertEqual(result["exit_code"], 0)
            self.assertEqual(stdout_path.read_text(encoding="utf-8"), "forwarded")
            self.assertEqual(stderr_path.read_text(encoding="utf-8"), "")


if __name__ == "__main__":
    unittest.main()
