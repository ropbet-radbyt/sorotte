# Native interactive CI contract — 2026-07-31

## Outcome and boundary

This slice adds a repository-owned, dispatch-only CI definition for the
existing strict ten-scenario Windows native GUI inventory. It does not claim
that GitHub-hosted Windows is interactive, that an external runner has been
provisioned, or that the native suite ran in this slice.

The lane is intentionally limited to `workflow_dispatch` with a required full
trusted commit SHA. It is not a pull-request, scheduled, merge-queue, or
required-branch gate while ephemeral interactive runner infrastructure is
unavailable and unverified. The existing native harness and strict evidence
validator are unchanged.

Files added:

```text
.github/actionlint.yaml
.github/workflows/gui-native-interactive.yml
scripts/tests/test_native_interactive_workflow.py
docs/evidence/test-coverage/native-interactive-ci-contract-20260731.md
```

No central coverage document was changed by this delegated slice.

## External runner and pre-checkout contract

GitHub may schedule the job only on a runner with the exact label inventory:

```text
self-hosted
Windows
X64
sorotte-native-interactive
sorotte-ephemeral
```

The external provisioner, not the workflow, must supply:

```text
SOROTTE_NATIVE_RUNNER_CONTRACT=sorotte-ephemeral-interactive-windows-v1
SOROTTE_NATIVE_RUNNER_INSTANCE_ID=<nonzero UUID unique to this runner instance>
SOROTTE_NATIVE_RUNNER_MAX_JOBS=1
```

The workflow and its policy tests forbid defining those attestations in
workflow or job environment. Before checkout, an inline fail-closed preflight
requires:

- a full lowercase 40-hex source SHA;
- GitHub Actions on Windows x64;
- the exact external contract and one-job lifetime attestations;
- a nonzero Windows session ID and a named non-service session;
- an Explorer shell in the runner process's session;
- access to the current Win32 input desktop; and
- a foreground window owned by the same session.

The preflight writes `preflight.json` before returning failure. Repository
content is not checked out or executed unless this preflight succeeds.

The attestations make the infrastructure boundary explicit, but an in-job
probe cannot prove that the control plane actually destroys the machine after
the job. Provisioning, one-job registration, and teardown remain obligations
of the external runner service.

## Source, tool, and prerequisite binding

Checkout is pinned to
`actions/checkout@11d5960a326750d5838078e36cf38b85af677262` with
`persist-credentials: false`, `clean: true`, depth one, and the exact requested
SHA. Before repository-controlled setup runs, a source-binding step requires
`git rev-parse HEAD` to equal the requested SHA and rejects tracked changes.
It retains `source-binding.json`.

The lane then uses:

```text
Rust:   1.97.1 via dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4
Python: 3.11 via actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065
Python prerequisites: requirements/legacy-python-interop.txt
```

Cargo output is isolated under a run/attempt-specific `runner.temp` path.
Workflow permissions are exactly `contents: read`; the workflow references no
secret and no GitHub environment.

## Exact native command and timeouts

The native step invokes the existing strict wrapper with no stderr allowlist,
caller binary, exploratory keep-open mode, or optional scenario:

```powershell
powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File scripts/gui-native-smoke.ps1 `
  -Json `
  -TimeoutMs 80000 `
  --scenario baseline `
  --scenario relaunch `
  --scenario drag-drop `
  --scenario loopback `
  --scenario menu-open-media `
  --scenario live-python `
  --scenario controlled-room `
  --scenario detached-missing-media `
  --scenario missing-media-continue `
  --scenario transport
```

Policy compares this exact ordered list with
`gui_native_smoke_contract.DEFAULT_REQUIRED_SCENARIOS`. Therefore validator
inventory drift or removal of one workflow scenario fails policy.

The existing wrapper derives a 910,000 ms maximum native process wait for ten
80,000 ms scenarios, its global completion boundary, and its 30,000 ms grace
period. Each of the two locked build phases retains its existing 600,000 ms
watchdog. The GitHub job has a 45-minute outer timeout.

The wrapper's validator still rejects every required skip, missing structured
capability, unexpected native stderr line, panic/background failure, and
nonzero producer status. The workflow supplies no stderr exception.

## Always-retained and fail-closed evidence

Preflight, source-binding, and lane-outcome JSON are written to a unique
`runner.temp` evidence directory. After a native attempt, a separate
always-running inventory step requires exactly one wrapper run directory and
these eight base files:

```text
native-report.json
native-stderr.log
contract-summary.json
invocation.json
build-stdout.log
build-stderr.log
harness-build-stdout.log
harness-build-stderr.log
```

It records every retained file's relative path, byte length, and SHA-256 in
`native-artifact-inventory.json`. A missing `native-report.json`, any other
base file, the complete run directory, or a second run directory fails this
step.

Artifact upload runs under `always()`, uses pinned
`actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02`,
retains both CI metadata and native wrapper output for 14 days, and treats a
completely missing upload path as an error. The final always-running gate
requires success from preflight, checkout, source binding, both tool setups,
prerequisite installation, native execution, native inventory, lane-summary
write, and artifact upload. Consequently a successful upload of preflight
metadata cannot mask a missing native report or failed strict contract.

Runner loss or workflow cancellation can prevent any post-failure upload; no
in-job design can retain evidence after the execution machine becomes
unavailable. Normal step, build, harness, validator, or policy failures are
covered by the always path above.

## Policy resistance

The dedicated policy suite accepts only the exact dispatch, runner,
preflight, checkout, tool, scenario, timeout, evidence, upload, and aggregate
contracts. Adversarial mutation tests reject:

- adding an automatic or untrusted trigger;
- removing the ephemeral runner label;
- self-asserting the one-job runner marker;
- removing the interactive desktop probe;
- checking out anything except the requested SHA;
- weakening the native timeout or scenario inventory;
- adding a native stderr allowlist;
- removing `native-report.json` from the required evidence inventory;
- warning instead of failing on missing upload evidence;
- dropping native output from the uploaded paths;
- omitting the native inventory result from final enforcement; and
- introducing a secret reference.

## Local validation

The following validation completed on the shared
`codex/test-coverage-design` worktree at base commit
`cfb8adf7f4768ea13673af005effcb11e6eee2d2`:

```text
C:\Users\shaun\go\bin\actionlint.exe .github/workflows/gui-native-interactive.yml
  passed

python -m unittest scripts.tests.test_native_interactive_workflow -v
  15/15 passed

python -m unittest discover -s scripts/tests -p "test_*.py" -v
  418/418 passed in 22.646s

PowerShell AST parsing of every workflow run block
  7/7 passed

Trailing-whitespace scan of the workflow, actionlint config, and policy test
  passed
```

Implementation hashes before this evidence file was added:

```text
79ba89fd328ed45af09f55890121a4c696d2a72bbb1c1b6651eb429095f9c541  .github/actionlint.yaml
b6d290c23e33b871a24c89b5d49dd686653488b8db812255d826334d5e4b3476  .github/workflows/gui-native-interactive.yml
69784fcfa053003d5d31d38de7480f85d5eab3a256434c6425ca329e94e73fb2  scripts/tests/test_native_interactive_workflow.py
```

Other agents had concurrent dirty files in the shared worktree. This slice did
not stage, restore, reset, clean, or otherwise modify those files.

## Genuine external blocker and next proof

No matching ephemeral interactive Windows runner or its post-job destruction
was available for local verification. The repository slice is therefore a
deployable, fail-closed manual lane definition, not evidence of a green native
CI run and not yet a required gate.

The next proof is operational: provision a fresh one-job runner in an unlocked
interactive Windows session with the exact labels and external attestations,
dispatch this workflow for a reviewed full commit SHA, require the final gate
to pass, retain the uploaded artifact, and independently confirm runner
destruction. Only after repeated green executions should the lane be promoted
to trusted merge-queue or scheduled policy.
