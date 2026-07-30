# Coverage-guided protocol parser evidence — 2026-07-30

## Result

Sorotte now has a bounded, source-bound libFuzzer/AddressSanitizer lane for
its own public JSON protocol parser. The first executable campaign found one
low-severity product defect:

`TC-PROTOCOL-004: Protocol floating-point values can drift across decode and re-encode`

The defect is deliberately unfixed. Two ordinary expected-failure
characterizations register its raw and typed forms. The continuing oracle
admits only that exact defect class and remains strict for structure,
integers, non-floating leaves, sign, non-finite values, and floating-point
drift larger than one ULP.

After that registration:

- a 45-second continuation passed 559,788 executions;
- a fresh canonical 180-second continuation passed 1,915,137 executions;
- the two continuations passed 2,474,925 executions in total;
- no independent crash, sanitizer report, hang, or invariant failure
  surfaced; and
- `TC-CLI-003` and `TC-PROTOCOL-004` remain unchanged.

The canonical report is bound to committed implementation SHA
`729214d0de7ced9c56da7361bda68dc75b831179`. Its 29-file source manifest was
identical before and after the campaign.

## Safety and scope

This is defensive quality assurance of Sorotte's own local Rust parser:

- the target calls only public `sorotte-protocol` functions;
- generated input is local and capped at 65,536 bytes;
- there is no network target, reconnaissance, credential operation,
  persistence, privilege change, or third-party interaction;
- AddressSanitizer and libFuzzer are used only to detect application crashes
  and violated parser invariants; and
- product defects found by this tranche are characterized, registered, and
  left unfixed.

All campaign output is retained under ignored `target/fuzz-ci/` directories.
No corpus, log, crash artifact, build output, or `fuzz/target/` file is
committed.

## Package, target, and oracle

The standalone `fuzz/` Cargo package has its own lockfile and these exact
direct pins:

| Input | Pin |
| --- | --- |
| cargo-fuzz | `0.13.2` |
| libfuzzer-sys | `0.4.13` |
| serde | `1.0.229` |
| serde_json | `1.0.151` |
| Rust toolchain | `nightly-2026-07-29` |
| sanitizer | AddressSanitizer |
| target | `fuzz/fuzz_targets/protocol_line.rs` |

For each valid UTF-8 input no larger than the protocol line limit, the target
calls every public raw, diagnostic, aggregate, singular, typed, and encoding
boundary:

```text
decode_line
decode_message_line_items
decode_message_lines
decode_message_line
encode_line
encode_message_line
```

The target checks:

1. Raw and diagnostic decoding agree on JSON syntax validity.
2. Invalid JSON cannot aggregate- or singular-decode.
3. Valid JSON always produces at least one diagnostic item.
4. A serde `MapAccess` visitor independently derives top-level source order.
   It does not call or copy Sorotte's order scanner.
5. The diagnostic item sequence equals the independent unique-key order.
6. Duplicate command items expose serde_json's surviving value.
7. Non-object JSON produces exactly one command-less diagnostic item.
8. Aggregate and singular typed decoding remain strict across all diagnostic
   items.
9. Aggregate messages preserve item count, kind, and order.
10. Every decoded raw value and typed message survives encode/decode with
    exact semantics, except for the registered `TC-PROTOCOL-004` class.

The byte entrypoint remains total for invalid UTF-8 by declining to invoke the
string parser. libFuzzer still mutates arbitrary bytes and therefore exercises
that boundary.

## Exact resource contract

The runner fails closed outside these limits:

| Resource | Limit |
| --- | ---: |
| input length | 65,536 bytes |
| per-input timeout | 5 seconds |
| RSS | 2,048 MiB |
| jobs | 1 |
| maximum accepted campaign duration | 900 seconds |
| CI job timeout | 25 minutes |
| automatic minimization timeout | 120 seconds per artifact |

Pull-request and `main` push campaigns run for 45 seconds. Scheduled and
manual campaigns run for 900 seconds. The local canonical continuation used a
fresh 180-second output directory.

The runner requires:

- an exact 40-character lowercase source SHA;
- exactly 14 direct, regular seed files;
- an output directory under `target/` that does not already exist;
- exact cargo-fuzz identity;
- complete libFuzzer final statistics on successful runs;
- direct, regular files for source, seed, corpus, and artifact manifests; and
- identical bound-source and seed-source manifests before and after execution.

It records the resolved command, tools, limits, source manifest, seed
manifest, final corpus, artifacts, minimization attempts, timestamps, exit
state, statistics, and evidence errors in `run-report.json`.

## Workflow trigger and security contract

`.github/workflows/rust-fuzz.yml` is path-filtered for pull requests and for
pushes to `main`. Its path inventory covers every fixed source bound by the
runner, either exactly or through the protocol/fuzz directory roots:

```text
Cargo.toml
rust-toolchain.toml
coverage/behaviors.toml
coverage/known-defects.toml
crates/sorotte-protocol/**
fuzz/**
.github/workflows/rust-fuzz.yml
scripts/known_defect_policy.py
scripts/tests/test_known_defect_policy.py
scripts/tests/test_protocol_fuzz_policy.py
```

The same workflow supports manual dispatch and a weekly Wednesday
`03:45 UTC` schedule. It has read-only contents permission, disables persisted
checkout credentials, pins every third-party action to a full commit, installs
the exact cargo-fuzz version with no fallback, verifies the toolchain at
runtime, validates the fuzz policy before building, and uploads evidence for
14 days even when the target fails. It contains no tolerated or masked fuzz
failure. Concurrency cancels superseded PR/push work but never scheduled or
manual campaigns.

`scripts/tests/test_protocol_fuzz_policy.py` structurally binds this contract,
including a regression that every fixed bound source is covered by a workflow
trigger. Both changed workflows pass the installed actionlint binary.

## Product finding: TC-PROTOCOL-004

The first real campaign reduced the failure to the five-byte input:

```text
70E70
```

The raw public boundary observes:

```text
before: 7.000000000000001e71
after:  7.000000000000002e71
```

The same adjacent-representation change occurs inside a valid typed protocol
message:

```json
{"State":{"playstate":{"position":70E70}}}
```

The retained crash artifact is 5 bytes and has SHA-256
`ccabbcc5ab3f05fab297b4d429f24fe96753ea6c63545bc547832d8ff202bf2e`.
The automatic `tmin` subprocess exited 1 and emitted an empty output, so that
zero-byte file is not represented as a successful minimization. The original
five-byte artifact itself is the exact reproducer above.

Two ordinary, non-ignored characterizations are registered:

```text
tests::known_defect_tc_protocol_004_raw_floating_point_roundtrip_is_exact
tests::known_defect_tc_protocol_004_typed_state_floating_point_roundtrip_is_exact
```

Both use the exact panic oracle:

```text
TC-PROTOCOL-004: protocol floating-point value changed across decode/encode/decode
```

No serde feature was enabled, no numeric input was rejected or clamped, and no
production behavior was repaired.

## Continuation classifier

The continuing target recursively compares the before/after JSON values. It
accepts a leaf only when:

- the leaves compare exactly; or
- both leaves are JSON numbers representable as finite `f64`;
- their signs are identical; and
- their `to_bits()` values differ by exactly one.

Arrays must retain length and pairwise structure. Objects must retain their
complete key sets and recursively matching values. Integers, booleans, nulls,
strings, non-finite conversions, structural changes, sign changes, and
larger floating-point changes remain failures. Typed-message drift is first
converted independently to JSON and subjected to the same recursive rule.

This classifier is intentionally limited to the registered defect class. It
is not presented as a positive proof that exact floating-point roundtrip is
correct.

## Portability and provenance hardening

The first WSL execution could not traverse the Windows worktree `.git`
pointer. The runner no longer asks local Git for the revision; callers must
supply `--source-sha`, and CI supplies `${{ github.sha }}`. The first
non-login WSL retry also lacked cargo, so local commands use `bash -lc`.

The canonical bound-source inventory contains:

- root manifest and pinned toolchain;
- fuzz workflow, package manifest, lockfile, runner, and target;
- behavior and known-defect registries;
- known-defect validator and both relevant policy suites; and
- every Rust source file under `crates/sorotte-protocol/src`.

Adding the workflow, registry, catalog, validator, and policy tests expanded
the bound inventory from 23 to 29 files. The policy suite derives its expected
inventory directly from the runner and proves both content and inventory drift
change the aggregate.

The first experiments used base SHA
`0748e4a8f07bad4ab30b26b22535ec969c3b10cf` while the implementation was
dirty. Their before/after file manifests are the exact source identity and are
explicitly retained as such. The final implementation was committed first;
the canonical campaign therefore uses its actual implementation HEAD as well
as the exact 29-file manifest.

## Experiment inventory

| Output | Status | Outcome |
| --- | --- | --- |
| `protocol-line-smoke` | `setup_failed` | WSL could not follow the Windows worktree Git pointer; replaced by required explicit source SHA |
| `protocol-line-smoke-v2` | `setup_failed` | cargo was absent from non-login WSL PATH; local invocation changed to `bash -lc` |
| `protocol-line-smoke-v3` | `failed` | genuine `TC-PROTOCOL-004` counterexample `70E70` |
| `protocol-line-smoke-v4` | `passed` | 45-second continuation with the exact one-ULP classifier |
| `protocol-line-deep-canonical-729214d-v1` | `passed` | fresh 180-second canonical campaign over committed implementation HEAD |

### Setup attempt hashes

`protocol-line-smoke` stopped before a fuzzer log existed:

- source manifest: 23 files, 272,337 bytes,
  `28b94fa4066032e8807419dae8b8d8a507f57971eb84dc241aaebb588cc8f5a7`;
- report: 7,872 bytes,
  `b5695a72ae43939fc2a147ac8046f4dd746c05af5a7832cc337b5201e1432c16`.

`protocol-line-smoke-v2` also stopped before a fuzzer log existed:

- source manifest: 23 files, 272,504 bytes,
  `60b759af3b67cff6df1bcd86206b6b6d351819b9f4c89092b13b488e772b65b3`;
- report: 7,880 bytes,
  `f1068e6c4173c88b00bee6607826f91bf0254a3c551395ffaccabb9e48298ad6`.

### Counterexample campaign

`protocol-line-smoke-v3` produced:

| Statistic | Value |
| --- | ---: |
| executed units | 108,863 |
| average executions/second | 9,896 |
| new units | 1,385 |
| slowest unit | 0 seconds |
| peak RSS | 452 MiB |
| final corpus | 683 files / 44,760 bytes |
| artifacts | 1 file / 5 bytes |

Attestations:

- source aggregate:
  `60b759af3b67cff6df1bcd86206b6b6d351819b9f4c89092b13b488e772b65b3`;
- final corpus aggregate:
  `5251028375878858da86eb804b15a15e748b8cc8744f7256a6e256b2df1e36c3`;
- artifact file:
  `ccabbcc5ab3f05fab297b4d429f24fe96753ea6c63545bc547832d8ff202bf2e`;
- report: 142,117 bytes,
  `8de3aae9bbe47873e767819380f0fa449db7aff06bb42c334394093e15c53b71`;
- log: 182,024 bytes,
  `1b8d6a483d0fea2fde960ade4f267cd2b23ce5dcd24f63d1891213c9922ea486`;
- source and seed source stable: yes;
- fuzzer exit: 1.

### First continuation

`protocol-line-smoke-v4` produced:

| Statistic | Value |
| --- | ---: |
| executed units | 559,788 |
| average executions/second | 12,169 |
| new units | 3,592 |
| slowest unit | 0 seconds |
| peak RSS | 485 MiB |
| final corpus | 1,201 files / 90,624 bytes |
| artifacts | 0 |

Attestations:

- source aggregate:
  `acb488f2c613f9204c792908bc2b4d4cafd6870ebad7abb5ef3a85bd2b1a31b0`;
- final corpus aggregate:
  `459117ca97036f26c9d79902a232ce9e41f3df1b07c94c42d3089f7ca228c750`;
- empty-artifact aggregate:
  `4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945`;
- report: 235,942 bytes,
  `0f7fd3fe21bd07f1536b876d7387e7c32708422caaa9ab39f18571b17641abe2`;
- log: 444,825 bytes,
  `654b947170af4c258ec73e0ad314fe0f4706d4ce99f75bae18fba5dab9015c78`;
- source and seed source stable: yes;
- fuzzer exit: 0.

### Canonical 180-second continuation

Invocation:

```text
wsl.exe -d Ubuntu \
  --cd /mnt/c/tmp/sorotte-test-coverage-design \
  bash -lc "python3 fuzz/run_protocol_fuzz.py \
    --toolchain nightly-2026-07-29 \
    --source-sha 729214d0de7ced9c56da7361bda68dc75b831179 \
    --seconds 180 \
    --seed-corpus crates/sorotte-protocol/tests/corpus/protocol_parser \
    --expected-seed-count 14 \
    --output-root target/fuzz-ci/protocol-line-deep-canonical-729214d-v1"
```

The report's resolved libFuzzer command retained:

```text
cargo +nightly-2026-07-29 fuzz run
  --fuzz-dir fuzz
  --sanitizer address
  --jobs 1
  protocol_line <fresh-corpus>
  --
  -max_total_time=180
  -max_len=65536
  -timeout=5
  -rss_limit_mb=2048
  -artifact_prefix=<fresh-artifact-directory>/
  -print_final_stats=1
```

Result:

| Statistic | Value |
| --- | ---: |
| status | passed |
| fuzzer exit | 0 |
| executed units | 1,915,137 |
| average executions/second | 10,580 |
| new units | 6,634 |
| slowest unit | 0 seconds |
| peak RSS | 519 MiB |
| final corpus | 1,888 files / 448,491 bytes |
| artifacts | 0 |
| evidence errors | 0 |

The run started at `2026-07-30T08:21:03.843456+00:00` and finished at
`2026-07-30T08:24:23.135009+00:00`, including setup and final evidence
collection.

Canonical attestations:

| Evidence | Identity |
| --- | --- |
| committed source SHA | `729214d0de7ced9c56da7361bda68dc75b831179` |
| bound source before/after | 29 files / 366,851 bytes |
| bound source aggregate | `ce6d09c2d9f491dd3756875ff44aa241e98dfe12d87f6bc8457618f414d8537a` |
| seed source | 14 files / 817 bytes |
| seed aggregate | `c60dd26c342ed911298f647acedf0b1e7ea045f14660e62e387e03894a17fbf4` |
| final corpus aggregate | `4fc44405406939efc3617d71adbec0f142460fe7cf648d557f02cf68364ed049` |
| empty-artifact aggregate | `4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945` |
| report | 364,338 bytes / `5b9054af47b3b766b2f63a4f0fff15826b29c4cd81fd933cd924da0e8b2588c9` |
| log | 824,333 bytes / `13edb5ce980d0c18a554b80c13daad7b77a529153defafdad7e4447147d1e854` |

The source and seed manifests were byte-identical before and after execution.

## Canonical tool identities

```text
cargo-fuzz 0.13.2
rustc 1.99.0-nightly (26ae60a9e 2026-07-28)
rustc commit 26ae60a9eeb20b4935be49d7a931a650fa1d2923
cargo 1.99.0-nightly (3efb1f477 2026-07-17)
LLVM 22.1.8
Python 3.12.3
Linux 6.6.87.2-microsoft-standard-WSL2 x86_64
```

## Policy and focused validation

The integrated implementation passed:

| Check | Result |
| --- | --- |
| protocol fuzz plus known-defect policy suites | 34/34 |
| combined mutation, CI, known-defect, and fuzz policy suites | 84/84 |
| real known-defect registry | 2 defects / 4 exact characterizations |
| exact `TC-PROTOCOL-004` selectors | 2/2 expected-failure characterizations |
| complete protocol package | 88 library + 6 parser integration tests |
| strict protocol Clippy | passed with warnings denied |
| formatting and diff whitespace | passed |
| fuzz workflow actionlint | passed |
| canonical 180-second ASan campaign | 1,915,137 executions; no independent failure |
| full workspace Clippy | passed all targets/features with warnings denied in 15.65 seconds |
| full workspace tests | passed locked/all-feature on the first run in 250.8 seconds |
| complete Python infrastructure/policy suite | 399/399 passed in 20.383 seconds |

The known-defect validator now additionally requires every registered
`expected_panic` to begin with its own defect identifier. A regression proves
that a characterization cannot name a different defect while satisfying the
rest of the inventory.

## Limitations

This evidence is deliberately narrow:

- one protocol-line target is not fuzzing of framed transport, session
  cancellation, reconnect, TLS, server dispatch, or mpv IPC;
- it does not replace the deterministic one-byte, split-CRLF, truncation,
  half-close, duplicate-key, corpus, or typed-protocol tests;
- it has no Python differential oracle;
- a finite 180-second continuation is not exhaustive;
- AddressSanitizer on Linux/WSL does not prove Windows-specific behavior,
  undefined-behavior freedom under every sanitizer, or allocation behavior
  outside the configured RSS bound;
- parser fuzzing does not prove SQLite durability, operating-system ACL or
  syscall behavior, real-player integration, semantic GUI behavior, or native
  GUI rendering/accessibility; and
- the one-ULP classifier is an explicit known-defect continuation allowance,
  not evidence that exact floating-point roundtrip is correct.

`TC-CLI-003` and `TC-PROTOCOL-004` remain open, registered, and deliberately
unfixed.
