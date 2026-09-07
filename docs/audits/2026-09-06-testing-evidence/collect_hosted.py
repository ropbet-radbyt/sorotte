"""Read-only GitHub evidence collection for the 0.2.9 testing-process audit.

Run from the audited worktree with Python 3.11+ and an authenticated gh CLI.
Raw API responses stay in target/testing-audit/hosted. No workflow is dispatched.
"""
import concurrent.futures
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[3]
RAW = ROOT / "target/testing-audit/hosted"
REPO = "repos/ropbet-radbyt/sorotte"


def read(endpoint, name):
    path = RAW / (name + ".json")
    if path.exists():
        return json.loads(path.read_text(encoding="utf-8-sig"))
    result = subprocess.run(["gh", "api", REPO + endpoint], capture_output=True,
                            text=True, encoding="utf-8", check=True, timeout=60)
    value = json.loads(result.stdout)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    return value


def main():
    RAW.mkdir(parents=True, exist_ok=True)
    pr = read("/pulls/32", "pr32")
    commits = read("/pulls/32/commits?per_page=100", "pr32-commits")
    if len(commits) == 100:
        raise RuntimeError("Commit pagination required")
    shas = {c["sha"] for c in commits} | {pr["merge_commit_sha"]}
    runs = []
    for page in range(1, 11):
        result = read("/actions/runs?per_page=100&page=" + str(page)
                      + "&created=2026-09-04..2026-09-06", "runs-" + str(page))
        runs.extend(r for r in result["workflow_runs"] if r["head_sha"] in shas)
        if len(result["workflow_runs"]) < 100:
            break
    else:
        raise RuntimeError("Run pagination incomplete")
    tasks = [(r, a) for r in runs for a in range(1, r["run_attempt"] + 1)]

    def collect(task):
        run, attempt = task
        data = read(f"/actions/runs/{run['id']}/attempts/{attempt}/jobs?per_page=100",
                    f"jobs-{run['id']}-{attempt}")
        if data["total_count"] > 100:
            raise RuntimeError("Job pagination required")
        return {"run_id": run["id"], "attempt": attempt, "jobs": data["jobs"]}

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        job_sets = list(pool.map(collect, tasks))
    (RAW / "index.json").write_text(json.dumps({"pr": pr, "commits": commits,
        "runs": runs, "job_sets": job_sets}, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"pr": 32, "commits": len(commits), "runs": len(runs),
                      "attempts": len(tasks), "jobs": sum(len(j["jobs"]) for j in job_sets)}))


if __name__ == "__main__":
    main()
