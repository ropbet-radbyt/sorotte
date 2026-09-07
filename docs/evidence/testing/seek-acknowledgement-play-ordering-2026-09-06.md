# Delayed Seek acknowledgement and newer Play intent

The [minimum-mpv job in run 34019076048, attempt 1](https://github.com/ropbet-radbyt/sorotte/actions/runs/34019076048/job/101448621315)
passed all four player semantics tests but failed its real-player system
lifecycle after 29 successful checks. That lifecycle used the built CLI and
server binaries; it did not consume package archives. Its source was the
prospective merge
`b40ea94f7689d70852c692e90677e26b332b1235` for PR head
`3bee8c3315031b38fc320f121da6ee1e2f211ef1`.

The failure occurred while waiting for the canonical Play immediately after a
near-tail Seek on the final, 14-second item. It did not reach the natural-EOF
assertion. The retained trace records:

| Elapsed seconds | Observation |
|---|---|
| 53.735 | Server commits Seek to 11.0 seconds, paused, transport revision 35. |
| 53.745 | Harness issues Play to the controller. |
| 53.749 | Real controller mpv reports playing at 11.0 seconds. |
| 53.952 | Real controller mpv reports paused at 11.2 seconds. |
| 61.770 | Canonical Play wait expires; server is still paused at revision 35. |

The client, server, mpv, FFmpeg and both fixture files have identical SHA256
digests in this failure and the preceding successful
[run 34016574883](https://github.com/ropbet-radbyt/sorotte/actions/runs/34016574883).
An isolated clean-head WSL replay also passed without changing the harness.
Those successes do not replace the original failed attempt or establish that
another unchanged retry would be safe.

## Deterministic reproduction

Two test-only reproductions retain the original production bytes. First,
adding actual transport revisions 34 and 35 to the existing legacy
seek-preparation/Play regression makes its no-Pause assertion fail. Second,
an ordered runtime sequence exercises the emitted command and acknowledgement:

1. Begin paused at canonical revision 34 with an attached player.
2. Issue Seek to 11.0 seconds and retain its actual client-ignore counter.
3. Issue Play before that Seek acknowledgement is processed. The immediate
   heartbeat has no playstate while acknowledgement is pending.
4. Receive the earlier self Seek acknowledgement at revision 35 with its exact
   counter, while ordered player observations show the newer Play.
5. Reconcile the player again.

Unmodified production drops the pending Play and issues `SetPaused(true)`.
The newer intent is still bound to revision 34 when the earlier Seek advances
canonical state to revision 35. The existing untagged test did not cover this
revision transition.

The hosted safe trace does not include controller outgoing wire frames, so it
does not prove the controller event loop's precise interleaving. The independent
runtime reproduction proves this lost-intent defect and the same physical Pause
outcome. The lifecycle assertions and eight-second canonical-state deadline
remain unchanged.

## Correction and evidence

A newer local pause/play intent may survive only the accepted acknowledgement
of its own earlier emitted Seek. Correlation must include the room, connection,
media, base revision, seek target, paused value and client-ignore counter. The
session must actually accept the next canonical revision before rebasing that
intent. Unrelated authority, rejected or repeated acknowledgements, later seeks,
failed dispatch and context changes must not grant this exception. The existing
requirement for current player observations before outbound state remains in
force.

The correction lives in
[`local_seek.rs`](../../../crates/sorotte-client-core/src/runtime/playback_coordination/local_seek.rs).
It captures the emitted Seek, binds a later intent to it, and finishes admission
after session reconciliation in both ordinary and ping-only paths. Rejected
traffic preserves a still-valid predecessor; accepted unrelated authority
retires it. A bounded counter watermark prevents reused or saturated legacy
counters from receiving the exception. Ambiguous rapid-Seek schedules retain
the existing behavior rather than gaining unproven acknowledgement authority.

The [focused regressions](../../../crates/sorotte-client-core/src/runtime/playback_coordination/tests/seek_echo_tests.rs)
exercise both Play and Pause delivery, every recorded wire identity, rejected
then accepted packets, context changes, repeated and failed commands, and counter
reuse. Later valid Seeks and an already-admitted acknowledgement are included:
the first mutation run exposed missing assertions for those sequences. That
original failed mutation attempt is retained at
`target/verification/seek-echo-mutation-attempt-1/`. Duplicate packet/canonical
predicates were consolidated into one check of accepted canonical state; the
original counter and preceding revision still have to be captured before
reconciliation.

The `client-local-seek-echo` mutation responsibility covers the whole
correlation module with those selected tests and requires 100% viable kills,
zero survivors and zero timeouts. Its results must be established separately
from the earlier 1,429-mutant campaign.

The final local source overlay passed 21 focused regressions, 1,615 affected
tests, the original ordered reproducer without changing its bytes, and all 39
real minimum-mpv lifecycle checks. Its second complete mutation attempt caught
all 72 viable mutations; the remaining generated replacement failed to compile
because `LocalSeekEchoCandidate` has no `Default`. That exact compiler outcome
is independently reviewed and recorded in policy. There were no survivors or
timeouts. `target/verification/seek-play-repair/repair-closure.json` binds the
nine source files and raw evidence, including the separate mutation review.
This local overlay on `c8f719cb268f824e10764c86d656a6aa538926bf` is development
evidence; the committed candidate still requires fresh hosted qualification.

Original logs, official artifact 9984962184 and its digest, binary/fixture
comparisons, the passing unchanged replay and both failing deterministic
reproductions are retained separately under
`target/verification/hosted/3bee8c33/mpv-minimum-failure-attempt-1/`.
`failure-review.json` binds their individual receipts and source identities.
Later repaired-source validation must identify its own source and attempts.

## Same-room request assertion and mutation selection

The complete campaign on `c4688afbd39d642e1d5597a3194f81307abb42b0`
retained one survivor in `client-participant-status-runtime--3-of-4`:
replacing `!=` with `==` in `begin_participant_status_room_switch`.
The existing public A-to-B-to-A regression passes on both versions: the
original clears the earlier Seek when leaving A, while the mutation clears it
when requesting A again. That sequence cannot distinguish the inverted guard.

An isolated replay of the exact retained mutation establishes the distinguishing
case. After Seek followed by Play, a request for the current room must preserve
the earlier Seek's correlation. The unmodified implementation then delivers the
newer Play on its ordinary heartbeat after acknowledging the Seek. The mutated
implementation clears the correlation and fails that outgoing-protocol
assertion. This is evidence of a missing assertion, not another shipped product
defect.

The permanent same-room regression checks the outgoing playing state, revision
and absence of a duplicate Seek. The original roundtrip test and its name remain;
shared assertion bodies also run through a `participant_status_` test so the
existing participant-status mutation selection exercises both public sequences.
Product code, mutation selection policy and kill requirements are unchanged.

The original-pass/mutant-fail replay, original passing roundtrip on both versions,
source hashes and exact mutation diff are retained in
`target/verification/hosted-mutation-fuzz/c4688afb/room-switch-survivor-reproduction-attempt-1/`.
The original hosted campaign remains failed; later validation is recorded as a
separate attempt.
