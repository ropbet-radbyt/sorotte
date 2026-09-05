#!/usr/bin/env python3
"""Run deterministic production scaling fixtures; timing comparisons are advisory."""
from __future__ import annotations

import argparse
import hashlib
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time

from artifact_input import ArtifactInputError, require_int, sha256_file, strict_json_load, strict_json_loads

SCHEMA = "sorotte-scaling-report-v1"
SAMPLE_SCHEMA = "sorotte-scaling-sample-v1"
FEATURES = "gui-semantic-smoke,live-python-interop"
ROOT = Path(__file__).resolve().parent.parent


class ScalingError(ValueError):
    pass


def command(argv: list[str], *, root: Path = ROOT, timeout: int = 300) -> str:
    result = subprocess.run(argv, cwd=root, capture_output=True, text=True, encoding="utf-8", errors="strict", timeout=timeout, check=False)
    if result.returncode:
        raise ScalingError(f"{Path(argv[0]).name} failed ({result.returncode}): {result.stderr[-4000:].strip()}")
    return result.stdout


def source_identity(root: Path) -> dict:
    def git(*args: str) -> str:
        return command(["git", "-c", f"safe.directory={root.as_posix()}", *args], root=root)
    digest = hashlib.sha256()
    names = git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0")
    for name in sorted(set(names)):
        if not name or not (name.startswith(("crates/", "scripts/", "fixtures/")) or name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")):
            continue
        path = root / name
        digest.update(name.encode("utf-8") + b"\0")
        digest.update(sha256_file(path).encode("ascii") if path.is_file() else b"deleted")
    return {"sha": git("rev-parse", "HEAD").strip(), "dirty": bool(git("status", "--porcelain")), "working_source_sha256": digest.hexdigest()}


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


def run_sample(binary: Path, case: str, *, extra_clone=False, churn_cycles=None, timeout=300) -> dict:
    argv = [str(binary), case]
    if extra_clone:
        argv.append("--inject-extra-roster-clone")
    if churn_cycles:
        argv.extend(["--churn-cycles", str(churn_cycles)])
    value = strict_json_loads(command(argv, timeout=timeout), max_bytes=16 * 1024 * 1024, expected_type=dict, label="scaling sample")
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
    try:
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
        source = source_identity(ROOT)
        if not options.skip_build:
            build = ["cargo", "build", "--locked", "-p", "sorotte-gui", "--example", "scaling_workloads", "--features", FEATURES, "--target-dir", str(target)]
            if options.profile == "release":
                build.append("--release")
            command(build, timeout=3600)
            if source_identity(ROOT)["working_source_sha256"] != source["working_source_sha256"]:
                raise ScalingError("source changed during build; retry after concurrent edits finish")
        binary = target / ("release" if options.profile == "release" else "debug") / "examples" / ("scaling_workloads.exe" if os.name == "nt" else "scaling_workloads")
        report = {"schema": SCHEMA, "name": options.name, "source": source, "profile": options.profile,
                  "features": FEATURES, "hardware": hardware(), "rustc": command(["rustc", "-vV"]).strip(),
                  "build_mode": "prebuilt_unverified_source" if options.skip_build else "built_for_this_run",
                  "binary_sha256": sha256_file(binary), "warmup": options.warmup, "samples": options.samples,
                  "generated_at_unix_seconds": int(time.time()), "cases": {},
                  "startup": {"status": "unavailable", "runner": "scripts/gui-startup-bench.ps1"},
                  "timing_policy": "advisory; no thresholds until baseline noise is established"}
        for case in options.cases:
            for _ in range(options.warmup):
                run_sample(binary, case, churn_cycles=options.churn_cycles, timeout=options.timeout)
            samples = [run_sample(binary, case, churn_cycles=options.churn_cycles, timeout=options.timeout) for _ in range(options.samples)]
            report["cases"][case] = {"fixture": samples[0]["fixture"], "distributions": summarize(samples), "raw_samples": samples}
        if options.verify_clone_sensitivity:
            control = run_sample(binary, "normal", churn_cycles=1, timeout=options.timeout)
            injected = run_sample(binary, "normal", extra_clone=True, churn_cycles=1, timeout=options.timeout)
            key = lambda sample: sample["server"]["list"]["allocation"]
            if key(injected)["allocated_bytes"] <= key(control)["allocated_bytes"] or key(injected)["allocation_calls"] <= key(control)["allocation_calls"]:
                raise ScalingError("extra full-roster clone did not increase measured allocation")
            report["clone_sensitivity"] = {"status": "passed", "control": key(control), "injected": key(injected)}
        if options.startup_report:
            startup = strict_json_load(options.startup_report, expected_type=dict, max_bytes=16*1024*1024)
            require_int(startup.get("schema_version"), label="startup schema", minimum=2, maximum=2)
            report["startup"] = {"status": "provided", "sha256": sha256_file(options.startup_report), "report": startup}
        report["comparison"] = compare(report, baseline, options.baseline_name) if baseline else {"status": "baseline_recorded", "baseline_name": options.name}
        options.output.parent.mkdir(parents=True, exist_ok=True)
        import json
        options.output.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
        print(f"scaling workloads passed: {options.output} ({report['comparison']['status']}; timings advisory)")
        return 0
    except (ScalingError, ArtifactInputError, OSError, subprocess.SubprocessError, KeyError, TypeError, UnicodeError) as error:
        print(f"scaling workload error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
