# Sorotte Synchronization

Sorotte coordinates a room's shared playback intent while keeping each participant's local player observations distinct from room authority.

## Language

**Canonical room playstate**:
The server-owned room position, pause state, and playback authority that may direct participants' players.
_Avoid_: Peer status, observed room state

**Participant status**:
Transient, privacy-safe evidence about one participant's local player, attributed by the authenticated Sorotte session. It is explanatory only and never controls room playback, readiness, or membership.
_Avoid_: Participant state, canonical telemetry

**Status epoch**:
The indivisible identity of participant status: protocol connection generation, negotiated capability, room membership, and authoritative playback scope. Evidence from different epochs must never be combined.
_Avoid_: Session timestamp, status version

**Playback scope**:
The server-authored `mediaGeneration`, optional `stateRevision`, and optional `transportRevision` to which player evidence is correlated.
_Avoid_: Client generation, file identity

**Player connection**:
The participant-reported relationship between the Sorotte client and its local media player.
_Avoid_: Sorotte connection

**Sorotte freshness**:
The server-derived age classification of the last accepted participant report while the participant remains a room member.
_Avoid_: Player connection, disconnected user

**Observation age**:
The elapsed time between player evidence and the participant report that carries it. Sparse position evidence retains its own observation age alongside the report-wide oldest-evidence age.
_Avoid_: Report age, latency

**Report age**:
The elapsed time since the server accepted a participant report, advanced locally after receipt by another client.
_Avoid_: Observation age, ping

**Room offset**:
A server-derived diagnostic difference between a participant position and canonical room position evaluated at one server time. Absence means uncorrelated evidence, never a zero offset.
_Avoid_: Client drift claim, synchronization command

**Snapshot mode**:
The declared completeness level of a room participant-status projection: full, compact, or unavailable.
_Avoid_: Complete snapshot when fields or rows are deliberately omitted
