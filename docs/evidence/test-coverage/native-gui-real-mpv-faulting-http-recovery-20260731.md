# Native GUI real-mpv faulting-HTTP recovery — 2026-07-31

## Scope and safety boundary

This slice is bounded defensive QA of Sorotte's own GUI, session, player, and
mpv integration. It launches only:

- the locally built Sorotte GUI;
- the exact installed `C:\Program Files\mpv\mpv.exe`;
- one strict Sorotte session fixture bound to an OS-assigned
  `127.0.0.1` port; and
- one purpose-built HTTP media fixture bound to a different OS-assigned
  `127.0.0.1` port.

The media is a generated silent PCM AU file retained under the run's ignored
`target/verification/` root. The HTTP fault and all request processing remain
on IPv4 loopback. There is no public network target, DNS lookup, credential,
reconnaissance, persistence, privilege change, or exploitation. The harness
owns and releases the GUI, mpv, session listener, HTTP listener, and their
sockets.

## Closed contract

The capability is an explicit opt-in:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts\gui-real-mpv-vertical.ps1 `
  -MpvPath "C:\Program Files\mpv\mpv.exe" `
  -TimeoutMs 80000 `
  -ExerciseFaultingHttpRecovery
```

The default healthy vertical remains a separate 13-assertion, 10-artifact
contract. Owned-process replacement remains a separate 20-assertion,
13-artifact contract. The faulting-HTTP mode has an exact 18-assertion,
11-artifact inventory and cannot be combined with owned-process replacement.

The HTTP inventory requires:

1. an isolated Sorotte configuration and generated 45-second PCM AU whose
   header declares all `4,320,000` PCM data bytes;
2. strict nonzero IPv4-loopback session listener, session peer, HTTP listener,
   and every HTTP peer;
3. the exact expected client/server Hello exchange and exact closed
   `playlistChange` then `playlistIndex` request/echo sequence;
4. physical native Open Media delivery of the exact loopback URL;
5. one attested GUI-owned mpv PID, parent, image path, image digest, supported
   version, and product-generated IPC endpoint;
6. a first `GET /generated-fault.au` with `Range: bytes=0-`, HTTP 200, no
   `Content-Length`, `Transfer-Encoding: chunked`, exactly `720,000` valid AU
   body bytes paced at `350,000` bytes/second, and then the deliberately
   invalid chunk-size line `not-a-chunk-size`;
7. mpv playback progress before the fault and an observed
   `eof-reached=true` while more than Sorotte's 15-second recovery threshold
   remains in the declared VOD;
8. exactly one second media GET, with a full `Content-Length: 4320024` and all
   `4,320,024` AU bytes;
9. same-process, same-IPC, same-URL, same-duration automatic reload and
   progress beyond the retained pre-fault position;
10. physical GUI Pause after recovery, no manual retry, and no foreign PID or
    IPC observation after the fault boundary; and
11. evidence retention before cleanup plus release of the GUI, owned mpv,
    session thread/socket, and HTTP thread/socket.

The Rust harness records the request rows independently of mpv. A Lua observer
records mpv's PID, IPC endpoint, path, filename, duration, position, pause,
`eof-reached`, and lifecycle events. The Python validator reloads every
artifact by SHA-256 and rejects missing, extra, or reordered assertions and
artifacts; duplicate/extra media GETs; boolean-for-integer schema confusion;
path, PID, IPC, digest, duration, or position drift; incomplete release; and
self-attested observation positions.

## Product findings and focused corrections

### TC-GUI-004: automatic direct HTTP media had no player candidate

The first native product RED delivered the exact trusted loopback URL through
the session playlist but never reached mpv. Automatic media resolution did not
treat a direct trusted HTTP(S) target as a playable candidate. Commit
`dad376d` (`Fix automatic remote media recovery`) adds that candidate and
positive GUI runtime coverage.

### TC-GUI-005: one in-flight remote load could be submitted repeatedly

After the direct-URL correction, authoritative row-ID reprojections could
submit the same physical `loadfile` more than once before mpv confirmed
`file-loaded`. Commit `dad376d` adds physical-media confirmation state and
deduplicates the pending target. The minimized native RED retained the
duplicate requests; the ordinary regression requires exactly one submission.

One pre-existing tracked-fallback test had treated command completion as media
success even though no media confirmation arrived. Commit `779c689`
(`Align tracked fallback confirmation test`) corrects that stale oracle:
command acknowledgement alone remains `Loading`.

### TC-PLAYER-004: keep-open suppressed the terminal event required by early-EOF recovery

The malformed chunk is reported by this mpv build as:

```text
[curl] transfer failed: Failure when receiving data from the peer
[lavf] EOF reached.
```

With Sorotte's intentional `--keep-open=always --keep-open-pause=yes`, mpv
then publishes `eof-reached=true` and pauses at the available media boundary;
it does not emit `end-file`. Sorotte already observed `eof-reached`, retained
generation/attempt/path/duration/position evidence, and had a bounded
same-generation early-EOF recovery transaction, but that transaction waited
for the absent `end-file`.

Commit `c0a55b2` (`Recover keep-open network EOFs`) connects the existing
provisional EOF fence to the existing recovery transaction when:

- the provisional EOF belongs to the exact active physical attempt;
- the target is network VOD, not local media or identified live media;
- seeking is not active;
- coherent duration and position evidence exists; and
- more than 15 seconds remain.

It preserves the existing maximum of two consecutive and five total attempts.
Progress, seek, playback restart, replacement, generation, and attachment
fences remain in force. Unit coverage proves positive keep-open recovery,
near-tail exclusion, local-media exclusion, contradictory-evidence
cancellation, retry bounds, and absence of premature terminal telemetry.

All three findings are fixed positive regressions. The current product
known-defect registry therefore remains empty.

## Preserved RED sequence

Every generated bundle was retained; none was reset, overwritten, cleaned, or
deleted.

| Bundle | What it established |
|---|---|
| `20260731T005239395Z-31476` | Product RED for `TC-GUI-004`: no generated URL reached mpv. |
| `20260731T011359238Z-47008` | Retained direct-URL candidate iteration before the product correction settled. |
| `20260731T011743708Z-59640` | Retained native/session iteration with no media GET. |
| `20260731T014212992Z-12856` | Strict fixture rejected an unaccounted startup frame; no product conclusion. |
| `20260731T014745526Z-45224` | Strict fixture exposed dynamic ping-shape drift; no product conclusion. |
| `20260731T015048830Z-49168` | Strict fixture exposed an incomplete playlist exchange; no product conclusion. |
| `20260731T015601959Z-58572` | Closed four-frame playlist exchange iteration; no media load yet. |
| `20260731T020624089Z-66772` | Product RED for repeated same-target load submission; four `loadfile` commands were retained. |
| `20260731T024225197Z-15772` | Minimized `TC-GUI-005` row-ID reprojection RED before `file-loaded`. |
| `20260731T030636225Z-52996` | WAV prefix advertised the wrong 4.266667-second media duration; harness-media RED. |
| `20260731T031234529Z-59764` | Seekable WAV recovery rewound through range behavior; unsuitable fault oracle. |
| `20260731T031628037Z-60592` | Accepted socket inherited nonblocking mode; harness transport RED. |
| `20260731T031809467Z-41756` | Seekable WAV range retry recovered below the required causal boundary; unsuitable oracle. |
| `20260731T032946340Z-51680` | AU plus full advertised length stalled without the intended terminal classification. |
| `20260731T035954474Z-61856` | A valid finite 720,000-byte response ended normally at 7.5 seconds; it was not a transport fault and correctly did not prove recovery. |
| `20260731T041125117Z-34960` | The final malformed chunk produced curl failure and a 7.5-second keep-open EOF, proving the missing `end-file` assumption and `TC-PLAYER-004`. |

All paths are below:

```text
target/verification/gui-real-mpv-faulting-http-recovery/
```

The sequence hardened the fixture and oracle rather than accepting whichever
behavior happened to occur. In particular, the finite valid response was not
relabelled a transport error, and the final contract records mpv's actual
keep-open `eof-reached` signal rather than manufacturing an `end-file`.

## First full GREEN after the keep-open correction

The first complete GREEN is:

```text
target/verification/gui-real-mpv-faulting-http-recovery/20260731T042404849Z-664
```

The implementation bytes match focused commit
`c0a55b2c57904e5a5d76c0879ba97e938417b681`; the only later change folded
into that commit before this record was finalized added near-tail/local-media
unit assertions and did not change the executed GUI binary.

```text
result:                         passed
assertions / artifacts:         18 / 11
GUI SHA-256:                    f9f8e8d3351f5124366c986300c3f230cba88f58d5c5a768eec6607dc31ef243
mpv SHA-256:                    2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0
mpv version:                    mpv v0.41.0-877-ge5486b96d
GUI / mpv PID:                 41280 / 58580
HTTP listener:                 127.0.0.1:61727
session listener / peer:       127.0.0.1:61728 / 127.0.0.1:61729
IPC endpoint:                  \\.\pipe\sorotte-gui-mpv-41280-1785471861942
generated bytes / SHA-256:     4320024 / de48fe1af9c5e46d4398da4bb4c4884005379168cedbd47ad17bbf0c31beec3d
first transmitted body bytes:  720000
pre-fault position:             0.595046 s
premature EOF position:         7.089507 s of declared 45 s
recovered position:             1.10609 s
request count:                  2
manual retries:                 0
foreign post-fault observations: 0
report duration:                23225 ms
wrapper runner duration:        23690 ms
```

The two exact media rows were:

```text
1  GET  Range=bytes=0-  200  Content-Length=none
   Transfer-Encoding=chunked  body=720000
   malformed chunk boundary=true  disconnected_early=true

2  GET  Range=bytes=0-  200  Content-Length=4320024
   Transfer-Encoding=none  body=4320024
   malformed chunk boundary=false  disconnected_early=false
```

The observation order was:

```text
1    initial file-loaded
11   positive pre-fault time-pos
76   eof-reached=true at 7.089507 / 45 seconds
79   same-process recovered file-loaded
95   recovered time-pos beyond retained pre-fault position
112  GUI-driven recovered pause=true
```

Selected bundle hashes:

```text
95a4cbe0ae4a49d8b48db89037961af73a25ce26a3b1b61a14726ff31964721e  harness-report.json
72f5a8b20cc9761ad1ffbad2b1c1dc3e08bc3cbe2a5149ffc8dfdff0f66f7d24  contract-summary.json
eaa892a73d507c786f9b49632df5effbffd1e462bce85b2a0de2885d7de31b54  invocation.json
1df2d5fc5e970ec2449cf22e89be4e0d500a48d660fb400926f64c395f325d82  faulting-http-recovery.json
11a351da1fec5474f83ac7b83a651579236bbbbb645654c70e9ac71a89d61c6a  mpv-observation.jsonl
f1f0efb128676cd71480ae9e926e6a3e97265f8a8250e7dcec497d3867043a7a  mpv.log
42d7357beada3100101ed3e7232df5f486a7d649b22f9fef5a1207077be60b1b  real-mpv-state.json
```

The GUI digest is identical before and after execution. The report attests
stable mpv PID, image, digest, IPC, URL, and duration across recovery.
`owned_mpv_terminated_after_gui_exit`, `server_thread_released`, and
`socket_released` are all true.

## Final canonical post-gate campaigns

After every build-producing validation gate completed, the healthy,
owned-process-recovery, and faulting-HTTP modes were run sequentially. The
faulting transfer ran last. All three passed against the same final GUI and
mpv bytes:

```text
GUI SHA-256: 673dda5226c433950d3074cb4f1b2b6d222802eda6e30cc8a9b5d6e0ef12271c
mpv SHA-256: 2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0
mpv version: mpv v0.41.0-877-ge5486b96d
```

| Mode | Canonical bundle | Result |
|---|---|---|
| Healthy | `target/verification/gui-real-mpv-vertical/20260731T044916649Z-67112` | 13 assertions, 10 artifacts, GUI/mpv PIDs `57104`/`64968`, 16,713 ms |
| Owned-process recovery | `target/verification/gui-real-mpv-owned-process-recovery/20260731T045019794Z-49868` | 20 assertions, 13 artifacts, GUI PID `22372`, mpv PID `61396` automatically replaced by `48892`, 25,435 ms |
| Faulting HTTP | `target/verification/gui-real-mpv-faulting-http-recovery/20260731T045105652Z-43360` | 18 assertions, 11 artifacts, stable GUI/mpv PIDs `54916`/`44104`, 22,157 ms |

The owned-process run terminated only the exact attested GUI child, observed
automatic replacement with a distinct PID and IPC endpoint, re-attested the
same image and digest, exercised Play/Pause on the replacement, fenced the old
process, and reaped the replacement on native GUI Exit.

The final fault run retained exactly two media requests. The first transmitted
`720,000` valid body bytes and the malformed chunk boundary. The second
transmitted the complete `4,320,024`-byte AU object. It observed
`eof-reached=true` at `7.095267` seconds of the declared 45 seconds, reloaded
at observation index 77, and reached `1.096448` seconds after recovery. PID
`44104`, the IPC endpoint
`\\.\pipe\sorotte-gui-mpv-54916-1785473470619`, URL, duration, executable,
and digest stayed stable. There was no manual retry or foreign post-fault
observation. Evidence was retained before cleanup; owned mpv termination,
server-thread release, and socket release are all true.

Selected final bundle hashes:

```text
2735324df56ca5476780341a2d08237ba2a57050b8e218624e265fcd32528ab6  healthy/harness-report.json
6de31c5042d2d76ccb81cbb9227dd30c24752c73200daeb7463df300d5056c96  healthy/contract-summary.json
9cb240c153b201db3a713f7285c33ff4941e7a4be5e7703e25e37c26b74366c9  healthy/invocation.json
2791765b6d120cb3f7c3dc640e7125b67726d398899478461e54d2d44262ba41  owned/harness-report.json
b9e038fed7004237e8e9faef74bf911ab7e51c686d7ddc0fccb4b280439760df  owned/contract-summary.json
198746e33f13666cd0a8d280afae437a963701751190d44bc57dec990baf47df  owned/invocation.json
258ff056a100baf347b00a6a4db96b01ce604cde4a5e1d6482757369fe3bb18  owned/owned-mpv-recovery.json
6e8d35b6b56bf7d44e27243fad28db531c3fd7b5a08f5b2ee6abd07e83152909  fault/harness-report.json
9d7a325770b84ee47b9b0376e46b334a8e3a23f50e38f478b47ef2af3711c47d  fault/contract-summary.json
d404f7d180f9cdb2a3b97d6e6207747607b1fde5fc319b40cfcf30fb84677897  fault/invocation.json
1f0f0e7cc409d4ab83fa8d272f9c07785e1949b148802585f7f7148e4e476597  fault/faulting-http-recovery.json
```

## Focused validation

Environment:

```text
Microsoft Windows NT 10.0.26200.0
rustc 1.97.1 (8bab26f4f 2026-07-14)
cargo 1.97.1 (c980f4866 2026-06-30)
mpv v0.41.0-877-ge5486b96d
```

Focused results:

```text
cargo test -p sorotte-player-mpv interrupted_network_stream_recovery_tests
# 15 passed, 0 failed

cargo test -p sorotte-gui --bin sorotte-gui-native-smoke \
  --features gui-native-smoke real_mpv_vertical
# 16 passed, 0 failed

python -m unittest scripts.tests.test_gui_real_mpv_vertical_contract -v
# 19 passed, 0 failed

cargo clippy -p sorotte-player-mpv --all-targets --all-features -- -D warnings
# passed

cargo clippy -p sorotte-gui --all-targets --all-features -- -D warnings
# passed

cargo fmt --all --check
git diff --check
# passed
```

Final integrated validation before the three canonical native campaigns:

```text
python -m unittest discover -s scripts/tests -p "test_*.py" -v
# 504/504 passed in 26.328 seconds before the native campaigns
# 504/504 passed again in 27.397 seconds after evidence finalization

actionlint -config-file .github/actionlint.yaml \
  .github/workflows/rust-ci.yml .github/workflows/rust-fuzz.yml
# passed

behavior catalog
# 20 behaviors, 51 exact proofs, 2 evidence lanes

ignored-test policy
# all 23 ignored tests accounted for: 7 maintenance, 12 manual, 4 PR

known-defect policy
# 0 defects, 0 characterizations

mutation policy
# 10 scheduled shards, 17 exact accepted compiler-unviable identities

cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
# passed in 7.28 seconds

cargo test --locked --workspace --all-features
# passed in 220.1 seconds with only registered ignored tests
```

Implementation binding:

```text
dc90540646c0b081259354832cc53e9e13bc4b062a4b8a2679ca2aca8ac92da2  crates/sorotte-player-mpv/src/adapter.rs
cb8a91781e4152f7abc4c3ceb02c815cc3bedd49fbc6333426d6096100a14de3  crates/sorotte-gui/src/bin/sorotte-gui-native-smoke/native_smoke_runner/real_mpv_vertical.rs
4ab34c4e21e7c3a782048ba4d95da52b7a2b73c1ac303156fb6dbc3b064104f2  scripts/gui_real_mpv_vertical_contract.py
ba1d4d27aa66f82660e28032cfe85b17e9e5ee18ac162a07a0236f237a83eb58  scripts/tests/test_gui_real_mpv_vertical_contract.py
```

## Limitations

- This is one Windows build and one exact installed mpv snapshot. It is not
  evidence for minimum-supported, newest-supported, macOS, BSD, or native
  Linux GUI behavior.
- The fault is one malformed HTTP/1.1 chunk boundary after a deterministic
  AU prefix. It does not cover a connection that remains open forever, DNS,
  TLS, HTTP/2, proxy, downloader, or public CDN behavior.
- The fixture proves the same-process reload path. Owned-process death and
  replacement remain covered by the separate recovery inventory.
- The 15-second remaining-duration gate and finite retry budgets reduce false
  recovery and loops; finite testing cannot prove every timing interleaving.
- Loopback ownership is asserted at bind and accept time. This is not an
  operating-system network sandbox and makes no claim about unrelated
  processes.
- Generated bundles remain ignored. This committed record binds them by exact
  paths, structured fields, and hashes; it does not place multi-megabyte
  screenshots, logs, or media under source control.
