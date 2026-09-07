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


class ReceiptWorkerFailure(AssertionError):
    def __init__(self, reason, diagnostics):
        self.diagnostics = diagnostics
        super().__init__(reason + "\nReceipt worker diagnostics: " + json.dumps(diagnostics, sort_keys=True))


class ReceiptWorkers:
    """Supervise this fixture's direct children without serial waits or pipe backpressure."""

    def __init__(self, root):
        self.root = root
        self.processes = {}
        self.started = time.monotonic()
        self.cleanup = None

    def __enter__(self):
        return self

    def start(self, name, command):
        if name in self.processes or self.cleanup is not None:
            raise ValueError("Receipt worker name reused or fixture already closed")
        with (self.root / f"{name}.stdout").open("xb") as stdout, (self.root / f"{name}.stderr").open("xb") as stderr:
            self.processes[name] = subprocess.Popen(
                command, stdout=stdout, stderr=stderr,
                creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0,
            )

    def close(self):
        if self.cleanup is not None:
            return
        self.cleanup = {"killed": [], "errors": []}
        cleanup_deadline = time.monotonic() + 10
        for name, process in self.processes.items():
            if process.poll() is None:
                try:
                    process.kill()  # Only the child bound to this retained Popen handle.
                    self.cleanup["killed"].append(name)
                except OSError as error:
                    self.cleanup["errors"].append(f"{name}: kill: {error}")
        for name, process in self.processes.items():
            try:
                process.wait(timeout=max(0, cleanup_deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                self.cleanup["errors"].append(f"{name}: did not exit within shared cleanup deadline")

    def diagnostics(self):
        workers = {}
        for name, process in self.processes.items():
            observation = {"pid": process.pid, "exit": process.poll(), "alive": process.poll() is None}
            for suffix in ("stdout", "stderr", "progress.json"):
                path = self.root / f"{name}.{suffix}"
                try:
                    observation[suffix] = path.read_text(encoding="utf-8", errors="backslashreplace")
                except OSError as error:
                    observation[suffix] = f"unavailable: {error}"
            workers[name] = observation
        return {"elapsed_seconds": time.monotonic() - self.started, "workers": workers, "cleanup": self.cleanup}

    def wait(self, deadline):
        reason = None
        while True:
            exits = {name: process.poll() for name, process in self.processes.items()}
            failed = [name for name, code in exits.items() if code not in (None, 0)]
            if failed:
                reason = "Receipt worker failed: " + ", ".join(failed)
                break
            if time.monotonic() >= deadline:
                reason = "Receipt workers exceeded the shared outer deadline"
                break
            if all(code == 0 for code in exits.values()):
                break
            time.sleep(0.01)
        self.close()
        diagnostics = self.diagnostics()
        if reason is None and any(worker["stderr"] for worker in diagnostics["workers"].values()):
            reason = "Receipt worker wrote unexpected stderr"
        if reason is not None or self.cleanup["errors"]:
            raise ReceiptWorkerFailure(reason or "Receipt worker cleanup failed", diagnostics)
        return diagnostics

    def __exit__(self, kind, error, traceback):
        self.close()
        if error is not None and not isinstance(error, ReceiptWorkerFailure):
            error.add_note("Receipt worker diagnostics: " + json.dumps(self.diagnostics(), sort_keys=True))


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
$readyClock=[Diagnostics.Stopwatch]::StartNew()
$workClock=$null
$writes=0
function Save-Progress {
    $workMs=if ($null -eq $workClock) { 0 } else { $workClock.ElapsedMilliseconds }
    [IO.File]::WriteAllText((Join-Path $Output 'writer.progress.json'),(@{writes=$writes;readiness_ms=$readyClock.ElapsedMilliseconds;work_ms=$workMs} | ConvertTo-Json -Compress))
}
try {
    Save-Progress
    Save-Receipt
    [IO.File]::WriteAllText((Join-Path $Output 'ready'),'ready')
    while (-not [IO.File]::Exists((Join-Path $Output 'reader-ready'))) {
        if ($readyClock.Elapsed.TotalSeconds -gt 20) { throw 'Reader did not start' }
        Start-Sleep -Milliseconds 1
    }
    $readyClock.Stop()
    $workClock=[Diagnostics.Stopwatch]::StartNew()
    for ($number=1;$number -le 600;$number++) {
        if ($workClock.Elapsed.TotalSeconds -gt 60) { throw 'Publisher work deadline exceeded' }
        $receipt.sequence=$number; Save-Receipt
        $writes=$number
        if ($number % 50 -eq 0) { Save-Progress }
    }
    [IO.File]::WriteAllText((Join-Path $Output 'done'),'done')
    '{"writes":600}'
} finally {
    try { Save-Progress } catch { [Console]::Error.WriteLine("Writer progress unavailable: $_") }
}
''', encoding="utf-8")
        reader_path = self.root / "reader.ps1"
        reader_path.write_text(r'''
param([string]$Helper,[string]$Output)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. $Helper
$readyClock=[Diagnostics.Stopwatch]::StartNew()
$workClock=$null
$reads=0; $versions=@{}; $lastSequence=$null
function Save-Progress {
    $workMs=if ($null -eq $workClock) { 0 } else { $workClock.ElapsedMilliseconds }
    [IO.File]::WriteAllText((Join-Path $Output 'reader.progress.json'),(@{reads=$reads;versions=$versions.Count;last_sequence=$lastSequence;readiness_ms=$readyClock.ElapsedMilliseconds;work_ms=$workMs} | ConvertTo-Json -Compress))
}
try {
    Save-Progress
    while (-not [IO.File]::Exists((Join-Path $Output 'ready'))) {
        if ($readyClock.Elapsed.TotalSeconds -gt 20) { throw 'Writer did not start' }
        Start-Sleep -Milliseconds 1
    }
    [IO.File]::WriteAllText((Join-Path $Output 'reader-ready'),'ready')
    $readyClock.Stop()
    $workClock=[Diagnostics.Stopwatch]::StartNew()
    $nextProgressMs=1000
    while (-not [IO.File]::Exists((Join-Path $Output 'done'))) {
        if ($workClock.Elapsed.TotalSeconds -gt 60) { throw 'Publisher did not finish' }
        $receipt=Read-NativeRunnerReceipt -Path (Join-Path $Output 'host-run.json')
        if ($receipt.instance -cne '00000000-0000-0000-0000-000000000123' -or $receipt.source_sha -cne ('a'*40) -or $receipt.body -cne ('payload'*150)) { throw 'Incomplete or wrong receipt' }
        $versions[[string]$receipt.sequence]=$true; $reads++; $lastSequence=$receipt.sequence
        if ($workClock.ElapsedMilliseconds -ge $nextProgressMs) { Save-Progress; $nextProgressMs=$workClock.ElapsedMilliseconds+1000 }
    }
    @{reads=$reads;versions=$versions.Count} | ConvertTo-Json -Compress
} finally {
    try { Save-Progress } catch { [Console]::Error.WriteLine("Reader progress unavailable: $_") }
}
''', encoding="utf-8")
        # One outer budget covers both workers, including PowerShell startup;
        # readiness never consumes either worker's 60-second publication budget.
        deadline = time.monotonic() + 90
        with ReceiptWorkers(self.root) as workers:
            workers.start("writer", [*POWERSHELL, str(writer_path), "-Controller", str(ROOT / "scripts/native-runner-sandbox.ps1"), "-Output", str(self.root)])
            workers.start("reader", [*POWERSHELL, str(reader_path), "-Helper", str(ROOT / "scripts/native-runner-receipt.ps1"), "-Output", str(self.root)])
            result = workers.wait(deadline)["workers"]
            self.assertEqual(json.loads(result["writer"]["stdout"])["writes"], 600)
            observed = json.loads(result["reader"]["stdout"])
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


class ReceiptWorkerSupervisionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def command(self, name, body):
        script = self.root / f"{name}.py"
        script.write_text("from pathlib import Path\nimport json, sys, time\nroot = Path(sys.argv[1])\n" + body, encoding="utf-8")
        return [sys.executable, "-B", "-u", str(script), str(self.root)]

    def test_slower_valid_worker_progress_is_awaited_after_peer_finishes(self):
        writer = self.command("writer", """
for sequence in range(4):
    (root / 'writer.progress.json').write_text(json.dumps({'sequence': sequence}))
    time.sleep(0.05)
print('writer complete')
""")
        reader = self.command("reader", "print('reader complete')\n")
        with ReceiptWorkers(self.root) as workers:
            deadline = time.monotonic() + 15
            workers.start("writer", writer)
            workers.start("reader", reader)
            result = workers.wait(deadline)
        self.assertEqual(result["cleanup"], {"killed": [], "errors": []})
        self.assertTrue(all(worker["exit"] == 0 and not worker["alive"] for worker in result["workers"].values()))
        self.assertEqual(json.loads(result["workers"]["writer"]["progress.json"])["sequence"], 3)
        self.assertEqual(result["workers"]["writer"]["stdout"], "writer complete\n")
        self.assertEqual(result["workers"]["reader"]["stdout"], "reader complete\n")

    def test_reader_failure_reports_both_streams_and_stops_hanging_writer(self):
        writer = self.command("writer", """
print('writer waiting')
(root / 'writer.progress.json').write_text('{"sequence":17}')
(root / 'writer-ready').write_text('ready')
while True: time.sleep(1)
""")
        reader = self.command("reader", """
deadline = time.monotonic() + 10
while not (root / 'writer-ready').exists():
    if time.monotonic() >= deadline: raise RuntimeError('writer never ready')
    time.sleep(0.01)
print('reader observed sequence 17')
(root / 'reader.progress.json').write_text('{"sequence":17}')
print('reader sentinel failure', file=sys.stderr)
sys.exit(7)
""")
        with ReceiptWorkers(self.root) as workers:
            deadline = time.monotonic() + 15
            workers.start("writer", writer)
            workers.start("reader", reader)
            with self.assertRaisesRegex(ReceiptWorkerFailure, "Receipt worker failed: reader") as failure:
                workers.wait(deadline)
        result = failure.exception.diagnostics
        self.assertEqual(result["cleanup"], {"killed": ["writer"], "errors": []})
        self.assertEqual(result["workers"]["reader"]["exit"], 7)
        self.assertIn("writer waiting", str(failure.exception))
        self.assertIn("reader observed sequence 17", str(failure.exception))
        self.assertIn("reader sentinel failure", str(failure.exception))
        for worker in result["workers"].values():
            self.assertFalse(worker["alive"])
            self.assertEqual(json.loads(worker["progress.json"])["sequence"], 17)

    def test_completed_workers_cannot_pass_an_expired_shared_deadline(self):
        with ReceiptWorkers(self.root) as workers:
            for name in ("writer", "reader"):
                workers.start(name, self.command(name, f"print('{name} complete')\n"))
            setup_deadline = time.monotonic() + 10
            for process in workers.processes.values():
                process.wait(timeout=max(0, setup_deadline - time.monotonic()))
            with self.assertRaisesRegex(ReceiptWorkerFailure, "shared outer deadline") as failure:
                workers.wait(time.monotonic() - 1)
        result = failure.exception.diagnostics
        self.assertEqual(result["cleanup"], {"killed": [], "errors": []})
        self.assertTrue(all(worker["exit"] == 0 and not worker["alive"] for worker in result["workers"].values()))

    def test_shared_deadline_reaps_both_ready_hangs_and_retains_diagnostics(self):
        with ReceiptWorkers(self.root) as workers:
            for name in ("writer", "reader"):
                workers.start(name, self.command(name, f"""
print('{name} ready')
(root / '{name}.progress.json').write_text('{{"ready":true}}')
(root / '{name}-ready').write_text('ready')
while True: time.sleep(1)
"""))
            readiness_deadline = time.monotonic() + 10
            while not all((self.root / f"{name}-ready").exists() for name in workers.processes):
                self.assertTrue(all(process.poll() is None for process in workers.processes.values()), workers.diagnostics())
                self.assertLess(time.monotonic(), readiness_deadline, workers.diagnostics())
                time.sleep(0.01)
            with self.assertRaisesRegex(ReceiptWorkerFailure, "shared outer deadline") as failure:
                workers.wait(time.monotonic() + 0.1)
        result = failure.exception.diagnostics
        self.assertEqual(result["cleanup"], {"killed": ["writer", "reader"], "errors": []})
        for name, worker in result["workers"].items():
            self.assertFalse(worker["alive"])
            self.assertIsNotNone(worker["exit"])
            self.assertIn(f"{name} ready", str(failure.exception))
            self.assertTrue(json.loads(worker["progress.json"])["ready"])


if __name__ == "__main__":
    unittest.main()
