from __future__ import annotations

import importlib.util
import json
import pathlib
import re
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
FUZZ_TARGET_PATH = REPO_ROOT / "fuzz" / "fuzz_targets" / "protocol_line.rs"
FRAMED_SESSION_TARGET_PATH = (
    REPO_ROOT / "fuzz" / "fuzz_targets" / "framed_session.rs"
)
FUZZ_RUNNER_PATH = REPO_ROOT / "fuzz" / "run_protocol_fuzz.py"
FUZZ_GITIGNORE_PATH = REPO_ROOT / "fuzz" / ".gitignore"

CORPUS_PATH = "crates/sorotte-protocol/tests/corpus/protocol_parser"
CORPUS_FILE_COUNT = 16
FRAMED_SESSION_CORPUS_PATH = "crates/sorotte-cli/tests/corpus/framed_session"
FRAMED_SESSION_CORPUS_FILE_COUNT = 14
FRAMED_SESSION_CORPUS_DIRECTORY = REPO_ROOT / FRAMED_SESSION_CORPUS_PATH
FUZZ_TOOLCHAIN = "nightly-2026-07-29"
FUZZ_SECONDS_EXPRESSION = (
    "${{ (github.event_name == 'pull_request' || github.event_name == 'push') "
    "&& '45' || '900' }}"
)
FUZZ_OUTPUT_PATH = "target/fuzz-ci/protocol-line"
FUZZ_TARGET = "protocol_line"
FRAMED_SESSION_OUTPUT_PATH = "target/fuzz-ci/framed-session"
FRAMED_SESSION_TARGET = "framed_session"

PINNED_USES = {
    "Checkout": (
        "actions/checkout@11d5960a326750d5838078e36cf38b85af677262"
    ),
    "Setup pinned nightly Rust": (
        "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4"
    ),
    "Setup Python": (
        "actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065"
    ),
    "Install pinned cargo-fuzz": (
        "taiki-e/install-action@41049aa56687c35e0afa74eed4f09cec4f9afabf"
    ),
    "Upload protocol fuzz evidence": (
        "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02"
    ),
}
EXPECTED_STEP_NAMES = [
    "Checkout",
    "Setup pinned nightly Rust",
    "Setup Python",
    "Install CI policy prerequisites",
    "Install pinned cargo-fuzz",
    "Verify exact fuzz toolchain",
    "Validate protocol fuzz policy",
    "Build protocol fuzz target",
    "Run bounded protocol fuzz target",
    "Upload protocol fuzz evidence",
]
EXPECTED_FRAMED_SESSION_STEP_NAMES = [
    "Checkout",
    "Setup pinned nightly Rust",
    "Setup Python",
    "Install CI policy prerequisites",
    "Install pinned cargo-fuzz",
    "Verify exact fuzz toolchain",
    "Validate protocol fuzz policy",
    "Build framed-session fuzz target",
    "Run bounded framed-session fuzz target",
    "Upload framed-session fuzz evidence",
]
EXPECTED_PATHS = [
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "coverage/behaviors.toml",
    "coverage/known-defects.toml",
    "crates/**",
    "fuzz/**",
    ".github/workflows/rust-fuzz.yml",
    "requirements/ci-policy.txt",
    "scripts/known_defect_policy.py",
    "scripts/tests/test_known_defect_policy.py",
    "scripts/tests/test_protocol_fuzz_policy.py",
]
USES_LINE = re.compile(
    r"^\s*uses:\s*([^@\s]+)@([0-9a-f]{40})\s+#\s+\S.*$",
    re.MULTILINE,
)


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


def workflow_path_covers(path: str) -> bool:
    return any(
        path == pattern
        or (
            pattern.endswith("/**")
            and path.startswith(pattern.removesuffix("**"))
        )
        for pattern in EXPECTED_PATHS
    )


def assert_workflow_contract(text: str) -> None:
    workflow = parse_workflow(text)
    require(workflow.get("permissions") == {"contents": "read"}, "read-only permissions")
    require(
        workflow.get("concurrency")
        == {
            "group": "sorotte-protocol-fuzz-${{ github.ref }}",
            "cancel-in-progress": (
                "${{ github.event_name != 'schedule' "
                "&& github.event_name != 'workflow_dispatch' }}"
            ),
        },
        "fuzz concurrency contract changed",
    )

    triggers = workflow.get("on")
    require(isinstance(triggers, dict), "workflow triggers must be explicit")
    require(
        set(triggers) == {"pull_request", "push", "workflow_dispatch", "schedule"},
        "workflow trigger set changed",
    )
    require(
        triggers["pull_request"] == {"paths": EXPECTED_PATHS},
        "pull-request paths changed",
    )
    require(
        triggers["push"] == {"branches": ["main"], "paths": EXPECTED_PATHS},
        "push must be restricted to main to avoid duplicate feature-branch runs",
    )
    require(triggers["workflow_dispatch"] == "", "dispatch must remain enabled")
    require(
        triggers["schedule"] == [{"cron": "45 3 * * 3"}],
        "weekly schedule changed",
    )

    require(
        set(workflow["jobs"]) == {"protocol-fuzz", "framed-session-fuzz"},
        "unexpected fuzz jobs",
    )
    job = workflow["jobs"]["protocol-fuzz"]
    require(job.get("runs-on") == "ubuntu-latest", "fuzzing must run on Linux")
    require(job.get("timeout-minutes") == "25", "job timeout must remain bounded")
    require(
        job.get("env") == {"FUZZ_SECONDS": FUZZ_SECONDS_EXPRESSION},
        "event-specific fuzz duration changed",
    )
    require(
        [step.get("name") for step in job.get("steps", [])] == EXPECTED_STEP_NAMES,
        "fuzz step order changed",
    )

    uses_matches = USES_LINE.findall(text)
    require(
        len(uses_matches) == len(PINNED_USES) * 2,
        "every action in both fuzz jobs must be commit-pinned",
    )
    for step_name, expected_uses in PINNED_USES.items():
        step = named_step(job, step_name)
        require(step.get("uses") == expected_uses, f"{step_name} action pin changed")

    checkout = named_step(job, "Checkout")
    require(
        checkout.get("with") == {"persist-credentials": "false"},
        "checkout credentials must not persist",
    )
    rust = named_step(job, "Setup pinned nightly Rust")
    require(
        rust.get("with")
        == {"toolchain": FUZZ_TOOLCHAIN, "components": "rust-src"},
        "nightly fuzz toolchain must be dated and include rust-src",
    )
    python = named_step(job, "Setup Python")
    require(python.get("with") == {"python-version": "3.11"}, "Python pin changed")
    installer = named_step(job, "Install pinned cargo-fuzz")
    require(
        installer.get("with")
        == {"tool": "cargo-fuzz@0.13.2", "fallback": "none"},
        "cargo-fuzz installation must remain exact and fail closed",
    )

    require(
        command_tokens(named_step(job, "Install CI policy prerequisites"))
        == [
            "python",
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "-r",
            "requirements/ci-policy.txt",
        ],
        "policy prerequisite command changed",
    )
    verify_command = named_step(job, "Verify exact fuzz toolchain").get("run", "")
    require(
        'test "$(cargo fuzz --version)" = "cargo-fuzz 0.13.2"' in verify_command,
        "cargo-fuzz runtime version check missing",
    )
    require(
        f"rustc +{FUZZ_TOOLCHAIN} -vV" in verify_command,
        "nightly runtime identity check missing",
    )
    require(
        command_tokens(named_step(job, "Validate protocol fuzz policy"))
        == [
            "python",
            "-m",
            "unittest",
            "scripts.tests.test_protocol_fuzz_policy",
            "-v",
        ],
        "standalone fuzz policy check changed",
    )
    require(
        command_tokens(named_step(job, "Build protocol fuzz target"))
        == [
            "cargo",
            f"+{FUZZ_TOOLCHAIN}",
            "fuzz",
            "build",
            "--fuzz-dir",
            "fuzz",
            "--sanitizer",
            "address",
            FUZZ_TARGET,
        ],
        "fuzz build contract changed",
    )
    require(
        command_tokens(named_step(job, "Run bounded protocol fuzz target"))
        == [
            "python",
            "fuzz/run_protocol_fuzz.py",
            "--toolchain",
            FUZZ_TOOLCHAIN,
            "--source-sha",
            "${{ github.sha }}",
            "--seconds",
            "${FUZZ_SECONDS}",
            "--seed-corpus",
            CORPUS_PATH,
            "--expected-seed-count",
            str(CORPUS_FILE_COUNT),
            "--output-root",
            FUZZ_OUTPUT_PATH,
        ],
        "bounded fuzz runner command changed",
    )

    upload = named_step(job, "Upload protocol fuzz evidence")
    require(upload.get("if") == "always()", "fuzz evidence must upload on failure")
    require(
        upload.get("with")
        == {
            "name": "sorotte-protocol-fuzz",
            "path": FUZZ_OUTPUT_PATH,
            "if-no-files-found": "error",
            "retention-days": "14",
            "overwrite": "true",
        },
        "fuzz evidence retention contract changed",
    )

    framed_job = workflow["jobs"]["framed-session-fuzz"]
    require(
        framed_job.get("runs-on") == "ubuntu-latest",
        "framed-session fuzzing must run on Linux",
    )
    require(
        framed_job.get("timeout-minutes") == "25",
        "framed-session job timeout must remain bounded",
    )
    require(
        framed_job.get("env") == {"FUZZ_SECONDS": FUZZ_SECONDS_EXPRESSION},
        "framed-session event-specific duration changed",
    )
    require(
        [step.get("name") for step in framed_job.get("steps", [])]
        == EXPECTED_FRAMED_SESSION_STEP_NAMES,
        "framed-session step order changed",
    )
    for step_name in (
        "Checkout",
        "Setup pinned nightly Rust",
        "Setup Python",
        "Install pinned cargo-fuzz",
    ):
        require(
            named_step(framed_job, step_name).get("uses")
            == PINNED_USES[step_name],
            f"framed-session {step_name} action pin changed",
        )
    require(
        named_step(framed_job, "Upload framed-session fuzz evidence").get("uses")
        == PINNED_USES["Upload protocol fuzz evidence"],
        "framed-session upload action pin changed",
    )
    require(
        named_step(framed_job, "Checkout").get("with")
        == {"persist-credentials": "false"},
        "framed-session checkout credentials must not persist",
    )
    require(
        named_step(framed_job, "Setup pinned nightly Rust").get("with")
        == {"toolchain": FUZZ_TOOLCHAIN, "components": "rust-src"},
        "framed-session nightly toolchain contract changed",
    )
    require(
        named_step(framed_job, "Setup Python").get("with")
        == {"python-version": "3.11"},
        "framed-session Python pin changed",
    )
    require(
        named_step(framed_job, "Install pinned cargo-fuzz").get("with")
        == {"tool": "cargo-fuzz@0.13.2", "fallback": "none"},
        "framed-session cargo-fuzz installation changed",
    )
    require(
        command_tokens(named_step(framed_job, "Install CI policy prerequisites"))
        == [
            "python",
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "-r",
            "requirements/ci-policy.txt",
        ],
        "framed-session policy prerequisite command changed",
    )
    framed_verify = named_step(
        framed_job,
        "Verify exact fuzz toolchain",
    ).get("run", "")
    require(
        'test "$(cargo fuzz --version)" = "cargo-fuzz 0.13.2"'
        in framed_verify,
        "framed-session cargo-fuzz runtime check missing",
    )
    require(
        f"rustc +{FUZZ_TOOLCHAIN} -vV" in framed_verify,
        "framed-session nightly runtime identity check missing",
    )
    require(
        command_tokens(named_step(framed_job, "Validate protocol fuzz policy"))
        == [
            "python",
            "-m",
            "unittest",
            "scripts.tests.test_protocol_fuzz_policy",
            "-v",
        ],
        "framed-session policy check changed",
    )
    require(
        command_tokens(named_step(framed_job, "Build framed-session fuzz target"))
        == [
            "cargo",
            f"+{FUZZ_TOOLCHAIN}",
            "fuzz",
            "build",
            "--fuzz-dir",
            "fuzz",
            "--sanitizer",
            "address",
            FRAMED_SESSION_TARGET,
        ],
        "framed-session fuzz build contract changed",
    )
    require(
        command_tokens(named_step(framed_job, "Run bounded framed-session fuzz target"))
        == [
            "python",
            "fuzz/run_protocol_fuzz.py",
            "--target",
            FRAMED_SESSION_TARGET,
            "--toolchain",
            FUZZ_TOOLCHAIN,
            "--source-sha",
            "${{ github.sha }}",
            "--seconds",
            "${FUZZ_SECONDS}",
            "--seed-corpus",
            FRAMED_SESSION_CORPUS_PATH,
            "--expected-seed-count",
            str(FRAMED_SESSION_CORPUS_FILE_COUNT),
            "--output-root",
            FRAMED_SESSION_OUTPUT_PATH,
        ],
        "bounded framed-session runner command changed",
    )
    framed_upload = named_step(framed_job, "Upload framed-session fuzz evidence")
    require(
        framed_upload.get("if") == "always()",
        "framed-session evidence must upload on failure",
    )
    require(
        framed_upload.get("with")
        == {
            "name": "sorotte-framed-session-fuzz",
            "path": FRAMED_SESSION_OUTPUT_PATH,
            "if-no-files-found": "error",
            "retention-days": "14",
            "overwrite": "true",
        },
        "framed-session evidence retention contract changed",
    )
    require("continue-on-error" not in text, "fuzz failures must never be tolerated")
    require("|| true" not in text, "workflow must not mask a failing command")


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
                "actions/checkout@11d5960a326750d5838078e36cf38b85af677262",
                "actions/checkout@v4",
            ),
            original.replace("--sanitizer address", "--sanitizer none"),
            original.replace('--source-sha "${{ github.sha }}"', "--source-sha bad"),
            original.replace("--expected-seed-count 16", "--expected-seed-count 1"),
            original.replace("--target framed_session", "--target protocol_line"),
            original.replace(
                "target/fuzz-ci/framed-session",
                "target/fuzz-ci/protocol-line",
            ),
        ]
        for mutation in mutations:
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
            "DEFAULT_MAX_PROTOCOL_LINE_BYTES",
            protocol_io,
        )
        self.assertIn("pub struct InboundProtocolLineReader", protocol_io)
        self.assertNotIn("cfg(feature = \"fuzz-support\")", protocol_io)

    def test_framed_session_seed_corpus_is_direct_and_covers_control_modes(self) -> None:
        entries = sorted(FRAMED_SESSION_CORPUS_DIRECTORY.iterdir())
        self.assertEqual(len(entries), FRAMED_SESSION_CORPUS_FILE_COUNT)
        self.assertTrue(
            all(entry.is_file() and not entry.is_symlink() for entry in entries)
        )
        payloads = [entry.read_bytes() for entry in entries]
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

    def test_runner_enforces_limits_and_failure_minimization(self) -> None:
        runner = load_runner()
        self.assertEqual(runner.TARGET_NAME, FUZZ_TARGET)
        self.assertEqual(
            runner.FRAMED_SESSION_TARGET_NAME,
            FRAMED_SESSION_TARGET,
        )
        self.assertEqual(
            runner.SUPPORTED_TARGETS,
            (FUZZ_TARGET, FRAMED_SESSION_TARGET),
        )
        self.assertEqual(runner.MAX_TOTAL_SECONDS, 900)
        self.assertEqual(runner.MAX_INPUT_BYTES, 65_536)
        self.assertEqual(runner.PER_INPUT_TIMEOUT_SECONDS, 5)
        self.assertEqual(runner.RSS_LIMIT_MB, 2_048)
        self.assertEqual(runner.REPORT_SCHEMA, "sorotte-protocol-fuzz-v1")
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


if __name__ == "__main__":
    unittest.main()
