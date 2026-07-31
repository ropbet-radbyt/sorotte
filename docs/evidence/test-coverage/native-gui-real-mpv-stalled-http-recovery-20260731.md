# Native GUI real-mpv stalled-HTTP recovery — 2026-07-31

## Scope and safety boundary

This slice is bounded defensive QA of Sorotte's own GUI, session, player, and
mpv integration. It launches only:

- the locally built Sorotte GUI;
- the exact installed `C:\Program Files\mpv\mpv.exe`;
- one strict Sorotte session fixture on an OS-assigned `127.0.0.1` port; and
- one purpose-built HTTP media fixture on a different OS-assigned
  `127.0.0.1` port.

The media is generated silent PCM AU retained under the ignored
`target/verification/` root. The valid framed response and all request
processing remain on IPv4 loopback. There is no public network target, DNS
lookup, credential, reconnaissance, persistence, privilege change, or
exploitation.

## Closed contract

The capability is a distinct opt-in:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts\gui-real-mpv-vertical.ps1 `
  -MpvPath "C:\Program Files\mpv\mpv.exe" `
  -TimeoutMs 80000 `
  -ExerciseStalledHttp
```

Healthy playback, owned-process replacement, malformed-HTTP recovery, and
valid stalled-read recovery are four mutually exclusive inventories. The
stalled-read mode has exactly 18 assertions and 11 artifacts.

The fixture serves a generated 45-second PCM AU object with:

- HTTP 200 and the complete `Content-Length: 4320024`;
- no transfer encoding or malformed framing;
- exactly `720,000` body bytes paced at `350,000` bytes/second;
- an open server response that emits no further body byte until cleanup; and
- one complete byte-zero recovery GET when Sorotte reloads the same URL.

The contract requires positive progress, a cache pause within 0.25 seconds of
the deterministic 7.49975-second playable-prefix boundary, at least 25 seconds
of server-side byte silence, and recovery within 50 seconds of prefix
completion. It rejects EOF, cache EOF, seeking, identified live media,
unexpected or intervening lifecycle rows, a foreign or unidentified PID/IPC
row, extra GETs, manual retries, and incomplete cleanup.

The request fixture records whether its response object was still retained
when the recovery GET was accepted. That is a harness-owned causal fact; it
does not claim that the kernel could independently prove peer-connection
liveness at that instant.

## Product finding

### TC-PLAYER-005: sustained cache stalls did not trigger bounded same-generation recovery

The first valid byte-silent native campaign reached `paused-for-cache=true` at
about 7.424 seconds while the original response remained retained, but Sorotte
never issued a recovery load.

Two response-boundary orderings contributed:

1. `start-file` and `playback-restart` can arrive before the authoritative
   playlist snapshot binds the accepted load attempt. The restart edge was
   discarded when no attempt was active, or could be projected onto a retained
   predecessor during a same-generation recovery. Binding the successor then
   reset its restart state, so the cache watchdog remained disarmed.
2. At the first cache-pause event, finite duration and path evidence could
   still leave the timeline classification `Unknown`. Requiring an already
   settled `Vod` label rejected the real VOD even though there was no positive
   live classification.

The correction:

- retains a restart that causally follows the exact deferred start;
- gives that deferred successor priority over a reducer-active predecessor;
- replays the restart exactly once after authoritative playlist binding;
- permits `Unknown` to arm only when neither `SlidingLive` nor
  generation-bound `ytdl_is_live` evidence identifies live media; and
- still requires coherent attachment, generation, attempt, network path,
  finite duration, position, remaining duration, cache pause, and retry-budget
  evidence before dispatch.

Deterministic regressions cover no active attempt, an already reducer-active
successor, a retained predecessor with an accepted same-generation successor,
unknown finite VOD, and positive-live exclusion. The retained-predecessor test
proves that the predecessor restart sequence is unchanged, the successor owns
the replay, and the successor can arm the cache watchdog.

## Harness hardening

Independent review closed these fail-closed requirements before the final
campaign:

- retain request and observation landmarks on RED paths;
- measure cache-pause recovery against the prefix-completion deadline;
- use absolute request, write, flush, and complete-response deadlines;
- retry transient `WouldBlock` and `TimedOut` socket operations only until
  those deadlines;
- require zero EOF observations and exactly one same-process `end-file` with
  reason `stop`, followed immediately by the recovered `file-loaded`;
- reject unidentified, stale, foreign, or intervening lifecycle rows;
- bind cache-pause position to the deterministic AU prefix boundary;
- retain evidence before cleanup and preserve it when a validator fails; and
- release and rebind the HTTP listener only after GUI and owned-mpv cleanup.

The committed Python validator reopens every artifact by SHA-256 and applies
the same closed schema and causal checks. Negative mutations cover truncated
silence, booleans substituted for numeric fields, framing drift, early
response release, extra GETs, premature EOF, cache-pause drift, invalid
`end-file` reason, missing or intervening lifecycle rows, unidentified and
foreign generations, position-boundary drift, and incomplete cleanup.

## Preserved RED sequence

All generated bundles remain preserved below
`target/verification/gui-real-mpv-stalled-http/`.

| Bundle | Result |
|---|---|
| `20260731T064003014Z-49828` | Product RED: valid response stalled at 7.424 seconds; no second GET or reload. |
| `20260731T071541899Z-64656` | Product RED after the first restart-order correction; watchdog still did not arm. |
| `20260731T072217276Z-64376` | Diagnostic RED: active file and restart were present, but the timeline was still `Unknown` at cache pause. |
| `20260731T073752729Z-19136` | First complete GREEN before the final retained-predecessor regression and correction. |
| `20260731T074608830Z-41896` | Complete native GREEN before the full crate suite exposed an over-broad deferred-restart rule. |
| `20260731T075226586Z-62536` | Independently approved final-source GREEN before the final all-gates GUI rebuild. |

The REDs were not relabelled as EOF or framing failures: the original response
declared its complete length, transmitted an exact valid prefix, remained
retained by the server, and emitted no terminal byte.

## Implementation-checkpoint post-gate GREEN

After the retained-predecessor correction, independent approval, complete
workspace validation, the semantic suite, and native smoke, the stalled mode
ran last against the final rebuilt GUI binary and passed:

```text
target/verification/gui-real-mpv-stalled-http/20260731T082105359Z-39796
```

```text
result:                              passed
assertions / artifacts:              18 / 11
GUI SHA-256:                         a680ec8323011e4083c51b2de64473f8a4b9ef1aef8507131d03eb721e22bab3
mpv SHA-256:                         2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0
mpv version:                         mpv v0.41.0-877-ge5486b96d
GUI / stable mpv PID:                62992 / 65068
HTTP listener:                       127.0.0.1:58386
session listener / peer:             127.0.0.1:58387 / 127.0.0.1:58388
generated bytes / SHA-256:           4320024 / de48fe1af9c5e46d4398da4bb4c4884005379168cedbd47ad17bbf0c31beec3d
first / recovery body bytes:         720000 / 4320024
server-side stalled duration:        29423 ms
pre-stall / cache-stall position:    0.514851 / 7.424 seconds
recovered position:                  7.961126 seconds
EOF observations before recovery:    0
end-file stop observations:          1
manual retries / invalid identities: 0 / 0
report duration:                     48794 ms
```

Observation order:

```text
1    initial file-loaded
13   positive pre-stall time-pos
95   paused-for-cache=true at 7.424 / 45 seconds
96   same-process end-file reason=stop
97   same-process recovered file-loaded
196  recovered time-pos beyond the stall position
197  GUI-driven recovered pause=true
```

Selected bundle hashes:

```text
e2ee43f25966d3f11038d05c8002d1166e32079234bdddb0ddd2f6ad655ab950  harness-report.json
b27f526462d4722111e4b9a83f45acab7ca9e951db66d264c80418beae531803  contract-summary.json
81c81b00ba915cb66815c611e9bf0a113c163895369a3a697945dac9f9bb507d  invocation.json
7a0fca1005a9c105c5da2c0b2841c19fc6b9cd91db5d86f2cb3556f266095226  stalled-http.json
267eff9e8b62412cd044f2b1faa4a963c6b4654ee3409f0b2667761d8cac8f1b  mpv-observation.jsonl
71327152c17f5aad289761e6d0e1288d9df76293dc7a703ba45f75c8dd89f3df  mpv.log
84e9c3b52d2dcf8b2e62aa066cf9e2607a02196516373a169019e78843ac5b19  real-mpv-state.json
```

## Final implementation-source GREEN, run last

After the final committed-source compatibility and WSL fuzz campaigns, all
519 Python self-tests, warning-denied workspace Clippy, the complete locked
all-feature workspace, a fresh GUI build, 14/14 semantic scenarios, native
smoke, and the other three real-mpv modes, the stalled mode deliberately ran
last:

```text
target/verification/gui-real-mpv-stalled-http/20260731T115707208Z-35432
```

```text
result:                              passed
assertions / artifacts:              18 / 11
GUI SHA-256:                         439174541d461db90fc66be088152024814e3ba4fe0d0d6b3add464103205d9e
mpv SHA-256:                         2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0
mpv version:                         mpv v0.41.0-877-ge5486b96d
GUI / stable mpv PID:                47132 / 3924
HTTP listener:                       127.0.0.1:59438
session listener / peer:             127.0.0.1:59439 / 127.0.0.1:59440
generated bytes / SHA-256:           4320024 / de48fe1af9c5e46d4398da4bb4c4884005379168cedbd47ad17bbf0c31beec3d
first / recovery body bytes:         720000 / 4320024
server-side stalled duration:        28962 ms
pre-stall / cache-stall position:    0.598078 / 7.424 seconds
recovered position:                  8.023747 seconds
EOF observations before recovery:    0
end-file stop observations:          1
manual retries / invalid identities: 0 / 0
report duration:                     47973 ms
```

Observation order:

```text
1    initial file-loaded
14   positive pre-stall time-pos
95   paused-for-cache=true at 7.424 / 45 seconds
96   same-process end-file reason=stop
97   same-process recovered file-loaded
201  recovered time-pos beyond the stall position
202  GUI-driven recovered pause=true
```

Selected bundle hashes:

```text
43c9fd5408b2d75055cb4d6126f9397e33ba9c9fa27d6e3fab0ffb053f93a675  harness-report.json
87dcb95ba3457c1bc473a70683d317e37f2109712313b80e4a689929779aac8b  contract-summary.json
5772f3de3f52cd9ed039d78444f0ba3e6aa045353b7ae6c50b192b3c784cca89  invocation.json
5023c051cb717e5f267c4d35aef372f2ca21b44037dfcd518241175184b5114c  stalled-http.json
c4331e63b72ade033ff194f77e1476e1752c6340d37a1b904fb68afa487de349  mpv-observation.jsonl
af9b34abea2e8cd6b6ef42ff9c633305ddf9cd550a00840bc86530b9f24d66f9  mpv.log
bb367d2ee551f7d3d55c5586a2cb68254485957118b98d7cb68d50707160269b  real-mpv-state.json
```

## Focused validation

```text
cargo test -p sorotte-player-mpv interrupted_network_stream_recovery_tests
# 19 passed, 0 failed

cargo test -p sorotte-gui --features gui-native-smoke \
  --bin sorotte-gui-native-smoke real_mpv_vertical
# 17 passed, 0 failed

python -m unittest scripts.tests.test_gui_real_mpv_vertical_contract -v
# 22 passed, 0 failed

cargo test -p sorotte-player-mpv --all-features
# 427 passed, 0 failed, 2 registered ignored

cargo clippy -p sorotte-player-mpv -p sorotte-gui \
  --all-targets --all-features -- -D warnings
# passed
```

Implementation binding:

```text
84c113344f91263f31f70325ec47395a6e0d652e1d1f7ca2ede8b726f1a68dbe  crates/sorotte-player-mpv/src/adapter.rs
0456d096d67c33edc27f0f423f7c5999598684876f44f8eed76cd9639ebe7b60  crates/sorotte-gui/src/bin/sorotte-gui-native-smoke/native_smoke_runner/real_mpv_vertical.rs
61de4346ad7b34b04d457722cc063dfaab2051a3628e54f48e22b68218079094  scripts/gui-real-mpv-vertical.ps1
23a928300ec8e7597fde1392c309e8acdd45edb3d17028314ed37979b5c643bd  scripts/gui_real_mpv_vertical_contract.py
55976aacd24f2fb153239a1a6daeaac409e047121df309b23c4bea63e5008d34  scripts/tests/test_gui_real_mpv_vertical_contract.py
```

## Limitations

- This is one Windows build and one installed mpv snapshot, not proof for the
  minimum-supported mpv, Linux, macOS, or BSD native GUI.
- The schedule covers one HTTP/1.1 response with a deterministic valid prefix
  and byte silence. It does not cover DNS, TLS, HTTP/2, proxy, downloader, or
  public-CDN behavior.
- Finite testing cannot prove every timing interleaving; deterministic reducer
  tests cover the response-boundary orderings that the native schedule exposed.
- Generated bundles remain ignored. This committed record binds them by exact
  paths, structured fields, and hashes without placing large logs, media, or
  screenshots under source control.
