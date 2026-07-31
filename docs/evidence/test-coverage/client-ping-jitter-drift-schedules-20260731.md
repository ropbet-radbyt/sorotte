# Client ping jitter, drift, and playback schedules — 2026-07-31

## Scope and source state

This slice addresses the client ping/time gap recorded in
`docs/TEST_COVERAGE_STRATEGY.md`: explicit-time arithmetic already had strong
examples and a zero-survivor mutation shard, while monotonicity, offset/drift
assumptions, jitter, scheduler latency, and resulting playback outcomes lacked
one deterministic schedule.

The audit began on `codex/test-coverage-design` at
`2e6746b4a0ec4fdee2bbe09328161f064d5ca772`. The slice remained uncommitted
while this draft was written so the parent pass can review and commit it
independently.

Files:

- `crates/sorotte-client-core/src/session/tests/ping_jitter_drift_schedule_tests.rs`
- `crates/sorotte-client-core/src/session/tests.rs`
- `docs/evidence/test-coverage/client-ping-jitter-drift-schedules-20260731.md`

No production code changed. Existing production APIs already accept explicit
ping-observation time, separate receipt/reply/ping clocks, explicit room
projection time, and a direct room-playstate desynchronization decision.

## Boundary

- Every timestamp, RTT sample, scheduler delay, room position, and playback
  observation is synthetic and passed explicitly.
- The tests use no sleeps, timers, processes, sockets, external network,
  credentials, persistence, or privilege boundary.
- An independent test oracle uses the literal legacy `0.85/0.15` smoothing
  rule and literal playback thresholds. It does not import the corresponding
  production constants.
- The tests make no cross-host clock-authenticity claim. They characterize
  arithmetic over received values and explicit local timestamps only.

## Deterministic schedules

Four top-level tests cover:

1. Eight ordered ping observations: baseline, moderate jitter, a large finite
   outlier, recovery, a backward wall-clock step with a valid same-sample RTT,
   a future echo after that step, a non-finite receive clock, and a negative
   server RTT. Every accepted sample matches the reference model; every
   rejected sample preserves all prior metrics atomically.
2. Twenty-seven affine-clock observations across three common offsets, three
   local clock rates, and three RTT samples. Common offset is metamorphically
   invariant. Local rate drift scales the measured client RTT, while the
   received server RTT remains an independent duration.
3. Six reply-clock schedules: zero, 20-millisecond, and 750-millisecond
   scheduler delay; backward and non-finite reply clocks; and paused playback.
   A full reconcile then proves room state is anchored at receipt, ages only by
   positive observation time, and adds forward delay exactly once.
4. Eight playback-decision steps compared with a narrow independent state
   model: immediate rewind, scheduler-projected rewind suppression, slowdown,
   scheduler-projected slowdown suppression, the start and completion of a
   sustained fast-forward window, `doSeek` suppression, and paused-room rewind
   using the raw target.

The production path exercised includes:

- `ClientPingMetricsLegacyCompatible::observe_inbound_state_at`;
- `run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at_clocks`;
- `adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible`;
- `current_room_playstate_legacy_ping_compatible_at`; and
- `runtime_actions_for_desync_correction_against_room_playstate`.

## Validation

Focused schedule:

```text
cargo test --locked -p sorotte-client-core --all-features ping_jitter -- --nocapture
```

Result: 4 passed, 0 failed, 0 ignored; 724 tests filtered.

Complete crate:

```text
cargo test --locked -p sorotte-client-core --all-features
```

Result: 728 passed, 0 failed, 0 ignored. Doc tests also passed. The unit suite
completed in 0.28 seconds after compilation.

Warning-denied lint:

```text
cargo clippy --locked -p sorotte-client-core --all-targets --all-features -- -D warnings
```

Result: passed.

Scoped `rustfmt --check` and `git diff --check` passed.

The first focused compile found two mistakes confined to the new test code: an
unconstrained float literal and a playback helper called on the runtime rather
than its session-update wrapper. Both were corrected before behavioral
execution. No production or harness RED was observed.

## Findings and limitations

No product or test-harness defect was found.

- A common clock offset cancels because the client subtracts its echoed local
  send time from its local receive time. Local clock-rate drift does not cancel;
  the resulting bias is measured here, not detected or corrected.
- `serverRtt` is treated as a received duration. The client cannot prove its
  remote clock source, offset, rate, path symmetry, or authenticity from this
  payload, and this evidence makes no such claim.
- A backward wall-clock step is accepted when the individual echoed sample
  still has a nonnegative RTT. An echo later than its observation is rejected
  atomically. The slice does not introduce a monotonic-clock type.
- Finite outliers are smoothed by the legacy moving average; they are not
  classified or discarded. The evidence proves compatibility behavior, not
  statistical optimality.
- Scheduler projection is deterministic through 750 milliseconds, but no
  executor scheduling, operating-system delivery latency, or network telemetry
  is measured.
- Playback schedules cover only currently exposed legacy rewind, slowdown,
  sustained fast-forward, pause, and `doSeek` decisions. Cache recovery,
  reconnect validation, readiness barriers, and physical player
  acknowledgement remain covered by their separate client/player suites.
