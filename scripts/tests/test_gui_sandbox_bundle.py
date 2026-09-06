from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import uuid
import xml.etree.ElementTree as ET

SCRIPTS = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import gui_sandbox_bundle as bundle  # noqa: E402
from scripts.tests import test_gui_native_smoke_contract as native_tests  # noqa: E402


class SandboxBundleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.run = pathlib.Path(self.temporary.name)
        payload = self.run / "payload"
        (payload / "bin").mkdir(parents=True)
        (self.run / "output").mkdir()
        for name in ("sorotte-gui.exe", "sorotte-gui-native-smoke.exe"):
            (payload / "bin" / name).write_bytes(name.encode())
        self.manifest = {
            "schema_version": 1,
            "kind": "sorotte-windows-sandbox",
            "run_id": str(uuid.uuid4()),
            "source_sha": "a" * 40,
            "source_root": r"C:\source & build\sorotte",
            "input_mode": "strict-physical",
            "timeout_ms": 80000,
            "wall_clock_timeout_ms": 910000,
            "scenarios": list(bundle.contract.DEFAULT_REQUIRED_SCENARIOS),
            "files": bundle.payload_inventory(payload),
        }
        self.seal()

    def seal(self) -> None:
        path = self.run / "payload/manifest.json"
        bundle.write_json(path, self.manifest)
        digest = bundle.digest(path)
        (self.run / "manifest.sha256").write_text(digest + "\n")
        (self.run / "run.wsb").write_text(
            bundle.sandbox_xml(self.run, self.manifest["source_root"], digest),
            encoding="utf-8",
        )

    def complete_result(self) -> dict:
        hashes = {
            name: self.manifest["files"][f"bin/{name}"]
            for name in ("sorotte-gui.exe", "sorotte-gui-native-smoke.exe")
        }
        result = {
            "run_id": self.manifest["run_id"],
            "manifest_sha256": bundle.digest(self.run / "payload/manifest.json"),
            "status": "passed",
            "guest_preflight_passed": True,
            "validator_exit_code": 0,
            "binary_sha256_before": dict(hashes),
            "binary_sha256_after": dict(hashes),
            "runner": {"exit_code": 0, "timed_out": False, "start_error": None},
        }
        report = native_tests.GuiNativeSmokeContractTests().complete_report(
            bundle.contract.DEFAULT_REQUIRED_SCENARIOS
        )
        report["binary"] = str(pathlib.PureWindowsPath(bundle.GUEST_WORK) / "bin/sorotte-gui.exe")
        bundle.write_json(self.run / "output/native-report.json", report)
        (self.run / "output/native-stderr.log").write_text("")
        self.write_result(result)
        return result

    def write_result(self, result: dict) -> None:
        bundle.write_json(self.run / "output/completion.json", result)

    def test_config_maps_only_staged_payload_and_fresh_output(self) -> None:
        config = ET.fromstring((self.run / "run.wsb").read_text())
        for name in ("Networking", "ClipboardRedirection", "AudioInput", "VideoInput", "PrinterRedirection"):
            self.assertEqual(config.findtext(name), "Disable")
        mappings = config.findall("MappedFolders/MappedFolder")
        self.assertEqual(len(mappings), 3)
        self.assertEqual(
            [(item.findtext("HostFolder"), item.findtext("ReadOnly")) for item in mappings],
            [(str(self.run / "payload"), "true"), (str(self.run / "output"), "false"),
             (str(self.run / "payload/compat-probes"), "true")],
        )
        self.assertEqual(
            mappings[2].findtext("SandboxFolder"),
            r"C:\source & build\sorotte\crates\sorotte-compat\scripts",
        )
        self.assertIn("-ManifestSha256", config.findtext("LogonCommand/Command"))
        bundle.validate_payload(self.run)

    def test_rejects_modified_missing_or_extra_payload_files(self) -> None:
        binary = self.run / "payload/bin/sorotte-gui.exe"
        original = binary.read_bytes()
        binary.write_bytes(b"replaced binary")
        with self.assertRaisesRegex(ValueError, "inventory"):
            bundle.validate_payload(self.run)
        binary.unlink()
        with self.assertRaisesRegex(ValueError, "inventory"):
            bundle.validate_payload(self.run)
        binary.write_bytes(original)
        (self.run / "payload/unexpected.txt").write_text("extra")
        with self.assertRaisesRegex(ValueError, "inventory"):
            bundle.validate_payload(self.run)

    def test_constraint_is_staged_and_bound_to_source_and_payload(self) -> None:
        repo = self.run / "source"
        requirements = repo / "requirements"
        requirements.mkdir(parents=True)
        (requirements / "legacy-python-interop.txt").write_text(
            "-c verification-constraints.txt\ntwisted==26.4.0\n", encoding="utf-8"
        )
        constraint = requirements / "verification-constraints.txt"
        constraint.write_text("twisted==26.4.0\n", encoding="utf-8")
        paths = b"requirements/legacy-python-interop.txt\0requirements/verification-constraints.txt\0"
        with mock.patch.object(bundle, "git", side_effect=[paths, b"a" * 40, paths, b"a" * 40]):
            original_source = bundle.source_state(repo)
            staged = bundle.stage_python_requirements(repo, self.run / "payload")
            self.assertEqual(staged.read_bytes(), (requirements / staged.name).read_bytes())
            self.manifest["files"] = bundle.payload_inventory(self.run / "payload")
            self.seal()
            bundle.validate_payload(self.run)
            constraint.write_text("twisted==0.0.0\n", encoding="utf-8")
            self.assertNotEqual(original_source, bundle.source_state(repo))
        staged_constraint = self.run / "payload/verification-constraints.txt"
        staged_constraint.write_bytes(constraint.read_bytes())
        with self.assertRaisesRegex(ValueError, "inventory"):
            bundle.validate_payload(self.run)
        staged_constraint.unlink()
        with self.assertRaisesRegex(ValueError, "inventory"):
            bundle.validate_payload(self.run)

    def test_missing_constraints_fail_before_any_partial_requirements_copy(self) -> None:
        repo = self.run / "source"
        requirements = repo / "requirements"
        requirements.mkdir(parents=True)
        (requirements / "legacy-python-interop.txt").write_text("-c verification-constraints.txt\n")
        with self.assertRaisesRegex(ValueError, "verification-constraints.txt"):
            bundle.stage_python_requirements(repo, self.run / "payload")
        self.assertFalse((self.run / "payload/legacy-python-interop.txt").exists())

    def test_rejects_manifest_mutation_even_with_unchanged_inventory(self) -> None:
        self.manifest["source_sha"] = "b" * 40
        bundle.write_json(self.run / "payload/manifest.json", self.manifest)
        with self.assertRaisesRegex(ValueError, "manifest was changed"):
            bundle.validate_payload(self.run)

    def test_rejects_network_or_mapping_changes_before_launch(self) -> None:
        path = self.run / "run.wsb"
        original = path.read_text()
        for replacement in (
            original.replace("<Networking>Disable", "<Networking>Enable"),
            original.replace("<ReadOnly>true", "<ReadOnly>false", 1),
        ):
            with self.subTest(config=replacement):
                path.write_text(replacement, encoding="utf-8")
                with self.assertRaisesRegex(ValueError, "launch contract"):
                    bundle.validate_payload(self.run)

    def test_rejects_partial_scenarios_uia_mode_and_unbounded_timeout(self) -> None:
        for field, value in (("scenarios", ["baseline"]), ("input_mode", "uia-only"),
                             ("timeout_ms", 0), ("wall_clock_timeout_ms", 1)):
            with self.subTest(field=field):
                original = self.manifest[field]
                self.manifest[field] = value
                self.seal()
                with self.assertRaises(ValueError):
                    bundle.validate_payload(self.run)
                self.manifest[field] = original

    def test_complete_strict_result_is_local_evidence(self) -> None:
        self.complete_result()
        result = bundle.validate_result(self.run)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["source_sha"], "a" * 40)
        self.assertFalse(result["ci_attested"])

    def test_rejects_stale_completion_or_mismatched_payload_digest(self) -> None:
        for field, value in (("run_id", str(uuid.uuid4())), ("manifest_sha256", "0" * 64)):
            with self.subTest(field=field):
                result = self.complete_result()
                result[field] = value
                self.write_result(result)
                with self.assertRaisesRegex(ValueError, "different sandbox run"):
                    bundle.validate_result(self.run)

    def test_rejects_failed_preflight_validator_and_native_runner(self) -> None:
        for field, value in (("status", "failed"), ("guest_preflight_passed", False),
                             ("validator_exit_code", 1)):
            with self.subTest(field=field):
                result = self.complete_result()
                result[field] = value
                self.write_result(result)
                with self.assertRaises(ValueError):
                    bundle.validate_result(self.run)
        for field, value in (("exit_code", 1), ("timed_out", True), ("start_error", "launch failed")):
            with self.subTest(runner_field=field):
                result = self.complete_result()
                result["runner"][field] = value
                self.write_result(result)
                with self.assertRaises((ValueError, bundle.contract.NativeSmokeContractError)):
                    bundle.validate_result(self.run)

    def test_rejects_changed_gui_or_harness_before_or_after_execution(self) -> None:
        for field in ("binary_sha256_before", "binary_sha256_after"):
            for name in ("sorotte-gui.exe", "sorotte-gui-native-smoke.exe"):
                with self.subTest(field=field, binary=name):
                    result = self.complete_result()
                    result[field][name] = "0" * 64
                    self.write_result(result)
                    with self.assertRaisesRegex(ValueError, "executable changed"):
                        bundle.validate_result(self.run)

    def test_rejects_wrong_guest_executable_in_otherwise_valid_report(self) -> None:
        self.complete_result()
        path = self.run / "output/native-report.json"
        report = bundle.read_json(path)
        report["binary"] = r"C:\unrelated\sorotte-gui.exe"
        bundle.write_json(path, report)
        with self.assertRaisesRegex(ValueError, "unexpected guest executable"):
            bundle.validate_result(self.run)

    def test_accepts_rust_canonical_windows_path_for_the_verified_binary(self) -> None:
        self.complete_result()
        path = self.run / "output/native-report.json"
        report = bundle.read_json(path)
        report["binary"] = "\\\\?\\" + report["binary"]
        bundle.write_json(path, report)
        self.assertEqual(bundle.validate_result(self.run)["status"], "passed")

    def test_extended_prefix_does_not_allow_another_binary_or_device_path(self) -> None:
        for binary in (r"\\?\C:\unrelated\sorotte-gui.exe",
                       r"\\?\UNC\server\SorotteSandboxWork\bin\sorotte-gui.exe",
                       r"\\.\C:\SorotteSandboxWork\bin\sorotte-gui.exe",
                       r"\\?\C:\SorotteSandboxWork\bin\..\sorotte-gui.exe"):
            with self.subTest(binary=binary):
                self.complete_result()
                path = self.run / "output/native-report.json"
                report = bundle.read_json(path)
                report["binary"] = binary
                bundle.write_json(path, report)
                with self.assertRaisesRegex(ValueError, "unexpected guest executable"):
                    bundle.validate_result(self.run)

    def test_host_rechecks_strict_contract_despite_guest_success_claim(self) -> None:
        self.complete_result()
        path = self.run / "output/native-report.json"
        report = bundle.read_json(path)
        report["interaction_steps"] = []
        bundle.write_json(path, report)
        with self.assertRaises(bundle.contract.NativeSmokeContractError):
            bundle.validate_result(self.run)

    @unittest.skipUnless(sys.platform == "win32", "Windows guest host-refusal guard")
    def test_guest_entrypoint_refuses_host_before_starting_desktop_work(self) -> None:
        if os.environ.get("USERNAME", "").lower() == "wdagutilityaccount":
            self.skipTest("test must run on the host")
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(SCRIPTS / "gui-sandbox-guest.ps1"), "-ManifestSha256", "0" * 64],
            capture_output=True, text=True, timeout=30,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("runs only inside Windows Sandbox", result.stderr)

    @unittest.skipUnless(sys.platform == "win32", "Windows DPI virtualization regression")
    def test_display_preflight_uses_physical_pixels_and_restores_dpi_context(self) -> None:
        source = (SCRIPTS / "gui-sandbox-guest.ps1").read_text(encoding="utf-8")
        native_source = re.search(r"Add-Type -TypeDefinition @'\n(.*?)\n'@", source, re.S)
        self.assertIsNotNone(native_source)
        native_path = self.run / "display-native.cs"
        native_path.write_text(native_source.group(1), encoding="utf-8")
        test_script = self.run / "check-display.ps1"
        test_script.write_text(r'''
param([string]$NativeSource)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition (Get-Content -LiteralPath $NativeSource -Raw)
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[StructLayout(LayoutKind.Explicit, Size = 220, CharSet = CharSet.Unicode)]
public struct TestDisplayMode {
    [FieldOffset(68)] public ushort Size;
    [FieldOffset(172)] public uint Width;
    [FieldOffset(176)] public uint Height;
}
public static class TestDisplayReference {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern bool EnumDisplaySettingsW(string device, int mode, ref TestDisplayMode settings);
    [DllImport("user32.dll")]
    public static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")]
    public static extern bool AreDpiAwarenessContextsEqual(IntPtr first, IntPtr second);
    public static TestDisplayMode CurrentMode() {
        TestDisplayMode mode = new TestDisplayMode();
        mode.Size = 220;
        if (!EnumDisplaySettingsW(null, -1, ref mode)) {
            throw new InvalidOperationException("Could not read the current physical display mode.");
        }
        return mode;
    }
}
'@
# EnumDisplaySettings always reports physical pixels, independently of the
# caller's DPI context. Use it as the reference, not another metric conversion.
$mode = [TestDisplayReference]::CurrentMode()
$original = [SorotteSandboxDesktop]::SetThreadDpiAwarenessContext([IntPtr](-1))
if ($original -eq [IntPtr]::Zero) { throw 'Could not set the test caller to DPI-unaware.' }
try {
    $size = [SorotteSandboxDesktop]::PhysicalScreenSize()
    $restored = [TestDisplayReference]::AreDpiAwarenessContextsEqual(
        [TestDisplayReference]::GetThreadDpiAwarenessContext(), [IntPtr](-1))
    [pscustomobject]@{
        width = $size[0]; height = $size[1]
        expected_width = $mode.Width; expected_height = $mode.Height
        restored = $restored
    } | ConvertTo-Json -Compress
}
finally { $null = [SorotteSandboxDesktop]::SetThreadDpiAwarenessContext($original) }
''', encoding="utf-8")
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(test_script), "-NativeSource", str(native_path)],
            capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertGreater(report["width"], 0)
        self.assertGreater(report["height"], 0)
        self.assertEqual(report["width"], report["expected_width"])
        self.assertEqual(report["height"], report["expected_height"])
        self.assertTrue(report["restored"])


if __name__ == "__main__":
    unittest.main()
