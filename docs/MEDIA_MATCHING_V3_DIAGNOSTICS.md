# Media Matching V3 Diagnostics

V3 has one production implementation: audio-only constellation matching with the fixed sampled-fast policy. The normal cache, GUI background indexing, diagnostic CLI, and report comparison all use that policy.

The fixed sampled-fast policy is:

- 3 sampled windows.
- 20 seconds per window.
- 60 total sampled seconds.
- 8000 Hz mono PCM through ffmpeg.
- 384 audio index/verify landmarks.
- ffmpeg audio extraction uses `-map 0:a:0`, `-vn`, `-sn`, and `-dn`.

The supported surface is intentionally narrow: fixed sampled-fast audio matching.

## Diagnostic CLI

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.json --output reports/audio.json --cache-root .media-match-v3-cache --refresh-cache
```

Useful modes:

```powershell
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.json --list-cases
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.json --validate-only
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.json --retrieval-benchmark-only --cache-root .media-match-v3-cache
cargo run -p sorotte-media-match --bin v3_diagnostics -- --cache-size-report --cache-root .media-match-v3-cache
cargo run -p sorotte-media-match --bin v3_diagnostics -- corpus.json --prepare-index-stats --cache-root .media-match-v3-cache
```

`--index-mode sampled-fast` is accepted for explicitness. Other index modes are invalid.

`--refresh-cache` ignores existing SQLite fingerprint rows for the selected files and writes new fixed sampled-fast records. Memory-cache reuse within a run is still allowed for duplicate paths.

## Manifest Shape

Candidate IDs are optional stable pair identities. They are recommended when comparing reports across machines or different roots. IDs must be non-empty and unique within a case.

```json
{
  "profile": "audio-constellation-v3",
  "baseDir": ".",
  "cases": [
    {
      "name": "same-episode-x264-x265",
      "query": "episode-x264.mkv",
      "candidates": [
        {
          "id": "same-episode-x264-x265",
          "path": "episode-x265.mkv",
          "expectedRetrieved": true,
          "maxRetrievalRank": 1,
          "skipDecisionExpectation": true,
          "expectWithinTopK": true,
          "maxTopRank": 3
        }
      ],
      "hardNegatives": [
        {
          "id": "wrong-episode-shared-op",
          "path": "wrong-episode.mkv",
          "mustNotBeTopRank": true,
          "mustNotBeatCandidateId": "same-episode-x264-x265"
        }
      ]
    }
  ]
}
```

For production retrieval validation, rank 1 is a strict ranking metric. User-facing success is top-K retrieval eligibility: the expected candidate should be within `maxTopRank`, default 3. Sampled-only matches are not autoplay-eligible.

## Report Comparison

```powershell
cargo run -p sorotte-media-match --bin v3_report_compare -- reports/baseline.json reports/current.json
cargo run -p sorotte-media-match --bin v3_report_compare -- --strict reports/baseline.json reports/current.json
cargo run -p sorotte-media-match --bin v3_report_compare -- --net-failures-only reports/baseline.json reports/current.json
```

Exit codes:

- `0`: comparison input is valid and the selected gate passes.
- `1`: comparison input is valid and the selected gate fails.
- `2`: invalid report, incompatible report, bad arguments, parse error, or unsupported mode.

Reports are validated before comparison. Duplicate comparison keys, blank candidate IDs, mismatched summary counts, unsupported index modes, or non-production sampled policies are invalid input.

Compatibility requires matching algorithm version, fingerprint cache version, profile, settings hash, and tuning. `--allow-different-profile`, `--allow-different-settings`, and `--allow-different-tuning` exist for explicit exploratory comparisons only. There is no normal cross-profile workflow.

## GUI And Runtime Policy

Background indexing writes only fixed sampled-fast audio records.

Exact local playlist matches are special-cased:

- If the current local file is exactly the shared playlist target and wire sharing is off, fingerprinting is skipped.
- If wire sharing is on, the local file may still be fingerprinted so the client can publish its sampled-fast signature to the room.

Autoplay remains conservative:

- Same normalized path can be treated as exact.
- Sampled-only evidence can be `Probable`.
- Sampled-only evidence is never `Strong`/autoplay eligible.

No automatic seek or sync behavior is attached to media matching in this branch.
Adjacent, split, or merged episodes may appear above the expected file in strict
rank-1 diagnostics; production validation should check that the expected file is
inside the top-K retrieval budget.

## Cache

The normal SQLite cache stores one fixed sampled-fast policy in the compact
audio-only schema. Records generated with any other settings hash are
incompatible and must be regenerated.

Cache size reports expose total bytes, anchor/index bytes, fingerprint blob bytes, row counts, and bytes per anchor/fingerprint. Schema resets are acceptable for V3 work.

## Troubleshooting

- Use `--validate-only` to catch missing paths before indexing.
- Use `--list-cases` to inspect the manifest without fingerprinting.
- Use `--refresh-cache` to rebuild stale or suspect cache rows.
- Use `--cache-size-report` to confirm the compact SQLite schema size.
- Use a minimal manifest with one query, one expected candidate, and hard negatives
  when triaging a rank collision.

## First Real-Corpus Checklist

1. Run `--validate-only`.
2. Run `--list-cases`.
3. Run a cold fixed sampled-fast report with `--refresh-cache`.
4. Run a warm fixed sampled-fast report with the same cache root.
5. Run `v3_report_compare` against cold and warm reports.
6. Check strict rank-1 results separately from top-K retrieval results.
7. Review hard negatives.
8. Do not tune thresholds until misses and rank collisions are categorized.
