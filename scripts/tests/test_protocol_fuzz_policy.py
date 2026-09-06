from __future__ import annotations

import copy
import contextlib
import hashlib
import importlib.util
import io
import json
import pathlib
import re
import shutil
import subprocess
import sys
import shlex
import tempfile
import tomllib
import types
import unittest
from unittest import mock
from typing import Any

import yaml


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "rust-fuzz.yml"
FUZZ_MANIFEST_PATH = REPO_ROOT / "fuzz" / "Cargo.toml"
FUZZ_LOCK_PATH = REPO_ROOT / "fuzz" / "Cargo.lock"
CLI_MANIFEST_PATH = REPO_ROOT / "crates" / "sorotte-cli" / "Cargo.toml"
CLI_LIB_PATH = REPO_ROOT / "crates" / "sorotte-cli" / "src" / "lib.rs"
CLI_PROTOCOL_IO_PATH = (
    REPO_ROOT / "crates" / "sorotte-cli" / "src" / "protocol_io.rs"
)
PLAYER_MPV_MANIFEST_PATH = (
    REPO_ROOT / "crates" / "sorotte-player-mpv" / "Cargo.toml"
)
PLAYER_MPV_LIB_PATH = REPO_ROOT / "crates" / "sorotte-player-mpv" / "src" / "lib.rs"
PLAYER_MPV_IPC_PATH = REPO_ROOT / "crates" / "sorotte-player-mpv" / "src" / "ipc.rs"
FUZZ_TARGET_PATH = REPO_ROOT / "fuzz" / "fuzz_targets" / "protocol_line.rs"
FRAMED_SESSION_TARGET_PATH = (
    REPO_ROOT / "fuzz" / "fuzz_targets" / "framed_session.rs"
)
MPV_FRAMED_TRANSCRIPT_TARGET_PATH = (
    REPO_ROOT / "fuzz" / "fuzz_targets" / "mpv_framed_transcript.rs"
)
FUZZ_RUNNER_PATH = REPO_ROOT / "fuzz" / "run_protocol_fuzz.py"
FUZZ_GITIGNORE_PATH = REPO_ROOT / "fuzz" / ".gitignore"

CORPUS_PATH = "crates/sorotte-protocol/tests/corpus/protocol_parser"
CORPUS_MANIFEST_PATH = REPO_ROOT / "coverage/fuzz-corpora.json"
CORPUS_MANIFEST = json.loads(CORPUS_MANIFEST_PATH.read_text(encoding="utf-8"))
APPLICABILITY_POLICY = json.loads((REPO_ROOT / "coverage/verification-lanes.json").read_text(encoding="utf-8"))
CORPUS_COUNTS = {target["id"]: len(target["files"]) for target in CORPUS_MANIFEST["targets"]}
CORPUS_FILE_COUNT = CORPUS_COUNTS["protocol_line"]
FRAMED_SESSION_CORPUS_PATH = "crates/sorotte-cli/tests/corpus/framed_session"
FRAMED_SESSION_CORPUS_FILE_COUNT = CORPUS_COUNTS["framed_session"]
FRAMED_SESSION_CORPUS_DIRECTORY = REPO_ROOT / FRAMED_SESSION_CORPUS_PATH
MPV_FRAMED_TRANSCRIPT_CORPUS_PATH = (
    "crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript"
)
MPV_FRAMED_TRANSCRIPT_CORPUS_FILE_COUNT = CORPUS_COUNTS["mpv_framed_transcript"]
MPV_FRAMED_TRANSCRIPT_CORPUS_DIRECTORY = (
    REPO_ROOT / MPV_FRAMED_TRANSCRIPT_CORPUS_PATH
)
FUZZ_TOOLCHAIN = "nightly-2026-07-29"
CARGO_FUZZ_INSTALL_COMMAND = [
    "cargo",
    f"+{FUZZ_TOOLCHAIN}",
    "install",
    "cargo-fuzz",
    "--version",
    "0.13.2",
    "--locked",
]
FUZZ_SECONDS_EXPRESSION = (
    "${{ (github.event_name == 'pull_request' || github.event_name == 'push') "
    "&& '45' || '900' }}"
)
FUZZ_OUTPUT_PATH = "target/fuzz-ci/protocol-line"
FUZZ_TARGET = "protocol_line"
FRAMED_SESSION_OUTPUT_PATH = "target/fuzz-ci/framed-session"
FRAMED_SESSION_TARGET = "framed_session"
MPV_FRAMED_TRANSCRIPT_OUTPUT_PATH = "target/fuzz-ci/mpv-framed-transcript"
MPV_FRAMED_TRANSCRIPT_TARGET = "mpv_framed_transcript"

PINNED_ACTIONS = {
    "actions/checkout": "3d3c42e5aac5ba805825da76410c181273ba90b1",
    "dtolnay/rust-toolchain": "4cda84d5c5c54efe2404f9d843567869ab1699d4",
    "actions/setup-python": "5fda3b95a4ea91299a34e894583c3862153e4b97",
    "actions/upload-artifact": "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
    "actions/download-artifact": "3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
    "taiki-e/install-action": "67729d5c413db75907f0ad1e39bb04b9c868ff60",
}
PRODUCERS = {
    "protocol-fuzz": (FUZZ_TARGET, CORPUS_PATH, FUZZ_OUTPUT_PATH, "sorotte-protocol-fuzz"),
    "framed-session-fuzz": (FRAMED_SESSION_TARGET, FRAMED_SESSION_CORPUS_PATH,
                            FRAMED_SESSION_OUTPUT_PATH, "sorotte-framed-session-fuzz"),
    "mpv-framed-transcript-fuzz": (MPV_FRAMED_TRANSCRIPT_TARGET, MPV_FRAMED_TRANSCRIPT_CORPUS_PATH,
                                    MPV_FRAMED_TRANSCRIPT_OUTPUT_PATH, "sorotte-mpv-framed-transcript-fuzz"),
}
ATTEMPT_SUFFIX = "-${{ github.run_id }}-${{ github.run_attempt }}"
BASE_EXPRESSION = "${{ github.event.pull_request.base.sha || github.event.before || 'HEAD^' }}"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def parse_workflow(text: str) -> dict[str, Any]:
    workflow = yaml.load(text, Loader=yaml.BaseLoader)
    require(isinstance(workflow, dict), "workflow must be a mapping")
    require(isinstance(workflow.get("jobs"), dict), "workflow jobs must be a mapping")
    return workflow


def named_step(job: dict[str, Any], name: str) -> dict[str, Any]:
    matches = [
        step
        for step in job.get("steps", [])
        if isinstance(step, dict) and step.get("name") == name
    ]
    require(len(matches) == 1, f"expected exactly one step named {name!r}")
    return matches[0]


def command_tokens(step: dict[str, Any]) -> list[str]:
    command = step.get("run")
    require(isinstance(command, str), "run step must contain a command string")
    return shlex.split(command, posix=True)


def assert_cargo_fuzz_installer(step: dict[str, Any], context: str) -> None:
    require(
        command_tokens(step) == CARGO_FUZZ_INSTALL_COMMAND,
        f"{context} cargo-fuzz installation must remain exact and locked",
    )
    require("uses" not in step, f"{context} must not use an unsupported installer action")
    require("with" not in step, f"{context} installer must not carry action inputs")


def command_step(job: dict[str, Any], prefix: list[str]) -> dict[str, Any]:
    matches = [step for step in job["steps"] if "run" in step
               and command_tokens(step)[:len(prefix)] == prefix]
    require(len(matches) == 1, f"expected exactly one command starting {prefix}")
    return matches[0]


def action_step(job: dict[str, Any], action: str) -> dict[str, Any]:
    matches = [step for step in job["steps"] if step.get("uses", "").startswith(action + "@")]
    require(len(matches) == 1, f"expected exactly one {action}")
    return matches[0]


def workflow_path_covers(path: str) -> bool:
    # A workflow trigger alone is insufficient: its selector must also require fuzz.
    scripts = str(REPO_ROOT / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    from scripts import verify
    return any(lane["id"] == "fuzz" and lane["selected"] for lane in verify.select([path], APPLICABILITY_POLICY))


def assert_workflow_contract(text: str) -> None:
    workflow = parse_workflow(text)
    require(workflow.get("permissions") == {"contents": "read"}, "read-only permissions")
    require(workflow.get("concurrency") == {
        "group": "sorotte-protocol-fuzz-${{ github.ref }}",
        "cancel-in-progress": "${{ github.event_name != 'schedule' && github.event_name != 'workflow_dispatch' }}",
    }, "fuzz concurrency contract changed")
    require(workflow.get("on") == {
        "pull_request": "", "push": {"branches": ["main"]}, "workflow_dispatch": "",
        "schedule": [{"cron": "45 3 * * 3"}],
    }, "always-present PR/main gate and weekly/manual qualification are required")
    require(workflow.get("env", {}).get("VERIFICATION_SHA") ==
            "${{ github.event.pull_request.head.sha || github.sha }}", "exact candidate SHA required")
    jobs = workflow["jobs"]
    require(set(jobs) == {"selection", "fuzz-required", *PRODUCERS}, "fuzz producer/gate graph changed")
    for job_id, job in jobs.items():
        require(job.get("runs-on") == "ubuntu-24.04", "reviewed Linux image required")
        for step in job["steps"]:
            require("continue-on-error" not in step and "continue-on-error" not in job, "failures must propagate")
            if "uses" in step:
                action, _, pin = step["uses"].partition("@")
                require(PINNED_ACTIONS.get(action) == pin, "every action must retain its reviewed commit pin")
            if "run" in step:
                require("if" not in step, "verification commands cannot be conditionally omitted")
            if step.get("uses", "").startswith("actions/upload-artifact@"):
                inputs = step.get("with", {})
                require(inputs.get("name", "").endswith(ATTEMPT_SUFFIX), "evidence must be immutable per attempt")
                require(inputs.get("if-no-files-found") == "error" and "overwrite" not in inputs,
                        "missing evidence and artifact overwrites must fail closed")
        checkout = action_step(job, "actions/checkout")["with"]
        require(checkout.get("ref") == "${{ env.VERIFICATION_SHA }}" and
                checkout.get("persist-credentials") == "false", "checkout must bind exact head without credentials")
        require(action_step(job, "actions/setup-python").get("with") == {"python-version": "3.11"}, "Python pin changed")
        if job_id in ("selection", "fuzz-required"):
            require(checkout.get("fetch-depth") == "0", "selection/gate require exact base history")
            require(job.get("timeout-minutes") == "5", "small gate timeout required")
    selection = jobs["selection"]
    require("if" not in selection and "needs" not in selection, "selection must always execute")
    require(selection.get("outputs") == {"selected": "${{ steps.select.outputs.selected }}"}, "selector output contract")
    command_step(selection, ["python", "scripts/verify.py", "preflight", "--phase", "static"])
    plan = command_step(selection, ["python", "scripts/verify.py", "plan"])
    require(command_tokens(plan) == ["python", "scripts/verify.py", "plan", "--base", "$BASE_SHA",
                                   "--head", "$VERIFICATION_SHA", "--output", "target/verification/plan.json"],
            "plan must bind base and head")
    require(plan.get("env") == {"BASE_SHA": BASE_EXPRESSION}, "plan base identity required")
    select = command_step(selection, ["python", "scripts/verify.py", "selected"])
    require(select.get("id") == "select" and command_tokens(select) == ["python", "scripts/verify.py", "selected",
        "--plan", "target/verification/plan.json", "--lane", "fuzz", "$FORCE", "--github-output", "$GITHUB_OUTPUT"],
        "selector must use the source-bound plan")
    require(select.get("env", {}).get("FORCE") ==
            "${{ (github.event_name == 'schedule' || github.event_name == 'workflow_dispatch') && '--force' || '' }}",
            "scheduled/manual runs must qualify the full campaign")
    require(action_step(selection, "actions/upload-artifact").get("with", {}).get("name") == "fuzz-plan" + ATTEMPT_SUFFIX,
            "source plan artifact identity")

    for job_id, (target, corpus, output, artifact) in PRODUCERS.items():
        job = jobs[job_id]
        require(job.get("needs") == "selection" and job.get("if") == "needs.selection.outputs.selected == 'true'",
                "only explicit applicability may omit a producer")
        require(job.get("timeout-minutes") == "25" and job.get("env") == {"FUZZ_SECONDS": FUZZ_SECONDS_EXPRESSION},
                "bounded event-specific duration required")
        require(action_step(job, "dtolnay/rust-toolchain").get("with") ==
                {"toolchain": FUZZ_TOOLCHAIN, "components": "rust-src"}, "exact ASan toolchain required")
        install = command_step(job, ["cargo", f"+{FUZZ_TOOLCHAIN}", "install"])
        assert_cargo_fuzz_installer(install, job_id)
        verify = command_step(job, ["test"])["run"]
        require('test "$(cargo fuzz --version)" = "cargo-fuzz 0.13.2"' in verify and
                f"rustc +{FUZZ_TOOLCHAIN} -vV" in verify, "runtime tool identity checks required")
        require(command_tokens(command_step(job, ["python", "-m", "unittest"])) ==
                ["python", "-m", "unittest", "scripts.tests.test_protocol_fuzz_policy", "-v"], "policy check required")
        require(command_tokens(command_step(job, ["python", "-m", "pip"])) ==
                ["python", "-m", "pip", "install", "--disable-pip-version-check", "-r", "requirements/ci-policy.txt"],
                "locked policy prerequisites required")
        build = command_step(job, ["cargo", f"+{FUZZ_TOOLCHAIN}", "fuzz", "build"])
        require(command_tokens(build) == ["cargo", f"+{FUZZ_TOOLCHAIN}", "fuzz", "build", "--fuzz-dir", "fuzz",
                                         "--sanitizer", "address", target], "ASan build target changed")
        run = command_step(job, ["python", "fuzz/run_protocol_fuzz.py"])
        expected = ["python", "fuzz/run_protocol_fuzz.py"]
        if target != FUZZ_TARGET: expected += ["--target", target]
        expected += ["--toolchain", FUZZ_TOOLCHAIN, "--source-sha", "${{ env.VERIFICATION_SHA }}", "--seconds",
                     "${FUZZ_SECONDS}", "--seed-corpus", corpus, "--corpus-manifest", "coverage/fuzz-corpora.json",
                     "--output-root", output]
        require(command_tokens(run) == expected, "source, target, reviewed corpus and budget must remain exact")
        require(job["steps"].index(install) < job["steps"].index(build) < job["steps"].index(run), "build/run order")
        uploads = [step for step in job["steps"] if step.get("with", {}).get("path") == output]
        require(len(uploads) == 1, "one retained artifact per target required")
        upload = uploads[0]
        require(upload.get("if") == "always()" and upload.get("with") == {
            "name": artifact + ATTEMPT_SUFFIX, "path": output, "if-no-files-found": "error", "retention-days": "14",
        }, "all failure evidence must upload without replacing past attempts")

    protocol = jobs["protocol-fuzz"]
    canary = command_step(protocol, ["python", "scripts/fuzz_tool_canary.py"])
    require(command_tokens(canary) == ["python", "scripts/fuzz_tool_canary.py", "--output", "target/fuzz-tool-canary"],
            "real pinned-tool canary required")
    replay = command_step(protocol, ["python", "scripts/fuzz_regressions.py"])
    require(command_tokens(replay) == ["python", "scripts/fuzz_regressions.py", "replay", "--output", "target/fuzz-regressions.json"],
            "retained product regression replay required")
    require(action_step(protocol, "taiki-e/install-action").get("with") ==
            {"tool": "cargo-nextest@0.9.137", "fallback": "none"}, "replay needs the pinned deterministic test runner")
    build = command_step(protocol, ["cargo", f"+{FUZZ_TOOLCHAIN}", "fuzz", "build"])
    require(protocol["steps"].index(canary) < protocol["steps"].index(replay) < protocol["steps"].index(build),
            "tool/replay canaries must precede the product campaign")
    evidence = [step for step in protocol["steps"] if step.get("with", {}).get("name") == "fuzz-tool-canary" + ATTEMPT_SUFFIX]
    require(len(evidence) == 1 and evidence[0].get("if") == "always()" and
            evidence[0]["with"]["path"].splitlines() == ["target/fuzz-tool-canary", "target/fuzz-regressions.json"],
            "tool and regression failures must retain evidence")

    gate = jobs["fuzz-required"]
    require(gate.get("name") == "fuzz-required" and gate.get("if") == "always()" and
            set(gate.get("needs", [])) == {"selection", *PRODUCERS}, "stable gate must observe every producer and selection")
    authority = command_step(gate, ["test"])
    require(command_tokens(authority) == ["test", "$SELECTION_RESULT", "=", "success"] and
            authority.get("env") == {"SELECTION_RESULT": "${{ needs.selection.result }}"}, "failed selection cannot become no-op")
    require(action_step(gate, "actions/download-artifact").get("with") ==
            {"name": "fuzz-plan" + ATTEMPT_SUFFIX, "path": "target/verification"}, "gate must use current-attempt plan")
    finalize = command_step(gate, ["python", "scripts/verify.py", "gate"])
    expected = ["python", "scripts/verify.py", "gate", "--lane", "fuzz", "--selected", "$SELECTED", "--plan",
                "target/verification/plan.json", "--base-sha", "$EXPECTED_BASE", "--source-sha", "$VERIFICATION_SHA"]
    expected_env = {"SELECTED": "${{ needs.selection.outputs.selected }}", "EXPECTED_BASE": BASE_EXPRESSION}
    for index, job_id in enumerate(PRODUCERS):
        expected += ["--expected-job", job_id, "--job-result", f"{job_id}=$RESULT_{index}"]
        expected_env[f"RESULT_{index}"] = "${{ needs['" + job_id + "'].result }}"
    expected += ["--output", "target/verification/fuzz-required.json"]
    require(command_tokens(finalize) == expected and finalize.get("env") == expected_env,
            "gate must validate source, applicability and every independent producer outcome")
    require(action_step(gate, "actions/upload-artifact").get("if") == "always()", "gate failure receipt must be retained")
    require("continue-on-error" not in text and "|| true" not in text, "workflow must not mask failure")


def load_runner() -> types.ModuleType:
    specification = importlib.util.spec_from_file_location(
        "sorotte_protocol_fuzz_runner", FUZZ_RUNNER_PATH
    )
    if specification is None or specification.loader is None:
        raise AssertionError("unable to load protocol fuzz runner")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


class ProtocolFuzzPolicyTests(unittest.TestCase):
    def test_workflow_is_bounded_pinned_and_fail_closed(self) -> None:
        assert_workflow_contract(WORKFLOW_PATH.read_text(encoding="utf-8"))

    def test_adversarial_workflow_weakening_is_rejected(self) -> None:
        original = WORKFLOW_PATH.read_text(encoding="utf-8")
        mutations = [
            original.replace("branches:\n      - main", "branches:\n      - '**'"),
            original.replace("&& '45' || '900'", "&& '45' || '1800'"),
            original.replace("timeout-minutes: 25", "timeout-minutes: 0"),
            original.replace("if: always()", "if: success()"),
            original.replace("if-no-files-found: error", "if-no-files-found: ignore"),
            original.replace(
                "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
                "actions/checkout@v4",
            ),
            original.replace("--sanitizer address", "--sanitizer none"),
            original.replace('--source-sha "${{ env.VERIFICATION_SHA }}"', "--source-sha bad"),
            original.replace("--corpus-manifest coverage/fuzz-corpora.json", "--corpus-manifest malicious.json"),
            original.replace("pull_request:", "pull_request:\n    paths: [docs/**]"),
            original.replace("--target framed_session", "--target protocol_line"),
            original.replace(
                "target/fuzz-ci/framed-session",
                "target/fuzz-ci/protocol-line",
            ),
            original.replace(
                "--target mpv_framed_transcript",
                "--target framed_session",
            ),
            original.replace(
                "--expected-job mpv-framed-transcript-fuzz",
                "--expected-job protocol-fuzz",
            ),
            original.replace(
                "target/fuzz-ci/mpv-framed-transcript",
                "target/fuzz-ci/framed-session",
            ),
        ]
        mutations += [
            original.replace("--base-sha \"$EXPECTED_BASE\"", "--base-sha HEAD"),
            original.replace("ref: ${{ env.VERIFICATION_SHA }}", "ref: main"),
            original.replace("-${{ github.run_attempt }}", ""),
            original.replace("--output target/fuzz-tool-canary", "--output target/fake-canary"),
            original.replace("scripts/fuzz_regressions.py replay", "scripts/fuzz_regressions.py validate"),
            original.replace("test \"$SELECTION_RESULT\" = success", "test true = true"),
            original.replace("--source-sha \"$VERIFICATION_SHA\"", "--source-sha HEAD"),
        ]
        for mutation in mutations:
            self.assertNotEqual(mutation, original, "adversarial workflow mutation must actually change source")
            with self.subTest(mutation=mutation):
                with self.assertRaises(AssertionError):
                    assert_workflow_contract(mutation)

    def test_fuzz_package_is_standalone_and_dependency_locked(self) -> None:
        manifest = tomllib.loads(FUZZ_MANIFEST_PATH.read_text(encoding="utf-8"))
        self.assertEqual(manifest["package"]["name"], "sorotte-fuzz")
        self.assertFalse(manifest["package"]["publish"])
        self.assertEqual(manifest["package"]["edition"], "2024")
        self.assertEqual(manifest["package"]["metadata"], {"cargo-fuzz": True})
        self.assertEqual(manifest["workspace"], {"members": ["."], "resolver": "2"})
        self.assertEqual(
            manifest["dependencies"],
            {
                "anyhow": "=1.0.104",
                "libfuzzer-sys": "=0.4.13",
                "serde": "=1.0.229",
                "serde_json": {
                    "version": "=1.0.151",
                    "features": ["float_roundtrip"],
                },
                "sorotte-cli": {
                    "path": "../crates/sorotte-cli",
                    "features": ["fuzz-support"],
                },
                "sorotte-client-app": {
                    "path": "../crates/sorotte-client-app",
                },
                "sorotte-player-api": {
                    "path": "../crates/sorotte-player-api",
                },
                "sorotte-player-mpv": {
                    "path": "../crates/sorotte-player-mpv",
                    "features": ["fuzz-support"],
                },
                "sorotte-protocol": {"path": "../crates/sorotte-protocol"},
                "tokio": {
                    "version": "=1.53.1",
                    "features": ["io-util", "rt", "sync"],
                },
            },
        )
        self.assertEqual(
            manifest["bin"],
            [
                {
                    "name": FUZZ_TARGET,
                    "path": "fuzz_targets/protocol_line.rs",
                    "test": False,
                    "doc": False,
                    "bench": False,
                },
                {
                    "name": FRAMED_SESSION_TARGET,
                    "path": "fuzz_targets/framed_session.rs",
                    "test": False,
                    "doc": False,
                    "bench": False,
                },
                {
                    "name": MPV_FRAMED_TRANSCRIPT_TARGET,
                    "path": "fuzz_targets/mpv_framed_transcript.rs",
                    "test": False,
                    "doc": False,
                    "bench": False,
                },
            ],
        )

        lock = tomllib.loads(FUZZ_LOCK_PATH.read_text(encoding="utf-8"))
        locked_versions = {
            (package["name"], package["version"]) for package in lock["package"]
        }
        self.assertIn(("anyhow", "1.0.104"), locked_versions)
        self.assertIn(("libfuzzer-sys", "0.4.13"), locked_versions)
        self.assertIn(("serde", "1.0.229"), locked_versions)
        self.assertIn(("serde_json", "1.0.151"), locked_versions)
        self.assertIn(("tokio", "1.53.1"), locked_versions)

    def test_target_exercises_every_public_decode_boundary_and_oracle(self) -> None:
        target = FUZZ_TARGET_PATH.read_text(encoding="utf-8")
        self.assertIn("#![no_main]", target)
        self.assertIn("fuzz_target!(|bytes: &[u8]|", target)
        self.assertIn("DEFAULT_MAX_PROTOCOL_LINE_BYTES", target)
        self.assertIn("std::str::from_utf8(bytes)", target)
        for public_function in (
            "decode_line(line)",
            "decode_message_line_items(line)",
            "decode_message_lines(line)",
            "decode_message_line(line)",
            "encode_line(&value)",
            "encode_message_line(message)",
        ):
            self.assertIn(public_function, target)
        self.assertIn("MapAccess", target)
        self.assertIn("IgnoredAny", target)
        self.assertIn("must preserve exact values", target)
        self.assertNotIn("matches_tc_protocol_004", target)
        self.assertNotIn("TC-PROTOCOL-004", target)
        self.assertNotIn("top_level_key_order", target)

    def test_framed_session_target_uses_real_reader_and_application_state(self) -> None:
        target = FRAMED_SESSION_TARGET_PATH.read_text(encoding="utf-8")
        self.assertIn("#![no_main]", target)
        self.assertIn("fuzz_target!(|bytes: &[u8]|", target)
        self.assertIn("InboundProtocolLineReader", target)
        self.assertIn("MAX_INBOUND_PROTOCOL_LINE_BYTES", target)
        self.assertIn("ClientApplication::with_default_session", target)
        self.assertIn("runtime.apply_protocol_line(", target)
        self.assertIn("read_with_one_cancellation", target)
        self.assertIn("ScheduledReader", target)
        self.assertIn("reference_outcome", target)
        self.assertIn("assert_session_invariants", target)
        self.assertIn("session.user_participant_status_at", target)
        self.assertIn("ParticipantStatusFreshness::Stale", target)
        self.assertIn("ParticipantStatusAvailability::Unsupported", target)
        self.assertIn("ParticipantStatusCorrelation::Exact", target)
        self.assertIn("position_sample_age_ms.is_some()", target)
        self.assertIn("input-derived frame bound", target)
        self.assertIn("framing schedules must preserve real session", target)
        self.assertNotIn("TcpStream", target)
        self.assertNotIn("UdpSocket", target)
        self.assertNotIn("std::net", target)

    def test_framed_session_support_is_a_feature_gated_exact_reexport(self) -> None:
        manifest = tomllib.loads(CLI_MANIFEST_PATH.read_text(encoding="utf-8"))
        self.assertEqual(manifest["features"]["fuzz-support"], [])
        library = CLI_LIB_PATH.read_text(encoding="utf-8")
        self.assertIn('#[cfg(feature = "fuzz-support")]', library)
        self.assertIn("pub mod fuzz_support", library)
        self.assertIn("InboundProtocolLineReader", library)
        self.assertIn("MAX_INBOUND_PROTOCOL_LINE_BYTES", library)
        protocol_io = CLI_PROTOCOL_IO_PATH.read_text(encoding="utf-8")
        self.assertIn(
            "MAX_INBOUND_PROTOCOL_LINE_BYTES: usize = "
            "SOROTTE_MAX_PROTOCOL_LINE_BYTES",
            protocol_io,
        )
        self.assertIn("pub struct InboundProtocolLineReader", protocol_io)
        self.assertNotIn("cfg(feature = \"fuzz-support\")", protocol_io)

    def test_mpv_target_uses_only_the_feature_gated_in_memory_worker_seam(self) -> None:
        target = MPV_FRAMED_TRANSCRIPT_TARGET_PATH.read_text(encoding="utf-8")
        self.assertIn("#![no_main]", target)
        self.assertIn("fuzz_target!(|bytes: &[u8]|", target)
        self.assertIn("run_in_memory_mpv_ipc_fuzz_case", target)
        self.assertIn("reference_run(&payload, end)", target)
        self.assertIn("MpvTranscript::new(records)", target)
        self.assertIn("replay_partitioned", target)
        self.assertIn("MpvLifecycleVerificationHarness::new()", target)
        self.assertIn("attachment replacement must fence the prior attempt", target)
        self.assertIn("successive logical loads must use distinct media generations", target)
        for forbidden in (
            "TcpStream",
            "UdpSocket",
            "std::net",
            "Command::new",
            "with_json_ipc",
            "connect_json_ipc",
        ):
            self.assertNotIn(forbidden, target)

    def test_mpv_worker_seam_is_exact_feature_gated_and_not_normally_exported(
        self,
    ) -> None:
        manifest = tomllib.loads(PLAYER_MPV_MANIFEST_PATH.read_text(encoding="utf-8"))
        self.assertEqual(manifest["features"]["fuzz-support"], ["test-support"])
        library = PLAYER_MPV_LIB_PATH.read_text(encoding="utf-8")
        self.assertIn('#[cfg(feature = "fuzz-support")]', library)
        self.assertIn("pub mod fuzz_support", library)
        self.assertIn("run_in_memory_mpv_ipc_fuzz_case", library)
        ipc = PLAYER_MPV_IPC_PATH.read_text(encoding="utf-8")
        self.assertIn("read_line_with(&mut self.read_buffer, line", ipc)
        self.assertIn("worker.send_command(", ipc)
        self.assertIn('json!(["get_property", "pause"])', ipc)
        self.assertIn("This deliberately narrow seam", ipc)
        self.assertNotIn("pub mod ipc", library)

    def test_mpv_seed_corpus_is_exact_direct_and_covers_script_end_modes(self) -> None:
        entries = sorted(MPV_FRAMED_TRANSCRIPT_CORPUS_DIRECTORY.iterdir())
        self.assertEqual(len(entries), MPV_FRAMED_TRANSCRIPT_CORPUS_FILE_COUNT)
        self.assertTrue(
            all(entry.is_file() and not entry.is_symlink() for entry in entries)
        )
        payloads = [entry.read_bytes() for entry in entries]
        self.assertTrue(all(len(payload) >= 4 for payload in payloads))
        self.assertEqual({payload[0] % 4 for payload in payloads}, {0, 1, 2, 3})
        self.assertEqual({payload[1] % 5 for payload in payloads}, {0, 1, 2, 3, 4})
        self.assertTrue(any(b'"event"' in payload for payload in payloads))
        self.assertTrue(any(b'"request_id":2' in payload for payload in payloads))
        self.assertTrue(any(payload.rstrip().endswith(b'{"event":') for payload in payloads))

    def test_framed_session_seed_corpus_is_direct_and_covers_control_modes(self) -> None:
        entries = sorted(FRAMED_SESSION_CORPUS_DIRECTORY.iterdir())
        self.assertEqual(len(entries), FRAMED_SESSION_CORPUS_FILE_COUNT)
        self.assertTrue(
            all(entry.is_file() and not entry.is_symlink() for entry in entries)
        )
        payloads = [entry.read_bytes() for entry in entries]
        self.assertTrue(
            any(entry.name == "participant-status-uncorrelated-offset.txt" for entry in entries)
        )
        ordinary = [
            payload
            for payload in payloads
            if not payload.startswith(b"!SEAM")
        ]
        seam = [payload for payload in payloads if payload.startswith(b"!SEAM")]
        self.assertTrue(all(len(payload) >= 4 for payload in ordinary))
        self.assertEqual(
            {payload[:1] for payload in ordinary},
            {b"0", b"1", b"2", b"3"},
        )
        self.assertTrue(any(payload[2] % 2 == 1 for payload in ordinary))
        self.assertEqual(
            {payload[:6] for payload in seam},
            {b"!SEAM0", b"!SEAM1", b"!SEAM2", b"!SEAM3"},
        )
        self.assertTrue(
            any(b'"sorotteParticipantStatusV1"' in payload for payload in ordinary)
        )
        self.assertTrue(
            any(
                b'"positionSampleAgeMs"' in payload
                and b'"snapshot"' in payload
                and b'"report"' in payload
                for payload in ordinary
            )
        )
        self.assertTrue(
            any(
                b'"sorotteParticipantStatusV1":false' in payload
                and b'"futurePhase"' in payload
                for payload in ordinary
            )
        )

    def test_runner_enforces_limits_and_failure_minimization(self) -> None:
        runner = load_runner()
        self.assertEqual(runner.TARGET_NAME, FUZZ_TARGET)
        self.assertEqual(
            runner.FRAMED_SESSION_TARGET_NAME,
            FRAMED_SESSION_TARGET,
        )
        self.assertEqual(
            runner.MPV_FRAMED_TRANSCRIPT_TARGET_NAME,
            MPV_FRAMED_TRANSCRIPT_TARGET,
        )
        self.assertEqual(
            runner.SUPPORTED_TARGETS,
            (
                FUZZ_TARGET,
                FRAMED_SESSION_TARGET,
                MPV_FRAMED_TRANSCRIPT_TARGET,
            ),
        )
        self.assertEqual(runner.MAX_TOTAL_SECONDS, 900)
        self.assertEqual(runner.MAX_INPUT_BYTES, 65_536)
        self.assertEqual(runner.PER_INPUT_TIMEOUT_SECONDS, 5)
        self.assertEqual(runner.RSS_LIMIT_MB, 2_048)
        self.assertEqual(runner.REPORT_SCHEMA, "sorotte-protocol-fuzz-v1")
        self.assertEqual(
            runner.MPV_FRAMED_TRANSCRIPT_REPORT_SCHEMA,
            "sorotte-mpv-framed-transcript-fuzz-v1",
        )
        self.assertEqual(runner.EXPECTED_CARGO_FUZZ_VERSION, "cargo-fuzz 0.13.2")
        self.assertIsNotNone(runner.SOURCE_SHA_PATTERN.fullmatch("0" * 40))
        for malformed in ("", "0" * 39, "0" * 41, "G" * 40, "A" * 40):
            with self.subTest(source_sha=malformed):
                self.assertIsNone(runner.SOURCE_SHA_PATTERN.fullmatch(malformed))
        self.assertEqual(
            runner.REQUIRED_FINAL_STATISTICS,
            (
                "number_of_executed_units",
                "average_exec_per_sec",
                "new_units_added",
                "slowest_unit_time_sec",
                "peak_rss_mb",
            ),
        )

        corpus = pathlib.Path("target/fuzz-ci/protocol-line/corpus")
        artifacts = pathlib.Path("target/fuzz-ci/protocol-line/artifacts")
        command = runner.fuzz_command(FUZZ_TOOLCHAIN, corpus, artifacts, 45)
        self.assertIn("-max_total_time=45", command)
        self.assertIn("-max_len=65536", command)
        self.assertIn("-timeout=5", command)
        self.assertIn("-rss_limit_mb=2048", command)
        self.assertIn("-print_final_stats=1", command)
        self.assertEqual(command.count("--sanitizer"), 1)
        self.assertEqual(command[command.index("--sanitizer") + 1], "address")
        framed_command = runner.fuzz_command(
            FUZZ_TOOLCHAIN,
            corpus,
            artifacts,
            45,
            FRAMED_SESSION_TARGET,
        )
        self.assertIn(FRAMED_SESSION_TARGET, framed_command)
        self.assertNotIn(FUZZ_TARGET, framed_command)
        mpv_command = runner.fuzz_command(
            FUZZ_TOOLCHAIN,
            corpus,
            artifacts,
            45,
            MPV_FRAMED_TRANSCRIPT_TARGET,
        )
        self.assertIn(MPV_FRAMED_TRANSCRIPT_TARGET, mpv_command)
        self.assertNotIn(FRAMED_SESSION_TARGET, mpv_command)

        minimize = runner.minimization_command(
            FUZZ_TOOLCHAIN,
            pathlib.Path("crash-input"),
            pathlib.Path("minimized-input"),
        )
        self.assertIn("tmin", minimize)
        self.assertTrue(
            any(argument.startswith("-exact_artifact_path=") for argument in minimize)
        )
        framed_minimize = runner.minimization_command(
            FUZZ_TOOLCHAIN,
            pathlib.Path("crash-input"),
            pathlib.Path("minimized-input"),
            FRAMED_SESSION_TARGET,
        )
        self.assertIn(FRAMED_SESSION_TARGET, framed_minimize)
        mpv_minimize = runner.minimization_command(
            FUZZ_TOOLCHAIN,
            pathlib.Path("crash-input"),
            pathlib.Path("minimized-input"),
            MPV_FRAMED_TRANSCRIPT_TARGET,
        )
        self.assertIn(MPV_FRAMED_TRANSCRIPT_TARGET, mpv_minimize)
        for command in (minimize, framed_minimize, mpv_minimize):
            self.assertFalse(any(arg.startswith("-max_len=") for arg in command))
            self.assertIn("-timeout=5", command)

    def test_runner_rejects_untrusted_source_identity_before_writing_evidence(
        self,
    ) -> None:
        runner = load_runner()
        with self.assertRaisesRegex(
            ValueError,
            "source SHA must be exactly 40 lowercase hexadecimal characters",
        ):
            runner.main(
                [
                    "--toolchain",
                    FUZZ_TOOLCHAIN,
                    "--source-sha",
                    "A" * 40,
                    "--seconds",
                    "1",
                    "--seed-corpus",
                    CORPUS_PATH,
                    "--expected-seed-count",
                    str(CORPUS_FILE_COUNT),
                    "--output-root",
                    FUZZ_OUTPUT_PATH,
                ]
            )

    def test_runner_binds_complete_harness_and_protocol_source_inventory(self) -> None:
        runner = load_runner()
        manifest = runner.bound_source_manifest(REPO_ROOT)
        actual_paths = {entry["path"] for entry in manifest["files"]}
        expected_paths = set(runner.BOUND_FIXED_SOURCE_PATHS)
        expected_paths.update(
            path.relative_to(REPO_ROOT).as_posix()
            for path in (REPO_ROOT / runner.PROTOCOL_SOURCE_DIRECTORY).rglob("*.rs")
        )

        self.assertEqual(actual_paths, expected_paths)
        self.assertTrue(
            all(workflow_path_covers(path) for path in expected_paths),
            "every source-bound input must trigger the fuzz workflow",
        )
        self.assertEqual(manifest["file_count"], len(expected_paths))
        self.assertEqual(
            manifest["total_bytes"],
            sum(entry["bytes"] for entry in manifest["files"]),
        )
        self.assertEqual(
            manifest["aggregate_sha256"],
            runner.manifest_digest(manifest["files"]),
        )

    def test_runner_binds_complete_framed_session_workspace_inventory(self) -> None:
        runner = load_runner()
        manifest = runner.bound_source_manifest(
            REPO_ROOT,
            FRAMED_SESSION_TARGET,
        )
        actual_paths = {entry["path"] for entry in manifest["files"]}
        expected_paths = set(runner.FRAMED_SESSION_BOUND_FIXED_SOURCE_PATHS)
        expected_paths.update(
            path.relative_to(REPO_ROOT).as_posix()
            for path in (
                REPO_ROOT / runner.FRAMED_SESSION_SOURCE_DIRECTORY
            ).rglob("*")
            if path.is_file()
        )

        self.assertEqual(actual_paths, expected_paths)
        self.assertTrue(
            all(workflow_path_covers(path) for path in expected_paths),
            "every framed-session source-bound input must trigger the workflow",
        )
        self.assertEqual(manifest["file_count"], len(expected_paths))
        self.assertEqual(
            manifest["total_bytes"],
            sum(entry["bytes"] for entry in manifest["files"]),
        )
        self.assertEqual(
            manifest["aggregate_sha256"],
            runner.manifest_digest(manifest["files"]),
        )

    def test_runner_binds_exact_player_mpv_target_and_crate_inventories(self) -> None:
        runner = load_runner()
        manifest = runner.bound_source_manifest(
            REPO_ROOT,
            MPV_FRAMED_TRANSCRIPT_TARGET,
        )
        actual_paths = {entry["path"] for entry in manifest["files"]}
        expected_paths = set(
            runner.MPV_FRAMED_TRANSCRIPT_BOUND_FIXED_SOURCE_PATHS
        )
        for source_directory in runner.MPV_FRAMED_TRANSCRIPT_SOURCE_DIRECTORIES:
            expected_paths.update(
                path.relative_to(REPO_ROOT).as_posix()
                for path in (REPO_ROOT / source_directory).rglob("*")
                if path.is_file()
            )

        self.assertEqual(actual_paths, expected_paths)
        self.assertTrue(
            all(workflow_path_covers(path) for path in expected_paths),
            "every player-mpv source-bound input must trigger the workflow",
        )
        self.assertEqual(manifest["file_count"], len(expected_paths))
        self.assertEqual(
            manifest["total_bytes"],
            sum(entry["bytes"] for entry in manifest["files"]),
        )
        self.assertEqual(
            manifest["aggregate_sha256"],
            runner.manifest_digest(manifest["files"]),
        )

    def test_runner_source_binding_detects_content_and_inventory_drift(self) -> None:
        runner = load_runner()
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            for relative_path in runner.BOUND_FIXED_SOURCE_PATHS:
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f"bound source: {relative_path}\n", encoding="utf-8")
            protocol_source = root / runner.PROTOCOL_SOURCE_DIRECTORY / "codec.rs"
            protocol_source.parent.mkdir(parents=True, exist_ok=True)
            protocol_source.write_text("pub fn decode() {}\n", encoding="utf-8")

            baseline = runner.bound_source_manifest(root)
            target = root / "fuzz" / "fuzz_targets" / "protocol_line.rs"
            target.write_text("changed harness\n", encoding="utf-8")
            content_drift = runner.bound_source_manifest(root)
            self.assertNotEqual(baseline, content_drift)

            target.write_text(
                "bound source: fuzz/fuzz_targets/protocol_line.rs\n",
                encoding="utf-8",
            )
            added_source = (
                root / runner.PROTOCOL_SOURCE_DIRECTORY / "new_protocol_path.rs"
            )
            added_source.write_text("pub fn added() {}\n", encoding="utf-8")
            inventory_drift = runner.bound_source_manifest(root)
            self.assertNotEqual(baseline, inventory_drift)

    def test_runner_parses_and_requires_complete_libfuzzer_statistics(self) -> None:
        runner = load_runner()
        log = "\n".join(
            [
                "stat::number_of_executed_units: 12345",
                "stat::average_exec_per_sec: 678",
                "stat::new_units_added: 9",
                "stat::slowest_unit_time_sec: 0",
                "stat::peak_rss_mb: 321",
            ]
        )
        statistics = runner.parse_final_statistics(log)
        runner.validate_final_statistics(statistics)
        self.assertEqual(statistics["number_of_executed_units"], 12345)
        self.assertEqual(statistics["peak_rss_mb"], 321)

        with self.assertRaisesRegex(ValueError, "duplicate"):
            runner.parse_final_statistics(
                log + "\nstat::number_of_executed_units: 12346"
            )
        with self.assertRaisesRegex(ValueError, "incomplete"):
            runner.validate_final_statistics({"number_of_executed_units": 1})
        with self.assertRaisesRegex(ValueError, "at least one"):
            runner.validate_final_statistics(
                {
                    name: 0
                    for name in runner.REQUIRED_FINAL_STATISTICS
                }
            )

    def test_runner_status_precedence_fails_closed_on_drift_and_evidence(self) -> None:
        runner = load_runner()
        common = {
            "exit_code": 0,
            "timed_out": False,
            "source_stable": True,
            "seed_source_stable": True,
            "evidence_errors": [],
        }
        self.assertEqual(runner.classify_status(**common), "passed")
        self.assertEqual(
            runner.classify_status(**(common | {"exit_code": 1})),
            "failed",
        )
        self.assertEqual(
            runner.classify_status(**(common | {"timed_out": True})),
            "timed_out",
        )
        self.assertEqual(
            runner.classify_status(
                **(common | {"evidence_errors": ["missing statistics"]})
            ),
            "evidence_failed",
        )
        self.assertEqual(
            runner.classify_status(**(common | {"source_stable": False})),
            "source_drift",
        )
        self.assertEqual(
            runner.classify_status(**(common | {"seed_source_stable": False})),
            "source_drift",
        )

    def test_runner_retains_report_when_tool_identity_setup_fails(self) -> None:
        runner = load_runner()
        target_root = REPO_ROOT / "target"
        target_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target_root) as temporary:
            output = pathlib.Path(temporary) / "setup-failure"
            with mock.patch.object(
                runner,
                "tool_identities",
                side_effect=ValueError("tool identity canary"),
            ):
                exit_code = runner.main(
                    [
                        "--toolchain",
                        FUZZ_TOOLCHAIN,
                        "--source-sha",
                        "0" * 40,
                        "--seconds",
                        "1",
                        "--seed-corpus",
                        CORPUS_PATH,
                        "--expected-seed-count",
                        str(CORPUS_FILE_COUNT),
                        "--output-root",
                        str(output),
                    ]
                )

            self.assertEqual(exit_code, 2)
            report = json.loads(
                (output / "run-report.json").read_text(encoding="utf-8")
            )
            self.assertEqual(report["status"], "setup_failed")
            self.assertIn("tool identity canary", report["setup_error"])
            self.assertIsNotNone(report["finished_at"])
            self.assertEqual(len(report["seed_corpus"]["files"]), CORPUS_FILE_COUNT)
            self.assertIsNotNone(report["source_bindings"]["before"])
            self.assertIsNotNone(report["source_sha"])

    def test_fuzz_generated_outputs_are_ignored_locally(self) -> None:
        self.assertEqual(
            FUZZ_GITIGNORE_PATH.read_text(encoding="utf-8").splitlines(),
            ["/target/", "__pycache__/", "*.py[cod]"],
        )


class FuzzCorpusAuthorityTests(unittest.TestCase):
    def fixture(self, root: pathlib.Path) -> dict:
        manifest = copy.deepcopy(CORPUS_MANIFEST)
        for target in manifest["targets"]:
            shutil.copytree(REPO_ROOT / target["directory"], root / target["directory"])
        (root / "coverage").mkdir()
        self.write_manifest(root, manifest)
        return manifest

    def write_manifest(self, root: pathlib.Path, manifest: dict) -> None:
        (root / "coverage/fuzz-corpora.json").write_text(json.dumps(manifest), encoding="utf-8")

    def test_manifest_binds_all_reviewed_original_bytes_and_retained_crashes(self) -> None:
        from scripts import fuzz_regressions
        manifest = fuzz_regressions.validate()
        self.assertEqual(set(fuzz_regressions.TARGET_DIRECTORIES), set(load_runner().SUPPORTED_TARGETS))
        self.assertEqual(manifest, CORPUS_MANIFEST)
        attributes = (REPO_ROOT / ".gitattributes").read_text(encoding="utf-8").splitlines()
        for target in manifest["targets"]:
            self.assertIn(target["directory"] + "/** -text", attributes)

    def test_malformed_or_weakened_manifest_is_rejected(self) -> None:
        from scripts import fuzz_regressions
        mutations = {
            "unsupported-schema": lambda d: d.update(schema_version=2),
            "boolean-schema": lambda d: d.update(schema_version=True),
            "unknown-field": lambda d: d.update(ignored=True),
            "empty-targets": lambda d: d.update(targets=[]),
            "missing-target": lambda d: d["targets"].pop(),
            "duplicate-target": lambda d: d["targets"].__setitem__(1, copy.deepcopy(d["targets"][0])),
            "unknown-target": lambda d: d["targets"][0].update(id="unknown"),
            "unreviewed-directory": lambda d: d["targets"][0].update(directory="target/corpus"),
            "empty-corpus": lambda d: d["targets"][0].update(files=[]),
            "duplicate-seed": lambda d: d["targets"][0]["files"].append(d["targets"][0]["files"][0]),
            "unsorted-seeds": lambda d: d["targets"][0]["files"].reverse(),
            "path-traversal": lambda d: d["targets"][0]["files"][0].update(name="../seed"),
            "windows-traversal": lambda d: d["targets"][0]["files"][0].update(name="..\\seed"),
            "false-byte-count": lambda d: d["targets"][0]["files"][0].update(bytes=True),
            "invalid-digest": lambda d: d["targets"][0]["files"][0].update(sha256="bad"),
            "empty-regressions": lambda d: d.update(regressions=[]),
            "duplicate-regression": lambda d: d["regressions"].append(d["regressions"][0]),
            "removed-required-id": lambda d: d["regressions"][0].update(id="other-regression"),
            "wrong-required-test": lambda d: d["regressions"][0].update(test="tests::different"),
            "filter-injection": lambda d: d["regressions"][0].update(test="tests::x) | all("),
            "dangling-seed": lambda d: d["regressions"][0].update(seed="target/new-seed"),
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            baseline = self.fixture(root)
            for label, mutate in mutations.items():
                with self.subTest(label=label):
                    candidate = copy.deepcopy(baseline)
                    mutate(candidate)
                    self.write_manifest(root, candidate)
                    with self.assertRaises((ValueError, OSError)):
                        fuzz_regressions.validate(root=root)
            duplicate_json = json.dumps(baseline).replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1')
            (root / "coverage/fuzz-corpora.json").write_text(duplicate_json, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
                fuzz_regressions.validate(root=root)

    def test_inventory_drift_and_rewritten_original_crash_fail_closed(self) -> None:
        from scripts import fuzz_regressions
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            manifest = self.fixture(root)
            corpus = root / manifest["targets"][0]["directory"]
            extra = corpus / "unreviewed"
            extra.write_bytes(b"seed")
            with self.assertRaisesRegex(ValueError, "identity changed"):
                fuzz_regressions.validate(root=root)
            extra.unlink()
            subdirectory = corpus / "nested"
            subdirectory.mkdir()
            with self.assertRaisesRegex(ValueError, "direct regular files"):
                fuzz_regressions.validate(root=root)
            subdirectory.rmdir()
            regression = manifest["regressions"][0]
            original = root / regression["seed"]
            original.write_bytes(original.read_bytes() + b"rewritten")
            for target in manifest["targets"]:
                for entry in target["files"]:
                    if target["directory"] + "/" + entry["name"] == regression["seed"]:
                        entry.update(bytes=original.stat().st_size, sha256=hashlib.sha256(original.read_bytes()).hexdigest())
            self.write_manifest(root, manifest)
            with self.assertRaisesRegex(ValueError, "original crash bytes changed"):
                fuzz_regressions.validate(root=root)

    def test_runner_requires_shared_manifest_authority_before_outputs_or_tools(self) -> None:
        runner = load_runner()
        with tempfile.TemporaryDirectory(dir=REPO_ROOT / "target") as temporary:
            output = pathlib.Path(temporary) / "campaign"
            with mock.patch.object(runner, "validate_corpus_manifest", side_effect=ValueError("empty reviewed corpus")), \
                    mock.patch.object(runner, "tool_identities") as tools:
                with self.assertRaisesRegex(ValueError, "empty reviewed corpus"):
                    runner.main(["--toolchain", FUZZ_TOOLCHAIN, "--source-sha", "0" * 40, "--seconds", "1",
                                 "--seed-corpus", CORPUS_PATH, "--output-root", str(output)])
                tools.assert_not_called()
                self.assertFalse(output.exists())

    def test_all_targets_bind_global_selector_tool_and_corpus_sources(self) -> None:
        runner = load_runner()
        shared = {".gitattributes", "coverage/fuzz-corpora.json", "coverage/verification-tools.toml",
                  "coverage/verification-lanes.json", "scripts/fuzz_regressions.py", "scripts/fuzz_tool_canary.py",
                  "scripts/verify.py", "scripts/verification_tools.py"}
        for target in runner.SUPPORTED_TARGETS:
            with self.subTest(target=target):
                paths = {entry["path"] for entry in runner.bound_source_manifest(REPO_ROOT, target)["files"]}
                self.assertTrue(shared.issubset(paths))

    def test_replay_rejects_empty_test_selection_and_keeps_failure_receipt(self) -> None:
        from scripts import fuzz_regressions
        with tempfile.TemporaryDirectory() as temporary:
            output = pathlib.Path(temporary) / "replay.json"
            def no_selected_tests(command, **kwargs):
                self.assertEqual(command[:4], ["cargo", "nextest", "run", "--locked"])
                self.assertIn("--lib", command)
                self.assertEqual(command[command.index("--no-tests") + 1], "fail")
                self.assertEqual(command[command.index("-E") + 1], "test(=" + CORPUS_MANIFEST["regressions"][0]["test"] + ")")
                self.assertTrue(kwargs["check"])
                self.assertEqual(kwargs["timeout"], 300)
                raise subprocess.CalledProcessError(4, command)
            with mock.patch.object(fuzz_regressions, "identity", return_value={"source_sha": "0" * 40}), \
                    mock.patch.object(fuzz_regressions.subprocess, "run", side_effect=no_selected_tests), \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(fuzz_regressions.main(["replay", "--output", str(output)]), 1)
            receipt = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(receipt["status"], "failed")
            self.assertEqual(receipt["attempts"][0]["status"], "failed")
            original = output.read_bytes()
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(fuzz_regressions.main(["validate", "--output", str(output)]), 1)
            self.assertEqual(output.read_bytes(), original)


class FuzzToolCanaryTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform == "linux", "Linux ASan process ownership contract")
    def test_command_timeout_terminates_owned_descendants(self) -> None:
        from scripts import fuzz_tool_canary as canary
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            code = ("import subprocess,sys,time; "
                    "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); "
                    "print(child.pid,flush=True); time.sleep(30)")
            log = root / "owned.log"
            with log.open("wb") as stream, self.assertRaises(subprocess.TimeoutExpired):
                canary.execute([sys.executable, "-c", code], root, stream, 1)
            child = int(log.read_text().strip())
            status = pathlib.Path(f"/proc/{child}/stat")
            if status.exists():
                self.assertEqual(status.read_text().split()[2], "Z", "owned descendant remained running")

    def simulate(self, output: pathlib.Path, defect: str | None = None) -> dict:
        from scripts import fuzz_tool_canary as canary
        statistics = "\n".join(f"stat::{name}: {1 if name == 'number_of_executed_units' else 0}"
                               for name in load_runner().REQUIRED_FINAL_STATISTICS)

        def execute(argv, cwd, stream, timeout):
            self.assertEqual(timeout, 180)
            if defect == "timeout": raise subprocess.TimeoutExpired(argv, timeout)
            if defect == "cancelled": raise KeyboardInterrupt("test cancellation")
            if argv[3] == "build":
                (cwd / "fuzz/Cargo.lock").write_text("test lock", encoding="utf-8")
                return 0
            if argv[3] == "tmin":
                self.assertFalse(any(arg.startswith("-max_len=") for arg in argv))
                path = pathlib.Path(next(arg.removeprefix("-exact_artifact_path=") for arg in argv
                                         if arg.startswith("-exact_artifact_path=")))
                payload = b"CANARY!"
                if defect == "lost-defect": payload = b"safe"
                if defect == "no-minimization": payload = (cwd / "original-crash").read_bytes()
                path.write_bytes(payload)
                if defect == "overwritten-original": (cwd / "original-crash").write_bytes(payload)
                return 0
            if "benign-input" in " ".join(argv):
                stream.write(("missing stats" if defect == "missing-statistics" else statistics).encode())
                return 0
            if defect == "crash-not-reproduced": return 0
            if defect == "unrelated-failure":
                stream.write(b"failed to start libFuzzer")
            elif defect != "bad-minimized-replay" or "original-crash" in " ".join(argv):
                stream.write(b"intentional minimizer canary")
            return 1

        version = "cargo-fuzz bad" if defect == "wrong-pin" else "cargo-fuzz 0.13.2"
        with mock.patch.object(canary, "identity", return_value={"source_sha": "0" * 40}), \
                mock.patch.object(canary, "os", types.SimpleNamespace(name="posix")), \
                mock.patch.object(canary.subprocess, "check_output", return_value=version), \
                mock.patch.object(canary, "execute", side_effect=execute):
            return canary.run(output)

    def test_canary_uses_actual_runner_minimizer_and_validates_statistics(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = pathlib.Path(temporary) / "canary"
            result = self.simulate(output)
            self.assertEqual(result["status"], "passed")
            self.assertEqual(len(result["commands"]), 5)
            self.assertEqual(result["benign_statistics"]["number_of_executed_units"], 1)
            self.assertEqual((output / "minimized-crash").read_bytes(), b"CANARY!")
            self.assertNotEqual(result["original_sha256"], result["minimized_sha256"])
            with self.assertRaisesRegex(ValueError, "fresh"):
                self.simulate(output)

    def test_canary_rejects_false_success_and_preserves_failed_attempts(self) -> None:
        for defect in ("wrong-pin", "missing-statistics", "crash-not-reproduced", "unrelated-failure",
                       "lost-defect", "no-minimization", "overwritten-original", "bad-minimized-replay",
                       "timeout", "cancelled"):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as temporary:
                output = pathlib.Path(temporary) / "canary"
                with self.assertRaises((ValueError, subprocess.TimeoutExpired, KeyboardInterrupt)):
                    self.simulate(output, defect)
                receipt = json.loads((output / "receipt.json").read_text(encoding="utf-8"))
                expected = "timed_out" if defect == "timeout" else "cancelled" if defect == "cancelled" else "failed"
                self.assertEqual(receipt["status"], expected)
                self.assertIn("error", receipt)
                self.assertIn("duration_seconds", receipt)
                if defect in ("timeout", "cancelled"):
                    self.assertEqual(receipt["commands"][0]["status"], expected)
                    self.assertIn("log_sha256", receipt["commands"][0])


if __name__ == "__main__":
    unittest.main()
