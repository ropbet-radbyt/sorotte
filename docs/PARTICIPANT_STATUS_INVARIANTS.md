# Participant Status Invariants

`sorotteParticipantStatusV1` crosses player, client, server, transport, and GUI boundaries. The following laws are release requirements, not presentation preferences.

## Authority laws

1. Participant status is advisory. Applying any valid, stale, malformed, or malicious status report must not change canonical playstate, readiness, playback-barrier state, membership, or player commands.
2. Identity and room come only from the authenticated server session. A report has no authority to name either.
3. Precise position, buffer, and offset fields are shown only for the current status epoch. A connection generation, room, capability, media generation, state revision, or transport revision change invalidates retained evidence atomically.
4. A scope-only update clears or re-correlates the prior snapshot immediately. An older snapshot revision cannot roll authoritative scope backward.

## Time laws

1. Player observation time, server receipt time, and UI projection time are distinct.
2. Sparse position telemetry carries a field-specific age alongside the report-wide oldest-evidence age. Position projection uses position observation age, never the oldest or newest unrelated field age.
3. Server projection includes only a validated forward-delay estimate. Without trustworthy correlation or delay evidence, room offset is absent.
4. Clock rollback fails closed: received status continues aging and heartbeat scheduling continues from a monotonic baseline.
5. Missing or invalid server report age cannot establish Sorotte freshness. Missing or invalid player-sample age cannot establish media-evidence freshness, but lifecycle-only reports remain fresh while their report heartbeat is current. Stale status never regains detail merely because a later layer lacks an age.

## Lifecycle laws

1. Player lifecycle evidence outranks old telemetry. Starting, disconnected, failed, and unavailable cannot be promoted by a late observation from an earlier adapter epoch.
2. Player failure does not imply Sorotte transport failure. CLI and GUI keep room membership, publish terminal player status, and retry attachment independently.
3. Every status removal invalidates the room snapshot cache in the same operation. No caller may erase retained status directly.
4. Capability withdrawal cancels queued and leased status extensions before retry.

## Delivery laws

1. Enqueue success is not delivery. A dependent player open waits for the terminal receipt of its causal playlist frame.
2. Later unrelated frames cannot delay a satisfied causal fence, and background reconciliation cannot bypass a pending fence.
3. Periodic status is coalescible and bounded. Overflow degrades explicitly from full to compact to unavailable without entering a reliable control queue.

## Required test matrix

Every change to participant status or an adjacent queue/lifecycle path must cover the affected row at the lowest layer and preserve the cross-layer acceptance test.

| Boundary | Required evidence |
| --- | --- |
| Protocol | Round trips, missing fields, unknown/malformed enum values, numeric bounds, frame-size fallback |
| Client session | Capability and room gating, monotonic snapshot revision, scope-only transition, reconnect, clock rollback, stale redaction |
| Player runtime | Every coarse phase, sparse-field ages, adapter epoch fencing, heartbeat rollback, advisory non-interference |
| Server | Authenticated attribution, sequence reset, lifecycle cache invalidation, exact correlation, delay-aware offset, full/compact/unavailable bounds |
| Transport | Leased cancellation, write failure/retry, causal delivery fence with later traffic, no global-queue starvation |
| CLI | Attach, telemetry, detach, failure containment, reattach while membership persists |
| GUI | Current-room filtering, typed status tone, stale/terminal tooltip, semantic room summary, native accessibility names |
| End to end | Client A report -> server retention/projection -> client B model -> presentation, with canonical state unchanged |

## Verification apparatus

- Run all-feature workspace tests and strict Clippy.
- Run GUI semantic and native accessibility smoke for visible changes.
- Run the protocol, client acceptance/reporting, server, client-app lifecycle, CLI lifecycle, GUI presentation, and causal delivery-fence participant-status mutation shards. Relevant behavior shards run for participant-status pull requests; the complete matrix remains a weekly/manual gate. Verify every retained report against the final source hashes before handoff. Retain generated reports below `target/`; never keep `mutants.out` in the worktree.
- Run the protocol and framed-session fuzz policies. The framed-session seed corpus must include negotiated participant reports, snapshots, capability withdrawal, and malformed status values.
- Treat a public Rust API change separately from an additive wire change. New extensible public status types use constructors/builders; downstream compile coverage must be updated before adding required fields or exhaustive enum assumptions.

## Decision index

- [`../CONTEXT.md`](../CONTEXT.md) defines the shared domain vocabulary.
- [`adr/0001-advisory-participant-status.md`](adr/0001-advisory-participant-status.md) records why status is advisory and authenticated.
- [`adr/0002-delivery-fenced-player-effects.md`](adr/0002-delivery-fenced-player-effects.md) records why player effects wait for exact transport receipts.
