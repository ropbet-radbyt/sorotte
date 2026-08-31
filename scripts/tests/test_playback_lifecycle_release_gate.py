from __future__ import annotations

import pathlib
import unittest

import yaml


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
GATE_PATH = WORKFLOWS / "playback-lifecycle-release-gate.yml"
GUI_RELEASE_PATH = WORKFLOWS / "sorotte-gui-release.yml"
SERVER_RELEASE_PATH = WORKFLOWS / "sorotte-server-release.yml"

ACTION_USES = {
    "actions/checkout": "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
    "actions/setup-python": "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97",
    "actions/upload-artifact": "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
    "dtolnay/rust-toolchain": "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4",
}


def load_workflow(path: pathlib.Path) -> dict[str, object]:
    parsed = yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
    if not isinstance(parsed, dict):
        raise AssertionError(f"workflow is not a mapping: {path}")
    return parsed


def normalized(value: str) -> str:
    return " ".join(part for part in value.split() if part != "\\")


def named_step(job: dict[str, object], name: str) -> dict[str, object]:
    steps = job.get("steps")
    if not isinstance(steps, list):
        raise AssertionError("workflow job has no steps")
    matches = [step for step in steps if isinstance(step, dict) and step.get("name") == name]
    if len(matches) != 1:
        raise AssertionError(f"expected one step named {name!r}, found {len(matches)}")
    return matches[0]


class PlaybackLifecycleReleaseGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.gate = load_workflow(GATE_PATH)
        cls.gui_release = load_workflow(GUI_RELEASE_PATH)
        cls.server_release = load_workflow(SERVER_RELEASE_PATH)
        jobs = cls.gate.get("jobs")
        if not isinstance(jobs, dict):
            raise AssertionError("release gate has no jobs mapping")
        cls.job = jobs["playback-lifecycle-release-gate"]

    def test_gate_is_reusable_and_candidate_sha_is_explicit(self) -> None:
        trigger = self.gate["on"]
        self.assertEqual(
            trigger["workflow_call"]["inputs"]["candidate_sha"],
            {
                "description": "Full Sorotte commit SHA whose release candidates must be verified",
                "required": "true",
                "type": "string",
            },
        )
        self.assertEqual(self.job["runs-on"], "ubuntu-24.04")
        self.assertEqual(self.job["env"]["CANDIDATE_SHA"], "${{ inputs.candidate_sha }}")
        bind = named_step(self.job, "Bind checkout to release candidate")
        self.assertEqual(
            normalized(bind["run"]),
            normalized(
                """
                test "$CANDIDATE_SHA" = "$GITHUB_SHA"
                test "$(git rev-parse 'HEAD^{commit}')" = "$CANDIDATE_SHA"
                test -z "$(git status --porcelain --untracked-files=no)"
                """
            ),
        )

    def test_gate_uses_reviewed_toolchain_and_release_mode_candidates(self) -> None:
        checkout = named_step(self.job, "Checkout exact Sorotte candidate")
        self.assertEqual(checkout["uses"], ACTION_USES["actions/checkout"])
        self.assertEqual(checkout["with"]["ref"], "${{ inputs.candidate_sha }}")
        mpv_checkout = named_step(self.job, "Checkout pinned official mpv source")
        self.assertEqual(mpv_checkout["uses"], ACTION_USES["actions/checkout"])
        self.assertEqual(mpv_checkout["with"]["repository"], "mpv-player/mpv")
        self.assertEqual(mpv_checkout["with"]["ref"], "${{ env.MPV_SOURCE_SHA }}")
        self.assertEqual(
            normalized(named_step(self.job, "Build exact release-mode lifecycle candidates")["run"]),
            "cargo build --locked --release -p sorotte-server -p sorotte-cli",
        )
        for step_name, action in (
            ("Setup Rust", "dtolnay/rust-toolchain"),
            ("Setup Python", "actions/setup-python"),
        ):
            self.assertEqual(named_step(self.job, step_name)["uses"], ACTION_USES[action])

    def test_gate_runs_system_candidate_stages_only_safe_evidence_and_fails_closed(self) -> None:
        system = named_step(self.job, "Run exact-candidate multi-client real mpv lifecycle")
        self.assertEqual(system["id"], "playback_lifecycle_system")
        self.assertEqual(system["continue-on-error"], "true")
        self.assertEqual(
            normalized(system["run"]),
            normalized(
                """
                python scripts/playback_lifecycle_system.py run
                  --server target/release/sorotte-server
                  --client target/release/sorotte-cli
                  --mpv target/mpv-supported-release/build/mpv
                  --ffmpeg ffmpeg
                  --artifact-dir target/verification/playback-lifecycle-release
                  --candidate-sha "$CANDIDATE_SHA"
                """
            ),
        )

        stage = named_step(self.job, "Validate and stage privacy-safe release evidence")
        self.assertEqual(stage["if"], "always()")
        self.assertEqual(stage["continue-on-error"], "true")
        self.assertIn("playback-lifecycle-release-safe", stage["run"])

        closed = named_step(self.job, "Require every declared lifecycle gap to be closed")
        self.assertEqual(closed["if"], "always()")
        self.assertEqual(closed["continue-on-error"], "true")
        self.assertEqual(
            normalized(closed["run"]),
            "python scripts/playback_lifecycle_model.py validate --model coverage/playback-lifecycle.toml --require-closed",
        )

        upload = named_step(self.job, "Upload privacy-safe lifecycle release evidence")
        self.assertEqual(upload["uses"], ACTION_USES["actions/upload-artifact"])
        self.assertEqual(upload["with"]["path"], "target/verification/playback-lifecycle-release-safe")
        self.assertNotIn("playback-lifecycle-release\n", upload["with"]["path"])

        enforce = named_step(self.job, "Enforce exact-candidate lifecycle release gate")
        self.assertEqual(enforce["if"], "always()")
        self.assertEqual(
            enforce["env"],
            {
                "PLAYBACK_LIFECYCLE_SYSTEM_OUTCOME": "${{ steps.playback_lifecycle_system.outcome }}",
                "PLAYBACK_LIFECYCLE_EVIDENCE_OUTCOME": "${{ steps.playback_lifecycle_evidence.outcome }}",
                "PLAYBACK_LIFECYCLE_CLOSED_OUTCOME": "${{ steps.playback_lifecycle_closed.outcome }}",
            },
        )
        self.assertEqual(enforce["run"].count("= success"), 3)

    def test_gui_and_server_publication_depend_on_the_reusable_gate(self) -> None:
        gui_jobs = self.gui_release["jobs"]
        gui_gate = gui_jobs["playback-lifecycle-release-gate"]
        self.assertEqual(
            gui_gate["uses"],
            "./.github/workflows/playback-lifecycle-release-gate.yml",
        )
        self.assertEqual(gui_gate["with"]["candidate_sha"], "${{ github.sha }}")
        self.assertEqual(
            gui_jobs["publish-release"]["needs"],
            ["gui-release", "playback-lifecycle-release-gate"],
        )

        server_jobs = self.server_release["jobs"]
        server_gate = server_jobs["playback-lifecycle-release-gate"]
        self.assertEqual(
            server_gate["uses"],
            "./.github/workflows/playback-lifecycle-release-gate.yml",
        )
        self.assertEqual(server_gate["with"]["candidate_sha"], "${{ github.sha }}")
        self.assertEqual(
            server_jobs["server-release"]["needs"],
            "playback-lifecycle-release-gate",
        )


if __name__ == "__main__":
    unittest.main()
