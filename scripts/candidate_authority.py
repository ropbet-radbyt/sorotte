"""Bind a pre-merge release candidate to its reviewed PR and unchanged main merge.

This module reads authority. It never builds, executes application tests, merges,
tags or publishes. Original candidate and integration SHAs remain distinct.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import time
import tomllib
from urllib.error import HTTPError
from urllib.parse import quote, urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener, urlopen
import zipfile

import artifact_input
import merge_gate as gate
import native_required
from verification_tools import ROOT, digest

WORKFLOW = ".github/workflows/stable-release.yml"
MAIN_WORKFLOW = ".github/workflows/main-qualification.yml"
MANIFEST_KIND = "sorotte-qualified-pr-candidate"
MAIN_CHECKS = {"main-qualified": MAIN_WORKFLOW}
SHA = re.compile(r"[0-9a-f]{40}\Z")
SHA256 = re.compile(r"sha256:[0-9a-f]{64}\Z")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


class GitHub(gate.GitHub):
    def download(self, artifact: dict, output: Path, *, limit: int) -> None:
        """Keep the API credential out of the signed artifact-host request."""
        request = Request(
            f"https://api.github.com/repos/{self.repository}/actions/artifacts/{artifact['id']}/zip",
            headers={"Authorization": f"Bearer {self.token}", "Accept": "application/vnd.github+json"},
        )
        try:
            with build_opener(NoRedirect).open(request, timeout=30):
                raise gate.GateError("artifact API did not return a signed download redirect")
        except HTTPError as error:
            try:
                if error.code != 302:
                    raise gate.GateError(f"artifact download authorization failed ({error.code})") from error
                location = error.headers.get("Location", "")
            finally:
                error.close()
        parsed = urlsplit(location)
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
            raise gate.GateError("artifact API returned an invalid download destination")
        output.parent.mkdir(parents=True, exist_ok=True)
        hashed, size = hashlib.sha256(), 0
        with urlopen(location, timeout=60) as response, output.open("xb") as stream:
            while chunk := response.read(1024 * 1024):
                size += len(chunk)
                if size > limit:
                    raise gate.GateError("artifact exceeds its download bound")
                stream.write(chunk)
                hashed.update(chunk)
        if f"sha256:{hashed.hexdigest()}" != artifact["digest"]:
            raise gate.GateError("downloaded artifact differs from its immutable GitHub digest")


def sha(value, label: str) -> str:
    if not isinstance(value, str) or not SHA.fullmatch(value) or value == "0" * 40:
        raise gate.GateError(f"{label} must be a full nonzero commit SHA")
    return value


def write(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def pr_subject(api, number: int, source: str, *, merged: bool) -> dict:
    sha(source, "candidate")
    if type(number) is not int or number <= 0:
        raise gate.GateError("candidate requires a positive PR number")
    pr = api.get(f"pulls/{number}")
    if (pr.get("number") != number or pr.get("head", {}).get("sha") != source
            or pr.get("head", {}).get("repo", {}).get("full_name") != api.repository
            or pr.get("base", {}).get("repo", {}).get("full_name") != api.repository
            or pr.get("base", {}).get("ref") != "main"
            or pr.get("state") != ("closed" if merged else "open")
            or pr.get("merged") is not merged):
        raise gate.GateError("candidate is not the requested repository PR head in the required state")
    base = sha(pr["base"].get("sha"), "PR base")
    reference = pr["head"].get("ref")
    if not isinstance(reference, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]*", reference):
        raise gate.GateError("candidate has an invalid branch reference")
    comparison = api.get(f"compare/{base}...{source}")
    if comparison.get("merge_base_commit", {}).get("sha") != base:
        raise gate.GateError("update the PR branch to include its current main base before qualification")
    if not merged:
        gate.current_source(api, base)
    return {"pull_request": number, "candidate_sha": source, "base_sha": base,
            "source_ref": f"refs/heads/{reference}", "merge_sha": pr.get("merge_commit_sha")}


def qualify_source(api, number: int, source: str, run_id: int) -> dict:
    subject = pr_subject(api, number, source, merged=False)
    run = api.get(f"actions/runs/{run_id}")
    validate_candidate_run(api, run, subject, complete=False)
    return {"schema_version": 1, "kind": "pr-candidate-authorization", "status": "passed",
            "repository": api.repository, **subject, "run_id": run_id,
            "run_attempt": run["run_attempt"], "publication_authorized": False,
            "created_at": datetime.now(timezone.utc).isoformat()}


def validate_candidate_run(api, run: dict, subject: dict, *, complete: bool) -> None:
    if (run.get("path") != WORKFLOW or run.get("event") != "workflow_dispatch"
            or run.get("head_sha") != subject["candidate_sha"]
            or f"refs/heads/{run.get('head_branch')}" != subject["source_ref"]
            or run.get("repository", {}).get("full_name") != api.repository
            or run.get("head_repository", {}).get("full_name") != api.repository
            or type(run.get("id")) is not int or run["id"] <= 0
            or type(run.get("run_attempt")) is not int or run["run_attempt"] <= 0):
        raise gate.GateError("candidate producer has different-source or untrusted workflow provenance")
    if complete and gate.pending_or_success(run, "latest pre-merge candidate qualification"):
        raise gate.PendingChecks("pre-merge candidate qualification is still running")
    native_required.actor_authority(api, run)


def required_artifacts(source: str, run_id: int, attempt: int) -> set[str]:
    return {
        "sorotte-gui-windows-x86_64", "sorotte-server-ubuntu-24.04", "sorotte-server-windows-2025",
        f"durable-qualification-{run_id}-{attempt}", f"qualified-container-{source}",
        f"server-container-verification-{run_id}-{attempt}",
    }


def artifact_identity(value: dict, *, source: str, run_id: int, name: str) -> dict:
    producer = value.get("workflow_run", {})
    if (value.get("name") != name or value.get("expired") is not False
            or type(value.get("id")) is not int or value["id"] <= 0
            or type(value.get("size_in_bytes")) is not int or value["size_in_bytes"] <= 0
            or not isinstance(value.get("digest"), str) or not SHA256.fullmatch(value["digest"])
            or producer.get("id") != run_id or producer.get("head_sha") != source
            or producer.get("repository_id") != producer.get("head_repository_id")
            or type(producer.get("repository_id")) is not int or producer["repository_id"] <= 0
            or type(producer.get("head_repository_id")) is not int):
        raise gate.GateError(f"{name}: missing, expired or foreign immutable artifact authority")
    return {key: value[key] for key in ("id", "name", "digest", "size_in_bytes")}


def select_artifacts(api, subject: dict, run: dict, names: set[str] | None = None) -> dict:
    artifacts = api.pages(f"actions/runs/{run['id']}/artifacts", "artifacts")
    if names is None:
        # A failed-job retry may retain the earlier successful container. Select
        # its latest actual evidence artifact, never guess the final run attempt
        # and never fall back from a newer expired or invalid artifact.
        pattern = re.compile(rf"server-container-verification-{run['id']}-([1-9][0-9]*)\Z")
        attempts = [int(match[1]) for item in artifacts
                    if (match := pattern.fullmatch(str(item.get("name", ""))))]
        if not attempts or any(attempt > run["run_attempt"] for attempt in attempts):
            raise gate.GateError("candidate has no valid actual container evidence attempt")
        names = required_artifacts(subject["candidate_sha"], run["id"], run["run_attempt"])
        names.remove(f"server-container-verification-{run['id']}-{run['run_attempt']}")
        names.add(f"server-container-verification-{run['id']}-{max(attempts)}")
    selected = {}
    for name in sorted(names):
        matches = [value for value in artifacts if value.get("name") == name]
        if len(matches) != 1:
            raise gate.GateError(f"{name}: expected one immutable candidate artifact")
        selected[name] = artifact_identity(matches[0], source=subject["candidate_sha"], run_id=run["id"], name=name)
    return selected


def seal(api, authorization: dict, run_id: int) -> dict:
    source, number = authorization.get("candidate_sha"), authorization.get("pull_request")
    fresh = qualify_source(api, number, source, run_id)
    for key in ("kind", "status", "repository", "candidate_sha", "base_sha", "source_ref", "pull_request", "run_id", "publication_authorized"):
        if authorization.get(key) != fresh[key]:
            raise gate.GateError(f"candidate authorization changed before sealing: {key}")
    if (type(authorization.get("run_attempt")) is not int
            or not 1 <= authorization["run_attempt"] <= fresh["run_attempt"]):
        raise gate.GateError("candidate authorization has an invalid original attempt")
    run = api.get(f"actions/runs/{run_id}")
    return {"schema_version": 1, "kind": MANIFEST_KIND, "status": "passed", **{
        key: fresh[key] for key in ("repository", "candidate_sha", "base_sha", "source_ref", "pull_request", "run_id", "run_attempt")},
        "artifacts": select_artifacts(api, fresh, run), "policy_sha256": digest(ROOT / "coverage/verification-lanes.json"),
        "publication_authorized": False}


def manifest_bytes(data: bytes) -> dict:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        entries = archive.infolist()
        if (len(entries) != 1 or entries[0].filename != "candidate.json"
                or entries[0].orig_filename != entries[0].filename or entries[0].file_size > 1024 * 1024):
            raise gate.GateError("candidate manifest artifact has an unexpected inventory")
        return artifact_input.strict_json_loads(archive.read(entries[0]), expected_type=dict, max_bytes=1024 * 1024)


def require_candidate(api, subject: dict, output: Path) -> dict:
    source = subject["candidate_sha"]
    branch = quote(subject["source_ref"].removeprefix("refs/heads/"), safe="")
    runs = api.pages(f"actions/workflows/stable-release.yml/runs?event=workflow_dispatch&branch={branch}&head_sha={source}", "workflow_runs")
    if not runs:
        raise gate.PendingChecks("dispatch qualification for the reviewed PR head before merging")
    if any(type(value.get("id")) is not int or value["id"] <= 0 for value in runs):
        raise gate.GateError("invalid candidate workflow inventory")
    latest = max(runs, key=lambda value: value["id"])
    run = api.get(f"actions/runs/{latest['id']}")
    validate_candidate_run(api, run, subject, complete=True)
    manifest_name = f"qualified-release-candidate-{source}"
    selected = select_artifacts(api, subject, run, {manifest_name})
    archive = output / f"candidate-{run['id']}-{run['run_attempt']}.zip"
    api.download(selected[manifest_name], archive, limit=2 * 1024 * 1024)
    value = manifest_bytes(archive.read_bytes())
    expected = {"schema_version": 1, "kind": MANIFEST_KIND, "status": "passed", "repository": api.repository,
                "run_id": run["id"], "run_attempt": run["run_attempt"], "publication_authorized": False,
                "policy_sha256": digest(ROOT / "coverage/verification-lanes.json"), **{
                    key: subject[key] for key in ("candidate_sha", "base_sha", "source_ref", "pull_request")}}
    if (set(value) != set(expected) | {"artifacts"} or any(value.get(key) != item for key, item in expected.items())
            or any(type(value.get(key)) is not int for key in ("schema_version", "run_id", "run_attempt", "pull_request"))
            or value.get("publication_authorized") is not False):
        raise gate.GateError("qualified manifest does not match this exact PR/base/producer/policy")
    if value["artifacts"] != select_artifacts(api, subject, run):
        raise gate.GateError("qualified artifact identities changed or expired after sealing")
    current = api.get(f"actions/runs/{run['id']}")
    if any(current.get(key) != run.get(key) for key in ("id", "head_sha", "run_attempt", "status", "conclusion")):
        raise gate.GateError("candidate producer changed during verification")
    return value


def pr_checks(api, subject: dict, required: dict) -> list[dict]:
    source = subject["candidate_sha"]
    selected = gate.ready_checks(api, source, required)
    result, runs = [], {}
    for name, check in selected.items():
        run_id = gate.check_run_id(api, check, source)
        if run_id not in runs:
            runs[run_id] = api.get(f"actions/runs/{run_id}")
        run = runs[run_id]
        if (run.get("head_sha") != source or run.get("path") != required[name]
                or run.get("event") != "pull_request" or run.get("id") != run_id
                or run.get("check_suite_id") != check["check_suite"]["id"]
                or run.get("repository", {}).get("full_name") != api.repository
                or run.get("head_repository", {}).get("full_name") != api.repository
                or type(run.get("run_attempt")) is not int or run["run_attempt"] <= 0):
            raise gate.GateError(f"{name}: required check is not the trusted exact-head PR producer")
        if gate.pending_or_success(run, f"{name} complete PR workflow"):
            raise gate.PendingChecks(f"{name}: PR workflow still running")
        result.append({"name": name, "check_id": check["id"], "run_id": run_id,
                       "run_attempt": run["run_attempt"], "workflow": run["path"], "source_sha": source})
    return result


def merged_subject(api, main: str, *, current: bool = True) -> dict:
    sha(main, "main integration")
    if current:
        gate.current_source(api, main)
    commit = api.get(f"git/commits/{main}")
    parents = commit.get("parents", [])
    if len(parents) != 2:
        raise gate.GateError("candidate promotion requires an ordinary two-parent PR merge")
    base, source = (sha(value.get("sha"), "merge parent") for value in parents)
    # GitHub's associated-PR endpoint is an array, unlike Actions inventories.
    associated = api.get(f"commits/{main}/pulls?per_page=100")
    if not isinstance(associated, list):
        raise gate.GateError("GitHub returned an invalid merged-PR inventory")
    matches = [value for value in associated if value.get("merge_commit_sha") == main and value.get("head", {}).get("sha") == source]
    if len(matches) != 1:
        raise gate.GateError("integration commit must identify exactly one merged candidate PR")
    subject = pr_subject(api, matches[0].get("number"), source, merged=True)
    source_commit = api.get(f"git/commits/{source}")
    tree = sha(commit.get("tree", {}).get("sha"), "integration tree")
    if (subject["merge_sha"] != main or subject["base_sha"] != base
            or source_commit.get("tree", {}).get("sha") != tree):
        raise gate.GateError("merge changed the qualified candidate tree or its qualified base")
    return {**subject, "main_sha": main, "tree_sha": tree}


def promote_main(api, main: str, required: dict, output: Path) -> dict:
    subject = merged_subject(api, main)
    checks = pr_checks(api, subject, required)
    candidate = require_candidate(api, subject, output)
    if merged_subject(api, main) != subject or pr_checks(api, subject, required) != checks:
        raise gate.GateError("main, PR identity or required checks changed during promotion")
    return {"schema_version": 1, "kind": "sorotte-main-candidate-promotion", "status": "passed",
            "repository": api.repository, **subject, "pr_checks": checks, "candidate": candidate,
            "application_tests_executed": False}


def authorize_release(api, source: str, required: dict, output: Path) -> dict:
    """Authorize publishing the exact tested head after its unchanged protected merge."""
    api.require_protection_token()
    main_sha = sha(api.get("branches/main").get("commit", {}).get("sha"), "current main")
    protection = gate.protected_source(api, main_sha, required)
    main_checks = gate.trusted_checks(api, main_sha, MAIN_CHECKS)
    promotion = promote_main(api, main_sha, required, output)
    if source != promotion["candidate_sha"]:
        raise gate.GateError("release must tag the exact pre-merge qualified candidate, not rebuild the integration commit")
    if (gate.protected_source(api, main_sha, required) != protection
            or gate.trusted_checks(api, main_sha, MAIN_CHECKS) != main_checks):
        raise gate.GateError("source protection or main qualification changed during release authorization")
    return {"schema_version": 2, "kind": "release-authorization", "status": "passed",
            "repository": api.repository, "candidate_sha": source, "source_sha": source,
            "main_sha": main_sha, "protection": protection, "producers": main_checks,
            "promotion": promotion, "created_at": datetime.now(timezone.utc).isoformat()}


def extract_artifact(archive: Path, output: Path) -> None:
    """Extract only bounded regular members below a fresh owned destination."""
    with zipfile.ZipFile(archive) as package:
        members = package.infolist()
        if not members or len(members) > 10000 or sum(item.file_size for item in members) > 4 * 1024**3:
            raise gate.GateError("candidate artifact exceeds its member or expanded-size bound")
        seen, files = set(), []
        for item in members:
            path = PurePosixPath(item.filename)
            mode = item.external_attr >> 16
            parts = item.filename.rstrip("/").split("/")
            if (item.orig_filename != item.filename or not parts
                    or any(part in {"", ".", ".."} or part.startswith(".") for part in parts)
                    or path.is_absolute() or "\\" in item.filename or ":" in item.filename
                    or "\x00" in item.filename or mode & 0o170000 not in {0, 0o100000, 0o040000}):
                raise gate.GateError("candidate artifact contains an unsafe or non-regular path")
            folded = path.as_posix().casefold().rstrip("/")
            if folded in seen:
                raise gate.GateError("candidate artifact contains duplicate or colliding paths")
            seen.add(folded)
            destination = output.joinpath(*parts)
            if destination.exists() or destination.is_symlink():
                raise gate.GateError("candidate artifact would overwrite existing content")
            if not item.is_dir():
                files.append((item, destination))
        # Validate every path before materializing any member. A file cannot be
        # another file's ancestor, including after Windows case folding.
        names = {PurePosixPath(item.filename).as_posix().casefold() for item, _ in files}
        if any(parent.as_posix().casefold() in names for item, _ in files for parent in PurePosixPath(item.filename).parents if str(parent) != "."):
            raise gate.GateError("candidate artifact contains overlapping file paths")
        for item, destination in files:
            destination.parent.mkdir(parents=True, exist_ok=True)
            with package.open(item) as incoming, destination.open("xb") as outgoing:
                shutil.copyfileobj(incoming, outgoing, length=1024 * 1024)


def materialize(api, manifest: dict, name: str, output: Path, *, main_sha: str | None) -> dict:
    """Shared byte handoff, exercised in candidate CI and again by publication."""
    selected = manifest["artifacts"][name]
    archive = output / ".download.zip"
    api.download(selected, archive, limit=4 * 1024**3)
    extract_artifact(archive, output)
    archive.unlink()
    files = {path.relative_to(output).as_posix(): {"sha256": artifact_input.sha256_file(path), "size": path.stat().st_size}
             for path in sorted(output.rglob("*"))
             if path.is_file() and path.relative_to(output).parts[0] != ".authority"}
    return {"schema_version": 1, "kind": "sorotte-candidate-artifact-download", "status": "passed",
            "candidate_sha": manifest["candidate_sha"], "main_sha": main_sha, "qualification_run_id": manifest["run_id"],
            "qualification_run_attempt": manifest["run_attempt"], "artifact": selected, "files": files}


def download_qualified(api, source: str, required: dict, run_id: int, name: str, output: Path) -> dict:
    output.mkdir(parents=True, exist_ok=False)
    authority = authorize_release(api, source, required, output / ".authority")
    manifest = authority["promotion"]["candidate"]
    if manifest["run_id"] != run_id or name not in manifest["artifacts"]:
        raise gate.GateError("download must name an artifact from the authorized pre-merge producer")
    return materialize(api, manifest, name, output, main_sha=authority["main_sha"])


def handoff(api, authorization: dict, run_id: int, output: Path) -> dict:
    """Exercise real Actions downloads and delivery tooling before the PR can pass."""
    import candidate_archives
    import container_candidate

    manifest = seal(api, authorization, run_id)
    output.mkdir(parents=True, exist_ok=False)
    source = manifest["candidate_sha"]
    paths = {}
    for name in manifest["artifacts"]:
        directory = output / name
        directory.mkdir()
        proof = materialize(api, manifest, name, directory, main_sha=None)
        write(directory / ".authority/download.json", proof)
        paths[name] = directory
    archive_paths = [paths[name] for name in (
        "sorotte-gui-windows-x86_64", "sorotte-server-ubuntu-24.04", "sorotte-server-windows-2025",
        f"durable-qualification-{run_id}-{manifest['run_attempt']}")]
    collected = candidate_archives.collect(archive_paths, output / "publication-files", source, server_only=False, premerge=True)
    evidence_name = next(name for name in paths if name.startswith(f"server-container-verification-{run_id}-"))
    evidence_dir = paths[evidence_name]
    local, _ = container_candidate.evidence(evidence_dir, source, api.repository)
    present = subprocess.run(["docker", "image", "inspect", local["id"]],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
    if present.returncode == 0:
        raise gate.GateError("handoff qualification requires a worker without the candidate image")
    restored = container_candidate.load(source, f"sorotte-server:test-{source}", evidence_dir,
                                        paths[f"qualified-container-{source}"], api.repository)
    result = {"schema_version": 1, "kind": "sorotte-candidate-handoff", "status": "passed", "candidate_sha": source,
              "archives": collected, "container": restored, "cold_restore": True, "publication_authorized": False}
    write(output / "handoff.json", result)
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("qualify-source", "seal", "handoff", "require", "promote-main", "authorize-release", "download"))
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--pull-request", type=int)
    parser.add_argument("--run-id", type=int, default=int(os.environ.get("GITHUB_RUN_ID", "0")))
    parser.add_argument("--authorization", type=Path)
    parser.add_argument("--artifact")
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--wait-seconds", type=int, default=0)
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY", ""))
    args = parser.parse_args(argv)
    try:
        if not 0 <= args.wait_seconds <= 5400 or args.output.exists():
            raise gate.GateError("use a fresh output path and a bounded wait")
        from release_qualification import clean_source
        clean_source(ROOT, args.source_sha)
        if args.command in {"authorize-release", "download"}:
            version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
            if os.environ.get("GITHUB_REF") not in {f"refs/tags/v{version}", f"refs/tags/server-v{version}"}:
                raise gate.GateError("publication must use the version tag declared by the qualified source")
        api = GitHub(args.repository, os.environ.get("GH_TOKEN", os.environ.get("GITHUB_TOKEN", "")),
                     os.environ.get("SOROTTE_PROTECTION_TOKEN", "") if args.command in {"authorize-release", "download"} else "")
        required = json.loads(gate.POLICY.read_text(encoding="utf-8"))["required_checks"]
        if args.command == "qualify-source":
            value = qualify_source(api, args.pull_request, args.source_sha, args.run_id)
        elif args.command in {"seal", "handoff"}:
            authorization = artifact_input.strict_json_load(args.authorization, expected_type=dict)
            if authorization.get("candidate_sha") != args.source_sha:
                raise gate.GateError("candidate authorization differs from the checkout")
            if args.command == "handoff":
                handoff(api, authorization, args.run_id, args.output)
                return 0
            value = seal(api, authorization, args.run_id)
        elif args.command == "promote-main":
            value = promote_main(api, args.source_sha, required, args.output.parent / "authority")
        elif args.command == "authorize-release":
            value = authorize_release(api, args.source_sha, required, args.output.parent / "authority")
        elif args.command == "download":
            value = download_qualified(api, args.source_sha, required, args.run_id, args.artifact, args.output)
            write(args.output / ".authority/download.json", value)
            return 0
        else:
            deadline = time.monotonic() + args.wait_seconds
            subject = pr_subject(api, args.pull_request, args.source_sha, merged=False)
            while True:
                try:
                    value = require_candidate(api, subject, args.output.parent / "authority")
                    fresh = pr_subject(api, args.pull_request, args.source_sha, merged=False)
                    if any(subject[key] != fresh[key] for key in ("candidate_sha", "base_sha", "source_ref", "pull_request")):
                        raise gate.GateError("PR source or base changed while qualification was pending")
                    break
                except gate.PendingChecks as error:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise
                    print(f"Candidate qualification pending: {error}", flush=True)
                    time.sleep(min(30, remaining))
        write(args.output, value)
        if args.github_output is not None:
            manifest = value["promotion"]["candidate"]
            with args.github_output.open("a", encoding="utf-8", newline="\n") as stream:
                stream.write(f"qualification_run_id={manifest['run_id']}\nqualification_run_attempt={manifest['run_attempt']}\n")
                container_evidence = next(name for name in manifest["artifacts"] if name.startswith(f"server-container-verification-{manifest['run_id']}-"))
                stream.write(f"container_evidence_artifact={container_evidence}\n")
        return 0
    except (gate.GateError, ValueError, OSError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
        if args.command == "handoff" and args.output.is_dir() and not (args.output / "handoff.json").exists():
            write(args.output / "handoff.json", {"schema_version": 1, "kind": "sorotte-candidate-handoff", "status": "failed",
                                                "candidate_sha": args.source_sha, "reason": str(error), "publication_authorized": False})
        print(f"{args.command} failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
