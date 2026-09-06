#!/usr/bin/env python3
"""Validate hosted check authority before stable qualification/publication.

Receipts are observations, not bearer tokens. Every publication re-queries GitHub.
No local success file, newest-green search, or equal-tree substitution is accepted.
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
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from verification_tools import ROOT, digest

POLICY = ROOT / "coverage/verification-lanes.json"
PENDING_STATUSES = {"queued", "in_progress", "waiting", "requested", "pending"}
PROTECTION_PATH = "branches/main/protection"
PROTECTION_SETUP = (
    "Configure the repository-scoped Administration-read GitHub App using "
    "SOROTTE_PROTECTION_APP_ID and SOROTTE_PROTECTION_APP_PRIVATE_KEY; pass its "
    "short-lived token as SOROTTE_PROTECTION_TOKEN. See docs/PROTECTION_READER_SETUP.md "
    "(activation is deferred to a follow-up)."
)


class GateError(ValueError):
    pass


class PendingChecks(GateError):
    """Only this condition may be retried by the readiness command."""


class GitHub:
    def __init__(self, repository: str, token: str, protection_token: str = ""):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise GateError("invalid repository identity")
        if not token:
            raise GateError("GH_TOKEN is required for check and source authority")
        self.repository = repository
        self.token = token
        self.protection_token = protection_token

    def require_protection_token(self) -> None:
        if not self.protection_token:
            raise GateError(f"SOROTTE_PROTECTION_TOKEN is required. {PROTECTION_SETUP}")

    def get(self, path: str):
        token = self.token
        if path == PROTECTION_PATH:
            self.require_protection_token()
            token = self.protection_token
        request = Request(f"https://api.github.com/repos/{self.repository}/{path}", headers={
            "Accept": "application/vnd.github+json", "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "sorotte-merge-gate",
        })
        try:
            with urlopen(request, timeout=30) as response:
                return json.load(response)
        except HTTPError as error:
            if path == PROTECTION_PATH and error.code in {401, 403, 404}:
                raise GateError(f"Cannot read complete classic main protection ({error.code}). {PROTECTION_SETUP}") from error
            raise GateError(f"GitHub authority lookup failed ({error.code}): {path}") from error

    def pages(self, path: str, key: str) -> list[dict]:
        result = []
        separator = "&" if "?" in path else "?"
        for page in range(1, 101):
            response = self.get(f"{path}{separator}per_page=100&page={page}")
            data = response.get(key) if isinstance(response, dict) else None
            if not isinstance(data, list) or any(not isinstance(item, dict) for item in data):
                raise GateError("GitHub returned an invalid authority inventory")
            result.extend(data)
            if len(data) < 100:
                return result
        raise GateError("authority pagination exceeded bound")


def validate_request(candidate: str, required: dict) -> None:
    if not isinstance(candidate, str) or not re.fullmatch(r"[0-9a-f]{40}", candidate):
        raise GateError("candidate must be a full immutable SHA")
    if (not isinstance(required, dict) or not required
            or any(not isinstance(name, str) or not name
                   or not isinstance(workflow, str)
                   or not re.fullmatch(r"\.github/workflows/[A-Za-z0-9_.-]+\.ya?ml", workflow)
                   for name, workflow in required.items())):
        raise GateError("required-check policy is missing or invalid")


def current_source(api, candidate: str) -> None:
    branch = api.get("branches/main")
    if not isinstance(branch, dict) or not isinstance(branch.get("commit"), dict):
        raise GateError("GitHub returned invalid main authority")
    if branch.get("commit", {}).get("sha") != candidate:
        raise GateError("candidate must be the exact current main commit")
    if branch.get("protected") is not True:
        raise GateError("main is not protected")


def protected_source(api, candidate: str, required: dict) -> dict:
    current_source(api, candidate)
    protection = api.get(PROTECTION_PATH)
    if not isinstance(protection, dict):
        raise GateError("GitHub returned invalid branch protection authority")
    checks = protection.get("required_status_checks") or {}
    if (not isinstance(checks, dict) or not isinstance(checks.get("contexts", []), list)
            or not isinstance(checks.get("checks", []), list)
            or any(not isinstance(entry, str) for entry in checks.get("contexts", []))
            or any(not isinstance(entry, dict) or not isinstance(entry.get("context"), str)
                   for entry in checks.get("checks", []))):
        raise GateError("GitHub returned invalid required protection checks")
    contexts = set(checks.get("contexts", [])) | {entry["context"] for entry in checks.get("checks", [])}
    if not set(required) <= contexts or checks.get("strict") is not True:
        raise GateError("main protection omits required contexts or up-to-date merge enforcement")
    admins = protection.get("enforce_admins")
    if not isinstance(admins, dict) or admins.get("enabled") is not True:
        raise GateError("main protection must enforce administrators")
    for name in ("allow_force_pushes", "allow_deletions"):
        flag = protection.get(name)
        if not isinstance(flag, dict) or flag.get("enabled") is not False:
            raise GateError("main protection permits destructive bypass or omits its explicit prohibition")
    return {"branch": "main", "sha": candidate, "required_contexts": sorted(contexts),
            "strict": True, "enforce_admins": True, "allow_force_pushes": False, "allow_deletions": False}


def check_run_id(api, check: dict, candidate: str) -> int:
    if check.get("head_sha") != candidate:
        raise GateError("check source does not match the exact candidate")
    app = check.get("app")
    if not isinstance(app, dict) or app.get("slug") != "github-actions":
        raise GateError("check is not authored by GitHub Actions")
    details = check.get("details_url")
    match = re.fullmatch(rf"https://github\.com/{re.escape(api.repository)}/actions/runs/([1-9]\d*)(?:/job/[1-9]\d*)?", details) if isinstance(details, str) else None
    if not match:
        raise GateError("check lacks a repository-owned Actions run")
    suite = check.get("check_suite")
    if (type(check.get("id")) is not int or check["id"] <= 0
            or not isinstance(suite, dict) or type(suite.get("id")) is not int or suite["id"] <= 0):
        raise GateError("check lacks a valid check or suite identity")
    return int(match.group(1))


def pending_or_success(value: dict, label: str) -> bool:
    status, conclusion = value.get("status"), value.get("conclusion")
    if not isinstance(status, str) or (conclusion is not None and not isinstance(conclusion, str)):
        raise GateError(f"{label}: malformed latest conclusion")
    if status in PENDING_STATUSES and conclusion is None:
        return True
    if status != "completed" or conclusion != "success":
        raise GateError(f"{label}: failed or invalid latest conclusion ({status}/{conclusion})")
    return False


def ready_checks(api, candidate: str, required: dict) -> dict[str, dict]:
    # One latest snapshot per poll. Reject every observed failure/invalid envelope
    # before considering pending work; no expensive run lookups until it is ready.
    checks = api.pages(f"commits/{candidate}/check-runs?filter=latest", "check_runs")
    if not isinstance(checks, list) or any(not isinstance(check, dict) for check in checks):
        raise GateError("GitHub returned an invalid check inventory")
    selected, pending = {}, []
    for name in required:
        matches = [check for check in checks if check.get("name") == name]
        if not matches:
            pending.append(f"{name} missing")
            continue
        if len(matches) != 1:
            raise GateError(f"{name}: expected one latest check, found {len(matches)}")
        check = matches[0]
        check_run_id(api, check, candidate)
        if pending_or_success(check, name):
            pending.append(f"{name} {check['status']}")
        selected[name] = check
    if pending:
        raise PendingChecks("; ".join(pending))
    return selected


def validate_check(api, check: dict, *, candidate: str, workflow: str, runs: dict | None = None) -> dict:
    run_id = check_run_id(api, check, candidate)
    if pending_or_success(check, check["name"]):
        raise PendingChecks(f"{check['name']} {check['status']}")
    if runs is None:
        runs = {}
    if run_id not in runs:
        runs[run_id] = api.get(f"actions/runs/{run_id}")
    run = runs[run_id]
    if not isinstance(run, dict):
        raise GateError("GitHub returned an invalid workflow-run authority")
    if (run.get("head_sha") != candidate or run.get("path") != workflow
            or not isinstance(run.get("repository"), dict) or run["repository"].get("full_name") != api.repository
            or not isinstance(run.get("head_repository"), dict) or run["head_repository"].get("full_name") != api.repository
            or run.get("event") != "push" or run.get("head_branch") != "main"
            or run.get("check_suite_id") != check["check_suite"]["id"]
            or type(run.get("id")) is not int or run["id"] != run_id
            or type(run.get("run_attempt")) is not int or run["run_attempt"] < 1
            or run.get("html_url") != f"https://github.com/{api.repository}/actions/runs/{run_id}"):
        raise GateError("check producer does not match trusted main-push workflow authority")
    if pending_or_success(run, f"{check['name']} complete workflow"):
        raise PendingChecks(f"{check['name']} complete workflow {run['status']}")
    return {"name": check["name"], "check_id": check["id"], "check_suite_id": run["check_suite_id"],
            "workflow": workflow, "run_id": run["id"], "run_attempt": run["run_attempt"],
            "head_sha": candidate, "conclusion": "success", "completed_at": check.get("completed_at"),
            "url": run["html_url"]}


def trusted_checks(api, candidate: str, required: dict) -> list[dict]:
    selected = ready_checks(api, candidate, required)
    runs, producers, pending = {}, [], []
    for name, workflow in required.items():
        try:
            producers.append(validate_check(api, selected[name], candidate=candidate, workflow=workflow, runs=runs))
        except PendingChecks as error:
            pending.append(str(error))
    if pending:
        raise PendingChecks("; ".join(pending))
    return producers


def wait_checks(api, candidate: str, required: dict, *, wait_seconds: int, poll_seconds: int) -> None:
    validate_request(candidate, required)
    if not 0 <= wait_seconds <= 7200 or not 1 <= poll_seconds <= 60:
        raise GateError("wait must be 0..7200 seconds; poll must be 1..60 seconds")
    deadline = time.monotonic() + wait_seconds
    while True:
        current_source(api, candidate)
        try:
            trusted_checks(api, candidate, required)
        except PendingChecks as error:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise GateError(f"timed out waiting for latest required checks: {error}") from error
            print(f"Awaiting complete main qualification: {error}", flush=True)
            time.sleep(min(poll_seconds, remaining))
            continue
        current_source(api, candidate)
        return


def authorize(api, candidate: str, required: dict) -> dict:
    validate_request(candidate, required)
    if isinstance(api, GitHub):
        api.require_protection_token()
    protection = protected_source(api, candidate, required)
    producers = trusted_checks(api, candidate, required)
    # A concurrent main update invalidates the authorization even if all old checks passed.
    if protected_source(api, candidate, required) != protection:
        raise GateError("main protection changed during authorization")
    return {"schema_version": 1, "kind": "release-authorization", "status": "passed",
            "repository": api.repository, "candidate_sha": candidate, "source_sha": candidate,
            "policy_sha256": digest(POLICY), "protection": protection, "producers": producers,
            "created_at": datetime.now(timezone.utc).isoformat()}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    release = sub.add_parser("authorize-release")
    release.add_argument("--output", type=Path, required=True)
    readiness = sub.add_parser("wait-checks", help="wait for normal main checks before minting a short-lived protection token")
    readiness.add_argument("--wait-seconds", type=int, default=0)
    readiness.add_argument("--poll-seconds", type=int, default=30)
    for command in (release, readiness):
        command.add_argument("--candidate-sha", required=True)
        command.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY"))
    args = parser.parse_args(argv)
    try:
        if args.command == "authorize-release":
            args.output.unlink(missing_ok=True)
        api = GitHub(args.repository or "", os.environ.get("GH_TOKEN", os.environ.get("GITHUB_TOKEN", "")),
                     os.environ.get("SOROTTE_PROTECTION_TOKEN", "") if args.command == "authorize-release" else "")
        if args.command == "authorize-release":
            api.require_protection_token()
        required = json.loads(POLICY.read_text(encoding="utf-8"))["required_checks"]
        if args.command == "wait-checks":
            wait_checks(api, args.candidate_sha, required, wait_seconds=args.wait_seconds, poll_seconds=args.poll_seconds)
            print("Latest required main checks and complete trusted workflows passed; protection authorization is still required.")
            return 0
        receipt = authorize(api, args.candidate_sha, required)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        return 0
    except (GateError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"{args.command} failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
