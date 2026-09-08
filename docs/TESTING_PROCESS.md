# Testing process

The required checks preserve the existing behavior, changed-line coverage, mutation,
fuzz, live interop and real-player responsibilities. The testing-apparatus audit is
the [design and baseline](audits/testing-apparatus-audit-2026-09-06.md); the
[implementation ledger](audits/testing-process-implementation-2026-09-06.md) records
what has actually been exercised. Historical timings are observations of those
revisions, not a current promise.

The [first coordinated stable-release attempt](audits/v0.2.10-release-attempt.md)
records three additional apparatus failures, their reproductions and the narrow
repairs in the corrected point-release candidate.
It also records the subsequent Windows receipt-canary timeout, controlled
reproduction, preserved assertions and bounded supervision repair.
The later persistence-fixture follow-up distinguishes an unexplained native abort
from a separately reproduced, unintended background HTTP dependency.
The scheduled-check follow-up records a nightly job publishing a skipped check
under a reserved release name, and the event isolation that prevents the collision.
The [fixture boundary follow-up](audits/test-fixture-boundaries-2026-09-08.md)
records two tests that passed on the PR and failed on the identical main tree:
an unobserved persistence commit before a hard restart and an accepted HTTP
socket inheriting nonblocking mode. It distinguishes controlled reproductions
from the incomplete diagnostics of the original executions.

## Local command ladder

Use the repository's Rust toolchain and Python 3.11–3.13. Install the reviewed
policy environment with `python -m pip install -r requirements/ci-policy.txt`.
Keep `TEMP` outside the checkout and use ordinary process permissions: Windows
verification must be able to terminate and wait for its own child processes.

```powershell
# No Rust compilation: syntax, responsibility/model/ignore/mutation/corpus policies,
# temporary files, process control and loopback. This does not operate a desktop.
python scripts/verify.py preflight --phase static --output target/verification/preflight.json

# Check installed producer versions and, when needed, the exact clean Python reference.
python scripts/verify.py preflight --phase tools --tool rust --tool cargo-nextest
python scripts/verify.py preflight --phase tools --tool cargo-llvm-cov --legacy ../syncplay

# Review both base and candidate obligations using immutable Git commits.
python scripts/verify.py plan --base BASE_SHA --head HEAD_SHA --output target/verification/plan.json

# Short feedback before integration. Choose the owning crate and exact regression.
cargo fmt --all -- --check
cargo test --locked -p sorotte-server --test server_release_verify fixture_timeout_preserves_primary_failure_and_next_case_runs_after_cleanup -- --exact
python scripts/verify.py run --lane regression --output target/verification/regression-attempt-1

# Full apparatus self-tests and both workspace execution modes, with streamed logs,
# source/input identity, primary failure and owned-process cleanup receipts.
python scripts/verify.py run --lane static --output target/verification/static-attempt-1
python scripts/verify.py run --lane workspace-default --output target/verification/workspace-default-attempt-1 --deadline-seconds 1200
python scripts/verify.py run --lane behavior --output target/verification/behavior-attempt-1
cargo test --locked --workspace --all-features --doc
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

Attempt directories must be fresh. Preserve a failed attempt and choose a new
directory for a retry. An unchanged retry is diagnostic evidence; it does not
erase the original failure or establish that it was a flake. `verify run` adds
Git trust only to its own process environment, detects source changes during the
run, and keeps nextest's failed-then-passed and leaked-process policies intact.
The `workspace-default` lane requires a clean checkout and executes
`cargo test --locked --workspace` through the release workspace receipt writer.
It uses Cargo's ordinary default-feature test harness, with no retry or
`RUST_TEST_THREADS` override, and preserves the workspace receipt plus the outer
attempt, process logs, elapsed time and owned cleanup. A timeout or missing
workspace receipt cannot qualify the lane. Its source is the actual checkout
commit, which can be the prospective PR merge rather than the event's PR head.

Ordinary Cargo and nextest runs discover the current tests themselves. Adding a
test requires no global inventory file, refresh command or unrelated count edit.
The required jobs continue to run the default-feature Cargo suite, all-feature
nextest and doctests in their existing execution modes.

Focused coverage jobs declare required regression tests for their behavior.
Every required name must pass, and every additional test selected by that job
must also pass. Headers, individual results and summaries must agree; duplicates,
unexpected skips, omitted required tests and tests outside the command's selector
are failures. Filtered-out totals are recorded as observations. Full updater
jobs additionally require zero filtered tests. A rename or removal of a required
regression needs a corresponding review of that job's behavior requirement.

Complete live compatibility keeps runtime discovery because its custom consumer
accounts for every discovered test as executed, explicitly ignored or skipped.
Required mode forbids optional skip paths; independent live sentinels and the
explicit fixture-generator ignore policy remain mandatory. New ordinary tests
are automatically included in that same execution accounting.

Mutation retains its independent per-scope discovery and source binding. Test
changes select their package and declared transitive responsibilities without a
global test-list update forcing an unrelated full campaign. Changes to mutation
policy, shared tooling, compiler inputs and lockfiles still require full work.

## Required checks and source subjects

`coverage/verification-lanes.json` declares the stable required checks and change
responsibilities. Documentation-only paths can produce validated no-op receipts;
unknown paths and changes to the apparatus conservatively select all obligations.
Base-policy obligations cannot disappear because the candidate edits a selector.
Fuzz and native selection currently include all crates, pending an independently
reviewed narrower dependency closure.

Ordinary behavior tests use GitHub's prospective PR merge. Coverage, mutation,
fuzz, package and native evidence identify the exact PR head; after merging,
required main-push runs identify the actual merge commit. Equal trees do not make
these source subjects interchangeable. Coverage producers stay parallel; API
compatibility has its own producer; formatting and static validation run before
expensive behavior work.

Both existing Linux and Windows Rust test workers require the default-feature
Cargo workspace run as well as all-feature nextest and doctests. The ordinary
Cargo harness runs tests concurrently within each binary; nextest uses a
different process model, and all-features can change which code executes. A pass
in one mode cannot substitute for the other. Default execution runs after the
pinned live-interop prerequisites, and its failure is retained while nextest can
still run. Each existing final test gate rejects a failed, missing or skipped
default outcome. Attempt-specific uploads run even after failure; the public
check names and aggregate dependency graph remain unchanged.

The default phase has a 1,200-second execution ceiling followed by the owned
supervisor's bounded cleanup reserve. The enclosing Linux and Windows workers
allow 55 and 45 minutes respectively for setup and both modes. These limits are
ceilings, not measurements of added runtime or a performance claim. The extra
default-feature build and test execution add work; retained per-attempt durations
show the actual cost for each candidate and runner.

Required aggregates reject missing, cancelled, failed and unexpectedly skipped
producers. A no-op must match independently supplied event base/head and a
recomputed plan. Artifact names identify attempts so retries retain earlier
evidence. Stable step IDs let policy tests tolerate label changes while preserving
commands, dependency edges, source authority and outcome enforcement.

The seven required check names are reserved for PR and main-push runs. Scheduled
and manually dispatched aggregates use event-qualified names, including jobs that
are skipped: GitHub still creates a check for a skipped job. Their concurrency
groups also include the event, so assurance runs cannot cancel or replace the
main-push run being qualified. Release authorization still requires exactly one
successful check from each trusted main-push producer; it never substitutes a
scheduled or manually dispatched result.

See [mutation campaigns](MUTATION_CAMPAIGNS.md) for balanced chunks, exact inventory
union and streaming cleanup; [native infrastructure](NATIVE_TEST_INFRASTRUCTURE.md)
for the one-job Sandbox controller, trusted candidate dispatch and diagnostic export;
and [release qualification](RELEASE_QUALIFICATION.md) for shared tested binaries,
archive consumption and approved-container digest promotion.

Promotion regressions run the complete verifier path with actual subprocesses
that produce independent data and diagnostic streams. They check valid and
invalid signature output, failed tools, tag/push ordering and authorization
failure immediately before manifest assignment. This catches command-wrapper
and bookkeeping defects before merge. Local executable fixtures do not replace
the live signature, registry and public-byte checks required for publication.

Native qualification requires a maintainer-authorized candidate and the isolated
interactive guest. The ordinary PR workflow runs only on hosted workers; it cannot
dispatch arbitrary PR code to the native runner. Missing desktop capability stays
an unavailable required proof. The pinned minimum/newest real-mpv tests and the
independent lifecycle oracle remain separate from fake-server readiness canaries.

## Inputs and evidence

### Updating verification tooling

For Rust, cargo-nextest, cargo-llvm-cov, Syft, Cosign and GitHub Actions, edit
the approved values in `coverage/verification-tools.toml`. A Rust update also
requires its reviewed `rust-windows` commit/host/LLVM identity from `rustc -vV`.
Action entries retain both the immutable commit SHA and the upstream review label.
Then use the existing verification entrypoint:

```powershell
python -m pip install -r requirements/ci-policy.txt
python scripts/verify.py pins
python scripts/verify.py pins --write
python scripts/verify.py pins --check
python scripts/verify.py preflight --phase static --output target/verification/pin-update-preflight.json
```

After installing the existing policy prerequisites, `pins` previews the exact
patch; `--write` applies it; `--check` reports drift. The static policy tests
require all projections to match. Selection preflight checks non-workflow
projections using only the Python standard library, before packages are installed.
Planning validates all locations before writing. It never downloads tools or
changes approval data, lockfiles, test inventories, historical evidence, Docker
image digests or native download hashes. Those inputs keep their existing
explicit review procedures.
Python dependency resolution, compatibility baselines and the nightly fuzz
toolchain likewise retain their separate procedures. This command does not
claim to resolve or qualify newer versions.

The updater changes declared scalar pin projections in wrappers, workflows,
workspace/native compiler settings and the container compiler installation.
Policy tests consume approved versions as data and independently exercise bad
versions, source identities, missing work and altered workflow authority. A
routine pin update therefore does not require replacing literals in those tests.
After projection checks, exercise the actual affected tools and existing required
lanes; agreeing declarations alone cannot qualify new tool behavior.

### Retaining qualification evidence

`coverage/verification-tools.toml` is the reviewed input manifest. Rust resolution
uses `--locked`; legacy Python interop verifies the exact clean upstream commit.
Dependency download caches contain checksum-verified Cargo registry archives.
Instrumented profiles, advisory decisions, compiled mutation targets and unverified
source directories are not restored from this cache. A corrupt archive is removed
only inside the owned cache so Cargo can reconstruct it from the lockfile.

```powershell
# Produces JSON plus a readable Markdown index. It does not grant release authority.
python scripts/verify.py ledger --source-sha HEAD_SHA --receipt target/verification/preflight.json --receipt target/verification/static-attempt-1/receipt.json --output target/verification/candidate.json

# Classify a concrete incident without changing its original evidence.
python scripts/verification_ledger.py annotate --receipt target/verification/static-attempt-1/receipt.json --disposition harness-defect --reason "Describe the reproduced mechanism and trace" --output target/verification/incident-1.json

# Input is a deduplicated GitHub job array with source, run and attempt identities.
python scripts/verification_ledger.py metrics --source-sha HEAD_SHA --jobs target/verification/jobs.json --output target/verification/timings.json
python scripts/assurance_registry.py --output target/verification/assurance-status.json
```

Timings distinguish execution span, total job-minutes, cancelled work and the
first failed step. They are not billing, a graph-derived critical path or proof
of product defects. Operator interventions and genuine flakiness stay unavailable
until a recorded incident establishes them. Keep setup/canary costs in before/after
comparisons; reducing invocation counts alone is not a performance result.

The assurance registry records owners, commands, environments and freshness
budgets. Missing source-bound evidence is explicitly unavailable. Scheduled
headless scaling captures normal/large resource invariants and clone sensitivity;
timings remain advisory. Actual 96/192-DPI profiles, screen-reader interaction,
optimized startup and privileged power-loss checks need their declared equipment.
Maintenance fixture generators are never scheduled to rewrite trusted inputs.

## Release activation

The new publication authorization needs classic branch-protection inspection
in addition to successful trusted main-push checks. The owner has configured the
[Protection reader App](PROTECTION_READER_SETUP.md); its expected Actions variable
and secret names are present. The first qualified-main authorization must verify
the App's actual read access. Publication fails closed when that proof is missing;
PR testing and native candidate qualification do not require the App.

Prepare and review code, finish hosted/native acceptance, activate the reviewed
required-check policy, and complete App setup before using the new stable/dev
publication path. Do not bypass a missing proof to make a release proceed.
