#!/usr/bin/env python3
"""Human-readable observations and timing feedback; never release authorization."""
from __future__ import annotations

import argparse
from datetime import datetime
import json
from pathlib import Path
import re
import sys

from verification_tools import digest

DISPOSITIONS = {"product-defect", "harness-defect", "environment-unavailable", "assertion-gap", "unclassified"}


def cell(value) -> str:
    if isinstance(value, (dict, list)):
        value = json.dumps(value, sort_keys=True)
    return str(value if value is not None else "unavailable").replace("|", "\\|").replace("\n", " ")


def render(index: dict) -> str:
    lines = ["# Candidate verification observations", "", f"Source: `{index['source_sha']}`.", "",
             "This index locates evidence. Each required gate separately validates its source, inputs and producer authority.", "",
             "| Lane / receipt | Status | Seconds | Primary failure | Cleanup | Disposition |",
             "|---|---|---:|---|---|---|"]
    for entry in index["entries"]:
        receipt = entry["receipt"]
        lines.append("| " + " | ".join(cell(value) for value in (
            receipt.get("lane", receipt.get("kind")), receipt.get("status", receipt.get("result")),
            receipt.get("duration_seconds", receipt.get("elapsed_seconds")), receipt.get("primary_failure"),
            receipt.get("cleanup"), receipt.get("disposition", "unclassified"))) + " |")
    for entry in index["entries"]:
        receipt = entry["receipt"]
        lines += ["", f"## {cell(receipt.get('lane', receipt.get('kind', 'Receipt')))}", "",
                  f"- Receipt: `{cell(entry['path'])}`; SHA-256 `{entry['sha256']}`.",
                  f"- Inputs: `{cell(receipt.get('identity', receipt.get('inputs')))}`.",
                  f"- Selected obligations: `{cell(receipt.get('lanes', receipt.get('job_results')))}`.",
                  f"- Replay: `{cell(receipt.get('replay_command', receipt.get('command')))}`.",
                  f"- Operator interventions: `{cell(receipt.get('operator_interventions'))}`."]
    return "\n".join(lines) + "\n"


def instant(value: str) -> datetime:
    result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if result.utcoffset() is None:
        raise ValueError("timing evidence requires a timezone")
    return result


def metrics(jobs: list[dict], source_sha: str) -> dict:
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha) or not jobs:
        raise ValueError("timing observations require an exact source and nonempty job inventory")
    seen = set()
    starts, ends, failures = [], [], []
    total = cancelled = setup = 0.0
    incomplete = []
    attempts = set()
    for job in jobs:
        if job.get("head_sha") != source_sha:
            raise ValueError("timing job belongs to another or missing source")
        if job["id"] in seen:
            raise ValueError("duplicate timing job would inflate job-minutes")
        seen.add(job["id"])
        attempts.add((job["run_id"], job.get("run_attempt")))
        if job.get("status") != "completed" or not job.get("started_at") or not job.get("completed_at"):
            incomplete.append(job["id"])
            continue
        start, end = instant(job["started_at"]), instant(job["completed_at"])
        duration = (end - start).total_seconds()
        if duration < 0:
            raise ValueError("job completion precedes start")
        starts.append(start)
        ends.append(end)
        total += duration
        if job.get("conclusion") == "cancelled":
            cancelled += duration
        for step in job.get("steps", []):
            if not step.get("started_at") or not step.get("completed_at"):
                continue
            step_start, step_end = instant(step["started_at"]), instant(step["completed_at"])
            if step_end < step_start or step_start < start or step_end > end:
                raise ValueError("step timing is outside its owning job")
            if step.get("conclusion") == "failure":
                failures.append(step_end)
            if re.match(r"(?i)^(set up|setup|install|checkout|prepare)\b", step.get("name", "")):
                setup += (step_end - step_start).total_seconds()
    origin = min(starts) if starts else None
    return {"schema_version": 1, "kind": "verification-timing-observation", "source_sha": source_sha,
            "jobs": len(jobs), "workflow_attempts": len(attempts), "incomplete_jobs": incomplete,
            "job_minutes": round(total / 60, 3), "cancelled_job_minutes": round(cancelled / 60, 3),
            "observed_job_span_seconds": (max(ends) - origin).total_seconds() if origin else None,
            "first_failed_step_seconds": (min(failures) - origin).total_seconds() if failures and origin else None,
            "setup_step_minutes": round(setup / 60, 3), "genuine_flaky_cases": None,
            "operator_interventions": None,
            "note": "Observed execution intervals, not billing or a dependency-graph critical path. Setup is label-based. A retry is not proof of flakiness; absent incident classification remains unavailable."}


def annotate(receipt: Path, disposition: str, reason: str) -> dict:
    if disposition not in DISPOSITIONS or not reason.strip():
        raise ValueError("classification requires a reviewed disposition and concrete evidence/reason")
    return {"schema_version": 1, "kind": "verification-incident-annotation", "receipt": str(receipt),
            "receipt_sha256": digest(receipt), "disposition": disposition, "reason": reason,
            "note": "Separate operator assessment; original attempt remains immutable."}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    timing = sub.add_parser("metrics")
    timing.add_argument("--jobs", type=Path, required=True, help="GitHub job JSON array including head_sha, run_id and run_attempt")
    timing.add_argument("--source-sha", required=True)
    classification = sub.add_parser("annotate")
    classification.add_argument("--receipt", type=Path, required=True)
    classification.add_argument("--disposition", choices=sorted(DISPOSITIONS), required=True)
    classification.add_argument("--reason", required=True)
    for command in (timing, classification):
        command.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = metrics(json.loads(args.jobs.read_text(encoding="utf-8")), args.source_sha) if args.command == "metrics" else annotate(args.receipt, args.disposition, args.reason)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as stream:
            stream.write(json.dumps(result, indent=2) + "\n")
        return 0
    except (ValueError, KeyError, OSError) as error:
        print(f"verification observation failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
