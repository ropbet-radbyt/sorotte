# Media Matching V3 Calibration Notes

Use this template while reviewing real-corpus diagnostic reports. Capture the
first observations before changing thresholds so tuning is based on report data,
not isolated fixtures.

| Case ID | Profile | Expected Class | Actual Class | Tier | Retrieved | Retrieval Rank | Offset Error | Issue Category | Notes | Action |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `same-episode-x264-x265` | `audio-constellation-v3` | `SameCutStrong` |  |  |  |  |  |  |  |  |
| `same-episode-x264-x265` | `combined-v3` | `SameCutStrong` |  |  |  |  |  |  |  |  |

## Issue Categories

- retrieval miss
- false SameCutStrong
- class too weak
- class too strong
- wrong SameMediaDifferentCut
- wrong SharedIntroOutroOnly
- offset error
- audio/video conflict
- crop/letterbox miss
- extraction time
- raw hit rows
- blob/index size
