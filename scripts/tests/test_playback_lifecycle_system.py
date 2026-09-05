from __future__ import annotations

import json
import pathlib
import socket
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock
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

    def test_write_failure_reconnect_preserves_canonical_authority(self) -> None:
        baseline = {
            "event": "playstate",
            "paused": True,
            "position_seconds": 7.0,
            "set_by": "controller",
            "transport_revision": 27,
        }
        self.assertTrue(system.canonical_playstate_matches_snapshot(baseline, baseline))
        self.assertTrue(
            system.canonical_playstate_matches_snapshot(
                {**baseline, "position_seconds": 7.1}, baseline
            )
        )
        # Captured minimum-mpv trace: controller -> late is a periodic reporter
        # change at the same paused position and transport revision, not a seek.
        self.assertTrue(
            system.canonical_playstate_matches_snapshot(
                {**baseline, "set_by": "late", "do_seek": False}, baseline
            )
        )
        for change in (
            {"event": "playlist-index"},
            {"paused": False},
            {"paused": 1},
            {"position_seconds": 0},
            {"position_seconds": 7.76},
            {"position_seconds": float("nan")},
            {"position_seconds": float("inf")},
            {"position_seconds": True},
            {"transport_revision": 26},
            {"transport_revision": 28},
            {"transport_revision": None},
            {"transport_revision": True},
            {"transport_revision": 27.0},
            {"set_by": "late", "transport_revision": 28},
        ):
            with self.subTest(change=change):
                self.assertFalse(
                    system.canonical_playstate_matches_snapshot(
                        {**baseline, **change}, baseline
                    )
                )
        for change in (
            {"paused": 1}, {"position_seconds": None},
            {"transport_revision": None}, {"transport_revision": True},
            {"transport_revision": -1}, {"transport_revision": 27.0},
        ):
            with self.subTest(baseline_change=change):
                self.assertFalse(
                    system.canonical_playstate_matches_snapshot(
                        baseline, {**baseline, **change}
                    )
                )

    def test_partition_status_authority_accepts_absence_or_nonfresh_projection(self) -> None:
        fresh = {
            "event": "participant-status-snapshot",
            "participants": ["follower"],
            "participant_views": {
                "follower": {
                    "availability": "fresh",
                    "playerConnection": "connected",
                }
            },
        }
        self.assertFalse(
            system.participant_status_authority_withdrawn(fresh, "follower")
        )

        for availability in ("delayed", "stale", "awaitingReport", "unavailable"):
            event = json.loads(json.dumps(fresh))
            event["participant_views"]["follower"]["availability"] = availability
            self.assertTrue(
                system.participant_status_authority_withdrawn(event, "follower")
            )

        absent = {
            "event": "participant-status-snapshot",
            "participants": ["controller"],
            "participant_views": {"controller": {"availability": "fresh"}},
        }
        self.assertTrue(
            system.participant_status_authority_withdrawn(absent, "follower")
        )
        self.assertFalse(
            system.participant_status_authority_withdrawn(
                {"event": "playstate"}, "follower"
            )
        )

    def test_scheduled_write_failure_is_bound_to_the_exact_large_frame_lease(self) -> None:
        records = [
            {
                "transition": "TX-DELIVERY-001",
                "identities": {"frame-receipt": 40, "frame-bytes": 512},
            },
            {
                "transition": "TX-WRITTEN-001",
                "identities": {"frame-receipt": 40},
            },
            {
                "transition": "TX-DELIVERY-001",
                "identities": {
                    "frame-receipt": 41,
                    "frame-bytes": system.WRITE_FAILURE_FRAME_PAYLOAD_BYTES + 100,
                },
            },
            {
                "transition": "TX-FAIL-001",
                "identities": {"frame-receipt": 42},
            },
        ]

        delivery = system.bounded_playback_frame_delivery(
            records,
            after=0,
            minimum_frame_bytes=system.WRITE_FAILURE_FRAME_PAYLOAD_BYTES,
        )
        self.assertEqual(delivery["identities"]["frame-receipt"], 41)
        self.assertIsNone(
            system.exact_leased_frame_failure(
                records,
                after=0,
                frame_receipt=41,
            ),
            "another frame failing must not terminate the target lease",
        )

        failed = {
            "transition": "TX-FAIL-001",
            "identities": {"frame-receipt": 41},
        }
        self.assertIs(
            system.exact_leased_frame_failure(
                [*records, failed],
                after=0,
                frame_receipt=41,
            ),
            failed,
        )
        with self.assertRaisesRegex(ValueError, "completed before the scheduled reset"):
            system.exact_leased_frame_failure(
                [
                    *records,
                    {
                        "transition": "TX-WRITTEN-001",
                        "identities": {"frame-receipt": 41},
                    },
                ],
                after=0,
                frame_receipt=41,
            )

    def test_fault_frame_is_bounded_and_only_the_follower_gets_a_write_barrier(self) -> None:
        self.assertGreaterEqual(system.WRITE_FAILURE_FRAME_PAYLOAD_BYTES, 256 * 1024)
        self.assertLess(system.WRITE_FAILURE_FRAME_PAYLOAD_BYTES, 512 * 1024 - 16 * 1024)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            harness = system.PlaybackLifecycleHarness(
                server_path=root / "server",
                client_path=root / "client",
                mpv_path=root / "mpv",
                ffmpeg_path=root / "ffmpeg",
                artifact_dir=root / "artifacts",
                candidate_sha="a" * 40,
            )
            controller = harness._client_environment(
                role="controller",
                server_port=1234,
                media=root / "media.mkv",
                ipc_path="controller-ipc",
            )
            follower = harness._client_environment(
                role="follower",
                server_port=1234,
                media=root / "media.mkv",
                ipc_path="follower-ipc",
            )

        self.assertNotIn("SOROTTE_LIFECYCLE_WRITE_BARRIER", controller)
        self.assertEqual(
            follower["SOROTTE_LIFECYCLE_WRITE_BARRIER"],
            system.LIFECYCLE_WRITE_BARRIER_MODE,
        )

    def test_lifecycle_write_barrier_markers_are_derived_from_follower_evidence(self) -> None:
        ready, release = system.lifecycle_write_barrier_paths(
            pathlib.Path("evidence/client-follower.jsonl")
        )
        self.assertEqual(
            ready,
            pathlib.Path("evidence/client-follower.jsonl.leased-frame-ready"),
        )
        self.assertEqual(
            release,
            pathlib.Path("evidence/client-follower.jsonl.leased-frame-release"),
        )

    def test_mpv_version_parser_accepts_reviewed_shapes_and_rejects_noise(self) -> None:
        self.assertEqual(system.parse_mpv_version("mpv 0.41.0 Copyright"), (0, 41, 0))
        self.assertEqual(system.parse_mpv_version("mpv v1.2.3-45-gabc"), (1, 2, 3))
        with self.assertRaisesRegex(ValueError, "semantic version"):
            system.parse_mpv_version("unrelated player 0.41.0")

    def test_av_fixture_generation_requests_bitexact_video_and_audio(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            fixture = root / "fixture.mkv"
            observed_command: list[str] = []

            def fake_run(command: list[str], **_kwargs: object) -> SimpleNamespace:
                observed_command.extend(command)
                pathlib.Path(command[-1]).write_bytes(b"deterministic-av-fixture")
                return SimpleNamespace(returncode=0, stdout="", stderr="")

            with mock.patch.object(system.subprocess, "run", side_effect=fake_run):
                metadata = system.generate_av_fixture(
                    root / "ffmpeg",
                    fixture,
                    1.25,
                    color="red",
                )

            self.assertEqual(observed_command[0], str(root / "ffmpeg"))
            self.assertIn("color=c=red:s=320x180:r=10:d=1.25", observed_command)
            self.assertIn("anullsrc=r=48000:cl=mono:d=1.25", observed_command)
            self.assertIn("ffv1", observed_command)
            self.assertIn("pcm_s16le", observed_command)
            self.assertIn("+bitexact", observed_command)
            self.assertEqual(observed_command[-1], str(fixture))
            self.assertEqual(metadata["container"], "matroska")
            self.assertEqual(metadata["video_codec"], "ffv1")
            self.assertEqual(metadata["audio_codec"], "pcm_s16le")
            self.assertEqual(metadata["duration_seconds"], 1.25)
            self.assertEqual(metadata["sha256"], system.sha256_file(fixture))

    def test_av_fixture_generation_fails_closed_when_ffmpeg_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            with mock.patch.object(
                system.subprocess,
                "run",
                return_value=SimpleNamespace(returncode=1, stdout="", stderr="failure"),
            ):
                with self.assertRaisesRegex(ValueError, "FFmpeg failed"):
                    system.generate_av_fixture(
                        root / "ffmpeg",
                        root / "fixture.mkv",
                        1.0,
                        color="blue",
                    )

    def test_contained_player_failure_audit_counts_only_the_failure_marker(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            clean = root / "client-controller.stderr.log"
            failed = root / "client-follower.stderr.log"
            clean.write_text(
                "warning: ordinary startup compatibility notice\n",
                encoding="utf-8",
            )
            failed.write_text(
                "warning: external player step 'synchronize room pause state' failed\n"
                "warning: external player step 'apply canonical seek' failed\n",
                encoding="utf-8",
            )

            self.assertEqual(
                system.contained_player_failure_counts(
                    {"controller": clean, "follower": failed}
                ),
                {"follower": 2},
            )

    def test_terminal_playlist_boundary_accepts_one_canonical_pause_and_bounded_players(self) -> None:
        records = {
            "controller": [
                {"event": "pause-changed", "media_slot": "media-2", "paused": False},
                {"event": "end-file", "reason": "eof", "media_slot": "media-2"},
            ],
            "follower": [
                {"event": "pause-changed", "media_slot": "media-2", "paused": False},
                {
                    "event": "pause-changed",
                    "media_slot": "media-2",
                    "paused": True,
                    "position_seconds": 13.8,
                },
            ],
            "late": [
                {"event": "pause-changed", "media_slot": "media-2", "paused": False},
                {"event": "end-file", "reason": "eof", "media_slot": "media-2"},
            ],
        }

        system.validate_terminal_playlist_boundary(
            [
                {
                    "event": "playstate",
                    "paused": False,
                    "position_seconds": 11.0,
                    "transport_revision": 20,
                },
                {
                    "event": "playstate",
                    "paused": True,
                    "position_seconds": 14.0,
                    "do_seek": False,
                    "transport_revision": 21,
                },
                {
                    "event": "playstate",
                    "paused": True,
                    "position_seconds": 14.0,
                    "do_seek": False,
                    "transport_revision": 21,
                },
            ],
            records,
            terminal_duration_seconds=14.0,
            initial_transport_revision=20,
        )

    def test_terminal_playlist_boundary_rejects_selection_mutation_duplicate_or_reload(self) -> None:
        one_eof = {
            "controller": [
                {
                    "event": "end-file",
                    "reason": "eof",
                    "media_slot": "media-2",
                }
            ]
        }
        terminal_states = [
            {
                "event": "playstate",
                "paused": True,
                "position_seconds": 14.0,
                "do_seek": False,
                "transport_revision": 21,
            },
            {
                "event": "playstate",
                "paused": True,
                "position_seconds": 14.0,
                "do_seek": False,
                "transport_revision": 21,
            },
        ]
        with self.assertRaisesRegex(ValueError, "mutated canonical selection"):
            system.validate_terminal_playlist_boundary(
                [
                    {"event": "playlist-index", "playlist_index": 0},
                    *terminal_states,
                ],
                one_eof,
                terminal_duration_seconds=14.0,
                initial_transport_revision=20,
            )
        with self.assertRaisesRegex(ValueError, "2 final-item EOF"):
            system.validate_terminal_playlist_boundary(
                terminal_states,
                {"controller": one_eof["controller"] * 2},
                terminal_duration_seconds=14.0,
                initial_transport_revision=20,
            )
        with self.assertRaisesRegex(ValueError, "reloaded media"):
            system.validate_terminal_playlist_boundary(
                terminal_states,
                {"controller": [*one_eof["controller"], {"event": "file-loaded"}]},
                terminal_duration_seconds=14.0,
                initial_transport_revision=20,
            )

    def test_terminal_playlist_boundary_rejects_unbounded_or_drifting_authority(self) -> None:
        one_eof = {
            "controller": [
                {"event": "end-file", "reason": "eof", "media_slot": "media-2"}
            ]
        }
        with self.assertRaisesRegex(ValueError, "never committed"):
            system.validate_terminal_playlist_boundary(
                [{"event": "playstate", "paused": False, "transport_revision": 20}],
                one_eof,
                terminal_duration_seconds=14.0,
                initial_transport_revision=20,
            )
        with self.assertRaisesRegex(ValueError, "continued projecting"):
            system.validate_terminal_playlist_boundary(
                [
                    {
                        "event": "playstate",
                        "paused": True,
                        "position_seconds": 13.8,
                        "do_seek": False,
                        "transport_revision": 21,
                    },
                    {
                        "event": "playstate",
                        "paused": True,
                        "position_seconds": 14.2,
                        "do_seek": False,
                        "transport_revision": 21,
                    },
                ],
                one_eof,
                terminal_duration_seconds=14.0,
                initial_transport_revision=20,
            )

    def test_natural_eof_successor_boundary_accepts_fresh_stable_origin(self) -> None:
        canonical = [
            {"event": "playstate", "paused": False, "transport_revision": 20},
            {"event": "playlist-index", "playlist_index": 1},
            {
                "event": "playstate",
                "paused": True,
                "position_seconds": 0.0,
                "do_seek": False,
                "transport_revision": 22,
            },
            {
                "event": "playstate",
                "paused": True,
                "position_seconds": 0.0,
                "do_seek": False,
                "transport_revision": 22,
            },
        ]
        records = {
            "controller": [
                {"event": "end-file", "reason": "eof", "media_slot": "media-1"},
                {
                    "event": "file-loaded",
                    "media_slot": "media-2",
                    "paused": True,
                    "position_seconds": 0.0,
                },
            ],
            "follower": [
                {
                    "event": "file-loaded",
                    "media_slot": "media-2",
                    "paused": True,
                    "position_seconds": 0.0,
                }
            ],
            "late": [
                {
                    "event": "file-loaded",
                    "media_slot": "media-2",
                    "paused": True,
                    "position_seconds": 0.0,
                }
            ],
        }

        revision = system.validate_natural_eof_successor_boundary(
            canonical,
            records,
            previous_transport_revision=20,
        )

        self.assertEqual(revision, 22)

    def test_natural_eof_successor_boundary_accepts_last_to_first_loop(self) -> None:
        canonical = [
            {"event": "playstate", "paused": False, "transport_revision": 30},
            {"event": "playlist-index", "playlist_index": 0},
            {
                "event": "playstate",
                "paused": True,
                "position_seconds": 0.0,
                "do_seek": False,
                "transport_revision": 31,
            },
            {
                "event": "playstate",
                "paused": True,
                "position_seconds": 0.0,
                "do_seek": False,
                "transport_revision": 31,
            },
        ]
        records = {
            "controller": [
                {"event": "end-file", "reason": "eof", "media_slot": "media-2"},
                {
                    "event": "file-loaded",
                    "media_slot": "media-1",
                    "paused": True,
                    "position_seconds": 0.0,
                },
            ],
            "follower": [
                {
                    "event": "file-loaded",
                    "media_slot": "media-1",
                    "paused": True,
                    "position_seconds": 0.0,
                }
            ],
        }

        revision = system.validate_natural_eof_successor_boundary(
            canonical,
            records,
            previous_transport_revision=30,
            expected_playlist_index=0,
            predecessor_media_slot="media-2",
            successor_media_slot="media-1",
        )

        self.assertEqual(revision, 31)

    def test_natural_eof_successor_boundary_rejects_completed_media_authority(self) -> None:
        canonical = [
            {"event": "playlist-index", "playlist_index": 1},
            {
                "event": "playstate",
                "paused": True,
                "position_seconds": 0.0,
                "do_seek": False,
                "transport_revision": 22,
            },
            {
                "event": "playstate",
                "paused": True,
                "position_seconds": 0.0,
                "do_seek": False,
                "transport_revision": 22,
            },
        ]
        records = {
            "controller": [
                {"event": "end-file", "reason": "eof", "media_slot": "media-1"},
                {
                    "event": "file-loaded",
                    "media_slot": "media-2",
                    "paused": True,
                    "position_seconds": 0.0,
                },
                {
                    "event": "position-changed",
                    "media_slot": "media-2",
                    "paused": True,
                    "position_seconds": 10.0,
                },
            ]
        }

        with self.assertRaisesRegex(ValueError, "completed-media position crossed"):
            system.validate_natural_eof_successor_boundary(
                canonical,
                records,
                previous_transport_revision=20,
            )

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
        self.assertIn("local last_media_slot = nil", script)
        self.assertIn("last_media_slot = observed_media_slot", script)
        self.assertIn(
            'mp.register_event("end-file", function(event) emit("end-file", event.reason, true) end)',
            script,
        )
        self.assertIn("emitted_media_slot = last_media_slot", script)
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

    def test_protocol_projection_preserves_nullable_selection_semantics(self) -> None:
        selected, _ = system.project_protocol_message(
            {"Set": {"playlistIndex": {"index": 2, "user": system.ROLE_USERNAMES["controller"]}}}
        )
        empty, _ = system.project_protocol_message(
            {"Set": {"playlistIndex": {"index": None, "user": system.ROLE_USERNAMES["controller"]}}}
        )

        self.assertEqual(
            selected,
            [
                {
                    "event": "playlist-index",
                    "selection_present": True,
                    "playlist_index": 2,
                    "set_by": "controller",
                }
            ],
        )
        self.assertEqual(
            empty,
            [
                {
                    "event": "playlist-index",
                    "selection_present": False,
                    "playlist_index": None,
                    "set_by": "controller",
                }
            ],
        )

    def test_protocol_projection_redacts_room_names_and_maps_membership_roles(self) -> None:
        events, response = system.project_protocol_message(
            {
                "Set": {
                    "user": {
                        system.ROLE_USERNAMES["observer"]: {
                            "room": {"name": "private-room-name"}
                        },
                        "unknown-private-user": {
                            "room": {"name": "another-private-room"}
                        },
                    }
                }
            }
        )

        self.assertIsNone(response)
        self.assertEqual(
            events,
            [
                {"event": "room-membership-update", "member_role": "observer"},
                {"event": "room-membership-update", "member_role": "other"},
            ],
        )
        serialized = json.dumps(events)
        self.assertNotIn("private-room-name", serialized)
        self.assertNotIn("unknown-private-user", serialized)

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

    def test_protocol_projection_exposes_only_the_safe_transport_revision(self) -> None:
        events, response = system.project_protocol_message(
            {
                "State": {
                    "playstate": {
                        "position": 7.0,
                        "paused": True,
                        "setBy": system.ROLE_USERNAMES["controller"],
                        "sorotteTransportRevision": 19,
                    }
                }
            }
        )

        self.assertIsNone(response)
        self.assertEqual(events[0]["transport_revision"], 19)
        self.assertEqual(events[0]["set_by"], "controller")

    def test_client_protocol_projection_omits_identity_media_and_raw_extensions(self) -> None:
        events = system.project_client_protocol_message(
            {
                "State": {
                    "playstate": {
                        "position": 0.8,
                        "paused": True,
                        "doSeek": False,
                        "setBy": "private-user",
                        "sorotteTransportRevision": 5,
                    },
                    "sorotteParticipantStatusV1": {
                        "report": {"privatePath": r"C:\private\movie.mkv"}
                    },
                }
            }
        )

        self.assertEqual(
            events,
            [
                {
                    "event": "client-playstate",
                    "paused": True,
                    "position_seconds": 0.8,
                    "do_seek": False,
                    "transport_revision": 5,
                }
            ],
        )
        serialized = json.dumps(events)
        self.assertNotIn("private-user", serialized)
        self.assertNotIn("privatePath", serialized)

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
            "participant_health": ["controller:fresh:connected:playing"],
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
                    "--ffmpeg",
                    str(pathlib.Path(directory) / "missing-ffmpeg"),
                    "--artifact-dir",
                    str(artifact_dir),
                    "--candidate-sha",
                    "a" * 40,
                    "--allow-unverified-candidate",
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
                ffmpeg_path=pathlib.Path("missing-ffmpeg"),
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
                "prerequisites": {
                    "candidate_attestation": {
                        "verified": True,
                        "mode": "verified-clean-checkout",
                        "checkout_sha": "a" * 40,
                        "dirty": False,
                    }
                },
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

    def test_candidate_attestation_requires_matching_clean_checkout(self) -> None:
        candidate_sha = "a" * 40

        def harness(*, allow_unverified_candidate: bool = False):
            return system.PlaybackLifecycleHarness(
                server_path=pathlib.Path("server"),
                client_path=pathlib.Path("client"),
                mpv_path=pathlib.Path("mpv"),
                ffmpeg_path=pathlib.Path("ffmpeg"),
                artifact_dir=pathlib.Path("artifacts"),
                candidate_sha=candidate_sha,
                allow_unverified_candidate=allow_unverified_candidate,
            )

        clean_head = system.subprocess.CompletedProcess(
            args=[], returncode=0, stdout=f"{candidate_sha}\n", stderr=""
        )
        clean_status = system.subprocess.CompletedProcess(
            args=[], returncode=0, stdout="", stderr=""
        )
        with mock.patch.object(
            system.subprocess, "run", side_effect=[clean_head, clean_status]
        ):
            verified = harness()
            self.assertEqual(verified._resolve_candidate_sha(), candidate_sha)
            self.assertEqual(
                verified.prerequisites["candidate_attestation"],
                {
                    "verified": True,
                    "mode": "verified-clean-checkout",
                    "checkout_sha": candidate_sha,
                    "dirty": False,
                },
            )

        dirty_status = system.subprocess.CompletedProcess(
            args=[], returncode=0, stdout=" M source.rs\n", stderr=""
        )
        with mock.patch.object(
            system.subprocess, "run", side_effect=[clean_head, dirty_status]
        ):
            with self.assertRaisesRegex(system.MissingPrerequisite, "clean source checkout"):
                harness()._resolve_candidate_sha()

        different_head = system.subprocess.CompletedProcess(
            args=[], returncode=0, stdout=f"{'b' * 40}\n", stderr=""
        )
        with mock.patch.object(
            system.subprocess, "run", side_effect=[different_head, clean_status]
        ):
            development = harness(allow_unverified_candidate=True)
            self.assertEqual(development._resolve_candidate_sha(), candidate_sha)
            self.assertEqual(
                development.prerequisites["candidate_attestation"]["mode"],
                "development-unverified",
            )

    def test_passed_safe_evidence_rejects_unverified_candidate_attestation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            artifact_dir = root / "artifacts"
            artifact_dir.mkdir()
            report = {
                "kind": system.REPORT_KIND,
                "result": "passed",
                "prerequisites": {
                    "candidate_attestation": {
                        "verified": False,
                        "mode": "development-unverified",
                        "checkout_sha": "a" * 40,
                        "dirty": True,
                    }
                },
                "artifacts": {"player_traces": []},
            }
            (artifact_dir / "report.json").write_text(
                json.dumps(report) + "\n", encoding="utf-8"
            )
            (artifact_dir / "causal-trace.jsonl").write_text("", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "verified clean candidate"):
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
            first.sendall(b"first-frame\n")
            self.assertEqual(
                self.receive_exact(first, len(b"first-frame\n")), b"first-frame\n"
            )
            self.wait_until(lambda: proxy.fragment_count > 2)
            accepted_before_cut = proxy.accepted_count
            upstream_before_cut = proxy.upstream_connection_count

            proxy.cut_and_hold()
            replacement = socket.create_connection(("127.0.0.1", proxy.port), timeout=2.0)
            replacement.settimeout(2.0)
            replacement.sendall(b"replacement-frame\n")
            self.wait_until(lambda: proxy.accepted_count > accepted_before_cut)
            self.assertEqual(proxy.upstream_connection_count, upstream_before_cut)

            proxy.resume()
            self.wait_until(
                lambda: proxy.upstream_connection_count > upstream_before_cut
            )
            self.assertEqual(
                self.receive_exact(replacement, len(b"replacement-frame\n")),
                b"replacement-frame\n",
            )
            replacement.close()
            first.close()
            proxy.close()
            stop.set()
            echo_thread.join(timeout=2.0)
            self.assertIsNone(proxy.error)

    def test_protocol_fault_proxy_drops_only_armed_participant_status_frames(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            upstream = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            upstream.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            upstream.bind(("127.0.0.1", 0))
            upstream.listen(1)
            upstream.settimeout(2.0)
            upstream_port = int(upstream.getsockname()[1])
            stop = threading.Event()

            def echo_server() -> None:
                try:
                    connection, _ = upstream.accept()
                    with connection:
                        while not stop.is_set():
                            try:
                                payload = connection.recv(4096)
                            except OSError:
                                return
                            if not payload:
                                return
                            connection.sendall(payload)
                finally:
                    upstream.close()

            echo_thread = threading.Thread(target=echo_server, daemon=True)
            echo_thread.start()
            root = pathlib.Path(directory)
            ledger = system.TraceLedger(root / "trace.jsonl", "status-proxy-test")
            proxy = system.ProtocolFaultProxy(
                upstream_host="127.0.0.1",
                upstream_port=upstream_port,
                role="follower",
                ledger=ledger,
            )
            client = socket.create_connection(("127.0.0.1", proxy.port), timeout=2.0)
            client.settimeout(0.2)
            ordinary = b'{"State":{"playstate":{"position":1.0,"paused":true}}}\n'
            dropped = b'{"State":{"playstate":{"position":2.0,"paused":true},"sorotteParticipantStatusV1":{"report":{"reportSequence":7}}}}\n'
            recovered = b'{"State":{"sorotteParticipantStatusV1":{"report":{"reportSequence":8}}}}\n'

            client.sendall(ordinary)
            self.assertEqual(self.receive_exact(client, len(ordinary)), ordinary)
            proxy.drop_next_participant_status_reports()
            client.sendall(dropped)
            self.wait_until(lambda: proxy.participant_status_dropped_count == 1)
            expected_without_status = (
                b'{"State":{"playstate":{"position":2.0,"paused":true}}}\n'
            )
            self.assertEqual(
                self.receive_exact(client, len(expected_without_status)),
                expected_without_status,
            )

            client.sendall(recovered)
            self.assertEqual(self.receive_exact(client, len(recovered)), recovered)
            self.assertEqual(proxy.participant_status_report_count, 2)
            self.assertEqual(proxy.participant_status_forwarded_count, 1)
            self.assertEqual(proxy.participant_status_dropped_count, 1)

            records = system.read_jsonl(root / "trace.jsonl")
            self.assertTrue(
                any(
                    record.get("event") == "participant-status-report-dropped"
                    and record.get("source_sequence") == 7
                    for record in records
                )
            )
            self.assertTrue(
                any(
                    record.get("event") == "participant-status-report-forwarded"
                    and record.get("source_sequence") == 8
                    for record in records
                )
            )

            client.close()
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
                ffmpeg_path=pathlib.Path("ffmpeg"),
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

    def test_player_progress_is_relative_to_each_players_current_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            traces = {
                "controller": [
                    {"media_slot": "media-1", "position_seconds": 0.0, "paused": False},
                    {"media_slot": "media-1", "position_seconds": 1.0, "paused": False},
                ],
                "follower": [
                    {"media_slot": "media-1", "position_seconds": 0.1, "paused": False},
                ],
            }
            harness = system.PlaybackLifecycleHarness(
                server_path=pathlib.Path("server"),
                client_path=pathlib.Path("client"),
                mpv_path=pathlib.Path("mpv"),
                ffmpeg_path=pathlib.Path("ffmpeg"),
                artifact_dir=root / "artifacts",
                candidate_sha="a" * 40,
            )
            for role, records in traces.items():
                trace = root / f"{role}.jsonl"
                trace.write_text(
                    "".join(json.dumps(record) + "\n" for record in records),
                    encoding="utf-8",
                )
                harness.clients[role] = SimpleNamespace(player_trace=trace)

            later = {
                "controller": {
                    "media_slot": "media-1",
                    "position_seconds": 1.5,
                    "paused": False,
                },
                "follower": {
                    "media_slot": "media-1",
                    "position_seconds": 0.6,
                    "paused": False,
                },
            }
            observed: list[tuple[str, int]] = []

            def player_record(role: str, after: int, predicate: object, **_kwargs: object) -> dict[str, object]:
                observed.append((role, after))
                self.assertTrue(predicate(later[role]))  # type: ignore[operator]
                return later[role]

            harness._player_record = player_record  # type: ignore[method-assign]
            harness._wait_player_progress(
                ("controller", "follower"),
                {role: len(records) for role, records in traces.items()},
                media_slot="media-1",
                minimum_delta=0.5,
            )

            self.assertEqual(observed, [("controller", 2), ("follower", 1)])


if __name__ == "__main__":
    unittest.main()
