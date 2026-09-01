from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "verify_server_release_artifact.py"
SPEC = importlib.util.spec_from_file_location("verify_server_release_artifact", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
artifact = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = artifact
SPEC.loader.exec_module(artifact)

SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567"
VERSION = "0.2.3"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class ArtifactBuilder:
    def __init__(self, root: Path, platform: str = "windows") -> None:
        self.artifacts_dir = root / "artifacts"
        self.artifacts_dir.mkdir()
        self.platform = platform
        self.suffix = ".zip" if platform == "windows" else ".tar.gz"
        self.root_name = f"sorotte-server-{VERSION}-{platform}-x86_64"
        self.archive_path = self.artifacts_dir / f"{self.root_name}{self.suffix}"
        self.binary_name = "sorotte-server.exe" if platform == "windows" else "sorotte-server"
        self.payloads = {
            self.binary_name: b"not-a-real-binary",
            "README.md": b"readme\n",
            "SERVER_RELEASE.md": b"release guide\n",
            "LICENSE": b"license\n",
        }

    def manifest(self, **overrides: object) -> dict[str, object]:
        value: dict[str, object] = {
            "schemaVersion": 1,
            "package": "sorotte-server",
            "version": VERSION,
            "platform": self.platform,
            "architecture": "x86_64",
            "sourceSha": SOURCE_SHA,
            "files": [
                {"path": name, "size": len(data), "sha256": digest(data)}
                for name, data in self.payloads.items()
            ],
        }
        value.update(overrides)
        return value

    def default_entries(
        self, manifest: dict[str, object] | bytes | None = None
    ) -> list[tuple[str, bytes, int | None]]:
        manifest_bytes = (
            json.dumps(self.manifest(), separators=(",", ":")).encode()
            if manifest is None
            else (
                manifest
                if isinstance(manifest, bytes)
                else json.dumps(manifest, separators=(",", ":")).encode()
            )
        )
        entries = [
            (f"{self.root_name}/{name}", data, None) for name, data in self.payloads.items()
        ]
        entries.append((f"{self.root_name}/manifest.json", manifest_bytes, None))
        return entries

    def write(
        self,
        *,
        entries: list[tuple[str, bytes, int | None]] | None = None,
        manifest: dict[str, object] | bytes | None = None,
        checksum_text: str | None = None,
    ) -> Path:
        entries = self.default_entries(manifest) if entries is None else entries
        if self.platform == "windows":
            with zipfile.ZipFile(self.archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
                directory = zipfile.ZipInfo(f"{self.root_name}/")
                directory.external_attr = (stat.S_IFDIR | 0o755) << 16
                archive.writestr(directory, b"")
                for name, data, mode in entries:
                    info = zipfile.ZipInfo(name)
                    info.compress_type = zipfile.ZIP_DEFLATED
                    info.external_attr = (
                        mode if mode is not None else stat.S_IFREG | 0o644
                    ) << 16
                    archive.writestr(info, data)
        else:
            with tarfile.open(self.archive_path, "w:gz") as archive:
                directory = tarfile.TarInfo(self.root_name)
                directory.type = tarfile.DIRTYPE
                directory.mode = 0o755
                archive.addfile(directory)
                for name, data, mode in entries:
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    info.mode = mode if mode is not None else 0o755
                    archive.addfile(info, io.BytesIO(data))
        archive_digest = artifact.sha256_file(self.archive_path)
        if checksum_text is None:
            checksum_text = f"{archive_digest}  {self.archive_path.name}\n"
        self.archive_path.with_name(f"{self.archive_path.name}.sha256").write_text(
            checksum_text, encoding="ascii", newline="\n"
        )
        return self.archive_path

    def verify(self) -> dict[str, object]:
        return artifact.verify_release(
            self.artifacts_dir,
            SOURCE_SHA,
            runtime_smoke=False,
            work_dir=self.artifacts_dir.parent / "work",
        )


class ReleaseArtifactHappyPathTests(unittest.TestCase):
    def test_valid_windows_archive_is_bound_in_report(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write()

            report = builder.verify()

            self.assertEqual(report["status"], "verified")
            self.assertEqual(report["package"]["sourceSha"], SOURCE_SHA)
            self.assertEqual(report["archive"]["sha256"], artifact.sha256_file(builder.archive_path))
            self.assertFalse(report["runtimeSmoke"]["performed"])
            self.assertEqual(
                {entry["path"] for entry in report["package"]["files"]},
                set(builder.payloads),
            )

    def test_valid_linux_tar_preserves_and_verifies_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary), platform="linux")
            builder.write()

            report = builder.verify()

            self.assertEqual(report["package"]["platform"], "linux")
            self.assertEqual(report["archive"]["name"], builder.archive_path.name)

    def test_optional_windows_symbols_archive_is_consumed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write()
            symbols = builder.artifacts_dir / f"{builder.root_name}-symbols.zip"
            with zipfile.ZipFile(symbols, "w") as archive:
                archive.writestr("sorotte_server.pdb", b"symbols")
            symbols.with_name(f"{symbols.name}.sha256").write_text(
                f"{artifact.sha256_file(symbols)}  {symbols.name}\n",
                encoding="ascii",
            )

            report = builder.verify()

            self.assertEqual(report["symbols"]["archive"], symbols.name)
            self.assertEqual(report["symbols"]["sha256"], artifact.sha256_file(symbols))

    def test_report_write_is_complete_json(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report_path = Path(temporary) / "nested" / "report.json"
            expected = {"status": "verified", "schemaVersion": 1}

            artifact.write_report(expected, report_path)

            self.assertEqual(json.loads(report_path.read_text(encoding="utf-8")), expected)
            self.assertEqual(list(report_path.parent.glob("*.tmp")), [])

    def test_failure_report_is_machine_readable_and_source_bound(self) -> None:
        report = artifact.failure_report(
            SOURCE_SHA.upper(), artifact.VerificationError("unsafe archive")
        )

        self.assertEqual(
            report,
            {
                "schemaVersion": 1,
                "status": "failed",
                "expectedSourceSha": SOURCE_SHA,
                "error": "unsafe archive",
            },
        )


class RuntimeConsumerContractTests(unittest.TestCase):
    def test_runtime_smoke_receives_only_the_exact_freshly_extracted_binary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write()
            observed: dict[str, Path] = {}

            def smoke(binary_path: Path, version: str, runtime_root: Path) -> dict[str, object]:
                self.assertEqual(version, VERSION)
                self.assertEqual(binary_path.read_bytes(), builder.payloads[builder.binary_name])
                self.assertEqual(binary_path.name, builder.binary_name)
                self.assertEqual(binary_path.parent.name, builder.root_name)
                self.assertFalse(runtime_root.exists())
                self.assertNotEqual(binary_path.parent, runtime_root)
                runtime_root.mkdir()
                observed["binary"] = binary_path
                observed["runtime"] = runtime_root
                return {"performed": True, "protocolHello": True}

            with mock.patch.object(artifact, "smoke_test_binary", side_effect=smoke):
                report = artifact.verify_release(
                    builder.artifacts_dir,
                    SOURCE_SHA,
                    runtime_smoke=True,
                    work_dir=Path(temporary) / "fresh-consumer-work",
                )

            self.assertIn("binary", observed)
            self.assertIn("runtime", observed)
            self.assertTrue(report["runtimeSmoke"]["performed"])
            self.assertTrue(report["runtimeSmoke"]["packageUnmodified"])

    def test_runtime_environment_removes_all_sorotte_configuration_overrides(self) -> None:
        override_names = [
            "SOROTTE_PASSWORD",
            "SOROTTE_SERVER_ROOMS_DB_FILE",
            "SOROTTE_SERVER_TLS_CERT_PATH",
            "SOROTTE_UNRECOGNIZED_FUTURE_OVERRIDE",
        ]
        with mock.patch.dict(
            os.environ,
            {name: f"untrusted-{index}" for index, name in enumerate(override_names)},
            clear=False,
        ):
            isolated = artifact._isolated_server_environment()

        for name in override_names:
            self.assertNotIn(name, isolated)
        if "PATH" in os.environ:
            self.assertEqual(isolated["PATH"], os.environ["PATH"])

    def test_clean_shutdown_uses_a_bounded_platform_signal_and_requires_zero_exit(self) -> None:
        class CleanProcess:
            returncode: int | None = None
            signal: int | None = None
            pid = 4242

            def poll(self) -> int | None:
                return self.returncode

            def send_signal(self, requested: int) -> None:
                self.signal = requested

            def wait(self, timeout: int) -> int:
                self.assert_timeout = timeout
                self.returncode = 0
                return 0

        process = CleanProcess()
        with mock.patch.object(artifact, "_send_windows_ctrl_c") as windows_ctrl_c:
            shutdown = artifact._request_clean_shutdown(process)

        self.assertTrue(shutdown["clean"])
        self.assertEqual(shutdown["exitCode"], 0)
        self.assertEqual(process.assert_timeout, artifact.SERVER_SHUTDOWN_TIMEOUT_SECONDS)
        if os.name == "nt":
            windows_ctrl_c.assert_called_once_with(process.pid)
            self.assertIsNone(process.signal)
        else:
            windows_ctrl_c.assert_not_called()
            self.assertEqual(process.signal, artifact.signal.SIGINT)

    def test_shutdown_timeout_forcibly_reaps_and_fails_verification(self) -> None:
        class StalledProcess:
            returncode: int | None = None
            killed = False
            waits = 0
            pid = 4343

            def poll(self) -> int | None:
                return self.returncode

            def send_signal(self, _requested: int) -> None:
                pass

            def wait(self, timeout: int) -> int:
                self.waits += 1
                if not self.killed:
                    raise subprocess.TimeoutExpired("sorotte-server", timeout)
                self.returncode = 1
                return 1

            def kill(self) -> None:
                self.killed = True

        process = StalledProcess()
        with mock.patch.object(artifact, "_send_windows_ctrl_c"):
            with self.assertRaisesRegex(
                artifact.VerificationError, "did not shut down cleanly"
            ):
                artifact._request_clean_shutdown(process)

        self.assertTrue(process.killed)
        self.assertEqual(process.returncode, 1)
        self.assertEqual(process.waits, 2)


class ChecksumAndSelectionTests(unittest.TestCase):
    def test_checksum_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write(checksum_text=f"{'0' * 64}  {builder.archive_path.name}\n")
            with self.assertRaisesRegex(artifact.VerificationError, "checksum mismatch"):
                builder.verify()

    def test_checksum_filename_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write(checksum_text=f"{'0' * 64}  another.zip\n")
            with self.assertRaisesRegex(artifact.VerificationError, "checksum names"):
                builder.verify()

    def test_noncanonical_checksum_forms_are_rejected(self) -> None:
        invalid_templates = [
            "{digest} *{name}\n",
            "{digest}  {name}\nextra\n",
            "{upper}  {name}\n",
        ]
        for template in invalid_templates:
            with self.subTest(template=template), tempfile.TemporaryDirectory() as temporary:
                builder = ArtifactBuilder(Path(temporary))
                builder.write()
                actual = artifact.sha256_file(builder.archive_path)
                checksum = template.format(
                    digest=actual, upper=actual.upper(), name=builder.archive_path.name
                )
                builder.archive_path.with_name(f"{builder.archive_path.name}.sha256").write_text(
                    checksum, encoding="ascii"
                )
                with self.assertRaisesRegex(artifact.VerificationError, "checksum must be"):
                    builder.verify()

    def test_multiple_primary_archives_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write()
            second = builder.artifacts_dir / "sorotte-server-9.9.9-windows-x86_64.zip"
            second.write_bytes(b"another archive")
            with self.assertRaisesRegex(artifact.VerificationError, "exactly one"):
                builder.verify()

    def test_unexpected_uploaded_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write()
            (builder.artifacts_dir / "unverified.txt").write_text("unexpected")
            with self.assertRaisesRegex(artifact.VerificationError, "inventory mismatch"):
                builder.verify()

    def test_platform_archive_type_mismatch_is_rejected(self) -> None:
        path = Path("sorotte-server-0.2.3-linux-x86_64.zip")
        with self.assertRaisesRegex(artifact.VerificationError, "must use"):
            artifact.parse_archive_identity(path)


class ArchiveBoundaryTests(unittest.TestCase):
    def _verify_zip_with_replacement(
        self, replacement: tuple[str, bytes, int | None], removed_suffix: str = "README.md"
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            entries = [
                entry
                for entry in builder.default_entries()
                if not entry[0].endswith(f"/{removed_suffix}")
            ]
            entries.append(replacement)
            builder.write(entries=entries)
            builder.verify()

    def test_unsafe_zip_paths_are_rejected_before_extraction(self) -> None:
        attacks = [
            ("../escape", "not normalized"),
            ("/absolute", "absolute"),
            ("C:/drive", "absolute"),
            ("root\\backslash", "backslash"),
            ("root//double", "not normalized"),
        ]
        for name, message in attacks:
            with self.subTest(name=name):
                with self.assertRaisesRegex(artifact.VerificationError, message):
                    self._verify_zip_with_replacement((name, b"attack", None))

    def test_duplicate_zip_member_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            entries = builder.default_entries()
            entries.append(entries[0])
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                builder.write(entries=entries)
            with self.assertRaisesRegex(artifact.VerificationError, "duplicate member"):
                builder.verify()

    def test_case_colliding_zip_members_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            entries = builder.default_entries()
            entries.append((f"{builder.root_name}/readme.md", b"collision", None))
            builder.write(entries=entries)
            with self.assertRaisesRegex(artifact.VerificationError, "case-colliding"):
                builder.verify()

    def test_zip_symbolic_link_is_rejected(self) -> None:
        mode = stat.S_IFLNK | 0o777
        with self.assertRaisesRegex(artifact.VerificationError, "symbolic link"):
            self._verify_zip_with_replacement(
                ("sorotte-server-0.2.3-windows-x86_64/README.md", b"target", mode)
            )

    def test_missing_extra_and_empty_members_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            entries = builder.default_entries()
            builder.write(entries=entries[:-1])
            with self.assertRaisesRegex(artifact.VerificationError, "inventory mismatch"):
                builder.verify()
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            entries = builder.default_entries()
            entries.append((f"{builder.root_name}/EXTRA", b"extra", None))
            builder.write(entries=entries)
            with self.assertRaisesRegex(artifact.VerificationError, "inventory mismatch"):
                builder.verify()
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            entries = [
                (name, b"" if name.endswith("/LICENSE") else data, mode)
                for name, data, mode in builder.default_entries()
            ]
            builder.write(entries=entries)
            with self.assertRaisesRegex(artifact.VerificationError, "must not be empty"):
                builder.verify()

    def test_missing_executable_and_extra_nested_path_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            entries = [
                entry
                for entry in builder.default_entries()
                if not entry[0].endswith(f"/{builder.binary_name}")
            ]
            builder.write(entries=entries)
            with self.assertRaisesRegex(
                artifact.VerificationError, "inventory mismatch.*sorotte-server.exe"
            ):
                builder.verify()
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            entries = builder.default_entries()
            entries.append(
                (
                    f"{builder.root_name}/nested/unmanifested.txt",
                    b"extra nested payload",
                    None,
                )
            )
            builder.write(entries=entries)
            with self.assertRaisesRegex(
                artifact.VerificationError, "inventory mismatch.*nested/unmanifested.txt"
            ):
                builder.verify()

    def test_corrupt_and_truncated_archives_fail_after_valid_checksum(self) -> None:
        for label, mutate in [
            ("corrupt", lambda _archive: b"not a ZIP archive"),
            ("truncated", lambda archive: archive[:-17]),
        ]:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                builder = ArtifactBuilder(Path(temporary))
                builder.write()
                corrupted = mutate(builder.archive_path.read_bytes())
                builder.archive_path.write_bytes(corrupted)
                checksum = builder.archive_path.with_name(
                    f"{builder.archive_path.name}.sha256"
                )
                checksum.write_text(
                    f"{artifact.sha256_file(builder.archive_path)}  "
                    f"{builder.archive_path.name}\n",
                    encoding="ascii",
                )

                with self.assertRaisesRegex(
                    artifact.VerificationError, "could not safely read ZIP archive"
                ):
                    builder.verify()

    def test_extraction_requires_a_fresh_destination(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write()
            destination = Path(temporary) / "already-used"
            destination.mkdir()

            with self.assertRaisesRegex(
                artifact.VerificationError, "destination must not already exist"
            ):
                artifact.safe_extract_archive(
                    builder.archive_path,
                    destination,
                    root_name=builder.root_name,
                    expected_relative_files={
                        builder.binary_name,
                        "README.md",
                        "SERVER_RELEASE.md",
                        "LICENSE",
                        "manifest.json",
                    },
                )

    def test_declared_file_size_limit_is_enforced(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write()
            with mock.patch.object(artifact, "MAX_FILE_BYTES", 4):
                with self.assertRaisesRegex(artifact.VerificationError, "size limit"):
                    builder.verify()

    def test_tar_links_and_special_files_are_rejected(self) -> None:
        special_types = [
            ("symbolic", tarfile.SYMTYPE),
            ("hard", tarfile.LNKTYPE),
            ("device", tarfile.CHRTYPE),
            ("fifo", tarfile.FIFOTYPE),
        ]
        for label, member_type in special_types:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                builder = ArtifactBuilder(Path(temporary), platform="linux")
                with tarfile.open(builder.archive_path, "w:gz") as archive:
                    directory = tarfile.TarInfo(builder.root_name)
                    directory.type = tarfile.DIRTYPE
                    archive.addfile(directory)
                    for name, data, _ in builder.default_entries():
                        info = tarfile.TarInfo(name)
                        if name.endswith("/README.md"):
                            info.type = member_type
                            info.linkname = "target"
                            info.size = 0
                            archive.addfile(info)
                        else:
                            info.size = len(data)
                            archive.addfile(info, io.BytesIO(data))
                builder.archive_path.with_name(f"{builder.archive_path.name}.sha256").write_text(
                    f"{artifact.sha256_file(builder.archive_path)}  {builder.archive_path.name}\n",
                    encoding="ascii",
                )
                with self.assertRaisesRegex(artifact.VerificationError, "special files"):
                    builder.verify()


class ManifestContractTests(unittest.TestCase):
    def test_source_commit_platform_and_version_drift_are_rejected(self) -> None:
        variants = [
            ({"sourceSha": "f" * 40}, "sourceSha mismatch"),
            ({"platform": "linux"}, "platform mismatch"),
            ({"version": "9.9.9"}, "version mismatch"),
            ({"architecture": "aarch64"}, "architecture mismatch"),
        ]
        for override, message in variants:
            with self.subTest(override=override), tempfile.TemporaryDirectory() as temporary:
                builder = ArtifactBuilder(Path(temporary))
                builder.write(manifest=builder.manifest(**override))
                with self.assertRaisesRegex(artifact.VerificationError, message):
                    builder.verify()

    def test_manifest_digest_and_size_drift_are_rejected(self) -> None:
        for key, value, message in [
            ("sha256", "0" * 64, "digest mismatch"),
            ("size", 1, "size mismatch"),
        ]:
            with self.subTest(key=key), tempfile.TemporaryDirectory() as temporary:
                builder = ArtifactBuilder(Path(temporary))
                manifest = builder.manifest()
                manifest["files"][0][key] = value
                builder.write(manifest=manifest)
                with self.assertRaisesRegex(artifact.VerificationError, message):
                    builder.verify()

    def test_manifest_missing_extra_and_duplicate_file_entries_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            manifest = builder.manifest()
            manifest["files"] = manifest["files"][:-1]
            builder.write(manifest=manifest)
            with self.assertRaisesRegex(artifact.VerificationError, "file inventory mismatch"):
                builder.verify()
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            manifest = builder.manifest()
            manifest["files"].append(
                {"path": "EXTRA", "size": 1, "sha256": digest(b"x")}
            )
            builder.write(manifest=manifest)
            with self.assertRaisesRegex(artifact.VerificationError, "unexpected file"):
                builder.verify()
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            manifest = builder.manifest()
            manifest["files"].append(dict(manifest["files"][0]))
            builder.write(manifest=manifest)
            with self.assertRaisesRegex(artifact.VerificationError, "duplicate file"):
                builder.verify()

    def test_duplicate_json_keys_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            valid = json.dumps(builder.manifest(), separators=(",", ":"))
            duplicate = valid.replace(
                '"schemaVersion":1', '"schemaVersion":1,"schemaVersion":1', 1
            ).encode()
            builder.write(manifest=duplicate)
            with self.assertRaisesRegex(artifact.VerificationError, "duplicate JSON key"):
                builder.verify()

    def test_manifest_schema_is_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write(manifest=builder.manifest(untrusted="value"))
            with self.assertRaisesRegex(artifact.VerificationError, "keys mismatch"):
                builder.verify()

    def test_expected_source_sha_must_be_full_hex(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = ArtifactBuilder(Path(temporary))
            builder.write()
            with self.assertRaisesRegex(artifact.VerificationError, "40 hexadecimal"):
                artifact.verify_release(
                    builder.artifacts_dir,
                    "short",
                    runtime_smoke=False,
                    work_dir=Path(temporary) / "work",
                )


class ReleaseWorkflowPolicyTests(unittest.TestCase):
    def test_release_workflow_verifies_the_package_before_upload(self) -> None:
        workflow = (REPO_ROOT / ".github" / "workflows" / "sorotte-server-release.yml").read_text(
            encoding="utf-8"
        )
        package_step = workflow.index("- name: Package server release")
        verify_step = workflow.index("- name: Verify packaged server release")
        upload_step = workflow.index("- name: Upload server package")
        self.assertLess(package_step, verify_step)
        self.assertLess(verify_step, upload_step)
        self.assertIn("scripts/verify_server_release_artifact.py", workflow)
        self.assertIn('--expected-source-sha "${{ github.sha }}"', workflow)
        self.assertIn("target/server-release/artifacts/*", workflow)
        self.assertIn("target/server-release/artifact-verification.json", workflow)
        report_step = workflow[workflow.index("- name: Upload verification report") : upload_step]
        self.assertIn("if: always()", report_step)

    def test_release_workflow_actions_are_commit_pinned(self) -> None:
        workflow = (REPO_ROOT / ".github" / "workflows" / "sorotte-server-release.yml").read_text(
            encoding="utf-8"
        )
        action_lines = [
            line.strip() for line in workflow.splitlines() if line.strip().startswith("uses:")
        ]
        self.assertGreaterEqual(len(action_lines), 5)
        local_workflows = [line for line in action_lines if line.startswith("uses: ./")]
        self.assertEqual(
            local_workflows,
            ["uses: ./.github/workflows/playback-lifecycle-release-gate.yml"],
        )
        for line in action_lines:
            if line.startswith("uses: ./"):
                continue
            reference = line.split("#", 1)[0].rsplit("@", 1)[1].strip()
            self.assertRegex(reference, r"^[0-9a-f]{40}$", line)


if __name__ == "__main__":
    unittest.main()
