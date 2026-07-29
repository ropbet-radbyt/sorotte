# Test coverage implementation findings

Original review: 2026-07-28
Lean-fix implementation update: 2026-07-29

Branch: `codex/test-coverage-design`

Original experimental base: `a08a06ea7c6cada5413b0dba73b16f940cfd46e1`
Current rebased base: `f3964ebc7f7b281b9b78f3bfb243ff65e5122e33`

This ledger separates product findings from failures in the new test
infrastructure. The original review deliberately left surfaced defects
unchanged. The 2026-07-29 update implements the non-controversial lean
solutions, applies the subsequently selected lifecycle and native-GUI
decisions, and converts every product-defect characterization into a positive
regression. The known-defect registry is now empty.

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
  passed on Windows before the rebase, on the rebased 0.2.4 tree in 27.2
  seconds, after the lean fixes in 8.54 seconds, and on the final tree in 7.33
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
- The behavior catalog validates 13 behavior IDs, 25 exact proofs, and two
  lanes. The known-defect registry now validates as empty.
- The ignored-test registry exactly classified all 25 source attributes:
  4 required pull-request proofs, 7 fixture-maintenance commands, 12 manual
  capability tests, and 2 expiring compatibility quarantines.
- The changed-line utility now passes all 71 LCOV/diff-policy cases; the
  canonical-map consumer passed 9 additional adversarial cases, the native
  converter passed 14, and the six-phase finalizer passed 19. Coverage includes immutable
  base/head critical-policy union, policy-deletion downgrade prevention,
  inline `#[cfg(test)]` denominator dilution, adversarial Rust lexical tokens,
  new-tag, updated-tag, missing-base, merge-base, provenance, and
  partial-phase failure contracts.
- Deterministic protocol ordering passed 3 new permutation/adversarial tests;
  all 6 production-worker framed IPC tests passed; all 35 server persistence
  tests pass, including positive atomic-migration and concurrent-secret
  convergence regressions.
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
