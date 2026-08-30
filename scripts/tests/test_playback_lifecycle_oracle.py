from __future__ import annotations

import pathlib
import sys
import tempfile
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import playback_lifecycle_oracle as oracle  # noqa: E402


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
MODEL_PATH = REPO_ROOT / "coverage" / "playback-lifecycle.toml"


class PlaybackLifecycleOracleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.spec = oracle.load_spec(MODEL_PATH, repo_root=REPO_ROOT)
        self.oracle = oracle.CausalOracle(self.spec)
        self.sequence = 0

    def event(
        self,
        transition_id: str,
        *,
        subject: str,
        identities: dict[str, int],
        causes: list[str] | None = None,
        trigger: str = "model-witness",
        role: str = "oracle",
        disposition: str = "applied",
        deadline_expired: bool = False,
        event_id: str | None = None,
    ) -> dict:
        self.sequence += 1
        transition = self.spec.transitions[transition_id]
        before = self.oracle.current_state(subject, transition.machine)
        observed = transition.destination if disposition == "applied" else before
        return {
            "schema_version": 1,
            "event_id": event_id or f"event-{self.sequence}",
            "run_id": "unit-run",
            "monotonic_ns": self.sequence * 1_000_000,
            "emitter": "unit-oracle",
            "process_role": role,
            "subject": subject,
            "machine": transition.machine,
            "transition": transition_id,
            "causal_predecessors": causes or [],
            "identities": identities,
            "target_kind": "generated-fixture",
            "trigger": trigger,
            "authority_before": before,
            "authority_after": observed,
            "expected_effect": transition.destination,
            "observed_effect": observed,
            "disposition": disposition,
            "deadline_ms": 5_000,
            "deadline_expired": deadline_expired,
        }

    def apply(self, transition_id: str, **kwargs) -> dict:
        event = self.event(transition_id, **kwargs)
        self.oracle.apply_event(event)
        return event

    def test_every_transition_source_has_an_executable_witness(self) -> None:
        summary = oracle.execute_transition_witnesses(self.spec)

        self.assertEqual(summary["transition_count"], 78)
        self.assertEqual(summary["transition_source_count"], 217)
        self.assertEqual(summary["witness_count"], 217)
        self.assertLessEqual(summary["maximum_witness_steps"], 7)
        self.assertIn("TRANSPORT-PAUSE-001@playing", summary["witnesses"])
        self.assertIn("PLAYLIST-SELECT-001@selected", summary["witnesses"])

    def test_committed_replay_suite_covers_the_cross_machine_seed_histories(self) -> None:
        summary = oracle.execute_schedule_suite(
            REPO_ROOT / "fixtures" / "playback-lifecycle",
            repo_root=REPO_ROOT,
        )

        self.assertEqual(summary["status"], "passed")
        self.assertEqual(summary["schedule_count"], 2)
        self.assertEqual(summary["event_count"], 121)
        self.assertEqual(summary["covered_transition_count"], 56)
        self.assertEqual(
            [item["schedule_id"] for item in summary["schedules"]],
            ["full-lifecycle-seed", "late-join-reconnect-seed"],
        )

    def test_dependent_pause_requires_its_exact_frame_receipt(self) -> None:
        transport_ids = {
            "attachment_epoch": 1,
            "media_generation": 1,
            "load_attempt": 1,
        }
        self.apply(
            "TRANSPORT-LOAD-001",
            subject="client-a",
            identities=transport_ids,
        )
        self.apply(
            "TRANSPORT-PLAY-001",
            subject="client-a",
            identities=transport_ids,
        )
        begin = self.apply(
            "TX-BEGIN-001",
            subject="pause-transaction",
            identities={"command_sequence": 1},
            trigger="user-intent",
        )
        delivery = self.apply(
            "TX-DELIVERY-001",
            subject="pause-transaction",
            identities={"command_sequence": 1},
            causes=[begin["event_id"]],
            trigger="user-intent",
        )

        premature = self.event(
            "TRANSPORT-PAUSE-001",
            subject="client-a",
            identities={
                **transport_ids,
                "command_sequence": 1,
                "frame_receipt": 1,
            },
            causes=[delivery["event_id"]],
            trigger="canonical-mutation",
        )
        with self.assertRaisesRegex(
            oracle.OracleError,
            "dependent player effect precedes its exact frame receipt",
        ):
            self.oracle.apply_event(premature)

        written = self.apply(
            "TX-WRITTEN-001",
            subject="pause-transaction",
            identities={"command_sequence": 1, "frame_receipt": 1},
            causes=[delivery["event_id"]],
            trigger="user-intent",
        )
        pause = self.apply(
            "TRANSPORT-PAUSE-001",
            subject="client-a",
            identities={
                **transport_ids,
                "command_sequence": 1,
                "frame_receipt": 1,
            },
            causes=[written["event_id"]],
            trigger="canonical-mutation",
        )

        self.assertEqual(pause["authority_after"], "paused")

    def test_natural_completion_requires_correlated_active_attempt_end(self) -> None:
        snapshot = self.apply(
            "PLAYLIST-SNAPSHOT-POPULATED-001",
            subject="room",
            identities={"playlist_revision": 1},
            trigger="server-snapshot",
            role="server",
        )
        uncorrelated = self.event(
            "PLAYLIST-SELECT-001",
            subject="room",
            identities={
                "playlist_revision": 1,
                "playlist_index_revision": 1,
                "media_generation": 1,
                "load_attempt": 1,
            },
            causes=[snapshot["event_id"]],
            trigger="natural-completion",
            role="client",
        )
        with self.assertRaisesRegex(
            oracle.OracleError,
            "lacks a correlated transport end",
        ):
            self.oracle.apply_event(uncorrelated)

        transport_ids = {
            "attachment_epoch": 1,
            "media_generation": 1,
            "load_attempt": 1,
        }
        load = self.apply(
            "TRANSPORT-LOAD-001",
            subject="client-a",
            identities=transport_ids,
            trigger="player-observation",
            role="player",
        )
        playing = self.apply(
            "TRANSPORT-PLAY-001",
            subject="client-a",
            identities=transport_ids,
            causes=[load["event_id"]],
            trigger="player-observation",
            role="player",
        )
        ended = self.apply(
            "TRANSPORT-END-001",
            subject="client-a",
            identities=transport_ids,
            causes=[playing["event_id"]],
            trigger="player-observation",
            role="player",
        )
        selected = self.apply(
            "PLAYLIST-SELECT-001",
            subject="room",
            identities={
                "playlist_revision": 1,
                "playlist_index_revision": 1,
                "media_generation": 1,
                "load_attempt": 1,
            },
            causes=[ended["event_id"]],
            trigger="natural-completion",
            role="client",
        )

        self.assertEqual(selected["authority_after"], "index-pending")

    def test_stale_generation_is_ignored_but_cannot_apply(self) -> None:
        first = {"playlist_revision": 1, "media_generation": 1}
        second = {"playlist_revision": 1, "media_generation": 2}
        self.apply("MEDIA-SELECT-001", subject="client-a", identities=first)
        self.apply("MEDIA-CLEAR-001", subject="client-a", identities=first)
        self.apply("MEDIA-SELECT-001", subject="client-a", identities=second)

        stale_apply = self.event(
            "MEDIA-RESOLVE-001",
            subject="client-a",
            identities=first,
        )
        with self.assertRaisesRegex(oracle.OracleError, "applied stale identities"):
            self.oracle.apply_event(stale_apply)

        stale_ignored = self.event(
            "MEDIA-RESOLVE-001",
            subject="client-a",
            identities=first,
            disposition="ignored-stale",
        )
        self.oracle.apply_event(stale_ignored)
        self.assertEqual(
            self.oracle.current_state("client-a", "media-resolution"),
            "unresolved",
        )

    def test_server_playlist_authority_cannot_be_claimed_by_client_role(self) -> None:
        event = self.event(
            "PLAYLIST-SNAPSHOT-EMPTY-001",
            subject="room",
            identities={"playlist_revision": 1},
            role="client",
        )

        with self.assertRaisesRegex(
            oracle.OracleError,
            "cannot claim server-playlist authority",
        ):
            self.oracle.apply_event(event)

    def test_later_success_does_not_erase_first_failed_requirement(self) -> None:
        identities = {
            "attachment_epoch": 1,
            "media_generation": 1,
            "load_attempt": 1,
        }
        failed = self.apply(
            "TRANSPORT-LOAD-001",
            subject="client-a",
            identities=identities,
            disposition="failed",
            deadline_expired=True,
        )
        self.apply(
            "TRANSPORT-LOAD-001",
            subject="client-a",
            identities=identities,
        )

        summary = self.oracle.summary()
        self.assertEqual(summary["status"], "failed")
        self.assertEqual(summary["first_failure_event"], failed["event_id"])

    def test_generated_ledger_round_trips_through_strict_verifier(self) -> None:
        event = self.apply(
            "APP-LAUNCH-001",
            subject="client-a",
            identities={"process_run": 1},
            trigger="startup",
            role="client",
        )
        with tempfile.TemporaryDirectory() as temporary:
            ledger_path = pathlib.Path(temporary) / "ledger.jsonl"
            oracle.write_ledger(ledger_path, [event])
            loaded = oracle.load_ledger(ledger_path)

        replay = oracle.verify_ledger(loaded, self.spec)
        self.assertEqual(replay.summary()["event_count"], 1)
        self.assertEqual(
            replay.current_state("client-a", "application"),
            "starting",
        )

    def test_ledger_rejects_duplicate_json_keys(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            ledger_path = pathlib.Path(temporary) / "ledger.jsonl"
            ledger_path.write_text(
                '{"schema_version":1,"schema_version":1}\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(oracle.OracleError, "duplicate JSON key"):
                oracle.load_ledger(ledger_path)

    def test_delta_debugger_preserves_order_and_minimizes_failure(self) -> None:
        values = ["noise-a", "begin", "noise-b", "fault", "noise-c"]

        minimized = oracle.shrink_sequence(
            values,
            lambda candidate: "begin" in candidate and "fault" in candidate,
        )

        self.assertEqual(minimized, ["begin", "fault"])

    def test_state_aware_explorer_is_deterministic_and_transition_complete(self) -> None:
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            first_summary = oracle.explore_lifecycle(
                self.spec,
                seed=0x5A17,
                case_count=3,
                steps_per_case=24,
                failure_dir=pathlib.Path(first),
            )
            second_summary = oracle.explore_lifecycle(
                self.spec,
                seed=0x5A17,
                case_count=3,
                steps_per_case=24,
                failure_dir=pathlib.Path(second),
            )

        self.assertEqual(first_summary, second_summary)
        self.assertEqual(first_summary["status"], "passed")
        self.assertEqual(first_summary["transition_count"], 78)
        self.assertEqual(first_summary["transition_source_count"], 217)
        self.assertEqual(first_summary["checked_invariant_count"], 15)
        self.assertEqual(
            set(first_summary["checked_invariants"]),
            self.spec.invariants,
        )
        self.assertEqual(first_summary["invalid_history_probe_count"], 9)
        self.assertEqual(first_summary["random_walk_event_count"], 72)

    def test_state_aware_explorer_seed_changes_composed_histories(self) -> None:
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            first_summary = oracle.explore_lifecycle(
                self.spec,
                seed=11,
                case_count=2,
                steps_per_case=32,
                failure_dir=pathlib.Path(first),
            )
            second_summary = oracle.explore_lifecycle(
                self.spec,
                seed=12,
                case_count=2,
                steps_per_case=32,
                failure_dir=pathlib.Path(second),
            )

        self.assertNotEqual(
            first_summary["event_stream_sha256"],
            second_summary["event_stream_sha256"],
        )

    def test_state_aware_explorer_ledger_replays_strictly(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            ledger_path = root / "passing.jsonl"
            summary = oracle.explore_lifecycle(
                self.spec,
                seed=23,
                case_count=1,
                steps_per_case=16,
                failure_dir=root / "failures",
                ledger_path=ledger_path,
            )
            events = oracle.load_ledger(ledger_path)
            replay = oracle.verify_ledger(events, self.spec)

        self.assertEqual(replay.summary()["status"], "passed")
        self.assertEqual(len(events), summary["event_count"])
        self.assertEqual(
            oracle.event_stream_digest(events),
            summary["event_stream_sha256"],
        )

    def test_state_aware_explorer_shrinks_and_persists_first_divergence(self) -> None:
        def inject_schema_fault(
            case_index: int,
            step_index: int,
            event: dict,
        ) -> dict:
            if case_index == 1 and step_index == 5:
                event["fault_probe"] = "injected"
            return event

        with tempfile.TemporaryDirectory() as temporary:
            failure_dir = pathlib.Path(temporary) / "failures"
            with self.assertRaisesRegex(
                oracle.OracleError,
                "minimized 5 events to 1",
            ):
                oracle.explore_lifecycle(
                    self.spec,
                    seed=31,
                    case_count=1,
                    steps_per_case=16,
                    failure_dir=failure_dir,
                    event_mutator=inject_schema_fault,
                )

            ledger_paths = list(failure_dir.glob("*.jsonl"))
            metadata_paths = list(failure_dir.glob("*.json"))
            self.assertEqual(len(ledger_paths), 1)
            self.assertEqual(len(metadata_paths), 1)
            minimized = oracle.load_ledger(ledger_paths[0])
            self.assertEqual(len(minimized), 1)
            with self.assertRaisesRegex(oracle.OracleError, "fault_probe"):
                oracle.verify_ledger(minimized, self.spec)
            metadata = metadata_paths[0].read_text(encoding="utf-8")

        self.assertIn('"failure_signature": "schema"', metadata)
        self.assertIn('"minimized_event_count": 1', metadata)

    def test_event_schema_cannot_carry_raw_path_or_url_fields(self) -> None:
        event = self.event(
            "APP-LAUNCH-001",
            subject="client-a",
            identities={"process_run": 1},
        )
        event["raw_path"] = "C:/private/media.mkv"

        with self.assertRaisesRegex(oracle.OracleError, "unexpected keys .*raw_path"):
            self.oracle.apply_event(event)


if __name__ == "__main__":
    unittest.main()
