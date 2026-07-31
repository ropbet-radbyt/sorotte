# Test coverage implementation findings

Original review: 2026-07-28
Lean-fix implementation update: 2026-07-29
Merged-profile implementation update: 2026-07-29
Compatibility-remediation update: 2026-07-29
Deterministic clock implementation update: 2026-07-30
Process-interruption persistence update: 2026-07-30
Outstanding-defect closure update: 2026-07-30
Deep-boundary testing update: 2026-07-30
Atomic TLS and parallel-boundary update: 2026-07-30
Raw-loopback framing update: 2026-07-30
Parser property/corpus, worker fault, config, and ping mutation update: 2026-07-30
Provenance-bound parser fuzz, configuration properties, and mutation-ratchet update: 2026-07-30
Final CLI/protocol defect resolution update: 2026-07-30
Framed transport, migration, updater handshake, and CLI mutation update: 2026-07-30
Controlled-room, platform-syscall, playlist-mutation, and framed-session fuzz update: 2026-07-30
Bounded native, real-mpv, disposable-replay, and container-publication update: 2026-07-31
Required-live, owned-process recovery, updater durability, and framed-mpv update: 2026-07-31
Generated compatibility, Unix kernel IPC/durability, and faulting-HTTP update: 2026-07-31
Client timing, generated Media Match, CLI composition, stalled-HTTP, and hosted-harness update: 2026-07-31
Coverage-finalizer update: 2026-07-31
Real-mpv arming and Plex fixture completion update: 2026-08-01

Branch: `codex/test-coverage-design`

Original experimental base: `a08a06ea7c6cada5413b0dba73b16f940cfd46e1`
Current rebased base: `f3964ebc7f7b281b9b78f3bfb243ff65e5122e33`

This ledger separates product findings from failures in the new test
infrastructure. The original review deliberately left surfaced defects
unchanged. The 2026-07-29 update implements the non-controversial lean
solutions, applies the subsequently selected lifecycle and native-GUI
decisions, and converts every product-defect characterization into a positive
regression. Later reconnect, TLS-rotation, protocol, process-supervision, and
updater experiments opened six additional defects. The 2026-07-30 closure
update implements all six, converts all eight expected-failure
characterizations into positive regressions, and leaves the executable defect
registry explicitly empty at that checkpoint. The subsequent deep-boundary
slice opened `TC-SERVER-004`; the current slice resolves it with an atomic
authenticated generation protocol and executable publisher proof. Parallel
adversarial reset/protocol/media-process work opened five narrow
characterizations: `TC-CLIENT-002`, `TC-PROTOCOL-002`/`003`, and
`TC-GUI-001`/`002`. The subsequent Plex selection/retry slice added
`TC-PLEX-001` and `TC-GUI-003`. This remediation slice fixes all seven,
converts their characterizations to positive regressions, and restores an
explicitly empty defect registry.
The later raw-loopback framing slice opened `TC-CLI-003` with two deterministic
characterizations. A connected-session `select!` can cancel a partially
completed inbound read after bytes have been consumed from the transport,
discarding the future-local prefix before CRLF. That defect remains
deliberately unfixed in that testing slice; at that checkpoint the registry
contained one defect and two exact characterizations.
The subsequent four-slice continuation added deterministic protocol
property/corpus tests, actor-owned persistence fault injection, and
zero-survivor configuration and ping mutation gates. These experiments found
no new product defect and did not modify `TC-CLI-003`; that checkpoint
therefore remained at one open defect and two characterizations.
The next continuation added zero-survivor persistence-arbitration and inbound
`Set` ordering mutation ratchets, black-box composition properties for all 30
environment-overridable settings, and a source-bound coverage-guided protocol
parser lane. The first executable parser campaign found
`TC-PROTOCOL-004`, an adjacent finite floating-point representation change
across raw and typed decode/encode/decode. Production behavior remained
unchanged at that checkpoint, which contained two open defects and four exact
characterizations.
The final defect-resolution slice moves the fragmented-line buffer to
connected-session ownership and enables serde_json's exact float-roundtrip
parser. All four former characterizations are now positive regressions, the
fuzz oracle is unconditional again, and the current registry is explicitly
empty.
The final workspace gate independently reproduced `TC-HARNESS-016`, a race in
the updater test's boundary-marker handshake. The subsequent continuation
resolves it with atomic marker publication and exact content acknowledgement.
That continuation also adds generated legacy-configuration migration
properties, generated framed-transport/session schedules, and a source-bound
CLI framing mutation ratchet. None of those coverage slices surfaced an
independent product defect. The mutation baseline did expose
`TC-HARNESS-017`, an unbounded test-helper loop and three adjacent
payload-limit oracle gaps; the bounded frame/EOF and exact LF/CRLF seams close
it in the same continuation.
The latest additional four-slice continuation adds independent-model
controlled-room configuration properties, real Windows and Unix SQLite
pathname-denial/recovery probes, a zero-survivor playlist shuffle/undo mutation
ratchet, and source-bound coverage-guided in-memory framed-session testing. The
short and canonical randomized campaigns were green, no red artifact or
independent product behavior surfaced, and no production fix was required.
The current product-defect registry remains explicitly empty.
The 2026-07-31 four-slice system-boundary continuation adds an externally
attested interactive-runner workflow contract, one executed native
GUI-to-real-mpv healthy vertical, a disposable block-replay capability, and a
build-once/load-first server-container publication contract. Only the local
real-mpv vertical executed its full capability path. The external interactive
runner, privileged block replay, and Docker/Syft/Cosign/GHCR chain remain
unexecuted and are not reported as green runtime evidence. No product behavior
changed and the product-defect registry remains explicitly empty.
The next 2026-07-31 continuation completed four more bounded slices. Required
live compatibility now has exhaustive skip-free accounting; native real-mpv
coverage now crosses one attested automatic owned-process replacement; updater
transactions now request parent-directory durability at every owned
directory-entry mutation; and a third source-bound randomized target covers
framed mpv IPC, transcript projection, and lifecycle fencing. The updater
characterization surfaced `TC-UPDATER-002`, which is fixed and positive in the
same tranche. The preserved framed-target and committed-run compatibility REDs
were independent oracle/wrapper defects, not product defects. The registry
remains explicitly empty.
The next bounded continuation adds a 256-case generated Rust/Python
JSON-framing differential, a real Linux updater directory-sync denial, 14
real-kernel Unix-domain-socket mpv IPC schedules, and a strict
faulting-loopback-HTTP real-mpv vertical. The first three slices found no
product defect. The native vertical exposed `TC-GUI-004`, `TC-GUI-005`, and
`TC-PLAYER-004`.
Production now admits trusted direct HTTP(S) media as an automatic player
candidate and fences an accepted same-target load across command completion,
pre-activation path observations, and GUI row-identity reprojection. Both
findings are positive regressions and the current product registry remains
explicitly empty.
The merged-profile work subsequently surfaced one intermittent player
observation failure and six strict legacy-parity failures. The remediation
slice isolated their ownership, fixed the product and harness defects, added
one ordering defect found by the strengthened oracle, and converted every case
into positive regression evidence. No expected failure, compatibility
exception, retry, or skip is used to make the required lane green.
The current four-slice continuation adds deterministic client jitter/drift and
playback schedules, promotes generated-media Media Match diagnostics into a
required capability lane, crosses the legacy CLI parser with a 256-case
configuration-composition oracle, and adds a distinct valid-framing,
byte-silent real-mpv recovery vertical. The CLI slice exposed and fixed
`TC-CLI-004` and `TC-CLI-005`; the native vertical exposed and fixed
`TC-PLAYER-005`. The independently approved final-source proof and the final
implementation-checkpoint stalled-read campaign are green. The first hosted diagnostic run also exposed
`TC-HARNESS-018` through `TC-HARNESS-024`. The second diagnostic proved the
corrected generated-media, complete live-compatibility, semantic, lifecycle,
Ubuntu server-release, and Windows nextest lanes, then exposed
`TC-HARNESS-025` through `TC-HARNESS-029`. Later fail-closed execution exposed
`TC-HARNESS-030` through `TC-HARNESS-038` and `TC-HARNESS-040` through
`TC-HARNESS-046`; `TC-HARNESS-039` records the noncanonical native-Windows
ASan environment diagnostic. All 29 hosted-continuation findings
(`TC-HARNESS-018` through `TC-HARNESS-046`) have focused dispositions and
positive regressions or exact-artifact replay. Their exact implementation-head
hosted confirmation passed; documentation-inclusive acceptance remains the
final publication boundary. None is a product behavior defect, and the
product-defect registry remains explicitly empty. `TC-HARNESS-044` binds the coverage
finalizer to the ordered two-platform map tuple; `TC-HARNESS-045` arms the
minimum-mpv HTTP stall only after both clients reach exact prepared and started
baselines; and `TC-HARNESS-046` waits for a complete Plex request header across
transient socket reads.

## 2026-07-31 bounded system-boundary tranche

Status: **Implemented with explicit capability boundaries; no product defect
surfaced**

The four slices deliberately separate repository policy from executed system
evidence:

- The native workflow requires an externally provisioned one-job ephemeral
  interactive Windows runner, checks its desktop before checkout, and retains
  the exact strict ten-scenario inventory. No matching runner was available,
  so the workflow is dispatch-only and is not yet a required gate.
- The genuine GUI-to-real-mpv lane passed locally with the exact GUI and mpv
  binaries digest-bound, generated local media, isolated configuration,
  IPv4-loopback session endpoints, exact client/server Hello objects, physical
  Open Media and Exit leaves, ordered real-mpv Play/Pause observations, and
  natural GUI/mpv/session cleanup. Its canonical contract has exactly 13
  assertions and 10 hashed artifacts. Missing mpv fails before build or GUI
  launch.
- The persistence capability accepts no caller device, image, mount, mapper,
  or work path. It creates only nonce-owned sparse files below `/var/tmp`,
  binds ordered data/log mapper operands to their recorded loop identities,
  and encodes complete-old-or-complete-new recovery at three replay cuts. The
  nonprivileged policy and plain-file worker model passed; the privileged
  `dm-log-writes` path did not run, so no power-loss result is claimed.
- The container workflow encodes a single loaded image identity across
  non-root plaintext/TLS restart persistence, SPDX generation, tag-only push,
  keyless signature/attestation, logout, and anonymous public GHCR comparison.
  The offline policy passes, but the image and publication capabilities did
  not run locally and remain CI-owned.

Iteration exposed test-infrastructure weaknesses rather than product defects:
foreground acquisition needed a bounded Windows input-queue attachment;
equivalent extended-length Windows paths needed narrow canonical handling; the
real-mpv validator initially accepted incomplete Hello/assertion identity; the
device-mapper guard initially proved only an unordered dependency set rather
than data/log roles; and the first container draft checked SQLite integrity
without proving a distinctive protocol write/restart/restore or rescanning the
exact SBOM source tag. Each oracle was strengthened before this tranche was
accepted. Failed native exploratory bundles were retained rather than
normalized away.

Exact commands, identities, red evidence, limitations, and future execution
steps are retained in:

- [`native-interactive-ci-contract-20260731.md`](evidence/test-coverage/native-interactive-ci-contract-20260731.md)
- [`native-gui-real-mpv-vertical-20260731.md`](evidence/test-coverage/native-gui-real-mpv-vertical-20260731.md)
- [`persistence-disposable-block-replay-harness-20260731.md`](evidence/test-coverage/persistence-disposable-block-replay-harness-20260731.md)
- [`server-container-build-load-publication-contract-20260731.md`](evidence/test-coverage/server-container-build-load-publication-contract-20260731.md)

## 2026-07-31 required-live, recovery, updater, and framed-mpv tranche

Status: **Implemented, source-bound, and locally green; one product durability
defect found and fixed**

The four slices and their independent findings are:

- The required-live wrapper pins the local Syncplay oracle commit, supported
  CPython family, exact package versions, both probes, and all 89 tracked
  fixtures. It discovers 143 tests, executes all 136 non-writing tests
  serially with zero skips, and accounts for the seven exact fixture writers.
  A preserved first committed-source run found a wrapper defect: the relative
  oracle path was attested from the repository but passed unchanged to Cargo,
  where crate-local working-directory resolution made 61 live tests fail.
  Passing the absolute already-attested path fixed the wrapper; the next
  source-bound matrix passed 136/136 in 47.394740 seconds.
- The native real-mpv recovery inventory terminates only an exact
  path/digest/parent-attested GUI child, observes product-owned automatic
  replacement with a new PID and IPC endpoint, reopens generated media through
  physical UI, observes replacement Play/Pause, rejects stale/foreign
  post-boundary events, and uses native Exit to reap both player generations.
  Its preserved RED disproved the assumed manual Retry modal because the
  active-session runtime automatically relaunched the managed player.
  Production behavior did not change.
- `TC-UPDATER-002` proved that flushed transaction files were not followed by
  containing-directory sync after entry creation, replace/rename, rollback,
  cleanup, or journal deletion. The updater now requests that boundary. A
  13-schedule disk-full/access-denied analogue matrix, real reversible Windows
  directory-share denial, 33/33 updater suite, all 11 process-termination
  boundaries, and the two installed-updater integration tests are positive.
- The framed-mpv target crosses the production line reader through a bounded
  in-memory transport, then transcript and lifecycle projections. Four chunk
  schedules, five terminal modes, and 12 seeds drive an independent oracle.
  Its first RED proved the reference incorrectly decoded trailing partial
  bytes on read disconnect; production only does so on EOF. After narrowing
  only that oracle rule, the committed-source 180-second campaign passed
  322,973 executions with 3,219 new units, stable 64-file source and 12-file
  seed bindings, and zero artifacts or evidence errors.

The compatibility and framed-mpv REDs were retained and were not normalized
into green results. Neither required a product behavior change. The updater
finding is resolved by production code and an ordinary positive regression,
so `coverage/known-defects.toml` remains explicitly empty.

Exact reports, hashes, commands, prerequisites, and limitations are retained
in:

- [`compat-required-live-interop-20260731.md`](evidence/test-coverage/compat-required-live-interop-20260731.md)
- [`native-gui-real-mpv-owned-process-recovery-20260731.md`](evidence/test-coverage/native-gui-real-mpv-owned-process-recovery-20260731.md)
- [`updater-transaction-storage-durability-20260731.md`](evidence/test-coverage/updater-transaction-storage-durability-20260731.md)
- [`framed-mpv-ipc-transcript-coverage-guided-20260731.md`](evidence/test-coverage/framed-mpv-ipc-transcript-coverage-guided-20260731.md)

## 2026-07-31 generated compatibility, Unix kernel, and faulting-HTTP tranche

Status: **Implemented and source-bound; two GUI product defects found and
fixed**

The four independently bounded slices are:

- A fixed-seed 256-case Rust/Python differential drives accepted,
  malformed-JSON, and malformed-UTF-8 byte lines through Sorotte and the actual
  pinned Syncplay `JSONCommandProtocol`. All cases matched. Its implementation-
  commit required-live report lists 144 tests, executes/passes 137, skips zero,
  and accounts for seven fixture writers.
- A Linux-only updater regression reaches the production directory
  read-open/`sync_all` path after a real rename under mode `0300`. It receives
  `EACCES`, retains authenticated old-state recovery, restores exact
  permissions, cleans every transaction artifact, and re-enters idempotently.
- Nine Linux tests execute 14 real Unix-domain-socket schedules through the
  production mpv client and worker: fragmentation, coalescing, correlation
  errors, malformed/truncated/EOF frames, half-close, timeout, path
  replacement, request-ID wraparound, worker shutdown, and namespace cleanup.
- The native real-mpv inventory serves generated PCM from an ephemeral strict
  IPv4-loopback HTTP endpoint, faults one paced chunked response with an
  invalid chunk-size boundary, requires mpv's early keep-open EOF signal and
  one complete recovery response, and retains exact
  GUI/mpv/session/IPC/media identity and resource release. It exposed
  `TC-GUI-004`, `TC-GUI-005`, and `TC-PLAYER-004`; all three are production
  fixes with ordinary positive regressions.

The native iteration retained every RED. Early failures hardened default
session-command allowlisting, dynamic-ping shape checks, exact four-frame
playlist exchange, endpoint rebind proof, integer-vs-boolean JSON schema
checks, partial HTTP-write evidence, and method/range/status accounting.
Product REDs separately proved the missing direct-URL candidate, repeated
same-target `loadfile` behavior, and the absence of `end-file` under
`keep-open=always` after an early malformed-transfer EOF. Healthy and
owned-process real-mpv contracts remain unchanged, and the current
known-defect registry remains empty.

Exact reports, hashes, commands, preserved REDs, and limitations are retained
in:

- [`compat-generated-json-framing-differential-20260731.md`](evidence/test-coverage/compat-generated-json-framing-differential-20260731.md)
- [`updater-linux-parent-directory-sync-real-syscall-20260731.md`](evidence/test-coverage/updater-linux-parent-directory-sync-real-syscall-20260731.md)
- [`player-unix-socket-ipc-kernel-20260731.md`](evidence/test-coverage/player-unix-socket-ipc-kernel-20260731.md)
- [`native-gui-real-mpv-faulting-http-recovery-20260731.md`](evidence/test-coverage/native-gui-real-mpv-faulting-http-recovery-20260731.md)

## 2026-07-31 client timing, generated-media, CLI composition, and stalled-HTTP tranche

Status: **Four primary slices implemented; three product defects found and
fixed; exact implementation-head hosted acceptance is green**

The four bounded slices are:

- Four deterministic client-core schedules cross explicit ping observation,
  separate receipt/reply clocks, affine drift, outliers, nonmonotonic and
  non-finite values, scheduler delay, room projection, and legacy playback
  decisions against an independent arithmetic/state oracle. They use no sleep,
  socket, process, or wall-clock dependency. The focused 4/4 schedule and full
  728/728 client-core suite passed without a product or harness finding.
- The former optional generated-media check is now a required Ubuntu
  `sorotte-media-match` integration target using real ffmpeg/ffprobe, generated
  local media, fixed report time, and a closed fingerprint/retrieval/decision
  oracle. `TC-HARNESS-018` corrected the first hosted fixture and its opaque
  failure diagnostic without lowering the `Probable` threshold. The corrected
  body passed in exact implementation-head workflow `30639113884`, job
  `91184230481`.
- The CLI parser/composition slice drives 256 fixed-seed cases through real
  environment, stored-setting, argument parsing, and override application.
  `TC-CLI-004` adds explicit per-occurrence replace/clear/invalid semantics,
  accepts attached long/short values, couples host and port atomically, and
  fails closed on missing required values. `TC-CLI-005` removes attached values
  from unknown-option diagnostics. The six focused tests, all 256 cases, the
  complete CLI suite, and strict crate Clippy are positive.
- The fourth native inventory serves a valid `Content-Length` response on
  strict IPv4 loopback, transmits exactly 720,000 bytes, then remains byte
  silent for at least 25 seconds without EOF. `TC-PLAYER-005` preserves the
  causal deferred start/restart edge and permits finite unknown-classified VOD
  to arm recovery only in the absence of positive live evidence. The final
  post-build implementation-source bundle `20260731T150829535Z-48288` passed
  18 assertions and 11 artifacts with
  the same GUI-owned mpv PID/IPC identity, zero pre-recovery EOFs, exactly one
  replacement `end-file` reason `stop`, a complete second GET, and complete
  cleanup. The full player suite passed 427 tests with two registered ignores.

The first hosted run was retained as diagnostic evidence rather than being
normalized into green. In addition to `TC-HARNESS-018`, it found an unquoted
Git revision expression, incomplete Rust component declarations, a
permanent-room oracle startup race, a Linux-only test re-export gate, and two
independent Windows process-fixture assumptions. These are
`TC-HARNESS-019` through `TC-HARNESS-024` below. Focused local regressions and
policy checks pass, and exact implementation-head workflow `30639113884`
subsequently passed every required producer and aggregate without relabelling
the diagnostic failures.

Exact primary evidence and the tranche-level pending/complete ledger are
retained in:

- [`client-ping-jitter-drift-schedules-20260731.md`](evidence/test-coverage/client-ping-jitter-drift-schedules-20260731.md)
- [`media-match-generated-media-capability-20260731.md`](evidence/test-coverage/media-match-generated-media-capability-20260731.md)
- [`cli-argument-configuration-composition-20260731.md`](evidence/test-coverage/cli-argument-configuration-composition-20260731.md)
- [`native-gui-real-mpv-stalled-http-recovery-20260731.md`](evidence/test-coverage/native-gui-real-mpv-stalled-http-recovery-20260731.md)
- [`next-four-test-slices-20260731.md`](evidence/test-coverage/next-four-test-slices-20260731.md)

## Experimental baseline

Before the shrinkable suite was added:

- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
  passed on Windows.
- `cargo test --locked --workspace --all-features` passed on Windows in
  248.9 seconds.
- The existing fixed generator executed 8,192 reducer transitions and passed.
- All eight pre-existing Rust behavior selectors were experimentally confirmed
  to discover exactly one non-ignored test.
- The GUI semantic binary listed exactly 14 scenarios.

## Implementation validation

After the coverage tranche was integrated:

- The current client-timing/generated-media/CLI/stalled-HTTP tranche has
  passed every locally available validation gate at this documentation
  checkpoint. Client-core passed
  728/728; the CLI composition module passed 6/6 including all 256 generated
  cases and its owning library passed 367 tests with eight registered ignores;
  the player suite passed 427 tests with two registered ignores; and the final
  stalled-read native bundle passed its closed 18-assertion/11-artifact
  contract. The committed implementation-head compatibility campaign listed 149,
  passed all 142 executable tests, and accounted for seven ignored writers;
  the complete strict selector passed 21/21 with 128 filtered out. The nextest-specific process module
  passed 8/8. Formatting, warning-denied locked workspace Clippy, locked
  all-feature workspace tests and doctests, 531 Python policy/infrastructure
  tests, the 180-second committed implementation-head WSL ASan campaign,
  54/54 exact Windows
  process coverage tests, a zero-unmapped Linux/Windows union replay, 14/14
  semantic scenarios, and native smoke passed. The final four-mode real-mpv sequence
  passed against one GUI/mpv digest, with stalled bundle
  `20260731T150829535Z-48288` last. Exact implementation-head workflow
  `30639113884` passed every required producer, 83.03% combined / 80.92%
  ordinary / 90.79% critical union coverage with zero unmapped lines, the
  corrected coverage finalizer, and the aggregate. Documentation-inclusive
  hosted acceptance is recorded only at its actual completion boundary.
  The committed-source static inventory now contains 3,816 Rust test
  attributes with 23 exact ignored-test dispositions: CLI is 377/8 and sim is
  17/4; all other crate counts are unchanged from the preceding table.
- The generated-compatibility/Unix-durability/Unix-IPC/faulting-HTTP
  continuation passed its fixed-seed 256-case Rust/Python differential with
  zero mismatches; the committed-source required-live report over `e3d8554`
  listed 144 tests, passed all 137 executable tests, accounted for the seven
  fixture writers, and skipped none. The real Linux updater denial and all
  nine Unix-domain-socket tests covering 14 production IPC schedules passed.
  Focused keep-open recovery passed 15/15 player tests, 16/16 native-harness
  tests, and 19/19 Python contract tests. Final integration passed 504/504
  Python policy/infrastructure tests in 26.328 seconds and the same 504/504
  documentation-sensitive suite again in 27.397 seconds after evidence
  finalization, plus formatting, whitespace, both changed workflows under
  actionlint, the 20-behavior/51-proof catalog, all 23 ignored-test
  dispositions, the ten-shard mutation policy with 17 exact accepted
  compiler-unviable identities, and the explicitly empty
  0-defect/0-characterization registry. Warning-denied locked
  all-target/all-feature workspace Clippy passed in 7.28 seconds; the complete
  locked all-feature workspace passed in 220.1 seconds with only registered
  ignores. At that prior three-mode checkpoint, after every build-producing
  gate, one GUI digest
  `673dda5226c433950d3074cb4f1b2b6d222802eda6e30cc8a9b5d6e0ef12271c`
  passed the healthy 13-assertion/10-artifact bundle
  `20260731T044916649Z-67112`, the owned-process 20-assertion/13-artifact
  bundle `20260731T045019794Z-49868`, and, last at that checkpoint, the
  faulting-HTTP 18-assertion/11-artifact bundle
  `20260731T045105652Z-43360`. The last run
  retained the malformed transfer, one complete recovery request,
  same-PID/same-IPC recovery, and complete player/server/socket release.
- The 2026-07-31 required-live/recovery/updater/framed-mpv continuation passed
  all 496 Python policy and infrastructure tests in 22.380 seconds,
  repository formatting and diff checks, both changed workflows under
  actionlint, the ten-shard mutation policy with 17 exact accepted unviables,
  and the explicitly empty 0-defect/0-characterization registry.
  Warning-denied all-target/all-feature workspace Clippy passed in 15.8
  seconds. The complete locked all-feature workspace suite passed on its first
  attempt in 257.5 seconds, including updater integration, real-Python server
  release verification, and doctests. The committed required-live report
  passed 136/136 executable tests with zero skips; the committed framed-mpv
  ASan campaign passed 322,973 executions with zero artifacts. After every
  build-producing gate, the final real-mpv recovery bundle
  `20260731T000220834Z-11868` passed 20 assertions/13 artifacts, and the same
  `b805d774...` GUI binary passed the healthy 13-assertion/10-artifact bundle
  `20260731T000311349Z-65428`. Both GUI PIDs and all three mpv PIDs were
  independently absent afterward.
- The final four-slice system-boundary integration passed 471/471 Python
  policy and infrastructure tests, warning-denied all-target/all-feature
  workspace Clippy in 23.7 seconds, and the complete locked all-feature
  workspace on its first attempt in 269.3 seconds. Formatting, whitespace,
  actionlint, the ten-shard mutation policy with 17 exact accepted
  compiler-unviable identities, and the explicitly empty product-defect
  registry passed. Focused ownership checks passed 6/6 GUI real-mpv Rust
  tests, 2/2 persistence Rust tests, and all 68 focused Python contract tests.
  After the full workspace build, a fresh canonical real-mpv run passed its
  exact 13-assertion/10-artifact contract and natural lifecycle cleanup.
- The latest four completed slices add 50,240 controlled-room generated cases
  across default/scheduled/stress depths; real Windows and Ubuntu WSL
  production-path denial with 50/50 Windows stress; and a playlist
  shuffle/undo mutation shard that improved from 12/26 viable caught, 12
  missed, and 2 timed out to 26/26 caught with zero misses/timeouts. The
  current scheduled policy therefore validates 10 shards, 484/484 viable
  mutants caught, and 17 exact accepted compiler-unviable identities.
  The fresh framed-session smoke passed 52,492 executions; the clean-source
  canonical campaign over
  `366fe28b18c50ebb5fb66eefae9a3f317ba9e75c` passed 292,528, retained stable
  881-file source and 14-file seed bindings, and produced zero artifacts,
  minimizations, or evidence errors. Focused CLI tests/Clippy, all three
  owning regressions, the 16-test fuzz policy, both changed workflows under
  actionlint, the pinned WSL ASan build, and formatting passed. Final
  integration then passed all 403 Python policy/infrastructure tests,
  warning-denied all-target/all-feature workspace Clippy in 8.56 seconds, and
  the complete locked all-feature workspace on its first attempt in 238.5
  seconds, including integration tests, real-Python release verification, and
  doctests. Exact evidence is retained in
  [`controlled-room-configuration-properties-20260730.md`](evidence/test-coverage/controlled-room-configuration-properties-20260730.md),
  [`persistence-platform-syscall-faults-20260730.md`](evidence/test-coverage/persistence-platform-syscall-faults-20260730.md),
  [`targeted-mutation-client-playlist-shuffle-20260730.md`](evidence/test-coverage/targeted-mutation-client-playlist-shuffle-20260730.md),
  and
  [`framed-session-coverage-guided-20260730.md`](evidence/test-coverage/framed-session-coverage-guided-20260730.md).
- The framed-transport/configuration-migration/updater-handshake continuation
  passed 4/4 generated framing tests, 50/50 post-ratchet framing replays,
  6,144 scheduled migration cases, the exact updater process test 100/100
  times, and the complete updater binary 30/30. The stable CLI framing
  mutation campaign selected 370 package tests and caught 33/33 viable
  mutants with zero misses, timeouts, or unviables. At that checkpoint, the
  scheduled policy validated nine shards with 458/458 viable mutations caught
  and the same 16 exact accepted compiler-unviable identities.
- Final integrated validation passed warning-denied all-target/all-feature
  workspace Clippy in 5.09 seconds, the complete locked all-feature workspace
  on its first attempt in 233.3 seconds, and all 399 Python
  policy/infrastructure tests in 21.203 seconds. Repository formatting,
  whitespace, actionlint, the 20-behavior/51-proof catalog, all 23 ignored-test
  dispositions, and the explicitly empty product-defect registry also passed.
- `cargo fmt --all --check` passed.
- The final provenance/mutation/property/fuzz continuation passed all 84
  combined mutation, CI, known-defect, and fuzz-policy tests. The shared
  mutation policy validates eight scheduled shards and 16 exact accepted
  compiler-unviable identities; its checked-in source-bound results catch all
  425 viable mutations with zero misses or timeouts. The focused owning
  selectors pass 7/7 persistence-arbitration tests and 38/38 client protocol
  tests, while the configuration composition suite passes 6,144 scheduled
  generated cases. The complete protocol package passes 88 library and 6
  parser integration tests, and strict protocol Clippy passes.
- The source-bound parser target first exposed and registered
  `TC-PROTOCOL-004`. Its 45-second continuation passed 559,788 executions.
  The pre-fix canonical 180-second campaign over committed SHA
  `729214d0de7ced9c56da7361bda68dc75b831179` passed 1,915,137 executions,
  added 6,634 corpus units, peaked at 519 MiB, retained a stable 29-file source
  manifest and 14-file seed manifest, and produced no artifact or independent
  failure. After the correction, a fresh exact-oracle 180-second campaign over
  `034e10511ae6473f0165f3028a026a0bad4f6db3` passed 1,994,358 executions,
  added 7,163 corpus units, peaked at 533 MiB, retained a stable 29-file source
  manifest and 16-file seed manifest, and produced no artifact. The executable
  known-defect policy now validates zero defects and zero characterizations.
- Final defect-resolution validation passed all-target/all-feature workspace
  Clippy in 22.173 seconds, all 399 Python infrastructure/policy tests in
  20.427 seconds, formatting, diff whitespace, actionlint, all direct
  registries, and a complete locked all-feature workspace retry in 208.298
  seconds. The positive CLI raw-framing matrix passed 50/50 serial runs
  (250/250 test executions), and the 16-file parser corpus passed 50/50 serial
  replays (800/800 files).
- The first complete workspace attempt stopped after 125.962 seconds at
  `TC-HARNESS-016`. Its exact updater test passed on immediate retry, failed at
  iteration 5 of a serial stress, and then passed a 20/20 diagnostic capture.
  The unchanged complete workspace retry passed; the initial failure remains
  recorded rather than being hidden by the green retry.
- The preceding tranche's final validation passed warning-denied
  all-target/all-feature workspace
  Clippy in 15.65 seconds. The complete locked all-feature workspace,
  including integration tests, the real-Python server release verifier, and
  every doctest, passed on its first run in 250.8 seconds. All 399 Python
  infrastructure and policy tests passed in 20.383 seconds; formatting,
  diff whitespace, both changed workflows, the eight-shard mutation policy,
  and the then-current registry also passed their exact gates.
- The parser/worker/configuration/ping checkpoint passed every focused and
  broad gate on the integrated tree. The protocol parser suite passed 6/6 at
  the scheduled 6,144-case depth; the worker-owned persistence family passed
  9/9; configuration passed 14/14; and ping passed 8/8. The complete owning
  packages pass 86 protocol unit plus 6 parser integration tests, 358 server
  library plus 14 binary-unit, 2 binary-integration, and 6 release-verification
  tests, 185 client-app tests, and 715 client-core tests.
  Source-bound mutation reports catch 98/98 viable configuration mutants and
  47/47 viable ping mutants with zero misses or timeouts; its six scheduled
  shards caught 395/395 viable mutations. The policy validated 14 exact
  accepted-unviable identities, while ping requires none.
- The exact integrated `cargo test --locked --workspace --all-features` gate
  passed on its first attempt in 217.002 seconds, including integration tests,
  the real-Python server release verifier, and every doctest. Warning-denied
  all-target/all-feature workspace Clippy passed in 10.606 seconds.
  All 386 Python policy/infrastructure tests passed in 19.436 seconds;
  actionlint accepted the changed workflow, and the executable known-defect
  policy at that checkpoint exactly matched one open defect and two
  characterizations.
- The four-slice 2026-07-30 checkpoint passed its complete owning surfaces:
  445/445 client session tests, 712/712 client-core tests, 356 CLI library
  tests with the same eight declared ignores, and 355/355 server library tests
  plus all server binary and release-verification tests. The complete locked
  all-feature workspace, including integration tests and doctests, passed in
  239.424 seconds; warning-denied all-target/all-feature workspace Clippy
  passed in 7.36 seconds.
- At that checkpoint all 386 Python policy/infrastructure tests passed.
  Actionlint accepted both changed workflows, the retained 34-profile Windows
  process report revalidated, the mutation policy validated four shards and
  11 exact accepted-unviable rewrites, and the executable known-defect policy
  exactly matched one open defect and its two characterizations.
- The deep-boundary tree passed the locked all-feature workspace in 205.1
  seconds after resolving `TC-HARNESS-015`, warning-denied all-target/
  all-feature workspace Clippy in 7.07 seconds, all 354 infrastructure/policy
  tests in 13.910 seconds, actionlint, and the exact one-defect/
  one-characterization known-defect policy. Ten complete all-feature CLI
  library stress runs passed 10/10 after the harness repair.
- The outstanding-defect closure passed all complete owning-crate suites:
  protocol 77/77, client-core 699/699, CLI 346 passing with its 8 declared
  ignores, and updater 22/22; the complete server package also passed. The
  exact locked all-feature workspace, including the real-Python release
  verifier and every doctest, exited 0 in 229.110 seconds.
- On that closure tree, all 354 Python policy/infrastructure tests passed in
  13.920 seconds, the explicit zero-defect/zero-characterization registry
  validated, and actionlint 1.7.12 reported no workflow error.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
  passed on Windows before the rebase, on the rebased 0.2.4 tree in 27.2
  seconds, after the lean fixes in 8.54 seconds, and on the final tree in 7.33
  seconds. The outstanding-defect closure passed the same gate in 13.23
  seconds.
- After the lean fixes, the authoritative
  `cargo test --locked --workspace --all-features` run passed in 180 seconds,
  including the real-Python server release verifier and every doctest. Two
  earlier broad candidates correctly exposed implementation regressions in
  the new shared policy: embedded `acceptedOperationId` redaction and legacy
  uncorrelated probe commands. Both were corrected and are now covered by the
  protocol suite and the six-test release verifier respectively. The exact
  final tree passed the same complete gate in 208 seconds.
- Before the rebase, `cargo test --locked --workspace --all-features` passed
  on retry in 181.7 seconds after the first attempt exposed TC-HARNESS-004.
  The complete rebased workspace, including doctests, then passed on its first
  attempt in 247.4 seconds. The final mutation-testing slice passed the same
  complete gate on its first attempt in 210.4 seconds.
- The repaired broad reducer-input property passed 10,000 generated cases in
  3.11 seconds without a defect classifier or unchecked reducer seam. Both
  `TC-PLAYER-001` successor-conflict variants and all former `TC-PLAYER-002`
  histories are positive regressions.
- One complete semantic run passed all 14 scenarios against Syncplay commit
  `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`, Twisted 25.5.0, pyOpenSSL
  25.3.0, and service-identity 24.2.0. A historical rerun exposed the
  intermittent 13/14 result recorded as TC-HARNESS-003; the pre-rebase final
  replay passed 14/14 in 29.7 seconds and the post-rebase replay passed 14/14
  in 39.2 seconds. Final validation passed 14/14 in 30 seconds.
- All 247 Python infrastructure tests passed on the completed mutation slice
  after the initial native-coverage work, and the lean-fix tree passed 248.
  The completed native identity/outcome/artifact implementation passed 252.
  After explicit LCOV dual-model and empty-known-defect policy coverage, all
  257 pass in 11.87 seconds: fail-closed evidence, parsed workflow policy,
  changed-line coverage, ignored/known-defect policy, strict
  native-contract/watchdog, explicit process-environment forwarding, package
  timestamp policy, and targeted mutation cases.
  The 26 mutation-specific cases cover strict policy, source/package binding,
  producer/version/command ownership, inventory and status reconciliation,
  artifact traversal, duplicate keys, timestamps, phase arguments, source
  drift, thresholds, and expiring unviable exceptions.
- The targeted `sorotte-secret` experiment held the mutation inventory at 44
  while improving viable kills from 22/43 (51.16%) to 43/43 (100.00%) using
  seven test-only oracles. No product defect surfaced and no production
  behavior changed during that mutation experiment. One compiler-infeasible
  const-context mutation is explicitly matched and expires for review on
  2026-10-31; exact proof is in
  `docs/evidence/test-coverage/targeted-mutation-20260729.md`.
- The behavior catalog validates 20 behavior IDs, 51 exact proofs, and two
  lanes. Before the closure slice, the executable registry contained six open
  defects and eight exact characterizations. It now validates as zero defects
  and zero characterizations at the closure checkpoint; each former expected
  failure is an ordinary positive regression at its owning boundary. The
  deep-boundary checkpoint validated as one open defect and one exact
  characterization for `TC-SERVER-004`; the subsequent remediation registry
  removed it and exactly matched seven reset/protocol/media-process/Plex
  characterizations. Those seven and the final CLI/protocol pair are now
  resolved, so the current registry is empty.
- The deterministic TLS model passed 10 consecutive runs: 2,430 generated
  histories and 12,150 checked transitions. Its in-flight real-network,
  restoration, and retry-cap selectors each passed 50/50 replays. The
  production-filesystem collision characterization passed 25/25 replays, and
  the complete server library suite passed 332/332 tests.
- The persistence crash matrix terminates a dedicated child process at 15
  production transactional boundaries and reopens each database in the parent.
  All five contracts passed 20 consecutive serial stress runs: 300 child
  process interruptions, 240 complete persistence-actor test executions, and
  no failed integrity, atomicity, or idempotence assertion. The complete
  server-library `persistence` selector passes 49/49 tests. Final locked
  all-feature workspace validation passed on its first run in 200.7 seconds,
  including 338/338 server library tests; full-workspace Clippy passed with
  warnings denied in 6.96 seconds.
- The strengthened known-defect policy corrected the TLS identifier from the
  already-used `TC-SERVER-001` to `TC-SERVER-003`, rejects duplicate finding
  headings and title drift, and now inventories multiline Rust
  `should_panic(expected = ...)` attributes. It now also requires each panic
  oracle to start with its own defect identifier; all 22 focused policy tests
  pass. The historical populated registry, the closure checkpoint's explicit
  zero-defect registry, the raw-framing checkpoint's single-defect registry,
  the former two-defect checkpoint, and the current empty registry all use the
  same fail-closed contract.
  The complete infrastructure suite at the earlier checkpoint passed 295/295
  tests in 12.421 seconds.
- The ignored-test registry exactly classifies all 23 source attributes:
  4 required pull-request proofs, 7 fixture-maintenance commands, and 12 manual
  capability tests. The two compatibility quarantines were retired after their
  timeout harness defect was fixed.
- The changed-line utility now passes all 71 LCOV/diff-policy cases; the
  canonical-map consumer passed 9 additional adversarial cases, the native
  converter passed 14, and the six-phase finalizer passed 19. Coverage
  includes immutable base/head critical-policy union, policy-deletion
  downgrade prevention, inline `#[cfg(test)]` denominator dilution,
  adversarial Rust lexical tokens, new-tag, updated-tag, missing-base,
  merge-base, provenance, and partial-phase failure contracts. The schema-2
  merged-profile collector, finalizer binding, stale-profile reset,
  lane-oracle, and workflow-policy additions bring the complete
  infrastructure suite to 284 passing tests in 11.046 seconds. The
  compatibility promotion and CI/release policy contracts bring the current
  suite to 290 passing tests in 12.394 seconds.
- Deterministic protocol ordering passed 3 new permutation/adversarial tests;
  all 6 production-worker framed IPC tests passed; the current 49-test server
  persistence selector includes positive atomic-migration,
  concurrent-secret-convergence, and process-interruption regressions.
- actionlint 1.7.12 reported no workflow syntax or expression errors before
  the final nextest workflow wiring. `actionlint` and Go were unavailable for
  the final replay; workflow parsing and adversarial mutation checks remain in
  the passing Python suite.
- `scripts/release-publication-policy-tests.ps1` passed under Windows
  PowerShell and PowerShell 7 after the rebase. After timestamp normalization,
  the 0.2.4 package-path suite also passes under both shells.
- An exact-final-source Windows cargo-llvm-cov 0.8.4 run passed the complete
  instrumented workspace in 235.1 seconds. Its LCOV artifact contains 395
  records, of which 310 have an `LF` or `LH` contradiction. The repaired
  diagnostic parser preserved both aggregate models exactly and evaluated the
  current diff from unique `DA` source lines; an independent PowerShell audit
  matched every count.
- The exact pinned nextest wrapper retained 3,458-test JUnit evidence and
  correctly remained red when TC-HARNESS-006 leaked a handle on its first
  attempt but passed its retry. After stdio isolation, the exact recovery test
  passed eleven consecutive pinned-nextest leak-policy runs.
- The completed TC-NATIVE-001 slice passes warning-denied Clippy for the whole
  all-feature workspace, all 1,109 non-ignored GUI library tests plus GUI
  binary/integration/doctest targets, the now automatically enrolled 25-test
  native harness suite, all 257 Python infrastructure tests, and the complete
  all-feature workspace test gate including the six-test real server release
  verifier.
- TC-NATIVE-001 is resolved with typed AccessKit menu identities, exact UIA
  inventory validation, separate detached and attached Open Media contracts,
  deterministic player-receipt evidence, structured capability outcomes, and
  screenshot/UIA failure artifacts. A final provenance-bound combined run
  passed with empty stderr as the third consecutive strengthened trial.
- Two subsequent attempts to run the complete ten-scenario native inventory
  correctly remained red and exposed TC-HARNESS-007 through TC-HARNESS-009
  plus TC-NATIVE-002. All four are now resolved: peer readiness has a
  two-sided bounded handshake, native connectivity is fail-closed and
  loopback-only, menu input is physically hit-tested and stress-gated, and
  File -> Exit has a bounded observable shutdown. Three consecutive stressed
  baselines and two consecutive complete ten-scenario runs passed with zero
  native stderr.
- Final current-source replay exposed TC-HARNESS-010 and also reopened
  TC-HARNESS-008. UIA could report a successful configuration-tab action while
  only focusing the tab, and the desktop could move the shared cursor between
  the harness's coordinate hit test and its button event. Top tabs now require
  content acknowledgement and have an exact focused-keyboard path; physical
  clicks atomically bind absolute move/down and move/up endpoints and never
  redeliver a toggle. The native binary's 25 contract tests are also enrolled
  in ordinary all-feature workspace testing (TC-HARNESS-011).
- Final validation rebuilt `sorotte-gui.exe` and passed the complete
  ten-scenario native inventory in 110,373 ms. Run
  `20260729T072511543Z-38900` has zero-byte stderr, native-report SHA-256
  `0c3524e9903ea05b52f4f2d350a76b7ca7bc62812b081305c9f6c7578b2225df`,
  and every capability outcome is `required-pass`.
- The merged-profile collector passed a complete local run using
  cargo-llvm-cov 0.8.4 and pinned Syncplay
  `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`. Schema 2 first proved it
  could remove 229 stale raw profiles and one stale merged profile. The
  exact-final replay then reset the preceding 36-profile trial, created 34
  workspace profiles in 180.969 seconds, created one profile from all 14
  semantic scenarios in 8.613 seconds, and created one from all four strict
  live-TLS cases in 1.101 seconds. Every lane removed zero prior profiles and
  LLVM merged exactly those 36 current-run profiles. The diagnostic summary
  reported 148,209 of 190,067 line instances covered (77.98%); the downstream
  source-bound map reported 145,016 of 183,712 unique physical lines
  (78.936596%).
- The attempted complete strict compatibility profile remained red in a
  durable replay: 129 passed, 6 failed, and 9 were ignored in 88.98 seconds.
  This is retained as historical discovery evidence rather than a current
  result.
- The remediation replay makes the strict live-reference boundary required
  with one mechanically inventoried `legacy_server_` selector. It passes 20/20
  tests with zero failed, zero ignored, and 121 filtered in 15.72 seconds:
  12 strict fanout scenarios, 4 TLS probes, 2 live state probes, and 2
  request-shim contracts. The deterministic Python model passes all 33 fanout
  cases, and all 16 captured Python trace comparisons pass exactly.
- An exact-final end-to-end cargo-llvm-cov 0.8.4 replay then removed 36 prior raw
  profiles and recreated exactly 36 current profiles: 34 from the locked
  all-feature workspace in 188.002 seconds, 1 from all 14 semantic scenarios
  in 8.456 seconds, and 1 from the complete 20-test live-reference lane in
  18.048 seconds. The merge check passed in 1.554 seconds and reported 148,594
  of 191,287 diagnostic line instances covered (77.68%).

## TC-PLAYER-001: concurrent external replacement corrupts predecessor linkage (resolved)

Status: **Resolved 2026-07-29; successor selection is exclusive**

Severity: **High**
Detection: shrinkable reducer-input history

Resolution: selecting a successor for a live predecessor now atomically
detaches every other attempt whose `replaced_attempt` backlink still claims
that predecessor. The selected successor keeps the reciprocal backlink;
unselected pending attempts are not speculatively failed and can still bind if
later physical evidence identifies them. A terminal predecessor may retain
historical provenance from an unselected attempt only while it has no selected
successor, which is not a contradictory graph.

The same helper runs at both transitions that can select a successor:
`ExternalLoadObserved` and `LoadAttemptAccepted`. This is the lean conflict
rule: the later authoritative physical observation or accepted load owns the
single successor edge, while unrelated attempt state and command outcomes are
left unchanged.

Minimal history in attachment epoch 1:

1. `ExternalLoadObserved(generation=1, playlist_entry=100, file_loaded=false)`
2. `LoadAttemptSubmitted(command=1, generation=1, target="property-target-0")`
3. `ExternalLoadObserved(generation=1, playlist_entry=101, file_loaded=false)`

Before the fix, the third transition tripped the reducer's own invariant:

```text
attempt predecessor points to another successor
```

This is a valid reducer-contract ordering: an external physical load exists, a
commanded replacement is submitted, and another external load is observed
before the submitted attempt is resolved. The reducer now preserves its graph
invariant after every transition.

The history was found while preparing the stale-epoch metamorphic property. It
failed during setup, before the stale observation was applied; it is therefore
not evidence of stale-epoch mutation. Its discovery seed belonged to an older
stale-property strategy and was removed because Proptest seeds are coupled to
both their source file and exact strategy shape. The minimized history is now
the positive
`tc_player_001_external_replacement_preserves_reciprocal_links` regression.
Deeper generation found the same graph overwrite when acceptance of a second
submitted attempt repointed a predecessor that still had a rejected successor
backlink. That event-kind variant is now the positive
`tc_player_001_acceptance_detaches_rejected_successor_backlink` regression.

The unchecked reducer seam and defect-family classifier were deleted. The
broad property now sends every generated transition through the ordinary
reducer and requires the complete invariant set after each step. It passed
10,000 generated histories in 3.11 seconds. The complete
`sorotte-player-mpv` library passed 407 tests with two opt-in capability tests
ignored.

Two adapter regressions prove the production ingress distinctions:

- an accepted load submitted through `MpvAdapter` detaches the rejected
  successor's stale claim before selecting itself;
- an authoritative playlist mismatch first terminalizes a contradicted active
  predecessor, then admits the external current entry without inventing a
  selected successor edge. The pending attempt's historical backlink is safe
  because the terminal predecessor selects none.

The full authoritative-reconciliation module passed all nine cases. The
known-defect registry entry and both `should_panic` characterizations were
removed; `PL-PROP-001` now names the graph invariant and its two positive
reducer proofs plus two adapter proofs. The exact experiment is retained in
[`player-successor-conflict-20260729.md`](evidence/test-coverage/player-successor-conflict-20260729.md).

## TC-PLAYER-002: delayed acceptance plus authoritative binding leaves terminal and active state (resolved)

Status: **Resolved 2026-07-29; seven variants are positive regressions**

Severity: **High**
Detection: shrinkable reducer-input history

Resolution: every path that reactivates a physical attempt now clears the
stale logical terminal and provisional EOF projection. The exact minimized
history plus cross-generation, superseding-submission, repeated-external,
loaded-external, terminal-external, and replaced-attempt variants now assert
the invariant after every transition and finish with an active attempt and no
logical terminal. The family classifier and defect registry entry were
removed. The full `sorotte-player-mpv` crate passed 391 tests with two
capability-dependent tests ignored.

Minimal history in attachment epoch 1:

1. `LoadAttemptSubmitted(command=1, generation=1, target="property-target-0")`
2. `ExternalLoadObserved(generation=1, playlist_entry=100, file_loaded=false)`
3. `LoadAttemptAccepted(attempt=1)`
4. `PlaylistSnapshot(current_entry=101, original_filename="property-target-0")`

Before the fix, the fourth transition tripped the reducer's own invariant:

```text
logical terminal playback still has an active physical attempt
```

The ordering models an external observation racing ahead of the command
acceptance response, followed by authoritative playlist reconciliation. The
old reducer reached a state that simultaneously claimed logical terminal
playback and an active physical owner.

Source- and strategy-scoped Proptest replay seed:

```text
21da6327ec034d62801fcab370f374a0861f646f68e76356ece4bb17fcf8741d
```

Before the fix, the broad property explicitly quarantined this invariant family after
detecting its causal state transition, leaving the pre-transition state intact
so later generated cases still execute. The classifier requires a valid
pre-state and an invalid post-state with both a concrete terminal physical
owner and a different current-epoch live physical owner under the retained
logical terminal outcome. It is independent of panic text, candidate count,
target spelling, triggering input kind, and predecessor linkage.

Those seven histories are retained under ordinary
`*_reactivation_clears_logical_terminal` positive test names.

## Reproduction

Run the complete positive property module:

```text
cargo test --locked -p sorotte-player-mpv --all-features --lib \
  lifecycle::property_tests -- --nocapture
```

Expected result on this branch: every case passes without `should_panic`,
quarantine, or defect-family classification. For a deeper deterministic
sample:

```text
$env:PROPTEST_CASES = "10000"
cargo test --locked -p sorotte-player-mpv --all-features --lib generated_reducer_input_histories_preserve_contracts
```

The stale-epoch property still exercises all epoch-bearing input kinds against
live current-epoch identity collisions for every generated setup. The two
successor-conflict regressions and seven terminal-reactivation variants are
ordinary merge-contract proofs.

## TC-PLAYER-003: property change can disappear between heartbeat acknowledgement and response (resolved)

Status: **Resolved 2026-07-29; test synchronization now waits for the command boundary**

Severity: **Harness defect; no product event loss found**
Detection: complete cargo-llvm-cov workspace run

The original failure was real execution but not a dropped adapter event. The
mock worker emitted heartbeat acknowledgement, property event, and command
response in order. The test stopped waiting as soon as it observed the
acknowledgement, then sampled ordinary events before the worker had ingressed
the property and response queues. Instrumentation made that legal schedule
more likely.

The test helper now waits for three observable facts: the heartbeat was sent,
its acknowledgement is cleared, and the nonblocking command is no longer
pending. It then checks the full-pump result. The exact regression passes 64
consecutive schedules, and the full `sorotte-player-mpv` suite remains green.
No production player behavior changed.

## TC-COMPAT-001: username-conflict fanout has no Rust match (resolved)

Status: **Resolved 2026-07-29; bounded legacy conflict allocation**

Severity: **Medium (legacy protocol parity)**
Detection: strict live legacy fanout comparison

Syncplay resolves successive collisions as `alice_`, `alice__`, `alice___`,
and so on. Sorotte again follows that observable allocation sequence, bounded
by the configured maximum Unicode-scalar username length so hostile collisions
cannot cause unbounded work. Direct server and compatibility tests prove the
sequence, Unicode boundary, and bounded fallback; the live scenario now
matches exactly.

## TC-COMPAT-002: persistent-room lifecycle emits an extra playlist index (resolved)

Status: **Resolved 2026-07-29; playlist and index mutations are independent**

Severity: **Medium (observable playlist fanout divergence)**
Detection: strict live legacy fanout comparison

A `playlistChange` now updates and broadcasts only the playlist while retaining
the last explicit index. It no longer synthesizes or broadcasts
`playlistIndex(0)`. Explicit index commands keep their validation and
persistence rules. Direct controller/persistence tests and strict live parity
prove the contract.

## TC-COMPAT-003: controlled-room playlist change doubles fanout (resolved)

Status: **Resolved 2026-07-29; exact authorized-recipient cardinality**

Severity: **Medium (observable multi-client playlist fanout divergence)**
Detection: strict live legacy fanout comparison

This was the multi-recipient manifestation of `TC-COMPAT-002`. Removing the
implicit index mutation leaves one `playlistChange` for each authorized
recipient and no secondary index broadcast. The strict comparator now checks
each recipient's complete ordered sequence, so both authorization and
cardinality are positive contracts.

## TC-COMPAT-004: permanent-room file load emits an extra playlist index (resolved)

Status: **Resolved 2026-07-29; legacy placeholder state without implicit fanout**

Severity: **Medium (persistent-room startup parity)**
Detection: strict live legacy fanout comparison

Configured permanent rooms initialize the same internal index placeholder
(`Some(0)`) as Syncplay, while playlist changes still do not emit an implicit
index message. Persistence sanitation remains intact. Direct permanent-room
tests and the live reference scenario cover startup and subsequent mutation.

## TC-COMPAT-005: persistent-room timeout parity aborts the legacy connection (resolved)

Status: **Resolved 2026-07-29; dual logical clocks**

Severity: **Harness defect**
Detection: complete strict legacy matrix

The fixture had been changed from a 10-second legacy advance to an 88-second
Sorotte advance to exercise Sorotte's extended media-match liveness. The live
Python harness slept the full 88 seconds, exceeded its 12.5-second socket
contract, and Windows reported error 10053. Scenarios now carry an optional
`legacyAdvanceSeconds`; this case advances Sorotte by 88 seconds and the pinned
legacy server by 10 seconds. Non-timing output remains exact, while periodic
`State` count is excluded only where the two clocks intentionally differ.

## TC-COMPAT-006: periodic-state timeout parity aborts the legacy connection (resolved)

Status: **Resolved 2026-07-29; dual logical clocks**

Severity: **Harness defect**
Detection: complete strict legacy matrix

This shared the same root cause as `TC-COMPAT-005` on the independent
state-maintenance path. The schema validates the legacy override, the live and
deterministic Python runners both consume it, and malformed values fail
fixture loading. Both former quarantines are ordinary required tests.

## TC-COMPAT-007: persistent-room list is delivered after join snapshots (resolved)

Status: **Resolved 2026-07-29; list precedes playlist snapshots and Hello**

Severity: **Medium (observable protocol ordering)**
Detection: strengthened per-recipient strict comparison

After the earlier value differences were removed, exact per-recipient
comparison exposed a further ordering defect. Syncplay sends the persistent
room `List` after readiness but before destination playlist/index snapshots and
before `Hello`; Sorotte delayed the list until after `Hello`. Join and
room-switch handlers now preserve the legacy order. Two direct server
regressions cover initial join and room switch, and the strict live matrix
proves the complete recipient transcript.

## TC-HARNESS-012: missing-feature sentinel changes legacy behavior (resolved)

Status: **Resolved 2026-07-29; version-derived Syncplay defaults**

Severity: **Harness correctness**
Detection: removal experiment against pinned Syncplay 1.7.5

The legacy request shim used a synthetic
`__syncplay_rs_missing_features__` feature key to avoid a first-client
Syncplay crash. That sentinel also changed list/UI capability behavior.
Removing it naively reproduced the upstream null-feature crash. The shim now
synthesizes Syncplay's exact version-derived defaults, using `realversion`
before `version`, while preserving every explicit feature map unchanged.
Focused request-shim tests and the live matrix enforce both paths; regenerated
traces contain no sentinel.

## TC-HARNESS-013: compatibility exceptions concealed exact parity (resolved)

Status: **Resolved 2026-07-29; narrow, explicit oracle boundaries**

Severity: **Harness correctness**
Detection: comparator audit

The compatibility assertions contained username remapping, playlist-index
equivalence, implicit-index filtering/alignment, and null-index trace
exceptions. Those transformations could conceal the exact defects above.
They were deleted. Live outputs are compared as complete ordered sequences per
recipient, without inventing a total order across independent sockets.
Only documented background idle `State` timing is excluded, and the
deterministic model excludes periodic `State` counts only for the two explicit
dual-clock timeout scenarios. A fail-closed guard rejects any new dual-clock
scenario, non-Hello/List command, or explicit playstate request until it
defines a scenario-specific State oracle. All 16 Python traces were recaptured
from the corrected model and compare exactly.

## TC-HARNESS-014: client trace assertions depend on normalized readiness and idle State (resolved)

Status: **Resolved 2026-07-29; nullable readiness and explicit-State ownership**

Severity: **Harness correctness**
Detection: full all-feature workspace replay after trace recapture

Two client-core tests assumed an old client with no readiness capability was
not ready (`Some(false)`) even though the pinned server emits
`isReady: null`. One also relied on an incidental periodic `State` in an older
capture even though the scenario contained no state-producing action.
Regenerating the traces from the corrected model exposed both assumptions.
The tests now preserve `None` as the meaningful unknown-readiness state and
prove that replay does not synthesize room playstate without an actual
`State` message. Focused replay passes all three client trace contracts.

The historical discovery replay passed 129 tests, failed six, and ignored
nine in 88.98 seconds. The current required live-reference selector passes
20/20 with no failures or ignores, while the full deterministic Python fanout
lane passes 33/33. Commands and the preserved before/after evidence are in
[`merged-profile-lanes-20260729.md`](evidence/test-coverage/merged-profile-lanes-20260729.md).

## TC-HARNESS-015: external-player fixture roles can terminate the parent libtest (resolved)

Status: **Resolved 2026-07-30; fixture-role observation is serialized**

Severity: **Harness correctness (the required workspace gate can exit or hang
without a product assertion failing)**
Detection: first combined all-feature workspace run after the deep-boundary
test slice

The CLI external-player tests temporarily set a process-global fixture role
before spawning the current test executable as an exact child test. The exact
fixture entrypoint is also an ordinary test in the parent libtest. It read the
role without taking the environment-domain mutex, so parallel scheduling could
make the parent entrypoint impersonate a child role.

The first combined run exited the entire CLI test binary with code `23`, the
intentional status of `early-exit-leaf`. The same unchecked read could instead
select the blocking `detached-leaf` role. Neither shape is a Rust assertion
failure, so the libtest can end abnormally or lose the normal per-test
diagnostic.

The fixture entrypoint now acquires the same `TestEnvGuard` used by every role
mutator before observing the role and retains it through dispatch. A child
process has its own mutex and therefore proceeds normally. In the parent, the
entrypoint either reads the original role before mutation or waits until the
mutating test restores it. The stdio coordinator now also performs its leaf
role change through that guard.

A barrier-driven regression holds the mutator guard, starts a role observer,
proves observation cannot complete during the transient role, releases the
guard, and requires the observer to receive the restored value. The owning
module passes 15/15, warning-denied CLI Clippy passes, and ten consecutive
complete all-feature CLI library runs pass 10/10 in 111.7 seconds. No retry or
test-thread serialization is needed.

## TC-HARNESS-016: updater interruption marker can be observed before its payload is written

Status: **Resolved 2026-07-30; positive required process regression**

Severity: **Harness correctness (the required workspace gate can fail before
testing updater recovery)**
Detection: full locked all-feature workspace gate followed by serial exact-test
stress

The updater process-interruption child previously published a boundary with
`fs::write(root.join("boundary-reached"), label)`. On Windows, creating the
file and filling its payload are separately observable. The parent polls only
`marker().exists()`, then immediately calls `read_to_string` and requires the
complete label. It can therefore observe the newly created zero-length file
before the child writes the boundary text.

The corrected test-only handshake creates a same-directory pending marker with
`create_new`, writes the complete label, flushes and syncs the file, closes it,
and atomically renames it to the published path. The parent now proceeds only
when reading that path returns the exact expected payload, while still failing
on premature child exit or deadline expiry. A deterministic preflight rejects
empty, partial, and incorrect contents and requires the pending path to be
absent after publication.

The first post-fix workspace run failed at that assertion:

```text
expected: "replaced-6"
observed: ""
```

The exact updater test passed on its first isolated retry, then failed at
iteration 5 of a bounded serial stress. A subsequent diagnostic capture passed
20/20, confirming an intermittent observation race. The failure occurs before
the parent kills the child and before either recovery subprocess is started,
so it is not evidence of an incomplete update or rollback.

After the correction, the exact 11-boundary process test passed 100/100 serial
replays in 64.8 seconds, covering 1,100 durable interruption boundaries and
2,200 recovery subprocesses. The complete updater binary passed 30/30 and the
strict process-lane policy inventory remained unchanged. The correction does
not claim parent-directory `fsync`, power-loss durability, or production
updater behavior. Exact evidence is retained in
[`updater-boundary-marker-handshake-20260730.md`](evidence/test-coverage/updater-boundary-marker-handshake-20260730.md).

This finding was never placed in `known-defects.toml`: that registry
inventories deterministic product expected-failure characterizations, while
this was an ordinary intermittent harness failure.

## TC-HARNESS-017: framed schedule helper did not bound non-consuming reads

Status: **Resolved 2026-07-30; positive source-bound mutation regression**

Severity: **Harness liveness (a scheduled mutation shard timed out instead of
classifying a broken framing decision)**
Detection: exploratory `cli-framing` cargo-mutants baseline

The first generated schedule helper used an unbounded
`while let Some(line) = read_line(...)` loop. Four mutants could therefore
leave the delimiter unconsumed or return a constant frame forever, turning a
clear contract violation into a per-mutant timeout. The same baseline exposed
three ordinary misses in split accumulated-length and same-buffer framing-CR
guards. None changed current production behavior; they identified missing
test termination and payload-limit oracles.

The corrected helper derives the exact maximum frame count from input LF
delimiters plus any unterminated suffix, requires every expected frame, and
performs one final EOF probe. Constant-frame and non-consuming readers now fail
promptly. Four exact MAX/MAX+1 LF/CRLF cases independently retain additive
split length and both same-buffer framing-CR decisions.

The initial 33-mutant experiment recorded 26 caught, three missed, and four
timed out outcomes, but its test source was strengthened while the remaining
mutants finished, so that aggregate is diagnostic rather than canonical. A
fresh campaign over the stable four-test oracle selected 370 package tests and
caught 33/33 viable mutants with zero misses, timeouts, or unviables. Exact
commands, hashes, the seven red cases, and both artifact roots are retained in
[`targeted-mutation-cli-framing-20260730.md`](evidence/test-coverage/targeted-mutation-cli-framing-20260730.md).

## TC-SEC-001: structured credential aliases survive transcript sanitization (resolved)

Status: **Resolved 2026-07-29; positive generated regression**

Severity: **High (sanitized diagnostic artifacts can retain credentials)**
Detection: generated nested/escaped credential-taint corpus

Resolution: transcript sanitization now delegates structured key decisions to
the shared `sorotte-secret` policy. The policy canonicalizes case and
punctuation while recognizing credential aliases including `credentials`,
camel-case credential suffixes, cookie/header names, and API keys. The former
characterization is now
`structured_credential_aliases_are_redacted_from_sanitized_transcript`, and
the full mpv privacy suite passes.

The new privacy suite generates credential canaries across seven nesting
levels, ordinary and Unicode-escaped JSON keys and values, URL/header/path
forms, malformed transcript input, JSON-lines round trips, `Debug`, diagnostic
dumps, and sanitizer idempotence. Recognized sensitive keys remain redacted
through every tested output.

The original experiment found five structurally credential-bearing aliases
outside the former key classifier:

```text
credentials
futureCredential
set-cookie
x-api-key
httpHeaders
```

Before the fix, each alias allowed the raw or encoded canary to survive the
sanitized transcript's JSON-lines export. This was a product privacy defect
rather than a weakness in the test oracle: the test checks raw,
Unicode-escaped, percent-encoded, and hexadecimal canary forms after transcript
construction and serialization.

The former executable characterization was:

```text
transcript::privacy_tests::
known_defect_tc_sec_001_structured_credential_aliases_leak_from_sanitized_transcript
```

It used `should_panic` with the exact assertion:

```text
structured credential aliases leaked from sanitized transcript
```

The broader generated corpus remains a required privacy proof.

## TC-SEC-002: escaped diagnostic credentials survive PlayerError redaction (resolved)

Status: **Resolved 2026-07-29; positive generated regression**

Severity: **High (reflected parser or transport diagnostics can disclose credentials)**
Detection: generated `PlayerError` display-taint corpus

Resolution: credential classification builds a lowercase ASCII projection for
classification only, recognizing `%HH` and JSON `\u00HH` key/delimiter forms
without returning decoded attacker-controlled text. `PlayerError` delegates to
this shared policy. The former characterization is now
`escaped_diagnostic_credentials_are_redacted_from_player_error`.

The ordinary generated corpus confirms that raw nested JSON, URL query,
header-style, percent-delimited, and quoted credentials are removed from
`PlayerError` display and debug outputs. Before the fix, four encoding variants
evaded the pre-display classifier:

```text
escaped-key       pass\u0077ord
escaped-colon     "password"\u003a
escaped-equals    password\u003d
encoded-key       access%5Ftoken
```

Each form retained its generated canary in the user-visible `Display` output.
The former executable characterization was:

```text
error_display_redaction_tests::
known_defect_tc_sec_002_escaped_diagnostic_credentials_leak_from_player_error
```

It used `should_panic` with the exact assertion:

```text
escaped diagnostic credential forms leaked from PlayerError
```

False-positive canaries preserve `unexpected token: EOF`, mpv
`request_id` diagnostics, `property not found`, and `client not found`.

## TC-SEC-003: prose-prefixed credential fields survive PlayerError redaction (resolved)

Status: **Resolved 2026-07-29; positive generated regression**

Severity: **High (ordinary reflected diagnostics can disclose credentials)**
Detection: generated `PlayerError` display-taint corpus

Resolution: the shared diagnostic grammar scans the bounded identifier
immediately preceding `=` or `:` regardless of harmless prose prefix, while
requiring a credential-shaped value for the ambiguous bare `token:` form. The
former characterization is now
`prose_prefixed_credential_fields_are_redacted_from_player_error`.

Even without escaped syntax, the former classifier assumed that a sensitive
key began immediately after one of a small set of structural delimiters.
Natural diagnostic prefixes therefore left four generated canaries visible:

```text
prose-colon      request failed with password: Bearer <canary>
prose-equals     upstream response includes token=<canary>
parenthesized    request failed (secret=<canary>)
arrow-colon      backend -> clientSecret: <canary>
```

The former executable characterization was:

```text
error_display_redaction_tests::
known_defect_tc_sec_003_prose_prefixed_credential_fields_leak_from_player_error
```

It used `should_panic` with the exact assertion:

```text
prose-prefixed credential fields leaked from PlayerError
```

The safe-diagnostic canaries above prove this did not turn every word before a
colon into a secret.

## TC-HARNESS-001: PowerShell timestamp coercion falsely fails package freshness (resolved)

Status: **Resolved 2026-07-29; cross-shell package suite passes**

Severity: **Medium (required Windows gate false negative)**
Detection: final package-policy validation

Resolution: the test now reads the Git commit epoch with `%ct`, preserves
PowerShell 7's already-parsed UTC `DateTime`, explicitly parses PowerShell
5.1's string form, and compares both values as Unix seconds. A source-policy
test prevents regression to `%cI` string equality. The complete package-path
suite passes under both `pwsh` and Windows PowerShell 5.1.

Before the fix, `scripts/package-path-boundary-tests.ps1` exited 1 at its
freshness assertion:

```text
dev package freshness must use the source commit timestamp, not rerun time
```

The package output itself is correct. An isolated reproduction generated:

```text
git_sha:       a08a06ea7c6cada5413b0dba73b16f940cfd46e1
manifest:      2026-07-27T11:49:33Z
commit in UTC: 2026-07-27T11:49:33Z
```

Under PowerShell 7.6.4, `ConvertFrom-Json` materializes
`created_at_utc` as `System.DateTime`. The test compares that object directly
with a formatted string at `scripts/package-path-boundary-tests.ps1:267`, so
equal timestamps compared unequal. The source packaging logic did not require
a change.

Final host comparison confirmed the boundary: Windows PowerShell 5.1 passed
the package-path suite, while `pwsh` 7.6.4 failed at the freshness assertion.
The required Windows workflow uses `pwsh`; that exact shell now passes.

## TC-SERVER-001: playlist JSON migration is not atomic across rows (resolved)

Status: **Resolved 2026-07-29; atomic failpoint regression**

Severity: **High (durability boundary permits partial migration)**
Detection: deterministic SQLite trigger failpoint

Resolution: `load_rooms()` now performs selection, decoding, every required
JSON/index repair, and commit inside one SQLite immediate transaction. A later
write failure rolls the entire migration back. The positive regression
requires zero migrated rows after the injected second-row failure, removes the
trigger, retries, and then requires both rows migrated. All 35 focused
persistence tests pass.

The original characterization seeds two legacy persistent-room rows whose
`playlistJson` columns are null. A SQLite trigger allows the first migration
update and aborts the second with:

```text
injected second migration failure
```

Before the transaction fix, `load_rooms()` returned that error, but inspection
before restart observed one already-migrated row:

```text
migrated_before_restart: 1
valid atomic results:     0 or 2
```

That result violated the old-or-new-complete migration invariant across the
selected rows. Recovery remained functional: after removing the failpoint and
reopening the store, both rows migrated and deserialized correctly. Recovery
did not make the original failure atomic.

The former minimized characterization was:

```text
tests::persistence_tests::
known_defect_playlist_json_migration_commits_rows_before_later_failure
```

It used `should_panic` with the exact atomicity assertion. The positive
replacement is
`playlist_json_migration_rolls_back_all_rows_after_later_failure`. Before the
fix it panicked with:

```text
playlist JSON migration must be atomic across rows, found 1 migrated rows
```

## TC-SERVER-002: concurrent quota-secret creation does not converge (resolved)

Status: **Resolved 2026-07-29; concurrent convergence regression**

Severity: **High (shared durable identity initialization can fail under concurrency)**
Detection: two-connection SQLite schedule with a pre-create barrier

Resolution: creation uses `INSERT ... ON CONFLICT(key) DO NOTHING` and always
rereads and validates the durable row. Both barrier-synchronized callers now
return the same 32-byte value, while corrupt pre-existing metadata still fails
closed without replacement. The positive test is
`concurrent_quota_secret_creation_converges_on_one_durable_value`.

Before the fix, `load_or_create_quota_secret()` performed a read followed by an
unconditional insert. A test-only barrier pauses two independent store
instances after both have observed that the metadata row is absent. Under the
old implementation, releasing both creators produced:

```text
successful creators: 1
failed creators:     1
failure action:      create quota secret
durable rows:        1
```

The winning 32-byte value was durable and remained valid. The losing caller
received SQLite's uniqueness failure rather than loading and returning the
winner. The required contract is stronger: every concurrent initializer must
return the same durable secret, because callers should not need to distinguish
first creation from convergence on a concurrently created value.

The former executable characterization was:

```text
tests::persistence_tests::
known_defect_concurrent_quota_secret_creation_does_not_converge
```

It used `should_panic` with the exact convergence assertion. The test-only
barrier seam remains solely to make the two-caller schedule deterministic.

## TC-NATIVE-001: native menu and Open Media behavior are identity-bound

Status: **Resolved 2026-07-29; native behavior is identity-bound and proven**

Severity: **High (native accessibility and required workflow gap)**
Detection: strict validation of a real Windows baseline

The previous native runner returned exit zero with `result: "ok"` and
`interaction_contract: "verified"`, while the same report contained:

```text
menu_labels: []
menu_contract: "skipped-no-native-menu"
open-media-file-skipped: menu item, fallback control, and quick-open button
                          discovery all timed out
```

The required `open-media-file` completion marker was absent. The final
accessibility snapshot was still on the setup/configuration surface. This is
not acceptable native evidence: a required workflow was neither discoverable
nor completed.

Root cause: egui rendered visible menu buttons, not a Win32 `HMENU`, and the
actual top-level widgets had no stable AccessKit author IDs. The old runner
therefore queried the wrong presentation layer and then treated discovery
failure as a skip. Its fallback also referenced a Quick Open Media node that
the product did not render. Repeated menu-open fallbacks could toggle the same
popup closed, so a later timeout did not prove that the product lacked a menu.

Resolution:

1. `MenuSectionId` is the typed source of truth for the five visible sections:
   `menu.section.file`, `.playback`, `.advanced`, `.window`, and `.help`.
   The egui renderer attaches those IDs to the actual menu-button responses
   exported through AccessKit. The fictional Quick Open Media node and fallback
   were removed.
2. The Windows runner enumerates UIA/AccessKit nodes and requires exactly one
   visible, enabled, bounded node for every typed ID with its exact label. It
   rejects missing, duplicate, mislabeled, hidden, or unreviewed section IDs;
   Win32 menu enumeration cannot satisfy the contract.
3. Menu commands open the section once, wait for one exact actionable leaf by
   stable automation ID, then physically click that leaf once. The pointer
   remains over the target long enough for egui to materialize the popup,
   avoiding the former open/close oscillation. Enabled-state probes dismiss
   the popup with Escape, verify that its action is absent, and reset focus
   through the stable Setup surface before the later invocation.
4. The detached baseline proves `menu.open_media` exists and is disabled when
   no player is attached. A separate `menu-open-media` scenario launches an
   isolated deterministic player, proves the same command is enabled, invokes
   File -> Open Media by stable IDs, and requires the room view transition.
5. The deterministic player writes an opt-in JSONL observation when its
   `open_file` boundary receives a path. The runner requires the exact selected
   path; visible text, a keyboard shortcut, or the room transition alone cannot
   substitute for runtime receipt.
6. The producer emits exact `required-pass` outcomes for native menu inventory,
   detached disablement, and attached delivery. The Python boundary validates
   each outcome's ID, source, and evidence and rejects missing, skipped,
   duplicate, forged-source, or extra outcomes.
7. On a live failure, every primary and secondary scenario path now attempts
   to write a screenshot and credential-redacted UIA/AccessKit tree to the
   wrapper-provided artifact directory before terminating the process.
   Capture errors are retained separately and never replace the original
   failure.

The final fresh-binary run required `baseline` and `menu-open-media`, returned
producer exit `0`, passed the strict validator, closed both GUI processes, and
emitted zero stderr bytes:

```text
artifact:
  target/verification/gui-native-smoke/20260729T031013862Z-47644
binary provenance: rebuilt-debug
binary sha256 before and after:
  4d2195914472228541507c7ad4622adb3e622a231a4741f714179240d8394551
raw report sha256:
  a73688f2f489c8a011a21fc6a12e1f1948ba431b9f533875af127c0165c258f3
producer / strict result: 0 / required-pass
reported duration: 23,566 ms
native stderr: 0 bytes
```

The final strengthened sequence passed three consecutive runs in 23,591,
23,339, and 23,566 ms. An adversarial replay against an existing pre-change
GUI binary failed closed on missing `menu.section.file` and preserved a
5,611,593-byte screenshot plus a 35,188-byte UIA tree at
`target/verification/gui-native-smoke/20260729T024239593Z-11936`.

Original preserved failure:

```text
artifact:
  target/verification/gui-native-smoke/20260728T054736251Z-64192
scenario: baseline
main GUI build: 25.858 seconds
native harness build: 4.755 seconds
direct runner: 54.373 seconds
producer exit: 0
wrapper exit: 1
binary sha256 before and after:
  e923e92ec096b3ddf1e8e527fed4ddf0475d1f3a5e99080511e9cd194bddf6e2
raw report sha256:
  a102c5dcbd8a653cd32b0c01675a332ecf677e8df7097a6bd7f12c8aa8f0aabe
strict result: failure (5 contract errors)
```

A sanitized, reviewable copy of the decision-relevant raw fields and strict
replay result is tracked in
[`docs/evidence/test-coverage/native-baseline-20260728.md`](evidence/test-coverage/native-baseline-20260728.md).
The resolved implementation and current evidence are recorded in
[`docs/evidence/test-coverage/native-menu-open-media-20260729.md`](evidence/test-coverage/native-menu-open-media-20260729.md).

The initial resolved two-scenario menu proof did not imply that the broader
native inventory was green. Two later all-scenario runs and two isolated
diagnostics surfaced TC-HARNESS-007 through TC-HARNESS-009 and TC-NATIVE-002
below. Their original evidence remains retained; the follow-up implementation
and full-inventory proof now resolve all four.

## TC-HARNESS-002: native baseline performs repeated placeholder DNS lookups (resolved)

Status: **Resolved 2026-07-29; detached baseline performs no network I/O**

Severity: **Medium (test isolation and diagnostic-noise gap)**
Detection: captured native-runner stderr

Resolution: the detached baseline retains representative persisted host values
but launches with startup saved-connect disabled. Connectivity scenarios
continue to own explicit loopback fixtures. A fresh strict baseline produced a
zero-byte `native-stderr.log` with no DNS/address-resolution messages. The
strict run still failed on `TC-NATIVE-001` only, preserving the independent
menu/Open Media decision rather than masking it. Evidence:
`target/verification/gui-native-smoke/20260729T001820735Z-50568`.

The original 47.597-second baseline emitted 19 instances of the error below.
The final provenance-bound rerun emitted 20 and failed with the same contract:

```text
Session transport TCP address resolution for syncplay.example:8999 failed:
No such host is known. (os error 11001)
```

The legacy runner ignored this stderr and still returned success. The strict
contract now rejects unexpected stderr, while the baseline itself no longer
initiates a connection. Real connectivity coverage remains loopback-only.

## TC-HARNESS-003: live Python semantic playlist observation is timing-sensitive (resolved)

Status: **Resolved 2026-07-29; correlated responses, cooperative runtime pumping,
and truthful peer capabilities**

Severity: **Medium (required semantic evidence can be flaky)**
Detection: preserved full semantic-suite reruns plus the reliable-transport
single-frame ownership contract

Resolution: every command issued by `LegacyServerPythonPeerHarness` receives a
monotonic `requestId`, and every success or error response echoes it. Rust
rejects missing, stale, or mismatched response IDs. The probe remains compatible
with existing uncorrelated test clients by omitting the response ID when the
command omitted it. Observation commands keep the caller's timeout as the
Python state deadline but allow a separate two-second delivery margin for
serialization, pipe scheduling, and receipt.

A later full semantic replay proved that response correlation alone was not the
complete fix. The flow applied `AppendSharedPlaylistEntries` optimistically,
called the runtime owner once, observed the expected shell playlist, and then
entered a blocking Python-side wait. Production transport deliberately stages
at most one receipt-owned protocol line per owner pump. A shared-playlist queue
is a compound batch, so the optimistic projection could satisfy the GUI wait
before the owner had accepted a receipt and advanced the remaining frames. The
blocking peer wait then starved the only component capable of making progress.
The peer correctly timed out with `observed=[]`.

The playlist and playlist-index peer waits now poll an immediate peer snapshot
while continuing to call the real `pump_and_apply` path. This preserves the
production transport's receipt ownership and advances every compound frame
without a sleep, retry, or test-only transport shortcut. Timeout diagnostics
include the last peer playlist, index, and room. The reference Python peer also
advertises `sharedPlaylists: true` and enables its shared-playlist client path,
matching the behavior the fixture exercises; a source contract test prevents
that declaration from drifting back to false.

The exact real-Python chat/playlist regression passed 10 consecutive processes
at approximately 2.24 seconds each. The five-test real-Python GUI interop family
then passed together, and `live-python-peer-connect-flow` passed three
independent semantic-suite processes. Two consecutive complete semantic suites
then passed 14/14 with no STARTTLS warning or unexpected stderr. The dedicated
evidence record is
[`docs/evidence/test-coverage/semantic-live-python-playlist-20260729.md`](evidence/test-coverage/semantic-live-python-playlist-20260729.md).

One preserved run passed 13 of 14 scenarios. `live-python-peer-connect-flow`
connected both peers and observed snapshot and bidirectional chat traffic, but
the Python peer timed out waiting for status `"playlist"`. Its captured state
still contained an empty playlist:

```text
status events observed: connected, snapshot, chat-message,
                        chat-command-sent, chat-message
playlist: []
failure: timed out waiting for status "playlist"
```

The same scenario passed in isolation immediately afterward (1 of 1 in 3.6
seconds), an earlier complete suite run passed all 14 scenarios, the final
pre-rebase replay passed 14/14 in 29.7 seconds, and the post-rebase replay
passed 14/14 in 39.2 seconds. A subsequent recurrence after the broader native
fixes supplied the missing causal evidence above: this was harness starvation,
not an application playlist race. The first failed attempt remains retained as
discovery evidence.

The lane still preserves per-scenario event timelines on every failure. A
retry may classify a failure; it must not overwrite or convert the first failed
attempt into passing evidence.

## TC-HARNESS-004: intermittent CLI failure poisons the shared test lock (resolved)

Status: **Resolved 2026-07-29; panic-safe environment ownership**

Severity: **Medium (workspace and coverage evidence can fail nondeterministically)**
Detection: first full Windows cargo-llvm-cov run; reproduced by ordinary workspace tests

Resolution: `TestEnvGuard` records each environment key's original value on
first mutation and restores all keys from `Drop`, including during unwind. It
recovers a poisoned domain mutex only after that restoration path, so one
assertion no longer cascades into unrelated lock failures. A regression
intentionally panics inside the guard, reacquires the poisoned mutex, and
proves restoration. The Plex root test no longer sleeps for 250 ms: its fake
Plex server signals after serving the timeline, and the Syncplay fixture closes
causally afterward. The full CLI run passes 333 tests with eight intentional
ignores.

The first instrumented run and a later ordinary locked all-feature workspace
run both stopped in the `sorotte-cli` library suite with the same totals:

```text
311 passed; 20 failed; 8 ignored
```

The first reported failure was
`connected_session_reports_plex_timeline_from_player_telemetry`. Nineteen
stored-settings tests then failed at the shared test lock with
`lock poisoned: PoisonError`. This cascade makes the first failure harder to
diagnose and turns one concurrent failure into a much larger red surface.

The ordinary reproduction proves coverage instrumentation is not required to
trigger the coupling. That run's isolated root selector passed 1 of 1 in 2.31
seconds, the complete CLI library passed 331 tests with 8 ignored in 10.84
seconds, and the full workspace retry—including doctests—passed in 181.7
seconds. Earlier instrumented follow-ups also passed:

- isolated instrumented test: 1 of 1 passed in 2.27 seconds;
- complete instrumented CLI library retry: 331 passed, 8 ignored in 10.85
  seconds;
- full instrumented workspace retry: passed and emitted LCOV in 184.5 seconds.

No retry result replaces the first failure. The original evidence supports the
test-isolation diagnosis now encoded by the unwind and causal-timeline
regressions; no product behavior change was needed.

The same Plex test name recurred in workflow run `30636380151`, Windows job
`91174920040`, but with a distinct root cause. That run executed 3,775 tests:
3,774 passed and the first attempt of
`connected_session_reports_plex_timeline_from_player_telemetry` failed before
its retry passed. The fail-on-flaky policy correctly rejected the retry-only
green result. The 2026-07-29 panic-safe environment ownership above remains
valid and unchanged; the accepted-socket partial-header fixture defect exposed
by the recurrence is separately recorded and resolved as `TC-HARNESS-046`.

## TC-HARNESS-005: cargo-llvm-cov emits contradictory LCOV line summaries

Status: **Resolved locally 2026-07-29; producer contradiction is retained as typed audit evidence**

Severity: **High for ambiguous LCOV consumers; Sorotte now names and enforces its line model**
Detection: strict replay through the new critical-path ratchet

Resolution: `scripts/diff_coverage.py --lcov` now treats unique `DA` records as
the only line-addressable model and preserves `LF`/`LH` as a separate producer
summary audit. It never substitutes one model for the other:

- each report declares `coverage_line_model = unique-da-source-lines`;
- every source record retains declared and computed counts plus the exact
  mismatched fields;
- aggregate declared and unique-`DA` counts remain separate;
- duplicate or malformed `DA`, impossible `LH > LF`, stale sources,
  out-of-range lines, and unsupported directives remain input errors;
- a declared `LF` cannot invent a missing executable mapping: an executable
  changed line without `DA` remains `unmapped` and fails policy;
- lexical structure may remain non-coverable, using the same conservative
  source scanner as the required physical-line gate.

The upstream cargo-llvm-cov output is still contradictory; Sorotte does not
claim to repair those producer bytes or choose an aggregate coverage
percentage. The surfaced local defect was that the diagnostic consumer could
only reject the artifact, leaving no safe, mechanical way to inspect its
line-addressable evidence. That consumer ambiguity is now removed.

An exact-final-source run completed the full instrumented all-feature
workspace in 235.1 seconds:

```text
cargo llvm-cov --locked --workspace --all-features --lcov \
  --output-path target/tc-harness-005-fixed.lcov
```

The 15,369,296-byte artifact has SHA-256
`1998ea2b60336018b796c5e2a6e14cd6cc58ac36377f6914993b86c18bd136bf`.
The repaired parser produced:

```text
source records:                     395
records with any LF/LH mismatch:    310
records with an LF mismatch:        308
records with an LH mismatch:        259
declared LH/LF:        148,045 / 190,067 = 77.89%
positive/unique DA:    144,853 / 183,712 = 78.84%
```

An independent PowerShell record scanner matched all seven aggregate counts
exactly. The long-standing
`crates/sorotte-cli/src/client_args/parser.rs` record still proves that the
models were preserved rather than normalized:

```text
LF:122
LH:75
unique DA records:       120
positive-hit DA records: 115
```

The end-to-end replay over the exact current Rust diff reached policy
evaluation instead of an input error. It correctly remained red:

changed DA-covered lines: 761 / 1,827 = 41.65%
lexical non-coverable lines: 323
unmapped executable lines: 126
ordinary result: failed
critical result: failed
```

That is the intended distinction: summary contradictions are diagnostic
metadata, while genuine missing mappings and low changed-line coverage remain
hard failures. The complete diff-coverage suite passes 71 cases, including
dual-model preservation, impossible-summary rejection, and missing-`DA`
adversarial coverage. Exact current-source evidence is retained in
[`lcov-dual-model-20260729.md`](evidence/test-coverage/lcov-dual-model-20260729.md).

The required CI gate remains the stronger source-bound native contract: pinned
LLVM JSON plus `llvm-cov show`, a source-hashed physical line map, immutable
base/head policy, and six-phase digest binding. The LCOV path is explicitly
diagnostic because LCOV itself does not bind each source record to source
bytes. See
[LLVM llvm-cov](https://www.llvm.org/docs/CommandGuide/llvm-cov.html),
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov), and
[LLVM issue 126307](https://github.com/llvm/llvm-project/issues/126307).

## TC-HARNESS-006: updater self-replacement intermittently leaks an inherited handle (resolved)

Status: **Resolved 2026-07-29; background stdio is isolated**

Severity: **High (the required workspace suite could silently green a leaked subprocess)**
Detection: pinned cargo-nextest 0.9.137 leak detection and diagnostic retry

Resolution: all three fire-and-forget updater spawn paths—recovery restart,
post-update restart, and detached helper delegation—now explicitly bind stdin,
stdout, and stderr to null before spawning. This prevents a restarted GUI or
helper from retaining nextest's capture handles. The exact recovery test passes
once under Cargo and eleven consecutive times under pinned cargo-nextest
0.9.137's checked-in 500 ms leak-fail profile, with no retry or leak result.

The first full-workspace diagnostic run passed 3,458 tests, skipped 21, and
returned zero despite reporting this exact result:

```text
LEAK [0.919s] sorotte-gui::updater_self_replacement_windows
  running_installed_updater_recovers_interrupted_replacement_and_restarts
```

The next hardened full run passed without reproducing the leak. A subsequent
run through the exact required wrapper did reproduce it: attempt one was
`LKFAIL` after 1.161 seconds, attempt two passed after 1.127 seconds, and the
run remained failed as flaky. cargo-nextest returned 100, the wrapper returned
1, and JUnit retained both the failed attempt and final result. The clean
second run and passing retry do not replace the first failed evidence.

A controlled test independently kept an inherited output handle open past the
500 ms policy bound. It produced `LKFAIL` at 0.520 and 0.521 seconds, returned
100, and encoded the attempts as JUnit `<error>` and `<rerunError>`. This
proves the policy detects the failure mechanism without relying on the updater
test's nondeterministic reproduction.

The required workspace runner fails both a leak and a pass-after-leak, retains
console/JUnit/policy artifacts, and rejects per-test attempts to weaken the
leak timeout. The sanitized original run record is preserved in
[`docs/evidence/test-coverage/nextest-flake-leak-20260728.md`](evidence/test-coverage/nextest-flake-leak-20260728.md).

## TC-HARNESS-007: live Python peers do not reliably appear in the full native inventory

Status: **Resolved 2026-07-29; explicit two-sided readiness contract**

Severity: **High (the required native inventory cannot produce a complete pass)**
Detection: real Windows UIA plus the live Python compatibility harness

The first complete ten-scenario run timed out in `live-python` while waiting
for `interop-py-peer`. After failure capture was extended to every secondary
scenario, the next complete run passed that point but timed out on the same
missing peer in `controlled-room`:

```text
artifact 20260729T032222583Z-54624
runner duration: 57,931 ms
failure scope: live-python

artifact 20260729T032952983Z-53868
runner duration: 59,418 ms
failure scope: controlled-room

error:
  timed out waiting for accessibility name "interop-py-peer"
last UIA snapshot:
  "Busy: no", "Connect Saved Server: enabled", "Reload: enabled",
  "Save: enabled", "Status: clean", "view: room"
```

The second run retained `failure-controlled-room.png` (5,611,593 bytes,
SHA-256
`962c73b222aa0f2e175024a62951643b31f5a04de6f182b21d94fa00a22acb43`)
and a 107-node redacted UIA tree (31,394 bytes, SHA-256
`0309d9f9cd93f038ae7899e0daec0a891338a038d5b7627bebc7b6a47f0a529c`).
The screenshot shows the GUI participant in the test-owned controlled room
without the Python peer.

The variation in failing scenario pointed to readiness/lifecycle
orchestration, not a deterministic controlled-room rendering defect.

Resolution: the Python probe now implements a bounded
`wait_for_user_presence` command and returns a structured `user-present`
snapshot. The Rust harness exposes the same operation. Initial connection,
reconnection, and controlled-room setup now share one deadline and require
both directions in order:

1. the Python peer reports login completion;
2. the Python peer observes the GUI participant in the legacy server roster;
3. Windows UIA observes the Python participant in the GUI roster.

No timeout was lengthened and no scenario retry was added. Two consecutive
complete ten-scenario runs reached every live-Python interaction marker and
passed the strict contract. The first proof was:

```text
artifact: target/verification/gui-native-smoke/20260729T044650510Z-42024
native duration: 111,871 ms
strict status: required-pass
native stderr: 0 bytes
raw report sha256:
  ba33aa0991001ebd83507a3ca0c23888ad62bf0f0811d7d0566c62ff8a9eb62e
```

## TC-HARNESS-008: native menu input can target the wrong live cursor coordinate

Status: **Resolved 2026-07-29; atomic coordinate-owned single delivery**

Severity: **High (the harness can claim an exact hit while clicking elsewhere)**
Detection: isolated `controlled-room` diagnostic, before scenario setup

An isolated retry failed during the mandatory primary baseline after 5,850 ms:

```text
artifact: target/verification/gui-native-smoke/20260729T033220059Z-4172
error:
  timed out waiting for menu leaf
  "menu.section.file"->"menu.exit" after opening the menu once
native stderr: 0 bytes
```

The retained screenshot shows File focused but no popup. The redacted UIA tree
contains no `menu.exit` leaf. Earlier strengthened baseline/menu runs passed
three times consecutively, so this is an interaction-driver flake rather than
a deterministic missing-control defect.

The first repair foregrounded the exact HWND, proved the target with UIA
`ElementFromPoint`, and split mouse-down and mouse-up. It also allowed one
recorded redelivery after 700 ms of closed UIA snapshots. Three focused
baselines passed, but this was not a complete solution.

A later current-source full inventory failed at
`20260729T055406467Z-55552`: neither physical delivery exposed `menu.exit`.
The final accessibility tree showed File visible but the popup closed. This
disproved the earlier resolution and exposed a second race: absence in sampled
UIA frames does not prove that a toggle click is no longer queued, so the
second click can close a late-opening popup.

The redelivery was removed. A pure one-click experiment then failed at
`20260729T055813632Z-45932`, disproving the narrower hypothesis that asynchronous
UIA `SetFocus` was the only cause. A diagnostic cursor acknowledgement made the
hidden ownership problem explicit at `20260729T060034291Z-42000`:

```text
expected File center: (64, 104)
live cursor before mouse-down: (0, 59)
```

The historical hit test proved the element at the *intended* coordinate, but
the zero-coordinate `SendInput` button event used the desktop's shared live
cursor. Another desktop actor could therefore redirect the click after the
hit test.

Final resolution: physical interaction no longer mixes UIA `SetFocus` with
pointer input. Virtual-desktop coordinates are normalized to Win32 absolute
coordinates. Mouse-down is sent atomically with a move to the exact target,
and mouse-up is sent atomically with a second move to the same target. Each
endpoint is therefore coordinate-owned even if another actor moves the shared
cursor between frames. UIA still verifies the exact target at that coordinate,
and opening completes only when the requested leaf appears. Toggle sections
are delivered exactly once; there is no retry path.

The required baseline performs 25 File-menu open/dismiss cycles and emits:

```text
capability: native.menu.physical-input
source: uia-hit-test+win32-sendinput
evidence: menu-input-stress-25, menu-input-single-delivery
```

Two unit tests cover absolute-coordinate endpoints, negative-origin virtual
desktops, invalid spans, and out-of-range points. Three consecutive
fresh-binary baselines passed the final implementation, covering 75
single-delivery physical menu transactions with zero stderr:

```text
20260729T060444829Z-53772
20260729T060546798Z-8848
20260729T060643945Z-32828
```

Two complete ten-scenario proofs at `20260729T060756380Z-55276` and
`20260729T061005422Z-54068` supplied a fourth and fifth passing stressed
baseline in the exact full-inventory context that reopened the defect. The
causal experiment and artifact hashes are retained in
[`docs/evidence/test-coverage/native-input-ownership-20260729.md`](evidence/test-coverage/native-input-ownership-20260729.md).

## TC-HARNESS-009: the full native inventory leaks fixture networking to stderr

Status: **Resolved 2026-07-29; fail-closed network ownership and zero stderr**

Severity: **High (strict evidence is guaranteed to fail even if UI assertions pass)**
Detection: strict native stderr policy

Both complete runs emitted repeated external resolution attempts for
`syncplay.example:8999` and `saved.example:8999`, followed by the expected
negative STARTTLS fixture warning and repeated required-TLS refusal messages.
The strict wrapper correctly rejected all of it as unexpected stderr. The two
logs were 2,615 and 2,716 bytes respectively.

TC-HARNESS-002 remains resolved for the detached baseline; this was a separate
composability problem in the broader scenario inventory.

Resolution: native launches now require one typed network mode: detached,
in-process loopback, or TCP loopback with an explicit environment or
saved-config bootstrap. Detached and in-process modes forcibly disable startup
saved-connect. TCP mode rejects any host that is not `localhost` or a parsed
loopback address before spawning the GUI, and ordinary test-owned TCP
fixtures are written as plaintext so they do not create unrelated STARTTLS
diagnostics. Unit tests cover accepted IPv4/IPv6 loopback forms and rejection
of `saved.example`.

The first post-fix full run then exposed an additional ownership defect rather
than being allowlisted:

```text
artifact: target/verification/gui-native-smoke/20260729T043944366Z-52548
behavioral result: ok
strict result: failure
native stderr: 330 bytes
cause: the missing-media mock server expired after 10 seconds while its
       owning native scenario took longer than 15 seconds
```

Scenario-owned mock servers now remain alive until the GUI has closed, its
process has been joined, and the scenario explicitly releases the fixture.
The isolated missing-media continuation passed in 15,855 ms with empty stderr
at `20260729T044615793Z-50012`; the transport fixture passed independently at
`20260729T044524472Z-45404`. Two subsequent complete inventories passed with
zero-byte stderr logs at `20260729T044650510Z-42024` and
`20260729T045502691Z-56304`. No stderr allowlist was added.

## TC-HARNESS-010: top-tab actions can acknowledge focus without changing content

Status: **Resolved 2026-07-29; content-acknowledged multimodal activation**

Severity: **Medium (native assertion waits on content after an unacknowledged action)**
Detection: final complete native inventory

The retained run at `20260729T053110335Z-34976` failed after 29.1 seconds:

```text
error: timed out waiting for accessibility name "Show OSD"
native stderr: 0 bytes
```

Its screenshot still showed Playback & Search content. The accessibility tree
proved that `configuration:tab:interface-system` was enabled, visible, and
focused while `configuration:tab:playback-search` was not focused; no
`Show OSD` node existed. Both the accessibility invocation and exact physical
click had returned success, so API completion was not behavior completion.

Resolution: top-tab selection now requires the expected content after each
strategy. It tries accessibility activation, exact physical input, then an
exact focused-keyboard activation, advancing only after a bounded missing
content acknowledgement. The keyboard path re-resolves the enabled and visible
automation ID, foregrounds its owning HWND, sets and verifies keyboard focus,
then sends a discrete Enter down/up transaction. Failure diagnostics preserve
all strategy errors plus the final accessibility snapshot.

Two unit tests prove escalation order and aggregate diagnostics. The primary
baseline deliberately switches Interface & System through the keyboard path
and requires both `Show OSD` and `Language`; every final baseline and full
inventory reports `config-tab-focused-keyboard-activation`. Three focused
baselines and two complete ten-scenario inventories passed this real Windows
contract. Evidence is retained in
[`docs/evidence/test-coverage/native-input-ownership-20260729.md`](evidence/test-coverage/native-input-ownership-20260729.md).

## TC-HARNESS-011: native harness unit contracts were excluded from workspace tests

Status: **Resolved 2026-07-29; native binary test target automatically enrolled**

Severity: **Medium (contract tests existed but the normal broad gate skipped them)**
Detection: explicit focused regression-test execution

`sorotte-gui-native-smoke` contained menu, artifact, setup, control-identity,
capture, and input unit tests while its Cargo binary target declared
`test = false`. An explicit `cargo test --bin sorotte-gui-native-smoke` ran
them, but `cargo test --workspace --all-features` did not discover the target.

The binary target now declares `test = true`. The native harness currently
runs 25 tests, including the new tab-escalation and absolute-coordinate
contracts, and the final all-feature workspace gate is required to prove they
remain enrolled.

## TC-NATIVE-002: File -> Exit can leave the GUI process alive

Status: **Resolved 2026-07-29; bounded and observable runtime shutdown**

Severity: **High (application shutdown and native cleanup contract)**
Detection: stable-ID File -> Exit invocation plus process watchdog

The second isolated diagnostic found `menu.exit`, issued its physical click,
then waited the full 80-second contract without observing process exit:

```text
artifact: target/verification/gui-native-smoke/20260729T033324498Z-53816
runner duration: 82,260 ms
error: timed out waiting for sorotte-gui to exit after close request
native stderr: 0 bytes
```

The retained 5,611,593-byte screenshot (SHA-256
`2c35e09fe6ac4594a712439b137e2496a6bb1a9cd76ea7964581dd0cd03b4ec3`)
shows the still-present window in a disabled/closing-looking state. The
39,760-byte redacted UIA tree has SHA-256
`ebc33a9bb12a4c9d640708ea28c939de589582ddadf1a6af07f16a04ea781318`.
Harness cleanup then terminated the child; no GUI or Python process remained.

Resolution: the GUI runtime pump now has explicit idempotent shutdown. Its
shared owner publishes a stop request, production polling exits when that
request is observed, and the caller waits for worker completion through a
condition variable plus `JoinHandle::is_finished`. The normal path joins the
worker. A worker that does not cooperate is diagnosed and detached after a
two-second product bound so a stuck adapter cannot keep the desktop process
alive indefinitely.

When the opt-in native-test observation path is configured, the product writes
JSONL milestones containing its PID. The File -> Exit contract now allows four
seconds and requires exactly this ordered causal trace:

```text
exit-action-applied
viewport-close-requested
runtime-stop-requested
runtime-worker-stopped
app-drop-complete
```

The native report exposes that proof as required capability
`native.shutdown.file-exit`, sourced from
`accesskit+eframe+lifecycle-jsonl`. A deliberately blocked runtime-owner
regression proves pump destruction returns within its 60 ms test bound, while
a normal-owner regression proves the worker is still joined. The three
consecutive stressed baselines and both successful complete inventories all
observed the exact five-event trace and process exit; no shutdown timeout was
lengthened.

## Persistence process-interruption experiment

Status: **Implemented 2026-07-30; no product defect surfaced**

Risk: **Critical (durable room, statistics, and quota identity state)**
Detection: exact child-process termination followed by production SQLite
reopen, integrity checking, and idempotent recovery

The previous persistence suite already proved version arbitration, queue
saturation, degraded/recovered reporting, database replacement, ordinary
restart, concurrent quota-secret creation, and transaction rollback after an
injected SQL error. It did not prove what survives when the process disappears
between a successful write and the next production step.

`SRV-PERSIST-001` adds a child-process crash matrix. The parent invokes only the
exact helper test, supplies one test-only crash point, and requires exit code
86. The helper calls `std::process::exit` from the production persistence path,
so Rust destructors and SQLite connection cleanup do not run. The parent then
opens the same database, requires `PRAGMA integrity_check` to return `ok`,
checks the exact durable state, performs normal recovery, and opens it again to
prove idempotence.

The 15 interruption points are:

| Boundary | Points | Required reopen state |
|---|---:|---|
| legacy schema expansion | 5 | exactly the committed column prefix; the next open completes all columns and metadata without changing the legacy row |
| playlist JSON/index migration | 2 | all rows retain their legacy values before commit, or all rows contain canonical JSON and normalized indices after commit |
| room save and delete | 4 | every field of the old room before commit, or every field of the replacement / complete deletion after commit |
| multi-row stats snapshot | 2 | zero snapshot rows before commit, or all three rows after commit |
| quota-secret creation | 2 | no metadata row before insertion, or one stable 32-byte secret after insertion |

The crash variables are compiled only under `cfg(test)` and are honored only
when the exact child helper role is also set. Normal tests cannot arm the seam,
and no in-process global failpoint can leak into a parallel test. The existing
production transaction, migration, actor, stats, and quota-secret paths remain
unchanged outside the conditional observation calls.

The complete persistence selector passes 49/49 tests. Twenty consecutive
serial actor-suite runs passed 240/240 tests and performed 300/300 child
process interruptions without an integrity, completeness, or idempotence
failure. No expected failure was added and no product behavior was changed.

This proves process-termination atomicity at the selected SQLite boundaries;
it does not claim power-loss durability, kernel/filesystem cache persistence,
disk-full or permission failure at every SQLite syscall, or durability of an
actor message that has not reached a transaction. Those remain separate
fault/filesystem and queue-durability decisions rather than being implied by
this green contract.

The policy audit around this work also found two ledger defects. The TLS
finding had reused `TC-SERVER-001`, already assigned to the resolved playlist
migration defect, so it is now `TC-SERVER-003`. The Rust inventory scanner also
ignored multiline `should_panic(expected = ...)` attributes. The validator now
parses multiline attributes, rejects duplicate finding headings and
case-insensitive title drift. Its 21 focused tests pass, and the current
registry at that closure checkpoint validates explicitly as zero defects and
zero characterizations. The subsequent deep-boundary TLS slice is recorded
below as one defect and one exact characterization.

## TC-CLIENT-001: reconnect playlist restore lacks acknowledgement fencing

Status: **Resolved 2026-07-30; both characterizations are positive regressions**

Severity: **High (playlist intent can be lost or overwrite newer authority)**
Detection: shrinkable reconnect reference-model design plus deterministic
event-schedule reproduction

The reconnect playlist handoff has separate snapshot and one-shot intent
fields but no acknowledgement fence. Two short schedules expose opposite
failures around the same missing ownership boundary.

First, an empty reconnect snapshot arms restoration and the runtime drain emits
`Set.playlistChange` plus `Set.playlistIndex`. The drain consumes the only
restore intent. If the transport disconnects before the server echoes that
playlist, the next reset has neither a current playlist projection nor a
durable restore snapshot, so the second reconnect emits nothing:

```text
capture local playlist
disconnect -> Hello -> empty server playlist -> emit restore
disconnect before echo -> Hello -> empty server playlist -> no restore
```

Second, an empty server snapshot can arm the old local restore while a newer
non-empty authoritative update is already queued in the same GUI transport
batch. The non-empty update clears the pre-Hello snapshot but not the armed
intent, so the subsequent drain overwrites newer server authority with the old
local playlist:

```text
disconnect -> Hello -> empty server playlist
non-empty authoritative playlist -> drain -> stale local restore emitted
```

Both schedules reproduce in ordinary `sorotte-client-core` tests without
sleep, sockets, or timing tolerance.

The implemented state machine has a distinct
`playlist_restore_pending_ack` record. Draining an armed intent moves a clone
into that record rather than destroying the desired state. A transport reset
moves it back into the reconnect snapshot; a non-empty server playlist clears
the snapshot, armed intent, and acknowledgement fence; and a matching echo is
therefore both canonical playlist state and acknowledgement. Capability
disablement clears all three states rather than leaving an inert restore
behind.

The independent reference model now represents snapshot, armed, and
awaiting-acknowledgement states separately and compares all three with
production after every generated transition. The two former expected failures,
the matching-echo retirement regression, all reconnect playlist tests, and the
128-case generated history suite pass as ordinary positive tests. There is no
retry, clock tolerance, or defect classifier in that proof.

## TC-CLIENT-002: Reconnect reset retains in-flight reducer transactions

Status: **Resolved 2026-07-30; reconnect transactions are invalidated**

Severity: **High (a completion from the disconnected session can seek or pause the replacement session)**
Detection: exhaustive fresh-reference reset projection plus stale completion
injection

`reset_sync_state_for_reconnect` previously retained both in-flight pause
transactions and `local_pause_change_health`. A completion from the
disconnected player/session could therefore still match reducer state after
reset and mutate the replacement session.

The former expected-failure prefix was:

```text
TC-CLIENT-002: reconnect reset retains in-flight reducer transactions
```

The owning reset now calls
`cancel_connection_scoped_playback_transactions`, clearing both pending
transactions and restoring `local_pause_change_health` to `Healthy` before
fresh-session projection. The 24-seed complete reset oracle no longer
normalizes any field: every result equals a fresh reference exactly. The
positive `reconnect_reset_rejects_stale_reducer_completions` regression proves
that stale position and pause completions emit no follow-up effects and cannot
mutate the cleared player projection. Reset idempotence remains covered.

## TC-SERVER-003: TLS rotation max-mtime token can miss bundle-member changes

Status: **Resolved 2026-07-30; content fingerprint and snapshot parsing implemented**

Severity: **High (stale certificate or private-key material can remain active)**
Detection: deterministic TLS metadata-clock extraction plus a pure
bundle-token collision experiment

The server reduces the three required bundle-member modification times to one
maximum `SystemTime`. That loses member identity. A real edit is invisible
whenever it changes a member that remains older than a different member:

```text
before:             privkey=10, cert=30, chain=20 -> token=30
after privkey edit: privkey=11, cert=30, chain=20 -> token=30
```

The runtime compares only that token with its last observation, so it retains
the previously loaded TLS context and performs no validation or reload. The
same collision class includes content replacement that preserves all observed
mtimes. This can leave an intentionally rotated certificate or private key
unused until another bundle member happens to acquire a later timestamp.

The former first characterization called the production max-token reducer
with the exact timestamps above and panicked only at:

```text
changing any required TLS bundle member must change the rotation token
```

The former second characterization wrote a valid bundle, assigned explicit
member timestamps on the real filesystem, loaded the production runtime,
replaced the older private-key member with invalid contents, and restored its
timestamp below the unchanged certificate maximum. It proved the token
collision and then observed the runtime still answer `startTLS=true`, panicking
only at:

```text
rotating a required TLS bundle member must invalidate the cached context
```

A `(mtime, length)` tuple per member was rejected as incomplete because it
still misses equal-mtime, equal-length replacement. Production now reads all
three required members into one captured snapshot and computes a
domain-separated, filename- and length-framed SHA-256 fingerprint. Rotation
comparison uses that fingerprint, and rustls parses the exact bytes that were
fingerprinted, eliminating the previous observation/load race. The injected
test clock remains available only to drive deterministic revision histories;
ordinary production and the real-filesystem regression use content identity.

Positive tests prove that an equal-length edit to each individual member
changes the fingerprint, and that replacing the older private key while
preserving its timestamp below the unchanged maximum invalidates the cached
context. Missing members retain the prior legacy retry behavior, and an
invalid captured snapshot is never installed as a `ServerConfig`.

## TC-SERVER-004: Sequential TLS bundle reads can install a cross-generation snapshot

Status: **Resolved 2026-07-30; atomic generations are the preferred publication contract**

Severity: **High (one installed TLS context can combine key, certificate, and
chain material from different rotation generations)**
Detection: per-member replacement scheduling through the production snapshot
reader

The content-fingerprint fix for `TC-SERVER-003` correctly fingerprints the
exact bytes later parsed by rustls. It therefore closes the
observation-versus-parse race, equal-length edits, and metadata collisions.
The three bundle members are still read sequentially from independently
mutable paths, however. The server has no generation boundary proving that all
three reads came from one publication.

The former characterization built two complete, distinct, independently
rustls-loadable generations and injected replacement at both possible
mid-capture boundaries:

```text
boundary 1: private key A | certificate B | chain B
boundary 2: private key A | certificate A | chain B
```

At both boundaries, the captured fingerprint differed from the fingerprint of
complete generation A and complete generation B, yet rustls accepted the mixed
snapshot as a server configuration. The former exact oracle was:

```text
rustls must never install a TLS bundle assembled from multiple observed generations
```

This was not a fingerprint collision and could not be solved by adding more
file metadata. Production now implements the immutable, versioned publication
protocol:

1. `current.json` has a strict `sorotte-tls-bundle-v1` schema and names one
   constrained generation below `generations/`.
2. The selector authenticates exact `privkey.pem`, `cert.pem`, and `chain.pem`
   byte lengths and canonical lowercase SHA-256 digests. Unknown/duplicate
   fields, traversal identifiers, oversized files, symlinks, reparse points,
   and digest drift fail closed.
3. The reader captures the selector, reads only that immutable generation,
   authenticates every member, constructs a snapshot from those bytes, and
   rechecks the selector. A concurrent switch retries rather than installing
   the stale capture.
4. The SWAG publisher resolves all three live links to one immutable
   Let's Encrypt archive directory and numeric lineage before copying. It
   rehashes the sources after capture, renames the complete staged generation,
   calls `sync`, and atomically renames a fully written selector. A failed
   selector replacement leaves the previous selector byte-for-byte unchanged.
5. Older generations remain available for readers that observed the previous
   selector. Temporary staging and selector files are removed on interruption.

The positive Rust regressions switch `current.json` after each of the three
member reads and accept only complete generation B, keep partial and complete
but unselected generations invisible, reject path escape/digest drift/
duplicate fields, and retain the active runtime context without consuming a
rotation retry while a selected generation is unavailable. The executable
shell integration performs two successive publications, proves generation A
is immutable after B becomes current, injects failure at the selector rename,
and rejects a mixed Certbot lineage before any target state is staged.

For compatibility, an absent `current.json` still selects loose
`cert.pem`/`chain.pem`/`privkey.pem` files. The reader accepts only two
identical consecutive framed captures. This rejects observed replacement
boundaries, but a stable mixed loose directory remains unknowable; the
operator guides therefore identify loose mode as static or externally
serialized compatibility only, not generation-atomic rotation.

The discovery schedule and `TC-HARNESS-015` reproduction are retained in
[`deep-boundary-slice-20260730.md`](evidence/test-coverage/deep-boundary-slice-20260730.md).
The resolved reader/publisher contract, eleven parallel boundary streams,
new defect characterizations, stress counts, and integrated validation are
retained in
[`atomic-tls-parallel-continuation-20260730.md`](evidence/test-coverage/atomic-tls-parallel-continuation-20260730.md).

## Local all-feature LCOV proof

The pinned `1.97.1-x86_64-pc-windows-msvc` toolchain was installed with the
minimal profile and lacked `llvm-tools-preview`. cargo-llvm-cov therefore
prompted interactively and appeared hung in captured execution. Explicit
provisioning completed in 237.5 seconds, after which the experiment ran
directly on the pinned toolchain:

```text
rustc 1.97.1 (8bab26f4f 2026-07-14)
commit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452
LLVM version: 22.1.6
```

The fresh successful producer artifact was:

```text
path: target/fresh-diff-coverage.lcov
size: 15,089,306 bytes
sha256: 24a96fa660daae828293b67f6505c315b593aace64ae8a15a3df27e0195a62a5
source records: 392
LLVM summary: 145,926 / 187,537 lines = 77.81%
explicit DA inventory: 142,777 / 181,281 lines = 78.76%
```

This is proof that the locked all-feature workspace can execute under
instrumentation and emit LCOV locally. It is not valid evidence that either
changed-line threshold passed: PR enforcement uses the exact event-aware base
and source-bound production changed-line denominator, rejects structurally
inconsistent or unmapped executable lines, and publishes phase-aware JSON even
when base resolution, profile generation, either native export, conversion, or
policy evaluation fails.

## TC-PROTOCOL-001: Duplicate nested Set members retain collapsed execution entries

Status: **Resolved 2026-07-30; first-position/last-value semantics are uniform**

Severity: **High (client and server can execute the same decoded line
differently)**
Detection: generated duplicate-command composites plus a minimized
deterministic nested-`Set` reproduction

The protocol codec intentionally gives duplicate top-level commands
first-position/last-value semantics. Nested `Set` decoding also collapses a
duplicate field to its final payload value, but its separate `command_order`
ledger retains every source occurrence. For an input shaped like:

```text
Set: ready(false), file(A), ready(true), file(B)
```

the decoded payload contains only `ready(true)` and `file(B)`, while
`command_order` still contains `ready,file,ready,file`. Server normalization
consumes each optional field once. Client normalization instead follows the
order ledger and clones the collapsed payload for each retained occurrence,
so it can apply the final ready and file values twice. The wire input,
therefore, has no single execution meaning across consumers.

The former expected-failure characterization asserted that each collapsed
nested member appears once in execution order and panicked only at:

```text
collapsed duplicate Set members must appear once in command order
```

The compatible fix deduplicates nested `command_order` by decoded key in first
source position while retaining serde's final decoded value, exactly matching
the established top-level rule. Escaped spellings of the same JSON key share
one execution position. Rejecting all duplicate keys remains a possible future
protocol-hardening decision, but is not required to give current peers one
deterministic meaning. Both duplicate-`Set` tests are now ordinary positive
regressions.

## TC-PROTOCOL-002: Duplicate top-level Set uses discarded payload order

Status: **Resolved 2026-07-30; nested order follows the surviving payload**

Severity: **High (the decoded payload and its execution ledger can describe
different commands)**
Detection: escaped duplicate top-level command with disjoint nested `Set`
members

Top-level duplicate commands intentionally retain the first source position
and the final serde payload value. The raw ordering scanner previously
attached the nested `Set.command_order` from the first/discarded payload to
the final/surviving value.

The former expected-failure oracle was:

```text
surviving duplicate Set payload must determine nested command execution order
```

The scanner now retains the last matching top-level object span, including
escaped spellings of `Set`, while leaving the established top-level
first-position/last-value rule intact. The positive
`duplicate_top_level_set_uses_surviving_payload_order` regression proves that
the surviving `playlistIndex,room` value and its execution ledger come from
the same occurrence. Nested shadows, non-object values, and direct-member
ordering remain covered independently.

## TC-PROTOCOL-003: Decoded item Debug exposes credential-bearing unknown command

Status: **Resolved 2026-07-30; unknown command names are non-reflective**

Severity: **Medium (untrusted wire text can cross a diagnostic redaction boundary)**
Detection: credential canary embedded in an unknown top-level command name

`DecodedMessageLineItem` already wrapped its raw payload in
`RedactedJsonValue`, but its custom `Debug` implementation previously rendered
an unknown optional `command` string verbatim.

The former expected-failure oracle was:

```text
credential-bearing unknown command must not appear in diagnostics
```

`Debug` now uses an exact allowlist for `Hello`, `Set`, `List`, `State`,
`Chat`, `Error`, and `TLS`; every other command renders the fixed
`<unknown-protocol-command>` marker. The public field remains unchanged for
protocol handling. The positive canary regression proves that neither the
unknown command nor its credential fragment crosses the diagnostic boundary.

## TC-GUI-001: Version probe accepts unusable successful output

Status: **Resolved 2026-07-30; tool-specific banners are validated**

Severity: **Medium (an unusable or wrong executable is reported healthy and persisted as a media tool)**
Detection: real exit-zero child processes with empty, invalid-UTF-8, and
unrelated stdout

The media-match version probe previously treated every exit-zero process as a
healthy ffmpeg/ffprobe tool, including empty, invalid-UTF-8, and unrelated
output.

The former expected-failure oracle was:

```text
successful process without a valid tool version must be rejected
```

The probe now strictly decodes the first nonempty captured line, requires the
anchored tool-specific prefix (`ffmpeg version ` or `ffprobe version `) and a
nonempty version suffix, rejects the wrong tool, and rejects an incomplete
truncated banner. Complete and unterminated valid banners remain supported.
The real-process regression rejects empty, invalid-UTF-8, and unrelated
exit-zero output while preserving nonzero exit status.

## TC-GUI-002: Version probe deadlocks on finite output larger than pipe capacity

Status: **Resolved 2026-07-30; both child pipes are drained concurrently**

Severity: **Medium (a finite successful child is killed and falsely reported timed out)**
Detection: real child writes 512 KiB to piped stdout and exits

The timeout loop previously polled `try_wait` before draining either child
pipe. A finite producer could fill a kernel pipe, block before exit, and be
misreported as timed out.

The former expected-failure oracle was:

```text
finite fake-tool output must be drained while the process runs
```

Two drain workers now start immediately after spawn. Each retains at most
64 KiB while continuing to drain all excess bytes; the parent preserves the
deadline/kill/reap loop and joins both workers after exit or termination. The
positive fixture writes 512 KiB to each of stdout and stderr, exits
successfully, proves both captures are bounded and marked truncated, and
deletes the executable afterward to prove the process was reaped.

## TC-PLEX-001: Plex playable-part selection ignores filename and size evidence

Status: **Resolved 2026-07-30; selection uses ranked identity evidence**

Severity: **High (a remotely shared file can remain unplayable or stream the
wrong Plex version despite uniquely identifying metadata)**
Detection: candidate-order-independent Plex part selection across exact
filename, exact size, duration, and missing-metadata cases

The Plex metadata item lookup succeeded, but `choose_playable_part` previously
ranked every playable `Part` only by duration difference. The discovery matrix
executed 20 forward/reverse cases: 16 false ambiguities and four wrong-part
selections.

The former expected-failure oracle was:

```text
TC-PLEX-001: Plex part selection must use filename and size evidence
```

The selector now narrows by exact basename, ASCII-folded basename, exact byte
size, and nearest known duration in that priority order. A stage with no match
contributes no evidence, response order never breaks a tie, and only a single
remaining candidate can be streamed. Plain shared filenames and `plex://`
URIs both propagate their available hints. All 20 former mismatches now select
the independent oracle's part; forward/reverse permutations agree. Genuine
duplicates and unidentified multipart media still fail closed, and duration
remains the final tie-break. The existing exhaustive public `PlexError` match
contract also remains source-compatible.

## TC-GUI-003: Permanent Plex ambiguity repeats as a transient miss

Status: **Resolved 2026-07-30; ambiguity is terminal for its context**

Severity: **Medium (an unchanged deterministic failure repeats network work,
notifications, and system-chat warnings indefinitely)**
Detection: two fake-clock automatic resolution cycles for one unchanged
playlist-resolution key

The GUI previously collapsed every `PlexError` into a string and recorded
ambiguity as a transient miss, producing repeated work and identical
notifications on the 2/5/15/30 second schedule.

The former expected-failure oracle was:

```text
TC-GUI-003: permanent Plex ambiguity must warn once without automatic retry
```

Plex now exposes an ambiguity classifier without adding a source-breaking
public error variant. The worker converts that classification into a typed GUI
`PermanentForContext` failure. The miss state retains the disposition with no
deadline, emits one redacted warning and one system-chat event, and projects
`Failed` with actionable detail rather than promising another retry. Row,
playlist generation, target, policy, or Plex operation-context changes clear
the terminal state; explicit source selection therefore rearms resolution.
Ordinary cache misses, network failures, and worker interruption retain the
existing bounded backoff and later-success behavior.

Notification deduplication alone is rejected: it hides the repeated message
but continues failed network and cache work and leaves the UI state false.

The completed design for all seven defects, including exact invariants,
alternatives, and acceptance tests, is in
[`OUTSTANDING_DEFECT_REMEDIATION_DESIGN.md`](OUTSTANDING_DEFECT_REMEDIATION_DESIGN.md).
The candidate-permutation matrix, independent oracle, GUI state-machine
schedule, stress totals, and limitations are retained in
[`plex-part-selection-retry-20260730.md`](evidence/test-coverage/plex-part-selection-retry-20260730.md).
The positive conversion, stress repetitions, broad gates, and preserved native
retry evidence are recorded in
[`outstanding-defect-remediation-20260730.md`](evidence/test-coverage/outstanding-defect-remediation-20260730.md).

## TC-CLI-001: Managed attach waits through its deadline after the child exits

Status: **Resolved 2026-07-30; managed retry observes child liveness**

Severity: **Medium (failed player startup remains unnecessarily blocked)**
Detection: exact child-process early-exit barrier and bounded parent clock

Managed launch starts an owned child and polls for its IPC endpoint. The retry
loop does not inspect `Child::try_wait`, so an immediately exited player is
indistinguishable from a still-starting player until the full connection
deadline expires. The deterministic fixture publishes its start and exits;
the production attach path is configured with a 300 ms deadline and still
burns that deadline instead of returning within the characterization's 200 ms
early-exit bound.

The former expected-failure oracle is:

```text
managed attach must stop retrying when its child exits
```

The retry loop now checks `Child::try_wait` after every failed IPC connection
attempt and before sleeping. An exited child returns immediately with a stable
error prefix and the platform exit status; a live child retains the existing
transient retry behavior, unsupported mpv versions still fail after one
attempt, and the last sleep is capped by the remaining deadline. The existing
guard retains ownership, reaping, and IPC cleanup. The 300 ms fixture now
returns within the 200 ms bound as a positive regression.

## TC-CLI-002: Unmanaged external launch inherits CLI stdin

Status: **Resolved 2026-07-30; player subprocess stdio is isolated**

Severity: **High (the player can consume commands or data intended for the
interactive CLI)**
Detection: nested exact-process coordinator with separate stdin, stdout, and
stderr sentinels

The unmanaged external-player path explicitly sends child stdout and stderr to
the null device but leaves stdin unspecified. `Command` therefore inherits the
CLI's stdin handle. The fixture proves a parent stdin token reaches the child
even though the child's stdout and stderr sentinels cannot leak back into the
parent. In an interactive session, an external player can race the CLI for
input or consume data intended for Sorotte.

The former expected-failure oracle is:

```text
external launch must not inherit the CLI stdin handle
```

The complete fix applies `.stdin(Stdio::null())` to both unmanaged and managed
player `Command` construction. Sorotte does not use stdin as a player-control
channel; mpv control is through IPC, so no supported behavior depends on
inheritance. The nested subprocess test now positively proves the sentinel
cannot reach the child, independently of the existing stdout/stderr and
detached-ownership proofs.

## TC-CLI-003: Connected-session select cancellation drops fragmented inbound protocol prefixes

Status: **Resolved 2026-07-30; partial framing state survives selected-read cancellation**

Severity: **High (ordinary TCP fragmentation can corrupt an otherwise valid
server frame and disconnect the client)**
Detection: gated partial-read and read-future-cancellation barriers through
the production CLI connected-session runner

`read_inbound_protocol_line` consumes available socket bytes into a
future-local `Vec`. The connected-session loop constructs that read directly
inside its outer `tokio::select!`. If another ready branch wins before the
line delimiter arrives, dropping the read future also drops the already
consumed prefix. The next read begins at the remaining suffix and reports
misleading JSON errors for a valid frame.

Two ordinary, non-ignored tests make the schedule exact. The
loopback peer first publishes a partial valid Hello; a test-only task-local
observer confirms those bytes were consumed. Closing a supplied local-input
channel forces a competing branch, and the observer confirms the partial read
was cancelled before the peer releases the remainder. One case continues one
application byte at a time; the other gates precisely between `\r` and `\n`.
The former expected-failure oracle was:

```text
TC-CLI-003: fragmented inbound protocol read lost bytes before the CRLF delimiter
```

`InboundProtocolLineReader` now owns the partial buffer for the lifetime of the
connected session. Each selected read borrows that state, so dropping the
future cannot discard an already consumed prefix. Completed frames move the
buffer out for UTF-8 decoding, while terminal I/O and line-limit errors clear
it. The same forced schedules are now positive:

```text
tests::raw_protocol_framing::one_byte_fragmentation_survives_select_cancellation
tests::raw_protocol_framing::split_crlf_survives_select_cancellation
```

Both accept the complete released Hello and reach
`ConnectedSessionExit::TransportClosed`.

The same slice positively proves server-side one-byte fragmentation, split
CRLF, coalescing, valid-prefix/fault-suffix ordering, truncation, half-close,
and peer isolation, plus every unaffected CLI framing outcome. Fifty serial
repetitions of each boundary selector passed. The initial ungated split-CRLF
test independently failed on stress iteration 12 before the deterministic
cancellation barrier was added.

The complete matrix, root-cause trace, correction, commands, and limitations
are retained in
[`raw-loopback-framing-20260730.md`](evidence/test-coverage/raw-loopback-framing-20260730.md).

## TC-PROTOCOL-004: Protocol floating-point values can drift across decode and re-encode

Status: **Resolved 2026-07-30; exact raw and typed float roundtrips restored**

Severity: **Low (ordinary synchronization magnitudes are not known to hit the
counterexample, but accepted finite JSON numbers do not satisfy the advertised
exact roundtrip invariant)**
Detection: pinned Linux libFuzzer/AddressSanitizer campaign over every public
protocol line decoder and encoder

The first 45-second coverage-guided parser run found the minimized five-byte
input `70E70` after 108,863 executions. `decode_line` represents that finite
JSON number as `7.000000000000001e71`; `encode_line` emits a decimal which the
same decoder reads as the adjacent `7.000000000000002e71`. The same one-ULP
change is reproducible inside the valid typed frame
`{"State":{"playstate":{"position":70E70}}}`. This disproves the prior exact
`Value` and `ProtocolMessage` roundtrip oracle without requiring malformed
input, a panic in production code, or sanitizer-detected memory unsafety.

Two ordinary, non-ignored expected-failure tests originally bound the raw and
typed counterexamples to:

```text
TC-PROTOCOL-004: protocol floating-point value changed across decode/encode/decode
```

The historical continuation classifier passed 559,788 executions in its first
45-second run. After provenance binding, a fresh pre-fix 180-second canonical
campaign passed 1,915,137 executions over SHA
`729214d0de7ced9c56da7361bda68dc75b831179`, with stable 29-file source and
14-file seed manifests, no artifact, and no independent failure.

The fix keeps serde_json pinned at 1.0.151 and enables its
`float_roundtrip` feature in the workspace and standalone fuzz package. The
raw and typed characterizations are now positive exact-equality regressions,
and both minimized inputs are checked-in seeds in a 16-file corpus. The
one-ULP classifier was deleted; raw `Value` and typed `ProtocolMessage`
roundtrips are unconditional exact assertions.

A fresh post-fix 180-second campaign over
`034e10511ae6473f0165f3028a026a0bad4f6db3` passed 1,994,358 executions,
added 7,163 corpus units, peaked at 533 MiB, retained stable 29-file source and
16-file seed manifests, and produced no artifact. The full toolchain identity,
source binding, counterexample, historical and exact-oracle continuations, and
limitations are retained in
[`protocol-coverage-guided-20260730.md`](evidence/test-coverage/protocol-coverage-guided-20260730.md).
The combined implementation diff, positive selectors, empty-registry proof,
native launch characterization, WSL campaign, and evidence hashes are in
[`outstanding-defect-resolution-20260730.md`](evidence/test-coverage/outstanding-defect-resolution-20260730.md).

## TC-UPDATER-001: Tampered prepared file prevents safe update rollback cleanup

Status: **Resolved 2026-07-30; uncommitted scratch is safely disposable**

Severity: **Medium (a detected staging fault can permanently block automatic
updates until manual cleanup)**
Detection: exact release-package updater experiment plus a minimized
post-preparation mutation hook

The release artifact consumer authenticated the exact GUI ZIP, launched its
installed updater, and then changed the contents of the already-prepared
`README.md` temporary file before replacement. The updater correctly rejected
that file's digest. Rollback then revalidated the same untrusted temporary
against the intended replacement digest before discarding it, returned
`rollback was incomplete`, and retained both the recovery journal and corrupt
temporary. A subsequent recovery repeats the same validation failure, even
though the original target remains recognizable and no valid rollback step
needs the temporary file.

The exact-package experiment reached this state after an earlier file had been
replaced; the reverse rollback restored that earlier target but still retained
the journal because the corrupt temporary entry failed. The minimized former
expected-failure characterization mutates a prepared file before its first
replacement and asserts the stronger desired invariant:

```text
tampered prepared replacement must not prevent rollback of an unchanged install
```

Rollback now treats an uncommitted temporary as disposable scratch: it retains
link/reparse-point and regular-file checks, but does not require the intended
replacement digest before removal. Target and backup authentication remains
strict because those files determine installed state, and committed-journal
cleanup still authenticates residual artifacts. One positive regression proves
an unchanged install, corrupt temporary, and journal are cleaned; a second
mutates the second prepared file after the first target was replaced and proves
both originals and every transaction artifact are restored. Quarantining
corrupt scratch for forensics was rejected as unnecessary state for this local
updater.

## TC-UPDATER-002: Updater transaction directory entries lacked a durability boundary

Status: **Resolved 2026-07-31; parent-directory sync is required after owned
entry mutations**

Severity: **High for storage durability (file contents could be acknowledged
without making the containing directory entry durable)**

Detection: narrow parent-directory-sync characterization followed by a
13-schedule deterministic storage-fault matrix and a real reversible Windows
share-denial probe

The updater wrote, flushed, and `sync_all`ed its recovery journal and prepared
files. It did not synchronize their containing directories after creating the
journal or prepared files, replacing/renaming targets, restoring rollback
state, deleting transaction artifacts, or removing the journal. The existing
11-boundary process-interruption suite therefore proved recovery after visible
file-content flushes but could not establish an OS-requested durability
boundary for the corresponding directory entries.

Before the fix, the narrow characterization failed with:

```text
TC-UPDATER-002: updater transaction completed without reaching a parent-directory sync boundary
```

Production now opens and `sync_all`s the parent after every updater-owned
entry mutation. Unix opens the directory directly. Windows uses a
write-capable directory handle with `FILE_FLAG_BACKUP_SEMANTICS` and
read/write/delete sharing, reaching `FlushFileBuffers`. Incomplete initial
journal writes remain disposable; a failure after a complete authenticated
uncommitted journal retains it for rollback; failures after the synced commit
record retain committed state for forward cleanup.

The positive regression injects failure at the first directory-sync boundary
and requires authenticated rollback and idempotent cleanup. The wider matrix
crosses 13 write, file-flush, replacement, removal, and parent-sync schedules
and requires complete old or complete new target bytes, authenticated journal
state, two successful recovery passes, an unchanged sibling sentinel, and no
remaining artifacts. A nonce-owned Windows directory held with exclusive
sharing produces the real sync denial; after the handle is released, the same
directory sync succeeds. The complete updater binary passes 33/33, including
all 11 real process-termination boundaries, and the installed-updater
integration retains both exact passing tests.

This closes the missing production syscall boundary. It does not claim that a
successful OS flush survives controller write-back caches, torn sectors,
kernel panic, physical power loss, or filesystems/storage stacks not executed
here. Exact schedules, commands, Microsoft API references, and limitations are
retained in
[`updater-transaction-storage-durability-20260731.md`](evidence/test-coverage/updater-transaction-storage-durability-20260731.md).

## TC-GUI-004: Automatic direct HTTP media had no player candidate

Status: **Resolved 2026-07-31; trusted direct HTTP(S) media enters automatic
resolution**

Severity: **High for remote-media usability (a session playlist accepted the
URL but never asked the attached player to load it)**

Detection: physical native Open Media through the strict
GUI/session/real-mpv/faulting-HTTP vertical

The first native RED retained the selected HTTP URL in the shared playlist and
completed its strict session exchange, but the loopback media server received
no request and mpv received no `loadfile`. Automatic resolution treated every
playlist string without a local origin as a local/media-search target. A
direct HTTP(S) media URL therefore produced no candidate, even after URL trust
preflight had accepted it.

Production now parses the target and admits only `http` or `https` URLs with a
host whose existing stream classifier identifies them as
`DirectMediaUrl`. The candidate remains behind the existing room trust check
and uses the ordinary synchronous core load path. Extractor pages such as a
YouTube watch URL still use Stream Support; `ftp`, custom schemes, malformed
URLs, and untrusted hosts remain unresolved.

Positive owner-level regressions cover direct HTTP and HTTPS acceptance,
unsupported-scheme rejection, trust rejection, and extractor-page
non-bypass. The strict native vertical then reaches the loopback server and
the real mpv process. The initial RED is retained at:

```text
target/verification/gui-real-mpv-faulting-http-recovery/20260731T005239395Z-31476
```

## TC-GUI-005: Same in-flight remote load could be submitted repeatedly

Status: **Resolved 2026-07-31; command acceptance, media activation, and row
reprojection are independently fenced**

Severity: **High for recovery correctness (duplicate `loadfile` commands abort
the predecessor before it can establish media identity)**

Detection: strict session fixture plus real-mpv log and deterministic
stable-generation row-identity regression

After `TC-GUI-004` was fixed, one strict native RED showed four identical
`loadfile` commands within 37 milliseconds. Each later command aborted the
predecessor while mpv was still opening the same URL. Two adjacent lifecycle
assumptions enabled the loop:

- a completed JSON-IPC command reply could be treated as media activation even
  though it proves only that mpv accepted `loadfile`; and
- a fresh `GuiPlaylistEntryId` projection could replace the row-scoped
  `Loading` attempt even while the physical player placeholder still
  represented the same target.

Production now retires command correlation on `Completed` but keeps the
attempt `Loading` behind an explicit media-confirmation fence. A matching path
observation while mpv is merely opening cannot substitute for
`file-loaded`; an authoritative physical-file-loaded snapshot or matching
media-success event clears the fence. Same-target row reprojection migrates
the new row-scoped attempt onto the already in-flight physical load instead of
issuing another command. A real terminal media failure still permits a
same-target retry, and a genuinely different remote target still supersedes
the old attempt.

The deterministic integration regression holds playlist generation and remote
revision stable while forcing four distinct row IDs. It requires exactly one
player open, then proves confirmed activation, terminal same-target retry, and
different-target supersession. Focused lifecycle tests separately cover
command-before-media and media-before-command order, pre-activation matching
path telemetry, superseded late completion, and authoritative snapshot
reacquisition. The duplicate-load RED is retained at:

```text
target/verification/gui-real-mpv-faulting-http-recovery/20260731T024225197Z-15772
```

## TC-PLAYER-004: Keep-open premature EOF never reached network recovery

Status: **Resolved 2026-07-31; an active-attempt keep-open EOF can start the
existing bounded same-generation recovery**

Severity: **High for interrupted remote VOD recovery (playback stopped early
while Sorotte remained attached and never requested the complete response)**

Detection: malformed chunked loopback HTTP response through the real native
GUI and exact installed mpv

The recovery adapter already subscribed to mpv's `eof-reached` property and
retained coherent attachment, generation, attempt, network path, duration, and
position evidence. It deliberately treated that property as provisional and
waited for `end-file` before starting the same-generation reload. Sorotte also
launches mpv with `--keep-open=always --keep-open-pause=yes`.

The final native RED transferred 720,000 valid bytes from a declared
45-second AU stream and then injected an invalid HTTP chunk-size line. mpv
logged a curl receive failure and libavformat EOF, played the available bytes,
published `eof-reached=true` around 7.5 seconds, and paused. Under keep-open it
did not publish `end-file`, so Sorotte's recovery transaction never ran. A
preceding valid finite-response RED was retained separately and rejected as a
transport oracle: a deliberately complete 720,000-byte HTTP response ending
normally is not evidence of a broken transfer.

Production now lets the exact active-attempt provisional EOF invoke the
existing recovery transaction. The same guards remain authoritative:

- network VOD only, with live and local media excluded;
- coherent non-seeking duration and position evidence;
- more than 15 seconds remaining;
- exact attachment, generation, and load-attempt identity;
- two consecutive and five total attempts at most; and
- progress, seek, restart, replacement, and late-event fencing.

The successor retains the logical generation, does not publish an early
terminal phase, and uses the retained position as its resume target. Positive
tests cover the keep-open property without any `end-file`, near-tail and local
exclusions, contradictory provisional evidence, retry budgets, and ordinary
`end-file` recovery. The strict real-mpv GREEN additionally requires exactly
one malformed first GET, one complete recovery GET, stable PID/IPC/URL/media
identity, resumed progress, native pause/exit, and complete socket/process
release.

The decisive RED is retained at:

```text
target/verification/gui-real-mpv-faulting-http-recovery/20260731T041125117Z-34960
```

The exact native fault/recovery outcome, strict request/session/process
oracles, intermediate harness REDs, and limitations are retained in
[`native-gui-real-mpv-faulting-http-recovery-20260731.md`](evidence/test-coverage/native-gui-real-mpv-faulting-http-recovery-20260731.md).

## TC-MEDIA-001: Production V3 timeline maps encoded unity as an invalid affine scale

Status: **Resolved 2026-07-31; production maps use an absolute affine scale
and current-position diagnostics are wired**

Severity: **Low for current users, medium for the mapping contract (the path
was debug-only, but every ordinary same-speed production map was unmappable)**

Detection: repository-wide known-issue audit followed by the new GUI
current-position summary regression

The executable known-defect registry, every prior `TC-*` finding, and the live
GitHub issue/PR inventory were empty. The only actionable source marker was a
duplicated TODO to thread the active session's local playback position into
Media Match V3 debug evidence. The first positive regression remained RED even
though ranking produced a valid one-segment `SameCutProbable` timeline map.

The cause was a representation mismatch at the map-construction boundary.
`MediaTimelineAlignment.scale_ppm` is drift from affine unity, so ordinary
same-speed media records `0`. `AlignedSegmentV3.scale_ppm` is the absolute
affine multiplier consumed by the forward and reverse mappers, where unity is
`1_000_000` and nonpositive values are invalid. Production copied the drift
value directly, so both mapper directions returned `None`.

Production now converts checked drift to the positive absolute affine scale
when it builds a timeline segment. Type documentation and debug labels make
the two units explicit. The GUI snapshots `local_position_seconds()` only
alongside a resolved current local path, carries it through both rebuild
request paths, converts only finite nonnegative timestamps inside the
`u32`-millisecond domain, and appends a mapped timestamp only to
`last_evidence`. Visible decisions, candidate order, readiness, autoplay,
seek, and synchronization behavior are unchanged. The existing mapper remains
fail closed outside an aligned segment, including edit gaps.

Positive regressions prove:

- a production sampled-audio decision emits affine unity and round-trips a
  position through both mapper directions;
- the GUI summary changes only debug evidence and reports the mapped candidate
  timestamp;
- a no-op persisted rebuild carries the position snapshot into its published
  evidence;
- missing, non-finite, negative, and out-of-domain positions are omitted; and
- a position inside an edit gap is not inferred.

The owning `sorotte-media-match` suite passed 84/84 tests. The owning
all-feature GUI suite passed 1,131 tests with its two registered ignores, plus
41 native-harness, 14 startup-benchmark, 33 updater-binary, and two updater
integration tests. Warning-denied all-target/all-feature Clippy passed for
both crates. `coverage/known-defects.toml` therefore remains explicitly empty.

## TC-MEDIA-002: Windows manifest activation aborted on transient sharing denial

Status: **Resolved 2026-07-31; the exact durable replacement retries only
transient Windows access conflicts within a fixed budget**

Severity: **Medium for Windows background index-refresh availability; low for
integrity because the failed activation preserved the preceding generation**

Detection: complete locked all-feature workspace test after resolving
`TC-MEDIA-001`

The 100-generation retention regression failed at epoch 25 when
`MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` returned
Windows error 5 while replacing `current-b.json`. The staging generation was
not activated, the previous two generations and both valid epoch-25 manifest
replicas remained intact, and no test process survived. The exact pre-retry
filesystem is preserved at:

```text
C:\Users\shaun\AppData\Local\Temp\sorotte-media-index-bounded-generations-live-59836-1785475960691087100
```

The activation path previously made one durable replacement attempt.
Short-lived filesystem scanner, indexer, or other noncooperating handles can
deny delete sharing on Windows even though a subsequent identical operation
is safe. Production now retries that same operation only for raw Windows
access-denied (5), sharing-violation (32), and lock-violation (33) errors. It
makes at most eight attempts with 5, 10, 20, 40, 80, 100, and 100 millisecond
delays, for at most 355 milliseconds of waiting. The operation retains
`MOVEFILE_WRITE_THROUGH`; nontransient errors still fail immediately, and a
persistent transient error returns the final native error without activating
or deleting the prior generation.

Deterministic policy regressions inject all three transient errors before
success, exhaust the exact retry budget under persistent denial, and prove an
unrelated native error is attempted once. The complete
`sorotte-media-match` suite passed 84/84 tests, warning-denied all-target
Clippy passed, and the original 100-generation retention regression passed 20
consecutive runs (2,000 activation cycles). The known-defect registry remains
empty. The complete workspace, policy, and final three-mode real-mpv results
are retained in
[`outstanding-known-issues-closure-20260731.md`](evidence/test-coverage/outstanding-known-issues-closure-20260731.md).

## TC-CLI-004: CLI argument composition did not preserve legacy occurrence semantics

Status: **Resolved 2026-07-31; generated parser/composition regression is
positive**

Severity: **Medium (valid legacy arguments could be rejected or compose into a
different startup configuration)**

Detection: five exact parser regressions plus a 256-case independent
configuration-composition oracle

The handwritten legacy parser accumulated final optional values without
representing the result of each occurrence. That produced four related
failures:

- attached long/short values such as `--host=value` were treated as unknown;
- a later empty optional value could not clear an earlier CLI-layer override;
- a later host without a port retained the earlier CLI port; and
- missing required host/name values were silently accepted.

The parser now represents each occurrence as unchanged, replace, clear, or
invalid. Host and optional port are one atomic CLI-layer value. Attached
long/short forms enter the same path as separated values; an empty attached
optional value can clear only the preceding CLI override; optional missing
room/password values retain legacy fall-through; and required host/name
occurrences without a value fail closed.

The fixed-seed campaign renders 16 scenario patterns 16 times, producing 208
valid and 48 invalid cases. Its model independently applies environment,
stored-setting, and CLI precedence without importing the production parser,
host parser, controlled-room normalizer, or override helper. The complete
focused module passes 6/6 and the owning CLI library passes 366 tests with its
eight registered ignores. Exact RED/GREEN counts and limits are retained in
[`cli-argument-configuration-composition-20260731.md`](evidence/test-coverage/cli-argument-configuration-composition-20260731.md).

## TC-CLI-005: Unknown attached option diagnostics reflected raw values

Status: **Resolved 2026-07-31; attached values are redacted at the diagnostic
boundary**

Severity: **High for diagnostic privacy (an unrecognized credential-shaped
value could be reproduced in user-visible output)**

Detection: generated credential canary in an unknown attached option

Before the correction,
`--api-token=CLI_UNKNOWN_OPTION_SECRET_CANARY` was retained verbatim in the
unknown-option diagnostic. Parser rejection was correct, but reflecting the
full token crossed the diagnostic redaction boundary.

Unknown arguments now retain bounded option identity while replacing
everything after the first `=`. Startup uses that same formatter. The focused
regression and every generated case require all server-password,
controlled-room-password, explicit-password, and unknown attached-value
canaries to be absent from production `Debug` and diagnostic output. This is
separate from `TC-CLI-004`: even a deliberately unsupported option must not
reflect an attached secret.

## TC-PLAYER-005: Sustained cache stalls did not trigger bounded same-generation recovery

Status: **Resolved 2026-07-31; independently approved deterministic and real-mpv
proofs are positive**

Severity: **High for stalled remote VOD recovery (valid framed input could stop
making progress indefinitely while Sorotte remained attached)**

Detection: strict valid-framing, byte-silent IPv4-loopback HTTP response through
the native GUI and exact installed mpv

The first campaign declared the complete 45-second AU length, sent exactly
720,000 valid bytes, retained the open response, and then emitted no byte or
EOF. mpv reached `paused-for-cache=true` near the deterministic playable-prefix
boundary, but Sorotte issued no second GET or reload. A second RED after the
first ordering correction proved that response acceptance alone had not
retained enough lifecycle evidence.

Two orderings contributed. `start-file` and `playback-restart` can precede the
authoritative playlist snapshot that binds an accepted attempt. The restart
could be discarded with no active attempt or projected onto a retained
predecessor and then cleared while binding its successor. At the first cache
pause, finite duration/path evidence could also leave classification `Unknown`;
requiring an already settled `Vod` label prevented a real finite VOD from
arming the watchdog.

Production now retains only the restart causally newer than the exact deferred
start, gives the accepted deferred successor priority over a retained
predecessor, and replays that restart once after authoritative binding. A
finite `Unknown` timeline can arm only when neither `SlidingLive` nor
generation-bound `ytdl_is_live` is positive. Attachment, generation, attempt,
network path, duration, position, remaining duration, cache pause, and retry
budget remain required.

The full player suite passed 427 tests with two registered ignores. The
finding's original post-gate native bundle
`target/verification/gui-real-mpv-stalled-http/20260731T115707208Z-35432`
passed 18 assertions and 11 artifacts using GUI SHA-256
`a680ec8323011e4083c51b2de64473f8a4b9ef1aef8507131d03eb721e22bab3`
and mpv SHA-256
`2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0`.
It retained 29,423 milliseconds of server-side silence, zero EOF observations
before recovery, exactly one same-process `end-file` reason `stop`, one
complete recovery GET, resumed progress, native pause, and complete cleanup.
All REDs and the closed validator contract are retained in
[`native-gui-real-mpv-stalled-http-recovery-20260731.md`](evidence/test-coverage/native-gui-real-mpv-stalled-http-recovery-20260731.md).

## TC-HARNESS-018: Generated Media Match fixture could not satisfy its required tier

Status: **Resolved 2026-07-31; hosted real-tool execution is positive**

Severity: **Harness correctness (the new required real-tool lane failed without
distinguishing fixture reachability from product matching)**

Detection: hosted required generated-media job `91093403053` in workflow run
`30610965479`

The first 30-second 440 Hz fixture reached real ffmpeg/ffprobe extraction and
retrieval but could schedule only one 20-second sampled-fast window. Its STFT
landmark span could not satisfy the `Probable` same-cut minimum, and its
stationary periodic signal populated competing offsets. The final assertion
reported only a JSON boolean, hiding the decision tier/class and retrieval
margin that explained the RED.

The corrected fixture is 120 seconds of fixed-seed broadband noise using
built-in FFV1/PCM codecs. It exercises all three non-overlapping sampled-fast
windows and retains the `Probable` requirement. A typed assertion now reports
reason, retrieval, rank, tier, class, decision notes, and top retrieval
diagnostics before JSON serialization is checked. The integration target
compiles and the ordinary 84-test media-match suite plus strict crate Clippy
pass locally. Hosted job `91111808305` in workflow run `30616813538`
successfully executed the corrected real ffmpeg/ffprobe capability body.

## TC-HARNESS-019: Unquoted Git commit dereference failed hosted shell policy

Status: **Resolved 2026-07-31; hosted preflight reached its next assertion**

Severity: **Harness portability (required Linux checks and mpv semantics stopped
before behavior execution)**

Detection: actionlint/ShellCheck and hosted mpv source-revision preflight in
workflow run `30610965479`

The workflow passed `HEAD^{commit}` unquoted inside a shell command
substitution. Shell parsing/policy rejected the brace-bearing revision before
the intended identity comparison. The preflight now calls
`git rev-parse 'HEAD^{commit}'`, and a workflow-policy regression binds that
form. Local actionlint and the focused CI policy suite pass. The correction
changes only source-identity verification. Workflow run `30616813538` passed
this quoted dereference and reached the separate annotated-tag object
comparison recorded as `TC-HARNESS-026`.

## TC-HARNESS-020: Partial Rust component setup conflicted with repository toolchain activation

Status: **Resolved 2026-07-31; hosted component setup is positive**

Severity: **Harness provisioning (required semantic and coverage jobs failed
before their behavior lanes)**

Detection: hosted GUI semantic and coverage jobs in workflow run
`30610965479`

Several jobs installed the pinned toolchain with either no components or only
`llvm-tools-preview`. Later repository toolchain activation attempted a lazy
component install and encountered the `rustfmt-preview`/`cargo-fmt` component
conflict. Every Rust setup in the CI, coverage, and mutation workflows now
declares `rustfmt, clippy`; coverage producers additionally declare
`llvm-tools-preview`. Static workflow tests require the complete component
sets. Local workflow policy and actionlint pass; hosted confirmation remains
positive in workflow run `30616813538`, where every required Rust setup
completed before its behavior or later independent harness assertion.

## TC-HARNESS-021: Legacy permanent-room scenarios could start before room loading completed

Status: **Resolved 2026-07-31; hosted complete live compatibility is positive**

Severity: **Harness correctness (strict parity could compare transient legacy
startup state with Sorotte durable state)**

Detection: hosted Ubuntu server-release verifier job `91093403065` in workflow
run `30610965479`

Pinned Syncplay v1.7.5 begins accepting TCP connections before its asynchronous
Twisted `adbapi` room-loading callback necessarily finishes. The hosted
scenario joined `permanent-room` during that window and received a transient
ordinary-room null playlist index rather than the configured permanent room's
seeded index zero.

When permanent rooms are configured, the live runner now connects a GUI probe
in a collision-safe `-temp` room, polls public `List` responses until every
configured key is an object, half-closes its write side, and waits for peer EOF
before scenario clients connect. Readiness and cleanup share the existing
six-second fail-closed bound. No sleep-only stabilization, trace normalization,
or Sorotte product change was added. The default compatibility suite passes
138/138 and the complete strict live selector passes 20/20 locally. Exact
evidence is retained in
[`legacy-permanent-room-startup-readiness-20260731.md`](evidence/test-coverage/legacy-permanent-room-startup-readiness-20260731.md);
hosted job `91111808378` passed all 138 executable compatibility tests with
seven exact writing fixtures and no unaccounted skip.

## TC-HARNESS-022: Portable external-player tests hid their helper on non-Windows hosts

Status: **Resolved 2026-07-31; hosted process probes are positive**

Severity: **Harness compilation (the Linux all-feature lane could not compile
portable external-launch tests)**

Detection: lifecycle evidence compilation in workflow run `30610965479`

`spawn_legacy_external_player_from_spec_legacy_compatible` was imported at the
CLI crate root only under `cfg(all(test, windows))`, while portable
external-launch tests use it on Linux. The helper itself is test-only but not
Windows-only. Its import is now gated by `cfg(test)`; only genuinely
Windows-specific IPC helpers retain the platform gate. The complete
external-launch module passes 15/15 locally. This is a test visibility fix, not
a production launch change. Hosted Linux all-feature job `91163394469` in
workflow run `30632931277` passed this boundary; the workflow's later
coverage-finalization failure is recorded separately as `TC-HARNESS-044`.

## TC-HARNESS-023: Hosted PowerShell media-tool fixtures exceeded their process deadline

Status: **Resolved 2026-07-31; hosted Windows server-release boundary is positive**

Severity: **Harness portability (two Windows process-probe tests timed out
under hosted load)**

Detection: hosted Windows server-release verification in workflow run
`30610965479`

The Windows media-tool version fixtures launched a new PowerShell process for
small stdout/stderr cases. Hosted startup and quoting cost could exceed the
five-second process bound even though the production timeout/reap behavior was
not at fault. Windows fixtures now use `cmd.exe` for the finite banner, empty,
unrelated, and nonzero-exit cases. Invalid UTF-8 is passed directly to the
extracted production parser, preserving that oracle without another shell.
The exact version-probe tests pass 4/4 and strict GUI-library Clippy passes
locally. The Windows server-release verifier in workflow run `30616813538`
passed those probes before a later independent player fixture exposed
`TC-HARNESS-028`. The corrected Windows server-release job `91163394510` in
workflow run `30632931277` later passed; that workflow's coverage finalization
remained a separate downstream concern.

## TC-HARNESS-024: Nextest could execute the parked child fixture as the parent test

Status: **Resolved 2026-07-31; hosted Windows nextest is positive**

Severity: **Harness liveness (the Windows all-feature nextest job could hang
inside a fixture role instead of running its owning assertion)**

Detection: hosted Windows all-feature nextest execution in workflow run
`30610965479`

The media-tool process fixtures recognized a child role solely from
`--exact <test-name>`. Nextest itself launches each ordinary test with that
shape, so the real parked-fixture test process could enter its intentional
infinite park before the owning timeout/reap test had copied and launched it.

Fixture dispatch now requires both the exact target arguments and the
nonce-owned copied executable stem `fake-media-match-tool`. A regression proves
that an ordinary `sorotte_gui` nextest invocation cannot become either child
role, while the copied image with the exact target can. The complete process
module passes 8/8 and strict GUI-library Clippy passes locally. The correction
is commit `a5ae5be`; the complete Windows nextest step passed in workflow run
`30616813538`.

## TC-HARNESS-025: Hosted coverage environment parsing retained shell syntax and a fixed merge-pool width

Status: **Resolved 2026-07-31; hosted coverage producer environment is positive**

Severity: **Harness portability and provenance (the required changed-line
coverage lane stopped before profile generation)**

Detection: coverage-diff job `91111808430` in workflow run `30616813538`

Pinned cargo-llvm-cov 0.8.4 emitted stable environment content whose
`LLVM_PROFILE_FILE` was POSIX-quoted and used `%4m`, derived from hosted
parallelism. The consumer retained the quotes as path bytes and required the
local `%32m` spelling. After quote removal, either condition independently
rejected the producer-owned target path.

Both coverage producers now request `cargo llvm-cov show-env --sh`, require
exact `export KEY=VALUE` lines, decode exactly one nonempty POSIX word with
`shlex` and no evaluation, and accept one producer-selected positive `%Nm`.
The profile basename is parsed left-to-right like an LLVM percent-token
stream: exactly one real `%p`, exactly one canonical positive uint32 `%Nm`, no
doubled or unknown percent token, and a `.profraw` suffix. Regressions cover
quoted spaces/apostrophes, pool widths 1, 3, 4, 32, and `UINT32_MAX`, malformed
quotes, multiple words/tokens, doubled escapes, leading zero, zero, and
overflow. Both focused lane modules pass 42/42.
Coverage job `91169713196` in workflow run `30632931277` successfully parsed
the hosted producer environment, generated its profiles and maps, and passed
policy before the separate `TC-HARNESS-044` evidence-finalization defect.

## TC-HARNESS-026: Annotated mpv tag identity was compared with its peeled commit

Status: **Resolved 2026-07-31; hosted minimum-mpv source identity is positive**

Severity: **Harness source identity (the required real-mpv lane stopped before
building its immutable supported source)**

Detection: mpv-pr-semantics job `91111808391` in workflow run `30616813538`

The workflow pinned annotated tag object
`2c219aa822df18a1b7fd9abe3e151cd93ad67307`. Checkout correctly detached at
its peeled commit `41f6a645068483470267271e1d09966ca3b9f413`, but the verifier
compared `HEAD^{commit}` with the tag-object SHA. Both objects were immutable;
the comparison mixed object types.

Checkout and exact verification now pin the peeled commit directly. The CI
policy test binds the same SHA to both fields, and actionlint plus the focused
policy suite pass. mpv job `91163394486` in workflow run `30632931277`
completed successfully with the peeled identity; its later workflow result was
unrelated coverage finalization.

## TC-HARNESS-027: POSIX TLS publisher tests assumed an executable checkout bit

Status: **Resolved 2026-07-31; hosted Linux publisher tests are positive**

Severity: **Harness portability (Linux behavior self-tests failed before the
all-feature Rust gate)**

Detection: Linux all-feature job `91111808436` in workflow run `30616813538`

`scripts/copy-swag-sorotte-certs.sh` is intentionally tracked mode `100644`,
and its documented invocation is through `sh`. The nested test launcher used
`exec "$2"`, so all three atomic publisher scenarios exited 126 on Linux.

The launcher now uses `exec sh "$2"` while preserving its fixture PATH,
argument quoting, environment, and exit status. Success, interruption, and
rejection tests pass 3/3; no executable-bit or production-script change was
introduced. Linux all-feature job `91163394469` in workflow run `30632931277`
passed this boundary.

## TC-HARNESS-028: Early-exit fake mpv could close before the client request write

Status: **Resolved 2026-07-31; hosted Windows all-feature boundary is positive**

Severity: **Harness scheduling (a Windows named-pipe regression could fail at
the wrong boundary)**

Detection: Windows server-release job `91111808474` in workflow run
`30616813538`

The early-exit fixture accepted the named-pipe connection and immediately
exited with code 23. A hosted scheduling window allowed the pipe to close
before the production client wrote its first JSON request, while the test was
intended to prove bounded terminal response handling after a valid request.

The child now consumes exactly one newline-terminated valid JSON request
before exit 23. The test still requires the exact exit code, bounded command
completion, unhealthy transport, one command failure, one disconnect, process
reap, and executable release. Its error oracle is role-specific: early exit
accepts only read/EOF outcomes, while the deliberately partial JSON role can
also report invalid JSON. The exact regression passed 50/50 consecutive runs
before and after oracle hardening, and the full all-feature player suite
passes. Windows all-feature job `91163394472` in workflow run `30632931277`
passed this boundary without relaxing the role-specific oracle.

## TC-HARNESS-029: Server latest-publication policy test used stale input syntax and proved only existence

Status: **Resolved 2026-07-31; hosted Windows publication policy is positive**

Severity: **Harness policy accuracy (the Windows all-feature gate rejected the
current guarded workflow and did not exclude a second unguarded latest tag)**

Detection: Windows all-feature job `91111808399` in workflow run
`30616813538`

The workflow uses the valid unified `inputs.push_latest` context, but the
PowerShell regression still expected `github.event.inputs.push_latest`.
Separately, checking for one guarded substring and an unrelated
`default: "false"` would not reject an additional unconditional `latest`
metadata entry.

The PowerShell policy normalizes line endings, requires exactly one raw
`latest` tag entry equal to the guarded dispatch expression, and binds the
complete disabled-by-default string choice declaration. A YAML-aware Python
policy independently asserts the exact input object and exclusive tag
inventory. Both policy paths pass locally.
Windows server-release job `91163394510` in workflow run `30632931277`
passed the corrected publication policy; the workflow's later
coverage-finalization failure is independent.

## TC-HARNESS-030: Non-Windows native preflight made the remaining runner body unreachable

Status: **Resolved 2026-07-31; later hosted Linux all-feature execution is positive**

Severity: **Harness portability (warning-denied Linux compilation stopped
before the workspace behavior gate)**

Detection: Linux all-feature job `91117196202` in workflow run
`30618496116`

The native real-mpv runner returned from an inline non-Windows `cfg` block,
leaving the rest of the function statically unreachable on Linux. The
platform restriction was intentional, but warning-denied Clippy correctly
rejected that layout.

Commit `7395cdf` extracts a typed platform preflight that returns `Ok(())` on
Windows and the exact fail-closed diagnostic elsewhere. A target-sensitive
unit regression keeps both branches reachable to their respective compilers.
The corrected Linux all-feature job in workflow run `30626889218` passed.

## TC-HARNESS-031: Compatibility coverage accounting was suffix-based and lower-bounded

Status: **Resolved 2026-07-31; exact source-bound accounting is positive**

Severity: **Harness completeness (new or ambiguously named compatibility
tests could evade the strict profile oracle)**

Detection: coverage-diff job `91117196164` in workflow run `30618496116`

The required-live report accepted any inventory above a minimum, while the
coverage selector searched for shortened test-name suffixes and carried a
handwritten filtered-out count. That was not a closed proof after the
compatibility inventory grew.

Commit `4fae099` binds the complete discovered inventory exactly, uses fully
qualified libtest identities, derives the filtered count from the same source
tuple, and rejects duplicate, missing, ignored, failed, or extra results.
Focused report-schema and coverage-oracle regressions are positive.

## TC-HARNESS-032: Duplex fault-model command deadline was below hosted scheduling noise

Status: **Resolved 2026-07-31; bounded fault-model regression is positive**

Severity: **Harness timing (the model could fail because its test thread was
descheduled, not because the transport violated the command contract)**

Detection: all-feature nextest execution in workflow run `30620966526`

The in-memory duplex history model used a 30-millisecond wall-clock command
deadline. Under hosted parallel load, an otherwise immediate scripted history
could lose that entire budget before the assertion observed its terminal
state. Commit `8dbc444` raises only this test-model deadline to one second;
production deadlines and the exact transport outcomes are unchanged.

## TC-HARNESS-033: Legacy step collection could finish before its first delayed frame

Status: **Resolved 2026-07-31; delayed-first-frame regression and complete
compatibility matrix are positive**

Severity: **Harness observation (valid asynchronous legacy output could be
recorded as absent)**

Detection: hosted compatibility/coverage execution in workflow run
`30620966526`

The collector applied its short post-activity idle window before observing any
activity. A first legacy frame delayed beyond that window therefore produced
an empty step even though it remained within the step's total bound. Commits
`d844d2e` and `ad410fc` separate the wait-for-first-frame phase from the
post-frame quiescence phase and retain the total deadline. A loopback
regression delays the first framed line beyond the idle interval and requires
its exact recovery.

## TC-HARNESS-034: Concurrent legacy checkout bootstrap was not process-safe

Status: **Resolved 2026-07-31; process-lock regression and hosted Linux lane
are positive**

Severity: **Harness setup integrity (parallel test processes could mutate the
same pinned-oracle checkout)**

Detection: all-feature execution in workflow run `30620966526`

The bootstrap was protected only by a process-local mutex. Separate nextest
processes could simultaneously create or replace the shared repository-local
legacy checkout. Commits `5d5e77a` and `ad410fc` add a bounded cross-process
file lock around readiness/bootstrap, retain the in-process guard, and make
the Linux workflow check out the immutable oracle before tests. An isolated
two-process regression proves non-overlapping ownership and release.

## TC-HARNESS-035: Pinned legacy permanent-room snapshots can alternate delayed playlist setters

Status: **Resolved 2026-07-31; context-exact canonicalization regression and
full live matrix are positive**

Severity: **Compatibility oracle determinism (one delayed legacy watcher
could attribute equivalent permanent-room playlist setters to Alice or Bob)**

Detection: repeated default-parallel required-live compatibility execution

In the exact permanent-room rejoin fixture, the pinned legacy server's delayed
watcher can emit the same playlist payload with only
`playlistChange.user`/`playlistIndex.user` alternating between Alice and Bob.
Rust remains strictly Alice. Commit `0e7a9bc` canonicalizes only the decoded
Bob Hello at scenario step 8, recipient `client-3`, in the permanent room,
and only those two setter fields. Wrong scenario, step, sender, recipient,
room, payload, index, non-Hello, and already-canonical cases remain unchanged.
This is not a general parity normalization.

## TC-HARNESS-036: LLVM exa-scale coverage counts were rejected as malformed

Status: **Resolved 2026-07-31; exact LLVM token grammar regressions are
positive**

Severity: **Harness parser completeness (valid high-count native coverage
could stop the required coverage lane)**

Detection: coverage-diff job `91137572870` in workflow run `30624838791`

The source-view parser recognized suffixes only through peta-scale, while the
pinned LLVM producer emitted the valid exa-scale token `18.4E`. Commit
`cea5fb7` implements the producer grammar through `E`, keeps annotation
handling symmetric, and rejects unsupported case, precision, suffix, and
unabbreviated forms. The focused coverage-profile module is positive.

## TC-HARNESS-037: Linux all-feature CI omitted pinned legacy Python prerequisites

Status: **Resolved 2026-07-31; later hosted Linux all-feature execution is positive**

Severity: **Harness provisioning (required live compatibility tests could
compile but fail before importing the pinned oracle stack)**

Detection: Linux all-feature job `91137572905` in workflow run
`30624838791`

The Linux job installed only CI-policy dependencies. Tests selected by the
all-feature workspace also require the pinned Twisted, pyOpenSSL, and
service-identity stack. Commit `404039b` installs both locked requirement
files after Python setup and before nextest; policy tests bind that exact
order and command. The corrected Linux all-feature job in workflow run
`30626889218` passed.

## TC-HARNESS-038: Released ephemeral ports could collide across parallel legacy servers

Status: **Resolved 2026-07-31; repeated default-parallel live matrices are positive**

Severity: **Harness concurrency (two local test processes could select the
same released startup port)**

Detection: full default-parallel required-live compatibility execution on
Windows (`WSAEADDRINUSE` / 10048)

The harness asked the OS for an ephemeral port, closed that listener, and
later launched the legacy server on the numeric port. Another parallel test
could obtain the same address in between. Commit `6ccfd3a` introduces a
listener-backed lease, an in-process mutex, and a bounded cross-process lock;
the listener is released only immediately before spawn and the guards remain
held through readiness. All six legacy server launch paths use it. The
regression proves listener retention, same-process exclusion,
cross-process exclusion, bounded completion, and post-release reuse.

## TC-HARNESS-039: Native Windows ASan fuzz invocation lacked a compatible runtime

Status: **Diagnostic only 2026-07-31; canonical Ubuntu WSL campaign is positive**

Severity: **Operator-environment limitation (no Sorotte source or target
failure)**

Detection: direct native-Windows invocation of the documented framing fuzz
runner at `6ccfd3a`

The noncanonical native invocation built the target but exited with
`STATUS_DLL_NOT_FOUND`; adding an older Visual Studio ASan directory changed
the result to `DLL_INIT_FAILED`, confirming that the pinned LLVM 22 target and
old runtime were incompatible. The failed bundle is preserved at
`target/fuzz-ci/mpv-framed-transcript-deep-6ccfd3a-v1`. No source, seed, or
artifact mutation occurred. The documented Ubuntu WSL campaign passed at the
same checkpoint and again at final implementation SHA `9f3cb60`.

## TC-HARNESS-040: Port-lease regression changed the strict compatibility count

Status: **Resolved 2026-07-31; exact 21-test selector regression is positive**

Severity: **Harness inventory maintenance (a new matching test correctly
failed the closed coverage oracle until explicitly reviewed)**

Detection: coverage-diff job in workflow run `30626889218`

The new `legacy_server_port_lease_serializes_startup_allocation` regression
matches the strict `legacy_server_` coverage selector. Cargo therefore ran 21
tests with 128 filtered out, while the source-bound oracle still required
20/129. All 21 tests passed; only the exact inventory assertion failed.
Commit `9f3cb60` adds the reviewed identity to the canonical tuple. The focused
policy module passes 27/27 and the strict Cargo selector passes 21/21.

## TC-HARNESS-041: Linux-only changed-line coverage mixed QA structure and Windows production bodies

Status: **Resolved 2026-07-31; exact local replay and hosted two-platform
producer/policy boundary are positive; whole-workflow failure is TC-HARNESS-044**

Severity: **Harness scope and platform completeness (the required gate
reported 47.39% with 1,883 unmapped lines after every originating behavior job
except coverage had passed)**

Detection: coverage-diff job `91147825269` in workflow run `30627601938`

The Linux map was asked to represent Windows-only updater, named-pipe,
process, and GUI bodies. The consumer also counted dedicated smoke, benchmark,
semantic, fuzz, complete test-support cfg items, and compiler-structural Rust
lines as production-executable scope. That combination inflated the
denominator and converted absent platform mappings into failures.

Commit `829ab98` keeps both ratchets unchanged and joins independently
source-validated Linux and exact-head Windows physical-line maps. It excludes
only exact repository QA paths, complete attached test/test-support/fuzz-support
items, and conservatively recognized compile-time or structural lines.
Duplicate map content, stale source bytes, ambiguous test-support items, and
executable-looking unmapped lines remain failures. The exact local replay
passes at 80.13% ordinary and 90.79% critical, 82.52% combined, with zero
unmapped lines.

Hosted workflow run `30632931277` subsequently regenerated both exact-source
maps and passed the unchanged policy at 1,724/2,150 ordinary (80.18%),
562/619 critical (90.79%), and 2,286/2,769 combined (82.55%), with zero
unmapped lines. Its later coverage evidence-finalization failure, followed by
the downstream aggregate failure, is the separate ordered-map binding defect
recorded as `TC-HARNESS-044`; it does not invalidate these
producer or policy results and is not relabelled as a passing workflow.

## TC-HARNESS-042: Windows process coverage inventory omitted four reviewed tests

Status: **Resolved 2026-07-31; exact 54-test producer is positive**

Severity: **Harness inventory maintenance (the source-bound Windows producer
correctly rejected new matching tests until their ownership was reviewed)**

Detection: exact local Windows process coverage generation while correcting
`TC-HARNESS-041`

The strict inventory still described 50 tests. Current source contains three
additional updater storage/directory-sync regressions and one nonce-owned
media-tool fixture identity regression. Commit `829ab98` adds those exact
identities and updates only the corresponding filtered-out counts. The clean
exact-head producer passes 54/54: 33 updater transaction, two installed
updater, eight named-pipe, three external-mpv, and eight media-tool tests.
Extra, missing, ignored, failed, or partially selected results remain fatal.

## TC-HARNESS-043: Windows CRLF source hashes could not be consumed by Linux coverage

Status: **Resolved 2026-07-31; fresh-checkout and hosted cross-platform source
binding are positive; whole-workflow failure is TC-HARNESS-044**

Severity: **Harness provenance portability (a valid Windows map could be
rejected as stale solely because Git materialized different line endings)**

Detection: pre-hosted exact-map union review at `829ab98`

Canonical maps hash represented source bytes exactly as stored. Windows Git
with global `core.autocrlf=true` materialized some Rust files as CRLF, while
the Linux consumer materialized LF. Commit `829ab98` adds the repository rule
`*.rs text eol=lf` and a policy regression binding it. A fresh Windows clone
retained global autocrlf but reported LF for the sampled Rust source and
matched the Linux artifact's digest. This preserves raw-byte source binding;
the map schema was not weakened or normalized after production.

## TC-HARNESS-044: coverage evidence finalizer rejected a valid two-platform union

Status: **Resolved 2026-07-31; exact downloaded-artifact replay and
implementation-head hosted finalization are positive**

Severity: **Harness provenance integration (all producers and policy thresholds
passed, but the required aggregate remained red)**

Detection: coverage job `91169713196`, followed by downstream aggregate job
`91171848135`, in workflow run `30632931277`

The coverage report correctly bound an ordered Linux/Windows physical-line map
union, while the coverage evidence finalizer still accepted only a single
retained map and reported that the diff report was not bound to the canonical
artifact.
The originating jobs, exact 54-test Windows producer, both map conversions,
and unchanged 80%/90% policy had already passed. The hosted union recorded
2,286/2,769 combined (82.55%), 1,724/2,150 ordinary (80.18%), 562/619
critical (90.79%), and zero unmapped lines.

Commit `2b8af5672cd27c727f3707b71ccd15a1292135c7` makes supplemental
maps repeatable finalizer inputs and binds the retained report to the complete
ordered primary-plus-supplemental tuple. Omission, reordering, duplication,
replacement, source drift, and content tampering fail closed. Six focused
regressions cover that contract. The exact downloaded failed artifacts replay
successfully under `target/hosted/30632931277/replay-root`; the corrected
phase artifact and union report retain SHA-256 identities
`b889d98a1e947b607a69c126d6b51ac46cb9d88e4bcbb40a734257d4c3c512b3`
and `4c4a3bc2e222230ac06bee1a8119317f51190553eaf56b313e17cbee47df565e`.
Exact implementation-head workflow `30639113884` then passed coverage job
`91190243453` and aggregate job `91192554763`. Its regenerated ordered union
covered 2,403/2,894 lines (83.03%), including 1,841/2,275 ordinary (80.92%)
and 562/619 critical (90.79%), with zero unmapped lines. The accepted report,
phase manifest, and aggregate retain SHA-256 identities
`c6187f3b8a9c4237c22be74c2884afc08de09d9a354ba563f8b496460a36500c`,
`df3efff1780babbb9cb371a8d1d07c41a4efbdcf4c5c50444b3333aeafa7f8c5`,
and `6ebc5ef4793609c515c3824484d2b7389fbbaeb182271ff047711823e88e5244`.

## TC-HARNESS-045: real-mpv HTTP stall could begin during startup

Status: **Resolved 2026-08-01; deterministic model, complete sim suite,
exact-head hosted minimum-mpv execution, and implementation-head aggregate are
positive**

Severity: **Harness phase ordering (startup seeking could be mistaken for the
post-start cache-stall recovery episode)**

Detection: mpv job `91174919979` in workflow run `30636380151`

The byte-triggered HTTP fixture could stall while the two real mpv clients were
still establishing their healthy baselines. At failure, the healthy client was
already `Playing` but had `seeking=true` and one command timeout; the stalling
client had no command timeout. The test timed out before establishing the
required timeout-free, seeking-clear started baseline, so product isolation
and recovery-budget assertions never ran. A preceding run with identical relevant
sim/client/player source passed; the intervening commit changed only workflow
and coverage-policy code. The retained failure therefore diagnoses harness
ordering rather than a player behavior regression.

Commit `bc5ef9dbcff08d194c449e051c8da46424324b8c` introduces an explicit
deferred stall arm and globally one-shot `(path, sorted stall index)` claim
across range/retry connections. Both clients must first reach exact
`ReadyPaused` at revision 1, then exact timeout-free `Playing` at revision 2
with seeking clear and no applied stall. Only then is the fault armed. The
real-mpv contract still requires exactly one completed stall, at most one seek
per observed recovery episode for the affected client, and zero post-start
seeks for the healthy peer. A separate deterministic concurrent-request
regression proves both handlers announce and park before arming, exactly one
claims the fault, and both full response bodies complete. The complete sim
suite passes 12/12 and warning-denied sim Clippy is green.
Exact-head workflow run `30639113884`, mpv job `91184230570`, subsequently
passed the corrected minimum-supported-mpv lane.

## TC-HARNESS-046: Plex fixture classified an incomplete request header

Status: **Resolved 2026-08-01; scripted transient-read regression,
production-path loopback oracle, exact-head Windows nextest, and
implementation-head aggregate are positive**

Severity: **Harness socket determinism (a required Windows test failed once
and passed only on retry; fail-on-flaky correctly kept the lane red)**

Detection: Windows job `91174920040` in workflow run `30636380151`

The fail-on-flaky nextest run executed 3,775 tests: 3,774 passed and
`sorotte-cli::tests::plex_watch_sync::connected_session_reports_plex_timeline_from_player_telemetry`
failed on TRY 1 in 2.102 seconds before passing on TRY 2 in 2.123 seconds. The
policy correctly returned status 100 rather than accepting the retry. Artifact
`8795822978` (`nextest-attempts-windows-1`) retains that candidate-head result
and the original first-request assertion, but not the received request bytes
or socket error kind. Source review supplied the diagnosis: the loopback Plex
listener was nonblocking, and after `accept` its read loop treated every error,
including transient `Interrupted`, `WouldBlock`, or `TimedOut`, as completion.
An empty or partial header could therefore be enqueued and classified as the
first request. This is a distinct recurrence in the test named by
`TC-HARNESS-004`, not a failure of that finding's panic-safe environment
ownership.

Commit `dd3012c1bcefa0a68520b063c5ae06f3e1b96f79` resets the accepted
socket to bounded blocking reads and accumulates up to a complete `CRLFCRLF`
terminator under one overall deadline. Incomplete EOF, fatal errors, and
headers that do not complete within the bounded loop are discarded without
incrementing the fixture request count. A deterministic scripted regression
places `Interrupted`, `WouldBlock`, and `TimedOut` around a split request and
requires all five reads plus the exact completed suffix. The existing
real-socket connected-session test retains its production-path sections ->
file lookup -> timeline order oracle and now reports captured request text on
failure. Focused Plex-watch-sync tests pass 3/3, the complete CLI package
passes, and warning-denied all-target CLI Clippy is green. Exact-head workflow
run `30639113884`, Windows job `91184230464`, subsequently passed 3,777/3,777
nextest cases with 19 skipped, exit zero, and no failure, flaky, or rerun
elements in artifact `8796957980` (`nextest-attempts-windows-1`).
