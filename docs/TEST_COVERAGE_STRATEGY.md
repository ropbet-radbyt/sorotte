# Sorotte Test Coverage and Verification Strategy

Status: proposal with implementation tranche and lean-fix follow-through

Audit date: 2026-07-28
Lean-fix implementation update: 2026-07-29
Merged-profile implementation update: 2026-07-29
GUI release artifact implementation update: 2026-07-30
Outstanding-defect closure update: 2026-07-30
Deep-boundary testing update: 2026-07-30

Historical audit baseline: pull request #15, `codex/fix-youtube-buffering-stall` at
`a08a06ea7c6cada5413b0dba73b16f940cfd46e1`

Current implementation base after rebase: `main` at
`f3964ebc7f7b281b9b78f3bfb243ff65e5122e33` (merged pull requests #15 and
#16)

Target audience: maintainers, reviewers, and release owners

## Implementation status on this branch

This branch implements the highest-leverage part of the proposal, then applies
the production fixes proven by that coverage. The final closure slice resolves
all six remaining registered defects, converts all eight expected failures
into positive regressions, and validates an explicitly empty defect registry
at that checkpoint. A later deep-boundary slice deterministically exposes and
registers one new TLS publication-atomicity defect, `TC-SERVER-004`:

- a fail-closed behavior catalog with 17 behavior IDs and 40 exact proofs;
- two Linux evidence lanes covering exact lifecycle libtests and the complete
  14-scenario GUI semantic inventory;
- Git SHA, repository, workflow-run, attempt, catalog, OS, selector, command,
  inventory, and required-job binding, including partial-rerun support;
- locked all-feature Linux and Windows tests, a Windows release-profile build,
  strict live compatibility and real-mpv jobs, and one aggregate required gate;
- parser-level adversarial tests and static workflow-policy tests;
- shrinkable reducer-input histories with independent epoch, per-epoch order,
  and at-most-once oracles, plus exhaustive stale-epoch metamorphic coverage;
- 2,048-case nightly depth for that property suite;
- separate required production changed-line gates at 80% for ordinary code and
  exactly 90% across 20 non-overlapping critical lifecycle, protocol,
  authorization, persistence, updater, and privacy paths, with a strict
  source-bound LLVM physical-line map, Git-diff binding, immutable base/head
  policy union, test-only path and inline `#[cfg(test)]` module exclusion, and
  unmapped-line failure; diagnostic LCOV uses unique `DA` source lines while
  preserving contradictory `LF`/`LH` summaries as typed audit evidence;
- event-aware base resolution: PR merge-base, exact branch/updated-tag
  `before`, new-tag default-branch merge-base only for all-zero `before`, and
  an explicit manual base, with raw/effective provenance and always-run phase
  JSON;
- fail-closed merged coverage profiles for the all-feature workspace, complete
  14-scenario GUI semantic suite, and the complete 20-test strict
  live-reference compatibility inventory, with pinned producer/reference
  identity, fresh raw-profile deltas, lane-specific behavioral oracles,
  retained logs, and a final LLVM merge check;
- immutable commit pins for every third-party action used by the Rust CI and
  coverage workflows, plus an exact verified mpv source commit;
- a fail-closed registry for all 23 ignored Rust tests, including explicit CI,
  manual-capability, and fixture-maintenance dispositions; the two former
  compatibility quarantines are now required passing tests;
- a schema-validated expected-failure registry; previously resolved product
  defects and the subsequently surfaced `TC-PLAYER-003` and `TC-COMPAT-001`
  through `TC-COMPAT-007` all have positive regressions; the final
  `TC-CLIENT-001`, `TC-SERVER-003`, `TC-PROTOCOL-001`, `TC-CLI-001`/`002`,
  and `TC-UPDATER-001` characterizations have also been converted; the current
  registry contains only the new `TC-SERVER-004` cross-generation TLS snapshot
  characterization;
- pinned nextest execution with one diagnostic retry, fail-on-flaky and
  500 ms fail-on-subprocess-leak semantics, JUnit attempt retention, zero-test
  rejection, and always-uploaded evidence;
- deterministic protocol-order, production-worker IPC fragmentation,
  coalescing, reordering, duplication, truncation and half-close,
  persistence-corruption, concurrent-secret, and migration-failpoint tests;
- generated credential-taint coverage across transcript serialization,
  diagnostic dumps, parser errors, and player error display;
- a strict ten-scenario native GUI contract with typed AccessKit menu
  identities, structured capability outcomes, fresh-binary provenance,
  preserved success and failure artifacts, forbidden skips, stderr
  enforcement, a bounded process-tree watchdog, acknowledged physical menu
  input, two-sided live-Python readiness, fail-closed loopback ownership, and
  observable bounded File -> Exit shutdown;
- positive regressions and lean production fixes for terminal reactivation,
  SQLite migration atomicity, concurrent durable-secret initialization, and
  all three credential-redaction families;
- deterministic harness fixes for cross-shell timestamp comparison,
  backward-compatible Python peer response correlation, panic-safe environment
  restoration, causal Plex synchronization, detached native network isolation,
  and updater stdio ownership;
- the first deterministic CLI time slice: exact paused-clock reconnect
  backoff/exhaustion, barrier-driven independent STARTTLS response and TLS
  handshake deadlines, an exact post-client-Hello server-Hello deadline, and a
  virtual-time real-loopback retry that forbids Hello or credentials before
  required TLS resolves;
- deterministic server TLS rotation: an injected metadata revision clock, 243
  exhaustive five-step reference-model histories, and real-network
  in-flight-context and recovery proofs with no filesystem timestamp waiting;
- deterministic process-interruption persistence: 15 child-process crash
  points across schema, row migration, room save/delete, stats snapshots, and
  quota-secret creation, each followed by integrity-checked and idempotent
  production reopen;
- immutable server and GUI archive consumers that bind source identity,
  checksums, closed inventories, manifests, and exact extracted bytes before
  upload; the GUI consumer additionally proves an isolated visible window,
  installed-updater self-replacement, and rollback after a real filesystem
  replacement failure, then reconsumes downloaded bytes before publication.

Experimentation surfaced seven reproducible product defect classes: two
lifecycle invariant failures, two persistence initialization/migration
atomicity failures, and three credential-redaction failures. They are
documented in
[TEST_COVERAGE_FINDINGS.md](TEST_COVERAGE_FINDINGS.md). All seven now have
positive regressions. The predecessor/successor lifecycle decision uses an
exclusive-successor graph rule and the randomized reducer property runs
without a quarantine or unchecked seam. The deterministic scheduling seams
remain compiled under `cfg(test)` and expose the real transition or
concurrency boundary. Native GUI
execution also proved that the old runner accepted missing native menus and a
skipped Open Media contract while emitting repeated outbound DNS failures.
The completed fix now proves exact UIA/AccessKit identities, detached
disablement, attached stable-ID invocation, and exact player receipt; the
detached baseline no longer performs startup network I/O. Mutation,
remaining deterministic clocks/network scheduling outside the CLI and TLS
rotation boundaries, power-loss and filesystem-syscall persistence faults,
coverage-guided fuzzing, sanitizers, interactive native CI, server-container
consumption, and public digest comparison remain proposed follow-on work.
The later compatibility-remediation experiment isolated four server parity
defects, one server message-ordering defect, and four harness/oracle defects.
All are resolved without a skip, retry, expected failure, or parity
normalization. The required live-reference selector passes 20/20 and the
deterministic Python fanout inventory passes 33/33.
Final validation also exposed a PowerShell type-coercion defect in the
package-path test harness; it now compares timestamp instants and passes under
both supported shells. The exact-final locked all-feature Windows run completed
in 235.1 seconds and emitted a 15,369,296-byte LCOV artifact. Its declared
aggregate is 148,045 / 190,067 lines, while the explicit `DA` inventory is
144,853 / 183,712; 310 of 395 source records contain an `LF` or `LH`
contradiction. The diagnostic consumer now preserves both models and evaluates
only unique `DA` line records, while missing executable mappings remain hard
failures. Ordinary and instrumented workspace runs exposed the CLI shared-lock
poison cascade, and pinned nextest exposed an updater subprocess-handle leak.
Both have deterministic regressions and lean harness fixes.

The merged-profile experiment then proved a clean, compatible 36-profile
workspace, semantic, and strict live-TLS view at 77.98% diagnostic
line-instance coverage. It also found a rare player event-observation failure
during one parallel instrumented workspace run and reproduced six failures in
the complete strict legacy fanout matrix. The follow-up investigation resolved
all seven, found and fixed one additional message-ordering defect after
strengthening the comparator, retired both timeout quarantines, and expanded
the required green compatibility claim to the exact 20-test live-reference
inventory. The resulting trace recapture also removed two stale client-core
assumptions about nullable readiness and incidental periodic State traffic.
The coverage gate still has no retry.

The next reconnect slice adds a 128-case, 64-step shrinkable client-core
reference model with seven state-aware event kinds: retry, Hello, empty and
non-empty initial server playlists, and transition/state/playlist drains.
Each executed step is compared with an independent semantic model, every
history is driven to an active terminal observation, and two final drain passes
prove one-shot behavior. `PROPTEST_CASES=2048` reuses the nightly depth budget.
The model-design experiment also surfaced `TC-CLIENT-001`: playlist restore
state was consumed before acknowledgement and was not cancelled after a newer
authoritative update. The resolution adds an explicit awaiting-acknowledgement
state: send preserves desired state, disconnect re-arms it, and a non-empty
authoritative update retires or supersedes it. The independent model compares
snapshot, armed, and pending-ack state after every transition; both minimized
schedules are now positive regressions.

The subsequent clock experiment converts the CLI reconnect scheduler and
STARTTLS phase timeout contract to paused Tokio time. It proves the exact
100/200/400 ms backoff sequence, proves exhaustion consumes no additional
time, and uses explicit request/ClientHello barriers before manually advancing
the response and handshake deadlines by 25 ms. The initial server Hello phase
has the same exact proof after the server observes the client Hello. A real
loopback retry remains in the proof stack for protocol reachability and
credential ordering, but its operating-system delivery latency is deliberately
not asserted as virtual time: the initial naive experiment measured 1.025
seconds for a 25 ms handshake because Tokio correctly advanced an otherwise
idle runtime while Windows was delivering socket I/O. Separating protocol
barriers from the clock oracle removed that scheduler-luck dependency without
changing production behavior. After the barrier repair, each of the four exact
proofs passed 50 consecutive executions (200/200 total), and the all-feature
CLI suite passed 335 tests with its eight declared ignores unchanged. No
additional product defect was exposed by this slice.

The TLS rotation clock slice replaces two retrying file-mtime helpers, each of
which could sleep for two seconds, with an explicit test-only bundle metadata
revision. The production reload state machine is exercised through 243
exhaustive five-step histories of cached invalid contents, invalid revision,
and valid revision; every context, acceptability, retry, response, and
transport action is compared with an independent model. Real-network tests use
the same revision source to prove an accepted handshake keeps its captured
context and a later valid bundle recovers before the retry cap. The first model
run completed all 1,215 transitions in 5.85 seconds without a timestamp poll.
The extraction experiment surfaced `TC-SERVER-003`: taking only the maximum of
three member mtimes loses member identity and can miss a real rotation.
Production now hashes filename- and length-framed contents for all three
members and parses the exact captured snapshot used for that fingerprint.
Equal-length replacement of each member and the real-filesystem below-maximum
timestamp collision are positive regressions. Earlier stress validation passed
the model 10/10 times (2,430 histories, 12,150 transitions), both real-network
proofs and the retry-cap proof 50/50 times each, and the filesystem schedule
25/25 times.

The persistence process-interruption slice makes the old-or-new durability
decision executable at the SQLite boundary. A test-only child role exits with
code 86 from 15 production stages without running Rust destructors. Five schema
stages prove every committed legacy-schema prefix can be reopened and completed
idempotently. Two playlist-migration stages prove the multi-row transaction is
entirely legacy before commit or entirely canonical after commit. Four actor
stages cover save and delete immediately after the SQL write and immediately
after commit. Two stats stages distinguish zero rows from a complete
three-version snapshot, and two quota-secret stages distinguish no metadata row
from one stable 32-byte value. The parent requires SQLite integrity before
normal recovery and checks the second reopen for idempotence. The five
contracts passed 20 consecutive actor-suite runs: 300/300 crash subprocesses
and 240/240 complete actor tests. The full server persistence selector passes
49/49. The final locked all-feature workspace passed in 200.7 seconds,
including 338/338 server library tests, and full-workspace warning-denied
Clippy passed in 6.96 seconds. This is process-termination evidence, not a
claim about power loss, kernel cache durability, disk-full behavior, or an
actor message not yet written to a transaction.

The accompanying policy audit found that the TLS defect had reused the already
assigned `TC-SERVER-001` identifier and that multiline Rust
`should_panic(expected = ...)` attributes escaped the executable inventory.
The TLS finding is now `TC-SERVER-003`; the known-defect validator parses
multiline attributes and rejects duplicate finding headings and title drift.
Its 21 focused tests pass against both populated fixtures and the real
closure checkpoint's explicitly empty registry; the historical
two-defect/four-characterization checkpoint therefore remains policy evidence
rather than current inventory. The same policy validates the current
one-defect/one-characterization `TC-SERVER-004` inventory.

The GUI release artifact slice turns the Windows ZIP into an independently
consumed contract rather than trusting the packaging step. Thirty-two
synthetic adversarial cases close archive selection, checksum and upload
inventory, path traversal/collision/link shape, duplicate JSON keys, both
manifest schemas, source/channel/version/timestamp agreement, every payload
digest, optional symbols, immutable action pins, and build-to-upload and
download-to-publication ordering. The real-byte consumer then launches the
extracted GUI in an isolated profile and requires a visible native window,
runs the extracted installed updater through self-replacement with the exact
ZIP, and forces a read-only later target after an earlier replacement so the
original install and all transaction artifacts must be restored. The
publication job independently reconsumes the downloaded bytes without
executing them again.

Post-authentication mutation of a prepared temporary exposed
`TC-UPDATER-001`: rejection was correct, but rollback authenticated the
disposable corrupt temporary before removing it, retained the recovery
journal, and blocked subsequent automatic recovery. Rollback now preserves
strict target/backup and link checks while deleting uncommitted regular-file
scratch regardless of its digest. Positive one- and two-file regressions prove
that both an unchanged install and an earlier replaced target recover without
transaction artifacts.

## Contents

- [Implementation status on this branch](#implementation-status-on-this-branch)
- [Executive decision](#executive-decision)
- [1. Scope and interpretation](#1-scope-and-interpretation)
- [2. Audit method and reproducible experiments](#2-audit-method-and-reproducible-experiments)
- [3. What the lifecycle report proves](#3-what-the-lifecycle-report-proves)
- [4. What escaped after the report](#4-what-escaped-after-the-report)
- [5. Whole-application coverage map](#5-whole-application-coverage-map)
- [6. Current quantitative coverage](#6-current-quantitative-coverage)
- [7. Priority findings](#7-priority-findings)
- [8. Target assurance architecture](#8-target-assurance-architecture)
- [9. New testing strategies](#9-new-testing-strategies)
- [10. Coverage policy](#10-coverage-policy)
- [11. Flake and failure policy](#11-flake-and-failure-policy)
- [12. CI design and budgets](#12-ci-design-and-budgets)
- [13. Concrete implementation backlog](#13-concrete-implementation-backlog)
- [14. Metrics that matter](#14-metrics-that-matter)
- [15. Anti-goals and guardrails](#15-anti-goals-and-guardrails)
- [16. Recommended report corrections](#16-recommended-report-corrections)
- [17. Final position](#17-final-position)

## Executive decision

Sorotte has a large, unusually thoughtful deterministic test suite. Its
player-lifecycle reducer, acknowledgement protocol, replay behavior, and
in-process client/GUI projections are substantially better tested than a raw
test count or the repository's 79% line-coverage headline suggests.

The principal problem is no longer a shortage of example tests. It is that:

1. the proof described by the lifecycle report is not mechanically required by
   pull-request CI;
2. several tests described as full-stack stop before IPC framing, worker
   scheduling, process, native GUI, and operating-system boundaries;
3. generated histories are finite custom simulations without a small
   independent reference model, shrinking, or persistent failure cases;
4. important test harnesses can skip required behavior and still report
   success;
5. at audit time, coverage was generated only on a scheduled default-feature
   Linux run and was not compared with the pull-request diff; this branch now
   gates changed lines and merges all-feature workspace, semantic, and strict
   live-TLS execution, while native Windows and the red full-compatibility
   matrix remain separate;
6. Windows-specific, process, timing, persistence-crash, packaging, and
   publication paths remain materially under-protected.

The recommended direction is not “add tests until the global percentage is
higher.” It is to build an executable assurance system:

- give every critical behavior a stable identifier and machine-readable proof
  requirements;
- make the existing all-feature lifecycle and semantic evidence mandatory;
- make skips, prerequisites, and unavailable capabilities explicit and strict;
- introduce shrinkable state-machine tests with an independent oracle;
- move selected fault injection below decoded JSON and across real process
  boundaries;
- gate changed code and critical modules with coverage and mutation evidence;
- reserve wide schedule exploration, fuzzing, chaos, and long soaks for
  nightly and weekly tiers;
- verify the actual package or container that will be published.

### Historical merge recommendation for pull request #15

At audit time, treat the pull request as conditionally mergeable, not as whole-application
verification-complete. Do not block it on the full 90-day program in this
document. Before merge, either complete the following minimum tranche or record
each omitted item as an explicit, owned risk:

1. run locked, all-feature workspace Clippy and tests on Linux;
2. run the all-feature core/workspace behavior suite on Windows, not only a
   release build;
3. require the focused lifecycle, GUI projection-chain, ordered-delivery, and
   14-scenario semantic suites;
4. make the complete live compatibility job fail when its oracle or
   prerequisites are unavailable;
5. encode the two admitted lifecycle follow-ups: the queued same-target
   predecessor-failure case and a poison adapter for every legacy getter;
6. correct the report's proof boundary and regenerate its evidence against the
   exact merge SHA;
7. either make native-smoke required contracts strict or stop counting its
   current permissive result as merge evidence.

The advanced property, fuzz, concurrency, mutation, sanitizer, and soak work
should follow immediately, but it should not be rushed into this already large
pull request.

## 1. Scope and interpretation

The request was interpreted as both:

- a review of the claims and evidence in
  [player-lifecycle-stabilization.md](player-lifecycle-stabilization.md),
  [player-lifecycle-verification.md](player-lifecycle-verification.md), and
  [player-lifecycle-followups.md](player-lifecycle-followups.md); and
- a whole-application test strategy for the branch expected to merge next.

At the time of inspection, the audit used the only open pull request, #15,
whose head was mergeable and clean. It contains 231 changed files,
59,702 additions, and 7,113 deletions relative to `main`. The lifecycle report
landed earlier in this history; 17 subsequent fix commits added 81 regression
test attributes. That makes the post-report history a useful escaped-defect
corpus rather than a reason to dismiss the report.

This review is timeboxed evidence, not a claim that every test assertion or
every production branch was manually inspected. The review covered:

- the lifecycle reports and their named proof surfaces;
- all workspace test topology and ignored tests;
- CI, coverage, release, package, and container workflows;
- current and historical GitHub Actions outcomes;
- line/function coverage from the latest downloadable artifact;
- focused all-feature lifecycle, GUI, semantic, native, core, and compatibility
  runs;
- static timing, concurrency, platform, and parser risk surfaces;
- implementation boundaries of the major verification harnesses.

## 2. Audit method and reproducible experiments

All local experiments ran from an isolated worktree at the exact pull-request
head. Each parallel audit used a separate target directory. No production
source was changed during experimentation.

### 2.1 Experiment log

| Evidence | Command or source | Result | Interpretation |
|---|---|---:|---|
| Pull-request state | `gh pr view 15` | Open, mergeable, clean; 231 files | Exact audit baseline, not the older report SHA |
| Audit-baseline static Rust tests | all plain and parameterized `#[test]` / `#[tokio::test(...)]` attributes | 3,468 attributes; 25 ignored | Historical baseline before this tranche; count is not behavioral coverage |
| Post-report regressions | zero-context diff from `9478112..a08a06e` | 81 added test attributes | Real escaped-defect corpus after the report's closure point |
| Player lifecycle | `cargo test -p sorotte-player-mpv --all-features lifecycle -- --nocapture` | 49 passed, 1 ignored; 38.4s cold, 8.1s test body | Cheap and suitable for a required PR gate |
| GUI lifecycle projection chain | `cargo test -p sorotte-gui --all-features lifecycle_verification_tests -- --test-threads=1` | 13 passed; 88.8s cold, 36.95s body | Strong in-process consumer/projection evidence |
| Ordered delivery | `cargo test -p sorotte-gui --all-features ordered_delivery_tests -- --nocapture --test-threads=1` | 14 passed; 4.3s warm | Cheap deterministic delivery proof |
| GUI semantic suite | `scripts/gui-semantic-suite.ps1 -Json` | 14/14 passed; 38.6s | Valuable user-workflow model, currently absent from CI |
| Native GUI smoke (legacy pre-hardening runner) | `scripts/gui-native-smoke.ps1 -Json -TimeoutMs 80000` | Exit success; 82.5s | False-positive risk: required-looking contracts were skipped; the strict wrapper now rejects this evidence |
| Six core crates, all features | locked all-feature tests for protocol, server, client-core, client-app, CLI, compat | 1,725 passed, 17 ignored; 3m53s | Practical on PR; one server release test consumed 99.79s |
| Compatibility details | `cargo test --locked --all-features -p sorotte-compat -- --nocapture` | 134 passed, 9 ignored, 16 “assertion skipped” messages | Green can mean the claimed live oracle did not run |
| Current PR Actions | runs `30263427047` and `30263426964` | All required checks green | Current workflow result is genuinely green, but scope is incomplete |
| Scheduled coverage | run `30251611866`, artifact `8647315691` | 78.99% lines, 78.68% functions, no branch records | Useful baseline from `main`, not the PR head |
| PR-head local coverage probe | `cargo llvm-cov --workspace --summary-only --locked` | Cold build exceeded the audit's 60s local probe | No invented PR percentage; GitHub's 288s generation is the valid cost estimate |
| Workflow rerun history | last 100 `rust-ci` runs | 9 runs had `run_attempt > 1`; sampled first attempts included concrete test/job failures | A rerun alone is not a flake; preserve the initial category and evidence |

The full local coverage build was deliberately stopped during the original
audit probe; therefore the historical per-crate percentages below are
explicitly from `main` SHA
`6add397946c5a20cb53a7f86def2046813cdebc9`. The implementation branch later
completed a preliminary locked all-feature Windows LCOV experiment at 79.00%
aggregate line coverage and a fresh exact-pinned-toolchain run whose declared
summary was 77.81%. The fresh artifact's explicit `DA` inventory instead
reports 78.76%, so neither percentage is accepted as changed-line policy
evidence. The successful producer run, structural contradiction, and
first-attempt concurrency failure are recorded in
[TEST_COVERAGE_FINDINGS.md](TEST_COVERAGE_FINDINGS.md); they do not
retroactively replace the historical per-crate snapshot.

### 2.2 Current pull-request timing budget

The successful pull-request run took:

| Job | Wall time | Dominant work |
|---|---:|---|
| Linux checks | 466s | workspace tests 319s; Clippy 101s |
| Real mpv | 214s | dependencies 53s; four tests 63/6/30/25s |
| Strict live TLS compatibility | 89s | test 66s |
| Windows release build | 529s | build 491s |
| GUI package/release | 834s | updater tests 321s; builds 475s |
| Scheduled coverage, separate run | 309s | coverage generation 288s |

The existing critical path is about 13.9 minutes because jobs run in parallel.
The focused lifecycle, GUI projection, ordered-delivery, and semantic evidence
fits into a separate two-to-four-minute cold job and should not materially
extend that path. A local Windows all-feature core run completed in under four
minutes. The immediate coverage improvements are therefore affordable.

### 2.3 Reproduction details

The audit used these isolated target roots so parallel runs could not contend:

```powershell
$env:CARGO_TARGET_DIR = 'C:\tmp\sorotte-coverage-gui-target'
cargo test --locked -p sorotte-player-mpv --all-features lifecycle -- --nocapture
cargo test --locked -p sorotte-gui --all-features lifecycle_verification_tests -- --test-threads=1
cargo test --locked -p sorotte-gui --all-features ordered_delivery_tests -- --nocapture --test-threads=1
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 80000

$env:CARGO_TARGET_DIR = 'C:\tmp\sorotte-coverage-core-target'
cargo test --locked --all-features `
  -p sorotte-protocol `
  -p sorotte-server `
  -p sorotte-client-core `
  -p sorotte-client-app `
  -p sorotte-cli `
  -p sorotte-compat
```

Static counts included parameterized Tokio attributes:

```powershell
rg -n '#\[(tokio::)?test(\([^]]*\))?\]' crates --glob '*.rs'
rg -n '#\[ignore' crates --glob '*.rs'
git diff -U0 9478112..a08a06e -- crates
```

GitHub evidence came from pull-request run
[30263427047](https://github.com/ropbet-radbyt/sorotte/actions/runs/30263427047),
GUI run
[30263426964](https://github.com/ropbet-radbyt/sorotte/actions/runs/30263426964),
and scheduled coverage run
[30251611866](https://github.com/ropbet-radbyt/sorotte/actions/runs/30251611866).
The LCOV artifact was downloaded with:

```powershell
gh run download 30251611866 `
  --name sorotte-lcov `
  --dir C:\tmp\sorotte-coverage-main-30251611866
```

LCOV totals were calculated from `LF`/`LH` and `FNF`/`FNH` records after
grouping `SF` paths by crate. Job and step durations came from GitHub Actions
timestamps; local wall times came from the isolated commands above. The audit
does not commit raw transient logs. The proposed evidence-shard system below is
the mechanism that should make future proof fully reproducible and durable.

## 3. What the lifecycle report proves

The report should be preserved as a serious engineering artifact. It correctly
captures important design decisions and has strong deterministic evidence:

- a typed lifecycle reducer with executable invariants;
- attachment, attempt, command, media-generation, and acknowledgement fencing;
- replay, gap detection, authoritative snapshot rebasing, compaction, and
  delayed acknowledgement;
- readable traces for replacement, buffering, EOF, disconnect, and recovery;
- invariants checked after every reducer step;
- an in-process projection chain through adapter output, client-core, and GUI;
- synthetic transcript replay;
- 14 semantic GUI scenarios;
- local real-mpv experiments that found defects ordinary unit tests did not.

The report's candid limitations are also useful. It says transcripts are
synthetic and identifies remaining follow-up debt. Those admissions should
remain.

### 3.1 Correct proof boundary

The strongest cross-layer tests currently prove:

```text
already-decoded mpv JSON
  -> adapter event handler and lifecycle reducer
  -> ordered producer batch
  -> direct client-core batch application
  -> direct GUI projection application
  -> manual acknowledgement and compaction
```

They do not prove:

```text
mpv process
  -> byte framing over named pipe or Unix socket
  -> command/response ID correlation
  -> IPC worker, queue, and runtime-owner pump
  -> delivery-mode selection for an attachment
  -> real Syncplay transport and server-visible state
  -> native GUI event loop
  -> rendering and accessibility
  -> user-visible recovery
```

The report should rename “full-stack lifecycle verification” to
“in-process lifecycle projection-chain verification” unless a system harness
crossing those boundaries is added.

### 3.2 Generated tests are strong finite simulations, not model checking

There are two different generated-history mechanisms and they should not be
conflated:

- `lifecycle.rs:5505-5578` runs 64 seeds by 128 steps but directly generates
  only 10 of the reducer's 32 input variants. It never directly generates such
  important inputs as `FileLoaded`, command rejection/completion,
  `TransportDelta`, `LocalFileChanged`, the seek-command family, an
  authoritative snapshot, or transport disconnect.
- `lifecycle/acceptance_tests.rs:1340-1460` has a broader roughly 24-way action
  grammar and runs four fixed history seeds for 384 steps, but its
  history/partition seeds are compiled into source, failure cases do not
  shrink, and the main oracle is production invariant preservation.
- GUI `trace_delivery.rs:966-1111` is substantially a fixed scenario template
  whose seed changes selected values, duplicate counts, terminal reason, and
  delivery partitioning. It is useful metamorphic replay, not arbitrary action
  generation.

These tests should remain as stable named regressions. A stateful property suite
should complement them with:

- a small independent reference model;
- explicit state-aware preconditions;
- valid and invalid transition grammars;
- shrinking;
- persisted minimized failures;
- per-step semantic assertions;
- liveness and bounded-resource properties;
- exhaustive partition enumeration for short histories.

### 3.3 The projection oracle has blind fields

The GUI verification projection marks multiple facts `Unavailable`, including
pending-event count, snapshot-required state, physical path/file-loaded state,
logical owner, and pending/terminal command results. The compatibility
comparator returns without checking a field when either side is `Unavailable`.
It also needs to compare the union of expected and actual attempt identifiers,
not only identifiers found on one side.

This means a layer can stop exposing a fact and a comparison may become weaker
instead of failing. The repair is an executable availability matrix:

| Fact | Producer must know | Client must know | GUI must know | May be unavailable |
|---|---:|---:|---:|---|
| attachment epoch | yes | yes | yes | no |
| active attempt/media generation | yes | yes | yes | no when attached |
| physical path/file-loaded | yes | policy decision | policy decision | explicitly declared |
| pending commands/results | yes | policy decision | usually no | explicitly declared |
| snapshot required/gap | yes | yes | diagnostic view | explicitly declared |

Each projection should assert its required availability before comparing values.
A transition that changes availability is itself a contract change.

### 3.4 Side effects are not an independent oracle

The projection chain largely feeds production-derived output into other
production consumers. A consistently wrong fact can converge across layers.
It also does not independently assert every consequential effect:

- session preparation or interruption;
- local-file publication;
- readiness changes;
- command dispatch and cancellation;
- recovery scheduling;
- protocol output;
- user-facing status and error state.

Add an effect ledger to the harness. Every scenario should state the expected
ordered effects and forbidden effects, and the ledger should be populated by
spies at the real service boundaries rather than reconstructed from the final
projection.

### 3.5 Feature-dependent proof is easy to omit

Test discovery found 693 client-core tests at default features and 695 with all
features. The two additional tests are the production batch-application and
verification-projection seams used by the lifecycle proof. Workspace feature
unification may happen to enable them today through another crate, but that is
dependency-topology luck, not a durable gate.

Every evidence record must include its feature set. CI should use explicit
`--all-features` for the documented proof and add a small feature-matrix job
where mutually exclusive or minimal feature sets have independent meaning.

## 4. What escaped after the report

Seventeen post-report fix commits added 81 regression tests. The defect classes
are more informative than the raw number:

| Escaped class | Representative regressions | Missing test dimension |
|---|---|---|
| Logical vs physical media boundary | acknowledged local-file change prepares a new boundary | downstream effect ledger |
| Cross-generation mapping | ordered transport maps adapter generation to pending logical media | independent identity model |
| Clock domains and queue dwell | ordered timestamp preserves queue dwell | typed/injected clocks; metamorphic time shift |
| Authority reconciliation | newer buffered pause event must not be overwritten | event reordering and causal model |
| Partial authority | failed post-playlist query must not resolve a partial snapshot | failpoints between snapshot reads |
| Cache pause semantics | cache pause must not become room pause | end-to-end semantic output |
| Delayed acknowledgement/overflow | late ACK and overflow fencing | concurrent schedule exploration |
| Privacy | nested credential canaries in transcripts | generated taint oracle |
| Poll/event races | polled load completion vs event load | framed worker/runtime schedule |
| Internal seeks | paused mpv internal seek cases | real player/system boundary |
| Protocol and actor ordering | transport action and actor commit order | model checking and deterministic network |
| Windows path/link safety | junction/symlink cleanup, open, and copy | Windows execution in CI |
| Server pressure | bounded queues and backpressure | load/soak and resource bounds |
| Release policy | publication/source-tip ordering | immutable artifact consumer tests |

This does not mean the original suite failed at its stated deterministic core.
It means that closing the reducer and projection model did not close the whole
application. The next investment should target qualitatively different
boundaries rather than adding hundreds more nearby examples.

## 5. Whole-application coverage map

| Surface | Existing strengths | Current enforcement or gap | Target assurance |
|---|---|---|---|
| Protocol codec/wire order | fixtures, additive extensions, malformed envelopes, ordering, redaction | hand-written raw JSON scanners have example-only coverage | roundtrip/metamorphic properties, byte fuzzing, differential Python oracle |
| Server network/auth/rooms | broad session, TLS, queue, permission, readiness tests; TLS rotation has content identity, deterministic and real-filesystem fault models, and real-network proofs | loose-file bundle publication is not generation-atomic (`TC-SERVER-004`); live matrix remains limited; non-TLS wall-clock tests remain | immutable versioned TLS publication, deterministic network simulation, strict live compatibility, load bounds |
| Server persistence | actor ordering, saturation, stale-version and degradation tests; positive corrupt-secret, concurrent-creation, and atomic row-migration regressions; 15 process-interruption stages with integrity-checked reopen | power-loss, disk-full/permission/syscall faults, and pre-transaction actor-message durability remain unproven | filesystem/storage failpoints, a pure arbitration model, schedule exploration, and platform durability probes |
| Client-core lifecycle | broad reducer/projection/reconnect examples, required shrinkable reconnect and post-emission acknowledgement models, and required all-feature execution | reset is a manual field list; session-level acknowledgement is modeled but transport-level reconnect/echo timing remains | carry the acknowledgement oracle through actual transport delivery, then add clock-controlled adapter schedules |
| CLI connected session | extensive reconnect/desync scenarios | 142 test-path sleeps; scheduler-luck risk | injected clock/timer, paused time, barriers, small real-socket tier |
| Player adapter | strong reducer/adapter tests, four real-mpv simulations, production-worker framed faults, and real Windows named-pipe fragmentation/correlation/disconnect/deadline coverage | the faulting peer is deterministic rather than real mpv; Windows named pipes cannot express independent socket half-close | retain the kernel-pipe matrix, add the Unix-socket equivalent, min/latest mpv, and real command/response traces |
| GUI runtime owner | many direct state and projection tests plus bounded shutdown through the actual threaded pump | most behavior still bypasses the pump; delivery-mode/mixed-getter debt remains | poison adapter through real refresh path, deterministic executor/clock |
| GUI semantic model | 14 scenarios, an exact required evidence lane, and explicit live-Python roster readiness | preview bridge rather than native render; one preserved historical timing failure | retain strict prerequisites and share the readiness protocol with native proof |
| Native GUI | typed AccessKit IDs, strict UIA inventory, acknowledged physical input, structured outcomes, detached/attached Open Media proof, two-sided Python readiness, fail-closed loopback fixtures, bounded observable shutdown, and pre-termination failure capture | the complete ten-scenario contract is locally green, but still needs an isolated interactive Windows CI lane and uses a deterministic player rather than real mpv | require the strict contract on an ephemeral interactive Windows lane, then add one real-mpv vertical slice |
| GUI render surface | many view-model tests | large low/zero-covered renderer files | structural accessibility tests and selected deterministic image baselines |
| Media match/stream helper | extensive index and extraction examples | low line coverage, process/error paths, ignored ffmpeg harness | parser properties, generated media, ffmpeg CI lane, failure injection |
| Python compatibility | fixtures and live TLS job | 16 assertions skipped in a green run; 77 skip-message sites | global require-live mode, pinned oracle revision, structured skip accounting |
| Settings/storage/update | atomic replace, path safety, before/after replacement hooks, and a committed/uncommitted multi-file recovery model are strong | OS process-kill/power-loss, filesystem durability, disk-full, and permission gaps remain | process interruption, filesystem syscall faults, parent-dir sync, and restart proof |
| Packaging/releases | strong path/publication scripts | package is not independently consumed before upload | extract, inspect, launch, update/rollback, provenance verification |
| Server container | non-root runtime | workflow builds and pushes in one step without smoke | build/load, protocol/TLS/persistence smoke, scan/SBOM, then push exact digest |

## 6. Current quantitative coverage

### 6.1 Static test inventory

| Crate | Test attributes | Ignored |
|---|---:|---:|
| `sorotte-cli` | 342 | 8 |
| `sorotte-client-app` | 182 | 0 |
| `sorotte-client-core` | 695 | 0 |
| `sorotte-compat` | 143 | 9 |
| `sorotte-core` | 2 | 0 |
| `sorotte-gui` | 1,157 | 2 |
| `sorotte-media-match` | 84 | 0 |
| `sorotte-player-api` | 21 | 0 |
| `sorotte-player-mpv` | 410 | 2 |
| `sorotte-plex` | 65 | 0 |
| `sorotte-protocol` | 48 | 0 |
| `sorotte-secret` | 4 | 0 |
| `sorotte-server` | 347 | 0 |
| `sorotte-sim` | 16 | 4 |
| **Total** | **3,516** | **25** |

Ignored tests are not one category:

- four `sorotte-sim` real-mpv cases are deliberately invoked by PR CI;
- player, CLI, and GUI real-mpv journeys remain local/manual;
- a GUI ffmpeg/ffprobe case is manual;
- compatibility fixture writers are maintenance tools disguised as ignored
  tests;
- two compatibility divergences are intentionally ignored.

Fixture generation should become an explicit tool/command. Intentional
divergence should be an executable named contract with expected differences.
Real integration tests should be assigned to a CI tier. No ignored test should
remain unclassified.

### 6.2 LCOV snapshot

The latest scheduled artifact reports:

- lines: 115,722 / 146,499 = 78.99%;
- functions: 9,424 / 11,977 = 78.68%;
- branches: no records.

Per-crate line coverage:

| Crate | Lines | Functions |
|---|---:|---:|
| client-core | 94.29% | 94.25% |
| core | 95.06% | 76.92% |
| sim | 93.60% | 92.86% |
| protocol | 93.22% | 92.98% |
| secret | 93.43% | 95.45% |
| player-api | 92.86% | 93.14% |
| server | 89.83% | 91.01% |
| plex | 89.53% | 87.36% |
| player-mpv | 89.10% | 86.47% |
| client-app | 86.73% | 90.46% |
| CLI | 77.92% | 77.05% |
| GUI | 70.45% | 70.15% |
| media-match | 58.59% | 55.96% |
| compatibility | 47.81% | 46.35% |

The aggregate hides major blind spots. Examples include:

- `render_egui/room_browser.rs`: 0 of 1,145 lines;
- `native_host/eframe_app.rs`: 0 of 354;
- `runtime_owner/requests/stream_helper.rs`: 0 of 280;
- GUI plugin renderer: 10 of 1,058;
- GUI playback controls: 10 of 306;
- GUI playlist renderer: 68 of 863;
- multiple compatibility process/parser files at 0%;
- media-match diagnostics at 0%;
- server `main` at roughly 39%.

Some of these values were instrumentation gaps rather than absent testing.
The implementation now merges the complete semantic inventory and strict
live-TLS compatibility selector with the workspace profiles. Updater process
variants, native interactive Windows, other OS-specific execution, and the
currently red complete legacy fanout matrix remain separate. That distinction
is exactly why every merged lane must attest both fresh instrumentation and
its own behavioral oracle.

### 6.2.1 Implementation-branch LCOV experiments

The original locked all-feature command completed locally on Windows
with cargo-llvm-cov 0.8.4 on the pinned Rust 1.97.1 toolchain:

```text
392 source files
LLVM LF/LH summary: 145,926 / 187,537 lines = 77.81%
explicit DA inventory: 142,777 / 181,281 lines = 78.76%
15,089,306-byte LCOV artifact
SHA-256: 24a96fa660daae828293b67f6505c315b593aace64ae8a15a3df27e0195a62a5
```

An independent audit found `LF` or `LH` contradictions in 309 of 392 records,
with no duplicate `DA` lines. At that point strict replay rejected the artifact
before computing either the 80% ordinary or 90% critical result. The first
instrumented attempt also exposed an intermittent CLI test failure and 19
secondary shared-lock poison failures. The same root failure later appeared
without instrumentation; its isolated selector and CLI suite passed, and the
full workspace retry including doctests passed in 181.7 seconds. This is
useful proof of producer feasibility and why the phase report must preserve
failed first attempts; details and artifact digest are in
[TEST_COVERAGE_FINDINGS.md](TEST_COVERAGE_FINDINGS.md#tc-harness-004-intermittent-cli-failure-poisons-the-shared-test-lock).

The repaired exact-final-source experiment completed the instrumented
workspace in 235.1 seconds and produced:

```text
395 source files
declared LF/LH summary: 148,045 / 190,067 lines = 77.89%
positive/unique DA:    144,853 / 183,712 lines = 78.84%
15,369,296-byte LCOV artifact
SHA-256: 1998ea2b60336018b796c5e2a6e14cd6cc58ac36377f6914993b86c18bd136bf
```

The contradiction remains widespread: 310 records mismatch at least one
summary, 308 mismatch `LF`, and 259 mismatch `LH`. The diagnostic consumer now
preserves both models, explicitly evaluates `unique-da-source-lines`, and
still fails missing executable `DA` mappings. A separate PowerShell scanner
matched every aggregate. The exact experiment is retained in
[`lcov-dual-model-20260729.md`](evidence/test-coverage/lcov-dual-model-20260729.md).

The named pinned toolchain initially lacked the LLVM tools component.
`llvm-tools-preview` was explicitly installed on that toolchain in 237.5
seconds before the fresh experiment. CI provisions the component explicitly
as well.

### 6.2.2 Merged behavioral profile experiment

`scripts/coverage_profile_lanes.py` now owns four ordered phases: all-feature
workspace execution, the exact 14-scenario semantic suite, the complete
20-test strict live-reference compatibility inventory, and an LLVM merge
check. It accepts only
cargo-llvm-cov 0.8.4 and pinned Syncplay commit
`d1c5f85af377c960c5a940707c4d01bc84fd9c3f`. External Cargo processes receive
the producer's parsed `show-env` contract and an isolated
`target/llvm-cov-target`; every execution lane must create or change at least
one recursively inventoried raw profile. Schema version 2 first removes and
attests stale generated raw/merged inputs, requires the workspace to start at
zero, binds profile-count continuity across every lane, detects content changes
even when size and mtime are unchanged, and forbids removal of an earlier
lane's evidence.

The current broadened local attestation passed:

| Lane | Behavioral result | Duration | Fresh profiles |
|---|---:|---:|---:|
| all-feature workspace | pass | 188.002s | 34 |
| GUI semantic | 14/14 | 8.456s | 1 |
| strict live reference | 20/20, 121 filtered | 18.048s | 1 |
| LLVM merge | `TOTAL` present | 1.554s | merge-only |

The historical first clean replay removed 229 raw and one merged profile from
earlier experiments. The exact-final replay removed 36 prior raw profiles, began
the workspace lane at zero, recreated exactly 36 profiles, and removed none
during a lane. Its merged diagnostic view covered 148,594 of 191,287 line
instances (77.68%). The earlier downstream source-bound experiment covered
145,016 of 183,712 unique physical lines (78.936596%); neither percentage
replaces the changed-line policy. All 290 Python infrastructure and
workflow-policy tests pass with the broadened oracle and workflow binding.

Two deliberately failing experiments constrained the original claim. A semantic run
that reused `target/debug` passed 14/14 but produced no profile delta and was
rejected. The complete strict legacy matrix passed 129 tests, failed six, and
ignored nine in 88.98 seconds, so it was not relabeled as a green coverage
lane. One parallel instrumented workspace run also exposed the intermittent
`TC-PLAYER-003` observation. Investigation proved a test synchronization race,
not product event loss. The compatibility divergences and two timeout harness
failures were likewise fixed at their owning boundaries. The promoted
`legacy_server_` lane now passes exactly 20 tests with zero ignored and 121
filtered; its inventory and counts are hard-coded in the oracle. Full
before/after evidence is in
[`merged-profile-lanes-20260729.md`](evidence/test-coverage/merged-profile-lanes-20260729.md).

### 6.3 Static risk and tooling inventory

The codebase is not only large; it has substantial schedule, process, and
platform surface:

| Static signal | Matches | Files |
|---|---:|---:|
| `Mutex` | 307 | 55 |
| atomics | 227 | 42 |
| `thread::spawn` | 75 | 30 |
| `tokio::spawn` | 163 | 48 |
| mpsc usage | 316 | 48 |
| unsafe blocks | 117 | 21 |
| Windows-gated code | 133 | 38 |
| Unix-gated code | 19 | 9 |

Most tests are inline unit/module tests; there are comparatively few
crate-boundary integration binaries. Inline tests are fast and can access
private state, but they do not automatically exercise public construction,
feature resolution, process startup, or cross-crate wiring.

The baseline audit found no use of Proptest/QuickCheck,
Loom/Shuttle/Turmoil, `cargo-fuzz`, `cargo-mutants`, `trybuild`, or
`cargo-semver-checks`. This branch introduces Proptest for lifecycle models,
cargo-nextest 0.9.137 for fail-on-flaky execution, and cargo-llvm-cov 0.8.4 for
line evidence. Fuzz/mutation tools, nightly/Miri, local mpv, and Docker were
not available in the original audit environment. This is a sequencing
constraint, not a product failure: introduce and pin tools one at a time,
record their versions in evidence, and establish a trustworthy baseline before
making a new signal blocking.

## 7. Priority findings

### Audit-baseline P0 — Existing proof was not enforced

[DEVELOPMENT.md](DEVELOPMENT.md) requires all-feature Clippy/tests and
semantic/native coverage for relevant changes. At the audited baseline,
pull-request [rust-ci.yml](../.github/workflows/rust-ci.yml) ran
default-feature Linux Clippy/tests, scheduled “deep” repeated default features
on two Ubuntu versions, Windows compiled release binaries without workspace
behavior tests, and the semantic suite was absent. This branch now requires
locked all-feature Linux and Windows execution plus exact semantic evidence;
the interactive native lane remains strict but unproven.

**Decision:** CI is the executable definition of done. A report may summarize
CI; it must not be a second, manually maintained source of truth.

### Audit-baseline P0 — Native smoke had permissive false-positive semantics

The pre-hardening local native run exited successfully while recording:

- `menu_labels: []`;
- `menu_contract: "skipped-no-native-menu"`;
- no usable Open Media discovery method;
- `open-media-file-skipped`;
- repeated failed DNS attempts to `syncplay.example:8999` and
  `saved.example:8999`.

The native adapter remains a deterministic synchronous test player, not the
acknowledged lifecycle adapter or real mpv. It now has an opt-in observation
boundary that proves the exact path delivered to `open_file` without changing
normal test-player behavior.

The historical producer converted an empty menu contract and failed Open Media
discovery into successful skips. That implementation and its raw output are
preserved in `docs/evidence/test-coverage/native-baseline-20260728.md`; the
current producer no longer contains either skip path.

The old wrapper selected only `baseline` and `relaunch` by default. The
hardened wrapper now requires all ten implemented scenarios, rejects forbidden
skips and unexpected stderr, and preserves failed native screenshots and
accessibility trees before terminating any primary or secondary live-window
scenario. The final strengthened detached/attached menu contract passed three
consecutive interactive runs. Two complete interactive attempts of all ten
scenarios correctly failed: first in `live-python`, then in `controlled-room`,
because `interop-py-peer` did not appear. Both also emitted forbidden
placeholder DNS and TLS diagnostics. Follow-up isolated diagnostics exposed a
File-popup input flake and a separate 80-second File -> Exit shutdown timeout.
These were recorded as TC-HARNESS-007 through TC-HARNESS-009 and TC-NATIVE-002.

The follow-up implementation resolves all four without broad retries or stderr
allowlists. The Python harness and UIA runner perform a two-sided roster
handshake under one deadline. Native launches use typed detached/in-process/TCP
loopback modes and reject non-loopback TCP hosts before spawn. Scenario-owned
servers remain alive until explicit release. Windows clicks require foreground
acknowledgement, an exact UIA hit test, and atomic absolute-coordinate
move/down and move/up endpoints; the baseline performs 25 single-delivery menu
transactions and contains no toggle redelivery path. File -> Exit requires an
ordered five-event product lifecycle trace and process exit within four
seconds, backed by a bounded threaded-runtime shutdown.

The final semantic replay also re-exposed the older live-Python playlist timing
gap. Optimistic shell state could satisfy a playlist assertion after one owner
pump, after which a blocking peer observation starved the receipt-owned
multi-frame transport. Peer playlist and index observations now poll snapshots
while continuing to pump the real owner, and the Python fixture truthfully
advertises the shared-playlist capability it exercises. This turns transport
progress into an explicit causal condition instead of depending on host
scheduling or retries.

Final current-source replay found a related native state-acknowledgement gap.
An Interface & System tab could be focused while Playback & Search content
remained active, even though both UIA and physical APIs returned success. Top
tabs now advance through accessibility, physical, and exact focused-keyboard
strategies only after the expected content appears. The baseline deliberately
proves the keyboard modality. That replay also falsified the menu retry model:
the shared desktop cursor moved between the intended-coordinate hit test and a
zero-coordinate `SendInput` button event. Physical endpoints are now atomically
bound to the normalized virtual-desktop target, and toggles are never
redelivered. The native binary's 25 unit contracts are enrolled in
`cargo test --workspace --all-features` rather than depending on an explicit
binary-target command.

Three final consecutive baseline runs covered 75 single-delivery physical menu
transactions. Complete ten-scenario runs at
`target/verification/gui-native-smoke/20260729T060756380Z-55276` and
`target/verification/gui-native-smoke/20260729T061005422Z-54068` then passed
all behavioral and strict checks in 110,173 and 109,614 ms respectively, both
with zero native stderr. The causal failures, rejected hypotheses, artifact
hashes, and final proof are preserved in
[`docs/evidence/test-coverage/native-input-ownership-20260729.md`](evidence/test-coverage/native-input-ownership-20260729.md).
The remaining work is CI placement, not an open native-contract defect.

**Decision:** every native/semantic contract must have one of four structured
states: `required-pass`, `optional-pass`, `optional-skip(reason)`, or
`failure`. A required skip is a failure. Unexpected stderr, panic, outbound
network, or background-task failure is a failure unless explicitly allowlisted
by scenario. Use only test-owned loopback endpoints. Preserve JSON, logs,
screenshot, accessibility tree, configuration, and process state on failure.

Native UI Automation needs an interactive Windows session. Run the strict
native lane on an ephemeral, isolated interactive runner with no production
secrets and no reusable developer state. If that is not available, restrict it
to trusted merge-queue commits rather than executing untrusted pull-request
code on a persistent machine. Keep the semantic model as the hosted-runner PR
gate.

### P0 — Compatibility can be green without its oracle

The all-feature compatibility run printed 16 assertion-skipped messages yet
returned success. Static inspection found 77 skip-message sites across 11
files. The required PR live job covers only a TLS subset; the full release
verifier is scheduled/manual.

**Decision:** add `SYNCPLAY_REQUIRE_LIVE_INTEROP=1`. Under this mode, a missing
Python dependency, legacy process, TLS capability, fixture, or enabled
assertion is a failure. Pin the Python Syncplay revision and prerequisite
versions. Emit structured executed/skipped counts. Run the complete required
matrix on PR; move only truly expensive or external canaries to nightly.

### Audit-baseline P0 — Windows behavior was compiled, not tested

The source contains at least 133 Windows-gated matches across 38 files. At the
audit baseline, recent escaped regressions included basename, junction,
symlink, updater, and replacement behavior, while the required Windows lane
only compiled. A release compile cannot catch a wrong Windows branch. This
branch now requires locked all-feature Windows tests, although the interactive
native boundary remains separately unproven.

**Decision:** require Windows all-feature behavior tests. Split them if the
workspace is too slow:

1. protocol/client-app/client-core/CLI/server/path/security;
2. GUI library and updater integration;
3. strict native scenarios on the interactive runner.

### P0 — Report evidence can drift

The report's counts demonstrate the drift: it records 681 client-core tests,
while current all-feature discovery finds 695; it records 1,089 GUI tests plus
two ignored, while the current filtered GUI library run implies 1,105 tests.
Most proof rows name behavior categories rather than exact test identifiers.
Nothing fails if a test is renamed, ignored, filtered out, or loses its oracle.

**Decision:** generate the report appendix from a behavior manifest and CI
evidence bundle tied to the exact SHA.

### P1 — Wall-clock tests dominate fragile paths

Static inventory found:

- 178 `thread::sleep` matches in 59 files;
- 146 Tokio sleep matches in 41 files;
- 569 `Instant::now` matches in 92 files;
- 134 `SystemTime::now` matches in 75 files;
- 142 CLI test-path sleeps alone.

A rerun history sample found a TLS rotation file-change failure and a real-mpv
cache timeout that passed on retry. The native and reconnect surfaces also rely
on polling deadlines.

**Decision:** introduce domain clocks, timer/sleeper traits, paused Tokio time,
and event barriers. Retain a thin real-time smoke layer. A retry is diagnostic;
pass-after-fail remains a flaky failure.

Branch implementation now covers the first CLI and server TLS rotation
boundaries. Reconnect backoff and terminal exhaustion have exact paused-clock
assertions; STARTTLS response and TLS handshake timeouts advance only after
their corresponding protocol barriers; the server-Hello timeout starts after
observed client-Hello delivery; and a real-loopback retry runs under virtual
time. TLS rotation uses an explicit metadata revision across exhaustive model
and real-network tests. Broad CLI sleep inventory, production content
fingerprinting, persistence, process supervision, and native GUI timing
remain.

### P1 — IPC/parser fault coverage starts too high

Lifecycle transcript capture starts after JSON decoding. Handwritten protocol
JSON-order scanners and IPC framing do not have coverage-guided fuzzing.

**Decision:** record and replay a sanitized duplex boundary with outgoing
command, command ID, synchronous response, incoming event, frame boundaries,
disconnect/error, attachment/generation, and monotonic offset. Add an in-memory
framed transport supporting split/coalesced/truncated frames, malformed JSON,
response/event reorder, duplicate/drop/delay, half-close, and reconnect.

### P1 — Persistence and atomic storage lack crash protocols

Existing persistence tests are strong at actor semantics. This branch now
has positive regressions for corrupt metadata, concurrent quota-secret
creation, and atomic multi-row playlist migration. `SRV-PERSIST-001` also
terminates a dedicated child process at 15 schema, transaction, actor, stats,
and secret-creation stages, then proves integrity, complete old-or-new state,
normal recovery, and a second idempotent reopen. Gaps now begin below that
process boundary: filesystem and power-loss durability, disk-full and
permission/syscall failure, plus queued actor intent that has not entered a
transaction.

**Decision:** define the durability contract—old complete state or new complete
state, never partial. Retain the implemented SQLite process-restart matrix and
extend the same rule through write/flush/sync/permission/rename/directory-sync
boundaries where Sorotte owns the filesystem protocol.

### P1 — Release artifacts are not consumed before publication

The GUI workflow builds, packages, and uploads. The server container workflow
builds and pushes in one step. Neither independently consumes the precise final
artifact before publication.

**Decision:** test immutable artifacts, not just source. Build/load first,
inspect and run, then publish the exact digest or archive already tested.

The supply-chain gate should also cover the mechanism that produces those
artifacts. This branch pins every third-party action in the Rust CI and
coverage workflows to a verified full commit SHA and adds pinned actionlint
validation. Other release/container workflows still use floating references,
and the container build uses tag-only base images and a Cargo build without
`--locked`. Finish pinning action and base-image digests, automate
dependency/action update pull requests, and run license/advisory/source policy.
These checks do not replace behavior tests; they prevent an unreviewed build
input from invalidating otherwise good evidence.

## 8. Target assurance architecture

### 8.1 Machine-readable behavior catalog

Create `coverage/behaviors.toml` and a small `xtask` verifier. Example:

```toml
[[behavior]]
id = "PL-ACK-003"
title = "A delayed snapshot acknowledgement cannot retire a newer overflow gap"
risk = "critical"
owners = ["player", "client-core", "gui"]

[[behavior.proof]]
kind = "model"
package = "sorotte-player-mpv"
target_kind = "lib"
target_name = "sorotte_player_mpv"
test = "lifecycle::tests::delayed_snapshot_ack_preserves_a_new_overflow_gap"
feature_mode = "all-features"
operating_systems = ["linux", "windows"]
required_lanes = ["lifecycle-contract"]

[[behavior.proof]]
kind = "projection-chain"
package = "sorotte-gui"
target_kind = "lib"
target_name = "sorotte_gui"
test = "app::runtime_owner::player::telemetry::lifecycle_verification_tests::trace_delivery::gap_snapshot_retains_indeterminate_until_ack_then_correlates_late_physical_effect"
feature_mode = "all-features"
operating_systems = ["linux"]
required_lanes = ["lifecycle-contract"]

[[behavior.proof]]
kind = "real-player"
scenario = "replacement-late-ack"
operating_systems = ["windows"]
required_lanes = ["nightly-native", "release-native"]

[behavior.invariants]
statements = [
  "old attachment batches never mutate the new attachment",
  "compaction never removes unacknowledged new-generation facts",
]
```

Suggested namespaces:

- `PL`: player lifecycle and IPC;
- `SYNC`: playback coordination and clocks;
- `NET`: transport, TLS, reconnect, and ordering;
- `SRV`: rooms, authorization, backpressure, and persistence;
- `CFG`: config, storage, and precedence;
- `GUI`: semantic/native/accessibility behavior;
- `COMPAT`: Python/legacy differential contracts;
- `MEDIA`: index, matching, Plex, and extraction;
- `SEC`: privacy, path, link, credential, and update trust;
- `REL`: package, container, update, and publication.

The verifier should fail when:

- a required behavior lacks a proof at its required tier;
- a referenced exact test or scenario no longer exists;
- a referenced test is ignored or filtered out;
- required live evidence was skipped;
- a critical behavior relies only on line coverage;
- a waiver lacks an owner and expiry.

Each isolated CI lane should emit a schema-versioned shard such as
`target/verification/evidence.lifecycle-contract.json` containing SHA, OS,
package, target, full test identifier, feature set, exact command, tool
versions, runtime, pass/fail/ignored/skipped counts, seeds, and artifact links.
Upload shards even on failure.

A final `verification-required` job should use `if: always()`, download every
required shard, compare each with the behavior manifest and exact pull-request
head SHA, inspect dependency-job conclusions, reject missing/duplicate/stale
shards, and generate:

- aggregated `evidence.json`;
- the human-readable report appendix;
- a concise missing/failed/skipped proof summary.

Branch protection should depend on this aggregator as well as any security
boundary that must not be reduced to an artifact assertion. This makes
cross-machine proof composition explicit instead of pretending isolated jobs
share one filesystem.

### 8.2 Four kinds of oracle

Use the weakest sufficient layer, but do not substitute one layer for another:

1. **Specification oracle:** a compact independent model or explicit expected
   facts/effects.
2. **Metamorphic oracle:** reordering, partitioning, time shifting, duplicate
   delivery, serialization roundtrip, and equivalent input transformations
   preserve declared behavior.
3. **Differential oracle:** Rust vs pinned Python Syncplay, supported mpv
   versions, writer vs parser, package manifest vs extracted artifact.
4. **Observational oracle:** protocol output, mpv state, native accessibility,
   file system, process lifecycle, logs, and public artifact digest.

Production code feeding another production projection is useful integration
coverage, but it is not an independent specification oracle.

### 8.3 Testability seams

Add narrow seams, not a parallel test implementation:

- monotonic `Clock` and wall-clock `UtcClock` newtypes;
- scheduler/timer abstraction or Tokio paused time;
- executor/spawner ownership with deterministic drain/shutdown;
- framed duplex IPC transport;
- resolver and connector interfaces;
- filesystem durability operations;
- process launcher and process observer;
- effect ledger for session/player/server/user-visible effects;
- strict capability/precondition reporting.

Keep the production implementations thin. The state machines behind them
should remain ordinary Rust that property and mutation tools can exercise.

## 9. New testing strategies

### 9.1 Stateful property and model testing

Adopt `proptest` first in player-mpv, client-core, protocol, client-app config,
server persistence arbitration, and media-match normalization.

The lifecycle model should generate:

- load/command submission, acceptance, rejection, supersession, completion,
  and unobserved completion;
- start/file-loaded/end/EOF/restart;
- position, seeking, phase, and transport deltas;
- local-file updates;
- seek submissions and outcomes;
- gaps, snapshots, timer advancement, disconnect, and attachment replacement;
- valid, stale, duplicate, unknown, and cross-generation identifiers;
- delivery partition, duplicate, drop, and delayed acknowledgement actions.

After every transition assert:

- current executable lifecycle invariants;
- first terminal result wins;
- identities and generations are monotonic and correctly fenced;
- a predecessor cannot leak presentation or ownership into its successor;
- no stale attachment changes the current attachment;
- duplicate application is idempotent;
- partitioning does not change semantic results;
- a gap requires a snapshot before incremental authority resumes;
- buffers, tombstones, queues, and retained results remain bounded;
- the independent model, reducer, adapter, client, and GUI agree on the facts
  each layer is required to retain;
- expected effects occur once and forbidden effects do not occur.

Policy:

- PR: 256 cases plus committed regression corpus;
- nightly: at least 4,096 cases per model, increasing where runtime permits;
- weekly: 10,000+ and multiple feature/OS configurations;
- always print and archive seed and minimized action sequence;
- commit `proptest-regressions` cases after triage.

Use exhaustive enumeration for short sequences and all delivery partitions;
random generation is not a substitute where the state space is small.

Reference: [Proptest state-machine testing](https://proptest-rs.github.io/proptest/proptest/state-machine.html).

### 9.2 Coverage-guided fuzzing

Create a workspace `fuzz/` package with targets for:

1. protocol line/JSON decoder and raw command-order scanner;
2. framed mpv JSON IPC decoder;
3. transcript decoder and sanitizer;
4. INI/config parser, precedence, and writer roundtrip;
5. media index/report and Plex response parsing;
6. lifecycle action/batch validation;
7. server pre-Hello and batched-command dispatch.

Seed corpora from existing protocol fixtures, lifecycle transcripts, semantic
configs, media reports, and minimized production failures.

Core properties:

- arbitrary bytes never panic, hang, or allocate without a bound;
- successful parsing preserves recognized command order;
- encode/decode is semantically stable;
- unknown fields remain additive;
- malformed lifecycle batches fail closed;
- generated nested credential canaries never appear in transcript/debug output;
- a fuzz crash becomes a normal regression test after minimization.

PR should compile all targets and replay the corpus, with a rotating 30–60s
smoke for changed parsers. Nightly should run 10–15 minutes per target in
parallel; weekly runs can be longer.

Reference: [Rust Fuzz Book](https://rust-fuzz.github.io/book/).

### 9.3 Deterministic network and concurrency schedules

Use different tools at different scales:

- use a deterministic Tokio network simulator such as Turmoil for server/client
  partitions, delay, disconnect, restart, and clock control after extracting a
  small network boundary;
- use Loom only for small synchronization primitives and queues;
- use Shuttle-style randomized deterministic schedule replay for larger actor
  and worker workflows where a full Loom port is inappropriate.

Initial Loom models:

- producer append concurrent with acknowledgement;
- reattachment concurrent with stale delivery;
- compaction concurrent with replay;
- queue overflow concurrent with replacement;
- persistence sender/drop/flush arbitration.

Initial deterministic network scenarios:

- reconnect during delayed Hello/state;
- half-close and TLS rotation;
- dropped/duplicated/reordered state and command lines;
- backpressure under slow reader;
- server restart with persistent room;
- two clients receiving logically equivalent histories in different partitions.

PR runs bounded tiny models. Nightly records and replays thousands of schedules.
Do not port the entire Tokio application to Loom: its value is exhaustive
exploration of carefully isolated primitives, and its limitations must remain
visible.

References:
[Loom](https://github.com/tokio-rs/loom) and
[Turmoil](https://tokio.rs/blog/2023-01-03-announcing-turmoil).

### 9.4 Failure injection and crash consistency

Add test-only failpoints at meaningful transactional boundaries:

- database begin/read/write/commit/busy;
- per-row migration;
- persistence actor dequeue/save/delete/result/flush;
- temporary file create/write/flush/file sync/permissions/replace;
- parent-directory sync;
- updater download/stage/verify/replace/rollback;
- stream-helper download/extract/verify/install;
- process launch/pipe connect/first command/shutdown.

Each failpoint test should restart from the produced on-disk state and prove:

- old valid state or complete new valid state;
- idempotent recovery;
- no corrupt partial record;
- no leaked secrets, temporary payloads, or unbounded retry;
- accurate degraded/recovered reporting.

Keep global failpoints in a separate process/test binary so parallel tests
cannot affect each other.

Branch implementation now covers 15 SQLite/process boundaries in an exact
child test process: every legacy schema step, playlist migration before/after
commit, room save/delete before/after commit, stats snapshot before/after
commit, and quota-secret generation/insertion. Each parent proof runs integrity
checking before recovery and a second reopen after it. Existing in-process
SQL-trigger tests retain degraded/recovered reporting coverage. Database
begin/busy/disk-full/permission injection, pre-transaction queue loss, OS
power-loss durability, and Sorotte-owned file/rename/directory-sync protocols
remain.

Reference: [Rust `fail` crate](https://docs.rs/fail).

### 9.5 Mutation testing

Run `cargo-mutants` against pure, high-risk modules first:

- lifecycle reducer and batch validation;
- attachment/generation fences and write-once outcomes;
- client reconnect reset/preservation;
- protocol key-order scanner and redaction;
- config precedence/atomic-state decisions;
- media identity/scoring;
- server authorization, backpressure, and persistence arbitration.

Baseline missed, unviable, and timeout mutants. Add tests for meaningful
survivors, classify true equivalents, then gate **new missed mutants in changed
critical code**. Do not start with the whole GUI renderer, FFI shims, generated
code, or entrypoints. Do not trust mutation results until flakiness is under
control.

Targets:

- no new surviving mutant on modified critical logic;
- at least 80% meaningful kill rate in selected pure critical modules after
  the initial baseline;
- every surviving critical mutant has an owned issue or justified exclusion.

Reference: [cargo-mutants](https://mutants.rs/getting-started.html).

Implementation status (2026-07-29): the scheduled matrix covers three critical
boundaries with pinned cargo-mutants 27.1.0. The original `sorotte-secret`
privacy shard moved from 22/43 to 43/43 viable mutations caught against an
identical 44-mutant inventory. Credential-classifier expansion later caused a
clean required-shard replay to fail with 29 missed and five timed-out mutants;
bounded scans and deterministic escape, key, hex, and token oracles now catch
121/121, with the original exact, expiring const exception still the only
unviable mutation. The `sorotte-server` authorization shard rejected an
unsuitable package-wide baseline, then used the real 16-caught/1-missed/
1-timeout library result to add deterministic grammar and salt-byte oracles.
Its focused namespace catches 19/19 with no exception. The
`sorotte-protocol` codec/redaction baseline caught 70/97 viable mutations.
Seventeen exact scanner, error-chain, and redaction tests plus bounded scanner
seams now catch 80/80 with zero misses or timeouts; eight generated
default-value replacements are compiler-infeasible and matched by exact,
expiring identities. The combined-file shard also exposed and regression
covered cargo-mutants' `function: null` metadata for top-level constants in
the attesting wrapper. Policy schema 2 and the strict wrapper bind
package/library target and test namespace as well as source hashes,
inventories, structured outcomes, status files, command phases, artifacts,
and producer exit. See
[`targeted-mutation-20260729.md`](evidence/test-coverage/targeted-mutation-20260729.md)
,
[`targeted-mutation-privacy-expansion-20260729.md`](evidence/test-coverage/targeted-mutation-privacy-expansion-20260729.md),
and
[`targeted-mutation-server-auth-20260729.md`](evidence/test-coverage/targeted-mutation-server-auth-20260729.md),
and
[`targeted-mutation-protocol-codec-20260729.md`](evidence/test-coverage/targeted-mutation-protocol-codec-20260729.md).
This establishes the mechanism and three critical shards, not workspace-wide
mutation assurance.

### 9.6 Genuine vertical player system harness

Build a test-owned system composed of:

- the actual GUI executable;
- real supported mpv;
- a loopback Sorotte server;
- a fault-injecting local HTTP media server;
- isolated config, storage, and generated local media.

Drive the GUI through Windows UI Automation/accessibility. Observe three
independent outputs:

1. visible/accessibility state;
2. Syncplay protocol output and room readiness;
3. mpv IPC/process state plus structured lifecycle/effect trace.

Future required PR or pre-merge scenarios once the Tranche D system harness
lands:

- open local media and reach active/readiness state;
- replace A with B while A emits late events;
- kill mpv and verify bounded GUI/runtime recovery;
- premature EOF or stalled HTTP read without logical-media corruption.

Nightly scenarios:

- `start-file` before `file-loaded`;
- timeout followed by late activation;
- IPC disconnect between response and event;
- gap/overflow and snapshot reacquisition;
- rapid A→B→C;
- seek during replacement;
- paused internal seek;
- sleep/resume and process relaunch;
- minimum and newest-supported mpv.

External YouTube should remain a nonblocking canary because availability,
credentials, rate limits, and media behavior are not controlled. All required
behavior should use generated local media and a fault-injecting loopback server.

### 9.7 Strict semantic, native, visual, and accessibility tests

Semantic suite:

- require all 14 current scenarios on PR;
- add lifecycle/recovery scenarios rather than only configuration and shell
  workflows;
- make every precondition explicit;
- reject unexpected warnings/background failures;
- publish structured result and trace artifacts.

Native suite:

- define required vs optional capabilities in scenario metadata;
- fail required skips;
- use loopback endpoints only and assert no unexpected outbound connection;
- assert roles, automation IDs, enabled state, patterns, focus, tab order, and
  keyboard-only workflows;
- preserve screenshot, UIA tree, logs, config, and process state on every
  failure;
- exercise high DPI, light/dark theme, and one long-text locale nightly;
- run on an ephemeral, isolated interactive Windows runner without production
  secrets or reusable developer state, or only on trusted merge-queue commits.

Visual coverage:

- keep review packets for broad exploratory layout inspection;
- add deterministic perceptual baselines only for a small set of critical,
  stable states;
- pair every image assertion with semantic/accessibility assertions;
- require human approval for intentional baseline changes.

### 9.8 Differential compatibility

Pin the sibling Python Syncplay oracle revision and hash every captured fixture.
For each compatibility behavior, record:

- Rust execution;
- live Python/legacy execution;
- normalized trace comparison;
- allowed named divergences;
- prerequisite/skip status.

Run the same generated protocol histories against both implementations where
possible. Unknown/additive fields and ordering should be compared semantically,
not by fragile full-string equality. Intentional timeout or liveness
differences should be active expected-difference tests, never permanent ignored
tests.

### 9.9 Privacy and security properties

Turn privacy into a generated taint property:

- generate unique secrets in nested maps, arrays, URLs, headers, filenames,
  advanced arguments, protocol extensions, and error messages;
- pass them through parsing, debug formatting, transcript capture, logging,
  diagnostics, GUI error projection, and crash artifacts;
- assert no raw or encoded canary is present;
- separately assert the safe diagnostic fields needed for support remain.

Branch implementation covers hundreds of recognized transcript and
`PlayerError` cases across nested maps/arrays, JSON escaping, URLs, headers,
cookies, paths, malformed parser input, fixed-point round trips, `Debug`, and
diagnostic dumps. The generated oracle surfaced three distinct unredacted
families, formerly recorded as `TC-SEC-001` through `TC-SEC-003`. A shared
structured-key and diagnostic credential policy now closes those families,
and their characterizations are positive regressions. Safe parser and mpv
request diagnostics are explicit false-positive canaries. Protocol, GUI
projection, process logs, and crash artifacts still need the same
generated-taint treatment.

For path and update security:

- generate symlink, junction, hardlink, alternate separator, case, reserved
  name, long path, and time-of-check/time-of-use mutations;
- run on Windows and Unix;
- use a test attacker that swaps path components between validation and use;
- verify update/package signatures, manifests, source SHA, and downgrade policy
  at the final consumption boundary.

### 9.10 Performance, load, soak, and chaos

Add microbenchmarks for:

- protocol encode/decode and raw order scanning;
- lifecycle reduction/batch compaction;
- media identity/index/scoring;
- config parse/write;
- server fanout/backpressure.

Hosted-runner numbers should be informational. Enforce relative thresholds only
on stable dedicated hardware.

Nightly 20–30-minute deterministic soak:

- many rooms/clients;
- reconnect storms;
- slow readers;
- repeated replacement/gap/recovery;
- bounded queue, tombstone, task, thread, handle, and memory growth;
- clean shutdown and persistence flush.

Weekly up-to-two-hour chaos:

- network latency, jitter, bandwidth cap, half-close, reset, and TLS rotation;
- HTTP stall, truncate, wrong range, premature EOF, and recovery;
- database busy/disk-full/corruption-at-boundary;
- process kill and restart;
- GUI relaunch and updater rollback.

Mechanical success criteria:

- finite convergence;
- no duplicated semantic completion;
- no divergent room state;
- no stale generation publication;
- resource bounds remain below declared limits;
- clean termination with no orphan process.

### 9.11 Undefined behavior and API compatibility

Add a nightly Miri shard for pure crates/targets first:

```text
cargo +nightly miri test \
  -p sorotte-secret \
  -p sorotte-core \
  -p sorotte-protocol \
  -p sorotte-player-api \
  --lib --locked
```

Use targeted Linux ASan/LSan for server, player-mpv, and media-match integration
tests. Consider TSan only for selected native-threaded boundaries and keep tool
limitations explicit.

Use a downstream fixture crate plus `trybuild` for intended public API
construction/compile failures. Run `cargo-semver-checks` when public crates
change. This is especially valuable for acknowledged batch types and adapter
traits whose accidental source breakage ordinary workspace tests cannot see.

References:
[Miri](https://github.com/rust-lang/miri) and
[cargo-semver-checks](https://docs.rs/crate/cargo-semver-checks/latest/source/README.md).

## 10. Coverage policy

### 10.1 Collect the tests users depend on

Pin `cargo-llvm-cov` and collect:

1. all-feature workspace tests;
2. focused lifecycle/integration binaries;
3. semantic suite under `cargo llvm-cov show-env`;
4. live compatibility;
5. Windows test profiles;
6. process/system harnesses where instrumentation is practical.

Merge compatible profiles or upload OS/lane flags to a service that presents a
merged and per-platform view. Keep “not instrumented” distinct from
“instrumented and not executed.”

Implementation status: items 1, 3, and 4 are fully merged. Live compatibility
now includes all 12 strict fanout scenarios, 4 TLS probes, 2 live state probes,
and 2 request-shim contracts. The collector parses
cargo-llvm-cov's `show-env`, isolates external instrumented builds, requires a
fresh raw-profile delta and exact behavioral oracle per lane, and performs a
real merge before export. Items 5 and 6 remain separate evidence unless their
platform and instrumentation contracts are demonstrably compatible.

### 10.2 Gates

Do not hard-code 78.99% as the first threshold: it is a default-feature main
snapshot and is not comparable with the desired all-feature merged run.

After establishing a reproducible merge-base and PR-head baseline:

- require at least 80% changed-line coverage across ordinary production code;
- require at least 90% changed-line coverage in lifecycle, protocol parsing,
  authorization, persistence arbitration, updater trust, and privacy logic;
- start critical-crate line floors at the reproducible baseline rounded down,
  generally 85–90%, then ratchet upward;
- fail any global or critical-module regression beyond a narrow rounding
  tolerance;
- annotate uncovered changed lines in the pull request;
- require an owner, reason, and expiry for coverage waivers;
- exclude only generated code, fixtures, and genuinely unreachable glue—not
  parsers, reducers, render logic, process code, or `main` merely because they
  are low.

Line coverage is a backstop. Critical behavior also needs a manifest entry and
an independent oracle. Branch coverage from cargo-llvm-cov is currently
unstable/nightly; gather it as nonblocking exploratory evidence until the tool
chain is suitable for a required gate. Region coverage is useful diagnostic
evidence.

Reference: [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov).

## 11. Flake and failure policy

This branch adopts pinned cargo-nextest 0.9.137 for workspace execution, JUnit,
retained attempt output, one diagnostic retry, and inherited-handle leak
detection. The checked-in profile, command-line override, producer exit, JUnit
content, and zero-test count are validated independently. The profile marks a
handle still open after 500 ms as failed; the command forces leak status to be
visible and rejects local per-test leak-timeout overrides. Controlled
fail-then-pass and inherited-handle experiments both returned nonzero and
retained every attempt in JUnit. Doctests remain a separate required Cargo
command.

The policy immediately exposed `TC-HARNESS-006`: the updater
self-replacement test reported `LEAK` after 0.919 seconds in one workspace
run, disappeared on the next, then reproduced as `LKFAIL` followed by `PASS`
under the exact required wrapper. The final run remained failed. The runner
therefore encodes the behavior mechanically while leaving the updater and its
test unchanged.

Policy:

- one automatic retry may collect a second evidence set;
- pass-after-fail is reported as flaky and fails the required check;
- an inherited subprocess handle open for at least 500 ms fails its attempt,
  and pass-after-leak remains a failing flaky result;
- no blanket retry of an entire job;
- every failure captures stdout/stderr, seed, test order, temp root, active
  processes, open ports where available, server log, mpv IPC log, transcript,
  and final telemetry;
- quarantines require an owner, issue, narrow selector, and expiry;
- a quarantined test is not counted as proof for a behavior;
- target less than 0.5% unexplained flaky executions over a rolling 200 runs.

The current artifact bundle contains console output, JUnit, and a policy
report. Process, port, seed, temp-root, server, mpv, transcript, and telemetry
capture remains boundary-specific follow-up rather than a claim of the generic
runner.

First deterministic repairs:

- replace file-mtime waiting in TLS rotation with explicit content/fingerprint
  change and injected metadata clock (test clock, production content
  fingerprint, exact-snapshot parsing, and collision regressions implemented);
- replace CLI reconnect sleeps with paused time and protocol barriers
  (implemented for reconnect backoff, STARTTLS response/handshake, initial
  Hello, and retry);
- distinguish real-mpv healthy-progress timeout from hard inactivity;
- use unique loopback ports and isolated config/storage roots;
- preserve live player evidence before cleanup or restart.

References: [cargo-nextest retries and flaky-result policy](https://nexte.st/docs/features/retries/)
and [leak detection](https://nexte.st/docs/features/leak-detection/).

## 12. CI design and budgets

### 12.1 Pull request — required, target 14–16 minutes critical path

| Job | Required work | Budget |
|---|---|---:|
| `rust-linux` | fmt; all-target/all-feature locked Clippy; all-feature locked workspace tests; doctests | 8–10m |
| `lifecycle-contract` | player lifecycle; GUI projection chain; ordered delivery; behavior-manifest verifier | 2–4m cold |
| `gui-semantic` | all 14 scenarios, strict skips/errors, artifact bundle | 1–2m |
| `rust-windows` | all-feature core/workspace behavior; GUI lib/updater/path/link tests | 8–12m |
| `compat-live` | pinned prerequisites; complete require-live matrix | 3–6m |
| `real-mpv-min` | existing four deterministic minimum-mpv cases | 4m |
| `coverage-diff` | all-feature merged Linux profiles and changed-line policy | 6m |
| `artifact-policy` | package scripts; conditional container build/load/smoke; dependency/action policy | 5–10m |

Run jobs in parallel. Use compiler/dependency caching with keys that include
toolchain, lockfile, features, target, and instrumentation mode. Put `--locked`
on every CI/release/container Cargo invocation.

Avoid duplicate work:

- run branch pushes only where no pull request exists, or restrict push to
  `main` and tags;
- keep `pull_request` for branches;
- use concurrency groups keyed by workflow and PR/branch;
- cancel superseded non-release runs;
- never cancel publication or destructive release operations mid-transaction.

The current branch triggers both push and pull-request copies of CI and GUI
release on the same SHA. Removing that duplication funds stronger gates.

### 12.2 Nightly — 30–60 minutes in parallel

- Linux and Windows all-feature matrices;
- strict native on an ephemeral interactive Windows runner or trusted
  merge-queue commit;
- minimum and latest supported mpv;
- high-case property models;
- rotated fuzz targets;
- Loom/Shuttle/Turmoil schedule exploration;
- exploratory branch coverage;
- strict full Python differential matrix;
- selected flake stress;
- Miri and sanitizer shards;
- deterministic load/soak;
- server release verification.

### 12.3 Weekly — no more than two hours wall clock in parallel

- mutation shards;
- long fuzzing and chaos;
- package/update/install/rollback system tests;
- container scan, SBOM, provenance, and runtime verification;
- performance trend on stable hardware;
- dependency/license/source policy;
- long native accessibility/locale/DPI matrix.

### 12.4 Release

Release must consume or reproduce an immutable green commit artifact:

1. verify source SHA and clean, locked dependency graph;
2. build once;
3. independently extract/load and inspect;
4. launch and execute protocol/player/update smoke;
5. verify version, channel, manifests, checksums, labels, non-root identity,
   writable/read-only paths, and rollback;
6. generate SBOM/provenance/signature;
7. publish the exact tested archive/digest;
8. query the public release/registry and compare digest/checksum.

For the server container, replace build-and-push with build-and-load, run
non-root TLS/protocol/persistence-restart smoke, scan, then push the same
content-addressed digest.

## 13. Concrete implementation backlog

### Tranche A — merge protection

1. Update [rust-ci.yml](../.github/workflows/rust-ci.yml) with:
   - `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
   - `cargo test --workspace --all-features --locked`;
   - the focused lifecycle and semantic commands from section 2.3;
   - `cargo test --workspace --all-features --locked` on Windows, or explicit
     core and GUI behavior shards if measured runtime requires a split;
   - concurrency/cancellation and nonduplicated push/PR triggers.
2. Add global require-live compatibility mode and run the full matrix.
3. Add the two tests from
   [player-lifecycle-followups.md](player-lifecycle-followups.md).
4. Repair GUI projection availability and attempt-union comparisons.
5. Make native required skips and unexpected errors fail; use loopback config.
6. Add `coverage/behaviors.toml`, evidence schema, and initial lifecycle IDs.
7. Re-run and generate report evidence at the exact merge SHA.

Acceptance:

- every documented lifecycle proof is visible as a required check or explicitly
  assigned to nightly/release;
- no required proof reports ignored/skipped;
- same-SHA duplicate CI runs are gone;
- current focused commands remain green;
- report wording matches the actual boundary.

### Tranche B — deterministic time and coverage

1. Introduce clock/timer/barrier seams in CLI reconnect and TLS rotation.
2. Configure nextest/JUnit/failure artifacts and fail-on-flaky/leak retry.
3. Establish all-feature merged coverage on merge base and PR head.
4. Enable changed-line and critical-module ratchets.
5. Classify every ignored test (25 at the audit baseline; 23 after retiring
   two fixed compatibility quarantines).
6. Move fixture capture to explicit maintenance commands.

Acceptance:

- high-risk reconnect behavior uses no arbitrary sleep for logical progress;
- TLS rotation evidence never polls for a filesystem timestamp transition;
- coverage comments distinguish uninstrumented lanes from uncovered code;
- no ignored test lacks tier, owner, and reason;
- reruns cannot silently turn red into green.

Branch progress: item 1 is implemented for the CLI reconnect, connection-phase
deadlines, and server TLS rotation test boundaries; items 2, 4, and 5 are
implemented. The broader CLI timer inventory and production TLS content
fingerprint remain. The exact 23-test ignored registry is workflow-bound.
Pinned nextest performs one evidence-producing retry but fails the required
gate on pass-after-fail or pass-after-leak, rejects empty JUnit, and retains
console/JUnit/policy artifacts. Its 500 ms leak contract exposed the
intermittent `TC-HARNESS-006` updater-test handle leak without changing the
updater or test. Coverage now has locked all-feature head profiles, pinned LLVM
JSON and native source-view exports, per-file source digests, strict
base/head/source binding, independent 80% ordinary and 90% critical
production-line gates, and hard failure for executable-looking lines
omitted by the Linux map. LLVM's aggregate line-instance summary remains
separate from the unique physical-line policy denominator. Critical classification uses
the validated union of the immutable base and head policy blobs, so the change
being measured cannot delete its own 90% rule; both digests and per-rule origins
are retained. Base choice is mechanically event-aware: one PR
merge base, exact nonzero branch or updated-tag `before`, one default-branch
merge base for new tags only, and an explicit manual base. Raw inputs,
requested/effective commits, merge bases, policy digest/rule provenance, and
each failed or passed phase are retained in JSON. The complete semantic
inventory and exact 20-test strict live-reference compatibility inventory are
merged with workspace profiles through a separately validated lane report.
Interactive native Windows and other OS-specific execution remain separate.
Test-only paths and complete inline
`#[cfg(test)]` modules are
reported outside the production denominator; the inline scanner masks Rust
comments and literals and fails closed on ambiguous or unclosed module bodies.
Contradictory LCOV summaries are now retained as a typed diagnostic model while
unique `DA` source lines drive the optional LCOV replay; malformed records and
missing executable mappings still fail. The required gate continues to consume
a source-bound canonical map attested to both native producer views. The fresh
producer, artifact hashes, line-model delta, adversarial inventory, and
six-phase proof are retained in
[`llvm-native-line-map-20260728.md`](evidence/test-coverage/llvm-native-line-map-20260728.md).
Deterministic clock seams now cover the first CLI connection-phase and server
TLS-rotation boundaries. Exact subprocess fixtures now cover managed
kill/wait/reap and IPC cleanup, already-exited cleanup, unmanaged lifetime
handoff, early-exit status, spawn diagnostics, and stdout/stderr containment.
They exposed open early-exit-deadline and inherited-stdin defects. Broader CLI
and persistence scheduling, OS hard-kill/permission faults, successful
managed-IPC attachment without real mpv, and native GUI timing remain.

### Tranche C — property, parser, and persistence faults

1. Add lifecycle and reconnect reference models with persisted shrink cases.
2. Add protocol, IPC, INI, transcript, and media parser fuzz targets.
3. Add framed IPC split/reorder/drop/disconnect harness.
4. Add persistence/config failpoints and restart assertions.
5. Add generated privacy taint tests.

Acceptance:

- every lifecycle input variant is generatable;
- minimized failures replay as normal tests;
- framed faults reach production decoding/worker entrypoints;
- every durability boundary proves old-or-new complete state;
- generated secrets never enter diagnostic artifacts.

Branch progress: reducer-input Proptest, exhaustive stale-epoch coverage, a
shrinkable reconnect restore reference model, shrinkable protocol byte/JSON/
supported-envelope/duplicate-composite properties, protocol order
permutations, split/coalesced/invalid IPC framing through the production
command worker, and a request-reactive duplex IPC model are implemented. The
duplex model exhausts 343 three-command histories over split/coalesced
success, recoverable rejection, stale duplicate, future reorder, read
half-close, and write disconnect, then separately proves gated delayed
ordering and a withheld-response terminal deadline. Corrupt quota-secret
preservation, a deterministic concurrent-secret schedule, a SQLite migration
failpoint, and 15 child-process persistence interruption points with
integrity-checked idempotent reopen are also implemented. Generated transcript
and `PlayerError` taint corpora cover
hundreds of nested, escaped, encoded, and round-tripped cases; all three
redaction families they found are now positive regressions backed by one
shared classification policy. The reconnect acknowledgement-fencing and
nested-`Set` execution-order defects are now positive regressions. A second
post-emission reconnect model and the real Windows named-pipe
fragmentation/correlation/disconnect matrix are implemented. This is not yet
coverage-guided fuzzing, generic client/server raw socket-byte framing, a
transport-level reconnect acknowledgement model, a Unix-domain socket
equivalent, filesystem/power-loss fault injection, or a durability contract
for actor intent that has not entered a transaction.

### Tranche D — real system and deep analysis

1. Build GUI + server + real-mpv + faulting-HTTP loopback harness.
2. Add targeted mutation gates for critical pure modules.
3. Add bounded concurrency models.
4. Add Miri/sanitizer/API compatibility lanes.
5. Add immutable package/container consumer tests.
6. Add nightly soak and weekly chaos/performance trends.

Item 2 is now partially implemented: weekly bounded privacy, server
controlled-room authorization, and protocol codec/redaction shards have 100%
viable kill ratchets and fail-closed evidence. Protocol transport/session
state, persistence arbitration, lifecycle, and configuration decision shards
remain outstanding. Item 5 is implemented for the standalone server archive:
the package contains a source-SHA-bound manifest with an exact payload
inventory, and the release workflow safely extracts, verifies, and executes
the exact archive and optional symbols bundle before uploading those same
files. The consumer rejects checksum drift, unsafe or colliding paths,
links/special files, decompression bounds, inventory/schema/source drift, and
unexpected upload-directory contents; then it requires the extracted binary's
version and a loopback protocol Hello. The Windows GUI archive now has the
same closed consumption plus cross-bound external/install manifests, isolated
visible-window launch, installed-updater self-replacement, real rollback, and
publisher-side reconsumption. Server containers, SBOM/signature verification,
and post-publication digest comparison remain.
The four-stream implementation, stress results, surfaced defects, and
clean-commit package digests are retained in
[`parallel-boundary-slice-20260730.md`](evidence/test-coverage/parallel-boundary-slice-20260730.md).
The GUI archive threat model, adversarial matrix, exact-byte runtime proof, and
surfaced updater defect are retained in
[`gui-release-artifact-20260730.md`](evidence/test-coverage/gui-release-artifact-20260730.md).
The subsequent reconnect acknowledgement, TLS snapshot, updater recovery, and
real Windows named-pipe matrices, including `TC-SERVER-004` and the resolved
`TC-HARNESS-015`, are retained in
[`deep-boundary-slice-20260730.md`](evidence/test-coverage/deep-boundary-slice-20260730.md).

Acceptance:

- at least four required vertical player scenarios cross the real process and
  native UI boundaries;
- no new critical mutant survives;
- concurrency failures replay by schedule;
- published bytes have already passed consumer tests;
- resource-bound failures are mechanical, not visual judgments.

## 14. Metrics that matter

Publish a monthly assurance scorecard:

| Metric | Initial target |
|---|---:|
| Critical behavior IDs with every required proof | 100% |
| Required tests skipped/ignored | 0 |
| Changed-line coverage, ordinary code | >=80% |
| Changed-line coverage, critical code | >=90% |
| New surviving critical mutants | 0 |
| Unexplained flake rate over 200 runs | <0.5% |
| Fuzz crashes without normal regression | 0 |
| Property failures without persisted minimized case | 0 |
| Required native contracts skipped | 0 |
| Unexpected outbound network in deterministic suites | 0 |
| Published artifacts not tested by digest/checksum | 0 |
| Expired waivers/quarantines | 0 |

Do not use total test count as a target. Do not reward duplicate example tests.
Track behavior, boundary, oracle independence, mutation strength, and escaped
defect class.

## 15. Anti-goals and guardrails

- Do not replace readable acceptance traces with only generated tests.
- Do not use a global coverage percentage as proof of lifecycle correctness.
- Do not retry flakes until green and call the run successful.
- Do not make external YouTube, public DNS, or rate-limited services required
  gates.
- Do not snapshot every GUI pixel; keep visual baselines few and intentional.
- Do not port the entire async application to Loom.
- Do not run whole-workspace mutation testing before establishing a stable
  baseline.
- Do not add test-only behavior branches to production state machines.
- Do not let an unavailable prerequisite return success in a required lane.
- Do not publish an artifact that differs from the one actually tested.
- Do not let a narrative report be the only place a critical invariant exists.

## 16. Recommended report corrections

Update the lifecycle documents when the first tranche lands:

1. say “in-process projection-chain” instead of “full-stack” for the current
   direct-application harness;
2. replace “every partition” with the exact strategies currently exercised, or
   add exhaustive short-history partition enumeration;
3. add the included/excluded boundary diagram from this document;
4. describe both generated-history mechanisms and their finite seed/action
   scope;
5. include the executable availability matrix and effect ledger;
6. generate counts and commands from `evidence.json`;
7. link every invariant to stable behavior IDs and exact required checks;
8. label synthetic and real boundary traces separately;
9. keep admitted follow-up debt visible until executable tests close it;
10. record exact SHA, OS, features, mpv/Python/tool versions, durations, and
    skip/ignored counts.

## 17. Final position

The current lifecycle suite is a strong deterministic core and should remain
fast, readable, and required. Its weakness is not that it has too few tests; it
is that several important conclusions extend beyond the boundaries actually
crossed, and CI does not enforce all the evidence the report records.

This tranche makes current proof materially stricter, adds the shrinkable
lifecycle model and deterministic boundary faults, and encodes changed-line
ratchets. It also demonstrates why gates must first run in fail-closed
diagnostic mode: strict native evidence, nextest, PowerShell, ordinary
workspace, and LCOV replay exposed real red states that permissive or
retry-only execution would have hidden. Previously fixed product defects have
positive regressions and TC-PLAYER-001 uses the selected
exclusive-successor rule. The later remediation resolves `TC-PLAYER-003`,
`TC-COMPAT-001` through `TC-COMPAT-007`, and the associated harness/oracle
defects at their owning boundaries. The strict live-reference inventory is
required, exact, and green; no defect quarantine or equivalence exception
turns those failures green.

The most valuable remaining next steps are:

1. extend the proven workspace + semantic + strict-live-reference merge to compatible
   updater/process and OS-specific profiles; keep interactive native Windows
   separate until a trustworthy runner exists;
2. promote the locally proven strict native inventory to an ephemeral,
   interactive Windows required lane and retain its zero-stderr policy;
3. implement immutable versioned TLS bundle publication for `TC-SERVER-004`,
   then extend the proven persistence crash boundary into pre-transaction
   arbitration and filesystem faults while expanding deterministic
   clock/schedule control into process supervision;
4. add coverage-guided parser fuzzing and mutation scoring for the critical
   behavior catalog;
5. add one genuine native GUI-to-real-mpv vertical harness with isolated
   configuration and complete failure artifacts;
6. extend the proven server and GUI archive consumers to the server container,
   then verify the public release/registry digest is the same tested content
   and add SBOM/signature policy.

That combination encodes behavior mechanically, searches the failure spaces
that produced the post-report regressions, and makes future verification
reports generated evidence rather than historical prose.
