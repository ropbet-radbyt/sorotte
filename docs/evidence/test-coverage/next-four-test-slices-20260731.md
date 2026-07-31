# Next four test slices — integrated evidence

Date: 2026-07-31

Branch: `codex/test-coverage-design`

Primary implementation snapshot: `9f3cb60fbe788575829931b56155f4bc0c19caf0`

Final test-coverage closure snapshot:
`829ab9824d20bc64b03179646c5e182d5c7a4bfb`

Hosted-harness corrections through:
`829ab9824d20bc64b03179646c5e182d5c7a4bfb`

## Status

The four primary slices are implemented and have focused local evidence.
`TC-CLI-004`, `TC-CLI-005`, and `TC-PLAYER-005` are fixed with ordinary
positive regressions. The final-source stalled-HTTP native campaign is green
and independently approved.

Multiple hosted workflows are retained as diagnostics, not relabelled as
acceptance. The first exposed `TC-HARNESS-018` through `TC-HARNESS-024`. The second proved the
corrected generated-media, complete live-compatibility, semantic, lifecycle,
Ubuntu server-release, and Windows nextest lanes, then exposed
`TC-HARNESS-025` through `TC-HARNESS-029`. Later fail-closed runs exposed
`TC-HARNESS-030` through `TC-HARNESS-038` and `TC-HARNESS-040` through
`TC-HARNESS-043`; the direct native-Windows ASan diagnostic is
`TC-HARNESS-039`. Every correction has a focused commit and positive local
regression. The exact-head required-live and WSL fuzz campaigns, 54-test
Windows process map, and local Linux/Windows coverage union are green. Final
documentation-inclusive hosted acceptance remains pending.

## Scope and safety boundary

This tranche covers Sorotte's own local Rust JSON framing/session, client
timing, CLI configuration, Media Match diagnostics, GUI/player integration,
and pinned compatibility harness.

- Generated client and CLI input is processed in memory.
- Generated Media Match input is local synthetic media in a nonce-owned
  temporary root.
- Native player traffic uses only the local Sorotte GUI, the exact installed
  mpv, and strict OS-assigned IPv4-loopback HTTP/session listeners.
- Compatibility execution uses the pinned local Syncplay checkout.
- No public network target, reconnaissance, credentials, persistence,
  privilege work, or exploitation is involved.

The pre-existing untracked handoff is not part of this evidence file and was
not modified.

## Primary implementation binding

| Slice | Primary commit(s) | Detailed evidence |
|---|---|---|
| deterministic client timing | `f2f3cf5` | [`client-ping-jitter-drift-schedules-20260731.md`](client-ping-jitter-drift-schedules-20260731.md) |
| required generated Media Match | `a3e4d06`, `7e3f649`, corrected fixture `c19f523` | [`media-match-generated-media-capability-20260731.md`](media-match-generated-media-capability-20260731.md) |
| CLI argument/configuration composition | `5e44bf1` | [`cli-argument-configuration-composition-20260731.md`](cli-argument-configuration-composition-20260731.md) |
| valid stalled-HTTP real-mpv recovery | `30994e5` | [`native-gui-real-mpv-stalled-http-recovery-20260731.md`](native-gui-real-mpv-stalled-http-recovery-20260731.md) |

Hosted-diagnostic corrections are separately focused:

| Finding | Correction commit | Local disposition |
|---|---|---|
| `TC-HARNESS-018` generated fixture reachability/diagnostics | `c19f523` | hosted generated-media job passed |
| `TC-HARNESS-019` unquoted Git revision expression | `b616cf2` | hosted source check reached the later peeled-identity assertion |
| `TC-HARNESS-020` incomplete Rust component declarations | `b616cf2`, policy alignment `23d86c9` | hosted Rust setup completed in every required job |
| `TC-HARNESS-021` legacy permanent-room startup race | `a00b088` | hosted complete live compatibility passed 138/138 |
| `TC-HARNESS-022` non-Windows test helper import | `912970c`, formatting `0dd6c79` | portable external-launch module 15/15; final Linux all-feature confirmation pending |
| `TC-HARNESS-023` hosted PowerShell process fixtures | `0b51fb0` | hosted Windows process probes passed before a later player test failed |
| `TC-HARNESS-024` nextest fixture-role collision | `a5ae5be` | hosted Windows nextest completed successfully |
| `TC-HARNESS-025` quoted/dynamic coverage environment | `854a805`, exact-token hardening `bcede9c` | both lane modules pass 42/42 against shell quoting and bounded `%Nm` variants |
| `TC-HARNESS-026` annotated mpv tag object compared with peeled commit | `3e03b70` | actionlint and exact CI source-policy tests pass |
| `TC-HARNESS-027` non-executable POSIX TLS fixture | `20b02a8` | all three atomic TLS publisher tests pass through explicit `sh` |
| `TC-HARNESS-028` early-exit named-pipe race | `0502fef`, oracle hardening `c7b134e` | exact regression passes 50/50 consecutively; full player suite passes |
| `TC-HARNESS-029` stale/non-exclusive latest-publication assertion | `ee7b9e9`, exclusivity hardening `df7df33` | PowerShell policy and YAML-aware workflow policy pass |
| `TC-HARNESS-030` unreachable non-Windows native preflight | `7395cdf` | target-sensitive preflight regression; later hosted Linux lane passed |
| `TC-HARNESS-031` non-exact compatibility profile inventory | `4fae099` | exact report and fully qualified libtest inventory policy pass |
| `TC-HARNESS-032` 30 ms duplex model deadline | `8dbc444` | bounded one-second in-memory fault-model deadline passes |
| `TC-HARNESS-033` delayed first legacy frame | `d844d2e`, hardening `ad410fc` | delayed-frame loopback regression and full live matrix pass |
| `TC-HARNESS-034` process-unsafe legacy checkout bootstrap | `5d5e77a`, hardening `ad410fc` | two-process lock regression and prechecked Linux oracle pass |
| `TC-HARNESS-035` delayed permanent-room setter alternate | `0e7a9bc` | exact-context positive/negative canonicalizer matrix and live suite pass |
| `TC-HARNESS-036` LLVM exa-scale count token | `cea5fb7` | exact `kMGTPE` grammar regressions pass |
| `TC-HARNESS-037` missing Linux legacy Python stack | `404039b` | pinned install-order policy and later hosted Linux lane pass |
| `TC-HARNESS-038` released ephemeral legacy server ports | `6ccfd3a` | in-process/cross-process lease regression; two full parallel live runs pass |
| `TC-HARNESS-039` incompatible native Windows ASan runtime | no source change | failed diagnostic preserved; canonical WSL ASan campaign passes |
| `TC-HARNESS-040` strict selector count after port-lease test | `9f3cb60` | exact 21 selected / 128 filtered policy and Cargo selector pass |
| `TC-HARNESS-041` Linux-only map plus overbroad production scope | `829ab98` | two-platform replay passes 82.52% overall, 80.13% ordinary, 90.79% critical, zero unmapped |
| `TC-HARNESS-042` stale Windows process inventory | `829ab98` | exact 54-test instrumented producer passes with reviewed filtered counts |
| `TC-HARNESS-043` platform-dependent Rust source bytes | `829ab98` | fresh Windows checkout stays LF under global autocrlf and matches Linux source digest |

## Slice results

### 1. Deterministic client jitter, drift, and playback schedules

Four tests use explicit synthetic timestamps and an independent literal model.
They cover:

- eight ping observations spanning jitter, a large finite outlier and
  recovery, a backward local clock, a future echo, non-finite receipt time,
  and negative server RTT;
- 27 affine schedules across three common offsets, three local clock rates,
  and three RTT samples;
- six receipt/reply clock schedules from zero through 750 milliseconds plus
  invalid and paused cases; and
- eight playback decision steps for rewind, scheduler suppression, slowdown,
  sustained fast-forward, `doSeek`, and paused-room behavior.

Focused result: 4/4. Complete `sorotte-client-core`: 728/728. Strict crate
Clippy passed. No product or harness defect was found. This proves current
legacy arithmetic over supplied values, not cross-host clock authenticity,
path symmetry, monotonic wall clocks, executor latency, or live telemetry.

### 2. Required generated-media Media Match V3

The former optional GUI test is now a direct ignored-by-default
`sorotte-media-match` integration target bound to a required non-scheduled
Ubuntu job. Real ffmpeg/ffprobe generate and inspect a 120-second fixed-seed
broadband FFV1/PCM Matroska fixture. The oracle requires exact duration,
positive landmarks and PCM bytes, all three sampled-fast windows, retrieval,
`Probable` expectation, populated decision, and JSON report shape.

Hosted job `91093403053` in workflow run `30610965479` retained the original
`TC-HARNESS-018` RED. The first 30-second sine fixture could not produce the
required aligned span and margin, and its boolean assertion hid the decision
diagnostic. The correction keeps the threshold and adds typed failure context.
The integration target compiles locally; the ordinary media-match suite passes
84/84 with the one capability test registered ignored. Local PATH did not
provide ffmpeg/ffprobe. Hosted job `91111808305` in workflow run
`30616813538` executed the corrected real-tool body successfully, so the
capability is no longer inferred from compilation alone.

### 3. CLI parser and configuration composition

The fixed-seed campaign executes 256 cases: 208 valid, 48 invalid, 64 with a
clear operation, and 112 with duplicate operations. It crosses process
environment, stored settings, the production legacy parser, and production
override application while the expected projection uses an independent
parser/composition model.

The retained five-test RED was 0 passed/5 failed. `TC-CLI-004` covered attached
values, duplicate clear/replace semantics, atomic host/port replacement, and
required-value rejection. `TC-CLI-005` covered an attached
credential-shaped value reflected in an unknown-option diagnostic.

After correction, the complete focused module passes 6/6, including all 256
generated cases. The CLI library passes 366 tests with eight registered
ignores, both integration tests pass, doctests pass, and strict all-target
Clippy passes.

### 4. Valid byte-silent real-mpv recovery

The fourth native mode declares a complete 4,320,024-byte AU body, transmits
exactly 720,000 bytes at a fixed pace, then retains the response while emitting
no further byte or EOF. Its contract requires:

- positive progress and cache pause within 0.25 seconds of the deterministic
  7.49975-second playable-prefix boundary;
- at least 25 seconds of server-side silence;
- zero EOF observations before recovery;
- exactly one same-process `end-file` with reason `stop`, immediately followed
  by the recovered `file-loaded`;
- one complete byte-zero recovery GET, resumed progress, native pause and Exit;
  and
- exact GUI/mpv/session/IPC/media identity plus complete release.

Two preserved product REDs are:

```text
target/verification/gui-real-mpv-stalled-http/20260731T064003014Z-49828
target/verification/gui-real-mpv-stalled-http/20260731T071541899Z-64656
```

They exposed `TC-PLAYER-005`: a restart newer than a deferred start could be
lost before authoritative attempt binding, and finite VOD could still be
classified `Unknown` when cache pause arrived. Production now preserves and
replays only the causal successor restart and permits `Unknown` to arm only
without positive live evidence.

The final implementation-source post-gate GREEN, run last after the full local build-producing
validation matrix, is:

```text
target/verification/gui-real-mpv-stalled-http/20260731T115707208Z-35432
```

| Field | Value |
|---|---|
| assertions / artifacts | 18 / 11 |
| GUI SHA-256 | `439174541d461db90fc66be088152024814e3ba4fe0d0d6b3add464103205d9e` |
| mpv SHA-256 | `2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0` |
| first / recovery bytes | 720,000 / 4,320,024 |
| server-side silence | 28,962 ms |
| cache-stall / recovered position | 7.424 / 8.023747 seconds |
| EOF / replacement stop | 0 / 1 |
| manual retries / invalid identities | 0 / 0 |

Focused player recovery tests pass 19/19; native-runner tests pass 17/17;
Python contract tests pass 22/22; the full player suite passes 427 tests with
two registered ignores; and strict player/GUI Clippy passes.

## Hosted diagnostic classification

Workflow run
[`30610965479`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30610965479)
was retained as a diagnostic run over the earlier implementation SHA. It is not
the final hosted result.

| Finding | Diagnostic classification | Correction |
|---|---|---|
| `TC-HARNESS-018` | generated 30-second sine could not reach `Probable`; boolean failure was opaque | 120-second fixed-seed broadband fixture, three windows, typed diagnostics |
| `TC-HARNESS-019` | unquoted `HEAD^{commit}` stopped shell policy/source verification | quote the revision expression and bind it in policy |
| `TC-HARNESS-020` | partial Rust components caused lazy toolchain component conflict | explicitly request `rustfmt, clippy` everywhere and LLVM tools on coverage jobs |
| `TC-HARNESS-021` | legacy permanent-room client connected before asynchronous room load | observe every room in public `List`, then half-close and await probe EOF |
| `TC-HARNESS-022` | portable external-player test helper was imported only on Windows | use `cfg(test)` for the portable helper |
| `TC-HARNESS-023` | hosted PowerShell startup/quoting exceeded media-tool process bounds | use `cmd.exe`; test invalid UTF-8 directly at the parser |
| `TC-HARNESS-024` | nextest's ordinary `--exact` launch matched the intentional parked child role | require exact target plus nonce-owned copied executable stem |

Workflow run
[`30616813538`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30616813538)
is the second retained diagnostic over implementation SHA
`23d86c970b6a72981029e4cccb98c5f45930e81f`. It proved generated Media Match,
complete live Python compatibility (138 executable, seven exact writing
fixtures), GUI semantic, lifecycle, Ubuntu server-release, and Windows
all-feature nextest behavior. It then exposed these additional harness-only
failures:

| Finding | Diagnostic classification | Correction |
|---|---|---|
| `TC-HARNESS-025` | `show-env` retained POSIX quotes and hard-coded `%32m`, while the hosted producer emitted quoted `%4m` | request stable `--sh` output, decode one word without evaluation, and parse exactly one bounded real `%p`/`%Nm` pair |
| `TC-HARNESS-026` | immutable annotated tag object `2c219aa...` peeled to commit `41f6a64...`, so the exact commit check compared different Git object types | pin checkout and verification to the peeled immutable commit |
| `TC-HARNESS-027` | the TLS publisher is intentionally tracked mode `100644`, but the Linux test tried to execute it directly | invoke the POSIX fixture explicitly through `sh` |
| `TC-HARNESS-028` | the early-exit fake mpv could close a named pipe before the client wrote its request | consume one exact request before exit 23 and retain role-specific terminal assertions |
| `TC-HARNESS-029` | the publication test expected the older `github.event.inputs` spelling and proved existence rather than exclusivity | bind the exact string-valued dispatch choice and require exactly one guarded `latest` entry |

Workflow run
[`30618496116`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30618496116)
then proved every required lane except Linux all-feature and coverage before
exposing `TC-HARNESS-030` and `TC-HARNESS-031`. Workflow run
[`30620966526`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30620966526)
retained the next fail-closed layer: `TC-HARNESS-032` through
`TC-HARNESS-034`.

Workflow run
[`30624838791`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30624838791)
at `ad410fc` passed lifecycle, complete live compatibility, GUI semantic,
real-mpv, generated Media Match, Windows all-feature behavior, and both server
release jobs. Its only originating failures were the valid LLVM token `18.4E`
(`TC-HARNESS-036`) and missing Linux legacy Python packages
(`TC-HARNESS-037`); the aggregate failed because those required jobs failed.

After local required-live repetition exposed and closed `TC-HARNESS-035` and
the port collision `TC-HARNESS-038`, workflow run
[`30626889218`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30626889218)
at `6ccfd3a` passed every originating job except coverage. Coverage ran and
passed all 21 selected compatibility tests, then correctly rejected the stale
20-test source tuple as `TC-HARNESS-040`. Commit `9f3cb60` binds the reviewed
new test identity. `TC-HARNESS-039` is the separately retained noncanonical
native-Windows ASan runtime diagnostic, not a hosted or product failure.

Corrected implementation-source workflow run
[`30627601938`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30627601938)
at `9f3cb60` passed every originating required job except coverage. The retained
Linux-only coverage report classified 7,899 coverable production lines at
47.39% and left 1,883 unmapped; the aggregate failed because coverage was
required. Review separated the overbroad QA/test-support/structural scope,
missing Windows platform map, stale 50-test Windows process inventory, and
CRLF/LF source-byte mismatch as `TC-HARNESS-041` through
`TC-HARNESS-043`. Commit `829ab98` corrects those contracts without lowering
the 80% ordinary or 90% critical ratchets. Exact details and retained map/report
hashes are in
[`platform-coverage-map-union-20260731.md`](platform-coverage-map-union-20260731.md).
The diagnostic run remains independent from the later
documentation-inclusive acceptance run.

No item in this table is a Sorotte product behavior defect. Each correction
retains the original strict assertion or timeout. None is being converted into
a skip, retry-only pass, lowered threshold, or normalized parity exception.

## Current committed-source static inventory

The exact committed implementation snapshot contains:

| Crate | Test attributes | Ignored |
|---|---:|---:|
| `sorotte-cli` | 376 | 8 |
| `sorotte-client-app` | 197 | 0 |
| `sorotte-client-core` | 728 | 0 |
| `sorotte-compat` | 149 | 7 |
| `sorotte-core` | 2 | 0 |
| `sorotte-gui` | 1,224 | 1 |
| `sorotte-media-match` | 89 | 1 |
| `sorotte-player-api` | 21 | 0 |
| `sorotte-player-mpv` | 438 | 2 |
| `sorotte-plex` | 68 | 0 |
| `sorotte-protocol` | 94 | 0 |
| `sorotte-secret` | 20 | 0 |
| `sorotte-server` | 392 | 0 |
| `sorotte-sim` | 16 | 4 |
| **Total** | **3,814** | **23** |

This is the count of plain and parameterized Rust test attributes, not a
behavioral coverage percentage. The ignored total is unchanged: the generated
Media Match capability moved from GUI to media-match ownership and is invoked
by its required hosted job rather than by ordinary Cargo execution.

## Local validation completed

| Boundary | Result |
|---|---|
| client timing schedules | 4/4 |
| complete client-core | 728/728 |
| CLI generated campaign | 256/256 cases |
| CLI focused module | 6/6 |
| CLI library / integration | 366 passed, 8 registered ignored / 2 passed |
| complete player-mpv | 427 passed, 2 registered ignored |
| stalled-HTTP native contract | 18 assertions / 11 artifacts |
| compatibility default / strict live | 142/142 / 21/21 (128 filtered) |
| portable external-launch module | 15/15 |
| media-tool version probes | 4/4 |
| nextest process-fixture module | 8/8 |
| strict focused Clippy gates | passed |
| focused workflow policy and actionlint | passed |
| hosted-fix Python regressions | 59/59 before review hardening; coverage/parser policy subset 52/52 after hardening |
| external mpv early-exit stress | 50/50 before and after role-specific oracle hardening |
| server publication policy | exact PowerShell script and YAML-aware structural policy passed |
| `cargo fmt --all --check` / `git diff --check` | passed / passed |
| complete Python policy/infrastructure suite | 525/525 |
| behavior / ignored / known-defect policy | 20 behaviors and 51 proofs / 23 exact ignores / 0 defects and 0 characterizations |
| mutation policy | 10 shards; 17 accepted unviable mutations |
| locked workspace Clippy, all targets/features | passed with warnings denied |
| locked workspace tests, all features and doctests | passed |
| GUI semantic suite | 14/14 |
| Windows native GUI smoke | passed; complete required scenario inventory |
| final committed-source compatibility | 149 listed; 142 passed; 7 ignored; 0 failed/skipped |
| final committed-source WSL ASan framing fuzz | 326,303 executions; 0 artifacts; source/seeds stable |
| exact Windows process coverage producer | 54/54; physical map 2,518 / 161,761 |
| Linux/Windows changed-line union replay | 82.52% overall; 80.13% ordinary; 90.79% critical; 0 unmapped |

The source-bound compatibility report is
`target/verification/compat-live-committed-829ab98-v1.json`. It validates
source and expected source
`829ab9824d20bc64b03179646c5e182d5c7a4bfb`, the immutable Syncplay oracle
`d1c5f85af377c960c5a940707c4d01bc84fd9c3f`, complete 149-test accounting,
and 48.529611 seconds of execution. Bundle identities are:

```text
be641cf0b556e424aede4adf5b848983c3c6aecade388163882cf7328b30b285  report
b0b11465dfd99640cc1a2be2e9458b1cc230579d953917d8c6c9876f6bda9ff3  stdout
96d63126df9b96f39864a6a7b322f70bc7014ad9eab9ee5114f26ce458a417a7  stderr
```

The canonical framing campaign is
`target/fuzz-ci/mpv-framed-transcript-deep-829ab98-wsl-v1`. In 180 seconds it
executed 326,303 units at 1,802/sec, added 3,220 units, retained 1,190 corpus
files / 66,395 bytes, reached 451 MiB peak RSS, and produced zero artifacts.
All 65 source bindings and all 12 seeds remained stable. Bundle identities are:

```text
cf32c5060accd566f51d5154a2bf30cd7d564009b17ed9152468d09cf1b2b65f  run-report.json
48bdc7bbd2a355458ef1799b6013efc04bf92724748e2c8554f45d7cbee3d55b  fuzz.log
e9f946720bf6576a8133eddc92d54df7c6eff660daa31ef338e90834e1c0d987  final corpus aggregate
4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945  empty artifact aggregate
```

The generated-media body did not execute locally because ffmpeg/ffprobe were
not on PATH. Its compile, ordinary crate tests, registry/catalog policy, and
strict Clippy passed locally; hosted job `91111808305` supplied the successful
real-tool proof.

The final implementation-source post-gate real-mpv campaigns used one rebuilt
GUI and one installed mpv identity. They ran only after full source, policy,
semantic, and native-smoke validation, with the valid stalled-read mode last:

| Mode | Bundle | Assertions / artifacts |
|---|---|---:|
| healthy | `target/verification/gui-real-mpv-vertical/20260731T115509993Z-33888` | 13 / 10 |
| owned-process recovery | `target/verification/gui-real-mpv-owned-process-recovery/20260731T115540382Z-33412` | 20 / 13 |
| malformed-HTTP recovery | `target/verification/gui-real-mpv-faulting-http-recovery/20260731T115618285Z-49988` | 18 / 11 |
| valid byte-silent stalled HTTP, run last | `target/verification/gui-real-mpv-stalled-http/20260731T115707208Z-35432` | 18 / 11 |

```text
GUI SHA-256: 439174541d461db90fc66be088152024814e3ba4fe0d0d6b3add464103205d9e
mpv SHA-256: 2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0
```

## Pending hosted and publication acceptance

Local implementation, policy, campaign, and documentation integration are
complete through `829ab9824d20bc64b03179646c5e182d5c7a4bfb` plus this
evidence update. The branch has not yet been promoted as the final remote head
at this checkpoint.

A fresh hosted workflow over the documentation-inclusive source must still
regenerate and consume both platform maps, pass every originating job and the
required aggregate, and reach a successful final conclusion. Until that
occurs, this record claims complete local closure and retained diagnostic
evidence; it does not claim final exact-source hosted completion.
