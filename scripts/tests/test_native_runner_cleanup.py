"""Run the actual Windows recovery entry point with isolated API/guest fixtures."""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
INSTANCE = "00000000-0000-0000-0000-000000000123"

# Only external process responses and clock durations are replaced. The actual
# recovery entry point, ownership checks, pagination, cancellation/drain, receipt
# publication, teardown ordering, and failure paths all run in Windows PowerShell.
API_FIXTURE = r'''
$script:RealCleanupCommand=${function:Invoke-NativeCleanupCommand}
$script:RealCleanupContext=${function:New-NativeCleanupContext}
function New-NativeCleanupContext($Directory,$BudgetMs,$ReceiptPath) {
    $context=& $script:RealCleanupContext $Directory $BudgetMs $ReceiptPath
    $context.cancel_grace_ms=45
    $context.force_grace_ms=45
    $context.unregister_grace_ms=500
    $context.poll_ms=5
    $context.call_timeout_ms=200
    if ($global:Case.mode -ceq 'api-hangs') { $context.budget_ms=400 }
    return $context
}
function Invoke-NativeCleanupCommand($FilePath,$Arguments,$TimeoutMs) {
    if ($FilePath -ceq 'fixture-python') {
        $global:Calls.Add('safe-export')
        if ($global:Case.mode -ceq 'exporter-hangs') {
            return (& $script:RealCleanupCommand -FilePath $global:Case.python -Arguments @('-c','import time; time.sleep(60)') -TimeoutMs 200)
        }
        return @{exit_code=0;timed_out=$false;stdout='';stderr=''}
    }
    if ($Arguments.Count -ne 4 -or $Arguments[0] -cne 'api' -or $Arguments[1] -cne '--method') { throw 'Unexpected API command' }
    $method=$Arguments[2]; $path=$Arguments[3]
    $global:Calls.Add($method+':'+$path)
    if ($global:Case.mode -ceq 'api-hangs') {
        return (& $script:RealCleanupCommand -FilePath $global:Case.python -Arguments @('-c','import time; time.sleep(60)') -TimeoutMs $TimeoutMs)
    }
    if ($global:Case.mode -ceq 'api-unavailable') { throw 'fixture network unavailable' }
    $value=$null; $code=0; $errorText=''
    if ($method -ceq 'GET' -and $path -ceq 'repos/ropbet-radbyt/sorotte/actions/runs/1') {
        $global:RunReads++
        $attempt=if ($global:Case.mode -ceq 'stale-attempt' -or ($global:Case.mode -ceq 'rerun-before-force' -and $global:NormalCancel) -or ($global:Case.mode -ceq 'rerun-before-normal' -and $global:RunReads -ge 3)) {2} else {1}
        $source=if ($global:Case.mode -ceq 'wrong-source') {'b'*40} else {'a'*40}
        $workflow=if ($global:Case.mode -ceq 'wrong-workflow') {'.github/workflows/unrelated.yml'} else {'.github/workflows/gui-native-interactive.yml'}
        $value=@{id=1;run_attempt=$attempt;head_sha=$source;path=$workflow;repository=@{full_name='ropbet-radbyt/sorotte'};head_repository=@{full_name='ropbet-radbyt/sorotte'};event='workflow_dispatch';status=$(if($global:Complete){'completed'}else{'in_progress'})}
    } elseif ($method -ceq 'GET' -and $path -ceq 'repos/ropbet-radbyt/sorotte/actions/jobs/2') {
        $name=if ($global:Case.mode -ceq 'wrong-runner') {'sorotte-sandbox-unrelated'} else {'sorotte-sandbox-'+$global:Case.instance}
        $attempt=if ($global:Case.mode -ceq 'wrong-job-attempt') {2} else {1}
        $value=@{id=2;run_id=1;run_attempt=$attempt;head_sha='a'*40;runner_id=77;runner_name=$name;status=$(if($global:Complete){'completed'}else{'in_progress'});conclusion=$(if($global:Case.passed){'success'}else{'cancelled'});labels=@('self-hosted','Windows','X64','sorotte-native-interactive','sorotte-ephemeral')}
    } elseif ($method -ceq 'GET' -and $path -ceq 'repos/ropbet-radbyt/sorotte/actions/runners?per_page=100&page=1') {
        $global:RunnerReads++
        if ($global:Case.mode -ceq 'passed-automatic' -and $global:RunnerReads -ge 2) { $global:RunnerPresent=$false }
        $runners=@(@{id=88;name='unrelated-runner';busy=$false})
        if ($global:RunnerPresent) {
            $identity=if ($global:Case.mode -ceq 'wrong-runner-id') {99} else {77}
            $runners+=@{id=$identity;name='sorotte-sandbox-'+$global:Case.instance;busy=$global:Busy}
        }
        $value=@{total_count=$runners.Count;runners=$runners}
    } elseif ($method -ceq 'POST' -and $path -ceq 'repos/ropbet-radbyt/sorotte/actions/runs/1/cancel') {
        if (-not $global:GuestPresent) { throw 'Cancellation happened after guest teardown' }
        $global:NormalCancel=$true
        if ($global:Case.mode -notin @('force-required','force-unresponsive','rerun-before-force','cancel-conflict')) {
            $global:Complete=$true; $global:Busy=$false
        }
        if ($global:Case.mode -ceq 'cancel-conflict') { $code=1; $errorText='gh: Cancellation already in progress. (HTTP 409)' }
    } elseif ($method -ceq 'POST' -and $path -ceq 'repos/ropbet-radbyt/sorotte/actions/runs/1/force-cancel') {
        if (-not $global:GuestPresent -or -not $global:NormalCancel) { throw 'Force cancellation bypassed normal guest-alive drain' }
        if ($global:Case.mode -cne 'force-unresponsive') { $global:Complete=$true; $global:Busy=$false }
    } elseif ($method -ceq 'DELETE' -and $path -ceq 'repos/ropbet-radbyt/sorotte/actions/runners/77') {
        if ($global:Busy -or ($global:Case.mode -ceq 'delete-race' -and -not $global:DeleteRetried)) {
            $global:DeleteRetried=$true
            $code=1; $errorText='gh: Runner is currently running a job and cannot be deleted. (HTTP 422)'
        } else { $global:RunnerPresent=$false }
    } else { throw 'Unexpected API endpoint or mutation' }
    return @{exit_code=$code;timed_out=$false;stdout=$(if($null -eq $value){''}else{ConvertTo-Json -InputObject $value -Depth 10 -Compress});stderr=$errorText}
}
'''

CONTROL_FIXTURE = r'''
function Invoke-CapturedProcess($FilePath,$Arguments,$WorkingDirectory,$ProcessTimeoutMs,$StdoutPath,$StderrPath) {
    if ($FilePath -cne 'fixture-wsb') { throw 'Unexpected process invocation' }
    $global:Calls.Add('wsb:'+$Arguments[0])
    if ($Arguments[0] -ceq 'stop') {
        if ($Arguments[1] -cne '--id' -or $Arguments[2] -cne $global:Case.instance) { throw 'Cleanup targeted another guest' }
        $global:GuestPresent=$false; $output='{}'
    } elseif ($Arguments[0] -ceq 'list') {
        $guests=@(@{Id='00000000-0000-0000-0000-000000000999'})
        if ($global:GuestPresent) { $guests+=@{Id=$global:Case.instance} }
        $output=@{WindowsSandboxEnvironments=$guests} | ConvertTo-Json -Depth 4 -Compress
    } else { throw 'Unexpected Sandbox action' }
    [IO.File]::WriteAllText($StdoutPath,$output)
    [IO.File]::WriteAllText($StderrPath,'')
    return @{exit_code=0;timed_out=$false}
}
'''

DRIVER = r'''
param([string]$Controller,[string]$Fixture)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$global:Case=Get-Content -LiteralPath $Fixture -Raw | ConvertFrom-Json
$global:Calls=[Collections.Generic.List[string]]::new()
$global:RunnerPresent=$true; $global:GuestPresent=$true
$global:Complete=$global:Case.passed; $global:Busy=-not $global:Complete
$global:NormalCancel=$false; $global:DeleteRetried=$false
$global:RunReads=0; $global:RunnerReads=0
function global:Get-Command {
    [CmdletBinding()]param([string]$Name)
    if ($Name -eq 'wsb.exe') {
        if ($global:Case.missing_wsb) { throw 'fixture Sandbox CLI unavailable' }
        return [pscustomobject]@{Source='fixture-wsb'}
    }
    if ($Name -eq 'python') {
        if ($global:Case.missing_python) { throw 'fixture Python unavailable' }
        return [pscustomobject]@{Source='fixture-python'}
    }
    if ($Name -eq 'gh.exe') { return [pscustomobject]@{Source='fixture-gh'} }
    throw 'Unexpected command lookup'
}
function global:fixture-python { $global:Calls.Add('safe-export'); $global:LASTEXITCODE=0 }
$attempts=[Collections.Generic.List[object]]::new()
if ($global:Case.request) {
    $scripts=Split-Path -Parent $Controller
    . (Join-Path $scripts 'gui-native-smoke-process.ps1')
    . (Join-Path $scripts 'native-runner-receipt.ps1')
    . (Join-Path $scripts 'native-runner-cleanup.ps1')
    $request=[ordered]@{source_sha='a'*40;run_id=1;run_attempt=1;job_id=2;instance=$global:Case.instance;cancellation='not-needed'}
    if ($global:Case.mode -ceq 'unbound-job') { $request.job_id=$null }
    if ($global:Case.mode -ceq 'unbound-attempt') { $request.run_attempt=$null }
    if ($global:Case.mode -ceq 'unbound-instance') { $request.instance=$null }
    $errorText=$null
    try {
        $context=New-NativeCleanupContext (Join-Path (Split-Path -Parent $Fixture) 'request-api') 30000 ''
        $request.cancellation=Stop-NativeQualificationRequest $context $request
    } catch { $errorText=$_.Exception.Message }
    $attempts.Add(@{receipt=$request;error=$errorText})
} else { for ($number=0;$number -lt $global:Case.recoveries;$number++) {
    $errorText=$null
    $heldToken=$null
    if ($global:Case.mode -ceq 'held-token' -and $number -eq 0) {
        $tokenPath=Join-Path (Split-Path -Parent (Split-Path -Parent $Controller)) ('target/verification/native-runners/'+$global:Case.instance+'/output/registration-token.json')
        [IO.File]::WriteAllText($tokenPath,'fixture-not-a-credential')
        $heldToken=[IO.File]::Open($tokenPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::None)
    }
    try { & $Controller -CleanupOnly -InstanceId $global:Case.instance }
    catch { $errorText=$_.Exception.Message }
    finally { if ($null -ne $heldToken) { $heldToken.Dispose() } }
    $receiptPath=Join-Path (Split-Path -Parent (Split-Path -Parent $Controller)) ('target/verification/native-runners/'+$global:Case.instance+'/host-run.json')
    $receipt=Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    $attempts.Add(@{receipt=$receipt;error=$errorText})
} }
@{attempts=@($attempts);calls=@($global:Calls);guest_present=$global:GuestPresent;runner_present=$global:RunnerPresent} | ConvertTo-Json -Depth 12 -Compress
'''


@unittest.skipUnless(sys.platform == "win32", "Windows PowerShell cleanup contract")
class NativeRunnerCleanupTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def recovery(self, mode="normal", *, passed=False, missing_wsb=False, missing_python=False, recoveries=1, request=False):
        root = self.root / f"{mode}-{missing_wsb}-{missing_python}"
        scripts = root / "scripts"
        scripts.mkdir(parents=True)
        for name in ("native-runner-sandbox.ps1", "native-runner-receipt.ps1", "native-runner-owner.ps1", "native-runner-cleanup.ps1"):
            shutil.copyfile(ROOT / "scripts" / name, scripts / name)
        with (scripts / "native-runner-cleanup.ps1").open("a", encoding="utf-8") as stream:
            stream.write(API_FIXTURE)
        (scripts / "gui-native-smoke-process.ps1").write_text(
            (ROOT / "scripts/gui-native-smoke-process.ps1").read_text() + CONTROL_FIXTURE, encoding="utf-8")
        (root / "verification").mkdir()
        shutil.copyfile(ROOT / "verification/windows-native-guest.json", root / "verification/windows-native-guest.json")
        run = root / "target/verification/native-runners" / INSTANCE
        (run / "output").mkdir(parents=True)
        (run / "host-run.json").write_text(json.dumps({
            "instance": INSTANCE, "repository": "ropbet-radbyt/sorotte", "runner_name": "sorotte-sandbox-" + INSTANCE,
            "source_sha": "a" * 40, "run_id": 1, "run_attempt": 1, "job_id": 2, "status": "passed" if passed else "running-job",
            "sandbox_stopped": False, "runner_removed": False, "automatic_unregister": False,
            "evidence_export": "unavailable", "evidence_directory": None, "cleanup_errors": [], "finished_at_utc": None,
        }), encoding="utf-8")
        fixture = root / "case.json"
        fixture.write_text(json.dumps({"mode": mode, "passed": passed, "missing_wsb": missing_wsb,
            "missing_python": missing_python, "recoveries": recoveries, "instance": INSTANCE, "python": sys.executable,
            "request": request}), encoding="utf-8")
        driver = root / "driver.ps1"
        driver.write_text(DRIVER, encoding="utf-8")
        started = time.monotonic()
        result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", str(driver), "-Controller", str(scripts / "native-runner-sandbox.ps1"), "-Fixture", str(fixture)],
            capture_output=True, text=True, timeout=25, creationflags=subprocess.CREATE_NO_WINDOW)
        self.assertEqual(result.returncode, 0, result.stderr)
        value = json.loads(result.stdout)
        value["duration"] = time.monotonic() - started
        value["captures"] = [json.loads(path.read_text()) for path in run.glob("cleanup-*/api-*.json")]
        value["unavailable"] = [json.loads(path.read_text()) for path in run.glob("safe-evidence-*/host-export-unavailable.json")]
        return value

    def test_interruption_cancels_and_drains_busy_job_before_guest_stop_and_delete(self):
        result = self.recovery()
        calls = result["calls"]
        cancel = "POST:repos/ropbet-radbyt/sorotte/actions/runs/1/cancel"
        delete = "DELETE:repos/ropbet-radbyt/sorotte/actions/runners/77"
        self.assertLess(calls.index("safe-export"), calls.index(cancel), result)
        self.assertLess(calls.index(cancel), calls.index("wsb:stop"), result)
        self.assertLess(calls.index("wsb:stop"), calls.index(delete), result)
        self.assertEqual(sum("force-cancel" in call for call in calls), 0, result)
        self.assertEqual([call for call in calls if call.startswith("DELETE")], [delete], result)
        self.assertTrue(result["attempts"][0]["receipt"]["runner_removed"], result)
        self.assertTrue(result["attempts"][0]["receipt"]["sandbox_stopped"], result)
        self.assertIsNone(result["attempts"][0]["error"], result)

    def test_unresponsive_normal_cancel_uses_guarded_force_cancel_while_guest_alive(self):
        result = self.recovery("force-required")
        calls = result["calls"]
        normal = calls.index("POST:repos/ropbet-radbyt/sorotte/actions/runs/1/cancel")
        force = calls.index("POST:repos/ropbet-radbyt/sorotte/actions/runs/1/force-cancel")
        self.assertLess(normal, force, result)
        self.assertGreater(calls[normal + 1:force].count("GET:repos/ropbet-radbyt/sorotte/actions/jobs/2"), 1, result)
        self.assertLess(force, calls.index("wsb:stop"), result)
        self.assertIsNone(result["attempts"][0]["error"], result)

    def test_concurrent_cancel_conflict_is_preserved_and_reobserved_before_force(self):
        result = self.recovery("cancel-conflict")
        self.assertTrue(any(item["result"] and "HTTP 409" in item["result"]["stderr"] for item in result["captures"]), result)
        self.assertIn("POST:repos/ropbet-radbyt/sorotte/actions/runs/1/force-cancel", result["calls"], result)
        self.assertTrue(result["attempts"][0]["receipt"]["runner_removed"], result)
        self.assertIsNone(result["attempts"][0]["error"], result)

    def test_api_failure_deadline_and_busy_job_never_suppress_guest_teardown(self):
        for mode in ("api-unavailable", "api-hangs", "force-unresponsive"):
            with self.subTest(mode=mode):
                result = self.recovery(mode)
                receipt = result["attempts"][0]["receipt"]
                self.assertTrue(receipt["sandbox_stopped"], result)
                self.assertFalse(receipt["runner_removed"], result)
                self.assertIn("job-drain-unconfirmed", receipt["cleanup_errors"], result)
                self.assertIn("runner-unregister-unconfirmed", receipt["cleanup_errors"], result)
                self.assertFalse(any(call.startswith("DELETE") for call in result["calls"]), result)
                self.assertLess(result["duration"], 10, result)
                if mode == "api-hangs":
                    self.assertTrue(any(item["result"] and item["result"]["timed_out"] for item in result["captures"]), result)

    def test_hung_exporter_is_killed_and_records_unavailable_before_independent_cleanup(self):
        result = self.recovery("exporter-hangs")
        receipt = result["attempts"][0]["receipt"]
        self.assertTrue(receipt["sandbox_stopped"], result)
        self.assertTrue(receipt["runner_removed"], result)
        self.assertEqual(receipt["evidence_export"], "unavailable", result)
        self.assertEqual(len(result["unavailable"]), 3, result)
        self.assertTrue(all(item["reason"] == "exporter-timeout" and item["authoritative"] is False for item in result["unavailable"]), result)
        self.assertLess(result["duration"], 10, result)

    def test_locked_token_cannot_block_teardown_and_later_recovery_removes_it(self):
        result = self.recovery("held-token", recoveries=2)
        first, second = result["attempts"]
        self.assertTrue(first["receipt"]["sandbox_stopped"], result)
        self.assertTrue(first["receipt"]["runner_removed"], result)
        self.assertFalse(first["receipt"]["tokens_removed"], result)
        self.assertIn("registration-token-removal-unconfirmed", first["receipt"]["cleanup_errors"], result)
        self.assertIsNotNone(first["error"], result)
        self.assertTrue(second["receipt"]["tokens_removed"], result)
        self.assertIsNone(second["error"], result)

    def test_stale_or_foreign_binding_refuses_mutation_but_stops_only_owned_guest(self):
        for mode in ("stale-attempt", "wrong-source", "wrong-workflow", "wrong-job-attempt", "wrong-runner", "wrong-runner-id", "rerun-before-normal", "rerun-before-force"):
            with self.subTest(mode=mode):
                result = self.recovery(mode)
                mutations = [call for call in result["calls"] if call.startswith(("POST", "DELETE"))]
                expected = ["POST:repos/ropbet-radbyt/sorotte/actions/runs/1/cancel"] if mode == "rerun-before-force" else []
                self.assertEqual(mutations, expected, result)
                self.assertTrue(result["attempts"][0]["receipt"]["sandbox_stopped"], result)
                self.assertFalse(result["attempts"][0]["receipt"]["runner_removed"], result)

    def test_passed_job_waits_for_automatic_unregister_without_cancel_or_delete(self):
        result = self.recovery("passed-automatic", passed=True)
        self.assertFalse(any(call.startswith(("POST", "DELETE")) for call in result["calls"]), result)
        self.assertTrue(result["attempts"][0]["receipt"]["automatic_unregister"], result)
        self.assertIsNone(result["attempts"][0]["error"], result)

    def test_manual_delete_can_never_become_automatic_unregister_on_recovery(self):
        result = self.recovery("passed-manual", passed=True, recoveries=2)
        self.assertEqual(len(result["attempts"]), 2)
        for attempt in result["attempts"]:
            self.assertFalse(attempt["receipt"]["automatic_unregister"], result)
            self.assertTrue(attempt["receipt"]["runner_delete_requested"], result)
            self.assertTrue(attempt["receipt"]["runner_removed"], result)
        self.assertFalse(any(call.startswith("POST") for call in result["calls"]), result)

    def test_delete_422_preserves_error_and_revalidates_before_retry(self):
        result = self.recovery("delete-race")
        calls = result["calls"]
        indices = [index for index, call in enumerate(calls) if call.startswith("DELETE")]
        self.assertEqual(len(indices), 2, result)
        self.assertIn("GET:repos/ropbet-radbyt/sorotte/actions/jobs/2", calls[indices[0] + 1:indices[1]], result)
        self.assertTrue(any(item["result"] and "HTTP 422" in item["result"]["stderr"] for item in result["captures"]), result)
        self.assertTrue(result["attempts"][0]["receipt"]["runner_removed"], result)

    def test_recovery_unregisters_even_when_sandbox_cli_or_python_is_unavailable(self):
        for missing_wsb, missing_python in ((True, False), (False, True), (True, True)):
            with self.subTest(missing_wsb=missing_wsb, missing_python=missing_python):
                result = self.recovery(missing_wsb=missing_wsb, missing_python=missing_python)
                receipt = result["attempts"][0]["receipt"]
                self.assertTrue(receipt["runner_removed"], result)
                self.assertEqual(receipt["sandbox_stopped"], not missing_wsb, result)
                self.assertEqual(receipt["evidence_export"], "unavailable" if missing_python else "exported", result)
                if missing_wsb:
                    self.assertIn("sandbox-stop-unconfirmed", receipt["cleanup_errors"], result)

    def test_qualifier_fallback_requires_bound_job_attempt_and_assigned_instance(self):
        result = self.recovery(request=True)
        self.assertEqual(result["attempts"][0]["receipt"]["cancellation"], "requested", result)
        self.assertEqual([call for call in result["calls"] if call.startswith("POST")],
                         ["POST:repos/ropbet-radbyt/sorotte/actions/runs/1/cancel"], result)
        for mode in ("unbound-job", "unbound-attempt", "unbound-instance", "stale-attempt", "wrong-source", "wrong-workflow", "wrong-job-attempt", "wrong-runner", "wrong-runner-id"):
            with self.subTest(mode=mode):
                result = self.recovery(mode, request=True)
                self.assertIsNotNone(result["attempts"][0]["error"], result)
                self.assertFalse(any(call.startswith(("POST", "DELETE", "wsb:")) for call in result["calls"]), result)


@unittest.skipUnless(sys.platform == "win32", "Windows owned cleanup API process")
class NativeCleanupProcessTests(unittest.TestCase):
    def test_deadline_kills_only_owned_child_and_utf8_output_is_explicit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            child = root / "child.py"
            marker = root / "pid"
            child.write_text("import os,pathlib,sys,time\npathlib.Path(sys.argv[1]).write_text(str(os.getpid()))\nprint('\\u2713 native',flush=True)\ntime.sleep(60)\n", encoding="utf-8")
            probe = root / "probe.ps1"
            probe.write_text(r'''
param([string]$Root,[string]$Python,[string]$Child,[string]$Marker)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
. (Join-Path $Root 'scripts/gui-native-smoke-process.ps1')
. (Join-Path $Root 'scripts/native-runner-cleanup.ps1')
$env:PYTHONUTF8='1'
$clock=[Diagnostics.Stopwatch]::StartNew()
$result=Invoke-NativeCleanupCommand -FilePath $Python -Arguments @($Child,$Marker) -TimeoutMs 1500
$owned=[int](Get-Content -LiteralPath $Marker -Raw)
$result.owned_absent=$null -eq (Get-Process -Id $owned -ErrorAction SilentlyContinue)
$result.duration_ms=$clock.ElapsedMilliseconds
$result | ConvertTo-Json -Compress
''', encoding="utf-8")
            unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"], creationflags=subprocess.CREATE_NO_WINDOW)
            try:
                result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                    "-File", str(probe), "-Root", str(ROOT), "-Python", sys.executable, "-Child", str(child), "-Marker", str(marker)],
                    capture_output=True, text=True, encoding="utf-8", timeout=15, creationflags=subprocess.CREATE_NO_WINDOW)
                self.assertEqual(result.returncode, 0, result.stderr)
                observed = json.loads(result.stdout)
                self.assertTrue(observed["timed_out"], observed)
                self.assertTrue(observed["owned_absent"], observed)
                self.assertEqual(observed["stdout"].strip(), "\u2713 native", observed)
                self.assertLess(observed["duration_ms"], 5000, observed)
                self.assertIsNone(unrelated.poll(), "Unrelated process was terminated")
            finally:
                if unrelated.poll() is None:
                    unrelated.kill()
                unrelated.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
