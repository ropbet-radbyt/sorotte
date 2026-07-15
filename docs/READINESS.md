# Readiness and automatic start

Sorotte readiness has two independent inputs:

- **User intent** is the participant's last deliberate Ready or Not Ready choice. The GUI control, CLI commands, and intentional player Play/Pause gestures can change it.
- **Technical playability** describes whether the current media generation is preparing, playable, temporarily blocked, or terminally blocked.

Loading, seeking, buffering, recovery, EOF, media refreshes, playlist transitions, and synchronization corrections never rewrite user intent. A failed player command also leaves the deliberate intent intact and is reported as a technical blocker.

## Automatic starts

For peers that negotiate `sorotteReadinessV2` and `sorottePlaybackBarrierV1`, the server owns the start decision. The default policy waits for every required participant to be Ready and technically playable for the same media generation. Generic playability and barrier-target readiness are independent evidence: a participant must also confirm that the exact prepare revision, target seek, and logical pause were applied. The server binds the commit to the evaluated readiness revision and broadcasts one canonical start; clients do not independently unpause from a local readiness snapshot.

Playlist skips and automatic advancement preserve user intent, create a new technical generation, and enter the same gate. Replaying the current item creates a fresh replay episode rather than locally rewinding past the gate.

Pause ownership prevents automatic systems from resuming an unrelated user pause. A readiness gate, buffering policy, or recovery flow may release only a pause that it owns.

## Mixed-version rooms

V2 participants without the playback-barrier capability and legacy peers are explicitly exposed as excluded legacy clients when a V2-governed start cohort is active. The default `RequireAllMembers` mixed-room policy blocks automatic start and reports `IncompatibleLegacyParticipant`, preserving the all-members contract. `ExcludeLegacy` remains an explicit compatibility opt-in, and `AskController` fails closed until a policy choice is made. Legacy Ready values remain visible, but the UI does not claim generation-scoped technical guarantees for those peers. Rooms using only the legacy protocol retain the previous compatibility behavior.

## Controls and status

The GUI shows pending local intent separately from the server-confirmed value and distinguishes states such as `Ready — buffering`, `Ready — recovery in progress`, and `Not Ready — technical failure`.

The CLI prints a deduplicated status line when a V2 participant changes, separating pending and canonical intent, technical phase and recovery, room/start eligibility, and cohort role; legacy rooms keep their existing output.

The CLI accepts:

- `ready` and `not-ready` for direct readiness changes;
- `play` and `pause` for intentional playback gestures;
- `p` as the existing pause toggle.

The server accepts technical reports only for the current membership epoch and in strictly increasing report-sequence order. Once playback has a server state revision, reports must carry that authoritative revision; a client-local coordinator revision is never accepted as a substitute. Ready/Not Ready compare-and-set uses the participant's user-intent revision, so unrelated technical or pause-owner changes do not create intent conflicts.

On reconnect, a client presents the opaque continuity token issued in the server Hello. A valid token restores acknowledged user intent, its revision, operation idempotency, and the technical ordering baseline. Transient technical playability and barrier readiness always reset to Preparing/Pending and require fresh player evidence. A missing or invalid token—even with the same display name—creates a fresh membership that defaults to Not Ready. Joining a different room also starts a fresh membership.
