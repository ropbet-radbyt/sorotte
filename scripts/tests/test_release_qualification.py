from __future__ import annotations

import copy
from contextlib import redirect_stderr, redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile
from unittest import mock

from scripts.verification_tools import pins as verification_pins

VERIFICATION_PINS = verification_pins()

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_assets as assets
import release_qualification as qualification
from scripts.tests.test_playback_release_gate import materialize_bundle, SHA, MODEL_PATH
from scripts import verify_server_container as container


class CheckoutSourceIdentityTests(unittest.TestCase):
    CORPUS_DIRECTORIES = (
        "crates/sorotte-protocol/tests/corpus/protocol_parser",
        "crates/sorotte-cli/tests/corpus/framed_session",
        "crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript",
    )
    OLD_ATTRIBUTES = (
        "*.rs text eol=lf\n"
        + "".join(f"{directory}/** -text\n" for directory in CORPUS_DIRECTORIES)
    ).encode()

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        empty_config = self.root / "empty-config"
        empty_config.write_bytes(b"")
        empty_hooks = self.root / "empty-hooks"
        empty_hooks.mkdir()
        # These local Git fixtures must not inherit user hooks, attributes,
        # signing requirements, or a parent worktree/index from the caller.
        environment = {key: value for key, value in os.environ.items()
                       if not key.upper().startswith("GIT_")}
        environment.update({
            "GIT_CONFIG_GLOBAL": str(empty_config), "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_ATTR_NOSYSTEM": "1", "GIT_CONFIG_COUNT": "3",
            "GIT_CONFIG_KEY_0": "core.attributesFile", "GIT_CONFIG_VALUE_0": str(empty_config),
            "GIT_CONFIG_KEY_1": "core.hooksPath", "GIT_CONFIG_VALUE_1": str(empty_hooks),
            "GIT_CONFIG_KEY_2": "commit.gpgSign", "GIT_CONFIG_VALUE_2": "false",
        })
        environment_patch = mock.patch.dict(os.environ, environment, clear=True)
        environment_patch.start()
        self.addCleanup(environment_patch.stop)

    def git(self, root: Path, *arguments: str) -> str:
        return subprocess.run(
            ["git", "-c", f"safe.directory={root.as_posix()}", *arguments],
            cwd=root, check=True, capture_output=True, text=True, timeout=15,
        ).stdout.strip()

    def checkout_fixture(self, attributes: bytes) -> tuple[str, dict[str, bytes], dict[str, Path]]:
        source = self.root / "source"
        source.mkdir()
        self.git(source, "init", "--quiet", "--template", str(self.root / "empty-hooks"))
        self.git(source, "config", "core.autocrlf", "false")
        self.git(source, "config", "core.eol", "lf")
        payloads = {
            ".gitattributes": attributes,
            "Cargo.toml": b'[workspace.package]\nversion = "0.2.10"\n',
            "Cargo.lock": b"# Fixture lockfile\nversion = 4\n",
            "src/lib.rs": b"pub fn fixture() -> bool {\n    true\n}\n",
            "scripts/preparation.py": b'print("fixture build input")\n',
            "scripts/prepare.ps1": b'Write-Output "fixture build input"\n',
            ".github/workflows/build.yml": b"name: fixture\non: workflow_dispatch\n",
            "coverage/policy.json": b'{"fixture": true}\n',
            "docs/release.md": b"Fixture release instructions\n",
            "coverage/playback-lifecycle.toml": MODEL_PATH.read_bytes().replace(b"\r\n", b"\n"),
            "images/fixture.ico": b"\x00\x00\x01\x00\xff\r\nicon\n",
            "images/fixture.png": b"\x89PNG\r\n\x1a\n\x00\xff\r\npayload\n",
        }
        for directory in self.CORPUS_DIRECTORIES:
            # No NUL in the framing seed: Git's binary heuristic alone cannot
            # protect the protocol's intentional CRLF delimiters.
            payloads[f"{directory}/framing-seed"] = b'{"frame":1}\r\n{"frame":2}\r\n'
            payloads[f"{directory}/binary-seed"] = b"\x00\xff\r\n\x80\n\r"
        for name, body in payloads.items():
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(body)
        self.git(source, "add", "--all")
        self.git(source, "-c", "user.name=Checkout fixture", "-c",
                 "user.email=checkout-fixture@example.invalid", "commit", "--quiet", "-m", "Fixture")
        sha = self.git(source, "rev-parse", "HEAD")
        checkouts = {}
        for name, autocrlf, eol in (("lf", "false", "lf"), ("autocrlf", "true", "lf"),
                                    ("crlf", "false", "crlf")):
            checkout = self.root / name
            self.git(self.root, "clone", "--quiet", "--local", "--no-hardlinks", "--no-checkout",
                     str(source), str(checkout))
            self.git(checkout, "config", "core.autocrlf", autocrlf)
            self.git(checkout, "config", "core.eol", eol)
            self.git(checkout, "checkout", "--quiet", "--detach", sha)
            self.assertEqual(self.git(checkout, "rev-parse", "HEAD"), sha)
            self.assertEqual(self.git(checkout, "status", "--porcelain", "--untracked-files=all"), "")
            self.assertEqual(self.git(checkout, "config", "core.autocrlf"), autocrlf)
            self.assertEqual(self.git(checkout, "config", "core.eol"), eol)
            checkouts[name] = checkout
        return sha, payloads, checkouts

    def source_bound_bundle(self, producer: Path, sha: str) -> tuple[Path, Path, dict]:
        bundle = self.root / "bundle"
        bundle.mkdir()
        manifest = materialize_bundle(bundle, "windows-x86_64")
        manifest["candidate_sha"] = sha
        manifest["product_version"] = "0.2.10"
        manifest["build_inputs"]["candidate_sha"] = sha
        manifest["build_inputs"]["source_files"] = qualification.clean_source(producer, sha)
        manifest["build_inputs"]["producer"]["workflow_sha"] = sha
        manifest["build_inputs"]["source_ref"] = "refs/tags/v0.2.10"
        manifest_path = bundle / "candidate-manifest.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        complete = self.root / "complete.json"
        complete.write_text(json.dumps({
            "kind": "sorotte-playback-release-complete-gate", "result": "passed",
            "candidate_sha": sha,
            "candidate_manifest_sha256": {
                "windows-x86_64": qualification.artifact_input.sha256_file(manifest_path),
                "linux-x86_64": "b" * 64,
            },
            "model_sha256": qualification.artifact_input.sha256_file(
                producer / "coverage/playback-lifecycle.toml"),
            "required_system_transitions": ["fixture-transition"],
            "system_transition_coverage": ["fixture-transition"],
        }), encoding="utf-8")
        return bundle, complete, manifest

    def test_same_clean_sha_under_rust_only_policy_does_not_authorize_different_input_bytes(self) -> None:
        sha, payloads, checkouts = self.checkout_fixture(self.OLD_ATTRIBUTES)
        bundle, complete, manifest = self.source_bound_bundle(checkouts["lf"], sha)
        consumer = checkouts["autocrlf"]
        actual = qualification.clean_source(consumer, sha)
        self.assertNotEqual(actual, manifest["build_inputs"]["source_files"])
        for path in ("Cargo.toml", "scripts/preparation.py"):
            self.assertEqual((consumer / path).read_bytes(), payloads[path].replace(b"\n", b"\r\n"))
        self.assertEqual((consumer / "src/lib.rs").read_bytes(), payloads["src/lib.rs"])
        with self.assertRaisesRegex(qualification.QualificationError,
                                    "qualified build source inputs differ from consumer checkout") as error:
            qualification.consume(bundle, complete, consumer, sha, "windows-x86_64", "stable", None)
        self.assertIn("'Cargo.toml'", str(error.exception))

    def test_repository_policy_preserves_exact_inputs_and_allows_cross_config_bundle_consumption(self) -> None:
        attributes = (MODEL_PATH.parents[1] / ".gitattributes").read_bytes()
        sha, payloads, checkouts = self.checkout_fixture(attributes)
        bundle, complete, manifest = self.source_bound_bundle(checkouts["lf"], sha)
        expected = {name: hashlib.sha256(body).hexdigest() for name, body in payloads.items()}
        for name, consumer in checkouts.items():
            with self.subTest(checkout=name):
                self.assertEqual(qualification.clean_source(consumer, sha), expected)
                for path, body in payloads.items():
                    self.assertEqual((consumer / path).read_bytes(), body, path)
                self.assertEqual(
                    qualification.consume(bundle, complete, consumer, sha, "windows-x86_64", "stable", None),
                    manifest,
                )


    def test_passing_workspace_command_cannot_certify_test_written_inputs(self) -> None:
        attributes = (MODEL_PATH.parents[1] / ".gitattributes").read_bytes()
        sha, _, checkouts = self.checkout_fixture(attributes)
        checkout = checkouts["lf"]
        qualification.clean_source(checkout, sha)
        original_run = subprocess.run
        cargo_commands = []
        private_contents = "PRIVATE_TEST_OUTPUT_MUST_NOT_APPEAR_IN_DIAGNOSTICS"

        def test_process(command, **kwargs):
            if command[0] != "cargo":
                return original_run(command, **kwargs)
            cargo_commands.append(command)
            self.assertEqual(kwargs["cwd"], checkout)
            generated = checkout / "test-cache" / ".media-index-activation.lock"
            generated.parent.mkdir()
            generated.write_text(private_contents, encoding="utf-8")
            (checkout / "src/lib.rs").write_text(private_contents, encoding="utf-8")
            return subprocess.CompletedProcess(command, 0)

        with mock.patch.object(qualification.subprocess, "run", side_effect=test_process):
            with self.assertRaisesRegex(qualification.QualificationError, "release source must be clean") as error:
                qualification.workspace_receipt(checkout, sha, "linux-x86_64", "default")
        self.assertEqual(cargo_commands, [["cargo", "test", "--locked", "--workspace"]])
        diagnostic = str(error.exception)
        self.assertIn("2 status entries", diagnostic)
        self.assertIn("src/lib.rs", diagnostic)
        self.assertIn("test-cache/.media-index-activation.lock", diagnostic)
        self.assertNotIn(private_contents, diagnostic)

    def test_dirty_source_diagnostic_bounds_the_path_inventory(self) -> None:
        attributes = (MODEL_PATH.parents[1] / ".gitattributes").read_bytes()
        sha, _, checkouts = self.checkout_fixture(attributes)
        checkout = checkouts["lf"]
        for index in range(12):
            (checkout / f"generated-{index:02}.txt").write_text("unreported contents", encoding="utf-8")
        with self.assertRaisesRegex(qualification.QualificationError, "12 status entries; first 10") as error:
            qualification.clean_source(checkout, sha)
        diagnostic = str(error.exception)
        self.assertIn("generated-00.txt", diagnostic)
        self.assertIn("generated-09.txt", diagnostic)
        self.assertNotIn("generated-10.txt", diagnostic)
        self.assertNotIn("generated-11.txt", diagnostic)
        self.assertNotIn("unreported contents", diagnostic)


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


class ContainerProducerTests(unittest.TestCase):
    def fixture(self, attempt: int = 2):
        api = "https://api.github.com/repos/owner/repo"
        run = {"id": 12, "head_sha": SHA, "repository": {"full_name": "owner/repo"},
            "head_repository": {"full_name": "owner/repo"}, "event": "push", "head_branch": "v0.2.10",
            "path": ".github/workflows/stable-release.yml", "status": "completed", "conclusion": "success", "run_attempt": attempt}
        job = {"id": 101, "run_id": 12, "run_attempt": 1, "head_sha": SHA, "head_branch": "v0.2.10",
            "run_url": f"{api}/actions/runs/12", "url": f"{api}/actions/jobs/101",
            "name": "container / publish", "status": "completed", "conclusion": "success",
            "started_at": "2026-09-06T01:00:00Z", "completed_at": "2026-09-06T01:30:00Z",
            "steps": [{"name": name, "number": index, "status": "completed", "conclusion": "success"}
                for index, name in enumerate(qualification.CONTAINER_PUBLICATION_STEPS, 10)]}
        artifact = {"id": 901, "name": "server-container-verification-12-1", "expired": False,
            "workflow_run": {"id": 12, "head_sha": SHA}, "url": f"{api}/actions/artifacts/901",
            "digest": "sha256:" + "a" * 64, "created_at": "2026-09-06T01:29:59Z"}
        return run, [job], [artifact]

    def select(self, run, jobs, artifacts):
        return qualification.select_container_producer(run, jobs, artifacts, SHA, "owner/repo", "12", "v0.2.10")

    def later_job(self, job, conclusion="success"):
        return {**copy.deepcopy(job), "id": 102, "run_attempt": 2, "conclusion": conclusion,
            "url": "https://api.github.com/repos/owner/repo/actions/jobs/102"}

    def test_attachment_only_retry_selects_original_container_artifact_and_attempt(self):
        run, jobs, artifacts = self.fixture()
        jobs.append({"id": 201, "name": "retain-release-qualification", "run_attempt": 2,
            "status": "completed", "conclusion": "success"})
        selected = self.select(run, jobs, artifacts)
        self.assertEqual((selected["publication_final_attempt"], selected["container_attempt"],
                          selected["container_job_id"], selected["artifact_id"]), (2, 1, 101, 901))
        self.assertEqual(selected["artifact_name"], "server-container-verification-12-1")

    def test_legacy_published_producer_keeps_its_original_runtime_obligations(self):
        run, jobs, artifacts = self.fixture()
        jobs[0]["steps"] = [{"name": name, "number": index, "status": "completed", "conclusion": "success"}
                            for index, name in enumerate(qualification.LEGACY_CONTAINER_PUBLICATION_STEPS, 10)]
        self.assertEqual(self.select(run, jobs, artifacts)["container_attempt"], 1)
        for name in qualification.LEGACY_CONTAINER_PUBLICATION_STEPS[:3]:
            broken = copy.deepcopy(jobs)
            next(step for step in broken[0]["steps"] if step["name"] == name)["conclusion"] = "skipped"
            with self.subTest(name=name), self.assertRaises(qualification.QualificationError):
                self.select(run, broken, artifacts)

    def test_later_successful_container_selects_its_own_artifact(self):
        run, jobs, artifacts = self.fixture()
        jobs.append(self.later_job(jobs[0]))
        artifacts.append({**artifacts[0], "id": 902, "name": "server-container-verification-12-2",
            "url": "https://api.github.com/repos/owner/repo/actions/artifacts/902"})
        selected = self.select(run, list(reversed(jobs)), artifacts)
        self.assertEqual((selected["container_attempt"], selected["container_job_id"], selected["artifact_id"]), (2, 102, 902))

    def test_newer_failed_skipped_cancelled_or_running_container_never_falls_back(self):
        for status, conclusion in (("completed", "failure"), ("completed", "skipped"),
                                   ("completed", "cancelled"), ("in_progress", None)):
            with self.subTest(status=status, conclusion=conclusion):
                run, jobs, artifacts = self.fixture()
                jobs.append({**self.later_job(jobs[0], conclusion), "status": status})
                with self.assertRaisesRegex(qualification.QualificationError, "latest actual"):
                    self.select(run, jobs, artifacts)

    def test_foreign_or_malformed_newer_container_is_rejected_before_selection(self):
        for key, wrong in (("run_id", 13), ("head_sha", "f" * 40), ("head_branch", "main"),
                           ("url", "https://api.github.com/repos/foreign/repo/actions/jobs/102"),
                           ("run_url", "https://api.github.com/repos/foreign/repo/actions/runs/12"),
                           ("run_attempt", 0), ("run_attempt", 3), ("run_attempt", True), ("run_attempt", None), ("id", True)):
            with self.subTest(key=key, wrong=wrong):
                run, jobs, artifacts = self.fixture()
                jobs.append({**self.later_job(jobs[0]), key: wrong})
                with self.assertRaisesRegex(qualification.QualificationError, "foreign, malformed or ambiguous"):
                    self.select(run, jobs, artifacts)

    def test_missing_duplicate_or_same_attempt_container_is_not_authority(self):
        for variant in ("missing", "same-id", "same-attempt"):
            with self.subTest(variant=variant):
                run, jobs, artifacts = self.fixture()
                if variant == "missing":
                    jobs[0]["name"] = "unrelated / publish"
                else:
                    jobs.append(copy.deepcopy(jobs[0]) if variant == "same-id" else {**self.later_job(jobs[0]), "run_attempt": 1})
                with self.assertRaises(qualification.QualificationError):
                    self.select(run, jobs, artifacts)

    def test_each_required_container_phase_must_have_executed_successfully_once(self):
        for name in qualification.CONTAINER_PUBLICATION_STEPS:
            for defect in ("missing", "failed", "skipped", "duplicate"):
                with self.subTest(name=name, defect=defect):
                    run, jobs, artifacts = self.fixture()
                    steps = jobs[0]["steps"]
                    step = next(item for item in steps if item["name"] == name)
                    if defect == "missing": steps.remove(step)
                    elif defect == "duplicate": steps.append(copy.deepcopy(step))
                    else: step["conclusion"] = defect
                    with self.assertRaisesRegex(qualification.QualificationError, "publication step"):
                        self.select(run, jobs, artifacts)

    def test_artifact_must_belong_to_exact_successful_job_window_and_source(self):
        for defect in ("missing", "duplicate", "expired", "source", "run", "repository", "digest", "attempt-name", "early", "late", "naive-time"):
            with self.subTest(defect=defect):
                run, jobs, artifacts = self.fixture()
                artifact = artifacts[0]
                if defect == "missing": artifacts.clear()
                if defect == "duplicate": artifacts.append(copy.deepcopy(artifact))
                if defect == "expired": artifact["expired"] = True
                if defect == "source": artifact["workflow_run"]["head_sha"] = "f" * 40
                if defect == "run": artifact["workflow_run"]["id"] = 13
                if defect == "repository": artifact["url"] = "https://api.github.com/repos/foreign/repo/actions/artifacts/901"
                if defect == "digest": artifact["digest"] = "absent"
                if defect == "attempt-name": artifact["name"] = "server-container-verification-12-2"
                if defect == "early": artifact["created_at"] = "2026-09-06T00:59:59Z"
                if defect == "late": artifact["created_at"] = "2026-09-06T01:30:01Z"
                if defect == "naive-time": artifact["created_at"] = "2026-09-06T01:20:00"
                with self.assertRaises(qualification.QualificationError):
                    self.select(run, jobs, artifacts)

    def test_parent_run_authority_cannot_be_replaced_by_matching_job_metadata(self):
        for key, wrong in (("repository", {"full_name": "foreign/repo"}), ("head_sha", "f" * 40),
                           ("head_repository", {"full_name": "foreign/repo"}), ("event", "workflow_dispatch"),
                           ("path", ".github/workflows/other.yml"), ("head_branch", "main"), ("conclusion", "failure")):
            with self.subTest(key=key):
                run, jobs, artifacts = self.fixture()
                run[key] = wrong
                with self.assertRaisesRegex(qualification.QualificationError, "trusted tag run"):
                    self.select(run, jobs, artifacts)

    def test_paginated_job_history_includes_newer_failure_on_second_page(self):
        run, jobs, artifacts = self.fixture()
        first = jobs + [{"id": index, "name": "other"} for index in range(200, 299)]
        second = [self.later_job(jobs[0], "failure")]
        get = mock.Mock(side_effect=[{"total_count": 101, "jobs": first}, {"total_count": 101, "jobs": second}])
        history = qualification.producer_collection(get, "actions/runs/12/jobs?filter=all", "jobs")
        self.assertEqual(get.call_args_list, [mock.call("actions/runs/12/jobs?filter=all&per_page=100&page=1"),
            mock.call("actions/runs/12/jobs?filter=all&per_page=100&page=2")])
        self.assertEqual(len(history), 101)
        with self.assertRaisesRegex(qualification.QualificationError, "latest actual"):
            self.select(run, history, artifacts)

    def test_pagination_rejects_truncation_duplicate_ids_and_changing_totals(self):
        first = [{"id": index} for index in range(1, 101)]
        for payloads in ([{"total_count": 101, "jobs": first[:1]}],
                         [{"total_count": 101, "jobs": first}, {"total_count": 101, "jobs": [{"id": 1}]}],
                         [{"total_count": 101, "jobs": first}, {"total_count": 102, "jobs": [{"id": 101}]}],
                         [{"total_count": 2001, "jobs": first}]):
            with self.subTest(payloads=len(payloads)), self.assertRaises(qualification.QualificationError):
                qualification.producer_collection(mock.Mock(side_effect=payloads), "jobs?filter=all", "jobs")

    def fake_api(self, run, jobs, artifacts, *, changed_after=False, changed_job=False, changed_artifact=False):
        responses = [run, {"total_count": len(jobs), "jobs": jobs}, {"total_count": len(artifacts), "artifacts": artifacts},
                     jobs[0], artifacts[0], {**run, "run_attempt": 3} if changed_after else run]
        responses = copy.deepcopy(responses)
        if changed_job: responses[3]["head_sha"] = "f" * 40
        if changed_artifact: responses[4]["expired"] = True
        endpoints = ["actions/runs/12", "actions/runs/12/jobs?filter=all&per_page=100&page=1",
            "actions/runs/12/artifacts?per_page=100&page=1", "actions/jobs/101", "actions/artifacts/901", "actions/runs/12"]
        def execute(command, **kwargs):
            self.assertEqual(command[:6], ["gh", "api", "--hostname", "github.com", "--method", "GET"])
            self.assertEqual(command[6], "repos/owner/repo/" + endpoints.pop(0))
            self.assertGreater(kwargs["timeout"], 0)
            self.assertLessEqual(kwargs["timeout"], 30)
            kwargs["stdout"].write(json.dumps(responses.pop(0)).encode())
            return subprocess.CompletedProcess(command, 0)
        return execute

    def test_cli_resolves_and_retains_two_attempt_original_authority(self):
        run, jobs, artifacts = self.fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with mock.patch.object(qualification.subprocess, "run", side_effect=self.fake_api(run, jobs, artifacts)), \
                 mock.patch.dict(os.environ, GITHUB_OUTPUT=str(root / "outputs")), redirect_stdout(io.StringIO()):
                result = qualification.main(["verify-producer-run", "--candidate-sha", SHA, "--repository", "owner/repo",
                    "--run-id", "12", "--version-tag", "v0.2.10", "--evidence-dir", str(root / "authority")])
            self.assertEqual(result, 0)
            self.assertEqual(dict(line.split("=", 1) for line in (root / "outputs").read_text().splitlines()), {
                "publication_final_attempt": "2", "container_attempt": "1", "container_job_id": "101", "artifact_id": "901"})
            receipt = qualification.read(root / "authority/producer.json")
            self.assertEqual(len(receipt["inputs"]), 6)
            for item in receipt["inputs"]:
                self.assertEqual(item["sha256"], qualification.artifact_input.sha256_file(root / "authority" / item["path"]))
                self.assertEqual(item["request_sha256"], qualification.artifact_input.sha256_file(root / "authority" / item["request_path"]))

    def test_cli_concurrent_rerun_cannot_emit_download_authority(self):
        run, jobs, artifacts = self.fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with mock.patch.object(qualification.subprocess, "run", side_effect=self.fake_api(run, jobs, artifacts, changed_after=True)), \
                 mock.patch.dict(os.environ, GITHUB_OUTPUT=str(root / "outputs")), redirect_stderr(io.StringIO()):
                result = qualification.main(["verify-producer-run", "--candidate-sha", SHA, "--repository", "owner/repo",
                    "--run-id", "12", "--version-tag", "v0.2.10", "--evidence-dir", str(root / "authority")])
            self.assertEqual(result, 1)
            self.assertFalse((root / "outputs").exists())
            self.assertFalse((root / "authority/producer.json").exists())
            self.assertEqual(len(list((root / "authority").glob("response-*.json"))), 6)

    def test_cli_changed_direct_job_or_artifact_cannot_emit_authority(self):
        for defect in ("changed_job", "changed_artifact"):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                run, jobs, artifacts = self.fixture()
                with mock.patch.object(qualification.subprocess, "run", side_effect=self.fake_api(run, jobs, artifacts, **{defect: True})), \
                     mock.patch.dict(os.environ, GITHUB_OUTPUT=str(root / "outputs")), redirect_stderr(io.StringIO()):
                    result = qualification.main(["verify-producer-run", "--candidate-sha", SHA, "--repository", "owner/repo",
                        "--run-id", "12", "--version-tag", "v0.2.10", "--evidence-dir", str(root / "authority")])
                self.assertEqual(result, 1)
                self.assertFalse((root / "outputs").exists())
                self.assertFalse((root / "authority/producer.json").exists())

    def test_global_lookup_deadline_stops_before_another_api_call(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(qualification.time, "monotonic", side_effect=[0, 301]), \
             mock.patch.object(qualification.subprocess, "run") as runner:
            with self.assertRaisesRegex(qualification.QualificationError, "five-minute deadline"):
                qualification.resolve_container_producer(SHA, "owner/repo", "12", "v0.2.10", Path(temporary) / "authority")
            runner.assert_not_called()


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
    def test_container_producer_names_match_actual_reusable_workflow(self) -> None:
        import yaml

        root = Path(__file__).resolve().parents[2] / ".github/workflows"
        parent = yaml.load((root / "stable-release.yml").read_text(), Loader=yaml.BaseLoader)
        child = yaml.load((root / "publish-server-container.yml").read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(parent["jobs"]["container"]["uses"], "./.github/workflows/publish-server-container.yml")
        self.assertNotIn("name", parent["jobs"]["container"])
        self.assertNotIn("name", child["jobs"]["publish"])
        self.assertEqual(qualification.CONTAINER_PRODUCER_JOB, "container / publish")
        names = [step.get("name") for step in child["jobs"]["publish"]["steps"]]
        indices = [names.index(name) for name in qualification.CONTAINER_PUBLICATION_STEPS]
        self.assertEqual(indices, sorted(set(indices)))

    def test_protection_reader_is_scoped_to_authority_steps_and_not_candidate_jobs(self) -> None:
        import yaml

        root = Path(__file__).resolve().parents[2]
        action = f"actions/create-github-app-token@{VERIFICATION_PINS['actions']['actions/create-github-app-token']['sha']}"
        files = ("stable-release.yml", "sorotte-server-release.yml", "sorotte-gui-release.yml", "publish-server-container.yml", "publish-qualified-archives.yml")
        authorizations = 0
        for name in files:
            workflow = yaml.load((root / ".github/workflows" / name).read_text(), Loader=yaml.BaseLoader)
            for job in workflow["jobs"].values():
                steps = job.get("steps", [])
                for index, step in enumerate(steps):
                    if not any(command in step.get("run", "") for command in (
                        "merge_gate.py authorize-release", "container_promotion.py prepare",
                        "candidate_authority.py authorize-release", "candidate_authority.py download",
                    )):
                        continue
                    authorizations += 1
                    tokens = [item for item in steps[:index] if item.get("uses") == action]
                    self.assertEqual(len(tokens), 1)
                    token = tokens[0]
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
                    self.assertIn(token.get("if"), (None, step.get("if")))
            if name != "stable-release.yml":
                self.assertEqual(workflow["on"]["workflow_call"]["secrets"], {"SOROTTE_PROTECTION_APP_PRIVATE_KEY": {"required": "true"}})
            else:
                consumers = {name for name, job in workflow["jobs"].items() if "secrets" in job}
                self.assertEqual(consumers, {"archives", "container"})
                for consumer in consumers:
                    self.assertEqual(workflow["jobs"][consumer]["secrets"], {"SOROTTE_PROTECTION_APP_PRIVATE_KEY": "${{ secrets.SOROTTE_PROTECTION_APP_PRIVATE_KEY }}"})
        self.assertEqual(authorizations, 9)
        for name in ("package-ci.yml", "playback-lifecycle-release-gate.yml", "gui-native-interactive.yml",
                     "qualify-gui-archive.yml", "qualify-server-archives.yml", "qualify-server-container.yml"):
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
        self.assertNotIn("push", workflow["on"])
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
