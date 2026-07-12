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

## Cache-stall and recovery algorithm

A cache pause never changes the user's logical pause state. While empty, loading, prebuffering, rebuffering, seeking, ended, failed, cache-paused, or recovering, the coordinator blocks ordinary drift correction. An intentionally paused, fully loaded mpv core is normally `ReadyPaused` with `core-idle=true`; that state permits prepare seeks and room play commands. This distinction prevents the old loop in which each new room timestamp caused another seek and discarded the buffer that had just filled without deadlocking normal paused playback.

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

Explicit room pause, explicit room seek, and source replacement remain authoritative and supersede the old recovery target. A recovery-owned catch-up rate is reset on every exit, including pause, media replacement, manual seek, adapter reset, degradation, disconnect, and barrier supersession; ownership is cleared only after the normal baseline rate is observed. Live/sliding seeks are clamped to the latest valid seekable range rather than sent beyond the window.

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

Sorotte also enables `cache=auto`, `cache-pause=yes`, and `cache-pause-initial=yes`. Typed options apply to managed and attached mpv. Later advanced player arguments win; the GUI shows both configured and effective values.

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

Capable clients publish generation/revision-bound transitions between buffering and recovered. The server uses a 750 ms pause debounce and 1.5 second resume hysteresis by default. It can pause for the controller, any eligible client, or a quorum. A policy-owned pause always fails open at its bounded maximum, disconnects remove participants safely, and legacy clients receive only the canonical compatible pause/resume state.

### Start synchronization

| Key | Default | Values |
| --- | ---: | --- |
| `streamingStartPolicy` | `immediate` | `immediate`, `wait-controller`, `wait-all`, `quorum` |
| `streamingStartQuorumPercent` | `75` | 1–100 |
| `streamingStartTimeout` | `15` | positive seconds; server clamps to 1–30 |
| `streamingStartTimeoutAction` | `continue` | `continue`, `remain-paused`, `ask-controller` |

With a non-immediate policy, the initiating client starts `sorottePlaybackBarrierV1`. The timeout action travels in the prepare request and is enforced atomically by the server. `continue` performs a best-effort commit. `remain-paused` ends the prepare phase as degraded without committing or transiently unpausing. `ask-controller` enters the distinct `awaitingDecision` phase while the canonical room remains paused; the GUI explains that the controller can decide manually. An authorized ordinary play/pause transition supplies that decision and retires `awaitingDecision` as terminal degraded history before applying the canonical room state. A later missing `started` acknowledgement is reported as `startedAckTimedOut` and never reuses the prepare-timeout action.

### Quality suggestions

`streamingQualityDowngradeSuggestions` defaults to `true`. After repeated rebuffering, or when an approximate selected bitrate is available and observed input is insufficient, Sorotte recommends a lower preset in GUI feedback and CLI telemetry diagnostics. It never changes quality or reopens media automatically.

## `sorottePlaybackBarrierV1`

The additive extension is nested inside existing `Hello.features`, `Set`, and `State` envelopes. Both server and client advertise `sorottePlaybackBarrierV1: true`; legacy peers never receive extension objects.

Lifecycle:

1. An authorized initiator sends `prepare` with an opaque logical ID, a strictly increasing connection-scoped request nonce, `newPlayback` or `replay` load intent, target, policy, quorum percentage when needed, timeout, and timeout action. The request generation is zero; clients do not claim room generations from local clocks.
2. The server assigns the next monotonically increasing media generation, pauses the canonical room, captures the currently capable cohort, excludes legacy peers, normalizes quorum and deadline, and broadcasts canonical prepare/status.
3. A client reports `ready` only after the exact logical source is `ReadyPaused`, logically paused, not cache-paused or seeking, within 0.5 seconds of the target, and the prepare-derived desired revision has been observed as applied. `core-idle=true` is expected for an intentionally paused mpv core and does not prevent readiness.
4. When controller/all/quorum readiness is satisfied, the server creates one server-owned revision, commits, and sends the compatible room unpause. At the bounded prepare deadline, the server atomically commits, remains paused, or awaits a controller decision according to the canonical timeout action.
5. A client reports `started` only after the committed revision has produced fresh forward position advancement. Load/seek starts additionally retain their restart evidence; an ordinary resume does not require mpv's `playback-restart` event.
6. The server publishes `complete` or explicit participant degradation. `prepareTimedOut` and `startedAckTimedOut` are separate participant outcomes. A retained terminal commit is diagnostic history only and has no authority over later ordinary pause/seek state.

The server binds reports to authenticated connections; clients cannot name another participant. Stale generations/revisions, client-authored generation claims, malformed values, unauthorized prepare/policy requests, competing prepares for an active generation, and optimistic playstate bundled with an acknowledgement are ignored. An exact nonce retry replays the canonical lifecycle without allocating or mutating room state, and an older nonce remains suppressed after a newer terminal generation replaces it. When the start barrier is disabled, the buffering-policy request carries the same nonce/load intent metadata so the server still allocates a fresh room generation instead of reusing retained terminal history. `transportRefresh` emits no new playback request and retains generation/revision, while an explicit terminal `replay` allocates a fresh generation even for the same logical media ID; this also lets a different or reconnected controller intentionally replay the source.

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

Telemetry diagnostics include coordinator phase, active recovery episode, degradation reason, buffer episode count, hard-seek count, and any quality downgrade suggestion. `PlaybackCoordinationSnapshot` and `PlaybackCoordinatorMetrics` also expose first-frame/start acknowledgement latency, total buffer duration, gentle catch-ups, stale generation/timestamp observations, command timeouts, applied revisions, skew, buffer headroom, and input rate.

The GUI exposes effective mpv settings and gives one-shot warnings for quality suggestions and controller timeout decisions. Diagnostics redact logical IDs and private media arguments. Never include Plex tokens, signed URLs, cookies, or authorization headers in reports.

## Tests

The default workspace suite is hermetic:

- pure coordinator tests cover cache loops, retained play during load, advancement-backed completion, bounded catch-up/seek/retry budgets, live-range clamping, URL refresh, and local-file non-regression;
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

Required Linux PR CI installs mpv and runs the short deterministic real-player check: paused load/seek/resume semantics, coordinator prepare/start followed by an ordinary aligned pause, and one bounded local-HTTP rebuffer episode:

```powershell
cargo test -p sorotte-sim --test mpv_rebuffer_harness real_mpv_pause_seek_resume_semantics -- --ignored --exact --nocapture
```

Nightly CI separately runs the longer local-HTTP fault and bounded-recovery harness:

```powershell
cargo test -p sorotte-sim --test mpv_rebuffer_harness real_mpv_clients_keep_seek_recovery_bounded_during_an_http_stall -- --ignored --exact --nocapture
```

The deterministic harnesses use generated local media and local HTTP fault media, not the public YouTube service or a live Plex server. External extractor availability, YouTube site changes, Plex transcoder behavior, and third-party network conditions remain suitable for opt-in smoke testing rather than required CI.

## Boundaries

- Sorotte offers lower-quality suggestions but never switches quality automatically.
- The barrier improves observed startup guarantees but does not promise frame-perfect simultaneous rendering.
- Future scheduled-start clock semantics are not implemented.
- Live/sliding media is range-safe, but independently resolved streams still need a shared program-time origin for strong cross-client live-edge guarantees.
- Advanced mpv arguments can override typed cache assumptions; include the GUI's effective options when diagnosing unexpected behavior.
