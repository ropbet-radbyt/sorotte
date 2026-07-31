# Cross-platform changed-line coverage-map union — 2026-07-31

## Outcome

Commit `829ab9824d20bc64b03179646c5e182d5c7a4bfb` corrects the
required changed-line gate without lowering either ratchet. The gate still
requires 80% for ordinary changed production Rust and 90% for the 20 critical
paths, but it now evaluates the union of source-bound Linux and Windows LLVM
physical-line maps. Every map is validated independently against the exact
checkout bytes before the line hit counts are combined by physical source
line.

The exact local replay passed:

```text
overall:  2,285 / 2,769 = 82.52%
ordinary: 1,723 / 2,150 = 80.13%
critical:   562 /   619 = 90.79%
unmapped: 0
```

This is test-harness and CI-policy work. It changes no Sorotte product
behavior, coverage threshold, critical-path rule, production timeout, or
compatibility equivalence.

## Retained hosted diagnostic

Workflow run
[`30627601938`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30627601938)
executed exact head `9f3cb60fbe788575829931b56155f4bc0c19caf0`. Every
originating required job except coverage passed, including Linux and Windows
all-feature behavior, complete live compatibility, GUI semantic and lifecycle
evidence, generated Media Match, real-mpv semantics, and both server-release
jobs. Coverage job
[`91147825269`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30627601938/job/91147825269)
generated and uploaded all artifacts, then failed closed in its finalizer.
The aggregate failed only because coverage was required.

The retained Linux-only report recorded:

```text
overall:  3,744 / 7,899 = 47.39%; 1,883 unmapped
ordinary: 3,079 / 7,141 = 43.11%; 1,819 unmapped
critical:   665 /   758 = 87.73%;    64 unmapped
```

Review found three independent scope errors and one platform gap:

1. dedicated native-smoke, semantic-smoke/suite, startup-benchmark, and fuzz
   entry points were counted as production even though they are QA harnesses;
2. only complete inline `#[cfg(test)] mod` bodies were excluded, while complete
   test-support functions, fields, imports, constants, and feature-gated
   harness items remained in the denominator;
3. compiler-structural declarations and expression glue absent from LLVM's
   executable physical-line map were treated as uncovered or unmapped; and
4. a Linux-only producer cannot map executable Windows-only updater,
   named-pipe, process, and GUI bodies.

These are `TC-HARNESS-041`. The failed report was not relabelled, thresholds
were not reduced, and no platform body was excused merely because Linux could
not instrument it.

## Corrected scope and map contract

`scripts/diff_coverage.py` now:

- accepts one to eight repeated `--coverage-map` inputs;
- fully validates the schema, tool identity, raw map digest, represented source
  digest, and line count of every input;
- rejects duplicate map content and unsafe or stale source bindings;
- unions identical source paths and retains the maximum binary hit value for
  each physical line, preventing duplicate denominator entries;
- reports the multi-input provenance as
  `llvm-physical-line-map-union` while retaining the single-map report shape;
- excludes only exact repository-owned QA entry points and conventional test,
  benchmark, example, and fuzz-target paths;
- excludes complete attached items under exact test/test-support/fuzz-support
  cfg attributes with a literal/comment-masking, delimiter-aware, fail-closed
  scanner; and
- recognizes complete compile-time declarations and conservative structural
  expression/pattern glue without treating a complete call or final expression
  as non-coverable.

The existing required Windows behavior job keeps its prospective-merge
checkout for the all-feature gate. After those checks, it makes a second
isolated checkout of exact `VERIFICATION_SHA`, generates the targeted Windows
process profiles there, exports pinned LLVM JSON and native text, builds the
source-bound physical line map, and uploads the complete evidence. The Linux
coverage job depends on that successful job, downloads the exact artifact,
and supplies both maps to the unchanged 80%/90% policy.

## Closed Windows process inventory

The source-bound Windows producer also correctly rejected four newly present
matching tests until they were reviewed. `TC-HARNESS-042` updates the exact
inventory from 50 to 54 tests:

- 33 updater transaction-process tests;
- two installed-updater self-replacement tests;
- eight named-pipe tests, with exactly 419 filtered out;
- three external-mpv process tests, with exactly 424 filtered out; and
- eight media-tool process tests, with exactly 1,125 filtered out.

The four reviewed additions are the updater deterministic storage-fault
matrix, two parent-directory sync-denial regressions, and the nonce-owned
copied media-tool fixture identity regression. Extra, missing, ignored,
failed, partially selected, or differently filtered results still fail.

## Cross-platform source-byte binding

The canonical map hashes source bytes exactly as stored. Windows Git globally
configured with `core.autocrlf=true` had checked out at least some Rust files
as CRLF, while the Linux consumer checked out LF. A valid Windows map would
therefore have been rejected as stale before union. This locally detected
pre-hosted defect is `TC-HARNESS-043`.

The repository now declares:

```text
*.rs text eol=lf
```

A fresh local clone at exact head retained the host's global
`core.autocrlf=true` but reported
`i/lf w/lf attr/text eol=lf` for the sampled Rust source. Its SHA-256 matched
the retained Linux map's source digest. CI policy tests bind the exact
attribute so future edits cannot silently reintroduce platform-dependent map
bytes.

## Exact local replay

The clean LF clone ran all 54 instrumented Windows tests successfully and
produced:

```text
LLVM physical map: 2,518 / 161,761 = 1.556617%
LLVM line summary: 2,561 / 167,732 = 1.526840%
```

The diagnostic run's retained Linux map was reusable because no tracked Rust
source byte changed between `9f3cb60` and `829ab98`; both maps independently
passed current-source digest validation. The replay compared base
`f3964ebc7f7b281b9b78f3bfb243ff65e5122e33` with exact head
`829ab9824d20bc64b03179646c5e182d5c7a4bfb` and recorded:

| Scope | Production changed | Inline test support | Non-coverable | Covered / coverable | Result |
|---|---:|---:|---:|---:|---|
| ordinary | 2,763 | 1,901 | 613 | 1,723 / 2,150 (80.13%) | passed |
| critical | 792 | 4,411 | 173 | 562 / 619 (90.79%) | passed |
| combined | 3,555 | 6,312 | 786 | 2,285 / 2,769 (82.52%) | passed |

The combined report classified 31,350 changed lines in 107 test-only files,
accounted for all 41,217 changed Rust lines across 194 files, and left zero
unmapped production lines.

Retained local identities:

```text
23315141b47280f28477079199a79be352b493ed09d1c52999920d4c14a032e1  hosted Linux physical-line map
6351f14e2a278a53c9a2a67d5dcba60784c23679fac8228e3125341ae729425d  exact-head Windows physical-line map
4a8c0db49cf5247386746822dbc3c8eeb15d546eaf52bc7564489c5a46443490  exact Windows lane report
9c2291697d59645599f04c16a8cf81a341916abe83bfd6cd60f08ba399aab24d  exact-head union diff report
```

## Validation

Passed locally:

- 118 focused coverage/Windows/workflow regressions;
- all 525 Python policy and infrastructure tests;
- actionlint for the changed workflow;
- `cargo fmt --all --check` and `git diff --check`;
- warning-denied all-target/all-feature workspace Clippy;
- the complete all-feature workspace test and doctest suite; and
- the exact 54-test Windows process profile producer plus map conversion and
  union replay above.

## Limitations

- The Windows map is a targeted supplement, not a claim of whole-workspace
  Windows line coverage. The Linux producer remains the broad merged profile.
- The local replay reused the source-identical Linux artifact from diagnostic
  run `30627601938`. The final hosted workflow must regenerate and consume both
  maps at its own exact documentation-inclusive SHA before this is hosted
  acceptance.
- Structural classification is deliberately conservative. Ambiguous or
  unterminated test-support items and executable-looking unmapped lines still
  fail closed.
- Local JSON, text, profile, and replay files remain ignored evidence under
  `target/`; their exact identities are recorded above and were not committed.
