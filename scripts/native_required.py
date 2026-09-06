"""Require exact-source native producer authority or a verified inapplicable lane.

This reads GitHub execution authority. Diagnostic exports, local smoke output,
an older successful source, and a same-tree commit are never substitutes.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import sys
import time
from urllib.parse import quote

# The shared front-door modules also support direct `python scripts/...` use.
if str(Path(__file__).resolve().parent) not in sys.path:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
import verify
from merge_gate import GitHub, GateError
from verification_tools import digest, git

WORKFLOW = ".github/workflows/gui-native-interactive.yml"
LABELS = {"self-hosted", "Windows", "X64", "sorotte-native-interactive", "sorotte-ephemeral"}


class NativePending(ValueError):
    pass


def validate_plan(path: Path, *, base: str, source: str) -> dict:
    if not all(re.fullmatch(r"[0-9a-f]{40}", value) and value != "0" * 40 for value in (base, source)):
        raise ValueError("native obligation requires full nonzero event base and source SHAs")
    if git("rev-parse", "HEAD") != source:
        raise ValueError("native obligation checkout is not the externally requested source")
    supplied = json.loads(path.read_text(encoding="utf-8"))
    if supplied.get("base_sha") != base or supplied.get("source_sha") != source:
        raise ValueError("native plan differs from the external event base/source")
    fresh = verify.plan(base, source)
    for key in ("schema_version", "kind", "base_sha", "source_sha", "policy_sha256", "paths", "lanes", "required_checks"):
        if supplied.get(key) != fresh[key]:
            raise ValueError(f"native selection receipt drift: {key}")
    lanes = [item for item in fresh["lanes"] if item["id"] == "native"]
    if len(lanes) != 1 or type(lanes[0].get("selected")) is not bool:
        raise ValueError("native lane must have one explicit applicability decision")
    return fresh


def actor_authority(api, run: dict) -> list[dict]:
    result = []
    for field in ("actor", "triggering_actor"):
        login = run.get(field, {}).get("login", "")
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]{0,38}", login):
            raise ValueError("native producer must be approved by an identifiable repository maintainer")
        permission = api.get(f"collaborators/{quote(login, safe='')}/permission")
        if (permission.get("user", {}).get("login", "").casefold() != login.casefold()
                or permission.get("permission") not in {"write", "maintain", "admin"}):
            raise ValueError("native producer actor lacks current repository write authority")
        result.append({"field": field, "login": login, "permission": permission["permission"]})
    return result


def validate_producer(api, run: dict, source: str) -> dict:
    if (run.get("head_sha") != source or run.get("path") != WORKFLOW
            or run.get("repository", {}).get("full_name") != api.repository
            or run.get("head_repository", {}).get("full_name") != api.repository
            or run.get("event") not in {"workflow_dispatch", "push", "schedule"}
            or (run.get("event") in {"push", "schedule"} and run.get("head_branch") != "main")
            or type(run.get("id")) is not int or type(run.get("run_attempt")) is not int
            or run["id"] <= 0 or run["run_attempt"] <= 0):
        raise ValueError("native producer has foreign, untrusted, or different-source provenance")
    actors = actor_authority(api, run)
    if run.get("status") != "completed":
        raise NativePending(f"native run {run['id']} attempt {run['run_attempt']} is {run.get('status')}")
    if run.get("conclusion") != "success":
        raise ValueError(f"latest native run {run['id']} attempt {run['run_attempt']} concluded {run.get('conclusion')}; retain its diagnostics and rerun after repair")
    jobs = api.pages(f"actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs", "jobs")
    native = [job for job in jobs if LABELS <= set(job.get("labels", []))]
    if len(native) != 1:
        raise ValueError("native producer lacks exactly one isolated physical Windows job")
    job = native[0]
    if (job.get("head_sha") != source or job.get("run_id") != run["id"]
            or job.get("status") != "completed" or job.get("conclusion") != "success"
            or not re.fullmatch(r"sorotte-sandbox-[0-9a-f-]{36}", job.get("runner_name", ""))
            or type(job.get("runner_id")) is not int or job["runner_id"] <= 0):
        raise ValueError("native producer job did not complete on the owned one-job Sandbox runner")
    # Bind the observations to this attempt. A concurrent rerun invalidates them.
    current = api.get(f"actions/runs/{run['id']}")
    if any(current.get(key) != run.get(key) for key in ("id", "run_attempt", "head_sha", "status", "conclusion")):
        raise NativePending("native producer changed attempt during authority lookup")
    return {"workflow": WORKFLOW, "run_id": run["id"], "run_attempt": run["run_attempt"],
            "source_sha": source, "job_id": job["id"], "runner_id": job["runner_id"],
            "event": run["event"], "actors": actors, "completed_at": job.get("completed_at"),
            "url": f"https://github.com/{api.repository}/actions/runs/{run['id']}", "conclusion": "success"}


def observe(api, source: str) -> dict:
    runs = api.pages(f"actions/workflows/gui-native-interactive.yml/runs?head_sha={source}", "workflow_runs")
    # Never search backward for a green run after a newer failure. Older or
    # foreign sources returned by a broken/malformed API filter also fail closed.
    if not runs:
        raise NativePending("no trusted native producer exists for the exact source")
    if any(type(run.get("id")) is not int for run in runs):
        raise ValueError("native producer inventory has invalid run identity")
    latest = max(runs, key=lambda run: run["id"])
    return validate_producer(api, api.get(f"actions/runs/{latest['id']}"), source)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-sha", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY"))
    parser.add_argument("--wait-seconds", type=int, default=0)
    parser.add_argument("--poll-seconds", type=int, default=30)
    args = parser.parse_args(argv)
    if not 0 <= args.wait_seconds <= 5400 or not 1 <= args.poll_seconds <= 60:
        parser.error("native wait must be 0..5400 seconds; poll must be 1..60 seconds")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    receipt = {"schema_version": 1, "kind": "native-required", "source_sha": args.source_sha,
               "base_sha": args.base_sha, "repository": args.repository, "status": "incomplete",
               "selected": None, "producer": None, "created_at": datetime.now(timezone.utc).isoformat(),
               "disposition": "unclassified"}
    def save():
        args.output.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    save()
    started = time.monotonic()
    try:
        planned = validate_plan(args.plan, base=args.base_sha, source=args.source_sha)
        receipt["plan_sha256"] = digest(args.plan)
        receipt["selected"] = next(item["selected"] for item in planned["lanes"] if item["id"] == "native")
        if not receipt["selected"]:
            receipt.update(status="passed", reason="verified no applicable native change")
        else:
            api = GitHub(args.repository or "", os.environ.get("GH_TOKEN", os.environ.get("GITHUB_TOKEN", "")))
            while True:
                try:
                    receipt["producer"] = observe(api, args.source_sha)
                    receipt["status"] = "passed"
                    break
                except NativePending as error:
                    receipt.update(reason=str(error), elapsed_seconds=round(time.monotonic() - started, 3))
                    save()
                    print(f"Native evidence pending for {args.source_sha}: {error}. Use scripts/native-runner-qualify.ps1 with the reviewed ref, exact source and prepared bundle.", flush=True)
                    remaining = args.wait_seconds - (time.monotonic() - started)
                    if remaining <= 0:
                        raise ValueError("native capability/evidence unavailable within the bounded wait; dispatch/provision the reviewed source and rerun native-required") from error
                    time.sleep(min(args.poll_seconds, remaining))
    except (ValueError, GateError, OSError, KeyError, TypeError, AttributeError) as error:
        receipt.update(status="failed", reason=str(error))
        print(f"native-required failed: {error}", file=sys.stderr)
    finally:
        receipt["elapsed_seconds"] = round(time.monotonic() - started, 3)
        save()
    return int(receipt["status"] != "passed")


if __name__ == "__main__":
    raise SystemExit(main())
