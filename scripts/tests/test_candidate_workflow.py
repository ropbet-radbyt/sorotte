"""Protect phase boundaries so main and tags cannot add application qualification."""
from __future__ import annotations

import copy
from pathlib import Path
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
QUALIFIERS = {
    "server-archives": "qualify-server-archives.yml",
    "gui-archive": "qualify-gui-archive.yml",
    "container-candidate": "qualify-server-container.yml",
}
PR_WORKFLOWS = ("rust-ci.yml", "rust-mutation.yml", "rust-fuzz.yml", "dependency-policy.yml",
                "package-ci.yml", "native-required.yml")


def workflows():
    return {path.name: yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
            for path in (ROOT / ".github/workflows").glob("*.yml")}


def step(job, identifier):
    matches = [value for value in job.get("steps", []) if value.get("id") == identifier]
    assert len(matches) == 1, identifier
    result = matches[0]
    assert "continue-on-error" not in result, identifier
    return result


def dependencies(job):
    needs = job.get("needs", [])
    return {needs} if isinstance(needs, str) else set(needs)


def validate(value):
    for path in ("stable-release.yml", "package-ci.yml", "main-qualification.yml", "publish-qualified-archives.yml", "publish-server-container.yml"):
        assert value[path]["permissions"]["pull-requests"] == "read"
    stable = value["stable-release.yml"]
    jobs = stable["jobs"]
    assert set(jobs) == {"authorize-source", "playback-lifecycle-release-gate", "default-workspace",
                         *QUALIFIERS, "retain-release-qualification", "publish-source", "archives", "container"}
    assert stable["on"]["push"] == {"tags": ["v*", "server-v*"]}
    assert stable["on"]["workflow_dispatch"]["inputs"]["publish"]["default"] == "false"
    assert jobs["authorize-source"]["if"] == "github.event_name == 'workflow_dispatch' && !inputs.publish"
    assert jobs["publish-source"]["if"] == "github.event_name == 'push' || inputs.publish"
    assert "candidate_authority.py qualify-source" in step(jobs["authorize-source"], "authorize_pr")["run"]
    assert "candidate_authority.py authorize-release" in step(jobs["publish-source"], "authorize")["run"]
    candidate_jobs = {"authorize-source", "playback-lifecycle-release-gate", "default-workspace",
                      *QUALIFIERS, "retain-release-qualification"}
    for name in candidate_jobs:
        job = jobs[name]
        assert "secrets" not in job
        assert all(permission == "read" for permission in job.get("permissions", stable["permissions"]).values())
        assert "SOROTTE_PROTECTION" not in str(job)
        assert "create-github-app-token" not in str(job)
        assert "continue-on-error" not in job
        if name != "authorize-source":
            assert "if" not in job  # Failed/skipped prerequisites must propagate.
    for name in ("playback-lifecycle-release-gate", "default-workspace"):
        assert dependencies(jobs[name]) == {"authorize-source"}
    assert jobs["playback-lifecycle-release-gate"]["with"] == {"candidate_sha": "${{ github.sha }}", "channel": "stable"}
    assert {row["platform"] for row in jobs["default-workspace"]["strategy"]["matrix"]["include"]} == {"linux-x86_64", "windows-x86_64"}
    for name, path in QUALIFIERS.items():
        assert dependencies(jobs[name]) == {"playback-lifecycle-release-gate", "default-workspace"}
        assert jobs[name]["uses"] == "./.github/workflows/" + path
        child = value[path]
        assert set(child["on"]) == {"workflow_call"}
        assert all(permission == "read" for permission in child["permissions"].values())
        assert "secrets." not in str(child)
        assert "create-github-app-token" not in str(child)
        assert "release_assets.py attach" not in str(child)
        assert "verify_server_container.py publish" not in str(child)

    retain = jobs["retain-release-qualification"]
    assert dependencies(retain) == set(QUALIFIERS)
    handoff, seal = step(retain, "handoff"), step(retain, "seal_candidate")
    for item, command in ((handoff, "handoff"), (seal, "seal")):
        assert "if" not in item
        assert f"candidate_authority.py {command} --source-sha \"$GITHUB_SHA\"" in item["run"]
        assert "--authorization target/durable-qualification/authorization/pr-candidate-authorization.json" in item["run"]
    assert retain["steps"].index(handoff) < retain["steps"].index(seal)
    assert "qualified-release-candidate-${{ github.sha }}" in str(retain)

    for name, path in (("archives", "publish-qualified-archives.yml"), ("container", "publish-server-container.yml")):
        assert dependencies(jobs[name]) == {"publish-source"}
        assert "if" not in jobs[name]
        assert jobs[name]["uses"] == "./.github/workflows/" + path
        assert jobs[name]["permissions"]["pull-requests"] == "read"
        for identity in ("qualification_run_id", "qualification_run_attempt"):
            assert jobs[name]["with"][identity] == "${{ needs.publish-source.outputs." + identity + " }}"
        publisher = value[path]["jobs"]["publish"]
        assert "verify_server_container.py smoke" not in str(publisher)
        assert "--skip-runtime-smoke" not in str(publisher)
        assert "cargo " not in str(publisher)
        assert "build-push-action" not in str(publisher)
        assert "scripts/verify_gui_release_artifact.py" not in str(publisher)
        assert "scripts/verify_server_release_artifact.py" not in str(publisher)
        assert "scripts/candidate_authority.py download" in str(publisher)
        assert "scripts/candidate_authority.py authorize-release" in str(publisher)
    archive = value["publish-qualified-archives.yml"]["jobs"]["publish"]
    assert jobs["container"]["with"]["container_evidence_artifact"] == "${{ needs.publish-source.outputs.container_evidence_artifact }}"
    container = value["publish-server-container.yml"]["jobs"]["publish"]
    restore = step(container, "restore_candidate")
    reauthorize = step(container, "reauthorize_publication")
    push = step(container, "ci_push_only_tags_of_the_already_tested_d_e9d5952d")
    assert "container_candidate.py load" in restore["run"] and "if" not in restore
    assert reauthorize["if"] == push["if"] == "inputs.publish"
    assert container["steps"].index(reauthorize) + 1 == container["steps"].index(push)
    for identifier in ("download", "collect", "authorize", "publish", "verify", "inventory"):
        assert "if" not in step(archive, identifier)
    assert [item["id"] for item in archive["steps"] if item.get("id") in {"download", "collect", "authorize", "publish", "verify", "inventory"}] == ["download", "collect", "authorize", "publish", "verify", "inventory"]

    main = value["main-qualification.yml"]
    assert main["on"] == {"push": {"branches": ["main"]}}
    assert set(main["jobs"]) == {"main-qualified"}
    assert all(permission == "read" for permission in main["permissions"].values())
    job = main["jobs"]["main-qualified"]
    assert set(item.get("id") for item in job["steps"]) == {"checkout", "python", "promote", "evidence"}
    assert int(job["timeout-minutes"]) <= 5
    assert "if" not in job and "continue-on-error" not in job
    assert step(job, "promote")["run"] == 'python scripts/candidate_authority.py promote-main --source-sha "$GITHUB_SHA" --output target/main-qualification/promotion.json'
    assert "if" not in step(job, "promote")
    for path in (*PR_WORKFLOWS, "gui-native-interactive.yml", "sorotte-gui-release.yml"):
        assert "push" not in value[path]["on"]
    for job in value["rust-mutation.yml"]["jobs"].values():
        for item in job.get("steps", []):
            if "FULL" in item.get("env", {}):
                assert item["env"]["FULL"] == "1"
    assert value["rust-mutation.yml"]["jobs"]["mutation"]["strategy"].get("max-parallel") == "8"
    container_candidate = value["qualify-server-container.yml"]["jobs"]["qualify"]
    assert "org.opencontainers.image.version=${{ steps.build_info.outputs.version }}" in step(container_candidate, "meta")["with"]["labels"].splitlines()
    package = value["package-ci.yml"]["jobs"]["package-required"]
    required = step(package, "candidate_required")
    assert required["if"] == "github.event_name == 'pull_request'"
    assert 'candidate_authority.py require --source-sha "$VERIFICATION_SHA" --pull-request "$PR_NUMBER"' in required["run"]
    assert required["env"]["PR_NUMBER"] == "${{ github.event.pull_request.number }}"
    base = step(value["rust-ci.yml"]["jobs"]["preflight"], "pr_base")
    assert base["if"] == "github.event_name == 'pull_request'"
    assert base["env"]["PR_BASE_SHA"] == "${{ github.event.pull_request.base.sha }}"
    assert "'merge-base', '--is-ancestor'" in base["run"] and "check=True" in base["run"]


class CandidateWorkflowTests(unittest.TestCase):
    def test_checked_in_pipeline_keeps_application_and_handoff_failures_in_pr(self):
        validate(workflows())

    def test_postmerge_execution_credentials_and_missing_premerge_gates_are_rejected(self):
        defects = {
            "postmerge-rust": lambda w: w["rust-ci.yml"]["on"].update(push={"branches": ["main"]}),
            "postmerge-native": lambda w: w["gui-native-interactive.yml"]["on"].update(push={"branches": ["main"]}),
            "tag-application": lambda w: w["stable-release.yml"]["jobs"]["authorize-source"].update({"if": "true"}),
            "candidate-secrets": lambda w: w["stable-release.yml"]["jobs"]["gui-archive"].update(secrets="inherit"),
            "candidate-write": lambda w: w["qualify-server-container.yml"]["permissions"].update(packages="write"),
            "early-packages": lambda w: w["stable-release.yml"]["jobs"]["server-archives"].update(needs="playback-lifecycle-release-gate"),
            "ignored-failure": lambda w: w["stable-release.yml"]["jobs"]["retain-release-qualification"].update({"if": "always()"}),
            "missing-container": lambda w: w["stable-release.yml"]["jobs"]["retain-release-qualification"]["needs"].remove("container-candidate"),
            "skipped-handoff": lambda w: step(w["stable-release.yml"]["jobs"]["retain-release-qualification"], "handoff").update({"if": "false"}),
            "tolerated-candidate": lambda w: step(w["package-ci.yml"]["jobs"]["package-required"], "candidate_required").update({"continue-on-error": "true"}),
            "optional-candidate": lambda w: step(w["package-ci.yml"]["jobs"]["package-required"], "candidate_required").update({"if": "false"}),
            "rebuild-container": lambda w: w["publish-server-container.yml"]["jobs"]["publish"]["steps"].append({"run": "cargo build --release"}),
            "test-main": lambda w: w["main-qualification.yml"]["jobs"]["main-qualified"]["steps"].append({"id": "late_test", "run": "cargo test"}),
            "missing-archive-authority": lambda w: step(w["publish-qualified-archives.yml"]["jobs"]["publish"], "authorize").update({"if": "false"}),
            "unbound-candidate": lambda w: w["stable-release.yml"]["jobs"]["archives"]["with"].update(qualification_run_id="123"),
            "wrong-base": lambda w: step(w["rust-ci.yml"]["jobs"]["preflight"], "pr_base")["env"].update(PR_BASE_SHA="HEAD^"),
            "assumed-container-attempt": lambda w: w["stable-release.yml"]["jobs"]["container"]["with"].update(container_evidence_artifact="server-container-verification-123-2"),
            "stale-container-authority": lambda w: step(w["publish-server-container.yml"]["jobs"]["publish"], "reauthorize_publication").update({"if": "false"}),
            "unbounded-mutation-concurrency": lambda w: w["rust-mutation.yml"]["jobs"]["mutation"]["strategy"].pop("max-parallel"),
            "sha-version-label": lambda w: step(w["qualify-server-container.yml"]["jobs"]["qualify"], "meta")["with"].update(labels="org.opencontainers.image.revision=${{ github.sha }}"),
        }
        original = workflows()
        for name, mutate in defects.items():
            with self.subTest(name=name):
                changed = copy.deepcopy(original)
                mutate(changed)
                self.assertNotEqual(changed, original)
                with self.assertRaises(AssertionError):
                    validate(changed)


if __name__ == "__main__":
    unittest.main()
