from __future__ import annotations

import pathlib
import unittest

import yaml

from scripts.verification_tools import pins as verification_pins

VERIFICATION_PINS = verification_pins()


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
GATE_PATH = WORKFLOWS / "playback-lifecycle-release-gate.yml"
WINDOWS_SUITE_PATH = REPO_ROOT / ".github/actions/windows-playback-qualification/action.yml"
GUI_RELEASE_PATH = WORKFLOWS / "sorotte-gui-release.yml"
SERVER_RELEASE_PATH = WORKFLOWS / "sorotte-server-release.yml"
CONTAINER_RELEASE_PATH = WORKFLOWS / "publish-server-container.yml"

ACTION_USES = {
    "actions/checkout": f"actions/checkout@{VERIFICATION_PINS['actions']['actions/checkout']['sha']}",
    "actions/setup-python": f"actions/setup-python@{VERIFICATION_PINS['actions']['actions/setup-python']['sha']}",
    "actions/download-artifact": f"actions/download-artifact@{VERIFICATION_PINS['actions']['actions/download-artifact']['sha']}",
    "actions/upload-artifact": f"actions/upload-artifact@{VERIFICATION_PINS['actions']['actions/upload-artifact']['sha']}",
    "dtolnay/rust-toolchain": f"dtolnay/rust-toolchain@{VERIFICATION_PINS['actions']['dtolnay/rust-toolchain']['sha']}",
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
        cls.container_release = load_workflow(CONTAINER_RELEASE_PATH)
        jobs = cls.gate.get("jobs")
        if not isinstance(jobs, dict):
            raise AssertionError("release gate has no jobs mapping")
        cls.linux = jobs["linux-candidate"]
        cls.windows = jobs["windows-candidate"]
        cls.windows_suite = load_workflow(WINDOWS_SUITE_PATH)["runs"]
        cls.media_tools = named_step(cls.windows_suite, "Download and verify pinned supported Windows media tools")["env"]
        cls.complete = jobs["complete-candidate"]

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
        self.assertEqual(self.linux["runs-on"], "ubuntu-24.04")
        self.assertEqual(self.linux["env"]["CANDIDATE_SHA"], "${{ inputs.candidate_sha }}")
        self.assertEqual(
            self.windows["runs-on"],
            [
                "self-hosted",
                "Windows",
                "X64",
                "sorotte-native-interactive",
                "sorotte-ephemeral",
            ],
        )
        self.assertEqual(self.windows["env"]["CANDIDATE_SHA"], "${{ inputs.candidate_sha }}")
        bind = named_step(self.linux, "Bind Linux checkout to release candidate")
        self.assertEqual(
            normalized(bind["run"]),
            normalized(
                """
                test "$CANDIDATE_SHA" = "$GITHUB_SHA"
                test "$(git rev-parse 'HEAD^{commit}')" = "$CANDIDATE_SHA"
                test -z "$(git status --porcelain --untracked-files=all)"
                """
            ),
        )

    def test_gate_uses_reviewed_toolchain_and_release_mode_candidates(self) -> None:
        checkout = named_step(self.linux, "Checkout exact Sorotte candidate")
        self.assertEqual(checkout["uses"], ACTION_USES["actions/checkout"])
        self.assertEqual(checkout["with"]["ref"], "${{ inputs.candidate_sha }}")
        mpv_checkout = named_step(self.linux, "Checkout pinned official mpv source")
        self.assertEqual(mpv_checkout["uses"], ACTION_USES["actions/checkout"])
        self.assertEqual(mpv_checkout["with"]["repository"], "mpv-player/mpv")
        self.assertEqual(mpv_checkout["with"]["ref"], "${{ env.MPV_SOURCE_SHA }}")
        self.assertEqual(
            normalized(named_step(self.linux, "Build exact Linux release candidates")["run"]),
            "cargo build --locked --release -p sorotte-server -p sorotte-cli",
        )
        bundle = normalized(named_step(self.linux, "Seal immutable Linux candidate bundle")["run"])
        self.assertIn("playback_release_gate.py bundle", bundle)
        self.assertIn("--platform linux-x86_64", bundle)
        self.assertIn("--artifact server=target/release/sorotte-server", bundle)
        self.assertIn("--artifact client=target/release/sorotte-cli", bundle)
        for step_name, action in (
            ("Setup Rust", "dtolnay/rust-toolchain"),
            ("Setup Python", "actions/setup-python"),
        ):
            self.assertEqual(named_step(self.linux, step_name)["uses"], ACTION_USES[action])

    def test_gate_runs_system_candidate_stages_only_safe_evidence_and_fails_closed(self) -> None:
        system = named_step(self.linux, "Run ordinary and terminal exact-candidate lifecycle")
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

        loop = normalized(named_step(self.linux, "Run loop-boundary exact-candidate lifecycle")["run"])
        self.assertIn("--loop-at-end-of-playlist", loop)
        start = normalized(named_step(self.linux, "Run generated every-phase start-gate lifecycle")["run"])
        self.assertIn("playback_start_gate_system.py", start)
        self.assertIn('--candidate-sha "$CANDIDATE_SHA"', start)

        stage = named_step(self.linux, "Validate and stage privacy-safe release evidence")
        self.assertEqual(stage["if"], "always()")
        self.assertEqual(stage["continue-on-error"], "true")
        self.assertIn("playback-lifecycle-release-safe", stage["run"])

        closed = named_step(self.linux, "Require every declared lifecycle gap to be closed")
        self.assertEqual(closed["if"], "always()")
        self.assertEqual(closed["continue-on-error"], "true")
        self.assertEqual(
            normalized(closed["run"]),
            "python scripts/playback_lifecycle_model.py validate --model coverage/playback-lifecycle.toml --require-closed",
        )

        upload = named_step(self.linux, "Upload privacy-safe lifecycle release evidence")
        self.assertEqual(upload["uses"], ACTION_USES["actions/upload-artifact"])
        self.assertEqual(upload["with"]["path"], "target/verification/playback-lifecycle-release-safe")
        self.assertNotIn("playback-lifecycle-release\n", upload["with"]["path"])

        enforce = named_step(self.linux, "Enforce exact-candidate lifecycle release gate")
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

        attestation = normalized(
            named_step(self.linux, "Attest exact Linux suite and candidate digests")["run"]
        )
        self.assertIn("playback_release_gate.py attest-linux", attestation)
        self.assertIn("--system-report", attestation)
        self.assertIn("--loop-report", attestation)
        self.assertIn("--start-report", attestation)

    def test_windows_ffmpeg_pin_uses_the_reviewed_release_archive(self) -> None:
        # A fixed digest on the rolling gyan.dev URL fails after the next build.
        self.assertEqual(
            self.media_tools["FFMPEG_ARCHIVE_URL"],
            "https://github.com/GyanD/codexffmpeg/releases/download/"
            "9.0.1/"
            "ffmpeg-9.0.1-full_build.7z",
        )
        self.assertEqual(
            self.media_tools["FFMPEG_ARCHIVE_SHA256"],
            "4b9c814cb07a1f90d05b768ef4eb2abbf89af94bbb924df5b7dbd6e64e1e2b96",
        )
        self.assertEqual(
            self.media_tools["FFMPEG_BINARY_SHA256"],
            "57c56e369d5b4873b4d93fc1a1d833cb7cd8bc9325c14b05c34ce60b22842d8a",
        )

    def test_windows_gate_pins_tools_and_consumes_exact_gui_status_candidate(self) -> None:
        preflight = normalized(
            named_step(self.windows, "Attest ephemeral interactive Windows runner")["run"]
        )
        self.assertIn("sorotte-ephemeral-interactive-windows-v1", preflight)
        self.assertIn("SessionId", preflight)
        self.assertIn("Explorer shell", preflight)
        self.assertEqual(
            self.media_tools["MPV_ARCHIVE_SHA256"],
            "6abdd47422bba77f21072660b460f9cceef5cbd89f35b07903fff07451db7879",
        )
        self.assertEqual(
            self.media_tools["MPV_BINARY_SHA256"],
            "547aaba0dec693894a271e26e83e413f00bc4063b4a00dc8a11d1ee88c6eaefe",
        )
        tools = normalized(
            named_step(self.windows_suite, "Download and verify pinned supported Windows media tools")["run"]
        )
        self.assertIn("MPV_ARCHIVE_SHA256", tools)
        self.assertIn("FFMPEG_ARCHIVE_SHA256", tools)
        self.assertIn("FFMPEG_BINARY_SHA256", tools)
        build = normalized(
            named_step(self.windows_suite, "Build exact Windows release candidates and native driver")["run"]
        )
        for binary in ("sorotte-server", "sorotte-cli", "sorotte-gui", "sorotte-gui-updater"):
            self.assertIn(binary, build)
        bundle = normalized(
            named_step(self.windows_suite, "Seal immutable Windows candidate bundle")["run"]
        )
        self.assertIn("--platform windows-x86_64", bundle)
        self.assertIn("--artifact gui=target/release/sorotte-gui.exe", bundle)
        vertical = normalized("\n".join(step.get("run", "") for step in self.windows_suite["steps"]))
        self.assertEqual(vertical.count("gui-real-mpv-vertical.ps1"), 4)
        for switch in (
            "ExerciseFaultingHttpRecovery",
            "ExerciseStalledHttp",
            "ExerciseOwnedMpvRecovery",
        ):
            self.assertIn(switch, vertical)
        status = normalized(
            named_step(
                self.windows_suite,
                "Run exact second-client native participant status composition",
            )["run"]
        )
        self.assertIn("playback_status_system.py", status)
        self.assertIn("target/release/sorotte-gui.exe", status)
        attestation = normalized(
            named_step(self.windows_suite, "Attest exact Windows suite and candidate digests")["run"]
        )
        self.assertIn("playback_release_gate.py attest-windows", attestation)
        self.assertEqual(attestation.count("--vertical-summary"), 4)

    def test_native_current_server_regressions_require_every_case_to_pass(self) -> None:
        # These library tests require a manually provisioned native runner.
        # Their CI obligation is this shared PR/candidate action, rather than
        # an ordinary first.yml integration-test job in ignored-test policy.
        step = next(step for step in self.windows_suite["steps"] if step["id"] == "vertical")
        self.assertNotIn("continue-on-error", step)
        self.assertNotIn("if", step)
        run = normalized(step["run"])
        self.assertIn("$env:SOROTTE_TEST_MPV_BIN = $env:MPV_PATH", run)
        self.assertIn("cargo test --locked -p sorotte-gui --lib real_mpv_current_server -- --ignored --show-output --test-threads=1", run)
        for case in (
            "real_mpv_current_server_local_file_append_select_then_edit",
            "real_mpv_current_server_playing_participant_status",
            "real_mpv_current_server_direct_http_resume_latency",
            "real_mpv_current_server_direct_plex_resume_latency",
        ):
            self.assertIn(f"'{case}'", run)
        self.assertIn('if (-not (Select-String -LiteralPath $regressionLog -SimpleMatch "::$requiredCase ... ok" -Quiet))', run)
        self.assertIn('throw "Required current-server mpv case did not pass: $requiredCase"', run)
        self.assertIn("$regressionExit = $LASTEXITCODE", run)
        self.assertIn("if ($regressionExit -ne 0 -or -not", run)
        self.assertIn(r"test result: ok\. 4 passed; 0 failed; 0 ignored;", run)

    def test_pr_and_release_use_the_same_complete_windows_suite(self) -> None:
        native = load_workflow(WORKFLOWS / "gui-native-interactive.yml")["jobs"]["native_interactive"]
        for job, source in ((native, "${{ inputs.source_sha || github.sha }}"), (self.windows, "${{ inputs.candidate_sha }}")):
            calls = [step for step in job["steps"] if step.get("uses") == "./.github/actions/windows-playback-qualification"]
            self.assertEqual(len(calls), 1)
            self.assertEqual(calls[0]["with"]["candidate_sha"], source)
        self.assertEqual(self.windows_suite["using"], "composite")
        for step in self.windows_suite["steps"]:
            self.assertNotIn("continue-on-error", step)
            self.assertNotIn("if", step)
        binding = named_step(self.windows_suite, "Bind shared playback qualification to the checked-out candidate")
        self.assertIn('$source -cne $env:GITHUB_SHA', binding["run"])
        self.assertIn('$source -cne $env:REQUESTED_CANDIDATE_SHA', binding["run"])
        for name, command in (
            ("Validate closed lifecycle declarations and harness contracts",
             "python scripts/playback_lifecycle_model.py validate --model coverage/playback-lifecycle.toml --require-closed"),
            ("Build exact Windows release candidates and native driver",
             "cargo build --locked --release -p sorotte-server -p sorotte-cli"),
        ):
            self.assertIn(command + "\nif ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }\n",
                          named_step(self.windows_suite, name)["run"])

    def test_platform_attestations_are_composed_and_cover_the_checked_out_model(self) -> None:
        self.assertEqual(
            self.complete["needs"], ["linux-candidate", "windows-candidate"]
        )
        self.assertEqual(self.complete["runs-on"], "ubuntu-24.04")
        linux_upload = named_step(
            self.linux, "Upload Linux platform lifecycle attestation"
        )
        windows_upload = named_step(
            self.windows_suite, "Upload Windows platform lifecycle attestation"
        )
        for upload in (linux_upload, windows_upload):
            self.assertEqual(upload["uses"], ACTION_USES["actions/upload-artifact"])
            self.assertEqual(upload["with"]["if-no-files-found"], "error")
        for name in (
            "Download Linux platform lifecycle attestation",
            "Download Windows platform lifecycle attestation",
        ):
            self.assertEqual(
                named_step(self.complete, name)["uses"],
                ACTION_USES["actions/download-artifact"],
            )
        proof = normalized(
            named_step(self.complete, "Prove complete model transition coverage")["run"]
        )
        self.assertIn("playback_release_gate.py attest-complete", proof)
        self.assertIn("--model coverage/playback-lifecycle.toml", proof)
        self.assertIn("playback_lifecycle_model.py validate", proof)

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

        coordinated = yaml.load((REPO_ROOT / ".github/workflows/stable-release.yml").read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
        jobs = coordinated["jobs"]
        self.assertEqual(jobs["playback-lifecycle-release-gate"]["needs"], "authorize-source")
        self.assertEqual(jobs["playback-lifecycle-release-gate"]["with"]["candidate_sha"], "${{ github.sha }}")
        for consumer in ("server-archives", "gui-archive", "container-candidate"):
            needs = jobs[consumer]["needs"]
            self.assertIn("playback-lifecycle-release-gate", [needs] if isinstance(needs, str) else needs)
            self.assertIn("default-workspace", needs)
        for publisher in ("archives", "container"):
            self.assertEqual(jobs[publisher]["needs"], "publish-source")
        self.assertIn("workflow_call", self.server_release["on"])
        self.assertNotIn("push", self.server_release["on"])
        self.assertNotIn("push", self.container_release["on"])
        self.assertEqual(sum(job.get("uses", "").endswith("/playback-lifecycle-release-gate.yml") for job in jobs.values()), 1)


if __name__ == "__main__":
    unittest.main()
