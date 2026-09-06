"""Exercise real watchdog process ownership without a guest or network request."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
INSTANCE = "00000000-0000-0000-0000-000000000123"
POWERSHELL = ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]


@unittest.skipUnless(sys.platform == "win32", "Windows watchdog ownership and deadline canaries")
class NativeRunnerWatchdogTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.scripts = self.root / "scripts"
        self.scripts.mkdir()
        for name in ("native-runner-watchdog.ps1", "native-runner-receipt.ps1", "native-runner-owner.ps1"):
            (self.scripts / name).write_bytes((ROOT / "scripts" / name).read_bytes())
        self.run_root = self.root / "target/verification/native-runners" / INSTANCE
        self.run_root.mkdir(parents=True)
        (self.run_root / "host-run.json").write_text(json.dumps({
            "instance": INSTANCE, "repository": "ropbet-radbyt/sorotte", "runner_name": "sorotte-sandbox-" + INSTANCE,
            "sandbox_stopped": False, "runner_removed": False,
        }), encoding="utf-8")
        (self.scripts / "native-runner-sandbox.ps1").write_text(r'''
param([switch]$CleanupOnly,[Guid]$InstanceId)
$root=Split-Path -Parent $PSScriptRoot
$owner=Get-Content -LiteralPath (Join-Path $root 'owner.json') -Raw | ConvertFrom-Json
if (-not $CleanupOnly -or $InstanceId.ToString() -cne '00000000-0000-0000-0000-000000000123') { throw 'Unowned recovery invocation' }
@{cleanup_called=$true;owner_alive=$null -ne (Get-Process -Id $owner.pid -ErrorAction SilentlyContinue)} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'cleanup.json')
''', encoding="utf-8")

    def child(self, script: Path, *arguments: str) -> subprocess.Popen:
        child = subprocess.Popen([*POWERSHELL, str(script), *arguments], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 text=True, creationflags=subprocess.CREATE_NO_WINDOW)

        def stop_owned():
            if child.poll() is None:
                child.terminate()
            child.communicate(timeout=10)

        self.addCleanup(stop_owned)
        return child

    def owner(self, name: str = "owner") -> tuple[subprocess.Popen, str, str]:
        script = self.root / (name + ".ps1")
        ready = self.root / (name + "-ready")
        script.write_text("param([string]$Ready)\n[IO.File]::WriteAllText($Ready,(Get-Process -Id $PID).StartTime.ToUniversalTime().ToString('o'))\nStart-Sleep -Seconds 60\n", encoding="utf-8")
        child = self.child(script, "-Ready", str(ready))
        deadline = time.monotonic() + 10
        while not ready.exists() and child.poll() is None and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(ready.exists(), "Harmless owned controller did not start")
        identity = ready.read_text()
        digest = hashlib.sha256(subprocess.list2cmdline(child.args).encode()).hexdigest()
        if name == "owner":
            (self.root / "owner.json").write_text(json.dumps({"pid": child.pid}), encoding="utf-8")
        return child, identity, digest

    def watchdog(self, owner: subprocess.Popen, started: str, command: str, *, deadline: bool = True) -> subprocess.Popen:
        path = self.scripts / "native-runner-watchdog.ps1"
        if deadline:
            source = path.read_text()
            clock = "$clock=[Diagnostics.Stopwatch]::StartNew()"
            self.assertEqual(source.count(clock), 1)
            # Only time is overridden in this isolated copy. The actual watchdog
            # ownership, termination, recovery and diagnostic paths execute.
            path.write_text(source.replace(clock, "$clock=[pscustomobject]@{Elapsed=[pscustomobject]@{TotalMinutes=1000}}"), encoding="utf-8")
        return self.child(path, "-ControllerPid", str(owner.pid), "-ControllerStartUtc", started,
                          "-ControllerCommandSha256", command, "-InstanceId", INSTANCE, "-TimeoutMinutes", "10")

    def test_deadline_stops_verified_controller_before_cleanup(self):
        owner, started, command = self.owner()
        watchdog = self.watchdog(owner, started, command)
        _, stderr = watchdog.communicate(timeout=20)
        self.assertEqual(watchdog.returncode, 0, stderr)
        self.assertIsNotNone(owner.poll(), "The controller can still resume after cleanup")
        cleanup = json.loads((self.root / "cleanup.json").read_text(encoding="utf-8-sig"))
        self.assertTrue(cleanup["cleanup_called"])
        self.assertFalse(cleanup["owner_alive"], "Cleanup raced a live controller")
        diagnostic = json.loads((self.run_root / "watchdog-observation.json").read_text())
        self.assertEqual(diagnostic["status"], "watchdog-completed")
        self.assertEqual(diagnostic["instance"], INSTANCE)

    def test_already_clean_receipt_records_completion_without_cleanup(self):
        owner, started, command = self.owner()
        receipt_path = self.run_root / "host-run.json"
        receipt = json.loads(receipt_path.read_text())
        receipt.update(sandbox_stopped=True, runner_removed=True, tokens_removed=True)
        receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
        watchdog = self.watchdog(owner, started, command)
        _, stderr = watchdog.communicate(timeout=20)
        self.assertEqual(watchdog.returncode, 0, stderr)
        self.assertIsNone(owner.poll(), "Completed cleanup must not terminate a live controller")
        self.assertFalse((self.root / "cleanup.json").exists())
        diagnostic = json.loads((self.run_root / "watchdog-observation.json").read_text())
        self.assertEqual(diagnostic["status"], "watchdog-completed")
        self.assertEqual(diagnostic["instance"], INSTANCE)

    def assert_failed_cleanup_is_not_completion(self, failure: str, expected: str):
        cleanup = self.scripts / "native-runner-sandbox.ps1"
        cleanup.write_text(cleanup.read_text() + "\n" + failure, encoding="utf-8")
        owner, started, command = self.owner()
        watchdog = self.watchdog(owner, started, command)
        _, stderr = watchdog.communicate(timeout=20)
        self.assertNotEqual(watchdog.returncode, 0, stderr)
        self.assertIsNotNone(owner.poll())
        self.assertTrue((self.root / "cleanup.json").exists())
        diagnostic = json.loads((self.run_root / "watchdog-observation.json").read_text())
        self.assertEqual(diagnostic["status"], "watchdog-failed")
        self.assertIn(expected, diagnostic["error"])

    def test_cleanup_exception_is_not_reported_as_completion(self):
        self.assert_failed_cleanup_is_not_completion("throw 'injected cleanup failure'", "injected cleanup failure")

    def test_cleanup_nonzero_script_exit_is_not_reported_as_completion(self):
        self.assert_failed_cleanup_is_not_completion("exit 17", "Native watchdog recovery script failed")

    def test_owner_death_recovers_without_affecting_another_process(self):
        owner, started, command = self.owner()
        unrelated, _, _ = self.owner("unrelated")
        receipt = self.run_root / "host-run.json"
        valid_receipt = receipt.read_text()
        receipt.write_text('{"incomplete":', encoding="utf-8")
        watchdog = self.watchdog(owner, started, command, deadline=False)
        observation = self.run_root / "watchdog-observation.json"
        deadline = time.monotonic() + 10
        while not observation.exists() and watchdog.poll() is None and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(observation.exists(), "Watchdog did not bind and observe the live owner")
        self.assertEqual(json.loads(observation.read_text())["status"], "receipt-read-unavailable")
        receipt.write_text(valid_receipt, encoding="utf-8")
        owner.terminate()
        owner.communicate(timeout=10)
        _, stderr = watchdog.communicate(timeout=20)
        self.assertEqual(watchdog.returncode, 0, stderr)
        self.assertFalse(json.loads((self.root / "cleanup.json").read_text(encoding="utf-8-sig"))["owner_alive"])
        self.assertIsNone(unrelated.poll())

    def test_leftover_token_keeps_recovery_required_after_guest_and_runner_are_gone(self):
        owner, started, command = self.owner()
        receipt_path = self.run_root / "host-run.json"
        receipt = json.loads(receipt_path.read_text())
        receipt.update(sandbox_stopped=True, runner_removed=True, tokens_removed=False)
        receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
        watchdog = self.watchdog(owner, started, command)
        _, stderr = watchdog.communicate(timeout=20)
        self.assertEqual(watchdog.returncode, 0, stderr)
        self.assertIsNotNone(owner.poll(), "Watchdog accepted incomplete token cleanup")
        self.assertTrue((self.root / "cleanup.json").exists(), "Watchdog skipped required token recovery")

    def test_live_pid_with_stale_creation_or_wrong_command_is_refused(self):
        owner, started, command = self.owner()
        for field, bad_start, bad_command in (("creation", "2000-01-01T00:00:00.0000000Z", command), ("command", started, "0" * 64)):
            with self.subTest(field=field):
                # Restore the production script for each independent attempt.
                (self.scripts / "native-runner-watchdog.ps1").write_bytes((ROOT / "scripts/native-runner-watchdog.ps1").read_bytes())
                watchdog = self.watchdog(owner, bad_start, bad_command)
                _, stderr = watchdog.communicate(timeout=20)
                self.assertNotEqual(watchdog.returncode, 0)
                self.assertIn(field + " identity mismatch", stderr)
                self.assertIsNone(owner.poll(), "A stale identity terminated the live process")
                self.assertFalse((self.root / "cleanup.json").exists())
                diagnostic = json.loads((self.run_root / "watchdog-observation.json").read_text())
                self.assertEqual(diagnostic["status"], "watchdog-failed")

    def test_retained_handle_revalidates_creation_and_command_before_termination(self):
        owner, started, command = self.owner()
        script = self.root / "revalidation.ps1"
        script.write_text(r'''
param([string]$Helper,[int]$OwnerPid,[string]$Started,[string]$Command)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. $Helper
$owner=Open-NativeControllerOwner -ControllerPid $OwnerPid -StartedUtc $Started -CommandSha256 $Command
try {
    $creationRejected=$false; $commandRejected=$false
    try { Stop-NativeControllerOwner -Owner $owner -StartedUtc '2000-01-01T00:00:00.0000000Z' -CommandSha256 $Command }
    catch { $creationRejected=$_.Exception.Message.Contains('creation identity mismatch') }
    try { Stop-NativeControllerOwner -Owner $owner -StartedUtc $Started -CommandSha256 ('0'*64) }
    catch { $commandRejected=$_.Exception.Message.Contains('command identity mismatch') }
    @{creation_rejected=$creationRejected;command_rejected=$commandRejected;still_alive=-not $owner.HasExited} | ConvertTo-Json -Compress
} finally { $owner.Dispose() }
''', encoding="utf-8")
        probe = self.child(script, "-Helper", str(self.scripts / "native-runner-owner.ps1"), "-OwnerPid", str(owner.pid),
                           "-Started", started, "-Command", command)
        stdout, stderr = probe.communicate(timeout=20)
        self.assertEqual(probe.returncode, 0, stderr)
        self.assertEqual(json.loads(stdout), {"creation_rejected": True, "command_rejected": True, "still_alive": True})
        self.assertIsNone(owner.poll())


if __name__ == "__main__":
    unittest.main()
