from __future__ import annotations

import contextlib
import ctypes
import io
import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import mutation_process as process


class MutationProcessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sm-process-")
        self.root = pathlib.Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def run_process(self, code, **kwargs):
        return process.run([sys.executable, "-c", code], cwd=self.root,
                           log_root=self.root / "attempt", heartbeat_seconds=0.05, **kwargs)

    def assert_dead(self, pid):
        if os.name == "nt":
            from ctypes import wintypes
            kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
            kernel.OpenProcess.restype = wintypes.HANDLE
            kernel.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
            kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            handle = kernel.OpenProcess(0x1000, False, pid)
            if not handle:
                self.assertEqual(ctypes.get_last_error(), 87)  # no longer exists
                return
            try:
                code = wintypes.DWORD()
                self.assertTrue(kernel.GetExitCodeProcess(handle, ctypes.byref(code)))
                self.assertNotEqual(code.value, 259)  # STILL_ACTIVE
            finally:
                kernel.CloseHandle(handle)
        else:
            stat = pathlib.Path(f"/proc/{pid}/stat")
            if stat.exists():
                self.assertEqual(stat.read_text().split(")", 1)[1].split()[0], "Z")
            else:
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)

    def test_slow_process_streams_output_and_phase_heartbeats_before_completion(self):
        observed = io.StringIO()
        with contextlib.redirect_stderr(observed):
            result = self.run_process("import time; print('building fixture',flush=True); time.sleep(.22)", timeout_seconds=3,
                                      progress=lambda: {"phase": "build", "completed": 0, "remaining": 1})
        self.assertEqual(result.returncode, 0)
        self.assertIn("building fixture", observed.getvalue())
        self.assertIn('"phase": "build"', observed.getvalue())
        self.assertEqual(result.execution["cleanup"]["status"], "passed")
        self.assertEqual(json.loads((self.root / "attempt/process.json").read_text())["status"], "completed")

    def test_timeout_retains_partial_logs_and_terminates_owned_descendants(self):
        code = "import subprocess,sys,pathlib,time; child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); pathlib.Path('child.pid').write_text(str(child.pid)); print('last completed case',flush=True); time.sleep(30)"
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(process.ProcessError) as raised:
            self.run_process(code, timeout_seconds=0.7, progress=lambda: {"last_completed": "mutant-3", "pending": ["mutant-4"]})
        self.assertEqual(raised.exception.receipt["status"], "timeout")
        self.assertEqual(raised.exception.receipt["cleanup"]["status"], "passed")
        self.assertEqual(raised.exception.receipt["progress"]["last_completed"], "mutant-3")
        self.assertIn("last completed case", (self.root / "attempt/stdout.txt").read_text())
        self.assert_dead(int((self.root / "child.pid").read_text()))

    def test_cancel_handler_retains_cancelled_receipt_and_restores_handlers(self):
        actual_signal = signal.signal
        handlers = {}
        previous = signal.getsignal(signal.SIGTERM)
        def install(signum, handler):
            handlers[signum] = handler
            return actual_signal(signum, handler)
        timer = threading.Timer(0.3, lambda: handlers[signal.SIGTERM](signal.SIGTERM, None))
        timer.start()
        try:
            with mock.patch.object(process.signal, "signal", side_effect=install), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(process.ProcessError) as raised:
                    self.run_process("import time; print('active',flush=True); time.sleep(20)", timeout_seconds=4)
            self.assertEqual(raised.exception.receipt["status"], "cancelled")
            self.assertEqual(raised.exception.receipt["cleanup"]["status"], "passed")
            self.assertIs(signal.getsignal(signal.SIGTERM), previous)
        finally:
            timer.cancel()
            timer.join()

    def test_split_pipe_reads_cannot_expose_secret_in_diagnostics(self):
        environment = {**os.environ, "FIXTURE_API_TOKEN": "sensitive-canary-value"}
        code = "import os,time,sys; value=os.environ['FIXTURE_API_TOKEN']; sys.stdout.write(value[:8]);sys.stdout.flush();time.sleep(.1);print(value[8:],flush=True)"
        observed = io.StringIO()
        with contextlib.redirect_stderr(observed):
            result = self.run_process(code, env=environment, timeout_seconds=4)
        self.assertIn("sensitive-canary-value", result.stdout)
        self.assertNotIn("sensitive-canary-value", observed.getvalue())
        self.assertNotIn("sensitive", observed.getvalue())
        self.assertIn("[redacted]", (self.root / "attempt/console.log").read_text())
        self.assertIn("[redacted]", (self.root / "attempt/stdout.txt").read_text())

    def test_output_limit_fails_and_cleanup_still_runs(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(process.ProcessError) as raised:
            self.run_process("import sys; sys.stdout.write('x'*9000)", max_capture_bytes=1000, timeout_seconds=4)
        self.assertEqual(raised.exception.receipt["status"], "output-limit")
        self.assertEqual(raised.exception.receipt["cleanup"]["status"], "passed")

    def test_existing_attempt_cannot_be_overwritten_by_a_retry(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.run_process("print('first failure')", timeout_seconds=4)
        before = (self.root / "attempt/process.json").read_bytes()
        with self.assertRaises(FileExistsError):
            self.run_process("print('retry passed')", timeout_seconds=4)
        self.assertEqual((self.root / "attempt/process.json").read_bytes(), before)

    def test_verbose_output_budget_does_not_silence_phase_heartbeats(self):
        observed = io.StringIO()
        with mock.patch.object(process, "MAX_CONSOLE_BYTES", 8), contextlib.redirect_stderr(observed):
            self.run_process("import time;print('verbose output'*20,flush=True);time.sleep(.2)", timeout_seconds=3,
                             progress=lambda: {"phase": "still advancing"})
        self.assertIn('"phase": "still advancing"', observed.getvalue())


if __name__ == "__main__":
    unittest.main()
