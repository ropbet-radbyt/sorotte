from __future__ import annotations

import copy
import ast
import pathlib
import re
import unittest
from typing import Any, Callable

import yaml

from scripts import gui_native_smoke_contract


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW_PATH = (
    REPO_ROOT / ".github" / "workflows" / "gui-native-interactive.yml"
)
ACTIONLINT_CONFIG_PATH = REPO_ROOT / ".github" / "actionlint.yaml"

CHECKOUT_ACTION = (
    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
)
RUST_ACTION = (
    "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4"
)
PYTHON_ACTION = (
    "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97"
)
UPLOAD_ACTION = (
    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"
)

RUNNER_LABELS = [
    "self-hosted",
    "Windows",
    "X64",
    "sorotte-native-interactive",
    "sorotte-ephemeral",
]
ATTESTATION_ENV_KEYS = {
    "SOROTTE_NATIVE_RUNNER_CONTRACT",
    "SOROTTE_NATIVE_RUNNER_INSTANCE_ID",
    "SOROTTE_NATIVE_RUNNER_MAX_JOBS",
    "SOROTTE_NATIVE_RUNNER_INPUTS_SHA256",
}
EVIDENCE_ROOT = (
    "${{ runner.temp }}\\sorotte-native-evidence-"
    "${{ github.run_id }}-${{ github.run_attempt }}"
)
NATIVE_ARTIFACT_ROOT = (
    "${{ github.workspace }}\\target\\verification\\gui-native-smoke"
)
CARGO_TARGET_ROOT = (
    "${{ runner.temp }}\\sorotte-native-"
    "${{ github.run_id }}-${{ github.run_attempt }}\\target"
)
REQUESTED_SHA = "${{ inputs.source_sha || github.sha }}"

STEP_NAMES = [
    "Attest ephemeral interactive runner before checkout",
    "Checkout exact trusted source",
    "Bind checkout to requested source",
    "Setup pinned Rust",
    "Setup pinned Python",
    "Install pinned native interop prerequisites",
    "Run exact strict native GUI inventory",
    "Validate and bind native evidence inventory",
    "Write native lane outcome evidence",
    "Upload all native interactive evidence",
    "Enforce complete native interactive lane",
]
REQUIRED_NATIVE_FILES = [
    "native-report.json",
    "native-stderr.log",
    "contract-summary.json",
    "invocation.json",
    "build-stdout.log",
    "build-stderr.log",
    "harness-build-stdout.log",
    "harness-build-stderr.log",
]
SUMMARY_OUTCOMES = {
    "REQUESTED_SOURCE_SHA": REQUESTED_SHA,
    "PREFLIGHT_OUTCOME": "${{ steps.preflight.outcome }}",
    "CHECKOUT_OUTCOME": "${{ steps.checkout.outcome }}",
    "RUST_OUTCOME": "${{ steps.rust.outcome }}",
    "PYTHON_OUTCOME": "${{ steps.python.outcome }}",
    "PREREQUISITES_OUTCOME": "${{ steps.prerequisites.outcome }}",
    "SOURCE_BINDING_OUTCOME": "${{ steps.source_binding.outcome }}",
    "NATIVE_OUTCOME": "${{ steps.native.outcome }}",
    "NATIVE_INVENTORY_OUTCOME": "${{ steps.native_inventory.outcome }}",
}
ENFORCED_OUTCOMES = {
    key: value
    for key, value in SUMMARY_OUTCOMES.items()
    if key != "REQUESTED_SOURCE_SHA"
}
ENFORCED_OUTCOMES.update(
    {
        "LANE_SUMMARY_OUTCOME": "${{ steps.lane_summary.outcome }}",
        "EVIDENCE_UPLOAD_OUTCOME": "${{ steps.evidence_upload.outcome }}",
        "SAFE_EXPORT_OUTCOME": "${{ steps.safe_export.outcome }}",
        "DISPLAY_OUTCOME": "${{ steps.display.outcome }}",
        "DISPLAY_REQUIRED": "${{ github.event_name == 'schedule' || (github.event_name == 'workflow_dispatch' && inputs.native_dpi != '' && inputs.native_dpi != 'none') }}",
    }
)


def parse_yaml(text: str) -> dict[str, Any]:
    parsed = yaml.load(text, Loader=yaml.BaseLoader)
    if not isinstance(parsed, dict):
        raise AssertionError("workflow must be a YAML mapping")
    return parsed


def step_by_name(job: dict[str, Any], name: str) -> dict[str, Any]:
    matches = [
        step
        for step in job.get("steps", [])
        if isinstance(step, dict) and (step.get("name") == name or step.get("id") == dict(zip(STEP_NAMES, ["preflight", "checkout", "source_binding", "rust", "python", "prerequisites", "native", "native_inventory", "lane_summary", "evidence_upload", "enforce"])).get(name))
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"expected exactly one step named {name!r}; found {len(matches)}"
        )
    return matches[0]


def require_fragments(text: str, fragments: list[str], *, label: str) -> None:
    missing = [fragment for fragment in fragments if fragment not in text]
    if missing:
        raise AssertionError(f"{label} is missing required fragments: {missing!r}")


def validate_native_interactive_workflow(workflow: dict[str, Any]) -> None:
    triggers = workflow.get("on")
    if not isinstance(triggers, dict) or set(triggers) != {"workflow_dispatch", "schedule", "push"}:
        raise AssertionError(
            "native execution must use trusted dispatch, scheduled main or main push"
        )
    dispatch = triggers["workflow_dispatch"]
    if not isinstance(dispatch, dict) or set(dispatch) != {"inputs"}:
        raise AssertionError("workflow_dispatch must contain only its input contract")
    inputs = dispatch["inputs"]
    if not isinstance(inputs, dict) or set(inputs) != {"source_sha", "native_dpi"}:
        raise AssertionError("native source and display-profile inputs are required")
    if inputs["source_sha"] != {
        "description": "Full trusted Sorotte commit SHA to validate",
        "required": "true",
        "type": "string",
    }:
        raise AssertionError("source_sha dispatch contract drifted")

    if workflow.get("permissions") != {"contents": "read"}:
        raise AssertionError("workflow permissions must remain contents-read only")
    if "${{ secrets." in str(workflow):
        raise AssertionError("native interactive workflow must not consume secrets")
    if "env" in workflow:
        raise AssertionError("workflow-level environment can bypass runner attestation")

    jobs = workflow.get("jobs")
    if triggers["push"] != {"branches": ["main"]}:
        raise AssertionError("automatic native execution must use only main pushes")
    if not isinstance(jobs, dict) or set(jobs) != {"selection", "native_interactive"}:
        raise AssertionError("workflow must contain hosted selection and one native_interactive job")
    selection = jobs["selection"]
    if selection.get("runs-on") != "ubuntu-24.04" or "if" in selection or "continue-on-error" in selection:
        raise AssertionError("native applicability must execute on an unprivileged hosted worker")
    selection_commands = "\n".join(step.get("run", "") for step in selection.get("steps", []))
    for fragment in ('test "$REQUESTED_SOURCE_SHA" = "$GITHUB_SHA"',
                     'scripts/verify.py plan --base "$BASE_SHA" --head "$GITHUB_SHA"',
                     '--lane native ${FORCE:+"$FORCE"} --github-output "$GITHUB_OUTPUT"'):
        if fragment not in selection_commands:
            raise AssertionError("native applicability or workflow source binding drifted")
    job = jobs["native_interactive"]
    if not isinstance(job, dict):
        raise AssertionError("native_interactive job must be a mapping")
    if job.get("needs") != "selection" or job.get("if") != "needs.selection.outputs.selected == 'true'":
        raise AssertionError("native job must follow successful hosted applicability")
    if job.get("runs-on") != RUNNER_LABELS:
        raise AssertionError("ephemeral interactive runner label contract drifted")
    if job.get("timeout-minutes") != "45":
        raise AssertionError("native interactive job timeout must remain 45 minutes")
    if "environment" in job or "continue-on-error" in job:
        raise AssertionError("native interactive job cannot weaken failure semantics")
    if job.get("env") != {"NATIVE_ARTIFACT_ROOT": NATIVE_ARTIFACT_ROOT,
                          "SYNCPLAY_LEGACY_ROOT": "${{ github.workspace }}\\.interop-cache\\syncplay-legacy",
                          "SYNCPLAY_REQUIRE_LIVE_INTEROP": "1"}:
        raise AssertionError("native artifact and required pinned interop environment drifted")
    if ATTESTATION_ENV_KEYS & set(job.get("env", {})):
        raise AssertionError("workflow cannot self-assert runner attestations")

    steps = job.get("steps")
    if not isinstance(steps, list):
        raise AssertionError("native_interactive steps must be an array")
    expected_ids = ["preflight", "checkout", "source_binding", "rust", "python", "prerequisites", "native", "display", "native_inventory", "lane_summary", "safe_export", "evidence_upload", "enforce"]
    if [step.get("id") for step in steps] != expected_ids:
        raise AssertionError("native trust, evidence, and execution dependency order drifted")
    for step in steps:
        if not isinstance(step, dict):
            raise AssertionError("every workflow step must be a mapping")
        env = step.get("env", {})
        if not isinstance(env, dict):
            raise AssertionError("step env must be a mapping")
        if ATTESTATION_ENV_KEYS & set(env):
            raise AssertionError("workflow cannot self-assert runner attestations")

    preflight = step_by_name(job, STEP_NAMES[0])
    if (
        preflight.get("id") != "preflight"
        or preflight.get("continue-on-error") != "true"
        or preflight.get("shell") != "pwsh"
        or "if" in preflight
    ):
        raise AssertionError("pre-checkout runner preflight execution contract drifted")
    if preflight.get("env") != {
        "REQUESTED_SOURCE_SHA": REQUESTED_SHA,
        "NATIVE_CI_EVIDENCE_ROOT": EVIDENCE_ROOT,
    }:
        raise AssertionError("preflight environment binding drifted")
    preflight_run = preflight.get("run", "")
    require_fragments(
        preflight_run,
        [
            "^[0-9a-f]{40}$",
            "Require-RunnerCondition ($requestedSha -ceq $env:GITHUB_SHA)",
            "SOROTTE_NATIVE_RUNNER_INPUTS_SHA256 -cmatch '^[0-9a-f]{64}$'",
            "SOROTTE_NATIVE_RUNNER_CONTRACT",
            "sorotte-ephemeral-interactive-windows-v1",
            "SOROTTE_NATIVE_RUNNER_INSTANCE_ID",
            "SOROTTE_NATIVE_RUNNER_MAX_JOBS",
            '[Guid]::TryParse',
            'runnerMaxJobs -ceq "1"',
            "SessionId",
            "session 0",
            "Get-Process -Name explorer",
            "OpenInputDesktop",
            "GetForegroundWindow",
            "preflight.json",
            "interactive runner preflight failed",
        ],
        label="preflight",
    )
    if preflight_run.index("preflight.json") > preflight_run.index(
        "interactive runner preflight failed"
    ):
        raise AssertionError("preflight failure evidence must be written before failure")

    checkout = step_by_name(job, STEP_NAMES[1])
    if (
        checkout.get("id") != "checkout"
        or checkout.get("if") != "steps.preflight.outcome == 'success'"
        or checkout.get("uses") != CHECKOUT_ACTION
        or checkout.get("with")
        != {
            "ref": REQUESTED_SHA,
            "fetch-depth": "1",
            "clean": "true",
            "persist-credentials": "false",
        }
    ):
        raise AssertionError("exact source checkout contract drifted")

    source_binding = step_by_name(job, STEP_NAMES[2])
    if (
        source_binding.get("id") != "source_binding"
        or source_binding.get("if") != "steps.checkout.outcome == 'success'"
        or source_binding.get("shell") != "pwsh"
        or source_binding.get("env")
        != {
            "REQUESTED_SOURCE_SHA": REQUESTED_SHA,
            "NATIVE_CI_EVIDENCE_ROOT": EVIDENCE_ROOT,
        }
    ):
        raise AssertionError("source-binding step contract drifted")
    require_fragments(
        source_binding.get("run", ""),
        [
            "git rev-parse HEAD",
            "-cne $env:REQUESTED_SOURCE_SHA",
            "git status --porcelain --untracked-files=no",
            "source-binding.json",
        ],
        label="source binding",
    )

    rust = step_by_name(job, STEP_NAMES[3])
    if {k: v for k, v in rust.items() if k != "name"} != {
        "id": "rust",
        "if": "steps.source_binding.outcome == 'success'",
        "uses": RUST_ACTION,
        "with": {"toolchain": "1.97.1"},
    }:
        raise AssertionError("pinned Rust setup contract drifted")

    python = step_by_name(job, STEP_NAMES[4])
    if {k: v for k, v in python.items() if k != "name"} != {
        "id": "python",
        "if": "steps.rust.outcome == 'success'",
        "uses": PYTHON_ACTION,
        "with": {"python-version": "3.12.10"},
    }:
        raise AssertionError("pinned Python setup contract drifted")

    prerequisites = step_by_name(job, STEP_NAMES[5])
    if (
        prerequisites.get("id") != "prerequisites"
        or prerequisites.get("if") != "steps.python.outcome == 'success'"
        or prerequisites.get("shell") != "pwsh"
        or "continue-on-error" in prerequisites
        or "python -m pip install --disable-pip-version-check -r requirements/legacy-python-interop.txt" not in prerequisites.get("run", "")
        or "python scripts/native_harness_canary.py --output" not in prerequisites.get("run", "")
        or "python scripts/verification_tools.py verify-legacy $env:SYNCPLAY_LEGACY_ROOT" not in prerequisites.get("run", "")
        or prerequisites.get("env") != {"CARGO_TARGET_DIR": CARGO_TARGET_ROOT}
    ):
        raise AssertionError("native prerequisite installation contract drifted")

    native = step_by_name(job, STEP_NAMES[6])
    if (
        native.get("id") != "native"
        or native.get("if") != "steps.prerequisites.outcome == 'success'"
        or native.get("continue-on-error") != "true"
        or native.get("shell") != "pwsh"
        or native.get("env") != {"CARGO_TARGET_DIR": CARGO_TARGET_ROOT}
    ):
        raise AssertionError("strict native execution step contract drifted")
    native_run = native.get("run", "")
    require_fragments(
        native_run,
        [
            "& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass",
            "-File scripts/gui-native-smoke.ps1",
            "-Json",
            "-TimeoutMs 80000",
            "-InputMode StrictPhysical",
            "exit $LASTEXITCODE",
        ],
        label="native command",
    )
    scenarios = re.findall(r"--scenario\s+([a-z0-9-]+)", native_run)
    if scenarios != list(gui_native_smoke_contract.DEFAULT_REQUIRED_SCENARIOS):
        raise AssertionError(
            "workflow scenario inventory differs from the strict validator inventory"
        )
    if native_run.count("-TimeoutMs 80000") != 1:
        raise AssertionError("native command must bind exactly one 80-second timeout")
    if native_run.count("-InputMode StrictPhysical") != 1:
        raise AssertionError("native command must bind exactly one strict physical input mode")
    if "UiaOnly" in native_run or "uia-only" in native_run:
        raise AssertionError("strict native workflow must not select local UIA-only mode")
    for forbidden in ("--allow-stderr", "-AllowStderr", "-KeepOpen", "-BinaryPath"):
        if forbidden in native_run:
            raise AssertionError(f"native command contains forbidden option {forbidden}")

    inventory = step_by_name(job, STEP_NAMES[7])
    if (
        inventory.get("id") != "native_inventory"
        or inventory.get("if") != "always()"
        or inventory.get("continue-on-error") != "true"
        or inventory.get("shell") != "pwsh"
        or inventory.get("env")
        != {
            "NATIVE_OUTCOME": "${{ steps.native.outcome }}",
            "NATIVE_CI_EVIDENCE_ROOT": EVIDENCE_ROOT,
        }
    ):
        raise AssertionError("native evidence inventory step contract drifted")
    inventory_run = inventory.get("run", "")
    required_block = re.search(
        r"\$requiredFiles\s*=\s*@\((.*?)\n\s*\)",
        inventory_run,
        flags=re.DOTALL,
    )
    if required_block is None:
        raise AssertionError("native evidence required-file inventory is missing")
    observed_required_files = re.findall(r'"([^"]+)"', required_block.group(1))
    if observed_required_files != REQUIRED_NATIVE_FILES:
        raise AssertionError("native evidence required-file inventory drifted")
    require_fragments(
        inventory_run,
        [
            '$env:NATIVE_OUTCOME -ne "skipped"',
            "expected exactly one native artifact run directory",
            "Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256",
            "native-artifact-inventory.json",
            "native evidence inventory failed",
        ],
        label="native evidence inventory",
    )

    summary = step_by_name(job, STEP_NAMES[8])
    if (
        summary.get("id") != "lane_summary"
        or summary.get("if") != "always()"
        or summary.get("shell") != "pwsh"
        or summary.get("env")
        != {
            **SUMMARY_OUTCOMES,
            "NATIVE_CI_EVIDENCE_ROOT": EVIDENCE_ROOT,
        }
        or "lane-outcomes.json" not in summary.get("run", "")
    ):
        raise AssertionError("always-written lane outcome evidence contract drifted")

    upload = step_by_name(job, STEP_NAMES[9])
    if (
        upload.get("id") != "evidence_upload"
        or upload.get("if") != "always()"
        or upload.get("uses") != UPLOAD_ACTION
        or "continue-on-error" in upload
    ):
        raise AssertionError("always-uploaded native evidence step contract drifted")
    upload_with = upload.get("with")
    if not isinstance(upload_with, dict):
        raise AssertionError("artifact upload inputs must be a mapping")
    if upload_with != {
        "name": "native-interactive-${{ inputs.source_sha || github.sha }}-${{ github.run_id }}-${{ github.run_attempt }}",
        "path": "${{ runner.temp }}\\native-safe-${{ github.run_id }}-${{ github.run_attempt }}",
        "if-no-files-found": "error", "retention-days": "90",
    }:
        raise AssertionError("native evidence must upload only the safe projection with immutable attempt identity")
    safe = next(step for step in steps if step.get("id") == "safe_export")
    if safe.get("if") != "always()":
        raise AssertionError("safe failure projection must always execute")
    require_fragments(safe.get("run", ""), ["native_failure_evidence.py export", "authoritative=$false", "exporter-unavailable-before-checkout-or-setup", "--run-attempt $env:GITHUB_RUN_ATTEMPT"], label="privacy-safe projection")
    display = next(step for step in steps if step.get("id") == "display")
    if display.get("if") != "steps.native.outcome == 'success' && (github.event_name == 'schedule' || (github.event_name == 'workflow_dispatch' && inputs.native_dpi != '' && inputs.native_dpi != 'none'))":
        raise AssertionError("display profiles require a passed isolated native inventory")
    require_fragments(display.get("run", ""), ["-ExpectedNativeDpi ([int]$env:EXPECTED_NATIVE_DPI)", "gui-display-matrix.ps1"], label="measured display profile")
    require_fragments(preflight_run, ["$env:GITHUB_EVENT_NAME -ne 'schedule'", "'refs/heads/main'"], label="scheduled source authorization")

    enforcement = step_by_name(job, STEP_NAMES[10])
    if (
        enforcement.get("if") != "always()"
        or enforcement.get("shell") != "pwsh"
        or "continue-on-error" in enforcement
        or enforcement.get("env") != ENFORCED_OUTCOMES
    ):
        raise AssertionError("final native lane enforcement contract drifted")
    enforcement_run = enforcement.get("run", "")
    require_fragments(
        enforcement_run,
        [
            "$env:NATIVE_OUTCOME",
            "$env:NATIVE_INVENTORY_OUTCOME",
            "$env:EVIDENCE_UPLOAD_OUTCOME",
            '$_.Value -ne "success"',
            "native interactive lane is incomplete",
        ],
        label="final enforcement",
    )


class NativeInteractiveWorkflowPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow_text = WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.workflow = parse_yaml(cls.workflow_text)

    def assert_policy_rejects(
        self,
        mutation: Callable[[dict[str, Any]], None],
    ) -> None:
        workflow = copy.deepcopy(self.workflow)
        mutation(workflow)
        with self.assertRaises(AssertionError):
            validate_native_interactive_workflow(workflow)

    def job(self, workflow: dict[str, Any]) -> dict[str, Any]:
        return workflow["jobs"]["native_interactive"]

    def mutate_step(
        self,
        workflow: dict[str, Any],
        name: str,
        mutation: Callable[[dict[str, Any]], None],
    ) -> None:
        mutation(step_by_name(self.job(workflow), name))

    def test_checked_in_workflow_passes_policy(self) -> None:
        validate_native_interactive_workflow(self.workflow)

    def test_actionlint_declares_only_external_native_labels(self) -> None:
        config = parse_yaml(ACTIONLINT_CONFIG_PATH.read_text(encoding="utf-8"))
        self.assertEqual(
            config,
            {
                "self-hosted-runner": {
                    "labels": [
                        "sorotte-native-interactive",
                        "sorotte-ephemeral",
                    ]
                }
            },
        )

    def test_untrusted_trigger_is_rejected(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: workflow["on"].update({"pull_request": {}})
        )

    def test_missing_ephemeral_runner_label_is_rejected(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.job(workflow)["runs-on"].remove(
                "sorotte-ephemeral"
            )
        )

    def test_workflow_cannot_self_attest_runner_lifetime(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.job(workflow)["env"].update(
                {"SOROTTE_NATIVE_RUNNER_MAX_JOBS": "1"}
            )
        )

    def test_interactive_desktop_preflight_cannot_be_removed(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[0],
                lambda step: step.update(
                    {"run": step["run"].replace("OpenInputDesktop", "RemovedProbe")}
                ),
            )
        )

    def test_checkout_must_use_requested_full_sha(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[1],
                lambda step: step["with"].update({"ref": "${{ github.sha }}"}),
            )
        )

    def test_checkout_subject_cannot_differ_from_authorized_workflow_revision(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[0],
                lambda step: step.update(
                    {"run": step["run"].replace(
                        "Require-RunnerCondition ($requestedSha -ceq $env:GITHUB_SHA)",
                        "Require-RunnerCondition ($true)",
                    )}
                ),
            )
        )

    def test_display_labels_can_change_without_weakening_step_contracts(self) -> None:
        workflow = copy.deepcopy(self.workflow)
        for step in self.job(workflow)["steps"]:
            step["name"] = "A clearer display label for " + step["id"]
        validate_native_interactive_workflow(workflow)

    def test_display_profile_is_requested_only_by_schedule_or_explicit_dispatch_input(self) -> None:
        steps = {step["id"]: step for step in self.job(self.workflow)["steps"]}
        display_if = steps["display"]["if"]
        required_if = steps["enforce"]["env"]["DISPLAY_REQUIRED"][3:-2].strip()
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["native_dpi"]["default"], "none")

        def evaluate(expression: str, event: str, dpi: str, outcome: str) -> bool:
            for token, value in (("github.event_name", event), ("inputs.native_dpi", dpi),
                                 ("steps.native.outcome", outcome)):
                expression = expression.replace(token, repr(value))
            tree = ast.parse(expression.replace("&&", " and ").replace("||", " or "), mode="eval")
            allowed = (ast.Expression, ast.BoolOp, ast.And, ast.Or, ast.Compare, ast.Eq, ast.NotEq, ast.Constant)
            self.assertTrue(all(isinstance(node, allowed) for node in ast.walk(tree)))
            return eval(compile(tree, "<workflow condition>", "eval"), {"__builtins__": {}})

        for event, dpi, required in (("push", "", False), ("push", "144", False),
                                     ("workflow_dispatch", "none", False), ("workflow_dispatch", "", False),
                                     ("workflow_dispatch", "96", True), ("workflow_dispatch", "144", True),
                                     ("workflow_dispatch", "192", True), ("schedule", "", True)):
            for outcome in ("success", "failure", "skipped"):
                with self.subTest(event=event, dpi=dpi, outcome=outcome):
                    self.assertEqual(evaluate(required_if, event, dpi, outcome), required)
                    self.assertEqual(evaluate(display_if, event, dpi, outcome), required and outcome == "success")

    def test_native_timeout_weakening_is_rejected(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[6],
                lambda step: step.update(
                    {"run": step["run"].replace("-TimeoutMs 80000", "-TimeoutMs 0")}
                ),
            )
        )

    def test_local_uia_only_mode_cannot_replace_strict_physical_ci(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[6],
                lambda step: step.update(
                    {
                        "run": step["run"].replace(
                            "-InputMode StrictPhysical", "-InputMode UiaOnly"
                        )
                    }
                ),
            )
        )

    def test_incomplete_native_scenario_inventory_is_rejected(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[6],
                lambda step: step.update(
                    {
                        "run": step["run"].replace(
                            "    --scenario transport\n", ""
                        )
                    }
                ),
            )
        )

    def test_native_stderr_allowlist_is_rejected(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[6],
                lambda step: step.update(
                    {"run": step["run"] + " --allow-stderr known-warning\n"}
                ),
            )
        )

    def test_missing_native_report_binding_is_rejected(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[7],
                lambda step: step.update(
                    {
                        "run": step["run"].replace(
                            '    "native-report.json",\n', ""
                        )
                    }
                ),
            )
        )

    def test_upload_cannot_warn_on_missing_evidence(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[9],
                lambda step: step["with"].update({"if-no-files-found": "warn"}),
            )
        )

    def test_upload_must_not_publish_raw_native_artifact_root(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[9],
                lambda step: step["with"].update({"path": EVIDENCE_ROOT}),
            )
        )

    def test_final_gate_must_enforce_native_inventory_outcome(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.mutate_step(
                workflow,
                STEP_NAMES[10],
                lambda step: step["env"].pop("NATIVE_INVENTORY_OUTCOME"),
            )
        )

    def test_secret_reference_is_rejected(self) -> None:
        self.assert_policy_rejects(
            lambda workflow: self.job(workflow)["env"].update(
                {"PRODUCTION_TOKEN": "${{ secrets.PRODUCTION_TOKEN }}"}
            )
        )


if __name__ == "__main__":
    unittest.main()
