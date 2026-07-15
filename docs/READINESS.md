# Readiness and automatic start

Sorotte readiness has two independent inputs:

- **User intent** is the participant's last deliberate Ready or Not Ready choice. The GUI control, CLI commands, and intentional player Play/Pause gestures can change it.
- **Technical playability** describes whether the current media generation is preparing, playable, temporarily blocked, or terminally blocked.

Loading, seeking, buffering, recovery, EOF, media refreshes, playlist transitions, and synchronization corrections never rewrite user intent. A failed player command also leaves the deliberate intent intact and is reported as a technical blocker.

## Automatic starts

For peers that negotiate `sorotteReadinessV2` and `sorottePlaybackBarrierV1`, the server owns the start decision. The default policy waits for every required participant to be Ready and technically playable for the same media generation. The server binds the commit to the evaluated readiness revision and broadcasts one canonical start; clients do not independently unpause from a local readiness snapshot.

Playlist skips and automatic advancement preserve user intent, create a new technical generation, and enter the same gate. Replaying the current item creates a fresh replay episode rather than locally rewinding past the gate.

Pause ownership prevents automatic systems from resuming an unrelated user pause. A readiness gate, buffering policy, or recovery flow may release only a pause that it owns.

## Mixed-version rooms

V2 participants without the playback-barrier capability and legacy peers are explicitly exposed as excluded legacy clients when a V2-governed start cohort is active. Their legacy Ready value remains visible, but the UI does not claim generation-scoped technical guarantees for them. Rooms using only the legacy protocol retain the previous compatibility behavior.

## Controls and status

The GUI shows pending local intent separately from the server-confirmed value and distinguishes states such as `Ready — buffering`, `Ready — recovery in progress`, and `Not Ready — technical failure`.

The CLI prints a deduplicated status line when a V2 participant changes, separating pending and canonical intent, technical phase and recovery, room/start eligibility, and cohort role; legacy rooms keep their existing output.

The CLI accepts:

- `ready` and `not-ready` for direct readiness changes;
- `play` and `pause` for intentional playback gestures;
- `p` as the existing pause toggle.

On reconnect to the same room membership, Sorotte preserves the latest acknowledged intent and reconciles it by operation identity and server revision. Joining a different room starts a fresh membership that defaults to Not Ready.
