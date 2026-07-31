# Cross-platform changed-line coverage-map union — 2026-07-31

## Outcome

Commit `829ab9824d20bc64b03179646c5e182d5c7a4bfb` corrects the
required changed-line gate without lowering either ratchet. Commit
`2b8af5672cd27c727f3707b71ccd15a1292135c7` then binds the coverage evidence
finalizer to that same ordered multi-map input instead of assuming a single
primary map. The gate still
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

Exact implementation-head workflow
[`30639113884`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30639113884)
subsequently regenerated both maps, accepted the corrected ordered-map
finalizer, and passed the required aggregate at
`dd3012c1bcefa0a68520b063c5ae06f3e1b96f79`. The fresh hosted union passed at
83.03% combined, 80.92% ordinary, and 90.79% critical with zero unmapped
lines. Documentation-inclusive exact-head acceptance remains a separate final
publication check.

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

## Retained TC-HARNESS-044 hosted diagnostic

Workflow run
[`30632931277`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30632931277)
executed exact head `a2441a30f1e98ba85d2384c2986f09b84a5dcb4f`.
Every originating behavior and evidence producer passed. Coverage job
[`91169713196`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30632931277/job/91169713196)
ran the exact 54-test Windows producer, regenerated both source-bound maps,
and passed the unchanged policy at:

```text
overall:  2,286 / 2,769 = 82.55%
ordinary: 1,724 / 2,150 = 80.18%
critical:   562 /   619 = 90.79%
unmapped: 0
```

The coverage job's final evidence-finalization phase failed; the downstream
verification aggregate consequently failed:

```text
diff-policy: diff-coverage report is not bound to the retained canonical line-map artifact
```

The report correctly declared an ordered Linux/Windows union, but the coverage
evidence finalizer still accepted and compared only one canonical map. This is
`TC-HARNESS-044`: a valid multi-map report was rejected after all producers
and thresholds had already passed. The failed phase artifact remains retained
with SHA-256
`9ebebeb0af5a957fe261aa5c57e6639ef6047884f0e4104fc8238f34369e5ea7`.
The originating map identities are:

```text
f7247f0a74a7a04a7689a40ca66eafbfc47214ed4311d6d1ea6d40a6d2f9023a  Linux map
b2bb2249c15b54af9d17ad179dac957d1a3508ea7796632687baf5c576d206e9  Windows map
4c4a3bc2e222230ac06bee1a8119317f51190553eaf56b313e17cbee47df565e  union diff report
```

Commit `2b8af5672cd27c727f3707b71ccd15a1292135c7` makes supplemental
maps repeatable finalizer inputs and requires the retained report to bind the
complete ordered primary-plus-supplemental tuple. Omission, reordering,
duplication, replacement, source drift, or content tampering remains fatal.
Six focused finalizer regressions cover those cases. Replaying the exact
downloaded artifacts under `target/hosted/30632931277/replay-root` passed;
the corrected phase artifact has SHA-256
`b889d98a1e947b607a69c126d6b51ac46cb9d88e4bcbb40a734257d4c3c512b3`
and retained the exact report identity above. The failed hosted finalizer is
kept as discovery evidence rather than converted into a passing run.

## Successful implementation-head hosted acceptance

Workflow run
[`30639113884`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30639113884)
executed exact implementation head
`dd3012c1bcefa0a68520b063c5ae06f3e1b96f79`. All originating required jobs
passed. Coverage job
[`91190243453`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30639113884/job/91190243453)
regenerated the broad Linux map from 120 fresh profiles and consumed the exact
54-test Windows process map. The corrected finalizer accepted the complete
ordered tuple and the unchanged policy passed:

```text
overall:  2,403 / 2,894 = 83.03%
ordinary: 1,841 / 2,275 = 80.92%
critical:   562 /   619 = 90.79%
unmapped: 0
```

Required aggregate job
[`91192554763`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30639113884/job/91192554763)
then passed with no manifest errors. Downloaded artifact identities are:

```text
c46a2e0075a82c1b0288f4d382932012a53181422a8ce08016f9eb075fad2542  Linux physical-line map
f883a3dbc347d26139e5d48b25c3b3b626b4bc7431355ffd10be9ca151513400  Windows physical-line map
c6187f3b8a9c4237c22be74c2884afc08de09d9a354ba563f8b496460a36500c  accepted union diff report
df3efff1780babbb9cb371a8d1d07c41a4efbdcf4c5c50444b3333aeafa7f8c5  successful phase manifest
6a6ec4184fcb66a3564c104fa92110deae7c1a2b5fb916199ee25200f5d54a8d  coverage-profile lane manifest
6ebc5ef4793609c515c3824484d2b7389fbbaeb182271ff047711823e88e5244  successful required aggregate
```

The Linux lanes contributed 110 workspace, three GUI-semantic, and seven
required-live profiles; the merge check added no profile. Artifact
`8797632792` retains the coverage evidence and artifact `8797641783` retains
the passing aggregate. The prior failed run remains the TC044 discovery RED.

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
- all 531 Python policy and infrastructure tests;
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
  run `30627601938`. Implementation-head run `30639113884` independently
  regenerated and consumed both maps successfully. A final workflow at the
  documentation-inclusive committed SHA remains the publication check.
- Structural classification is deliberately conservative. Ambiguous or
  unterminated test-support items and executable-looking unmapped lines still
  fail closed.
- Local JSON, text, profile, and replay files remain ignored evidence under
  `target/`; their exact identities are recorded above and were not committed.
