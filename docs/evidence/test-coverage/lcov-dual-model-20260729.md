# LCOV dual-model consumer proof

Date: 2026-07-29

Finding: `TC-HARNESS-005`

## Question

Can Sorotte consume cargo-llvm-cov LCOV safely when `LF`/`LH` summaries
contradict the unique `DA` inventory, without hiding missing executable lines
or pretending the producer contradiction is resolved?

## Consumer contract

`scripts/diff_coverage.py --lcov` now names
`unique-da-source-lines` as its only changed-line model. It retains declared
`LF`/`LH` values in a separate structured audit containing:

- total, repository, and ignored-external record counts;
- records with any, `LF`, or `LH` mismatch;
- separate aggregate declared and unique-`DA` counts;
- every mismatched record's source, fields, and both count pairs.

Contradictory summaries are evidence, not line mappings. Duplicate or malformed
`DA`, impossible `LH > LF`, unsupported directives, stale or out-of-range
sources, and duplicate source records remain input errors. An executable
changed line absent from `DA` remains unmapped and fails policy even if
declared `LF` claims more lines.

The required CI path remains the stronger source-bound LLVM JSON plus native
text map. LCOV remains diagnostic because its records do not bind themselves
to exact source bytes.

## Current-source producer run

```text
cargo llvm-cov --locked --workspace --all-features --lcov \
  --output-path target/tc-harness-005-fixed.lcov
```

Result:

```text
instrumented workspace: passed
elapsed:                235.1 seconds
artifact bytes:         15,369,296
SHA-256:                1998ea2b60336018b796c5e2a6e14cd6cc58ac36377f6914993b86c18bd136bf
```

The repaired parser reported:

```text
source records:                     395
records with any LF/LH mismatch:    310
records with an LF mismatch:        308
records with an LH mismatch:        259
declared LH/LF:        148,045 / 190,067
positive/unique DA:    144,853 / 183,712
```

An independent PowerShell scanner over the same bytes matched every aggregate
exactly. It also matched the stable `parser.rs` contradiction:

```text
declared LF/LH: 122 / 75
unique DA:      120
positive DA:    115
```

## End-to-end fail-closed replay

The exact current Rust diff was evaluated with:

```text
python scripts/diff_coverage.py \
  --repo-root . \
  --lcov target/tc-harness-005-fixed.lcov \
  --diff target/tc-harness-005-current-rust.diff \
  --minimum 80 \
  --json-out target/tc-harness-005-policy-report.json
```

The parser reached policy evaluation despite 310 producer-summary
contradictions. The policy correctly failed on real evidence:

```text
DA-covered changed lines: 761 / 1,827 = 41.65%
lexical non-coverable:    323
unmapped executable:      126
ordinary result:          failed
critical result:          failed
```

The report binds the LCOV SHA-256, declares
`coverage_kind = legacy-lcov-da-diagnostic`, declares the exact line model,
and retains all mismatch records. This proves that summary contradictions no
longer make the diagnostic artifact unusable while neither a favorable
summary nor a missing `DA` entry can green the policy.

## Adversarial suite

```text
python -m unittest scripts.tests.test_diff_coverage
71 passed; 2.876 seconds
```

New cases prove dual-model preservation, internally impossible summary
rejection, that declared `LF` cannot invent coverage for an absent executable
`DA` line, and that the CLI prints a producer-mismatch warning even when the
unique-`DA` changed-line policy passes.
