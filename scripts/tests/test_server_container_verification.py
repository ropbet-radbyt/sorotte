from __future__ import annotations

import base64
import copy
import hashlib
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
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


def actual_command_streams(respond):
    """Route external commands to a bounded child without replacing pipe capture."""
    run = subprocess.run

    def child(command, **kwargs):
        stdout, stderr, returncode = respond(list(command))
        kwargs["timeout"] = min(kwargs["timeout"], 10)
        return run(
            [sys.executable, "-c",
             "import sys; sys.stderr.write(sys.argv[2]); sys.stderr.flush(); "
             "sys.stdout.write(sys.argv[1]); sys.stdout.flush(); sys.exit(int(sys.argv[3]))",
             stdout, stderr, str(returncode)],
            **kwargs,
        )

    return mock.patch.object(container.subprocess, "run", side_effect=child)


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
                    "startTls": True,
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


def valid_signature_output(
    *,
    docker_reference: str = IMAGE_NAME,
    signature_type: str = "cosign container image signature",
) -> list[dict[str, object]]:
    return [
        {
            "critical": {
                "identity": {"docker-reference": docker_reference},
                "image": {"docker-manifest-digest": MANIFEST_DIGEST},
                "type": signature_type,
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


class CommandStreamTests(unittest.TestCase):
    def test_machine_output_is_separate_from_actual_child_diagnostics(self) -> None:
        command = [sys.executable, "-c",
                   "import sys; print('verified successfully', file=sys.stderr, flush=True); "
                   "print('{\"verified\": true}')"]
        result = container._run(command, timeout=10)
        self.assertEqual(json.loads(result.stdout), {"verified": True})
        self.assertEqual(result.stderr, "verified successfully\n")

    def test_nonzero_child_preserves_both_streams_and_exit_status(self) -> None:
        command = [sys.executable, "-c",
                   "import sys; print('partial result'); "
                   "print('certificate verification failed', file=sys.stderr); sys.exit(7)"]
        with self.assertRaises(container.VerificationError) as failure:
            container._run(command, timeout=10)
        self.assertIn("command exited 7", str(failure.exception))
        self.assertIn("stdout:\npartial result", str(failure.exception))
        self.assertIn("stderr:\ncertificate verification failed", str(failure.exception))
        result = container._run(command, timeout=10, check=False)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, "partial result\n")
        self.assertEqual(result.stderr, "certificate verification failed\n")

    def test_docker_json_and_scalar_output_ignore_successful_stderr(self) -> None:
        responses = {
            ("docker", "image", "inspect", TEST_IMAGE): (json.dumps(valid_local_inspection()), "daemon notice\n", 0),
            ("docker", "port", "owned-fixture", "8999/tcp"): ("127.0.0.1:43210\n", "daemon notice\n", 0),
        }
        with actual_command_streams(lambda command: responses[tuple(command)]):
            self.assertEqual(container.inspect_local_image(
                TEST_IMAGE, expected_source_sha=SOURCE_SHA, expected_source_url=SOURCE_URL,
            )["id"], LOCAL_IMAGE_ID)
            self.assertEqual(container._published_loopback_port("owned-fixture"), 43210)


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
    def test_actual_container_stderr_is_retained_and_satisfies_log_markers(self) -> None:
        for marker, require_shutdown in (
            (container.CONTAINER_STARTUP_LOG_MARKER, False),
            (container.CONTAINER_SHUTDOWN_LOG_MARKER, True),
        ):
            with self.subTest(marker=marker), tempfile.TemporaryDirectory() as temporary:
                responses = {
                    ("docker", "logs", "owned-fixture"): ("stdout diagnostic\n", marker + "\n", 0),
                    ("docker", "rm", "--force", "owned-fixture"): ("", "", 0),
                }
                path = pathlib.Path(temporary) / "container.log"
                with actual_command_streams(lambda command: responses[tuple(command)]):
                    container._write_container_log_and_remove(
                        "owned-fixture", path, require_shutdown_marker=require_shutdown,
                    )
                self.assertIn("stdout diagnostic\n", path.read_text())
                self.assertIn(marker + "\n", path.read_text())

    def test_actual_container_removal_failure_keeps_stderr_and_primary_error(self) -> None:
        responses = {
            ("docker", "logs", "owned-fixture"): (container.CONTAINER_STARTUP_LOG_MARKER, "", 0),
            ("docker", "rm", "--force", "owned-fixture"): ("removal incomplete\n", "container still busy\n", 9),
        }
        with tempfile.TemporaryDirectory() as temporary:
            with actual_command_streams(lambda command: responses[tuple(command)]):
                with self.assertRaises(container.VerificationError) as failure:
                    try:
                        raise container.VerificationError("original protocol failure")
                    finally:
                        container._write_container_log_and_remove(
                            "owned-fixture", pathlib.Path(temporary) / "container.log",
                            require_shutdown_marker=False,
                        )
            for detail in ("original protocol failure", "docker rm exited 9",
                           "removal incomplete", "container still busy"):
                self.assertIn(detail, str(failure.exception))

    def test_starttls_negotiation_requires_exact_ack_and_clean_boundary(self) -> None:
        session = mock.Mock()
        session.buffered = b""

        def receive_until(matchers: object, description: str) -> dict[str, object]:
            matcher = matchers["startTls"]  # type: ignore[index]
            self.assertEqual(description, "STARTTLS acceptance")
            self.assertFalse(matcher({"TLS": {"startTLS": "false"}}))
            self.assertFalse(matcher({"TLS": {"startTLS": True}}))
            self.assertTrue(matcher({"TLS": {"startTLS": "true"}}))
            return {"startTls": {"TLS": {"startTLS": "true"}}}

        session.receive_until.side_effect = receive_until
        container._negotiate_start_tls(session)
        session.send.assert_called_once_with({"TLS": {"startTLS": "send"}})

        session.buffered = b"unexpected plaintext"
        with self.assertRaisesRegex(
            container.VerificationError,
            "beyond the STARTTLS acknowledgement",
        ):
            container._negotiate_start_tls(session)

    def test_protocol_hello_rejects_overlong_test_identity(self) -> None:
        with self.assertRaisesRegex(
            container.VerificationError, "default 16-character limit"
        ):
            container._hello_message("u" * 17, "room")

    def test_protocol_hello_requires_exact_canonical_identity_echo(self) -> None:
        session = mock.Mock()
        expected = {
            "Hello": {
                "username": "writer-7e91",
                "room": {"name": "room-7e91"},
            }
        }

        def receive_until(matchers: object, description: str) -> dict[str, object]:
            matcher = matchers["hello"]  # type: ignore[index]
            self.assertEqual(description, "protocol Hello")
            self.assertFalse(
                matcher(
                    {
                        "Hello": {
                            "username": "writer-7e9",
                            "room": {"name": "room-7e91"},
                        }
                    }
                )
            )
            self.assertFalse(
                matcher(
                    {
                        "Hello": {
                            "username": "writer-7e91",
                            "room": {"name": "other-room"},
                        }
                    }
                )
            )
            self.assertTrue(matcher(expected))
            return {"hello": expected}

        session.receive_until.side_effect = receive_until
        self.assertEqual(
            container._protocol_hello(
                session,
                username="writer-7e91",
                room="room-7e91",
            ),
            expected,
        )
        session.send.assert_called_once_with(
            container._hello_message("writer-7e91", "room-7e91")
        )

    def test_stop_requires_clean_exited_state_after_direct_sigint(self) -> None:
        command_results = [
            subprocess.CompletedProcess(["docker", "kill"], 0, stdout="name\n"),
            subprocess.CompletedProcess(["docker", "wait"], 0, stdout="0\n"),
        ]
        state = {
            "Status": "exited",
            "Running": False,
            "Dead": False,
            "ExitCode": 0,
            "OOMKilled": False,
            "Error": "",
        }
        with (
            mock.patch.object(container, "_run", side_effect=command_results) as run,
            mock.patch.object(container, "_docker_json", return_value=state),
        ):
            self.assertEqual(
                container._stop_and_inspect_container("container-name"),
                {
                    "error": "",
                    "exitCode": 0,
                    "oomKilled": False,
                    "signal": "SIGINT",
                },
            )

        self.assertEqual(
            run.call_args_list,
            [
                mock.call(
                    ["docker", "kill", "--signal=SIGINT", "container-name"],
                    timeout=container.SERVER_STOP_TIMEOUT_SECONDS,
                ),
                mock.call(
                    ["docker", "wait", "container-name"],
                    timeout=container.SERVER_STOP_TIMEOUT_SECONDS,
                ),
            ],
        )

    def test_stop_rejects_unclean_inspected_state(self) -> None:
        variants = [
            {"Status": "dead"},
            {"Running": True},
            {"Dead": True},
            {"ExitCode": 1},
            {"OOMKilled": True},
            {"Error": "daemon failure"},
        ]
        for update in variants:
            with self.subTest(update=update):
                state = {
                    "Status": "exited",
                    "Running": False,
                    "Dead": False,
                    "ExitCode": 0,
                    "OOMKilled": False,
                    "Error": "",
                }
                state.update(update)
                command_results = [
                    subprocess.CompletedProcess(
                        ["docker", "kill"], 0, stdout="name\n"
                    ),
                    subprocess.CompletedProcess(
                        ["docker", "wait"], 0, stdout="0\n"
                    ),
                ]
                with (
                    mock.patch.object(container, "_run", side_effect=command_results),
                    mock.patch.object(container, "_docker_json", return_value=state),
                    self.assertRaisesRegex(
                        container.VerificationError, "did not stop cleanly"
                    ),
                ):
                    container._stop_and_inspect_container("container-name")

    def test_container_log_capture_accepts_startup_log_without_shutdown_text(self) -> None:
        marker = "sorotte-server listening on 0.0.0.0:8999\n"
        log_results = [
            subprocess.CompletedProcess(
                ["docker", "logs"], 0, stdout=marker
            ),
            subprocess.CompletedProcess(["docker", "rm"], 0, stdout=""),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "container.log"
            with (
                mock.patch.object(container, "_run", side_effect=log_results) as run,
                mock.patch.object(container.time, "sleep") as sleep,
            ):
                container._write_container_log_and_remove(
                    "container-name",
                    path,
                    require_shutdown_marker=False,
                )

            self.assertEqual(
                path.read_text(encoding="utf-8"),
                marker,
            )
            self.assertEqual(
                run.call_args_list,
                [
                    mock.call(["docker", "logs", "container-name"], check=False),
                    mock.call(
                        ["docker", "rm", "--force", "container-name"], check=False
                    ),
                ],
            )
            sleep.assert_not_called()

    def test_container_log_capture_waits_for_delayed_startup_marker(self) -> None:
        marker = "sorotte-server listening on 0.0.0.0:8999\n"
        log_results = [
            subprocess.CompletedProcess(["docker", "logs"], 0, stdout=""),
            subprocess.CompletedProcess(["docker", "logs"], 0, stdout=marker),
            subprocess.CompletedProcess(["docker", "rm"], 0, stdout=""),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "container.log"
            with (
                mock.patch.object(container, "_run", side_effect=log_results) as run,
                mock.patch.object(container.time, "sleep") as sleep,
            ):
                container._write_container_log_and_remove(
                    "container-name",
                    path,
                    require_shutdown_marker=False,
                )

            self.assertEqual(path.read_text(encoding="utf-8"), marker)
            self.assertEqual(run.call_count, 3)
            sleep.assert_called_once_with(container.CONTAINER_LOG_CAPTURE_RETRY_SECONDS)

    def test_container_log_capture_accepts_completed_shutdown_marker(self) -> None:
        log = (
            "sorotte-server listening on 0.0.0.0:8999\n"
            "sorotte-server: shutdown requested; draining client sessions\n"
        )
        outputs = [
            subprocess.CompletedProcess(["docker", "logs"], 0, stdout=log),
            subprocess.CompletedProcess(["docker", "rm"], 0, stdout=""),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "container.log"
            with mock.patch.object(container, "_run", side_effect=outputs):
                container._write_container_log_and_remove(
                    "container-name",
                    path,
                    require_shutdown_marker=True,
                )

            self.assertEqual(path.read_text(encoding="utf-8"), log)

    def test_container_log_capture_requires_shutdown_marker_after_completed_stop(
        self,
    ) -> None:
        startup = "sorotte-server listening on 0.0.0.0:8999\n"
        outputs = [
            subprocess.CompletedProcess(
                ["docker", "logs"], 0, stdout=startup
            )
            for _ in range(3)
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
                    "did not retain the graceful shutdown barrier",
                ):
                    container._write_container_log_and_remove(
                        "container-name",
                        path,
                        require_shutdown_marker=True,
                    )

            self.assertEqual(path.read_text(encoding="utf-8"), startup)
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

    def test_container_log_capture_preserves_primary_scenario_failure(self) -> None:
        outputs = [
            subprocess.CompletedProcess(
                ["docker", "logs"], 0, stdout="unrelated log\n"
            ),
            subprocess.CompletedProcess(["docker", "rm"], 0, stdout=""),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "container.log"
            try:
                raise container.VerificationError("primary watcher join failure")
            except container.VerificationError:
                with (
                    mock.patch.object(container, "CONTAINER_LOG_CAPTURE_ATTEMPTS", 1),
                    mock.patch.object(container, "_run", side_effect=outputs),
                    self.assertRaisesRegex(
                        container.VerificationError,
                        "primary watcher join failure; container diagnostics/cleanup "
                        "also failed: .*startup listener marker",
                    ),
                ):
                    container._write_container_log_and_remove(
                        "container-name",
                        path,
                        require_shutdown_marker=False,
                    )

    def test_container_cleanup_appends_removal_failure_to_primary_error(self) -> None:
        startup = "sorotte-server listening on 0.0.0.0:8999\n"
        outputs = [
            subprocess.CompletedProcess(["docker", "logs"], 0, stdout=startup),
            subprocess.CompletedProcess(
                ["docker", "rm"], 1, stdout="container is busy\n"
            ),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "container.log"
            try:
                raise container.VerificationError("primary protocol failure")
            except container.VerificationError:
                with (
                    mock.patch.object(container, "_run", side_effect=outputs),
                    self.assertRaisesRegex(
                        container.VerificationError,
                        "primary protocol failure; container diagnostics/cleanup "
                        "also failed: docker rm exited 1: container is busy",
                    ),
                ):
                    container._write_container_log_and_remove(
                        "container-name",
                        path,
                        require_shutdown_marker=False,
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
            drift = valid_runtime_report()
            plaintext = drift["scenarios"][0]  # type: ignore[index]
            plaintext["shutdown"]["error"] = "daemon failure"  # type: ignore[index]
            write_json(path, drift)
            with self.assertRaisesRegex(container.VerificationError, "shut down cleanly"):
                container.parse_runtime_report(path)
            drift = valid_runtime_report()
            tls = drift["scenarios"][1]  # type: ignore[index]
            tls["tls"]["startTls"] = False  # type: ignore[index]
            write_json(path, drift)
            with self.assertRaisesRegex(container.VerificationError, "prove STARTTLS"):
                container.parse_runtime_report(path)

    def test_raw_persisted_room_row_is_exact_and_does_not_create_wal_sidecars(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "rooms.sqlite3"
            connection = sqlite3.connect(path)
            self.assertEqual(
                connection.execute("PRAGMA journal_mode=WAL").fetchone(),
                ("wal",),
            )
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
            sidecars = [
                pathlib.Path(f"{path}-shm"),
                pathlib.Path(f"{path}-wal"),
            ]
            self.assertFalse(any(sidecar.exists() for sidecar in sidecars))

            database_evidence = container._verify_sqlite(
                path,
                "stopped container rooms database",
            )
            self.assertEqual(database_evidence["integrityCheck"], "ok")
            self.assertFalse(any(sidecar.exists() for sidecar in sidecars))

            evidence = container._verify_persisted_room_row(
                path,
                room="container-persisted-room",
                playlist=["container-alpha.mkv", "container-beta.mkv"],
                playlist_index=1,
                position=137.25,
            )
            self.assertEqual(evidence["integrityCheck"], "ok")
            self.assertEqual(evidence["row"]["persistenceVersion"], 4)
            self.assertFalse(any(sidecar.exists() for sidecar in sidecars))

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
            self.assertFalse(any(sidecar.exists() for sidecar in sidecars))


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
    def test_actual_push_diagnostic_stream_retains_digest_and_rejects_divergence(self) -> None:
        for conflicting_stdout in (False, True):
            with self.subTest(conflicting_stdout=conflicting_stdout), tempfile.TemporaryDirectory() as temporary:
                def respond(command):
                    if command[:3] == ["docker", "image", "tag"]:
                        return "", "", 0
                    self.assertEqual(command[:3], ["docker", "image", "push"])
                    stdout = f"digest: sha256:{'f' * 64}\n" if conflicting_stdout else "push progress\n"
                    return stdout, f"digest: {MANIFEST_DIGEST}\n", 0

                with mock.patch.object(container, "inspect_local_image", return_value={"id": LOCAL_IMAGE_ID}):
                    with actual_command_streams(respond):
                        arguments = dict(
                            local_image=TEST_IMAGE, image_name=IMAGE_NAME,
                            tags_path=self._tags(pathlib.Path(temporary)),
                            expected_source_sha=SOURCE_SHA, expected_source_url=SOURCE_URL,
                        )
                        if conflicting_stdout:
                            with self.assertRaisesRegex(container.VerificationError, "exactly one"):
                                container.publish_tested_image(**arguments)
                        else:
                            self.assertEqual(container.publish_tested_image(**arguments)["digest"], MANIFEST_DIGEST)

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

    def test_signature_output_accepts_only_canonical_cosign_v3_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "signature.json"
            v3_type = "https://sigstore.dev/cosign/sign/v1"
            digest_identity = f"{IMAGE_NAME}@{MANIFEST_DIGEST}"
            write_json(
                path,
                valid_signature_output(
                    docker_reference=digest_identity,
                    signature_type=v3_type,
                ),
            )
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
            invalid_records = [
                (f"{IMAGE_NAME}:latest", v3_type),
                (f"{IMAGE_NAME}@sha256:{'f' * 64}", v3_type),
                (f"ghcr.io/ropbet-radbyt/other@{MANIFEST_DIGEST}", v3_type),
                (digest_identity, "https://example.invalid/signature"),
            ]
            for docker_reference, signature_type in invalid_records:
                with self.subTest(
                    docker_reference=docker_reference,
                    signature_type=signature_type,
                ):
                    write_json(
                        path,
                        valid_signature_output(
                            docker_reference=docker_reference,
                            signature_type=signature_type,
                        ),
                    )
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


class PromotionStreamTests(unittest.TestCase):
    BANNER = "\nVerification for the requested digest --\nThe cosign claims were validated\n\n"

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / "runtime").mkdir()
        self.output = self.root / "promotion"
        self.signature = valid_signature_output(
            docker_reference=f"{IMAGE_NAME}@{MANIFEST_DIGEST}",
            signature_type="https://sigstore.dev/cosign/sign/v1",
        )
        self.signature[0]["optional"]["workflowSourceSha"] = SOURCE_SHA
        write_json(self.root / "runtime/runtime-report.json", valid_runtime_report())
        write_json(self.root / "sbom.spdx.json", valid_sbom())
        sbom_digest = container._sha256_file(self.root / "sbom.spdx.json")
        write_json(self.root / "sbom-report.json", valid_sbom_report(sbom_digest=sbom_digest))
        published = valid_publish_report()
        version = f"{IMAGE_NAME}:v0.2.3"
        published["tags"].append(version)
        published["pushes"].append({"tag": version, "digest": MANIFEST_DIGEST})
        write_json(self.root / "publish-report.json", published)
        public = valid_publication_report(sbom_digest=sbom_digest)
        public["verificationPolicy"]["workflowSourceSha"] = SOURCE_SHA
        public["publicReferences"].append({"reference": version, "digest": MANIFEST_DIGEST})
        write_json(self.root / "publication-report.json", public)
        write_json(self.root / "signature-verification.json", self.signature)
        write_json(self.root / "attestation-verification.json", valid_attestation_output())

    def promote(self, **kwargs):
        return container.promote_approved_digest(
            evidence_dir=self.root, expected_digest=MANIFEST_DIGEST,
            expected_source_sha=SOURCE_SHA, expected_source_url=SOURCE_URL,
            version_tag="v0.2.3", output_dir=self.output,
            **kwargs,
        )

    def cosign_response(self, command):
        self.assertEqual(command[0], "cosign")
        self.assertIn(command[1], {"verify", "verify-attestation"})
        self.assertEqual(command[-3:], ["--output", "json", f"{IMAGE_NAME}@{MANIFEST_DIGEST}"])
        self.assertEqual(command[command.index("--certificate-identity") + 1], WORKFLOW_IDENTITY)
        self.assertEqual(command[command.index("--certificate-github-workflow-sha") + 1], SOURCE_SHA)
        payload = self.signature if command[1] == "verify" else valid_attestation_output()
        return json.dumps(payload) + "\n", self.BANNER, 0

    def promote_with_actual_streams(self, *, authorize=None):
        # Exercise promotion and both final gates, replacing only external tools
        # and registry reads. The real subprocess pipe options reach the child.
        events = []

        def respond(command):
            events.append(tuple(command))
            if command[0] == "cosign":
                return self.cosign_response(command)
            self.assertEqual(command, [
                "docker", "buildx", "imagetools", "create", "--prefer-index=false",
                "--tag", f"{IMAGE_NAME}:latest", f"{IMAGE_NAME}@{MANIFEST_DIGEST}",
            ])
            self.assertIn("public-read", events)
            return "assigned approved digest\n", "registry diagnostic\n", 0

        def fetch(*_args, **_kwargs):
            events.append("public-read")
            return {"schemaVersion": 2}, MANIFEST_DIGEST, b"same-manifest"

        with (
            actual_command_streams(respond),
            mock.patch.object(container, "_anonymous_ghcr_token", return_value="anonymous"),
            mock.patch.object(container, "_fetch_public_manifest", side_effect=fetch),
            mock.patch.object(container, "_verify_public_config",
                              return_value=valid_publication_report()["publicConfig"]),
        ):
            final = self.promote(**({} if authorize is None else {
                "before_latest_assignment": lambda: authorize(events),
            }))
        return final, events

    def test_promotion_parses_actual_stdout_with_success_banner_on_stderr(self) -> None:
        final, events = self.promote_with_actual_streams()
        self.assertEqual(final["status"], "passed")
        self.assertEqual(final["registryManifestDigest"], MANIFEST_DIGEST)
        self.assertEqual(json.loads((self.output / "signature-verification.json").read_text()), self.signature)
        self.assertEqual(json.loads((self.output / "attestation-verification.json").read_text()), valid_attestation_output())
        mutation = [index for index, event in enumerate(events) if isinstance(event, tuple) and event[0] == "docker"]
        self.assertEqual(len(mutation), 1)
        self.assertEqual(events[0][:2], ("cosign", "verify"))
        self.assertEqual(events[1][:2], ("cosign", "verify-attestation"))
        self.assertEqual(events[2:mutation[0]], ["public-read"] * 3)
        self.assertEqual(events[mutation[0] + 1:], ["public-read"] * 4)
        promoted = container.parse_publish_report(self.output / "publish-report.json")
        self.assertEqual(promoted["tags"], [f"{IMAGE_NAME}:sha-{SOURCE_SHA}",
                                            f"{IMAGE_NAME}:v0.2.3", f"{IMAGE_NAME}:latest"])

    def test_already_present_latest_preserves_tag_and_push_order_without_duplicates(self) -> None:
        published = json.loads((self.root / "publish-report.json").read_text())
        latest = f"{IMAGE_NAME}:latest"
        published["tags"].insert(1, latest)
        published["pushes"].insert(1, {"tag": latest, "digest": MANIFEST_DIGEST})
        write_json(self.root / "publish-report.json", published)
        public = json.loads((self.root / "publication-report.json").read_text())
        public["publicReferences"].append({"reference": latest, "digest": MANIFEST_DIGEST})
        write_json(self.root / "publication-report.json", public)
        final, _ = self.promote_with_actual_streams()
        self.assertEqual(final["status"], "passed")
        promoted = container.parse_publish_report(self.output / "publish-report.json")
        self.assertEqual(promoted["tags"], published["tags"])
        self.assertEqual(promoted["pushes"], published["pushes"])

    def test_final_authorization_runs_after_public_reads_immediately_before_assignment(self) -> None:
        final, events = self.promote_with_actual_streams(authorize=lambda events: events.append("authorize"))
        self.assertEqual(final["status"], "passed")
        assignment = next(index for index, event in enumerate(events)
                          if isinstance(event, tuple) and event[0] == "docker")
        self.assertEqual(events[2:assignment], ["public-read"] * 3 + ["authorize"])
        self.assertEqual(events.count("authorize"), 1)

    def test_final_authorization_failure_prevents_assignment(self) -> None:
        observed = []

        def reject(events):
            observed.extend(events)
            raise container.VerificationError("protected tooling main changed")

        with self.assertRaisesRegex(container.VerificationError, "protected tooling main changed"):
            self.promote_with_actual_streams(authorize=reject)
        self.assertEqual(observed[2:], ["public-read"] * 3)
        self.assertFalse(any(isinstance(event, tuple) and event[0] == "docker" for event in observed))
        self.assertFalse((self.output / "publish-report.json").exists())

    def test_nonzero_cosign_rejects_valid_stdout_before_registry_mutation(self) -> None:
        for failing_command in ("verify", "verify-attestation"):
            with self.subTest(command=failing_command):
                self.output = self.root / failing_command

                def respond(command):
                    stdout, stderr, _ = self.cosign_response(command)
                    return stdout, stderr + "certificate verification failed\n", 7 if command[1] == failing_command else 0

                authorize = mock.Mock()
                with actual_command_streams(respond) as run, mock.patch.object(container, "_anonymous_ghcr_token") as registry:
                    with self.assertRaises(container.VerificationError) as failure:
                        self.promote(before_latest_assignment=authorize)
                self.assertIn("command exited 7", str(failure.exception))
                self.assertIn("certificate verification failed", str(failure.exception))
                self.assertIn("stdout:\n", str(failure.exception))
                self.assertIn("stderr:\n", str(failure.exception))
                self.assertTrue(all(call.args[0][0] == "cosign" for call in run.call_args_list))
                self.assertEqual(run.call_count, 1 if failing_command == "verify" else 2)
                registry.assert_not_called()
                authorize.assert_not_called()
                self.assertFalse((self.output / "publish-report.json").exists())

    def test_successful_cosign_with_invalid_stdout_remains_rejected(self) -> None:
        cases = [(phase, invalid) for phase in ("verify", "verify-attestation")
                 for invalid in (self.BANNER + "{}", '{"payload":"a","payload":"b"}', "")]
        for phase, invalid in cases:
            with self.subTest(phase=phase, stdout=invalid), tempfile.TemporaryDirectory() as temporary:
                self.output = pathlib.Path(temporary) / "promotion"

                def respond(command):
                    stdout, stderr, status = self.cosign_response(command)
                    return (invalid if command[1] == phase else stdout), stderr, status

                authorize = mock.Mock()
                with actual_command_streams(respond) as run, mock.patch.object(container, "_anonymous_ghcr_token") as registry:
                    with self.assertRaises(container.VerificationError):
                        self.promote(before_latest_assignment=authorize)
                self.assertTrue(all(call.args[0][0] == "cosign" for call in run.call_args_list))
                registry.assert_not_called()
                authorize.assert_not_called()
                self.assertFalse((self.output / "publish-report.json").exists())


class ImmutableBuildMetadataCommandTests(unittest.TestCase):
    def setUp(self) -> None:
        workflow = yaml.load(WORKFLOW_PATH.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
        self.step = next(step for step in workflow["jobs"]["publish"]["steps"]
                         if step.get("id") == "build_info")
        self.temporary = tempfile.TemporaryDirectory(prefix="container-metadata-")
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.environment = {
            key: value for key, value in os.environ.items()
            if not key.startswith("GIT_")
        }
        self.environment.update({
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_AUTHOR_NAME": "Metadata fixture",
            "GIT_AUTHOR_EMAIL": "metadata@example.invalid",
            "GIT_COMMITTER_NAME": "Metadata fixture",
            "GIT_COMMITTER_EMAIL": "metadata@example.invalid",
            "GIT_AUTHOR_DATE": "2020-01-02T03:04:05+0000",
        })
        self.git("init", "--quiet", "--template=", ".")
        self.output = self.root / "github-output.txt"
        self.output.write_text("existing=preserved\n", encoding="utf-8")
        self.environment["GITHUB_OUTPUT"] = str(self.output)

    def git(self, *arguments: str) -> str:
        result = subprocess.run(
            ["git", "-c", f"safe.directory={self.root.as_posix()}", "-c", "commit.gpgsign=false",
             *arguments],
            cwd=self.root, env=self.environment, capture_output=True, text=True, timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def commit(self, committer_date: str) -> str:
        self.environment["GIT_COMMITTER_DATE"] = committer_date
        self.git("commit", "--quiet", "--allow-empty", "-m", "Metadata fixture")
        return self.git("rev-parse", "HEAD")

    def run_metadata(self, source: str) -> subprocess.CompletedProcess[str]:
        self.assertEqual(self.step.get("shell"), "python")
        script = self.root / "actual-workflow-metadata.py"
        script.write_text(self.step["run"], encoding="utf-8")
        environment = {**self.environment, "GITHUB_SHA": source}
        return subprocess.run(
            [sys.executable, str(script)], cwd=self.root, env=environment,
            capture_output=True, text=True, timeout=15,
        )

    def test_actual_workflow_command_uses_requested_commit_in_canonical_utc(self) -> None:
        for committer_date, expected in (
            ("2026-09-07T13:53:22+1000", "2026-09-07T03:53:22Z"),
            ("2026-09-07T00:15:00+1000", "2026-09-06T14:15:00Z"),
            ("2026-09-07T23:45:00-0730", "2026-09-08T07:15:00Z"),
        ):
            with self.subTest(committer_date=committer_date):
                source = self.commit(committer_date)
                self.assertNotEqual(source, self.commit("2027-01-01T00:00:00+0000"))
                self.output.write_text("existing=preserved\n", encoding="utf-8")
                result = self.run_metadata(source)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.output.read_text(encoding="utf-8"),
                                 f"existing=preserved\ncreated={expected}\n")
                inspected = valid_local_inspection()
                inspected[0]["Config"]["Labels"][container.EXPECTED_CREATED_LABEL] = expected
                with mock.patch.object(container, "_docker_json", return_value=inspected):
                    identity = container.inspect_local_image(
                        TEST_IMAGE, expected_source_sha=SOURCE_SHA, expected_source_url=SOURCE_URL,
                    )
                self.assertEqual(identity["created"], expected)

    def test_missing_commit_fails_without_appending_a_created_label(self) -> None:
        self.commit("2026-09-07T13:53:22+1000")
        before = self.output.read_bytes()
        result = self.run_metadata("0" * 40)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.output.read_bytes(), before)


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
            {"contents": "read", "actions": "read", "checks": "read", "id-token": "write", "packages": "write"},
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
            "docker/build-push-action@53b7df96c91f9c12dcc8a07bcb9ccacbed38856a",
            uses,
        )
        self.assertIn(
            "anchore/sbom-action@3ad7283483fc7af8ff2b4ea19663c2d5ca935e26",
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

    def assert_latest_promotion_contract(self, workflow) -> None:
        # Tag refs implicitly add latest unless the action's auto flavor is disabled.
        self.assertEqual(set(workflow["on"]), {"workflow_call", "workflow_dispatch"})
        self.assertEqual(set(workflow["on"]["workflow_call"]["inputs"]), {"publish"})
        publish = workflow["jobs"]["publish"]
        metadata_steps = [step for step in publish["steps"] if step.get("uses", "").startswith("docker/metadata-action@")]
        self.assertEqual(len(metadata_steps), 1)
        self.assertEqual(
            metadata_steps[0]["with"].get("flavor"),
            "latest=false",
        )
        dispatch = workflow["on"]["workflow_dispatch"]["inputs"]
        self.assertEqual(set(dispatch), {"publication_run_id", "approved_digest", "version_tag"})
        for item in dispatch.values():
            self.assertEqual(item.get("required"), "true")
            self.assertEqual(item.get("type"), "string")
            self.assertNotIn("default", item)
        metadata_tags = metadata_steps[0]["with"]["tags"]
        self.assertNotIn("latest", metadata_tags)
        promotion = workflow["jobs"]["promote-approved-digest"]
        self.assertEqual(promotion.get("if"), "inputs.publication_run_id != ''")
        commands = "\n".join(step.get("run", "") for step in promotion["steps"])
        self.assertIn("container_promotion.py prepare", commands)
        self.assertIn("container_promotion.py promote", commands)
        self.assertNotIn("verify_server_container.py promote", commands)
        self.assertNotIn("docker build", commands)
        self.assertFalse(any("build-push-action" in step.get("uses", "") for step in promotion["steps"]))
        authority = [index for index, step in enumerate(promotion["steps"]) if "container_promotion.py prepare" in step.get("run", "")]
        assignment = [index for index, step in enumerate(promotion["steps"]) if "container_promotion.py promote" in step.get("run", "")]
        self.assertEqual([len(authority), len(assignment)], [1, 1])
        prepare = promotion["steps"][authority[0]]
        promote = promotion["steps"][assignment[0]]
        self.assertEqual(prepare["id"], "promotion-authority")
        self.assertLess(authority[0], assignment[0])
        expected_environment = {
            "GH_TOKEN": "${{ github.token }}",
            "SOROTTE_PROTECTION_TOKEN": "${{ steps.protection-token.outputs.token }}",
            "PUBLICATION_RUN_ID": "${{ inputs.publication_run_id }}",
            "VERSION_TAG": "${{ inputs.version_tag }}",
            "APPROVED_DIGEST": "${{ inputs.approved_digest }}",
        }
        for step in (prepare, promote):
            self.assertEqual(step.get("env"), expected_environment)
            for argument in ('--tooling-sha "$GITHUB_SHA"', '--publication-run-id "$PUBLICATION_RUN_ID"',
                             '--version-tag "$VERSION_TAG"', '--approved-digest "$APPROVED_DIGEST"'):
                self.assertIn(argument, step["run"])
            self.assertNotIn("continue-on-error", step)
            self.assertNotIn("if", step)
        self.assertIn("--output-dir target/promotion-authority/initial", prepare["run"])
        for argument in ("--initial-authority target/promotion-authority/initial/authority.json",
                         "--evidence-dir target/approved-container-publication",
                         "--authority-dir target/promotion-authority",
                         "--output-dir target/container-promotion",
                         "--report target/container-promotion/final-gate-report.json"):
            self.assertIn(argument, promote["run"])
        downloads = [(index, step) for index, step in enumerate(promotion["steps"])
                     if step.get("uses", "").startswith("actions/download-artifact@")]
        self.assertEqual(len(downloads), 1)
        index, download = downloads[0]
        self.assertLess(authority[0], index)
        self.assertLess(index, assignment[0])
        self.assertEqual(download["with"], {
            "github-token": "${{ github.token }}", "run-id": "${{ inputs.publication_run_id }}",
            "artifact-ids": "${{ steps.promotion-authority.outputs.artifact_id }}", "digest-mismatch": "error",
            "path": "target/approved-container-publication"})
        self.assertNotIn("continue-on-error", download)
        cleanup_index = next(index for index, step in enumerate(promotion["steps"])
                             if step.get("id") == "ci_remove_promotion_registry_credentials_cc7de03d")
        cleanup = promotion["steps"][cleanup_index]
        self.assertLess(assignment[0], cleanup_index)
        self.assertEqual(cleanup["if"], "always()")
        self.assertIn("docker logout ghcr.io || status=$?", cleanup["run"])
        self.assertIn("cp -R target/promotion-authority target/container-promotion/authority || status=$?", cleanup["run"])
        self.assertIn('exit "$status"', cleanup["run"])

    def test_latest_promotion_requires_explicit_verified_digest_without_rebuilding(self) -> None:
        self.assert_latest_promotion_contract(self.workflow)

    def test_latest_promotion_policy_rejects_automatic_unverified_or_rebuilt_images(self) -> None:
        for defect in ("automatic-trigger", "optional-input", "default-input", "automatic-latest",
                       "metadata-latest", "unguarded-promotion", "rebuild", "missing-authority",
                       "missing-promotion", "tolerated-authority", "tolerated-promotion", "early-promotion",
                       "early-download", "guessed-final-attempt", "foreign-download-run", "foreign-artifact",
                       "digest-warning", "missing-producer-retention", "conditional-producer-retention",
                       "prepare-github-token", "promote-github-token", "prepare-protection-token", "promote-protection-token",
                       "prepare-tooling", "promote-tooling", "prepare-producer", "promote-producer",
                       "prepare-version", "promote-version", "prepare-digest", "promote-digest",
                       "foreign-initial-authority", "missing-final-authority", "old-direct-promotion"):
            with self.subTest(defect=defect):
                candidate = copy.deepcopy(self.workflow)
                promotion = candidate["jobs"]["promote-approved-digest"]
                prepare = next(step for step in promotion["steps"] if "container_promotion.py prepare" in step.get("run", ""))
                promote = next(step for step in promotion["steps"] if "container_promotion.py promote" in step.get("run", ""))
                metadata = next(step for step in candidate["jobs"]["publish"]["steps"] if step.get("uses", "").startswith("docker/metadata-action@"))["with"]
                if defect == "automatic-trigger": candidate["on"]["push"] = {}
                if defect == "optional-input": candidate["on"]["workflow_dispatch"]["inputs"]["approved_digest"]["required"] = "false"
                if defect == "default-input": candidate["on"]["workflow_dispatch"]["inputs"]["version_tag"]["default"] = "v0.2.9"
                if defect == "automatic-latest": metadata["flavor"] = "latest=auto"
                if defect == "metadata-latest": metadata["tags"] += "\ntype=raw,value=latest"
                if defect == "unguarded-promotion": promotion["if"] = "true"
                if defect == "rebuild": promotion["steps"].append({"uses": "docker/build-push-action@" + "a" * 40})
                if defect == "missing-authority": promotion["steps"].remove(prepare)
                if defect == "missing-promotion": promotion["steps"].remove(promote)
                if defect == "tolerated-authority": prepare["continue-on-error"] = "true"
                if defect == "tolerated-promotion": promote["continue-on-error"] = "true"
                if defect == "early-promotion":
                    promotion["steps"].remove(promote)
                    promotion["steps"].insert(0, promote)
                if defect == "early-download":
                    index = next(index for index, step in enumerate(promotion["steps"]) if step.get("uses", "").startswith("actions/download-artifact@"))
                    promotion["steps"].insert(0, promotion["steps"].pop(index))
                if defect in {"guessed-final-attempt", "foreign-download-run", "foreign-artifact", "digest-warning"}:
                    download = next(step for step in promotion["steps"] if step.get("uses", "").startswith("actions/download-artifact@"))["with"]
                    if defect == "guessed-final-attempt":
                        del download["artifact-ids"]
                        download["name"] = "server-container-verification-${{ inputs.publication_run_id }}-${{ steps.producer.outputs.attempt }}"
                    if defect == "foreign-download-run": download["run-id"] = "123"
                    if defect == "foreign-artifact": download["artifact-ids"] = "${{ steps.producer.outputs.artifact_id }}"
                    if defect == "digest-warning": download["digest-mismatch"] = "warn"
                if defect in {"missing-producer-retention", "conditional-producer-retention"}:
                    cleanup = next(step for step in promotion["steps"] if step.get("id") == "ci_remove_promotion_registry_credentials_cc7de03d")
                    if defect == "missing-producer-retention": cleanup["run"] = "docker logout ghcr.io"
                    if defect == "conditional-producer-retention": cleanup["if"] = "success()"
                for phase, step in (("prepare", prepare), ("promote", promote)):
                    if defect == f"{phase}-github-token": step["env"]["GH_TOKEN"] = "${{ steps.protection-token.outputs.token }}"
                    if defect == f"{phase}-protection-token": del step["env"]["SOROTTE_PROTECTION_TOKEN"]
                    for label, argument in (("tooling", "--tooling-sha"), ("producer", "--publication-run-id"),
                                            ("version", "--version-tag"), ("digest", "--approved-digest")):
                        if defect == f"{phase}-{label}": step["run"] = step["run"].replace(argument, "--unbound-input")
                if defect == "foreign-initial-authority": promote["run"] = promote["run"].replace("target/promotion-authority/initial/authority.json", "target/foreign/authority.json")
                if defect == "missing-final-authority": promote["run"] = promote["run"].replace("--authority-dir", "--unbound-authority")
                if defect == "old-direct-promotion": promote["run"] = promote["run"].replace("container_promotion.py promote", "verify_server_container.py promote")
                with self.assertRaises(AssertionError):
                    self.assert_latest_promotion_contract(candidate)

    def test_gui_development_upload_checks_current_main_before_publication(self) -> None:
        workflow = yaml.load((REPO_ROOT / ".github/workflows/sorotte-gui-release.yml").read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
        step = next(step for step in workflow["jobs"]["publish-release"]["steps"]
                    if step.get("id") == "ci_publish_dev_package_to_github_release_fffdea3c")
        command = step["run"]
        self.assertNotIn("continue-on-error", step)
        guard = './scripts/assert-github-source-tip.ps1 -ExpectedSha $env:GITHUB_SHA -Remote origin -Branch main'
        self.assertIn(guard, command)
        self.assertLess(command.index(guard), command.index("gh release create"))

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
        self.assertEqual(sbom["syft-version"], "v1.51.1")
        self.assertEqual(sbom["upload-artifact"], "false")
        cosign = self.by_name["Install pinned Cosign"]["with"]
        self.assertEqual(cosign["cosign-release"], "v3.1.3")
        sign = self.by_name["Keylessly sign and attest the exact tested digest"][
            "run"
        ]
        for required in [
            '--annotations "sourceSha=$GITHUB_SHA"',
            '--annotations "workflowSourceSha=$WORKFLOW_SOURCE_SHA"',
        ]:
            self.assertIn(required, sign)
        self.assertEqual(sign.count("--annotations "), 2)
        self.assertNotIn("--annotation ", sign)
        verify = self.by_name["Verify keyless identity and workflow claims"]["run"]
        signer = "https://github.com/${{ github.repository }}/.github/workflows/publish-server-container.yml@${{ github.ref }}"
        self.assertEqual(self.by_name["Verify keyless identity and workflow claims"]["env"]["EXPECTED_IDENTITY"], signer)
        self.assertIn(signer, self.by_name["Compare every public tag, digest, config, SBOM, and signature subject"]["run"])
        for required in [
            "--certificate-identity",
            "--certificate-oidc-issuer",
            "--certificate-github-workflow-repository",
            "--certificate-github-workflow-sha",
            '--annotations "sourceSha=$GITHUB_SHA"',
            '--annotations "workflowSourceSha=$WORKFLOW_SOURCE_SHA"',
            "verify-attestation",
        ]:
            self.assertIn(required, verify)
        self.assertEqual(verify.count("--annotations "), 2)
        self.assertNotIn("--annotation ", verify)

    def test_public_comparison_is_anonymous_bounded_and_after_logout(self) -> None:
        names = [step["name"] for step in self.steps]
        logout = names.index("Remove registry credentials before public comparison")
        public = names.index(
            "Compare every public tag, digest, config, SBOM, and signature subject"
        )
        self.assertLess(logout, public)
        self.assertEqual(self.steps[logout]["if"], "${{ always() && inputs.publish }}")
        self.assertEqual(self.steps[public]["if"], "${{ success() && inputs.publish }}")
        command = self.steps[public]["run"]
        self.assertIn("verify-publication", command)
        self.assertIn("--expected-workflow-sha", command)

    def test_always_final_gate_and_evidence_retention_make_skips_fail(self) -> None:
        final = self.by_name["Enforce every container publication phase"]
        upload = self.by_name["Retain all container verification evidence"]
        self.assertEqual(final["if"], "${{ always() && inputs.publish }}")
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
        self.assertEqual(upload["with"]["retention-days"], "90")

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
