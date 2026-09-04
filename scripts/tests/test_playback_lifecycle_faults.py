from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import playback_lifecycle_faults as faults  # noqa: E402


def schedule_with(*steps: dict[str, object]) -> faults.FaultSchedule:
    return faults.FaultSchedule.parse(
        {
            "schema_version": 1,
            "kind": faults.SCHEDULE_KIND,
            "schedule_id": "unit-schedule",
            "seed": 17,
            "steps": list(steps),
        }
    )


class PlaybackLifecycleFaultTests(unittest.TestCase):
    def test_closed_schedule_validates_boundary_action_and_bounds(self) -> None:
        schedule = schedule_with(
            {
                "id": "delay-first-response",
                "boundary": "server-to-client-frame",
                "action": "delay",
                "occurrence": 1,
                "value": 25,
            },
            {
                "id": "reset-third-request",
                "boundary": "client-to-server-frame",
                "action": "reset",
                "occurrence": 3,
                "value": 1,
            },
        )
        self.assertEqual(len(schedule.steps), 2)
        with self.assertRaisesRegex(faults.FaultScheduleError, "invalid"):
            schedule_with(
                {
                    "id": "invalid-fragment",
                    "boundary": "player-process",
                    "action": "fragment",
                    "occurrence": 1,
                    "value": 1,
                }
            )
        with self.assertRaisesRegex(faults.FaultScheduleError, r"\[1, 5000\]"):
            schedule_with(
                {
                    "id": "unbounded-delay",
                    "boundary": "server-worker",
                    "action": "delay",
                    "occurrence": 1,
                    "value": 5001,
                }
            )

    def test_cursor_replays_exact_occurrence_order_and_records_trace(self) -> None:
        schedule = schedule_with(
            {
                "id": "fragment-second-request",
                "boundary": "client-to-server-frame",
                "action": "fragment",
                "occurrence": 2,
                "value": 3,
            },
            {
                "id": "stall-first-worker",
                "boundary": "server-worker",
                "action": "worker-stall",
                "occurrence": 1,
                "value": 10,
            },
        )
        with tempfile.TemporaryDirectory() as directory:
            trace_path = pathlib.Path(directory) / "fault-replay.jsonl"
            ledger = faults.FaultReplayLedger(trace_path, schedule.schedule_id)
            cursor = faults.FaultScheduleCursor(schedule, ledger=ledger)
            applied: list[str] = []
            self.assertIsNone(
                cursor.checkpoint("client-to-server-frame", lambda step: applied.append(step.id))
            )
            self.assertEqual(
                cursor.checkpoint(
                    "client-to-server-frame", lambda step: applied.append(step.id)
                ).id,
                "fragment-second-request",
            )
            self.assertEqual(
                cursor.checkpoint("server-worker", lambda step: applied.append(step.id)).id,
                "stall-first-worker",
            )
            cursor.assert_consumed()
            ledger.close()
            self.assertEqual(applied, ["fragment-second-request", "stall-first-worker"])
            records = faults.read_replay_trace(trace_path)
            self.assertEqual([record["step_id"] for record in records], applied)
            self.assertTrue(all(record["outcome"] == "applied" for record in records))

    def test_cursor_fails_closed_when_a_required_step_never_occurs(self) -> None:
        schedule = schedule_with(
            {
                "id": "missing-response",
                "boundary": "player-ipc-response",
                "action": "ipc-withhold",
                "occurrence": 2,
                "value": 100,
            }
        )
        cursor = faults.FaultScheduleCursor(schedule)
        cursor.checkpoint("player-ipc-response", lambda _step: None)
        with self.assertRaisesRegex(faults.FaultScheduleError, "missing-response"):
            cursor.assert_consumed()

    def test_delta_debugger_persists_only_the_reproducing_fault_core(self) -> None:
        schedule = schedule_with(
            {
                "id": "noise-a",
                "boundary": "client-worker",
                "action": "worker-stall",
                "occurrence": 1,
                "value": 1,
            },
            {
                "id": "required-reset",
                "boundary": "player-ipc-response",
                "action": "ipc-reset",
                "occurrence": 1,
                "value": 1,
            },
            {
                "id": "noise-b",
                "boundary": "server-worker",
                "action": "worker-stall",
                "occurrence": 1,
                "value": 1,
            },
        )
        attempts: list[tuple[str, ...]] = []

        def reproduces(candidate: faults.FaultSchedule) -> bool:
            ids = tuple(step.id for step in candidate.steps)
            attempts.append(ids)
            return "required-reset" in ids

        minimized = faults.shrink_fault_schedule(schedule, reproduces)
        self.assertEqual([step.id for step in minimized.steps], ["required-reset"])
        self.assertGreater(len(attempts), 1)

    def test_schedule_round_trip_is_atomic_and_rejects_extra_fields(self) -> None:
        schedule = schedule_with(
            {
                "id": "process-exit",
                "boundary": "player-process",
                "action": "ipc-process-exit",
                "occurrence": 1,
                "value": 1,
            }
        )
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "schedule.json"
            schedule.write_atomic(path)
            self.assertEqual(faults.FaultSchedule.read(path), schedule)
            raw = json.loads(path.read_text(encoding="utf-8"))
            raw["unexpected"] = True
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(faults.FaultScheduleError, "closed schema"):
                faults.FaultSchedule.read(path)


if __name__ == "__main__":
    unittest.main()
