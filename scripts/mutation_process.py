"""Bounded subprocess ownership and live diagnostics for mutation attempts.

Parser input stays byte-for-byte intact in the returned stdout/stderr. Console
output is independently bounded and redacted. Windows children enter a Job
Object before the command starts; POSIX children own a new process group.
"""
from __future__ import annotations

import contextlib
import ctypes
import hashlib
import json
import os
import pathlib
import queue
import re
import signal
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Sequence


MAX_CAPTURE_BYTES = 64 * 1024 * 1024
MAX_CONSOLE_BYTES = 2 * 1024 * 1024
MAX_HEARTBEAT_BYTES = 512 * 1024
HEARTBEAT_SECONDS = 20.0
CLEANUP_SECONDS = 10.0


class ProcessError(RuntimeError):
    def __init__(self, message: str, receipt: dict):
        super().__init__(message)
        self.receipt = receipt


def redact(value: str, environment: dict[str, str] | None = None) -> str:
    for key, secret in (environment or os.environ).items():
        if re.search(r"TOKEN|PASSWORD|SECRET|API_KEY", key, re.I) and len(secret) >= 4:
            value = value.replace(secret, "[redacted]")
    return re.sub(
        r"(?i)((?:token|password|secret|api[_-]?key)\s*[=:]\s*)[^\s&]+",
        r"\1[redacted]", value,
    )


class WindowsJob:
    """A handle-owned kill-on-close job, never an image-name process kill."""
    def __init__(self) -> None:
        from ctypes import wintypes

        class BasicLimits(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                        ("flags", wintypes.DWORD), ("minimum", ctypes.c_size_t),
                        ("maximum", ctypes.c_size_t), ("active", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling", wintypes.DWORD)]

        class IoCounters(ctypes.Structure):
            _fields_ = [(name, ctypes.c_uint64) for name in
                        ("read_ops", "write_ops", "other_ops", "read", "write", "other")]

        class ExtendedLimits(ctypes.Structure):
            _fields_ = [("basic", BasicLimits), ("io", IoCounters),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]

        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.kernel.CreateJobObjectW.restype = wintypes.HANDLE
        self.kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self.kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.kernel.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self.kernel.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.handle = self.kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.kernel.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            self.close()
            raise ctypes.WinError(ctypes.get_last_error())

    def assign(self, process: subprocess.Popen) -> None:
        if not self.kernel.AssignProcessToJobObject(self.handle, int(process._handle)):
            raise ctypes.WinError(ctypes.get_last_error())

    def stop(self) -> None:
        if not self.kernel.TerminateJobObject(self.handle, 125):
            raise ctypes.WinError(ctypes.get_last_error())
        class Accounting(ctypes.Structure):
            _fields_ = [("user", ctypes.c_int64), ("kernel", ctypes.c_int64),
                        ("period_user", ctypes.c_int64), ("period_kernel", ctypes.c_int64),
                        ("page_faults", ctypes.c_uint32), ("total", ctypes.c_uint32),
                        ("active", ctypes.c_uint32), ("terminated", ctypes.c_uint32)]
        deadline = time.monotonic() + CLEANUP_SECONDS
        while True:
            accounting = Accounting()
            if not self.kernel.QueryInformationJobObject(self.handle, 1, ctypes.byref(accounting), ctypes.sizeof(accounting), None):
                raise ctypes.WinError(ctypes.get_last_error())
            if not accounting.active:
                break
            if time.monotonic() >= deadline:
                raise OSError("owned job still has live descendants after cleanup deadline")
            time.sleep(0.02)

    def close(self) -> None:
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


def _write_receipt(path: pathlib.Path | None, receipt: dict) -> None:
    if path is not None:
        temporary = path.with_suffix(".tmp")
        temporary.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        temporary.replace(path)


def run(
    argv: Sequence[str], *, cwd: pathlib.Path, env: dict[str, str] | None = None,
    timeout_seconds: float = 1800, log_root: pathlib.Path | None = None,
    label: str = "process", progress: Callable[[], dict] | None = None,
    heartbeat_seconds: float = HEARTBEAT_SECONDS,
    max_capture_bytes: int = MAX_CAPTURE_BYTES,
) -> subprocess.CompletedProcess[str]:
    if timeout_seconds <= 0 or heartbeat_seconds <= 0:
        raise ValueError("process deadline and heartbeat must be positive")
    started = time.monotonic()
    receipt = {"schema_version": 1, "kind": "sorotte-owned-process", "phase": label,
               "command": [redact(arg, env) for arg in argv], "cwd": str(cwd),
               "status": "incomplete", "elapsed_seconds": 0, "returncode": None,
               "cleanup": {"status": "pending", "ownership": "job-object" if os.name == "nt" else "process-group"}}
    receipt_path = None
    log = None
    if log_root is not None:
        log_root.mkdir(parents=True, exist_ok=False)
        receipt_path = log_root / "process.json"
        log = (log_root / "console.log").open("x", encoding="utf-8")
    _write_receipt(receipt_path, receipt)
    messages: queue.Queue = queue.Queue(maxsize=256)
    captures = {"stdout": bytearray(), "stderr": bytearray()}
    console_bytes = 0
    heartbeat_bytes = 0
    process = None
    job = None
    previous_signals = {}
    interrupted = threading.Event()
    readers: list[threading.Thread] = []
    stop_readers = threading.Event()
    display_pending = {channel: bytearray() for channel in captures}
    discard_line = set()
    failure = None

    def emit(value: str, *, heartbeat: bool = False) -> None:
        nonlocal console_bytes, heartbeat_bytes
        value = redact(value, env)
        remaining = max(0, MAX_HEARTBEAT_BYTES - heartbeat_bytes if heartbeat else MAX_CONSOLE_BYTES - console_bytes)
        encoded = value.encode("utf-8", errors="replace")[:remaining]
        if encoded:
            rendered = encoded.decode("utf-8", errors="replace")
            print(rendered, end="", flush=True, file=sys.stderr)
            if log:
                log.write(rendered)
                log.flush()
            if heartbeat:
                heartbeat_bytes += len(encoded)
            else:
                console_bytes += len(encoded)

    def enqueue(value) -> bool:
        while not stop_readers.is_set():
            try:
                messages.put(value, timeout=0.1)
                return True
            except queue.Full:
                continue
        return False

    def reader(stream, channel: str) -> None:
        try:
            while data := stream.read1(4096):
                if not enqueue((channel, data)):
                    return
        finally:
            enqueue((channel, None))

    def display(channel: str, data: bytes, *, final: bool = False) -> None:
        # Redact complete lines, so a secret split across pipe reads cannot leak.
        pending = display_pending[channel]
        pending.extend(data)
        while b"\n" in pending:
            line, _, rest = pending.partition(b"\n")
            pending[:] = rest
            if channel not in discard_line:
                emit((line + b"\n").decode("utf-8", errors="replace"))
            discard_line.discard(channel)
        if len(pending) > 65536:
            pending.clear()
            discard_line.add(channel)
        if final and pending and channel not in discard_line:
            emit(pending.decode("utf-8", errors="replace"))
            pending.clear()

    try:
        if threading.current_thread() is threading.main_thread():
            for signum in (signal.SIGINT, signal.SIGTERM):
                previous_signals[signum] = signal.signal(signum, lambda *_: interrupted.set())
        options = dict(cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                       stdin=subprocess.DEVNULL)
        command = list(argv)
        if os.name == "nt":
            job = WindowsJob()
            # Wait for our parent to assign ownership before creating descendants.
            gate = "import json,subprocess,sys; sys.stdin.buffer.read(1); sys.exit(subprocess.call(json.loads(sys.argv[1]),stdin=subprocess.DEVNULL))"
            command = [sys.executable, "-c", gate, json.dumps(list(argv))]
            options.update(stdin=subprocess.PIPE, creationflags=subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP)
        else:
            options["start_new_session"] = True
        process = subprocess.Popen(command, **options)
        receipt["pid"] = process.pid
        if job:
            job.assign(process)
            process.stdin.write(b"g")
            process.stdin.flush()
            process.stdin.close()
        for channel in captures:
            thread = threading.Thread(target=reader, args=(getattr(process, channel), channel), daemon=True)
            thread.start()
            readers.append(thread)
        emit(f"[{label}] started: {' '.join(receipt['command'])}\n", heartbeat=True)
        next_heartbeat = started + heartbeat_seconds
        closed = set()
        while len(closed) != 2 or process.poll() is None:
            now = time.monotonic()
            receipt["elapsed_seconds"] = round(now - started, 3)
            if interrupted.is_set():
                receipt["status"] = "cancelled"
                raise RuntimeError("process was cancelled")
            if now - started >= timeout_seconds:
                receipt["status"] = "timeout"
                raise RuntimeError(f"process exceeded {timeout_seconds:g}s deadline; cleanup reserve is {CLEANUP_SECONDS:g}s")
            if now >= next_heartbeat:
                if progress:
                    receipt["progress"] = progress()
                concise = {key: value for key, value in receipt.get("progress", {}).items() if key not in {"pending", "failing"}}
                emit(f"[{label}] {receipt['elapsed_seconds']}s {json.dumps(concise, sort_keys=True)}\n", heartbeat=True)
                _write_receipt(receipt_path, receipt)
                next_heartbeat = now + heartbeat_seconds
            try:
                channel, data = messages.get(timeout=min(0.1, heartbeat_seconds))
            except queue.Empty:
                continue
            if data is None:
                closed.add(channel)
                display(channel, b"", final=True)
                continue
            if len(captures[channel]) + len(data) > max_capture_bytes:
                receipt["status"] = "output-limit"
                raise RuntimeError(f"{channel} exceeded the bounded {max_capture_bytes}-byte parser input limit")
            captures[channel].extend(data)
            # Inventory JSON remains exact in memory, but is not noisy console progress.
            if "--json" not in argv and not ("--list" in argv and channel == "stdout"):
                display(channel, data)
        receipt["returncode"] = process.returncode
        receipt["status"] = "completed"
    except (OSError, RuntimeError, KeyboardInterrupt) as error:
        failure = error
        if isinstance(error, KeyboardInterrupt):
            receipt["status"] = "cancelled"
        receipt["error"] = redact(str(error), env)
    finally:
        cleanup_errors = []
        stop_readers.set()
        if process is not None:
            try:
                if job:
                    job.stop()
                else:
                    with contextlib.suppress(ProcessLookupError):
                        os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=CLEANUP_SECONDS)
            except (OSError, subprocess.TimeoutExpired) as error:
                cleanup_errors.append(str(error))
                # The gating helper has no descendants if assignment failed.
                with contextlib.suppress(OSError):
                    process.kill()
            for thread in readers:
                thread.join(timeout=0.5)
            for channel in captures:
                if getattr(process, channel):
                    getattr(process, channel).close()
        if job:
            job.close()
        for signum, previous in previous_signals.items():
            signal.signal(signum, previous)
        receipt["cleanup"]["status"] = "failed" if cleanup_errors else "passed"
        receipt["cleanup"]["errors"] = cleanup_errors
        receipt["elapsed_seconds"] = round(time.monotonic() - started, 3)
        receipt["capture"] = {channel: {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                              for channel, data in captures.items()}
        if log_root is not None:
            for channel, data in captures.items():
                (log_root / f"{channel}.txt").write_text(redact(data.decode("utf-8", errors="replace"), env), encoding="utf-8")
            if "--json" in argv or "--list" in argv:
                # These bytes are parser authority, separate from redacted diagnostics.
                (log_root / "parser.stdout").write_bytes(captures["stdout"])
        if progress:
            with contextlib.suppress(OSError, ValueError):
                receipt["progress"] = progress()
        _write_receipt(receipt_path, receipt)
        if log:
            log.close()
    if failure or cleanup_errors:
        raise ProcessError(str(failure or cleanup_errors), receipt)
    try:
        completed = subprocess.CompletedProcess(list(argv), receipt["returncode"],
                                                captures["stdout"].decode("utf-8"), captures["stderr"].decode("utf-8"))
    except UnicodeError as error:
        receipt["status"] = "invalid-output"
        _write_receipt(receipt_path, receipt)
        raise ProcessError(str(error), receipt) from error
    completed.execution = receipt
    return completed
