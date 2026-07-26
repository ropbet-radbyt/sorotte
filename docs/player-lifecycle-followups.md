# Player lifecycle stabilization follow-ups

This file records defects discovered while implementing player lifecycle
stabilization that are outside the branch's ownership scope. They are not fixed
here unless they block lifecycle validation or present immediate data-loss or
security risk.

## Live Python peer readiness smoke timeout

- Severity: medium (test reliability/interoperability signal).
- Baseline: `fe80cc75f2c2933b75298f865e2d528bcf73adfb`, before lifecycle production
  changes.
- Reproduction:

  ```text
  cargo test -p sorotte-gui --all-features runtime_owner -- --test-threads=1
  ```

- Result: 438 passed, 1 failed, 1 ignored, 558 filtered in the selected group.
- Failing test:
  `app::smoke_tests::live_python_smoke::gui_persisted_config_runtime_owner_projects_live_python_peer_shared_playlist_open_interop`.
- Isolated retry: failed again.
- Trace: the smoke timed out waiting for peer readiness; the GUI self user was
  ready while the Python peer remained not ready.
- Likely owner: live Python interoperability/runtime smoke harness, not mpv
  attachment, physical load-attempt, or ordered player-event ownership.
- Disposition: fixed here because it deterministically blocked the mandatory
  workspace test gate. The synthetic GUI player reported an unpaused open even
  though the real managed-mpv launch contract starts paused; that invented a
  native Play gesture and promoted the local user to Ready. The harness now
  mirrors the real paused startup. The failing scenario and all five live-Python
  GUI smoke flows pass.

## Plex server-selection test lacked persistent storage

- Severity: low (test harness reliability).
- Reproduction:
  `cargo test -p sorotte-gui --all-features selecting_plex_server_clears_stale_server_scoped_workers -- --nocapture`.
- Trace: the test constructed the runtime owner without a writable config path,
  so the intentionally fallible settings-persistence step returned before
  server-scoped worker invalidation. The assertion then observed the deliberately
  untouched stale receiver.
- Likely owner: GUI Plex runtime-owner test harness, not player lifecycle.
- Disposition: fixed here because it deterministically blocked the mandatory
  workspace test gate. The test now uses a test-local config path; production
  behavior is unchanged. The isolated test and all 24 Plex runtime-owner tests
  pass.

## GUI action-count tests admitted unrelated startup hydration

- Severity: low (parallel test reliability).
- Reproduction:
  `cargo test -p sorotte-gui --all-features` intermittently added a public-server
  hydration failure action to exact chat/player action-count assertions; the TCP
  chat test also failed deterministically when the remote fetch completed during
  its echo wait.
- Trace: the unexpected chat row was
  `Startup public-server hydration failed: ...`, and the resulting assertion
  panic dropped the loopback server release channel.
- Likely owner: GUI runtime-owner test fixtures, not player lifecycle or chat
  transport.
- Disposition: fixed here because the failures blocked the mandatory workspace
  gate. The affected isolated fixtures now use the documented explicit-empty
  public-server cache, keeping remote startup work outside their action streams.
  Both chat transports pass, including 100 repeated TCP runs, and the attached
  player projection test passes in isolation.

## CLI ping-metrics fixture aborted parallel Windows connections

- Severity: low (parallel test reliability).
- Reproduction: `cargo test --workspace --all-features` failed consistently,
  while the exact ping-metrics test passed in isolation.
- Trace: the synthetic server shut down its write half and immediately dropped
  a socket that still had unread client writes. Windows surfaced that teardown
  race to the client as `WSAECONNABORTED` (`os error 10053`) instead of EOF.
- Likely owner: CLI loopback test fixture, not player lifecycle or production
  session transport.
- Disposition: fixed here because it blocked the mandatory workspace gate. The
  server now drains the peer after announcing EOF, with a bounded timeout.

## Reviewed lifecycle authority questions

- Baseline:
  `a47b6e035608bb03f1a1dd59986375653963b39a`
  (`Separate physical player lifecycle ownership`).
- Origin classification: defensive assignment audit only.
- Reachability: none of these questions produced a failure in real use, real
  mpv, transcript replay, the vertical deterministic harness, generated
  histories, or the reducer/model.
- Independent review: 2026-07-26, against
  `fe18a43bf4b6588511e0c87b8c29366a4cdd1769`.
- Overall disposition: no executable P0/P1 defect remains. Questions 1, 2, and
  4 are closed; question 3 is retained as a nonblocking hardening test;
  question 5 is a P3 API-contract clarification and is documented in
  `PlayerAdapter`.

The assignment audit originally recorded additional questions. Full-stack
tests proved and corrected snapshot semantic inference, `LocalFileChanged`
duplicate authority, physical `LoadAttemptActive` semantic inference,
pre-file-loaded snapshot path confirmation, command-timeout physical ownership
loss, and missing logical-revocation delivery. Their exact failures and
authority corrections are recorded in
`docs/player-lifecycle-verification.md`.

### 1. Semantic failure outcomes and physical terminality

**Answer: safe under the current reducer contract; no production change
required.**

The consumers mark a binding physically terminal when they receive `Failed`,
`NeverStarted`, or `TransportDisconnected`, as well as when they receive an
explicit `LoadAttemptTerminal` event. Those semantic results are not generic
command failures:

- `Failed` and `TransportDisconnected` are produced from
  `commit_physical_attempt_terminal`;
- synchronous rejection emits `NeverStarted` in the same reducer transition
  that commits the physical `NeverStarted` terminal state; and
- `Superseded` and `Indeterminate`, whose physical effects may still arrive,
  do not mark the consumer binding terminal.

The consumer mark is therefore a redundant projection of a reducer-owned fact,
not an independent terminality decision. It also preserves the terminal
projection when recovery compacts an explicit telemetry event while retaining
the semantic result. A future helper such as
`PlayerLoadAttemptResult::implies_physical_terminal` could centralize the
mapping, but it is maintainability work rather than a merge requirement.

**Disposition: closed as answered.**

### 2. Ordered command outcomes through the legacy GUI progress handler

**Answer: safe as a presentation bridge; it is not a second physical ownership
authority.**

Before the existing playlist-resolution handler mutates state, it requires the
exact player command ID and media generation stored by the current resolution
attempt. Its outputs are presentation and fallback states (`Active`,
`Indeterminate`, `Failed`, or `Superseded`); it does not select the physical
transport owner, active load attempt, playlist-entry owner, or physical
terminality. In particular, `CompletionNotObserved` becomes `Indeterminate`,
not a definitive load failure.

The reducer still owns the single attachment-scoped semantic terminal result.
Passing `LoadAttemptId` through this bridge could make that relationship more
explicit, but is not required for correctness at the current boundary.

**Disposition: closed as answered; optional later simplification only.**

### 3. Attempt-fenced media failures and target-string presentation

**Answer: acceptable for the current acknowledged mpv producer; retain one
nonblocking hardening test.**

The ordered GUI consumer verifies attachment epoch, load-attempt ID, media
generation, and associated command identity before it constructs a legacy
media-load failure. Only that already-fenced result delegates presentation to
the target-oriented handler. Reducer semantic results are write-once, and
successor acceptance emits logical revocation plus `Superseded` where
applicable, so an ordinary stale same-target attempt cannot manufacture a
second semantic failure against its successor.

The remaining theoretical sequence is deliberately narrower: attempt A has a
queued failure for target T, attempt B becomes the current resolution attempt
for T, and A's queued outcome is applied before B's attempt identity is
attached. Production scheduling and the verification harness did not reproduce
that sequence. A future deterministic test should construct it directly. If it
fails, the narrow correction is an attempt-keyed
`handle_ordered_playlist_load_failure(load_attempt_id, media_generation,
command_id, failure)` path, while retaining a separate synchronous-rejection
path for the interval before an attempt is bound.

**Disposition: nonblocking hardening test; not an open merge defect.**

### 4. Representative no-legacy-drain poison getter

**Answer: a test-coverage limitation, not evidence of duplicate production
authority.**

Acknowledged GUI refresh selects the batch path and returns after draining it;
it does not continue into the legacy command, playback, transport, media-load,
local-file, or observation getters. The mpv acknowledged producer likewise
does not use those compatibility queues to establish lifecycle ownership.

A stronger test adapter may panic from every legacy lifecycle getter while
advertising `OrderedAcknowledgedBatches`, and should be exercised through both
GUI and client-core refresh paths. That would comprehensively lock in the
no-mixed-mode rule without changing production behavior.

**Disposition: closed as nonblocking test debt.**

### 5. `PlayerEventDeliveryMode` stability

**Answer: the mode is stable for an attachment; the current mpv implementation
is correct.**

The public `PlayerAdapter` contract now states that the delivery mode must
remain constant for the lifetime of an attachment. Changing mode requires a new
attachment epoch or an equivalent explicit consumer reset, and an adapter must
not expose lifecycle ownership through both modes within one attachment.

Sampling the mode once per refresh remains a possible cleanup, but repeated
reads are safe under the explicit contract and do not justify changing the
verified lifecycle implementation.

**Disposition: P3 API hardening documented; not a merge blocker.**
