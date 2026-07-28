# Test coverage implementation findings

Date: 2026-07-28

Branch: `codex/test-coverage-design`

Original experimental base: `a08a06ea7c6cada5413b0dba73b16f940cfd46e1`
Current rebased base: `f3964ebc7f7b281b9b78f3bfb243ff65e5122e33`

This ledger separates product findings from failures in the new test
infrastructure. Per the scope of this branch, product behavior has not been
changed to make these tests pass.

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

- `cargo fmt --all --check` passed.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
  passed on Windows before the rebase and again on the rebased 0.2.4 tree in
  27.2 seconds.
- Before the rebase, `cargo test --locked --workspace --all-features` passed
  on retry in 181.7 seconds after the first attempt exposed TC-HARNESS-004.
  The complete rebased workspace, including doctests, then passed on its first
  attempt in 247.4 seconds. The final mutation-testing slice passed the same
  complete gate on its first attempt in 210.4 seconds.
- The final broad reducer-input property passed 2,048 generated cases in 1.57
  seconds after replaying eight minimized seeds. Both known-defect families
  are classified from their invalid post-transition graphs, not panic text,
  event kind, candidate count, or one history shape.
- One complete semantic run passed all 14 scenarios against Syncplay commit
  `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`, Twisted 25.5.0, pyOpenSSL
  25.3.0, and service-identity 24.2.0. A historical rerun exposed the
  intermittent 13/14 result recorded as TC-HARNESS-003; the pre-rebase final
  replay passed 14/14 in 29.7 seconds and the post-rebase replay passed 14/14
  in 39.2 seconds.
- All 247 Python infrastructure tests passed on the completed mutation slice
  after the native-coverage work: fail-closed evidence, parsed workflow policy,
  changed-line coverage, ignored-test policy, strict native-contract/watchdog,
  and targeted mutation cases.
  The 26 mutation-specific cases cover strict policy, source/package binding,
  producer/version/command ownership, inventory and status reconciliation,
  artifact traversal, duplicate keys, timestamps, phase arguments, source
  drift, thresholds, and expiring unviable exceptions.
- The targeted `sorotte-secret` experiment held the mutation inventory at 44
  while improving viable kills from 22/43 (51.16%) to 43/43 (100.00%) using
  seven test-only oracles. No product defect surfaced and no production
  behavior changed. One compiler-infeasible const-context mutation is
  explicitly matched and expires for review on 2026-10-31; exact proof is in
  `docs/evidence/test-coverage/targeted-mutation-20260729.md`.
- The behavior catalog validated 13 behavior IDs, 22 exact proofs, and two
  lanes. The known-defect registry validated seven defects and 14 exact
  characterizations.
- The ignored-test registry exactly classified all 25 source attributes:
  4 required pull-request proofs, 7 fixture-maintenance commands, 12 manual
  capability tests, and 2 expiring compatibility quarantines.
- The changed-line utility retained all 68 strict LCOV/diff-policy cases; the
  canonical-map consumer passed 9 additional adversarial cases, the native
  converter passed 14, and the six-phase finalizer passed 19. Coverage includes immutable
  base/head critical-policy union, policy-deletion downgrade prevention,
  inline `#[cfg(test)]` denominator dilution, adversarial Rust lexical tokens,
  new-tag, updated-tag, missing-base, merge-base, provenance, and
  partial-phase failure contracts.
- Deterministic protocol ordering passed 3 new permutation/adversarial tests;
  all 6 production-worker framed IPC tests passed; all 35 server persistence
  tests passed after combining this tranche with the six persistence tests
  merged on `main`, with the known migration failure characterized below.
- actionlint 1.7.12 reported no workflow syntax or expression errors before
  the final nextest workflow wiring. `actionlint` and Go were unavailable for
  the final replay; workflow parsing and adversarial mutation checks remain in
  the passing Python suite.
- `scripts/release-publication-policy-tests.ps1` passed under Windows
  PowerShell and PowerShell 7 after the rebase. The 0.2.4 package-path suite
  passed under Windows PowerShell; PowerShell 7 reproduced TC-HARNESS-001 at
  the same freshness assertion.
- A fresh Windows cargo-llvm-cov 0.8.4 run on the pinned toolchain passed the
  complete instrumented workspace and produced a 15,089,306-byte LCOV artifact
  after explicit LLVM-tools provisioning. Strict replay rejected its
  contradictory summaries as TC-HARNESS-005.
- The exact pinned nextest wrapper retained 3,458-test JUnit evidence and
  correctly remained red when TC-HARNESS-006 leaked a handle on its first
  attempt but passed its retry.
- The strict native baseline rejected evidence that the legacy runner had
  reported as `result: "ok"`; raw evidence and its digest are recorded below.

## TC-PLAYER-001: concurrent external replacement corrupts predecessor linkage

Status: **Open; reproducible; production code intentionally unchanged**

Severity: **High**
Detection: shrinkable reducer-input history

Minimal history in attachment epoch 1:

1. `ExternalLoadObserved(generation=1, playlist_entry=100, file_loaded=false)`
2. `LoadAttemptSubmitted(command=1, generation=1, target="property-target-0")`
3. `ExternalLoadObserved(generation=1, playlist_entry=101, file_loaded=false)`

The third transition trips the reducer's own invariant:

```text
attempt predecessor points to another successor
```

This is a plausible asynchronous ordering: an external physical load exists, a
commanded replacement is submitted, and another external load is observed
before the submitted attempt is resolved. Adapter-level reachability has not
yet been proven, but the public reducer input contract currently accepts the
history and creates an internally contradictory predecessor/successor graph.

The history was found while preparing the stale-epoch metamorphic property. It
failed during setup, before the stale observation was applied; it is therefore
not evidence of stale-epoch mutation. Its discovery seed belonged to an older
stale-property strategy and was removed because Proptest seeds are coupled to
both their source file and exact strategy shape. The minimized history is
durably encoded by the executable known-defect characterization
`known_defect_tc_player_001_external_replacement_breaks_predecessor_links`.
Deeper generation also found the same graph overwrite when acceptance of a
second submitted attempt repointed a predecessor that still had a rejected
successor backlink. That event-kind variant is separately encoded by
`known_defect_tc_player_001_acceptance_overwrites_predecessor_link`.

The broad property does not match the panic string or require an external-load
input. It runs the real transition through a test-only unchecked reducer seam,
requires a valid pre-state, then verifies the exact contradictory post-state:
a newly selected reciprocal successor and at least one preserved stale
backlink to the same predecessor. Same-text failures without that graph remain
red.

## TC-PLAYER-002: delayed acceptance plus authoritative binding leaves terminal and active state

Status: **Open; reproducible; production code intentionally unchanged**

Severity: **High**
Detection: shrinkable reducer-input history

Minimal history in attachment epoch 1:

1. `LoadAttemptSubmitted(command=1, generation=1, target="property-target-0")`
2. `ExternalLoadObserved(generation=1, playlist_entry=100, file_loaded=false)`
3. `LoadAttemptAccepted(attempt=1)`
4. `PlaylistSnapshot(current_entry=101, original_filename="property-target-0")`

The fourth transition trips the reducer's own invariant:

```text
logical terminal playback still has an active physical attempt
```

The ordering models an external observation racing ahead of the command
acceptance response, followed by authoritative playlist reconciliation. The
reducer reaches a state that simultaneously claims logical terminal playback
and an active physical owner.

Source- and strategy-scoped Proptest replay seed:

```text
21da6327ec034d62801fcab370f374a0861f646f68e76356ece4bb17fcf8741d
```

The broad property explicitly quarantines this known invariant family after
detecting its causal state transition, leaving the pre-transition state intact
so later generated cases still execute. The classifier requires a valid
pre-state and an invalid post-state with both a concrete terminal physical
owner and a different current-epoch live physical owner under the retained
logical terminal outcome. It is independent of panic text, candidate count,
target spelling, triggering input kind, and predecessor linkage.

The minimized history has the executable known-defect characterization
`known_defect_tc_player_002_delayed_acceptance_retains_terminal_active_state`.
Six additional deterministic characterizations cover cross-generation,
superseding-submission, repeated-external, loaded-external,
terminal-external, and replaced-attempt variants.

## Reproduction

Run the full property module, including all executable known-defect
characterizations:

```text
cargo test --locked -p sorotte-player-mpv --all-features --lib \
  lifecycle::property_tests -- --nocapture
```

Expected result on this branch: all 14 tests pass. Nine named known-defect
characterizations across two defect IDs pass only because `should_panic`
requires their exact invariant failures. The broad reducer-input property
classifies only the two contradictory post-state graph families and continues
through persisted and novel cases. The stale-epoch property deterministically
exercises all epoch-bearing input kinds against live current-epoch identity
collisions for every generated setup.

The known-defect characterizations are intentionally absent from
`coverage/behaviors.toml`: they prove undesired current behavior, not a merge
contract. When production is fixed, convert every characterization for that
defect ID into a positive regression and remove the corresponding family
classifier.

## Required follow-up outside this branch

For each finding:

1. prove or disprove adapter-level reachability with an ingress trace;
2. decide the authoritative conflict rule for external versus commanded loads;
3. fix the reducer in a product-behavior change;
4. convert the named characterization into a positive deterministic regression
   and remove the matching defect-family classifier;
5. rerun lifecycle projection, semantic, native, compatibility, and full
   all-feature workspace gates.

## TC-SEC-001: structured credential aliases survive transcript sanitization

Status: **Open; deterministic; sanitizer behavior intentionally unchanged**

Severity: **High (sanitized diagnostic artifacts can retain credentials)**
Detection: generated nested/escaped credential-taint corpus

The new privacy suite generates credential canaries across seven nesting
levels, ordinary and Unicode-escaped JSON keys and values, URL/header/path
forms, malformed transcript input, JSON-lines round trips, `Debug`, diagnostic
dumps, and sanitizer idempotence. Recognized sensitive keys remain redacted
through every tested output.

The same experiment found five structurally credential-bearing aliases outside
the current key classifier:

```text
credentials
futureCredential
set-cookie
x-api-key
httpHeaders
```

For each alias, the raw or encoded canary survives the sanitized transcript's
JSON-lines export. This is a product privacy defect rather than a weakness in
the test oracle: the test checks raw, Unicode-escaped, percent-encoded, and
hexadecimal canary forms after transcript construction and serialization.

The executable characterization is:

```text
transcript::privacy_tests::
known_defect_tc_sec_001_structured_credential_aliases_leak_from_sanitized_transcript
```

It uses `should_panic` with the exact assertion:

```text
structured credential aliases leaked from sanitized transcript
```

The sanitizer was not expanded on this branch. When the sensitive-key policy
is deliberately fixed, convert this characterization into a positive generated
regression and keep the broader generated corpus as a required privacy proof.

## TC-SEC-002: escaped diagnostic credentials survive PlayerError redaction

Status: **Open; deterministic; error behavior intentionally unchanged**

Severity: **High (reflected parser or transport diagnostics can disclose credentials)**
Detection: generated `PlayerError` display-taint corpus

The ordinary generated corpus confirms that raw nested JSON, URL query,
header-style, percent-delimited, and quoted credentials are removed from
`PlayerError` display and debug outputs. Four encoding variants evade the
current pre-display classifier:

```text
escaped-key       pass\u0077ord
escaped-colon     "password"\u003a
escaped-equals    password\u003d
encoded-key       access%5Ftoken
```

Each form retains its generated canary in the user-visible `Display` output.
The executable characterization is:

```text
error_display_redaction_tests::
known_defect_tc_sec_002_escaped_diagnostic_credentials_leak_from_player_error
```

It uses `should_panic` with the exact assertion:

```text
escaped diagnostic credential forms leaked from PlayerError
```

No redaction behavior was changed. A product fix should normalize or safely
decode only the credential-key and delimiter grammar before classification,
then convert this into a positive regression without broadly hiding useful
non-secret parser diagnostics.

## TC-SEC-003: prose-prefixed credential fields survive PlayerError redaction

Status: **Open; deterministic; error behavior intentionally unchanged**

Severity: **High (ordinary reflected diagnostics can disclose credentials)**
Detection: generated `PlayerError` display-taint corpus

Even without escaped syntax, the classifier assumes that a sensitive key
begins immediately after one of a small set of structural delimiters. Natural
diagnostic prefixes therefore leave four generated canaries visible:

```text
prose-colon      request failed with password: Bearer <canary>
prose-equals     upstream response includes token=<canary>
parenthesized    request failed (secret=<canary>)
arrow-colon      backend -> clientSecret: <canary>
```

The executable characterization is:

```text
error_display_redaction_tests::
known_defect_tc_sec_003_prose_prefixed_credential_fields_leak_from_player_error
```

It uses `should_panic` with the exact assertion:

```text
prose-prefixed credential fields leaked from PlayerError
```

No redaction behavior was changed. The eventual product fix needs a bounded
credential-field grammar that recognizes these prefixes without turning every
word before a colon into a secret and hiding useful diagnostics.

## TC-HARNESS-001: PowerShell timestamp coercion falsely fails package freshness

Status: **Open; reproducible; test harness intentionally unchanged**

Severity: **Medium (required Windows gate false negative)**
Detection: final package-policy validation

`scripts/package-path-boundary-tests.ps1` exits 1 at its freshness assertion:

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
equal timestamps compare unequal. The source packaging logic was not changed.
The gate remains red until a separate harness fix normalizes both operands to
the same type.

Final host comparison confirmed the boundary: Windows PowerShell 5.1 passed
the package-path suite, while `pwsh` 7.6.4 failed at the freshness assertion.
The required Windows workflow uses `pwsh`, so this remains a real required-check
failure rather than a stale local observation.

## TC-SERVER-001: playlist JSON migration is not atomic across rows

Status: **Open; reproducible; production code intentionally unchanged**

Severity: **High (durability boundary permits partial migration)**
Detection: deterministic SQLite trigger failpoint

The characterization seeds two legacy persistent-room rows whose
`playlistJson` columns are null. A SQLite trigger allows the first migration
update and aborts the second with:

```text
injected second migration failure
```

`load_rooms()` returns that error, but inspection before restart observes one
already-migrated row:

```text
migrated_before_restart: 1
valid atomic results:     0 or 2
```

This violates the old-or-new-complete migration invariant across the selected
rows. Recovery remains functional: after removing the failpoint and reopening
the store, both rows migrate and deserialize correctly. Recovery does not make
the original failure atomic.

The minimized executable characterization is:

```text
tests::persistence_tests::
known_defect_playlist_json_migration_commits_rows_before_later_failure
```

It uses `should_panic` with the exact atomicity assertion, so the existing
defect remains visible while the rest of the persistence suite continues.
Focused execution passed all 35 persistence tests. The characterization accepts
only the two atomic outcomes—zero rows migrated or both rows migrated—and
currently panics with:

```text
playlist JSON migration must be atomic across rows, found 1 migrated rows
```

When the persistence implementation is fixed, convert this into a positive
regression that prohibits the partial count of one.

## TC-SERVER-002: concurrent quota-secret creation does not converge

Status: **Open; deterministic; production behavior intentionally unchanged**

Severity: **High (shared durable identity initialization can fail under concurrency)**
Detection: two-connection SQLite schedule with a pre-create barrier

`load_or_create_quota_secret()` performs a read followed by an unconditional
insert. A test-only barrier now pauses two independent store instances after
both have observed that the metadata row is absent. Releasing both creators
produces:

```text
successful creators: 1
failed creators:     1
failure action:      create quota secret
durable rows:        1
```

The winning 32-byte value is durable and remains valid. The losing caller
receives SQLite's uniqueness failure rather than loading and returning the
winner. The required contract is stronger: every concurrent initializer must
return the same durable secret, because callers should not need to distinguish
first creation from convergence on a concurrently created value.

The executable characterization is:

```text
tests::persistence_tests::
known_defect_concurrent_quota_secret_creation_does_not_converge
```

It uses `should_panic` with the exact convergence assertion. The only
production-source change is a `cfg(test)` entrypoint around the same inner
operation so the schedule can be made deterministic; the production wrapper
still executes the original read/generate/insert sequence. When the product
behavior is fixed, convert the characterization into a positive two-caller
regression and remove the test-only known-defect expectation.

## TC-NATIVE-001: required native menu and Open Media behavior are unproven

Status: **Open; strict harness rejects it; application code unchanged**

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

The new strict wrapper rejects missing menus, required skips, missing scenario
markers, unexpected stderr, open processes, schema drift, duplicate JSON keys,
producer/report contradictions, binary mutation, and hung process trees. Its
default path performs locked GUI and harness builds before starting the
watchdog; an explicitly supplied binary is bound by path and SHA-256 but is not
claimed to be fresh. It does not change the GUI implementation. Before making
native smoke a required CI lane, determine whether the missing menu is an
application accessibility defect, an unsupported presentation mode, or a
runner discovery defect, then prove the chosen contract on a trusted
interactive Windows worker.

Preserved experiment:

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

## TC-HARNESS-002: native baseline performs repeated placeholder DNS lookups

Status: **Open; reproducible; underlying configuration intentionally unchanged**

Severity: **Medium (test isolation and diagnostic-noise gap)**
Detection: captured native-runner stderr

The original 47.597-second baseline emitted 19 instances of the error below.
The final provenance-bound rerun emitted 20 and failed with the same contract:

```text
Session transport TCP address resolution for syncplay.example:8999 failed:
No such host is known. (os error 11001)
```

The legacy runner ignored this stderr and still returned success. The strict
contract now rejects it and preserves the complete stderr log, but this branch
does not change the saved configuration, transport behavior, or application.
A required native lane needs an explicit loopback endpoint and must demonstrate
that no external DNS or network dependency is attempted.

## TC-HARNESS-003: live Python semantic playlist observation is timing-sensitive

Status: **Open; intermittent; product and harness code intentionally unchanged**

Severity: **Medium (required semantic evidence can be flaky)**
Detection: preserved full semantic-suite rerun; later clean replay

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
passed 14/14 in 39.2 seconds. This is not enough evidence to classify the
cause as either an application race or a harness scheduling defect. The
failure remains recorded rather than repaired on this branch.

This lane is now required, but its reliability contract is incomplete.
Preserve per-scenario event timelines on every failure and replace timeout-only
playlist readiness with an explicit causal acknowledgement or deterministic
scheduler. A retry may classify a failure; it must not overwrite or convert
the first failed attempt into passing evidence.

## TC-HARNESS-004: intermittent CLI failure poisons the shared test lock

Status: **Open; intermittent; product and harness code intentionally unchanged**

Severity: **Medium (workspace and coverage evidence can fail nondeterministically)**
Detection: first full Windows cargo-llvm-cov run; reproduced by ordinary workspace tests

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

No retry result replaces the first failure. The evidence supports an
intermittent concurrency/test-isolation classification, but does not prove
whether the initial Plex timeline assertion race is in product behavior or
test setup. Preserve the first failing assertion and per-test environment
ownership in CI, avoid poisoning a process-wide lock after one test panics,
and reproduce on the Linux worker before making a product diagnosis.

## TC-HARNESS-005: cargo-llvm-cov emits contradictory LCOV line summaries

Status: **Mitigated for physical changed-line policy; LCOV contradiction remains open and its parser is intentionally unchanged**

Severity: **High for generic LCOV consumers; no longer blocks Sorotte's source-bound physical-line gate**
Detection: strict replay through the new critical-path ratchet

Both the preserved all-feature LCOV artifact and a fresh run on the pinned
toolchain fail structural validation before either the ordinary 80% or
critical 90% threshold is evaluated. The fresh command completed the entire
instrumented workspace successfully in 253.8 seconds:

```text
cargo llvm-cov --locked --workspace --all-features --lcov \
  --output-path target/fresh-diff-coverage.lcov
```

The fresh tracefile contains 392 source records. An independent record-by-
record audit found:

```text
records with any LF/LH mismatch: 309
records with an LF mismatch:     305
records with an LH mismatch:     256
records with both mismatched:    252
records with duplicate DA lines:   0
```

Its `crates/sorotte-cli/src/client_args/parser.rs` record repeats the original
contradiction exactly:

```text
LF:122
LH:75
unique DA records:       120
positive-hit DA records: 115
duplicate DA lines:      0
```

Across the complete fresh artifact, the declared summaries total
145,926 / 187,537 lines (77.81%), exactly matching LLVM's aggregate report.
The explicit positive/unique `DA` inventory instead totals
142,777 / 181,281 (78.76%). The disagreement is therefore stable, widespread,
and not explained by duplicate line entries. It is consistent with an
incompatibility between cargo-llvm-cov's merged generic-instantiation output
and strict LCOV consumers, but this experiment does not identify which
representation is semantically authoritative for every changed source line.

The LCOV parser therefore remains fail-closed, and no threshold result is
reported for either LCOV replay. The required Sorotte gate now uses a narrower
contract appropriate to a physical Git diff:

1. one successful locked all-feature profile run;
2. LLVM JSON exported with `--skip-functions` to attest the pinned producer,
   schema, file list, segments, and aggregate line-instance summary;
3. the native `llvm-cov show` text view to identify exact physical source-line
   execution;
4. a strict converter that compares every displayed row with the checkout and
   hashes both producer artifacts plus every source file;
5. a changed-line consumer that independently rechecks the canonical schema,
   source digests, line order, binary execution values, totals, and producer;
6. a finalizer that binds the base, profiles, JSON, text, line map, and policy
   as six named phases and cross-checks each artifact digest.

This separation follows the native tool boundaries: LLVM documents JSON export
as region/function/summary data and `show` as the annotated source view, while
the open LLVM exact-line export issue confirms that JSON does not currently
identify every physical line needed for this policy. See
[LLVM llvm-cov](https://www.llvm.org/docs/CommandGuide/llvm-cov.html),
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov), and
[LLVM issue 126307](https://github.com/llvm/llvm-project/issues/126307).

A fresh current-source experiment completed all instrumented workspace tests
in 250.9 seconds and then produced:

```text
LLVM JSON:  14,043,267 bytes
SHA-256:    e01b2c38ea017c16d6b29494ce6496099552fa01428b446ec1058b2c0693f104

native text: 13,643,322 bytes
SHA-256:     25ce18c6aed4d7d2c238e1db6303c08e76288b33a03aacacecded54bd55a900c

canonical map: 8,958,361 bytes
SHA-256:      74b428fe3688ed3b147648f0dd3db21f0f9ccdaa6344e90d64143b503cb5b541

unique physical lines: 145,272 / 183,106 = 79.337651%
LLVM line instances:   152,964 / 195,568 = 78.215250%
explicit retained delta: +12,462 total / +7,692 covered

tracked Rust diff: 32 / 32 executable lines = 100.00%
structural changed lines: 14
unmapped changed lines: 0
ordinary result: not applicable
critical result: passed
```

The compact complete phase artifact is 194,894 bytes with SHA-256
`06c796d2598945f4b39eddd9c953704d0c6f35947c02c6cf3b6014b346290da0`.
It passed all six phase and digest bindings. This does not declare LCOV fixed,
discard summaries, invent missing `DA` entries, or choose the more favorable
aggregate. It explicitly preserves two different line models and uses only
the source-bound physical model for physical changed-line policy.

## TC-HARNESS-006: updater self-replacement intermittently leaks an inherited handle

Status: **Open; intermittent; updater and test intentionally unchanged**

Severity: **High (the required workspace suite could silently green a leaked subprocess)**
Detection: pinned cargo-nextest 0.9.137 leak detection and diagnostic retry

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

The required workspace runner now fails both a leak and a pass-after-leak,
retains console/JUnit/policy artifacts, and rejects per-test attempts to weaken
the leak timeout. Root-causing or repairing the updater test is deliberately
outside this coverage branch. The sanitized run record is preserved in
[`docs/evidence/test-coverage/nextest-flake-leak-20260728.md`](evidence/test-coverage/nextest-flake-leak-20260728.md).

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
