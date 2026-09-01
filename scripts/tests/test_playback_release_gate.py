from __future__ import annotations

import copy
import hashlib
import json
import pathlib
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import playback_release_gate as gate


SHA = "a" * 40
REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
MODEL_PATH = REPO_ROOT / "coverage" / "playback-lifecycle.toml"
SYSTEM_COVERAGE_PATH = REPO_ROOT / "coverage" / "playback-lifecycle-system.toml"


def lifecycle_summary(*transitions: str) -> dict[str, object]:
    return {
        "schema_version": 1,
        "kind": "sorotte-playback-lifecycle-evidence-validation",
        "result": "passed",
        "transitions": {transition: 1 for transition in transitions},
    }


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value), encoding="utf-8")


def materialize_bundle(root: Path, platform: str) -> dict[str, object]:
    files: dict[str, object] = {}
    suffix = ".exe" if platform == "windows-x86_64" else ""
    for role in sorted(gate.PLATFORM_ROLES[platform]):
        name = f"sorotte-{role}{suffix}"
        body = f"candidate-{platform}-{role}".encode()
        (root / name).write_bytes(body)
        files[role] = {
            "file_name": name,
            "size": len(body),
            "sha256": hashlib.sha256(body).hexdigest(),
        }
    manifest = {
        "schema_version": 1,
        "kind": gate.BUNDLE_KIND,
        "result": "passed",
        "candidate_sha": SHA,
        "platform": platform,
        "product_version": "0.2.8",
        "files": files,
    }
    write_json(root / "candidate-manifest.json", manifest)
    return manifest


def system_report(manifest: dict[str, object], *, loop: bool) -> dict[str, object]:
    required = gate.BASE_CHECKS | (gate.LOOP_CHECKS if loop else frozenset())
    report: dict[str, object] = {
        "schema_version": 1,
        "kind": "sorotte-playback-lifecycle-system",
        "result": "passed",
        "candidate_sha": SHA,
        "prerequisites": {
            "candidate_attestation": {
                "verified": True,
                "checkout_sha": SHA,
                "dirty": False,
                "mode": "exact-clean-head",
            },
            "server": {"sha256": manifest["files"]["server"]["sha256"]},
            "client": {"sha256": manifest["files"]["client"]["sha256"]},
        },
        "checks": [
            {"id": check, "status": "passed", "detail": "synthetic"}
            for check in sorted(required)
        ],
        "fault_schedule": {
            "actions": sorted(gate.FAULT_ACTIONS),
            "step_count": 12,
        },
        "lifecycle_summary": lifecycle_summary("APP-LAUNCH-001"),
    }
    if loop:
        report["playlist_policy"] = "loop-at-end"
    return report


def start_report(manifest: dict[str, object]) -> dict[str, object]:
    return {
        "schema_version": 1,
        "kind": "sorotte-playback-start-gate-system",
        "result": "passed",
        "candidate_sha": SHA,
        "candidate_binding": "exact-clean-head",
        "server": {"sha256": manifest["files"]["server"]["sha256"]},
        "phase_coverage": list(gate.START_PHASES),
        "transition_coverage": list(gate.START_TRANSITIONS),
        "scenario_coverage": list(gate.START_SCENARIOS),
        "lifecycle_summary": {
            **lifecycle_summary(*gate.START_TRANSITIONS),
            "cross_process_edge_count": len(gate.START_TRANSITIONS),
        },
    }


class CandidateBundleTests(unittest.TestCase):
    def test_validates_closed_exact_bundle_and_rejects_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = materialize_bundle(root, "linux-x86_64")
            self.assertEqual(gate.validate_bundle(manifest, root)["candidate_sha"], SHA)
            server = root / manifest["files"]["server"]["file_name"]
            server.write_bytes(b"mutated")
            with self.assertRaisesRegex(gate.ReleaseGateError, "differs"):
                gate.validate_bundle(manifest, root)

    def test_rejects_extra_file_and_missing_role(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = materialize_bundle(root, "linux-x86_64")
            (root / "unexpected.bin").write_bytes(b"x")
            with self.assertRaisesRegex(gate.ReleaseGateError, "inventory"):
                gate.validate_bundle(manifest, root)
            (root / "unexpected.bin").unlink()
            del manifest["files"]["client"]
            with self.assertRaisesRegex(gate.ReleaseGateError, "role inventory"):
                gate.validate_bundle(manifest, root)


class LinuxAttestationTests(unittest.TestCase):
    def test_requires_exact_candidate_digests_faults_and_playlist_modes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = materialize_bundle(root, "linux-x86_64")
            base_path = root / "base.json"
            loop_path = root / "loop.json"
            start_path = root / "start.json"
            write_json(base_path, system_report(manifest, loop=False))
            write_json(loop_path, system_report(manifest, loop=True))
            write_json(start_path, start_report(manifest))
            gate.validate_system_report(base_path, bundle_manifest=manifest, loop=False)
            gate.validate_system_report(loop_path, bundle_manifest=manifest, loop=True)
            gate.validate_start_report(start_path, manifest)

            broken = system_report(manifest, loop=False)
            broken["fault_schedule"]["actions"].remove("reset")
            write_json(root / "broken.json", broken)
            with self.assertRaisesRegex(gate.ReleaseGateError, "fault action"):
                gate.validate_system_report(
                    root / "broken.json", bundle_manifest=manifest, loop=False
                )

    def test_rejects_development_start_binding(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = materialize_bundle(root, "linux-x86_64")
            report = start_report(manifest)
            report["candidate_binding"] = "development-unverified"
            path = root / "start.json"
            write_json(path, report)
            with self.assertRaisesRegex(gate.ReleaseGateError, "exact candidate"):
                gate.validate_start_report(path, manifest)


class WindowsAttestationTests(unittest.TestCase):
    def test_status_projection_requires_all_exact_product_digests(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = materialize_bundle(root, "windows-x86_64")
            report = {
                "schema_version": 1,
                "kind": "sorotte-playback-status-system",
                "result": "passed",
                "candidate_sha": SHA,
                "checks": sorted(gate.STATUS_CHECKS),
                "prerequisites": {
                    role: {"sha256": manifest["files"][role]["sha256"]}
                    for role in ("server", "client", "gui")
                }
                | {"mpv": {"sha256": "b" * 64}},
                "projection": {"visible": True, "status_label": "Ready fresh"},
                "lifecycle_summary": lifecycle_summary("STATUS-FRESH-001"),
            }
            path = root / "status.json"
            write_json(path, report)
            gate.validate_status_report(path, manifest, "b" * 64)
            report["prerequisites"]["gui"]["sha256"] = "c" * 64
            write_json(root / "wrong.json", report)
            with self.assertRaisesRegex(gate.ReleaseGateError, "gui digest"):
                gate.validate_status_report(root / "wrong.json", manifest, "b" * 64)


class SystemCoverageRegistryTests(unittest.TestCase):
    def test_registry_exactly_assigns_every_required_system_transition(self) -> None:
        all_transitions, required_system = gate.model_transition_inventory(MODEL_PATH)
        _, suites = gate.model_system_suite_inventory(
            MODEL_PATH,
            all_transitions=all_transitions,
            required_system=required_system,
        )
        assigned = {
            transition
            for suite in suites.values()
            for transition in suite["transitions"]
        }
        self.assertEqual(assigned, required_system)

    def test_named_suite_rejects_missing_assigned_transition(self) -> None:
        all_transitions, required_system = gate.model_transition_inventory(MODEL_PATH)
        _, suites = gate.model_system_suite_inventory(
            MODEL_PATH,
            all_transitions=all_transitions,
            required_system=required_system,
        )
        assigned = list(suites["exact-gui-owned-process-recovery"]["transitions"])
        missing = assigned.pop()
        summary = lifecycle_summary(*assigned)
        with self.assertRaisesRegex(gate.ReleaseGateError, missing):
            gate.require_suite_coverage(
                "exact-gui-owned-process-recovery",
                summary,
                platform="windows-x86_64",
                suites=suites,
            )

    def test_media_missing_is_attributed_to_player_loss_not_resolution_failure(self) -> None:
        all_transitions, required_system = gate.model_transition_inventory(MODEL_PATH)
        _, suites = gate.model_system_suite_inventory(
            MODEL_PATH,
            all_transitions=all_transitions,
            required_system=required_system,
        )

        self.assertNotIn(
            "MEDIA-MISSING-001",
            suites["exact-gui-faulting-http"]["transitions"],
        )
        self.assertIn(
            "MEDIA-MISSING-001",
            suites["exact-gui-owned-process-recovery"]["transitions"],
        )

    def test_load_terminal_is_attributed_to_terminal_http_not_open_stall(self) -> None:
        all_transitions, required_system = gate.model_transition_inventory(MODEL_PATH)
        _, suites = gate.model_system_suite_inventory(
            MODEL_PATH,
            all_transitions=all_transitions,
            required_system=required_system,
        )

        self.assertIn(
            "LOAD-TERMINAL-001",
            suites["exact-gui-faulting-http"]["transitions"],
        )
        self.assertNotIn(
            "LOAD-TERMINAL-001",
            suites["exact-gui-stalled-http"]["transitions"],
        )


class CompleteGateTests(unittest.TestCase):
    def platform_gate(self, platform: str) -> dict[str, object]:
        _, required_system = gate.model_transition_inventory(MODEL_PATH)
        report: dict[str, object] = {
            "schema_version": 1,
            "kind": gate.PLATFORM_KIND,
            "result": "passed",
            "candidate_sha": SHA,
            "platform": platform,
            "candidate_manifest_sha256": "d" * 64,
            "candidate_files": {"server": {}},
            "closed_gaps": ["GAP-TRACE-001", "GAP-RELEASE-001"],
            "model_sha256": gate.sha256_file(MODEL_PATH),
            "system_coverage_sha256": gate.sha256_file(SYSTEM_COVERAGE_PATH),
            "required_system_transitions": sorted(required_system),
            "system_transition_coverage": sorted(required_system),
            "suite_reports": {"suite": "e" * 64},
            "claims": ["claim"],
        }
        if platform == "windows-x86_64":
            report["tool_digests"] = {"mpv": "f" * 64}
        return report

    def test_complete_gate_rejects_platform_gap_disagreement(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            linux = root / "linux.json"
            windows = root / "windows.json"
            write_json(linux, self.platform_gate("linux-x86_64"))
            windows_report = self.platform_gate("windows-x86_64")
            windows_report["closed_gaps"] = ["GAP-TRACE-001"]
            write_json(windows, windows_report)
            args = type(
                "Args",
                (),
                {
                    "candidate_sha": SHA,
                    "linux_gate": str(linux),
                    "windows_gate": str(windows),
                    "model": str(MODEL_PATH),
                    "output": str(root / "complete.json"),
                },
            )()
            with self.assertRaisesRegex(gate.ReleaseGateError, "disagree"):
                gate.attest_complete(args)

    def test_complete_gate_rejects_transition_missing_from_both_platforms(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            linux = root / "linux.json"
            windows = root / "windows.json"
            linux_report = self.platform_gate("linux-x86_64")
            windows_report = self.platform_gate("windows-x86_64")
            missing = linux_report["required_system_transitions"][0]
            linux_report["system_transition_coverage"].remove(missing)
            windows_report["system_transition_coverage"].remove(missing)
            write_json(linux, linux_report)
            write_json(windows, windows_report)
            args = type(
                "Args",
                (),
                {
                    "candidate_sha": SHA,
                    "linux_gate": str(linux),
                    "windows_gate": str(windows),
                    "model": str(MODEL_PATH),
                    "output": str(root / "complete.json"),
                },
            )()
            with self.assertRaisesRegex(
                gate.ReleaseGateError, "does not cover every required.*missing"
            ):
                gate.attest_complete(args)


if __name__ == "__main__":
    unittest.main()
