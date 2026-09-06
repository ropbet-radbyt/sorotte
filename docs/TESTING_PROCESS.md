# Testing process

The required checks preserve the existing behavior, changed-line coverage, mutation,
fuzz, live interop and real-player responsibilities. The testing-apparatus audit is
the [design and baseline](audits/testing-apparatus-audit-2026-09-06.md); the
[implementation ledger](audits/testing-process-implementation-2026-09-06.md) records
what has actually been exercised. Historical timings are observations of those
revisions, not a current promise.

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

# Full apparatus self-tests and ordinary all-feature behavior, with streamed logs,
# source/input identity, primary failure and owned-process cleanup receipts.
python scripts/verify.py run --lane static --output target/verification/static-attempt-1
python scripts/verify.py run --lane behavior --output target/verification/behavior-attempt-1
cargo test --locked --workspace --all-features --doc
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

Attempt directories must be fresh. Preserve a failed attempt and choose a new
directory for a retry. An unchanged retry is diagnostic evidence; it does not
erase the original failure or establish that it was a flake. `verify run` adds
Git trust only to its own process environment, detects source changes during the
run, and keeps nextest's failed-then-passed and leaked-process policies intact.
Compile-dependent inventory preparation is a separate step:

```powershell
python scripts/test_inventory.py propose --output target/verification/proposed-inventory.json
python scripts/test_inventory.py diff --proposed target/verification/proposed-inventory.json
python scripts/test_inventory.py check --output target/verification/checked-inventory.json
python scripts/verify.py run --lane coverage-canary --output target/verification/coverage-attempt-1
```

Review inventory additions, removals and ignore changes before updating
`coverage/test-inventories.json`. Discovery cannot overwrite that authority.
Complete inventories supply totals; exact named selections still define required
responsibilities. Empty selections, missing required tests and unexpected skips
remain failures.

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

Required aggregates reject missing, cancelled, failed and unexpectedly skipped
producers. A no-op must match independently supplied event base/head and a
recomputed plan. Artifact names identify attempts so retries retain earlier
evidence. Stable step IDs let policy tests tolerate label changes while preserving
commands, dependency edges, source authority and outcome enforcement.

See [mutation campaigns](MUTATION_CAMPAIGNS.md) for balanced chunks, exact inventory
union and streaming cleanup; [native infrastructure](NATIVE_TEST_INFRASTRUCTURE.md)
for the one-job Sandbox controller, trusted candidate dispatch and diagnostic export;
and [release qualification](RELEASE_QUALIFICATION.md) for shared tested binaries,
archive consumption and approved-container digest promotion.

Native qualification requires a maintainer-authorized candidate and the isolated
interactive guest. The ordinary PR workflow runs only on hosted workers; it cannot
dispatch arbitrary PR code to the native runner. Missing desktop capability stays
an unavailable required proof. The pinned minimum/newest real-mpv tests and the
independent lifecycle oracle remain separate from fake-server readiness canaries.

## Inputs and evidence

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
in addition to successful trusted main-push checks. The user has postponed
[Protection reader App setup](PROTECTION_READER_SETUP.md) to a follow-up. No App
or credentials have been created. Publication authorization deliberately fails
with an actionable setup error until that dependency is configured; PR testing
and native candidate qualification do not require the App.

Prepare and review code, finish hosted/native acceptance, activate the reviewed
required-check policy, and complete App setup before using the new stable/dev
publication path. Do not bypass a missing proof to make a release proceed.
