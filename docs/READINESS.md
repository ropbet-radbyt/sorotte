# Readiness and automatic start

Sorotte readiness has two independent inputs:

- **User intent** is the participant's last deliberate Ready or Not Ready choice. The GUI control, CLI commands, and intentional player Play/Pause gestures can change it.
- **Technical playability** describes whether the current media generation is preparing, playable, temporarily blocked, or terminally blocked.

Loading, seeking, buffering, recovery, EOF, media refreshes, playlist transitions, and synchronization corrections never rewrite user intent. A failed player command also leaves the deliberate intent intact and is reported as a technical blocker.

## Automatic starts

For peers that negotiate `sorotteReadinessV2` and `sorottePlaybackBarrierV1`, a coordinated-start policy gives the server the start decision. The all-eligible policy waits for every required participant to be Ready and technically playable for the same media generation. Generic playability and barrier-target readiness are independent evidence: a participant must also confirm that the exact prepare revision, target seek, and logical pause were applied. The server binds the commit to the evaluated readiness revision and broadcasts one canonical start; clients do not independently unpause from a local readiness snapshot.

Standard uses immediate start. It still establishes a technical generation so player failures and recovery appear in readiness, but technical status does not install a start gate or change canonical pause state. Player evidence remains bound to the locally prepared playlist selection: observations or command failures from a predecessor cannot become readiness or buffering reports for a successor while its source is being resolved. Replaying the same row also fences predecessor evidence until the new preparation or physical reset is complete.

Playlist skips, replay, replacement, and automatic advancement preserve user intent and retire the predecessor's start gate, technical evidence, and buffering pause ownership. A fresh prepare establishes the successor generation and follows the selected start policy. Editing an unselected playlist entry preserves the active generation.

Pause ownership prevents automatic systems from resuming an unrelated user pause. A readiness gate, buffering policy, or recovery flow may release only a pause that it owns.

## Mixed-version rooms

V2 participants without the playback-barrier capability and legacy peers are explicitly exposed as excluded legacy clients when a V2-governed start cohort is active. The default `RequireAllMembers` mixed-room policy blocks automatic start and reports `UnsupportedParticipant`, preserving the all-members contract. `ExcludeUnsupported` remains an explicit compatibility opt-in. Legacy Ready values remain visible, but the UI does not claim generation-scoped technical guarantees for those peers. Rooms using only the legacy protocol retain the previous compatibility behavior.

## Controls and status

The GUI shows pending local intent separately from the server-confirmed value and distinguishes states such as `Ready — buffering`, `Ready — recovery in progress`, and `Not Ready — technical failure`.

The CLI prints a deduplicated status line when a V2 participant changes, separating pending and canonical intent, technical phase and recovery, room/start eligibility, and cohort role; legacy rooms keep their existing output.

The CLI accepts:

- `ready` and `not-ready` for direct readiness changes;
- `play` and `pause` for intentional playback gestures;
- `p` as the existing pause toggle.

The server accepts technical reports only for the current membership epoch and in strictly increasing report-sequence order. Once playback has a server state revision, reports must carry that authoritative revision; a client-local coordinator revision is never accepted as a substitute. Ready/Not Ready compare-and-set uses the participant's user-intent revision, so unrelated technical or pause-owner changes do not create intent conflicts.

A participant joining current playback receives its retained prepare, commit revision, and lifecycle phase even after coordinated start completes. This supplies the revision for fresh technical evidence without replaying the historical start. A retired playlist selection is not restored by this snapshot.

On reconnect, a client presents the opaque continuity token issued in the server Hello or its private readiness membership snapshot. A valid token restores acknowledged user intent, its revision, operation idempotency, and the technical ordering baseline. Transient technical playability and barrier readiness always reset to Preparing/Pending and require fresh player evidence. A missing or invalid token—even with the same display name—creates a fresh membership that defaults to Not Ready. Joining a different room also starts a fresh membership and issues a new token scoped to that room and membership epoch, so later reconnects preserve intent acknowledged there.
