from __future__ import annotations

import copy
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import candidate_authority as candidate
import merge_gate as gate

BASE, HEAD, MAIN, TREE = (letter * 40 for letter in "abcd")
REQUIRED = {"merge-required": ".github/workflows/rust-ci.yml",
            "package-required": ".github/workflows/package-ci.yml"}


def packed(value: dict) -> bytes:
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("candidate.json", json.dumps(value))
    return stream.getvalue()


class FakeAPI:
    repository = "owner/repo"

    def __init__(self, *, merged=False):
        self.merged = merged
        self.branch = {"protected": True, "commit": {"sha": MAIN if merged else BASE}}
        self.pr = {"number": 59, "state": "closed" if merged else "open", "merged": merged,
                   "head": {"sha": HEAD, "ref": "codex/example", "repo": {"full_name": self.repository}},
                   "base": {"sha": BASE, "ref": "main", "repo": {"full_name": self.repository}},
                   "merge_commit_sha": MAIN}
        self.comparison = {"merge_base_commit": {"sha": BASE}}
        self.commits = {MAIN: {"sha": MAIN, "tree": {"sha": TREE}, "parents": [{"sha": BASE}, {"sha": HEAD}]},
                        HEAD: {"sha": HEAD, "tree": {"sha": TREE}, "parents": [{"sha": BASE}]}}
        self.run = {"id": 123, "run_attempt": 1, "head_sha": HEAD, "head_branch": "codex/example",
                    "path": candidate.WORKFLOW, "event": "workflow_dispatch", "status": "completed", "conclusion": "success",
                    "repository": {"full_name": self.repository}, "head_repository": {"full_name": self.repository},
                    "actor": {"login": "owner"}, "triggering_actor": {"login": "owner"}}
        self.runs = [self.run]
        self.permission = {"permission": "admin", "user": {"login": "owner"}}
        self.artifacts = [self.artifact(index + 10, name) for index, name in enumerate(sorted(candidate.required_artifacts(HEAD, 123, 1)))]
        self.artifacts.append(self.artifact(99, f"qualified-release-candidate-{HEAD}"))
        self.manifest = {"schema_version": 1, "kind": candidate.MANIFEST_KIND, "status": "passed",
                         "repository": self.repository, "candidate_sha": HEAD, "base_sha": BASE,
                         "source_ref": "refs/heads/codex/example", "pull_request": 59, "run_id": 123, "run_attempt": 1,
                         "publication_authorized": False,
                         "policy_sha256": candidate.digest(candidate.ROOT / "coverage/verification-lanes.json"),
                         "artifacts": {value["name"]: {key: value[key] for key in ("id", "name", "digest", "size_in_bytes")}
                                       for value in self.artifacts[:-1]}}
        self.checks, self.pr_runs = [], {}
        for index, (name, workflow) in enumerate(REQUIRED.items()):
            run_id = index + 200
            self.checks.append({"id": index + 300, "name": name, "head_sha": HEAD,
                                "status": "completed", "conclusion": "success", "app": {"slug": "github-actions"},
                                "check_suite": {"id": index + 400},
                                "details_url": f"https://github.com/{self.repository}/actions/runs/{run_id}/job/100"})
            self.pr_runs[run_id] = {**self.run, "id": run_id, "event": "pull_request", "path": workflow,
                                    "check_suite_id": index + 400}
        self.calls = []
        self.downloads = []
        self.run_changed = False
        self.protection_token = True
        self.protection = {"required_status_checks": {"strict": True, "contexts": list(REQUIRED)},
                           "enforce_admins": {"enabled": True}, "allow_force_pushes": {"enabled": False},
                           "allow_deletions": {"enabled": False}}
        self.main_check = {"id": 500, "name": "main-qualified", "head_sha": MAIN,
                           "status": "completed", "conclusion": "success", "app": {"slug": "github-actions"},
                           "check_suite": {"id": 600},
                           "details_url": f"https://github.com/{self.repository}/actions/runs/400"}
        self.pr_runs[400] = {**self.run, "id": 400, "head_sha": MAIN, "head_branch": "main", "event": "push",
                             "path": candidate.MAIN_WORKFLOW, "check_suite_id": 600,
                             "html_url": f"https://github.com/{self.repository}/actions/runs/400"}

    def require_protection_token(self):
        if not self.protection_token:
            raise gate.GateError("protection reader required")

    @staticmethod
    def artifact(identifier, name):
        return {"id": identifier, "name": name, "digest": "sha256:" + "e" * 64,
                "size_in_bytes": 2048, "expired": False,
                "workflow_run": {"id": 123, "head_sha": HEAD, "repository_id": 1, "head_repository_id": 1}}

    def get(self, path):
        self.calls.append(path)
        if path == "branches/main":
            return copy.deepcopy(self.branch)
        if path == gate.PROTECTION_PATH:
            return copy.deepcopy(self.protection)
        if path == "pulls/59":
            return copy.deepcopy(self.pr)
        if path == f"compare/{BASE}...{HEAD}":
            return copy.deepcopy(self.comparison)
        if path.startswith("git/commits/"):
            return copy.deepcopy(self.commits[path.rsplit("/", 1)[1]])
        if path == f"commits/{MAIN}/pulls?per_page=100":
            return [copy.deepcopy(self.pr)]
        if path == "collaborators/owner/permission":
            return copy.deepcopy(self.permission)
        if path.startswith("actions/runs/"):
            run_id = int(path.rsplit("/", 1)[1])
            value = self.run if run_id == 123 else self.pr_runs[run_id]
            value = copy.deepcopy(value)
            if self.run_changed and self.downloads and run_id == 123:
                value["run_attempt"] += 1
            return value
        raise AssertionError(path)

    def pages(self, path, key):
        self.calls.append(path)
        if key == "workflow_runs":
            assert "event=workflow_dispatch&branch=codex%2Fexample&head_sha=" + HEAD in path
            return copy.deepcopy(self.runs)
        if key == "check_runs":
            if path == f"commits/{MAIN}/check-runs?filter=latest":
                return [copy.deepcopy(self.main_check)]
            assert path == f"commits/{HEAD}/check-runs?filter=latest"
            return copy.deepcopy(self.checks)
        assert path == "actions/runs/123/artifacts" and key == "artifacts"
        return copy.deepcopy(self.artifacts)

    def download(self, artifact, output, *, limit):
        self.downloads.append(artifact["id"])
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(packed(self.manifest))


class SourcePromotionTests(unittest.TestCase):
    def promote(self, api):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(candidate, "urlopen", side_effect=AssertionError("unexpected network")):
            return candidate.promote_main(api, MAIN, REQUIRED, Path(temporary))

    def test_unchanged_merge_promotes_original_candidate_without_relabelling_source(self):
        result = self.promote(FakeAPI(merged=True))
        self.assertEqual(result["main_sha"], MAIN)
        self.assertEqual(result["candidate_sha"], HEAD)
        self.assertEqual(result["candidate"]["candidate_sha"], HEAD)
        self.assertFalse(result["application_tests_executed"])
        self.assertFalse(result["candidate"]["publication_authorized"])
        self.assertEqual(result["base_sha"], BASE)
        self.assertEqual(result["tree_sha"], TREE)

    def test_publication_binds_protected_integration_to_original_pr_head(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = candidate.authorize_release(FakeAPI(merged=True), HEAD, REQUIRED, Path(temporary))
        self.assertEqual((result["source_sha"], result["main_sha"]), (HEAD, MAIN))
        self.assertEqual(result["promotion"]["candidate"]["run_id"], 123)
        self.assertEqual([check["name"] for check in result["producers"]], ["main-qualified"])

    def test_publication_rejects_wrong_tag_source_failed_main_and_weakened_protection(self):
        mutations = {
            "missing-app": lambda a: setattr(a, "protection_token", False),
            "failed-main": lambda a: a.main_check.update(conclusion="failure"),
            "fake-main": lambda a: a.pr_runs[400].update(event="workflow_dispatch"),
            "different-main-workflow": lambda a: a.pr_runs[400].update(path=".github/workflows/other.yml"),
            "outdated-allowed": lambda a: a.protection["required_status_checks"].update(strict=False),
            "required-check-removed": lambda a: a.protection["required_status_checks"].update(contexts=[]),
            "admin-bypass": lambda a: a.protection["enforce_admins"].update(enabled=False),
            "failed-candidate": lambda a: a.run.update(conclusion="failure"),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                api = FakeAPI(merged=True)
                mutate(api)
                with self.assertRaises(gate.GateError):
                    candidate.authorize_release(api, HEAD, REQUIRED, Path(temporary))
        with tempfile.TemporaryDirectory() as temporary, self.assertRaisesRegex(gate.GateError, "exact pre-merge"):
            candidate.authorize_release(FakeAPI(merged=True), MAIN, REQUIRED, Path(temporary))

    def test_rejects_changed_merge_outdated_base_and_unrelated_equal_tree(self):
        mutations = {
            "merge-resolution-change": lambda a: a.commits[MAIN]["tree"].update(sha="f" * 40),
            "different-parent": lambda a: a.commits[MAIN]["parents"][1].update(sha="f" * 40),
            "squash-merge": lambda a: a.commits[MAIN].update(parents=[{"sha": BASE}]),
            "outdated-base": lambda a: a.comparison["merge_base_commit"].update(sha="f" * 40),
            "different-pr-source": lambda a: a.pr["head"].update(sha="f" * 40),
            "main-moved": lambda a: a.branch["commit"].update(sha="f" * 40),
            "not-merged": lambda a: a.pr.update(merged=False),
            "foreign-head": lambda a: a.pr["head"]["repo"].update(full_name="foreign/repo"),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                api = FakeAPI(merged=True)
                mutate(api)
                with self.assertRaises(gate.GateError):
                    self.promote(api)
                self.assertEqual(api.downloads, [])

    def test_required_pr_failure_or_foreign_check_blocks_promotion(self):
        mutations = [lambda a: a.checks[0].update(conclusion="failure"),
                     lambda a: a.checks[0].update(conclusion="skipped"),
                     lambda a: a.checks.append(copy.deepcopy(a.checks[0])),
                     lambda a: a.pr_runs[200].update(event="push"),
                     lambda a: a.pr_runs[200].update(path=".github/workflows/other.yml"),
                     lambda a: a.pr_runs[200].update(conclusion="failure"),
                     lambda a: a.checks[0]["app"].update(slug="other")]
        for mutate in mutations:
            api = FakeAPI(merged=True)
            mutate(api)
            with self.assertRaises(gate.GateError):
                self.promote(api)
            self.assertEqual(api.downloads, [])

    def test_failed_latest_candidate_does_not_fall_back_to_older_green(self):
        api = FakeAPI(merged=True)
        api.runs.insert(0, {**api.run, "id": 122})
        api.run["conclusion"] = "failure"
        with self.assertRaisesRegex(gate.GateError, "failed or invalid"):
            self.promote(api)
        self.assertEqual(api.downloads, [])

    def test_missing_candidate_is_pending_and_never_starts_tests(self):
        api = FakeAPI(merged=True)
        api.runs = []
        with self.assertRaisesRegex(gate.PendingChecks, "before merging"):
            self.promote(api)
        self.assertEqual(api.downloads, [])

    def test_candidate_actor_source_workflow_and_branch_are_authority(self):
        for field, value in (("head_sha", "f" * 40), ("path", ".github/workflows/other.yml"),
                             ("event", "push"), ("head_branch", "main"), ("run_attempt", True)):
            api = FakeAPI(merged=True)
            api.run[field] = value
            with self.subTest(field=field), self.assertRaises(gate.GateError):
                self.promote(api)
        api = FakeAPI(merged=True)
        api.permission["permission"] = "read"
        with self.assertRaisesRegex(ValueError, "write authority"):
            self.promote(api)

    def test_expired_missing_duplicate_or_changed_artifact_blocks_promotion(self):
        for variant in ("expired", "missing", "duplicate", "digest", "id", "foreign-source"):
            api = FakeAPI(merged=True)
            if variant == "expired": api.artifacts[0]["expired"] = True
            if variant == "missing": api.artifacts.pop(0)
            if variant == "duplicate": api.artifacts.append(copy.deepcopy(api.artifacts[0]))
            if variant == "digest": api.artifacts[0]["digest"] = "sha256:" + "f" * 64
            if variant == "id": api.artifacts[0]["id"] += 100
            if variant == "foreign-source": api.artifacts[0]["workflow_run"]["head_sha"] = "f" * 40
            with self.subTest(variant=variant), self.assertRaises(gate.GateError):
                self.promote(api)

    def test_manifest_cannot_claim_another_base_pr_attempt_or_policy(self):
        for key, wrong in (("base_sha", "f" * 40), ("pull_request", 60), ("run_attempt", 2),
                           ("candidate_sha", MAIN), ("source_ref", "refs/heads/main"),
                           ("policy_sha256", "f" * 64), ("publication_authorized", True),
                           ("publication_authorized", 0), ("schema_version", True), ("run_attempt", 1.0)):
            api = FakeAPI(merged=True)
            api.manifest[key] = wrong
            with self.subTest(key=key), self.assertRaisesRegex(gate.GateError, "exact PR/base/producer/policy"):
                self.promote(api)

    def test_rerun_started_during_verification_invalidates_observation(self):
        api = FakeAPI(merged=True)
        api.run_changed = True
        with self.assertRaisesRegex(gate.GateError, "changed during verification"):
            self.promote(api)


class CandidateSealingTests(unittest.TestCase):
    def test_open_up_to_date_pr_can_qualify_but_cannot_authorize_publication(self):
        api = FakeAPI()
        authorization = candidate.qualify_source(api, 59, HEAD, 123)
        self.assertFalse(authorization["publication_authorized"])
        sealed = candidate.seal(api, authorization, 123)
        self.assertEqual(sealed, api.manifest)
        self.assertNotIn(gate.PROTECTION_PATH, api.calls)

    def test_failed_job_retry_retains_original_container_attempt_without_rebuilding(self):
        api = FakeAPI()
        authorization = candidate.qualify_source(api, 59, HEAD, 123)
        api.run["run_attempt"] = 2
        durable = next(item for item in api.artifacts if item["name"] == "durable-qualification-123-1")
        durable.update(id=80, name="durable-qualification-123-2")
        api.manifest = candidate.seal(api, authorization, 123)
        self.assertEqual(api.manifest["run_attempt"], 2)
        self.assertIn("server-container-verification-123-1", api.manifest["artifacts"])
        with tempfile.TemporaryDirectory() as temporary:
            self.assertEqual(candidate.require_candidate(api, candidate.pr_subject(api, 59, HEAD, merged=False), Path(temporary)), api.manifest)
        newer = api.artifact(81, "server-container-verification-123-2")
        newer["expired"] = True
        api.artifacts.append(newer)
        with self.assertRaisesRegex(gate.GateError, "expired"):
            candidate.seal(api, authorization, 123)

    def test_head_or_base_changed_while_qualifying_cannot_be_sealed(self):
        for variant in ("head", "base", "state", "attempt"):
            api = FakeAPI()
            authorization = candidate.qualify_source(api, 59, HEAD, 123)
            if variant == "head": api.pr["head"]["sha"] = "f" * 40
            if variant == "base": api.branch["commit"]["sha"] = "f" * 40
            if variant == "state": api.pr.update(state="closed", merged=True)
            if variant == "attempt": api.run["run_attempt"] = 2
            with self.subTest(variant=variant), self.assertRaises(gate.GateError):
                candidate.seal(api, authorization, 123)

    def test_manifest_archive_has_one_bounded_unambiguous_member(self):
        value = FakeAPI().manifest
        self.assertEqual(candidate.manifest_bytes(packed(value)), value)
        for names in (("other.json",), ("../candidate.json",), ("candidate.json", "extra.json")):
            stream = io.BytesIO()
            with zipfile.ZipFile(stream, "w") as archive:
                for name in names:
                    archive.writestr(name, json.dumps(value))
            with self.subTest(names=names), self.assertRaises(gate.GateError):
                candidate.manifest_bytes(stream.getvalue())


if __name__ == "__main__":
    unittest.main()
