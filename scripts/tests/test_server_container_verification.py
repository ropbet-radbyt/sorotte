from __future__ import annotations

import base64
import hashlib
import json
import pathlib
import sqlite3
import subprocess
import tempfile
import unittest
import urllib.error
from unittest import mock

import yaml

from scripts import verify_server_container as container


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "publish-server-container.yml"
DOCKERFILE_PATH = REPO_ROOT / "Dockerfile.server"
SOURCE_SHA = "a" * 40
WORKFLOW_SHA = "b" * 40
LOCAL_IMAGE_ID = f"sha256:{'c' * 64}"
MANIFEST_DIGEST = f"sha256:{'d' * 64}"
SBOM_DIGEST = f"sha256:{'e' * 64}"
IMAGE_NAME = "ghcr.io/ropbet-radbyt/sorotte-server"
TEST_IMAGE = f"sorotte-server:test-{SOURCE_SHA}"
SOURCE_URL = "https://github.com/ropbet-radbyt/sorotte"
WORKFLOW_IDENTITY = (
    "https://github.com/ropbet-radbyt/sorotte/"
    ".github/workflows/publish-server-container.yml@refs/tags/v0.2.3"
)


def write_json(path: pathlib.Path, value: object) -> None:
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


def valid_local_inspection() -> list[dict[str, object]]:
    return [
        {
            "Id": LOCAL_IMAGE_ID,
            "Os": "linux",
            "Architecture": "amd64",
            "Config": {
                "User": "sorotte",
                "Entrypoint": ["sorotte-server"],
                "Cmd": container.EXPECTED_DEFAULT_COMMAND,
                "Labels": {
                    container.EXPECTED_SOURCE_LABEL: SOURCE_URL,
                    container.EXPECTED_REVISION_LABEL: SOURCE_SHA,
                    container.EXPECTED_CREATED_LABEL: "2026-07-31T00:00:00Z",
                    container.EXPECTED_LICENSE_LABEL: "Apache-2.0",
                },
            },
            "RootFS": {
                "Type": "layers",
                "Layers": [f"sha256:{'1' * 64}", f"sha256:{'2' * 64}"],
            },
        }
    ]


def valid_runtime_report() -> dict[str, object]:
    def scenario(name: str, *, tls: bool) -> dict[str, object]:
        accepted = {
            "playlist": ["container-alpha.mkv", "container-beta.mkv"],
            "playlistIndex": 1,
            "playstate": {"paused": True, "position": 137.25},
        }

        def raw_row(version: int) -> dict[str, object]:
            return {
                "integrityCheck": "ok",
                "row": {
                    "createdAt": 1_785_430_800.0,
                    "lastSavedUpdate": 1_785_430_801.0,
                    "name": "container-persisted-room",
                    "ownerBucket": f"quota:v1:{'9' * 64}",
                    "persistenceVersion": version,
                    "playlist": "container-alpha.mkv\ncontainer-beta.mkv",
                    "playlistIndex": 1,
                    "playlistJson": (
                        '["container-alpha.mkv","container-beta.mkv"]'
                    ),
                    "position": 137.25,
                },
            }

        return {
            "clientSessionDrained": True,
            "containerId": "3" * 64,
            "databases": [
                {
                    "integrityCheck": "ok",
                    "path": "rooms.sqlite3",
                    "sha256": f"sha256:{'4' * 64}",
                    "size": 4096,
                },
                {
                    "integrityCheck": "ok",
                    "path": "stats.sqlite3",
                    "sha256": f"sha256:{'5' * 64}",
                    "size": 4096,
                },
            ],
            "log": f"{name}.log",
            "persistence": (
                None
                if tls
                else {
                    "accepted": accepted,
                    "rawAfterRestart": raw_row(5),
                    "rawAfterWrite": raw_row(4),
                    "restored": json.loads(json.dumps(accepted)),
                    "room": "container-persisted-room",
                    "sameLoadedImage": True,
                    "sameStateDirectory": True,
                }
            ),
            "protocolHello": True,
            "restart": (
                None
                if tls
                else {
                    "clientSessionDrained": True,
                    "containerId": "8" * 64,
                    "log": f"{name}-restart.log",
                    "protocolHello": True,
                    "shutdown": {
                        "error": "",
                        "exitCode": 0,
                        "oomKilled": False,
                        "signal": "SIGINT",
                    },
                }
            ),
            "scenario": name,
            "shutdown": {
                "error": "",
                "exitCode": 0,
                "oomKilled": False,
                "signal": "SIGINT",
            },
            "tls": (
                {
                    "cipher": "TLS_AES_256_GCM_SHA384",
                    "peerCertificateSha256": f"sha256:{'6' * 64}",
                    "version": "TLSv1.3",
                }
                if tls
                else None
            ),
        }

    return {
        "image": TEST_IMAGE,
        "localImage": {
            "architecture": "amd64",
            "created": "2026-07-31T00:00:00Z",
            "entrypoint": ["sorotte-server"],
            "id": LOCAL_IMAGE_ID,
            "os": "linux",
            "rootfsDiffIds": [f"sha256:{'1' * 64}"],
            "source": SOURCE_URL,
            "sourceSha": SOURCE_SHA,
            "user": "sorotte",
        },
        "schemaVersion": 1,
        "scenarios": [
            scenario("plaintext-persistence", tls=False),
            scenario("tls-persistence", tls=True),
        ],
        "status": "passed",
    }


def valid_sbom() -> dict[str, object]:
    return {
        "SPDXID": "SPDXRef-DOCUMENT",
        "creationInfo": {
            "created": "2026-07-31T00:00:00Z",
            "creators": ["Tool: syft-1.44.0"],
        },
        "dataLicense": "CC0-1.0",
        "documentNamespace": "https://example.invalid/sbom",
        "name": "sorotte-server",
        "packages": [{"SPDXID": "SPDXRef-Package", "name": "sorotte-server"}],
        "spdxVersion": "SPDX-2.3",
    }


def valid_sbom_report(*, sbom_digest: str = SBOM_DIGEST) -> dict[str, object]:
    return {
        "bindingMode": "pinned-syft-input-plus-daemon-reinspection",
        "image": TEST_IMAGE,
        "localImageId": LOCAL_IMAGE_ID,
        "packageCount": 1,
        "rootfsDiffIds": [f"sha256:{'1' * 64}"],
        "sbomSha256": sbom_digest,
        "schemaVersion": 1,
        "source": SOURCE_URL,
        "sourceSha": SOURCE_SHA,
        "spdxVersion": "SPDX-2.3",
        "status": "passed",
    }


def valid_publish_report() -> dict[str, object]:
    tag = f"{IMAGE_NAME}:sha-{SOURCE_SHA}"
    return {
        "digest": MANIFEST_DIGEST,
        "image": IMAGE_NAME,
        "localImageId": LOCAL_IMAGE_ID,
        "pushes": [{"digest": MANIFEST_DIGEST, "tag": tag}],
        "schemaVersion": 1,
        "source": SOURCE_URL,
        "sourceSha": SOURCE_SHA,
        "status": "passed",
        "tags": [tag],
    }


def valid_signature_output() -> list[dict[str, object]]:
    return [
        {
            "critical": {
                "identity": {"docker-reference": IMAGE_NAME},
                "image": {"docker-manifest-digest": MANIFEST_DIGEST},
                "type": "cosign container image signature",
            },
            "optional": {
                "sourceSha": SOURCE_SHA,
                "workflowSourceSha": WORKFLOW_SHA,
            },
        }
    ]


def valid_attestation_output() -> dict[str, object]:
    statement = {
        "_type": "https://in-toto.io/Statement/v0.1",
        "predicate": valid_sbom(),
        "predicateType": "https://spdx.dev/Document",
        "subject": [
            {
                "digest": {"sha256": MANIFEST_DIGEST.removeprefix("sha256:")},
                "name": IMAGE_NAME,
            }
        ],
    }
    return {
        "payloadType": "application/vnd.in-toto+json",
        "payload": base64.b64encode(
            json.dumps(statement, separators=(",", ":")).encode()
        ).decode(),
        "signatures": [{"sig": "test"}],
    }


def valid_publication_report(*, sbom_digest: str = SBOM_DIGEST) -> dict[str, object]:
    return {
        "attestations": 1,
        "digest": MANIFEST_DIGEST,
        "image": IMAGE_NAME,
        "localImageId": LOCAL_IMAGE_ID,
        "publicConfig": {
            "configDigest": LOCAL_IMAGE_ID,
            "layers": [{"digest": f"sha256:{'7' * 64}", "size": 1024}],
            "rootfsDiffIds": [f"sha256:{'1' * 64}"],
        },
        "publicReferences": [
            {
                "digest": MANIFEST_DIGEST,
                "reference": f"{IMAGE_NAME}:sha-{SOURCE_SHA}",
            },
            {
                "digest": MANIFEST_DIGEST,
                "reference": f"{IMAGE_NAME}@{MANIFEST_DIGEST}",
            },
        ],
        "sbomSha256": sbom_digest,
        "schemaVersion": 1,
        "signatures": 1,
        "sourceSha": SOURCE_SHA,
        "status": "passed",
        "verificationPolicy": {
            "certificateGithubWorkflowSha": SOURCE_SHA,
            "certificateIdentity": WORKFLOW_IDENTITY,
            "certificateIssuer": "https://token.actions.githubusercontent.com",
            "workflowSourceSha": WORKFLOW_SHA,
        },
    }


class JsonAndIdentityPolicyTests(unittest.TestCase):
    def test_duplicate_json_keys_fail_closed(self) -> None:
        with self.assertRaisesRegex(container.VerificationError, "duplicate JSON key"):
            container._load_json_bytes(b'{"status":"passed","status":"failed"}', "report")

    def test_image_and_source_identities_are_canonical(self) -> None:
        self.assertEqual(container._validate_image_name(IMAGE_NAME), IMAGE_NAME)
        self.assertEqual(container._validate_source_url(SOURCE_URL), SOURCE_URL)
        self.assertEqual(
            container._validate_publication_scope(IMAGE_NAME, SOURCE_URL),
            ("ropbet-radbyt", "sorotte"),
        )
        with self.assertRaisesRegex(container.VerificationError, "source owner's"):
            container._validate_publication_scope(
                "ghcr.io/other-owner/sorotte-server", SOURCE_URL
            )
        for image in [
            "docker.io/owner/image",
            "ghcr.io/Owner/image",
            "ghcr.io/owner/image:latest",
            "ghcr.io/owner/../image",
        ]:
            with self.subTest(image=image), self.assertRaises(container.VerificationError):
                container._validate_image_name(image)
        for source in [
            "http://github.com/owner/repo",
            "https://evil.example/owner/repo",
            "https://github.com/owner/repo?ref=main",
        ]:
            with self.subTest(source=source), self.assertRaises(container.VerificationError):
                container._validate_source_url(source)

    def test_tag_inventory_requires_exact_full_source_sha_tag(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "tags.txt"
            path.write_bytes(
                f"{IMAGE_NAME}:sha-{SOURCE_SHA}\n{IMAGE_NAME}:v0.2.3\n".encode(
                    "ascii"
                )
            )
            self.assertEqual(
                container.read_publication_tags(
                    path,
                    expected_image=IMAGE_NAME,
                    expected_source_sha=SOURCE_SHA,
                ),
                [f"{IMAGE_NAME}:sha-{SOURCE_SHA}", f"{IMAGE_NAME}:v0.2.3"],
            )
            path.write_bytes(f"{IMAGE_NAME}:sha-{SOURCE_SHA[:12]}\n".encode("ascii"))
            with self.assertRaisesRegex(container.VerificationError, "missing"):
                container.read_publication_tags(
                    path,
                    expected_image=IMAGE_NAME,
                    expected_source_sha=SOURCE_SHA,
                )

    def test_tag_inventory_rejects_duplicates_foreign_images_and_noncanonical_lines(
        self,
    ) -> None:
        variants = [
            f"{IMAGE_NAME}:sha-{SOURCE_SHA}\n{IMAGE_NAME}:sha-{SOURCE_SHA}\n",
            f"ghcr.io/other/project:sha-{SOURCE_SHA}\n",
            f"{IMAGE_NAME}:sha-{SOURCE_SHA}\r\n",
            f" {IMAGE_NAME}:sha-{SOURCE_SHA}\n",
        ]
        for value in variants:
            with self.subTest(value=value), tempfile.TemporaryDirectory() as temporary:
                path = pathlib.Path(temporary) / "tags.txt"
                path.write_bytes(value.encode("ascii"))
                with self.assertRaises(container.VerificationError):
                    container.read_publication_tags(
                        path,
                        expected_image=IMAGE_NAME,
                        expected_source_sha=SOURCE_SHA,
                    )


class LocalImageConsumerTests(unittest.TestCase):
    def test_container_log_capture_waits_for_delayed_shutdown_marker(self) -> None:
        marker = "sorotte-server: shutdown requested; draining client sessions\n"
        log_results = [
            subprocess.CompletedProcess(
                ["docker", "logs"], 0, stdout="sorotte-server listening\n"
            ),
            subprocess.CompletedProcess(
                ["docker", "logs"],
                0,
                stdout=f"sorotte-server listening\n{marker}",
            ),
            subprocess.CompletedProcess(["docker", "rm"], 0, stdout=""),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "container.log"
            with (
                mock.patch.object(container, "_run", side_effect=log_results) as run,
                mock.patch.object(container.time, "sleep") as sleep,
            ):
                container._write_container_log_and_remove("container-name", path)

            self.assertEqual(
                path.read_text(encoding="utf-8"),
                f"sorotte-server listening\n{marker}",
            )
            self.assertEqual(
                run.call_args_list,
                [
                    mock.call(["docker", "logs", "container-name"], check=False),
                    mock.call(["docker", "logs", "container-name"], check=False),
                    mock.call(
                        ["docker", "rm", "--force", "container-name"], check=False
                    ),
                ],
            )
            sleep.assert_called_once_with(container.CONTAINER_LOG_CAPTURE_RETRY_SECONDS)

    def test_container_log_capture_fails_bounded_and_still_removes_container(
        self,
    ) -> None:
        outputs = [
            subprocess.CompletedProcess(
                ["docker", "logs"], 0, stdout=f"snapshot-{index}\n"
            )
            for index in range(3)
        ]
        outputs.append(subprocess.CompletedProcess(["docker", "rm"], 0, stdout=""))
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "container.log"
            with (
                mock.patch.object(container, "CONTAINER_LOG_CAPTURE_ATTEMPTS", 3),
                mock.patch.object(container, "_run", side_effect=outputs) as run,
                mock.patch.object(container.time, "sleep") as sleep,
            ):
                with self.assertRaisesRegex(
                    container.VerificationError,
                    "did not log the graceful shutdown barrier",
                ):
                    container._write_container_log_and_remove("container-name", path)

            self.assertEqual(path.read_text(encoding="utf-8"), "snapshot-2\n")
            self.assertEqual(run.call_count, 4)
            self.assertEqual(
                run.call_args_list[-1],
                mock.call(
                    ["docker", "rm", "--force", "container-name"], check=False
                ),
            )
            self.assertEqual(
                sleep.call_args_list,
                [
                    mock.call(container.CONTAINER_LOG_CAPTURE_RETRY_SECONDS),
                    mock.call(container.CONTAINER_LOG_CAPTURE_RETRY_SECONDS),
                ],
            )

    def test_local_image_inspection_binds_config_digest_labels_entrypoint_and_layers(
        self,
    ) -> None:
        with mock.patch.object(
            container, "_docker_json", return_value=valid_local_inspection()
        ):
            evidence = container.inspect_local_image(
                TEST_IMAGE,
                expected_source_sha=SOURCE_SHA,
                expected_source_url=SOURCE_URL,
            )
        self.assertEqual(evidence["id"], LOCAL_IMAGE_ID)
        self.assertEqual(evidence["sourceSha"], SOURCE_SHA)
        self.assertEqual(len(evidence["rootfsDiffIds"]), 2)

    def test_local_image_inspection_rejects_identity_and_runtime_drift(self) -> None:
        variants = [
            (("Id",), "sha256:short"),
            (("Architecture",), "arm64"),
            (("Config", "User"), "root"),
            (("Config", "Entrypoint"), ["/bin/sh"]),
            (("Config", "Cmd"), ["--help"]),
            (
                ("Config", "Labels", container.EXPECTED_REVISION_LABEL),
                "f" * 40,
            ),
            (("RootFS", "Layers"), []),
        ]
        for path, value in variants:
            with self.subTest(path=path):
                inspection = valid_local_inspection()
                target: object = inspection[0]
                for key in path[:-1]:
                    target = target[key]  # type: ignore[index]
                target[path[-1]] = value  # type: ignore[index]
                with mock.patch.object(container, "_docker_json", return_value=inspection):
                    with self.assertRaises(container.VerificationError):
                        container.inspect_local_image(
                            TEST_IMAGE,
                            expected_source_sha=SOURCE_SHA,
                            expected_source_url=SOURCE_URL,
                        )

    def test_runtime_report_schema_is_closed_and_requires_both_real_boundaries(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            path = root / "runtime.json"
            container._validate_artifacts_root(
                root, root / "target" / "server-container-verification"
            )
            with self.assertRaisesRegex(container.VerificationError, "under"):
                container._validate_artifacts_root(root, root / "outside")
            with self.assertRaisesRegex(container.VerificationError, "dedicated child"):
                container._validate_artifacts_root(root, root / "target")
            write_json(path, valid_runtime_report())
            self.assertEqual(container.parse_runtime_report(path)["status"], "passed")
            drift = valid_runtime_report()
            drift["skipped"] = True
            write_json(path, drift)
            with self.assertRaisesRegex(container.VerificationError, "keys mismatch"):
                container.parse_runtime_report(path)
            drift = valid_runtime_report()
            drift["scenarios"] = [drift["scenarios"][0]]  # type: ignore[index]
            write_json(path, drift)
            with self.assertRaisesRegex(container.VerificationError, "required scenarios"):
                container.parse_runtime_report(path)
            drift = valid_runtime_report()
            drift["scenarios"].append(drift["scenarios"][0])  # type: ignore[union-attr,index]
            write_json(path, drift)
            with self.assertRaisesRegex(container.VerificationError, "required scenarios"):
                container.parse_runtime_report(path)
            drift = valid_runtime_report()
            plaintext = drift["scenarios"][0]  # type: ignore[index]
            plaintext["persistence"]["restored"]["playlistIndex"] = 0  # type: ignore[index]
            write_json(path, drift)
            with self.assertRaisesRegex(container.VerificationError, "exactly equal"):
                container.parse_runtime_report(path)
            drift = valid_runtime_report()
            plaintext = drift["scenarios"][0]  # type: ignore[index]
            raw_restart = plaintext["persistence"]["rawAfterRestart"]  # type: ignore[index]
            raw_restart["row"]["position"] = 0.0  # type: ignore[index]
            write_json(path, drift)
            with self.assertRaisesRegex(container.VerificationError, "raw position"):
                container.parse_runtime_report(path)

    def test_raw_persisted_room_row_requires_exact_payload_and_integrity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "rooms.sqlite3"
            connection = sqlite3.connect(path)
            connection.execute(
                "CREATE TABLE persistent_rooms ("
                "name TEXT PRIMARY KEY, playlist TEXT, playlistJson TEXT, "
                "playlistIndex INTEGER, position REAL, lastSavedUpdate REAL, "
                "persistenceVersion INTEGER, ownerBucket TEXT, createdAt REAL)"
            )
            connection.execute(
                "INSERT INTO persistent_rooms VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    "container-persisted-room",
                    "container-alpha.mkv\ncontainer-beta.mkv",
                    '["container-alpha.mkv","container-beta.mkv"]',
                    1,
                    137.25,
                    1_785_430_801.0,
                    4,
                    f"quota:v1:{'9' * 64}",
                    1_785_430_800.0,
                ),
            )
            connection.commit()
            connection.close()

            evidence = container._verify_persisted_room_row(
                path,
                room="container-persisted-room",
                playlist=["container-alpha.mkv", "container-beta.mkv"],
                playlist_index=1,
                position=137.25,
            )
            self.assertEqual(evidence["integrityCheck"], "ok")
            self.assertEqual(evidence["row"]["persistenceVersion"], 4)

            connection = sqlite3.connect(path)
            connection.execute(
                "UPDATE persistent_rooms SET position = 0.0 "
                "WHERE name = 'container-persisted-room'"
            )
            connection.commit()
            connection.close()
            with self.assertRaisesRegex(container.VerificationError, "payload mismatch"):
                container._verify_persisted_room_row(
                    path,
                    room="container-persisted-room",
                    playlist=["container-alpha.mkv", "container-beta.mkv"],
                    playlist_index=1,
                    position=137.25,
                )


class SbomPolicyTests(unittest.TestCase):
    def test_sbom_is_duplicate_free_spdx_from_syft_and_binds_runtime_image_id(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            sbom_path = root / "sbom.json"
            runtime_path = root / "runtime.json"
            write_json(sbom_path, valid_sbom())
            runtime = valid_runtime_report()
            write_json(runtime_path, runtime)
            with mock.patch.object(
                container,
                "inspect_local_image",
                return_value=runtime["localImage"],
            ) as inspect:
                report = container.verify_sbom(
                    sbom_path=sbom_path, runtime_report_path=runtime_path
                )
            inspect.assert_called_once_with(
                TEST_IMAGE,
                expected_source_sha=SOURCE_SHA,
                expected_source_url=SOURCE_URL,
            )
            self.assertEqual(report["localImageId"], LOCAL_IMAGE_ID)
            self.assertEqual(report["packageCount"], 1)
            self.assertEqual(
                report["rootfsDiffIds"], [f"sha256:{'1' * 64}"]
            )
            self.assertEqual(report["sbomSha256"], container._sha256_file(sbom_path))

    def test_sbom_requires_syft_packages_and_canonical_spdx_license(self) -> None:
        variants = [
            ("packages", []),
            ("dataLicense", "MIT"),
            ("spdxVersion", "CycloneDX-1.6"),
            ("creationInfo", {"creators": ["Tool: unknown"]}),
            ("creationInfo", {"creators": ["Tool: syft-1.43.0"]}),
        ]
        for key, value in variants:
            with self.subTest(key=key), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                sbom = valid_sbom()
                sbom[key] = value
                write_json(root / "sbom.json", sbom)
                runtime = valid_runtime_report()
                write_json(root / "runtime.json", runtime)
                with mock.patch.object(
                    container,
                    "inspect_local_image",
                    return_value=runtime["localImage"],
                ):
                    with self.assertRaises(container.VerificationError):
                        container.verify_sbom(
                            sbom_path=root / "sbom.json",
                            runtime_report_path=root / "runtime.json",
                        )

    def test_sbom_reinspection_rejects_substituted_local_image_identity(self) -> None:
        variants = [
            ("id", f"sha256:{'f' * 64}"),
            ("rootfsDiffIds", [f"sha256:{'0' * 64}"]),
            ("source", "https://github.com/ropbet-radbyt/substituted"),
        ]
        for key, value in variants:
            with self.subTest(key=key), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                runtime = valid_runtime_report()
                write_json(root / "runtime.json", runtime)
                write_json(root / "sbom.json", valid_sbom())
                substituted = dict(runtime["localImage"])  # type: ignore[arg-type]
                substituted[key] = value
                with mock.patch.object(
                    container, "inspect_local_image", return_value=substituted
                ):
                    with self.assertRaisesRegex(container.VerificationError, "changed"):
                        container.verify_sbom(
                            sbom_path=root / "sbom.json",
                            runtime_report_path=root / "runtime.json",
                        )


class PublishPolicyTests(unittest.TestCase):
    def _tags(self, root: pathlib.Path) -> pathlib.Path:
        path = root / "tags.txt"
        path.write_bytes(
            f"{IMAGE_NAME}:sha-{SOURCE_SHA}\n{IMAGE_NAME}:v0.2.3\n".encode("ascii")
        )
        return path

    def test_publish_tags_only_the_inspected_loaded_image_and_requires_one_digest(
        self,
    ) -> None:
        commands: list[list[str]] = []

        def run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            commands.append(list(command))
            output = (
                f"pushed\nlatest: digest: {MANIFEST_DIGEST} size: 1234\n"
                if command[:3] == ["docker", "image", "push"]
                else ""
            )
            return subprocess.CompletedProcess(command, 0, output, "")

        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            with mock.patch.object(
                container,
                "inspect_local_image",
                return_value={"id": LOCAL_IMAGE_ID},
            ), mock.patch.object(container, "_run", side_effect=run):
                report = container.publish_tested_image(
                    local_image=TEST_IMAGE,
                    image_name=IMAGE_NAME,
                    tags_path=self._tags(root),
                    expected_source_sha=SOURCE_SHA,
                    expected_source_url=SOURCE_URL,
                )
        self.assertEqual(report["digest"], MANIFEST_DIGEST)
        self.assertEqual(report["localImageId"], LOCAL_IMAGE_ID)
        self.assertEqual(len(report["pushes"]), 2)
        self.assertFalse(any("build" in command for command in commands))
        self.assertEqual(
            [command[:3] for command in commands],
            [
                ["docker", "image", "tag"],
                ["docker", "image", "push"],
                ["docker", "image", "tag"],
                ["docker", "image", "push"],
            ],
        )

    def test_publish_rejects_tag_digest_divergence(self) -> None:
        observed = iter([MANIFEST_DIGEST, f"sha256:{'f' * 64}"])

        def run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            if command[:3] == ["docker", "image", "push"]:
                return subprocess.CompletedProcess(
                    command, 0, f"digest: {next(observed)} size: 1\n", ""
                )
            return subprocess.CompletedProcess(command, 0, "", "")

        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            with mock.patch.object(
                container, "inspect_local_image", return_value={"id": LOCAL_IMAGE_ID}
            ), mock.patch.object(container, "_run", side_effect=run):
                with self.assertRaisesRegex(container.VerificationError, "diverged"):
                    container.publish_tested_image(
                        local_image=TEST_IMAGE,
                        image_name=IMAGE_NAME,
                        tags_path=self._tags(root),
                        expected_source_sha=SOURCE_SHA,
                        expected_source_url=SOURCE_URL,
                    )

    def test_publish_requires_docker_push_to_report_exactly_one_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            with mock.patch.object(
                container, "inspect_local_image", return_value={"id": LOCAL_IMAGE_ID}
            ), mock.patch.object(
                container,
                "_run",
                return_value=subprocess.CompletedProcess([], 0, "no digest\n", ""),
            ):
                with self.assertRaisesRegex(container.VerificationError, "exactly one"):
                    container.publish_tested_image(
                        local_image=TEST_IMAGE,
                        image_name=IMAGE_NAME,
                        tags_path=self._tags(root),
                        expected_source_sha=SOURCE_SHA,
                        expected_source_url=SOURCE_URL,
                    )


class RegistryIdentityTests(unittest.TestCase):
    def test_public_manifest_retries_bounded_eventual_consistency_and_rehashes_bytes(
        self,
    ) -> None:
        manifest = {
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {},
            "layers": [],
        }
        payload = json.dumps(manifest, separators=(",", ":")).encode()
        digest = f"sha256:{hashlib.sha256(payload).hexdigest()}"
        error = urllib.error.HTTPError(
            "https://ghcr.io/test", 404, "not found", {}, None
        )
        responses = iter(
            [
                error,
                (payload, {"docker-content-digest": digest}),
            ]
        )
        sleeps: list[float] = []

        def request(*_args: object, **_kwargs: object) -> tuple[bytes, dict[str, str]]:
            response = next(responses)
            if isinstance(response, Exception):
                raise response
            return response

        with mock.patch.object(container, "_registry_get", side_effect=request):
            parsed, observed, raw = container._fetch_public_manifest(
                "owner/repo", "v1", token="anonymous", sleep=sleeps.append
            )
        self.assertEqual(parsed, manifest)
        self.assertEqual(observed, digest)
        self.assertEqual(raw, payload)
        self.assertEqual(sleeps, [container.REGISTRY_RETRY_BASE_SECONDS])

    def test_public_manifest_rejects_header_body_digest_mismatch(self) -> None:
        payload = json.dumps(
            {
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
            }
        ).encode()
        with mock.patch.object(
            container,
            "_registry_get",
            return_value=(payload, {"docker-content-digest": MANIFEST_DIGEST}),
        ), self.assertRaisesRegex(container.VerificationError, "did not converge"):
            container._fetch_public_manifest(
                "owner/repo", "v1", token="anonymous", sleep=lambda _seconds: None
            )

    def test_public_config_cross_binds_local_config_digest_labels_and_layer_inventory(
        self,
    ) -> None:
        config_document = {
            "architecture": "amd64",
            "config": {
                "Cmd": container.EXPECTED_DEFAULT_COMMAND,
                "Entrypoint": ["sorotte-server"],
                "Labels": {
                    container.EXPECTED_SOURCE_LABEL: SOURCE_URL,
                    container.EXPECTED_REVISION_LABEL: SOURCE_SHA,
                },
                "User": "sorotte",
            },
            "os": "linux",
            "rootfs": {"diff_ids": [f"sha256:{'1' * 64}"], "type": "layers"},
        }
        payload = json.dumps(config_document, separators=(",", ":")).encode()
        config_digest = f"sha256:{hashlib.sha256(payload).hexdigest()}"
        manifest = {
            "config": {
                "digest": config_digest,
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "size": len(payload),
            },
            "layers": [
                {
                    "digest": f"sha256:{'2' * 64}",
                    "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                    "size": 1024,
                }
            ],
        }
        with mock.patch.object(
            container, "_registry_get", return_value=(payload, {})
        ):
            evidence = container._verify_public_config(
                "owner/repo",
                manifest,
                token="anonymous",
                expected_local_image_id=config_digest,
                expected_source_sha=SOURCE_SHA,
                expected_source_url=SOURCE_URL,
            )
        self.assertEqual(evidence["configDigest"], config_digest)
        self.assertEqual(len(evidence["layers"]), 1)
        with mock.patch.object(container, "_registry_get", return_value=(payload, {})):
            with self.assertRaisesRegex(container.VerificationError, "tested local image ID"):
                container._verify_public_config(
                    "owner/repo",
                    manifest,
                    token="anonymous",
                    expected_local_image_id=LOCAL_IMAGE_ID,
                    expected_source_sha=SOURCE_SHA,
                    expected_source_url=SOURCE_URL,
                )


class CosignEvidenceTests(unittest.TestCase):
    def test_signature_output_binds_digest_repository_and_workflow_annotations(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "signature.json"
            write_json(path, valid_signature_output())
            self.assertEqual(
                container.verify_cosign_signature_output(
                    path,
                    expected_image=IMAGE_NAME,
                    expected_digest=MANIFEST_DIGEST,
                    expected_annotations={
                        "sourceSha": SOURCE_SHA,
                        "workflowSourceSha": WORKFLOW_SHA,
                    },
                ),
                1,
            )
            drift = valid_signature_output()
            drift[0]["optional"]["workflowSourceSha"] = "f" * 40  # type: ignore[index]
            write_json(path, drift)
            with self.assertRaises(container.VerificationError):
                container.verify_cosign_signature_output(
                    path,
                    expected_image=IMAGE_NAME,
                    expected_digest=MANIFEST_DIGEST,
                    expected_annotations={
                        "sourceSha": SOURCE_SHA,
                        "workflowSourceSha": WORKFLOW_SHA,
                    },
                )

    def test_attestation_output_binds_spdx_predicate_subject_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            path = root / "attestation.json"
            predicate_path = root / "sbom.json"
            write_json(path, valid_attestation_output())
            write_json(predicate_path, valid_sbom())
            self.assertEqual(
                container.verify_cosign_attestation_output(
                    path,
                    expected_digest=MANIFEST_DIGEST,
                    expected_image=IMAGE_NAME,
                    expected_predicate_path=predicate_path,
                ),
                1,
            )
            with self.assertRaises(container.VerificationError):
                container.verify_cosign_attestation_output(
                    path, expected_digest=f"sha256:{'f' * 64}"
                )
            drift = valid_sbom()
            drift["name"] = "different-image"
            write_json(predicate_path, drift)
            with self.assertRaises(container.VerificationError):
                container.verify_cosign_attestation_output(
                    path,
                    expected_digest=MANIFEST_DIGEST,
                    expected_image=IMAGE_NAME,
                    expected_predicate_path=predicate_path,
                )

    def test_cosign_json_stream_rejects_duplicates_and_empty_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "output.json"
            path.write_text("", encoding="utf-8")
            with self.assertRaisesRegex(container.VerificationError, "must not be empty"):
                container._decode_json_stream(path, "cosign")
            path.write_text('{"payload":"a","payload":"b"}', encoding="utf-8")
            with self.assertRaisesRegex(container.VerificationError, "duplicate JSON key"):
                container._decode_json_stream(path, "cosign")


class PublicationAndFinalGateTests(unittest.TestCase):
    def _write_common(self, root: pathlib.Path) -> dict[str, pathlib.Path]:
        paths = {
            "runtime": root / "runtime.json",
            "sbom": root / "sbom.json",
            "sbom_report": root / "sbom-report.json",
            "publish": root / "publish.json",
            "signature": root / "signature.json",
            "attestation": root / "attestation.json",
            "public": root / "public.json",
        }
        write_json(paths["runtime"], valid_runtime_report())
        write_json(paths["sbom"], valid_sbom())
        actual_sbom_digest = container._sha256_file(paths["sbom"])
        write_json(paths["sbom_report"], valid_sbom_report(sbom_digest=actual_sbom_digest))
        write_json(paths["publish"], valid_publish_report())
        write_json(paths["signature"], valid_signature_output())
        write_json(paths["attestation"], valid_attestation_output())
        write_json(
            paths["public"],
            valid_publication_report(sbom_digest=actual_sbom_digest),
        )
        return paths

    def test_publication_verification_checks_every_tag_and_digest_anonymously(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            paths = self._write_common(root)
            manifest = {"schemaVersion": 2}
            with mock.patch.object(
                container, "_anonymous_ghcr_token", return_value="anonymous"
            ) as token, mock.patch.object(
                container,
                "_fetch_public_manifest",
                return_value=(manifest, MANIFEST_DIGEST, b"same-manifest"),
            ) as fetch, mock.patch.object(
                container,
                "_verify_public_config",
                return_value={
                    "configDigest": LOCAL_IMAGE_ID,
                    "layers": [{"digest": f"sha256:{'7' * 64}", "size": 1024}],
                    "rootfsDiffIds": [f"sha256:{'1' * 64}"],
                },
            ):
                report = container.verify_publication(
                    publish_report_path=paths["publish"],
                    sbom_path=paths["sbom"],
                    sbom_report_path=paths["sbom_report"],
                    signature_path=paths["signature"],
                    attestation_path=paths["attestation"],
                    expected_workflow_identity=WORKFLOW_IDENTITY,
                    expected_workflow_sha=WORKFLOW_SHA,
                    sleep=lambda _seconds: None,
                )
        token.assert_called_once_with("ropbet-radbyt/sorotte-server")
        self.assertEqual(fetch.call_count, 2)
        self.assertEqual(len(report["publicReferences"]), 2)
        self.assertEqual(
            report["verificationPolicy"]["certificateGithubWorkflowSha"], SOURCE_SHA
        )

    def test_publication_verification_rejects_tag_digest_or_manifest_divergence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            paths = self._write_common(root)
            with mock.patch.object(
                container, "_anonymous_ghcr_token", return_value="anonymous"
            ), mock.patch.object(
                container,
                "_fetch_public_manifest",
                return_value=({}, f"sha256:{'f' * 64}", b"wrong"),
            ):
                with self.assertRaisesRegex(container.VerificationError, "resolved to"):
                    container.verify_publication(
                        publish_report_path=paths["publish"],
                        sbom_path=paths["sbom"],
                        sbom_report_path=paths["sbom_report"],
                        signature_path=paths["signature"],
                        attestation_path=paths["attestation"],
                        expected_workflow_identity=WORKFLOW_IDENTITY,
                        expected_workflow_sha=WORKFLOW_SHA,
                    )

    def test_final_gate_accepts_only_cross_bound_complete_phase_reports(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            paths = self._write_common(pathlib.Path(temporary))
            final = container.enforce_final_gate(
                runtime_report_path=paths["runtime"],
                sbom_path=paths["sbom"],
                sbom_report_path=paths["sbom_report"],
                publish_report_path=paths["publish"],
                signature_path=paths["signature"],
                attestation_path=paths["attestation"],
                publication_report_path=paths["public"],
            )
            self.assertEqual(final["localImageId"], LOCAL_IMAGE_ID)
            self.assertEqual(final["registryManifestDigest"], MANIFEST_DIGEST)

    def test_final_gate_fails_on_missing_skipped_or_divergent_phase(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            paths = self._write_common(root)
            paths["signature"].unlink()
            with self.assertRaisesRegex(container.VerificationError, "missing"):
                container.enforce_final_gate(
                    runtime_report_path=paths["runtime"],
                    sbom_path=paths["sbom"],
                    sbom_report_path=paths["sbom_report"],
                    publish_report_path=paths["publish"],
                    signature_path=paths["signature"],
                    attestation_path=paths["attestation"],
                    publication_report_path=paths["public"],
                )
            paths = self._write_common(root)
            public = valid_publication_report(
                sbom_digest=container._sha256_file(paths["sbom"])
            )
            public["localImageId"] = f"sha256:{'f' * 64}"
            public["publicConfig"]["configDigest"] = f"sha256:{'f' * 64}"  # type: ignore[index]
            write_json(paths["public"], public)
            with self.assertRaisesRegex(container.VerificationError, "divergent"):
                container.enforce_final_gate(
                    runtime_report_path=paths["runtime"],
                    sbom_path=paths["sbom"],
                    sbom_report_path=paths["sbom_report"],
                    publish_report_path=paths["publish"],
                    signature_path=paths["signature"],
                    attestation_path=paths["attestation"],
                    publication_report_path=paths["public"],
                )

    def test_final_gate_rejects_substituted_valid_sbom_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            paths = self._write_common(pathlib.Path(temporary))
            substituted = valid_sbom()
            substituted["documentNamespace"] = "https://example.invalid/substituted-sbom"
            substituted["packages"] = [
                {"SPDXID": "SPDXRef-Substituted", "name": "substituted-package"}
            ]
            write_json(paths["sbom"], substituted)
            with self.assertRaisesRegex(container.VerificationError, "bytes changed"):
                container.enforce_final_gate(
                    runtime_report_path=paths["runtime"],
                    sbom_path=paths["sbom"],
                    sbom_report_path=paths["sbom_report"],
                    publish_report_path=paths["publish"],
                    signature_path=paths["signature"],
                    attestation_path=paths["attestation"],
                    publication_report_path=paths["public"],
                )


class WorkflowPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow_text = WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.workflow = yaml.load(cls.workflow_text, Loader=yaml.BaseLoader)
        cls.job = cls.workflow["jobs"]["publish"]
        cls.steps = cls.job["steps"]
        cls.by_name = {step["name"]: step for step in cls.steps}

    def test_permissions_runner_timeout_and_concurrency_are_fail_closed(self) -> None:
        self.assertEqual(
            self.workflow["permissions"],
            {"contents": "read", "id-token": "write", "packages": "write"},
        )
        self.assertEqual(self.job["runs-on"], "ubuntu-24.04")
        self.assertEqual(self.job["timeout-minutes"], "45")
        self.assertEqual(self.workflow["concurrency"]["cancel-in-progress"], "false")

    def test_every_action_is_immutable_commit_pinned(self) -> None:
        uses = [step["uses"] for step in self.steps if "uses" in step]
        self.assertGreaterEqual(len(uses), 7)
        for value in uses:
            reference = value.split("@", 1)[1]
            self.assertRegex(reference, r"^[0-9a-f]{40}$", value)
        self.assertIn(
            "docker/build-push-action@ee4ca427a2f43b6a16632044ca514c076267da23",
            uses,
        )
        self.assertIn(
            "anchore/sbom-action@e22c389904149dbc22b58101806040fa8d37a610",
            uses,
        )
        self.assertIn(
            "sigstore/cosign-installer@6f9f17788090df1f26f669e9d70d6ae9567deba6",
            uses,
        )

    def test_build_occurs_once_loads_locally_and_never_pushes(self) -> None:
        build_steps = [
            step
            for step in self.steps
            if step.get("uses", "").startswith("docker/build-push-action@")
        ]
        self.assertEqual(len(build_steps), 1)
        build = build_steps[0]["with"]
        self.assertEqual(build["load"], "true")
        self.assertEqual(build["push"], "false")
        self.assertEqual(build["platforms"], "linux/amd64")
        self.assertEqual(build["tags"], "${{ env.TEST_IMAGE }}")
        self.assertEqual(build["provenance"], "false")
        self.assertEqual(build["sbom"], "false")

    def test_latest_promotion_is_one_explicit_disabled_dispatch_choice(
        self,
    ) -> None:
        push_latest = self.workflow["on"]["workflow_dispatch"]["inputs"][
            "push_latest"
        ]
        self.assertEqual(
            push_latest,
            {
                "description": (
                    "Also promote this exact tested digest to latest"
                ),
                "required": "true",
                "default": "false",
                "type": "choice",
                "options": ["true", "false"],
            },
        )
        tag_lines = []
        for line in self.by_name[
            "Define publication tags and OCI labels"
        ]["with"]["tags"].splitlines():
            stripped = line.strip()
            fields = stripped.split(",")
            if "type=raw" in fields and "value=latest" in fields:
                tag_lines.append(stripped)
        self.assertEqual(
            tag_lines,
            [
                "type=raw,value=latest,enable=${{ "
                "github.event_name == 'workflow_dispatch' && "
                "inputs.push_latest == 'true' }}"
            ],
        )

    def test_smoke_and_sbom_finish_before_registry_login_or_push(self) -> None:
        names = [step["name"] for step in self.steps]
        smoke = names.index("Consume the loaded image through real server boundaries")
        sbom = names.index("Bind SBOM bytes to the tested local image ID")
        login = names.index("Login only after local consumption passes")
        publish = names.index("Push only tags of the already-tested daemon image")
        self.assertLess(smoke, sbom)
        self.assertLess(sbom, login)
        self.assertLess(login, publish)
        self.assertIn(
            "verify_server_container.py smoke",
            self.by_name["Consume the loaded image through real server boundaries"]["run"],
        )
        self.assertIn(
            "verify_server_container.py verify-sbom",
            self.by_name["Bind SBOM bytes to the tested local image ID"]["run"],
        )

    def test_publish_uses_only_the_loaded_image_and_exact_full_sha_tag(self) -> None:
        publish = self.by_name["Push only tags of the already-tested daemon image"]["run"]
        self.assertIn("verify_server_container.py publish", publish)
        self.assertIn("--local-image", publish)
        self.assertNotIn("docker build", publish)
        metadata = self.by_name["Define publication tags and OCI labels"]["with"]
        self.assertIn("type=raw,value=sha-${{ github.sha }}", metadata["tags"])
        self.assertIn(
            "org.opencontainers.image.revision=${{ github.sha }}", metadata["labels"]
        )
        self.assertIn(
            "org.opencontainers.image.source=https://github.com/${{ github.repository }}",
            metadata["labels"],
        )

    def test_syft_and_cosign_versions_are_explicit_and_keyless_identity_is_exact(self) -> None:
        sbom = self.by_name["Generate SPDX SBOM from the tested local image"]["with"]
        self.assertEqual(sbom["image"], "${{ env.TEST_IMAGE }}")
        self.assertEqual(sbom["syft-version"], "v1.44.0")
        self.assertEqual(sbom["upload-artifact"], "false")
        cosign = self.by_name["Install pinned Cosign"]["with"]
        self.assertEqual(cosign["cosign-release"], "v3.0.6")
        verify = self.by_name["Verify keyless identity and workflow claims"]["run"]
        for required in [
            "--certificate-identity",
            "--certificate-oidc-issuer",
            "--certificate-github-workflow-repository",
            "--certificate-github-workflow-sha",
            '--annotation "sourceSha=$GITHUB_SHA"',
            '--annotation "workflowSourceSha=$WORKFLOW_SOURCE_SHA"',
            "verify-attestation",
        ]:
            self.assertIn(required, verify)

    def test_public_comparison_is_anonymous_bounded_and_after_logout(self) -> None:
        names = [step["name"] for step in self.steps]
        logout = names.index("Remove registry credentials before public comparison")
        public = names.index(
            "Compare every public tag, digest, config, SBOM, and signature subject"
        )
        self.assertLess(logout, public)
        self.assertEqual(self.steps[logout]["if"], "always()")
        self.assertEqual(self.steps[public]["if"], "success()")
        command = self.steps[public]["run"]
        self.assertIn("verify-publication", command)
        self.assertIn("--expected-workflow-sha", command)

    def test_always_final_gate_and_evidence_retention_make_skips_fail(self) -> None:
        final = self.by_name["Enforce every container publication phase"]
        upload = self.by_name["Retain all container verification evidence"]
        self.assertEqual(final["if"], "always()")
        self.assertIn("final-gate", final["run"])
        for phase in [
            "--runtime-report",
            "--sbom-report",
            "--publish-report",
            "--signature-verification",
            "--attestation-verification",
            "--publication-report",
        ]:
            self.assertIn(phase, final["run"])
        self.assertEqual(upload["if"], "always()")
        self.assertEqual(upload["with"]["if-no-files-found"], "error")
        self.assertEqual(upload["with"]["retention-days"], "30")

    def test_dockerfile_frontend_and_base_images_are_digest_pinned(self) -> None:
        dockerfile = DOCKERFILE_PATH.read_text(encoding="utf-8")
        self.assertRegex(
            dockerfile.splitlines()[0],
            r"^# syntax=docker/dockerfile:1@sha256:[0-9a-f]{64}$",
        )
        from_lines = [
            line for line in dockerfile.splitlines() if line.startswith("FROM ")
        ]
        self.assertEqual(len(from_lines), 2)
        for line in from_lines:
            self.assertRegex(line, r"@sha256:[0-9a-f]{64}(?: AS \w+)?$")
        self.assertIn(
            "RUN cargo build --release --locked -p sorotte-server --bin sorotte-server",
            dockerfile,
        )
        for label in [
            "org.opencontainers.image.source",
            "org.opencontainers.image.revision",
            "org.opencontainers.image.created",
        ]:
            self.assertIn(label, dockerfile)


if __name__ == "__main__":
    unittest.main()
