from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_assets as assets
import release_qualification as qualification
from scripts.tests.test_playback_release_gate import materialize_bundle, SHA, MODEL_PATH
from scripts import verify_server_container as container


class QualificationReceiptTests(unittest.TestCase):
    def complete(self, root: Path, manifest: dict) -> Path:
        path = root.parent / "complete.json"
        path.write_text(json.dumps({"kind": "sorotte-playback-release-complete-gate", "result": "passed",
            "candidate_sha": SHA, "candidate_manifest_sha256": {"linux-x86_64": qualification.artifact_input.sha256_file(root / "candidate-manifest.json"), "windows-x86_64": "b" * 64},
            "model_sha256": qualification.artifact_input.sha256_file(MODEL_PATH),
            "required_system_transitions": ["transition"], "system_transition_coverage": ["transition"]}), encoding="utf-8")
        return path

    def test_bundle_consumer_requires_source_channel_full_receipt_and_actual_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "bundle"
            root.mkdir()
            manifest = materialize_bundle(root, "linux-x86_64")
            complete = self.complete(root, manifest)
            repo = MODEL_PATH.parents[1]
            with mock.patch.object(qualification, "clean_source", return_value=manifest["build_inputs"]["source_files"]):
                qualification.consume(root, complete, repo, SHA, "linux-x86_64", "stable", None)
                with self.assertRaisesRegex(qualification.QualificationError, "channel"):
                    qualification.consume(root, complete, repo, SHA, "linux-x86_64", "dev", None)
                broken = json.loads(complete.read_text())
                del broken["candidate_manifest_sha256"]["windows-x86_64"]
                complete.write_text(json.dumps(broken))
                with self.assertRaisesRegex(qualification.QualificationError, "platform"):
                    qualification.consume(root, complete, repo, SHA, "linux-x86_64", "stable", None)
                self.complete(root, manifest)
                binary = root / manifest["files"]["server"]["file_name"]
                binary.write_bytes(b"another build from the same SHA")
                with self.assertRaisesRegex(ValueError, "differs"):
                    qualification.consume(root, complete, repo, SHA, "linux-x86_64", "stable", None)

    def test_source_changed_or_foreign_producer_cannot_reuse(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "bundle"
            root.mkdir()
            manifest = materialize_bundle(root, "linux-x86_64")
            complete = self.complete(root, manifest)
            with mock.patch.object(qualification, "clean_source", return_value={"Cargo.lock": "changed"}):
                with self.assertRaisesRegex(qualification.QualificationError, "inputs differ"):
                    qualification.consume(root, complete, MODEL_PATH.parents[1], SHA, "linux-x86_64", "stable", None)
            with mock.patch.object(qualification, "clean_source", return_value=manifest["build_inputs"]["source_files"]):
                with self.assertRaisesRegex(qualification.QualificationError, "producer"):
                    qualification.consume(root, complete, MODEL_PATH.parents[1], SHA, "linux-x86_64", "stable", "999")

    def test_workspace_receipt_is_default_platform_profile_and_run_specific(self) -> None:
        source = {"Cargo.lock": "a" * 64}
        value = {"schema_version": 1, "kind": "sorotte-release-workspace-receipt", "result": "passed", "candidate_sha": SHA,
            "platform": "linux-x86_64", "features": "default", "profile": "test", "instrumentation": "none",
            "command": ["cargo", "test", "--locked", "--workspace"], "source_files": source, "rustc": "compiler",
            "producer": {"run_id": "12", "repository": "owner/repo"}}
        with mock.patch.object(qualification, "clean_source", return_value=source), mock.patch.object(qualification, "run", return_value="compiler"), mock.patch.dict(os.environ, {"GITHUB_REPOSITORY": "owner/repo"}):
            qualification.validate_workspace(value, Path.cwd(), SHA, "linux-x86_64", "12")
            for key, wrong in (("features", "all"), ("platform", "windows-x86_64"), ("candidate_sha", "f" * 40), ("profile", "release"), ("instrumentation", "coverage"), ("result", "cancelled")):
                with self.subTest(key=key), self.assertRaises(qualification.QualificationError):
                    qualification.validate_workspace({**value, key: wrong}, Path.cwd(), SHA, "linux-x86_64", "12")
            with self.assertRaisesRegex(qualification.QualificationError, "provenance"):
                qualification.validate_workspace(value, Path.cwd(), SHA, "linux-x86_64", "13")
            for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS"):
                with self.subTest(key=key), mock.patch.dict(os.environ, {key: "-C instrument-coverage"}), self.assertRaisesRegex(qualification.QualificationError, "instrumented environment"):
                    qualification.validate_workspace(value, Path.cwd(), SHA, "linux-x86_64", "12")

    def test_workspace_writer_runs_tests_and_never_receipts_failure(self) -> None:
        with mock.patch.object(qualification, "clean_source", return_value={"Cargo.lock": "a" * 64}), mock.patch.object(qualification.subprocess, "run", side_effect=subprocess.CalledProcessError(1, ["cargo"])) as runner:
            with self.assertRaises(subprocess.CalledProcessError):
                qualification.workspace_receipt(Path.cwd(), SHA, "linux-x86_64", "default")
            self.assertEqual(runner.call_args.args[0], ["cargo", "test", "--locked", "--workspace"])

    def test_wrong_or_dirty_legacy_checkout_fails_before_behavior(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "syncplayServer.py").write_text("fixture")
            with mock.patch.object(qualification, "run", return_value="f" * 40), self.assertRaisesRegex(qualification.QualificationError, "pinned"):
                qualification.verify_legacy(root)
            with mock.patch.object(qualification, "run", side_effect=[qualification.LEGACY_SHA, " M syncplayServer.py"]), self.assertRaisesRegex(qualification.QualificationError, "clean"):
                qualification.verify_legacy(root)

    def test_archive_requires_actual_runtime_and_exact_bundle_binary(self) -> None:
        manifest = {"candidate_sha": SHA, "files": {"server": {"file_name": "sorotte-server", "sha256": "a" * 64}}}
        report = {"status": "verified", "package": {"sourceSha": SHA, "name": "sorotte-server", "files": [{"path": "sorotte-server", "sha256": "a" * 64}]}, "runtimeSmoke": {"performed": True}}
        qualification.validate_package(report, manifest)
        wrong = copy.deepcopy(report)
        wrong["package"]["files"][0]["sha256"] = "b" * 64
        with self.assertRaisesRegex(qualification.QualificationError, "different binary"):
            qualification.validate_package(wrong, manifest)
        report["runtimeSmoke"]["performed"] = False
        with self.assertRaisesRegex(qualification.QualificationError, "runtime"):
            qualification.validate_package(report, manifest)

    def test_explicit_producer_requires_complete_trusted_tag_run(self) -> None:
        value = {"id": 12, "head_sha": SHA, "repository": {"full_name": "owner/repo"}, "head_repository": {"full_name": "owner/repo"}, "event": "push", "head_branch": "v0.2.9", "path": ".github/workflows/stable-release.yml", "status": "completed", "conclusion": "success", "run_attempt": 2}
        self.assertEqual(qualification.validate_producer_run(value, SHA, "owner/repo", "12", "v0.2.9"), 2)
        for key, wrong in (("head_sha", "b" * 40), ("event", "pull_request"), ("head_branch", "main"), ("conclusion", "cancelled"), ("path", ".github/workflows/untrusted.yml"), ("run_attempt", True)):
            with self.subTest(key=key), self.assertRaises(qualification.QualificationError):
                qualification.validate_producer_run({**value, key: wrong}, SHA, "owner/repo", "12", "v0.2.9")


class PublicAssetTests(unittest.TestCase):
    def test_anonymous_public_comparison_fails_changed_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "server.zip").write_bytes(b"approved")
            expected = {"size": 8, "sha256": hashlib.sha256(b"approved").hexdigest()}
            with mock.patch.object(assets, "public_digest", return_value=expected):
                self.assertEqual(assets.verify_public("owner/repo", "v0.2.9", root)["result"], "passed")
            with mock.patch.object(assets, "public_digest", return_value={"size": 8, "sha256": "b" * 64}), self.assertRaisesRegex(assets.AssetError, "differs"):
                assets.verify_public("owner/repo", "v0.2.9", root)

    def test_attachment_refuses_to_replace_existing_different_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "server.zip").write_bytes(b"approved")
            result = subprocess.CompletedProcess([], 0, json.dumps({"assets": [{"name": "server.zip"}]}))
            with mock.patch.object(assets.subprocess, "run", return_value=result) as runner, mock.patch.object(assets, "public_digest", return_value={"size": 8, "sha256": "b" * 64}), self.assertRaisesRegex(assets.AssetError, "different bytes"):
                assets.attach("owner/repo", "v0.2.9", root)
            self.assertEqual(runner.call_count, 1)

    def test_durable_receipts_are_complete_and_byte_stable_across_download_times(self) -> None:
        kinds = ["release-authorization", "sorotte-playback-release-candidate-bundle", "sorotte-playback-release-platform-gate", "sorotte-playback-release-complete-gate", "sorotte-release-workspace-receipt"]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "evidence"
            evidence.mkdir()
            for index, kind in enumerate(kinds):
                if kind in {"release-authorization", "sorotte-playback-release-complete-gate"}:
                    (evidence / f"{index}.json").write_text(json.dumps({"kind": kind, "candidate_sha": SHA, "result": "passed"}))
                else:
                    for platform in qualification.PLATFORMS:
                        (evidence / f"{index}-{platform}.json").write_text(json.dumps({"kind": kind, "candidate_sha": SHA, "result": "passed", "platform": platform}))
            qualification.archive_evidence(evidence, root / "first", SHA)
            for path in evidence.iterdir():
                os.utime(path, (1_700_000_000, 1_700_000_000))
            qualification.archive_evidence(evidence, root / "second", SHA)
            name = f"sorotte-qualification-{SHA}.zip"
            self.assertEqual((root / "first" / name).read_bytes(), (root / "second" / name).read_bytes())
            with zipfile.ZipFile(root / "first" / name) as archive:
                self.assertIn("receipt-index.json", archive.namelist())
            (evidence / "0.json").unlink()
            with self.assertRaisesRegex(qualification.QualificationError, "missing an authority"):
                qualification.archive_evidence(evidence, root / "incomplete", SHA)
            (evidence / "private.log").write_text("do not publish")
            with self.assertRaisesRegex(qualification.QualificationError, "structured JSON"):
                qualification.archive_evidence(evidence, root / "unsafe", SHA)


class PackageWorkflowTests(unittest.TestCase):
    def test_protection_reader_is_scoped_to_authority_steps_and_not_candidate_jobs(self) -> None:
        import yaml

        root = Path(__file__).resolve().parents[2]
        action = "actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349"
        files = ("stable-release.yml", "sorotte-server-release.yml", "sorotte-gui-release.yml", "publish-server-container.yml")
        authorizations = 0
        for name in files:
            workflow = yaml.load((root / ".github/workflows" / name).read_text(), Loader=yaml.BaseLoader)
            for job in workflow["jobs"].values():
                steps = job.get("steps", [])
                for index, step in enumerate(steps):
                    if "merge_gate.py authorize-release" not in step.get("run", ""):
                        continue
                    authorizations += 1
                    token = steps[index - 1]
                    self.assertEqual(token["uses"], action)
                    self.assertEqual(token["with"], {
                        "app-id": "${{ vars.SOROTTE_PROTECTION_APP_ID }}",
                        "private-key": "${{ secrets.SOROTTE_PROTECTION_APP_PRIVATE_KEY }}",
                        "owner": "${{ github.repository_owner }}",
                        "repositories": "${{ github.event.repository.name }}",
                        "permission-administration": "read",
                    })
                    self.assertEqual(step["env"]["GH_TOKEN"], "${{ github.token }}")
                    self.assertEqual(step["env"]["SOROTTE_PROTECTION_TOKEN"], "${{ steps.protection-token.outputs.token }}")
                    self.assertNotIn("--wait-seconds", step["run"])
                    self.assertEqual(token.get("if"), step.get("if"))
            if name != "stable-release.yml":
                self.assertEqual(workflow["on"]["workflow_call"]["secrets"], {"SOROTTE_PROTECTION_APP_PRIVATE_KEY": {"required": "true"}})
            else:
                consumers = {name for name, job in workflow["jobs"].items() if "secrets" in job}
                self.assertEqual(consumers, {"server-archives", "gui-archive", "container"})
                for consumer in consumers:
                    self.assertEqual(workflow["jobs"][consumer]["secrets"], {"SOROTTE_PROTECTION_APP_PRIVATE_KEY": "${{ secrets.SOROTTE_PROTECTION_APP_PRIVATE_KEY }}"})
        self.assertEqual(authorizations, 7)
        for name in ("package-ci.yml", "playback-lifecycle-release-gate.yml", "gui-native-interactive.yml"):
            text = (root / ".github/workflows" / name).read_text()
            self.assertNotIn("SOROTTE_PROTECTION_APP_PRIVATE_KEY", text)
            self.assertNotIn("create-github-app-token", text)
        workflow = yaml.load((root / ".github/workflows/sorotte-gui-release.yml").read_text(), Loader=yaml.BaseLoader)
        steps = workflow["jobs"]["authorize-source"]["steps"]
        wait = next(index for index, step in enumerate(steps) if "merge_gate.py wait-checks" in step.get("run", ""))
        self.assertEqual(steps[wait + 1]["uses"], action)
        self.assertEqual(steps[wait]["env"], {"GH_TOKEN": "${{ github.token }}"})
        manifest = json.loads((root / "docs/protection-app-manifest.json").read_text())
        self.assertEqual(manifest["default_permissions"], {"administration": "read", "metadata": "read"})
        self.assertFalse(manifest["public"])
        self.assertEqual(manifest["default_events"], [])

    def test_package_required_is_independent_of_publication_and_includes_all_archives(self) -> None:
        import yaml

        root = Path(__file__).resolve().parents[2]
        workflow = yaml.load((root / ".github/workflows/package-ci.yml").read_text(), Loader=yaml.BaseLoader)
        self.assertIn("pull_request", workflow["on"])
        self.assertEqual(workflow["on"]["push"]["branches"], ["main"])
        jobs = workflow["jobs"]
        self.assertEqual(jobs["package-required"]["if"], "always()")
        self.assertEqual(set(jobs["package-required"]["needs"]), {"preflight", "archive"})
        self.assertEqual({(row["package"], row["runner"]) for row in jobs["archive"]["strategy"]["matrix"]["include"]}, {("gui", "windows-2025"), ("server", "windows-2025"), ("server", "ubuntu-24.04")})
        commands = "\n".join(step.get("run", "") for job in jobs.values() for step in job.get("steps", []))
        self.assertNotIn("authorize-release", commands)
        self.assertNotIn("--skip-runtime-smoke", commands)
        self.assertIn("verify_gui_release_artifact.py", commands)
        self.assertIn("verify_server_release_artifact.py", commands)
        self.assertIn("updater_self_replacement_windows", commands)
        aggregate = "\n".join(step.get("run", "") for step in jobs["package-required"]["steps"])
        self.assertIn('test "$PREFLIGHT_RESULT" = success', aggregate)
        self.assertIn('verify.py gate --lane release --selected "$SELECTED"', aggregate)
        self.assertIn('--expected-job archive --job-result "archive=$ARCHIVE_RESULT"', aggregate)


class ContainerPromotionTests(unittest.TestCase):
    def run_promotion(self, root: Path, expected: str = "sha256:" + "a" * 64):
        return container.promote_approved_digest(evidence_dir=root, expected_digest=expected,
            expected_source_sha=SHA, expected_source_url="https://github.com/owner/repo", version_tag="v0.2.9", output_dir=root / "promotion")

    def fixtures(self):
        image = "ghcr.io/owner/sorotte-server"
        digest = "sha256:" + "a" * 64
        published = {"image": image, "source": "https://github.com/owner/repo", "sourceSha": SHA, "digest": digest,
            "tags": [f"{image}:v0.2.9", f"{image}:sha-{SHA}"], "pushes": []}
        public = {"verificationPolicy": {"certificateIdentity": "https://github.com/owner/repo/.github/workflows/publish-server-container.yml@refs/tags/v0.2.9", "workflowSourceSha": SHA}}
        final = {"registryManifestDigest": digest, "sourceSha": SHA}
        return published, public, final

    def test_promotion_copies_only_approved_digest_with_fresh_verification(self) -> None:
        published, public, final = self.fixtures()
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(container, "enforce_final_gate", return_value=final), mock.patch.object(container, "parse_publish_report", return_value=published), mock.patch.object(container, "parse_publication_report", return_value=public), mock.patch.object(container, "verify_publication", return_value={"status": "passed"}) as verify, mock.patch.object(container, "_run", return_value=subprocess.CompletedProcess([], 0, "[]")) as runner:
            self.run_promotion(Path(temporary))
            commands = [call.args[0] for call in runner.call_args_list]
            docker = [cmd for cmd in commands if cmd[0] == "docker"]
            self.assertEqual(docker, [["docker", "buildx", "imagetools", "create", "--prefer-index=false", "--tag", "ghcr.io/owner/sorotte-server:latest", "ghcr.io/owner/sorotte-server@" + final["registryManifestDigest"]]])
            self.assertEqual([cmd[1] for cmd in commands if cmd[0] == "cosign"], ["verify", "verify-attestation"])
            for cmd in commands:
                if cmd[0] == "cosign":
                    self.assertEqual(cmd[cmd.index("--certificate-identity") + 1], public["verificationPolicy"]["certificateIdentity"])
            self.assertEqual(verify.call_count, 2)

    def test_top_level_workflow_is_not_the_reusable_signer_identity(self) -> None:
        published, public, final = self.fixtures()
        public["verificationPolicy"]["certificateIdentity"] = "https://github.com/owner/repo/.github/workflows/stable-release.yml@refs/tags/v0.2.9"
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(container, "enforce_final_gate", return_value=final), mock.patch.object(container, "parse_publish_report", return_value=published), mock.patch.object(container, "parse_publication_report", return_value=public), mock.patch.object(container, "_run") as runner:
            with self.assertRaisesRegex(container.VerificationError, "signed by the trusted"):
                self.run_promotion(Path(temporary))
            runner.assert_not_called()

    def test_wrong_digest_or_failed_fresh_public_checks_never_promotes(self) -> None:
        published, public, final = self.fixtures()
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(container, "enforce_final_gate", return_value=final), mock.patch.object(container, "parse_publish_report", return_value=published), mock.patch.object(container, "parse_publication_report", return_value=public), mock.patch.object(container, "_run", return_value=subprocess.CompletedProcess([], 0, "[]")) as runner, mock.patch.object(container, "verify_publication", side_effect=container.VerificationError("public identity differs")):
            with self.assertRaisesRegex(container.VerificationError, "requested digest"):
                self.run_promotion(Path(temporary), "sha256:" + "f" * 64)
            runner.assert_not_called()
            with self.assertRaisesRegex(container.VerificationError, "public identity"):
                self.run_promotion(Path(temporary))
            self.assertFalse(any(call.args[0][0] == "docker" for call in runner.call_args_list))


if __name__ == "__main__":
    unittest.main()
