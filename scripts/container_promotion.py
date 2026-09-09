#!/usr/bin/env python3
"""Authorize digest-only promotion with separate tooling and released sources.

Every assignment re-queries live authority. A prior JSON receipt only fixes the
identities that must still match; it never grants publication permission.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import uuid

import merge_gate as gate
import candidate_authority as candidate
import release_qualification as qualification
import verification_tools
import verify_server_container as container

ROOT = verification_tools.ROOT
REPOSITORY = "ropbet-radbyt/sorotte"
WORKFLOW = ".github/workflows/publish-server-container.yml"
TAG = re.compile(r"(?:server-)?v[0-9]+\.[0-9]+\.[0-9]+")


class PromotionError(ValueError):
    pass


def write(path: Path, value: dict) -> None:
    qualification.write(path, value)


def failure(path: Path, error: Exception, environ: dict, **fields) -> None:
    message = str(error)
    for key in ("GH_TOKEN", "GITHUB_TOKEN", "SOROTTE_PROTECTION_TOKEN"):
        if environ.get(key):
            message = message.replace(environ[key], "<redacted>")
    write(path, {"schema_version": 1, "status": "failed", "error_type": type(error).__name__,
                 "error": message[:2000], **fields})


def request(args) -> dict:
    value = {key: getattr(args, key) for key in
             ("tooling_sha", "publication_run_id", "version_tag", "approved_digest")}
    if (not isinstance(value["tooling_sha"], str) or not qualification.SHA.fullmatch(value["tooling_sha"])
            or not isinstance(value["publication_run_id"], str)
            or not re.fullmatch(r"[1-9][0-9]*", value["publication_run_id"])
            or not isinstance(value["version_tag"], str) or not TAG.fullmatch(value["version_tag"])
            or not isinstance(value["approved_digest"], str)
            or not re.fullmatch(r"sha256:[0-9a-f]{64}", value["approved_digest"])):
        raise PromotionError("promotion requires exact tooling SHA, publication run, stable tag and digest")
    return value


def operator_identity(value: dict, environ: dict) -> dict:
    expected = {"GITHUB_REPOSITORY": REPOSITORY, "GITHUB_EVENT_NAME": "workflow_dispatch",
                "GITHUB_REF": "refs/heads/main", "GITHUB_SHA": value["tooling_sha"],
                "GITHUB_WORKFLOW_SHA": value["tooling_sha"],
                "GITHUB_WORKFLOW_REF": f"{REPOSITORY}/{WORKFLOW}@refs/heads/main"}
    if any(environ.get(key) != item for key, item in expected.items()):
        raise PromotionError("operator must execute the exact current-main promotion workflow by manual dispatch")
    for key in ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT"):
        item = environ.get(key)
        if not isinstance(item, str) or not re.fullmatch(r"[1-9][0-9]*", item):
            raise PromotionError("operator run and actual attempt are required")
        expected[key] = item
    return expected


def assert_checkout(tooling_sha: str) -> dict:
    outputs = []
    for command in (("rev-parse", "HEAD"), ("status", "--porcelain=v1", "--untracked-files=all")):
        result = subprocess.run(["git", "-c", f"safe.directory={ROOT.as_posix()}", *command],
                                cwd=ROOT, capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise PromotionError("cannot verify the promotion tooling checkout")
        outputs.append(result.stdout.strip())
    if outputs != [tooling_sha, ""]:
        raise PromotionError("promotion tooling checkout must be clean at the exact operator SHA")
    return {"tooling_sha": tooling_sha, "policy_sha256": verification_tools.digest(gate.POLICY)}


class RecordedGitHub(gate.GitHub):
    """Reuse the strict check/protection client; retain GET data without headers."""
    def __init__(self, output: Path, environ: dict):
        super().__init__(REPOSITORY, environ.get("GH_TOKEN", environ.get("GITHUB_TOKEN", "")),
                         environ.get("SOROTTE_PROTECTION_TOKEN", ""))
        self.output = output
        self.output.mkdir()
        self.deadline = time.monotonic() + 300
        self.observations = []

    def get(self, path: str):
        if time.monotonic() >= self.deadline:
            raise PromotionError("promotion authority exceeded its five-minute lookup budget")
        number = len(self.observations) + 1
        record = self.output / f"observation-{number:03}.json"
        item = {"method": "GET", "url": f"https://api.github.com/repos/{self.repository}/{path}"}
        self.observations.append({"path": str(record), "endpoint": path})
        try:
            value = super().get(path)
        except Exception as error:
            write(record, {**item, "error_type": type(error).__name__})
            self.observations[-1]["sha256"] = verification_tools.digest(record)
            raise
        write(record, {**item, "observed_at": datetime.now(timezone.utc).isoformat(), "response": value})
        self.observations[-1]["sha256"] = verification_tools.digest(record)
        return value


def validate_operator_run(api, operator: dict) -> None:
    run_id = int(operator["GITHUB_RUN_ID"])
    run = api.get(f"actions/runs/{run_id}")
    if (run.get("id"), run.get("run_attempt"), run.get("head_sha"), run.get("event"),
            run.get("head_branch"), run.get("path"), run.get("html_url")) != (
            run_id, int(operator["GITHUB_RUN_ATTEMPT"]), operator["GITHUB_SHA"], "workflow_dispatch",
            "main", WORKFLOW, f"https://github.com/{REPOSITORY}/actions/runs/{run_id}"):
        raise PromotionError("official operator run differs from the executing workflow")
    if (run.get("repository", {}).get("full_name") != REPOSITORY
            or run.get("head_repository", {}).get("full_name") != REPOSITORY
            or run.get("status") not in gate.PENDING_STATUSES or run.get("conclusion") is not None):
        raise PromotionError("operator run is foreign or no longer active")


def release_identity(api, version_tag: str, tooling_sha: str) -> dict:
    prefix = f"https://api.github.com/repos/{REPOSITORY}"
    reference = api.get(f"git/ref/tags/{version_tag}")
    obj = reference.get("object", {})
    tag_sha = obj.get("sha")
    if (reference.get("ref") != f"refs/tags/{version_tag}" or obj.get("type") != "tag"
            or not isinstance(tag_sha, str) or not qualification.SHA.fullmatch(tag_sha)
            or reference.get("url") != f"{prefix}/git/refs/tags/{version_tag}"):
        raise PromotionError("release must have the exact annotated tag reference")
    tag = api.get(f"git/tags/{tag_sha}")
    source = tag.get("object", {}).get("sha")
    if ((tag.get("sha"), tag.get("tag"), tag.get("url"), tag.get("object", {}).get("type"))
            != (tag_sha, version_tag, f"{prefix}/git/tags/{tag_sha}", "commit")
            or not isinstance(source, str) or not qualification.SHA.fullmatch(source)):
        raise PromotionError("annotated release tag must directly bind one immutable source commit")
    release = api.get("releases/latest")
    release_id = release.get("id")
    if (type(release_id) is not int or release_id < 1 or release.get("tag_name") != version_tag
            or release.get("draft") is not False or release.get("prerelease") is not False
            or release.get("url") != f"{prefix}/releases/{release_id}"
            or release.get("html_url") != f"https://github.com/{REPOSITORY}/releases/tag/{version_tag}"
            or not isinstance(release.get("published_at"), str) or not release["published_at"]):
        raise PromotionError("requested release is not the current published latest stable release")
    comparison = api.get(f"compare/{source}...{tooling_sha}")
    ahead = comparison.get("ahead_by")
    if (comparison.get("base_commit", {}).get("sha") != source
            or comparison.get("merge_base_commit", {}).get("sha") != source
            or type(comparison.get("behind_by")) is not int or comparison["behind_by"] != 0
            or type(ahead) is not int or ahead < 0
            or comparison.get("status") != ("identical" if source == tooling_sha else "ahead")
            or (ahead == 0) != (source == tooling_sha)):
        raise PromotionError("published source must be an ancestor of the exact tooling main commit")
    return {"candidate_sha": source, "version_tag": version_tag, "annotated_tag_object": tag_sha,
            "release_id": release_id, "release_published_at": release["published_at"]}


def container_authority(api, value: dict, source: str) -> dict:
    run_id, version = value["publication_run_id"], value["version_tag"]
    run = api.get(f"actions/runs/{run_id}")
    qualification.validate_producer_run(run, source, REPOSITORY, run_id, version)
    jobs = qualification.producer_collection(api.get, f"actions/runs/{run_id}/jobs?filter=all", "jobs")
    artifacts = qualification.producer_collection(api.get, f"actions/runs/{run_id}/artifacts", "artifacts")
    selected = qualification.select_container_producer(run, jobs, artifacts, source, REPOSITORY, run_id, version)
    job = api.get(f"actions/jobs/{selected['container_job_id']}")
    artifact = api.get(f"actions/artifacts/{selected['artifact_id']}")
    if (job != next(item for item in jobs if item["id"] == selected["container_job_id"])
            or artifact != next(item for item in artifacts if item["id"] == selected["artifact_id"])):
        raise PromotionError("selected original container job or artifact changed during lookup")
    after = api.get(f"actions/runs/{run_id}")
    if qualification.validate_producer_run(after, source, REPOSITORY, run_id, version) != selected["publication_final_attempt"]:
        raise PromotionError("publication attempt changed during authority lookup")
    return selected


def released_source_checks(api, source: str, tooling: str, required: dict) -> list[dict]:
    """Retain legacy release evidence; require the unchanged merge for PR candidates.

    Select by the producer event, never by falling back after a failed validator.
    Older published versions predate pre-merge qualification and retain their
    original trusted main-push checks.
    """
    checks = gate.ready_checks(api, source, required)
    events = {api.get(f"actions/runs/{gate.check_run_id(api, check, source)}").get("event")
              for check in checks.values()}
    if events == {"push"}:
        return gate.trusted_checks(api, source, required)
    if events != {"pull_request"}:
        raise PromotionError("released source has mixed or untrusted qualification events")
    associated = api.get(f"commits/{source}/pulls?per_page=100")
    if not isinstance(associated, list):
        raise PromotionError("released candidate has no merged PR inventory")
    matches = [pr for pr in associated if pr.get("head", {}).get("sha") == source and pr.get("merged_at")]
    if len(matches) != 1:
        raise PromotionError("released candidate must identify exactly one merged PR")
    integration = candidate.sha(matches[0].get("merge_commit_sha"), "released candidate integration")
    subject = candidate.merged_subject(api, integration, current=False)
    comparison = api.get(f"compare/{integration}...{tooling}")
    if (subject["candidate_sha"] != source
            or comparison.get("merge_base_commit", {}).get("sha") != integration):
        raise PromotionError("released candidate integration must remain an ancestor of current main")
    # Original publication and retained immutable container evidence are verified
    # separately; promotion never rebuilds or re-runs application qualification.
    return candidate.pr_checks(api, subject, required)


def collect_authority(value: dict, output: Path, environ: dict) -> dict:
    output.mkdir(parents=True, exist_ok=False)
    write(output / "request.json", value)
    try:
        operator = operator_identity(value, environ)
        checkout = assert_checkout(value["tooling_sha"])
        api = RecordedGitHub(output / "observations", environ)
        api.require_protection_token()
        required = qualification.read(gate.POLICY)["required_checks"]
        validate_operator_run(api, operator)
        protection = gate.protected_source(api, value["tooling_sha"], required)
        tooling = {"candidate_sha": value["tooling_sha"], "protection": protection,
                   "producers": gate.trusted_checks(api, value["tooling_sha"], candidate.MAIN_CHECKS)}
        release = release_identity(api, value["version_tag"], value["tooling_sha"])
        source = release["candidate_sha"]
        historical = released_source_checks(api, source, value["tooling_sha"], required)
        producer = container_authority(api, value, source)
        # Repeat mutable source/channel/check authority after producer lookup.
        if release_identity(api, value["version_tag"], value["tooling_sha"]) != release:
            raise PromotionError("release tag or latest stable identity changed during lookup")
        if released_source_checks(api, source, value["tooling_sha"], required) != historical:
            raise PromotionError("released source check authority changed during lookup")
        if (gate.protected_source(api, value["tooling_sha"], required) != tooling["protection"]
                or gate.trusted_checks(api, value["tooling_sha"], candidate.MAIN_CHECKS) != tooling["producers"]):
            raise PromotionError("tooling protection or trusted checks changed during lookup")
        validate_operator_run(api, operator)
        if assert_checkout(value["tooling_sha"]) != checkout:
            raise PromotionError("tooling checkout changed during authorization")
        identity = {**release, "operator": operator, "tooling_checkout": checkout,
                    "tooling_protection": tooling["protection"], "tooling_producers": tooling["producers"],
                    "historical_producers": historical, "container_producer": producer,
                    "approved_digest": value["approved_digest"]}
        receipt = {"schema_version": 1, "kind": "container-promotion-authority", "status": "passed",
                   "repository": REPOSITORY, "request": value, "identity": identity,
                   "tooling_sha": value["tooling_sha"], "candidate_sha": source,
                   "created_at": datetime.now(timezone.utc).isoformat(),
                   "tooling_authorization": tooling, "observations": api.observations}
        write(output / "authority.json", receipt)
        return receipt
    except Exception as error:
        failure(output / "failure.json", error, environ, phase="live-promotion-authority", request=value)
        raise


def evidence_hashes(directory: Path) -> dict:
    if not directory.is_dir() or directory.is_symlink():
        raise PromotionError("original downloaded publication evidence directory is required")
    files = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink() or (not path.is_file() and not path.is_dir()):
            raise PromotionError("publication evidence contains an indirect or nonregular input")
        if path.is_file():
            files[path.relative_to(directory).as_posix()] = verification_tools.digest(path)
    if not files:
        raise PromotionError("original publication evidence is empty")
    return files


def promote(args, value: dict, environ: dict) -> dict:
    authority_dir, output_dir = args.authority_dir.resolve(), args.output_dir.resolve()
    if authority_dir.is_relative_to(output_dir) or output_dir.is_relative_to(authority_dir):
        raise PromotionError("authority evidence and fresh container output must be separate directories")
    operation = authority_dir / ("promotion-" + str(uuid.uuid4()))
    operation.mkdir(parents=True, exist_ok=False)
    callback_completed = False
    try:
        initial_path = args.initial_authority.resolve()
        initial_digest = verification_tools.digest(initial_path)
        initial = qualification.read(initial_path)
        if (type(initial.get("schema_version")) is not int or initial["schema_version"] != 1
                or initial.get("kind") != "container-promotion-authority" or initial.get("status") != "passed"
                or initial.get("repository") != REPOSITORY or initial.get("request") != value
                or initial.get("candidate_sha") != initial.get("identity", {}).get("candidate_sha")):
            raise PromotionError("initial observation does not fix this exact promotion request")
        operator_identity(value, environ)
        assert_checkout(value["tooling_sha"])
        source = initial["candidate_sha"]
        if not isinstance(source, str) or not qualification.SHA.fullmatch(source):
            raise PromotionError("initial observation lacks the historical publication source")
        evidence = evidence_hashes(args.evidence_dir)

        def before_latest_assignment() -> None:
            nonlocal callback_completed
            if callback_completed:
                raise PromotionError("latest assignment authority callback was invoked more than once")
            fresh = collect_authority(value, operation / "before-assignment", environ)
            if (verification_tools.digest(initial_path) != initial_digest
                    or fresh["identity"] != initial["identity"]
                    or evidence_hashes(args.evidence_dir) != evidence):
                raise PromotionError("initial tag/release/source/producer/artifact or downloaded evidence changed")
            callback_completed = True

        result = container.promote_approved_digest(
            evidence_dir=args.evidence_dir, expected_digest=value["approved_digest"],
            expected_source_sha=source, expected_source_url=f"https://github.com/{REPOSITORY}",
            version_tag=value["version_tag"], output_dir=output_dir,
            before_latest_assignment=before_latest_assignment)
        if not callback_completed:
            raise PromotionError("container promotion returned without fresh assignment authority")
        container._write_json(args.report.resolve(), result)
        write(operation / "receipt.json", {
            "schema_version": 1, "kind": "container-promotion-operation", "status": "passed",
            "request": value, "tooling_sha": value["tooling_sha"], "candidate_sha": source,
            "initial_authority": {"path": str(initial_path), "sha256": initial_digest},
            "before_assignment_authority": {"path": str(operation / "before-assignment/authority.json"),
                                            "sha256": verification_tools.digest(operation / "before-assignment/authority.json")},
            "original_evidence_files": evidence, "report_sha256": verification_tools.digest(args.report.resolve()),
            "limitation": "GitHub authority reads and the registry assignment are separate operations, not an atomic cross-service transaction."})
        return result
    except Exception as error:
        failure(operation / "failure.json", error, environ, phase="promotion", request=value,
                latest_assignment_may_have_occurred=callback_completed)
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("prepare")
    prepare.add_argument("--output-dir", type=Path, required=True)
    promotion = commands.add_parser("promote")
    for name in ("initial-authority", "evidence-dir", "authority-dir", "output-dir", "report"):
        promotion.add_argument("--" + name, type=Path, required=True)
    for command in (prepare, promotion):
        for name in ("tooling-sha", "publication-run-id", "version-tag", "approved-digest"):
            command.add_argument("--" + name, required=True)
    args = parser.parse_args(argv)
    environ = dict(os.environ)
    try:
        value = request(args)
        if args.command == "prepare":
            receipt = collect_authority(value, args.output_dir, environ)
            outputs = {"candidate_sha": receipt["candidate_sha"],
                       "artifact_id": receipt["identity"]["container_producer"]["artifact_id"]}
            if environ.get("GITHUB_OUTPUT"):
                with Path(environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8", newline="\n") as stream:
                    for key, item in outputs.items():
                        stream.write(f"{key}={item}\n")
            print(json.dumps(outputs))
        else:
            promote(args, value, environ)
        return 0
    except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError, container.VerificationError) as error:
        # Detailed, redacted diagnostics live in the fresh authority operation.
        print(f"container promotion {args.command} failed: {type(error).__name__}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
