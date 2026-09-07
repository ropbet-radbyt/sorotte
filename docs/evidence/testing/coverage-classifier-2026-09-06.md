# Coverage classifier incident, 6 September 2026

The original CI attempt for candidate `72f93a123144484bff9c341a6bf81626d3155949` failed because the changed-line classifier treated three Rust syntax wrappers as executable lines. Both actual LLVM producers left these lines uninstrumented. The surrounding evaluation, pattern binding and rejection paths had coverage. This was a defect in the testing apparatus; the preserved evidence does not indicate an additional playback defect.

The failure is [Rust run 34027768378, attempt 1](https://github.com/ropbet-radbyt/sorotte/actions/runs/34027768378), job `101473914778` (`coverage-diff`). The original aggregate artifact is `9987854690`, ZIP SHA-256 `50931380809cac4fe06352dd07b14ee4318035c7bc85bc11c4f6e6be5776dba7`. The successful Linux and Windows producer artifacts are `9987833134` and `9987849098`. They remain separate from the failed aggregate.

All three affected lines are in `crates/sorotte-client-core/src/runtime/playback_coordination/local_seek.rs`:

| Line | Exact syntax | Original classification |
| --- | --- | --- |
| 48 | `) else {` | Unmapped |
| 60 | `) else {` | Unmapped |
| 138 | `let Some(LocalSeekEchoCandidate {` | Unmapped |

The LLVM JSON has no line segments at these locations, and the independently generated LLVM text view has blank count columns. Linux recorded positive hits on the tuple inputs, both rejection returns, the nested pattern fields, its initializer and its missing-candidate return. Every source digest in the 426-file Linux map and 379-file Windows map was rechecked against the committed candidate bytes. The aggregate contains byte-identical copies of those maps.

| Preserved input | SHA-256 |
| --- | --- |
| Linux physical-line map | `022431aeefa573bdd9d04e070fda5553de47f82a472c24a6258da8f1a3f5ca59` |
| Windows physical-line map | `846e3af787a81e01b8c47c80f3ed600275ab843d59526e2159208b746b28acf2` |
| Linux raw LLVM JSON | `6ed104614b7857ab984917bb80f9777f0e4445fffca603362658d5b544c479c7` |
| Linux raw LLVM text | `3fe73eb3d87e20eeb1acc0c93a9fd2bf173cb83430debff365f930b9458cfc88` |
| Windows raw LLVM JSON | `3fa35de864a9720cee1d92749541153aa45c561159443c46c5eaf9e6cb39ae8f` |
| Windows raw LLVM text | `a2bf7dddfff0341e40d0b8c066ddfd05aeda5ffdbd4288fba381dd1b74e13c3d` |

The narrow classifier repair recognizes a bare closing delimiter followed by `else {`, and a nested constructor/struct pattern opening before an initializer. Executable initializers, guards and trailing expressions remain subject to coverage. A mapped zero-hit line remains authoritative. Thresholds, coverage producers, path selection and source-digest checks are unchanged.

Replaying the repaired classifier over the original maps changed exactly the three rows above from `unmapped` to `non-coverable`. Ordinary coverage stayed **86.38%** and critical coverage stayed **95.03%**, above their unchanged 80% and 90% thresholds. Every other line decision and the executable hit/miss counts stayed the same. This replay is recorded in the isolated checkout at `target/verification/let-else-original-map-replay-2/{receipt.json,semantic-comparison.json}`.

The first local replay is also preserved. It exited 2 because the existing main worktree's physical source bytes did not match the LLVM source digest for `crates/sorotte-cli/src/lib.rs`. That checkout retained an older line-ending representation despite a clean Git status. The second replay used a fresh Git LF checkout; no source-digest check was disabled or changed. The first error remains in `target/verification/let-else-original-map-replay-1/`.

The small coverage canary now includes actual tuple `let-else` and nested `Some(Payload { ... })` patterns. Its original five hit/miss markers and child-process proof remain. Seven additional executable markers require independent JSON/text agreement for both initializers and both accepted/rejected outcomes. Two structural markers require absent JSON regions, blank text counts and correct lexical classification. Missing executable mappings, zero hits, mismatched views, duplicate identities and instrumentation incorrectly attributed to a wrapper are rejected.

| Actual local LLVM run | Original classifier | Repaired classifier |
| --- | --- | --- |
| Windows, pinned cargo-llvm-cov 0.8.4 / Rust 1.97.1 | Four LF build/export commands passed, then the wrapper classification failed | Eight build/export commands passed across LF and CRLF |
| Linux, same pins | Four LF build/export commands passed, then the wrapper classification failed | Eight build/export commands passed across LF and CRLF |

The original classifier was loaded from immutable candidate bytes for both failed canary attempts. The repaired attempts used classifier SHA-256 `83ba5eb885bf3b2c0834bdb6f5d1941eb7db931346cbf92e3803a82b5e74636e` and canary SHA-256 `8685e57c42a18ebde2dcff03d262c3135ef26747eb0125abaa0f412abeda0768`. Used inputs were hashed before and after execution and remained unchanged. Source, command logs, raw maps and receipts are retained under `target/verification/pattern-canary/` in the isolated checkout. Linux reused the existing scoped tool installation and used a fresh temporary checkout; no global tool installation or hosted retry occurred.

The combined focused validation passed 94 tests: 85 classifier tests and nine pattern-canary tests. A separate 34-test canary/frontdoor/observation run also passed; its nine pattern tests overlap the first run and are not additional distinct tests. The original classifier's failing regression attempts remain recorded.

The independent historical candidate closure is `target/verification/hosted/72f93a12/ci-review.json` in the main implementation worktree. It retains all 24 official artifact ZIPs and their member hashes: 15 Rust artifacts, five package artifacts and four dependency artifacts. Linux nextest passed 4,298 cases and Windows passed 4,327, including all 22 current Seek correlation tests on each platform, with no failure/flaky/rerun/skipped JUnit elements. The explicit selected-out inventories were 19 on Linux and 25 on Windows. The real minimum-mpv lifecycle passed 39 checks, and its four separate exact real-player semantics tests passed once each. Those results do not override the original failed coverage gate.

This note records an original hosted failure and local repair validation. It does not claim qualification of a later commit, a new hosted run or a release.
