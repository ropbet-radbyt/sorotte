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

## Verification-only lifecycle authority questions

- Severity: unclassified; static possibilities only.
- Baseline:
  `a47b6e035608bb03f1a1dd59986375653963b39a`
  (`Separate physical player lifecycle ownership`).
- Reachability: not observed in real use, real mpv, a captured transcript, the
  vertical deterministic harness, or a reducer/model failure at the time of
  inventory.
- Origin classification: defensive audit only.
- Disposition: do not change production behavior unless an executable
  adapter-to-consumer trace proves divergence.

The independent assignment audit originally recorded six questions. Full-stack
tests have since proved and corrected snapshot semantic inference,
`LocalFileChanged` duplicate authority, physical `LoadAttemptActive` semantic
inference, pre-file-loaded snapshot path confirmation, command-timeout physical
ownership loss, and missing logical-revocation delivery. Their exact failures
and authority corrections are recorded in
`docs/player-lifecycle-verification.md`.

The remaining static-only questions are:

1. Both consumers mark a binding physically terminal for semantic failure
   outcomes as well as for an explicit `LoadAttemptTerminal` event.
2. The GUI mirrors ordered command outcomes into its legacy
   playlist-resolution command-progress handler, which can update resolution
   state without a `LoadAttemptId`.
3. The GUI's attempt-fenced ordered media-outcome bridge delegates failure
   presentation to target-string-keyed legacy handling; no stale
   same-target failure divergence is currently executable.
4. The acknowledged-mode no-legacy-drain test instruments one representative
   legacy getter rather than all compatibility getters.
5. GUI refresh assumes `PlayerEventDeliveryMode` stays stable for an
   attachment; production mpv does, but the public adapter contract does not
   state that invariant explicitly.

The authority inventory and evidence gate for these questions are maintained in
`docs/player-lifecycle-verification.md`.
