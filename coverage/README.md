# Behavior verification

`behaviors.toml` is the source of truth for the behavior claims that Sorotte
currently treats as merge contracts. It intentionally records behavior and
proof identity, not coverage percentages or arbitrary shell commands.

The catalog is enforced by `scripts/behavior_evidence.py`:

```text
python -m pip install -r requirements/ci-policy.txt
python scripts/behavior_evidence.py validate --catalog coverage/behaviors.toml
python scripts/ignored_test_policy.py validate --registry coverage/ignored-tests.toml
python scripts/known_defect_policy.py validate \
  --registry coverage/known-defects.toml \
  --catalog coverage/behaviors.toml
python -m unittest discover -s scripts/tests -p "test_*.py" -v
```

The CI workflow runs two evidence lanes:

- `lifecycle-contract` discovers and executes each Rust proof as one exact,
  non-ignored libtest.
- `gui-semantic` compares the live scenario inventory with the catalog and
  executes all 14 scenarios, including those not yet promoted to named
  behavior proofs.

Each lane writes a shard even when a proof fails, and continues through the
remaining proofs. The `verification-required` job rejects a missing or
duplicate lane, a shard from another workflow run or Git revision, a future or
invalid attempt, a different repository or catalog digest, a skipped/ignored
proof, an incomplete semantic inventory, or any failed dependency job. A
successful shard from an earlier attempt of the same workflow run is accepted
so GitHub's “rerun failed jobs” flow remains usable. The runner verifies both
before and after proof execution that the supplied evidence SHA is the
checked-out Git `HEAD`, and rejects tracked or untracked source changes.
Ignored build/evidence directories remain permitted. Catalog entries cannot
supply commands or environment assignments.

For a local lifecycle evidence run on an operating system declared by the
lane, supply stable local metadata. The SHA must equal `git rev-parse HEAD`;
the worktree must be clean, and the initial catalog lanes are Linux-only:

```text
python scripts/behavior_evidence.py run-lane \
  --catalog coverage/behaviors.toml \
  --lane lifecycle-contract \
  --sha <40-character-git-sha> \
  --repository local/sorotte \
  --run-id local \
  --run-attempt 1 \
  --os linux \
  --output target/verification/evidence.lifecycle-contract.json
```

The shrinkable lifecycle suite fuzzes the reducer input contract with 128
Proptest cases of up to 64 transitions by default; it does not claim that every
generated ordering is adapter-reachable. Set `PROPTEST_CASES=2048` for the
nightly-depth budget. Every generated transition now passes through the
ordinary invariant-checking reducer without a known-defect classifier.
`TC-PLAYER-001` is represented by two positive regressions proving exclusive
successor selection for external-observation and load-acceptance conflicts.
The former `TC-PLAYER-002` histories are also ordinary positive regressions
proving reactivation clears stale logical-terminal state.

Proptest seeds under
`crates/sorotte-player-mpv/proptest-regressions/` are source-file and
strategy-shape scoped. They improve replay while a strategy remains stable,
while named deterministic regressions remain the durable behavior contract.
`known-defects.toml` is retained as an empty, schema-validated registry so a
future expected-failure characterization cannot become implicit. Unresolved
`TC-PLAYER-003` and `TC-COMPAT-001` through `TC-COMPAT-006` are not entered
there: they remain red through an existing intermittent test or the ordinary
strict compatibility tests rather than being wrapped in `should_panic`.

`scripts/tests/test_ci_policy.py` mechanically binds the aggregate's required
job names to the locked all-feature, semantic, compatibility, real-mpv,
Windows, release-build, and evidence commands in the workflow. Repository
review policy is still required to protect that test and workflow from
coordinated weakening.

`ignored-tests.toml` must exactly match every Rust `#[ignore = "reason"]`
attribute under `crates/`. Each entry has an owner, prerequisites, supported
operating systems, and one of four currently used dispositions: required
pull-request CI, manual capability, fixture maintenance, or expiring
quarantine. Unsupported conditional or reasonless ignore attributes fail.
Pull-request entries are additionally checked against exact
`--ignored --exact` workflow invocations.

`known-defects.toml` is the schema-validated inventory for any undesirable
behavior intentionally represented by a Rust expected-failure
characterization and is empty on this branch. If a future entry is added, the
validator exactly matches every Rust `known_defect_*`
`should_panic(expected = "...")` characterization to its source, package,
selector, panic oracle, owner, finding, and expiry. A missing or stale entry,
bare `should_panic`, expired defect, drifted panic oracle, or selector also
listed as a positive behavior proof fails CI. Passing because a
characterization panicked is therefore never presented as proof that the
application behaves correctly. Once a defect is fixed, its characterizations
must become positive regressions and the corresponding registry entry must be
removed.

Required workspace execution uses pinned cargo-nextest 0.9.137 through
`scripts/nextest_ci.py`. The checked profile allows one diagnostic retry but
fails the gate when a failed or leaked first attempt later passes. An inherited
subprocess handle still open after 500 ms is a failed result. The wrapper
rejects a drifted binary version or profile, per-test leak-timeout overrides,
empty or malformed JUnit, failed/rerun/flaky attempt elements, and a nonzero
producer. Console, JUnit, and machine-readable policy evidence are always
uploaded; doctests run as a separate required Cargo command.

That contract exposed an intermittent inherited-handle leak in
`sorotte-gui::updater_self_replacement_windows` test
`running_installed_updater_recovers_interrupted_replacement_and_restarts`.
One run reported `LEAK` after 0.919 seconds, a clean rerun did not reproduce
it, and a later checked run failed even though its retry passed. The updater
and test remain unchanged under `TC-HARNESS-006`; the real and controlled
inherited-handle evidence is retained in
[`docs/evidence/test-coverage/nextest-flake-leak-20260728.md`](../docs/evidence/test-coverage/nextest-flake-leak-20260728.md).

CI generates locked all-feature instrumentation profiles on the exact verified
head, then exports two native views from the same profiles:

- LLVM JSON, with functions omitted, attests the pinned producer, export
  schema, file identities, segments, and aggregate summaries.
- `llvm-cov show` text supplies the exact execution state of each physical
  source line.

`scripts/llvm_cov_line_map.py` accepts only cargo-llvm-cov 0.8.4 and LLVM
coverage JSON 3.1.0, requires the workspace manifest, rejects unknown fields
and text rows, and compares every displayed source row with the checkout. Its
canonical artifact hashes both producer views and every represented source
file. LLVM's aggregate line-instance summary is retained separately from the
unique physical-line map; disagreement is explicit diagnostic evidence, not a
value to normalize.

Base resolution is event-aware and fail-closed:

- pull requests use exactly one merge base between the PR base tip and head;
- branch pushes and updated-tag pushes use the exact nonzero event `before`
  commit;
- newly created tags use exactly one merge base against the fetched remote
  default branch only when event `before` is all zeroes;
- manual runs require an explicit full base commit SHA.

The JSON evidence preserves raw event inputs, ref type, default-branch
name/ref/SHA when used, requested base, effective base, and every merge base.
An always-run finalizer records six independently named phases—base resolution,
profile generation, JSON export, native text export, line-map conversion, and
diff policy—even when an earlier phase fails. It hashes all three retained
coverage artifacts, checks that the line map refers to the exact JSON and text
bytes, and checks that the diff report refers to the exact line-map bytes.

The policy requires 80% coverage over ordinary changed production lines and
exactly 90% over paths in `diff-coverage-policy.toml`. Its 20 non-overlapping
rules cover lifecycle, protocol parsing, authorization, persistence
arbitration, updater trust, and privacy. Rules must name existing production
files or directories; globs, test-only targets, overlaps, missing targets,
threshold changes, and configurable exclusions fail. A rename into a critical
path materializes the complete target, while a rename out retains its critical
classification through the old path. Base/head runs load each revision's
policy blob from Git, validate it against that same immutable tree, and classify
with the non-overlapping union. Deleting a critical rule in the same change
therefore cannot lower its code from 90% to 80%; exact duplicate rules are
deduplicated and cross-revision overlaps fail closed. Explicit `--diff` mode
rejects a patch that changes the policy because it has no trusted base policy.
Ordinary and critical results, both policy digests, rule policy origins, the
matching rule, and the path match origin are reported independently.

Obvious test-only paths are reported but excluded from the denominator.
Complete inline `#[cfg(test)] mod ... { ... }` ranges in production files are
also reported separately and excluded; the scanner masks comments and Rust
literals before brace matching and fails closed on ambiguous or unclosed
module bodies. Other `cfg(test)` items remain production scope. Comments,
attributes, imports, signatures, and structural punctuation are non-coverable.
Executable-looking changed lines missing from the canonical physical-line map
are unmapped and fail, so a Linux report cannot silently excuse a new
platform-gated body. `scripts/diff_coverage.py --lcov` remains a diagnostic
compatibility mode. It now declares `unique-da-source-lines` as the only
changed-line model and retains contradictory `LF`/`LH` summaries as a separate
structured audit. Malformed or duplicate `DA`, impossible summaries, stale
records, and missing executable mappings still fail closed. `TC-HARNESS-005`
is therefore resolved for Sorotte's consumer without rewriting the
contradictory producer artifact or choosing a favorable aggregate. The
required gate continues to use the stronger source-bound dual-native contract.

The fresh local producer experiment, exact artifact hashes, adversarial cases,
and six-phase result are retained in
[`llvm-native-line-map-20260728.md`](../docs/evidence/test-coverage/llvm-native-line-map-20260728.md).
The LCOV consumer resolution and current-source cross-audit are retained in
[`lcov-dual-model-20260729.md`](../docs/evidence/test-coverage/lcov-dual-model-20260729.md).

## Merged behavioral coverage profiles

The coverage producer does not stop at workspace unit and integration tests.
`scripts/coverage_profile_lanes.py` collects and attests compatible profiles
from:

- the locked all-feature workspace;
- the exact 14-scenario GUI semantic inventory;
- four strict live-TLS tests against pinned Syncplay commit
  `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`;
- a final cargo-llvm-cov merge check.

The wrapper accepts only cargo-llvm-cov 0.8.4, applies its `show-env` contract
to external Cargo processes, isolates those builds in
`target/llvm-cov-target`, removes and attests stale generated raw/merged
profiles before execution, recursively hashes current profiles, requires the
workspace lane to start at zero, and requires a fresh profile delta plus
continuous inventory from every execution lane. Content hashes detect changes
even when size and mtime are unchanged; a lane may not remove prior profiles,
and the merge may not mutate them. The wrapper also validates the semantic
JSON and exact libtest counts, selectors, skip markers, commands, environment,
logs, producer, and pinned reference revision.

The complete strict legacy fanout matrix is not claimed by this green
profile. A real replay passed 129 tests and failed six; those divergences are
tracked as `TC-COMPAT-001` through `TC-COMPAT-006`. Native interactive Windows
profiles also remain a separate evidence boundary. Exact experiments and
limits are retained in
[`merged-profile-lanes-20260729.md`](../docs/evidence/test-coverage/merged-profile-lanes-20260729.md).

## Targeted mutation evidence

The first scheduled mutation shard covers the pure privacy boundary in
`sorotte-secret`. It deliberately does not mutate the whole workspace.
`coverage/mutation-policy.toml` pins cargo-mutants 27.1.0, the package and
literal source file, all-feature locked Cargo execution, two workers,
per-command timeouts, a 100% viable kill requirement, zero missed mutants, and
zero timeouts. Its one compiler-infeasible const mutation is matched by stable
structured identity and has an expiring review date; both a new exception and
a stale exception fail.

Run it locally with:

```text
cargo install cargo-mutants --version 27.1.0 --locked
python scripts/mutation_ci.py validate \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard privacy-secret
python scripts/mutation_ci.py run \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard privacy-secret \
  --results-root target/mutation-ci/privacy-secret \
  --output target/verification/mutation-privacy-secret.json
```

The wrapper disables repository-local cargo-mutants configuration, lists the
inventory before execution, hashes configured sources before and after, and
reconciles every structured outcome with the inventory, status files,
build/test phases, logs, diffs, policy, and producer exit. The weekly workflow
uploads both the compact attestation and raw producer evidence even on
failure.

The initial experiment caught 22 of 43 viable mutants. Seven test-only oracles
then caught all 43 while preserving the identical 44-mutant inventory; one
const-context replacement remained unviable. Commands, timings, hashes,
survivor classification, and limitations are retained in
[`targeted-mutation-20260729.md`](../docs/evidence/test-coverage/targeted-mutation-20260729.md).

Local generation requires both the pinned cargo subcommand and the Rust LLVM
tools component, the legacy Python requirements, and the pinned Syncplay
checkout:

```text
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.8.4 --locked
python -m pip install -r requirements/legacy-python-interop.txt
git clone https://github.com/Syncplay/syncplay.git \
  .interop-cache/syncplay-legacy
git -C .interop-cache/syncplay-legacy checkout \
  d1c5f85af377c960c5a940707c4d01bc84fd9c3f
SYNCPLAY_LEGACY_ROOT=.interop-cache/syncplay-legacy \
python scripts/coverage_profile_lanes.py run \
  --repo-root . \
  --output target/verification/coverage-profile-lanes.json
cargo llvm-cov report --json --skip-functions \
  --output-path target/diff-coverage.json
cargo llvm-cov report --text \
  --output-path target/diff-coverage.txt
python scripts/llvm_cov_line_map.py \
  --repo-root . \
  --llvm-json target/diff-coverage.json \
  --llvm-text target/diff-coverage.txt \
  --output target/verification/coverage-line-map.json
```

`cargo llvm-cov` prompts interactively when the LLVM component is missing;
captured or headless runs can therefore appear hung unless the component is
provisioned first. CI installs it explicitly.

`scripts/gui-native-smoke.ps1` now treats the complete native inventory as required
by default, prebuilds the GUI and native harness, binds the report to the GUI
path and SHA-256, preserves raw output and producer exit state, rejects skips,
duplicate JSON keys, unexpected stderr, and binary mutation, and kills a hung
process tree on a derived wall-clock deadline. This remains a trusted
interactive-Windows lane: a hosted noninteractive runner must not be counted
as equivalent evidence.

The native bundle retains screenshots, redacted UI Automation trees, isolated
configuration, structured capability outcomes, invocation identity, process
exit, and scenario logs. Loopback-only fixture policy plus stderr rejection
catches the networking failures observed in this work; OS-level network
isolation is still required before claiming that silent outbound traffic is
impossible.

Known product findings deliberately left unfixed by this coverage branch are
tracked in [`docs/TEST_COVERAGE_FINDINGS.md`](../docs/TEST_COVERAGE_FINDINGS.md).
