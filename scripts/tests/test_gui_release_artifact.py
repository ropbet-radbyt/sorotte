from __future__ import annotations

import hashlib
import importlib.util
import json
import stat
import sys
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = REPO_ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))
SCRIPT_PATH = SCRIPTS / "verify_gui_release_artifact.py"
SPEC = importlib.util.spec_from_file_location("verify_gui_release_artifact", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
artifact = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = artifact
SPEC.loader.exec_module(artifact)

SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567"
VERSION = "0.2.4"
CREATED_AT = "2026-07-30T01:02:03Z"


def digest(body: bytes) -> str:
    return hashlib.sha256(body).hexdigest()


class GuiArtifactBuilder:
    def __init__(self, root: Path) -> None:
        self.artifacts_dir = root / "artifacts"
        self.artifacts_dir.mkdir()
        self.archive_path = (
            self.artifacts_dir
            / f"sorotte-gui-{VERSION}-windows-x86_64.zip"
        )
        self.payloads = {
            "sorotte-gui.exe": b"synthetic GUI executable",
            "sorotte-gui-updater.exe": b"synthetic updater executable",
            "README.md": b"readme\n",
            "LICENSE": b"license\n",
            "resources/sorotte_syncplayintf.lua": b"lua fixture\n",
        }

    def install_manifest(self, **overrides: object) -> dict[str, object]:
        value: dict[str, object] = {
            "schema": "sorotte-gui-install-manifest-v2",
            "app": "sorotte-gui",
            "channel": "dev",
            "version": VERSION,
            "git_sha": SOURCE_SHA,
            "created_at_utc": CREATED_AT,
            "target": "windows-x86_64",
            "files": [
                {"path": relative, "sha256": digest(body)}
                for relative, body in sorted(self.payloads.items())
            ],
        }
        value.update(overrides)
        return value

    def install_manifest_bytes(
        self,
        value: dict[str, object] | bytes | None = None,
    ) -> bytes:
        if isinstance(value, bytes):
            return value
        return json.dumps(
            self.install_manifest() if value is None else value,
            separators=(",", ":"),
        ).encode()

    def default_entries(
        self,
        manifest: dict[str, object] | bytes | None = None,
    ) -> list[tuple[str, bytes, int | None]]:
        entries = [
            (relative, body, None) for relative, body in self.payloads.items()
        ]
        entries.append(
            (
                "sorotte-install.json",
                self.install_manifest_bytes(manifest),
                None,
            )
        )
        return entries

    def update_manifest(
        self,
        archive_digest: str,
        **overrides: object,
    ) -> dict[str, object]:
        value: dict[str, object] = {
            "schema": "sorotte-gui-update-manifest-v1",
            "app": "sorotte-gui",
            "channel": "dev",
            "version": VERSION,
            "git_sha": SOURCE_SHA,
            "created_at_utc": CREATED_AT,
            "target": "windows-x86_64",
            "package": self.archive_path.name,
            "sha256": archive_digest,
        }
        value.update(overrides)
        return value

    def write(
        self,
        *,
        entries: list[tuple[str, bytes, int | None]] | None = None,
        install_manifest: dict[str, object] | bytes | None = None,
        update_overrides: dict[str, object] | None = None,
        update_manifest_bytes: bytes | None = None,
        checksum_text: str | None = None,
    ) -> Path:
        entries = (
            self.default_entries(install_manifest)
            if entries is None
            else entries
        )
        with zipfile.ZipFile(
            self.archive_path,
            "w",
            compression=zipfile.ZIP_DEFLATED,
        ) as archive:
            for name, body, mode in entries:
                info = zipfile.ZipInfo(name)
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = (
                    mode if mode is not None else stat.S_IFREG | 0o644
                ) << 16
                archive.writestr(info, body)
        archive_digest = artifact.sha256_file(self.archive_path)
        if checksum_text is None:
            checksum_text = f"{archive_digest}  {self.archive_path.name}\n"
        self.archive_path.with_name(f"{self.archive_path.name}.sha256").write_text(
            checksum_text,
            encoding="ascii",
            newline="\n",
        )
        manifest_path = self.artifacts_dir / "sorotte-update-manifest.json"
        if update_manifest_bytes is not None:
            manifest_path.write_bytes(update_manifest_bytes)
        else:
            manifest = self.update_manifest(
                archive_digest,
                **(update_overrides or {}),
            )
            manifest_path.write_text(
                json.dumps(manifest, separators=(",", ":")),
                encoding="utf-8",
            )
        return self.archive_path

    def add_symbols(self, entries: dict[str, bytes]) -> Path:
        symbols = self.artifacts_dir / f"{self.archive_path.stem}-symbols.zip"
        with zipfile.ZipFile(symbols, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for name, body in entries.items():
                archive.writestr(name, body)
        symbols.with_name(f"{symbols.name}.sha256").write_text(
            f"{artifact.sha256_file(symbols)}  {symbols.name}\n",
            encoding="ascii",
        )
        return symbols

    def verify(
        self,
        *,
        runtime_smoke: bool = False,
    ) -> dict[str, object]:
        return artifact.verify_release(
            self.artifacts_dir,
            SOURCE_SHA,
            "dev",
            runtime_smoke=runtime_smoke,
            work_dir=self.artifacts_dir.parent / "work",
        )


class GuiArtifactHappyPathTests(unittest.TestCase):
    def test_valid_archive_binds_both_manifests_and_every_payload(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()

            report = builder.verify()

            self.assertEqual(report["status"], "verified")
            self.assertEqual(report["package"]["sourceSha"], SOURCE_SHA)
            self.assertEqual(report["package"]["channel"], "dev")
            self.assertEqual(
                {entry["path"] for entry in report["package"]["files"]},
                set(builder.payloads),
            )
            self.assertEqual(
                report["archive"]["sha256"],
                artifact.sha256_file(builder.archive_path),
            )
            self.assertFalse(report["runtimeProof"]["performed"])

    def test_optional_symbols_are_checksum_verified_and_extracted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            symbols = builder.add_symbols(
                {
                    "sorotte_gui.pdb": b"GUI symbols",
                    "sorotte_gui_updater.pdb": b"updater symbols",
                }
            )

            report = builder.verify()

            self.assertEqual(report["symbols"]["archive"], symbols.name)
            self.assertEqual(
                report["symbols"]["files"],
                ["sorotte_gui.pdb", "sorotte_gui_updater.pdb"],
            )

    def test_runtime_experiments_receive_the_exact_extracted_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            runtime_report = {
                "performed": True,
                "guiLaunch": {"visibleMainWindow": True},
                "updaterSuccess": {"selfReplacement": True},
                "updaterRollback": {"originalInstallRestored": True},
            }
            observed_gui_digest: list[str] = []

            def observe_runtime(
                package_root: Path,
                _archive_path: Path,
                _archive_digest: str,
                _runtime_root: Path,
            ) -> dict[str, object]:
                observed_gui_digest.append(
                    artifact.sha256_file(package_root / "sorotte-gui.exe")
                )
                return runtime_report

            with mock.patch.object(
                artifact,
                "run_runtime_experiments",
                side_effect=observe_runtime,
            ) as runner:
                report = builder.verify(runtime_smoke=True)

            self.assertEqual(report["runtimeProof"], runtime_report)
            _package_root, archive_path, archive_digest, _runtime_root = runner.call_args.args
            self.assertEqual(archive_path, builder.archive_path.resolve())
            self.assertEqual(archive_digest, artifact.sha256_file(builder.archive_path))
            self.assertEqual(
                observed_gui_digest,
                [digest(builder.payloads["sorotte-gui.exe"])],
            )

    def test_elevated_runtime_requires_refusal_and_skips_mutation_rollback(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package_root = root / "package"
            package_root.mkdir()
            runtime_root = root / "runtime"
            elevated_refusal = {
                "performed": True,
                "elevatedRefusal": True,
                "refusedBeforeMutation": True,
            }
            with (
                mock.patch.object(artifact.os, "name", "nt"),
                mock.patch.object(
                    artifact,
                    "_current_process_is_elevated",
                    return_value=True,
                ),
                mock.patch.object(
                    artifact,
                    "smoke_test_gui",
                    return_value={"visibleMainWindow": True},
                ),
                mock.patch.object(
                    artifact,
                    "smoke_test_updater_success",
                    return_value=elevated_refusal,
                ) as updater,
                mock.patch.object(artifact, "smoke_test_updater_rollback") as rollback,
            ):
                report = artifact.run_runtime_experiments(
                    package_root,
                    root / "package.zip",
                    "a" * 64,
                    runtime_root,
                )

            self.assertEqual(
                report["executionContext"],
                {"processElevated": True},
            )
            self.assertEqual(report["updaterSuccess"], elevated_refusal)
            self.assertFalse(report["updaterRollback"]["performed"])
            self.assertEqual(
                report["updaterRollback"]["coveredBy"],
                "updaterSuccess.elevatedRefusal",
            )
            updater.assert_called_once_with(
                package_root,
                root / "package.zip",
                "a" * 64,
                runtime_root,
                process_elevated=True,
            )
            rollback.assert_not_called()

    def test_non_elevated_runtime_requires_success_and_rollback(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package_root = root / "package"
            package_root.mkdir()
            runtime_root = root / "runtime"
            with (
                mock.patch.object(artifact.os, "name", "nt"),
                mock.patch.object(
                    artifact,
                    "_current_process_is_elevated",
                    return_value=False,
                ),
                mock.patch.object(
                    artifact,
                    "smoke_test_gui",
                    return_value={"visibleMainWindow": True},
                ),
                mock.patch.object(
                    artifact,
                    "smoke_test_updater_success",
                    return_value={"selfReplacement": True},
                ) as updater,
                mock.patch.object(
                    artifact,
                    "smoke_test_updater_rollback",
                    return_value={"originalInstallRestored": True},
                ) as rollback,
            ):
                report = artifact.run_runtime_experiments(
                    package_root,
                    root / "package.zip",
                    "b" * 64,
                    runtime_root,
                )

            self.assertEqual(
                report["executionContext"],
                {"processElevated": False},
            )
            updater.assert_called_once_with(
                package_root,
                root / "package.zip",
                "b" * 64,
                runtime_root,
                process_elevated=False,
            )
            rollback.assert_called_once_with(
                package_root,
                root / "package.zip",
                "b" * 64,
                runtime_root,
            )

    def test_elevated_packaged_updater_refusal_is_exact_and_nonmutating(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package_root = root / "package"
            payloads = {
                relative: f"packaged {relative}\n".encode()
                for relative in artifact.PACKAGE_PAYLOADS
            }
            for relative, body in payloads.items():
                destination = artifact._path(package_root, relative)
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(body)
            (package_root / artifact.INSTALL_MANIFEST).write_bytes(
                artifact._manifest_bytes("1.0.0", payloads)
            )
            package_path = root / "package.zip"
            package_path.write_bytes(b"exact package fixture")
            runtime_root = root / "runtime"
            runtime_root.mkdir()

            def refuse(command: list[str], **_kwargs: object) -> object:
                log_path = Path(command[command.index("--log") + 1])
                log_path.write_text(
                    f"{artifact.ELEVATED_UPDATER_REFUSAL}\n",
                    encoding="utf-8",
                )
                return artifact.subprocess.CompletedProcess(command, 1)

            with mock.patch.object(artifact.subprocess, "run", side_effect=refuse):
                report = artifact.smoke_test_updater_success(
                    package_root,
                    package_path,
                    artifact.sha256_file(package_path),
                    runtime_root,
                    process_elevated=True,
                )

            self.assertTrue(report["elevatedRefusal"])
            self.assertTrue(report["refusedBeforeMutation"])
            self.assertFalse(report["selfReplacement"])
            self.assertFalse(report["exactPackageInstalled"])
            self.assertEqual(artifact._transaction_leftovers(runtime_root), [])

            accepted_root = root / "elevated-accepted"
            accepted_root.mkdir()
            accepted = artifact.subprocess.CompletedProcess([], 0)
            with (
                mock.patch.object(artifact.subprocess, "run", return_value=accepted),
                self.assertRaisesRegex(
                    artifact.VerificationError,
                    "elevated packaged updater unexpectedly accepted",
                ),
            ):
                artifact.smoke_test_updater_success(
                    package_root,
                    package_path,
                    artifact.sha256_file(package_path),
                    accepted_root,
                    process_elevated=True,
                )

    def test_failure_report_binds_source_and_channel(self) -> None:
        report = artifact.failure_report(
            SOURCE_SHA.upper(),
            "dev",
            artifact.VerificationError("unsafe package"),
        )
        self.assertEqual(
            report,
            {
                "schemaVersion": 1,
                "status": "failed",
                "expectedSourceSha": SOURCE_SHA,
                "expectedChannel": "dev",
                "error": "unsafe package",
            },
        )


class SelectionAndChecksumTests(unittest.TestCase):
    def test_multiple_primary_archives_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            (
                builder.artifacts_dir
                / "sorotte-gui-9.9.9-windows-x86_64.zip"
            ).write_bytes(b"second")
            with self.assertRaisesRegex(artifact.VerificationError, "exactly one"):
                builder.verify()

    def test_checksum_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write(
                checksum_text=f"{'0' * 64}  {builder.archive_path.name}\n"
            )
            with self.assertRaisesRegex(artifact.VerificationError, "checksum mismatch"):
                builder.verify()

    def test_unexpected_uploaded_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            (builder.artifacts_dir / "unverified.txt").write_text(
                "extra",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                artifact.VerificationError,
                "artifact directory inventory mismatch",
            ):
                builder.verify()

    def test_symbols_archive_and_checksum_are_atomic_as_a_pair(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            symbols = builder.artifacts_dir / f"{builder.archive_path.stem}-symbols.zip"
            with zipfile.ZipFile(symbols, "w") as archive:
                archive.writestr("sorotte_gui.pdb", b"symbols")
            with self.assertRaisesRegex(
                artifact.VerificationError,
                "inventory mismatch|checksum",
            ):
                builder.verify()

    def test_expected_source_sha_and_channel_are_closed_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            with self.assertRaisesRegex(artifact.VerificationError, "exactly 40"):
                artifact.verify_release(
                    builder.artifacts_dir,
                    "not-a-sha",
                    "dev",
                    runtime_smoke=False,
                )
            with self.assertRaisesRegex(artifact.VerificationError, "expected channel"):
                artifact.verify_release(
                    builder.artifacts_dir,
                    SOURCE_SHA,
                    "nightly",
                    runtime_smoke=False,
                )


class ArchiveBoundaryTests(unittest.TestCase):
    def test_path_traversal_member_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            entries = builder.default_entries()
            entries[0] = ("../sorotte-gui.exe", entries[0][1], None)
            builder.write(entries=entries)
            with self.assertRaisesRegex(artifact.VerificationError, "not normalized"):
                builder.verify()

    def test_windows_separator_is_normalized_like_the_shipped_updater(self) -> None:
        # Python's Windows zip writer canonicalizes backslashes before emitting
        # an entry, so exercise the GUI-specific consumer primitive directly.
        self.assertEqual(
            artifact._canonical_gui_member_path(
                "resources\\sorotte_syncplayintf.lua",
                is_directory=False,
            ),
            "resources/sorotte_syncplayintf.lua",
        )

    def test_mixed_member_separators_are_rejected(self) -> None:
        with self.assertRaisesRegex(artifact.VerificationError, "mixes path separators"):
            artifact._canonical_gui_member_path(
                "resources/subdir\\sorotte_syncplayintf.lua",
                is_directory=False,
            )

    def test_duplicate_archive_member_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            entries = builder.default_entries()
            entries.append(entries[0])
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                builder.write(entries=entries)
            with self.assertRaisesRegex(
                artifact.VerificationError,
                "duplicate or case-colliding",
            ):
                builder.verify()

    def test_case_colliding_archive_members_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            entries = builder.default_entries()
            entries.append(("readme.md", b"collision", None))
            builder.write(entries=entries)
            with self.assertRaisesRegex(artifact.VerificationError, "case-colliding"):
                builder.verify()

    def test_missing_or_extra_archive_member_is_rejected(self) -> None:
        for mutation in ("missing", "extra"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                builder = GuiArtifactBuilder(Path(temporary))
                entries = builder.default_entries()
                if mutation == "missing":
                    entries = [entry for entry in entries if entry[0] != "LICENSE"]
                else:
                    entries.append(("unexpected.dll", b"extra", None))
                builder.write(entries=entries)
                with self.assertRaisesRegex(
                    artifact.VerificationError,
                    "archive inventory mismatch",
                ):
                    builder.verify()

    def test_empty_payload_is_rejected_before_manifest_trust(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.payloads["README.md"] = b""
            builder.write()
            with self.assertRaisesRegex(artifact.VerificationError, "must not be empty"):
                builder.verify()


class ExternalManifestTests(unittest.TestCase):
    def test_update_manifest_source_channel_and_package_are_cross_bound(self) -> None:
        cases = {
            "git_sha": ("f" * 40, "git_sha mismatch"),
            "channel": ("stable", "channel mismatch"),
            "package": ("another.zip", "package mismatch"),
            "sha256": ("0" * 64, "sha256 mismatch"),
            "version": ("9.9.9", "version mismatch"),
        }
        for field, (value, expected_error) in cases.items():
            with self.subTest(field=field), tempfile.TemporaryDirectory() as temporary:
                builder = GuiArtifactBuilder(Path(temporary))
                builder.write(update_overrides={field: value})
                with self.assertRaisesRegex(
                    artifact.VerificationError,
                    expected_error,
                ):
                    builder.verify()

    def test_update_manifest_unknown_or_missing_key_is_rejected(self) -> None:
        for mutation in ("extra", "missing"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                builder = GuiArtifactBuilder(Path(temporary))
                builder.write()
                path = builder.artifacts_dir / "sorotte-update-manifest.json"
                value = json.loads(path.read_text(encoding="utf-8"))
                if mutation == "extra":
                    value["download_url"] = "https://untrusted.example/package.zip"
                else:
                    value.pop("target")
                path.write_text(json.dumps(value), encoding="utf-8")
                with self.assertRaisesRegex(artifact.VerificationError, "keys mismatch"):
                    builder.verify()

    def test_update_manifest_duplicate_json_key_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            path = builder.artifacts_dir / "sorotte-update-manifest.json"
            body = path.read_text(encoding="utf-8")
            body = body.replace(
                '"app":"sorotte-gui"',
                '"app":"sorotte-gui","app":"shadow"',
                1,
            )
            path.write_text(body, encoding="utf-8")
            with self.assertRaisesRegex(artifact.VerificationError, "duplicate JSON key"):
                builder.verify()

    def test_update_manifest_timestamp_is_canonical_and_real(self) -> None:
        for value in (
            "2026-07-30T01:02:03+00:00",
            "2026-02-30T01:02:03Z",
            123,
        ):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as temporary:
                builder = GuiArtifactBuilder(Path(temporary))
                builder.write(update_overrides={"created_at_utc": value})
                with self.assertRaisesRegex(
                    artifact.VerificationError,
                    "UTC timestamp",
                ):
                    builder.verify()


class InstallManifestTests(unittest.TestCase):
    def test_install_manifest_metadata_must_match_external_manifest(self) -> None:
        cases = {
            "git_sha": "f" * 40,
            "channel": "stable",
            "version": "9.9.9",
            "created_at_utc": "2026-07-30T01:02:04Z",
            "target": "linux-x86_64",
        }
        for field, value in cases.items():
            with self.subTest(field=field), tempfile.TemporaryDirectory() as temporary:
                builder = GuiArtifactBuilder(Path(temporary))
                builder.write(
                    install_manifest=builder.install_manifest(**{field: value})
                )
                with self.assertRaisesRegex(
                    artifact.VerificationError,
                    f"{field} mismatch",
                ):
                    builder.verify()

    def test_install_manifest_digest_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            manifest = builder.install_manifest()
            manifest["files"][0]["sha256"] = "0" * 64
            builder.write(install_manifest=manifest)
            with self.assertRaisesRegex(
                artifact.VerificationError,
                "digest mismatch",
            ):
                builder.verify()

    def test_install_manifest_inventory_is_exact(self) -> None:
        for mutation in ("missing", "extra", "duplicate"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                builder = GuiArtifactBuilder(Path(temporary))
                manifest = builder.install_manifest()
                files = manifest["files"]
                assert isinstance(files, list)
                if mutation == "missing":
                    files.pop()
                elif mutation == "extra":
                    files.append({"path": "extra.dll", "sha256": "0" * 64})
                else:
                    files.append(dict(files[0]))
                builder.write(install_manifest=manifest)
                with self.assertRaisesRegex(
                    artifact.VerificationError,
                    "unexpected file entry|duplicate|inventory mismatch",
                ):
                    builder.verify()

    def test_install_manifest_unsafe_path_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            manifest = builder.install_manifest()
            manifest["files"][0]["path"] = "../sorotte-gui.exe"
            builder.write(install_manifest=manifest)
            with self.assertRaisesRegex(artifact.VerificationError, "not normalized"):
                builder.verify()

    def test_install_manifest_unknown_key_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            manifest = builder.install_manifest(provenance={"builder": "untrusted"})
            builder.write(install_manifest=manifest)
            with self.assertRaisesRegex(artifact.VerificationError, "keys mismatch"):
                builder.verify()

    def test_install_manifest_duplicate_json_key_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            body = builder.install_manifest_bytes().replace(
                b'"app":"sorotte-gui"',
                b'"app":"sorotte-gui","app":"shadow"',
                1,
            )
            builder.write(install_manifest=body)
            with self.assertRaisesRegex(artifact.VerificationError, "duplicate JSON key"):
                builder.verify()


class SymbolsBoundaryTests(unittest.TestCase):
    def test_symbols_inventory_is_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            builder.add_symbols({"unexpected.pdb": b"symbols"})
            with self.assertRaisesRegex(artifact.VerificationError, "unexpected file"):
                builder.verify()

    def test_empty_symbols_archive_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            builder.add_symbols({})
            with self.assertRaisesRegex(
                artifact.VerificationError,
                "at least one known PDB",
            ):
                builder.verify()

    def test_symbols_checksum_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            builder = GuiArtifactBuilder(Path(temporary))
            builder.write()
            symbols = builder.add_symbols({"sorotte_gui.pdb": b"symbols"})
            symbols.with_name(f"{symbols.name}.sha256").write_text(
                f"{'0' * 64}  {symbols.name}\n",
                encoding="ascii",
            )
            with self.assertRaisesRegex(artifact.VerificationError, "checksum mismatch"):
                builder.verify()


class WorkflowContractTests(unittest.TestCase):
    def test_packager_writes_both_manifests_as_bomless_utf8(self) -> None:
        packager = (REPO_ROOT / "scripts" / "package-gui-release.ps1").read_text(
            encoding="utf-8"
        )
        install_marker = packager[
            packager.index("$installMarker = [ordered]@{") :
            packager.index("$archivePath =", packager.index("$installMarker = [ordered]@{"))
        ]
        self.assertIn("Write-Utf8ArtifactFile", install_marker)
        self.assertNotIn("Set-Content", install_marker)
        self.assertIn("[System.Text.UTF8Encoding]::new($false)", packager)
        self.assertIn(
            "Write-Utf8ArtifactFile -Path $manifestPath",
            packager,
        )

    def test_gui_release_uses_immutable_action_revisions(self) -> None:
        workflow = (
            REPO_ROOT / ".github" / "workflows" / "sorotte-gui-release.yml"
        ).read_text(encoding="utf-8")
        self.assertNotIn("actions/checkout@v4", workflow)
        self.assertNotIn("actions/upload-artifact@v4", workflow)
        self.assertNotIn("actions/download-artifact@v4", workflow)
        self.assertNotIn("dtolnay/rust-toolchain@stable", workflow)
        for revision in (
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
            "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97",
            "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
            "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
            "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4",
        ):
            self.assertIn(revision, workflow)

    def test_exact_artifact_is_verified_before_upload_and_publication(self) -> None:
        workflow = (
            REPO_ROOT / ".github" / "workflows" / "sorotte-gui-release.yml"
        ).read_text(encoding="utf-8")
        package = workflow.index("- name: Package GUI release")
        consume = workflow.index("- name: Verify exact GUI release artifact")
        upload = workflow.index("- name: Upload GUI package artifact")
        download = workflow.index("- name: Download GUI package artifact")
        reconsume = workflow.index(
            "- name: Reverify downloaded GUI release artifact"
        )
        publish = workflow.index("- name: Attach package to GitHub Release")
        self.assertLess(package, consume)
        self.assertLess(consume, upload)
        self.assertLess(download, reconsume)
        self.assertLess(reconsume, publish)
        self.assertIn("--expected-source-sha \"$env:GITHUB_SHA\"", workflow)
        self.assertIn(
            "--expected-channel \"$env:SOROTTE_GUI_RELEASE_CHANNEL\"",
            workflow,
        )
        publication_block = workflow[reconsume:publish]
        self.assertIn("--skip-runtime-smoke", publication_block)

    def test_failure_reports_are_uploaded_but_unverified_packages_are_not(self) -> None:
        workflow = (
            REPO_ROOT / ".github" / "workflows" / "sorotte-gui-release.yml"
        ).read_text(encoding="utf-8")
        self.assertIn("- name: Upload GUI artifact verification report", workflow)
        self.assertIn("- name: Upload publication verification report", workflow)
        self.assertGreaterEqual(workflow.count("if: always()"), 2)
        upload_block = workflow[
            workflow.index("- name: Upload GUI package artifact") :
            workflow.index("  publish-release:")
        ]
        self.assertIn("path: target/gui-release/artifacts/*", upload_block)
        self.assertNotIn("if: always()", upload_block)

    def test_rolling_dev_tag_push_retains_its_scoped_checkout_credential(self) -> None:
        workflow = (
            REPO_ROOT / ".github" / "workflows" / "sorotte-gui-release.yml"
        ).read_text(encoding="utf-8")
        publication_checkout = workflow[
            workflow.index("- name: Checkout trusted publication revision") :
            workflow.index("- name: Setup Python", workflow.index("  publish-release:"))
        ]
        self.assertNotIn("persist-credentials: false", publication_checkout)
        publication_job = workflow[workflow.index("  publish-release:") :]
        self.assertIn("contents: write", publication_job)
        self.assertIn('git push origin "refs/tags/$tag" --force', publication_job)


if __name__ == "__main__":
    unittest.main()
