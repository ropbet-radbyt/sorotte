# LLVM native physical-line coverage experiment — 2026-07-28

> Follow-up, 2026-07-29: this document preserves the original native-map
> experiment. The required source-bound contract remains unchanged. The
> diagnostic LCOV consumer now uses explicit unique-`DA` line semantics while
> retaining `LF`/`LH` contradictions as audit evidence; see
> [`lcov-dual-model-20260729.md`](lcov-dual-model-20260729.md).

## Question

Can Sorotte enforce coverage of physical lines added by a Git diff without
accepting cargo-llvm-cov's contradictory LCOV `LF`/`LH` and `DA` models?

The experiment does not attempt to repair or normalize LCOV. It tests a
different, explicit producer contract built from two native LLVM views emitted
from the same instrumentation profiles.

## Environment

```text
base Git commit: f3964ebc7f7b281b9b78f3bfb243ff65e5122e33
branch: codex/test-coverage-design
host: Windows
Rust: 1.97.1-x86_64-pc-windows-msvc
cargo-llvm-cov: 0.8.4
LLVM coverage JSON schema: 3.1.0
date: 2026-07-28 Australia/Sydney
```

The branch contained the uncommitted test-coverage implementation. The
canonical artifact binds that exact source state independently of Git by
recording a SHA-256 for all 392 represented Rust files and comparing every
native source-view row with the checkout. CI will run the same contract on a
committed exact head.

## Commands

```powershell
cargo llvm-cov --locked --workspace --all-features --no-report

cargo llvm-cov report --json --skip-functions `
  --output-path target/current-diff-coverage.json

cargo llvm-cov report --text `
  --output-path target/current-diff-coverage.txt

python scripts/llvm_cov_line_map.py `
  --repo-root . `
  --llvm-json target/current-diff-coverage.json `
  --llvm-text target/current-diff-coverage.txt `
  --output target/verification/current-coverage-line-map.json

git diff --output=target/current-rust.diff -- crates

python scripts/diff_coverage.py `
  --repo-root . `
  --coverage-map target/verification/current-coverage-line-map.json `
  --critical-policy coverage/diff-coverage-policy.toml `
  --diff target/current-rust.diff `
  --minimum 80 `
  --json-out target/verification/current-fresh-diff-coverage.json

python scripts/coverage_ci_guard.py finalize `
  --base-outcome success `
  --profiles-outcome success `
  --llvm-json-outcome success `
  --llvm-text-outcome success `
  --line-map-outcome success `
  --policy-outcome success `
  --base-report target/verification/current-coverage-base.json `
  --llvm-json target/current-diff-coverage.json `
  --llvm-text target/current-diff-coverage.txt `
  --line-map target/verification/current-coverage-line-map.json `
  --policy-report target/verification/current-fresh-diff-coverage.json `
  --output target/verification/current-coverage-ci-phases.json
```

## Results

The fresh instrumented workspace completed successfully in 250.9 seconds.
Both reports exported from the resulting profiles in 2.7 seconds when run in
parallel. Strict conversion completed in 1.4 seconds; changed-line analysis
completed in 1.3 seconds; final cross-artifact validation completed in 0.5
seconds.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| LLVM JSON | 14,043,267 | `e01b2c38ea017c16d6b29494ce6496099552fa01428b446ec1058b2c0693f104` |
| LLVM native text | 13,643,322 | `25ce18c6aed4d7d2c238e1db6303c08e76288b33a03aacacecded54bd55a900c` |
| canonical physical-line map | 8,958,361 | `74b428fe3688ed3b147648f0dd3db21f0f9ccdaa6344e90d64143b503cb5b541` |
| changed-line policy report | 158,747 | `2822429ca123669744088d42905511b98b8bbf9ebb5193a0e6bc28ec59b02569` |
| compact six-phase report | 194,894 | `06c796d2598945f4b39eddd9c953704d0c6f35947c02c6cf3b6014b346290da0` |

The two native measurements are deliberately retained as different models:

| Model | Covered | Total | Percent |
|---|---:|---:|---:|
| unique physical source lines from `llvm-cov show` | 145,272 | 183,106 | 79.337651% |
| LLVM JSON aggregate line instances | 152,964 | 195,568 | 78.215250% |
| explicit LLVM-minus-physical delta | +7,692 | +12,462 | n/a |

The tracked Rust diff result was:

```text
coverable changed lines: 32
covered changed lines: 32
non-coverable structural lines: 14
unmapped changed lines: 0
ordinary class: not applicable
critical class: passed at 100.00% against the fixed 90.00% ratchet
overall: passed
```

The finalizer passed all six phases and verified these links:

```text
LLVM JSON bytes ─┐
                 ├─ canonical map input digests
LLVM text bytes ─┘

canonical map bytes ─ changed-line report input digest

base + profiles + JSON + text + map + policy ─ one ordered phase report
```

## Independent negative evidence

The historical LCOV parser and its tests were not weakened. Both preserved
LCOV artifacts still fail on their `LF`/`LH` versus unique `DA` contradictions.
The new map does not consume LCOV at all.

The converter and consumer are exercised by the complete 220-test Python
infrastructure suite. The new focused adversarial inventory includes:

| Boundary | Focused tests | Examples |
|---|---:|---|
| native converter | 14 | duplicate JSON keys, unknown fields, tool/schema/manifest drift, source/header mismatch, malformed segments, nonempty branch data, row ordering, separators, CRLF, tabs, invalid count tokens |
| canonical map consumer | 9 | stale source digest/count, producer drift, duplicate/map schema fields, nonbinary/duplicate/out-of-range lines, summary/delta mutation, dual-input ambiguity |
| six-phase finalizer | 19 total | missing/skipped producer phase, raw-artifact mutation, duplicate JSON keys, failed converter report, line-map digest mismatch, policy-to-map digest mismatch |
| legacy diff/LCOV policy | 68 | contradictory LCOV summary rejection remains covered, plus immutable base/head policy and Git/source binding |
| parsed workflow policy | 11 total | exact pinned profile/JSON/text/map/policy/finalizer commands and artifact set |

All 220 tests passed in 9.726 seconds. The real fresh canonical artifact also
passed the strengthened parser after the adversarial suite was added and
reproduced the same SHA-256, proving deterministic conversion for identical
inputs.

## What this proves

- The all-feature workspace can generate fresh profiles under the pinned
  toolchain.
- Native JSON and source views from those profiles can be bound to each other
  and to the exact checkout.
- A physical Git diff can be evaluated against unique physical source-line
  execution without interpreting contradictory LCOV summaries.
- Producer, converter, policy, and phase artifacts fail independently and are
  retained with cross-digests.
- Aggregate LLVM line-instance semantics remain visible rather than being
  silently replaced by the physical model.

## What this does not prove

- It does not fix cargo-llvm-cov's LCOV output or make the historical LCOV
  artifacts valid for strict generic consumers.
- It does not merge semantic GUI, native GUI, live compatibility, Windows-only,
  or real external-player profiles into the Linux pull-request denominator.
- It does not make 79.337651% aggregate physical coverage a product-quality
  guarantee; merge policy is based on changed production lines and independent
  behavior proofs.
- The local explicit-diff experiment is not a substitute for a hosted
  pull-request run. Base/head behavior is covered mechanically, and CI must
  still execute it against the event-derived immutable revisions.
- It does not fix any application defect discovered by the larger coverage
  effort.

## Primary format references

- [LLVM `llvm-cov` command guide](https://www.llvm.org/docs/CommandGuide/llvm-cov.html)
- [cargo-llvm-cov documentation](https://github.com/taiki-e/cargo-llvm-cov)
- [LLVM issue 126307: exact covered line identities in JSON](https://github.com/llvm/llvm-project/issues/126307)
