"""Replay named native harness failure schedules without operating a desktop."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import time

try:
    from mutation_process import run as owned_run
except ModuleNotFoundError:
    from scripts.mutation_process import run as owned_run

ROOT = Path(__file__).resolve().parents[1]
INVENTORY = ROOT / "coverage/native-harness-canaries.json"


def cargo_args(case: dict) -> list[str]:
    args = ["cargo", "test", "--locked", "-p", case["package"], *case["target"]]
    if case["features"]:
        args += ["--features", ",".join(case["features"])]
    return args


def validate_inventory(value: dict) -> None:
    if value.get("schema_version") != 1 or value.get("kind") != "sorotte-native-harness-canaries":
        raise ValueError("unsupported native canary inventory")
    cases = value["cases"]
    if not cases or len({case["id"] for case in cases}) != len(cases):
        raise ValueError("canaries must be a nonempty unique inventory")
    for case in cases:
        if not re.fullmatch(r"[a-z0-9_-]+", case["id"]) or not case["responsibility"]:
            raise ValueError("each canary needs an identity and responsibility")
        if not re.fullmatch(r"[a-zA-Z0-9_:]+", case["test"]):
            raise ValueError("canary requires an exact Rust test name")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    value = json.loads(INVENTORY.read_text())
    validate_inventory(value)
    if args.validate_only:
        return 0
    args.output.parent.mkdir(parents=True, exist_ok=True)
    artifact_root = args.output.with_suffix("")
    artifact_root.mkdir(exist_ok=False)
    source = subprocess.check_output(["git", "-c", f"safe.directory={ROOT.as_posix()}", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    report = {"schema_version": 1, "kind": "sorotte-native-harness-readiness", "source_sha": source,
              "inventory_sha256": hashlib.sha256(INVENTORY.read_bytes()).hexdigest(),
              "tracked_source_clean": not subprocess.check_output(["git", "-c", f"safe.directory={ROOT.as_posix()}", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True).strip(),
              "authoritative": False, "result": "running", "cases": []}

    def save():
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    save()
    try:
        for case in value["cases"]:
            command = [*cargo_args(case), case["test"], "--", "--exact"]
            started = time.monotonic()
            result = owned_run(command, cwd=ROOT, timeout_seconds=900,
                               log_root=artifact_root / case["id"], label=case["id"])
            text = result.stdout + result.stderr
            passed = result.returncode == 0 and "test result: ok. 1 passed; 0 failed; 0 ignored;" in text
            report["cases"].append({"id": case["id"], "command": command, "result": "passed" if passed else "failed",
                                    "elapsed_seconds": time.monotonic() - started,
                                    "process_receipt_sha256": hashlib.sha256(
                                        (artifact_root / case["id"] / "process.json").read_bytes()).hexdigest()})
            save()
            if not passed:
                raise ValueError(f"native canary failed or executed zero/ignored tests: {case['id']}")
        result = owned_run([sys.executable, "-m", "unittest", *value["python_suites"]],
                           cwd=ROOT, timeout_seconds=180, log_root=artifact_root / "python-contracts",
                           label="native-python-contracts")
        if result.returncode:
            raise ValueError("native report rejection canaries failed")
        report["result"] = "passed"
    except (OSError, ValueError, RuntimeError, KeyboardInterrupt) as error:
        report["result"] = "failed"
        report["error"] = str(error)
        save()
        return 1
    save()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
