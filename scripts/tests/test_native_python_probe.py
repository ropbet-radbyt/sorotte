"""Real isolated Python and Windows guest readiness, without guest/network access."""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
PROBE = ROOT / "scripts/native_python_probe.py"


class NativePythonProbeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.root = Path(cls.temporary.name).resolve()
        cls.runtime = cls.root / "cache/Python/test/x64"
        cls.empty = cls.root / "no-pip"
        for path, extra in ((cls.runtime, []), (cls.empty, ["--without-pip"])):
            subprocess.run([sys.executable, "-I", "-B", "-m", "venv", *extra, str(path)],
                           check=True, capture_output=True, timeout=60)
        cls.python = cls.runtime / ("Scripts/python.exe" if sys.platform == "win32" else "bin/python")
        cls.empty_python = cls.empty / ("Scripts/python.exe" if sys.platform == "win32" else "bin/python")
        details = subprocess.run([str(cls.python), "-I", "-B", "-c",
                                  'import importlib.metadata,json,sys,sysconfig;print(json.dumps({"version":".".join(map(str,sys.version_info[:3])),"pip":importlib.metadata.version("pip"),"packages":sysconfig.get_path("purelib")}))'],
                                 check=True, capture_output=True, text=True, timeout=15)
        details = json.loads(details.stdout)
        cls.packages = Path(details["packages"])
        module = cls.packages / "native_probe_namespace/part/__init__.py"
        module.parent.mkdir(parents=True)
        module.write_text("READY = True\n", encoding="utf-8")
        metadata = cls.packages / "native_probe_fixture-1.0.dist-info"
        metadata.mkdir()
        (metadata / "METADATA").write_text("Metadata-Version: 2.1\nName: native-probe-fixture\nVersion: 1.0\n", encoding="utf-8")
        (cls.packages / "native_probe_fixture-nspkg.pth").write_text("# namespace fixture; never copied into a runtime\n", encoding="utf-8")
        (metadata / "RECORD").write_text("native_probe_namespace/part/__init__.py,,\nnative_probe_fixture-nspkg.pth,,\nnative_probe_fixture-1.0.dist-info/METADATA,,\nnative_probe_fixture-1.0.dist-info/RECORD,,\n", encoding="utf-8")
        cls.contract = {"schema_version": 1, "kind": "sorotte-native-python-contract",
                        "python_version": details["version"],
                        "requirements": {"pip": details["pip"], "native-probe-fixture": "1.0"},
                        "constraints": {"pip": details["pip"], "native-probe-fixture": "1.0"},
                        "imports": ["unittest", "pip._internal.cli.main", "native_probe_namespace.part"]}

    def invoke(self, *, python=None, contract=None, extra=(), env=None):
        return subprocess.run([str(python or self.python), "-I", "-B", str(PROBE), "--contract-json",
                               json.dumps(contract or self.contract), *extra], capture_output=True,
                              text=True, encoding="utf-8", timeout=30, env=env)

    def test_exact_interpreter_executes_pip_and_real_third_party_namespace_import(self):
        result = self.invoke(extra=("--collect-files",))
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["result"], "passed")
        self.assertEqual(report["distributions"], self.contract["requirements"])
        self.assertEqual(report["imports"], self.contract["imports"])
        self.assertTrue(any(path.endswith("native_probe_namespace/part/__init__.py") for path in report["distribution_files"]))
        self.assertFalse(any(path.endswith(".pth") for path in report["distribution_files"]))

    def test_missing_pip_cannot_borrow_host_packages_from_pythonpath(self):
        result = self.invoke(python=self.empty_python, env={**os.environ, "PYTHONPATH": str(self.packages)})
        self.assertNotEqual(result.returncode, 0)
        report = json.loads(result.stderr)
        self.assertEqual(report["result"], "failed")
        self.assertIn("cannot execute pip", report["error"])
        self.assertIn("No module named pip", report["error"])

    def test_present_distribution_does_not_substitute_for_working_import(self):
        contract = copy.deepcopy(self.contract)
        contract["imports"].append("native_probe_missing_binary")
        foreign = self.root / "foreign-package"
        foreign.mkdir(exist_ok=True)
        (foreign / "native_probe_missing_binary.py").write_text("READY=True", encoding="utf-8")
        result = self.invoke(contract=contract, env={**os.environ, "PYTHONPATH": str(foreign)})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("native_probe_missing_binary", json.loads(result.stderr)["error"])

    def test_missing_policy_parser_is_rejected_even_with_working_pip_and_interop_fixture(self):
        contract = copy.deepcopy(self.contract)
        contract["requirements"]["pyyaml"] = "6.0.2"
        contract["constraints"]["pyyaml"] = "6.0.2"
        contract["imports"].append("yaml")
        result = self.invoke(contract=contract)
        self.assertNotEqual(result.returncode, 0)
        error = json.loads(result.stderr)["error"]
        self.assertIn("pyyaml", error.lower())
        self.assertNotIn("cannot execute pip", error)

    def test_system_site_path_cannot_supply_a_dependency_from_another_runtime(self):
        foreign = self.root / "foreign-runtime"
        metadata = foreign / "foreign_dependency-1.0.dist-info"
        metadata.mkdir(parents=True)
        (metadata / "METADATA").write_text("Metadata-Version: 2.1\nName: foreign-dependency\nVersion: 1.0\n", encoding="utf-8")
        path_hook = self.packages / "native_probe_foreign_path.pth"
        path_hook.write_text(str(foreign) + "\n", encoding="utf-8")
        contract = copy.deepcopy(self.contract)
        contract["requirements"]["foreign-dependency"] = "1.0"
        contract["constraints"]["foreign-dependency"] = "1.0"
        try:
            result = self.invoke(contract=contract)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("outside the selected Python runtime", json.loads(result.stderr)["error"])
        finally:
            path_hook.unlink()

    def test_unreviewed_transitive_requirement_fails_without_installing(self):
        metadata = self.packages / "native_probe_fixture-1.0.dist-info/METADATA"
        previous = metadata.read_bytes()
        try:
            metadata.write_bytes(previous + b"Requires-Dist: unreviewed-dependency>=1\n")
            result = self.invoke()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("dependency constraint", json.loads(result.stderr)["error"])
        finally:
            metadata.write_bytes(previous)

    def test_wrong_version_is_not_reported_as_ready(self):
        contract = copy.deepcopy(self.contract)
        contract["constraints"]["native-probe-fixture"] = "9.0"
        result = self.invoke(contract=contract)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected 9.0", json.loads(result.stderr)["error"])

    @unittest.skipUnless(sys.platform == "win32", "real Windows PowerShell guest cache publication")
    def test_actual_guest_probe_publishes_cache_marker_only_after_readiness(self):
        source = (ROOT / "scripts/native-runner-guest.ps1").read_text()
        start = source.index("    Invoke-Tool 'python-preflight'")
        end = source.index("    Copy-Item", start)
        actual = source[start:end]
        self.assertIn('"$pythonVersion\\x64.complete"', actual)
        tools = self.root / "guest-tools"
        tools.mkdir()
        (tools / "python-runtime-probe.py").write_bytes(PROBE.read_bytes())
        (tools / "python-runtime-contract.json").write_text(json.dumps(self.contract), encoding="utf-8")
        script = self.root / "guest-python-readiness.ps1"
        script.write_text(r'''
param([string]$Guest,[string]$Helper,[string]$Tools,[string]$Runtime,[string]$Output,[string]$Version)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$toolsRoot=$Tools; $outputRoot=$Output; $workRoot=$Output; $pythonVersion=$Version
$env:PYTHONHOME=$Runtime
. $Helper
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($Guest,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Guest syntax failed' }
$invoke=$ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Invoke-Tool'},$false)
if ($invoke.Count -ne 1) { throw 'Expected the actual guest process helper' }
. ([ScriptBlock]::Create($invoke[0].Extent.Text))
''' + actual, encoding="utf-8")
        # Venv uses Scripts/python.exe. The guest selects Runtime/python.exe;
        # point Runtime at Scripts so the actual selected executable is used.
        for label, python, accepted in (("ready", self.python, True), ("missing-pip", self.empty_python, False)):
            with self.subTest(label=label):
                output = self.root / ("guest-" + label)
                output.mkdir()
                result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(script),
                                         "-Guest", str(ROOT / "scripts/native-runner-guest.ps1"), "-Helper", str(ROOT / "scripts/gui-native-smoke-process.ps1"),
                                         "-Tools", str(tools), "-Runtime", str(python.parent), "-Output", str(output), "-Version", str(output)],
                                        capture_output=True, text=True, timeout=30,
                                        creationflags=subprocess.CREATE_NO_WINDOW)
                self.assertEqual(result.returncode == 0, accepted, result.stderr)
                self.assertEqual((output / "x64.complete").exists(), accepted)
                self.assertFalse((output / "ready.json").exists())
                if not accepted:
                    self.assertIn("No module named pip", (output / "python-preflight.stderr.log").read_text(encoding="utf-8-sig"))


if __name__ == "__main__":
    unittest.main()
