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

The runner uses `SOROTTE_MEDIA_MATCH_FFMPEG` and
`SOROTTE_MEDIA_MATCH_FFPROBE` when set; otherwise it resolves `ffmpeg` and
`ffprobe` from `PATH`. V3 does not require `fpcalc`.

## Manifest

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
  and piecewise fitting counts
- expectation pass/fail status and failure reason per candidate

`mustBeRetrieved` fails a candidate when direct pairwise matching would pass but
the shared V3 SQLite retrieval path did not shortlist that candidate.

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

