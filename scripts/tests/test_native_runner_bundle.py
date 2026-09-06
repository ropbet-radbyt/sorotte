from __future__ import annotations

import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from scripts import native_harness_canary as canary
from scripts import native_runner_bundle as bundle

ROOT = Path(__file__).resolve().parents[2]


class NativeRunnerBundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.profile_path = self.root / "profile.json"
        self.profile = bundle.read(bundle.PROFILE)
        # Fixture bytes replace hashes only in this independent test profile.
        for name, item in self.profile["downloads"].items():
            (self.source / name).write_bytes(name.encode())
            item["sha256"] = hashlib.sha256(name.encode()).hexdigest()
        for directory in self.profile["tool_directories"]:
            (self.source / directory).mkdir(exist_ok=True)
        for name in self.profile["required_files"]:
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(name.encode())
        self.profile_path.write_text(json.dumps(self.profile))
        self.patch = mock.patch.object(bundle, "PROFILE", self.profile_path)
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.output = self.root / "output"

    def prepare(self):
        return bundle.prepare(self.source, self.output, self.profile_path)

    def test_cold_and_warm_bundle_preserve_exact_input_inventory(self):
        manifest = self.prepare()
        self.assertEqual(bundle.validate(self.output), manifest)
        self.assertEqual(manifest["files"], bundle.inventory(self.source))
        with self.assertRaises(FileExistsError):
            self.prepare()

    def test_corrupt_cached_download_fails_without_registration(self):
        (self.source / "runner.zip").write_bytes(b"corruption")
        with self.assertRaisesRegex(ValueError, "cached download digest"):
            self.prepare()
        self.assertFalse((self.output / "tools-manifest.json").exists())

    def test_failed_download_removes_partial_bytes(self):
        class Response(io.BytesIO):
            def geturl(self):
                return "https://example.invalid/archive"
        destination = self.root / "download.zip"
        with mock.patch.object(bundle.urllib.request, "urlopen", return_value=Response(b"corruption")):
            with self.assertRaisesRegex(ValueError, "download digest"):
                bundle.download("https://example.invalid/archive", "a" * 64, destination)
        self.assertFalse(destination.exists())
        self.assertEqual(list(self.root.glob("*.partial-*")), [])

    def test_malformed_missing_extra_and_drifted_tool_inputs_fail(self):
        self.prepare()
        path = self.output / "tools/python312/python.exe"
        path.write_bytes(b"drift")
        with self.assertRaisesRegex(ValueError, "closure"):
            bundle.validate(self.output)
        path.unlink()
        with self.assertRaisesRegex(ValueError, "closure"):
            bundle.validate(self.output)

    def test_private_credential_inventory_is_rejected(self):
        (self.source / "git/.credentials").write_text("canary-private")
        with self.assertRaisesRegex(ValueError, "credentials"):
            self.prepare()

    def test_installed_collection_excludes_unrelated_python_packages_and_startup_hooks(self):
        installed = self.root / "installed"
        installed.mkdir()
        sources = {}
        for key in ("git", "powershell", "cmake", "7zip", "python", "windows_sdk"):
            path = installed / key
            path.mkdir()
            sources[key] = str(path)
        msvc = installed / self.profile["msvc_version"]
        for path in ("bin/Hostx64/x64/cl.exe", "include/vector", "lib/x64/libcmt.lib"):
            full = msvc / path
            full.parent.mkdir(parents=True, exist_ok=True)
            full.write_text("compiler")
        sources["msvc"] = str(msvc)
        ninja = installed / "ninja.exe"
        ninja.write_text("ninja")
        sources["ninja"] = str(ninja)
        sdk = Path(sources["windows_sdk"])
        version = self.profile["sdk_version"]
        for path in [f"bin/{version}/x64", *[f"Include/{version}/{name}" for name in ("ucrt", "shared", "um", "winrt")],
                     f"Lib/{version}/ucrt/x64", f"Lib/{version}/um/x64"]:
            (sdk / path).mkdir(parents=True)
        python = Path(sources["python"])
        (python / "DLLs").mkdir()
        (python / "python.exe").write_text("interpreter")
        for name in ("pip", "unrelated-private-package"):
            package = python / "Lib/site-packages" / name
            package.mkdir(parents=True)
            (package / "__init__.py").write_text(name)
        (python / "Lib/sitecustomize.py").write_text("private startup")
        (python / "Lib/os.py").write_text("standard library")
        result = bundle.collect_installed(sources, self.root / "collected")
        self.assertTrue((result / "python312/Lib/site-packages/pip/__init__.py").is_file())
        self.assertFalse((result / "python312/Lib/site-packages/unrelated-private-package").exists())
        self.assertFalse((result / "python312/Lib/sitecustomize.py").exists())

    def test_manifest_or_reviewed_profile_cannot_drift(self):
        self.prepare()
        manifest_path = self.output / "tools-manifest.json"
        manifest_path.write_text(manifest_path.read_text().replace('"max_jobs": 1', '"max_jobs": 2'))
        with self.assertRaisesRegex(ValueError, "changed after preparation"):
            bundle.validate(self.output)

    def test_paths_must_stay_inside_portable_bundle(self):
        for path in ("../private", "C:/private", "/private", "a\\private"):
            profile = copy.deepcopy(self.profile)
            profile["required_files"].append(path)
            with self.assertRaisesRegex(ValueError, "escapes"):
                bundle.validate_profile(profile)

    def assignment(self):
        assignment = {"source_sha": "a" * 40, "run_id": 12, "run_attempt": 2, "job_id": 34}
        run = {"id": 12, "head_sha": "a" * 40, "run_attempt": 2,
               "path": ".github/workflows/gui-native-interactive.yml", "event": "workflow_dispatch",
               "head_repository": {"full_name": "ropbet-radbyt/sorotte"}}
        job = {"id": 34, "run_id": 12, "head_sha": "a" * 40, "status": "queued",
               "labels": ["self-hosted", "Windows", "X64", "sorotte-native-interactive", "sorotte-ephemeral"]}
        return assignment, run, job

    def test_only_exact_trusted_run_attempt_and_job_can_register(self):
        assignment, run, job = self.assignment()
        bundle.validate_assignment(assignment, run, job, self.profile)
        for index, key, value in ((1,"head_sha","b"*40), (1,"run_attempt",1), (1,"event","pull_request"),
                                  (1,"head_repository",{"full_name":"foreign/fork"}), (1,"path",".github/workflows/untrusted.yml"),
                                  (2,"id",35), (2,"status","completed"), (2,"labels",["self-hosted"]),
                                  (0,"run_id",True)):
            values = copy.deepcopy([assignment, run, job])
            values[index][key] = value
            with self.subTest(index=index,key=key), self.assertRaises(ValueError):
                bundle.validate_assignment(*values, self.profile)

    def test_named_canaries_require_nonempty_exact_inventory(self):
        inventory = json.loads(canary.INVENTORY.read_text())
        canary.validate_inventory(inventory)
        self.assertIn("--locked", canary.cargo_args(inventory["cases"][0]))
        inventory["cases"].append(copy.deepcopy(inventory["cases"][0]))
        with self.assertRaisesRegex(ValueError, "unique"):
            canary.validate_inventory(inventory)

    def test_controller_exports_before_teardown_and_retries_independent_cleanup(self):
        source = (ROOT / "scripts/native-runner-sandbox.ps1").read_text()
        cleanup = source[source.index("function Remove-OwnedInstance"):source.index("if ($CleanupOnly)")]
        self.assertLess(cleanup.index("Export-Diagnostic"), cleanup.index("Invoke-Control 'stop'"))
        self.assertIn("sandbox-stop-unconfirmed", cleanup)
        self.assertIn("runner-unregister-unconfirmed", cleanup)
        self.assertIn('"$tokenPath.pending"', cleanup)
        self.assertIn("$MaximumPages=100", source)
        self.assertNotIn("api --paginate --slurp", source)
        self.assertLess(source.index("Guest readiness identity mismatch"), source.index("registration-token\" 'POST'"))
        self.assertIn("validate-assignment", source)

    def test_one_job_source_hook_and_foreground_contract_are_preserved(self):
        source = (ROOT / "scripts/native-runner-guest.ps1").read_text()
        for required in ("WDAGUtilityAccount", "Microsoft Corporation", "Virtual Machine", "SessionName", "OpenInputDesktop",
                         "--ephemeral --disableupdate", "--unattended", "GITHUB_RUN_ATTEMPT", "GITHUB_SHA", "GITHUB_REPOSITORY",
                         "ACTIONS_RUNNER_HOOK_JOB_STARTED", "git\\bin\\bash.exe", "native_failure_evidence.py"):
            self.assertIn(required, source)
        self.assertNotIn("svc.sh", source)
        self.assertNotIn("--runasservice", source)
        self.assertNotIn("git config --global", source)

    @unittest.skipUnless(sys.platform == "win32", "PowerShell syntax and host-refusal proof")
    def test_windows_guest_refuses_untrusted_bootstrap_without_side_effects(self):
        result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                                 str(ROOT / "scripts/native-runner-guest.ps1"), "-InstanceId", "00000000-0000-0000-0000-000000000001",
                                 "-ToolsManifestSha256", "0"*64, "-ScriptSha256", "0"*64, "-HelperSha256", "0"*64, "-ExporterSha256", "0"*64],
                                capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        # On an ordinary host the account check rejects before any desktop probe.
        # When this same canary runs inside the intended Sandbox, the deliberately
        # wrong self digest rejects before the work root or any tool is touched.
        self.assertRegex(result.stderr, "runs only in Windows Sandbox|Bootstrap identity mismatch")

    @unittest.skipUnless(sys.platform == "win32", "Windows PowerShell controller pagination")
    def test_controller_paginates_with_old_cli_and_rejects_partial_or_unbounded_inventory(self):
        # Extract the production API functions without invoking the controller's
        # setup/registration/cleanup. A gh stub accepts only the old CLI's plain
        # request syntax; real PowerShell executes every page and error branch.
        probe = self.root / "pagination.ps1"
        probe.write_text(r'''
param([string]$Controller,[string]$Fixture)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$errors=$null
$tokens=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($Controller,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Controller syntax error' }
$functions=$ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -in @('Api','Api-Items')},$false)
if ($functions.Count -ne 2) { throw 'Expected exact production API helpers' }
. ([ScriptBlock]::Create(($functions | ForEach-Object { $_.Extent.Text }) -join "`n"))
$case=Get-Content -LiteralPath $Fixture -Raw | ConvertFrom-Json
$script:calls=[Collections.Generic.List[string]]::new()
function gh.exe {
    if ($args.Count -ne 4 -or $args[0] -cne 'api' -or $args[1] -cne '--method' -or $args[2] -cne 'GET' -or $args[3] -notmatch '^repos/example/repo/actions/runners\?per_page=100&page=([0-9]+)$') {
        throw 'CLI arguments are not compatible with the installed old gh'
    }
    $pageNumber=[int]$Matches[1]
    $script:calls.Add([string]$args[3])
    if ($pageNumber -gt $case.pages.Count) { throw 'Unexpected extra API page' }
    $page=$case.pages[$pageNumber-1]
    $global:LASTEXITCODE=$page.exit_code
    if ($page.exit_code -eq 0) { $page.response | ConvertTo-Json -Depth 8 -Compress }
}
$emitted=[Collections.Generic.List[object]]::new()
try {
    Api-Items 'repos/example/repo/actions/runners?per_page=100' 'runners' $case.maximum_pages | ForEach-Object { $emitted.Add($_) }
    @{status='passed';ids=@($emitted | ForEach-Object { $_.id });calls=@($script:calls);error=$null} | ConvertTo-Json -Depth 5 -Compress
} catch {
    @{status='rejected';ids=@($emitted | ForEach-Object { $_.id });calls=@($script:calls);error=$_.Exception.Message} | ConvertTo-Json -Depth 5 -Compress
}
''', encoding="utf-8")

        def page(first: int, last: int, total: int, exit_code: int = 0) -> dict:
            return {"exit_code": exit_code,
                    "response": {"total_count": total, "runners": [{"id": item} for item in range(first, last + 1)]}}

        cases = [
            ("two-pages", [page(1, 100, 103), page(101, 103, 103)], 2, "passed", 2, list(range(1, 104)), ""),
            ("empty", [page(1, 0, 0)], 2, "passed", 1, [], ""),
            ("exact-bound", [page(1, 100, 100)], 1, "passed", 1, list(range(1, 101)), ""),
            ("overflow", [page(1, 100, 201)], 2, "rejected", 1, [], "bounded pagination"),
            ("later-api-failure", [page(1, 100, 103), page(101, 103, 103, 1)], 2, "rejected", 2, [], "request failed"),
            ("truncated", [page(1, 99, 103)], 2, "rejected", 1, [], "ended before"),
            ("count-drift", [page(1, 100, 103), page(101, 104, 104)], 2, "rejected", 2, [], "changed during"),
            ("duplicate", [page(1, 100, 103), page(100, 102, 103)], 2, "rejected", 2, [], "repeated an item"),
            ("missing-envelope", [{"exit_code": 0, "response": {"runners": []}}], 2, "rejected", 1, [], "incomplete"),
        ]
        for name, pages, limit, status, calls, ids, error in cases:
            with self.subTest(case=name):
                fixture = self.root / (name + ".json")
                fixture.write_text(json.dumps({"maximum_pages": limit, "pages": pages}), encoding="utf-8")
                result = subprocess.run(
                    ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                     "-File", str(probe), "-Controller", str(ROOT / "scripts/native-runner-sandbox.ps1"), "-Fixture", str(fixture)],
                    capture_output=True, text=True, timeout=15,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                observed = json.loads(result.stdout)
                self.assertEqual(observed["status"], status, observed)
                self.assertEqual(observed["ids"], ids, observed)
                self.assertEqual(len(observed["calls"]), calls, observed)
                self.assertEqual(observed["calls"], [f"repos/example/repo/actions/runners?per_page=100&page={number}" for number in range(1, calls + 1)])
                if error:
                    self.assertIn(error, observed["error"])

    @unittest.skipUnless(sys.platform == "win32", "Windows PowerShell inherited module lookup")
    def test_controller_and_watchdog_use_runtime_utility_commands_despite_shadowed_module_path(self):
        # Reproduce the Python -> WinPS seam without requiring a particular
        # installed pwsh version. An inherited Core-first path may resolve a
        # different Utility module; the exact runtime import must restore both
        # controller hashing and independent-watchdog JSON observation.
        module_root = self.root / "inherited-modules"
        shadow = module_root / "Microsoft.PowerShell.Utility"
        shadow.mkdir(parents=True)
        (shadow / "shadow.psm1").write_text("function Get-FileHash { throw 'shadow hash' }\n", encoding="utf-8")
        manifest = shadow / "Microsoft.PowerShell.Utility.psd1"
        manifest.write_text("@{RootModule='shadow.psm1';ModuleVersion='999.0.0';FunctionsToExport=@('Get-FileHash');CmdletsToExport=@()}\n", encoding="utf-8")
        data = self.root / "manifest-input"
        data.write_bytes(b"exact-reviewed-tool-manifest")
        probe = self.root / "module-preflight.ps1"
        probe.write_text(r'''
param([string]$Controller,[string]$Watchdog,[string]$Shadow,[string]$Data)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$results=[Collections.Generic.List[object]]::new()
foreach ($source in @($Controller,$Watchdog)) {
    Import-Module $Shadow -Force
    $hashShadowed=$false
    try { Get-FileHash -LiteralPath $Data -Algorithm SHA256 | Out-Null } catch { $hashShadowed=$_.Exception.Message -ceq 'shadow hash' }
    if (-not $hashShadowed) { throw 'Fixture did not reproduce shadowed Utility hashing' }
    $tokens=$null; $errors=$null
    $ast=[Management.Automation.Language.Parser]::ParseFile($source,[ref]$tokens,[ref]$errors)
    if ($errors.Count) { throw 'Native entry point syntax failed' }
    $imports=$ast.FindAll({param($node) $node -is [Management.Automation.Language.CommandAst] -and $node.GetCommandName() -ceq 'Import-Module' -and $node.Extent.Text.Contains('Microsoft.PowerShell.Utility.psd1')},$false)
    if ($imports.Count -ne 1) { throw 'Expected one explicit runtime Utility import in the actual entry point' }
    . ([ScriptBlock]::Create($imports[0].Extent.Text))
    $hash=(Get-FileHash -LiteralPath $Data -Algorithm SHA256).Hash.ToLowerInvariant()
    $parsed='{"valid":true}' | ConvertFrom-Json
    $module=(Get-Command Get-FileHash).Module.Path
    if ($module -ine (Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1')) { throw 'Hash command did not come from this PowerShell runtime' }
    if ((Get-Command ConvertFrom-Json).Module.Path -ine $module) { throw 'JSON command did not come from this PowerShell runtime' }
    $results.Add(@{source=[IO.Path]::GetFileName($source);hash=$hash;json_valid=$parsed.valid;shadow_reproduced=$true})
}
ConvertTo-Json -InputObject @($results) -Depth 4 -Compress
''', encoding="utf-8")
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(probe),
             "-Controller", str(ROOT / "scripts/native-runner-sandbox.ps1"), "-Watchdog", str(ROOT / "scripts/native-runner-watchdog.ps1"),
             "-Shadow", str(manifest), "-Data", str(data)],
            capture_output=True, text=True, timeout=15, env={**os.environ, "PSModulePath": str(module_root)},
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        observed = json.loads(result.stdout)
        self.assertEqual(len(observed), 2)
        for item in observed:
            self.assertTrue(item["shadow_reproduced"], item)
            self.assertTrue(item["json_valid"], item)
            self.assertEqual(item["hash"], hashlib.sha256(data.read_bytes()).hexdigest(), item)

    @unittest.skipUnless(sys.platform == "win32", "Windows PowerShell independent cleanup")
    def test_recovery_unregisters_even_when_sandbox_cli_or_python_is_unavailable(self):
        probe = self.root / "recover.ps1"
        probe.write_text(r'''
param([string]$Controller,[string]$Fixture,[string]$Instance)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$case=Get-Content -LiteralPath $Fixture -Raw | ConvertFrom-Json
$global:SorotteFixtureCalls=[Collections.Generic.List[string]]::new()
$global:SorotteFixtureRunnerPresent=$true
$global:SorotteFixtureGuestPresent=$true
$global:SorotteFixtureInstance=$Instance
$global:SorotteFixtureMissingWsb=$case.missing_wsb
$global:SorotteFixtureMissingPython=$case.missing_python
function global:Get-Command {
    [CmdletBinding()]param([string]$Name)
    $global:SorotteFixtureCalls.Add('lookup:'+$Name)
    if ($Name -eq 'wsb.exe') {
        if ($global:SorotteFixtureMissingWsb) { throw 'fixture Sandbox CLI unavailable' }
        return [pscustomobject]@{Source='fixture-wsb'}
    }
    if ($Name -eq 'python') {
        if ($global:SorotteFixtureMissingPython) { throw 'fixture Python unavailable' }
        return [pscustomobject]@{Source='fixture-python'}
    }
    throw 'Unexpected external command lookup'
}
function global:fixture-python { $global:SorotteFixtureCalls.Add('safe-export'); $global:LASTEXITCODE=0 }
function global:gh.exe {
    if ($args.Count -ne 4 -or $args[0] -cne 'api' -or $args[1] -cne '--method') { throw 'Unexpected CLI invocation' }
    $global:LASTEXITCODE=0
    $global:SorotteFixtureCalls.Add($args[2]+':'+$args[3])
    if ($args[2] -ceq 'DELETE' -and $args[3] -ceq 'repos/ropbet-radbyt/sorotte/actions/runners/77') {
        $global:SorotteFixtureRunnerPresent=$false
        return
    }
    if ($args[2] -ceq 'GET' -and $args[3] -ceq 'repos/ropbet-radbyt/sorotte/actions/runners?per_page=100&page=1') {
        if ($global:SorotteFixtureRunnerPresent) {
            @{total_count=1;runners=@(@{id=77;name='sorotte-sandbox-'+$Instance})} | ConvertTo-Json -Depth 4 -Compress
        } else { '{"total_count":0,"runners":[]}' }
        return
    }
    throw 'Unexpected API endpoint or mutation'
}
$errorText=$null
try { & $Controller -CleanupOnly -InstanceId $Instance }
catch { $errorText=$_.Exception.Message }
$receiptPath=Join-Path (Split-Path -Parent (Split-Path -Parent $Controller)) ('target/verification/native-runners/'+$Instance+'/host-run.json')
$receipt=Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
@{receipt=$receipt;calls=@($global:SorotteFixtureCalls);error=$errorText} | ConvertTo-Json -Depth 8 -Compress
''', encoding="utf-8")
        helper = r'''
function Invoke-CapturedProcess {
    param($FilePath,$Arguments,$WorkingDirectory,$ProcessTimeoutMs,$StdoutPath,$StderrPath)
    if ($FilePath -cne 'fixture-wsb') { throw 'Unexpected process invocation' }
    $global:SorotteFixtureCalls.Add('wsb:'+$Arguments[0])
    if ($Arguments[0] -ceq 'stop') {
        if ($Arguments[1] -cne '--id' -or $Arguments[2] -cne $global:SorotteFixtureInstance) { throw 'Cleanup targeted another guest' }
        $global:SorotteFixtureGuestPresent=$false
        $output='{}'
    } elseif ($Arguments[0] -ceq 'list') {
        if ($global:SorotteFixtureGuestPresent) { $output=@{WindowsSandboxEnvironments=@(@{Id=$global:SorotteFixtureInstance})} | ConvertTo-Json -Depth 4 -Compress }
        else { $output='{"WindowsSandboxEnvironments":[]}' }
    } else { throw 'Unexpected Sandbox action' }
    [IO.File]::WriteAllText($StdoutPath,$output)
    [IO.File]::WriteAllText($StderrPath,'')
    return @{exit_code=0;timed_out=$false}
}
'''
        instance = "00000000-0000-0000-0000-000000000123"
        for missing_wsb, missing_python in ((True, False), (False, True), (True, True)):
            with self.subTest(missing_wsb=missing_wsb, missing_python=missing_python):
                fixture_root = self.root / f"recovery-{missing_wsb}-{missing_python}"
                scripts = fixture_root / "scripts"
                scripts.mkdir(parents=True)
                controller = scripts / "native-runner-sandbox.ps1"
                controller.write_text((ROOT / "scripts/native-runner-sandbox.ps1").read_text(), encoding="utf-8")
                (scripts / "native-runner-receipt.ps1").write_text((ROOT / "scripts/native-runner-receipt.ps1").read_text(), encoding="utf-8")
                (scripts / "native-runner-owner.ps1").write_text((ROOT / "scripts/native-runner-owner.ps1").read_text(), encoding="utf-8")
                (scripts / "gui-native-smoke-process.ps1").write_text(helper, encoding="utf-8")
                run = fixture_root / "target/verification/native-runners" / instance
                (run / "output").mkdir(parents=True)
                (run / "host-run.json").write_text(json.dumps({
                    "instance": instance, "repository": "ropbet-radbyt/sorotte", "runner_name": "sorotte-sandbox-" + instance,
                    "source_sha": "a" * 40, "run_id": 1, "run_attempt": 1, "status": "failed",
                    "sandbox_stopped": False, "runner_removed": False, "automatic_unregister": False,
                    "evidence_export": "unavailable", "evidence_directory": None, "cleanup_errors": [], "finished_at_utc": None,
                }), encoding="utf-8")
                fixture = fixture_root / "case.json"
                fixture.write_text(json.dumps({"missing_wsb": missing_wsb, "missing_python": missing_python}), encoding="utf-8")
                result = subprocess.run(
                    ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(probe),
                     "-Controller", str(controller), "-Fixture", str(fixture), "-Instance", instance],
                    capture_output=True, text=True, timeout=15,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                observed = json.loads(result.stdout)
                self.assertIn("DELETE:repos/ropbet-radbyt/sorotte/actions/runners/77", observed["calls"], observed)
                self.assertTrue(observed["receipt"]["runner_removed"], observed)
                self.assertEqual(observed["receipt"]["sandbox_stopped"], not missing_wsb, observed)
                if missing_wsb:
                    self.assertIn("sandbox-stop-unconfirmed", observed["receipt"]["cleanup_errors"])
                    self.assertIn("cleanup remains unconfirmed", observed["error"])
                else:
                    self.assertIn("wsb:stop", observed["calls"])
                    self.assertIsNone(observed["error"])
                self.assertEqual(observed["receipt"]["evidence_export"], "unavailable" if missing_python else "exported")


if __name__ == "__main__":
    unittest.main()
