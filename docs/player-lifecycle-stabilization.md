# Player lifecycle stabilization

Status: implementation design and verification contract.

Design baseline:

- branch: `codex/upgrade-rust-stable-dependencies`
- commit: `fe80cc75f2c2933b75298f865e2d528bcf73adfb`
- stabilization branch: `codex/player-lifecycle-stabilization`

Integration base:

- branch: `origin/codex/fix-youtube-buffering-stall`
- commit: `0ed6223b504d57416af313a7369d5d8a1f20d190`
- rebased and audited: 2026-07-25

This work preserves the Rust 1.97.1/dependency upgrade and the working network
media policy, cache-stall, and premature-EOF recovery behavior on the source
branch. It changes ownership and delivery semantics, not the network protocol or
the visible meaning of play, pause, seek, or media replacement.

The integration audit retained the pushed branch's authoritative pre-command
playlist baseline, bounded reconciliation and accepted-load expiry, IPC ingress
and per-field observation clocks, provisional EOF and cache-stall recovery
budgets, and system-seek fencing. Where both branches introduced ownership
models, those behaviors were translated into the pure `PlayerLifecycleState`
reducer instead of keeping a second adapter-private load registry. The pushed
observation-batch contract remains available for existing GUI/native-seek
inference, while new lifecycle consumers use the acknowledged event-batch
contract exclusively.

## Current implementation

`sorotte-player-api` currently exposes independent getters for sparse transport
telemetry, complete cache observations, tracked-command progress, and media-load
outcomes. `sorotte-player-mpv::MpvAdapter` stores those outputs in separate
queues. The GUI drains the queues by type and reconstructs a processing order.
The client-core runtime has a second, similar drain path.

The mpv adapter currently has one `PlayerMediaGeneration` per submitted load,
an optional pending generation, an optional active generation, an active
playlist-entry ID, and a map from playlist-entry ID directly to generation.
`start-file` can select a generation through the playlist map and then fall back
to the pending generation. `end-file` removes the playlist-to-generation
mapping and contains fallback selection involving active and pending
generations. That shape cannot represent two physical mpv file episodes for the
same logical media and makes a delayed event able to inherit current ownership.

Tracked commands are held separately in `PendingTrackedCommand`. Acceptance,
completion, supersession, and failure are reported, but the generic timeout path
currently makes timeout a physical ownership boundary. Replacement IPC resets
bridge-specific state but does not establish one identity domain that scopes
every playlist, command, seek, lifecycle, and ordered-event object.

Transport telemetry is explicitly sparse and supports `merge_from`. Cache
telemetry is complete for cache fields, but there is no complete transport
snapshot contract. Queue overflow can discard ordinary updates. Command and
media-load outcomes are drained rather than retained until a consumer
acknowledges successful application.

Existing recovery behavior that must remain effective:

- network media options are applied inside mpv's load hook and are correlated
  by hook instance/configuration generation/load sequence;
- a pending Sorotte load does not accept the stale path of the previous file;
- cache snapshots clear omitted cache metrics;
- a seek begins a fresh cache-evidence epoch;
- premature transport EOF recovery reloads the stream without changing the
  room-visible logical media;
- real-mpv cache-cap, pause/seek/resume, HTTP-stall, and premature-disconnect
  harnesses exercise the supported mpv baseline.

## Final component boundary

The implementation has three layers:

1. `sorotte-player-api` owns public identities and the ordered batch, snapshot,
   delta, semantic-outcome, and acknowledgement contracts.
2. `sorotte-player-mpv::lifecycle` is a pure reducer. It owns all attachment,
   physical-attempt, command-semantic, playlist binding, supersession, EOF, and
   reconciliation decisions. It has no IPC, filesystem, GUI, sleep, or wall
   clock dependency.
3. `MpvAdapter` translates raw ingress into reducer inputs, executes reducer
   effects, performs at most one bounded authoritative query group per
   maintenance cycle, and publishes reducer output. The GUI and client core
   apply batches; they do not infer physical load ownership.

The reducer boundary is:

```rust
fn reduce_player_lifecycle(
    state: PlayerLifecycleState,
    input: PlayerLifecycleInput,
) -> (PlayerLifecycleState, Vec<PlayerLifecycleEffect>);
```

The in-place implementation may use `&mut PlayerLifecycleState` internally to
avoid cloning retained targets, but tests exercise it as the pure transition
above. Ownership decisions exist only in this reducer.

## Identity domains and scopes

| Identity | Scope | Purpose |
| --- | --- | --- |
| `PlayerAttachmentEpoch` | One accepted mpv-core attachment | Prevents IDs or events from a replaced core matching the new core |
| `PlayerEventOrder` | One attachment epoch | Earliest-ingress causal order (`attachment_epoch`, `sequence`) |
| `PlayerMediaGeneration` | Logical adapter media | Binds room/coordinator state across same-media physical recovery |
| `LoadAttemptId` | One physical `loadfile` effect | Owns one mpv playlist/file episode |
| mpv playlist-entry ID | One attachment epoch | Binds raw lifecycle events to a physical attempt |
| `PlayerCommandId` | One attachment epoch | Owns semantic command progress |
| acknowledgement token | One adapter batch stream | Makes retained outcomes and batch replay explicit |

No numeric comparison across identity domains has meaning. Every reducer input
that can mutate attachment-owned state carries or is stamped with its attachment
epoch. The adapter increments the epoch only after accepting a supported
replacement core.

## Physical load-attempt state machine

Each physical load is represented by:

```rust
struct LoadAttempt {
    id: LoadAttemptId,
    attachment_epoch: PlayerAttachmentEpoch,
    media_generation: PlayerMediaGeneration,
    command_id: Option<PlayerCommandId>,
    requested_target: String,
    playlist_entry_id: Option<i64>,
    baseline_playlist_entry_ids: BTreeSet<i64>,
    replaced_attempt: Option<LoadAttemptId>,
    superseded_by: Option<LoadAttemptId>,
    state: LoadAttemptState,
    semantic_outcome_emitted: bool,
}
```

The reducer owns:

```rust
load_attempts: BTreeMap<LoadAttemptId, LoadAttempt>
playlist_entry_attempts: HashMap<i64, LoadAttemptId>
active_load_attempt: Option<LoadAttemptId>
```

States:

```text
Submitting
  -> AcceptedUnbound
  -> Bound
  -> Starting
  -> Active
  -> SupersededMayStillEmit { successor }
  -> MayStillEmit
  -> Terminal(Ended | Failed(kind) | NeverStarted | TransportDisconnected)
```

`Bound`, `Starting`, and `Active` may transition to
`SupersededMayStillEmit`. `AcceptedUnbound` may transition to `MayStillEmit`
when semantic completion is not observed. Supersession never deletes an
attempt.

### Submission

The adapter acquires the authoritative playlist baseline before submission when
available, allocates the attempt and command IDs, and reduces
`LoadCommandSubmitted` before sending `loadfile`. Lifecycle events received
while awaiting the command response already have ingress order and are reduced
in that order after the response boundary.

A synchronous rejection terminally rejects only the newly submitted attempt.
The prior active or accepted attempt remains unchanged. Acceptance changes the
new attempt to `AcceptedUnbound` and changes its predecessor to
`SupersededMayStillEmit`.

Same-generation cache-stall and premature-EOF recovery allocate a new attempt
while retaining the `PlayerMediaGeneration`.

### Strict playlist binding

A playlist entry can bind only from unique causal evidence:

- the entry was recorded during the command boundary;
- the entry is newly present relative to the captured baseline;
- the playlist entry's original filename matches the requested target;
- a lifecycle event carries an entry ID already bound to the attempt; or
- another documented signal uniquely identifies the attempt.

Resolved physical path is not sufficient because a URL can resolve to a
different stream URL or local cache path. An attempt is not retired merely
because it is absent from one playlist snapshot.

Unknown or ambiguous IDs mutate no attempt. They arm
`load_lifecycle_reconciliation_required` and retain the raw lifecycle evidence
for strict reconciliation.

### Physical terminal commit

Every physical terminal mutation uses one reducer transition,
`commit_physical_attempt_terminal`. It validates epoch and identity, is
idempotent, clears provisional EOF for that attempt, closes the attempt,
resolves matching command/load outcomes once, retires attempt-local cache and
timeline state, and emits at most one ordered logical terminal event.

A physical terminal can become a logical `Ended` or `Failed` only when it owns
the active physical attempt, has no accepted successor in the same logical
generation, has no current replacement, matches the current attachment, and
has not already been applied.

## Player-command state machine

Command semantics and physical-effect lifetime are separate:

```text
Submitted
  -> Accepted
  -> Completed
  -> Superseded
  -> Failed(SynchronouslyRejected | PlayerFailure)
  -> CompletionNotObserved
  -> TransportDisconnected
```

Every submitted tracked command has exactly one terminal semantic outcome.
`CompletionNotObserved` replaces the old interpretation of timeout as proof
that the effect cannot arrive. An accepted load or seek can retain a
`MayStillEmit`/`MayStillArrive` physical ownership record after semantic
supersession or observational timeout.

Semantic outcomes are inserted into a retained, ordered store. They remain
available in every compatible batch until the acknowledgement token covering
them is acknowledged. Repeated acknowledgement is harmless; an unknown or
future token is rejected.

## Ordered-event semantics

Raw mpv messages receive a monotonically increasing sequence at the earliest
adapter ingress boundary. The order is:

```rust
struct PlayerEventOrder {
    attachment_epoch: PlayerAttachmentEpoch,
    sequence: u64,
}
```

One raw message produces one normalized `SequencedPlayerEvent` containing all
of its semantic consequences. Reducer effects caused by a command boundary,
timer, or authoritative query receive an order at that boundary. Observation
timestamps remain monotonic freshness/projection data; they do not determine
causal order.

High-frequency transport deltas may be coalesced. Attachment changes, media
boundaries, event gaps, command outcomes, and load-attempt outcomes cannot be
silently removed. Removing a sequenced telemetry item sets an explicit gap and
sequence boundary.

Pump partitioning does not affect reduction. Consumers sort neither by event
type nor timestamp: they apply the batch's event vector in its supplied order.

## Deltas, snapshots, semantic outcomes, and batches

These are separate contracts:

- `PlayerTransportDelta` is sparse. An omitted field retains the consumer's
  value.
- `PlayerTransportSnapshot` is complete for every coordination field. Each
  field is `Known(value)`, `KnownAbsent`, or `Unavailable`.
- `PlayerSemanticOutcome` contains command and physical-load outcomes and is
  retained until acknowledgement.
- `PlayerEventBatch` carries the attachment epoch, sequence boundary, optional
  authoritative snapshot, ordered events, retained semantic outcomes, and one
  acknowledgement token.

Snapshot meanings:

- `Known(value)`: replace the field.
- `KnownAbsent`: clear the field while retaining knowledge that the player
  authoritatively has no value.
- `Unavailable`: clear stale authoritative data and mark the property
  unavailable.

The GUI and client core use dedicated `rebase_from_snapshot` methods. A snapshot
never passes through sparse `merge_from`/`observe_transport` logic.

Overflow sets a gap condition and builds recovery state outside the ordinary
queue. `take_player_event_batch` does not clear adapter-level lifecycle
ambiguity. An unacknowledged batch is returned again, so applying a batch and
acknowledging it is one consumer transaction. Applying a repeated event or
snapshot is idempotent by order/boundary.

Recovery never manufactures a success or failure. Where command or attempt
completion is unknown, the outcome is `CompletionNotObserved` or another
explicit indeterminate state.

## Attachment replacement

A candidate replacement is version-validated before any current state changes.
On acceptance:

1. Stamp one final old-epoch boundary.
2. Finish every old accepted command exactly once as
   `TransportDisconnected` and retain those outcomes until acknowledged.
3. Terminally disconnect old physical attempts.
4. Clear old playlist bindings, active attempt, seek ownership, provisional
   EOF, active path/file identity, cache-stall recovery, and inference state.
5. Increment `PlayerAttachmentEpoch` and reset the per-epoch event sequence.
6. Install the new IPC client and register observers.
7. Acquire an authoritative new-core playlist/path/transport snapshot outside
   the lossy event queue.

An unsupported candidate replacement leaves the existing attachment and epoch
untouched. Reuse of playlist-entry or command IDs by a new core cannot match
old objects.

## EOF classification

`eof-reached=true` creates provisional evidence for its owning attempt. It is
not terminal by itself and no elapsed timer can make it terminal.

The candidate is cancelled by same-attempt `eof-reached=false`, playback
restart, forward position progress, seek/seeking evidence, transition to
playing/buffering, an accepted replacement, or other evidence that the attempt
remains active.

Logical terminal playback is committed only by:

- a causally matched `end-file`; or
- a complete authoritative snapshot proving no active file, no current entry,
  no accepted successor, no pending attempt, and no contradictory restart or
  progress evidence.

An old attempt's EOF or `end-file` closes only that attempt after
same-generation recovery has a successor.

## Reconciliation

Adapter lifecycle ambiguity and consumer queue gaps are distinct.

The adapter exposes:

```text
Resolved
AuthoritativeIdle
AwaitingAcceptedAttempt
IncompleteSnapshot
TransportFailure
```

Maintenance performs at most one playlist/path query group per cycle.
Incomplete results use bounded exponential backoff with a maximum delay and no
busy loop. Empty playlist plus absent path is authoritative idle. Empty
playlist plus a nonempty path is incomplete and retried. A recently accepted
unbound attempt can remain awaiting within a semantic deadline, but command
timeout does not erase it.

Getters never perform repeated synchronous reacquisition. Taking a consumer
batch does not clear reconciliation-required state.

## Player action ownership and native actions

Every Sorotte-issued seek owns:

- attachment epoch;
- media generation;
- raw player-local target;
- effective room target;
- command ID;
- dispatch observation boundary.

Matching occurs in raw player coordinates before applying the current user
offset. Superseded or completion-not-observed seeks retain a compact
`MayStillArrive` record across a consumer gap. A matching late observation is
consumed as a Sorotte-owned effect and is never republished as a native user
seek.

The GUI may apply offsets, route accepted normalized events, execute effects,
and update display state. It does not bind a lifecycle event to a load based on
the newest/only transition, logical generation, current path, or timing.
Native-seek inference runs only after attachment, media generation, event
order, freshness, and command ownership validation.

## Reducer inputs and effects

The pure inputs include command submission/acceptance/rejection, playlist and
path snapshots, `start-file`, `file-loaded`, `end-file`, EOF, restart, position,
seeking, disconnect/replacement, consumer event gap, authoritative snapshot,
acknowledgement, and fake-clock advancement.

Effects include sending a command, requesting one authoritative query group,
emitting one ordered event, retaining a semantic outcome, and scheduling
bounded reconciliation. The adapter executes effects; it does not repeat their
ownership decisions.

## Executable invariants

The model asserts these after every deterministic or generated transition:

### Attachment and identity

- Every attempt belongs to exactly one attachment epoch and media generation.
- A playlist-entry ID maps to at most one attempt within an epoch.
- No object from one attachment matches an event from another.
- `active_load_attempt`, when present, is an existing nonterminal attempt.

### Commands

- Every submitted command reaches exactly one semantic terminal state.
- No command has conflicting terminal outcomes.
- Timeout cannot erase ownership of an accepted physical effect.
- Superseded commands may retain event-capable physical ownership.
- Outcomes remain available until acknowledged.

### Loads

- A lifecycle event mutates at most one attempt.
- Ambiguous ownership mutates no attempt.
- A superseded attempt cannot terminate its successor.
- Same-generation recovery always has a new attempt ID.
- Logical media has at most one active physical attempt.
- An accepted successor blocks an old terminal from becoming logical terminal.
- Each attempt emits at most one semantic load outcome.
- Absence from one playlist snapshot never retires an accepted attempt.

### Events and snapshots

- Order is monotonic within an attachment.
- Removed telemetry always creates an announced gap.
- Pump partitioning cannot change final state or outcomes.
- Snapshot application replaces; it never sparse-merges.
- Overflow cannot lose semantic outcomes.
- Gap recovery converges in finitely many batches.
- Replaying an unacknowledged batch is idempotent.

### EOF

- EOF property evidence alone is nonterminal.
- Contradictory evidence cancels provisional EOF.
- Terminal commit happens at most once.
- Duplicate `end-file` is idempotent.
- Old EOF cannot terminate a recovery successor.

### GUI and client core

- Rejected/stale positions do not update authoritative state.
- Delayed but valid lifecycle evidence is not dropped.
- GUI and client core agree on attachment/media binding.
- Native actions are classified only after observation acceptance.
- Snapshot rebasing clears stale pause, cache, seeking, seekability, EOF, error,
  ranges, rate, and input-rate state.

Debug assertions cover cheap map/active-attempt relationships in production.

## Failure and event-ordering matrix

| Trace | Owning transition | Required result |
| --- | --- | --- |
| A active; B accepted; C accepted; B start/end; C starts | B's entry maps to B | B closes; C and logical media remain nonterminal |
| A active; B accepted; C synchronously rejected | C only | B retains ownership and can complete |
| generation 7 attempt A; recovery B accepted; A ends | A only | No logical failure/end; B can start and load |
| Unknown `start-file` ID with multiple accepted attempts | none | Defer and reconcile; no guessed mutation |
| `start-file` and `file-loaded` arrive before the accepted entry appears in an authoritative snapshot | none until the snapshot binds the entry | Retain both ingress observations; replay identity-dependent start state and `file-loaded` exactly once after binding |
| Mutating-command response precedes its property events and no later command reads the socket | exact event-bound command/attempt after harvest | A central, rate-limited nonblocking fence harvests the events; the response itself never completes the command |
| Accepted C absent from one stale playlist snapshot | none | C remains accepted/unbound |
| Empty playlist and absent path | authoritative idle | Idle or accepted-unobserved, bounded query count |
| Empty playlist and present path | incomplete | Backoff and retry; no terminal guess |
| Queue gap before accepted seek physically lands | retained seek owner | Late landing is system-owned, not native |
| Core 1 ID 1; replace; Core 2 ID 1 | epoch-scoped lookup | No Core 1 state matches Core 2 |
| `eof=true`; restart/progress/seek | provisional EOF | Candidate cancelled; never timer-terminal |
| `eof=true`; matching `end-file` and no successor | attempt terminal commit | Exactly one logical terminal |
| `end-file` duplicated | same attempt | Second input is an idempotent no-op |
| Snapshot after overflow omits prior error/ranges/rate | snapshot rebase | Stale values cleared |
| Batch repeated before acknowledgement | same boundary/token | Same consumer state, no duplicate native action |

## Compatibility consequences

The network protocol, supported player configuration, and mpv minimum version
do not change.

The player API gains additive identity/event/batch/snapshot types and optional
trait methods with compatibility defaults. Existing sparse getters remain for
third-party adapters and legacy consumers during this branch. Mpv's legacy
getter projection is not used by the Sorotte GUI/client-core batch path.
External adapters that do not implement batches continue through the legacy
path and do not gain the new recovery guarantees until they opt in.

`PlayerMediaGeneration` becomes explicitly logical at the coordination
boundary. Physical episode identity moves to `LoadAttemptId`; code must not
infer an attempt from a generation.

## Observability and replay

Lifecycle debug output includes applicable attachment epoch, event sequence,
media generation, load attempt, playlist entry, command, redacted target kind,
state before/after, ownership decision, and reconciliation reason. Targets use
the existing credential/URL redaction policy.

A test/debug transcript record contains:

```text
attachment_epoch
ingress_sequence
monotonic_receipt_time
command_id
playlist_entry_id
raw_json
```

Committed transcript fixtures use synthetic local/HTTP/YouTube-like targets and
contain no credentials or private media URLs. The lifecycle dump and transcript
replayer are deterministic.

## Verification map

| Invariant or behavior | Proof |
| --- | --- |
| Existing cache/YouTube recovery | characterization tests plus current real-mpv cache-cap, HTTP-stall, and premature-disconnect harnesses |
| Attachment and attempt identity | pure model identity/invariant tests |
| A to B to C replacement | deterministic scheduler with error and stop variants |
| Synchronous rejection isolation | deterministic rejection scenario |
| Same-generation recovery | old end before/after successor start and old error variants |
| Strict binding/reconciliation | stale playlist, external current item, later causal entry, and empty-player tests |
| Fast load before binding | deferred `start-file`/`file-loaded` adapter regression plus required real-mpv pause/seek/resume semantics |
| Post-response event harvesting | pending Play completes without an unrelated command; active media delivers `end-file` after its load command is already terminal |
| Command/effect lifetime separation | timeout/supersession plus late load and seek effects |
| Epoch isolation | reattachment with reused playlist ID and pending commands |
| EOF evidence | restart, progress, seeking, matched end, duplicate end, and recovery successor tests |
| Ordered delivery | single-event, one-batch, exhaustive small partitions, and seeded randomized partitions |
| Snapshot replacement | GUI and client-core stale-field clearing tests |
| Outcome retention | overflow, repeated batch, acknowledgement, and invalid-token tests |
| Native seek ownership | overlapping system seeks, offset change, gap, and late landing tests |
| Generated histories | seeded load/recovery/reject/duplicate/gap/disconnect/reattach histories with invariants after every input |
| Real ingress | sanitized transcript replay for local, HTTP, YouTube-like, recovery, buffering seek, rapid replacement, keep-open, and reattachment traces |

The deterministic scheduler supports delayed, grouped, duplicated, dropped,
and overflowed inputs; command-response loss; old events after successor
acceptance; ID reuse; and explicit fake-clock advancement. Tests use no sleeps
and log their seed on failure.

## Design deviations

The brief's preferred reducer is adopted. Three compatibility accommodations are
intentional:

1. Existing per-type getters remain with default trait behavior so independent
   third-party player adapters remain source-compatible. Sorotte's mpv-to-GUI
   and mpv-to-client-core paths use the ordered batch contract, preserving the
   ordered-event invariants.
2. Transcript fixtures use sanitized synthetic equivalents when a real service
   URL would contain a renewable signature or user-specific identifier. Real
   mpv capture tooling can record a private trace for local diagnosis, while
   only the redacted normalized trace is committed.
3. A small set of pre-existing scripted adapter tests do not expose mpv's
   authoritative playlist response. Exact-target compatibility shims therefore
   remain under `cfg(test)` so those transports can establish their historical
   setup state. Production builds never bind an attempt from a sole pending
   generation, a path match, or a single candidate: they require the reducer's
   `(attachment_epoch, load_attempt_id)` ownership and an authoritative
   playlist-entry binding. The strict production check and the test-only shim
   are both covered by focused adapter tests.

### Fast real-mpv ingress discovery

The first required Linux real-mpv run after integration exposed this ordered
trace:

1. `OpenFile` was synchronously accepted.
2. mpv emitted `start-file` for the new playlist entry before an authoritative
   playlist query had bound that entry to the accepted attempt.
3. mpv emitted `file-loaded` in the same fast load episode.
4. Later reconciliation bound and started the attempt, and transport properties
   described `ReadyPaused`, but the already-consumed `file-loaded` observation
   was unavailable to complete the command.

The false assumption was that every fast `start-file`/`file-loaded` pair would
remain buffered in the IPC client until the reconciliation query completed.
The ordinary event pump can consume both before reconciliation starts.

The replacement keeps adapter-ingress observations that cannot yet be reduced:
the `start-file` record retains its attachment epoch, playlist-entry ID, and
pre-bound transport fields; the matching `file-loaded` record retains the same
causal identity and a private target value. Neither mutates an attempt while
ownership is ambiguous. Once the reducer binds the entry from the authoritative
playlist, the adapter applies the deferred identity-dependent start boundary,
reduces `file-loaded` exactly once in ingress order, and then republishes the
authoritative post-start transport properties for readiness consumers.
Replacement starts, terminal events, transport loss, and attachment replacement
invalidate the retained records.

This preserves supersession and timeout behavior because ownership still comes
only from the reducer's strict playlist mapping. It introduces no path or
single-pending-attempt heuristic, does not cross an attachment epoch, and keeps
duplicate terminal command outcomes idempotent. The
`ambiguous_load_lifecycle_reacquires_playlist_ownership_on_later_maintenance`
adapter regression now delivers `file-loaded` before binding and proves that
the later exact snapshot completes only the owning command exactly once.

### Post-response real-mpv event-harvest discovery

The replacement CI run then exposed a second ordered trace:

1. The owning `file-loaded` completed `OpenFile`, and the adapter's coherent
   metadata poll reached its stable quiescent state.
2. Sorotte sent `set_property pause false` for
   `Play(StartAfterLoad)`, and mpv returned a successful command response.
3. mpv changed `pause` to false, but the corresponding property events arrived
   after that response.
4. No later metadata command was required, so the IPC worker stopped reading
   the socket. The adapter remained `ReadyPaused` and the accepted Play timed
   out even though an immediate authoritative diagnostic read reported
   `pause=false`.

The false assumption was that a normal event getter drove the IPC transport.
Getters only drained events that the worker had already harvested, while the
worker read mpv's stream only until the response for an active command.
Previously, repeated metadata queries accidentally kept the stream moving;
correctly consuming the load request removed that incidental behavior.

The replacement adds a central nonblocking event fence using a harmless
`get_property pause`. It is single-flight and shared by all maintenance/getter
paths, runs at most every 100 ms while a command or media attempt is active,
and backs off to 500 ms while attached and idle so an external manual mpv load
can still be observed. The fence response has no semantic meaning. It only
causes the worker to harvest every earlier event, which then enters the same
sequenced reducer path as every other observation. It does not issue the
playlist/path/duration/file-size reconciliation group.

Delayed and duplicate events therefore keep their original identity and
idempotence rules. Superseded and timed-out commands are not revived by the
fence; only still-owned observations can complete a command. Attachment
replacement clears both the cadence and in-flight identity, and a fence
transport failure follows the existing disconnect path. Deterministic adapter
regressions prove that a Play completes from post-response pause/position
events without an unrelated mutation and that active media still delivers its
generation-scoped `end-file` after the load command has already completed.

### Source-baseline discovery

The brief described the checked-out source as the combined
version-upgrade/YouTube-buffering branch. The actual clean checkout recorded
above is the version/dependency-upgrade branch and does not contain the later
premature-EOF reload implementation. The completed recovery behavior exists in
the prior isolated `codex/fix-youtube-buffering-stall` worktree and is used as a
behavioral/code reference.

The unsuitable assumption was that this branch could merely reorganize an
already-present reload path. The replacement design incorporates the proven
cache-stall and premature-EOF signals directly into the new physical-attempt
state machine. Recovery keeps the logical generation, allocates a successor
attempt, and lets delayed/duplicate old-attempt events close only the old
attempt. Characterization, same-generation recovery, transcript, and real-mpv
tests prove that the user-visible improvement is preserved without transplanting
generation-only ownership shortcuts.

If implementation evidence requires another deviation, this section must be
updated with the false assumption, replacement design, preserved invariants,
event trace, and added tests before the code is considered complete.
