# Media Matching V3 Diagnostics

Sorotte's V3 diagnostic runner evaluates real media pairs with the same V3
fingerprinting, SQLite anchor index, retrieval stats, and decision diagnostics
used by the runtime media matcher.

## Run

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- manifest.json --output report.json
```

By default the runner creates a temporary cache root and deletes it after a
successful run. If expectations fail, the temporary cache is retained for
inspection.

Use a persistent cache root when comparing multiple runs:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- manifest.json --output report.json --cache-root .media-match-v3-cache
```

Use `--keep-cache` to retain an automatically generated temporary cache:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- manifest.json --keep-cache
```

Use `--list-cases` to inspect a manifest without touching media files, and
`--validate-only` to parse, resolve, and verify referenced files without
fingerprinting:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- manifest.json --list-cases
cargo run -p sorotte-media-match --bin v3_diagnostics -- manifest.json --validate-only
```

Use `--case` one or more times to run or inspect only selected cases:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- manifest.json --case same-episode-x264-x265 --output reports/one-case.json --cache-root .media-match-v3-cache
```

Use `--refresh-cache` to ignore matching SQLite fingerprint records for the
selected manifest/cases, re-extract them, and overwrite those V3 cache rows:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-refresh.json --cache-root .media-match-v3-cache-audio --refresh-cache
```

Use `--index-mode` to separate retrieval calibration from full verification:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-full.json --cache-root .media-match-v3-cache-audio --index-mode full
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-sparse-full.json --cache-root .media-match-v3-cache-audio-sparse-full --index-mode sparse-full
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.sampled.json --output reports/audio-sampled-fast.json --cache-root .media-match-v3-cache-audio-sampled-fast --index-mode sampled-fast
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.sampled.json --output reports/audio-sampled-normal.json --cache-root .media-match-v3-cache-audio-sampled-normal --index-mode sampled-normal
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-sampled-then-full.json --cache-root .media-match-v3-cache-audio-promote --index-mode sampled-then-full --max-full-promotions 3
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-production.json --cache-root .media-match-v3-cache-audio-production --index-mode production --max-full-promotions 3
```

Modes:

- `full`: full-file verification fingerprints are used for both retrieval and
  direct decisions.
- `sparse-full`: full-duration audio is decoded with lower-cost extraction
  settings and a smaller final landmark set. It is useful for likely/current
  candidates, but direct decisions are still capped below `Strong`; dense
  `full` verification is still required for `SameCutStrong` autoplay.
- `sampled-fast`: body-distributed audio windows are decoded for a fast
  retrieval index, using fewer/shorter windows and a smaller target landmark
  set. This is the intended first-pass background indexing mode.
- `sampled-normal` (also accepted as `sampled`): uses the larger sampled
  window set for fallback retrieval calibration.
- Sampled-only direct decisions are capped below `Strong`; sampled-only records
  are not autoplay-eligible as `SameCutStrong`.
- `sampled-then-full`: build the sampled index first, retrieve against that
  steady-state index, then full-verify promoted query/candidate pairs for
  direct decisions. By default the top three retrieved candidates per query are
  eligible for promotion; use `--max-full-promotions N` or
  `--promote-expected-candidates` when a diagnostic run intentionally needs a
  different full-verification budget.
- `production`: simulate the runtime policy. It builds sampled-fast records for
  every selected manifest file, retrieves from that index, then dense
  full-verifies only the top promoted candidate(s). The report separates
  `productionSampledIndexMillis`, `productionFullPromotionMillis`, and
  `productionTotalMillis`, plus sampled-indexed and full-promoted file counts,
  worker counts, and integer `filesPerMinute` throughput. Candidate rows also
  include `sampledRetrievalRank` and `finalVerifiedRank` when production
  promotion verifies a sampled hit.

Dense full audio profiles are experimental verification benchmarks, not
background indexing modes. `dense-current` is the correctness baseline. Use
`--dense-audio-profile` to run one candidate profile, or
`--bench-dense-audio-profiles` to produce a machine-readable JSON matrix for
all current candidates:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --index-mode full --dense-audio-profile dense-current --output reports/dense-current.json --cache-root .cache-dense-current --refresh-cache
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --index-mode full --dense-audio-profile dense-gated-v2 --output reports/dense-gated-v2.json --cache-root .cache-dense-gated-v2 --refresh-cache
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --index-mode full --dense-audio-profile dense-fast-combined-candidate --output reports/dense-fast-candidate.json --cache-root .cache-dense-fast --refresh-cache
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --index-mode full --bench-dense-audio-profiles --output reports/dense-profile-matrix.json --cache-root .cache-dense-matrix --refresh-cache
cargo run -p sorotte-media-match --bin v3_report_compare -- --allow-different-settings --allow-different-tuning reports/dense-current.json reports/dense-gated-v2.json
```

Benchmark profiles include `dense-current`, `dense-realfft`, `dense-8k`,
`dense-hop2048`, `dense-8k-hop2048`,
`dense-8k-window1024-hop1024`, `dense-max-peaks-4`,
`dense-pair-retain-16` (also accepted as `dense-pair-retain-lower`),
`dense-gated` (also accepted as `dense-gated-v2`), and
`dense-fast-combined-candidate`. The non-current
profiles change the fingerprint config hash and should be compared with
explicit compatibility allow flags. `dense-realfft` currently occupies the
real-FFT benchmark slot without changing the default frontend; the lower-cost
sample-rate, hop, peak, and pair-retain profiles are the active candidates until
a dedicated real FFT backend is added without a new heavyweight dependency.
`dense-gated` keeps the dense-current spectral settings but adds anchor/target
pair gates to reduce candidate-pair enumeration. A successful gated run should
preserve the same class/tier/offset behavior while lowering
`candidatePairsConsidered`, raising `candidatePairsSkippedByAnchorGate` or
`candidatePairsSkippedByTargetGate`, and reducing direct-decision broad global
fit time. Dense-current currently has two separate pair explosions to watch:
extraction candidate pairs and direct global-fit pair rescans.
Dense profile reports include decode/drain, analyzer, pairing, reservoir,
candidate-pair, and direct-decision timing fields. Do not promote a dense
candidate profile to default unless it preserves same-episode strength, wrong
OP/ED rejection, retrieval rank, offset accuracy, and improves extraction by a
material margin on a mixed corpus.

V3 requires only `ffmpeg` and `ffprobe`. The runner uses
`SOROTTE_MEDIA_MATCH_FFMPEG` and `SOROTTE_MEDIA_MATCH_FFPROBE` when set;
otherwise it resolves `ffmpeg` and `ffprobe` from `PATH`.

With `--cache-root`, the runner reuses valid records from the shared
`index-v3.sqlite3` cache before invoking `ffmpeg`. Cache reuse requires the
normalized path, modified time, size, profile, and fingerprint config hash to
match. The report's `settingsHash` is this V3 fingerprint config hash; it
includes the extraction settings, media-match algorithm version, fingerprint
cache version, V3 schema/version markers, and the reported V3 tuning values.
Within one run, duplicate paths are served by an in-memory cache before SQLite.

Fingerprint source labels:

- `fresh`: extracted during this diagnostic run.
- `memory-cache`: reused from the current process after the same path/settings
  were already loaded or extracted in this run.
- `sqlite-cache`: loaded from the persistent V3 SQLite cache.

`summary.totalExtractionMillis` is current-run fresh extraction time only.
Fingerprints loaded from `sqlite-cache` still report blob/index counts, but they
do not add extraction time.
Diagnostic runs use two-pass indexing: first every selected query/candidate
fingerprint is loaded or extracted and saved into the V3 SQLite index, then
retrieval and direct decisions are evaluated. This makes cold and warm retrieval
quality comparable because every case queries against the same selected indexed
population.
The summary has two source-count families:

- `uniqueFreshFingerprintCount`, `uniqueMemoryCacheFingerprintCount`, and
  `uniqueSqliteCacheFingerprintCount` count each normalized path/settings
  fingerprint once per report.
- `freshFingerprintReportCount`, `memoryCacheFingerprintReportCount`, and
  `sqliteCacheFingerprintReportCount` count every query, candidate, and
  hard-negative report row source occurrence.

A duplicate candidate path can have row source `memory-cache` while the unique
counts still count only the first source for that path/settings key.

Current real-corpus measurements on the Bakemonogatari audio-only manifest put
dense full verification extraction around 13 seconds per 25-minute file on the
tested Windows machine after online reservoirs removed repeated compactions.
Dense full now reports separate PCM drain, analyzer-thread, channel
backpressure, reservoir acceptance, and raw-emission counters so decode
backpressure can be separated from Rust analysis cost. Direct decisions also
report pair collection, fast audio verifier, global fit, timeline-map, evidence
formatting, and total decision timing so verifier regressions are separate from
extraction regressions. Sparse-full is still
experimental: it is lower density and non-autoplay by policy, but it is not the
default speed path unless future corpus reports show a substantial win.
Sampled-fast is the target background-index path. Retrieval and same-cut
verification are fast once fingerprints exist. Use sampled index mode for fast
background shortlist calibration and dense full verification for any `Strong` /
`SameCutStrong` autoplay-eligible result.

Runtime/background rebuilds follow the same split: sampled-fast records are
created in parallel for the library index, while dense full verification is
reserved for the current/open media and top sampled retrieval candidate(s). The
default promotion budget is three candidates per query; sampled-only matches stay
`Probable` and not autoplay eligible.

For retrieval calibration, add case-level `hardNegatives` for same-series wrong
episodes, shared OP/ED cases, adjacent episodes, music-heavy episodes, and
recap/preview-heavy episodes. Hard negatives are fingerprinted into the same
sampled index but are reported separately from positive candidate expectations.
Use `mustNotBeTopRank` when a negative must never win rank 1, and
`mustNotBeatCandidateId` when it must not outrank the expected candidate with
that `id`. Reports include `hardNegativeBestRank`,
`hardNegativeCountAboveCorrect`, `top1IsExpected`, `topKExpectedPresent`, and a
`retrievalMargin` block with top, expected, and best-negative score/offset
scores. Each returned candidate also has `retrievedCandidateDetails` with rank,
total score, best offset bin, best and second offset scores, body/edge region
counts, approximate span, audio/video hit counts, and ratio to the next
candidate. Each returned candidate also reports `queryDurationMs`,
`candidateDurationMs`, `durationCompatibility`, `shortClipPenaltyApplied`, and
`robustScore`; the robust score is used only to rerank the permissive sampled
retrieval shortlist, so short OP/ED clips and one-region collisions remain
discoverable but should not outrank coherent full-episode evidence unless the
evidence is overwhelming. Sampled-only hard-negative diagnostics do not make
any candidate autoplay eligible.

Large warm-cache retrieval reports break `retrievalElapsedMs` into named
stages. The key fields are `statsDirtyCheckMillis`, `statsRefreshMillis`,
`queryAnchorLoadMillis`, `commonBucketFilterMillis`, `sqlPrepareMillis`,
`sqlExecuteMillis`, `sqlHitFetchMillis`, `rustAggregationMillis`,
`robustRerankMillis`, `candidateSortMillis`, `candidateMetadataLoadMillis`,
`retrievedCandidateDetailBuildMillis`, `retrievedPathLoadMillis`, and
`pathLookupMillis`. `retrievalMeasuredStageMillis` and
`retrievalUnaccountedMillis` make timing coverage explicit; warm benchmark runs
should keep unaccounted time close to zero. The `statsRefreshRan`,
`statsBucketsRefreshed`, `statsAnchorRowsScanned`,
`anchorStatsDirtyBeforeRun`, and `anchorStatsDirtyAfterRun` fields show whether
the run paid an anchor-stats refresh cost. Use this command after bulk index
builds or cache surgery to refresh all V3 anchor statistics once before warm
retrieval calibration:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.sampled.json --cache-root .media-match-v3-cache-audio-sampled-fast --prepare-index-stats
```

For a warm-cache retrieval benchmark, use `--retrieval-benchmark-only` with a
sampled-fast cache. The run still validates retrieval expectations and hard
negatives, but skips direct pair decisions and dense promotion so
`retrievalTotalMillis` reflects lookup/ranking cost. The CLI accepts
`--retrieval-strategy auto`, `temp-table`, or `bucket-fetch`; `auto` currently
uses the temp-table indexed join because it is faster on the noisy 4k sampled
index while `bucket-fetch` remains available for comparison:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.sampled.json --index-mode sampled-fast --retrieval-benchmark-only --retrieval-strategy auto --output reports/audio-retrieval-benchmark.json --cache-root .media-match-v3-cache-audio-sampled-fast
```

Use production promotion expectations when rank 1 is useful as a quality metric
but not required for user success. `maxPromotionRank` and
`expectWithinPromotionBudget` let a sampled-fast candidate pass the retrieval
stage when it is within the dense full promotion budget; `maxRetrievalRank`
remains the stricter rank-quality gate.

```json
{
  "id": "same-episode-split-or-merged",
  "path": "expected-episode.mkv",
  "expectedRetrieved": true,
  "maxPromotionRank": 3,
  "expectWithinPromotionBudget": true,
  "skipDecisionExpectation": true
}
```

## Cache Size Reports

Use `--cache-size-report` to inspect a V3 SQLite cache without running
fingerprinting or retrieval:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- --cache-size-report --cache-root .media-match-v3-cache-audio-sampled-fast --output reports/cache-size.json
```

The report includes page counts, free pages, table/index row counts, dbstat
object sizes when SQLite exposes the `dbstat` virtual table, fingerprint blob
bytes, anchor-index bytes, bytes per fingerprint, and bytes per anchor.
Diagnostic run reports also copy the high-level size fields into `summary`:
`dbTotalBytes`, `dbAnchorIndexBytes`, `dbFingerprintBytes`, `dbStatsBytes`,
`dbIndexBytes`, `dbBytesPerFingerprint`, and `dbBytesPerAnchor`.

Current V3 cache schema stores the 32-byte settings hash once in `settings_v3`
and stores sampled-fast anchors as normalized bucket/occurrence rows:
`anchor_buckets_v3(settingsId, modality, bucket, documentFrequency)` plus
`anchor_occurrences_v3(bucketId, fileId, tMs, weight)`. This replaces the older
per-anchor settings-hash layout and removes the separate `anchor_stats_v3`
table for current caches. V3 cache schema resets are expected during active
development; use `--cache-size-report` before and after rebuilding when
measuring storage changes.

On the 4,007-file sampled-fast noisy Monogatari/Anime cache, the compact schema
reduced the SQLite file from roughly 510 MiB to roughly 87 MiB while preserving
the same 1,536,000 anchor occurrences. Treat those numbers as machine/cache
specific, but sampled-fast caches should now be closer to the tens-of-MiB range
than the hundreds-of-MiB range for a few thousand files.

## Corpus Calibration Workflow

Run the audio-first profile before the combined profile. This separates
retrieval/audio alignment failures from video-hardening behavior:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-before.json --cache-root .media-match-v3-cache
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.combined.json --output reports/combined-before.json --cache-root .media-match-v3-cache
```

Warm-cache workflow:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-cold.json --cache-root .media-match-v3-cache-audio
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-warm.json --cache-root .media-match-v3-cache-audio
cargo run -p sorotte-media-match --bin v3_report_compare -- reports/audio-cold.json reports/audio-warm.json
```

Cold and warm reports may differ in `uniqueFreshFingerprintCount`,
`uniqueSqliteCacheFingerprintCount`, `freshFingerprintReportCount`,
`sqliteCacheFingerprintReportCount`, and `totalExtractionMillis`. That is
expected when file identity and settings match and the warm run avoids
re-extraction.

When testing an extraction-code change, either rely on the fingerprint config
hash changing with the cache version/tuning/settings change, or pass
`--refresh-cache`. Do not assume an old cache is valid after changing audio or
video landmark generation. The current hash intentionally includes the reported
V3 tuning values, so tuning changes may create a new cache namespace even when a
specific threshold is retrieval-only. Retrieval or classification code changes
that do not change the reported tuning/hash can reuse the same cache root. For a
cold extraction performance run, use a new cache root or `--refresh-cache` and
inspect fresh extraction timings, blob bytes, and index rows.

After an algorithm or threshold change, rerun with the same manifests and cache
root into new report names, then compare the JSON reports:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-after.json --cache-root .media-match-v3-cache
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.combined.json --output reports/combined-after.json --cache-root .media-match-v3-cache
cargo run -p sorotte-media-match --bin v3_report_compare -- reports/audio-before.json reports/audio-after.json
cargo run -p sorotte-media-match --bin v3_report_compare -- reports/combined-before.json reports/combined-after.json
```

The comparison tool reports new failures, resolved failures, class/tier changes,
retrieval-rank changes, offset-error changes, and aggregate metric deltas. It
uses regression behavior by default and exits nonzero when the current report has
any new expectation failure, any baseline pair missing from the current report,
any new failed pair added in the current report, or any new `mustBeRetrieved`
retrieval miss that was not already a miss in the baseline. A resolved failure
does not cancel out a new failure.

Reports must be compatible by default. `v3_report_compare` rejects comparisons
unless `algorithmVersion`, `fingerprintCacheVersion`, `profile`, `settingsHash`,
and `tuning` all match. Use explicit allow flags only for exploratory
comparisons:

```powershell
v3_report_compare [--strict|--net-failures-only] [--allow-different-profile] [--allow-different-settings] [--allow-different-tuning] baseline.json current.json
```

Do not compare an `audio-constellation-v3` report with a `combined-v3` report
unless you intentionally pass allow flags. Cross-profile comparisons usually
need both `--allow-different-profile` and `--allow-different-settings` because
the profile changes the settings hash. Same-profile comparisons are the normal
calibration path.

Exploratory cross-profile example:

```powershell
cargo run -p sorotte-media-match --bin v3_report_compare -- --allow-different-profile --allow-different-settings reports/audio.json reports/combined.json
```

Do not compare reports from different tuning unless that mismatch is the thing
being inspected.

Comparison modes:

- Default regression mode:
  `v3_report_compare baseline.json current.json`
- Strict current-quality mode:
  `v3_report_compare --strict baseline.json current.json`
- Net failure-count mode:
  `v3_report_compare --net-failures-only baseline.json current.json`

Strict mode exits nonzero for any current failed expectation, any current
`mustBeRetrieved` retrieval miss, or any missing baseline pair. Net mode keeps
the old behavior and exits nonzero only when the current report has more failed
expectations than the baseline.

Exit codes:

- `0`: comparison completed and the selected mode did not fail.
- `1`: comparison completed and the selected mode failed.
- `2`: usage error, invalid JSON, invalid diagnostic report input, or
  incompatible reports.

Reports are validated before comparison. Summary counts must match the candidate
rows, retrieval and aggregate fingerprint totals must match their detailed
metrics, candidate IDs must be non-empty, and duplicate comparison keys are
invalid input rather than regressions.
Report validation assumes reports were generated by the current
`v3_diagnostics` runner and schema. Older or hand-edited reports may fail
validation.

The comparison output includes a top-level `summary` with regression status and
unresolved-failure status plus counts for baseline/current failures, new
failures, resolved failures, missing pairs, new pairs, new failed pairs,
retrieval misses, and new retrieval misses. It also reports aggregate deltas,
including extraction time, retrieval time, source-count metrics, blob bytes,
index rows, and raw hit rows. A top-level `compatibility` block records whether
algorithm version, fingerprint cache version, profile, settings hash, and tuning
matched. A `compatibilityOptions` block records which mismatches were
intentionally allowed for that comparison.

Generated reports should never contain duplicate comparison keys. Validation
rejects duplicate keys before comparison because they make pair-level comparison
ambiguous, and normal comparison output does not include duplicate-key arrays.

Review failures in this order:

1. `mustBeRetrieved`
2. direct decision class/tier
3. offset error
4. autoplay eligibility
5. raw hit rows / common bucket pressure
6. extraction time / blob bytes

## First Calibration Run

Before changing thresholds, build a small but mixed manifest and capture both
profiles with a stable cache root:

1. Start with 5-10 known-good same-cut pairs.
2. Add 3-5 wrong-episode/shared-intro pairs.
3. Add 2-3 different-cut pairs.
4. Add 1-2 dub or same-video-different-audio cases.
5. Add 1-2 crop/letterbox cases.
6. Run `audio-constellation-v3` first, then `combined-v3`.
7. Record retrieval misses, wrong class, false `SameCutStrong`, offset error,
   raw hit row spikes, extraction time, and blob/index size.

Use report filenames that include the profile and either a timestamp or commit
label, for example `reports/audio-2026-05-26.json` and
`reports/combined-2026-05-26.json`. For commit-to-commit comparisons, keep the
same manifests and `--cache-root`, then compare JSON reports with
`v3_report_compare`. If you also need a raw field-level view, follow up with
`git diff --no-index`.

Do not tune thresholds until at least one small mixed corpus report exists for
both `audio-constellation-v3` and `combined-v3`. Tune from report patterns, not
from a single isolated fixture.

Capture first-run notes in a small table before tuning. A reusable template is
checked in at
[`docs/examples/media_matching_v3_calibration_notes.template.md`](examples/media_matching_v3_calibration_notes.template.md).

| Case ID | Expected Class | Actual Class | Retrieval Rank | Offset Error | Issue Category | Likely Cause | Action |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `same-episode-x264-x265` | `SameCutStrong` |  |  |  |  |  |  |

Use these issue categories consistently:

- retrieval miss
- false `SameCutStrong`
- class too weak
- class too strong
- wrong `SameMediaDifferentCut`
- wrong `SharedIntroOutroOnly`
- offset error
- audio/video conflict
- crop/letterbox miss
- extraction time
- raw hit rows
- blob/index size

## Manifest

Start from
[`docs/examples/media_matching_v3_manifest.example.json`](examples/media_matching_v3_manifest.example.json)
for full verification runs, or
[`docs/examples/media_matching_v3_manifest.sampled.example.json`](examples/media_matching_v3_manifest.sampled.example.json)
for sampled-index retrieval-only runs, and replace the placeholder paths with
local media paths.

```json
{
  "profile": "combined-v3",
  "baseDir": "media",
  "cases": [
    {
      "name": "same-episode-x264-x265",
      "query": "episode-x264.mkv",
      "candidates": [
        {
          "path": "episode-x265.mkv",
          "id": "same-episode-x264-x265",
          "expectedClass": "SameCutStrong",
          "minimumTier": "Strong",
          "expectedOffsetMs": 0,
          "maxOffsetErrorMs": 1000,
          "autoplayEligible": true,
          "mustBeRetrieved": true
        }
      ]
    }
  ]
}
```

Relative paths resolve against `baseDir` when present, otherwise against the
manifest directory. Absolute paths are preserved.

Candidate `id` is optional but recommended for real corpus runs. Report
comparison matches pairs by `case.name` plus `candidateId` when `id` is present;
otherwise it falls back to `case.name` plus the candidate path. Use `id` when
reports may be generated from different media roots or machines. Path fallback is
fine for one-machine runs where report paths are stable. Candidate IDs must be
non-empty and unique within a case. When IDs are absent, duplicate candidate
paths within the same case are rejected to avoid ambiguous comparison keys.

Profiles:

- `audio-constellation-v3`
- `combined-v3`

## Report Fields

The JSON report includes:

- `cacheRoot` and `cacheRetained`
- algorithm version, fingerprint cache version, profile, fingerprint config
  hash, and tuning values
- fingerprint source counts: unique counts
  (`uniqueFreshFingerprintCount`, `uniqueMemoryCacheFingerprintCount`,
  `uniqueSqliteCacheFingerprintCount`) and report-row occurrence counts
  (`freshFingerprintReportCount`, `memoryCacheFingerprintReportCount`,
  `sqliteCacheFingerprintReportCount`)
- extraction diagnostics: timings, audio/video landmark counts, blob bytes, and
  streaming audio metrics, including ffmpeg process wall time, PCM drain thread
  time, analyzer thread time, channel backpressure time, queued PCM bytes,
  analyzer time, peak selection time, pairing time, reservoir time, reservoir
  acceptance/rejection counts, final selection time, and sampled/full decoded
  audio seconds/windows
- retrieval diagnostics: bucket counts, skipped common buckets, raw hit rows,
  scored candidates, elapsed time, and retrieved candidate paths
- decision diagnostics: tier, V3 class, explanation, offset, scale, segment
  count, total aligned span, largest gap, edge-only flag, audio/video conflict,
  autoplay eligibility, and piecewise fitting counts
- expectation pass/fail status and failure reason per candidate

`mustBeRetrieved` fails a candidate when direct pairwise matching would pass but
the shared V3 SQLite retrieval path did not shortlist that candidate.
For sampled-index manifests, prefer retrieval-only expectations:
`expectedRetrieved: true`, `maxRetrievalRank: 1`, and
`skipDecisionExpectation: true`. That lets sampled-only runs pass when the
retrieval index is healthy without requiring `Strong`, `SameCutStrong`, or
autoplay eligibility. Use full verification manifests for those stronger
expectations.

Treat retrieval misses differently from direct decision failures:

- A retrieval miss means the indexed landmark query did not shortlist the
  expected file. Inspect bucket counts, skipped-common counts, raw hit rows, and
  retrieval rank first.
- A direct decision failure means the candidate was compared but the evidence
  did not satisfy the expected class/tier. Inspect aligned span, segment count,
  edge-only status, audio/video conflict, offset, and piecewise fit metrics.

Autoplay remains conservative: exact identity can be eligible, and the only
non-exact V3 class eligible for strong same-media autoplay is `SameCutStrong`
with tier `Strong` and user policy allowing it.

## Failure Checklist

Use report data to decide what to tune or fix. Do not tune thresholds from one
isolated fixture without checking the broader corpus.

- Candidate not retrieved: check `retrieved`, `retrievalRank`,
  `queryBucketsSkippedCommon`, `rawHitRowsProcessed`, and whether the expected
  file has enough indexed landmarks.
- Retrieved but direct decision failed: check decision tier/class, segment count,
  total aligned span, largest gap, edge-only status, and audio/video conflict.
- Wrong class: compare expected edit structure with `segmentCount`,
  `totalAlignedSpanMs`, `largestGapMs`, and `edgeOnly`.
- Offset error: compare `offsetSeconds` against `expectedOffsetMs`; then inspect
  piecewise segment starts and `scalePpm`.
- Unexpected autoplay eligibility: confirm only exact identity or `Strong` +
  `SameCutStrong` can pass, and verify the manifest `autoplayEligible`
  expectation.
- Large raw hit row count or common-bucket pressure: inspect skipped-common
  buckets, raw hit rows, and whether static/common audio or video landmarks need
  better rarity filtering.
- Cold extraction cost: inspect `ffmpegProcessWallMillis`,
  `pcmDrainThreadMillis`, `analyzerThreadMillis`, `channelBackpressureMillis`,
  `maxQueuedPcmBytes`, `analyzerMillis`, `peakSelectionMillis`,
  `pairingMillis`, `reservoirMillis`, `landmarksAcceptedIntoReservoir`,
  `landmarksRejectedByReservoir`, `finalSelectionMillis`, `sqliteSaveMillis`,
  and `indexInsertMillis`. If full extraction is the bottleneck, compare with
  `--index-mode sampled-fast`, `--index-mode sampled-normal`, and
  `--index-mode production` before changing thresholds. Runtime rebuild logs
  also include `background/sampledFast/fullVerify` worker counts,
  `queueWait`, `workerWall`, `sqliteWriter`, and indexed/cancelled/resumed file
  counts.

## Dry-Run Command Sequence

For a first corpus dry run, this docs-only command sequence is the recommended
runner convenience. Keep one stable cache per profile and write reports with a
commit or date label:

```powershell
$cacheAudio = ".media-match-v3-cache-audio"
$cacheCombined = ".media-match-v3-cache-combined"
$label = "before"

cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output "reports/audio-$label.json" --cache-root $cacheAudio
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.combined.json --output "reports/combined-$label.json" --cache-root $cacheCombined
```

After a later patch, rerun with a new label and compare:

```powershell
$label = "after"

cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output "reports/audio-$label.json" --cache-root $cacheAudio
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.combined.json --output "reports/combined-$label.json" --cache-root $cacheCombined
cargo run -p sorotte-media-match --bin v3_report_compare -- reports/audio-before.json reports/audio-after.json
cargo run -p sorotte-media-match --bin v3_report_compare -- reports/combined-before.json reports/combined-after.json
```

After validation work, run a self-comparison smoke check before changing
thresholds:

```powershell
cargo run -p sorotte-media-match --bin v3_report_compare -- reports/audio-before.json reports/audio-before.json
```

Self-comparison should produce no regressions, no missing pairs, and no changes.

Run `audio-constellation-v3` first to isolate audio retrieval/alignment issues,
then run `combined-v3` to evaluate video hardening. Keep separate stable cache
roots and report sequences per profile. Compare reports before tuning thresholds.
When first failures appear, classify them as retrieval miss, direct decision
mismatch, class too weak, class too strong, offset error, or cost/storage issue.

Manual real-corpus validation checklist:

1. Run `v3_diagnostics --validate-only` for the manifest.
2. Run `v3_diagnostics --list-cases` and confirm the case IDs are expected.
3. Run sampled-fast audio retrieval with retrieval-only expectations.
4. Run sampled-normal audio retrieval for any cases sampled-fast misses.
5. Run production mode to measure sampled-fast indexing plus top-candidate full
   promotion.
6. Run sparse-full audio with full-duration but non-autoplay expectations when
   probing lower-cost verification.
7. Run dense full audio verification with `--refresh-cache` for truth labels.
8. Self-compare the warm full audio report with `v3_report_compare`.
9. Run cold and warm `combined-v3` reports only after audio retrieval/alignment is understood.
10. Self-compare the warm combined report with `v3_report_compare`.
11. Fill in the calibration notes template for every failure or suspicious cost.
12. Do not tune thresholds until failures are categorized.

## Recommended Corpus

Use a corpus with cases that stress retrieval, alignment, and false-positive
resistance:

- remux of the same source
- x264 vs x265 or AV1
- AAC vs Opus
- TV vs BD offset
- inserted studio/logo segment
- trimmed intro or removed recap
- wrong episode with shared OP/ED
- dub or same-video-different-audio
- same-audio-different-video
- hard subtitles
- crop or letterbox changes
- long movie
