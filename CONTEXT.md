# Sorotte Synchronization

Sorotte coordinates a room's shared playback intent while keeping each participant's local player observations distinct from room authority.

## Language

### Shared authority

**Playback lifecycle**:
The complete progression from participant and player availability through room membership, media selection, preparation, synchronized playback, replacement or recovery, and termination.
_Avoid_: Player lifecycle when only one local player is meant

**Canonical room playstate**:
The server-owned room position, pause state, and playback authority that may direct participants' players.
_Avoid_: Peer status, observed room state

**Canonical playlist selection**:
The server-owned playlist contents and selected entry for a room. A participant's local queue or currently open path is an observation, not canonical selection.
_Avoid_: Local playlist, mpv playlist

**Playlist selection generation**:
The identity of one accepted selection or replay of a canonical playlist entry. Selecting the same numeric row again creates a successor generation even when the visible index and playlist contents are unchanged.
_Avoid_: Playlist contents revision, row number

**Playback transaction**:
A causally bounded attempt to turn an accepted intent into canonical room authority and observable participant effects. It ends in convergence or an explicit rejection, failure, or supersession.
_Avoid_: Button click, untracked command

**Convergence**:
A participant has applied canonical room intent to the matching media generation and its player observation is within the declared playback bounds, or the participant has exposed a specific inability to do so.
_Avoid_: Connected, probably synced

### Logical and physical playback identity

**Media generation**:
The identity of one logical room-media selection. Physical reload or recovery may preserve it, while replacement with different logical media advances it.
_Avoid_: Load attempt, playlist entry

**Player attachment**:
One bounded relationship between a Sorotte client and a local player process. Evidence from a retired attachment cannot affect its successor.
_Avoid_: Sorotte connection, player process lifetime

**Load attempt**:
One physical effort within a player attachment to make a selected medium active. Several attempts may belong to one media generation during replacement or recovery.
_Avoid_: Media generation, playback session

**Local player observation**:
Evidence of physical player behavior such as loading, pause, position, buffering, seek, or end of file. It never becomes room authority merely because it is locally visible.
_Avoid_: Canonical state, server state

**Natural completion**:
Correlated evidence that the active load attempt reached the intended media end for one exact playlist contents revision, row, and selection generation. Cache pause, seek-to-end ambiguity, and interrupted transport are not natural completion without the required corroboration.
_Avoid_: Any pause near duration, any EOF flag

**Recovery successor**:
A new physical load attempt that preserves the logical media generation while replacing an interrupted or unusable predecessor.
_Avoid_: New media, playlist advance

### Readiness and observation

**User readiness intent**:
A participant's explicit declaration about willingness to start the current room media.
_Avoid_: Player ready, technical readiness

**Technical playability**:
Evidence that a participant can load and play the current media generation. It is distinct from user readiness intent.
_Avoid_: User ready, connected

**Start gate**:
The server-owned decision phase that coordinates whether and when a room may start a media generation.
_Avoid_: Local autoplay timer, participant status

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
