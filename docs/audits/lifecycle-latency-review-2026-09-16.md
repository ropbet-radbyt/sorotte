# Playlist and player lifecycle latency review

Date: 16 September 2026 (Australia/Sydney)  
Source: `b86696ec6075d2b5bf92bec17d4abc02277847eb` (`v0.2.17` workspace), refreshed from `origin/main`.  
Review worktree: `tmp/lifecycle-latency-review`, branch `codex/lifecycle-latency-review`.

This document records the **baseline review phase**. The worktree now also
contains an uncommitted prototype, measured in the
[implementation experiment and follow-up plan](lifecycle-latency-implementation-2026-09-16.md).
Statements below about unchanged production behavior and baseline defaults
describe the earlier phase, not the current worktree.

## Conclusion

There is a reproducible, avoidable multi-second delay on the local-file path.
Media Match extraction runs on a worker, but preparing and committing its index
still block the GUI runtime owner. Repeated fingerprint lookups add whole-index
validation to that same thread. This delays handling the next playlist command
and servicing the session, including propagation to peers with Media Match disabled.

Two Windows Sandbox runs with a synthetic index approximately the size of the
installed index reproduced **4.0–5.3 seconds to select/load the second file**
and **3.6–3.7 seconds to propagate a subsequent append to peers**. An empty index
or disabled Media Match produced tens-of-milliseconds results for those steps.

The recommended first implementation is to move index preparation, validation,
and commit I/O off the runtime owner and cache generation-bound fingerprint
lookups. Changing polling intervals is lower priority. Preserve the existing
playlist, server, resolution, and player ownership boundaries.

A separate rapid-control issue also needs a correctness follow-up: a seek shortly
after Pause can reach local mpv, be reported as successful, then be rolled back
when the server rejects its older transport revision.

## What was measured

Three production GUI runtime owners, each using its normal runtime thread and
threaded TCP transport, connected to the production server actor/network loop on
loopback. Each owned a separate real mpv process and separate local copies of
generated 60-second video/audio media. Independent mpv Lua observations recorded
`file-loaded`, pause, and position changes.

The initiator and one peer had Media Match fingerprinting/wire sharing enabled;
the third peer had the plugin disabled. Broad background warmup was disabled to
isolate exact-file fingerprint sharing. Files had matching basenames, so this
does not require approximate audio matching on the receiving peers.

The Sandbox had 8 GiB RAM and networking/device/clipboard redirection disabled.
All measured media and SQLite work ran on its local virtual disk. Mapped folders
were used to transfer the payload and export results. No physical input was
injected into the host.

The existing index's file size was inspected read-only: 103,092,224 bytes
(98.3 MiB). Its contents were not used. Synthetic databases used the real schema
plus a padding table; the 98 MiB case was 102,948,864 bytes. This isolates the
copy/validation cost of index size, rather than modeling a real library's row
distribution.

### End-to-end examples

Times are action submission to independently observed mpv load, or to the
playlist change appearing in the client's shell projection. Values below are
rounded milliseconds from retained Sandbox run 02. Subsequent control failures
do not invalidate these completed earlier observations, but the entire run was
not a passing functional qualification.

| Configuration / action | Initiator | Peer, Media Match on | Peer, Media Match off |
|---|---:|---:|---:|
| Empty index: first file loaded | 31 | 31 | 31 |
| Empty index: append second row | 8 | 16 | 17 |
| Empty index: select/load second file | 45 | 76 | 88 |
| Empty index: append third row | 6 | 13 | 19 |
| 98 MiB index: first file loaded | 132 | 162 | 162 |
| 98 MiB index: append second row | 535 | 542 | 559 |
| 98 MiB index: select/load second file | **5,308** | **4,876** | **4,876** |
| 98 MiB index: append third row | 179 | **3,577** | **3,578** |

Run 03 independently repeated the large-index result: select/load took
4,317 / 3,960 / 3,960 ms; the next append took 237 / 3,712 / 3,713 ms.
Both Media Match configurations completed append → select → another append →
delete in that run. The follow-on edit is significant because the first add to
an empty room can complete before fingerprint preparation starts blocking.

Ordinary completed Play/Pause transitions generally took about 6–104 ms across
the clients. This is a local processing baseline, not a WAN latency claim or a
guarantee that every control request succeeds.

Evidence:

- [Run 02 timeline, including the five-second selection](../../target/latency-review/sandbox-baseline-02/output/gui-on-98mib/stage-trace.json)
- [Run 03 large-index measurements](../../target/latency-review/sandbox-baseline-03/output/gui-on-98mib/measurements.json)
- [Run 03 empty-index measurements](../../target/latency-review/sandbox-baseline-03/output/gui-on-empty/measurements.json)
- [Payload/source identities](../../target/latency-review/sandbox-baseline-03/payload/manifest.json)
- [Source additions sealed for run 03](../../target/latency-review/sandbox-baseline-03/source/)
- [Tracked experiment patch sealed for run 03](../../target/latency-review/sandbox-baseline-03/experiment.patch)

## Findings

### P1 — Whole-index preparation and activation block unrelated runtime work

[queue_exact_playlist_signature_worker](../../crates/sorotte-gui/src/app/runtime_owner/requests/media_match.rs)
calls `prepare_media_match_index_rebuild_backup` before spawning the extraction
thread. Worker completion is consumed by `pump_media_match_background_worker`,
which calls `commit_media_match_background_index_backup` on the runtime owner.
The same pattern also serves broader index jobs and checkpoint completion.

[MediaIndexBuildTransaction](../../crates/sorotte-media-match/src/media_index.rs)
copies and validates the complete SQLite database at preparation, then copies,
validates, and durably activates it at commit. Its online backup advances only
64 pages before sleeping 5 ms. With ordinary 4 KiB pages, 98 MiB requires about
392 pauses per copy: almost two seconds of deliberate waiting before the I/O,
checks, and durable activation costs are included.

Measured medians across three iterations on the host:

| Synthetic padding | Prepare | Commit | Open activated index |
|---|---:|---:|---:|
| 0 MiB | 15 ms | 27 ms | 4 ms |
| 8 MiB | 223 ms | 242 ms | 15 ms |
| 32 MiB | 869 ms | 909 ms | 48 ms |
| 98 MiB | **2,601 ms** | **2,693 ms** | **149 ms** |

The Sandbox primitive test independently measured 2,407 / 2,505 / 90 ms at
98 MiB. In the complete three-client run, individual owner pumps lasted as long
as 3,751 ms. The host's second-file selection arrived while the previous
fingerprint was being committed.

An optimized release-build cross-check measured **2,566 ms preparation /
2,650 ms commit / 147 ms activated open** at 98 MiB (three-run medians).
The multi-second copy cost therefore persists outside the debug GUI probe.

A test-only 1,024-page batch experiment retained all validation and activation
semantics. At 98 MiB, host medians fell to **651 ms preparation / 792 ms commit**.
That supports the diagnosis, but still leaves excessive synchronous work.
The production batch remains 64 pages.

Evidence: [baseline](../../target/latency-review/index-host-baseline/index-timings.json),
[batch-size experiment](../../target/latency-review/index-host-batch1024/index-timings.json),
[Sandbox primitive measurements](../../target/latency-review/sandbox-baseline-02/output/index-baseline/index-timings.json).
The [release-build measurements](../../target/latency-review/index-release-baseline/index-timings.json)
also record retained-session point lookups averaging about 0.006–0.008 ms.

### P1 — A fingerprint lookup repeatedly performs whole-index validation

[runtime_detached.rs](../../crates/sorotte-gui/src/app/runtime_detached.rs)
computes the wire signature during session/player synchronization.
[media_match_record_for_path](../../crates/sorotte-gui/src/app/media_match_support.rs)
opens a new `MediaIndexSession` for each lookup.
Opening an activated generation invokes
[`PRAGMA quick_check`](../../crates/sorotte-media-match/src/v3_index.rs), plus
schema validation. This work is repeated even while a missing fingerprint is
still being generated.

The runtime pump invokes session synchronization multiple times before handling
queued GUI requests and between transport phases. During the reproduced
five-second selection, repeated individual lookups cost approximately
118–223 ms, in addition to a 2,597 ms commit. Thus moving only the copy will
leave a second source of delay.

A point lookup through an already-open validated session averaged about
0.056–0.070 ms in the 98 MiB synthetic test. This is evidence that repeated
database opening/validation dominates that experiment; it is not a measurement
of real-library audio-match query complexity.

Use a retained index session or worker-owned read service keyed to the active
generation, with cached present/missing fingerprint results keyed by root,
generation, file identity/size/mtime, and extraction settings. Invalidate on
activation, file change, settings/root change, or an explicit retry. Keep full
integrity validation when admitting a generation and on recovery; do not simply
delete it.

### P2 — Rapid controls can be rejected after local mpv has already changed

With an empty index in run 03, a seek 200 ms after a completed Pause:

1. Staged a request for 10 seconds with transport revision 24.
2. Moved local mpv to 10 seconds in about 7 ms and showed a success notification.
3. Received a canonical corrective response at revision 26, around 0.835 seconds.
4. Returned local mpv to the old position; peers never reached 10 seconds.
5. Succeeded for all three clients in 9–18 ms when issued again two seconds later.

The large-index case repeated the same failure and successful later retry.
The Media Match-off case also lost a rapid Play: its revision-18 intent met a
revision-20 correction. That run retained a test failure rather than counting
the rollback as successful playback.

This is a command/authority handoff issue, not a long operation that eventually
completes. The server's stale-revision protection must remain. Investigate
serializing or conditionally rebasing a user's pending command across their
own preceding acknowledgement; distinguish a competing room command that must
win. A discarded command also needs an explicit terminal outcome. The current
[seek success notification](../../crates/sorotte-gui/src/app/runtime_owner/requests/playback.rs)
only proves that local mpv accepted a command.

The packet sequence and rollback are confirmed. The full cause of the intervening
revision advances has not been established in this pass.

Evidence: [empty-index packet trace](../../target/latency-review/sandbox-baseline-03/output/gui-on-empty/stage-trace.json),
[seek outcomes](../../target/latency-review/sandbox-baseline-03/output/gui-on-empty/measurements.json),
[failed rapid Play](../../target/latency-review/sandbox-baseline-03/output/gui-off.stderr.log).

### Additional areas to instrument before changing

| Area | Evidence and risk | Next measurement |
|---|---|---|
| Local path validation | `reconcile_local_shared_playlist_media_paths` calls `is_file()` across retained local bindings on repeated pumps. Quick resolution and import also perform synchronous metadata/path checks. | Large playlists, sleeping disks, mapped drives/UNC paths; count and time filesystem calls. The multi-second impact here was not reproduced. |
| mpv command submission | `capture_authoritative_playlist_baseline` reads the player's playlist before loading; `loadfile` and other IPC commands wait for replies. The command timeout is five seconds, despite transport having its own actor. | Delay an IPC response independently of file loading; verify incoming room actions continue and command/attachment/media identities survive timeout and supersession. Five seconds is a failure deadline, not an expected load duration. |
| Scheduling | GUI runtime idle poll is 50 ms, TCP worker poll is 25 ms, player readback maintenance interval is 100 ms. Some paths already wake on work. | Trace event-to-wake delay after removing blocking I/O. Do not start by globally reducing timers or creating a busy loop. |
| Unresolved media | Folder search, fingerprint extraction, approximate matching, storage access, and stream startup can genuinely take seconds. | Cold vs warm cache, nested libraries, differing filenames, missing files, concurrent scans, real video files and remote storage. |
| Room readiness/buffering | Waiting for participant readiness, eligibility, or a configured barrier can be intentional. | Time request → server decision → release separately from receiving the media and from physical player readiness. |

## Implementation plan

### 1. Remove the two measured blockers

- Give a background index service ownership of preparation, copying, validation,
  durable activation, and retained read sessions. The GUI runtime continues to
  own user intent and consumption of results.
- Bind every result to its job, config root/settings, and index generation.
  Keep manifest compare-and-swap, stale-base rejection, atomic activation,
  recovery validation, cancellation, and cleanup guarantees.
- Keep reusable index results separate from permission to open the currently
  selected media. A stale selection result must never open or control a player.
- Cache positive and negative exact-fingerprint results until their inputs or
  the active index generation change.
- Consider larger backup batches within the worker after measuring contention.
  Treat batch tuning as an additional reduction, not the responsiveness fix.

### 2. Close the rapid-command correctness gap

- Reproduce Pause → Seek and Pause → Play with controlled delivery of the
  preceding acknowledgement and a concurrent canonical revision change.
- Preserve the server's rejection of stale commands. Define when a later user
  intent may be retried, when it is superseded, and what the UI reports.
- Keep playlist write receipts, canonical selection/reset acknowledgement,
  local resolution, adapter command acceptance, and physical observation
  distinct throughout this work.

### 3. Add stage-specific diagnostics and useful feedback

Use one interaction ID with per-process monotonic timestamps and explicit
connection/room/media/playlist/job identities:

`queued → handled → playlist staged → frame written → server applied →
peer received → source resolved → player command accepted → file loaded →
playback ready/advancing → UI projected`

Instrument index preparation/extraction/commit separately from that sequence.
A write receipt means the bytes were written; it does not mean the server
accepted the mutation. A `file-loaded` event is not proof of first-frame
presentation or advancing playback. Do not subtract unsynchronized clocks
between machines.

Surface the actual outstanding condition when a step exceeds a short threshold
(e.g. 300–500 ms, to be tuned):

- “Finding this file in your media folders” with the current search phase.
- “Identifying the audio for Media Match” with extraction progress.
- “Waiting for the room to accept the selection” when that authority is pending.
- “mpv accepted the file; waiting for it to finish opening.”
- “Waiting for Alex to be ready” or the existing buffering/barrier reason.
- A clear failed/superseded outcome for a command the room did not accept.

Index maintenance should continue without holding unrelated playlist or player
commands. Explain maintenance in diagnostics; a loading animation would not
address the measured scheduling defect.

### 4. Acceptance matrix and proposed budgets

After implementation, use release-built GUI clients with real rendering and
repeat cold/warm trials at 0 / 8 / 32 / 100+ MiB, Media Match off/on, and active
scan/cancel/commit conditions. Include first add, append without changing media,
select another item, append/delete in the same session, and rapid controls.

Proposed engineering targets, to validate rather than advertise:

- Normal owner work stays within a 10 ms budget; no long disk/DB/IPC wait on it.
- Local playlist projection p95 under 100 ms.
- Same-name local-file append propagation p95 under 150 ms on loopback.
- Already-attached local selection/load observation p95 under 250 ms on the
  reference machine; actual first-frame presentation measured separately.
- Increasing index size must not materially increase unrelated command latency.

Then inject 25 / 100 / 250 ms RTT, jitter, delayed write/acknowledgement, reordered
player observations, and a slow resolver/player. Record at least 30 repetitions
per performance cell before quoting p95. Preserve all ownership and stale-result
regressions, including failure/cancellation during database activation.

## Artifacts and validation (baseline phase)

Only opt-in experiment code and test-only tracing were added. Product ownership,
production backup size, player commands, and user-facing behavior are unchanged.
Nothing was committed, pushed, merged, or published. Existing dirty work in the
original checkout was preserved.

The experiment source is in the two `latency_review.rs` modules, the
`latency_review_probe_tests.rs` recorder, and
[sandbox launcher](../../scripts/lifecycle-latency-sandbox.ps1) /
[guest runner](../../scripts/lifecycle-latency-guest.ps1).
The only batching override is compiled under `cfg(test)`.

The retained campaign is observational and contains failures:

- Sandbox run 01 did not execute usable probes. Runtime DLL staging and process
  exit-code collection were corrected before using any Sandbox timings.
- Run 02 reproduced the slow playlist path and seek rollback.
- Run 03 completed both Media Match scenarios, including follow-on edits and
  explicit seek outcome collection; the disabled scenario retained a rapid-Play
  failure. A test process completing does not imply every recorded seek succeeded.
- All three Sandboxes were stopped by exact ID; host cleanup receipts are
  retained beside each run.

Validation passed: formatting and diff-whitespace checks; Clippy for the GUI
and Media Match libraries/tests with warnings denied; one append/select/edit
regression, four playlist-delivery-fence regressions, and 27 media-index
integrity/activation/recovery regressions. The optimized primitive benchmark
also completed. See the [evidence summary](../../target/latency-review/summary.json)
and [validation logs](../../target/latency-review/build-logs/).
Full workspace, native rendering, hosted release, and mutation qualification
were not run for this experiment-only review.

### Reproducing the probes

From this review worktree, build the opt-in test executables, then use a fresh
output directory for every run:

```powershell
# Run from the checkout root. Record the executable paths Cargo prints.
cargo test --locked --offline -p sorotte-gui --lib --no-run
cargo test --locked --offline -p sorotte-media-match --lib --no-run
$latencyRun = Join-Path (Get-Location) ('target\latency-review\rerun-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/lifecycle-latency-sandbox.ps1 `
    -RunDirectory $latencyRun -GuiProbePath $guiTestExecutable -IndexProbePath $indexTestExecutable `
    -MpvPath $mpvExecutable -FfmpegPath $ffmpegExecutable -FfprobePath $ffprobeExecutable `
    -MediaPath $generatedMedia -RequiredIndexMediaPath $generatedRequiredIndexMedia
```

Assign the variables above to the freshly built test executables, standalone
mpv/FFmpeg/ffprobe and two synthetic media fixtures. These measurements used
the retained `target/latency-review/media/generated.mkv` and the 180-second
`generated-required.mkv`; real media is unnecessary. The launcher seals input
hashes, launches an isolated offline Sandbox, and stops that exact Sandbox in
`finally`. It refuses to run alongside an existing Sandbox and returns a
failure if the probes or cleanup fail. Read `output/completion.json` and each
scenario's measurements for the actual outcomes and timing limits.

The index primitive can also be repeated without launching players or a VM:

```powershell
$env:SOROTTE_LATENCY_OUTPUT = Join-Path (Get-Location) ('target\latency-review\index-rerun-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
cargo test --locked --offline --release -p sorotte-media-match --lib latency_review_index_transaction_scaling -- --ignored --nocapture --test-threads=1
```

During the baseline phase, an unset `SOROTTE_REVIEW_BACKUP_PAGES` selected 64
pages. The current prototype defaults to 1,024 pages. Set the test-only override
to `64` to repeat the old index primitive; use the baseline checkout to repeat
the unchanged GUI behavior. The commands above follow the current checkout's
contents.

### Measurement limits

GUI timing runs used debug test builds, mpv `v0.41.0-877-ge5486b96d`, generated
media, null audio/video outputs, and a 5 ms shell-observation loop. They exercise
real runtime/TCP/player behavior but exclude native rendering, file-picker
interaction, WAN/TLS latency, actual user media, and the user's complete mpv
configuration. They establish a reproducible mechanism and an optimization
priority, not a universal bound for the original live session.
