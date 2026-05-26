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

V3 requires only `ffmpeg` and `ffprobe`. The runner uses
`SOROTTE_MEDIA_MATCH_FFMPEG` and `SOROTTE_MEDIA_MATCH_FFPROBE` when set;
otherwise it resolves `ffmpeg` and `ffprobe` from `PATH`.

## Corpus Calibration Workflow

Run the audio-first profile before the combined profile. This separates
retrieval/audio alignment failures from video-hardening behavior:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.audio.json --output reports/audio-before.json --cache-root .media-match-v3-cache
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.combined.json --output reports/combined-before.json --cache-root .media-match-v3-cache
```

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
unless `algorithmVersion`, `profile`, `settingsHash`, and `tuning` all match.
Use explicit allow flags only for exploratory comparisons:

```powershell
v3_report_compare [--strict|--net-failures-only] [--allow-different-profile] [--allow-different-settings] [--allow-different-tuning] baseline.json current.json
```

Do not compare an `audio-constellation-v3` report with a `combined-v3` report
unless you intentionally pass `--allow-different-profile`. Likewise, do not
compare reports from different settings hashes or tuning unless that mismatch is
the thing being inspected.

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

The comparison output includes a top-level `summary` with regression status and
unresolved-failure status plus counts for baseline/current failures, new
failures, resolved failures, missing pairs, new pairs, new failed pairs,
retrieval misses, and new retrieval misses. It also reports aggregate deltas,
including extraction time, retrieval time, blob bytes, index rows, and raw hit
rows. A top-level `compatibility` block records whether algorithm version,
profile, settings hash, and tuning matched.

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

Capture first-run notes in a small table before tuning:

| Case ID | Expected Class | Actual Class | Retrieval Rank | Offset Error | Issue Category | Likely Cause | Action |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `same-episode-x264-x265` | `SameCutStrong` |  |  |  |  |  |  |

Use these issue categories consistently:

- retrieval miss
- false `SameCutStrong`
- wrong `SameMediaDifferentCut`
- wrong `SharedIntroOutroOnly`
- offset error
- extraction time
- raw hit rows
- blob/index size

## Manifest

Start from
[`docs/examples/media_matching_v3_manifest.example.json`](examples/media_matching_v3_manifest.example.json)
and replace the placeholder paths with local media paths.

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
- algorithm version, profile, settings hash, and tuning values
- extraction diagnostics: timings, audio/video landmark counts, blob bytes, and
  streaming audio metrics
- retrieval diagnostics: bucket counts, skipped common buckets, raw hit rows,
  scored candidates, elapsed time, and retrieved candidate paths
- decision diagnostics: tier, V3 class, explanation, offset, scale, segment
  count, total aligned span, largest gap, edge-only flag, audio/video conflict,
  autoplay eligibility, and piecewise fitting counts
- expectation pass/fail status and failure reason per candidate

`mustBeRetrieved` fails a candidate when direct pairwise matching would pass but
the shared V3 SQLite retrieval path did not shortlist that candidate.

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

## Dry-Run Command Sequence

For a first corpus dry run, keep one stable cache per profile and write reports
with a commit or date label:

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
