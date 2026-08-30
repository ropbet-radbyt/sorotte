from __future__ import annotations

import json
import pathlib
import socket
import sys
import tempfile
import threading
import time
import unittest
import wave
from types import SimpleNamespace


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import playback_lifecycle_system as system  # noqa: E402


class PlaybackLifecycleSystemTests(unittest.TestCase):
    @staticmethod
    def wait_until(predicate, timeout: float = 2.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.01)
        raise AssertionError("timed out waiting for test condition")

    @staticmethod
    def receive_exact(connection: socket.socket, size: int) -> bytes:
        payload = bytearray()
        while len(payload) < size:
            chunk = connection.recv(size - len(payload))
            if not chunk:
                raise AssertionError("socket closed before the expected payload arrived")
            payload.extend(chunk)
        return bytes(payload)

    def test_role_usernames_are_unique_and_fit_the_server_contract(self) -> None:
        usernames = tuple(system.ROLE_USERNAMES.values())

        self.assertEqual(len(set(usernames)), len(usernames))
        self.assertTrue(all(1 <= len(username) <= 16 for username in usernames))

    def test_mpv_version_parser_accepts_reviewed_shapes_and_rejects_noise(self) -> None:
        self.assertEqual(system.parse_mpv_version("mpv 0.41.0 Copyright"), (0, 41, 0))
        self.assertEqual(system.parse_mpv_version("mpv v1.2.3-45-gabc"), (1, 2, 3))
        with self.assertRaisesRegex(ValueError, "semantic version"):
            system.parse_mpv_version("unrelated player 0.41.0")

    def test_generated_wav_is_deterministic_and_has_the_declared_shape(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            first = root / "first.wav"
            second = root / "second.wav"

            first_metadata = system.generate_pcm_wav(first, 1.25)
            second_metadata = system.generate_pcm_wav(second, 1.25)

            self.assertEqual(first_metadata, second_metadata)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with wave.open(str(first), "rb") as fixture:
                self.assertEqual(fixture.getnchannels(), 1)
                self.assertEqual(fixture.getsampwidth(), 2)
                self.assertEqual(fixture.getframerate(), 48_000)
                self.assertEqual(fixture.getnframes(), 60_000)

    def test_mpv_observer_emits_slots_and_never_emits_a_path_field(self) -> None:
        script = system.render_mpv_observer_lua(
            role="controller",
            trace_path=pathlib.Path(r"C:\private\trace.jsonl"),
            first_media_name="first-generated.wav",
            second_media_name="second-generated.wav",
        )

        self.assertIn('return "media-1"', script)
        self.assertIn('return "media-2"', script)
        self.assertIn('event = event_name', script)
        self.assertNotIn("path = mp.get_property", script)
        self.assertNotIn("uri =", script)

    def test_protocol_projection_redacts_playlist_entries_and_maps_participants(self) -> None:
        message = {
            "Set": {
                "playlistChange": {
                    "files": [
                        r"C:\Users\person\private-one.mkv",
                        "https://media.invalid/two?token=private",
                    ],
                    "user": system.ROLE_USERNAMES["controller"],
                }
            }
        }
        events, response = system.project_protocol_message(message)

        self.assertIsNone(response)
        self.assertEqual(
            events,
            [
                {
                    "event": "playlist-change",
                    "playlist_size": 2,
                    "set_by": "controller",
                }
            ],
        )
        serialized = json.dumps(events)
        self.assertNotIn("private-one", serialized)
        self.assertNotIn("token=private", serialized)

        status_message = {
            "State": {
                "sorotteParticipantStatusV1": {
                    "snapshot": {
                        "revision": 7,
                        "participants": {
                            system.ROLE_USERNAMES["controller"]: {
                                "availability": "fresh",
                                "playerConnection": "connected",
                                "phase": "playing",
                                "positionSeconds": 4.2,
                            },
                            "unrecognized-private-name": {"availability": "unsupported"},
                        },
                    }
                }
            }
        }
        events, _ = system.project_protocol_message(status_message)
        status = events[0]
        self.assertEqual(status["status_revision"], 7)
        self.assertEqual(status["participants"], ["controller", "other"])
        self.assertEqual(
            status["participant_views"]["controller"],
            {
                "availability": "fresh",
                "playerConnection": "connected",
                "phase": "playing",
            },
        )
        self.assertNotIn("unrecognized-private-name", json.dumps(status))
        self.assertNotIn("positionSeconds", json.dumps(status))

    def test_protocol_projection_combines_ping_and_ignore_obligations(self) -> None:
        events, response = system.project_protocol_message(
            {
                "State": {
                    "ping": {"latencyCalculation": 12.5},
                    "ignoringOnTheFly": {"server": 9},
                }
            }
        )

        self.assertEqual(
            events,
            [{"event": "server-ignore-observed", "server_ignore_counter": 9}],
        )
        self.assertEqual(response["State"]["ping"]["latencyCalculation"], 12.5)
        self.assertGreater(response["State"]["ping"]["clientLatencyCalculation"], 0.0)
        self.assertEqual(response["State"]["ignoringOnTheFly"], {"server": 9})

    def test_trace_contract_accepts_only_whitelisted_privacy_safe_fields(self) -> None:
        safe = {
            "schema_version": 1,
            "kind": system.TRACE_KIND,
            "sequence": 1,
            "elapsed_ms": 2,
            "correlation_id": "abc123",
            "source": "real-mpv",
            "role": "controller",
            "event": "file-loaded",
            "media_slot": "media-1",
            "paused": True,
        }
        system.assert_privacy_safe_trace_record(safe)

        with self.assertRaisesRegex(ValueError, "non-whitelisted"):
            system.assert_privacy_safe_trace_record({**safe, "path": "/private/movie.mkv"})
        with self.assertRaisesRegex(ValueError, "credential-bearing"):
            system.assert_privacy_safe_trace_record(
                {**safe, "detail": "https://media.invalid/a?token=private"}
            )
        with self.assertRaisesRegex(ValueError, "local path"):
            system.assert_privacy_safe_trace_record(
                {**safe, "detail": r"C:\Users\person\private.mkv"}
            )

    def test_error_redaction_removes_paths_and_query_credentials(self) -> None:
        redacted = system.redact_sensitive_text(
            r"failed C:\Users\person\movie.mkv and /home/person/other.mkv "
            "at https://media.invalid/file?X-Plex-Token=secret&mode=1 "
            "with password=hunter2"
        )

        self.assertNotIn("person", redacted)
        self.assertNotIn("secret", redacted)
        self.assertNotIn("hunter2", redacted)
        self.assertIn("<redacted-path>", redacted)
        self.assertIn("<redacted>", redacted)

    def test_jsonl_reader_ignores_only_an_incomplete_tail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "trace.jsonl"
            path.write_text('{"sequence":1}\n{"sequence":', encoding="utf-8")
            self.assertEqual(system.read_jsonl(path), [{"sequence": 1}])

            path.write_text('{"sequence":1}\nnot-json\n', encoding="utf-8")
            with self.assertRaises(json.JSONDecodeError):
                system.read_jsonl(path)

    def test_missing_prerequisite_is_an_artifact_backed_non_pass(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            artifact_dir = pathlib.Path(directory) / "artifacts"
            exit_code = system.main(
                [
                    "run",
                    "--server",
                    str(pathlib.Path(directory) / "missing-server"),
                    "--client",
                    str(pathlib.Path(directory) / "missing-client"),
                    "--mpv",
                    str(pathlib.Path(directory) / "missing-mpv"),
                    "--artifact-dir",
                    str(artifact_dir),
                    "--candidate-sha",
                    "a" * 40,
                    "--client-runtime-seconds",
                    "9",
                ]
            )

            self.assertEqual(exit_code, system.MISSING_PREREQUISITE_EXIT)
            report = json.loads((artifact_dir / "report.json").read_text(encoding="utf-8"))
            self.assertEqual(report["result"], "skipped")
            self.assertEqual(report["failure"]["stage"], "binary-preflight")
            self.assertEqual(report["candidate_sha"], "a" * 40)
            serialized = json.dumps(report)
            self.assertNotIn(directory, serialized)
            self.assertTrue((artifact_dir / "causal-trace.jsonl").is_file())
            safe_dir = pathlib.Path(directory) / "safe-evidence"
            self.assertEqual(
                system.main(
                    [
                        "stage-safe-evidence",
                        "--artifact-dir",
                        str(artifact_dir),
                        "--output-dir",
                        str(safe_dir),
                    ]
                ),
                0,
            )
            safe_manifest = json.loads(
                (safe_dir / "evidence-manifest.json").read_text(encoding="utf-8")
            )
            self.assertEqual(safe_manifest["result"], "skipped")

    def test_populated_artifact_directory_is_rejected_without_overwriting_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            artifact_dir = pathlib.Path(directory) / "artifacts"
            artifact_dir.mkdir()
            sentinel = artifact_dir / "report.json"
            sentinel.write_text('{"result":"failed","attempt":"first"}\n', encoding="utf-8")
            harness = system.PlaybackLifecycleHarness(
                server_path=pathlib.Path("missing-server"),
                client_path=pathlib.Path("missing-client"),
                mpv_path=pathlib.Path("missing-mpv"),
                artifact_dir=artifact_dir,
                candidate_sha="a" * 40,
                client_runtime_seconds=9.0,
            )

            self.assertEqual(harness.run(), 2)
            self.assertEqual(
                sentinel.read_text(encoding="utf-8"),
                '{"result":"failed","attempt":"first"}\n',
            )
            self.assertFalse((artifact_dir / "causal-trace.jsonl").exists())

    def test_safe_evidence_staging_excludes_raw_logs_scripts_and_media(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            artifact_dir = root / "artifacts"
            output_dir = root / "safe"
            artifact_dir.mkdir()
            report = {
                "schema_version": 1,
                "kind": system.REPORT_KIND,
                "result": "failed",
                "candidate_sha": "a" * 40,
                "artifacts": {
                    "causal_trace": "causal-trace.jsonl",
                    "player_traces": ["player-controller.jsonl"],
                    "process_logs": ["client-controller.stderr.log"],
                },
            }
            (artifact_dir / "report.json").write_text(
                json.dumps(report) + "\n", encoding="utf-8"
            )
            safe_trace = {
                "schema_version": 1,
                "kind": system.TRACE_KIND,
                "sequence": 1,
                "elapsed_ms": 2,
                "correlation_id": "abc123",
                "source": "harness",
                "role": "orchestrator",
                "event": "verification-failed",
                "detail": "seek-authority",
            }
            (artifact_dir / "causal-trace.jsonl").write_text(
                json.dumps(safe_trace) + "\n", encoding="utf-8"
            )
            player_trace = {
                "schema_version": 1,
                "kind": system.PLAYER_TRACE_KIND,
                "sequence": 1,
                "observed_at_ms": 3,
                "role": "controller",
                "event": "file-loaded",
                "media_slot": "media-1",
                "paused": True,
            }
            (artifact_dir / "player-controller.jsonl").write_text(
                json.dumps(player_trace) + "\n", encoding="utf-8"
            )
            (artifact_dir / "client-controller.stderr.log").write_text(
                r"private C:\Users\person\movie.mkv token=secret", encoding="utf-8"
            )
            (artifact_dir / "player-controller-observer.lua").write_text(
                r'local trace_path = "C:\Users\person\trace.jsonl"', encoding="utf-8"
            )
            generated = artifact_dir / "generated-media"
            generated.mkdir()
            (generated / "private.wav").write_bytes(b"raw-media")

            manifest = system.stage_privacy_safe_evidence(artifact_dir, output_dir)

            self.assertEqual(manifest["result"], "failed")
            self.assertEqual(
                {path.name for path in output_dir.iterdir()},
                {
                    "report.json",
                    "causal-trace.jsonl",
                    "player-controller.jsonl",
                    "evidence-manifest.json",
                },
            )
            staged_text = "\n".join(
                path.read_text(encoding="utf-8") for path in output_dir.iterdir()
            )
            self.assertNotIn("person", staged_text)
            self.assertNotIn("secret", staged_text)
            self.assertNotIn("raw-media", staged_text)

    def test_safe_evidence_staging_rejects_a_path_bearing_player_record(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            artifact_dir = root / "artifacts"
            artifact_dir.mkdir()
            report = {
                "kind": system.REPORT_KIND,
                "result": "passed",
                "artifacts": {
                    "player_traces": ["player-controller.jsonl"],
                },
            }
            (artifact_dir / "report.json").write_text(
                json.dumps(report) + "\n", encoding="utf-8"
            )
            (artifact_dir / "causal-trace.jsonl").write_text("", encoding="utf-8")
            unsafe = {
                "schema_version": 1,
                "kind": system.PLAYER_TRACE_KIND,
                "sequence": 1,
                "role": "controller",
                "event": "file-loaded",
                "path": "/home/person/private.mkv",
            }
            (artifact_dir / "player-controller.jsonl").write_text(
                json.dumps(unsafe) + "\n", encoding="utf-8"
            )

            with self.assertRaisesRegex(ValueError, "non-whitelisted"):
                system.stage_privacy_safe_evidence(artifact_dir, root / "safe")

    def test_protocol_fault_proxy_fragments_then_holds_and_releases_reconnect(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            upstream = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            upstream.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            upstream.bind(("127.0.0.1", 0))
            upstream.listen(2)
            upstream.settimeout(2.0)
            upstream_port = int(upstream.getsockname()[1])
            stop = threading.Event()

            def echo_server() -> None:
                try:
                    while not stop.is_set():
                        try:
                            connection, _ = upstream.accept()
                        except (socket.timeout, OSError):
                            continue
                        with connection:
                            while not stop.is_set():
                                try:
                                    payload = connection.recv(4096)
                                except OSError:
                                    break
                                if not payload:
                                    break
                                connection.sendall(payload)
                finally:
                    upstream.close()

            echo_thread = threading.Thread(target=echo_server, daemon=True)
            echo_thread.start()
            root = pathlib.Path(directory)
            ledger = system.TraceLedger(root / "trace.jsonl", "proxy-test")
            proxy = system.ProtocolFaultProxy(
                upstream_host="127.0.0.1",
                upstream_port=upstream_port,
                role="follower",
                ledger=ledger,
            )
            first = socket.create_connection(("127.0.0.1", proxy.port), timeout=2.0)
            first.sendall(b"first-frame")
            self.assertEqual(self.receive_exact(first, len(b"first-frame")), b"first-frame")
            self.wait_until(lambda: proxy.fragment_count > 2)
            accepted_before_cut = proxy.accepted_count
            upstream_before_cut = proxy.upstream_connection_count

            proxy.cut_and_hold()
            replacement = socket.create_connection(("127.0.0.1", proxy.port), timeout=2.0)
            replacement.settimeout(2.0)
            replacement.sendall(b"replacement-frame")
            self.wait_until(lambda: proxy.accepted_count > accepted_before_cut)
            self.assertEqual(proxy.upstream_connection_count, upstream_before_cut)

            proxy.resume()
            self.wait_until(
                lambda: proxy.upstream_connection_count > upstream_before_cut
            )
            self.assertEqual(
                self.receive_exact(replacement, len(b"replacement-frame")),
                b"replacement-frame",
            )
            replacement.close()
            first.close()
            proxy.close()
            stop.set()
            echo_thread.join(timeout=2.0)
            self.assertIsNone(proxy.error)

    def test_initial_player_verification_waits_for_delayed_proxy_accounting(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            harness = system.PlaybackLifecycleHarness(
                server_path=pathlib.Path("server"),
                client_path=pathlib.Path("client"),
                mpv_path=pathlib.Path("mpv"),
                artifact_dir=pathlib.Path(directory) / "artifacts",
                candidate_sha="a" * 40,
                client_runtime_seconds=9.0,
            )
            proxy = SimpleNamespace(
                role="follower",
                error=None,
                fragment_count=0,
                forwarded_bytes=0,
            )
            passed_checks: list[str] = []

            def start_client(role: str, _first: pathlib.Path, _second: pathlib.Path) -> None:
                if role == "follower":
                    harness.proxies[role] = proxy

            harness._start_client = start_client  # type: ignore[method-assign]
            harness._player_record = lambda *_args, **_kwargs: {}  # type: ignore[method-assign]
            harness._wait_player_state = lambda *_args, **_kwargs: None  # type: ignore[method-assign]
            harness._pass = (  # type: ignore[method-assign]
                lambda check_id, _detail: passed_checks.append(check_id)
            )

            def publish_accounting() -> None:
                proxy.fragment_count = 2
                proxy.forwarded_bytes = 1

            publisher = threading.Timer(0.1, publish_accounting)
            publisher.start()
            try:
                harness._verify_initial_players(
                    pathlib.Path("media-one.wav"),
                    pathlib.Path("media-two.wav"),
                )
            finally:
                publisher.join(timeout=1.0)

            self.assertEqual(proxy.fragment_count, 2)
            self.assertEqual(proxy.forwarded_bytes, 1)
            self.assertIn("follower-protocol-fragmentation-active", passed_checks)


if __name__ == "__main__":
    unittest.main()
