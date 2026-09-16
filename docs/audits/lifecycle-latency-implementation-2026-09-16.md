# Playlist lifecycle: implementation experiment and follow-up plan

Date: 16 September 2026 (Australia/Sydney).  
Base: `b86696ec6075d2b5bf92bec17d4abc02277847eb`, v0.2.17.  
Worktree: `tmp/lifecycle-latency-review`, branch `codex/lifecycle-latency-review`.  
Status: uncommitted prototype and review; not release-qualified.

Follow-up: the native Pause defect and exact-signature job invalidation risk
below have since been addressed. See the
[native Pause follow-up and release plan](native-pause-lifecycle-review-2026-09-16.md)
for the final 36-operation Sandbox campaign and current validation. The tables
below retain the original implementation-run measurements.

## Result

The experiment removes the measured multi-second index stalls from the GUI
runtime owner. With a 98 MiB synthetic index, selecting an already resolvable
local file fell from **4.0–5.3 seconds to 24–84 milliseconds**. A subsequent append
reached peers in **16–24 ms**, compared with about **3.7 seconds** before.

A file that genuinely needs identification is a different case. A peer with a
differently named, unindexed copy needed **15.33 seconds** before mpv opened it.
The playlist row appeared in **6–55 ms**, and an append while that peer was still
waiting propagated in **6–47 ms**. Indexing remains real work, but it no longer
holds unrelated playlist/session work on the runtime thread.

The prototype also prevents an observed server-command echo from being issued
again as a new local pause gesture. Rapid Pause → Seek stayed at the requested
position in all three original final configurations. Native mpv Pause was an
intermittent correctness defect at that stage, reproduced before and after the
performance changes. The linked follow-up now records its fix and repeated
real-player validation; hosted release qualification remains outstanding.

## Measurements

Same production GUI runtime owners, loopback TCP server, three independent real
mpv processes and external Lua observations as the
[baseline review](lifecycle-latency-review-2026-09-16.md).
Client order in all tables is host, peer with Media Match, peer without it.
Times are milliseconds; these are individual observations, not p95 estimates.

### Same-name files, 98 MiB index

| Action | Baseline host / MM peer / local peer | Prototype host / MM peer / local peer |
|---|---:|---:|
| First file physically opened | 147 / 63 / 23 | 19 / 25 / 85 |
| Append second row | 250 / 258 / 259 | 9 / 17 / 17 |
| Select/open second file | 4,317 / 3,960 / 3,960 | 24 / 78 / 84 |
| Append third during fingerprint work | 237 / 3,712 / 3,713 | 6 / 24 / 16 |
| Delete after selection | See baseline trace | 9 / 18 / 19 |

The table uses retained baseline run 03 and implementation run 05. The wider
4.0–5.3 second baseline range also includes run 02. A first add was not always
slow: the large stall was especially visible when an index operation overlapped
the next command. This explains why a single happy-path add test can miss it.

In the final 98 MiB run, runtime-pump medians were **3.7–5.0 ms**, with observed
maxima **43–73 ms**. In the required-index run the maxima were **35–45 ms**.
The index prepare/commit spans now ran on named background worker threads.
These finite samples do not establish a worst-case bound.

### What happens when indexing is required

The host and local-only peer had `episode-1.mkv`. The Media Match peer had only
`alternate-copy.mkv`, containing identical generated 180-second audio/video.
Its index contained no fingerprint for that file and background warmup was on.
Host wire sharing was on. No filename shortcut could resolve the MM peer.

| Observation | Host | MM peer | Local-only peer |
|---|---:|---:|---:|
| Playlist row visible | 6 | 24 | 55 |
| File physically opened | 18 | **15,330** | 67 |
| Subsequent append, while MM peer waiting | 6 | 47 | 29 |

The causal sequence was:

1. The room accepted and distributed the playlist item immediately.
2. The host opened its known local file and extracted its shareable signature.
3. The unresolved peer received that signature and identified its local candidate.
4. Runtime approval admitted the completed index generation; the result woke
   resolution, which selected the candidate through the existing source policy.
5. mpv opened the candidate **paused**. Opening was not autoplay authorization.

The host's extraction reported **5.29 seconds**, and the peer's **5.37 seconds**.
Host signature publication occurred about **7.40 seconds after the add**. In this
cold path those identification jobs were substantially sequential. Index
preparation/commit each added roughly **0.65–0.84 seconds** on their workers,
plus lookup, scheduling and resolution work. This is not network latency.

The match had 384 aligned audio anchors spanning 114.464 seconds, classified
`SameCutProbable`. Existing policy allowed opening the candidate, but the
sampled-only evidence did not grant full autoplay verification. The matching
thresholds and readiness gates were unchanged.

An earlier 60-second fixture also indexed successfully, but only provided
18.688 seconds of aligned evidence. Its `Weak` / `PartialOverlap` classification
correctly prevented automatic selection. Waiting longer would not turn that
completed weak match into a strong one. The UI must distinguish **indexing**,
**insufficient matching evidence**, and **waiting for playback readiness**.

### Index work itself

Three-trial medians for a synthetic 102,948,864-byte database in the Sandbox:

| Primitive | Baseline | Prototype |
|---|---:|---:|
| Prepare staged generation | 2,453 ms | 493 ms |
| Commit generation | 2,483 ms | 609 ms |
| Validated open | 95 ms | 102 ms |

The prototype uses 1,024 SQLite pages per backup step instead of 64. The backup
API yields 5 ms between steps, so the former setting accumulated seconds of
deliberate waits. Integrity validation, activation locking, durability and
recovery remain in place. Validated-open cost did not disappear: the reader
retains an admitted connection on a worker instead of repeatedly paying it on
the runtime thread.

Evidence caveat: implementation runs 03–05 retained a stale `"64"` label in the
primitive benchmark JSON even though their sealed source used 1,024. The raw
evidence is preserved. The probe now reports the same effective batch setting
used by the backup helper; no recorded times were rewritten.

## Prototype changes and ownership

- **Worker owns all index I/O.** Preparation, extraction, activation and abort
  cleanup moved together into `media_match_worker.rs`. The runtime grants an
  explicit activation/abort decision for that job. A dropped decision channel
  aborts an unadmitted stage. Once granted, atomic activation finishes; a late
  cancellation suppresses selected-media effects and reports that the completed
  index was retained.
- **Scope survives asynchronous work.** Root and settings changes reject stale
  admission. Player-path and room-target changes suppress obsolete selected-media
  results. A finished job cannot migrate to a successor job's channel.
- **Exact lookups are asynchronous.** `media_match_lookup.rs` owns one reader
  worker, one in-flight request, a bounded inventory cache and a fingerprint
  cache bound to root, path, extraction settings, file metadata and invalidation
  epoch. Pending, missing and failed results are distinct. Results wake the
  runtime and retry selected-item resolution.
- **Reader admission remains enforced.** `MediaIndexRecordReader` checks manifest
  replicas, generation directories, database metadata and SQLite data version.
  A changed or failed observation re-enters normal admission/recovery. The old
  connection is released before recovery or cache removal.
- **Reset waits for owned cleanup.** Cache clear and full application-data reset
  cancel/finish the index worker and release the reader before deleting its
  files. Completion is asynchronous and still goes through the runtime's shell
  action boundary.
- **Source signatures stay available.** Publishing the current source's
  fingerprint no longer consumes its sharing scope and withdraws it on the next
  pump. Receivers still do not acquire unsolicited signature-sharing ownership.
- **Player-command observations remain telemetry.** A matching observed pause
  command receipt is consumed once, preventing it from re-originating as a new
  local gesture. A subsequent native edge still takes the normal input path.
  Server command revision checks were not relaxed.
- **Feedback names actual work.** A selected unresolved item whose current
  scoped job is indexing shows `Resolving` with “Media Match is indexing local
  files for this item.” Existing progress distinguishes preparation, extraction
  and saving. The feedback is not applied to another item or an obsolete root.

The playlist frame/receipt fence, source preference policy, server room intent,
player load correlation and readiness decisions retain their existing owners.

## Outstanding review findings and recommended order

### 1. Native Pause race: addressed in follow-up

In the final run, a peer's native mpv Pause was observed locally in 8–16 ms but
was reversed in both Media Match configurations. Neither other player settled
paused within the 1.5-second observation window. The disabled case passed, but
an unchanged-base disabled case also reproduced reversal. It is timing-sensitive
and not attributable simply to Media Match being enabled.

The test uses an mpv Lua command to change the physical player's pause property,
without sending a Sorotte GUI pause request. It exercises native property input,
not a physical keyboard event. Follow-up reproduced three interacting causes:
correction before stable confirmation, excessive recovery ownership and
same-revision heartbeats canceling the candidate. The fix retains server
stale-command rejection and passed all 36 native operations in the final mixed
configuration campaign. See the linked follow-up for failing-before evidence.

The rapid Pause → Seek echo correction is promising: all final cases stayed at
10 seconds. This is not proof of all timing permutations, and the second seek
to the already-reached target provides no additional movement-latency sample.

### 2. Reduce cold identification work without changing confidence policy

Prioritize the selected file ahead of broad library warmup. Investigate whether
an already running scan can supply a compatible candidate signature instead of
preparing another whole-index transaction. Preserve job identity, file-version
checks, stage isolation and cancellation semantics. Reuse valid fingerprints
across repeated selections; never promote weak evidence merely to shorten a
wait. The 15-second example is real work today, not an immutable lower bound.

### 3. Finish cache/job invalidation and slow-storage review before promotion

The path-only exact-signature job key has been replaced by a key containing
file version, index root and settings. Queue-level regressions cover replacement
and changed context without suppressing required work. The follow-up records
this correction; the slow-storage validation below remains outstanding.

Filesystem metadata checks and some tool-health checks still run on the owner.
Local disk timings do not cover sleeping drives or NAS paths. Move expensive
checks behind the existing resolver/worker boundary where those measurements
justify it. The 250 ms external-index refresh policy also needs review under
multiple processes and very large inventories.

### 4. Add concise lifecycle diagnostics and qualify the change

Keep a correlated per-action timeline from queued command through room
acceptance, source resolution, mpv load and readiness. Show the outstanding
stage after a short threshold; show insufficient-evidence or rejected-command
outcomes promptly. Keep tool counters and database timings in diagnostics.

Split the production changes into reviewable index-worker, retained-reader,
signature-lifetime, command-observation and feedback slices. Then run repeated
release/native-rendering trials, delayed/reordered player events, real libraries,
NAS/sleeping disks and 25/100/250 ms RTT plus jitter. Use at least 30 repetitions
per performance cell before quoting p95. Full hosted/release qualification is
still required before shipping.

## Validation and retained evidence

- GUI unit suite: 1,227 passed, 8 ignored; Media Match: 101 passed, 2 ignored.
- All 14 GUI semantic scenarios passed, including indexing tooltip projection,
  asynchronous reset and both live Python interoperability cases.
- Full workspace tests: **4,323 passed, 25 ignored**. Clippy across the workspace
  and all targets passed with warnings denied. Formatting and diff-whitespace
  checks passed. Optional/ignored external gates are not release qualification.
- Worker tests cover activation blocking without blocking the runtime, aborted
  staging, dropped admission channels, obsolete root/settings/selection results
  and cancellation after activation. Reader tests cover generation replacement,
  WAL changes, corruption, invalidation and releasing connections before clear.
- Sandbox run 05 collected all four GUI cases. Its `passed` process status means
  collection completed; native Pause failure flags remain in the measurements.
  Every Sandbox was stopped by its exact ID and the final inventory was empty.
- Original dirty checkout was preserved. No commit, push, merge or release.

Evidence:

- [Final experiment and cleanup receipts](../../target/latency-review/sandbox-implementation-05/)
- [Required-index measurements](../../target/latency-review/sandbox-implementation-05/output/gui-required-index-98mib/measurements.json)
- [98 MiB measurements](../../target/latency-review/sandbox-implementation-05/output/gui-on-98mib/measurements.json)
- [Earlier weak-match evidence](../../target/latency-review/sandbox-implementation-04/output/gui-required-index-98mib/measurements.json)
- [Baseline native-control comparison](../../../lifecycle-native-control-review/target/latency-review/sandbox-native-baseline-01/output/gui-off/measurements.json)
- [Validation logs](../../target/latency-review/build-logs/)
- [Machine-readable implementation summary](../../target/latency-review/implementation-summary.json)

The final Sandbox sealed the tracked diff, all untracked Rust sources and input
binary/media hashes before launch. Subsequent edits only adjusted semantic reset
waiting and made the primitive's batch-size label use its actual setting.
The current full checks cover those edits.

GUI timings used debug test builds, null mpv audio/video outputs, a 5 ms observer,
generated media and effectively zero network RTT. They exclude native rendering,
first visible frame, real media diversity, WAN/TLS effects and personal mpv
configuration. The synthetic index uses the real schema plus padding, not a
real 98 MiB library's row distribution. These are measured mechanisms and a
prototype comparison, not advertised end-user latency guarantees.

To repeat semantic coverage in this worktree, point `SYNCPLAY_LEGACY_ROOT` at
the already installed `.interop-cache/syncplay-legacy` in the original checkout.
Use the [launcher](../../scripts/lifecycle-latency-sandbox.ps1) with a fresh run
directory after rebuilding both test executables. Supply explicit executable
and fixture paths as in the [probe recipe](lifecycle-latency-review-2026-09-16.md#reproducing-the-probes).
The 180-second fixture is used
only for the required-index case. Baseline reproduction requires the baseline
checkout; current default backup batching is 1,024 pages.
