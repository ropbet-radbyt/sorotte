# Testing-process implementation

Base: `4000eca69b52003b66e81b6998d15c555e7eb6d1` (main after 0.2.9).
Branch: `codex/testing-process-hardening`. The original checkout and its unrelated
work remain intact; implementation uses a separate worktree. The
[audit and evidence packet](testing-apparatus-audit-2026-09-06.md) define T01–T18.

This ledger distinguishes implemented contracts, local execution and external
activation. It does not attest a later commit, product release or unexecuted
environment. This implementation did not create an App or credentials. The owner
subsequently reported installing the App; fresh API reads confirmed its expected
Actions configuration names and main's required-check policy. App-token read
authorization remains a separate qualified-main check.
The table records implementation evidence and its acceptance boundaries. Current
candidate results belong to the implementation PR and its immutable Actions attempts; this
document is not a live declaration that every external gate has passed.

| Task | Implemented responsibility | Evidence and remaining acceptance |
|---|---|---|
| T01 | Seven stable aggregate checks; conservative immutable change plans; required-producer outcome and trusted main authority validation; independently authorized native candidates. | Adversarial source, baseline, producer, skip/cancellation and authority tests. Fresh API reads confirmed strict main protection with all seven checks bound to GitHub Actions, administrator enforcement and no force pushes/deletion. The [disposable enforcement record](../evidence/testing/protection-enforcement-2026-09-06.md) binds the passing documentation-only revision and subsequent HTTP 405 missing-check rejection to their exact sources. Final implementation checks remain separately required. |
| T02 | `verify preflight` and `verify plan`; Linux/Windows early self-tests; existing responsibility/model/ignore/mutation/corpus validators; owned process, loopback, external TEMP, pinned tool and clean legacy checks. | Integrated non-compiling Windows preflight passed. Restricted process permissions were correctly rejected; ordinary process permissions passed. Inventory discovery is separately labeled compilation. Native session/media checks stay in guest readiness. |
| T03 | Frozen mutant inventories, balanced execution chunks, exact-union finalizer, fresh shared test listings, isolated targets and reviewed compiler exceptions. | Actual seven-mutant/three-chunk campaign and positive/negative tool canary. Candidate `3bee8c33` passed the full 1,429-mutant campaign. The failed `c4688afb` GUI baseline and room-request survivor remain preserved. Candidate `72f93a12` passed all 1,503 mutations across 25 responsibilities and 45 chunks: 1,412 caught and 91 reviewed compiler-only occurrences, with no survivors or timeouts. Its observed 44m09s total does not establish a controlled speed improvement or meet the provisional 35-minute target. Later candidates require their own complete campaign. |
| T04 | Streaming bounded/redacted output, phase heartbeats, owned Job/process-group cleanup, incomplete timeout/cancellation receipts and immutable attempts. | Actual canary execution and process fault tests, including owned Linux descendant timeout cleanup. Required nextest failed-then-passed and process-leak policies remain intact. |
| T05 | Reviewed complete test inventories; propose/diff/check; derived compatibility/mpv/GUI/server totals; exact required selections; source/corpus identity. | Real Windows nextest discovery for all seven reviewed scopes; addition/removal/ignore and empty/ambiguous inventory tests. No automatic acceptance of a reduced inventory. |
| T06 | Real LLVM canary: ordinary tests, standalone binary, child process, multiline Rust, LF/CRLF variants, JSON and independently parsed source views. Full collection waits for the canary. | Actual Windows and Linux producer canaries pass. The [pattern-classifier incident](../evidence/testing/coverage-classifier-2026-09-06.md) retains the original hosted failure and expands the original five markers with seven independently required executable markers and two structural wrappers. Both platforms reproduced the original classifier failure and passed the repair with LF/CRLF inputs. Mapped zero counts, missing/duplicate maps and removed prerequisites still fail. Full coverage thresholds remain 80/90 with immutable base/head union. |
| T07 | One coordinated stable qualification; sealed source/build/platform/media bundles and exact tested binaries; independent archive consumers; approved container digest promotion. | Release authority, bundle mutation and consumption tests; actionlint and PowerShell parsing. A real coordinated release/public promotion has not been performed. The owner configured the App; its token's policy-read authorization remains unexercised. |
| T08 | Versioned guest inputs, reviewed integrated Sandbox scripts, one-job runner controller, readiness before registration, watchdog and narrow teardown. | Candidate `c4688afb` passed independently reviewed interruption and cancellation drills; candidate `72f93a12` passed a fresh complete positive native run and its independently bound required-check consumer. The repaired 18,521-file bundle passed its copied-Python dependency/import probe; positive qualification ran seven exact Rust canaries, 90 Python cases and ten StrictPhysical scenarios. All owned resources were removed. Later candidates still need their own positive source-bound qualification. |
| T09 | Always-run privacy-safe native failure projection, host fallback export, separate passing attestations, source/run/attempt attribution and durable release receipt ZIP. | Credential/path canaries and failed/unavailable export tests. The `c4688afb` interruption and cancellation drills verified diagnostic export before guest teardown, final cleanup export and completed watchdog receipts. Durable public release publication remains outside this implementation task. |
| T10 | Extracted fake-server exchange contract, independent real-server conversation, seven named native readiness canaries for recorded 0.2.9 seams. | One real-server conversation, seven exact Rust canaries, and 22 native unit/socket tests passed locally. These do not substitute for actual minimum/newest mpv or independent lifecycle qualification. |
| T11 | Per-case server resources; removed shared poisoning mutex; bounded semantic deadline, recent frames and logs; primary incident plus separate cleanup record. | All seven real-server release fixture cases passed in 93.14 seconds. Injected absent-Hello case separately proves test-site attribution, port closure and the next case's exchange. Optional live-Python paths require the strict prepared environment; a default local pass does not prove those paths ran. |
| T12 | Format/static validation before expensive behavior; independent 15-crate semver producer; preserved parallel Linux/Windows coverage; early Linux/Windows preflight. | Structural and deliberately weakened dependency/order tests plus real preflight. Hosted first-feedback and full critical-path measurements remain observational acceptance work. |
| T13 | Reviewed tool/reference manifest, lock-bound resolution, exact clean legacy source, fixed hosted OS families, verified immutable Cargo archive cache and isolated instrumentation/mutation storage. | Cache corruption/confinement and input-identity tests. Python constraints and resolution evidence are checked independently. Warm/cold hosted comparisons must include setup and compilation costs. No compiled-product cache was introduced without measured evidence. |
| T14 | Explicit server Prepare/Behavior/Build/Archive stages; validated default-workspace receipts; independently consumed archive bytes; automated server-asset attachment. | Wrong-source/features/platform receipts cannot suppress required tests; all-feature and default-feature obligations remain distinct. The `72f93a12` package run passed independent archive/payload review. Its elevated GUI consumption covered launch and updater refusal, not installation/replacement/rollback; later candidate packaging remains separately required. |
| T15 | Owned manual/scheduled registry with missing/stale/current states; weekly headless scaling; trusted native full/144-DPI schedule and explicit 96/192 dispatch. | All 26 capabilities have owners/commands/environments and freshness budgets. Actual Windows normal/large scaling and clone-sensitivity probes passed; its final run retained ten observations and twelve successful owned-process cleanups. Published registry evidence stays unavailable until recorded with its source/date/artifact. Screen-reader, actual DPI, optimized startup and privileged storage claims require their stated equipment. Maintenance generators never run automatically. |
| T16 | Exact retained 0.2.9 crash inventory and cheap deterministic replay; actual pinned fuzz build/run/minimize/replay canary; original bytes and execution statistics retained. | Actual Linux canary passed in 9.473 seconds, preserving the original 50 bytes and minimizing to seven; retained product regression passed 1/1 with pinned nextest. Relevant PR and full scheduled seeded exploration remain required. |
| T17 | Source/input-bound attempts, primary failure/replay/cleanup, JSON plus readable receipt index, separate evidence-backed incident annotations and job timing observations. | Tests distinguish parallel execution span from job-minutes, preserve cancelled attempts, reject duplicate/wrong-source timing, and avoid calling an unchanged retry a proven flake. Final local and hosted reports live under their explicit run/attempt paths. |
| T18 | Workflow-scoped stable step IDs and reviewed contract catalog; independent required-graph, failure-tolerance, source/filter and pinned-action invariants. | Label-only changes are accepted; duplicated/missing contracts, weakened dependencies and conditional or tolerated failures are rejected. Existing strict behavioral/policy tests remain alongside real tool canaries. |

## Evidence locations and interpretation

Local receipts are in the implementation worktree's ignored `target` directories;
they are diagnostic implementation evidence, not public release authorization:

- `target/verification/integrated-preflight.json`
- `target/verification/coverage-canary-integrated/receipt.json`
- `target/mutation-agent/linux-coverage-canary-persistent-tool/canary/receipt.json`
- `target/verification/static-final-attempt-1/receipt.json`
- `target/verification/static-final-attempt-2/receipt.json`
- `target/verification/scaling-final-attempt-2/workloads.json.attempt/receipt.json`
- `target/verification/native-host-readiness.json`
- `target/verification/native-readiness-owned-process.json`
- `target/verification/server-fixture-failures/` (the intentional timeout and cleanup)
- `target/mutation-agent/real-campaign-final/required.json`
- `target/mutation-agent/native-canary-final/canary.json`
- `target/mutation-agent/linux-fuzz-canary-3/canary/receipt.json`
- `target/mutation-agent/fuzz-regression-replay.json`

Each required workflow retains its own immutable attempt artifacts. Final reports
must state the exact candidate, completed checks, original failures, unchanged
retries and remaining external activation. Do not treat these local receipts as
evidence for a different source merely because the tree is similar.

The audit's 35-minute critical-path target has not been established for this implementation.
Mutation kill counts measure assertion sensitivity, not discovered product bugs.
The earlier unexplained Windows socket timeout remains unclassified; the new
fixture isolates its failure consequences without inventing a historical cause.

The first integrated apparatus run after policy repair passed 965 Python tests
with four explicit platform/privilege skips. Clippy and workspace doctests also
passed. The subsequent strict live-interop Rust run was correctly rejected:
4,276 tests passed, but one updater test failed and then passed on retry. The
original attempt, including JUnit and policy output, is preserved at
`target/verification/behavior-failed-nextest-1`; its incident assessment remains
unclassified at `target/verification/behavior-incident-1.json`.

That updater fixture discarded the junction command's diagnostics and reused
PID-named roots. The repair gives each case an owned unique root, records command
and filesystem errors, prepares the junction separately, and renames it onto the
exact journal artifact being tested. It also removes a silent setup-failure bypass
from an existing package-link test. Product code and recovery assertions remain
unchanged. All 43 updater tests and 100 repetitions of the original six-case matrix
passed with zero retries. An open-handle replacement test and occupied-path
diagnostic test cover the repaired fixture. A held-file probe did not reproduce
the original OS failure; neither these successes nor the unrelated discovery of
`cmd` forward-slash parsing establish its historical cause.

The repaired full workspace then passed all 4,279 tests in 124.880 seconds, with
23 explicitly skipped manual/platform cases and zero failure, rerun or flaky
elements in JUnit. Strict live interoperability used the clean pinned Syncplay
commit and a separately installed constrained Python runtime. The saved second
attempt is `target/verification/behavior-passed-nextest-2`; it supplements the
failed first attempt instead of replacing it.

At the initial implementation freeze, the apparatus suite passed 992 tests in
73.395 seconds, with four explicit
platform/privilege skips. All workflows passed actionlint and Rust formatting
passed. Final Windows scaling acceptance took 114.648 seconds, including a cached
0.35-second build, two warmups, six measured normal/large samples and two deliberate
clone-sensitivity probes. Both source and build-input checks remained stable. The
new sidecar preserves every completed observation, first failure, replay command
and owned-process cleanup even when a later command fails or is cancelled; the
existing successful scaling-report schema remains unchanged. Real process failure,
timeout/descendant cleanup, cancellation, immutable-attempt and source/binary-drift
tests cover these paths. These local timing observations do not establish a hosted
critical-path improvement.

Subsequent candidate qualification exposed additional product and apparatus
failures. The [Windows publication evidence](../evidence/testing/windows-settings-publication-2026-09-06.md)
records the settings-reader repair and separately retained raw-filesystem
quarantine. The [Seek acknowledgement evidence](../evidence/testing/seek-acknowledgement-play-ordering-2026-09-06.md)
records a lost Play intent after a delayed earlier Seek acknowledgement, including
the original hosted trace and deterministic runtime reproductions. These findings
are distinct from the successful mutation campaigns, which measure assertion
sensitivity.

Candidate `3bee8c3315031b38fc320f121da6ee1e2f211ef1` passed its repaired coverage
comparison at 85.12% ordinary and 91.26% critical changed-line coverage, with zero
unmapped lines. Its full native attempt failed before GUI scenarios because the
canary runtime lacked PyYAML and nested WinPS inherited incompatible module paths.
The [native infrastructure record](../NATIVE_TEST_INFRASTRUCTURE.md) preserves
those prerequisites, original attempts and cleanup boundaries. These observations
do not qualify a later repaired candidate.

The final local seek/native repair passed all 1,050 apparatus tests in 173.518
seconds, with four explicit platform/privilege skips and successful owned-process
cleanup. All seven reviewed test inventories matched without updates. Its core
overlay also passed 21 focused regressions, 1,615 affected tests, the unchanged
original failing sequence, all 39 real minimum-mpv lifecycle checks and all 72
viable mutations in the new responsibility. One exact generated replacement was
compiler-unviable; there were no survivors or timeouts. These results are bound
under `target/verification/seek-echo-native-static-attempt-2/`,
`target/verification/seek-play-repair/repair-closure.json` and
`target/verification/seek-echo-mutation-attempt-2/`. Final documentation additions
record these outcomes after the source-bound test attempts; they do not change
the tested product or apparatus. Committed hosted and native acceptance remains
separate.

The [subsequent qualification-fixture record](../evidence/testing/qualification-fixture-failures-2026-09-06.md)
preserves candidate `c4688af`'s successful native qualification and distinct
Windows fixture-clock, protocol-echo deadline and mutation-baseline diagnostic
failures. The two fixtures have controlled before/after reproductions. The GUI
baseline process abort remains unclassified; supported failed-baseline outcomes
now retain their primary phase and log identity instead of an unknown-summary
error. These apparatus repairs leave production native scripts, the lifecycle
proxy, product Rust code, coverage thresholds and mutation kill requirements
unchanged.

The combined fixture/diagnostic repair passed all 1,058 apparatus tests in
143.307 seconds, with four explicit capability skips and successful owned-process
cleanup. Its immutable local attempt is
`target/verification/apparatus-fixture-static-attempt-1/`; final hosted checks
must still use the committed repair source. This observed duration is not a
controlled before/after performance comparison.

The complete `c4688afb` mutation attempt ended with 1,391 caught mutations,
91 reviewed compiler-only outcomes and one room-request survivor. Twenty GUI
mutations never ran after the failed unmodified baseline; no mutant timed out.
All 1,503 planned identities, 45 chunks and 25 responsibilities were reconciled,
with original official artifacts retained. The required aggregate correctly
failed. The [room-request evidence](../evidence/testing/seek-acknowledgement-play-ordering-2026-09-06.md#same-room-request-assertion-and-mutation-selection)
records the new observable assertion and shared mutation selection without a
product or policy change. Its frozen test overlay passed all 891 Linux
client-core library tests, 23 focused Windows tests, formatting and affected
Clippy checks. Complete mutation and hosted qualification retain separate,
source-bound attempts; these local successes do not replace a failed campaign.

Candidate `72f93a12` completed its full mutation, fuzz, dependency, packaging and
native attempts. Its Linux and Windows nextest runs passed 4,298 and 4,327 tests
with no failing, flaky or rerun JUnit elements. The required Rust aggregate still
failed on three uninstrumented syntax wrappers; that original failure is retained
in the [coverage classifier record](../evidence/testing/coverage-classifier-2026-09-06.md).
The repaired classifier passes the original maps with only those three line
decisions changed, leaving ordinary coverage at 86.38% and critical coverage at
95.03%. The four-file repair and its real LLVM canaries passed independent review.
Its full local apparatus run passed 1,071 tests with four explicit capability
skips; all 16 static preflight checks and owned-process cleanup passed. Inputs
remained byte-identical across the attempt. These local results are retained in
the isolated `tmp/testing-coverage-let-else` checkout under
`target/verification/coverage-pattern-static-independent-attempt-1/`. The
committed repair still requires its own hosted checks and native positive run;
passing disposable-PR evidence does not replace them.

## App authorization

The [App setup guide](../PROTECTION_READER_SETUP.md) and credential-free manifest
record the authorization contract. The owner has supplied the expected Actions
configuration; stable/dev publication must still prove a repository-scoped
Administration-read token can inspect the policy. Ordinary PR checks and native
candidate qualification use their normal read authority.
No release/tag/product publication is part of this implementation task.
