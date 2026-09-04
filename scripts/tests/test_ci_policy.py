from __future__ import annotations

import copy
import pathlib
import re
import shlex
import tomllib
import unittest
from typing import Any

import yaml


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
GIT_ATTRIBUTES_PATH = REPO_ROOT / ".gitattributes"
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "rust-ci.yml"
COVERAGE_WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "rust-coverage.yml"
MUTATION_WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "rust-mutation.yml"
CATALOG_PATH = REPO_ROOT / "coverage" / "behaviors.toml"
IGNORED_TESTS_PATH = REPO_ROOT / "coverage" / "ignored-tests.toml"
KNOWN_DEFECTS_PATH = REPO_ROOT / "coverage" / "known-defects.toml"
MUTATION_POLICY_PATH = REPO_ROOT / "coverage" / "mutation-policy.toml"
PACKAGE_PATH_BOUNDARY_TEST_PATH = REPO_ROOT / "scripts" / "package-path-boundary-tests.ps1"
CI_REQUIREMENTS = REPO_ROOT / "requirements" / "ci-policy.txt"
LEGACY_REQUIREMENTS = REPO_ROOT / "requirements" / "legacy-python-interop.txt"
LEGACY_SYNCPLAY_SHA = "d1c5f85af377c960c5a940707c4d01bc84fd9c3f"
MPV_MINIMUM_SHA = "41f6a645068483470267271e1d09966ca3b9f413"
MPV_NEWEST_SHA = "d12f2ce19c918875981e00ed276f153bdf40a2ac"
MPV_MATRIX_EXPRESSION = (
    "${{ fromJSON((github.event_name == 'schedule' || "
    "github.event_name == 'workflow_dispatch') && "
    "'[\"minimum\",\"newest\"]' || '[\"minimum\"]') }}"
)
MPV_SOURCE_EXPRESSION = (
    "${{ matrix.mpv_identity == 'minimum' && "
    + f"'{MPV_MINIMUM_SHA}' || '{MPV_NEWEST_SHA}'"
    + " }}"
)
HEAD_REF = "${{ env.VERIFICATION_SHA }}"
ACTION_PINS = {
    "actions/checkout": (
        "3d3c42e5aac5ba805825da76410c181273ba90b1",
        "v7.0.1",
    ),
    "dtolnay/rust-toolchain": (
        "4cda84d5c5c54efe2404f9d843567869ab1699d4",
        "stable resolved 2026-07-28",
    ),
    "actions/setup-python": (
        "5fda3b95a4ea91299a34e894583c3862153e4b97",
        "v7.0.0",
    ),
    "actions/setup-go": (
        "924ae3a1cded613372ab5595356fb5720e22ba16",
        "v6.5.0",
    ),
    "actions/upload-artifact": (
        "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        "v7.0.1",
    ),
    "actions/download-artifact": (
        "3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
        "v8.0.1",
    ),
    "taiki-e/install-action": (
        "67729d5c413db75907f0ad1e39bb04b9c868ff60",
        "v2.85.7",
    ),
}
PINNED_USES = {
    action: f"{action}@{sha}" for action, (sha, _comment) in ACTION_PINS.items()
}
NODE24_ACTIONS = frozenset(
    {
        "actions/checkout",
        "actions/setup-python",
        "actions/setup-go",
        "actions/upload-artifact",
        "actions/download-artifact",
    }
)
USES_LINE = re.compile(
    r"^\s*uses:\s*([^@\s]+)@([^\s#]+)\s+#\s+(.+?)\s*$",
    re.MULTILINE,
)


def normalized(value: str) -> str:
    return " ".join(part for part in value.split() if part != "\\")


def parse_workflow(workflow: str) -> dict[str, Any]:
    parsed = yaml.load(workflow, Loader=yaml.BaseLoader)
    if not isinstance(parsed, dict) or not isinstance(parsed.get("jobs"), dict):
        raise AssertionError("workflow must contain a jobs mapping")
    return parsed


def named_step(jobs: dict[str, Any], job_id: str, name: str) -> dict[str, Any]:
    job = jobs[job_id]
    matches = [
        step
        for step in job.get("steps", [])
        if isinstance(step, dict) and step.get("name") == name
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"job {job_id} must contain exactly one step named {name!r}; "
            f"found {len(matches)}"
        )
    return matches[0]


def requirement_lines(path: pathlib.Path) -> list[str]:
    return [
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if line and not line.startswith("#")
    ]


def logical_shell_commands(run: str) -> list[list[str]]:
    commands: list[list[str]] = []
    pending = ""
    for raw_line in run.splitlines():
        line = raw_line.strip()
        if not line or (line.startswith("#") and not pending):
            continue
        continued = line.endswith("\\")
        fragment = line[:-1].rstrip() if continued else line
        pending = f"{pending} {fragment}".strip()
        if continued:
            continue
        try:
            commands.append(shlex.split(pending, posix=True))
        except ValueError as error:
            raise AssertionError(f"invalid shell command {pending!r}: {error}") from error
        pending = ""
    if pending:
        raise AssertionError("shell command ends with an unterminated continuation")
    return commands


def ignored_test_cargo_command(entry: dict[str, Any]) -> list[str]:
    source = pathlib.PurePosixPath(entry["source"])
    parts = source.parts
    if (
        len(parts) != 4
        or parts[0] != "crates"
        or parts[2] != "tests"
        or source.suffix != ".rs"
    ):
        raise AssertionError(
            f"{entry['id']} pull-request ignored test must be a direct integration "
            "test target under crates/<crate>/tests"
        )
    manifest_path = REPO_ROOT / parts[0] / parts[1] / "Cargo.toml"
    with manifest_path.open("rb") as handle:
        package = tomllib.load(handle)["package"]["name"]
    return [
        "cargo",
        "test",
        "--locked",
        "-p",
        package,
        "--test",
        source.stem,
        entry["test"],
        "--",
        "--ignored",
        "--exact",
        "--nocapture",
    ]


def validate_pull_request_ignored_bindings(
    jobs: dict[str, Any],
    catalog: dict[str, Any],
    ignored_tests: dict[str, Any],
) -> None:
    entries = [
        entry
        for entry in ignored_tests["ignored_test"]
        if entry["tier"] == "pull-request"
    ]
    if not entries:
        raise AssertionError("ignored-test registry must contain pull-request entries")
    aggregate_job = jobs.get("verification_required")
    if not isinstance(aggregate_job, dict):
        raise AssertionError("verification_required aggregate job is missing")
    needs = aggregate_job.get("needs")
    if not isinstance(needs, list):
        raise AssertionError("verification_required.needs must be a list")
    required_results = catalog["policy"]["required_jobs"]
    aggregate = named_step(
        jobs,
        "verification_required",
        "Aggregate required behavior evidence",
    )
    aggregate_env = aggregate.get("env")
    if not isinstance(aggregate_env, dict):
        raise AssertionError("aggregate result step must have an env mapping")
    aggregate_commands = logical_shell_commands(aggregate.get("run", ""))
    if len(aggregate_commands) != 1:
        raise AssertionError("aggregate result step must be one logical command")
    aggregate_tokens = aggregate_commands[0]

    for job_id in sorted({entry["required_job"] for entry in entries}):
        if job_id not in jobs:
            raise AssertionError(f"ignored-test required job {job_id!r} is missing")
        if job_id not in needs:
            raise AssertionError(
                f"ignored-test required job {job_id!r} is not an aggregate dependency"
            )
        if job_id not in required_results:
            raise AssertionError(
                f"ignored-test required job {job_id!r} is not catalog-required"
            )
        job = jobs[job_id]
        if "continue-on-error" in job:
            raise AssertionError(
                f"ignored-test required job {job_id!r} cannot tolerate failure"
            )
        if_condition = job.get("if")
        if if_condition not in (None, "github.event_name != 'schedule'"):
            raise AssertionError(
                f"ignored-test required job {job_id!r} is not unconditionally "
                "enabled for pull requests"
            )

        bracket_result = f"${{{{ needs['{job_id}'].result }}}}"
        dot_result = f"${{{{ needs.{job_id}.result }}}}"
        env_keys = [
            key
            for key, value in aggregate_env.items()
            if value in {bracket_result, dot_result}
        ]
        if len(env_keys) != 1:
            raise AssertionError(
                f"ignored-test required job {job_id!r} must expose exactly one "
                "aggregate result environment variable"
            )
        expected_result = f"{job_id}=${env_keys[0]}"
        actual_results = [
            aggregate_tokens[index + 1]
            for index, token in enumerate(aggregate_tokens[:-1])
            if token == "--job-result"
        ]
        if actual_results.count(expected_result) != 1:
            raise AssertionError(
                f"ignored-test required job {job_id!r} result is not aggregated "
                "exactly once"
            )

    for entry in entries:
        job_id = entry["required_job"]
        expected = ignored_test_cargo_command(entry)
        matches: list[dict[str, Any]] = []
        for step in jobs[job_id].get("steps", []):
            if not isinstance(step, dict):
                continue
            for command in logical_shell_commands(step.get("run", "")):
                if command == expected:
                    matches.append(step)
        if len(matches) != 1:
            raise AssertionError(
                f"{entry['id']} must have exactly one exact cargo invocation in "
                f"{job_id}; found {len(matches)}"
            )
        step = matches[0]
        if "if" in step:
            raise AssertionError(f"{entry['id']} invocation cannot be conditional")
        if "continue-on-error" in step:
            raise AssertionError(f"{entry['id']} invocation cannot tolerate failure")


def validate_parallel_ci_graph(jobs: dict[str, Any]) -> None:
    windows_workers = [
        "rust_windows_tests",
        "rust_windows_release",
        "rust_windows_coverage",
    ]
    for job_id in windows_workers:
        job = jobs.get(job_id)
        if not isinstance(job, dict):
            raise AssertionError(f"parallel Windows worker {job_id!r} is missing")
        if job.get("runs-on") != "windows-latest":
            raise AssertionError(f"parallel Windows worker {job_id!r} moved off Windows")
        if job.get("if") != "github.event_name != 'schedule'":
            raise AssertionError(f"parallel Windows worker {job_id!r} is not a PR gate")
        if "needs" in job:
            raise AssertionError(f"parallel Windows worker {job_id!r} was serialized")
        if "continue-on-error" in job:
            raise AssertionError(f"parallel Windows worker {job_id!r} tolerates failure")

    aggregate = jobs.get("rust_windows")
    if not isinstance(aggregate, dict):
        raise AssertionError("rust_windows aggregate is missing")
    if aggregate.get("needs") != windows_workers:
        raise AssertionError("rust_windows aggregate is not bound to every worker")
    if aggregate.get("if") != "${{ always() && github.event_name != 'schedule' }}":
        raise AssertionError("rust_windows aggregate does not run fail-closed")
    aggregate_step = named_step(
        jobs,
        "rust_windows",
        "Enforce complete parallel Windows behavior gate",
    )
    if aggregate_step.get("env") != {
        "WINDOWS_TESTS_RESULT": "${{ needs.rust_windows_tests.result }}",
        "WINDOWS_RELEASE_RESULT": "${{ needs.rust_windows_release.result }}",
        "WINDOWS_COVERAGE_RESULT": "${{ needs.rust_windows_coverage.result }}",
    }:
        raise AssertionError("rust_windows aggregate result bindings changed")
    if normalized(aggregate_step.get("run", "")) != normalized(
        """
        test "$WINDOWS_TESTS_RESULT" = success
        test "$WINDOWS_RELEASE_RESULT" = success
        test "$WINDOWS_COVERAGE_RESULT" = success
        """
    ):
        raise AssertionError("rust_windows aggregate does not require every success")

    linux_producer = jobs.get("coverage_linux")
    if not isinstance(linux_producer, dict) or "needs" in linux_producer:
        raise AssertionError("Linux coverage producer is missing or serialized")
    coverage_policy = jobs.get("coverage_diff")
    if not isinstance(coverage_policy, dict):
        raise AssertionError("coverage_diff policy job is missing")
    if coverage_policy.get("needs") != ["rust_windows_coverage", "coverage_linux"]:
        raise AssertionError("coverage_diff is not bound directly to both producers")
    step_names = {step.get("name") for step in coverage_policy.get("steps", [])}
    forbidden_producer_steps = {
        "Generate merged behavioral coverage profiles",
        "Export pinned LLVM JSON",
        "Export native LLVM source view",
        "Build source-bound physical line map",
    }
    if step_names & forbidden_producer_steps:
        raise AssertionError("coverage production was moved back onto the policy path")
    expected_downloads = {
        "Download exact Windows process coverage evidence": {
            "name": "verification-windows-process-coverage",
            "path": "target/windows-process-coverage",
        },
        "Download exact Linux merged coverage evidence": {
            "name": "verification-linux-merged-coverage",
            "path": "target",
        },
    }
    for name, settings in expected_downloads.items():
        step = named_step(jobs, "coverage_diff", name)
        if step.get("uses") != PINNED_USES["actions/download-artifact"]:
            raise AssertionError(f"{name} no longer uses the pinned download action")
        if step.get("with") != settings:
            raise AssertionError(f"{name} no longer consumes the exact producer artifact")
    finalizer = named_step(
        jobs,
        "coverage_diff",
        "Enforce complete changed-line coverage evidence",
    )
    producer_result = "${{ needs.coverage_linux.result }}"
    for key in (
        "PROFILES_OUTCOME",
        "LLVM_JSON_OUTCOME",
        "LLVM_TEXT_OUTCOME",
        "LINE_MAP_OUTCOME",
    ):
        if finalizer.get("env", {}).get(key) != producer_result:
            raise AssertionError(f"coverage finalizer lost producer binding {key}")

    required_needs = jobs.get("verification_required", {}).get("needs", [])
    if "rust_windows" not in required_needs or "coverage_diff" not in required_needs:
        raise AssertionError("public aggregates are not required")
    if any(worker in required_needs for worker in windows_workers):
        raise AssertionError("internal Windows workers leaked into the public contract")
    if "coverage_linux" in required_needs:
        raise AssertionError("internal Linux producer leaked into the public contract")


class CiPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow_text = WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.workflow = parse_workflow(cls.workflow_text)
        cls.jobs = cls.workflow["jobs"]
        cls.coverage_workflow_text = COVERAGE_WORKFLOW_PATH.read_text(
            encoding="utf-8"
        )
        cls.coverage_workflow = parse_workflow(cls.coverage_workflow_text)
        cls.mutation_workflow_text = MUTATION_WORKFLOW_PATH.read_text(
            encoding="utf-8"
        )
        cls.mutation_workflow = parse_workflow(cls.mutation_workflow_text)
        with CATALOG_PATH.open("rb") as handle:
            cls.catalog = tomllib.load(handle)
        with IGNORED_TESTS_PATH.open("rb") as handle:
            cls.ignored_tests = tomllib.load(handle)
        with KNOWN_DEFECTS_PATH.open("rb") as handle:
            cls.known_defects = tomllib.load(handle)
        with MUTATION_POLICY_PATH.open("rb") as handle:
            cls.mutation_policy = tomllib.load(handle)

    def assert_exact_run(
        self,
        jobs: dict[str, Any],
        job_id: str,
        name: str,
        expected: str,
        *,
        continue_on_error: str | None = None,
        allowed_if: str | None = None,
    ) -> dict[str, Any]:
        step = named_step(jobs, job_id, name)
        if allowed_if is None:
            self.assertNotIn("if", step)
        else:
            self.assertEqual(step.get("if"), allowed_if)
        self.assertEqual(normalized(step.get("run", "")), normalized(expected))
        if continue_on_error is None:
            self.assertNotIn("continue-on-error", step)
        else:
            self.assertEqual(step.get("continue-on-error"), continue_on_error)
        return step

    def assert_mpv_version_matrix(self, jobs: dict[str, Any]) -> None:
        job = jobs["mpv-pr-semantics"]
        self.assertEqual(job.get("name"), "mpv semantics (${{ matrix.mpv_identity }})")
        self.assertNotIn("if", job)
        self.assertNotIn("continue-on-error", job)
        self.assertEqual(job.get("timeout-minutes"), "30")
        self.assertEqual(
            job.get("strategy"),
            {
                "fail-fast": "false",
                "matrix": {"mpv_identity": MPV_MATRIX_EXPRESSION},
            },
        )
        self.assertEqual(
            job.get("env"),
            {
                "MPV_MATRIX_IDENTITY": "${{ matrix.mpv_identity }}",
                "MPV_SOURCE_SHA": MPV_SOURCE_EXPRESSION,
                "MPV_MINIMUM_SOURCE_SHA": MPV_MINIMUM_SHA,
                "MPV_NEWEST_SOURCE_SHA": MPV_NEWEST_SHA,
                "MPV_MINIMUM_VERSION": "0.41.0",
            },
        )

        checkout = named_step(
            jobs,
            "mpv-pr-semantics",
            "Checkout pinned official mpv source",
        )
        self.assertEqual(checkout.get("uses"), PINNED_USES["actions/checkout"])
        self.assertEqual(
            checkout.get("with"),
            {
                "repository": "mpv-player/mpv",
                "ref": "${{ env.MPV_SOURCE_SHA }}",
                "path": "target/mpv-supported",
                "persist-credentials": "false",
            },
        )

        verify_source = self.assert_exact_run(
            jobs,
            "mpv-pr-semantics",
            "Verify supported mpv source revision",
            "test \"$(git rev-parse 'HEAD^{commit}')\" = \"$MPV_SOURCE_SHA\"",
        )
        self.assertEqual(verify_source.get("working-directory"), "target/mpv-supported")

        self.assert_exact_run(
            jobs,
            "mpv-pr-semantics",
            "Verify supported mpv version",
            """
            export PATH="$GITHUB_WORKSPACE/target/mpv-supported/build:$PATH"
            python3 scripts/mpv_version_matrix.py validate \
              --identity "$MPV_MATRIX_IDENTITY" \
              --source-sha "$MPV_SOURCE_SHA" \
              --minimum-source-sha "$MPV_MINIMUM_SOURCE_SHA" \
              --newest-source-sha "$MPV_NEWEST_SOURCE_SHA" \
              --minimum-version "$MPV_MINIMUM_VERSION" \
              --binary mpv
            """,
        )
        self.assert_exact_run(
            jobs,
            "mpv-pr-semantics",
            "Verify Sorotte candidate revision",
            "test \"$(git rev-parse 'HEAD^{commit}')\" = \"$GITHUB_SHA\"",
        )
        self.assert_exact_run(
            jobs,
            "mpv-pr-semantics",
            "Build packaged playback lifecycle candidates",
            "cargo build --locked -p sorotte-server -p sorotte-cli",
        )
        system_step = self.assert_exact_run(
            jobs,
            "mpv-pr-semantics",
            "Required packaged multi-client real mpv lifecycle",
            """
            python3 scripts/playback_lifecycle_system.py run \
              --server target/debug/sorotte-server \
              --client target/debug/sorotte-cli \
              --mpv target/mpv-supported/build/mpv \
              --ffmpeg ffmpeg \
              --artifact-dir target/verification/playback-lifecycle-system \
              --candidate-sha "$GITHUB_SHA"
            """,
            continue_on_error="true",
        )
        self.assertEqual(system_step.get("id"), "playback_lifecycle_system")
        self.assertEqual(system_step.get("continue-on-error"), "true")

        evidence_step = self.assert_exact_run(
            jobs,
            "mpv-pr-semantics",
            "Validate and stage privacy-safe lifecycle evidence",
            """
            python3 scripts/playback_lifecycle_system.py stage-safe-evidence \
              --artifact-dir target/verification/playback-lifecycle-system \
              --output-dir target/verification/playback-lifecycle-safe-evidence
            """,
            continue_on_error="true",
            allowed_if="always()",
        )
        self.assertEqual(evidence_step.get("id"), "playback_lifecycle_evidence")
        self.assertEqual(evidence_step.get("if"), "always()")
        self.assertEqual(evidence_step.get("continue-on-error"), "true")

        upload_step = named_step(
            jobs,
            "mpv-pr-semantics",
            "Upload privacy-safe packaged lifecycle evidence",
        )
        self.assertEqual(upload_step.get("uses"), PINNED_USES["actions/upload-artifact"])
        self.assertEqual(
            upload_step.get("if"),
            "${{ always() && steps.playback_lifecycle_evidence.outcome == 'success' }}",
        )
        self.assertEqual(
            upload_step.get("with"),
            {
                "name": "playback-lifecycle-system-${{ matrix.mpv_identity }}",
                "path": "target/verification/playback-lifecycle-safe-evidence",
                "if-no-files-found": "error",
                "retention-days": "14",
                "overwrite": "true",
            },
        )

        enforcement = self.assert_exact_run(
            jobs,
            "mpv-pr-semantics",
            "Enforce packaged lifecycle verification and safe evidence",
            """
            test "$PLAYBACK_LIFECYCLE_SYSTEM_OUTCOME" = success
            test "$PLAYBACK_LIFECYCLE_EVIDENCE_OUTCOME" = success
            """,
            allowed_if="always()",
        )
        self.assertEqual(enforcement.get("if"), "always()")
        self.assertEqual(
            enforcement.get("env"),
            {
                "PLAYBACK_LIFECYCLE_SYSTEM_OUTCOME": "${{ steps.playback_lifecycle_system.outcome }}",
                "PLAYBACK_LIFECYCLE_EVIDENCE_OUTCOME": "${{ steps.playback_lifecycle_evidence.outcome }}",
            },
        )

    def test_every_external_action_is_pinned_to_reviewed_commit(self) -> None:
        for path, workflow_text in (
            (WORKFLOW_PATH, self.workflow_text),
            (COVERAGE_WORKFLOW_PATH, self.coverage_workflow_text),
            (MUTATION_WORKFLOW_PATH, self.mutation_workflow_text),
        ):
            matches = USES_LINE.findall(workflow_text)
            self.assertTrue(matches, f"{path} must contain action uses")
            parsed_uses = [
                step["uses"]
                for job in parse_workflow(workflow_text)["jobs"].values()
                for step in job.get("steps", [])
                if "uses" in step
            ]
            self.assertEqual(len(matches), len(parsed_uses))
            for action, revision, comment in matches:
                with self.subTest(path=path.name, action=action):
                    self.assertIn(action, ACTION_PINS)
                    expected_revision, expected_comment = ACTION_PINS[action]
                    self.assertEqual(revision, expected_revision)
                    self.assertRegex(revision, r"^[0-9a-f]{40}$")
                    self.assertEqual(comment, expected_comment)
                    self.assertIn(f"{action}@{revision}", parsed_uses)

    def test_official_javascript_actions_are_node24_pinned_repo_wide(self) -> None:
        seen: set[str] = set()
        for path in sorted(WORKFLOWS_DIR.glob("*.yml")):
            workflow_text = path.read_text(encoding="utf-8")
            for action, revision, comment in USES_LINE.findall(workflow_text):
                if action not in NODE24_ACTIONS:
                    continue
                with self.subTest(path=path.name, action=action):
                    expected_revision, expected_comment = ACTION_PINS[action]
                    self.assertEqual(revision, expected_revision)
                    self.assertEqual(comment, expected_comment)
                    seen.add(action)
        self.assertEqual(seen, NODE24_ACTIONS)

    def test_rust_coverage_sources_are_canonical_lf_on_every_platform(self) -> None:
        self.assertEqual(
            GIT_ATTRIBUTES_PATH.read_text(encoding="utf-8").splitlines(),
            [
                "# Cross-platform LLVM line maps bind to identical Rust source bytes.",
                "*.rs text eol=lf",
            ],
        )

    def test_required_jobs_install_repository_rust_components_eagerly(self) -> None:
        setup_by_job = {
            "checks": ("Setup Rust", "rustfmt, clippy"),
            "lifecycle_contract": ("Setup Rust", "rustfmt, clippy"),
            "gui_semantic": ("Setup Rust", "rustfmt, clippy"),
            "rust_windows_tests": ("Setup Rust", "rustfmt, clippy"),
            "rust_windows_release": ("Setup Rust", "rustfmt, clippy"),
            "rust_windows_coverage": (
                "Setup Rust coverage toolchain",
                "rustfmt, clippy, llvm-tools-preview",
            ),
            "coverage_linux": (
                "Setup Rust coverage toolchain",
                "rustfmt, clippy, llvm-tools-preview",
            ),
            "compat-live-tls": ("Setup Rust", "rustfmt, clippy"),
            "media-match-generated-media": ("Setup Rust", "rustfmt, clippy"),
            "mpv-pr-semantics": ("Setup Rust", "rustfmt, clippy"),
            "nightly-deep": ("Setup Rust", "rustfmt, clippy"),
            "server-release-verify": ("Setup Rust", "rustfmt, clippy"),
        }
        for job_id, (step_name, components) in setup_by_job.items():
            with self.subTest(job_id=job_id):
                step = named_step(self.jobs, job_id, step_name)
                self.assertEqual(
                    step.get("uses"),
                    PINNED_USES["dtolnay/rust-toolchain"],
                )
                self.assertEqual(
                    step.get("with"),
                    {
                        "toolchain": "1.97.1",
                        "components": components,
                    },
                )

    def test_server_release_verification_only_deduplicates_workspace(self) -> None:
        job = self.jobs["server-release-verify"]
        self.assertEqual(
            job.get("if"),
            "github.event_name == 'workflow_dispatch' || "
            "github.event_name == 'schedule'",
        )
        self.assertNotIn("needs", job)
        self.assertEqual(
            job.get("strategy"),
            {
                "fail-fast": "false",
                "matrix": {"runner": ["ubuntu-latest", "windows-latest"]},
            },
        )
        self.assertEqual(job.get("runs-on"), "${{ matrix.runner }}")

        verification = self.assert_exact_run(
            self.jobs,
            "server-release-verify",
            "Strict server release verification",
            "./scripts/server-release-verify.ps1 -NoWorkspace",
        )
        self.assertEqual(verification.get("shell"), "pwsh")

    def test_package_freshness_compares_timestamp_instants(self) -> None:
        script = PACKAGE_PATH_BOUNDARY_TEST_PATH.read_text(encoding="utf-8")

        self.assertIn("--format=%ct", script)
        self.assertIn("$manifestCreatedAt.ToUnixTimeSeconds()", script)
        self.assertNotIn("--format=%cI", script)
        self.assertNotIn(
            "$guiManifest.created_at_utc -ne $expectedCreatedAt",
            script,
        )

    def test_required_jobs_have_structurally_bound_commands(self) -> None:
        expected_jobs = {
            "checks",
            "lifecycle_contract",
            "gui_semantic",
            "rust_windows_tests",
            "rust_windows_release",
            "rust_windows_coverage",
            "rust_windows",
            "coverage_linux",
            "coverage_diff",
            "compat-live-tls",
            "media-match-generated-media",
            "mpv-pr-semantics",
            "verification_required",
        }
        self.assertTrue(expected_jobs <= set(self.jobs))

        linux_job = self.jobs["checks"]
        self.assertEqual(
            linux_job.get("env"),
            {
                "SYNCPLAY_LEGACY_ROOT": (
                    "${{ github.workspace }}/.interop-cache/syncplay-legacy"
                )
            },
        )
        linux_checkout = named_step(self.jobs, "checks", "Checkout")
        self.assertEqual(
            linux_checkout.get("with"),
            {
                "fetch-depth": "0",
                "persist-credentials": "false",
            },
        )
        linux_legacy_checkout = named_step(
            self.jobs,
            "checks",
            "Checkout pinned legacy reference for Linux tests",
        )
        self.assertNotIn("if", linux_legacy_checkout)
        self.assertNotIn("continue-on-error", linux_legacy_checkout)
        self.assertEqual(
            linux_legacy_checkout.get("uses"),
            PINNED_USES["actions/checkout"],
        )
        self.assertEqual(
            linux_legacy_checkout.get("with"),
            {
                "repository": "Syncplay/syncplay",
                "ref": LEGACY_SYNCPLAY_SHA,
                "path": ".interop-cache/syncplay-legacy",
                "persist-credentials": "false",
            },
        )
        linux_step_names = [step.get("name") for step in linux_job["steps"]]
        self.assertLess(
            linux_step_names.index("Checkout"),
            linux_step_names.index("Checkout pinned legacy reference for Linux tests"),
        )
        self.assertLess(
            linux_step_names.index("Checkout pinned legacy reference for Linux tests"),
            linux_step_names.index("Nextest fail-on-flaky workspace tests"),
        )
        self.assertLess(
            linux_step_names.index("Setup Python"),
            linux_step_names.index("Install Linux test prerequisites"),
        )
        self.assertLess(
            linux_step_names.index("Install Linux test prerequisites"),
            linux_step_names.index("Nextest fail-on-flaky workspace tests"),
        )

        self.assert_exact_run(
            self.jobs,
            "checks",
            "Install Linux test prerequisites",
            "python -m pip install --disable-pip-version-check "
            "-r requirements/ci-policy.txt "
            "-r requirements/legacy-python-interop.txt",
        )
        actionlint_setup = named_step(
            self.jobs,
            "checks",
            "Setup actionlint toolchain",
        )
        self.assertEqual(
            actionlint_setup.get("uses"),
            PINNED_USES["actions/setup-go"],
        )
        self.assertEqual(
            actionlint_setup.get("with"),
            {"go-version": "1.26.x", "cache": "false"},
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Validate GitHub Actions workflows",
            "go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12",
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Behavior evidence self-tests",
            'python -m unittest discover -s scripts/tests -p "test_*.py" -v',
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Validate behavior catalog",
            "python scripts/behavior_evidence.py validate "
            "--catalog coverage/behaviors.toml",
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Validate playback lifecycle model",
            "python scripts/playback_lifecycle_model.py validate "
            "--model coverage/playback-lifecycle.toml",
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Execute playback lifecycle oracle",
            "python scripts/playback_lifecycle_oracle.py witness-summary "
            "--model coverage/playback-lifecycle.toml --compact",
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Replay playback lifecycle seeds",
            "python scripts/playback_lifecycle_oracle.py run-suite "
            "--schedule-dir fixtures/playback-lifecycle",
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Required state-aware playback lifecycle exploration",
            """
            python scripts/playback_lifecycle_oracle.py explore
            --model coverage/playback-lifecycle.toml
            --seed 0x50A077E20260831
            --cases 64
            --steps 128
            --failure-dir target/verification/playback-lifecycle-model-failures
            --compact
            """,
            continue_on_error="true",
        )
        exploration_enforcement = self.assert_exact_run(
            self.jobs,
            "checks",
            "Enforce state-aware playback lifecycle exploration",
            'test "$LIFECYCLE_EXPLORE_OUTCOME" = success',
            allowed_if="always()",
        )
        self.assertEqual(
            exploration_enforcement["env"],
            {"LIFECYCLE_EXPLORE_OUTCOME": "${{ steps.lifecycle_explore.outcome }}"},
        )
        self.assert_exact_run(
            self.jobs,
            "nightly-deep",
            "Nightly state-aware playback lifecycle exploration",
            """
            python scripts/playback_lifecycle_oracle.py explore
            --model coverage/playback-lifecycle.toml
            --seed 0x50A077E20260831
            --cases 512
            --steps 256
            --failure-dir target/verification/playback-lifecycle-model-failures
            --compact
            """,
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Validate ignored test policy",
            "python scripts/ignored_test_policy.py validate "
            "--registry coverage/ignored-tests.toml",
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Validate known-defect characterizations",
            "python scripts/known_defect_policy.py validate "
            "--registry coverage/known-defects.toml "
            "--catalog coverage/behaviors.toml",
        )
        self.assert_exact_run(
            self.jobs,
            "checks",
            "Cargo clippy",
            "cargo clippy --locked --workspace --all-targets --all-features "
            "-- -D warnings",
        )
        linux_nextest_installer = named_step(
            self.jobs,
            "checks",
            "Install pinned cargo-nextest",
        )
        self.assertNotIn("if", linux_nextest_installer)
        self.assertNotIn("continue-on-error", linux_nextest_installer)
        self.assertEqual(
            linux_nextest_installer.get("uses"),
            PINNED_USES["taiki-e/install-action"],
        )
        self.assertEqual(
            linux_nextest_installer.get("with"),
            {
                "tool": "cargo-nextest@0.9.137",
                "fallback": "none",
            },
        )
        linux_semver_installer = named_step(
            self.jobs,
            "checks",
            "Install pinned cargo-semver-checks",
        )
        self.assertEqual(
            linux_semver_installer.get("if"),
            "github.event_name == 'pull_request'",
        )
        self.assertNotIn("continue-on-error", linux_semver_installer)
        self.assertEqual(
            linux_semver_installer.get("uses"),
            PINNED_USES["taiki-e/install-action"],
        )
        self.assertEqual(
            linux_semver_installer.get("with"),
            {
                "tool": "cargo-semver-checks@0.50.0",
                "fallback": "none",
            },
        )
        linux_semver = self.assert_exact_run(
            self.jobs,
            "checks",
            "Enforce public Rust API compatibility",
            """set -euo pipefail
baseline_sha="${{ github.event.pull_request.base.sha }}"
git cat-file -e "${baseline_sha}^{commit}"
for package in \\
  sorotte-secret \\
  sorotte-protocol \\
  sorotte-core \\
  sorotte-server \\
  sorotte-media-match \\
  sorotte-client-core \\
  sorotte-client-app \\
  sorotte-player-api \\
  sorotte-player-mpv \\
  sorotte-lifecycle-evidence \\
  sorotte-plex \\
  sorotte-cli \\
  sorotte-gui \\
  sorotte-sim \\
  sorotte-compat
do
  if ! git cat-file -e "${baseline_sha}:crates/${package}/Cargo.toml" 2>/dev/null
  then
    echo "Skipping new package absent from baseline: ${package}"
    continue
  fi
  cargo semver-checks \\
    --package "$package" \\
    --baseline-rev "$baseline_sha"
done""",
            allowed_if="github.event_name == 'pull_request'",
        )
        self.assertLess(
            linux_step_names.index("Install pinned cargo-semver-checks"),
            linux_step_names.index("Enforce public Rust API compatibility"),
        )
        self.assertNotIn("env", linux_semver)
        self.assertEqual(linux_semver.get("shell"), "bash")
        linux_nextest = self.assert_exact_run(
            self.jobs,
            "checks",
            "Nextest fail-on-flaky workspace tests",
            "python scripts/nextest_ci.py run --repo-root .",
            continue_on_error="true",
        )
        self.assertEqual(linux_nextest.get("id"), "nextest")
        linux_doctests = self.assert_exact_run(
            self.jobs,
            "checks",
            "Cargo doctests",
            "cargo test --locked --workspace --all-features --doc",
            continue_on_error="true",
        )
        self.assertEqual(linux_doctests.get("id"), "doctests")
        linux_attempts = named_step(
            self.jobs,
            "checks",
            "Upload Linux nextest attempt evidence",
        )
        self.assertEqual(linux_attempts.get("if"), "always()")
        self.assertEqual(
            linux_attempts.get("uses"),
            PINNED_USES["actions/upload-artifact"],
        )
        self.assertEqual(
            linux_attempts.get("with"),
            {
                "name": "nextest-attempts-linux-${{ github.run_attempt }}",
                "path": "target/nextest/ci",
                "if-no-files-found": "error",
                "retention-days": "14",
                "overwrite": "true",
            },
        )
        linux_enforcement = self.assert_exact_run(
            self.jobs,
            "checks",
            "Enforce complete Linux test gate",
            """
            test "$NEXTEST_OUTCOME" = success
            test "$DOCTEST_OUTCOME" = success
            """,
            allowed_if="always()",
        )
        self.assertEqual(
            linux_enforcement.get("env"),
            {
                "NEXTEST_OUTCOME": "${{ steps.nextest.outcome }}",
                "DOCTEST_OUTCOME": "${{ steps.doctests.outcome }}",
            },
        )

        coverage_producer = self.jobs["coverage_linux"]
        self.assertNotIn("needs", coverage_producer)
        self.assertEqual(
            coverage_producer.get("env"),
            {
                "SYNCPLAY_LEGACY_ROOT": (
                    "${{ github.workspace }}/.interop-cache/syncplay-legacy"
                )
            },
        )
        coverage_legacy = named_step(
            self.jobs,
            "coverage_linux",
            "Checkout pinned legacy reference for merged coverage",
        )
        self.assertEqual(
            coverage_legacy.get("uses"),
            PINNED_USES["actions/checkout"],
        )
        self.assertEqual(
            coverage_legacy.get("with"),
            {
                "repository": "Syncplay/syncplay",
                "ref": LEGACY_SYNCPLAY_SHA,
                "path": ".interop-cache/syncplay-legacy",
                "persist-credentials": "false",
            },
        )
        self.assert_exact_run(
            self.jobs,
            "coverage_linux",
            "Install merged coverage prerequisites",
            "python -m pip install --disable-pip-version-check "
            "-r requirements/legacy-python-interop.txt",
        )
        coverage_installer = named_step(
            self.jobs,
            "coverage_linux",
            "Install pinned cargo-llvm-cov",
        )
        self.assertNotIn("if", coverage_installer)
        self.assertNotIn("continue-on-error", coverage_installer)
        self.assertEqual(
            coverage_installer.get("uses"),
            PINNED_USES["taiki-e/install-action"],
        )
        self.assertEqual(
            coverage_installer.get("with"),
            {"tool": "cargo-llvm-cov@0.8.4"},
        )
        profiles = self.assert_exact_run(
            self.jobs,
            "coverage_linux",
            "Generate merged behavioral coverage profiles",
            """
            python scripts/coverage_profile_lanes.py run
            --repo-root .
            --output target/verification/coverage-profile-lanes.json
            """,
            continue_on_error="true",
        )
        self.assertEqual(profiles.get("id"), "coverage_profiles")
        llvm_json = self.assert_exact_run(
            self.jobs,
            "coverage_linux",
            "Export pinned LLVM JSON",
            "cargo llvm-cov report --json --skip-functions "
            "--output-path target/diff-coverage.json",
            continue_on_error="true",
            allowed_if="steps.coverage_profiles.outcome == 'success'",
        )
        self.assertEqual(llvm_json.get("id"), "llvm_json")
        llvm_text = self.assert_exact_run(
            self.jobs,
            "coverage_linux",
            "Export native LLVM source view",
            "cargo llvm-cov report --text "
            "--output-path target/diff-coverage.txt",
            continue_on_error="true",
            allowed_if="steps.coverage_profiles.outcome == 'success'",
        )
        self.assertEqual(llvm_text.get("id"), "llvm_text")
        line_map = self.assert_exact_run(
            self.jobs,
            "coverage_linux",
            "Build source-bound physical line map",
            """
            python scripts/llvm_cov_line_map.py
            --repo-root .
            --llvm-json target/diff-coverage.json
            --llvm-text target/diff-coverage.txt
            --output target/verification/coverage-line-map.json
            """,
            continue_on_error="true",
            allowed_if=(
                "steps.llvm_json.outcome == 'success' && "
                "steps.llvm_text.outcome == 'success'"
            ),
        )
        self.assertEqual(line_map.get("id"), "line_map")
        coverage_upload = named_step(
            self.jobs,
            "coverage_linux",
            "Upload Linux merged coverage evidence",
        )
        self.assertEqual(coverage_upload.get("if"), "always()")
        self.assertEqual(
            coverage_upload.get("uses"),
            PINNED_USES["actions/upload-artifact"],
        )
        self.assertEqual(
            coverage_upload.get("with"),
            {
                "name": "verification-linux-merged-coverage",
                "path": (
                    "target/diff-coverage.json\n"
                    "target/diff-coverage.txt\n"
                    "target/verification/coverage-line-map.json\n"
                    "target/verification/coverage-profile-lanes.json\n"
                    "target/verification/coverage-profile-logs/\n"
                ),
                "if-no-files-found": "warn",
                "retention-days": "14",
                "overwrite": "true",
            },
        )
        coverage_enforcement = self.assert_exact_run(
            self.jobs,
            "coverage_linux",
            "Enforce complete Linux coverage producer",
            """
            test "$PROFILES_OUTCOME" = success
            test "$LLVM_JSON_OUTCOME" = success
            test "$LLVM_TEXT_OUTCOME" = success
            test "$LINE_MAP_OUTCOME" = success
            """,
            allowed_if="always()",
        )
        self.assertEqual(
            coverage_enforcement.get("env"),
            {
                "PROFILES_OUTCOME": "${{ steps.coverage_profiles.outcome }}",
                "LLVM_JSON_OUTCOME": "${{ steps.llvm_json.outcome }}",
                "LLVM_TEXT_OUTCOME": "${{ steps.llvm_text.outcome }}",
                "LINE_MAP_OUTCOME": "${{ steps.line_map.outcome }}",
            },
        )

        coverage_job = self.jobs["coverage_diff"]
        self.assertEqual(
            coverage_job.get("needs"),
            ["rust_windows_coverage", "coverage_linux"],
        )
        self.assertNotIn("env", coverage_job)
        windows_coverage_download = named_step(
            self.jobs,
            "coverage_diff",
            "Download exact Windows process coverage evidence",
        )
        self.assertEqual(
            windows_coverage_download.get("uses"),
            PINNED_USES["actions/download-artifact"],
        )
        self.assertEqual(
            windows_coverage_download.get("with"),
            {
                "name": "verification-windows-process-coverage",
                "path": "target/windows-process-coverage",
            },
        )
        linux_coverage_download = named_step(
            self.jobs,
            "coverage_diff",
            "Download exact Linux merged coverage evidence",
        )
        self.assertEqual(
            linux_coverage_download.get("uses"),
            PINNED_USES["actions/download-artifact"],
        )
        self.assertEqual(
            linux_coverage_download.get("with"),
            {
                "name": "verification-linux-merged-coverage",
                "path": "target",
            },
        )
        resolve_base = self.assert_exact_run(
            self.jobs,
            "coverage_diff",
            "Resolve changed-line coverage base",
            """
            python scripts/coverage_ci_guard.py resolve-base
            --repo-root .
            --event-name "$EVENT_NAME"
            --verification-sha "$VERIFICATION_SHA"
            --pull-request-base "$PULL_REQUEST_BASE_SHA"
            --push-before "$PUSH_BEFORE_SHA"
            --push-ref-type "$PUSH_REF_TYPE"
            --default-branch "$DEFAULT_BRANCH"
            --dispatch-base "$DISPATCH_BASE_SHA"
            --github-env "$GITHUB_ENV"
            --output target/verification/coverage-base.json
            """,
            continue_on_error="true",
        )
        self.assertEqual(resolve_base.get("id"), "coverage_base")
        self.assertEqual(
            resolve_base.get("env"),
            {
                "EVENT_NAME": "${{ github.event_name }}",
                "PULL_REQUEST_BASE_SHA": (
                    "${{ github.event.pull_request.base.sha || '' }}"
                ),
                "PUSH_BEFORE_SHA": "${{ github.event.before || '' }}",
                "PUSH_REF_TYPE": "${{ github.ref_type }}",
                "DEFAULT_BRANCH": "${{ github.event.repository.default_branch }}",
                "DISPATCH_BASE_SHA": "${{ inputs.coverage_base_sha || '' }}",
            },
        )
        policy = self.assert_exact_run(
            self.jobs,
            "coverage_diff",
            "Enforce production changed-line coverage",
            """
            python scripts/diff_coverage.py
            --repo-root .
            --coverage-map target/verification/coverage-line-map.json
            --coverage-map target/windows-process-coverage/verification/coverage-windows-process-line-map.json
            --critical-policy coverage/diff-coverage-policy.toml
            --base "$COVERAGE_BASE_SHA"
            --head "$VERIFICATION_SHA"
            --minimum 80
            --json-out target/verification/diff-coverage.json
            """,
            continue_on_error="true",
            allowed_if="steps.coverage_base.outcome == 'success'",
        )
        self.assertEqual(policy.get("id"), "policy")
        finalizer = self.assert_exact_run(
            self.jobs,
            "coverage_diff",
            "Enforce complete changed-line coverage evidence",
            """
            python scripts/coverage_ci_guard.py finalize
            --base-outcome "$BASE_OUTCOME"
            --profiles-outcome "$PROFILES_OUTCOME"
            --llvm-json-outcome "$LLVM_JSON_OUTCOME"
            --llvm-text-outcome "$LLVM_TEXT_OUTCOME"
            --line-map-outcome "$LINE_MAP_OUTCOME"
            --policy-outcome "$POLICY_OUTCOME"
            --base-report target/verification/coverage-base.json
            --llvm-json target/diff-coverage.json
            --llvm-text target/diff-coverage.txt
            --line-map target/verification/coverage-line-map.json
            --supplemental-line-map target/windows-process-coverage/verification/coverage-windows-process-line-map.json
            --policy-report target/verification/diff-coverage.json
            --profile-lanes target/verification/coverage-profile-lanes.json
            --output target/verification/coverage-ci-phases.json
            """,
            allowed_if="always()",
        )
        self.assertEqual(
            finalizer.get("env"),
            {
                "BASE_OUTCOME": "${{ steps.coverage_base.outcome }}",
                "PROFILES_OUTCOME": "${{ needs.coverage_linux.result }}",
                "LLVM_JSON_OUTCOME": "${{ needs.coverage_linux.result }}",
                "LLVM_TEXT_OUTCOME": "${{ needs.coverage_linux.result }}",
                "LINE_MAP_OUTCOME": "${{ needs.coverage_linux.result }}",
                "POLICY_OUTCOME": "${{ steps.policy.outcome }}",
            },
        )

        self.assert_exact_run(
            self.jobs,
            "lifecycle_contract",
            "Reducer and acceptance lifecycle inventory",
            "cargo test --locked -p sorotte-player-mpv --all-features --lib "
            "lifecycle:: -- --nocapture",
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "lifecycle_contract",
            "GUI lifecycle projection inventory",
            "cargo test --locked -p sorotte-gui --all-features --lib "
            "lifecycle_verification_tests -- --test-threads=1",
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "lifecycle_contract",
            "GUI ordered-delivery inventory",
            "cargo test --locked -p sorotte-gui --all-features --lib "
            "ordered_delivery_tests -- --nocapture --test-threads=1",
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "lifecycle_contract",
            "Run exact behavior proofs",
            """
            python scripts/behavior_evidence.py run-lane
            --catalog coverage/behaviors.toml
            --lane lifecycle-contract
            --sha "$VERIFICATION_SHA"
            --repository "${{ github.repository }}"
            --run-id "${{ github.run_id }}"
            --run-attempt "${{ github.run_attempt }}"
            --os linux
            --output target/verification/evidence.lifecycle-contract.json
            """,
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "lifecycle_contract",
            "Enforce complete lifecycle lane",
            """
            test "$REDUCER_OUTCOME" = success
            test "$PROJECTION_OUTCOME" = success
            test "$DELIVERY_OUTCOME" = success
            test "$EVIDENCE_OUTCOME" = success
            """,
            allowed_if="always()",
        )

        self.assert_exact_run(
            self.jobs,
            "gui_semantic",
            "Run complete semantic inventory on prospective merge",
            """
            cargo run --quiet --locked -p sorotte-gui
            --features gui-semantic-smoke,live-python-interop
            --bin sorotte-gui-semantic-suite --
            --json
            """,
        )
        self.assert_exact_run(
            self.jobs,
            "gui_semantic",
            "Run complete semantic inventory",
            """
            python scripts/behavior_evidence.py run-lane
            --catalog coverage/behaviors.toml
            --lane gui-semantic
            --sha "$VERIFICATION_SHA"
            --repository "${{ github.repository }}"
            --run-id "${{ github.run_id }}"
            --run-attempt "${{ github.run_attempt }}"
            --os linux
            --output target/verification/evidence.gui-semantic.json
            """,
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "gui_semantic",
            "Reconcile exact evidence prerequisites",
            "python -m pip install --disable-pip-version-check "
            "-r requirements/legacy-python-interop.txt",
        )
        self.assert_exact_run(
            self.jobs,
            "gui_semantic",
            "Enforce complete semantic lane",
            'test "$EVIDENCE_OUTCOME" = success',
            allowed_if="always()",
        )

        windows_nextest_installer = named_step(
            self.jobs,
            "rust_windows_tests",
            "Install pinned cargo-nextest",
        )
        self.assertNotIn("if", windows_nextest_installer)
        self.assertNotIn("continue-on-error", windows_nextest_installer)
        self.assertEqual(
            windows_nextest_installer.get("uses"),
            PINNED_USES["taiki-e/install-action"],
        )
        self.assertEqual(
            windows_nextest_installer.get("with"),
            {
                "tool": "cargo-nextest@0.9.137",
                "fallback": "none",
            },
        )
        windows_nextest = self.assert_exact_run(
            self.jobs,
            "rust_windows_tests",
            "Nextest fail-on-flaky workspace tests",
            "python scripts/nextest_ci.py run --repo-root .",
            continue_on_error="true",
        )
        self.assertEqual(windows_nextest.get("id"), "nextest")
        self.assert_exact_run(
            self.jobs,
            "rust_windows_tests",
            "Validate Windows semver wrapper",
            "python -m unittest scripts.tests.test_semver_wrapper -v",
        )
        windows_doctests = self.assert_exact_run(
            self.jobs,
            "rust_windows_tests",
            "Cargo doctests",
            "cargo test --locked --workspace --all-features --doc",
            continue_on_error="true",
        )
        self.assertEqual(windows_doctests.get("id"), "doctests")
        windows_attempts = named_step(
            self.jobs,
            "rust_windows_tests",
            "Upload Windows nextest attempt evidence",
        )
        self.assertEqual(windows_attempts.get("if"), "always()")
        self.assertEqual(
            windows_attempts.get("uses"),
            PINNED_USES["actions/upload-artifact"],
        )
        self.assertEqual(
            windows_attempts.get("with"),
            {
                "name": "nextest-attempts-windows-${{ github.run_attempt }}",
                "path": "target/nextest/ci",
                "if-no-files-found": "error",
                "retention-days": "14",
                "overwrite": "true",
            },
        )
        windows_test_enforcement = self.assert_exact_run(
            self.jobs,
            "rust_windows_tests",
            "Enforce complete Windows test gate",
            """
            if ($env:NEXTEST_OUTCOME -ne "success") { throw "nextest failed or found a flaky test" }
            if ($env:DOCTEST_OUTCOME -ne "success") { throw "doctests failed" }
            """,
            allowed_if="always()",
        )
        self.assertEqual(
            windows_test_enforcement.get("env"),
            {
                "NEXTEST_OUTCOME": "${{ steps.nextest.outcome }}",
                "DOCTEST_OUTCOME": "${{ steps.doctests.outcome }}",
            },
        )

        self.assert_exact_run(
            self.jobs,
            "rust_windows_release",
            "Locked release-profile GUI and updater build",
            "cargo build --locked --release -p sorotte-gui "
            "--bin sorotte-gui --bin sorotte-gui-updater",
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "rust_windows_release",
            "Package path boundary regressions",
            "./scripts/package-path-boundary-tests.ps1",
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "rust_windows_release",
            "Release publication policy regressions",
            "./scripts/release-publication-policy-tests.ps1",
            continue_on_error="true",
        )
        windows_release_enforcement = self.assert_exact_run(
            self.jobs,
            "rust_windows_release",
            "Enforce complete Windows release gate",
            """
            if ($env:RELEASE_BUILD_OUTCOME -ne "success") { throw "release build failed" }
            if ($env:PACKAGE_PATHS_OUTCOME -ne "success") { throw "package path tests failed" }
            if ($env:RELEASE_POLICY_OUTCOME -ne "success") { throw "release policy tests failed" }
            """,
            allowed_if="always()",
        )
        self.assertEqual(
            windows_release_enforcement.get("env"),
            {
                "RELEASE_BUILD_OUTCOME": "${{ steps.release_build.outcome }}",
                "PACKAGE_PATHS_OUTCOME": "${{ steps.package_paths.outcome }}",
                "RELEASE_POLICY_OUTCOME": "${{ steps.release_policy.outcome }}",
            },
        )

        windows_coverage_installer = named_step(
            self.jobs,
            "rust_windows_coverage",
            "Install pinned cargo-llvm-cov for Windows process coverage",
        )
        self.assertNotIn("if", windows_coverage_installer)
        self.assertNotIn("continue-on-error", windows_coverage_installer)
        self.assertEqual(
            windows_coverage_installer.get("uses"),
            PINNED_USES["taiki-e/install-action"],
        )
        self.assertEqual(
            windows_coverage_installer.get("with"),
            {"tool": "cargo-llvm-cov@0.8.4"},
        )
        windows_coverage_checkout = named_step(
            self.jobs,
            "rust_windows_coverage",
            "Checkout exact Windows coverage revision",
        )
        self.assertEqual(
            windows_coverage_checkout.get("uses"),
            PINNED_USES["actions/checkout"],
        )
        self.assertEqual(
            windows_coverage_checkout.get("with"),
            {
                "ref": HEAD_REF,
                "persist-credentials": "false",
            },
        )
        windows_coverage = self.assert_exact_run(
            self.jobs,
            "rust_windows_coverage",
            "Generate exact Windows process coverage profiles",
            """
            python scripts/coverage_windows_process_lanes.py run
            --repo-root .
            --output target/verification/coverage-windows-process-lanes.json
            """,
            continue_on_error="true",
        )
        self.assertEqual(windows_coverage.get("id"), "windows_coverage")
        self.assertNotIn("working-directory", windows_coverage)
        windows_llvm_json = self.assert_exact_run(
            self.jobs,
            "rust_windows_coverage",
            "Export Windows LLVM JSON",
            "cargo llvm-cov report --json --skip-functions "
            "--output-path target/coverage-windows-process.json",
            continue_on_error="true",
            allowed_if="steps.windows_coverage.outcome == 'success'",
        )
        self.assertEqual(windows_llvm_json.get("id"), "windows_llvm_json")
        self.assertNotIn("working-directory", windows_llvm_json)
        self.assertEqual(
            windows_llvm_json.get("env"),
            {"CARGO_TARGET_DIR": "target/llvm-cov-windows-process"},
        )
        windows_llvm_text = self.assert_exact_run(
            self.jobs,
            "rust_windows_coverage",
            "Export Windows native LLVM source view",
            "cargo llvm-cov report --text "
            "--output-path target/coverage-windows-process.txt",
            continue_on_error="true",
            allowed_if="steps.windows_coverage.outcome == 'success'",
        )
        self.assertEqual(windows_llvm_text.get("id"), "windows_llvm_text")
        self.assertNotIn("working-directory", windows_llvm_text)
        self.assertEqual(
            windows_llvm_text.get("env"),
            {"CARGO_TARGET_DIR": "target/llvm-cov-windows-process"},
        )
        windows_line_map = self.assert_exact_run(
            self.jobs,
            "rust_windows_coverage",
            "Build Windows source-bound physical line map",
            """
            python scripts/llvm_cov_line_map.py
            --repo-root .
            --llvm-json target/coverage-windows-process.json
            --llvm-text target/coverage-windows-process.txt
            --output target/verification/coverage-windows-process-line-map.json
            """,
            continue_on_error="true",
            allowed_if=(
                "steps.windows_llvm_json.outcome == 'success' && "
                "steps.windows_llvm_text.outcome == 'success'"
            ),
        )
        self.assertEqual(windows_line_map.get("id"), "windows_line_map")
        self.assertNotIn("working-directory", windows_line_map)
        windows_coverage_upload = named_step(
            self.jobs,
            "rust_windows_coverage",
            "Upload Windows process coverage evidence",
        )
        self.assertEqual(windows_coverage_upload.get("if"), "always()")
        self.assertEqual(
            windows_coverage_upload.get("uses"),
            PINNED_USES["actions/upload-artifact"],
        )
        windows_upload_settings = windows_coverage_upload.get("with", {})
        self.assertEqual(
            {
                key: value
                for key, value in windows_upload_settings.items()
                if key != "path"
            },
            {
                "name": "verification-windows-process-coverage",
                "if-no-files-found": "warn",
                "retention-days": "14",
                "overwrite": "true",
            },
        )
        self.assertEqual(
            windows_upload_settings["path"].splitlines(),
            [
                "target/coverage-windows-process.json",
                "target/coverage-windows-process.txt",
                "target/verification/coverage-windows-process-line-map.json",
                "target/verification/coverage-windows-process-lanes.json",
                "target/verification/coverage-windows-process-logs/",
            ],
        )
        windows_coverage_enforcement = self.assert_exact_run(
            self.jobs,
            "rust_windows_coverage",
            "Enforce complete Windows coverage gate",
            """
            if ($env:WINDOWS_COVERAGE_OUTCOME -ne "success") { throw "Windows process coverage profiles failed" }
            if ($env:WINDOWS_LLVM_JSON_OUTCOME -ne "success") { throw "Windows LLVM JSON export failed" }
            if ($env:WINDOWS_LLVM_TEXT_OUTCOME -ne "success") { throw "Windows LLVM source export failed" }
            if ($env:WINDOWS_LINE_MAP_OUTCOME -ne "success") { throw "Windows physical line map failed" }
            """,
            allowed_if="always()",
        )
        self.assertEqual(
            windows_coverage_enforcement.get("env"),
            {
                "WINDOWS_COVERAGE_OUTCOME": "${{ steps.windows_coverage.outcome }}",
                "WINDOWS_LLVM_JSON_OUTCOME": "${{ steps.windows_llvm_json.outcome }}",
                "WINDOWS_LLVM_TEXT_OUTCOME": "${{ steps.windows_llvm_text.outcome }}",
                "WINDOWS_LINE_MAP_OUTCOME": "${{ steps.windows_line_map.outcome }}",
            },
        )

        windows_aggregate = self.jobs["rust_windows"]
        self.assertEqual(
            windows_aggregate.get("if"),
            "${{ always() && github.event_name != 'schedule' }}",
        )
        self.assertEqual(
            windows_aggregate.get("needs"),
            [
                "rust_windows_tests",
                "rust_windows_release",
                "rust_windows_coverage",
            ],
        )
        windows_aggregate_step = self.assert_exact_run(
            self.jobs,
            "rust_windows",
            "Enforce complete parallel Windows behavior gate",
            """
            test "$WINDOWS_TESTS_RESULT" = success
            test "$WINDOWS_RELEASE_RESULT" = success
            test "$WINDOWS_COVERAGE_RESULT" = success
            """,
        )
        self.assertEqual(
            windows_aggregate_step.get("env"),
            {
                "WINDOWS_TESTS_RESULT": "${{ needs.rust_windows_tests.result }}",
                "WINDOWS_RELEASE_RESULT": "${{ needs.rust_windows_release.result }}",
                "WINDOWS_COVERAGE_RESULT": "${{ needs.rust_windows_coverage.result }}",
            },
        )

        compatibility = self.assert_exact_run(
            self.jobs,
            "compat-live-tls",
            "Strict complete live Python compatibility",
            "python scripts/compat_live_interop.py run --repo-root . "
            "--output target/verification/compat-live-interop.json",
        )
        self.assertEqual(
            compatibility.get("env"),
            {"SYNCPLAY_REQUIRE_LIVE_INTEROP": "1"},
        )
        compatibility_upload = named_step(
            self.jobs,
            "compat-live-tls",
            "Upload live compatibility evidence",
        )
        self.assertEqual(
            compatibility_upload.get("uses"),
            PINNED_USES["actions/upload-artifact"],
        )
        self.assertEqual(compatibility_upload.get("if"), "always()")
        self.assertEqual(
            compatibility_upload.get("with", {}).get("if-no-files-found"),
            "error",
        )

        self.assert_exact_run(
            self.jobs,
            "media-match-generated-media",
            "Install generated-media tools",
            """
            sudo apt-get update
            sudo apt-get install --yes --no-install-recommends ffmpeg
            """,
        )
        self.assert_exact_run(
            self.jobs,
            "media-match-generated-media",
            "Verify generated-media tools",
            """
            command -v ffmpeg
            command -v ffprobe
            ffmpeg -version
            ffprobe -version
            """,
        )
        self.assert_exact_run(
            self.jobs,
            "media-match-generated-media",
            "Required generated-media Media Match V3 diagnostic",
            """
            cargo test --locked -p sorotte-media-match --test generated_media_v3
            v3_manifest_harness_runs_small_synthetic_case
            -- --ignored --exact --nocapture
            """,
        )

        self.assert_mpv_version_matrix(self.jobs)

        expected_mpv = {
            "Required real mpv pause, seek, resume, and bounded-fetch semantics": """
                export PATH="$GITHUB_WORKSPACE/target/mpv-supported/build:$PATH"
                cargo test --locked -p sorotte-sim --test mpv_rebuffer_harness
                real_mpv_pause_seek_resume_semantics -- --ignored --exact --nocapture
            """,
            "Required real mpv cache-cap drain and input-resume semantics": """
                export PATH="$GITHUB_WORKSPACE/target/mpv-supported/build:$PATH"
                cargo test --locked -p sorotte-sim --test mpv_rebuffer_harness
                real_mpv_cache_cap_drains_and_input_resumes
                -- --ignored --exact --nocapture
            """,
            "Required real mpv premature-disconnect recovery semantics": """
                export PATH="$GITHUB_WORKSPACE/target/mpv-supported/build:$PATH"
                cargo test --locked -p sorotte-sim --test mpv_rebuffer_harness
                real_mpv_premature_http_disconnect_recovers_same_media_generation
                -- --ignored --exact --nocapture
            """,
            "Full deterministic HTTP-stall and rebuffer harness": """
                export PATH="$GITHUB_WORKSPACE/target/mpv-supported/build:$PATH"
                cargo test --locked -p sorotte-sim --test mpv_rebuffer_harness
                real_mpv_clients_keep_seek_recovery_bounded_during_an_http_stall
                -- --ignored --exact --nocapture
            """,
        }
        for name, command in expected_mpv.items():
            self.assert_exact_run(self.jobs, "mpv-pr-semantics", name, command)

    def test_parallel_ci_graph_rejects_serial_or_fail_open_mutations(self) -> None:
        validate_parallel_ci_graph(self.jobs)
        mutations: list[tuple[str, dict[str, Any]]] = []

        missing_worker = copy.deepcopy(self.jobs)
        missing_worker["rust_windows"]["needs"].remove("rust_windows_release")
        mutations.append(("missing-windows-worker", missing_worker))

        serialized_windows = copy.deepcopy(self.jobs)
        serialized_windows["rust_windows_release"]["needs"] = "rust_windows_tests"
        mutations.append(("serialized-windows-worker", serialized_windows))

        serialized_policy = copy.deepcopy(self.jobs)
        serialized_policy["coverage_diff"]["needs"] = [
            "rust_windows",
            "coverage_linux",
        ]
        mutations.append(("serialized-coverage-policy", serialized_policy))

        weakened_aggregate = copy.deepcopy(self.jobs)
        aggregate = named_step(
            weakened_aggregate,
            "rust_windows",
            "Enforce complete parallel Windows behavior gate",
        )
        aggregate["run"] = aggregate["run"].replace(
            'test "$WINDOWS_RELEASE_RESULT" = success',
            'test "$WINDOWS_RELEASE_RESULT" != failure',
        )
        mutations.append(("weakened-windows-aggregate", weakened_aggregate))

        missing_download = copy.deepcopy(self.jobs)
        missing_download["coverage_diff"]["steps"] = [
            step
            for step in missing_download["coverage_diff"]["steps"]
            if step.get("name") != "Download exact Linux merged coverage evidence"
        ]
        mutations.append(("missing-linux-artifact", missing_download))

        producer_on_policy_path = copy.deepcopy(self.jobs)
        producer_on_policy_path["coverage_diff"]["steps"].append(
            {
                "name": "Generate merged behavioral coverage profiles",
                "run": "echo serialized",
            }
        )
        mutations.append(("producer-on-policy-path", producer_on_policy_path))

        unbound_finalizer = copy.deepcopy(self.jobs)
        finalizer = named_step(
            unbound_finalizer,
            "coverage_diff",
            "Enforce complete changed-line coverage evidence",
        )
        finalizer["env"]["LINE_MAP_OUTCOME"] = "success"
        mutations.append(("unbound-producer-result", unbound_finalizer))

        for mutation, jobs in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                validate_parallel_ci_graph(jobs)

    def test_mpv_version_matrix_rejects_missing_newest_or_floating_sources(
        self,
    ) -> None:
        mutations: list[tuple[str, dict[str, Any]]] = []

        missing_newest = copy.deepcopy(self.jobs)
        missing_newest["mpv-pr-semantics"]["strategy"]["matrix"][
            "mpv_identity"
        ] = "${{ fromJSON('[\"minimum\"]') }}"
        mutations.append(("missing-newest", missing_newest))

        floating_newest = copy.deepcopy(self.jobs)
        floating_newest["mpv-pr-semantics"]["env"]["MPV_NEWEST_SOURCE_SHA"] = (
            "master"
        )
        mutations.append(("floating-newest", floating_newest))

        collapsed_endpoints = copy.deepcopy(self.jobs)
        collapsed_endpoints["mpv-pr-semantics"]["env"]["MPV_NEWEST_SOURCE_SHA"] = (
            MPV_MINIMUM_SHA
        )
        mutations.append(("collapsed-endpoints", collapsed_endpoints))

        fail_fast = copy.deepcopy(self.jobs)
        fail_fast["mpv-pr-semantics"]["strategy"]["fail-fast"] = "true"
        mutations.append(("fail-fast", fail_fast))

        for mutation, jobs in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                self.assert_mpv_version_matrix(jobs)

    def test_pull_request_ignored_tests_are_explicitly_invoked(self) -> None:
        validate_pull_request_ignored_bindings(
            self.jobs,
            self.catalog,
            self.ignored_tests,
        )

    def test_ignored_test_bindings_reject_adversarial_workflow_mutations(self) -> None:
        first_entry = next(
            entry
            for entry in self.ignored_tests["ignored_test"]
            if entry["tier"] == "pull-request"
        )

        def clone() -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
            return (
                copy.deepcopy(self.jobs),
                copy.deepcopy(self.catalog),
                copy.deepcopy(self.ignored_tests),
            )

        def exact_step(jobs: dict[str, Any]) -> dict[str, Any]:
            expected = ignored_test_cargo_command(first_entry)
            return next(
                step
                for step in jobs[first_entry["required_job"]]["steps"]
                if any(
                    command == expected
                    for command in logical_shell_commands(step.get("run", ""))
                )
            )

        jobs, catalog, registry = clone()
        step = exact_step(jobs)
        step["run"] = step["run"].replace("cargo test", "echo cargo test", 1)
        with self.subTest(mutation="echo-substring"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

        jobs, catalog, registry = clone()
        step = exact_step(jobs)
        step["run"] = (
            "# cargo test --locked -p sorotte-sim --test mpv_rebuffer_harness "
            f"{first_entry['test']} -- --ignored --exact --nocapture\n"
            "echo ignored test was documented"
        )
        with self.subTest(mutation="comment-substring"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

        jobs, catalog, registry = clone()
        exact_step(jobs)["if"] = "false"
        with self.subTest(mutation="disabled-step"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

        jobs, catalog, registry = clone()
        exact_step(jobs)["continue-on-error"] = "true"
        with self.subTest(mutation="tolerated-failure"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

        jobs, catalog, registry = clone()
        jobs[first_entry["required_job"]]["if"] = "false"
        with self.subTest(mutation="disabled-required-job"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

        jobs, catalog, registry = clone()
        jobs["verification_required"]["needs"].remove(first_entry["required_job"])
        with self.subTest(mutation="missing-aggregate-dependency"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

        jobs, catalog, registry = clone()
        catalog["policy"]["required_jobs"].remove(first_entry["required_job"])
        with self.subTest(mutation="not-catalog-required"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

        jobs, catalog, registry = clone()
        aggregate = named_step(
            jobs,
            "verification_required",
            "Aggregate required behavior evidence",
        )
        required_job = first_entry["required_job"]
        result_value = f"${{{{ needs['{required_job}'].result }}}}"
        result_key = next(
            key for key, value in aggregate["env"].items() if value == result_value
        )
        aggregate["env"].pop(result_key)
        with self.subTest(mutation="missing-aggregate-result"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

        jobs, catalog, registry = clone()
        aggregate = named_step(
            jobs,
            "verification_required",
            "Aggregate required behavior evidence",
        )
        result_value = f"${{{{ needs['{required_job}'].result }}}}"
        result_key = next(
            key for key, value in aggregate["env"].items() if value == result_value
        )
        aggregate["run"] = aggregate["run"].replace(
            f'--job-result "{required_job}=${result_key}" \\',
            "",
            1,
        )
        with self.subTest(mutation="result-env-not-passed"), self.assertRaises(
            AssertionError
        ):
            validate_pull_request_ignored_bindings(jobs, catalog, registry)

    def test_catalog_required_results_are_all_aggregated(self) -> None:
        required = self.catalog["policy"]["required_jobs"]
        self.assertEqual(
            required,
            [
                "checks",
                "lifecycle-contract",
                "gui-semantic",
                "rust-windows",
                "coverage-diff",
                "compat-live-tls",
                "media-match-generated-media",
                "mpv-pr-semantics",
            ],
        )
        self.assertEqual(
            self.jobs["verification_required"]["needs"],
            [
                "checks",
                "lifecycle_contract",
                "gui_semantic",
                "rust_windows",
                "coverage_diff",
                "compat-live-tls",
                "media-match-generated-media",
                "mpv-pr-semantics",
            ],
        )
        aggregate = self.assert_exact_run(
            self.jobs,
            "verification_required",
            "Aggregate required behavior evidence",
            """
            python scripts/behavior_evidence.py aggregate
            --catalog coverage/behaviors.toml
            --expected-sha "$VERIFICATION_SHA"
            --expected-repository "${{ github.repository }}"
            --expected-run-id "${{ github.run_id }}"
            --expected-run-attempt "${{ github.run_attempt }}"
            --job-result "checks=$CHECKS_RESULT"
            --job-result "lifecycle-contract=$LIFECYCLE_RESULT"
            --job-result "gui-semantic=$SEMANTIC_RESULT"
            --job-result "rust-windows=$WINDOWS_RESULT"
            --job-result "coverage-diff=$COVERAGE_RESULT"
            --job-result "compat-live-tls=$COMPAT_RESULT"
            --job-result "media-match-generated-media=$MEDIA_MATCH_RESULT"
            --job-result "mpv-pr-semantics=$MPV_RESULT"
            --input target/downloaded-evidence/lifecycle/evidence.lifecycle-contract.json
            --input target/downloaded-evidence/semantic/evidence.gui-semantic.json
            --output target/verification/evidence.aggregate.json
            """,
        )
        self.assertEqual(
            set(aggregate["env"]),
            {
                "CHECKS_RESULT",
                "LIFECYCLE_RESULT",
                "SEMANTIC_RESULT",
                "WINDOWS_RESULT",
                "COVERAGE_RESULT",
                "COMPAT_RESULT",
                "MEDIA_MATCH_RESULT",
                "MPV_RESULT",
            },
        )

    def sorotte_checkouts(self, job_id: str) -> list[dict[str, Any]]:
        return [
            step
            for step in self.jobs[job_id]["steps"]
            if step.get("uses") == PINNED_USES["actions/checkout"]
            and "repository" not in step.get("with", {})
        ]

    def test_general_pr_gates_use_merge_revision_and_evidence_uses_head(self) -> None:
        for job_id in (
            "checks",
            "compat-live-tls",
            "media-match-generated-media",
            "mpv-pr-semantics",
        ):
            checkouts = self.sorotte_checkouts(job_id)
            self.assertEqual(len(checkouts), 1)
            self.assertNotIn("ref", checkouts[0].get("with", {}))

        for job_id in ("rust_windows_tests", "rust_windows_release"):
            windows = self.sorotte_checkouts(job_id)
            self.assertEqual(len(windows), 1)
            self.assertNotIn("ref", windows[0].get("with", {}))

        windows_coverage = self.sorotte_checkouts("rust_windows_coverage")
        self.assertEqual(len(windows_coverage), 1)
        self.assertEqual(windows_coverage[0]["with"]["ref"], HEAD_REF)
        self.assertNotIn("path", windows_coverage[0]["with"])
        self.assertNotIn("clean", windows_coverage[0]["with"])
        self.assertEqual(self.sorotte_checkouts("rust_windows"), [])

        lifecycle = self.sorotte_checkouts("lifecycle_contract")
        coverage_producer = self.sorotte_checkouts("coverage_linux")
        coverage = self.sorotte_checkouts("coverage_diff")
        aggregate = self.sorotte_checkouts("verification_required")
        self.assertEqual(lifecycle[0]["with"]["ref"], HEAD_REF)
        self.assertEqual(coverage_producer[0]["with"]["ref"], HEAD_REF)
        self.assertEqual(coverage[0]["with"]["ref"], HEAD_REF)
        self.assertEqual(coverage[0]["with"]["fetch-depth"], "0")
        self.assertEqual(aggregate[0]["with"]["ref"], HEAD_REF)

        semantic = self.sorotte_checkouts("gui_semantic")
        self.assertEqual(len(semantic), 2)
        self.assertNotIn("ref", semantic[0].get("with", {}))
        self.assertEqual(semantic[1]["with"]["ref"], HEAD_REF)
        self.assertEqual(semantic[1]["with"]["path"], "evidence-source")
        self.assertNotIn("clean", semantic[1]["with"])
        evidence_step = named_step(
            self.jobs,
            "gui_semantic",
            "Run complete semantic inventory",
        )
        self.assertEqual(evidence_step["working-directory"], "evidence-source")
        prerequisites = named_step(
            self.jobs,
            "gui_semantic",
            "Reconcile exact evidence prerequisites",
        )
        self.assertEqual(prerequisites["working-directory"], "evidence-source")

    def test_evidence_reruns_replace_artifacts(self) -> None:
        expected_artifacts = {
            "lifecycle_contract": "verification-lifecycle-contract",
            "gui_semantic": "verification-gui-semantic",
            "rust_windows_coverage": "verification-windows-process-coverage",
            "coverage_linux": "verification-linux-merged-coverage",
            "coverage_diff": "verification-coverage-diff",
            "verification_required": "verification-aggregate",
        }
        for job_id, artifact_name in expected_artifacts.items():
            uploads = [
                step
                for step in self.jobs[job_id]["steps"]
                if step.get("uses") == PINNED_USES["actions/upload-artifact"]
            ]
            self.assertEqual(len(uploads), 1)
            self.assertEqual(uploads[0]["with"]["name"], artifact_name)
            self.assertEqual(uploads[0]["with"]["overwrite"], "true")
            self.assertEqual(uploads[0].get("if"), "always()")
            if job_id == "coverage_diff":
                self.assertEqual(
                    uploads[0]["with"]["path"].splitlines(),
                    [
                        "target/verification/coverage-ci-phases.json",
                        "target/verification/coverage-base.json",
                        "target/verification/coverage-profile-lanes.json",
                        "target/verification/coverage-profile-logs/",
                        "target/verification/coverage-line-map.json",
                        "target/verification/diff-coverage.json",
                        "target/windows-process-coverage/verification/coverage-windows-process-line-map.json",
                        "target/diff-coverage.json",
                        "target/diff-coverage.txt",
                    ],
                )
                self.assertEqual(
                    uploads[0]["with"]["if-no-files-found"],
                    "error",
                )

    def test_ci_authority_and_external_inputs_are_minimized(self) -> None:
        self.assertEqual(self.workflow["permissions"], {"contents": "read"})
        for job in self.jobs.values():
            for step in job.get("steps", []):
                if step.get("uses") == PINNED_USES["actions/checkout"]:
                    self.assertEqual(
                        step.get("with", {}).get("persist-credentials"),
                        "false",
                    )

        syncplay_checkouts = [
            step
            for job in self.jobs.values()
            for step in job.get("steps", [])
            if step.get("uses") == PINNED_USES["actions/checkout"]
            and step.get("with", {}).get("repository") == "Syncplay/syncplay"
        ]
        self.assertEqual(len(syncplay_checkouts), 6)
        self.assertTrue(
            all(
                checkout["with"]["ref"] == LEGACY_SYNCPLAY_SHA
                for checkout in syncplay_checkouts
            )
        )
        self.assertEqual(requirement_lines(CI_REQUIREMENTS), ["PyYAML==6.0.2"])
        self.assertEqual(
            requirement_lines(LEGACY_REQUIREMENTS),
            [
                "twisted==25.5.0",
                "pyopenssl==25.3.0",
                "service_identity==24.2.0",
            ],
        )

    def test_scheduled_coverage_is_locked_all_feature_and_reproducible(self) -> None:
        coverage_jobs = self.coverage_workflow["jobs"]
        self.assertEqual(self.coverage_workflow["permissions"], {"contents": "read"})
        self.assertEqual(
            coverage_jobs["coverage"].get("env"),
            {
                "SYNCPLAY_LEGACY_ROOT": (
                    "${{ github.workspace }}/.interop-cache/syncplay-legacy"
                )
            },
        )
        checkout = named_step(coverage_jobs, "coverage", "Checkout")
        self.assertEqual(
            checkout.get("uses"),
            PINNED_USES["actions/checkout"],
        )
        self.assertEqual(
            checkout.get("with"),
            {"persist-credentials": "false"},
        )
        legacy_checkout = named_step(
            coverage_jobs,
            "coverage",
            "Checkout pinned legacy reference for merged coverage",
        )
        self.assertEqual(
            legacy_checkout.get("uses"),
            PINNED_USES["actions/checkout"],
        )
        self.assertEqual(
            legacy_checkout.get("with"),
            {
                "repository": "Syncplay/syncplay",
                "ref": LEGACY_SYNCPLAY_SHA,
                "path": ".interop-cache/syncplay-legacy",
                "persist-credentials": "false",
            },
        )
        coverage_rust = named_step(coverage_jobs, "coverage", "Setup Rust")
        self.assertEqual(
            coverage_rust.get("uses"),
            PINNED_USES["dtolnay/rust-toolchain"],
        )
        self.assertEqual(
            coverage_rust.get("with"),
            {
                "toolchain": "1.97.1",
                "components": "rustfmt, clippy, llvm-tools-preview",
            },
        )
        windows_rust = named_step(
            coverage_jobs,
            "windows-process-coverage",
            "Setup Rust",
        )
        self.assertEqual(
            windows_rust.get("uses"),
            PINNED_USES["dtolnay/rust-toolchain"],
        )
        self.assertEqual(
            windows_rust.get("with"),
            {
                "toolchain": "1.97.1",
                "components": "rustfmt, clippy, llvm-tools-preview",
            },
        )
        python_setup = named_step(
            coverage_jobs,
            "coverage",
            "Setup Python",
        )
        self.assertEqual(
            python_setup.get("uses"),
            PINNED_USES["actions/setup-python"],
        )
        self.assertEqual(python_setup.get("with"), {"python-version": "3.11"})
        self.assert_exact_run(
            coverage_jobs,
            "coverage",
            "Install merged coverage prerequisites",
            "python -m pip install --disable-pip-version-check "
            "-r requirements/legacy-python-interop.txt",
        )
        installer = named_step(
            coverage_jobs,
            "coverage",
            "Install pinned cargo-llvm-cov",
        )
        self.assertEqual(
            installer.get("uses"),
            PINNED_USES["taiki-e/install-action"],
        )
        self.assertEqual(installer.get("with"), {"tool": "cargo-llvm-cov@0.8.4"})
        self.assert_exact_run(
            coverage_jobs,
            "coverage",
            "Generate merged behavioral coverage profiles",
            """
            python scripts/coverage_profile_lanes.py run
            --repo-root .
            --output target/verification/coverage-profile-lanes.json
            """,
        )
        self.assert_exact_run(
            coverage_jobs,
            "coverage",
            "Export pinned LLVM JSON",
            "cargo llvm-cov report --json --skip-functions "
            "--output-path target/coverage.json",
        )
        self.assert_exact_run(
            coverage_jobs,
            "coverage",
            "Export native LLVM source view",
            "cargo llvm-cov report --text --output-path target/coverage.txt",
        )
        self.assert_exact_run(
            coverage_jobs,
            "coverage",
            "Build source-bound physical line map",
            """
            python scripts/llvm_cov_line_map.py
            --repo-root .
            --llvm-json target/coverage.json
            --llvm-text target/coverage.txt
            --output target/coverage-line-map.json
            """,
        )
        upload = named_step(coverage_jobs, "coverage", "Upload coverage artifact")
        self.assertEqual(
            upload.get("uses"),
            PINNED_USES["actions/upload-artifact"],
        )
        self.assertEqual(
            upload.get("with"),
            {
                "name": "sorotte-llvm-coverage",
                "path": (
                    "target/coverage.json\n"
                    "target/coverage.txt\n"
                    "target/coverage-line-map.json\n"
                    "target/verification/coverage-profile-lanes.json\n"
                    "target/verification/coverage-profile-logs/\n"
                ),
                "if-no-files-found": "error",
                "retention-days": "14",
                "overwrite": "true",
            },
        )

    def test_scheduled_mutation_shard_is_pinned_bounded_and_fail_closed(self) -> None:
        jobs = self.mutation_workflow["jobs"]
        self.assertEqual(set(jobs), {"mutation", "participant-status-evidence-set"})
        self.assertEqual(self.mutation_workflow["permissions"], {"contents": "read"})
        self.assertEqual(
            self.mutation_workflow["on"],
            {
                "workflow_dispatch": "",
                "pull_request": {
                    "paths": [
                        ".github/workflows/rust-mutation.yml",
                        "coverage/mutation-policy.toml",
                        "coverage/mutation-report-set.json",
                        "scripts/mutation_ci.py",
                        "crates/sorotte-protocol/src/lib.rs",
                        "crates/sorotte-protocol/src/state.rs",
                        "crates/sorotte-protocol/src/participant_status.rs",
                        "crates/sorotte-player-api/src/lib.rs",
                        "crates/sorotte-player-mpv/src/adapter.rs",
                        "crates/sorotte-player-mpv/src/adapter/reconnection.rs",
                        "crates/sorotte-player-mpv/src/adapter/state.rs",
                        "crates/sorotte-player-mpv/src/adapter/player_adapter.rs",
                        "crates/sorotte-client-core/src/**",
                        "crates/sorotte-client-app/src/**",
                        "crates/sorotte-cli/src/**",
                        "crates/sorotte-server/src/**",
                        "crates/sorotte-gui/src/**",
                    ]
                },
                "schedule": [{"cron": "15 4 * * 0"}],
            },
        )
        self.assertEqual(
            self.mutation_workflow["concurrency"],
            {
                "group": "sorotte-mutation-${{ github.ref }}",
                "cancel-in-progress": "true",
            },
        )
        job = jobs["mutation"]
        self.assertEqual(job["name"], "Mutation (${{ matrix.shard }})")
        self.assertEqual(job["runs-on"], "ubuntu-latest")
        self.assertEqual(job["timeout-minutes"], "120")
        self.assertNotIn("if", job, "job-level if cannot access matrix context")
        pull_request_shards = [
            "participant-status-protocol",
            "client-participant-status",
            "client-participant-status-runtime",
            "client-participant-status-outbox",
            "server-participant-status",
            "gui-participant-status",
            "gui-playlist-delivery-fence",
            "player-mpv-explicit-ipc-retry",
            "client-app-participant-status-lifecycle",
            "cli-participant-status-lifecycle",
        ]
        all_shards = [
            "privacy-secret",
            "server-auth",
            "protocol-codec",
            *pull_request_shards,
            "client-reconnect-state",
            "client-runtime-config",
            "client-ping",
            "server-persistence-arbitration",
            "client-inbound-order",
            "client-playlist-shuffle",
            "cli-framing",
        ]
        compact_json = lambda values: '["' + '","'.join(values) + '"]'
        matrix_expression = (
            "${{ fromJSON(\n"
            "  github.event_name == 'pull_request'\n"
            f"  && '{compact_json(pull_request_shards)}'\n"
            f"  || '{compact_json(all_shards)}'\n"
            ") }}"
        )

        participant_status_boundaries = {
            "crates/sorotte-protocol/src/lib.rs",
            "crates/sorotte-protocol/src/state.rs",
            "crates/sorotte-protocol/src/participant_status.rs",
            "crates/sorotte-player-api/src/lib.rs",
            "crates/sorotte-player-mpv/src/adapter.rs",
            "crates/sorotte-player-mpv/src/adapter/reconnection.rs",
            "crates/sorotte-player-mpv/src/adapter/state.rs",
            "crates/sorotte-player-mpv/src/adapter/player_adapter.rs",
            "crates/sorotte-client-core/src/**",
            "crates/sorotte-client-app/src/**",
            "crates/sorotte-cli/src/**",
            "crates/sorotte-server/src/**",
            "crates/sorotte-gui/src/**",
        }
        self.assertLessEqual(
            participant_status_boundaries,
            set(self.mutation_workflow["on"]["pull_request"]["paths"]),
            "every participant-status production boundary must trigger its mutation shards",
        )
        self.assertEqual(
            job["strategy"],
            {
                "fail-fast": "false",
                "matrix": {"shard": matrix_expression},
            },
        )
        self.assertNotIn("continue-on-error", job)

        checkout = named_step(jobs, "mutation", "Checkout")
        self.assertEqual(checkout.get("uses"), PINNED_USES["actions/checkout"])
        self.assertEqual(
            checkout.get("with"),
            {"persist-credentials": "false"},
        )
        rust = named_step(jobs, "mutation", "Setup Rust")
        self.assertEqual(rust.get("uses"), PINNED_USES["dtolnay/rust-toolchain"])
        self.assertEqual(
            rust.get("with"),
            {
                "toolchain": "1.97.1",
                "components": "rustfmt, clippy",
            },
        )
        python = named_step(jobs, "mutation", "Setup Python")
        self.assertEqual(python.get("uses"), PINNED_USES["actions/setup-python"])
        self.assertEqual(python.get("with"), {"python-version": "3.11"})
        installer = named_step(jobs, "mutation", "Install pinned cargo-mutants")
        self.assertEqual(
            installer.get("uses"),
            PINNED_USES["taiki-e/install-action"],
        )
        self.assertEqual(
            installer.get("with"),
            {"tool": "cargo-mutants@27.1.0", "fallback": "none"},
        )
        self.assert_exact_run(
            jobs,
            "mutation",
            "Validate mutation policy",
            """
            python scripts/mutation_ci.py validate
            --repo-root .
            --policy coverage/mutation-policy.toml
            --shard ${{ matrix.shard }}
            """,
        )
        self.assert_exact_run(
            jobs,
            "mutation",
            "Run source-bound mutation shard",
            """
            python scripts/mutation_ci.py run
            --repo-root .
            --policy coverage/mutation-policy.toml
            --shard ${{ matrix.shard }}
            --results-root target/mutation-ci/${{ matrix.shard }}
            --output target/verification/mutation-${{ matrix.shard }}.json
            """,
        )
        self.assert_exact_run(
            jobs,
            "mutation",
            "Verify mutation evidence matches current source",
            """
            python scripts/mutation_ci.py verify-report
            --repo-root .
            --policy coverage/mutation-policy.toml
            --shard ${{ matrix.shard }}
            --report target/verification/mutation-${{ matrix.shard }}.json
            """,
        )
        upload = named_step(jobs, "mutation", "Upload mutation evidence")
        self.assertEqual(upload.get("if"), "always()")
        self.assertEqual(
            upload.get("uses"),
            PINNED_USES["actions/upload-artifact"],
        )
        self.assertEqual(
            upload.get("with"),
            {
                "name": "sorotte-mutation-${{ matrix.shard }}",
                "path": (
                    "target/verification/mutation-${{ matrix.shard }}.json\n"
                    "target/mutation-ci/${{ matrix.shard }}\n"
                ),
                "if-no-files-found": "error",
                "retention-days": "14",
                "overwrite": "true",
            },
        )

        evidence_job = jobs["participant-status-evidence-set"]
        self.assertEqual(evidence_job["name"], "Participant-status mutation evidence set")
        self.assertEqual(evidence_job["if"], "${{ always() }}")
        self.assertEqual(evidence_job["needs"], "mutation")
        self.assertEqual(evidence_job["runs-on"], "ubuntu-latest")
        self.assertEqual(evidence_job["timeout-minutes"], "30")
        evidence_checkout = named_step(
            jobs,
            "participant-status-evidence-set",
            "Checkout",
        )
        self.assertEqual(evidence_checkout.get("uses"), PINNED_USES["actions/checkout"])
        download = named_step(
            jobs,
            "participant-status-evidence-set",
            "Download mutation reports",
        )
        self.assertEqual(
            download.get("uses"),
            PINNED_USES["actions/download-artifact"],
        )
        self.assertEqual(
            download.get("with"),
            {
                "pattern": "sorotte-mutation-*",
                "path": "target",
                "merge-multiple": "true",
            },
        )
        self.assert_exact_run(
            jobs,
            "participant-status-evidence-set",
            "Verify the uniquely selected participant-status report set",
            """
            python scripts/mutation_ci.py verify-report-set
            --repo-root .
            --policy coverage/mutation-policy.toml
            --manifest coverage/mutation-report-set.json
            """,
        )

        self.assertEqual(
            self.mutation_policy,
            {
                "schema_version": 3,
                "cargo_mutants_version": "27.1.0",
                "required_report_set": [
                    {
                        "id": "participant-status",
                        "shards": [
                            "participant-status-protocol",
                            "client-participant-status",
                            "client-participant-status-runtime",
                            "client-participant-status-outbox",
                            "server-participant-status",
                            "gui-participant-status",
                            "gui-playlist-delivery-fence",
                            "player-mpv-explicit-ipc-retry",
                            "client-app-participant-status-lifecycle",
                            "cli-participant-status-lifecycle",
                        ],
                    }
                ],
                "shard": [
                    {
                        "id": "privacy-secret",
                        "owner": "secrets",
                        "package": "sorotte-secret",
                        "files": ["crates/sorotte-secret/src/lib.rs"],
                        "mutant_filter": "",
                        "test_target": "package",
                        "test_filter": "",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "server-auth",
                        "owner": "server-security",
                        "package": "sorotte-server",
                        "files": ["crates/sorotte-server/src/auth.rs"],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": "auth::tests::",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "protocol-codec",
                        "owner": "protocol-safety",
                        "package": "sorotte-protocol",
                        "files": [
                            "crates/sorotte-protocol/src/codec.rs",
                            "crates/sorotte-protocol/src/redacted_debug.rs",
                        ],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": "",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "participant-status-protocol",
                        "owner": "participant-status",
                        "package": "sorotte-protocol",
                        "files": [
                            "crates/sorotte-protocol/src/participant_status.rs"
                        ],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": "",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-participant-status",
                        "owner": "participant-status",
                        "package": "sorotte-client-core",
                        "files": [
                            "crates/sorotte-client-core/src/inbound.rs",
                            "crates/sorotte-client-core/src/session/apply.rs",
                            "crates/sorotte-client-core/src/session/helpers.rs",
                            "crates/sorotte-client-core/src/session/"
                            "participant_status.rs",
                            "crates/sorotte-client-core/src/views.rs",
                        ],
                        "mutant_filter": (
                            "(participant_status|ParticipantStatus|"
                            "ClientParticipantStatus)"
                        ),
                        "test_target": "lib",
                        "test_filter": (
                            "session::tests::participant_status_tests::"
                        ),
                        "jobs": 2,
                        "timeout_seconds": 90,
                        "build_timeout_seconds": 180,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-participant-status-runtime",
                        "owner": "participant-status",
                        "package": "sorotte-client-core",
                        "files": [
                            "crates/sorotte-client-core/src/runtime/"
                            "accessors.rs",
                            "crates/sorotte-client-core/src/runtime/"
                            "playback_coordination.rs",
                            "crates/sorotte-client-core/src/runtime/"
                            "queued_control.rs",
                        ],
                        "mutant_filter": (
                            "(participant_status|ParticipantStatus|"
                            "record_observation_outcomes|"
                            "commit_mapped_transport_observation|"
                            "observe_transport_with_semantics|"
                            "reset_sync_state_for_reconnect|"
                            "delete field logical_pause from struct "
                            "PlayerTransportDelta expression in "
                            "ClientRuntime<P, C>::apply_ordered_event)"
                        ),
                        "test_target": "lib",
                        "test_filter": (
                            "runtime::playback_coordination::tests::"
                            "participant_status_"
                        ),
                        "jobs": 2,
                        "timeout_seconds": 120,
                        "build_timeout_seconds": 240,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-participant-status-outbox",
                        "owner": "participant-status",
                        "package": "sorotte-client-core",
                        "files": ["crates/sorotte-client-core/src/outbox.rs"],
                        "mutant_filter": (
                            "(cancel_pending_participant_status_reports|"
                            "strip_participant_status_at|"
                            "push_connection_scoped_state)"
                        ),
                        "test_target": "lib",
                        "test_filter": "outbox::tests::participant_status_",
                        "jobs": 2,
                        "timeout_seconds": 90,
                        "build_timeout_seconds": 180,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "server-participant-status",
                        "owner": "participant-status",
                        "package": "sorotte-server",
                        "files": [
                            "crates/sorotte-server/src/inbound.rs",
                            "crates/sorotte-server/src/runtime_handlers.rs",
                            "crates/sorotte-server/src/runtime_maintenance.rs",
                            "crates/sorotte-server/src/runtime_playback_barrier.rs",
                        ],
                        "mutant_filter": (
                            "(participant_status|ParticipantStatus|"
                            "collect_due_periodic_updates_at|"
                            "delete field (set_by|"
                            "transport_revision|"
                            "client_latency_calculation|"
                            "client_ignoring_counter|server_rtt_seconds|"
                            "latency_calculation_seconds) from struct "
                            "StateSyncOptions expression in ServerRuntime::"
                            "periodic_state_sync_message_for_client_at)"
                        ),
                        "test_target": "lib",
                        "test_filter": "tests::participant_status_tests::",
                        "jobs": 2,
                        "timeout_seconds": 120,
                        "build_timeout_seconds": 240,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "gui-participant-status",
                        "owner": "participant-status",
                        "package": "sorotte-gui",
                        "files": [
                            "crates/sorotte-gui/src/app/widget_views/"
                            "main_window/summary.rs"
                        ],
                        "mutant_filter": (
                            "(participant_status|"
                            "member_report_evidence_is_unavailable|"
                            "member_player_availability_label|"
                            "member_logical_pause_label|"
                            "member_playback_rate_label|"
                            "member_generation_label|"
                            "member_revision_label)"
                        ),
                        "test_target": "lib",
                        "test_filter": (
                            "app::widget_views::tests::"
                            "main_window_controls::"
                        ),
                        "jobs": 2,
                        "timeout_seconds": 120,
                        "build_timeout_seconds": 240,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "gui-playlist-delivery-fence",
                        "owner": "gui-transport",
                        "package": "sorotte-gui",
                        "files": [
                            "crates/sorotte-gui/src/app/runtime_owner.rs",
                            "crates/sorotte-gui/src/app/runtime_owner/"
                            "player/media_open.rs",
                            "crates/sorotte-gui/src/app/runtime_owner/"
                            "player/media_search.rs",
                            "crates/sorotte-gui/src/app/runtime_owner/"
                            "player/playlist_sync.rs",
                            "crates/sorotte-gui/src/app/runtime_owner/"
                            "requests/session_controls.rs",
                            "crates/sorotte-gui/src/app/runtime_owner/"
                            "session_transport.rs",
                            "crates/sorotte-gui/src/app/runtime_owner/"
                            "startup_player.rs",
                            "crates/sorotte-gui/src/app/runtime_stack/"
                            "playlist_delivery_fence.rs",
                            "crates/sorotte-gui/src/app/runtime_stack/"
                            "client_core_adapter/delivery_fence.rs",
                            "crates/sorotte-gui/src/app/runtime_stack/"
                            "client_core_adapter/runtime_adapter_impl.rs",
                        ],
                        "mutant_filter": (
                            "(Gui(PendingSharedPlaylistOpen|"
                            "PlaylistProtocolDeliveryFence)|"
                            "playlist.*delivery_fence|"
                            "clear_session_causal_player_effect_state|"
                            "finish_shared_playlist_open_after_delivery|"
                            "resume_pending_shared_playlist_open_if_ready|"
                            "delete field (username|room).*"
                            "StoredClientSettingsMvp|"
                            "delete field direct_target.*"
                            "GuiLocalMediaSearchAliases)"
                        ),
                        "test_target": "lib",
                        "test_filter": (
                            "app::runtime_owner::tests::playlist_runtime_"
                            "tests::open_insert_and_local_media::"
                        ),
                        "jobs": 2,
                        "timeout_seconds": 180,
                        "build_timeout_seconds": 300,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "player-mpv-explicit-ipc-retry",
                        "owner": "participant-status",
                        "package": "sorotte-player-mpv",
                        "files": [
                            "crates/sorotte-player-mpv/src/adapter/reconnection.rs",
                        ],
                        "mutant_filter": (
                            "(disconnected_with_json_ipc_retry|"
                            "maintain_json_ipc_reconnection)"
                        ),
                        "test_target": "lib",
                        "test_filter": (
                            "adapter::version_policy_tests::"
                            "explicit_json_ipc_retry_"
                        ),
                        "jobs": 2,
                        "timeout_seconds": 120,
                        "build_timeout_seconds": 240,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-app-participant-status-lifecycle",
                        "owner": "participant-status",
                        "package": "sorotte-client-app",
                        "files": [
                            "crates/sorotte-client-app/src/application.rs",
                            "crates/sorotte-client-app/src/"
                            "participant_status_presentation.rs",
                        ],
                        "mutant_filter": (
                            "(synchronize_player_availability|"
                            "record_contained_external_player_failure|"
                            "run_participant_status_heartbeat|"
                            "ParticipantStatusReportPresentation::("
                            "from_client_view|position_evidence_is_eligible|"
                            "buffer_evidence_is_eligible|headline_label)|"
                            "delete field (policy|quorum_percent|"
                            "maximum_pause_seconds) from struct "
                            "PlaybackBarrierRoomBufferingConfig expression in "
                            "ClientApplication<P>::apply_settings)"
                        ),
                        "test_target": "lib",
                        "test_filter": "",
                        "jobs": 2,
                        "timeout_seconds": 120,
                        "build_timeout_seconds": 240,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "cli-participant-status-lifecycle",
                        "owner": "participant-status",
                        "package": "sorotte-cli",
                        "files": [
                            "crates/sorotte-cli/src/session_runner/"
                            "connected_session/execution.rs"
                        ],
                        "mutant_filter": (
                            "(synchronize_connected_session_player_availability|"
                            "contain_connected_session_player_failure|"
                            "planned_local_runtime_action_is_player_bound|"
                            "contain_planned_local_runtime_action_result|"
                            "run_contained_planned_local_runtime_action|"
                            "run_connected_session_branch_runtime_steps_"
                            "legacy_compatible)"
                        ),
                        "test_target": "lib",
                        "test_filter": (
                            "session_runner::connected_session::execution::tests::"
                        ),
                        "jobs": 2,
                        "timeout_seconds": 120,
                        "build_timeout_seconds": 240,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-reconnect-state",
                        "owner": "client-lifecycle",
                        "package": "sorotte-client-core",
                        "files": [
                            "crates/sorotte-client-core/src/session/reconnect.rs"
                        ],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": "session::tests::",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-runtime-config",
                        "owner": "client-configuration",
                        "package": "sorotte-client-app",
                        "files": [
                            "crates/sorotte-client-app/src/"
                            "legacy_runtime_config.rs"
                        ],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": "legacy_runtime_config::tests::",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-ping",
                        "owner": "client-networking",
                        "package": "sorotte-client-core",
                        "files": [
                            "crates/sorotte-client-core/src/ping.rs"
                        ],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": "session::tests::ping_tests::",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "server-persistence-arbitration",
                        "owner": "server-persistence",
                        "package": "sorotte-server",
                        "files": [
                            "crates/sorotte-server/src/persistence_actor/"
                            "persistence_arbitration.rs"
                        ],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": (
                            "persistence_actor::persistence_arbitration_tests::"
                        ),
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-inbound-order",
                        "owner": "client-protocol",
                        "package": "sorotte-client-core",
                        "files": [
                            "crates/sorotte-client-core/src/inbound_order.rs"
                        ],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": "session::tests::protocol_tests::",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "client-playlist-shuffle",
                        "owner": "client-playlist",
                        "package": "sorotte-client-core",
                        "files": [
                            "crates/sorotte-client-core/src/session/playlist/"
                            "shuffle_helpers.rs"
                        ],
                        "mutant_filter": "",
                        "test_target": "lib",
                        "test_filter": (
                            "session::tests::playlist_tests::"
                            "shuffle_undo_tests::"
                        ),
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                    {
                        "id": "cli-framing",
                        "owner": "cli-transport",
                        "package": "sorotte-cli",
                        "files": [
                            "crates/sorotte-cli/src/protocol_io.rs"
                        ],
                        "mutant_filter": "",
                        "test_target": "package",
                        "test_filter": "",
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    },
                ],
                "accepted_unviable": [
                    {
                        "id": "const-default-is-not-const",
                        "shard": "privacy-secret",
                        "file": "crates/sorotte-secret/src/lib.rs",
                        "function": "RedactedCommandArgs::from_count",
                        "return_type": "-> Self",
                        "genre": "FnValue",
                        "replacement": "Default::default()",
                        "reason": (
                            "cargo-mutants inserts a non-const Default::default "
                            "call into a const fn, so Rust rejects the mutant "
                            "before tests can run"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "protocol-error-source-dyn-default",
                        "shard": "protocol-codec",
                        "file": "crates/sorotte-protocol/src/codec.rs",
                        "function": (
                            "<impl std::error::Error for "
                            "ProtocolError>::source"
                        ),
                        "return_type": (
                            "-> Option<&(dyn std::error::Error +'static)>"
                        ),
                        "genre": "FnValue",
                        "replacement": (
                            "Some(Box::leak(Box::new(Default::default())))"
                        ),
                        "reason": (
                            "cargo-mutants cannot construct Default::default "
                            "for the dynamic Error trait object required by "
                            "this source return type"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "protocol-error-from-default",
                        "shard": "protocol-codec",
                        "file": "crates/sorotte-protocol/src/codec.rs",
                        "function": (
                            "<impl From<serde_json::Error> for "
                            "ProtocolError>::from"
                        ),
                        "return_type": "-> Self",
                        "genre": "FnValue",
                        "replacement": "Default::default()",
                        "reason": (
                            "ProtocolError intentionally has no Default "
                            "implementation, so the generated replacement "
                            "cannot type-check"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "protocol-command-order-default",
                        "shard": "protocol-codec",
                        "file": "crates/sorotte-protocol/src/codec.rs",
                        "function": (
                            "decode_protocol_message_with_command_order"
                        ),
                        "return_type": (
                            "-> Result<ProtocolMessage, ProtocolError>"
                        ),
                        "genre": "FnValue",
                        "replacement": "Ok(Default::default())",
                        "reason": (
                            "ProtocolMessage has no semantically valid "
                            "Default variant, so the generated successful "
                            "replacement cannot type-check"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "protocol-message-lines-default-element",
                        "shard": "protocol-codec",
                        "file": "crates/sorotte-protocol/src/codec.rs",
                        "function": "decode_message_lines",
                        "return_type": (
                            "-> Result<Vec<ProtocolMessage>, ProtocolError>"
                        ),
                        "genre": "FnValue",
                        "replacement": "Ok(vec![Default::default()])",
                        "reason": (
                            "ProtocolMessage has no semantically valid "
                            "Default variant, so the generated vector element "
                            "cannot type-check"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "protocol-line-items-default-element",
                        "shard": "protocol-codec",
                        "file": "crates/sorotte-protocol/src/codec.rs",
                        "function": "decode_message_line_items",
                        "return_type": (
                            "-> Result<Vec<DecodedMessageLineItem>, "
                            "ProtocolError>"
                        ),
                        "genre": "FnValue",
                        "replacement": "Ok(vec![Default::default()])",
                        "reason": (
                            "DecodedMessageLineItem has no meaningful Default "
                            "value, so the generated vector element cannot "
                            "type-check"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "protocol-message-line-default",
                        "shard": "protocol-codec",
                        "file": "crates/sorotte-protocol/src/codec.rs",
                        "function": "decode_message_line",
                        "return_type": (
                            "-> Result<ProtocolMessage, ProtocolError>"
                        ),
                        "genre": "FnValue",
                        "replacement": "Ok(Default::default())",
                        "reason": (
                            "ProtocolMessage has no semantically valid "
                            "Default variant, so the generated successful "
                            "replacement cannot type-check"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "protocol-hello-value-default",
                        "shard": "protocol-codec",
                        "file": "crates/sorotte-protocol/src/codec.rs",
                        "function": "extract_hello",
                        "return_type": (
                            "-> Result<HelloPayload, ProtocolError>"
                        ),
                        "genre": "FnValue",
                        "replacement": "Ok(Default::default())",
                        "reason": (
                            "HelloPayload requires identity fields and has no "
                            "Default implementation, so the generated "
                            "successful replacement cannot type-check"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "protocol-hello-message-default",
                        "shard": "protocol-codec",
                        "file": "crates/sorotte-protocol/src/codec.rs",
                        "function": "extract_hello_from_message",
                        "return_type": (
                            "-> Result<HelloPayload, ProtocolError>"
                        ),
                        "genre": "FnValue",
                        "replacement": "Ok(Default::default())",
                        "reason": (
                            "HelloPayload requires identity fields and has no "
                            "Default implementation, so the generated "
                            "successful replacement cannot type-check"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "client-reconnect-ping-let-chain-or",
                        "shard": "client-reconnect-state",
                        "file": (
                            "crates/sorotte-client-core/src/session/reconnect.rs"
                        ),
                        "function": (
                            "ClientSession::reconcile_ping_only_state_response"
                        ),
                        "return_type": "-> StatePayload",
                        "genre": "BinaryOperator",
                        "replacement": "||",
                        "reason": (
                            "cargo-mutants replaces the Rust let-chain && "
                            "with ||, but let expressions are only valid in "
                            "&& chains so the generated mutant cannot parse"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "client-reconnect-state-let-chain-or",
                        "shard": "client-reconnect-state",
                        "file": (
                            "crates/sorotte-client-core/src/session/reconnect.rs"
                        ),
                        "function": (
                            "ClientSession::reconcile_normalized_state_and_"
                            "build_response_with_local_state_change_override"
                        ),
                        "return_type": "-> StatePayload",
                        "genre": "BinaryOperator",
                        "replacement": "||",
                        "reason": (
                            "cargo-mutants replaces the Rust let-chain && "
                            "with ||, but let expressions are only valid in "
                            "&& chains so the generated mutant cannot parse"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "client-runtime-config-host-let-chain-or",
                        "shard": "client-runtime-config",
                        "file": (
                            "crates/sorotte-client-app/src/"
                            "legacy_runtime_config.rs"
                        ),
                        "function": (
                            "parse_host_and_optional_port_from_host_arg_"
                            "legacy_compatible"
                        ),
                        "return_type": "-> (String, Option<u16>)",
                        "genre": "BinaryOperator",
                        "replacement": "||",
                        "reason": (
                            "cargo-mutants replaces the Rust let-chain && "
                            "with ||, but let expressions are only valid in "
                            "&& chains so both generated sites fail to parse"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "client-runtime-config-room-let-chain-or",
                        "shard": "client-runtime-config",
                        "file": (
                            "crates/sorotte-client-app/src/"
                            "legacy_runtime_config.rs"
                        ),
                        "function": (
                            "normalize_controlled_room_input_legacy_compatible"
                        ),
                        "return_type": "-> (String, Option<String>)",
                        "genre": "BinaryOperator",
                        "replacement": "||",
                        "reason": (
                            "cargo-mutants replaces the Rust let-chain && "
                            "with ||, but let expressions are only valid in "
                            "&& chains so both generated sites fail to parse"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "client-runtime-config-fallback-let-chain-or",
                        "shard": "client-runtime-config",
                        "file": (
                            "crates/sorotte-client-app/src/"
                            "legacy_runtime_config.rs"
                        ),
                        "function": (
                            "stored_client_settings_runtime_snapshot_"
                            "legacy_compatible"
                        ),
                        "return_type": (
                            "-> StoredClientSettingsRuntimeSnapshot"
                        ),
                        "genre": "BinaryOperator",
                        "replacement": "||",
                        "reason": (
                            "cargo-mutants replaces the Rust let-chain && "
                            "with ||, but let expressions are only valid in "
                            "&& chains so the generated fallback mutant "
                            "cannot parse"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": (
                            "server-persistence-arbitration-enqueue-default"
                        ),
                        "shard": "server-persistence-arbitration",
                        "file": (
                            "crates/sorotte-server/src/persistence_actor/"
                            "persistence_arbitration.rs"
                        ),
                        "function": "RoomPersistenceArbitration::enqueue",
                        "return_type": "-> RoomEffectEnqueueDisposition",
                        "genre": "FnValue",
                        "replacement": "Default::default()",
                        "reason": (
                            "cargo-mutants requests Default for a semantic "
                            "three-way disposition enum that intentionally "
                            "has no safe default"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": (
                            "server-persistence-arbitration-effect-default"
                        ),
                        "shard": "server-persistence-arbitration",
                        "file": (
                            "crates/sorotte-server/src/persistence_actor/"
                            "persistence_arbitration.rs"
                        ),
                        "function": (
                            "RoomPersistenceArbitration::desired_effects"
                        ),
                        "return_type": "-> Vec<ServerPersistenceEffect>",
                        "genre": "FnValue",
                        "replacement": "vec![Default::default()]",
                        "reason": (
                            "cargo-mutants requests Default for a persistence "
                            "effect whose required room or snapshot identity "
                            "has no valid default"
                        ),
                        "review_by": "2026-10-31",
                    },
                    {
                        "id": "client-playlist-shuffle-let-chain-or",
                        "shard": "client-playlist-shuffle",
                        "file": (
                            "crates/sorotte-client-core/src/session/playlist/"
                            "shuffle_helpers.rs"
                        ),
                        "function": (
                            "ClientSession::local_playlist_target_index_from_"
                            "changed_playlist_legacy_compatible"
                        ),
                        "return_type": "-> usize",
                        "genre": "BinaryOperator",
                        "replacement": "||",
                        "reason": (
                            "cargo-mutants replaces each of the forward- and "
                            "backward-search Rust let-chain && sites with ||, "
                            "but let expressions are only valid in && chains "
                            "so both generated mutants cannot parse"
                        ),
                        "review_by": "2026-10-31",
                    },
                    *[
                        {
                            "id": identifier,
                            "shard": "participant-status-protocol",
                            "file": (
                                "crates/sorotte-protocol/src/"
                                "participant_status.rs"
                            ),
                            "function": function,
                            "return_type": "-> Self",
                            "genre": "FnValue",
                            "replacement": "Default::default()",
                            "reason": (
                                "The extensible protocol builder return type "
                                "intentionally has no Default because a "
                                "default would invent required status "
                                "identity, so the generated replacement "
                                "cannot type-check"
                            ),
                            "review_by": "2026-11-30",
                        }
                        for identifier, function in [
                            (
                                "participant-status-scope-state-revision-default",
                                "ParticipantPlaybackScope::with_state_revision",
                            ),
                            (
                                "participant-status-scope-transport-revision-default",
                                "ParticipantPlaybackScope::with_transport_revision",
                            ),
                            (
                                "participant-status-report-playback-scope-default",
                                "ParticipantStatusReport::with_playback_scope",
                            ),
                            (
                                "participant-status-report-timeline-default",
                                "ParticipantStatusReport::with_timeline_kind",
                            ),
                            (
                                "participant-status-report-position-default",
                                "ParticipantStatusReport::with_position_seconds",
                            ),
                            (
                                "participant-status-report-logical-pause-default",
                                "ParticipantStatusReport::with_logical_paused",
                            ),
                            (
                                "participant-status-report-playback-rate-default",
                                "ParticipantStatusReport::with_playback_rate",
                            ),
                            (
                                "participant-status-report-cache-pause-default",
                                "ParticipantStatusReport::with_paused_for_cache",
                            ),
                            (
                                "participant-status-report-cache-percent-default",
                                "ParticipantStatusReport::with_cache_percent",
                            ),
                            (
                                "participant-status-report-buffered-ahead-default",
                                "ParticipantStatusReport::with_buffered_ahead_seconds",
                            ),
                            (
                                "participant-status-report-sample-age-default",
                                "ParticipantStatusReport::with_sample_age_ms",
                            ),
                            (
                                "participant-status-report-position-age-default",
                                "ParticipantStatusReport::with_position_sample_age_ms",
                            ),
                            (
                                "participant-status-snapshot-mode-default",
                                "ParticipantStatusSnapshot::with_mode",
                            ),
                        ]
                    ],
                    {
                        "id": "participant-status-directional-report-default",
                        "shard": "participant-status-protocol",
                        "file": (
                            "crates/sorotte-protocol/src/"
                            "participant_status.rs"
                        ),
                        "function": "decode_report",
                        "return_type": (
                            "-> serde_json::Result<Option<"
                            "ParticipantStatusReport>>"
                        ),
                        "genre": "FnValue",
                        "replacement": "Ok(Some(Default::default()))",
                        "reason": (
                            "The directional decoder's generated success "
                            "value requires Default for "
                            "ParticipantStatusReport, but a report requires "
                            "explicit sequence and lifecycle evidence and "
                            "intentionally has no valid Default"
                        ),
                        "review_by": "2026-11-30",
                    },
                    {
                        "id": (
                            "client-participant-status-runtime-"
                            "player-availability-default"
                        ),
                        "shard": "client-participant-status-runtime",
                        "file": (
                            "crates/sorotte-client-core/src/runtime/"
                            "playback_coordination.rs"
                        ),
                        "function": (
                            "RuntimePlaybackCoordination::"
                            "participant_status_player_availability"
                        ),
                        "return_type": "-> ParticipantPlayerConnection",
                        "genre": "FnValue",
                        "replacement": "Default::default()",
                        "reason": (
                            "cargo-mutants requests Default for the explicit "
                            "player-availability enum, which intentionally "
                            "has no semantically safe default, so the "
                            "generated replacement cannot type-check"
                        ),
                        "review_by": "2026-11-30",
                    },
                    {
                        "id": (
                            "client-participant-status-runtime-"
                            "pending-report-default"
                        ),
                        "shard": "client-participant-status-runtime",
                        "file": (
                            "crates/sorotte-client-core/src/runtime/"
                            "playback_coordination.rs"
                        ),
                        "function": (
                            "RuntimePlaybackCoordination::"
                            "pending_participant_status_report"
                        ),
                        "return_type": (
                            "-> Option<PendingParticipantStatusReport>"
                        ),
                        "genre": "FnValue",
                        "replacement": "Some(Default::default())",
                        "reason": (
                            "cargo-mutants requests Default for a pending "
                            "report whose report, fingerprint, and send "
                            "timestamp are all required, so the generated "
                            "replacement cannot type-check"
                        ),
                        "review_by": "2026-11-30",
                    },
                    {
                        "id": (
                            "client-participant-status-outbox-"
                            "cancel-let-chain-or"
                        ),
                        "shard": "client-participant-status-outbox",
                        "file": "crates/sorotte-client-core/src/outbox.rs",
                        "function": (
                            "ProtocolOutbox::"
                            "cancel_pending_participant_status_reports"
                        ),
                        "return_type": "",
                        "genre": "BinaryOperator",
                        "replacement": "||",
                        "reason": (
                            "cargo-mutants changes the && connector before "
                            "a Rust let-chain to ||, which rustc rejects "
                            "because let-chain conditions support only &&"
                        ),
                        "review_by": "2026-11-30",
                    },
                    *[
                        {
                            "id": identifier,
                            "shard": shard,
                            "file": file,
                            "function": function,
                            "return_type": return_type,
                            "genre": genre,
                            "replacement": replacement,
                            **(
                                {
                                    "expected_count": {
                                        "client-participant-status-user-view-let-chain-or": 3,
                                        "client-participant-status-apply-update-let-chain-or": 6,
                                        "server-participant-status-snapshot-let-chain-or": 5,
                                        "player-mpv-explicit-ipc-retry-instant-duration-multiply": 2,
                                    }[identifier]
                                }
                                if identifier
                                in {
                                    "client-participant-status-user-view-let-chain-or",
                                    "client-participant-status-apply-update-let-chain-or",
                                    "server-participant-status-snapshot-let-chain-or",
                                    "player-mpv-explicit-ipc-retry-instant-duration-multiply",
                                }
                                else {}
                            ),
                            "reason": reason,
                            "review_by": "2026-11-30",
                        }
                        for (
                            identifier,
                            shard,
                            file,
                            function,
                            return_type,
                            genre,
                            replacement,
                            reason,
                        ) in [
                            (
                                "client-participant-status-view-from-wire-default",
                                "client-participant-status",
                                "crates/sorotte-client-core/src/views.rs",
                                "ClientParticipantStatusView::from_wire",
                                "-> Self",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for a "
                                    "client status view that requires an "
                                    "explicit server projection and derived "
                                    "freshness state, so the generated "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                "client-participant-status-view-aged-by-default",
                                "client-participant-status",
                                "crates/sorotte-client-core/src/views.rs",
                                "ClientParticipantStatusView::aged_by",
                                "-> Self",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for a "
                                    "client status view that requires an "
                                    "explicit server projection and derived "
                                    "freshness state, so the generated "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "client-participant-status-view-"
                                    "fail-closed-default"
                                ),
                                "client-participant-status",
                                "crates/sorotte-client-core/src/views.rs",
                                "ClientParticipantStatusView::fail_closed_stale",
                                "-> Self",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for a "
                                    "client status view that requires an "
                                    "explicit server projection and derived "
                                    "freshness state, so the generated "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                "client-participant-status-user-view-default",
                                "client-participant-status",
                                (
                                    "crates/sorotte-client-core/src/session/"
                                    "participant_status.rs"
                                ),
                                "ClientSession::user_participant_status_at",
                                "-> Option<ClientParticipantStatusView>",
                                "FnValue",
                                "Some(Default::default())",
                                (
                                    "cargo-mutants requests Default for a "
                                    "client status view inside Some, but the "
                                    "view requires an explicit server "
                                    "projection and derived freshness state, "
                                    "so the replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "client-participant-status-user-view-"
                                    "let-chain-or"
                                ),
                                "client-participant-status",
                                (
                                    "crates/sorotte-client-core/src/session/"
                                    "participant_status.rs"
                                ),
                                "ClientSession::user_participant_status_at",
                                "-> Option<ClientParticipantStatusView>",
                                "BinaryOperator",
                                "||",
                                (
                                    "cargo-mutants changes each of three "
                                    "observed && connectors in a Rust "
                                    "let-chain to ||, which rustc rejects "
                                    "because let-chain conditions support "
                                    "only &&"
                                ),
                            ),
                            (
                                (
                                    "client-participant-status-"
                                    "authoritative-scope-default"
                                ),
                                "client-participant-status",
                                (
                                    "crates/sorotte-client-core/src/session/"
                                    "participant_status.rs"
                                ),
                                (
                                    "ClientSession::"
                                    "participant_status_authoritative_scope"
                                ),
                                "-> Option<ParticipantPlaybackScope>",
                                "FnValue",
                                "Some(Default::default())",
                                (
                                    "cargo-mutants requests Default for a "
                                    "playback scope inside Some, but the scope "
                                    "requires an explicit media generation "
                                    "and intentionally has no Default, so the "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "client-participant-status-apply-update-"
                                    "let-chain-or"
                                ),
                                "client-participant-status",
                                (
                                    "crates/sorotte-client-core/src/session/"
                                    "participant_status.rs"
                                ),
                                (
                                    "ClientSession::"
                                    "apply_participant_status_update"
                                ),
                                "",
                                "BinaryOperator",
                                "||",
                                (
                                    "cargo-mutants changes each of six "
                                    "observed && connectors in Rust let-chain "
                                    "conditions to ||, which rustc rejects "
                                    "because let-chain conditions support "
                                    "only &&"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-"
                                    "normalize-report-default"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_handlers.rs"
                                ),
                                "normalize_participant_status_report",
                                "-> Option<ParticipantStatusReport>",
                                "FnValue",
                                "Some(Default::default())",
                                (
                                    "cargo-mutants requests Default for a "
                                    "participant report inside Some, but a "
                                    "report requires an explicit sequence, "
                                    "player connection, and playback phase, "
                                    "so the replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-"
                                    "availability-default"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                "participant_status_availability",
                                "-> ParticipantStatusAvailability",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for the "
                                    "explicit freshness and capability "
                                    "availability enum, which intentionally "
                                    "has no semantically safe default, so the "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-"
                                    "correlation-default"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                "participant_status_correlation",
                                "-> ParticipantStatusCorrelation",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for the "
                                    "three-way correlation enum, "
                                    "which intentionally has no semantically "
                                    "safe default, so the generated "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-"
                                    "compact-snapshot-default"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                "compact_participant_status_snapshot",
                                "-> ParticipantStatusSnapshot",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for a "
                                    "status snapshot that requires an "
                                    "explicit revision, projection mode, and "
                                    "participant map, so the generated "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-"
                                    "unavailable-snapshot-default"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                "unavailable_participant_status_snapshot",
                                "-> ParticipantStatusSnapshot",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for a "
                                    "status snapshot that requires an "
                                    "explicit revision, projection mode, and "
                                    "participant map, so the generated "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-"
                                    "split-message-default"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                (
                                    "split_participant_status_from_"
                                    "reliable_passthrough"
                                ),
                                "-> Vec<ProtocolMessage>",
                                "FnValue",
                                "vec![Default::default()]",
                                (
                                    "cargo-mutants requests Default for a "
                                    "protocol message inside the split "
                                    "delivery vector, but every message "
                                    "requires an explicit protocol variant "
                                    "and ProtocolMessage intentionally has "
                                    "no Default, so the generated "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-periodic-"
                                    "updates-element-default"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                (
                                    "ServerRuntime::"
                                    "collect_due_periodic_updates_at"
                                ),
                                (
                                    "-> Result<Vec<DirectedProtocolMessage>, "
                                    "ServerRuntimeError>"
                                ),
                                "FnValue",
                                "Ok(vec![Default::default()])",
                                (
                                    "cargo-mutants requests Default for a "
                                    "directed protocol message inside Ok, "
                                    "but every message requires an explicit "
                                    "authenticated recipient and protocol "
                                    "payload, so the generated replacement "
                                    "cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-barrier-"
                                    "scope-message-default"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_playback_barrier.rs"
                                ),
                                (
                                    "ServerRuntime::replace_room_barrier_"
                                    "participant_status_scope"
                                ),
                                "-> Vec<DirectedProtocolMessage>",
                                "FnValue",
                                "vec![Default::default()]",
                                (
                                    "cargo-mutants requests Default for a "
                                    "directed protocol message in the "
                                    "barrier-scope replacement vector, but "
                                    "every message requires an explicit "
                                    "authenticated recipient and protocol "
                                    "payload, so the generated replacement "
                                    "cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-clear-client-"
                                    "let-chain-or"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                (
                                    "ServerRuntime::"
                                    "clear_participant_status_for_client"
                                ),
                                "",
                                "BinaryOperator",
                                "||",
                                (
                                    "cargo-mutants changes the observed && "
                                    "connector in a Rust let-chain to ||, "
                                    "which rustc rejects because let-chain "
                                    "conditions support only &&"
                                ),
                            ),
                            (
                                "server-participant-status-scope-default",
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                (
                                    "ServerRuntime::"
                                    "participant_status_scope_for_room"
                                ),
                                "-> ParticipantPlaybackScope",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for a "
                                    "playback scope that requires an explicit "
                                    "media generation and intentionally has "
                                    "no Default, so the generated replacement "
                                    "cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "server-participant-status-snapshot-"
                                    "let-chain-or"
                                ),
                                "server-participant-status",
                                (
                                    "crates/sorotte-server/src/"
                                    "runtime_maintenance.rs"
                                ),
                                (
                                    "ServerRuntime::"
                                    "participant_status_snapshot_for_client_at"
                                ),
                                (
                                    "-> Option<"
                                    "ParticipantStatusStateExtension>"
                                ),
                                "BinaryOperator",
                                "||",
                                (
                                    "cargo-mutants changes five observed && "
                                    "connectors in Rust let-chain conditions "
                                    "to ||, which rustc rejects because "
                                    "let-chain conditions support only &&"
                                ),
                            ),
                            (
                                (
                                    "client-app-participant-status-"
                                    "presentation-default"
                                ),
                                "client-app-participant-status-lifecycle",
                                (
                                    "crates/sorotte-client-app/src/"
                                    "participant_status_presentation.rs"
                                ),
                                (
                                    "ParticipantStatusReportPresentation::"
                                    "from_client_view"
                                ),
                                "-> Self",
                                "FnValue",
                                "Default::default()",
                                (
                                    "The generated replacement requires "
                                    "Default for a presentation derived from "
                                    "an explicit participant-status view and "
                                    "freshness state, so it cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "player-mpv-explicit-ipc-retry-"
                                    "instant-duration-multiply"
                                ),
                                "player-mpv-explicit-ipc-retry",
                                (
                                    "crates/sorotte-player-mpv/src/adapter/"
                                    "reconnection.rs"
                                ),
                                (
                                    "MpvAdapter::"
                                    "maintain_json_ipc_reconnection_using_clock"
                                ),
                                "",
                                "BinaryOperator",
                                "*",
                                (
                                    "cargo-mutants replaces Instant plus "
                                    "Duration with multiplication at both "
                                    "retry deadlines, but Rust deliberately "
                                    "defines no Instant multiplication operator"
                                ),
                            ),
                            (
                                "cli-contained-player-failure-default",
                                "cli-participant-status-lifecycle",
                                (
                                    "crates/sorotte-cli/src/session_runner/"
                                    "connected_session/execution.rs"
                                ),
                                "contain_connected_session_player_failure",
                                "-> ContainedConnectedSessionPlayerFailure",
                                "FnValue",
                                "Default::default()",
                                (
                                    "the contained failure intentionally "
                                    "retains an anyhow error and has no valid "
                                    "Default, so the generated replacement "
                                    "cannot type-check"
                                ),
                            ),
                            (
                                "cli-run-contained-player-action-true-default",
                                "cli-participant-status-lifecycle",
                                (
                                    "crates/sorotte-cli/src/session_runner/"
                                    "connected_session/execution.rs"
                                ),
                                "run_contained_planned_local_runtime_action",
                                (
                                    "-> anyhow::Result<(bool, Option<"
                                    "ContainedConnectedSessionPlayerFailure>)>"
                                ),
                                "FnValue",
                                "Ok((true, Some(Default::default())))",
                                (
                                    "the generated Some payload requires "
                                    "Default for the contained failure type, "
                                    "which intentionally has no valid Default"
                                ),
                            ),
                            (
                                "cli-run-contained-player-action-false-default",
                                "cli-participant-status-lifecycle",
                                (
                                    "crates/sorotte-cli/src/session_runner/"
                                    "connected_session/execution.rs"
                                ),
                                "run_contained_planned_local_runtime_action",
                                (
                                    "-> anyhow::Result<(bool, Option<"
                                    "ContainedConnectedSessionPlayerFailure>)>"
                                ),
                                "FnValue",
                                "Ok((false, Some(Default::default())))",
                                (
                                    "the generated Some payload requires "
                                    "Default for the contained failure type, "
                                    "which intentionally has no valid Default"
                                ),
                            ),
                            (
                                "cli-contain-player-action-result-true-default",
                                "cli-participant-status-lifecycle",
                                (
                                    "crates/sorotte-cli/src/session_runner/"
                                    "connected_session/execution.rs"
                                ),
                                "contain_planned_local_runtime_action_result",
                                (
                                    "-> anyhow::Result<(bool, Option<"
                                    "ContainedConnectedSessionPlayerFailure>)>"
                                ),
                                "FnValue",
                                "Ok((true, Some(Default::default())))",
                                (
                                    "the generated Some payload requires "
                                    "Default for the contained failure type, "
                                    "which intentionally has no valid Default"
                                ),
                            ),
                            (
                                "cli-contain-player-action-result-false-default",
                                "cli-participant-status-lifecycle",
                                (
                                    "crates/sorotte-cli/src/session_runner/"
                                    "connected_session/execution.rs"
                                ),
                                "contain_planned_local_runtime_action_result",
                                (
                                    "-> anyhow::Result<(bool, Option<"
                                    "ContainedConnectedSessionPlayerFailure>)>"
                                ),
                                "FnValue",
                                "Ok((false, Some(Default::default())))",
                                (
                                    "the generated Some payload requires "
                                    "Default for the contained failure type, "
                                    "which intentionally has no valid Default"
                                ),
                            ),
                            (
                                "cli-branch-runtime-player-failure-default",
                                "cli-participant-status-lifecycle",
                                (
                                    "crates/sorotte-cli/src/session_runner/"
                                    "connected_session/execution.rs"
                                ),
                                (
                                    "run_connected_session_branch_runtime_"
                                    "steps_legacy_compatible"
                                ),
                                (
                                    "-> Option<"
                                    "ContainedConnectedSessionPlayerFailure>"
                                ),
                                "FnValue",
                                "Some(Default::default())",
                                (
                                    "the generated Some payload requires "
                                    "Default for the contained failure type, "
                                    "which intentionally has no valid Default"
                                ),
                            ),
                            (
                                "client-participant-status-runtime-phase-default",
                                "client-participant-status-runtime",
                                (
                                    "crates/sorotte-client-core/src/runtime/"
                                    "playback_coordination.rs"
                                ),
                                (
                                    "RuntimePlaybackCoordination::"
                                    "participant_status_phase"
                                ),
                                "-> ParticipantPlaybackPhase",
                                "FnValue",
                                "Default::default()",
                                (
                                    "cargo-mutants requests Default for the explicit "
                                    "participant playback phase enum, which intentionally "
                                    "has no semantically safe default, so the generated "
                                    "replacement cannot type-check"
                                ),
                            ),
                            (
                                (
                                    "client-participant-status-runtime-"
                                    "observe-actions-default"
                                ),
                                "client-participant-status-runtime",
                                (
                                    "crates/sorotte-client-core/src/runtime/"
                                    "playback_coordination.rs"
                                ),
                                (
                                    "RuntimePlaybackCoordination::"
                                    "observe_transport_with_semantics"
                                ),
                                "-> Vec<PlaybackCoordinatorAction>",
                                "FnValue",
                                "vec![Default::default()]",
                                (
                                    "cargo-mutants requests Default for a playback "
                                    "coordinator action, but every action requires an "
                                    "explicit causal revision, command, or failure reason "
                                    "and intentionally has no Default"
                                ),
                            ),
                            (
                                (
                                    "client-participant-status-runtime-"
                                    "flush-let-chain-or"
                                ),
                                "client-participant-status-runtime",
                                (
                                    "crates/sorotte-client-core/src/runtime/"
                                    "accessors.rs"
                                ),
                                (
                                    "ClientSessionUpdate<'a>::"
                                    "flush_participant_status_transition"
                                ),
                                "",
                                "BinaryOperator",
                                "||",
                                (
                                    "cargo-mutants changes the && connector before a Rust "
                                    "let-chain to ||, which rustc rejects because let-chain "
                                    "conditions support only &&"
                                ),
                            ),
                            (
                                (
                                    "client-participant-status-runtime-"
                                    "queued-state-let-chain-or"
                                ),
                                "client-participant-status-runtime",
                                (
                                    "crates/sorotte-client-core/src/runtime/"
                                    "queued_control.rs"
                                ),
                                (
                                    "ClientRuntime<P, QueuedRuntimeControl>::"
                                    "queue_connection_scoped_state_with_participant_status"
                                ),
                                "-> bool",
                                "BinaryOperator",
                                "||",
                                (
                                    "cargo-mutants changes the && connector before a Rust "
                                    "let-chain to ||, which rustc rejects because let-chain "
                                    "conditions support only &&"
                                ),
                            ),
                        ]
                    ],
                ],
            },
        )

    def test_runtime_status_mutant_filter_binds_context_collateral(self) -> None:
        shard = next(
            shard
            for shard in self.mutation_policy["shard"]
            if shard["id"] == "client-participant-status-runtime"
        )
        self.assertIn(
            "crates/sorotte-client-core/src/runtime/accessors.rs",
            shard["files"],
        )
        mutant_filter = shard["mutant_filter"]
        self.assertRegex(
            "ClientSessionUpdate<'a>::reset_sync_state_for_reconnect",
            mutant_filter,
        )
        context = (
            " from struct PlayerTransportDelta expression in "
            "ClientRuntime<P, C>::apply_ordered_event"
        )

        self.assertRegex(
            f"delete field logical_pause{context}",
            mutant_filter,
        )

        for neighbor in (
            f"delete field load_attempt_id{context}",
            f"delete field media_generation{context}",
            f"delete field phase{context}",
            f"delete field playback_rate{context}",
            (
                "delete field load_attempt_id from struct PlayerSnapshotDelta "
                "expression in ClientRuntime<P, C>::apply_ordered_event"
            ),
            (
                "delete field load_attempt_id from struct PlayerTransportDelta "
                "expression in ClientRuntime<P, C>::apply_player_event"
            ),
        ):
            with self.subTest(neighbor=neighbor):
                self.assertIsNone(re.search(mutant_filter, neighbor))

    def test_outbox_status_mutant_filter_is_narrow_and_behavior_owned(self) -> None:
        shard = next(
            shard
            for shard in self.mutation_policy["shard"]
            if shard["id"] == "client-participant-status-outbox"
        )
        self.assertEqual(
            shard["files"],
            ["crates/sorotte-client-core/src/outbox.rs"],
        )
        self.assertEqual(
            shard["test_filter"],
            "outbox::tests::participant_status_",
        )
        mutant_filter = shard["mutant_filter"]
        for function in (
            "cancel_pending_participant_status_reports",
            "strip_participant_status_at",
            "push_connection_scoped_state",
        ):
            with self.subTest(function=function):
                self.assertRegex(function, mutant_filter)
        for neighbor in (
            "push_back",
            "push_readiness_intent",
            "release_front",
            "acknowledge_front",
        ):
            with self.subTest(neighbor=neighbor):
                self.assertIsNone(re.search(mutant_filter, neighbor))

    def test_gui_playlist_fence_filter_binds_only_observed_context_collateral(self) -> None:
        shard = next(
            shard
            for shard in self.mutation_policy["shard"]
            if shard["id"] == "gui-playlist-delivery-fence"
        )
        mutant_filter = shard["mutant_filter"]
        for owned in (
            "GuiPendingSharedPlaylistOpen::replace_delivery_fence",
            "GuiPlaylistProtocolDeliveryFence::note_frame_written",
            "queue_playlist_entry_with_delivery_fence",
            "clear_session_causal_player_effect_state",
            "delete field username from struct StoredClientSettingsMvp",
            "delete field room from struct StoredClientSettingsMvp",
            "delete field direct_target from struct Self expression in GuiLocalMediaSearchAliases::for_target",
        ):
            with self.subTest(owned=owned):
                self.assertRegex(owned, mutant_filter)

        for neighbor in (
            "delete field player_path from struct StoredClientSettingsMvp",
            "delete field username from struct StoredClientSettingsRuntimeSnapshot",
            "delete field fallback_title from struct Self expression in GuiLocalMediaSearchAliases::for_target",
            "clear_media_match_remote_lookup_state",
        ):
            with self.subTest(neighbor=neighbor):
                self.assertIsNone(re.search(mutant_filter, neighbor))

    def test_server_status_mutant_filter_binds_context_collateral(self) -> None:
        shard = next(
            shard
            for shard in self.mutation_policy["shard"]
            if shard["id"] == "server-participant-status"
        )
        mutant_filter = shard["mutant_filter"]
        context = (
            " from struct StateSyncOptions expression in ServerRuntime::"
            "periodic_state_sync_message_for_client_at"
        )

        for field in (
            "set_by",
            "transport_revision",
            "client_latency_calculation",
            "client_ignoring_counter",
            "server_rtt_seconds",
            "latency_calculation_seconds",
        ):
            with self.subTest(field=field):
                self.assertRegex(f"delete field {field}{context}", mutant_filter)

        for neighbor in (
            f"delete field playback_revision{context}",
            (
                "delete field set_by from struct OtherStateSyncOptions "
                "expression in ServerRuntime::"
                "periodic_state_sync_message_for_client_at"
            ),
            (
                "delete field set_by from struct StateSyncOptions expression "
                "in ServerRuntime::periodic_state_sync_message"
            ),
        ):
            with self.subTest(neighbor=neighbor):
                self.assertIsNone(re.search(mutant_filter, neighbor))

    def test_client_app_status_mutant_filter_binds_context_collateral(self) -> None:
        shard = next(
            shard
            for shard in self.mutation_policy["shard"]
            if shard["id"] == "client-app-participant-status-lifecycle"
        )
        self.assertIn(
            "crates/sorotte-client-app/src/participant_status_presentation.rs",
            shard["files"],
        )
        mutant_filter = shard["mutant_filter"]
        for function in (
            "ParticipantStatusReportPresentation::from_client_view",
            "ParticipantStatusReportPresentation::position_evidence_is_eligible",
            "ParticipantStatusReportPresentation::buffer_evidence_is_eligible",
            "ParticipantStatusReportPresentation::headline_label",
        ):
            with self.subTest(function=function):
                self.assertRegex(function, mutant_filter)
        context = (
            " from struct PlaybackBarrierRoomBufferingConfig expression in "
            "ClientApplication<P>::apply_settings"
        )

        for field in ("policy", "quorum_percent", "maximum_pause_seconds"):
            with self.subTest(field=field):
                self.assertRegex(f"delete field {field}{context}", mutant_filter)

        for neighbor in (
            f"delete field grace_seconds{context}",
            (
                "delete field policy from struct OtherRoomBufferingConfig "
                "expression in ClientApplication<P>::apply_settings"
            ),
            (
                "delete field policy from struct PlaybackBarrierRoomBufferingConfig "
                "expression in ClientApplication<P>::apply_other_settings"
            ),
        ):
            with self.subTest(neighbor=neighbor):
                self.assertIsNone(re.search(mutant_filter, neighbor))

    def test_feature_pushes_are_not_duplicated_with_pull_request_runs(self) -> None:
        triggers = self.workflow["on"]
        self.assertEqual(triggers["push"]["branches"], ["main"])
        self.assertEqual(triggers["push"]["tags"], ["v*", "server-v*"])
        self.assertIn("pull_request", triggers)
        self.assertEqual(
            triggers["workflow_dispatch"],
            {
                "inputs": {
                    "coverage_base_sha": {
                        "description": (
                            "Full commit SHA used as the explicit changed-line "
                            "coverage base"
                        ),
                        "required": "true",
                        "type": "string",
                    }
                }
            },
        )
        self.assertNotIn('"${VERIFICATION_SHA}^"', self.workflow_text)
        self.assertNotIn("candidate=", self.workflow_text)
        self.assertEqual(
            self.workflow["concurrency"]["cancel-in-progress"],
            "${{ github.event_name != 'schedule' && "
            "github.event_name != 'workflow_dispatch' }}",
        )

    def test_nextest_gate_cannot_be_disabled_or_tolerated(self) -> None:
        command = "python scripts/nextest_ci.py run --repo-root ."
        replaced = self.workflow_text.replace(
            f"        run: {command}",
            f"        run: echo no-op\n        # {command}",
            1,
        )
        mutated_jobs = parse_workflow(replaced)["jobs"]
        with self.assertRaises(AssertionError):
            self.assert_exact_run(
                mutated_jobs,
                "checks",
                "Nextest fail-on-flaky workspace tests",
                command,
                continue_on_error="true",
            )

        disabled = self.workflow_text.replace(
            "      - name: Nextest fail-on-flaky workspace tests\n"
            "        id: nextest",
            "      - name: Nextest fail-on-flaky workspace tests\n"
            "        if: false\n"
            "        id: nextest",
            1,
        )
        disabled_jobs = parse_workflow(disabled)["jobs"]
        with self.assertRaises(AssertionError):
            self.assert_exact_run(
                disabled_jobs,
                "checks",
                "Nextest fail-on-flaky workspace tests",
                command,
                continue_on_error="true",
            )

        weakened_enforcement = self.workflow_text.replace(
            '          test "$NEXTEST_OUTCOME" = success',
            '          test "$NEXTEST_OUTCOME" != cancelled',
            1,
        )
        weakened_jobs = parse_workflow(weakened_enforcement)["jobs"]
        with self.assertRaises(AssertionError):
            self.assert_exact_run(
                weakened_jobs,
                "checks",
                "Enforce complete Linux test gate",
                """
                test "$NEXTEST_OUTCOME" = success
                test "$DOCTEST_OUTCOME" = success
                """,
                allowed_if="always()",
            )


if __name__ == "__main__":
    unittest.main()
