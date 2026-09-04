# Playback lifecycle assurance contract

Status: authoritative composition contract for playback verification

Machine-readable companion: `coverage/playback-lifecycle.toml`

Specialized contracts remain authoritative inside their boundaries:

- `docs/player-lifecycle-stabilization.md` for mpv attachment, command, load-attempt, event, acknowledgement, and recovery ownership;
- `docs/PARTICIPANT_STATUS_INVARIANTS.md` for advisory participant evidence;
- `docs/READINESS.md` and `docs/STREAM_SYNCHRONIZATION.md` for readiness and playback-barrier behavior;
- `coverage/behaviors.toml` for exact merge-proof identities.

This document owns how those domains compose into one user-visible lifecycle. If a specialized contract and this composition contract appear to disagree, stop and resolve the authority boundary rather than silently weakening either one.

## Scope

The playback lifecycle begins before a client or player exists and ends only after all session, player, server, and evidence ownership is released. It includes:

1. application and player startup;
2. transport negotiation and room membership;
3. canonical playlist publication and selection;
4. media resolution and physical load attempts;
5. readiness and coordinated start;
6. play, pause, seek, buffering, correction, and status reporting;
7. natural completion and playlist progression;
8. replacement, reconnect, player recovery, room switch, and late join;
9. explicit failure and user-visible inability to converge;
10. clean shutdown with no stale effect or orphaned process.

The currently reported playback failures are seed histories used to test this contract. They do not define its scope.

## Composition boundary

The complete causal path is:

```text
native user or player observation
  -> GUI/runtime request ownership
  -> client-core transaction and protocol intent
  -> terminal frame-delivery receipt
  -> actual server validation and canonical commit
  -> server fanout or authoritative snapshot
  -> peer client-core transaction
  -> peer player command and physical observation
  -> acknowledgement, compaction, and convergence evidence
```

A test that enters after the first boundary or exits before the last boundary proves only the portion it crosses. Mock servers, direct reducer calls, direct projection application, and product-derived final snapshots remain valuable but are not whole-lifecycle proof.

## Identity and authority

The following identities are distinct and must never be compared or substituted merely because their numeric values happen to match:

| Identity | Scope | Advances when |
|---|---|---|
| process run | one harness or application execution | a new process execution begins |
| protocol connection generation | one client/server transport ownership period | the client establishes a replacement transport |
| room membership epoch | one authenticated membership in one room | join, leave, room switch, or connection replacement changes membership |
| room transport-authority revision | one canonical play/pause/seek causal boundary | the server accepts a canonical transport mutation |
| player attachment epoch | one client/player relationship | the player adapter or owned player is replaced |
| media generation | one logical room-media selection | canonical logical media changes |
| playlist revision | one canonical playlist contents revision | the server accepts different playlist contents |
| playlist selection generation | one canonical selection or replay | the server accepts a selected-entry event, including replay of the same row |
| canonical playlist epoch | one opaque server-issued compare-and-set version spanning playlist contents and selection | the server accepts any playlist contents or selected-entry mutation, including replay of the same row |
| load attempt | one physical load effort | a physical load or recovery successor is allocated |
| player command | one player-side semantic command transaction | a new load, play, pause, or seek command is submitted |
| protocol mutation receipt | one causally required outbound frame | the frame enters terminal delivered or failed state |
| participant report sequence | one status epoch | a participant publishes the next advisory report |

Authority is similarly separated:

- The server owns canonical room playstate, playlist selection, room membership, readiness snapshots, and start-gate decisions.
- A client owns its pending intent and the decision to expose success, rejection, or failure to its user.
- A player attachment owns physical player observations and load-attempt identity.
- Participant status owns no canonical decision; it is advisory evidence only.
- The verification oracle owns expected lifecycle facts and effects; product projections cannot define their own expected result.

The canonical playlist epoch is an equality-only fencing token. It is not a playlist contents revision or selection generation and must never be ordered or compared numerically with either. A coherent server snapshot publishes the same epoch with contents and index; reconnecting clients replace any retired token with that snapshot. A new server process may restart the token because no old connection can remain authoritative across the process boundary.

The room transport-authority revision is also an equality fence, but it owns playstate samples rather than playlist selection. The server includes the current nonzero revision as `sorotteTransportRevision` on every Sorotte-authored playstate. A current client echoes that revision on local Seek and State samples. A tagged sample whose revision is zero, older, or otherwise different cannot mutate canonical playstate or refresh the server's slowest-client projection; the sender receives a forced current correction instead. The client refuses a backwards tagged revision and, when any new revision or explicit canonical Seek arrives, keeps every response ping-only until sampled player pause and, for Seek, position evidence match that authority. This includes the first tagged room baseline and the path where server authority arrives before the player has emitted any sample. A user Play or Pause staged after room join but before that first baseline is not discarded: it binds to the first non-seek revision and is emitted with that exact equality token, while a first canonical Seek remains server-owned and supersedes the earlier transport-only intent. The fence survives any number of intervening server ticks, so a slow mpv acknowledgement cannot turn a later tick into a pre-effect sample carrying the new revision.

Missing `sorotteTransportRevision` remains an explicit legacy-compatibility path: the server cannot infer an epoch that an older peer did not send. Such samples retain legacy handling and therefore do not provide the stronger causal proof claimed by tagged current peers. A client that has not observed a tagged playstate in its current membership likewise accepts a legacy server, but observing one establishes revision support until that membership or connection ends; a later untagged frame cannot downgrade the fence. A new membership clears the local comparison token because a room can be destroyed and recreated with a fresh revision sequence. Compatibility tests must keep these distinctions visible rather than treating absence as revision zero or silently fabricating correlation.

## Lifecycle machines

`coverage/playback-lifecycle.toml` is the source of truth for state and transition identifiers. The machines are orthogonal: a participant can be reconnecting while its old player attempt may still emit, or be technically playable while the room start gate waits for user intent.

### Application process

The application progresses from not started through startup, running, stopping, and terminated. Startup failure must enter an explicit stopping or terminal path. Shutdown is incomplete while an owned player, server connection, worker, listener, or evidence writer remains live.

### Player attachment

The local player progresses through absent, launching, connecting, attached, disconnected, relaunching, and stopped. Replacement advances the attachment epoch before successor evidence can be accepted. Old-epoch events are strict no-ops after their required terminal handoff.

### Session and room membership

Transport progresses through disconnected, connecting, awaiting Hello, active, reconnecting, and closing. Room membership separately progresses through outside, joining, joined, switching, and leaving. Reconnect and room switch fence connection-scoped transactions before successor authority is installed.

### Canonical playlist selection

The stronger atomic selection boundary is negotiated at room scope. If any current member advertises `sorottePlaybackBarrierV1`, the room uses Sorotte lifecycle authority and every member receives the ordinary compatible playlist and State fanout needed to converge. A legacy-only room retains Python Syncplay's independent `playlistChange` and `playlistIndex` semantics and therefore does not claim the stronger paused-zero selection invariant; exact Python trace parity mechanically verifies that boundary.

Playlist authority progresses through unknown, empty, populated, selected, mutation pending, index pending, and exhausted. Local UI projection, local player playlist, and media resolver output cannot commit canonical contents or index. Every accepted contents or selection event advances an opaque server-issued canonical playlist epoch, and every accepted selection establishes a successor selection generation even when it replays the same row. A natural completion may publish a guarded progression only while its local playlist contents revision, row, and selection generation remain current. The request carries its expected canonical row and epoch; the server commits only if both still match. Simultaneous or late contenders that lose that compare-and-set are consumed as no-ops without another canonical fanout. Within negotiated Sorotte lifecycle authority, every changed selection, whether explicit, guarded, or caused by replacing the selected entry while its numeric index remains stable, atomically retires the predecessor's canonical position and cached watcher samples, advances transport authority, announces the successor selection after its contents, and then publishes paused position zero; no predecessor sample may cross that boundary. Because playlist selection and transport authority are separate wire frames, each player owner pauses an observed retiring file before issuing the successor load and retains a pause-at-zero hold until both authoritative file evidence and an accepted post-selection playstate exist. During that interval, legacy room synchronization is suspended: the retained playstate still belongs to the predecessor and cannot issue a play, pause, or seek against an EOF-idle player or the successor. An already empty attachment proceeds directly to load because it has no predecessor and mpv may not expose a pause property while idle. Even a matching same-path observation delivered late from before the unload is retired evidence: every dispatched restore waits for a fresh post-command file observation before applying its physical reset. Physical reset proof is scoped to both the selection and player-attachment epoch: it runs once while State is delayed, re-arms for every accepted selection, and must run again on a replacement attachment. The coordinator treats that interval as a causal fence, so a delayed Play derived from the predecessor cannot land after the successor pause. A canonical Seek received while the physical load remains in flight is retained as successor authority, never applied to the retiring media, and replayed after the load/reset fence opens. A same-row replay still needs a one-shot paused-zero State; that frame uses connection-scoped FIFO causal delivery and cannot merge with periodic or participant-status State. An empty canonical contents snapshot retires any invalid selection, logical media, pending telemetry, and player-generation scope; current player owners pause and unload exactly once, while a later populated snapshot remains unselected until a distinct canonical index arrives. When the verified row is the last item and looping is disabled, an authorized client instead publishes one pause at a finite terminal position under the current room transport revision; the server's revision fence turns concurrent terminal contenders into one canonical commit and prevents time from continuing beyond the media duration.

### Media resolution

A selected logical target progresses through absent, unresolved, resolving, playable, missing, untrusted, or failed. Resolution outcome is correlated to the selected playlist revision and media generation. Late resolution from a retired selection cannot open media or change readiness.

### Physical load attempt

A physical load progresses through none, submitting, accepted-unbound, bound, starting, active, may-still-emit, terminal, and recovery allocation. Physical ownership and semantic outcome remain separate. Recovery may allocate a successor attempt without advancing the logical media generation.

### Local transport observation

The physical player observation progresses among unavailable, loading, paused, playing, seeking, cache-paused, provisional-EOF, ended, and failed. Natural completion requires correlated active-attempt evidence. Cache pause, transport interruption, and an isolated EOF property are nonterminal until their specialized contract says otherwise.

### Canonical transaction delivery

A user or system intent progresses through quiescent, local effect pending, protocol delivery pending, server commit pending, committed, peer application pending, converged, rejected, failed, or superseded. Success is not inferred from a local player effect. Semantic command ownership and physical player observation remain separate while an edge is in flight: the authorized pending Play or Pause is the only local value allowed to request canonical mutation, and a preceding player sample cannot invert it. Buffering authority may continue to suppress Play, but it cannot convert an explicit Pause back into Play. The intent retires only after canonical acknowledgement and a same-generation physical observation agree. Any player effect that depends on a protocol mutation remains fenced on the exact terminal frame receipt. Observational State such as heartbeat and participant status may coalesce to its newest complete value. A one-shot causal State, currently used by same-selection playlist reset, is connection-scoped FIFO: it survives behind earlier reliable control and remains a distinct frame ahead of later observational State.

### Readiness and start gate

The start gate progresses through inactive, waiting for intent, waiting for technical readiness, ready to commit, committed, and degraded. User readiness, technical playability, participant status, and canonical pause ownership remain separate facts. A late joiner must receive enough authority to converge or expose its inability.

### Participant status

Status progresses through unavailable, awaiting, fresh, delayed, stale, and withdrawn within one indivisible status epoch. Periodic complete snapshots self-heal coalesced loss. Status never seeks, pauses, advances a playlist, admits readiness, or commits a start.

## Global invariants

The machine-readable model assigns these identifiers to every affected transition.

### Safety

- `LIFE-AUTH-001`: only a validated server transition changes canonical room playstate or playlist selection; a pending authorized local Play or Pause, rather than stale player telemetry, supplies the local mutation value.
- `LIFE-EPOCH-001`: tagged evidence from a retired connection, membership, room transport-authority revision, attachment, media generation, playlist contents revision, selection generation, or load attempt, and a guarded playlist request carrying a retired canonical playlist epoch, cannot mutate successor authority; a new transport revision cannot be acknowledged with pre-effect player state, a retained predecessor playstate cannot drive legacy room synchronization on its canonical playlist owner while a successor reset is pending, predecessor Play authority cannot cross that uncommitted physical reset, and every changed canonical selection in a room with negotiated Sorotte lifecycle authority retires predecessor position and watcher samples before successor state is published.
- `LIFE-IDENT-001`: identities from different domains are never compared as interchangeable counters.
- `LIFE-DELIVERY-001`: a dependent player effect cannot precede terminal delivery of its exact causal protocol frame.
- `LIFE-ONCE-001`: one accepted semantic transition, including concurrent equivalent intents, produces at most one canonical commit and at most one terminal client result.
- `LIFE-EOF-001`: only correlated natural completion may request canonical playlist progression; the server commits it only while the expected canonical row and playlist epoch still match, and consumes stale contenders without fanout.
- `LIFE-STATUS-001`: participant status remains advisory, privacy-safe, epoch-bound, and absent when correlation is insufficient.
- `LIFE-SNAPSHOT-001`: an authoritative snapshot installs one internally coherent generation of room, playlist, playstate, readiness, and status facts.

### Liveness

- `LIFE-EXIT-001`: every transient state has a bounded transition to progress, retry, explicit failure, or termination.
- `LIFE-CONVERGE-001`: after accepted canonical authority and bounded faults cease, every capable participant converges or exposes a specific inability.
- `LIFE-REJOIN-001`: reconnect and late join obtain a current authoritative snapshot without depending on missed deltas.
- `LIFE-RECOVERY-001`: a recoverable player or transport failure cannot leave the lifecycle permanently terminal while a bounded successor is available.
- `LIFE-SHUTDOWN-001`: interactive and targetable platform shutdown signals enter one bounded drain that releases every owned process, task, socket, temporary root, and evidence writer.

### Observability and privacy

- `LIFE-TRACE-001`: every causal transition can be attributed to a run, actor, epoch, generation, revision, sequence, source, and result without recording secret or raw media identity.
- `LIFE-FAILURE-001`: a failed requirement preserves the first divergent boundary and does not become success because a retry later passes.

## Causal evidence ledger

The system harness and opt-in product recorder will emit the same schema-versioned event shape. Required fields are:

- schema version and run identifier;
- monotonic timestamp and emitting process role;
- connection generation, room-membership epoch, player-attachment epoch, media generation, playlist contents revision/selection generation, load attempt, command, frame receipt, and report sequence when applicable;
- redacted target kind rather than raw path, URL, room, username, token, or credential;
- source transition and causal predecessor identifiers;
- authority before and after;
- expected effect, observed effect, and terminal disposition;
- bounded deadline and whether it expired;
- product role, version, and digest in system evidence, stored once per process inventory; executable paths remain local and are never published.

Unknown or inapplicable identities are absent, never fabricated as zero. Events from different clocks are ordered only through explicit causal edges; wall-clock subtraction across processes is diagnostic and cannot establish authority.

## Assurance layers

Every critical transition requires all three layers:

1. **Model**: a small independent oracle generates valid and invalid histories, checks invariants after every step, shrinks failures, and persists minimized schedules.
2. **Seam**: exact producer/consumer or protocol-boundary tests prove framing, validation, ordering, rejection, acknowledgement, and effect ownership.
3. **System**: packaged actual server and client binaries, multiple isolated clients, and real supported players traverse the transition while an external oracle observes raw protocol, process, player, and UI evidence.

Line coverage, a product-derived final projection, or a mock-server native flow cannot replace a missing layer. They remain useful supporting evidence.

`coverage/playback-lifecycle-system.toml` is the closed system-proof registry. It maps each required release suite to the exact transition identifiers that the suite must emit in its validated causal ledger. Model validation rejects an unknown suite, source, platform, transition, or non-system transition assignment. Platform attestation rejects a passed report whose ledger omits any transition assigned to that named suite, and the complete attestation requires the exact union of the Linux and Windows platform coverage to equal every system-required model transition. A transition therefore cannot acquire system coverage from a descriptive claim or a blanket tier promotion.

## Independent model explorer

`scripts/playback_lifecycle_oracle.py explore` is the required model-layer gate. It begins with the shortest executable path to all 217 transition/source pairs, then performs deterministic state-aware interleavings across all 11 machines and multiple isolated client, room, transaction, player, and server subjects. Every accepted event is checked against every invariant assigned to that transition. A separate adversarial inventory proves that invalid authority, identity, causal edge, deadline, privacy schema, duplicate, retired epoch, uncorrelated EOF, and premature dependent-effect histories are rejected.

Pull requests run the fixed seed at 64 cases of 128 steps. The nightly job runs 512 cases of 256 steps under the same declared seed and emits deterministic event-stream digests, making any history exactly reproducible without relying on timing. Run the pull-request budget locally with:

```text
python scripts/playback_lifecycle_oracle.py explore \
  --model coverage/playback-lifecycle.toml \
  --seed 0x50A077E20260831 \
  --cases 64 \
  --steps 128 \
  --failure-dir target/verification/playback-lifecycle-model-failures \
  --compact
```

On the first unexpected divergence, the explorer preserves that failure class, delta-debugs the ordered ledger, and writes a minimized privacy-safe JSONL replay plus metadata. It refuses to overwrite an artifact with the same seed, case, and failure signature. This closes the independent composition-model gap; it does not substitute for process, protocol-fault, GUI, real-player, or release-artifact evidence.

## Packaged system harness

`scripts/playback_lifecycle_system.py` is the first system-layer composition runner. It takes explicit paths to the exact candidate server and client, the supported mpv executable, and the FFmpeg fixture generator, then records every executable's SHA-256 digest against a full candidate Git SHA. A publishable run additionally requires that SHA to equal the harness checkout's `HEAD` and requires a clean tracked-and-untracked source tree. It does not build, discover, or silently substitute a different product binary.

One run creates two deterministic Matroska A/V fixtures with FFV1 video and PCM audio. Using video containers is part of the contract: audio-only fixture names activate Sorotte's intentional music-loop behavior and cannot prove the no-loop terminal video boundary. The harness then verifies this ordinary production path:

1. launch the packaged server on an ephemeral IPv4 loopback listener;
2. connect an independent protocol observer and publish a two-item canonical playlist;
3. launch two isolated packaged CLI clients, each owning a real managed mpv;
4. drive play, pause, and seek through the controller's production stdin command path;
5. require canonical server commits and physical convergence in both players;
6. launch a third client after the seek and require snapshot-only catch-up;
7. require advancing participant-status snapshots from all three real players;
8. drop exactly one follower status report and require the next complete periodic report to self-heal it;
9. suppress only follower status reports until the live member ages through delayed and stale, release that lane, and require a fresh report to recover detail without changing canonical transport or playlist authority;
10. replace the selected entry while its numeric index remains zero, require a fresh canonical selection and transport revision, and require all three real players to load the replacement paused at zero;
11. restore the original selected entry under the same numeric index and require a second fresh paused-zero physical generation;
12. publish an empty canonical playlist, require coherent empty contents plus null selection, and require every real player to unload;
13. restore populated contents without implicit selection, select the first row through the production controller command path, and require one fresh paused-at-zero canonical transport generation plus physical reload everywhere;
14. move to a paused baseline, cut and hold the follower's fragmenting TCP proxy, and require its participant-status withdrawal;
15. start playback while the follower is absent, release its replacement transport, and require the same production CLI and real mpv to catch up without overwriting canonical state;
16. resume near the end of the generated first item, correlate each terminal mpv event to its last known media slot, let every completing client contend with the same canonical row and epoch, require exactly one server index commit with no stale-loser fanout, require a fresh paused-at-zero successor transport revision after that selection, and prove every player loads and remains at the second-item origin across a server refresh without the completed position resurfacing;
17. seek and resume the final video item, require at least one correlated physical EOF to commit exactly one successor transport revision, then prove every real player either ends or pauses near the endpoint while canonical selection remains fixed and repeated server refreshes hold one finite paused position;
18. let every client reach its bounded normal exit, revalidate that the terminal pause, position, selection, and physical players stayed bounded for the remainder of the run, require participant-status withdrawal and owned IPC cleanup, then require the server's signal-driven drain to exit cleanly.

The observer records playlist contents only as a count and generated media only as `media-1` or `media-2`. Its causal JSONL schema rejects path, URL, credential, token, and unknown fields. Per-player Lua observers also emit only stable role, media slot, coarse transport properties, and the terminal reason. Candidate paths remain in local generated scripts and process logs, not in the privacy-safe report or causal ledger.

The fragmenting proxy projects client-to-server playstate frames while relaying them, but never stores raw protocol bytes. Its closed event contains only the stable role, finite position, pause value, one-shot Seek flag, and optional transport revision. Usernames, room names, participant payloads, media identities, URLs, paths, and unknown extension fields are discarded before the event reaches the causal ledger. This projection is what distinguished an old-revision delayed frame from a same-frame new-revision/pre-effect sample during the lifecycle sweep.

Run it locally after building the candidates:

```text
cargo build --locked -p sorotte-server -p sorotte-cli
python scripts/playback_lifecycle_system.py run \
  --server target/debug/sorotte-server \
  --client target/debug/sorotte-cli \
  --mpv /exact/path/to/mpv \
  --ffmpeg /exact/path/to/ffmpeg \
  --artifact-dir target/verification/playback-lifecycle-system \
  --candidate-sha <40-character-git-sha>
```

On Windows, use the corresponding `.exe` paths. Exit code `125` means a declared executable was unavailable and writes a `result = skipped` report; it is never a pass. Exit code `1` is a lifecycle failure with the first divergent stage. Exit code `0` requires every check above. Pull-request CI builds its pinned mpv first and treats the harness as a required step in `mpv-pr-semantics`.

For diagnosis while code is still intentionally dirty, `--allow-unverified-candidate` permits a development run but marks its candidate attestation `development-unverified`. Passed evidence with that marker is deliberately rejected by `stage-safe-evidence`, so it cannot be mistaken for exact-candidate release proof.

Before CI publishes evidence, a separate fail-closed command revalidates every causal and player record, rejects unknown or sensitive fields, and copies only the report, causal ledger, player projections, and a digest manifest into a fresh directory:

```text
python scripts/playback_lifecycle_system.py stage-safe-evidence \
  --artifact-dir target/verification/playback-lifecycle-system \
  --output-dir target/verification/playback-lifecycle-safe-evidence
```

Raw process logs, generated Lua, client configuration, media fixtures, IPC names, and executable paths are deliberately excluded. CI records the harness and staging outcomes separately and fails unless both succeeded; a skipped harness may publish its safe diagnostic report but cannot satisfy the required gate.

The artifact directory must be absent or empty. Reusing a populated directory is rejected before execution so a later pass cannot overwrite an earlier failed attempt; choose a new attempt directory when replaying a failure.

## Deterministic schedule grammar

The independent model and system orchestrator share named schedule operations:

- start, stop, kill, relaunch, attach, detach, connect, disconnect, half-close, and reconnect;
- join, leave, switch room, and replace connection or membership;
- publish, replace, clear, select, advance, exhaust, shuffle, and restore playlist authority;
- resolve playable, missing, untrusted, ambiguous, delayed, or failed media;
- submit, accept, reject, time out, supersede, delay, duplicate, and acknowledge commands or frames;
- start-file, file-loaded, play, pause, seek, cache pause, progress, EOF, end-file, and recovery successor;
- prepare, become technically playable, declare readiness, commit start, degrade, and recover;
- emit, coalesce, delay, stale, withdraw, and refresh participant status;
- partition one participant, slow one reader, stall one worker, overflow one bounded queue, and resume after the fault;
- join a new participant before or after every authoritative transition.

The TCP proxy may delay, fragment, throttle, half-close, or reset a stream. It must not claim that impossible within-stream byte reordering is a production network schedule. Cross-channel causal reordering is exercised at the owning worker or event boundary.

## Current proof and gap map

All eight implementation gaps in the machine model are closed. Closure means the production behavior, executable proof capability, and fail-closed release contract exist; it does not claim that an untested future candidate has passed. Every release candidate must still produce fresh exact-SHA hosted evidence through the reusable gate.

| Gap | Closure mechanism | Candidate-time proof |
|---|---|---|
| `GAP-MODEL-001` | transition-complete state-aware exploration, all 15 assigned invariants, invalid-history probes, deterministic shrinking, and persisted replay | closed-model validation and oracle execution at the candidate SHA |
| `GAP-TRACE-001` | one versioned privacy-safe ledger across server, CLI, GUI, client-core, player, proxy, harness, and oracle roles, with strict causal merge validation | regenerated ledger summaries whose digests and transition inventories are bound into every suite report |
| `GAP-SYSTEM-001` | exact server, three isolated packaged CLI clients, supported real mpv processes, an independent observer, and exact native-GUI compositions | ordinary system walk plus all Windows exact-GUI suites declared by the system-proof registry |
| `GAP-FAULT-001` | a closed replayable schedule schema and deterministic protocol, advisory, HTTP, worker, IPC, and process fault boundaries | ordinary scheduled fault walk plus faulting HTTP, stalled HTTP, and owned-process exact-GUI suites |
| `GAP-PLAYLIST-001` | canonical same-row/replacement/empty/restore handling, guarded EOF contention, ordinary advance, last-item bound, and loop-at-end proof | ordinary and loop system ledgers must contain every playlist transition assigned by the registry |
| `GAP-START-001` | an external production-wire oracle traversing every start-gate phase under late join, slow resolution, partition, reconnect, timeout, and sleep/resume | exact-server start-gate suite and bound causal ledger |
| `GAP-STATUS-001` | packaged cadence, loss self-healing, aging, recovery, withdrawal, reconnect, and a named second-client native GUI projection | packaged status walk plus exact GUI AccessKit projection and screenshot |
| `GAP-RELEASE-001` | immutable candidate bundles, separate Linux and Windows platform attestations, and one composed complete attestation | both hosted platform gates must bind the same model, registry, candidate SHA, binary digests, and exact transition union before publication |

Gaps remain first-class entries in `coverage/playback-lifecycle.toml`, with owners, risk, affected transitions, and mechanical closure criteria. Any future regression in the proof capability reopens the relevant entry; publication always runs the validator with `--require-closed` and cannot treat a local development-unverified run as release evidence.

## Completion criteria

The playback-lifecycle assurance goal is complete only when:

- the model validates with no open gap under `--require-closed`;
- every state and transition is reachable in the independent model;
- every critical transition has model, seam, and system evidence bound to the exact candidate SHA;
- generated failures shrink and persist as replayable schedules;
- the actual-server multi-client real-player suite covers a minimal transition-complete set of walks;
- nightly deterministic schedule exploration and soak have declared resource and convergence bounds;
- failure artifacts contain the causal ledger and first divergence without secrets;
- release verification executes the exact packaged binaries and publishes only their tested digests;
- a fail-then-pass run remains a failure until its first failure is explained and closed;
- currently known playback symptoms are ordinary replay seeds, not special-case acceptance rules.
