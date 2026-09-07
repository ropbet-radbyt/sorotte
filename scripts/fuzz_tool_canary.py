#!/usr/bin/env python3
"""Real pinned cargo-fuzz build/run/tmin/replay canary on a tiny deliberate defect."""
from __future__ import annotations
import argparse
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time

try:
    from .verification_tools import ROOT, digest, identity, pins
except ImportError:
    from verification_tools import ROOT, digest, identity, pins


def execute(argv: list[str], cwd: Path, stream, timeout: int) -> int:
    """Own cargo and all of its descendants, including libFuzzer's tmin children."""
    process = subprocess.Popen(argv, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT,
                               stdin=subprocess.DEVNULL, start_new_session=True)
    try:
        return process.wait(timeout=timeout)
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=10)


def run(output: Path) -> dict:
    if output.exists(): raise ValueError("canary output must be fresh")
    output.mkdir(parents=True)
    record = {"schema_version": 1, "kind": "fuzz-tool-canary", "identity": None,
              "status": "incomplete", "commands": []}
    receipt = output / "receipt.json"
    started = time.monotonic()
    previous_termination = None
    def save(): receipt.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    def command(argv, expected_success=True):
        log = output / f"command-{len(record['commands'])}.log"
        record["commands"].append({"argv": argv, "status": "running"})
        save()
        try:
            with log.open("wb") as stream:
                exit_code = execute(argv, output, stream, 180)
            record["commands"][-1].update(exit_code=exit_code, status="completed")
        except BaseException as error:
            record["commands"][-1].update(status="timed_out" if isinstance(error, subprocess.TimeoutExpired)
                                          else "cancelled" if isinstance(error, KeyboardInterrupt) else "failed",
                                          error=str(error))
            raise
        finally:
            if log.is_file(): record["commands"][-1]["log_sha256"] = digest(log)
            save()
        if (exit_code == 0) != expected_success:
            raise ValueError(f"unexpected fuzz tool outcome: {log}")
        return log.read_text(encoding="utf-8", errors="replace")
    save()
    try:
        record["identity"] = identity()
        if os.name == "nt":
            raise ValueError("ASan fuzz canary requires the reviewed Linux nightly environment")
        if threading.current_thread() is threading.main_thread():
            def terminate(*_): raise KeyboardInterrupt("fuzz canary cancelled")
            previous_termination = signal.signal(signal.SIGTERM, terminate)
        spec = importlib.util.spec_from_file_location("sorotte_fuzz_runner_canary", ROOT / "fuzz/run_protocol_fuzz.py")
        runner = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = runner
        spec.loader.exec_module(runner)
        chain = pins()["tools"]["fuzz-toolchain"]
        version = subprocess.check_output(["cargo", "fuzz", "--version"], text=True, timeout=30).strip()
        if version != f"cargo-fuzz {pins()['tools']['cargo-fuzz']}": raise ValueError("wrong cargo-fuzz pin")
        record["cargo_fuzz_version"] = version
        record["fuzz_toolchain"] = chain
        fixture = output / "fuzz"
        fixture.mkdir()
        (fixture / "Cargo.toml").write_text('''[package]
name="sorotte-fuzz-tool-canary"
version="0.0.0"
edition="2024"
[package.metadata]
cargo-fuzz=true
[workspace]
[dependencies]
libfuzzer-sys="=0.4.13"
[[bin]]
name="protocol_line"
path="canary.rs"
test=false
doc=false
bench=false
''', encoding="utf-8")
        (fixture / "canary.rs").write_text('''#![no_main]
libfuzzer_sys::fuzz_target!(|bytes: &[u8]| {
    assert!(!bytes.starts_with(b"CANARY!"), "intentional minimizer canary");
});
''', encoding="utf-8")
        prefix = runner.cargo_fuzz_prefix(chain)
        command([*prefix, "build", "--fuzz-dir", "fuzz", "--sanitizer", "address", "protocol_line"])
        benign = output / "benign-input"
        original = output / "original-crash"
        minimized = output / "minimized-crash"
        benign.write_bytes(b"safe input")
        original.write_bytes(b"CANARY! preserve these original bytes independently")
        original_hash = digest(original)
        def replay(path):
            return [*prefix, "run", "--fuzz-dir", "fuzz", "--sanitizer", "address", "protocol_line", str(path), "--", "-runs=1", "-print_final_stats=1"]
        safe_log = command(replay(benign))
        statistics = runner.parse_final_statistics(safe_log)
        runner.validate_final_statistics(statistics)
        crash_log = command(replay(original), expected_success=False)
        if "intentional minimizer canary" not in crash_log:
            raise ValueError("failure did not exercise deliberate target defect")
        command(runner.minimization_command(chain, original, minimized))
        if digest(original) != original_hash: raise ValueError("minimizer overwrote original failing input")
        if not minimized.read_bytes().startswith(b"CANARY!") or minimized.stat().st_size >= original.stat().st_size:
            raise ValueError("minimized input lost the causal defect")
        replay_log = command(replay(minimized), expected_success=False)
        if "intentional minimizer canary" not in replay_log: raise ValueError("minimized defect did not replay")
        record.update(status="passed", original_sha256=original_hash, minimized_sha256=digest(minimized),
                      cargo_lock_sha256=digest(fixture / "Cargo.lock"),
                      benign_statistics=statistics)
        return record
    except BaseException as error:
        record.update(status="timed_out" if isinstance(error, subprocess.TimeoutExpired)
                      else "cancelled" if isinstance(error, KeyboardInterrupt) else "failed", error=str(error))
        raise
    finally:
        record["duration_seconds"] = round(time.monotonic() - started, 3)
        save()
        if previous_termination is not None:
            signal.signal(signal.SIGTERM, previous_termination)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        run(args.output.resolve())
        return 0
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(error, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__": raise SystemExit(main())
