from __future__ import annotations

import copy
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock
from urllib.error import HTTPError
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import candidate_archives
import candidate_authority
import container_candidate
from scripts.tests import test_server_container_verification as fixtures


class ArtifactTransportTests(unittest.TestCase):
    def test_api_credential_is_not_forwarded_to_storage_and_bytes_match_digest(self):
        payload = b"immutable qualified bytes"
        descriptor = {"id": 123, "digest": "sha256:" + hashlib.sha256(payload).hexdigest()}
        api = candidate_authority.GitHub("owner/repo", "normal-token", "protection-token")
        redirect = HTTPError("https://api.github.com/", 302, "Found", {"Location": "https://storage.example/opaque-download"}, None)
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(candidate_authority, "build_opener") as opener, mock.patch.object(candidate_authority, "urlopen", return_value=io.BytesIO(payload)) as storage:
            opener.return_value.open.side_effect = redirect
            output = Path(temporary) / "candidate.zip"
            api.download(descriptor, output, limit=1024)
            self.assertEqual(output.read_bytes(), payload)
            request = opener.return_value.open.call_args.args[0]
            self.assertEqual(request.get_header("Authorization"), "Bearer normal-token")
            storage.assert_called_once_with("https://storage.example/opaque-download", timeout=60)

    def test_bad_digest_oversize_and_insecure_redirect_fail_closed(self):
        for variant in ("digest", "size", "redirect"):
            descriptor = {"id": 123, "digest": "sha256:" + hashlib.sha256(b"candidate").hexdigest()}
            if variant == "digest": descriptor["digest"] = "sha256:" + "f" * 64
            location = "http://storage.example/file" if variant == "redirect" else "https://storage.example/file"
            failure = HTTPError("https://api.github.com/", 302, "Found", {"Location": location}, None)
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as temporary, mock.patch.object(candidate_authority, "build_opener") as opener, mock.patch.object(candidate_authority, "urlopen", return_value=io.BytesIO(b"candidate")):
                opener.return_value.open.side_effect = failure
                with self.assertRaises(candidate_authority.gate.GateError):
                    candidate_authority.GitHub("owner/repo", "token").download(descriptor, Path(temporary) / "download.zip", limit=2 if variant == "size" else 100)

    def test_zip_rejects_traversal_links_collisions_and_private_authority_members(self):
        cases = [("../outside",), ("/outside",), ("C:/outside",), ("a\\b",),
                 (".authority/download.json",), ("same", "SAME"), ("parent", "parent/child")]
        for names in cases:
            with self.subTest(names=names), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive = root / "archive.zip"
                with zipfile.ZipFile(archive, "w") as stream:
                    for name in names:
                        # ZipInfo's constructor normalizes the host separator.
                        # Set the raw member name afterwards to exercise an
                        # actual backslash entry on Windows as well as Unix.
                        info = zipfile.ZipInfo("member")
                        info.filename = name
                        info.orig_filename = name
                        stream.writestr(info, b"data")
                destination = root / "output"
                destination.mkdir()
                with self.assertRaises(candidate_authority.gate.GateError):
                    candidate_authority.extract_artifact(archive, destination)
                self.assertEqual(list(destination.iterdir()), [])
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.zip"
            info = zipfile.ZipInfo("linked")
            info.external_attr = 0o120777 << 16
            with zipfile.ZipFile(archive, "w") as stream:
                stream.writestr(info, b"../outside")
            with self.assertRaises(candidate_authority.gate.GateError):
                candidate_authority.extract_artifact(archive, root / "output")

    def test_regular_nested_files_are_materialized_without_overwriting(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.zip"
            with zipfile.ZipFile(archive, "w") as stream:
                stream.writestr("runtime/report.json", b'{"status":"passed"}')
            output = root / "output"
            candidate_authority.extract_artifact(archive, output)
            self.assertEqual((output / "runtime/report.json").read_bytes(), b'{"status":"passed"}')
            with self.assertRaisesRegex(candidate_authority.gate.GateError, "overwrite"):
                candidate_authority.extract_artifact(archive, output)


class ArchiveCollectionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.inputs = []
        roles = ("sorotte-gui-windows-x86_64", "sorotte-server-ubuntu-24.04",
                 "sorotte-server-windows-2025", "durable-qualification-123-1")
        for index, role in enumerate(roles):
            directory = self.root / role
            directory.mkdir()
            name = f"archive-{index}.zip"
            payload = f"qualified archive {index}".encode()
            (directory / name).write_bytes(payload)
            (directory / (name + ".sha256")).write_text(hashlib.sha256(payload).hexdigest() + "  " + name + "\n")
            files = {path.name: {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "size": path.stat().st_size}
                     for path in directory.iterdir()}
            proof = {"kind": "sorotte-candidate-artifact-download", "status": "passed",
                     "candidate_sha": fixtures.SOURCE_SHA, "main_sha": "b" * 40,
                     "qualification_run_id": 123, "qualification_run_attempt": 1,
                     "artifact": {"name": role}, "files": files}
            (directory / ".authority").mkdir()
            (directory / ".authority/download.json").write_text(json.dumps(proof))
            self.inputs.append(directory)

    def test_collects_qualified_bytes_and_preserves_original_producer(self):
        report = candidate_archives.collect(self.inputs, self.root / "public", fixtures.SOURCE_SHA, server_only=False)
        self.assertEqual(len(report["files"]), 8)
        self.assertFalse(report["rebuilt"])
        self.assertFalse(report["application_tests_executed"])
        self.assertEqual(report["qualification_run_id"], 123)
        for directory in self.inputs:
            for path in directory.iterdir():
                if path.is_file():
                    self.assertEqual((self.root / "public" / path.name).read_bytes(), path.read_bytes())

    def test_changed_or_missing_archive_cannot_be_published(self):
        (self.inputs[0] / "archive-0.zip").write_bytes(b"changed after download")
        with self.assertRaisesRegex(ValueError, "bytes changed"):
            candidate_archives.collect(self.inputs, self.root / "public", fixtures.SOURCE_SHA, server_only=False)
        self.assertFalse((self.root / "public").exists())

    def test_omitted_platform_and_mixed_candidate_producers_fail(self):
        with self.assertRaisesRegex(ValueError, "archive group"):
            candidate_archives.collect(self.inputs[:-1], self.root / "missing", fixtures.SOURCE_SHA, server_only=False)
        proof_path = self.inputs[0] / ".authority/download.json"
        proof = json.loads(proof_path.read_text())
        proof["qualification_run_id"] += 1
        proof_path.write_text(json.dumps(proof))
        with self.assertRaisesRegex(ValueError, "one original candidate"):
            candidate_archives.collect(self.inputs, self.root / "mixed", fixtures.SOURCE_SHA, server_only=False)

    def test_premerge_handoff_cannot_claim_main_and_cannot_be_used_for_publication(self):
        for directory in self.inputs:
            proof_path = directory / ".authority/download.json"
            proof = json.loads(proof_path.read_text())
            proof["main_sha"] = None
            proof_path.write_text(json.dumps(proof))
        with self.assertRaises(ValueError):
            candidate_archives.collect(self.inputs, self.root / "invalid-public", fixtures.SOURCE_SHA, server_only=False)
        report = candidate_archives.collect(self.inputs, self.root / "preview", fixtures.SOURCE_SHA, server_only=False, premerge=True)
        self.assertIsNone(report["main_sha"])


class ContainerTransferTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.evidence = self.root / "evidence"
        (self.evidence / "runtime").mkdir(parents=True)
        self.runtime = fixtures.valid_runtime_report()
        fixtures.write_json(self.evidence / "runtime/runtime-report.json", self.runtime)
        fixtures.write_json(self.evidence / "sbom.spdx.json", fixtures.valid_sbom())
        sbom_digest = "sha256:" + hashlib.sha256((self.evidence / "sbom.spdx.json").read_bytes()).hexdigest()
        fixtures.write_json(self.evidence / "sbom-report.json", fixtures.valid_sbom_report(sbom_digest=sbom_digest))
        self.bundle = self.root / "bundle"
        self.commands = []

    def execute(self, command, **_kwargs):
        self.commands.append(command)
        if command[:3] == ["docker", "image", "save"]:
            Path(command[4]).write_bytes(b"test fixture for an exported image")
        return subprocess.CompletedProcess(command, 0)

    def seal(self):
        with mock.patch.object(container_candidate.container, "inspect_local_image", return_value=copy.deepcopy(self.runtime["localImage"])), mock.patch.object(container_candidate.subprocess, "run", side_effect=self.execute):
            return container_candidate.seal(fixtures.SOURCE_SHA, fixtures.TEST_IMAGE, self.evidence, self.bundle, "ropbet-radbyt/sorotte")

    def test_restores_original_config_and_layer_identity_without_building_or_running(self):
        self.seal()
        with mock.patch.object(container_candidate.container, "inspect_local_image", return_value=copy.deepcopy(self.runtime["localImage"])), mock.patch.object(container_candidate.subprocess, "run", side_effect=self.execute):
            result = container_candidate.load(fixtures.SOURCE_SHA, fixtures.TEST_IMAGE, self.evidence, self.bundle, "ropbet-radbyt/sorotte")
        self.assertEqual([command[2] for command in self.commands], ["save", "load", "tag"])
        self.assertEqual(self.commands[0][-1], fixtures.LOCAL_IMAGE_ID)
        self.assertEqual(result["local_image"], self.runtime["localImage"])
        self.assertFalse(result["rebuilt"])
        self.assertFalse(result["application_tests_executed"])

    def test_changed_image_archive_is_rejected_before_loading_docker(self):
        self.seal()
        (self.bundle / "candidate-image.tar").write_bytes(b"changed archive")
        with mock.patch.object(container_candidate.subprocess, "run") as execute, self.assertRaisesRegex(ValueError, "export or its original"):
            container_candidate.load(fixtures.SOURCE_SHA, fixtures.TEST_IMAGE, self.evidence, self.bundle, "ropbet-radbyt/sorotte")
        execute.assert_not_called()

    def test_mismatched_runtime_or_sbom_does_not_export_an_image(self):
        report = json.loads((self.evidence / "sbom-report.json").read_text())
        report["localImageId"] = "sha256:" + "f" * 64
        fixtures.write_json(self.evidence / "sbom-report.json", report)
        with mock.patch.object(container_candidate.subprocess, "run") as execute, self.assertRaisesRegex(ValueError, "same tested source"):
            self.seal()
        execute.assert_not_called()

    def test_different_restored_image_is_rejected_before_tagging(self):
        self.seal()
        wrong = copy.deepcopy(self.runtime["localImage"])
        wrong["id"] = "sha256:" + "f" * 64
        self.commands = []
        with mock.patch.object(container_candidate.container, "inspect_local_image", return_value=wrong), mock.patch.object(container_candidate.subprocess, "run", side_effect=self.execute), self.assertRaisesRegex(ValueError, "restored image differs"):
            container_candidate.load(fixtures.SOURCE_SHA, fixtures.TEST_IMAGE, self.evidence, self.bundle, "ropbet-radbyt/sorotte")
        self.assertEqual([command[2] for command in self.commands], ["load"])

    def test_complete_handoff_consumes_uploaded_bytes_and_requires_cold_restoration(self):
        self.seal()
        source = fixtures.SOURCE_SHA
        names = candidate_authority.required_artifacts(source, 123, 1)
        payloads = {}
        for index, name in enumerate(sorted(names)):
            stream = io.BytesIO()
            with zipfile.ZipFile(stream, "w") as archive:
                directory = (self.bundle if name.startswith("qualified-container-") else
                             self.evidence if name.startswith("server-container-verification-") else None)
                if directory is None:
                    filename = f"archive-{index}.zip"
                    payload = f"qualified bytes {index}".encode()
                    archive.writestr(filename, payload)
                    archive.writestr(filename + ".sha256", hashlib.sha256(payload).hexdigest() + "  " + filename + "\n")
                else:
                    for path in sorted(directory.rglob("*")):
                        if path.is_file():
                            archive.write(path, path.relative_to(directory).as_posix())
            payloads[name] = stream.getvalue()
        manifest = {"candidate_sha": source, "run_id": 123, "run_attempt": 1,
                    "artifacts": {name: {"id": index + 1, "name": name, "digest": "sha256:" + hashlib.sha256(data).hexdigest(),
                                          "size_in_bytes": len(data)} for index, (name, data) in enumerate(payloads.items())}}
        downloaded = []

        def download(descriptor, output, *, limit):
            data = payloads[descriptor["name"]]
            assert len(data) <= limit
            downloaded.append(descriptor["name"])
            output.write_bytes(data)

        api = SimpleNamespace(repository="ropbet-radbyt/sorotte", download=download)
        for cold in (True, False):
            self.commands = []
            downloaded.clear()

            def execute(command, **kwargs):
                result = self.execute(command, **kwargs)
                if command[:3] == ["docker", "image", "inspect"]:
                    result.returncode = 1 if cold else 0
                return result

            with self.subTest(cold=cold), mock.patch.object(candidate_authority, "seal", return_value=manifest), mock.patch.object(container_candidate.container, "inspect_local_image", return_value=copy.deepcopy(self.runtime["localImage"])), mock.patch.object(candidate_authority.subprocess, "run", side_effect=execute):
                output = self.root / ("cold" if cold else "warm")
                if cold:
                    result = candidate_authority.handoff(api, {}, 123, output)
                    self.assertTrue(result["cold_restore"])
                    self.assertFalse(result["publication_authorized"])
                    self.assertEqual(len(result["archives"]["files"]), 8)
                    self.assertEqual([command[2] for command in self.commands], ["inspect", "load", "tag"])
                else:
                    with self.assertRaisesRegex(candidate_authority.gate.GateError, "worker without"):
                        candidate_authority.handoff(api, {}, 123, output)
                    self.assertEqual([command[2] for command in self.commands], ["inspect"])
                self.assertEqual(set(downloaded), names)


if __name__ == "__main__":
    unittest.main()
