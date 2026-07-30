# Test-coverage continuation handoff — 2026-07-30

## Completion note

This handoff is a historical checkpoint. Its tranche-local instruction not to
repair discovered product behavior was superseded by the user's explicit
request to fix the outstanding defects. `TC-CLI-003` was corrected in
`b938561`, `TC-PROTOCOL-004` was corrected in `034e105`, all four
characterizations are positive regressions, and the registry is empty.
Current proof is in
[`outstanding-defect-resolution-20260730.md`](outstanding-defect-resolution-20260730.md).

## Purpose and safety context

This handoff resumes a defensive software-quality review of the Sorotte Rust
application. The coverage-guided work is a bounded local test of Sorotte's own
JSON protocol parser:

- it invokes only public functions in `sorotte-protocol`;
- it accepts at most 64 KiB of local generated input;
- it performs no network access, reconnaissance, credential work, persistence,
  privilege change, or interaction with a third-party target;
- AddressSanitizer and libFuzzer are being used only to find application
  crashes and violated parser invariants; and
- any product defect found by this tranche must be characterized and recorded,
  not repaired.

Automated safety classification interrupted the delegated parser-fuzz slice
twice despite that scope. There is no offensive-security objective in this
work.

## Repository coordinates

Continue in this exact isolated worktree:

```text
C:\tmp\sorotte-test-coverage-design
```

Do not continue in the ordinary checkout at
`C:\Users\shaun\Documents\workspace\sorotte`; the uncommitted implementation
exists only in the isolated worktree.

Repository state when this handoff was written:

```text
branch:       codex/test-coverage-design
HEAD:         0748e4a8f07bad4ab30b26b22535ec969c3b10cf
remote HEAD:  0748e4a8f07bad4ab30b26b22535ec969c3b10cf
worktree:     dirty, intentionally uncommitted
staged files: none
```

The branch and remote were identical before this tranche. Preserve all dirty
files: they are the current test implementation, not disposable output.

## Governing instructions

1. Continue the main test-coverage plan ambitiously.
2. If a test surfaces an application defect, do not fix production behavior in
   this tranche. Add a narrow executable characterization, register it, retain
   the evidence, and continue around only the exact known defect class.
3. Harness, test-policy, and evidence-integrity defects may be repaired because
   they determine whether the testing strategy is trustworthy.
4. Use `apply_patch` for source and documentation edits.
5. Do not stage anything under `target/` or `fuzz/target/`.
6. No GUI behavior changed in these four slices, so GUI semantic/native suites
   are not required unless later work expands into GUI code.
7. Finish with focused validation, full workspace Clippy/tests, intentional
   commits, and a push of `codex/test-coverage-design`.

## Four delegated slices

Four distinct delegated work packages were dispatched. Three completed
independently. The coverage-guided parser lane was interrupted by automated
safety false positives and was continued by the root agent.

| Slice | State | Outcome |
| --- | --- | --- |
| Server persistence arbitration mutation | implemented and independently evidenced | 25/25 viable mutants caught; 2 exact compiler-unviable identities; no product defect |
| Client inbound `Set` ordering mutation | implemented and independently evidenced | 5/5 viable mutants caught; no exceptions; no product defect |
| Configuration composition properties | implemented and independently evidenced | 30 supported fields; 6,144 scheduled cases and 30,000 stress cases; no product defect |
| Coverage-guided protocol parsing | implementation and first experiments complete; evidence/integration unfinished | found and registered `TC-PROTOCOL-004`; 559,788-execution continuation passed |

## Slice 1: server persistence arbitration mutation ratchet

### Files

```text
crates/sorotte-server/src/persistence_actor.rs
crates/sorotte-server/src/persistence_actor/persistence_arbitration.rs
crates/sorotte-server/src/persistence_actor/persistence_arbitration_tests.rs
coverage/mutation-policy.toml
.github/workflows/rust-mutation.yml
scripts/tests/test_ci_policy.py
docs/evidence/test-coverage/targeted-mutation-server-persistence-arbitration-20260730.md
```

### Design

The existing desired-state/version/coalescing/retry/recovery decisions were
moved without behavior changes into a private internal arbitration module.
SQLite operations, worker orchestration, events, and reporting remain in
`persistence_actor.rs`. Tests are outside the mutated source file.

### Proof

- unchanged-source baseline: 3/25 viable caught, 22 missed;
- final: 25/25 viable caught, 0 missed, 0 timeouts;
- 2 compiler-unviable mutations are exact, source-bound, expiring policy
  entries;
- target source before/after SHA-256:
  `a7348d3346906a494cfb7dfe40b1c377c66248189b705f4a4d327db0c20ba406`;
- focused tests: 7/7;
- stress: 350/350 test executions;
- existing room worker suite: 9/9;
- full server package: 365 library, 14 binary-unit, 2 binary-integration, and
  6 release-verification tests passed;
- checked-in-policy report: 17,558 bytes, SHA-256
  `840a478b8146fdff35e7a763479b22137644f29c8578c676876978003a825ff8`;
- strict server Clippy, formatting, and diff checks passed.

The central policy now contains the
`server-persistence-arbitration` shard and the two exact accepted-unviable
identities.

## Slice 2: client inbound `Set` ordering mutation ratchet

### Files

```text
crates/sorotte-client-core/src/inbound_order.rs
crates/sorotte-client-core/src/inbound.rs
crates/sorotte-client-core/src/lib.rs
crates/sorotte-client-core/src/session/tests/protocol_tests.rs
coverage/mutation-policy.toml
.github/workflows/rust-mutation.yml
scripts/tests/test_ci_policy.py
docs/evidence/test-coverage/targeted-mutation-client-inbound-order-20260730.md
```

### Design

The existing 26-line ordering helper was moved verbatim into a private module
so it can have a bounded mutation shard. Three independent tests encode:

- partial wire order followed by missing canonical commands;
- unknown commands retaining their exact positions;
- canonical completion without wire metadata;
- no duplicate canonical commands;
- exact preservation of a complete noncanonical permutation; and
- the unchanged full `SetPayload` paired with every ordered command.

### Proof

- baseline: 4/5 viable mutants caught;
- final: 5/5 caught, no misses, timeouts, or exceptions;
- target source before/after SHA-256:
  `50eb172d41813a9ffa8958a10d138f591db593341922d909a9a5010ab755b0db`;
- focused selector: 38/38;
- stress: 150/150 new-test executions;
- full client-core package: 718/718;
- checked-in-policy report: 9,245 bytes, SHA-256
  `86ef4313a996e21dac3d726324752a569573432dd1be46d4034491086dc313cf`;
- strict client-core Clippy, formatting, and diff checks passed.

The central policy now contains the `client-inbound-order` shard with no
exception.

## Slice 3: configuration composition properties

### Files

```text
crates/sorotte-client-app/Cargo.toml
Cargo.lock
crates/sorotte-client-app/tests/configuration_composition_properties.rs
docs/evidence/test-coverage/configuration-composition-properties-20260730.md
```

### Design

This is a black-box integration suite through the public client-app boundary:

```text
generated stored DTO
  -> INI upsert
  -> INI parse
  -> runtime snapshot
  -> environment-aware startup configuration plan
```

An independent model covers all 30 environment-overridable fields. The three
properties prove roundtrip/idempotence, forward-compatible unknown-content
preservation, single-field noninterference, and exact environment suppression.
The fixed seed is `0xC0F1_6C0A_2026_0730`.

### Proof

- default: 1,536 generated cases, 4/4 tests, 0.19 seconds;
- scheduled: 6,144 generated cases, 4/4 tests, 0.75 seconds;
- stress: 30,000 generated cases, 4/4 tests, 3.80 seconds;
- live zero and malformed case budgets failed closed;
- full client-app package: 185 library plus 4 integration tests;
- strict client-app Clippy, formatting, and diff checks passed;
- no production source changed and no product defect surfaced.

## Slice 4: coverage-guided protocol parser

### Source and policy files

```text
.github/workflows/rust-fuzz.yml
fuzz/.gitignore
fuzz/Cargo.toml
fuzz/Cargo.lock
fuzz/fuzz_targets/protocol_line.rs
fuzz/run_protocol_fuzz.py
scripts/tests/test_protocol_fuzz_policy.py
crates/sorotte-protocol/src/tests.rs
coverage/known-defects.toml
scripts/known_defect_policy.py
scripts/tests/test_known_defect_policy.py
docs/TEST_COVERAGE_FINDINGS.md
```

The final evidence document does not yet exist:

```text
docs/evidence/test-coverage/protocol-coverage-guided-20260730.md
```

### Harness contract

- standalone `cargo-fuzz` package;
- `cargo-fuzz 0.13.2`;
- `libfuzzer-sys 0.4.13`;
- dated toolchain `nightly-2026-07-29`;
- AddressSanitizer;
- one byte-oriented target;
- maximum input: 65,536 bytes;
- per-input timeout: 5 seconds;
- RSS limit: 2,048 MiB;
- campaign cap: 900 seconds;
- all public raw, diagnostic, aggregate, singular, and typed protocol
  decoder/encoder entrypoints;
- an independent serde `MapAccess` source-order oracle;
- duplicate-command surviving-value checks;
- typed message roundtrip checks;
- source, seed, corpus, artifact, tool, command, and final-statistics
  attestations.

The workflow is path-filtered on pull requests, runs only on `main` pushes to
avoid duplicate feature-branch work, supports manual dispatch, and schedules a
weekly 900-second campaign. PR/push campaigns use 45 seconds. Actions and
fuzz dependencies are exact-pinned; evidence uploads even on failure.

### Portability hardening already completed

The first WSL attempt could not follow a Windows worktree `.git` pointer.
`run_protocol_fuzz.py` now requires:

```text
--source-sha <exact 40-character lowercase hexadecimal SHA>
```

CI passes `${{ github.sha }}`. The runner validates the value before writing
evidence. This removes local Git metadata traversal from the WSL execution
path.

WSL non-login execution also lacked cargo. Local commands must use
`bash -lc`, as shown below.

### Newly surfaced product defect

`TC-PROTOCOL-004: Protocol floating-point values can drift across decode and
re-encode` is open and deliberately unfixed.

The first real campaign minimized the input to:

```text
70E70
```

Observed raw boundary:

```text
before: 7.000000000000001e71
after:  7.000000000000002e71
```

The same one-ULP change reproduces in a valid typed protocol message:

```json
{"State":{"playstate":{"position":70E70}}}
```

Two ordinary `#[should_panic(expected = "...")]` characterizations are in
`crates/sorotte-protocol/src/tests.rs`:

```text
tests::known_defect_tc_protocol_004_raw_floating_point_roundtrip_is_exact
tests::known_defect_tc_protocol_004_typed_state_floating_point_roundtrip_is_exact
```

Both panic with:

```text
TC-PROTOCOL-004: protocol floating-point value changed across decode/encode/decode
```

Do not enable a serde feature, reject the number, clamp it, or otherwise repair
production behavior in this tranche.

The continuing fuzz target admits only the registered class: the JSON
structure must be identical, every non-floating leaf must remain exact, and a
changed finite float must retain its sign and differ by exactly one ULP.
Structural, integer, sign, non-finite, or larger numeric drift still fails.

### Defect-policy hardening already completed

While checking TOML table ordering, the validator was proven to accept a
characterization whose panic text named a different defect. This is a
test-infrastructure defect, not product behavior. It is fixed:

```text
each expected_panic must start with "<its own defect id>: "
```

The new regression and all existing policy tests pass. The current real
registry validates as:

```text
2 defects, 4 characterizations
```

The other open defect is the pre-existing, unchanged `TC-CLI-003` fragmented
connected-session read-cancellation defect with two characterizations.

## Coverage-guided experiment inventory

All outputs are under ignored `target/fuzz-ci/` and must not be committed.

| Output | Result | Meaning |
| --- | --- | --- |
| `protocol-line-smoke` | `setup_failed` | WSL could not follow the Windows worktree Git pointer; fixed by explicit source SHA |
| `protocol-line-smoke-v2` | `setup_failed` | cargo absent from non-login WSL PATH; fixed by `bash -lc` |
| `protocol-line-smoke-v3` | `failed` | genuine `TC-PROTOCOL-004` counterexample |
| `protocol-line-smoke-v4` | `passed` | independent continuation with the exact one-ULP classifier |

### Counterexample run (`protocol-line-smoke-v3`)

- executed units: 108,863;
- average executions/second: 9,896;
- new units: 1,385;
- peak RSS: 452 MiB;
- final corpus: 683 files;
- artifacts: 1;
- source bindings stable: yes;
- seed source stable: yes;
- bound-source aggregate:
  `60b759af3b67cff6df1bcd86206b6b6d351819b9f4c89092b13b488e772b65b3`;
- report SHA-256:
  `8de3aae9bbe47873e767819380f0fa449db7aff06bb42c334394093e15c53b71`;
- log SHA-256:
  `1b8d6a483d0fea2fde960ade4f267cd2b23ce5dcd24f63d1891213c9922ea486`;
- minimized five-byte artifact SHA-256:
  `ccabbcc5ab3f05fab297b4d429f24fe96753ea6c63545bc547832d8ff202bf2e`.

### Continuation run (`protocol-line-smoke-v4`)

- status: passed;
- fuzzer exit: 0;
- executed units: 559,788;
- average executions/second: 12,169;
- new units: 3,592;
- peak RSS: 485 MiB;
- final corpus: 1,201 files / 90,624 bytes;
- artifacts: 0;
- source bindings stable: yes;
- seed source stable: yes;
- bound-source files: 23 / 275,428 bytes;
- bound-source aggregate:
  `acb488f2c613f9204c792908bc2b4d4cafd6870ebad7abb5ef3a85bd2b1a31b0`;
- final corpus aggregate:
  `459117ca97036f26c9d79902a232ce9e41f3df1b07c94c42d3089f7ca228c750`;
- report SHA-256:
  `0f7fd3fe21bd07f1536b876d7387e7c32708422caaa9ab39f18571b17641abe2`;
- log SHA-256:
  `654b947170af4c258ec73e0ad314fe0f4706d4ce99f75bae18fba5dab9015c78`.

Exact tool identities in the successful report:

```text
cargo-fuzz 0.13.2
rustc 1.99.0-nightly (26ae60a9e 2026-07-28)
rustc commit 26ae60a9eeb20b4935be49d7a931a650fa1d2923
cargo 1.99.0-nightly (3efb1f477 2026-07-17)
LLVM 22.1.8
Python 3.12.3
Linux 6.6.87.2-microsoft-standard-WSL2 x86_64
```

## Validation already completed in the root integration pass

| Check | Result |
| --- | --- |
| `python -m unittest scripts.tests.test_mutation_ci scripts.tests.test_ci_policy -v` | 50/50 passed before the later fuzz-policy edits |
| shared mutation policy validation | 8 shards, 16 accepted-unviable identities |
| checked-in persistence mutation replay | 25/25 viable caught, 2 exact unviable |
| checked-in inbound-order mutation replay | 5/5 viable caught |
| `actionlint .github/workflows/rust-mutation.yml` | passed earlier in the tranche |
| combined known-defect and fuzz-policy tests | 34/34 passed after latest hardening |
| real known-defect registry | 2 defects, 4 characterizations |
| exact `TC-PROTOCOL-004` tests | 2/2 passed as expected-failure characterizations |
| `cargo fmt --all -- --check` | passed after the latest Rust edits |
| `git diff --check` | passed after the latest edits |
| 45-second ASan continuation | 559,788 executions, no independent failure |

`actionlint` is not currently discoverable in this PowerShell process
(`Get-Command actionlint` and `where.exe actionlint` returned nothing). The
fuzz workflow passed actionlint before the explicit `--source-sha` argument was
added, so it must be linted again from the installation/path available to the
new task.

## Important provenance hardening before the canonical run

`BOUND_FIXED_SOURCE_PATHS` in `fuzz/run_protocol_fuzz.py` currently binds the
root/toolchain/protocol/fuzz manifests, lockfile, runner, target, and every
protocol Rust source. Before calling the slice complete, add these policy
inputs to that tuple:

```text
.github/workflows/rust-fuzz.yml
coverage/known-defects.toml
scripts/known_defect_policy.py
scripts/tests/test_known_defect_policy.py
scripts/tests/test_protocol_fuzz_policy.py
```

The registered one-ULP allowance depends on those files. Binding them prevents
a campaign report from remaining apparently stable if its workflow,
registration, or fail-closed policy changes during execution. Consider
binding `coverage/behaviors.toml` as well because it supplies the positive
proof catalog to known-defect validation.

The existing policy test computes its expected inventory from
`BOUND_FIXED_SOURCE_PATHS`, so the change should require little or no test
rewriting. Re-run the policy tests and use a fresh output directory after any
binding change.

Local reports use the uncommitted branch base SHA plus an exact before/after
file manifest. For final canonical evidence, prefer:

1. validate and commit the implementation sources;
2. run the fuzzer on that clean implementation commit using its actual HEAD
   SHA;
3. add the generated statistics and hashes to the evidence document; and
4. commit the documentation separately.

That avoids presenting the base commit SHA as though it alone identified dirty
source; the aggregate file manifest remains the exact binding for the earlier
experimental runs.

## Ordered continuation plan

### 1. Orient without changing state

```powershell
Set-Location C:\tmp\sorotte-test-coverage-design
git status --short --branch
git rev-parse HEAD
git rev-parse origin/codex/test-coverage-design
```

Expected branch/head:

```text
codex/test-coverage-design
0748e4a8f07bad4ab30b26b22535ec969c3b10cf
```

### 2. Apply the provenance hardening

Expand `BOUND_FIXED_SOURCE_PATHS` as described above. Run:

```powershell
python -m unittest scripts.tests.test_protocol_fuzz_policy `
  scripts.tests.test_known_defect_policy -v

python scripts/known_defect_policy.py validate `
  --registry coverage/known-defects.toml `
  --repo-root . `
  --catalog coverage/behaviors.toml
```

Expected registry result:

```text
2 defects, 4 characterizations
```

### 3. Run a longer independent continuation

Use a fresh output directory every time; the runner rejects stale evidence.
Until the implementation is committed, the SHA below is the branch base and
the report's file manifest is the exact dirty-source identity:

```powershell
wsl.exe -d Ubuntu `
  --cd /mnt/c/tmp/sorotte-test-coverage-design `
  bash -lc "python3 fuzz/run_protocol_fuzz.py --toolchain nightly-2026-07-29 --source-sha 0748e4a8f07bad4ab30b26b22535ec969c3b10cf --seconds 180 --seed-corpus crates/sorotte-protocol/tests/corpus/protocol_parser --expected-seed-count 14 --output-root target/fuzz-ci/protocol-line-deep-v1"
```

If it finds another failure:

- do not alter production protocol behavior;
- retain the report, log, artifact, and minimized artifact;
- determine whether it is `TC-PROTOCOL-004`, a harness defect, or an
  independent product defect;
- add a new registered characterization only for an independent product
  defect;
- narrow any continuation allowance to the exact registered class.

### 4. Write the missing fuzz evidence

Create:

```text
docs/evidence/test-coverage/protocol-coverage-guided-20260730.md
```

It must include:

- safety/scope statement;
- target and oracle design;
- exact pins and resource limits;
- workflow trigger/security contract;
- all four current experiment attempts;
- the `TC-PROTOCOL-004` raw and typed reproductions;
- the exact continuation classifier;
- runner portability/provenance hardening;
- tool versions and rustc commit;
- report/log/artifact/source/corpus hashes;
- policy/known-defect validation;
- focused protocol tests and Clippy;
- the longer continuation statistics;
- explicit limitations: one parser target is not transport/session fuzzing,
  OS durability, real-player, or native-GUI coverage.

### 5. Update central strategy documents

The following integration is still missing:

```text
coverage/README.md
docs/TEST_COVERAGE_STRATEGY.md
docs/TEST_COVERAGE_FINDINGS.md
```

`docs/TEST_COVERAGE_FINDINGS.md` already contains the new
`TC-PROTOCOL-004` heading, but its opening summary still says one defect and
two characterizations. Update current-state statements to:

```text
2 open defects, 4 exact characterizations
```

Also integrate:

- 8 scheduled mutation shards;
- 425 viable scheduled mutations caught in total;
- 0 misses and 0 timeouts;
- 16 exact accepted compiler-unviable identities;
- the persistence arbitration ratchet;
- the inbound ordering ratchet;
- the 30-field configuration composition properties;
- the true coverage-guided protocol parser lane;
- the discovered and deliberately unfixed `TC-PROTOCOL-004`;
- evidence links for all four slices.

Do not rewrite historical checkpoint counts; update only statements that claim
to describe the current registry or current policy.

### 6. Focused validation

At minimum:

```powershell
python -m unittest scripts.tests.test_mutation_ci `
  scripts.tests.test_ci_policy `
  scripts.tests.test_known_defect_policy `
  scripts.tests.test_protocol_fuzz_policy -v

python scripts/mutation_ci.py validate `
  --repo-root . `
  --policy coverage/mutation-policy.toml

python scripts/known_defect_policy.py validate `
  --registry coverage/known-defects.toml `
  --repo-root . `
  --catalog coverage/behaviors.toml

cargo test --locked -p sorotte-protocol --all-features
cargo clippy --locked -p sorotte-protocol --all-targets --all-features -- -D warnings

$env:PROPTEST_CASES = "2048"
cargo test --locked -p sorotte-client-app `
  --test configuration_composition_properties -- --nocapture
Remove-Item Env:PROPTEST_CASES

cargo test --locked -p sorotte-client-core `
  session::tests::protocol_tests:: --all-features

cargo test --locked -p sorotte-server `
  persistence_actor::persistence_arbitration_tests:: --all-features
```

Re-run actionlint on both changed workflows from the installed binary:

```text
.github/workflows/rust-fuzz.yml
.github/workflows/rust-mutation.yml
```

### 7. Full repository gate

```powershell
cargo fmt --all -- --check
git diff --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
python -m unittest discover -s scripts/tests -p "test_*.py" -v
```

If a full-suite failure appears timing-related, isolate and retry it before
changing code. Do not turn a flake into a speculative product change.

### 8. Commit and push

No files are staged or committed yet. Keep commits focused. One reasonable
layout is:

1. `Add persistence arbitration mutation ratchet`
2. `Add client inbound ordering mutation ratchet`
3. `Add configuration composition properties`
4. `Add coverage-guided protocol parser testing`
5. `Update test coverage strategy`

The shared mutation workflow/policy can be included with the two mutation
slices or in one focused policy integration commit. The known-defect registry,
protocol characterizations, fuzz workflow, runner, target, and policy
hardening belong together.

Before staging, use:

```powershell
git status --short --untracked-files=all
```

Confirm that no files under these paths are staged:

```text
target/
fuzz/target/
fuzz/__pycache__/
```

Then push:

```powershell
git push origin codex/test-coverage-design
```

Verify local and remote branch heads match after the push.

## Current dirty source/evidence inventory

Modified tracked files:

```text
.github/workflows/rust-mutation.yml
Cargo.lock
coverage/known-defects.toml
coverage/mutation-policy.toml
crates/sorotte-client-app/Cargo.toml
crates/sorotte-client-core/src/inbound.rs
crates/sorotte-client-core/src/lib.rs
crates/sorotte-client-core/src/session/tests/protocol_tests.rs
crates/sorotte-protocol/src/tests.rs
crates/sorotte-server/src/persistence_actor.rs
docs/TEST_COVERAGE_FINDINGS.md
scripts/known_defect_policy.py
scripts/tests/test_ci_policy.py
scripts/tests/test_known_defect_policy.py
```

Untracked source/evidence files:

```text
.github/workflows/rust-fuzz.yml
crates/sorotte-client-app/tests/configuration_composition_properties.rs
crates/sorotte-client-core/src/inbound_order.rs
crates/sorotte-server/src/persistence_actor/persistence_arbitration.rs
crates/sorotte-server/src/persistence_actor/persistence_arbitration_tests.rs
docs/evidence/test-coverage/configuration-composition-properties-20260730.md
docs/evidence/test-coverage/targeted-mutation-client-inbound-order-20260730.md
docs/evidence/test-coverage/targeted-mutation-server-persistence-arbitration-20260730.md
fuzz/.gitignore
fuzz/Cargo.lock
fuzz/Cargo.toml
fuzz/fuzz_targets/protocol_line.rs
fuzz/run_protocol_fuzz.py
scripts/tests/test_protocol_fuzz_policy.py
```

This handoff file is also untracked until committed.

## Suggested opening message for the new task

```text
Continue the Sorotte test-coverage implementation from
C:\tmp\sorotte-test-coverage-design\docs\evidence\test-coverage\test-slices-handoff-20260730.md.
Read the entire handoff first and work only in
C:\tmp\sorotte-test-coverage-design on codex/test-coverage-design. This is
bounded defensive QA of our own local Rust JSON parser, not offensive security
work: no network target, reconnaissance, credentials, or exploitation is in
scope. Preserve every dirty file. Do not fix the open product defects
TC-CLI-003 or TC-PROTOCOL-004; characterize any independent defect and
continue. Finish the provenance binding, longer fuzz run, evidence and central
docs, full validation, focused commits, and push.
```
