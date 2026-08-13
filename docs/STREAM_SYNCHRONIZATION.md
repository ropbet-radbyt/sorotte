# Stream Synchronization

Sorotte uses one source-independent playback coordinator for local files, Plex streams, direct network media, and extractor-backed URLs such as YouTube. Once an active media transaction has established a generation identity, players that expose transport telemetry use the coordinator for room-driven loading, cache stalls, command completion, bounded recovery, reconnect correction, and start acknowledgements; GUI and CLI adapters execute its decisions and forward observations. Adapters without transport telemetry, and sessions with no coordinated media transaction to reconcile, retain the legacy direct-correction path for compatibility and do not receive the coordinator's observation-backed guarantees.

The defaults preserve legacy behavior: starts are immediate and each client buffers independently. Stronger start and room-buffering policies are opt-in.

## Architecture

```text
logical media identity + desired room revision
                    |
                    v
       source-independent client coordinator
          |                         ^
          | tracked commands        | generation-aware observations
          v                         |
      mpv adapter <---- cache / transport / lifecycle ---- mpv
          |
          +-- local file
          +-- Plex resolved URL (stable item identity retained separately)
          +-- YouTube URL -> mpv/yt-dlp extractor
```

Logical identity is opaque on the wire. Sorotte hashes local media metadata without including local paths; recognized YouTube URL forms normalize to the video ID before hashing. Plex keeps its stable logical item separate from expiring, credential-bearing playback URLs. A URL refresh can therefore create a new local load attempt without changing the room media generation.

The coordinator compares positions on the room timeline. GUI telemetry subtracts the user's offset from both player position and seekable ranges before forwarding an observation; the offset is added only at the final outbound `SetPosition` boundary. Server-owned barrier and room-buffering transitions remain authoritative even when their compatible `setBy` username is the local controller, while ordinary legacy self-echoes remain suppressed.

## Transport and command guarantees

Every local load has a `PlayerMediaGeneration`. Adapter replacement also advances a transport epoch. Stale events from an old load or adapter cannot mutate the current coordinator.

The mpv adapter reports:

- `Empty`, `Loading`, `Prebuffering`, `ReadyPaused`, `Playing`, `Rebuffering`, `Seeking`, `Ended`, and `Failed` phases;
- logical pause separately from cache-induced pause;
- `paused-for-cache`, `seeking`, `seekable`, `core-idle`, EOF, and playback-restart sequence;
- position, playback rate, seekable ranges, buffered-ahead hints, and input rate;
- generation-correlated load success and failure.

`open_file`, play, pause, and seek commands use tracked IDs in production mpv paths. IPC acceptance means only that mpv accepted a command. Completion requires matching observations:

- pause: logical pause is observed;
- seek: seeking has ended and position is within tolerance;
- play: logical play, cache release, and fresh forward advancement are observed; starts after load/seek additionally require a playback restart newer than the baseline captured when that lifecycle operation began, while an ordinary resume does not;
- open: the requested load reaches a matching lifecycle outcome.

Commands time out, cool down, and consume a bounded retry budget. Once the budget is exhausted, the revision degrades once and no further command is issued until new room intent, a new load, or a new adapter epoch resets the budget.

Reconnect restoration is another desired-state revision when transport telemetry and an active coordinator media identity are available. It waits through loading, seeking, and cache pause, and command acceptance alone never counts as successful correction. The legacy direct reconnect path remains for adapters without transport telemetry and for sessions that have no media transaction the coordinator could safely identify; advertising telemetry capability alone cannot create an unfinishable coordinator reconciliation.

## Room-participant status snapshots

Clients and servers that negotiate `sorotteParticipantStatusV1` publish an advisory view of each capable participant's observed player state. A client derives a compact report from the same generation-aware telemetry used by the playback coordinator and sends it on player, phase, scope, cache-pause, and failure transitions plus the normal state heartbeat. The report includes stable player/phase/timeline enums and, when known, a correlated `playbackScope` (media generation and room-state revision), position, logical pause, playback rate, buffered-ahead time, cache-refill progress, and sample age. It never includes a username, room name, media path or URL, logical media identifier, credentials, arbitrary player options, raw player errors, or a client-computed room offset.

The server binds each report to the authenticated connection, accepts only strictly increasing non-zero sequences and bounded finite values, and retains only the newest observation. A capable recipient's periodic `State` normally carries a full current-room snapshot, including explicit `unsupported` and `awaitingReport` entries plus server-computed report age. If that representation cannot fit the frame limit, the server declares `compact` mode and omits precise optional evidence; if even the compact population cannot fit, it declares `unavailable` mode with no participant rows. Consumers must honor that mode rather than treating omitted rows or fields as fresh evidence. A dropped or coalesced periodic frame repairs itself on the next snapshot, and a late joiner needs no status history. Retained status is removed on room changes, disconnects, capability downgrade, connection replacement or fencing, and media-generation replacement; status is never exposed across rooms.

The room dashboard keeps four concepts separate: whether the member is still in the room, whether its status heartbeat is fresh, whether its player is observable, and what coarse playback phase was observed. Reports up to 3 seconds old are fresh, reports from 3 through 10 seconds are delayed, and older reports are stale while the room session remains present. Legacy peers, capable peers awaiting their first report, unavailable or disconnected players, and stale reports are displayed distinctly. The room-level banner separately shows authoritative room intent and a compact advisory participant summary, so a room that is supposed to be playing remains distinguishable from a participant that is loading, prebuffering, seeking, rebuffering, failed, or ended.

Room offset is derived only by the server and remains informational: positive means ahead of the room and negative means behind. The server emits it only for a fresh VOD report in the exact current non-zero media generation, room-state revision, and transport-authority revision, with an allowed phase and enough current pause/rate/cache evidence to compare the participant and canonical room positions at the same server time. Legacy `ListUserEntry.position` remains a non-live fallback and never becomes a precise offset. Cache-refill percentage is progress toward mpv's cache-pause target, not total download progress. Participant status never drives pause, seek, readiness, start-barrier, or recovery decisions.

## Cache-stall and recovery algorithm

A cache pause never changes the user's logical pause state. While empty, loading, prebuffering, rebuffering, seeking, ended, failed, cache-paused, or recovering, the coordinator blocks ordinary position correction. An authoritative pause is still latched immediately; only its seek/alignment and observation-backed revision completion wait for safe transport evidence. Pause-command timeouts exclude cache-paused intervals where mpv masks the logical pause property. An intentionally paused, fully loaded mpv core is normally `ReadyPaused` with `core-idle=true`; that state permits prepare seeks and room play commands. This distinction prevents the old loop in which each new room timestamp caused another seek and discarded the buffer that had just filled without deadlocking normal paused playback.

Recovery is one generation-scoped episode:

1. retain the latest desired room state while the player is blocked;
2. wait for a post-cache position baseline and a later forward-advancing sample;
3. measure lag against the moving room anchor;
4. choose at most one recovery strategy for the episode;
5. require continuous observed advancement and convergence for the configured stability interval before closing it.

With the default `balanced` policy:

- lag up to 1 second continues without correction;
- moderate lag on a rate-capable transport uses gentle catch-up, capped by `streamingMaxCatchupRate`; a non-seekable transport that cannot change rate terminates explicitly as degraded instead of retaining recovery ownership indefinitely;
- without at least 2 seconds of observed buffer headroom, catch-up is conservatively capped at `1.03`;
- lag at or above `streamingHardSeekThreshold` may spend at most `streamingMaxHardSeeks` hard seeks;
- a catch-up episode receives an expected-convergence deadline capped at 300 seconds;
- renewed buffering after a hard seek, residual large lag after the seek, a missed catch-up deadline, a non-seekable target, or an exhausted command budget produces an explicit degraded result instead of another seek loop.

Other policies are:

- `preserve-content`: never skip watched content to catch up; persistent lag degrades and remains correction-blocked;
- `stay-closest`: prefer a bounded hard seek when outside negligible lag;
- `pause-room`: ask the room controller to pause instead of locally chasing an unsafe target.

Explicit room pause, explicit room seek, and source replacement remain authoritative and supersede the old recovery target. A recovery-owned catch-up rate is reset on every exit, including pause, media replacement, manual seek, adapter reset, degradation, disconnect, and barrier supersession; ownership is cleared only after the normal baseline rate is observed. Live/sliding seeks are clamped to a confirmed locally usable interval rather than sent beyond the window, except when the exact target is already present in another disjoint cached interval.

### Preparing a seek outside the local cache

A room-driven seek on network VOD may be valid even when its target is not in mpv's currently cached ranges. Sorotte therefore treats the ranges as a confidence signal, not as permission to clamp an ordinary VOD seek. The client normalizes, orders, and merges overlapping ranges, then classifies the requested target as cached, requiring a fetch, unknown because range telemetry is absent, outside a live window, or non-seekable. An absent range report is never interpreted as proof that the target is cached. For extractor-backed sources opened conservatively as VOD, the mpv adapter promotes only the current load to live/sliding on positive yt-dlp `is_live` evidence; finite-duration remote media is VOD, while a durationless remote source without positive evidence remains explicitly unknown. Supported mpv releases (0.41.0 or newer) expose that evidence as `ytdl_is_live` metadata. Absent, false, or temporarily unavailable live metadata leaves a durationless timeline unknown rather than guessing from duration or cache shape. Cache ranges remain local-cache evidence, not a claim about the source's complete DVR window. Live/sliding media is different from VOD: a target outside a known interval is clamped or rejected explicitly rather than sent as an unbounded seek, and Sorotte waits boundedly instead of guessing when no safe interval is known.

When a network target may need data, the client creates one generation-scoped seek-preparation episode. It freezes the requested target and the room anchor, sends at most one primary seek, and retains newer room state without chasing its advancing timestamp. Only a newer explicit room seek, media replacement, or cancellation supersedes that target. Preparation latches local pause even when the room intends to play, preventing frames from the wrong position while timeline classification, seeking, or refill is unresolved; playback is released only after observation-backed alignment. Local-file seeks bypass this lifecycle and continue to use normal player behavior.

Preparation and rebuffer recovery answer different questions. Preparation waits for the requested timestamp to become usable; recovery then makes one bounded decision about joining the room. Preparation is ready only after seeking and cache pause have ended, the observed position is within tolerance, and either the configured headroom is present or mpv has ended refill. The subsequent decision resumes normally for negligible lag, uses bounded gentle catch-up where safe, may spend the episode's one remaining hard alignment seek, or ends explicitly as degraded. An immediate post-seek stall remains part of the same lifecycle and does not reset the hard-seek allowance. Every episode terminates as ready, superseded, cancelled, or degraded.

The visible status deliberately distinguishes `Seeking`, `Fetching stream data`, `Buffer refill`, `Ready`, `Catching up`, and `Degraded`. Cache buffering percentage is refill progress toward mpv's cache-pause target, not media download progress. Buffered-ahead seconds may be shown when observed, but Sorotte does not claim an ETA from approximate or missing cache-duration and input-rate telemetry.

The current implementation is client-owned and does not pause other participants while one client fetches. Its panel offers `Keep waiting`, an opt-in `Join nearest buffered position` when a useful range is close enough, and `Cancel and remain here` while cancellation is still safe. Waiting never changes room state; joining a buffered position is explicit because it can skip content; and cancelling leaves the client at its current position. Asking the room to pause would require controller authority or a compatible room policy and is not part of this client-only increment.

A future lower-quality retry must also be explicit. For YouTube, its transport contract is to reload one preset lower, keep the same logical room media identity, preserve the frozen target and preparation episode, and use a `transportRefresh` rather than create a new room generation. Changing `ytdl-format` after extraction does not replace already selected streams, so a reload is required. Plex can expose this action only when its integration has a real transcoder-quality API; changing a client label without requesting a different transcode would be misleading.

A future optional room-level mid-play seek barrier could coordinate prepare, participant readiness, commit, and observed start across a controlled watch party. That protocol is intentionally outside the current client-only lifecycle; public rooms and rooms with legacy peers retain independent recovery.

## Streaming settings

Settings live in `[client_settings]` in `sorotte.ini` and are editable in the GUI Streaming section.

### Quality and mpv cache

| Key | Default | Effect |
| --- | ---: | --- |
| `streamingQualityPreset` | `auto` | Optional `ytdl-format`: `auto`, `best`, `balanced`, `1080p`, `720p`, `480p`, `compatibility`, or `custom`. |
| `streamingCustomFormat` | unset | Exact trimmed format expression when preset is `custom`. |
| `streamingBufferTarget` | `5` | `cache-pause-wait`. |
| `streamingReadAhead` | `30` | `cache-secs`; normalized to at least the target. |
| `streamingMemoryCacheMiB` | `150` | `demuxer-max-bytes`. |
| `streamingDiskCacheEnabled` | `false` | `cache-on-disk=yes/no`. |

Sorotte also enables `cache=auto`, `cache-pause=yes`, and `cache-pause-initial=yes` for Sorotte-opened network media. These are mpv per-file options in managed and attached players: mpv restores the prior values when the stream ends, so local files keep the player's cache defaults and user configuration. Later advanced player arguments win for network media; the GUI shows both configured and effective values.

The mpv network-policy hook applies options in a fixed semantic order, attempts every option even when one is rejected, and reads back an allowlisted set of effective cache properties after `file-loaded`. The result is explicitly `Applied`, `PartiallyApplied`, or `Failed`; protocol v3 keeps the legacy `failed` wire status for partial application and carries the precise state in an additive field, so an older adapter still fails closed. A rejected write or mismatched critical read-back never reports a ready policy. `MpvNetworkMediaDiagnosticSnapshot` correlates that result with the media and policy generations plus demuxer idle, cache duration, forward bytes, input rate, reader/cache-end timestamps, cache EOF/underrun, cache pause, and transport phase. The additive `PlayerAdapter::take_cache_telemetry_update` channel carries each complete generation-scoped cache observation, so omitted metrics and seek boundaries clear older same-generation evidence without changing the established public transport-update shape. Diagnostic values are parsed and canonicalized by fixed option type; the snapshot never retains media URLs, paths, credentials, invalid free-form cache values, or arbitrary advanced option values.

Set `SOROTTE_CLIENT_LOG_PLAYER_TELEMETRY=1` for a CLI support reproduction. In addition to playback telemetry, the CLI emits a change-gated `mpv-network-media` line for successful and failed policy applications. It includes media/policy generation, hook load sequence, verification state, ordered per-option applied/rejected/mismatched results, desired/effective allowlisted cache values, and current cache evidence; raw media targets and arbitrary advanced-option values are excluded.

### Recovery

| Key | Default | Range/values |
| --- | ---: | --- |
| `streamingRecoveryPolicy` | `balanced` | `preserve-content`, `balanced`, `stay-closest`, `pause-room` |
| `streamingMaxCatchupRate` | `1.05` | 1.0–1.25 |
| `streamingHardSeekThreshold` | `8` | positive seconds |
| `streamingMaxHardSeeks` | `1` | non-negative integer |
| `streamingStabilityInterval` | `4` | positive seconds |
| `streamingRecoveryRetryBudget` | `1` | non-negative integer |
| `streamingRecoveryCooldown` | `10` | non-negative seconds |

### Coordinated room buffering

| Key | Default | Values |
| --- | ---: | --- |
| `streamingRoomBufferingPolicy` | `independent` | `independent`, `pause-controller`, `pause-eligible`, `quorum` |
| `streamingRoomQuorumPercent` | `75` | 1–100 |
| `streamingRoomMaxPause` | `30` | positive seconds; server clamps to 1–60 |

Non-independent room buffering is accepted only in a controlled room and only from its authenticated controller. This prevents a public-room participant or an unauthenticated client from turning buffering reports into a hostage mechanism.

Capable clients publish generation/revision-bound transitions between buffering and recovered. The server sends a complete current policy/status snapshot after Hello, a room switch, or a capability upgrade, and that snapshot rearms one report of the client's current transport state. This keeps late joiners and reconnecting clients able to contribute to the same dynamic cohort used by `pause-eligible` and `quorum`. The server uses a 750 ms pause debounce and 1.5 second resume hysteresis by default. It can pause for the controller, any eligible client, or a quorum. A policy-owned pause always fails open at its bounded maximum, disconnects remove participants safely, and legacy clients receive only the canonical compatible pause/resume state.

### Start synchronization

| Key | Default | Values |
| --- | ---: | --- |
| `streamingStartPolicy` | `immediate` | `immediate`, `wait-controller`, `wait-all`, `quorum` |
| `streamingStartQuorumPercent` | `75` | 1–100 |
| `streamingStartTimeout` | `15` | positive seconds; server clamps to 1–30 |
| `streamingStartTimeoutAction` | `continue` | `continue`, `remain-paused`, `ask-controller` |

An omitted `streamingStartPolicy`, including in settings files created by older releases, keeps the compatibility behavior of starting immediately. Coordinated policies such as `wait-all` remain explicit opt-ins.

With a non-immediate policy, the initiating client starts `sorottePlaybackBarrierV1`. The timeout action travels in the prepare request and is enforced atomically by the server. `continue` performs a best-effort commit. `remain-paused` ends the prepare phase as degraded without committing or transiently unpausing. `ask-controller` enters the distinct `awaitingDecision` phase while the canonical room remains paused; the GUI explains that the controller can decide manually. An authorized ordinary play/pause transition supplies that decision and retires `awaitingDecision` as terminal degraded history before applying the canonical room state. A later missing `started` acknowledgement is reported as `startedAckTimedOut` and never reuses the prepare-timeout action.

### Quality suggestions

`streamingQualityDowngradeSuggestions` defaults to `true`. After repeated rebuffering, or when an approximate selected bitrate is available and observed input is insufficient, Sorotte recommends a lower preset in GUI feedback and CLI telemetry diagnostics. It never changes quality or reopens media automatically.

## `sorottePlaybackBarrierV1`

The additive extension is nested inside existing `Hello.features`, `Set`, and `State` envelopes. Both server and client advertise `sorottePlaybackBarrierV1: true`; legacy peers never receive extension objects.

Lifecycle:

1. An authorized initiator sends `prepare` with an opaque logical ID, a random operation ID retained across reconnects, a strictly increasing connection-scoped request nonce, `newPlayback` or `replay` load intent, target, policy, quorum percentage when needed, timeout, and timeout action. The request generation is zero; clients do not claim room generations from local clocks.
2. The server assigns the next monotonically increasing media generation, pauses the canonical room, captures the currently capable cohort, excludes legacy peers, normalizes quorum and deadline, and broadcasts canonical prepare/status.
3. A client reports `ready` only after the exact logical source is `ReadyPaused`, logically paused, not cache-paused or seeking, within 0.5 seconds of the target, and the prepare-derived desired revision has been observed as applied. `core-idle=true` is expected for an intentionally paused mpv core and does not prevent readiness.
4. When controller/all/quorum readiness is satisfied, the server creates one server-owned revision, commits, and sends the compatible room unpause. At the bounded prepare deadline, the server atomically commits, remains paused, or awaits a controller decision according to the canonical timeout action.
5. A client reports `started` only after the committed revision has produced fresh forward position advancement. Load/seek starts additionally retain their restart evidence; an ordinary resume does not require mpv's `playback-restart` event.
6. The server publishes `complete` or explicit participant degradation. `prepareTimedOut` and `startedAckTimedOut` are separate participant outcomes. A retained terminal commit is diagnostic history only and has no authority over later ordinary pause/seek state.

The server binds reports to authenticated connections; clients cannot name another participant. Stale generations/revisions, client-authored generation claims, malformed values, unauthorized prepare/policy requests, competing prepares from another owner, and optimistic playstate bundled with an acknowledgement are ignored. The same initiating client and room-join sequence may supersede its own preparing or committed lifecycle with a higher nonce and identity-consistent `newPlayback` or `replay`; the old generation first becomes terminal with the distinct `superseded` reason. An exact connection retry replays the canonical lifecycle without allocating or mutating room state, while the stable operation ID makes the same request idempotent across replacement connections. An older nonce remains suppressed after a newer generation replaces it. When the start barrier is disabled, the buffering-policy request carries the same identity/load-intent metadata so the server still allocates a fresh room generation instead of reusing retained terminal history. `transportRefresh` emits no new playback request and retains generation/revision, while an explicit terminal `replay` allocates a fresh generation even for the same logical media ID.

Playback-barrier Set frames are reliable only inside their connection, room, local-media generation, and request-nonce scope. Reconnect, room replacement, media supersession, and explicit cancellation remove serialized bytes while durable chat and playlist commands remain queued; lease-token receipts prevent a cancelled staged request from acknowledging the durable message that follows it. Transport write completion releases only those bytes: semantic intent remains pending until a canonical prepare or buffering-policy response matches both the operation ID and request nonce.

After a new Hello and any required controller authentication, the current operation sends an explicit recovery query, including when its start barrier is already terminal but still owns the ongoing buffering policy. The server answers deterministically with the retained prepare/status/commit and policy when the operation exists, the existing same-media lifecycle when another operation already owns it, or an explicit absence/conflict disposition. Only an explicit `absent` response permits a retry: a nonterminal operation retries the same start identity exactly once, while a terminal operation issues only a fresh policy `transportRefresh`. Exact recovery transfers ownership without allocating another room generation, fences the superseded connection at the full protocol boundary, and routes an immediate transport close to it; ordinary playstate, Hello, playlist, room, and extension commands remain inert until physical disconnect cleanup.

Operation IDs use a small ASCII-safe representation. Current identity lives only in canonical barrier or buffering-control state. When a different operation displaces it, the server retains a fixed-size identity digest in a monotonic 120-second replay tombstone, beginning at supersession time. Per-room and server-wide bounds cap that cache; capacity pressure returns the correlated, nonfatal `requestResult: retryLater` disposition before consuming the request nonce or mutating generations. The client keeps the semantic media intent, stays connected, and retries the same operation ID and nonce after the bounded server delay with capped backoff. Expired tombstones are collected by periodic maintenance and admission. New operation identities are additionally rate-limited per connection and room, while exact retries and recovery queries are exempt. Uncontrolled-room policy-only `Independent` requests are coalesced onto one generation without tombstones because their identity cannot coordinate or change playback. A local room-switch request cancels both serialized and semantic state; pre-authentication media that was never serialized may still follow an authoritative room change and emit once after authorization. State outbox coalescing separately merges `ready`, `started`, and `transport` inside `sorottePlaybackBarrierV1`, so one observation cannot overwrite another before transport delivery.

Mixed rooms remain compatible:

- only capable clients are readiness participants;
- legacy clients cannot hold a barrier or room buffering policy;
- legacy clients still receive ordinary pause, commit, and resume playstate;
- guarantees apply to the capable cohort, not to a legacy player's internal startup.

The wire model includes optional `startAt`, but the current server commits at its server-owned anchor time; it does not schedule a future clock-synchronized start.

## Diagnostics

CLI diagnostics are opt-in:

```powershell
$env:SOROTTE_CLIENT_LOG_PLAYER_TELEMETRY = "1"
$env:SOROTTE_CLIENT_LOG_PLAYER_DRIFT_DIAGNOSTICS = "1"
$env:SOROTTE_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS = "1"
```

Telemetry diagnostics include coordinator phase, active recovery episode, active seek-preparation state, target availability, refill progress or observed headroom, degradation reason, buffer episode count, hard-seek count, and any quality downgrade suggestion. Seek diagnostics report observed state only: they do not invent download progress or an ETA. `PlaybackCoordinationSnapshot` and `PlaybackCoordinatorMetrics` also expose first-frame/start acknowledgement latency, total buffer duration, gentle catch-ups, stale generation/timestamp observations, command timeouts, applied revisions, skew, buffer headroom, and input rate.

The GUI exposes effective mpv settings and gives one-shot warnings for quality suggestions and controller timeout decisions. Diagnostics redact logical IDs and private media arguments. Never include Plex tokens, signed URLs, cookies, or authorization headers in reports.

## Tests

The default workspace suite is hermetic:

- pure coordinator tests cover cache loops, retained play during load, advancement-backed completion, frozen out-of-cache seek targets, explicit seek supersession, bounded catch-up/seek/retry budgets, live-range clamping, transport-quality refresh identity, and local-file non-regression;
- synthetic mpv JSON IPC tests cover lifecycle phases, command IDs, cache/seeking properties, failures, and stale generations;
- protocol/client/server tests cover prepare/ready/commit/started, strict identity/revision validation, quorum rounding, mixed legacy rooms, authorization, disconnects, debounce/hysteresis, atomic start-timeout policies, and bounded room-buffering fail-open behavior;
- `sorotte-sim` provides a deterministic HTTP media server with first-byte delay, bandwidth limits, burst stalls, range delay, temporary disconnects, and multi-client convergence assertions;
- GUI semantic tests cover persistence and the typed streaming configuration surface. The semantic DSL does not synthesize mpv telemetry or server barrier messages, so coordinator, barrier, and recovery lifecycles are covered by lower-layer runtime/server tests and the real-mpv harnesses instead.

Useful commands:

```powershell
cargo test -p sorotte-client-core playback_coordinator
cargo test -p sorotte-player-mpv
cargo test -p sorotte-protocol playback_barrier
cargo test -p sorotte-server playback_barrier
cargo test -p sorotte-sim
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
```

Required Linux PR CI builds the minimum supported mpv 0.41.0 release and runs deterministic real-player checks: paused load/seek/resume semantics, coordinator prepare/start followed by an ordinary aligned pause, one bounded local-HTTP rebuffer episode, a byte-cap idle/drain/input-resume sequence, and the longer HTTP-stall recovery harness:

```powershell
cargo test -p sorotte-sim --test mpv_rebuffer_harness real_mpv_pause_seek_resume_semantics -- --ignored --exact --nocapture
```

The byte-cap regression uses a high time limit and a small packet cap so it completes in seconds. Its local HTTP fixture fills the initial cap, holds further response bytes until the observed cache has drained below half, and then releases them. The test requires intentional `demuxer-cache-idle`, a post-release cache increase with positive input, continued playback beyond the initially cached media, and no underrun/rebuffer observation:

```powershell
cargo test -p sorotte-sim --test mpv_rebuffer_harness real_mpv_cache_cap_drains_and_input_resumes -- --ignored --exact --nocapture
```

The same required mpv 0.41.0 CI job also runs the longer local-HTTP fault and bounded-recovery harness:

```powershell
cargo test -p sorotte-sim --test mpv_rebuffer_harness real_mpv_clients_keep_seek_recovery_bounded_during_an_http_stall -- --ignored --exact --nocapture
```

That regression separates player startup from fault injection. Both clients
must first acknowledge `ReadyPaused` at revision 1 with seeking clear, then
acknowledge timeout-free `Playing` at revision 2. Only after both exact
baselines are visible does the fixture arm one globally claimed path stall.
The fault must apply and complete exactly once across range/retry connections,
the affected client must issue at most one seek in each observed recovery
episode, and the healthy peer must perform no post-start seek. A separate
deterministic concurrent-request regression requires both parked handlers to
resume and return their complete response bodies after one globally claimed
stall. This prepared -> started -> armed ordering prevents startup timing from
being misclassified as cache-stall recovery evidence.

The deterministic harnesses use generated local media and local HTTP fault media, not the public YouTube service or a live Plex server. External extractor availability, YouTube site changes, Plex transcoder behavior, and third-party network conditions remain suitable for opt-in smoke testing rather than required CI.

## Boundaries

- Sorotte offers lower-quality suggestions but never switches quality automatically.
- The barrier improves observed startup guarantees but does not promise frame-perfect simultaneous rendering.
- Future scheduled-start clock semantics are not implemented.
- Live/sliding media is range-safe, but independently resolved streams still need a shared program-time origin for strong cross-client live-edge guarantees.
- Advanced mpv arguments can override typed cache assumptions; include the GUI's effective options when diagnosing unexpected behavior.
