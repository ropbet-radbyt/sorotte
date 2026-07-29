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
MPV_SUPPORTED_SHA = "2c219aa822df18a1b7fd9abe3e151cd93ad67307"
HEAD_REF = "${{ env.VERIFICATION_SHA }}"
ACTION_PINS = {
    "actions/checkout": (
        "11d5960a326750d5838078e36cf38b85af677262",
        "v4.4.0",
    ),
    "dtolnay/rust-toolchain": (
        "4cda84d5c5c54efe2404f9d843567869ab1699d4",
        "stable resolved 2026-07-28",
    ),
    "actions/setup-python": (
        "a26af69be951a213d495a4c3e4e4022e16d87065",
        "v5.6.0",
    ),
    "actions/setup-go": (
        "924ae3a1cded613372ab5595356fb5720e22ba16",
        "v6.5.0",
    ),
    "actions/upload-artifact": (
        "ea165f8d65b6e75b540449e92b4886f43607fa02",
        "v4.6.2",
    ),
    "actions/download-artifact": (
        "d3f86a106a0bac45b974a628896c90dbdf5c8093",
        "v4.3.0",
    ),
    "taiki-e/install-action": (
        "41049aa56687c35e0afa74eed4f09cec4f9afabf",
        "v2.85.2",
    ),
}
PINNED_USES = {
    action: f"{action}@{sha}" for action, (sha, _comment) in ACTION_PINS.items()
}
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
            "rust_windows",
            "coverage_diff",
            "compat-live-tls",
            "mpv-pr-semantics",
            "verification_required",
        }
        self.assertTrue(expected_jobs <= set(self.jobs))

        self.assert_exact_run(
            self.jobs,
            "checks",
            "Install CI policy prerequisites",
            "python -m pip install --disable-pip-version-check "
            "-r requirements/ci-policy.txt",
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

        coverage_job = self.jobs["coverage_diff"]
        self.assertEqual(
            coverage_job.get("env"),
            {
                "SYNCPLAY_LEGACY_ROOT": (
                    "${{ github.workspace }}/.interop-cache/syncplay-legacy"
                )
            },
        )
        coverage_legacy = named_step(
            self.jobs,
            "coverage_diff",
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
            "coverage_diff",
            "Install merged coverage prerequisites",
            "python -m pip install --disable-pip-version-check "
            "-r requirements/legacy-python-interop.txt",
        )
        coverage_installer = named_step(
            self.jobs,
            "coverage_diff",
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
        profiles = self.assert_exact_run(
            self.jobs,
            "coverage_diff",
            "Generate merged behavioral coverage profiles",
            """
            python scripts/coverage_profile_lanes.py run
            --repo-root .
            --output target/verification/coverage-profile-lanes.json
            """,
            continue_on_error="true",
            allowed_if="steps.coverage_base.outcome == 'success'",
        )
        self.assertEqual(profiles.get("id"), "coverage_profiles")
        llvm_json = self.assert_exact_run(
            self.jobs,
            "coverage_diff",
            "Export pinned LLVM JSON",
            "cargo llvm-cov report --json --skip-functions "
            "--output-path target/diff-coverage.json",
            continue_on_error="true",
            allowed_if="steps.coverage_profiles.outcome == 'success'",
        )
        self.assertEqual(llvm_json.get("id"), "llvm_json")
        llvm_text = self.assert_exact_run(
            self.jobs,
            "coverage_diff",
            "Export native LLVM source view",
            "cargo llvm-cov report --text "
            "--output-path target/diff-coverage.txt",
            continue_on_error="true",
            allowed_if="steps.coverage_profiles.outcome == 'success'",
        )
        self.assertEqual(llvm_text.get("id"), "llvm_text")
        line_map = self.assert_exact_run(
            self.jobs,
            "coverage_diff",
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
        policy = self.assert_exact_run(
            self.jobs,
            "coverage_diff",
            "Enforce production changed-line coverage",
            """
            python scripts/diff_coverage.py
            --repo-root .
            --coverage-map target/verification/coverage-line-map.json
            --critical-policy coverage/diff-coverage-policy.toml
            --base "$COVERAGE_BASE_SHA"
            --head "$VERIFICATION_SHA"
            --minimum 80
            --json-out target/verification/diff-coverage.json
            """,
            continue_on_error="true",
            allowed_if=(
                "steps.coverage_base.outcome == 'success' && "
                "steps.line_map.outcome == 'success'"
            ),
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
                "PROFILES_OUTCOME": "${{ steps.coverage_profiles.outcome }}",
                "LLVM_JSON_OUTCOME": "${{ steps.llvm_json.outcome }}",
                "LLVM_TEXT_OUTCOME": "${{ steps.llvm_text.outcome }}",
                "LINE_MAP_OUTCOME": "${{ steps.line_map.outcome }}",
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
            "rust_windows",
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
            "rust_windows",
            "Nextest fail-on-flaky workspace tests",
            "python scripts/nextest_ci.py run --repo-root .",
            continue_on_error="true",
        )
        self.assertEqual(windows_nextest.get("id"), "nextest")
        windows_doctests = self.assert_exact_run(
            self.jobs,
            "rust_windows",
            "Cargo doctests",
            "cargo test --locked --workspace --all-features --doc",
            continue_on_error="true",
        )
        self.assertEqual(windows_doctests.get("id"), "doctests")
        windows_attempts = named_step(
            self.jobs,
            "rust_windows",
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
        self.assert_exact_run(
            self.jobs,
            "rust_windows",
            "Locked release-profile GUI and updater build",
            "cargo build --locked --release -p sorotte-gui "
            "--bin sorotte-gui --bin sorotte-gui-updater",
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "rust_windows",
            "Package path boundary regressions",
            "./scripts/package-path-boundary-tests.ps1",
            continue_on_error="true",
        )
        self.assert_exact_run(
            self.jobs,
            "rust_windows",
            "Release publication policy regressions",
            "./scripts/release-publication-policy-tests.ps1",
            continue_on_error="true",
        )
        windows_enforcement = self.assert_exact_run(
            self.jobs,
            "rust_windows",
            "Enforce complete Windows behavior gate",
            """
            if ($env:NEXTEST_OUTCOME -ne "success") { throw "nextest failed or found a flaky test" }
            if ($env:DOCTEST_OUTCOME -ne "success") { throw "doctests failed" }
            if ($env:RELEASE_BUILD_OUTCOME -ne "success") { throw "release build failed" }
            if ($env:PACKAGE_PATHS_OUTCOME -ne "success") { throw "package path tests failed" }
            if ($env:RELEASE_POLICY_OUTCOME -ne "success") { throw "release policy tests failed" }
            """,
            allowed_if="always()",
        )
        self.assertEqual(
            windows_enforcement.get("env"),
            {
                "NEXTEST_OUTCOME": "${{ steps.nextest.outcome }}",
                "DOCTEST_OUTCOME": "${{ steps.doctests.outcome }}",
                "RELEASE_BUILD_OUTCOME": "${{ steps.release_build.outcome }}",
                "PACKAGE_PATHS_OUTCOME": "${{ steps.package_paths.outcome }}",
                "RELEASE_POLICY_OUTCOME": "${{ steps.release_policy.outcome }}",
            },
        )

        compatibility = self.assert_exact_run(
            self.jobs,
            "compat-live-tls",
            "Strict live legacy TLS parity",
            "cargo test --locked -p sorotte-compat --all-features "
            "legacy_server_live_tls_ -- --nocapture",
        )
        self.assertEqual(
            compatibility.get("env"),
            {
                "SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY": "1",
                "SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY": "1",
            },
        )

        mpv_checkout = named_step(
            self.jobs,
            "mpv-pr-semantics",
            "Checkout minimum supported official mpv",
        )
        self.assertEqual(
            mpv_checkout.get("uses"),
            PINNED_USES["actions/checkout"],
        )
        self.assertEqual(
            mpv_checkout.get("with"),
            {
                "repository": "mpv-player/mpv",
                "ref": MPV_SUPPORTED_SHA,
                "path": "target/mpv-supported",
                "persist-credentials": "false",
            },
        )
        verify_mpv_source = self.assert_exact_run(
            self.jobs,
            "mpv-pr-semantics",
            "Verify supported mpv source revision",
            f'test "$(git rev-parse HEAD^{{commit}})" = "{MPV_SUPPORTED_SHA}"',
        )
        self.assertEqual(
            verify_mpv_source.get("working-directory"),
            "target/mpv-supported",
        )

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
        aggregate["run"] = aggregate["run"].replace(
            f'--job-result "{required_job}=$MPV_RESULT" \\',
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
        for job_id in ("checks", "rust_windows", "compat-live-tls", "mpv-pr-semantics"):
            checkouts = self.sorotte_checkouts(job_id)
            self.assertEqual(len(checkouts), 1)
            self.assertNotIn("ref", checkouts[0].get("with", {}))

        lifecycle = self.sorotte_checkouts("lifecycle_contract")
        coverage = self.sorotte_checkouts("coverage_diff")
        aggregate = self.sorotte_checkouts("verification_required")
        self.assertEqual(lifecycle[0]["with"]["ref"], HEAD_REF)
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
        self.assertEqual(len(syncplay_checkouts), 5)
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
        self.assertEqual(set(jobs), {"mutation"})
        self.assertEqual(self.mutation_workflow["permissions"], {"contents": "read"})
        self.assertEqual(
            self.mutation_workflow["on"],
            {
                "workflow_dispatch": "",
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
        self.assertEqual(job["runs-on"], "ubuntu-latest")
        self.assertEqual(job["timeout-minutes"], "120")
        self.assertNotIn("continue-on-error", job)

        checkout = named_step(jobs, "mutation", "Checkout")
        self.assertEqual(checkout.get("uses"), PINNED_USES["actions/checkout"])
        self.assertEqual(
            checkout.get("with"),
            {"persist-credentials": "false"},
        )
        rust = named_step(jobs, "mutation", "Setup Rust")
        self.assertEqual(rust.get("uses"), PINNED_USES["dtolnay/rust-toolchain"])
        self.assertEqual(rust.get("with"), {"toolchain": "1.97.1"})
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
            --shard privacy-secret
            """,
        )
        self.assert_exact_run(
            jobs,
            "mutation",
            "Run source-bound privacy mutation shard",
            """
            python scripts/mutation_ci.py run
            --repo-root .
            --policy coverage/mutation-policy.toml
            --shard privacy-secret
            --results-root target/mutation-ci/privacy-secret
            --output target/verification/mutation-privacy-secret.json
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
                "name": "sorotte-mutation-privacy-secret",
                "path": (
                    "target/verification/mutation-privacy-secret.json\n"
                    "target/mutation-ci/privacy-secret\n"
                ),
                "if-no-files-found": "error",
                "retention-days": "14",
                "overwrite": "true",
            },
        )

        self.assertEqual(
            self.mutation_policy,
            {
                "schema_version": 1,
                "cargo_mutants_version": "27.1.0",
                "shard": [
                    {
                        "id": "privacy-secret",
                        "owner": "secrets",
                        "package": "sorotte-secret",
                        "files": ["crates/sorotte-secret/src/lib.rs"],
                        "jobs": 2,
                        "timeout_seconds": 60,
                        "build_timeout_seconds": 120,
                        "minimum_viable_kill_percent": "100.00",
                        "max_missed": 0,
                        "max_timeouts": 0,
                        "require_baseline": True,
                    }
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
                    }
                ],
            },
        )

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
