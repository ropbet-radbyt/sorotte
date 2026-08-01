# Next four test slices — integrated evidence

Date: 2026-07-31

Hosted closure continuation: 2026-08-01 AEST

Branch: `codex/test-coverage-design`

Primary implementation snapshot: `9f3cb60fbe788575829931b56155f4bc0c19caf0`

Platform-map implementation snapshot:
`829ab9824d20bc64b03179646c5e182d5c7a4bfb`

Hosted-harness corrections through:
`dd3012c1bcefa0a68520b063c5ae06f3e1b96f79`

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
`TC-HARNESS-039`. Two later runs exposed `TC-HARNESS-044` through
`TC-HARNESS-046`: aggregate multi-map binding, real-mpv startup/fault phase
ordering, and complete Plex fixture reads. Every correction has a focused
commit and positive local regression or exact downloaded-artifact replay. The
committed implementation-head required-live and WSL fuzz campaigns, 54-test
Windows process map, and local Linux/Windows coverage union are green. Exact
implementation-head workflow `30639113884` also passed every required producer,
the corrected coverage finalizer, and the aggregate. Documentation-inclusive
workflow `30679354953` subsequently finished green at exact workflow-bearing
head `612917ac8461040549217453bdebfc5001f2378c`, closing the former publication
check for this tranche.

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
| `TC-HARNESS-022` non-Windows test helper import | `912970c`, formatting `0dd6c79` | portable external-launch module 15/15; hosted Linux all-feature job `91163394469` passed |
| `TC-HARNESS-023` hosted PowerShell process fixtures | `0b51fb0` | hosted Windows server-release job `91163394510` passed |
| `TC-HARNESS-024` nextest fixture-role collision | `a5ae5be` | hosted Windows nextest completed successfully |
| `TC-HARNESS-025` quoted/dynamic coverage environment | `854a805`, exact-token hardening `bcede9c` | both lane modules pass 42/42; hosted job `91169713196` generated maps and passed policy before TC044 |
| `TC-HARNESS-026` annotated mpv tag object compared with peeled commit | `3e03b70` | actionlint/source policy and hosted mpv job `91163394486` pass |
| `TC-HARNESS-027` non-executable POSIX TLS fixture | `20b02a8` | all three publisher tests and hosted Linux job `91163394469` pass |
| `TC-HARNESS-028` early-exit named-pipe race | `0502fef`, oracle hardening `c7b134e` | exact 50/50 stress, full player suite, and hosted Windows job `91163394472` pass |
| `TC-HARNESS-029` stale/non-exclusive latest-publication assertion | `ee7b9e9`, exclusivity hardening `df7df33` | both local policies and hosted Windows release job `91163394510` pass |
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
| `TC-HARNESS-041` Linux-only map plus overbroad production scope | `829ab98` | local replay passes; hosted job `91169713196` passes 82.55% overall, 80.18% ordinary, 90.79% critical, zero unmapped |
| `TC-HARNESS-042` stale Windows process inventory | `829ab98` | exact 54-test instrumented producer passes with reviewed filtered counts |
| `TC-HARNESS-043` platform-dependent Rust source bytes | `829ab98` | fresh Windows and hosted cross-platform raw-source binding pass; TC044 is the later finalizer defect |
| `TC-HARNESS-044` single-map coverage evidence finalizer rejected a valid union | `2b8af56` | exact downloaded artifacts replay with the complete ordered two-map tuple; omission/reordering/duplication/tampering regressions pass |
| `TC-HARNESS-045` HTTP stall armed during real-mpv startup | `bc5ef9d` | prepared -> started -> armed regression, complete sim suite, and hosted mpv job `91184230570` pass |
| `TC-HARNESS-046` accepted Plex socket read could be incomplete | `dd3012c` | scripted split header, production-path loopback oracle, full CLI package, and 3,777/3,777 hosted nextest cases pass |

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

The final post-build GREEN, run last after the complete locked all-feature
workspace, strict Clippy, actionlint, and the preceding three native modes, is:

```text
target/verification/gui-real-mpv-stalled-http/20260731T150829535Z-48288
```

| Field | Value |
|---|---|
| assertions / artifacts | 18 / 11 |
| GUI SHA-256 | `093fc9315c738eb683cf1cb5aa34c226a69307535e27c86faa088ef3cc7dfaf3` |
| mpv SHA-256 | `2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0` |
| first / recovery bytes | 720,000 / 4,320,024 |
| server-side silence | 29,197 ms |
| cache-stall / recovered position | 7.424 / 8.013835 seconds |
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

Workflow run
[`30632931277`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30632931277)
at `a2441a30f1e98ba85d2384c2986f09b84a5dcb4f` passed every originating
behavior and evidence producer. Coverage job `91169713196` executed the exact
54-test Windows inventory and passed the two-platform policy at 82.55%
combined, 80.18% ordinary, and 90.79% critical with zero unmapped lines. The
coverage job then failed in its evidence-finalization phase because that
finalizer still accepted a single map while the report declared an ordered
union; the downstream aggregate consequently failed (`TC-HARNESS-044`). Commit
`2b8af5672cd27c727f3707b71ccd15a1292135c7` binds the complete ordered
primary-plus-supplemental tuple. The exact downloaded failure replays
successfully under `target/hosted/30632931277/replay-root`; the failed hosted
run remains diagnostic evidence.

Workflow run
[`30636380151`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30636380151)
at `2b8af5672cd27c727f3707b71ccd15a1292135c7` exposed two independent
originating harness failures. mpv job `91174919979` allowed its byte-triggered
stall to begin before both clients reached a healthy startup baseline
(`TC-HARNESS-045`). Windows job `91174920040` ran 3,775 tests, passed 3,774,
and retained a first-attempt Plex connected-session failure followed by a
retry pass; fail-on-flaky correctly returned 100 (`TC-HARNESS-046`). Commit
`bc5ef9dbcff08d194c449e051c8da46424324b8c` adds exact prepared -> started
-> armed real-mpv phases and one globally claimed stall. Commit
`dd3012c1bcefa0a68520b063c5ae06f3e1b96f79` accumulates only complete Plex
headers across transient reads. Neither diagnostic run is relabelled as
acceptance.

Exact implementation-head workflow run
[`30639113884`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30639113884)
at `dd3012c1bcefa0a68520b063c5ae06f3e1b96f79` then passed every required
producer and the aggregate. mpv job `91184230570` passed TC045, Windows job
`91184230464` passed 3,777/3,777 nextest cases without a flaky or rerun element,
coverage job `91190243453` accepted the corrected ordered two-map finalizer at
2,403/2,894 combined (83.03%), 1,841/2,275 ordinary (80.92%), and 562/619
critical (90.79%) with zero unmapped lines, and aggregate job `91192554763`
passed. This is the positive implementation-source acceptance run; the earlier
diagnostic artifacts remain immutable.

No item in this table is a Sorotte product behavior defect. Each correction
retains the original strict assertion or timeout. None is being converted into
a skip, retry-only pass, lowered threshold, or normalized parity exception.

## Current committed-source static inventory

The exact committed implementation snapshot contains:

| Crate | Test attributes | Ignored |
|---|---:|---:|
| `sorotte-cli` | 377 | 8 |
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
| `sorotte-sim` | 17 | 4 |
| **Total** | **3,816** | **23** |

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
| CLI library / integration | 367 passed, 8 registered ignored / 2 passed |
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
| complete Python policy/infrastructure suite | 531/531 |
| behavior / ignored / known-defect policy | 20 behaviors and 51 proofs / 23 exact ignores / 0 defects and 0 characterizations |
| mutation policy | 10 shards; 17 accepted unviable mutations |
| locked workspace Clippy, all targets/features | passed with warnings denied |
| locked workspace tests, all features and doctests | passed |
| GUI semantic suite | 14/14 |
| Windows native GUI smoke | passed; complete required scenario inventory |
| committed implementation-head compatibility | 149 listed; 142 passed; 7 ignored; 0 failed/skipped |
| committed implementation-head WSL ASan framing fuzz | 328,559 executions; 0 artifacts; source/seeds stable |
| exact Windows process coverage producer | 54/54; physical map 2,518 / 161,761 |
| Linux/Windows changed-line union replay | 82.52% overall; 80.13% ordinary; 90.79% critical; 0 unmapped |
| exact downloaded TC044 finalizer replay | ordered Linux/Windows tuple accepted; omission/reordering/duplication/tampering remain rejected |
| exact implementation-head hosted workflow | every required producer, 83.03% / 80.92% / 90.79% zero-unmapped coverage, corrected finalizer, and aggregate passed |
| complete `sorotte-sim` after TC045 | 12/12 |
| Plex fixture TC046 focused / complete CLI package | 3/3 / passed |

The latest source-bound compatibility report is
`target/verification/compat-live-committed-dd3012c-v1.json`. It validates
source and expected source
`dd3012c1bcefa0a68520b063c5ae06f3e1b96f79`, the immutable Syncplay oracle
`d1c5f85af377c960c5a940707c4d01bc84fd9c3f`, complete 149-test accounting,
and 48.280455 seconds of execution. Bundle identities are:

```text
be21707fe709e3bea95568f85dfef78d497643af1b8bd21d4db08ef489599801  report
8b3ebce8ecdc21bb6890f70b9016a46a7dfdcec4e09a42de85bbb93cb5c6c6b0  stdout
6e8e6cb7e00a08ed9f7119fa6e69dcc19730334c94af5a498eca598d13ba7100  stderr
```

The latest canonical framing campaign is
`target/fuzz-ci/mpv-framed-transcript-deep-dd3012c-wsl-v1`. In 180 seconds it
executed 328,559 units at 1,815/sec, added 3,080 units, retained 1,250 corpus
files / 76,115 bytes, reached 452 MiB peak RSS, and produced zero artifacts.
All 65 source bindings and all 12 seeds remained stable. Bundle identities are:

```text
85c6b70b53910b9d905bf7e0ba9135d286a953ba539fad4249ad44c1b43659db  run-report.json
c5e91640577e2cddde21ce1a00329db0642399759d6b7fb2e6ac8f301a08811c  fuzz.log
8f67fe7dcc8fbcf844c4f9f383054d7126111e95c12f368e37f976b07ca2705d  final corpus aggregate
4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945  empty artifact aggregate
```

The generated-media body did not execute locally because ffmpeg/ffprobe were
not on PATH. Its compile, ordinary crate tests, registry/catalog policy, and
strict Clippy passed locally; hosted job `91111808305` supplied the successful
real-tool proof.

The final post-build real-mpv campaigns used one rebuilt
GUI and one installed mpv identity. They ran only after full source, policy,
semantic, and native-smoke validation, with the valid stalled-read mode last:

| Mode | Bundle | Assertions / artifacts |
|---|---|---:|
| healthy | `target/verification/gui-real-mpv-vertical/20260731T150646167Z-51104` | 13 / 10 |
| owned-process recovery | `target/verification/gui-real-mpv-owned-process-recovery/20260731T150721289Z-5208` | 20 / 13 |
| malformed-HTTP recovery | `target/verification/gui-real-mpv-faulting-http-recovery/20260731T150757355Z-3800` | 18 / 11 |
| valid byte-silent stalled HTTP, run last | `target/verification/gui-real-mpv-stalled-http/20260731T150829535Z-48288` | 18 / 11 |

```text
GUI SHA-256: 093fc9315c738eb683cf1cb5aa34c226a69307535e27c86faa088ef3cc7dfaf3
mpv SHA-256: 2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0
```

## Documentation-inclusive hosted acceptance

Implementation source and focused corrections are committed and pushed through
`dd3012c1bcefa0a68520b063c5ae06f3e1b96f79`. The fresh source-bound
compatibility and framed-mpv campaigns above use that exact implementation
head. Exact-head workflow `30639113884` passed every originating job, the
corrected two-map coverage finalizer, and the required aggregate.

Commit `289845ad0eafd8ff94f90b6020818aecb63560f2` then committed the integrated
documentation and evidence. Later workflow-bearing head
`612917ac8461040549217453bdebfc5001f2378c` retained that complete record and
added only bounded CI topology, verifier, and action-runtime changes.
Documentation-inclusive workflow `30679354953` regenerated and consumed both
platform maps, passed every originating job and aggregate, and reached a final
successful conclusion after rerunning one failed Windows server-release job.
Attempt 1 retained a legacy Python playlist-observation timeout; the complete
job passed on attempt 2 without a source change. The final suite has zero
annotations and nine nonexpired evidence artifacts.

This closes the historical exact-source publication boundary for the tranche.
It does not promote the separately unexecuted interactive Windows,
native-Windows minimum/newest mpv, privileged block-replay, or public GHCR
capabilities. Exact chronology, timing, retained failure, and artifact identity
are recorded in
[`hosted-ci-closure-20260801.md`](hosted-ci-closure-20260801.md).
