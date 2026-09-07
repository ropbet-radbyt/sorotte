from __future__ import annotations

import argparse
import copy
from contextlib import ExitStack, redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import container_promotion as promotion

TOOL, SOURCE, TAG_SHA = "a" * 40, "b" * 40, "c" * 40
DIGEST = "sha256:" + "d" * 64
VERSION = "v0.2.11"
REPO = promotion.REPOSITORY
API = f"https://api.github.com/repos/{REPO}"
WORKFLOW = ".github/workflows/rust-ci.yml"
REQUIRED = {"verification-required": WORKFLOW, "merge-required": WORKFLOW}
REQUEST = dict(tooling_sha=TOOL, publication_run_id="12", version_tag=VERSION, approved_digest=DIGEST)
ENV = dict(GITHUB_REPOSITORY=REPO, GITHUB_EVENT_NAME="workflow_dispatch", GITHUB_REF="refs/heads/main",
           GITHUB_SHA=TOOL, GITHUB_WORKFLOW_SHA=TOOL,
           GITHUB_WORKFLOW_REF=f"{REPO}/{promotion.WORKFLOW}@refs/heads/main",
           GITHUB_RUN_ID="99", GITHUB_RUN_ATTEMPT="1", GH_TOKEN="test-normal-secret",
           SOROTTE_PROTECTION_TOKEN="test-protection-secret")


class FakeAPI(promotion.gate.GitHub):
    """Real gate/producer validators consume these independent API envelopes."""
    def __init__(self):
        super().__init__(REPO, ENV["GH_TOKEN"], ENV["SOROTTE_PROTECTION_TOKEN"])
        self.observations = []
        self.calls = {}
        self.on_get = None
        self.branch = {"protected": True, "commit": {"sha": TOOL}}
        self.protection = {
            "required_status_checks": {"strict": True, "contexts": list(REQUIRED)},
            "enforce_admins": {"enabled": True}, "allow_force_pushes": {"enabled": False},
            "allow_deletions": {"enabled": False}}
        self.reference = {"ref": f"refs/tags/{VERSION}", "url": f"{API}/git/refs/tags/{VERSION}",
                          "object": {"type": "tag", "sha": TAG_SHA}}
        self.tag = {"sha": TAG_SHA, "tag": VERSION, "url": f"{API}/git/tags/{TAG_SHA}",
                    "object": {"type": "commit", "sha": SOURCE}}
        self.release = {"id": 700, "tag_name": VERSION, "draft": False, "prerelease": False,
                        "url": f"{API}/releases/700", "html_url": f"https://github.com/{REPO}/releases/tag/{VERSION}",
                        "published_at": "2026-09-07T20:00:00Z"}
        self.comparison = {"base_commit": {"sha": SOURCE}, "merge_base_commit": {"sha": SOURCE},
                           "status": "ahead", "ahead_by": 1, "behind_by": 0}
        self.runs, self.checks = {}, {}
        for run_id, source, suite in ((42, TOOL, 9), (43, SOURCE, 10)):
            self.runs[run_id] = dict(id=run_id, head_sha=source, run_attempt=1, path=WORKFLOW,
                repository={"full_name": REPO}, head_repository={"full_name": REPO}, event="push", head_branch="main",
                status="completed", conclusion="success", check_suite_id=suite,
                html_url=f"https://github.com/{REPO}/actions/runs/{run_id}")
            self.checks[source] = [dict(id=run_id*10+index, name=name, head_sha=source, status="completed", conclusion="success",
                app={"slug": "github-actions"}, check_suite={"id": suite}, details_url=self.runs[run_id]["html_url"],
                completed_at="2026-09-07T19:00:00Z") for index, name in enumerate(REQUIRED, 1)]
        self.runs[99] = dict(id=99, run_attempt=1, head_sha=TOOL, event="workflow_dispatch", head_branch="main",
            path=promotion.WORKFLOW, status="in_progress", conclusion=None,
            repository={"full_name": REPO}, head_repository={"full_name": REPO},
            html_url=f"https://github.com/{REPO}/actions/runs/99")
        self.runs[12] = dict(id=12, run_attempt=1, head_sha=SOURCE, event="push", head_branch=VERSION,
            path=".github/workflows/stable-release.yml", status="completed", conclusion="success",
            repository={"full_name": REPO}, head_repository={"full_name": REPO})
        self.jobs = [dict(id=101, run_id=12, run_attempt=1, head_sha=SOURCE, head_branch=VERSION,
            run_url=f"{API}/actions/runs/12", url=f"{API}/actions/jobs/101", name="container / publish",
            status="completed", conclusion="success", started_at="2026-09-07T19:00:00Z", completed_at="2026-09-07T19:30:00Z",
            steps=[dict(name=name, number=index, status="completed", conclusion="success")
                   for index, name in enumerate(promotion.qualification.CONTAINER_PUBLICATION_STEPS, 10)])]
        self.artifacts = [dict(id=901, name="server-container-verification-12-1", expired=False,
            workflow_run={"id": 12, "head_sha": SOURCE}, url=f"{API}/actions/artifacts/901",
            digest="sha256:" + "e"*64, created_at="2026-09-07T19:29:59Z")]

    def get(self, path):
        self.calls[path] = self.calls.get(path, 0) + 1
        if self.on_get:
            self.on_get(path, self.calls[path])
        self.observations.append({"endpoint": path})
        if path == "branches/main": value = self.branch
        elif path == promotion.gate.PROTECTION_PATH: value = self.protection
        elif path == f"git/ref/tags/{VERSION}": value = self.reference
        elif path == f"git/tags/{TAG_SHA}": value = self.tag
        elif path == "releases/latest": value = self.release
        elif path == f"compare/{SOURCE}...{TOOL}": value = self.comparison
        elif path.startswith("commits/"):
            source = path.split("/")[1]
            value = {"check_runs": self.checks[source]}
        elif path.startswith("actions/runs/12/jobs?"): value = {"total_count": len(self.jobs), "jobs": self.jobs}
        elif path.startswith("actions/runs/12/artifacts?"): value = {"total_count": len(self.artifacts), "artifacts": self.artifacts}
        elif path.startswith("actions/jobs/"): value = next(row for row in self.jobs if row["id"] == int(path.split("/")[-1]))
        elif path.startswith("actions/artifacts/"): value = next(row for row in self.artifacts if row["id"] == int(path.split("/")[-1]))
        else: value = self.runs[int(path.split("/")[-1])]
        return copy.deepcopy(value)


class PromotionAuthorityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        policy = self.root / "policy.json"
        policy.write_text(json.dumps({"required_checks": REQUIRED}))
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(mock.patch.object(promotion.gate, "POLICY", policy))
        self.checkout = self.stack.enter_context(mock.patch.object(promotion, "assert_checkout",
            return_value={"tooling_sha": TOOL, "policy_sha256": promotion.verification_tools.digest(policy)}))
        self.api = FakeAPI()
        self.factory = self.stack.enter_context(mock.patch.object(promotion, "RecordedGitHub", return_value=self.api))

    def prepare(self, name="initial", environ=None):
        return promotion.collect_authority(REQUEST, self.root/name, environ or ENV)

    def assert_rejected(self, name="initial", environ=None):
        with self.assertRaises((ValueError, KeyError, TypeError)):
            self.prepare(name, environ)
        self.assertFalse((self.root/name/"authority.json").exists())
        self.assertEqual(promotion.qualification.read(self.root/name/"failure.json")["status"], "failed")

    def test_distinct_tooling_and_historical_source_require_both_real_gates(self):
        result = self.prepare()
        self.assertEqual(result["candidate_sha"], SOURCE)
        self.assertEqual(result["tooling_authorization"]["candidate_sha"], TOOL)
        self.assertEqual({p["head_sha"] for p in result["identity"]["historical_producers"]}, {SOURCE})
        self.assertEqual(result["identity"]["container_producer"]["artifact_id"], 901)
        self.assertGreaterEqual(self.api.calls["branches/main"], 3)
        for source in (TOOL, SOURCE):
            self.assertEqual(self.api.calls[f"commits/{source}/check-runs?filter=latest&per_page=100&page=1"], 2)
        text = (self.root/"initial/authority.json").read_text()
        self.assertNotIn(ENV["GH_TOKEN"], text)
        self.assertNotIn(ENV["SOROTTE_PROTECTION_TOKEN"], text)

    def test_manual_current_main_operator_environment_is_closed(self):
        for index, (key, wrong) in enumerate((
            ("GITHUB_REPOSITORY", "foreign/repo"), ("GITHUB_EVENT_NAME", "push"),
            ("GITHUB_REF", f"refs/tags/{VERSION}"), ("GITHUB_SHA", SOURCE),
            ("GITHUB_WORKFLOW_SHA", SOURCE), ("GITHUB_WORKFLOW_REF", f"{REPO}/{promotion.WORKFLOW}@refs/tags/{VERSION}"),
            ("GITHUB_RUN_ID", "0"), ("GITHUB_RUN_ATTEMPT", ""))):
            with self.subTest(key=key): self.assert_rejected(str(index), {**ENV, key: wrong})
        self.factory.assert_not_called()

    def test_api_operator_run_must_match_dispatch_attempt_and_repository(self):
        for index, (key, wrong) in enumerate((("run_attempt", 2), ("head_sha", SOURCE), ("event", "push"),
            ("path", ".github/workflows/other.yml"), ("head_repository", {"full_name": "foreign/repo"}),
            ("status", "completed"), ("html_url", "https://example.org/99"))):
            with self.subTest(key=key):
                self.api = FakeAPI(); self.factory.return_value = self.api
                self.api.runs[99][key] = wrong
                self.assert_rejected(str(index))

    def test_tooling_protection_and_current_main_cannot_be_substituted(self):
        for index, defect in enumerate(("main", "protected", "strict", "admins", "deletion")):
            with self.subTest(defect=defect):
                self.api = FakeAPI(); self.factory.return_value = self.api
                if defect == "main": self.api.branch["commit"]["sha"] = SOURCE
                if defect == "protected": self.api.branch["protected"] = False
                if defect == "strict": self.api.protection["required_status_checks"]["strict"] = False
                if defect == "admins": self.api.protection["enforce_admins"]["enabled"] = False
                if defect == "deletion": self.api.protection["allow_deletions"]["enabled"] = True
                self.assert_rejected(str(index))

    def test_historical_and_tooling_checks_retain_failure_duplicate_and_event_rules(self):
        index = 0
        for source in (TOOL, SOURCE):
            for defect in ("failure", "duplicate", "pending", "scheduled", "foreign"):
                with self.subTest(source=source, defect=defect):
                    self.api = FakeAPI(); self.factory.return_value = self.api
                    if defect == "failure": self.api.checks[source][0]["conclusion"] = "failure"
                    if defect == "pending": self.api.checks[source][0].update(status="in_progress", conclusion=None)
                    if defect == "duplicate": self.api.checks[source].append(copy.deepcopy(self.api.checks[source][0]))
                    if defect == "scheduled": self.api.runs[42 if source == TOOL else 43]["event"] = "schedule"
                    if defect == "foreign": self.api.checks[source][0]["details_url"] = "https://github.com/other/repo/actions/runs/42"
                    self.assert_rejected(str(index)); index += 1

    def test_only_exact_annotated_latest_stable_ancestor_is_accepted(self):
        for index, defect in enumerate(("lightweight", "nested", "foreign-tag", "old-release", "draft", "prerelease", "unpublished", "foreign-release", "diverged", "behind", "wrong-base")):
            with self.subTest(defect=defect):
                self.api = FakeAPI(); self.factory.return_value = self.api
                if defect == "lightweight": self.api.reference["object"]["type"] = "commit"
                if defect == "nested": self.api.tag["object"]["type"] = "tag"
                if defect == "foreign-tag": self.api.tag["url"] = "https://api.github.com/repos/foreign/repo/git/tags/" + TAG_SHA
                if defect == "old-release": self.api.release["tag_name"] = "v0.2.12"
                if defect == "draft": self.api.release["draft"] = True
                if defect == "prerelease": self.api.release["prerelease"] = True
                if defect == "unpublished": self.api.release["published_at"] = None
                if defect == "foreign-release": self.api.release["url"] = "https://api.github.com/repos/foreign/repo/releases/700"
                if defect == "diverged": self.api.comparison["merge_base_commit"]["sha"] = "f"*40
                if defect == "behind": self.api.comparison["behind_by"] = 1
                if defect == "wrong-base": self.api.comparison["base_commit"]["sha"] = TOOL
                self.assert_rejected(str(index))

    def test_identity_drift_during_collection_does_not_emit_authority(self):
        def change(path, count):
            if path == "releases/latest" and count == 2:
                self.api.release.update(id=701, url=f"{API}/releases/701")
        self.api.on_get = change
        self.assert_rejected()

    def test_newer_failed_or_ambiguous_container_cannot_reuse_old_green(self):
        for index, defect in enumerate(("failed", "duplicate", "foreign", "expired", "different-source", "parent-failure")):
            with self.subTest(defect=defect):
                self.api = FakeAPI(); self.factory.return_value = self.api
                if defect == "failed":
                    self.api.runs[12]["run_attempt"] = 2
                    self.api.jobs.append({**self.api.jobs[0], "id": 102, "url": f"{API}/actions/jobs/102", "run_attempt": 2, "conclusion": "failure"})
                if defect == "duplicate": self.api.jobs.append(copy.deepcopy(self.api.jobs[0]))
                if defect == "foreign": self.api.jobs[0]["run_id"] = 13
                if defect == "expired": self.api.artifacts[0]["expired"] = True
                if defect == "different-source": self.api.runs[12]["head_sha"] = TOOL
                if defect == "parent-failure": self.api.runs[12]["conclusion"] = "failure"
                self.assert_rejected(str(index))

    def test_parent_durable_retry_keeps_actual_successful_container_attempt(self):
        self.api.runs[12]["run_attempt"] = 2
        self.api.jobs.append(dict(id=103, name="retain-release-qualification", run_attempt=2))
        result = self.prepare()["identity"]["container_producer"]
        self.assertEqual((result["publication_final_attempt"], result["container_attempt"], result["artifact_id"]), (2, 1, 901))

    def test_prepare_cli_emits_only_after_full_live_validation(self):
        outputs = self.root/"outputs"
        argv = ["prepare", "--tooling-sha", TOOL, "--publication-run-id", "12", "--version-tag", VERSION,
                "--approved-digest", DIGEST, "--output-dir", str(self.root/"initial")]
        with mock.patch.dict(os.environ, {**ENV, "GITHUB_OUTPUT": str(outputs)}, clear=True), redirect_stdout(io.StringIO()):
            self.assertEqual(promotion.main(argv), 0)
        self.assertEqual(outputs.read_text().splitlines(), ["candidate_sha="+SOURCE, "artifact_id=901"])
        self.api.runs[12]["conclusion"] = "failure"
        outputs.unlink()
        argv[-1] = str(self.root/"failed")
        with mock.patch.dict(os.environ, {**ENV, "GITHUB_OUTPUT": str(outputs)}, clear=True), redirect_stderr(io.StringIO()):
            self.assertEqual(promotion.main(argv), 1)
        self.assertFalse(outputs.exists())

    def promotion_args(self):
        self.prepare()
        evidence = self.root/"evidence"; evidence.mkdir(); (evidence/"original.json").write_text("{}")
        return argparse.Namespace(initial_authority=self.root/"initial/authority.json", evidence_dir=evidence,
            authority_dir=self.root/"authority", output_dir=self.root/"container", report=self.root/"container/final-gate-report.json")

    def container_fixtures(self):
        image = f"ghcr.io/{REPO.split('/')[0]}/sorotte-server"
        published = dict(image=image, source=f"https://github.com/{REPO}", sourceSha=SOURCE, digest=DIGEST,
                         tags=[f"{image}:{VERSION}", f"{image}:sha-{SOURCE}"], pushes=[])
        public = {"verificationPolicy": {"certificateIdentity": f"https://github.com/{REPO}/{promotion.WORKFLOW}@refs/tags/{VERSION}", "workflowSourceSha": SOURCE}}
        final = dict(status="passed", registryManifestDigest=DIGEST, sourceSha=SOURCE)
        stack = ExitStack()
        stack.enter_context(mock.patch.object(promotion.container, "enforce_final_gate", return_value=final))
        stack.enter_context(mock.patch.object(promotion.container, "parse_publish_report", return_value=published))
        stack.enter_context(mock.patch.object(promotion.container, "parse_publication_report", return_value=public))
        stack.enter_context(mock.patch.object(promotion.container, "verify_publication", return_value={"status": "passed"}))
        runner = stack.enter_context(mock.patch.object(promotion.container, "_run", return_value=subprocess.CompletedProcess([], 0, "[]", "")))
        return stack, runner

    def test_real_promotion_path_preserves_historical_signer_and_single_digest_assignment(self):
        args = self.promotion_args()
        stack, runner = self.container_fixtures()
        with stack:
            result = promotion.promote(args, REQUEST, ENV)
        commands = [call.args[0] for call in runner.call_args_list]
        docker = [command for command in commands if command[0] == "docker"]
        self.assertEqual(docker, [["docker", "buildx", "imagetools", "create", "--prefer-index=false", "--tag",
            f"ghcr.io/{REPO.split('/')[0]}/sorotte-server:latest", f"ghcr.io/{REPO.split('/')[0]}/sorotte-server@{DIGEST}"]])
        for command in commands[:2]:
            self.assertEqual(command[command.index("--certificate-github-workflow-sha")+1], SOURCE)
            self.assertNotIn(TOOL, command)
        self.assertEqual(result["sourceSha"], SOURCE)
        receipts = list(args.authority_dir.glob("promotion-*/receipt.json"))
        self.assertEqual(len(receipts), 1)
        record = promotion.qualification.read(receipts[0])
        self.assertEqual((record["tooling_sha"], record["candidate_sha"]), (TOOL, SOURCE))
        self.assertTrue(Path(record["before_assignment_authority"]["path"]).exists())

    def test_live_latest_drift_in_actual_callback_prevents_registry_mutation(self):
        args = self.promotion_args()
        self.api.release.update(id=701, url=f"{API}/releases/701")
        stack, runner = self.container_fixtures()
        with stack, self.assertRaisesRegex(promotion.PromotionError, "initial tag/release"):
            promotion.promote(args, REQUEST, ENV)
        self.assertEqual([call.args[0][0] for call in runner.call_args_list], ["cosign", "cosign"])
        failures = list(args.authority_dir.glob("promotion-*/failure.json"))
        self.assertFalse(promotion.qualification.read(failures[0])["latest_assignment_may_have_occurred"])

    def test_tampered_initial_receipt_cannot_act_as_bearer_authority(self):
        args = self.promotion_args()
        initial = promotion.qualification.read(args.initial_authority)
        initial["identity"]["historical_producers"] = []
        args.initial_authority.write_text(json.dumps(initial))
        stack, runner = self.container_fixtures()
        with stack, self.assertRaisesRegex(promotion.PromotionError, "initial tag/release"):
            promotion.promote(args, REQUEST, ENV)
        self.assertFalse(any(call.args[0][0] == "docker" for call in runner.call_args_list))

    def test_failed_fresh_historical_check_prevents_assignment(self):
        args = self.promotion_args()
        self.api.checks[SOURCE][0]["conclusion"] = "failure"
        stack, runner = self.container_fixtures()
        with stack, self.assertRaises(promotion.gate.GateError):
            promotion.promote(args, REQUEST, ENV)
        self.assertFalse(any(call.args[0][0] == "docker" for call in runner.call_args_list))
        self.assertEqual(len(list(args.authority_dir.glob("promotion-*/before-assignment/failure.json"))), 1)

    def test_current_tooling_check_drift_in_callback_prevents_assignment(self):
        args = self.promotion_args()
        self.api.checks[TOOL][0]["conclusion"] = "failure"
        stack, runner = self.container_fixtures()
        with stack, self.assertRaises(promotion.gate.GateError):
            promotion.promote(args, REQUEST, ENV)
        self.assertFalse(any(call.args[0][0] == "docker" for call in runner.call_args_list))

    def test_new_main_commit_in_callback_invalidates_old_operator(self):
        args = self.promotion_args()
        self.api.branch["commit"]["sha"] = "f"*40
        stack, runner = self.container_fixtures()
        with stack, self.assertRaisesRegex(promotion.gate.GateError, "exact current main"):
            promotion.promote(args, REQUEST, ENV)
        self.assertFalse(any(call.args[0][0] == "docker" for call in runner.call_args_list))

    def test_initial_receipt_changed_during_signature_verification_rejects(self):
        args = self.promotion_args()
        stack, runner = self.container_fixtures()
        def execute(command):
            if command[:2] == ["cosign", "verify"]:
                initial = promotion.qualification.read(args.initial_authority)
                initial["created_at"] = "changed after initial read"
                args.initial_authority.write_text(json.dumps(initial))
            return subprocess.CompletedProcess(command, 0, "[]", "")
        runner.side_effect = execute
        with stack, self.assertRaisesRegex(promotion.PromotionError, "initial tag/release"):
            promotion.promote(args, REQUEST, ENV)
        self.assertFalse(any(call.args[0][0] == "docker" for call in runner.call_args_list))

    def test_failed_public_verification_after_assignment_retains_partial_mutation_boundary(self):
        args = self.promotion_args()
        stack, runner = self.container_fixtures()
        with stack, mock.patch.object(promotion.container, "verify_publication", side_effect=[
                {"status": "passed"}, promotion.container.VerificationError("post-assignment mismatch")]), \
                self.assertRaisesRegex(promotion.container.VerificationError, "post-assignment"):
            promotion.promote(args, REQUEST, ENV)
        self.assertEqual(sum(call.args[0][0] == "docker" for call in runner.call_args_list), 1)
        failed = list(args.authority_dir.glob("promotion-*/failure.json"))
        self.assertTrue(promotion.qualification.read(failed[0])["latest_assignment_may_have_occurred"])
        self.assertFalse(args.report.exists())

    def test_changed_downloaded_bytes_between_verification_and_assignment_rejects(self):
        args = self.promotion_args()
        def factory(*_):
            (args.evidence_dir/"original.json").write_text('{"changed":true}')
            return self.api
        self.factory.side_effect = factory
        stack, runner = self.container_fixtures()
        with stack, self.assertRaisesRegex(promotion.PromotionError, "downloaded evidence changed"):
            promotion.promote(args, REQUEST, ENV)
        self.assertFalse(any(call.args[0][0] == "docker" for call in runner.call_args_list))

    def test_failed_signature_never_reaches_authority_or_assignment(self):
        args = self.promotion_args()
        self.factory.reset_mock()
        stack, runner = self.container_fixtures()
        runner.side_effect = promotion.container.VerificationError("signature failed")
        with stack, self.assertRaises(promotion.container.VerificationError):
            promotion.promote(args, REQUEST, ENV)
        self.factory.assert_not_called()
        self.assertFalse(any(call.args[0][0] == "docker" for call in runner.call_args_list))

    def test_authority_cannot_precreate_or_live_inside_fresh_container_output(self):
        args = self.promotion_args()
        args.authority_dir = args.output_dir/"authority"
        with self.assertRaisesRegex(promotion.PromotionError, "separate directories"):
            promotion.promote(args, REQUEST, ENV)
        self.assertFalse(args.output_dir.exists())

    def test_existing_evidence_is_never_overwritten_on_repeated_prepare(self):
        self.prepare()
        before = {p.relative_to(self.root/"initial").as_posix():p.read_bytes() for p in (self.root/"initial").rglob("*") if p.is_file()}
        with self.assertRaises(FileExistsError): self.prepare()
        self.assertEqual(before, {p.relative_to(self.root/"initial").as_posix():p.read_bytes() for p in (self.root/"initial").rglob("*") if p.is_file()})


class LocalContractTests(unittest.TestCase):
    def test_request_rejects_partial_digests_nonstable_tags_and_noncanonical_run_ids(self):
        self.assertEqual(promotion.request(argparse.Namespace(**REQUEST)), REQUEST)
        for key, wrong in (("tooling_sha", "abc"), ("approved_digest", "d"*64), ("publication_run_id", "01"),
                           ("publication_run_id", "12\n"), ("version_tag", "v0.2.11-rc1"), ("version_tag", "../../main")):
            with self.subTest(key=key, wrong=wrong), self.assertRaises(promotion.PromotionError):
                promotion.request(argparse.Namespace(**{**REQUEST, key: wrong}))

    def test_recorded_transport_preserves_gets_without_credentials_and_bounds_calls(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            api = promotion.RecordedGitHub(root/"observations", ENV)
            with mock.patch.object(promotion.gate.GitHub, "get", return_value={"commit": {"sha": TOOL}}):
                self.assertEqual(api.get("branches/main"), {"commit": {"sha": TOOL}})
            record = promotion.qualification.read(root/"observations/observation-001.json")
            self.assertEqual((record["method"], record["url"]), ("GET", API+"/branches/main"))
            text = json.dumps(record)
            self.assertNotIn("Authorization", text)
            self.assertNotIn(ENV["GH_TOKEN"], text)
            with mock.patch.object(promotion.time, "monotonic", return_value=api.deadline), mock.patch.object(promotion.gate.GitHub, "get") as get:
                with self.assertRaisesRegex(promotion.PromotionError, "five-minute"):
                    api.get("branches/main")
                get.assert_not_called()

    def test_failure_receipt_redacts_tokens(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/"failure.json"
            promotion.failure(path, ValueError(ENV["GH_TOKEN"]+" "+ENV["SOROTTE_PROTECTION_TOKEN"]), ENV)
            text = path.read_text()
            self.assertNotIn(ENV["GH_TOKEN"], text)
            self.assertNotIn(ENV["SOROTTE_PROTECTION_TOKEN"], text)
            self.assertIn("<redacted>", text)


if __name__ == "__main__":
    unittest.main()
