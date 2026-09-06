#!/usr/bin/env python3
"""Run deterministic production scaling fixtures; timing comparisons are advisory."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time

from artifact_input import ArtifactInputError, require_int, sha256_file, strict_json_load, strict_json_loads
from mutation_process import ProcessError, redact, run as run_owned

SCHEMA = "sorotte-scaling-report-v1"
SAMPLE_SCHEMA = "sorotte-scaling-sample-v1"
FEATURES = "gui-semantic-smoke,live-python-interop"
ROOT = Path(__file__).resolve().parent.parent
MAX_SAMPLE_BYTES = 16 * 1024 * 1024
ENVIRONMENT_KEYS = ("PATH", "CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "CARGO_BUILD_JOBS",
                    "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "RUSTFLAGS", "RUSTDOCFLAGS", "CARGO_ENCODED_RUSTFLAGS",
                    "CARGO_ENCODED_RUSTDOCFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                    "CC", "CXX", "AR", "INCLUDE", "LIB", "LIBPATH", "TEMP", "TMP")


class ScalingError(ValueError):
    pass


class Attempt:
    """One immutable attempt namespace, with a live receipt until finalization."""

    def __init__(self, output: Path, argv: list[str]):
        if os.path.lexists(output):
            raise ScalingError("output must be fresh; preserve the previous report and use a new output path")
        self.output = output
        self.root = output.with_name(output.name + ".attempt")
        self.root.mkdir(parents=True, exist_ok=False)
        self.started = time.monotonic()
        replay = list(argv)
        for index, value in enumerate(replay):
            if value == "--output" and index + 1 < len(replay):
                replay[index + 1] = "FRESH_ATTEMPT_REPORT.json"
            elif value.startswith("--output="):
                replay[index] = "--output=FRESH_ATTEMPT_REPORT.json"
        self.record = {"schema_version": 1, "kind": "scaling-workload-attempt", "status": "incomplete",
                       "command": [redact(value) for value in argv], "cwd": str(ROOT),
                       "replay_command": [redact(value) for value in replay], "primary_failure": None,
                       "started_at_unix_seconds": time.time(), "elapsed_seconds": 0,
                       "environment": {key: redact(os.environ[key]) for key in ENVIRONMENT_KEYS if key in os.environ},
                       "source_before": None, "source_after": None, "commands": [], "observations": [],
                       "first_failure": None, "report": None}
        self.save()

    def save(self) -> None:
        self.record["elapsed_seconds"] = round(time.monotonic() - self.started, 3)
        if source := self.record["source_before"]:
            self.record["identity"] = {"source_sha": source["sha"], "working_source_sha256": source["working_source_sha256"]}
        cleanups = [entry["execution"]["cleanup"] for entry in self.record["commands"] if "execution" in entry]
        pending = any(entry["status"] == "incomplete" for entry in self.record["commands"])
        self.record["cleanup"] = {
            "status": "failed" if any(item["status"] == "failed" for item in cleanups) else "pending" if pending
                      else "passed" if cleanups and len(cleanups) == len(self.record["commands"]) else "unavailable",
            "commands": len(cleanups), "ownership": sorted({item["ownership"] for item in cleanups}),
        }
        temporary = self.root / "receipt.tmp"
        temporary.write_text(json.dumps(self.record, indent=2, allow_nan=False) + "\n", encoding="utf-8")
        temporary.replace(self.root / "receipt.json")

    def fail(self, phase: str, error: BaseException, entry: dict | None = None) -> None:
        if self.record["first_failure"] is None:
            self.record["primary_failure"] = f"{phase}: {redact(str(error))}"
            self.record["first_failure"] = {"phase": phase, "error": redact(str(error)),
                                            "elapsed_seconds": round(time.monotonic() - self.started, 3),
                                            "command": entry["command"] if entry else None,
                                            "diagnostics": entry["diagnostics"] if entry else None}

    def run(self, argv: list[str], *, root: Path, timeout: float, label: str, max_bytes: int) -> str:
        logs = self.root / f"command-{len(self.record['commands']) + 1:03d}"
        entry = {"phase": label, "command": [redact(value) for value in argv], "cwd": str(root),
                 "timeout_seconds": timeout, "diagnostics": logs.name, "status": "incomplete"}
        self.record["commands"].append(entry)
        self.save()
        try:
            result = run_owned(argv, cwd=root, timeout_seconds=timeout, log_root=logs, label=label,
                               max_capture_bytes=max_bytes)
            entry["execution"] = result.execution
            entry["status"] = "passed" if result.returncode == 0 else "failed"
            if result.returncode:
                raise ScalingError(f"{label} failed ({result.returncode}); diagnostics: {logs}; {redact(result.stderr[-4000:].strip())}")
            return result.stdout
        except ProcessError as error:
            entry["execution"] = error.receipt
            entry["status"] = error.receipt["status"]
            self.fail(label, error, entry)
            raise
        except (ScalingError, OSError, ValueError, KeyboardInterrupt) as error:
            entry["status"] = "cancelled" if isinstance(error, KeyboardInterrupt) else "failed"
            self.fail(label, error, entry)
            raise
        finally:
            self.save()

    def observe(self, value: dict, *, case: str, phase: str, index: int) -> None:
        path = self.root / f"observation-{len(self.record['observations']) + 1:03d}.json"
        with path.open("x", encoding="utf-8") as stream:
            stream.write(json.dumps(value, indent=2, allow_nan=False) + "\n")
        self.record["observations"].append({"case": case, "phase": phase, "index": index,
                                            "path": path.name, "sha256": sha256_file(path)})
        self.save()


def command(argv: list[str], *, root: Path = ROOT, timeout: int = 300, attempt: Attempt | None = None,
            label: str = "command", max_bytes: int = 64 * 1024 * 1024) -> str:
    if attempt is not None:
        return attempt.run(argv, root=root, timeout=timeout, label=label, max_bytes=max_bytes)
    result = subprocess.run(argv, cwd=root, capture_output=True, text=True, encoding="utf-8", errors="strict", timeout=timeout, check=False)
    if result.returncode:
        raise ScalingError(f"{Path(argv[0]).name} failed ({result.returncode}): {result.stderr[-4000:].strip()}")
    return result.stdout


def source_identity(root: Path, *, with_inputs: bool = False) -> dict:
    def git(*args: str) -> str:
        return command(["git", "-c", f"safe.directory={root.as_posix()}", *args], root=root)
    digest = hashlib.sha256()
    inputs = {}
    names = git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0")
    for name in sorted(set(names)):
        if not name or not (name.startswith(("crates/", "scripts/", "fixtures/", ".cargo/")) or name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".gitattributes")):
            continue
        path = root / name
        digest.update(name.encode("utf-8") + b"\0")
        inputs[name] = sha256_file(path) if path.is_file() else "deleted"
        digest.update(inputs[name].encode("ascii"))
    identity = {"sha": git("rev-parse", "HEAD").strip(), "dirty": bool(git("status", "--porcelain")), "working_source_sha256": digest.hexdigest()}
    if with_inputs:
        identity["inputs"] = inputs
    return identity


def hardware() -> dict:
    cpu = platform.processor()
    if sys.platform.startswith("linux"):
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    memory = None
    if sys.platform == "win32":
        import ctypes
        class Memory(ctypes.Structure):
            _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [(name, ctypes.c_ulonglong) for name in ("total", "available", "page_total", "page_available", "virtual_total", "virtual_available", "extended")]
        state = Memory()
        state.length = ctypes.sizeof(state)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(state)):
            memory = state.total
    elif hasattr(os, "sysconf"):
        memory = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
    return {"system": platform.system(), "release": platform.release(), "machine": platform.machine(), "cpu": cpu,
            "logical_cpus": os.cpu_count(), "physical_memory_bytes": memory}


def distribution(values: list[int | float]) -> dict:
    if not values or any(type(value) not in (int, float) or not math.isfinite(value) for value in values):
        raise ScalingError("distribution requires finite numeric observations")
    ordered = sorted(values)
    return {"count": len(values), "min": ordered[0], "median": statistics.median(ordered),
            "p95": ordered[max(0, math.ceil(len(ordered) * 0.95) - 1)], "max": ordered[-1],
            "mean": statistics.mean(ordered), "standard_deviation": statistics.pstdev(ordered)}


def numeric_metrics(value, prefix="") -> dict[str, list[int | float]]:
    output: dict[str, list[int | float]] = {}
    def visit(item, path):
        if type(item) in (int, float):
            output.setdefault(path, []).append(item)
        elif isinstance(item, dict):
            for key, child in item.items():
                visit(child, f"{path}.{key}" if path else key)
        elif isinstance(item, list):
            for child in item:
                visit(child, path)
    visit(value, prefix)
    return output


def validate_sample(value: dict, case: str) -> None:
    if value.get("schema") != SAMPLE_SCHEMA or value.get("correctness") != "passed":
        raise ScalingError("workload schema or correctness result invalid")
    require_int(value.get("fixture_version"), label="fixture_version", minimum=2, maximum=2)
    fixture = value.get("fixture", {})
    if fixture.get("name") != case:
        raise ScalingError("workload case identity differs")
    for key in ("roster", "empty_rooms", "metadata_bytes", "playlist_items", "server_playlist_items", "inventory", "anchors_per_file", "gui_pumps", "churn_cycles"):
        require_int(fixture.get(key), label=key, minimum=1, maximum=100_000)
    require_int(value["server"]["playlist"].get("accepted_items"), label="accepted playlist items", minimum=fixture["server_playlist_items"], maximum=fixture["server_playlist_items"])
    require_int(value["server"]["playlist"].get("accepted_recipients"), label="accepted playlist recipients", minimum=fixture["roster"], maximum=fixture["roster"])
    network, media, recovery = value["network"], value["media"], value["recovery"]
    # Independently check emitted resource evidence, rather than accepting a success label.
    for key in ("retained_connections", "retained_network_workers"):
        require_int(network.get(key), label=key, minimum=0, maximum=0)
    if network.get("joined_network_workers") is not True or not network.get("checkpoints"):
        raise ScalingError("network worker joins/checkpoints missing")
    for checkpoint in network["checkpoints"]:
        for key in ("active_connections", "unauthenticated_connections", "queued_bytes", "address_buckets"):
            require_int(checkpoint["resources"].get(key), label=key, minimum=0, maximum=0)
        require_int(checkpoint["resources"].get("peak_queued_bytes"), label="queue peak", minimum=0, maximum=network["queue_byte_limit"])
    require_int(media.get("retained_staging_directories"), label="retained staging", minimum=0, maximum=0)
    if media.get("inventory_count") != fixture["inventory"] or media.get("fingerprint_count") != fixture["inventory"]:
        raise ScalingError("generated media inventory incomplete")
    require_int(recovery.get("maximum_retained_attempts"), label="retained attempts", minimum=0, maximum=2)
    if len(value["gui"]["projection"]["pump_nanoseconds"]) != fixture["gui_pumps"]:
        raise ScalingError("projection sample count differs")


def run_sample(binary: Path, case: str, *, extra_clone=False, churn_cycles=None, timeout=300,
               attempt: Attempt | None = None, label: str = "sample") -> dict:
    argv = [str(binary), case]
    if extra_clone:
        argv.append("--inject-extra-roster-clone")
    if churn_cycles:
        argv.extend(["--churn-cycles", str(churn_cycles)])
    value = strict_json_loads(command(argv, timeout=timeout, attempt=attempt, label=label, max_bytes=MAX_SAMPLE_BYTES),
                             max_bytes=MAX_SAMPLE_BYTES, expected_type=dict, label="scaling sample")
    validate_sample(value, case)
    return value


def summarize(samples: list[dict]) -> dict:
    observations: dict[str, list[int | float]] = {}
    for sample in samples:
        for section in ("server", "network", "media", "recovery", "gui"):
            for key, values in numeric_metrics(sample[section], section).items():
                observations.setdefault(key, []).extend(values)
    return {key: distribution(values) for key, values in sorted(observations.items())}


def compare(report: dict, baseline: dict, name: str) -> dict:
    if baseline.get("schema") != SCHEMA or baseline.get("name") != name:
        raise ScalingError("baseline schema or name differs")
    for key in ("profile", "hardware", "features"):
        if report[key] != baseline.get(key):
            raise ScalingError(f"baseline {key} differs; use a separate named baseline")
    if set(report["cases"]) != set(baseline.get("cases", {})):
        raise ScalingError("baseline case selection differs")
    rows = {}
    for case, current in report["cases"].items():
        previous = baseline["cases"][case]
        if current["fixture"] != previous["fixture"]:
            raise ScalingError("baseline fixture differs")
        if current["distributions"].keys() != previous["distributions"].keys():
            raise ScalingError("baseline metric inventory differs")
        rows[case] = {}
        for key, metric in current["distributions"].items():
            old = previous["distributions"][key]["median"]
            new = metric["median"]
            if type(old) not in (int, float) or not math.isfinite(old):
                raise ScalingError("baseline median must be finite numeric")
            rows[case][key] = {"baseline_median": old, "current_median": new, "delta": new-old,
                               "delta_percent": (new-old)*100/old if old else None}
    return {"status": "compared", "baseline_name": name, "baseline_source": baseline["source"], "timing_thresholds": None, "metrics": rows}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--name", required=True, help="named report/baseline identity")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--baseline-name")
    parser.add_argument("--cases", nargs="+", choices=["normal", "large"], default=["normal", "large"])
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--warmup", type=int, default=1)
    parser.add_argument("--churn-cycles", type=int)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--profile", choices=["dev", "release"], default="dev")
    parser.add_argument("--target-dir", type=Path, default=Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")))
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--verify-clone-sensitivity", action="store_true")
    parser.add_argument("--startup-report", type=Path, help="existing gui-startup-bench report; never included in projection timings")
    options = parser.parse_args(argv)
    attempt = None
    phase = "validate"
    try:
        attempt = Attempt(options.output, [sys.executable, str(Path(__file__).resolve()), *(sys.argv[1:] if argv is None else argv)])
        if not 1 <= options.samples <= 100 or not 0 <= options.warmup <= 100 or not 1 <= options.timeout <= 3600:
            raise ScalingError("samples/warmup/timeout outside supported bounds")
        if len(set(options.cases)) != len(options.cases):
            raise ScalingError("duplicate workload case")
        if options.churn_cycles is not None and not 1 <= options.churn_cycles <= 100_000:
            raise ScalingError("churn cycles must be in 1..=100000")
        if bool(options.baseline) != bool(options.baseline_name):
            raise ScalingError("--baseline and --baseline-name must be supplied together")
        if options.baseline and options.output.resolve() == options.baseline.resolve():
            raise ScalingError("comparison output must preserve the named baseline")
        baseline = strict_json_load(options.baseline, expected_type=dict, max_bytes=64*1024*1024) if options.baseline else None
        target = options.target_dir.resolve()
        phase = "source-before"
        attempt.record["source_before"] = source_identity(ROOT, with_inputs=True)
        source = {key: value for key, value in attempt.record["source_before"].items() if key != "inputs"}
        attempt.save()
        if not options.skip_build:
            phase = "build"
            build = ["cargo", "build", "--locked", "-p", "sorotte-gui", "--example", "scaling_workloads", "--features", FEATURES, "--target-dir", str(target)]
            if options.profile == "release":
                build.append("--release")
            command(build, root=ROOT, timeout=3600, attempt=attempt, label=phase)
            phase = "source-after-build"
            attempt.record["source_after_build"] = source_identity(ROOT, with_inputs=True)
            attempt.save()
            if any(attempt.record["source_after_build"][key] != source[key] for key in ("sha", "working_source_sha256")):
                raise ScalingError("source changed during build; retry after concurrent edits finish")
        binary = target / ("release" if options.profile == "release" else "debug") / "examples" / ("scaling_workloads.exe" if os.name == "nt" else "scaling_workloads")
        phase = "compiler-identity"
        report = {"schema": SCHEMA, "name": options.name, "source": source, "profile": options.profile,
                  "features": FEATURES, "hardware": hardware(), "rustc": command(["rustc", "-vV"], root=ROOT, attempt=attempt, label=phase).strip(),
                  "build_mode": "prebuilt_unverified_source" if options.skip_build else "built_for_this_run",
                  "binary_sha256": sha256_file(binary), "warmup": options.warmup, "samples": options.samples,
                  "generated_at_unix_seconds": int(time.time()), "cases": {},
                  "startup": {"status": "unavailable", "runner": "scripts/gui-startup-bench.ps1"},
                  "timing_policy": "advisory; no thresholds until baseline noise is established"}
        attempt.record["binary"] = {"path": str(binary), "sha256_before": report["binary_sha256"],
                                    "build_mode": report["build_mode"]}
        attempt.save()
        for case in options.cases:
            for index in range(options.warmup):
                phase = f"{case}-warmup-{index + 1}"
                value = run_sample(binary, case, churn_cycles=options.churn_cycles, timeout=options.timeout, attempt=attempt, label=phase)
                attempt.observe(value, case=case, phase="warmup", index=index + 1)
            samples = []
            for index in range(options.samples):
                phase = f"{case}-sample-{index + 1}"
                value = run_sample(binary, case, churn_cycles=options.churn_cycles, timeout=options.timeout, attempt=attempt, label=phase)
                attempt.observe(value, case=case, phase="sample", index=index + 1)
                if samples and value["fixture"] != samples[0]["fixture"]:
                    raise ScalingError("fixture changed between samples; distributions would be incomparable")
                samples.append(value)
            report["cases"][case] = {"fixture": samples[0]["fixture"], "distributions": summarize(samples), "raw_samples": samples}
        if options.verify_clone_sensitivity:
            phase = "clone-control"
            control = run_sample(binary, "normal", churn_cycles=1, timeout=options.timeout, attempt=attempt, label=phase)
            attempt.observe(control, case="normal", phase=phase, index=1)
            phase = "clone-injected"
            injected = run_sample(binary, "normal", extra_clone=True, churn_cycles=1, timeout=options.timeout, attempt=attempt, label=phase)
            attempt.observe(injected, case="normal", phase=phase, index=1)
            key = lambda sample: sample["server"]["list"]["allocation"]
            if key(injected)["allocated_bytes"] <= key(control)["allocated_bytes"] or key(injected)["allocation_calls"] <= key(control)["allocation_calls"]:
                raise ScalingError("extra full-roster clone did not increase measured allocation")
            report["clone_sensitivity"] = {"status": "passed", "control": key(control), "injected": key(injected)}
        if options.startup_report:
            phase = "startup-report"
            startup = strict_json_load(options.startup_report, expected_type=dict, max_bytes=16*1024*1024)
            require_int(startup.get("schema_version"), label="startup schema", minimum=2, maximum=2)
            report["startup"] = {"status": "provided", "sha256": sha256_file(options.startup_report), "report": startup}
        phase = "comparison"
        report["comparison"] = compare(report, baseline, options.baseline_name) if baseline else {"status": "baseline_recorded", "baseline_name": options.name}
        phase = "source-after-samples"
        attempt.record["source_after"] = source_identity(ROOT, with_inputs=True)
        attempt.record["binary"]["sha256_after"] = sha256_file(binary)
        attempt.save()
        if any(attempt.record["source_after"][key] != source[key] for key in ("sha", "working_source_sha256")):
            raise ScalingError("source changed during workload measurement; preserve this attempt and run a fresh one after edits finish")
        if attempt.record["binary"]["sha256_after"] != report["binary_sha256"]:
            raise ScalingError("workload binary changed during measurement")
        phase = "publish-report"
        with options.output.open("x", encoding="utf-8") as stream:
            stream.write(json.dumps(report, indent=2, allow_nan=False) + "\n")
        attempt.record["report"] = {"path": str(options.output), "sha256": sha256_file(options.output)}
        attempt.record["status"] = "passed"
        print(f"scaling workloads passed: {options.output} ({report['comparison']['status']}; timings advisory)")
        return 0
    except (ScalingError, ArtifactInputError, ProcessError, OSError, subprocess.SubprocessError, KeyError, TypeError, UnicodeError, KeyboardInterrupt) as error:
        if attempt is not None:
            attempt.record["status"] = (error.receipt["status"] if isinstance(error, ProcessError) and error.receipt["status"] in ("timeout", "cancelled")
                                         else "cancelled" if isinstance(error, KeyboardInterrupt) else "failed")
            entry = next((entry for entry in reversed(attempt.record["commands"]) if entry["phase"] == phase), None)
            attempt.fail(phase, error, entry)
            attempt.save()
        print(f"scaling workload error: {redact(str(error))}", file=sys.stderr)
        return 1
    finally:
        if attempt is not None:
            if attempt.record["source_after"] is None:
                try:
                    attempt.record["source_after"] = source_identity(ROOT, with_inputs=True)
                except (ScalingError, OSError, subprocess.SubprocessError, UnicodeError) as error:
                    attempt.record["source_after_error"] = redact(str(error))
            attempt.record["completed_at_unix_seconds"] = time.time()
            attempt.save()


if __name__ == "__main__":
    raise SystemExit(main())
