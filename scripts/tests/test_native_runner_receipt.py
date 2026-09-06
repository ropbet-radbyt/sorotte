"""Real Windows I/O canaries for interrupted native-controller evidence."""
from __future__ import annotations

import ctypes
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[2]
POWERSHELL = ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]
INSTANCE = "00000000-0000-0000-0000-000000000123"
LOAD_PUBLISHER = r'''
param([string]$Controller,[string]$Output)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path (Split-Path -Parent $Controller) 'native-runner-receipt.ps1')
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($Controller,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Controller syntax failed' }
$functions=$ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Save-Receipt'},$false)
if ($functions.Count -ne 1) { throw 'Expected the actual production publisher' }
. ([ScriptBlock]::Create($functions[0].Extent.Text))
$receiptPath=Join-Path $Output 'host-run.json'
$receipt=[ordered]@{instance='00000000-0000-0000-0000-000000000123';source_sha=('a'*40);sequence=0;body=('payload'*150)}
'''


@unittest.skipUnless(sys.platform == "win32", "Windows atomic receipt and watchdog canaries")
class NativeRunnerReceiptTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def child(self, script: Path, *arguments: str) -> subprocess.Popen:
        process = subprocess.Popen([*POWERSHELL, str(script), *arguments], stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, creationflags=subprocess.CREATE_NO_WINDOW)

        def stop_owned():
            if process.poll() is None:
                process.terminate()
            process.communicate(timeout=10)

        self.addCleanup(stop_owned)
        return process

    def wait_file(self, path: Path, process: subprocess.Popen):
        deadline = time.monotonic() + 10
        while not path.exists() and process.poll() is None and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(path.exists(), f"Owned helper did not publish {path.name}; exit={process.poll()}")

    def successful_result(self, process: subprocess.Popen) -> str:
        stdout, stderr = process.communicate(timeout=25)
        self.assertEqual(process.returncode, 0, stderr)
        self.assertEqual(stderr, "")
        return stdout

    def test_real_controller_publisher_keeps_complete_receipts_visible_during_concurrent_reads(self):
        writer_path = self.root / "writer.ps1"
        writer_path.write_text(LOAD_PUBLISHER + r'''
Save-Receipt
[IO.File]::WriteAllText((Join-Path $Output 'ready'),'ready')
$clock=[Diagnostics.Stopwatch]::StartNew()
while (-not [IO.File]::Exists((Join-Path $Output 'reader-ready'))) {
    if ($clock.Elapsed.TotalSeconds -gt 10) { throw 'Reader did not start' }
    Start-Sleep -Milliseconds 1
}
for ($number=1;$number -le 600;$number++) { $receipt.sequence=$number; Save-Receipt }
[IO.File]::WriteAllText((Join-Path $Output 'done'),'done')
'{"writes":600}'
''', encoding="utf-8")
        reader_path = self.root / "reader.ps1"
        reader_path.write_text(r'''
param([string]$Helper,[string]$Output)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. $Helper
$clock=[Diagnostics.Stopwatch]::StartNew()
while (-not [IO.File]::Exists((Join-Path $Output 'ready'))) {
    if ($clock.Elapsed.TotalSeconds -gt 10) { throw 'Writer did not start' }
    Start-Sleep -Milliseconds 1
}
[IO.File]::WriteAllText((Join-Path $Output 'reader-ready'),'ready')
$reads=0; $versions=@{}
while (-not [IO.File]::Exists((Join-Path $Output 'done'))) {
    if ($clock.Elapsed.TotalSeconds -gt 20) { throw 'Publisher did not finish' }
    $receipt=Read-NativeRunnerReceipt -Path (Join-Path $Output 'host-run.json')
    if ($receipt.instance -cne '00000000-0000-0000-0000-000000000123' -or $receipt.source_sha -cne ('a'*40) -or $receipt.body -cne ('payload'*150)) { throw 'Incomplete or wrong receipt' }
    $versions[[string]$receipt.sequence]=$true; $reads++
}
@{reads=$reads;versions=$versions.Count} | ConvertTo-Json -Compress
''', encoding="utf-8")
        writer = self.child(writer_path, "-Controller", str(ROOT / "scripts/native-runner-sandbox.ps1"), "-Output", str(self.root))
        reader = self.child(reader_path, "-Helper", str(ROOT / "scripts/native-runner-receipt.ps1"), "-Output", str(self.root))
        self.assertEqual(json.loads(self.successful_result(writer))["writes"], 600)
        observed = json.loads(self.successful_result(reader))
        self.assertGreater(observed["reads"], 20)
        self.assertGreater(observed["versions"], 10)
        self.assertEqual(json.loads((self.root / "host-run.json").read_text())["sequence"], 600)
        self.assertEqual(list(self.root.glob("host-run.json.pending*")), [])

    def test_interrupted_real_publisher_preserves_last_complete_receipt(self):
        receipt_path = self.root / "host-run.json"
        original = json.dumps({"instance": INSTANCE, "sequence": "last-complete"}).encode()
        receipt_path.write_bytes(original)
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_void_p,
                                      ctypes.c_ulong, ctypes.c_ulong, ctypes.c_void_p]
        kernel.CreateFileW.restype = ctypes.c_void_p
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        kernel.CloseHandle.restype = ctypes.c_int
        # Deliberately omit FILE_SHARE_DELETE, holding publication at its atomic
        # replacement boundary while the owned writer has a complete temp file.
        handle = kernel.CreateFileW(str(receipt_path), 0x80000000, 3, None, 3, 0, None)
        self.assertNotEqual(handle, ctypes.c_void_p(-1).value, ctypes.get_last_error())
        try:
            script = self.root / "interrupted-writer.ps1"
            script.write_text(LOAD_PUBLISHER + "\nSave-Receipt\n", encoding="utf-8")
            writer = self.child(script, "-Controller", str(ROOT / "scripts/native-runner-sandbox.ps1"), "-Output", str(self.root))
            deadline = time.monotonic() + 10
            while not list(self.root.glob("host-run.json.pending*")) and writer.poll() is None and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(list(self.root.glob("host-run.json.pending*")), "Real publisher never reached the replacement boundary")
            time.sleep(0.2)
            self.assertIsNone(writer.poll(), "Publisher unexpectedly passed the incompatible sharing lock")
            writer.terminate()  # Only the child held by this Popen process handle.
            writer.communicate(timeout=10)
            self.assertEqual(receipt_path.read_bytes(), original)
        finally:
            kernel.CloseHandle(handle)
        probe = self.root / "read-retained.ps1"
        probe.write_text("param([string]$Helper,[string]$Receipt)\n. $Helper\nRead-NativeRunnerReceipt -Path $Receipt | ConvertTo-Json -Compress\n", encoding="utf-8")
        retained = self.child(probe, "-Helper", str(ROOT / "scripts/native-runner-receipt.ps1"), "-Receipt", str(receipt_path))
        self.assertEqual(json.loads(self.successful_result(retained))["sequence"], "last-complete")

    def test_watchdog_records_unavailable_receipt_and_keeps_observing_owner(self):
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ("native-runner-watchdog.ps1", "native-runner-receipt.ps1", "native-runner-owner.ps1"):
            (scripts / name).write_bytes((ROOT / "scripts" / name).read_bytes())
        (scripts / "native-runner-sandbox.ps1").write_text("throw 'Unexpected recovery: owned controller is alive'\n", encoding="utf-8")
        run_root = self.root / "target/verification/native-runners" / INSTANCE
        run_root.mkdir(parents=True)
        receipt = run_root / "host-run.json"
        receipt.write_text('{"interrupted":', encoding="utf-8")
        owner_script = self.root / "owner.ps1"
        owner_script.write_text("param([string]$Ready)\n[IO.File]::WriteAllText($Ready,(Get-Process -Id $PID).StartTime.ToUniversalTime().ToString('o'))\nStart-Sleep -Seconds 30\n", encoding="utf-8")
        owner = self.child(owner_script, "-Ready", str(self.root / "owner-ready"))
        self.wait_file(self.root / "owner-ready", owner)
        watchdog = self.child(scripts / "native-runner-watchdog.ps1", "-ControllerPid", str(owner.pid),
                              "-ControllerStartUtc", (self.root / "owner-ready").read_text(),
                              "-ControllerCommandSha256", hashlib.sha256(subprocess.list2cmdline(owner.args).encode()).hexdigest(),
                              "-InstanceId", INSTANCE, "-TimeoutMinutes", "10")
        self.wait_file(run_root / "watchdog-observation.json", watchdog)
        observation = json.loads((run_root / "watchdog-observation.json").read_text())
        self.assertEqual(observation["status"], "receipt-read-unavailable")
        self.assertEqual(observation["instance"], INSTANCE)
        self.assertIsNone(owner.poll())
        receipt.write_text(json.dumps({"instance": INSTANCE, "repository": "ropbet-radbyt/sorotte",
                                       "runner_name": "sorotte-sandbox-" + INSTANCE,
                                       "sandbox_stopped": True, "runner_removed": True}), encoding="utf-8")
        self.successful_result(watchdog)
        self.assertIsNone(owner.poll())


if __name__ == "__main__":
    unittest.main()
