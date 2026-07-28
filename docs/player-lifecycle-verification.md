# Player lifecycle verification

Status: verification complete; ready for human merge review.

Verification source:

- branch: `codex/player-lifecycle-stabilization`
- commit: `a47b6e035608bb03f1a1dd59986375653963b39a`
- commit subject: `Separate physical player lifecycle ownership`
- isolated verification branch: `codex/player-lifecycle-verification`

This document inventories lifecycle authority before the verification harness
changes any production behavior. The acknowledged `PlayerEventBatch` path is
authoritative for Sorotte's mpv integration. Compatibility outputs may mirror
that path for older adapters or presentation, but they may not create a second
load-attempt or physical-transport authority.

## Baseline

The source checkpoint was clean and passed the following before verification
files were added:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 80000
SOROTTE_TEST_MPV_BIN=C:\Program Files\mpv\mpv.exe
cargo test -p sorotte-player-mpv --all-features \
  tests::smoke_tests::real_mpv_bridge_lifecycle_over_json_ipc \
  -- --ignored --exact --nocapture
```

Results:

- formatting: passed;
- strict all-target, all-feature Clippy: passed;
- full all-feature workspace tests: passed;
- GUI semantic suite: 14 of 14 passed;
- Windows native smoke: passed;
- real mpv bridge lifecycle: passed with
  `mpv v0.41.0-877-ge5486b96d`.

There were no pre-existing checkpoint failures.

## Authority inventory

| Concept | Sole authority | Derived copies | Permitted writers | Consumers |
| --- | --- | --- | --- | --- |
| attachment epoch | lifecycle reducer | adapter and API batch headers; consumer epoch fences | accepted attachment replacement only | adapter, GUI, client-core |
| physical load-attempt identity | lifecycle reducer | adapter physical projection; consumer attempt bindings | reducer only | adapter, GUI, client-core, playlist resolution |
| physical transport owner | lifecycle reducer event or authoritative snapshot projection | adapter `active_load_attempt_id`; GUI and client-core `transport_owner_attempt` | reducer-owned start, activation, terminal, disconnect, or snapshot handoff only | transport telemetry, playback coordination, readiness |
| logical ownership | lifecycle reducer | per-attempt consumer flags | reducer only | semantic media resolution and readiness |
| semantic load result | lifecycle reducer retained outcome | GUI and client-core per-attempt projections | reducer only | media resolution, readiness, command reporting |
| physical terminality | lifecycle reducer | consumer attempt bindings and tombstones | reducer only | adapter, GUI, client-core |
| command semantic terminality | lifecycle reducer | compatibility command progress and consumer result ledgers | reducer only | coordinator, GUI, compatibility consumers |
| current physical path | attempt-keyed adapter projection | authoritative snapshot and `LocalFileChanged` projection | authoritative physical boundary for the owning attempt | GUI and client-core presentation |
| transport phase | attempt-keyed physical projection | delta or complete snapshot | reducer boundary plus adapter normalization of raw mpv properties | GUI, client-core |
| playlist-resolution state | GUI playlist-resolution owner | status text and UI projection | GUI only, correlated by `LoadAttemptId` once supplied | GUI |
| native/system seek ownership | reducer plus the compatibility ownership ledger | GUI classification projection | explicit dispatch, observation, gap, supersession, and replacement boundaries | GUI, client-core |

No numeric comparison across attachment epoch, media generation, load attempt,
playlist entry, command, event sequence, or acknowledgement-token domains has
ownership meaning.

## Assignment audit

Classifications:

- **authority**: authoritative ownership transition;
- **reducer output**: application of an explicit reducer event or outcome;
- **snapshot**: complete authoritative rebase;
- **UI**: derived presentation or playlist-resolution projection;
- **compatibility**: legacy output for non-batch adapters or old consumers;
- **suspicious**: possible independent inference requiring an executable
  divergence before any production change.

### Reducer-owned attempt facts

| Mutation | Location | Classification |
| --- | --- | --- |
| initialize and permanently revoke `logical_ownership_revoked` | `sorotte-player-mpv/src/lifecycle.rs` load submission and successor/external replacement transitions | authority |
| initialize attempt terminal metadata | `sorotte-player-mpv/src/lifecycle.rs` load allocation | authority |
| set terminal state and `physical_terminal_sequence` | `commit_physical_attempt_terminal` | authority |
| emit at most one semantic result | reducer retained-outcome transition | authority |

The reducer is the only production writer of logical revocation, semantic
completion, and physical terminality.

### Adapter physical projection

The five fields below are one atomic physical projection:

```text
active_load_attempt_id
active_media_generation
active_playlist_entry_id
current_path
active_file_loaded
```

| Mutation family | Location | Classification |
| --- | --- | --- |
| initialize or replace all fields | `adapter/state.rs`; attachment reset in `adapter.rs` | authority |
| install an attempt-keyed projection | `install_physical_projection` | reducer output or snapshot |
| clear an attempt-keyed projection | `clear_physical_projection` | reducer output or snapshot |
| update a path only when the attempt key still matches | keyed path helpers in `adapter.rs` | authority |
| start, file-loaded, restart, end-file, disconnect | raw-event handlers in `adapter.rs` | reducer output |
| authoritative playlist/path reconciliation | `reconcile_lifecycle_from_authority` and `publish_reconciled_transport_state` | snapshot |
| no-IPC simulated completion | `adapter/player_adapter.rs` | compatibility/test projection |

No production assignment was found that selects the physical owner from only a
generation, target/path match, newest attempt, command ID, semantic `Loaded`,
one missing playlist snapshot, or elapsed time.

### Adapter pending request bookkeeping

`pending_load_request` and `pending_load_generation` describe a submitted
target. They are not the current physical path.

| Mutation family | Classification |
| --- | --- |
| set on command submission | authority bookkeeping |
| clear on synchronous rejection, file-loaded, physical error, quiescence, or attachment replacement | reducer output or authority |
| temporarily take and restore around legacy polling | compatibility |
| complete immediately in no-IPC simulation | compatibility/test projection |

The submitted target is never assigned to `current_path` before a physical
ownership boundary.

### Adapter transport phase

`transport_phase` is normalized by the adapter from reducer ownership plus raw
mpv properties. Permitted writes are:

- attachment initialization/replacement;
- restoration after synchronous command rejection;
- quiescence or authoritative empty rebase;
- normalized property changes;
- start, seek, restart, file-loaded, end-file, and disconnect boundaries;
- test-support fixtures.

The phase does not independently choose an attempt.

### GUI and client-core ordered consumers

Both consumers hold the same attempt binding:

```text
media_generation
command_id
playlist_entry_id
owns_transport
semantic_load_result
physical_terminal
logical_ownership_revoked
```

Permitted mutations are:

| Mutation family | Classification |
| --- | --- |
| attachment reset | authority fence |
| complete snapshot rebase | snapshot |
| `LoadAttemptBound`, `LoadAttemptStarting`, `LoadAttemptActive` | reducer output |
| `Loaded`, `Superseded`, `Indeterminate`, failure outcome | reducer output, with audit-sensitive cases recorded below |
| `LoadAttemptTerminal` | reducer output |
| acknowledgement compaction | delivery bookkeeping |

Transport deltas are accepted only from the explicit
`transport_owner_attempt`.

### GUI playlist resolution

`playlist_resolution_attempt.state` and
`playlist_resolution_attempt.load_attempt_id` are GUI-owned, but the attempt
identity is supplied by reducer output rather than inferred.

| Mutation family | Classification |
| --- | --- |
| create, scope-reset, retry, candidate selection | UI authority |
| attach `LoadAttemptId` from a bound/starting/active event | reducer output |
| `Loaded`, `Indeterminate`, late `Active`, `Superseded` | reducer output |
| command-progress/media-load/local-file compatibility projection | compatibility/UI |

Candidate, provider, logical override, failure evidence, and fallback state
remain owned by this one GUI attempt.

### GUI local-file and placeholder projection

`player_local_file_placeholder` is an explicitly logical UI projection created
for an accepted resolution candidate. It is not physical-path evidence.

`player_local_file` may be written by:

- attachment reset;
- logical placeholder creation;
- ordered `LocalFileChanged` after physical-owner validation;
- a complete authoritative snapshot;
- candidate confirmation/rejection;
- the mode-guarded legacy local-file path for adapters that do not use
  acknowledged batches.

The ordinary legacy local-file drain returns early in acknowledged batch mode.

## Compatibility audit

| Channel | Permitted in acknowledged mpv mode | Forbidden authority |
| --- | --- | --- |
| `PlayerObservationBatch` | not drained; acknowledged delivery is selected instead | attempt binding, physical owner, semantic load result |
| legacy `PlayerCommandProgress` | an ordered command result may be translated into the existing playlist-resolution presentation handler | physical terminality or load ownership |
| legacy `PlayerMediaLoadOutcome` | an ordered, attempt-fenced result may be translated into existing presentation handlers | current attempt or independently authoritative success/failure |
| legacy local-file update | not drained; ordered `LocalFileChanged` is owner-checked | path-based attempt inference |
| legacy transport telemetry | not drained | ownership in acknowledged mode |

The production GUI and client-core select one delivery mode before draining.
The acknowledged mpv path does not also drain the ordinary legacy queues.
The internal GUI command-progress mirror remains a static verification target
because it deliberately translates an ordered semantic result into an existing
playlist-resolution compatibility handler. The ordered media-outcome bridge
first validates exact attempt identity, but its failure-presentation handler is
still target-string keyed. No executable stale-failure/same-target divergence
was found; that is recorded as a follow-up rather than changed speculatively.

## Verification projection

The shared verification projection must retain, where observable:

- attachment epoch and sequence/acknowledgement state;
- physical transport attempt, generation, playlist entry, path, file-loaded
  state, and phase as one projection;
- logical owner;
- per-attempt semantic result, logical revocation, and physical terminality;
- complete transport snapshot fields;
- pending and terminal command results;
- GUI playlist-resolution attempt identity, candidate/provider, state,
  fallback, and placeholder status.

Each extractor marks fields it cannot claim rather than guessing them. The
harness compares every field claimed by two or more layers after each delivered
batch.

## Full-stack harness and trace matrix

`MpvLifecycleVerificationHarness` drives the real adapter entry points with a
deterministic clock:

```text
raw decoded mpv JSON or tracked command result
  -> MpvAdapter raw handlers
  -> PlayerLifecycleState
  -> exact PlayerEventBatch
  -> client-core production batch application
  -> GUI production batch application and real PlaylistResolutionAttempt
  -> byte-identical replay before acknowledgement
  -> adapter acknowledgement and consumer compaction
```

The harness also supplies authoritative playlist/path snapshots, event gaps,
attachment replacement, command acceptance/rejection, delayed
acknowledgement, duplicates, and partitioned pumping. It does not construct
reducer-only inputs for the principal traces.

The deterministic matrix covers:

| Trace | Full-stack coverage |
| --- | --- |
| A | local file with duration, network VOD, YouTube/extractor target, and same-generation recovery; `start-file` ownership plus Loading/cache telemetry are asserted before `file-loaded` |
| B | quiescent late start remains fail-closed; late active evidence cannot overwrite `Indeterminate`; timeout after an already-started load retains physical ownership |
| C | real Plex GUI candidate/provider/logical override recovers from `Indeterminate` on matching late active/local-file evidence |
| D | replacement accepted but never started, followed by predecessor-current, empty, successor-current, and unrelated external-X authoritative outcomes |
| E | same-generation recovery with predecessor terminal before and after successor `file-loaded` |
| F | accepted fallback logically supersedes the old attempt; suppressed late old evidence cannot recover GUI resolution or replace fallback |
| G | attachment replacement delivers and acknowledges the old epoch's exactly-once terminal handoff before a new attempt reuses playlist entry 1 |
| H | event gap, authoritative snapshot, delayed acknowledgement, byte-identical replay, idempotent consumers, late physical evidence, and finite convergence |

Generated histories exercise one-event, all-event, fixed, and randomized pump
partitions, repeated unacknowledged batches, and acknowledgement delayed across
one or more pumps. Logged seeds:

```text
history:   0x00dd5eed  0xc0ffee42  0xdec0de01
partition: 0x51a7e123  0xa77ac411  0x9e3779b9
```

The generated event vocabulary includes A/B/C loads, same-generation recovery,
timeout and late success, fallback dispatch, seek, pause/play, cache pause,
event gap and snapshot, disconnect/reattach, duplicate terminal input, and an
external mpv load. Final projections and semantic outcomes are invariant across
delivery plans.

## Proven executable findings

### Snapshot physical ownership was mistaken for semantic success

- Suspected invariant: a physical owner observed after `start-file` but before
  `file-loaded` is not semantically loaded.
- Highest practical layer: raw mpv ingress through `MpvAdapter`, an event-gap
  authoritative snapshot, the client-core ordered consumer, the GUI ordered
  consumer, and a real GUI playlist-resolution attempt.
- Exact checkpoint failure:

  ```text
  gap snapshot before file-loaded: attempt semantic result
  left: Known(Some(Loaded))
  right: Known(None)
  ```

- Origin: regression introduced by
  `a47b6e035608bb03f1a1dd59986375653963b39a`; its parent did not have the
  consumer semantic-completion field that the snapshot path initialized to
  `true`.
- Reachability: reproduced through the full-stack deterministic harness.
- Incorrect authority: both ordered consumers inferred semantic `Loaded` from
  `PlayerActiveLoadSnapshot`, whose contract described only the physical
  transport owner. The GUI also treated a snapshot path as confirmed
  file-loaded evidence.
- Correction: the reducer now retains its exact, write-once semantic load
  result and publishes it together with explicit physical-file-loaded and
  monotonic logical-revocation facts. Consumers apply those snapshot facts
  directly. A starting snapshot preserves a GUI logical placeholder; a loaded
  snapshot still restores confirmed media.

### Late physical activation overwrote retained Indeterminate

- Suspected invariant: `LoadAttemptActive` and `LocalFileChanged` are physical
  evidence and cannot emit or invent a second semantic result.
- Highest practical layer: accepted tracked load, semantic deadline, late raw
  `start-file`, raw path and `file-loaded`, adapter batch, both production
  consumers, GUI resolution owner, replay, and acknowledgement.
- Exact checkpoint failure:

  ```text
  late active after indeterminate: attempt semantic result
  left: Known(Some(Loaded))
  right: Known(Some(Indeterminate))
  ```

- Origin: regression introduced by the latest lifecycle push.
- Reachability: reproduced through the full-stack deterministic harness.
- Incorrect authority: both consumers initialized semantic completion from
  physical `LoadAttemptActive`; client-core additionally installed ownership
  and semantic success from `LocalFileChanged`.
- Correction: consumer bindings retain the reducer's exact first semantic
  result. Physical activation can restore transport and GUI resolution without
  changing `Indeterminate`. Client-core accepts an ordered local-file update
  only for the existing reducer-declared transport owner and never creates an
  attempt, ownership, or semantic success from the path.
- Consequence validation: after exact semantic retention was introduced, the
  existing client-core regression
  `indeterminate_load_outcome_preserves_binding_for_a_late_active_event` proved
  that late positive physical evidence no longer cleared the stale timeout-only
  technical-failure presentation. The recovery predicate now permits an active,
  nonterminal, non-revoked attempt with either `Loaded` or `Indeterminate` to
  clear that presentation, while an explicit assertion proves that its semantic
  result remains `Indeterminate`.

### Semantic timeout revoked an already-started physical owner

- Suspected invariant: command/load semantic expiry does not revoke a physical
  owner already established by `start-file`.
- Highest practical layer: raw `start-file`, deterministic semantic deadline,
  adapter batch, both production consumers, replay, acknowledgement, and the
  shared projection.
- Exact checkpoint failure:

  ```text
  semantic timeout after physical start: adapter to client: physical transport owner
  left: Known(LoadAttemptId(1))
  right: KnownAbsent
  ```

  The client retained the raw-event owner while the reducer/adapter had cleared
  it, which proved the producer authority was wrong.
- Origin: regression introduced by the latest lifecycle push.
- Reachability: reproduced through the full-stack deterministic harness.
- Incorrect authority: both reducer timeout paths converted a started attempt
  to quiescent and the adapter consequently cleared its complete physical
  projection.
- Correction: semantic expiry clears the semantic completion deadline and emits
  exactly one `Indeterminate` result. Only an attempt without correlated raw or
  deferred `start-file` evidence becomes quiescent. An authoritative playlist
  snapshot may project the reducer enum as `Starting` before that raw boundary,
  so the enum name alone is not ownership evidence. Once `start-file` has been
  observed for the reducer-owned attempt, semantic expiry retains physical
  ownership and consumers likewise apply `Indeterminate` without clearing it.
- Consequence validation: separating authoritative-current evidence from the
  raw `start-file` bit initially let a deferred quiescent start publish two
  ordered `LoadAttemptStarting` events. The reducer regression failed with
  `left: 2`, `right: 1`. A quiescent authoritative snapshot now binds the
  attempt without publishing a raw-start event; immediate deferred replay emits
  the one fail-closed event. This correction is reducer-owned defensive
  hardening introduced and proved during verification, not a runtime defect
  attributed to the source checkpoint.

### Logical revocation was absent from the acknowledged event contract

- Suspected invariant: when reducer acceptance of successor B revokes
  predecessor A, every ordered consumer must receive that same monotonic fact
  even if A already emitted semantic `Loaded`.
- Highest practical layer: A/D/E replacement traces and F fallback acceptance
  through the adapter, batch, client-core, GUI, replay, and acknowledgement.
- Exact checkpoint failure:

  ```text
  resolution F fallback accepted and bound: adapter to client: attempt logical revocation
  left: Known(false)
  right: Known(true)
  ```

- Origin: regression introduced by the latest lifecycle push.
- Reachability: reproduced through multiple full-stack deterministic traces.
- Incorrect authority: the reducer wrote `logical_ownership_revoked`, but its
  only attempted downstream signal was a semantic `Superseded` outcome. The
  exactly-once semantic ledger correctly suppresses that second outcome after
  `Loaded`, leaving no event for the independent logical transition.
- Correction: the reducer emits
  `LoadAttemptLogicalOwnershipRevoked { attempt_id, media_generation,
  successor_attempt_id }`. Both consumers apply that reducer event and never
  infer revocation from target, generation, or newest-attempt ordering.

### Authoritative reconciliation mixed predecessor identity with successor path

- Suspected invariant: attempt ID, generation, playlist entry, path,
  file-loaded flag, and phase must describe one physical projection.
- Highest practical layer: trace D after B times out and an event-gap
  authoritative query reports B as current while policy keeps quiescent B
  fail-closed pending correlated `file-loaded`.
- Exact checkpoint failure:

  ```text
  trace D successor appeared: physical path
  left: Known("https://media.example.test/never-started-b.mkv")
  right: Known("https://media.example.test/predecessor-a.mkv")
  ```

- Origin: regression introduced by the latest lifecycle push.
- Reachability: reproduced through the full-stack deterministic harness.
- Incorrect authority: `publish_reconciled_transport_state` paired the reducer's
  retained owner A with raw properties belonging to authoritative current entry
  B.
- Correction: reconciliation republishes raw properties only when the
  authoritative current playlist entry matches the reducer-owned active
  attempt. Quiescent B may bind and remain physically correlated without its
  path or telemetry being projected onto A. Empty, B-current,
  predecessor-current, and unrelated external-X outcomes are all covered.

### No-poll file-loaded fallback discarded current raw metadata

- Suspected invariant: generation-fenced raw path/duration observations followed
  by correlated `file-loaded` must produce the same owner-keyed local-file
  delivery when a coherent IPC poll returns no value.
- Highest practical layer: trace A local file from raw mpv property ingress
  through the adapter and both production consumers.
- Exact checkpoint failure:

  ```text
  trace A local file: correlated physical path must be delivered as LocalFileChanged
  ```

- Origin: previously missed defect.
- Reachability: reproduced through the full-stack deterministic harness.
- Incorrect authority: the file-loaded fallback rebuilt metadata from the
  submitted target and discarded raw observations already fenced to the active
  generation, preventing a ready local-file update.
- Correction: a successful coherent poll still wins. When it has no result, the
  adapter reuses only path, duration, and size observations proven current for
  the active generation; predecessor metadata remains excluded. Local, network
  VOD, extractor/YouTube, and same-generation recovery targets are covered.

### Authoritative snapshot field invariants

`PlayerActiveLoadSnapshot` now carries three explicit facts:

| Field | Invariant | Sole writer | Invalidation | Consumers |
| --- | --- | --- | --- | --- |
| `physical_file_loaded` | true only after the keyed attempt crosses mpv `file-loaded`; path and `start-file` are insufficient | lifecycle reducer snapshot projection | new physical owner, terminal/empty state, attachment replacement | adapter, GUI, client-core |
| `semantic_load_result` | first reducer result wins permanently; late physical evidence never overwrites it | reducer outcome emission | attempt retirement or attachment replacement only | GUI, client-core, media resolution/readiness |
| `logical_ownership_revoked` | monotonic for the attempt even while it remains physically current | reducer supersession boundary | attempt retirement or attachment replacement only | GUI, client-core, media resolution/readiness |

These fields describe one attempt identity and generation. Consumers reject any
attempt-keyed delta or local-file presentation that is not owned by that same
physical binding.

## Completion validation

All completion checks were run after the authority corrections:

| Command or suite | Result |
| --- | --- |
| `cargo fmt --all -- --check` | passed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | passed |
| `cargo test --workspace --all-features` | passed; the Windows build compiled the platform-gated rebuffer harness but had no runnable rebuffer case on this platform |
| `cargo test -p sorotte-player-api --all-features` | 17 unit tests and 1 source-compatibility test passed |
| `cargo test -p sorotte-player-mpv --all-features` | 353 passed, 2 opt-in smoke tests ignored |
| `cargo test -p sorotte-client-core --all-features` | 681 passed |
| `cargo test -p sorotte-gui --all-features` | 1,089 passed and 2 ignored; auxiliary GUI binaries also passed 12, 20, and 2 tests |
| GUI `lifecycle_verification_tests` | all 13 full-stack trace tests passed |
| sanitized transcript replay under every partition | passed |
| generated lifecycle histories | passed |
| `scripts/gui-semantic-suite.ps1 -Json` | 14 of 14 passed |
| legacy permissive `scripts/gui-native-smoke.ps1 -Json -TimeoutMs 80000` | passed in the interactive Windows desktop session |
| real mpv JSON-IPC bridge lifecycle | passed with `mpv v0.41.0-877-ge5486b96d` |

The first native-smoke launch through the noninteractive Node helper could not
send keyboard input across its Windows logon-session boundary. The identical
script passed when run in the interactive desktop session; this was a test
launcher limitation, not a product failure.

That historical pass predates the strict native contract. Replaying its raw
report through the current validator fails because required native menus and
Open Media completion were absent and the run performed repeated placeholder
DNS lookups. See `docs/TEST_COVERAGE_FINDINGS.md`; do not treat the historical
green result as current native-contract evidence.

The finite verification stop conditions are satisfied: the reducer, adapter,
client-core, and GUI projections agree after every scripted step; every required
trace A-H passes; replay and pump partitioning converge; compatibility channels
do not establish lifecycle authority in acknowledged mpv mode; no executable
P0/P1 lifecycle defect remains. The source checkpoint branch was not modified,
and this verification branch should now receive human merge review without
further speculative lifecycle changes.

## Reviewed follow-up dispositions

Independent review against
`fe18a43bf4b6588511e0c87b8c29366a4cdd1769` found no new executable P0/P1
defect and resolved the five remaining static questions:

1. semantic `Failed`, `NeverStarted`, and `TransportDisconnected` results imply
   a reducer-owned physical terminal transition, so the consumer terminal mark
   is a safe redundant projection;
2. the legacy GUI command-progress handler is an identity-gated presentation
   bridge and does not establish physical lifecycle ownership;
3. target-oriented failure presentation is reached only after the exact
   attempt fence; a deliberately constructed same-target queued-failure test is
   retained as nonblocking hardening;
4. poisoning every legacy getter would strengthen the no-mixed-mode regression
   test but does not reveal a duplicate production authority; and
5. `PlayerEventDeliveryMode` is required to remain stable for an attachment,
   which is now explicit in the public adapter contract.

The full reasoning, residual test ideas, and final dispositions are recorded in
`docs/player-lifecycle-followups.md`. None warrants reopening production
lifecycle behavior before merge. Any future executable failure must still name
the owning authority and be reduced to a deterministic regression before a
production correction is made.
