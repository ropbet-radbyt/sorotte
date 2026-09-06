"""Summarize collected Actions evidence without executing tests or workflows."""
import collections
import datetime as dt
import hashlib
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[3]
RAW = ROOT / "target/testing-audit/hosted"
OUT = pathlib.Path(__file__).resolve().parent


def instant(value):
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00")) if value else None


def seconds(start, end):
    return (instant(end) - instant(start)).total_seconds() if start and end else 0


def main():
    source = RAW / "index.json"
    data = json.loads(source.read_text(encoding="utf-8"))
    runs = {r["id"]: r for r in data["runs"] if r["path"].startswith(".github/workflows/")}
    jobs = {}
    carried = 0
    for batch in sorted(data["job_sets"], key=lambda b: b["attempt"]):
        if batch["run_id"] not in runs:
            continue
        for job in batch["jobs"]:
            # GitHub copies already-successful jobs into later attempt responses,
            # with NEW job IDs but original execution timestamps. Do not bill them twice.
            key = (batch["run_id"], job["name"], job["started_at"],
                   job["completed_at"], job["conclusion"])
            if key in jobs:
                carried += 1
                continue
            jobs[key] = {
                "run_id": batch["run_id"], "attempt": batch["attempt"],
                "job_id": job["id"], "name": job["name"],
                "sha": runs[batch["run_id"]]["head_sha"],
                "conclusion": job["conclusion"],
                "started_at": job["started_at"], "completed_at": job["completed_at"],
                "execution_seconds": seconds(job["started_at"], job["completed_at"])
                    if job["conclusion"] != "skipped" else 0,
                "url": job["html_url"],
                "steps": [{"name": s["name"], "conclusion": s["conclusion"],
                           "outcome": s.get("outcome"),
                           "seconds": seconds(s["started_at"], s["completed_at"])}
                          for s in job.get("steps", [])],
            }
    all_jobs = list(jobs.values())
    candidates = []
    for sha in [c["sha"] for c in data["commits"]] + [data["pr"]["merge_commit_sha"]]:
        selected_runs = [r for r in runs.values() if r["head_sha"] == sha]
        selected_jobs = [j for j in all_jobs if j["sha"] == sha]
        candidates.append({"sha": sha, "workflow_runs": len(selected_runs),
            "run_conclusions": dict(collections.Counter(r["conclusion"] for r in selected_runs)),
            "execution_minutes": round(sum(j["execution_seconds"] for j in selected_jobs) / 60, 2)})
    release_sha = data["pr"]["merge_commit_sha"]
    final_sha = data["pr"]["head"]["sha"]
    lifecycle = [j for j in all_jobs if j["sha"] == release_sha
                 and "candidate lifecycle" in j["name"]]
    mutation = [j for j in all_jobs if j["name"].startswith("Mutation (")]
    summary = {
        "schema_version": 1,
        "captured_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "source_index_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "baseline_sha": release_sha, "final_candidate_sha": final_sha,
        "pr_url": data["pr"]["html_url"], "pr_created_at": data["pr"]["created_at"],
        "pr_merged_at": data["pr"]["merged_at"],
        "pr_open_seconds": seconds(data["pr"]["created_at"], data["pr"]["merged_at"]),
        "scope": "PR 32 commit SHAs and merge SHA; Sep 4-6 API runs; repository workflow paths only",
        "metric_limits": "Observed job duration, not billed CPU or elapsed developer effort. Carried jobs deduplicated by run/name/timestamps/conclusion. Cancelled jobs are not assumed wasted or flaky.",
        "runs": len(runs), "attempt_responses": sum(b["run_id"] in runs for b in data["job_sets"]),
        "unique_job_executions_or_skips": len(jobs), "carried_job_records_removed": carried,
        "run_conclusions": dict(collections.Counter(r["conclusion"] for r in runs.values())),
        "execution_minutes": round(sum(j["execution_seconds"] for j in all_jobs) / 60, 2),
        "mutation_execution_minutes": round(sum(j["execution_seconds"] for j in mutation) / 60, 2),
        "cancelled_mutation_execution_minutes": round(sum(j["execution_seconds"] for j in mutation if j["conclusion"] == "cancelled") / 60, 2),
        "merge_lifecycle_producers": len(lifecycle),
        "merge_lifecycle_execution_minutes": round(sum(j["execution_seconds"] for j in lifecycle) / 60, 2),
        "merge_lifecycle_jobs": lifecycle,
        "candidates": candidates,
        "final_candidate_jobs": [j for j in all_jobs if j["sha"] == final_sha],
        "failed_jobs": [j for j in all_jobs if j["conclusion"] in ("failure", "timed_out")],
        "runs_index": [{k: r[k] for k in ("id", "name", "path", "event", "head_sha", "run_attempt", "conclusion", "created_at", "updated_at", "html_url")}
                       for r in runs.values()],
    }
    (OUT / "hosted-summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in summary.items() if not isinstance(v, list)}, indent=2))


if __name__ == "__main__":
    main()
