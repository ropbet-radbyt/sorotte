## Recommendation

Build V3 as an **audio-first sparse-landmark matcher with video hardening**, not as a bigger V2.

The core change should be: replace V2's Chromaprint-token-derived audio anchors and per-frame luma hashes with a **native Sorotte fingerprint engine** that produces sparse, time-local, high-entropy landmarks across the whole timeline. Use audio for primary retrieval because it is cheaper to decode than video and is usually the strongest signal across remuxes/re-encodes. Use video only as a second-stage confirmer or fallback for dubs, missing audio, commentary tracks, or audio collisions.

The closest known model is Shazam-style spectrogram peak-pair fingerprinting: extract robust local spectral peaks, hash pairs of peaks, store each hash with its anchor time, then find candidates by hash hits and offset/line alignment. Wang's Shazam paper explicitly frames good fingerprints as temporally localized, translation-invariant, robust, and sufficiently entropic; it uses spectrogram peaks, pairs time-frequency points into compact hashes, stores the hash with offset time, and scores matches by clustering offset differences. ([Columbia Electrical Engineering][1])

This is a better fit for Sorotte than V2 because Sorotte needs **same-timeline evidence**, offsets, trims, re-encodes, and false-positive resistance, not a global metadata identifier.

---

## What the literature implies for V3

Audio-fingerprinting surveys describe the same constraints you are running into: fingerprints must remain compact, robust to compression/distortion, able to identify excerpts, tolerant of shifts/lack of synchronization, and searchable without scanning the whole database. They also call out the engineering trade-off between dimensionality reduction and information loss, plus the need for fast, memory-efficient, updateable search structures.

The Shazam design is directly relevant because it turns audio into sparse time-local landmark hashes. It avoids comparing raw audio or large token vectors. The database lookup is an inverted index from hash to `(file_id, time)`, and the decision is based on whether hits form a strong offset cluster or diagonal relationship in time. That maps cleanly to Sorotte's problem: "can I align this local playback timeline to another media file?" ([Columbia Electrical Engineering][1])

There is also research on local audio fingerprints that are invariant to time/frequency scale changes and can estimate tempo/pitch transformations. That is useful for V3's drift and speed-change goals, but I would not implement full pitch-scale invariance first; it is more complex than Sorotte likely needs for ordinary TV/DVD/BD timing differences. Borrow the idea of **local features plus robust scale/offset fitting**, not necessarily the full time-chroma pipeline. ([arXiv][2])

For video, PDQ/TMK+PDQF is the relevant family of prior work: PDQ is image similarity hashing, and TMK+PDQF is video similarity hashing; Facebook open-sourced those algorithms and the independent paper reviews their behavior under common transformations. The lesson for Sorotte is not "embed Facebook's full pipeline," but that video should be treated as a **temporal similarity signal**, not just isolated frame hashes. ([arXiv][3])

Recent ACR work also points toward compact fingerprints, temporal correlation, and approximate nearest-neighbor style retrieval as ways to improve speed and storage. I would not jump to GPU/ANN dependencies for Sorotte V3, but the same idea applies locally: store a compact retrieval sketch and load richer binary blobs only for a short candidate list. ([arXiv][4])

---

## Chosen V3 strategy

Use one strategy:

```text
V3 = audio-first sparse constellation landmarks
   + compact binary verify blobs
   + tiny retrieval index
   + IDF/rarity-weighted offset voting
   + robust piecewise timeline alignment
   + optional video hardening/fallback
```

Do not build V3 around deep neural embeddings, full-video hashing for every file, or "more Chromaprint." Those are the wrong default trade-offs for a local desktop Syncplay-style client:

* Deep embeddings add model size, runtime dependencies, privacy questions, and platform complications.
* Full-video indexing is expensive and unnecessary for the common case.
* Chromaprint is useful, but V3 needs control over landmark density, timestamp locality, rarity, blob encoding, and alignment.

The important design principle is **two levels of fingerprint data**:

1. A **small retrieval index**: a few rare landmarks per file, stored in queryable SQLite rows.
2. A **richer verification blob**: more landmarks in packed binary form, loaded only for top candidates.

V2 stores summary blobs plus anchor tables. V3 should reduce SQLite row overhead by indexing only a curated subset and keeping verification data in compact blobs.

---

## V3 architecture

### 1. Fingerprint modalities

#### Audio primary: `audio-constellation-v3`

Pipeline:

```text
ffmpeg decode audio -> mono PCM @ 11025 or 16000 Hz
Rust STFT -> log magnitude spectrum
local peak picking -> sparse constellation points
peak-pair hashing -> 32-bit or 40-bit landmarks
deterministic winnowing -> bounded full-timeline coverage
rarity-aware retrieval subset -> small SQLite index
```

A landmark should look like:

```rust
struct AudioLandmarkV3 {
    hash: u32,       // quantized f1, f2, dt, optional band flags
    t_ms: u32,       // anchor time
    weight: u8,      // peak strength / rarity / quality
}
```

Target density:

```text
verification blob:
  episodes: 384-768 audio landmarks
  movies:   512-1024 audio landmarks

retrieval index:
  episodes: 96-192 indexed audio landmarks
  movies:   128-256 indexed audio landmarks
```

Do **not** store every generated peak-pair hash. Generate many internally, then winnow to a bounded, full-body representation. This preserves coverage while keeping DB size predictable.

#### Video secondary: `video-scene-v3`

Video should be optional/hardening by default.

Pipeline:

```text
ffmpeg/ffprobe selected frames or scene/keyframe frames
border/crop normalization
multi-region perceptual descriptors
temporal visual shingles
rare retrieval subset
verification blob
```

Each selected frame should produce more than one descriptor:

```text
global pHash / DCT hash
center-crop hash
edge/gradient hash
optional low-resolution color histogram hash
```

Then build temporal shingles:

```text
hash(frame_i), hash(frame_j), delta_t
```

This is closer to "video landmarks" than raw single-frame matching. It improves resistance to static frames, black frames, repeated intros, watermarks, hard subtitles, and letterbox/crop changes.

Suggested storage:

```rust
struct VideoLandmarkV3 {
    bucket: u32,     // LSH or shingle bucket
    hash64: u64,    // verification hash
    t_ms: u32,
    kind: u8,       // global, crop, edge, temporal-shingle
    weight: u8,
}
```

Target density:

```text
verification blob:
  64-192 visual landmarks

retrieval index:
  16-64 visual landmarks
```

### 2. SQLite schema

Use new tables. No compatibility constraints.

```sql
CREATE TABLE media_files_v3 (
    file_id INTEGER PRIMARY KEY,
    normalized_path TEXT NOT NULL UNIQUE,
    modified_unix_millis INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL,
    duration_ms INTEGER,
    container_format TEXT,
    video_codec TEXT,
    audio_codec TEXT,
    width INTEGER,
    height INTEGER,
    updated_unix_millis INTEGER NOT NULL
);

CREATE TABLE fingerprints_v3 (
    file_id INTEGER NOT NULL,
    algorithm_version INTEGER NOT NULL,
    settings_hash BLOB NOT NULL,
    status TEXT NOT NULL,
    duration_ms INTEGER,
    audio_blob BLOB,
    video_blob BLOB,
    audio_verify_count INTEGER NOT NULL DEFAULT 0,
    video_verify_count INTEGER NOT NULL DEFAULT 0,
    audio_index_count INTEGER NOT NULL DEFAULT 0,
    video_index_count INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    updated_unix_millis INTEGER NOT NULL,
    PRIMARY KEY (file_id, algorithm_version, settings_hash),
    FOREIGN KEY (file_id) REFERENCES media_files_v3(file_id) ON DELETE CASCADE
);

CREATE TABLE anchor_index_v3 (
    algorithm_version INTEGER NOT NULL,
    settings_hash BLOB NOT NULL,
    modality INTEGER NOT NULL,       -- 1 audio, 2 video
    bucket INTEGER NOT NULL,
    file_id INTEGER NOT NULL,
    t_ms INTEGER NOT NULL,
    weight INTEGER NOT NULL,
    PRIMARY KEY (
        algorithm_version, settings_hash, modality, bucket, file_id, t_ms
    )
);

CREATE INDEX idx_anchor_index_v3_lookup
    ON anchor_index_v3(algorithm_version, settings_hash, modality, bucket);

CREATE TABLE anchor_stats_v3 (
    algorithm_version INTEGER NOT NULL,
    settings_hash BLOB NOT NULL,
    modality INTEGER NOT NULL,
    bucket INTEGER NOT NULL,
    document_frequency INTEGER NOT NULL,
    updated_unix_millis INTEGER NOT NULL,
    PRIMARY KEY (algorithm_version, settings_hash, modality, bucket)
);
```

Key difference from V2: **the index table is not the full fingerprint**. It is a retrieval sketch. Verification uses the blob.

### 3. Binary blob format

Use versioned binary, not JSON.

```text
magic: "SMM3"
version: u16
duration_ms: u32/u64
section_count: u8

section audio:
  count varint
  entries:
    delta_t_ms varint
    hash u32
    weight u8

section video:
  count varint
  entries:
    delta_t_ms varint
    bucket u32
    hash64 u64
    kind u8
    weight u8
```

Use delta-coded sorted timestamps. Add zstd only after measurement; varint/delta packing may already be enough. Keep decoding simple.

Target average sizes:

```text
audio-only V3 fast:
  blob: 1.5-4 KB
  index rows: 96-192 rows/file

audio+video hardened:
  blob: 3-8 KB
  index rows: 128-256 audio + 16-64 video rows/file
```

This can still be smaller than V2 in total DB size if V3 indexes far fewer rows and avoids duplicating all verification anchors in SQLite rows.

---

## Matching algorithm

### Candidate retrieval

1. Fingerprint the current file with `audio-constellation-v3`.
2. Pick retrieval anchors:

   * rare anchors first,
   * distributed across the timeline,
   * avoid first/last edge regions unless the file is short.
3. Query `anchor_index_v3` by bucket.
4. Skip or downweight high-document-frequency buckets using `anchor_stats_v3`.
5. For each hit, vote:

```text
candidate_id
offset_bin = candidate_t_ms - query_t_ms
score += idf(bucket) * anchor_weight
```

6. Keep top `N` candidates by:

   * weighted score,
   * dominant offset-bin score,
   * evidence span,
   * number of distinct timeline regions.

Suggested defaults:

```text
top candidates to verify: 16
minimum indexed audio hits: 4
minimum dominant-offset score: tuned from corpus
skip buckets appearing in >5% of indexed files
```

### Verification

Load `audio_blob` and optional `video_blob` for top candidates. Then run robust alignment.

Use three fitting modes:

```text
constant offset:
  candidate_t = query_t + offset

affine:
  candidate_t = scale * query_t + offset

piecewise:
  local aligned segments chained across timeline
```

V2's affine fitting is a good start. V3 should add **piecewise segment chaining**:

1. Generate matching landmark pairs.
2. Build local affine hypotheses.
3. Score short aligned regions.
4. Chain compatible regions with dynamic programming.
5. Return a timeline map, not just one offset.

This matters for:

```text
TV broadcast vs DVD/BD with extra logos
trimmed intro
removed recap
different ending credits
PAL/NTSC-style speed differences
special edition scenes
commercial cuts
```

For Syncplay, the best output is:

```rust
struct MediaTimelineMapV3 {
    global_tier: MatchTierV3,
    current_position_tier: MatchTierV3,
    segments: Vec<AlignedSegmentV3>,
}

struct AlignedSegmentV3 {
    query_start_ms: u32,
    query_end_ms: u32,
    candidate_start_ms: u32,
    candidate_end_ms: u32,
    scale_ppm: i32,
    audio_score: f32,
    video_score: f32,
    confidence: f32,
}
```

A file can be globally "partial overlap" but locally safe for the current playback position.

---

## Decision tiers

V3 should distinguish more cases than V2:

```rust
enum MatchClassV3 {
    SameCutStrong,
    SameCutProbable,
    SameMediaDifferentCut,
    SameVideoDifferentAudio,
    SameAudioDifferentVideo,
    PartialOverlap,
    SharedIntroOutroOnly,
    Reject,
    Unknown,
}
```

Only `SameCutStrong` should be autoplay-eligible by default.

Rules:

```text
SameCutStrong:
  audio landmarks align over broad body span,
  offset/scale or piecewise map is stable,
  no contradictory video evidence if video is available,
  evidence not concentrated in intro/outro/common regions.

SameMediaDifferentCut:
  strong aligned body segments, but duration/edit structure differs.

SameVideoDifferentAudio:
  video aligns strongly, audio missing/conflicting; useful for dubs.

SameAudioDifferentVideo:
  audio aligns but video conflicts; diagnostic only unless user allows.

PartialOverlap:
  enough segment evidence for local mapping, not enough for global same-media.

SharedIntroOutroOnly:
  evidence concentrated at edges or high-frequency common anchors.

Reject:
  insufficient or contradictory evidence.
```

This is how V3 covers collisions V2 will still struggle with.

---

## Extraction workflow

### Default background behavior

```text
inventory only on startup/root change
audio V3 fingerprints lazily for current file and likely candidates
background warmup: audio-only V3
video V3: only for ambiguous matches, dubs, missing audio, or explicit hardening
```

This is the biggest speed win. Do not video-fingerprint the whole library by default.

### Current-file matching

```text
1. Ensure current file has audio V3 fingerprint.
2. Query audio index.
3. Fingerprint missing top filename/duration candidates with audio V3.
4. Verify top candidates using audio blobs.
5. If result is weak/probable/conflicting, run video V3 on current + top candidates.
6. Return decision + timeline map.
```

### Tooling

Use ffmpeg only for media decoding. Do the actual fingerprint extraction in Rust:

```text
audio:
  ffmpeg -v error -nostdin -i INPUT -vn -ac 1 -ar 11025 -f s16le -

video hardening:
  ffmpeg/ffprobe selected frames, ideally sparse scene/keyframe extraction
```

Use Rust crates for signal processing:

```text
rustfft or realfft
smallvec / bytemuck if useful
optional zstd only after measurement
```

Avoid `fpcalc` in V3. That removes one external tool and gives you control over storage, timing, density, and matching.

---

## How V3 improves over V2

### Smaller

V3 stores:

```text
one compact verification blob
plus a small retrieval subset
```

rather than treating every anchor as both verification data and an indexed SQLite row.

It also avoids JSON and avoids storing raw token vectors.

### Faster

V3's default path is audio-only and streaming. It avoids video extraction unless needed. Candidate verification loads blobs only for top candidates, not all records.

The Shazam-style inverted index and offset histogram are built for this retrieval shape: query hashes produce candidate/time hits, then the decision becomes finding statistically meaningful offset clusters or aligned lines. ([Columbia Electrical Engineering][1])

### More accurate

V3 improves the false-positive story with:

```text
rarity/IDF weighting
edge-region downweighting
piecewise timeline alignment
audio/video conflict classes
video hardening for dubs and audio collisions
body-span requirements
```

### More durable

V3 covers:

```text
different encodes
remuxes
audio codec changes
offsets
mild drift
trimmed intro/outro
TV vs BD body overlap
shared OP/ED
same video with different dub
same audio over different video
audio missing or broken
hard subtitles / letterbox / crop changes, via video hardening
```

---

## Trade-offs

### Shazam-style audio landmarks vs Chromaprint-derived anchors

Use Shazam-style landmarks.

Chromaprint is easy and already integrated conceptually, but V3 needs exact control over time locality, landmark density, rarity, storage, and full-timeline coverage. Shazam-style landmarks give you the right native structure for offset voting and robust timeline fitting.

### Full audio decode vs sampled windows

Use full low-rate audio decode with bounded landmark selection.

Sampled windows are faster, but they risk missing matches when edits shift content away from proportional sample points. Full low-rate audio decode plus winnowing gives better coverage while still bounding storage.

### Video always-on vs video hardening

Use video hardening only when needed.

Always-on video indexing will be slower and larger. Audio will solve the common case. Video should resolve the cases audio cannot: dubs, commentary tracks, missing audio, same soundtrack over different video, and high-risk collisions.

### Exact Hamming fallback vs better LSH

Prefer better LSH and rarity weighting. Keep bounded fallback only as a safety net.

Full Hamming fallback is simple but can become expensive. V3 should build better multi-projection buckets and maintain document-frequency stats so common buckets do not dominate.

---

## Implementation phases

### Phase 1: V3 schema and binary blobs

Implement `sorotte-media-match-v3` structures in the existing crate.

Deliverables:

```text
MediaFingerprintProfile::AudioConstellationV3
MediaFingerprintProfile::VideoSceneV3 or CombinedV3
AudioLandmarkV3
VideoLandmarkV3
MediaFingerprintBlobV3
encode/decode tests
media_files_v3
fingerprints_v3
anchor_index_v3
anchor_stats_v3
clear-cache support
status display for v3
```

### Phase 2: native audio constellation extractor

Implement audio extraction with ffmpeg PCM decode and Rust FFT.

Algorithm detail:

```text
sample rate: 11025 Hz initially
channels: mono
window: 2048 samples
hop: 512 samples
frequency range: about 250-5000 Hz
spectrogram magnitude: log scale
peak picking: local max in time/frequency neighborhood
density: top N peaks per second or per tile
pairing:
  anchor peak with target peaks 0.4-5.0s after anchor
  quantize f1, f2, dt
  hash to u32
winnowing:
  keep full-body distributed rare/strong anchors
  cap verification count
  cap index count
```

Tests:

```text
same synthetic audio -> same landmarks
offset synthetic audio -> offset recovered
noise/compression-ish transform -> enough landmarks survive
different audio -> no strong match
```

### Phase 3: retrieval and blob verification

Implement the retrieval cascade:

```text
extract/query current file
select rare indexed anchors
SQLite candidate lookup
offset histogram
top-N candidate shortlist
load v3 blobs
robust affine alignment
piecewise segment chaining
decision tiers
```

Tests:

```text
same file
offset
trimmed intro
extra logo
different episode same intro/outro
same audio different video
same video different audio
partial overlap
```

### Phase 4: video hardening

Implement sparse video descriptors and temporal shingles.

Start simple:

```text
frame selection:
  scene/keyframe/sparse time sampling
border crop detection:
  ignore black bars
hashes:
  global DCT pHash
  center crop pHash
  edge/gradient hash
temporal shingle:
  hash_a, hash_b, delta_t bucket
```

Use video only for:

```text
audio unavailable
audio weak/probable
audio/video conflict
explicit hardening
background idle hardening
```

### Phase 5: empirical tuning

Build a corpus and benchmark:

```text
same remux
x264 vs x265 vs AV1
AAC vs Opus vs FLAC
stereo vs 5.1 downmix
TV vs BD offset
trimmed OP
different ED
wrong adjacent episode same OP/ED
dub
commentary
silent/missing audio
hard subtitles
letterbox/crop
trailer vs movie
same song over different video
```

Report:

```text
DB bytes/file
index rows/file
fingerprint time/file
candidate lookup time
verification time
false strong count
offset error
segment-map quality
```

---

## Codex prompt

```text
Implement Media Matching V3 with no backward-compatibility requirement.

Goal:
V3 should be faster, smaller, and more accurate than V2. It should use an audio-first sparse landmark design inspired by Shazam-style time-frequency peak-pair fingerprinting, plus optional video hardening. Do not build V3 as a larger V2. Do not depend on fpcalc for V3.

Core architecture:
1. Add a new V3 profile:
   - AudioConstellationV3 or CombinedV3.
   - V3 uses native Rust audio fingerprint extraction from decoded PCM.
   - Use ffmpeg only to decode audio/video, not to compute fingerprints.

2. Add V3 SQLite schema:
   - media_files_v3
   - fingerprints_v3
   - anchor_index_v3
   - anchor_stats_v3
   The index table stores only a small retrieval subset. Full verification anchors live in compact binary blobs.

3. Add V3 binary blob format:
   - magic "SMM3"
   - explicit format version
   - duration_ms
   - audio landmark section
   - video landmark section
   - varint/delta-coded timestamps
   - no JSON arrays
   Add round-trip tests and corrupted-input tests.

4. Implement audio-constellation-v3 extraction:
   - ffmpeg command:
     ffmpeg -v error -nostdin -i INPUT -vn -ac 1 -ar 11025 -f s16le -
   - Stream PCM into Rust.
   - Compute STFT with rustfft or realfft.
   - Use log magnitude.
   - Detect local spectral peaks in time/frequency tiles.
   - Pair anchor peaks with target peaks in a future target zone.
   - Hash quantized f1, f2, and delta_t into a u32 landmark hash.
   - Store anchor timestamp t_ms and weight.
   - Generate more raw landmarks internally, then winnow to bounded verification and retrieval sets.
   - Cover the whole timeline, not only the first 120 seconds.
   - Avoid overrepresenting intro/outro regions.

5. Retrieval:
   - Use retrieval landmarks only in anchor_index_v3.
   - Maintain anchor_stats_v3 document frequencies.
   - Skip or downweight common buckets.
   - Score candidates by weighted hits and offset histograms.
   - Keep top N candidates, then load V3 blobs for verification.

6. Verification:
   - Load full audio/video V3 blobs for top candidates.
   - Fit constant offset, affine scale+offset, and piecewise aligned segments.
   - Return a timeline map:
     candidate_time = scale * query_time + offset for each segment.
   - Distinguish:
     SameCutStrong,
     SameCutProbable,
     SameMediaDifferentCut,
     SameVideoDifferentAudio,
     SameAudioDifferentVideo,
     PartialOverlap,
     SharedIntroOutroOnly,
     Reject,
     Unknown.
   - Only SameCutStrong is autoplay-eligible.

7. Video hardening:
   - Do not run video extraction for every file by default.
   - Run it for ambiguous/probable matches, audio-unavailable cases, dubs, or explicit hardening.
   - Extract sparse frames via scene/keyframe/time sampling.
   - Normalize black bars/crops.
   - Compute global pHash/DCT, center-crop hash, edge hash, and temporal shingles.
   - Store compact video verification blob and small retrieval subset.

8. Workflow:
   - Startup/root scan inventories only.
   - Current file gets audio V3 fingerprint lazily.
   - Missing top candidates get audio V3 lazily.
   - Background warmup defaults to audio V3 only.
   - Video V3 runs as hardening only.

9. Tests:
   - binary blob round trips
   - offset recovery
   - affine drift recovery
   - piecewise alignment with inserted/removed intro
   - wrong episode with shared OP/ED must not be SameCutStrong
   - same video different dub
   - same audio different video
   - missing audio
   - deleted/modified file invalidation
   - DB size and index-row-count benchmarks
   - extraction timing benchmark

10. Acceptance targets:
   - V3 audio-only average storage should be below V2 combined average for normal episodes.
   - V3 default background warmup must avoid video extraction.
   - V3 must produce no Strong decisions for shared-intro/outro wrong-episode fixtures.
   - Simple offset error should be within +/-1s.
   - Piecewise segment mapping should handle at least trimmed intro and extra-logo fixtures.
```

## First task for Codex

Start with Phase 1 and Phase 2 only. Do not ask Codex to implement all of V3 in one pass.

```text
First implement V3 blob/schema and audio constellation extraction. Add tests and a debug command/report that shows:
- audio landmarks generated
- retrieval landmarks selected
- blob bytes
- index rows
- extraction time
- offset recovery on synthetic fixtures
```

Once that works, implement retrieval and verification. Then add video hardening.

[1]: https://www.ee.columbia.edu/~dpwe/papers/Wang03-shazam.pdf "Microsoft Word - ISMIR-2003-Shazam-rev2.doc"
[2]: https://arxiv.org/abs/1304.0793 "[1304.0793] A local fingerprinting approach for audio copy detection"
[3]: https://arxiv.org/abs/1912.07745 "[1912.07745] PDQ & TMK + PDQF -- A Test Drive of Facebook's Perceptual Hashing Algorithms"
[4]: https://arxiv.org/abs/2305.09559 "[2305.09559] Robust and lightweight audio fingerprint for Automatic Content Recognition"
