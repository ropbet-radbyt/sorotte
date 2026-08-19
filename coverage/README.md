# Behavior verification

`behaviors.toml` is the source of truth for the behavior claims that Sorotte
currently treats as merge contracts. It intentionally records behavior and
proof identity, not coverage percentages or arbitrary shell commands.

The catalog is enforced by `scripts/behavior_evidence.py`:

```text
python -m pip install -r requirements/ci-policy.txt
python scripts/behavior_evidence.py validate --catalog coverage/behaviors.toml
python scripts/ignored_test_policy.py validate --registry coverage/ignored-tests.toml
python scripts/known_defect_policy.py validate \
  --registry coverage/known-defects.toml \
  --catalog coverage/behaviors.toml
python -m unittest discover -s scripts/tests -p "test_*.py" -v
```

The CI workflow runs two evidence lanes:

- `lifecycle-contract` discovers and executes each Rust proof as one exact,
  non-ignored libtest.
- `gui-semantic` compares the live scenario inventory with the catalog and
  executes all 14 scenarios, including those not yet promoted to named
  behavior proofs.

Each lane writes a shard even when a proof fails, and continues through the
remaining proofs. The `verification-required` job rejects a missing or
duplicate lane, a shard from another workflow run or Git revision, a future or
invalid attempt, a different repository or catalog digest, a skipped/ignored
proof, an incomplete semantic inventory, or any failed dependency job. A
successful shard from an earlier attempt of the same workflow run is accepted
so GitHub's “rerun failed jobs” flow remains usable. The runner verifies both
before and after proof execution that the supplied evidence SHA is the
checked-out Git `HEAD`, and rejects tracked or untracked source changes.
Ignored build/evidence directories remain permitted. Catalog entries cannot
supply commands or environment assignments.

For a local lifecycle evidence run on an operating system declared by the
lane, supply stable local metadata. The SHA must equal `git rev-parse HEAD`;
the worktree must be clean, and the initial catalog lanes are Linux-only:

```text
python scripts/behavior_evidence.py run-lane \
  --catalog coverage/behaviors.toml \
  --lane lifecycle-contract \
  --sha <40-character-git-sha> \
  --repository local/sorotte \
  --run-id local \
  --run-attempt 1 \
  --os linux \
  --output target/verification/evidence.lifecycle-contract.json
```

The shrinkable lifecycle suite fuzzes the reducer input contract with 128
Proptest cases of up to 64 transitions by default; it does not claim that every
generated ordering is adapter-reachable. The client-core reconnect suite uses
the same budget for state-aware schedules of retry, Hello, initial server
playlist authority, and transition/state/playlist drains. It compares every
executed step with a small independent model and always completes each history
through two final drains so liveness and at-most-once behavior are observed.
Set `PROPTEST_CASES=2048` for the nightly-depth budget. Every generated
lifecycle transition passes through the ordinary invariant-checking reducer
without a known-defect classifier.
`TC-PLAYER-001` is represented by two positive regressions proving exclusive
successor selection for external-observation and load-acceptance conflicts.
The former `TC-PLAYER-002` histories are also ordinary positive regressions
proving reactivation clears stale logical-terminal state.

Proptest seeds under each participating crate's `proptest-regressions/`
directory are source-file and strategy-shape scoped. They improve replay while
a strategy remains stable, while named deterministic regressions remain the
durable behavior contract. The seven 2026-07-30 reconnect, protocol,
media-tool process, Plex part-selection, and Plex retry characterizations were
converted to ordinary positive regressions after their production fixes
landed. The later raw-loopback framing slice registered `TC-CLI-003` after
proving that a future-local partial frame was lost when another
connected-session `select!` branch cancelled its read before CRLF. That state
now belongs to the session and both forced-cancellation cases are positive.
The coverage-guided protocol parser lane subsequently registered
`TC-PROTOCOL-004` for an adjacent floating-point representation change across
raw and typed decode/encode/decode. serde_json's `float_roundtrip` parser
feature now preserves both minimized cases exactly, and the fuzz target no
longer has a one-ULP allowance. The current registry is explicitly empty; all
four former characterizations are ordinary positive regressions.
Final validation independently exposed `TC-HARNESS-016`: the updater
process-interruption parent could observe its marker file before the child had
written the boundary label. The child now atomically renames a flushed,
synced, complete pending marker and the parent acknowledges only the exact
expected payload. The process regression passed 100/100 serial replays. This
resolved harness defect remains outside `known-defects.toml`, which is
reserved for exact product expected-failure characterizations.
`TC-SERVER-004` is now resolved by immutable authenticated TLS generations and
an atomic selector, with a documented double-capture compatibility fallback
for static loose files. The reconnect acknowledgement fence and TLS max-mtime
collision are also positive regressions, as are `TC-PLAYER-003` and
`TC-COMPAT-001` through `TC-COMPAT-007`.
The 2026-07-31 continuation added complete required-live compatibility
accounting, real-mpv owned-process recovery, updater parent-directory sync,
and coverage-guided framed-mpv testing. It exposed and fixed
`TC-UPDATER-002`, the missing updater directory-entry durability boundary.
The two randomized/committed-run REDs were test-oracle or wrapper defects and
were corrected without changing compatible product behavior. The product
registry remains explicitly empty.
The next bounded continuation adds generated Rust/Python framing differential
coverage, a real Linux updater directory-sync denial, real Unix-domain-socket
mpv IPC schedules, and a faulting-loopback-HTTP real-mpv vertical. The first
three slices found no product defect. The native HTTP slice exposed and fixes
two GUI media-resolution defects plus the player recovery gap where
`keep-open=always` publishes a premature `eof-reached` without `end-file`.
All three are ordinary positive regressions, so the current product registry
remains explicitly empty.
The current 2026-07-31 four-slice tranche adds deterministic client
jitter/drift/playback schedules, a required real-ffmpeg/ffprobe generated Media
Match lane, a 256-case legacy CLI parser/configuration-composition oracle, and
a fourth native real-mpv mode for a valid complete-length response that remains
byte-silent without EOF. `TC-CLI-004`, `TC-CLI-005`, and `TC-PLAYER-005` are
positive; all local validation and the final implementation-source four-mode
real-mpv campaigns are green, with final post-build stalled bundle
`20260731T150829535Z-48288` run last. The first
hosted run exposed `TC-HARNESS-018` through `TC-HARNESS-024`; the second proved
generated Media Match, complete live compatibility, semantic, lifecycle,
Ubuntu server-release, and Windows nextest behavior before exposing
`TC-HARNESS-025` through `TC-HARNESS-029`. Later fail-closed campaigns exposed
`TC-HARNESS-030` through `TC-HARNESS-046`: platform reachability, exact
inventory, bounded fixture timing, legacy checkout/frame/port coordination,
one context-exact pinned-legacy setter alternate, LLVM exa-scale parsing,
Linux prerequisite installation, one noncanonical native-ASan environment
limitation, strict-count updates, two-platform physical-line coverage, and
cross-platform source-byte identity, coverage-finalizer union binding,
real-mpv fault arming, and complete Plex fixture headers. All 29
hosted-continuation findings (`TC-HARNESS-018` through `TC-HARNESS-046`) have
focused dispositions and positive regressions or exact-artifact replay. The
committed implementation-head compatibility and WSL fuzz campaigns, 54-test
Windows process map, and local two-platform coverage replay are green.
Exact implementation-head workflow `30639113884` also passed every required
producer, the corrected two-map coverage finalizer, and the aggregate.
Documentation-inclusive workflow `30679354953` subsequently finished green at
exact workflow-bearing head
`612917ac8461040549217453bdebfc5001f2378c`. Its first attempt retained one
Windows server-release Python playlist-observation timeout; GitHub's
failed-job rerun repeated the complete strict job successfully without a
source change. The final suite contains 16 successful checks, the expected
schedule-only skip, zero annotations, and nine nonexpired evidence artifacts.
Detailed records are
[`client-ping-jitter-drift-schedules-20260731.md`](../docs/evidence/test-coverage/client-ping-jitter-drift-schedules-20260731.md),
[`media-match-generated-media-capability-20260731.md`](../docs/evidence/test-coverage/media-match-generated-media-capability-20260731.md),
[`cli-argument-configuration-composition-20260731.md`](../docs/evidence/test-coverage/cli-argument-configuration-composition-20260731.md),
[`native-gui-real-mpv-stalled-http-recovery-20260731.md`](../docs/evidence/test-coverage/native-gui-real-mpv-stalled-http-recovery-20260731.md),
and the integrated
[`next-four-test-slices-20260731.md`](../docs/evidence/test-coverage/next-four-test-slices-20260731.md)
ledger. The hosted closure, retained failed attempt, CI timing checkpoints,
Node 24 action pins, and unchanged external limits are recorded in
[`hosted-ci-closure-20260801.md`](../docs/evidence/test-coverage/hosted-ci-closure-20260801.md).

A 2026-08-01 Pro review reopened the CLI proof at the grammar boundary: the
original generated oracle exercised `-x=value`, but not canonical
short-attached `-xVALUE` or flag clusters. The correction is differentially
bound to the actual pinned Python `ConfigurationGetter`, adds actual-process
privacy and pre-side-effect endpoint tests, bounds loose symlink-following TLS
reads, removes the reusable production one-shot protocol reader, and narrows
room-persistence latest-wins wording to an eventual live-service guarantee.
The complete disposition and validation boundary are recorded in
[`pro-review-remediation-20260801.md`](../docs/evidence/test-coverage/pro-review-remediation-20260801.md).

The pre-optimization full matrix spent 33m30 executing after its queue wait.
Parallel Windows producers and an independent Linux coverage producer reduced
the first complete parallel run to 19m33 while preserving the public
aggregates and exact artifacts. That run exposed the Windows strict server
verifier as a 19m28 outlier. CI now uses `-NoWorkspace` only to omit the
workspace test already executed by the required all-feature workers; the
successful exact-head Windows verifier rerun took 10m49. The first-party
checkout, Python, upload, and download actions are full-SHA-pinned to Node 24
majors, and the accepted run emitted no runtime-deprecation annotation.

`NET-DEADLINE-001` makes the first deterministic CLI clock slice required
evidence. Paused Tokio time proves the exact 100/200/400 ms reconnect schedule
and that exhaustion adds no terminal delay. Explicit loopback protocol
barriers separately prove that the STARTTLS response and TLS handshake phases
each receive their full configured deadline. A third barrier proves the
initial server Hello deadline starts after the client Hello is written and
expires exactly when configured. A paused-time real-socket proof then
exercises timeout, reconnect, exhaustion, and the rule that Hello and
credentials cannot be sent while required STARTTLS is unresolved. Exact time
is asserted only after a protocol barrier; operating-system loopback delivery
is not treated as a virtual-clock oracle.

`NET-TLS-001` removes filesystem timestamp waiting from TLS rotation evidence.
A test-only metadata revision clock drives 243 exhaustive five-step histories
through cached corruption, invalid rotation, and valid rotation. Each response,
transport action, context state, acceptability gate, and retry count is checked
against an independent model. Real-network proofs retain the captured context
for an already accepted handshake, deny later clients after invalid rotation,
and recover after a valid pre-cap restoration. Retry exhaustion remains an
explicit terminal legacy contract. The former `TC-SERVER-003` collision is now
a positive production-filesystem proof: all three members contribute to a
content fingerprint and rustls parses the exact captured snapshot.

`SRV-PERSIST-001` makes process interruption an executable persistence
contract. A dedicated child test process terminates without Rust destructors at
15 production transactional boundaries: five schema steps, two playlist-row
migration steps, four room save/delete commit steps, two stats snapshot steps,
and two quota-secret creation steps. The parent process reopens the actual
SQLite file after every interruption, requires `PRAGMA integrity_check` to
return `ok`, distinguishes the exact pre-commit and post-commit state, and
proves a second recovery pass is idempotent. Crash-point environment variables
are honored only by the exact helper test in the child process, so parallel
tests cannot arm a global in-process failpoint.

`scripts/tests/test_ci_policy.py` mechanically binds the aggregate's required
job names to the locked all-feature, semantic, compatibility, real-mpv,
Windows, release-build, and evidence commands in the workflow. Repository
review policy is still required to protect that test and workflow from
coordinated weakening.

`ignored-tests.toml` must exactly match every Rust `#[ignore = "reason"]`
attribute under `crates/`. Each entry has an owner, prerequisites, supported
operating systems, and one of four supported dispositions: required
pull-request CI, manual capability, fixture maintenance, or expiring
quarantine. The schema retains all four dispositions, but the current 23-test
registry has no quarantines. Unsupported conditional or reasonless ignore attributes fail.
Pull-request entries are additionally checked against exact
`--ignored --exact` workflow invocations.

`known-defects.toml` is the schema-validated inventory for any undesirable
behavior intentionally represented by a Rust expected-failure
characterization. The validator exactly matches every Rust `known_defect_*`
`should_panic(expected = "...")` characterization to its source, package,
selector, panic oracle, owner, finding, and expiry. A missing or stale entry,
bare `should_panic`, malformed or unterminated multiline attribute, expired
defect, duplicate finding identifier, drifted heading/title or panic oracle,
or selector also listed as a positive behavior proof fails CI. Passing because a
characterization panicked is therefore never presented as proof that the
application behaves correctly. Once a defect is fixed, its characterizations
must become positive regressions and the corresponding registry entry must be
removed.

Required workspace execution uses pinned cargo-nextest 0.9.137 through
`scripts/nextest_ci.py`. The checked profile allows one diagnostic retry but
fails the gate when a failed or leaked first attempt later passes. An inherited
subprocess handle still open after 500 ms is a failed result. The wrapper
rejects a drifted binary version or profile, per-test leak-timeout overrides,
empty or malformed JUnit, failed/rerun/flaky attempt elements, and a nonzero
producer. Console, JUnit, and machine-readable policy evidence are always
uploaded; doctests run as a separate required Cargo command.

That contract exposed an intermittent inherited-handle leak in
`sorotte-gui::updater_self_replacement_windows` test
`running_installed_updater_recovers_interrupted_replacement_and_restarts`.
One run reported `LEAK` after 0.919 seconds, a clean rerun did not reproduce
it, and a later checked run failed even though its retry passed. The updater
and test remain unchanged under `TC-HARNESS-006`; the real and controlled
inherited-handle evidence is retained in
[`docs/evidence/test-coverage/nextest-flake-leak-20260728.md`](../docs/evidence/test-coverage/nextest-flake-leak-20260728.md).

CI generates locked all-feature instrumentation profiles on the exact verified
head, then exports two native views from the same profiles:

- LLVM JSON, with functions omitted, attests the pinned producer, export
  schema, file identities, segments, and aggregate summaries.
- `llvm-cov show` text supplies the exact execution state of each physical
  source line.

`scripts/llvm_cov_line_map.py` accepts only cargo-llvm-cov 0.8.4 and LLVM
coverage JSON 3.1.0, requires the workspace manifest, rejects unknown fields
and text rows, and compares every displayed source row with the checkout. Its
canonical artifact hashes both producer views and every represented source
file. LLVM's aggregate line-instance summary is retained separately from the
unique physical-line map; disagreement is explicit diagnostic evidence, not a
value to normalize.

Base resolution is event-aware and fail-closed:

- pull requests use exactly one merge base between the PR base tip and head;
- branch pushes and updated-tag pushes use the exact nonzero event `before`
  commit;
- newly created tags use exactly one merge base against the fetched remote
  default branch only when event `before` is all zeroes;
- manual runs require an explicit full base commit SHA.

The JSON evidence preserves raw event inputs, ref type, default-branch
name/ref/SHA when used, requested base, effective base, and every merge base.
An always-run finalizer records six independently named Linux phases—base
resolution, profile generation, JSON export, native text export, line-map
conversion, and diff policy—even when an earlier phase fails. It hashes the
primary JSON, text, and line-map artifacts and checks their exact producer
chain. The diff report separately records the exact digest and producer
metadata for every supplied platform map.

The policy requires 80% coverage over ordinary changed production lines and
exactly 90% over paths in `diff-coverage-policy.toml`. Its 20 non-overlapping
rules cover lifecycle, protocol parsing, authorization, persistence
arbitration, updater trust, and privacy. Rules must name existing production
files or directories; globs, test-only targets, overlaps, missing targets,
threshold changes, and configurable exclusions fail. A rename into a critical
path materializes the complete target, while a rename out retains its critical
classification through the old path. Base/head runs load each revision's
policy blob from Git, validate it against that same immutable tree, and classify
with the non-overlapping union. Deleting a critical rule in the same change
therefore cannot lower its code from 90% to 80%; exact duplicate rules are
deduplicated and cross-revision overlaps fail closed. Explicit `--diff` mode
rejects a patch that changes the policy because it has no trusted base policy.
Ordinary and critical results, both policy digests, rule policy origins, the
matching rule, and the path match origin are reported independently.

Conventional test/benchmark/example paths and exact repository-owned native,
semantic, startup-benchmark, and fuzz harness entry points are reported but
excluded from the denominator. Complete attached items under exact
test/test-support/fuzz-support cfg attributes in production files are also
reported separately and excluded. The scanner masks comments and Rust
literals, tracks delimiters, and fails closed on ambiguous or unclosed items;
unrelated platform cfg items remain production scope. Comments, attributes,
imports, signatures, compile-time declarations, structural expression/pattern
glue, and punctuation are non-coverable only when the conservative lexical
classifier can prove a complete structural form.

Pull-request production and policy are separate fail-closed jobs. The
`coverage_linux` job checks out the exact verification head, runs the complete
merged-profile producer, exports both pinned LLVM views, builds the canonical
line map, and uploads `verification-linux-merged-coverage`.
`rust_windows_coverage` independently creates and uploads the exact-head
Windows map. Only after both producers succeed does `coverage_diff` download
their artifacts, resolve the event-specific base, run the two-map policy, and
revalidate every retained producer artifact through the
`coverage_ci_guard.py finalize` command. The finalizer's profile, LLVM JSON,
LLVM text, and line-map outcomes
are bound to the upstream Linux producer result rather than synthetic success
values. The public `rust-windows` result is likewise a small always-run
aggregate over independent Windows test, release, and coverage workers. This
keeps the required check names stable while removing the former Windows then
Linux-coverage serial chain.

The required gate consumes the union of the broad Linux merged-profile map and
an exact-head targeted Windows process map. Each map is source-bound and
validated independently; duplicate content is rejected, and identical
physical source lines are combined once using their maximum binary hit value.
The Windows producer is a closed 54-test inventory covering updater
transactions/self-replacement, named-pipe and external-mpv processes, and
media-tool process faults. Rust source is forced to LF through `.gitattributes`
so raw source digests are identical on Windows and Linux. Executable-looking
changed lines missing from the combined canonical maps remain unmapped and
fail, so neither platform silently excuses the other's production body.
The coverage evidence finalizer consumes that same complete ordered
primary-plus-supplemental tuple and requires the retained diff report to bind
every exact map in order. Missing, reordered, duplicated, replaced, or
tampered maps fail closed; `TC-HARNESS-044` preserves the hosted single-map
coverage-finalization RED and the successful exact-artifact multi-map replay.
Implementation-head workflow `30639113884` then regenerated both maps, passed
the finalizer, and accepted 2,403/2,894 combined lines (83.03%), including
1,841/2,275 ordinary (80.92%) and 562/619 critical (90.79%), with zero
unmapped lines; its required aggregate also passed.
Exact local replay and artifact identities are retained in
[`platform-coverage-map-union-20260731.md`](../docs/evidence/test-coverage/platform-coverage-map-union-20260731.md).

`scripts/diff_coverage.py --lcov` remains a diagnostic
compatibility mode. It now declares `unique-da-source-lines` as the only
changed-line model and retains contradictory `LF`/`LH` summaries as a separate
structured audit. Malformed or duplicate `DA`, impossible summaries, stale
records, and missing executable mappings still fail closed. `TC-HARNESS-005`
is therefore resolved for Sorotte's consumer without rewriting the
contradictory producer artifact or choosing a favorable aggregate. The
required gate continues to use the stronger source-bound dual-native contract.

The fresh local producer experiment, exact artifact hashes, adversarial cases,
and six-phase result are retained in
[`llvm-native-line-map-20260728.md`](../docs/evidence/test-coverage/llvm-native-line-map-20260728.md).
The LCOV consumer resolution and current-source cross-audit are retained in
[`lcov-dual-model-20260729.md`](../docs/evidence/test-coverage/lcov-dual-model-20260729.md).

## Merged behavioral coverage profiles

The coverage producer does not stop at workspace unit and integration tests.
`scripts/coverage_profile_lanes.py` collects and attests compatible profiles
from:

- the locked all-feature workspace;
- the exact 14-scenario GUI semantic inventory;
- the complete 21-test strict live-reference inventory against pinned Syncplay
  commit `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`: 12 fanout scenarios, 4 TLS
  probes, 2 live state probes, 2 request-shim contracts, and one resource-lease
  regression;
- a final cargo-llvm-cov merge check.

The wrapper accepts only cargo-llvm-cov 0.8.4, applies its `show-env` contract
to external Cargo processes, isolates those builds in
`target/llvm-cov-target`, removes and attests stale generated raw/merged
profiles before execution, recursively hashes current profiles, requires the
workspace lane to start at zero, and requires a fresh profile delta plus
continuous inventory from every execution lane. Content hashes detect changes
even when size and mtime are unchanged; a lane may not remove prior profiles,
and the merge may not mutate them. The wrapper also validates the semantic
JSON and exact libtest counts, selectors, skip markers, commands, environment,
logs, producer, and pinned reference revision.

The broad live-reference selector passes 21/21 with no ignored cases and 128
filtered out under a fail-closed expected inventory. A historical discovery replay passed 129 tests
and failed six; that red evidence and the subsequent product/harness
remediation are both retained rather than normalized away. Native interactive
Windows profiles remain a separate evidence boundary. Exact experiments and
limits are retained in
[`merged-profile-lanes-20260729.md`](../docs/evidence/test-coverage/merged-profile-lanes-20260729.md).

`scripts/coverage_windows_process_lanes.py` owns a separate
`windows-x86_64-msvc` profile domain. It requires exactly 54 noninteractive
tests across updater transactions, installed-updater self-replacement, mpv
named-pipe and external-process faults, and media-tool child-process faults.
Every lane must add a fresh nonempty profile; all profiles must remain
continuous and merge-compatible; the final merge may not mutate them. Producer
identity, Rust host, source state, commands, inventories, filtered counts,
logs, and the native-UI exclusion are schema-bound. The real experiment
retained 34 profiles in the historical final local replay. Hosted run
`30632931277` then executed the exact 54-test producer and retained 75 fresh
profiles at its exact source checkpoint. The Windows job always uploads its report
and logs, including on failure. Interactive UI Automation remains a separate
uninstrumented contract.

## Protocol parser property and corpus evidence

`crates/sorotte-protocol/tests/protocol_parser_robustness.rs` exercises the
public byte, raw-JSON, line-item, and typed-message entrypoints through ordinary
locked Cargo tests. A fixed-seed Proptest suite covers bounded arbitrary bytes,
arbitrary Unicode, and insert/replace/delete/truncate mutations of 16 checked-in
UTF-8 corpus files. A serde streaming `MapAccess` visitor independently derives
top-level source order and surviving duplicate-key values; it does not call or
copy the production order scanner.

The default run executes 1,536 generated cases. The existing scheduled
`PROPTEST_CASES=2048` depth executes 6,144 cases without another workflow. Both
depths, all 16 corpus entries, 50 consecutive corpus replays, the complete
protocol crate, and strict Clippy passed. The corpus includes the minimized
raw and typed float regressions. This is deterministic parser robustness
evidence, not coverage-guided fuzzing or transport scheduling.
Commands, corpus inventory, oracle boundaries, and results are retained in
[`protocol-property-corpus-20260730.md`](../docs/evidence/test-coverage/protocol-property-corpus-20260730.md).

## Coverage-guided protocol parser evidence

The standalone `fuzz/` package adds a source-bound libFuzzer/AddressSanitizer
target for every public raw, diagnostic, aggregate, singular, typed, and
encoding protocol-line boundary. An independent serde `MapAccess` visitor
checks source order and duplicate-key surviving values. The runner pins
`cargo-fuzz 0.13.2`, `libfuzzer-sys 0.4.13`, and
`nightly-2026-07-29`; caps input at 65,536 bytes, each input at 5 seconds,
RSS at 2,048 MiB, and campaigns at 900 seconds; rejects stale output; and
attests the exact source, seed, command, tools, corpus, artifacts, statistics,
and before/after stability.

The first real campaign found `TC-PROTOCOL-004`: `70E70` changed from
`7.000000000000001e71` to the adjacent
`7.000000000000002e71` across raw and valid typed
decode/encode/decode. Enabling serde_json 1.0.151's `float_roundtrip` feature
corrects both forms. Two positive regressions and two checked-in corpus seeds
retain the minimized inputs. The former narrow continuation classifier was
deleted; every raw and typed roundtrip now requires exact equality.

The first continuation passed 559,788 executions. A fresh 180-second campaign
over committed SHA `729214d0de7ced9c56da7361bda68dc75b831179` passed
1,915,137 executions with no artifact or independent failure under the
historical narrow allowance. A post-fix 180-second campaign over committed
SHA `034e10511ae6473f0165f3028a026a0bad4f6db3` passed 1,994,358 executions
with exact oracles and no artifact. Its 29-file bound-source and 16-file seed
manifests were stable. Pull-request and
`main`-push path filters cover every fixed bound input, and the scheduled
workflow runs the same fail-closed runner for 900 seconds. Exact experiments,
tool identities, hashes, historical classifier, resolution, and limitations
are retained in
[`protocol-coverage-guided-20260730.md`](../docs/evidence/test-coverage/protocol-coverage-guided-20260730.md).
The combined CLI/protocol correction and empty-registry proof are retained in
[`outstanding-defect-resolution-20260730.md`](../docs/evidence/test-coverage/outstanding-defect-resolution-20260730.md).

## Framed transport schedule evidence

Four ordinary CLI tests extend the cancellation-safe line accumulator through
the real application/session boundary. A test-owned reader supplies 82 exact
fragmentation and coalescing schedules; a Tokio duplex cancels at every byte
offset before the first frame's CRLF; and generated EOF schedules cover every
proper Ready-frame truncation plus valid unterminated, lone-CR, and
CRLF-terminated frames. Exact/MAX+1 LF and CRLF seams retain accumulated-length
and framing-CR decisions. An input-derived frame-count bound plus a final EOF
probe fails promptly if a reader does not consume input. Exact line bytes and
final username, room, active phase, and readiness state are required.

The suite passed 4/4, the existing real-loopback framing family passed 5/5,
and the final selector passed 50/50 serial replays after its mutation-driven
liveness and payload-limit additions. This deterministic schedule matrix is
not coverage-guided transport fuzzing. Its oracle, commands, hashes, and limits
are retained in
[`framed-transport-schedules-20260730.md`](../docs/evidence/test-coverage/framed-transport-schedules-20260730.md).

## Coverage-guided framed transport/session evidence

A second source-bound libFuzzer/AddressSanitizer target now drives the exact
production CLI line accumulator and public `ClientApplication` entirely in
memory. An independent byte-framing oracle covers coalesced, bytewise,
fixed-width, and deterministic pseudo-random schedules; one first-frame
cancellation; LF/CRLF, EOF, UTF-8, and MAX/MAX+1 seams; and complete
schedule-independent session projections. Fourteen checked-in seeds cover
every control mode and size seam. The target has no network API.

A fresh 30-second smoke passed 52,492 executions. The clean-source canonical
180-second campaign over commit
`366fe28b18c50ebb5fb66eefae9a3f317ba9e75c` passed 292,528 executions,
retained a stable 881-file source binding and stable 14-file seed binding,
produced zero artifacts or evidence errors, and required no minimization. The
scheduled/manual workflow retains the same fail-closed runner for 900 seconds.
Exact commands, manifests, hashes, tool/resource pins, statistics, and
limitations are retained in
[`framed-session-coverage-guided-20260730.md`](../docs/evidence/test-coverage/framed-session-coverage-guided-20260730.md).

## Coverage-guided framed mpv IPC and transcript evidence

A third source-bound libFuzzer/AddressSanitizer target now drives the
production mpv line reader through an in-memory scripted transport, then
checks queued-event/response ordering, transcript projection, and
attachment/media-generation fencing. Four deterministic chunk schedules cross
five terminal modes. A separate reference decoder retains event duplicates
and response barriers, while 12 checked-in seeds cover success, rejection,
invalid JSON, reordered IDs, duplicate events, dropped responses, timeout,
disconnect, and trailing post-response corruption.

The first campaign preserved an oracle counterexample: partial buffered bytes
must be decoded on EOF, not on a non-EOF read disconnect. The corrected oracle
keeps newline-complete malformed JSON and trailing malformed EOF data as
protocol corruption. The clean committed-source 180-second campaign over
`3cd64ce2e2f0a51a7e31b9862a6bde9cd40c6f16` passed 322,973 executions,
added 3,219 units, retained stable 64-file source and 12-file seed bindings,
and produced zero artifacts or evidence errors. Exact RED/GREEN identities,
commands, hashes, limits, and limitations are retained in
[`framed-mpv-ipc-transcript-coverage-guided-20260731.md`](../docs/evidence/test-coverage/framed-mpv-ipc-transcript-coverage-guided-20260731.md).

## Unix-domain-socket mpv IPC kernel evidence

Nine Linux-only tests now cross the production `MpvJsonIpcClient`, Unix stream
deadlines, line reader, worker thread, response correlation, event queues, and
drop path through real kernel Unix-domain sockets. Fourteen deterministic
schedules cover bytewise fragmentation, coalesced event/response order,
stale/future/duplicate IDs, malformed and truncated frames, EOF, pre-request
disconnect, write-half-close, timeout, same-path replacement, request-ID
wraparound, worker shutdown, and owned fixture cleanup.

The Ubuntu WSL2 focused suite passed 9/9. The complete player crate passed 418
unit tests with its one explicit real-mpv opt-in ignored, followed by both
integration tests; Windows and Ubuntu warning-denied Clippy passed. No product
or harness defect surfaced. This is Linux WSL2 kernel evidence with a synthetic
mpv peer, not macOS/BSD coverage or a substitute for real-mpv lifecycle
testing. Exact schedules, commands, environment, and limitations are retained
in
[`player-unix-socket-ipc-kernel-20260731.md`](../docs/evidence/test-coverage/player-unix-socket-ipc-kernel-20260731.md).

## Configuration composition property evidence

A black-box `sorotte-client-app` integration suite generates all 30
environment-overridable persisted fields and crosses the public INI
upsert/parse, runtime-snapshot, and environment-aware startup-plan boundary.
Its independent model proves exact roundtrip and idempotence, preservation of
unknown INI content, single-field noninterference, and exact suppression of
only the stored field whose environment value is present. The fixed seed is
`0xC0F1_6C0A_2026_0730`; invalid case budgets fail closed.

The scheduled depth passed 6,144 generated cases and the stress depth passed
30,000. No production source changed and no product defect surfaced. Exact
field inventory, case budgets, commands, hashes, and limitations are retained
in
[`configuration-composition-properties-20260730.md`](../docs/evidence/test-coverage/configuration-composition-properties-20260730.md).

## Controlled-room configuration property evidence

A separate black-box client-app suite uses public normalization, command
presentation, INI persistence, runtime resolution, and environment-aware
startup composition boundaries with an independent legacy controlled-room
model. Four fixed-seed properties cover canonical reconstruction and
idempotence, malformed/passwordless inputs, explicit/history precedence,
unrelated-environment noninterference, typed server/room credential isolation,
TLS selection, and `Debug` redaction.

The default, scheduled, and stress depths passed 2,048, 8,192, and 40,000
generated cases respectively. Invalid case budgets fail closed. No production
source changed and no product defect surfaced. Exact grammar, model, commands,
hashes, and limitations are retained in
[`controlled-room-configuration-properties-20260730.md`](../docs/evidence/test-coverage/controlled-room-configuration-properties-20260730.md).

## Configuration migration property evidence

A separate black-box suite begins with legacy INI spellings rather than
canonical DTOs. It covers mixed casing and whitespace, BOM and CRLF,
boolean/language/enum aliases, absent post-legacy start policy, legacy
list/map/server containers, and malformed typed values. Every case crosses
parse, in-place update, fresh canonical rewrite, reparse, and runtime snapshot.
The expected DTO is independently constructed, rewrites must be idempotent,
and a valid sentinel proves malformed values do not discard unrelated state.

The default and scheduled depths passed 1,536 and 6,144 generated cases. No
production source changed and no defect surfaced. The fixed seed, grammar,
oracles, commands, source hash, and limitations are retained in
[`configuration-migration-properties-20260730.md`](../docs/evidence/test-coverage/configuration-migration-properties-20260730.md).

## Updater boundary-marker handshake evidence

The test-only updater child publishes its process-interruption boundary through
a same-directory pending file, complete write, flush, file sync, close, and
rename. The parent requires the exact expected payload while retaining its
premature-exit and timeout checks. A deterministic preflight rejects empty,
partial, and incorrect markers. The exact 11-boundary process regression passed
100/100 serial replays, covering 2,200 recovery subprocesses; the complete
updater binary passed 30/30.

This resolves `TC-HARNESS-016` without changing production updater behavior or
claiming parent-directory sync or power-loss durability. Root cause, commands,
hashes, and limits are retained in
[`updater-boundary-marker-handshake-20260730.md`](../docs/evidence/test-coverage/updater-boundary-marker-handshake-20260730.md).

## Updater transaction storage durability evidence

`TC-UPDATER-002` characterized the updater's missing containing-directory
flush after journal/prepared-file creation, replacement/rename, rollback,
cleanup, and journal removal. Production now synchronizes the parent directory
at each owned directory-entry durability boundary, retaining an authenticated
uncommitted journal on a failed journal-directory sync and retaining committed
cleanup state after the commit record.

A thread-local one-shot fault seam crosses 13 disk-full/access-denied schedules
and requires complete old or complete new bytes, authenticated recovery,
idempotent second recovery, no artifacts, and an untouched sibling sentinel.
A real reversible Windows share denial reaches the directory-sync syscall.
The final updater suite passed 33/33, including all 11 process-termination
boundaries, and the installed-updater integration still passed its two exact
tests. This proves requested OS flush boundaries, not physical power-loss or
device-cache persistence. Exact commands and limitations are retained in
[`updater-transaction-storage-durability-20260731.md`](../docs/evidence/test-coverage/updater-transaction-storage-durability-20260731.md).

## Linux updater parent-directory sync real-syscall evidence

A Linux-only regression applies mode `0300` to a nonce-owned updater target
directory after the first production rename. Write/search permission allows
the rename, while the production read-open and `sync_all()` directory boundary
fails with `EACCES` (`os error 13`). The test requires an authenticated
uncommitted journal, a complete old install, exact unmanaged sentinels,
permission restoration, artifact-free recovery, and an idempotent second
recovery.

The focused Ubuntu WSL2 syscall test passed as UID 1000, the complete updater
binary passed 28/28, and Linux plus Windows warning-denied checks passed. No
product defect surfaced: the test executes the already-fixed Unix
`TC-UPDATER-002` boundary with a real reversible host denial. It does not claim
physical power-loss, device-cache, torn-write, real disk-full, or privileged
behavior. Exact construction and limits are retained in
[`updater-linux-parent-directory-sync-real-syscall-20260731.md`](../docs/evidence/test-coverage/updater-linux-parent-directory-sync-real-syscall-20260731.md).

## Persistence worker fault evidence

Three ordinary server tests now cross the production
`RoomPersistenceService` boundary and fault its actor-owned SQLite connection
immediately before the real transaction. They prove that worker-owned
`SQLITE_FULL` and deterministic `query_only` write denial preserve the raw
eight-column durable row and integrity, retain unresolved desired state,
project degradation once, recover on the same connection, emit exactly one
recovery transition, and survive a normal close/reopen. A separate real VFS
path collision proves startup retains `SQLITE_CANTOPEN`, the action, and the
database path.

The three regressions passed 150/150 focused stress repetitions and the
complete server package. The test-only hook exposes the exact production
connection and effect under `cfg(test)`; it does not introduce a production
failure branch. This is not an NTFS/POSIX ACL, kernel power-loss, torn-sector,
`fsync`, or storage-cache claim. Exact fault construction and raw-state oracles
are retained in
[`persistence-worker-faults-20260730.md`](../docs/evidence/test-coverage/persistence-worker-faults-20260730.md).

## Persistence platform syscall fault evidence

Platform-specific server tests now impose real host-filesystem denial on a
checkpointed production SQLite database. Windows holds a no-share kernel file
handle, independently proves rename/delete error 32, and requires the
production worker open to retain `SQLITE_CANTOPEN`. Unix displaces the database
and places a directory at its production pathname, then requires the same
classified worker failure. Both paths prove unchanged database bytes, all
eight persisted columns, `PRAGMA integrity_check = ok`, removal of the host
condition, normal worker write/flush, and close/reopen recovery.

The Windows probe passed 50/50 serial stress executions and the Unix
counterpart passed under Ubuntu WSL. No production behavior changed and no
defect surfaced. This closes ordinary platform open/rename/delete denial; it
does not claim parent-directory sync, device-cache persistence, torn-sector
behavior, or physical power-loss durability. Exact construction, hashes,
commands, and scope are retained in
[`persistence-platform-syscall-faults-20260730.md`](../docs/evidence/test-coverage/persistence-platform-syscall-faults-20260730.md).

## Targeted mutation evidence

The scheduled mutation matrix covers the pure privacy boundary in
`sorotte-secret`, controlled-room authorization in `sorotte-server`, raw
command-order/error/redaction behavior in `sorotte-protocol`, reconnect/state-
acknowledgement and ping/RTT decisions in `sorotte-client-core`, and persisted
runtime-configuration precedence in `sorotte-client-app`. Two additional
ratchets cover room-persistence arbitration in `sorotte-server` and inbound
`Set` command completion/order in `sorotte-client-core`. A ninth shard binds
the CLI inbound framing accumulator and its package-level transport/session
oracles. A tenth shard binds client playlist snapshot, target-index, shuffle
seed/PRNG, permutation, and undo decisions. It deliberately does not mutate
the whole workspace. Ten participant-status prevention shards extend that
matrix across the additive protocol DTO, client acceptance and freshness,
client reporting lifecycle, independently coalesced client outbox delivery,
server validation/projection, GUI presentation, the client-app and CLI
lifecycle owners, explicit mpv IPC retry, and the causal playlist delivery
fence. The nine behavior
shards use policy-owned mutant-name regular
expressions so their scheduled scope stays bounded to those invariants.

Cargo-mutants selects the `PlayerTransportDelta.logical_pause` field-deletion
entry from the enclosing ordered-event context alongside the client runtime
status transition. That shard admits only this exact field, struct, and
function identity; its policy self-test rejects neighboring fields, structs,
and functions instead of widening the shard to every ordered-event mutation.

The server selector similarly admits only the five `StateSyncOptions` fields
that cargo-mutants selects from the enclosing periodic status-projection
function: `set_by`, `client_latency_calculation`, `client_ignoring_counter`,
`server_rtt_seconds`, and `latency_calculation_seconds`. These are viable
mutations, not exceptions, so the scheduled server tests must kill them; the
policy self-test rejects neighboring fields, structs, and functions.

The recorded client projection diagnostic classified 14 compiler-unviable
outcomes as seven structured identities: five missing-`Default` view/scope
replacements and two identity groups covering Rust let-chain `&&` to `||`
rewrites. A fresh current-source campaign exercised 135 mutants, caught all
121 viable mutants, and matched all 14 unviable outcomes to those exact
expiring identities with zero misses or timeouts.

The recorded server diagnostic classified 13 compiler-unviable outcomes as
nine structured identities: seven missing-`Default` report, availability,
correlation, snapshot, scope, and directed-message replacements plus two let-chain rewrite
identities. The scope-transition, cache, per-field evidence-clock, and
rollback-safe scheduling work expanded the current inventory to 161 mutants.
A fresh full-shard campaign caught all 148 viable mutants and matched all 13
unviable outcomes to the exact expiring identities with zero misses or
timeouts. None of the earlier viable misses became exceptions.

The first GUI playlist-fence diagnostic reported 22 mutants, eight viable
misses, and three nominally unviable outcomes. Two of those outcomes were
Windows rustc/metadata failures rather than source-invalid mutations; fresh
serial replays caught both, so they are not exceptions. Removing four
equivalent discarded bitwise expressions and isolating the production fence
types from unrelated adapter defaults made the live inventory 42. The frozen
full replay caught all 41 viable mutants with zero misses or
timeouts; its sole accepted identity is the `&&` to `||` rewrite in the
delivery-completion let-chain, which rustc rejects syntactically.

Room-persistence arbitration's newest-version rule is an eventual service
guarantee while the worker remains alive, not synchronous acknowledgement
durability at every instruction boundary. A newer desired effect can arrive
after an older effect's final currency check; if the process then terminates
before the worker applies or an explicit flush/shutdown acknowledges the newer
effect, recovery may observe the preceding committed state. The arbitration
proof prevents stale queued work from overtaking newer live work; it does not
turn asynchronous enqueue into a durable commit acknowledgement.

`coverage/mutation-policy.toml` pins cargo-mutants 27.1.0, each package and
literal source file, the package/library test target, optional test selector
prefix, and optional source-bound mutant-name expression, all-feature
locked Cargo execution, two workers, per-command timeouts, a 100% viable kill
requirement, zero missed mutants, and zero timeouts. Inventory outside a
declared expression is rejected. The privacy shard's one compiler-infeasible
const mutation is matched by stable structured identity and has an expiring
review date; both a new exception and a stale exception fail. Server
authorization requires no exception. The protocol shard has eight exact,
expiring compiler-infeasible default-value substitutions; its 80 viable
mutations must all be caught.

Run it locally with:

```text
cargo install cargo-mutants --version 27.1.0 --locked
python scripts/mutation_ci.py validate \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard protocol-codec
python scripts/mutation_ci.py run \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard protocol-codec \
  --results-root target/mutation-ci/protocol-codec \
  --output target/verification/mutation-protocol-codec.json
python scripts/mutation_ci.py verify-report \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard protocol-codec \
  --report target/verification/mutation-protocol-codec.json
```

The wrapper disables repository-local cargo-mutants configuration, lists the
inventory before execution, hashes configured sources plus workspace test,
fixture, Cargo, toolchain, policy, and wrapper inputs before and after, and
reconciles every structured outcome with the inventory, status files,
build/test phases, logs, diffs, policy, and producer exit. The weekly workflow
re-verifies the compact attestation against the final source bytes before it
uploads both the attestation and raw producer evidence for each matrix shard,
even on failure. A previously passing report is therefore not release evidence
after any bound source or test-input change. Test scope is not trusted from
configuration alone: every producer phase must exactly retain the configured
target and namespace, and report verification reruns the exact test-list command
and compares its canonical inventory digest. The participant-status aggregate
job then verifies the unique report paths in
`coverage/mutation-report-set.json`; unselected historical reports are not part
of the release evidence set.

The original privacy experiment moved from 22/43 to 43/43 viable mutants
caught while preserving its 44-mutant inventory. After credential-classifier
expansion, a clean replay exposed 29 missed and five timed-out mutants; bounded
scans and deterministic escape/key/token oracles now catch 121/121 with the
same one accepted const exception. The authorization experiment rejected a
package-wide timed-out baseline, then exposed one missed and one timed-out
mutant at library scope. Deterministic negative grammar and salt-byte oracles
now catch 19/19 through a focused 7-test namespace in 113.36 seconds. The
protocol baseline caught only 70/97 viable mutations; 17 exact scanner,
error-chain, and redaction oracles plus bounded scanner seams now catch 80/80
with zero misses or timeouts.
The reconnect baseline first proved a reconnect-only selector was too narrow,
then the owning `session::tests::` selector exposed seven surviving decisions.
Four focused contracts now catch 30/30 viable mutants; two exact let-chain
rewrites are compiler-unviable and expire on 2026-10-31. The wrapper also
preflights `cargo test --list --format terse`, records its 445-test digest, and
rejects zero tests, namespace escape, or zero mutants before execution.
The runtime-configuration baseline caught only 52/101 viable mutations.
Four whole-contract precedence and normalization tests now catch 98/98; five
generated let-chain parse failures collapse to three exact expiring policy
identities. The ping baseline caught 43/52. Input-validity, zero/equality,
moving-average, forward-delay, and wall-clock oracles caught every observable
survivor. One remaining comparison mutant was algebraically equivalent, so the
formula was behavior-preservingly normalized to a base delay plus a
nonnegative delta; the final source-bound inventory catches 47/47 with no
exception. The persistence-arbitration baseline caught only 3/25 viable
mutants; seven deterministic state-machine tests now catch 25/25, with two
exact expiring compiler-unviable identities. The inbound-order baseline caught
4/5; three independent command-order oracles now catch 5/5 without an
exception. The exploratory CLI framing baseline captured three missed
payload-length/CRLF decisions and four timed-out non-consuming/constant-frame
mutants (`TC-HARNESS-017`). An input-derived frame bound, required EOF probe,
and four exact MAX/MAX+1 LF/CRLF seams now catch all 33/33 viable framing
mutants through a 370-test package scope, with no timeout or exception. The
earlier aggregate
baseline is diagnostic only because its test oracle was strengthened while it
finished; the fresh stable-source campaign is the canonical attestation.
The playlist-shuffle baseline caught only 12/26 viable mutants, missed 12, and
timed out on two non-progress loop changes. Deterministic snapshot, index,
seed-framing, PRNG, golden-permutation, and 512-seed invariant oracles plus a
narrow test-only completion guard now catch 26/26 viable mutants with no miss
or timeout. Two let-chain rewrites are represented by one exact expiring
compiler-unviable identity.
Across the ten established shards, all 484 attested viable mutations are
caught with zero misses and zero timeouts. Their 17 exact accepted
compiler-unviable identities remain policy-bound. The participant-status
protocol campaign separately caught all 32 viable mutations and bound 14 exact
compiler-unviable substitutions. Fresh current-source campaigns for the nine
behavior shards caught 481/481 viable mutants: client acceptance and freshness
121, client reporting lifecycle 85, independently coalesced client outbox 10,
server normalization/projection 148, client-app lifecycle and presentation 35,
CLI lifecycle 13, GUI presentation 18, causal playlist delivery fencing 41,
and explicit mpv IPC retry 10. Together with the protocol shard, the ten
participant-status campaigns caught 513/513 viable mutants across 567 total
outcomes; all 54
compiler-unviable outcomes matched exact expiring policy
identities, with zero misses and zero timeouts. Raw reports remain below the
ignored `target/` tree; the wrapper rejects relative, absolute, traversal, and
symlink-resolved output escapes. Pull-request path selection includes every
participant-status production boundary, and the pull-request/weekly workflows
reproduce the policy-owned shards.
Commands, timings, hashes, classifications, and limitations are retained in
[`targeted-mutation-20260729.md`](../docs/evidence/test-coverage/targeted-mutation-20260729.md),
[`targeted-mutation-privacy-expansion-20260729.md`](../docs/evidence/test-coverage/targeted-mutation-privacy-expansion-20260729.md),
[`targeted-mutation-server-auth-20260729.md`](../docs/evidence/test-coverage/targeted-mutation-server-auth-20260729.md),
[`targeted-mutation-protocol-codec-20260729.md`](../docs/evidence/test-coverage/targeted-mutation-protocol-codec-20260729.md),
[`targeted-mutation-client-reconnect-20260730.md`](../docs/evidence/test-coverage/targeted-mutation-client-reconnect-20260730.md),
[`targeted-mutation-config-20260730.md`](../docs/evidence/test-coverage/targeted-mutation-config-20260730.md),
[`targeted-mutation-client-ping-20260730.md`](../docs/evidence/test-coverage/targeted-mutation-client-ping-20260730.md),
[`targeted-mutation-server-persistence-arbitration-20260730.md`](../docs/evidence/test-coverage/targeted-mutation-server-persistence-arbitration-20260730.md),
[`targeted-mutation-client-inbound-order-20260730.md`](../docs/evidence/test-coverage/targeted-mutation-client-inbound-order-20260730.md),
and
[`targeted-mutation-cli-framing-20260730.md`](../docs/evidence/test-coverage/targeted-mutation-cli-framing-20260730.md),
and
[`targeted-mutation-client-playlist-shuffle-20260730.md`](../docs/evidence/test-coverage/targeted-mutation-client-playlist-shuffle-20260730.md).

Local generation requires both the pinned cargo subcommand and the Rust LLVM
tools component, the legacy Python requirements, and the pinned Syncplay
checkout:

```text
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.8.4 --locked
python -m pip install -r requirements/legacy-python-interop.txt
git clone https://github.com/Syncplay/syncplay.git \
  .interop-cache/syncplay-legacy
git -C .interop-cache/syncplay-legacy checkout \
  d1c5f85af377c960c5a940707c4d01bc84fd9c3f
SYNCPLAY_LEGACY_ROOT=.interop-cache/syncplay-legacy \
python scripts/coverage_profile_lanes.py run \
  --repo-root . \
  --output target/verification/coverage-profile-lanes.json
cargo llvm-cov report --json --skip-functions \
  --output-path target/diff-coverage.json
cargo llvm-cov report --text \
  --output-path target/diff-coverage.txt
python scripts/llvm_cov_line_map.py \
  --repo-root . \
  --llvm-json target/diff-coverage.json \
  --llvm-text target/diff-coverage.txt \
  --output target/verification/coverage-line-map.json
```

`cargo llvm-cov` prompts interactively when the LLVM component is missing;
captured or headless runs can therefore appear hung unless the component is
provisioned first. CI installs it explicitly.

## Native interactive and real-mpv system evidence

`scripts/gui-native-smoke.ps1` treats the complete native inventory as required
by default, prebuilds the GUI and native harness, binds the report to the GUI
path and SHA-256, preserves raw output and producer exit state, rejects skips,
duplicate JSON keys, unexpected stderr, and binary mutation, and kills a hung
process tree on a derived wall-clock deadline.

For ordinary local development, `-InputMode UiaOnly` runs a fixed menu
inventory and UIA File -> Exit lifecycle without desktop-wide mouse, keyboard,
or cursor injection. The driver rejects `SendInput` and cursor movement before
dispatch, fails if any such attempt is reached, emits explicit
`optional-skip(reason=local-uia-mode)` outcomes for physical and focused-keyboard
capabilities, and stamps the summary `local-pass` plus `authoritative=false`.
The strict validator requires `input_mode=strict-physical`, so local evidence
cannot be substituted for CI proof. The implementation and first live bundle
are recorded in
[`native-interactive-local-uia-development-20260801.md`](../docs/evidence/test-coverage/native-interactive-local-uia-development-20260801.md).

`.github/workflows/gui-native-interactive.yml` now owns a dispatch-only,
fail-closed contract for running that exact inventory on a separately
provisioned, one-job, ephemeral interactive Windows runner. It checks the
external runner attestations and interactive desktop before checkout, binds
checkout to a requested full SHA, retains the exact evidence inventory, and
has no stderr exception. No matching external runner was available in this
slice, so the workflow is implemented but is not yet an executed or required
merge gate. The exact contract and operational blocker are retained in
[`native-interactive-ci-contract-20260731.md`](../docs/evidence/test-coverage/native-interactive-ci-contract-20260731.md).

The native bundle retains screenshots, redacted UI Automation trees, isolated
configuration, structured capability outcomes, invocation identity, process
exit, and scenario logs. Loopback-only fixture policy plus stderr rejection
catches the networking failures observed in this work; OS-level network
isolation is still required before claiming that silent outbound traffic is
impossible.

The opt-in `scripts/gui-real-mpv-vertical.ps1` lane crosses the actual native
GUI and managed-player boundary with an exact digest-bound mpv binary,
generated local WAV, isolated configuration, IPv4-loopback session fixture,
physical Open Media and Exit leaves, and real mpv Play/Pause observations. At
the prior three-mode checkpoint, a post-gate local run passed its exact
13-assertion, 10-artifact contract in bundle
`20260731T044916649Z-67112`; the missing-mpv preflight failed closed
before build or launch. This is local Windows evidence, not a new CI gate. The
pass, retained red bundles, strict path/Hello/process identity, and limitations
are recorded in
[`native-gui-real-mpv-vertical-20260731.md`](../docs/evidence/test-coverage/native-gui-real-mpv-vertical-20260731.md).

The separate recovery inventory terminates only an mpv PID already attested as
the GUI's direct child with the exact preflight image path and digest. It then
requires bounded automatic replacement with a new PID and IPC endpoint,
re-attests parent/path/digest/arguments, reopens generated local media through
physical native UI, observes replacement-mpv Play/Pause, rejects stale or
foreign post-boundary observations, and proves native Exit reaps both player
generations and the GUI. The healthy default remains its original
13-assertion/10-artifact contract; recovery is an explicit
20-assertion/13-artifact opt-in. The preserved RED disproved a manual-modal
oracle because production automatically relaunched the managed player. Exact
bundle identities and limitations are retained in
[`native-gui-real-mpv-owned-process-recovery-20260731.md`](../docs/evidence/test-coverage/native-gui-real-mpv-owned-process-recovery-20260731.md).
At the prior three-mode checkpoint, post-gate bundle
`20260731T045019794Z-49868` replaced exact attested PID `61396` with PID
`48892`, used distinct managed IPC endpoints, and released both generations.

The faulting-HTTP inventory serves generated PCM AU media bytes from a strict
ephemeral IPv4-loopback listener. The AU header declares the complete
45-second stream. The first paced HTTP GET has no `Content-Length`, uses
chunked transfer, emits exactly 720,000 valid AU body bytes, and then injects
an invalid chunk-size boundary; one subsequent GET must transfer the complete
AU body with its exact length. The same attested GUI-owned mpv process,
executable, IPC endpoint, URL, media identity, and duration must progress
before the fault, publish `eof-reached=true` with more than the bounded
15-second recovery threshold remaining, reload, progress beyond the retained
pre-fault position, pause, retain complete evidence, and release the GUI,
player, session, HTTP listener, and IPC resources. The opt-in contract is
closed at 18 assertions and 11 artifacts; healthy and owned-process modes
remain unchanged. The preserved REDs and all three resolved findings are
retained in
[`native-gui-real-mpv-faulting-http-recovery-20260731.md`](../docs/evidence/test-coverage/native-gui-real-mpv-faulting-http-recovery-20260731.md).
At that prior three-mode checkpoint, bundle
`20260731T045105652Z-43360` ran last and passed with the same GUI digest as the
healthy and owned-process campaigns. It retained exactly two requests,
same-PID/same-IPC recovery, no manual retry, and complete player/server/socket
release.

The fourth native mode keeps a valid complete-length AU response open after a
deterministic playable prefix, emits no further byte and no EOF for at least 25
seconds, and requires one bounded same-process cache-stall recovery. Its final
post-build implementation-source bundle `20260731T150829535Z-48288` ran last after the
healthy, owned-process, and malformed-HTTP modes and passed 18 assertions with
11 artifacts. Exact silence, request, process, IPC, media, and digest evidence
is retained in
[`native-gui-real-mpv-stalled-http-recovery-20260731.md`](../docs/evidence/test-coverage/native-gui-real-mpv-stalled-http-recovery-20260731.md).

The required Linux `mpv-pr-semantics` lane is separate from those four Windows
native GUI modes. Pull-request and push runs retain the peeled mpv `v0.41.0`
minimum. Scheduled and manually dispatched runs expand the same fail-closed
job to both that minimum and reviewed post-release snapshot
`d12f2ce19c918875981e00ed276f153bdf40a2ac`, 330 official commits ahead. Both
sources are immutable and verified after checkout; a floating `master`, a
missing or collapsed endpoint, and matrix fail-fast are policy failures. Each
selected endpoint executes pause/seek/resume, cache-cap drain, premature HTTP
disconnect recovery, and the full stalled-HTTP recovery harness. The
implementation and committed-source newest-snapshot campaigns both passed
4/4. A standalone validator accepts bounded release/development version forms
while retaining exact source and minimum-version checks; exact correction-head
run `30673650173` passed both endpoint jobs and all eight selected executions.
Exact source selection, the upstream libplacebo boundary, parser diagnostic,
and artifact hashes are retained in
[`mpv-version-matrix-20260801.md`](../docs/evidence/test-coverage/mpv-version-matrix-20260801.md).

After `TC-HARNESS-045`, both real-mpv clients must first reach exact
`ReadyPaused` at revision 1 and exact timeout-free `Playing` at revision 2 with
seeking clear. Only then does the fixture arm one globally claimed path stall
across range/retry connections. The affected client may issue at most one seek
per observed recovery episode, the healthy peer must perform no post-start
seek, and the stall must apply and complete exactly once. A separate
deterministic concurrent-request regression requires both parked handlers to
resume and return their complete response bodies. This preserves the strict
recovery/isolation oracle while preventing startup timing from triggering the
fault prematurely.

The required Windows nextest lane also retains its first-attempt flake as
evidence. In run `30636380151`, job `91174920040` executed 3,775 tests: 3,774
passed, while the Plex connected-session test failed once and passed on retry;
fail-on-flaky correctly rejected the job. `TC-HARNESS-046` is distinct from
the panic-safe shared-environment fix in `TC-HARNESS-004`: the loopback Plex
fixture's read loop treated every socket error as request completion and could
count an empty or partial accepted-socket header as a request. The retained
failed artifact did not capture the request bytes or error kind; that
mechanism is the source diagnosis. Commit
`dd3012c1bcefa0a68520b063c5ae06f3e1b96f79` accepts only a
complete `CRLFCRLF` header across `Interrupted`, `WouldBlock`, and `TimedOut`
reads under one deadline. A scripted split-header regression is positive and
the real-socket test retains its production-path sections -> file lookup ->
timeline order oracle with stronger failure diagnostics. Exact-head run
`30639113884` then passed 3,777/3,777 Windows nextest cases with no flaky
attempt. Every other required producer, coverage-diff job `91190243453`, and
aggregate job `91192554763` also completed successfully at exact implementation
head `dd3012c1bcefa0a68520b063c5ae06f3e1b96f79`.

## Required live Python compatibility evidence

`SYNCPLAY_REQUIRE_LIVE_INTEROP=1` now turns every missing oracle, Python
process/package, legacy process, TLS, fixture, and disabled-parity path into a
closed failure while preserving optional developer behavior when absent. A
selector-free wrapper pins the legacy commit and Python package versions,
hashes probes and all 89 fixtures, discovers the complete/ignored inventory,
executes every non-writing all-feature test serially, and validates exhaustive
disjoint accounting plus exact-key JSON.

The historical committed-source local report over `3cd64ce` listed 143 tests,
passed all 136 executable tests, skipped zero, and matched the seven exact
fixture writers. A preserved committed-run RED found that an attested relative
oracle path was passed unchanged to Cargo and resolved from the crate working
directory; the wrapper now passes the absolute already-attested path. After
adding the generated differential below, the prior committed-source report
over `e3d8554` listed 144 tests, passed all 137 executable tests, skipped zero,
and retained the same seven fixture writers. The current committed
inventory is 149 tests: 142 executable tests and the same seven fixture
writers. The historical coverage-policy checkpoint at `829ab98` passed all
142 with zero failures or skips. A fresh local report at `dd3012c` passed the
same complete accounting in 48.280455 seconds, and exact-head hosted run
`30639113884` independently passed it again. Neither result relabels the
immutable `829ab98` artifact.
The report/log hashes, prerequisite identities, missing-prerequisite
proof, RED, and local-vs-hosted limitations are retained in
[`compat-required-live-interop-20260731.md`](../docs/evidence/test-coverage/compat-required-live-interop-20260731.md).

## Generated Rust/Python JSON framing differential evidence

A fixed-seed, 256-case required-live test drives generated byte lines through
Sorotte's production UTF-8 and protocol decoder and the actual pinned
Syncplay `JSONCommandProtocol`. It covers all seven commands, escaped keys,
surrounding whitespace, multi-command objects, more than 100 duplicate-key
cases, 16 malformed-JSON cases, and 16 malformed-UTF-8 cases. Closed request
and response schemas require exact unique-ID and accepted/rejected accounting;
rejected inputs may dispatch no partial commands.

All 256 cases matched. The implementation-commit matrix over
`e3d8554a61aea9dc1fe8252540e22aff5b134bb6` listed 144 tests, executed and
passed 137, skipped none, and accounted for seven exact fixture writers in
47.920239 seconds. Generated input remains in process-owned memory; this is a
line-level differential, not a socket segmentation or stateful server test.
Exact seed, report and manifest hashes, command, and limitations are retained
in
[`compat-generated-json-framing-differential-20260731.md`](../docs/evidence/test-coverage/compat-generated-json-framing-differential-20260731.md).

## Disposable persistence replay capability

`scripts/persistence_power_loss_harness.py` and its test-only Rust driver add a
nonce-owned, opt-in Linux `dm-log-writes` replay capability over newly created
sparse images only. The safety policy, plain-temporary-file production-worker
phase model, Windows compilation, and nonprivileged WSL preflight passed. The
privileged device-mapper capability did not run: `replay-log` was absent and
the unprivileged WSL process could not access device mapper. Consequently this
slice makes no power-loss, device-cache, torn-write, or physical-media
durability claim. The exact old-or-new recovery oracle, ownership guards,
preflight output, and future invocation are retained in
[`persistence-disposable-block-replay-harness-20260731.md`](../docs/evidence/test-coverage/persistence-disposable-block-replay-harness-20260731.md).

## Server-container consumer and publication contract

The server-container workflow now builds and loads once, inspects and runs the
loaded non-root image, generates an SPDX SBOM from that local identity, pushes
only tags of the consumed daemon image, signs and attests its resulting
manifest digest, logs out, and compares every anonymous public GHCR reference
with the tested config and RootFS identity. Its final gate cross-binds runtime,
restart/persistence, SBOM, push, signature, attestation, and public-registry
reports. Actions, Dockerfile frontend, base images, Syft, and Cosign are pinned.

The offline policy suite passed, but Docker, Syft, Cosign, and a publication
target were unavailable locally. No image build, container run, push,
signature, attestation, or public-registry comparison is claimed until the CI
workflow produces its complete green artifact. The contract and that remaining
execution boundary are recorded in
[`server-container-build-load-publication-contract-20260731.md`](../docs/evidence/test-coverage/server-container-build-load-publication-contract-20260731.md).

Historical product and harness findings, their completed dispositions, the
explicitly empty current product-defect registry, and the remaining external
capability boundaries are tracked in
[`docs/TEST_COVERAGE_FINDINGS.md`](../docs/TEST_COVERAGE_FINDINGS.md).
