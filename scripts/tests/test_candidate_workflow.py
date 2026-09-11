"""Protect phase boundaries so main and tags cannot add application qualification."""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

from scripts import compat_live_interop

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
    for path in PR_WORKFLOWS:
        assert value[path]["env"]["VERIFICATION_SHA"] == "${{ github.event.pull_request.head.sha || github.sha }}"
        for job in value[path]["jobs"].values():
            for item in job.get("steps", []):
                assert "GITHUB_SHA" not in item.get("run", ""), (path, item.get("id"))
                assert "github.sha" not in item.get("run", ""), (path, item.get("id"))
    for job in value["rust-mutation.yml"]["jobs"].values():
        for item in job.get("steps", []):
            if "FULL" in item.get("env", {}):
                assert item["env"]["FULL"] == "1"
    assert value["rust-mutation.yml"]["jobs"]["mutation"]["strategy"].get("max-parallel") == "8"
    container_candidate = value["qualify-server-container.yml"]["jobs"]["qualify"]
    assert "org.opencontainers.image.version=${{ steps.build_info.outputs.version }}" in step(container_candidate, "meta")["with"]["labels"].splitlines()
    package = value["package-ci.yml"]["jobs"]["package-required"]
    assert package["if"] == "${{ !cancelled() }}"
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
            "uncancellable-package-gate": lambda w: w["package-ci.yml"]["jobs"]["package-required"].update({"if": "always()"}),
            "failure-skipping-package-gate": lambda w: w["package-ci.yml"]["jobs"]["package-required"].update({"if": "success()"}),
            "rebuild-container": lambda w: w["publish-server-container.yml"]["jobs"]["publish"]["steps"].append({"run": "cargo build --release"}),
            "test-main": lambda w: w["main-qualification.yml"]["jobs"]["main-qualified"]["steps"].append({"id": "late_test", "run": "cargo test"}),
            "missing-archive-authority": lambda w: step(w["publish-qualified-archives.yml"]["jobs"]["publish"], "authorize").update({"if": "false"}),
            "unbound-candidate": lambda w: w["stable-release.yml"]["jobs"]["archives"]["with"].update(qualification_run_id="123"),
            "wrong-base": lambda w: step(w["rust-ci.yml"]["jobs"]["preflight"], "pr_base")["env"].update(PR_BASE_SHA="HEAD^"),
            "assumed-container-attempt": lambda w: w["stable-release.yml"]["jobs"]["container"]["with"].update(container_evidence_artifact="server-container-verification-123-2"),
            "stale-container-authority": lambda w: step(w["publish-server-container.yml"]["jobs"]["publish"], "reauthorize_publication").update({"if": "false"}),
            "unbounded-mutation-concurrency": lambda w: w["rust-mutation.yml"]["jobs"]["mutation"]["strategy"].pop("max-parallel"),
            "sha-version-label": lambda w: step(w["qualify-server-container.yml"]["jobs"]["qualify"], "meta")["with"].update(labels="org.opencontainers.image.revision=${{ github.sha }}"),
            "merge-instead-of-head": lambda w: step(w["rust-ci.yml"]["jobs"]["mpv-pr-semantics"], "ci_verify_sorotte_candidate_revision_03e2dbaf").update(run='test "$(git rev-parse HEAD)" = "$GITHUB_SHA"'),
        }
        original = workflows()
        for name, mutate in defects.items():
            with self.subTest(name=name):
                changed = copy.deepcopy(original)
                mutate(changed)
                self.assertNotEqual(changed, original)
                with self.assertRaises(AssertionError):
                    validate(changed)


class PullRequestSourceCommandTests(unittest.TestCase):
    """Exercise source propagation with different real head and event commits."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="candidate-source-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.environment = {key: value for key, value in os.environ.items()
                            if not key.startswith("GIT_") and key != "VERIFICATION_SHA"}
        self.environment.update({
            "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "safe.directory",
            "GIT_CONFIG_VALUE_0": self.root.as_posix(),
            "GIT_AUTHOR_NAME": "Source fixture", "GIT_AUTHOR_EMAIL": "source@example.invalid",
            "GIT_COMMITTER_NAME": "Source fixture", "GIT_COMMITTER_EMAIL": "source@example.invalid",
        })
        self.git("init", "--quiet", "--template=", ".")
        self.git("-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", "Reviewed PR head")
        self.head = self.git("rev-parse", "HEAD")
        self.git("-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", "Different event commit")
        self.event_sha = self.git("rev-parse", "HEAD")
        self.assertNotEqual(self.head, self.event_sha)
        self.git("checkout", "--quiet", "--detach", self.head)
        self.environment.update(GITHUB_SHA=self.event_sha, VERIFICATION_SHA=self.head)

    def git(self, *arguments):
        result = subprocess.run(["git", *arguments], cwd=self.root, env=self.environment,
                                capture_output=True, text=True, encoding="utf-8", timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_live_interop_uses_selected_head_and_rejects_invalid_or_wrong_selections(self):
        expected = {"commit_sha": self.head, "expected_commit_sha": self.head}
        self.assertEqual(compat_live_interop.verify_source(self.root, self.environment), expected)
        for selected in ("", "bad", self.event_sha):
            with self.subTest(selected=selected):
                with self.assertRaises(compat_live_interop.InteropContractError):
                    compat_live_interop.verify_source(
                        self.root, {**self.environment, "GITHUB_SHA": self.head, "VERIFICATION_SHA": selected})
        fallback = {key: value for key, value in self.environment.items() if key != "VERIFICATION_SHA"}
        with self.assertRaises(compat_live_interop.InteropContractError):
            compat_live_interop.verify_source(self.root, fallback)
        fallback["GITHUB_SHA"] = self.head
        self.assertEqual(compat_live_interop.verify_source(self.root, fallback), expected)
        fallback.pop("GITHUB_SHA")
        self.assertEqual(compat_live_interop.verify_source(self.root, fallback), expected)

    def test_actual_mpv_workflow_checks_and_passes_the_selected_head(self):
        bash = shutil.which("bash")
        if os.name == "nt":
            git = Path(shutil.which("git"))
            bash = next((str(path) for path in (git.parent / "bash.exe", git.parent.parent / "bin/bash.exe")
                         if path.is_file()), None)
        if not bash:
            self.skipTest("Bash is unavailable; the Linux PR worker requires it")
        job = workflows()["rust-ci.yml"]["jobs"]["mpv-pr-semantics"]
        guard = step(job, "ci_verify_sorotte_candidate_revision_03e2dbaf")["run"]
        for source, passed in ((self.head, True), (self.event_sha, False)):
            result = subprocess.run([bash, "-e", "-c", guard], cwd=self.root,
                                    env={**self.environment, "VERIFICATION_SHA": source},
                                    capture_output=True, text=True, encoding="utf-8", timeout=15)
            self.assertEqual(result.returncode == 0, passed, result.stderr)

        # Capture the application's actual argv at the process boundary without
        # substituting shell expansion or running the expensive player suite.
        script = self.root / "scripts/playback_lifecycle_system.py"
        script.parent.mkdir()
        script.write_text("import json, sys\nprint(json.dumps(sys.argv[1:]))\n", encoding="utf-8")
        python = shlex.quote(Path(sys.executable).as_posix())
        lifecycle = next(item for item in job["steps"] if item.get("id") == "playback_lifecycle_system")
        command = f'python3() {{ {python} "$@"; }}\n' + lifecycle["run"]
        result = subprocess.run([bash, "-e", "-c", command], cwd=self.root, env=self.environment,
                                capture_output=True, text=True, encoding="utf-8", timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        arguments = json.loads(result.stdout)
        self.assertEqual(arguments[arguments.index("--candidate-sha") + 1], self.head)


if __name__ == "__main__":
    unittest.main()
