from __future__ import annotations

import copy
import pathlib
import sys
import unittest
from unittest import mock


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import playback_lifecycle_model as lifecycle  # noqa: E402


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
MODEL_PATH = REPO_ROOT / "coverage" / "playback-lifecycle.toml"
SYSTEM_COVERAGE_PATH = REPO_ROOT / "coverage" / "playback-lifecycle-system.toml"


class PlaybackLifecycleModelTests(unittest.TestCase):
    def setUp(self) -> None:
        self.model = lifecycle.load_toml(MODEL_PATH, "test lifecycle model")

    def validate(self, *, require_closed: bool = False) -> dict:
        return lifecycle.validate_model(
            self.model,
            repo_root=REPO_ROOT,
            require_closed=require_closed,
        )

    def test_repository_model_is_structurally_complete(self) -> None:
        summary = self.validate()

        self.assertEqual(summary["machine_count"], 11)
        self.assertEqual(summary["state_count"], 77)
        self.assertEqual(summary["transition_count"], 78)
        self.assertEqual(summary["invariant_count"], 15)
        self.assertEqual(summary["gap_count"], 8)
        self.assertEqual(summary["system_suite_count"], 8)
        self.assertEqual(summary["system_covered_transition_count"], 75)
        self.assertEqual(summary["open_gaps"], [])
        self.assertEqual(summary["transitions_missing_tiers"], {})

    def test_release_gate_rejects_open_gaps(self) -> None:
        self.model["gap"][0]["status"] = "open"
        with self.assertRaisesRegex(
            lifecycle.ModelError,
            "open lifecycle gaps remain",
        ):
            self.validate(require_closed=True)

    def test_unknown_top_level_key_is_rejected(self) -> None:
        self.model["unexpected"] = True

        with self.assertRaisesRegex(lifecycle.ModelError, "unexpected keys"):
            self.validate()

    def test_duplicate_transition_identity_is_rejected_across_machines(self) -> None:
        first = self.model["machine"][0]["transition"][0]["id"]
        self.model["machine"][1]["transition"][0]["id"] = first

        with self.assertRaisesRegex(
            lifecycle.ModelError,
            f"duplicate transition id {first}",
        ):
            self.validate()

    def test_unknown_behavior_evidence_is_rejected(self) -> None:
        transition = self.model["machine"][0]["transition"][0]
        transition["evidence"] = ["PL-NOT-REGISTERED-999"]

        with self.assertRaisesRegex(
            lifecycle.ModelError,
            "references unknown behaviors",
        ):
            self.validate()

    def test_critical_transition_must_require_all_proof_tiers(self) -> None:
        transition = self.model["machine"][0]["transition"][1]
        self.assertEqual(transition["risk"], "critical")
        transition["required_tiers"] = ["model", "seam"]

        with self.assertRaisesRegex(
            lifecycle.ModelError,
            "must require model, seam, and system",
        ):
            self.validate()

    def test_missing_proof_tier_requires_an_open_gap(self) -> None:
        transition = next(
            transition
            for machine in self.model["machine"]
            for transition in machine["transition"]
            if transition["id"] == "MEDIA-RESOLVE-001"
        )
        transition["covered_tiers"] = ["model"]
        transition["gaps"] = []

        with self.assertRaisesRegex(
            lifecycle.ModelError,
            "missing tiers .* without an open gap",
        ):
            self.validate()

    def test_system_registry_transition_must_be_emitted_by_a_named_suite(self) -> None:
        registry = lifecycle.load_toml(SYSTEM_COVERAGE_PATH, "test system registry")
        mutated = copy.deepcopy(registry)
        for suite in mutated["suite"]:
            suite["transitions"] = [
                transition
                for transition in suite["transitions"]
                if transition != "PLAYER-LOSS-001"
            ]
        original_load = lifecycle.load_toml

        def load(path: pathlib.Path, label: str) -> dict:
            if label == "system coverage registry":
                return mutated
            return original_load(path, label)

        with mock.patch.object(lifecycle, "load_toml", side_effect=load):
            with self.assertRaisesRegex(
                lifecycle.ModelError,
                "PLAYER-LOSS-001.*missing tiers.*system",
            ):
                self.validate(require_closed=True)

    def test_system_registry_rejects_unknown_transition(self) -> None:
        registry = lifecycle.load_toml(SYSTEM_COVERAGE_PATH, "test system registry")
        mutated = copy.deepcopy(registry)
        mutated["suite"][0]["transitions"].append("UNKNOWN-SYSTEM-999")
        original_load = lifecycle.load_toml

        def load(path: pathlib.Path, label: str) -> dict:
            if label == "system coverage registry":
                return mutated
            return original_load(path, label)

        with mock.patch.object(lifecycle, "load_toml", side_effect=load):
            with self.assertRaisesRegex(
                lifecycle.ModelError,
                "references unknown transitions.*UNKNOWN-SYSTEM-999",
            ):
                self.validate()

    def test_unreachable_state_is_rejected(self) -> None:
        machine = self.model["machine"][0]
        machine["state"].append(
            {
                "id": "orphaned",
                "title": "Orphaned",
                "kind": "transient",
                "description": "A deliberately unreachable test state.",
            }
        )

        with self.assertRaisesRegex(
            lifecycle.ModelError,
            "has unreachable states .*orphaned",
        ):
            self.validate()

    def test_unreferenced_gap_is_rejected(self) -> None:
        gap_id = "GAP-RELEASE-001"
        for machine in self.model["machine"]:
            for transition in machine["transition"]:
                transition["gaps"] = [
                    value for value in transition["gaps"] if value != gap_id
                ]

        with self.assertRaisesRegex(
            lifecycle.ModelError,
            f"unreferenced lifecycle gaps: .*{gap_id}",
        ):
            self.validate()


if __name__ == "__main__":
    unittest.main()
