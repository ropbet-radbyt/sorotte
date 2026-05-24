# GPT Pro Media Matching Review Output

Based on static inspection of the branch and the public repository context; I did not run a media benchmark corpus locally. The main conclusion is that the current branch is a useful prototype, but the durable solution should be a **compact anchor-indexed timeline matcher**, not a “store full JSON fingerprint records and compare them later” system.

Sorotte’s project context matters here: it is a Rust Syncplay-compatible client/server with GUI/CLI pieces, and its current player target is mpv. The feature should therefore answer a Syncplay-specific question: “are these two users playing the same underlying media timeline, and can we safely use that to reduce false ‘different file’ warnings or permit autoplay/readiness?” It does **not** need to become a general global media-identification database. ([GitHub][1])

## Executive recommendation

Do not try to fix this primarily with SQLite compression. Compression would reduce the 12 KB rows, but it would leave the slow extraction path, weak offset modeling, and non-queryable JSON blob design intact.

Build v2 around four ideas:

1. **Inventory first, fingerprint lazily.** Scan roots cheaply, but fingerprint the current file and a small candidate set first. Background warmup should be opportunistic, not a prerequisite for use.

2. **Store compact anchors, not raw fingerprints.** Convert audio/video fingerprints into time-stamped anchor hashes. Store them in binary blobs and/or normalized anchor tables, not JSON decimal arrays.

3. **Use an inverted SQLite index.** Given the current file’s anchors, look up files sharing anchors, group by candidate, then run expensive alignment only on top candidates.

4. **Match by timeline alignment.** Use offset voting and robust fitting of `candidate_time = scale * query_time + offset`, with strong/probable/weak decisions based on coverage, span, modality agreement, and drift.

This directly addresses your stated goals: smaller DB, faster indexing, robustness across encodes, and support for offsets/drift between TV/DVD/BD versions.

---

## What the current implementation does

The new crate `sorotte-media-match` defines fast and full profiles. The fast profile samples 12 video frames and 120 seconds of audio; the full profile samples video every 10 seconds up to 720 frames and intends to use full audio. A `MediaFingerprintRecord` stores identity, settings, duration, a container fingerprint, optional audio tokens, and optional video frame hashes.

The extraction pipeline shells out to `ffprobe` for duration, `fpcalc` for Chromaprint audio tokens, and `ffmpeg` for video frames. The full video path uses one `ffmpeg` process with `fps=1/{interval}`, but the fast video path launches one `ffmpeg` process per sampled timestamp. With the default fast profile, that means about **12 ffmpeg launches per file**, plus `ffprobe` and `fpcalc`. For 4,000 files, that is roughly 48,000 `ffmpeg` process launches before considering decoding/seeking cost.

The SQLite schema currently has one `fingerprints` row per `(normalized_path, profile)` and stores `record_json TEXT NOT NULL`, plus duplicated columns such as `normalized_path`, `profile`, `modified_unix_millis`, `size_bytes`, `algorithm_version`, `extraction_settings_json`, and `duration_seconds`. Loading the cache reads `record_json` back into full Rust records. The save path serializes the whole `MediaFingerprintRecord` into JSON and inserts/replaces it.

The peer-sharing path places a `mediaMatch` object into the Syncplay-style file payload. The local client reads peer signatures from that field and compares them against the local fast record. The autoplay gate only uses strong media-match tiers when the configured autoplay policy allows it, which is the right safety shape.

---

## Why the DB is large

Your breakdown is consistent with the implementation. The dominant storage cost is `record_json`, especially `audio.fingerprint_tokens: Vec<u32>`, serialized as decimal JSON. That is the worst format for this data: every 32-bit token becomes several ASCII digits plus commas, JSON field names, brackets, and duplicated metadata.

The current schema also duplicates data. For each row, it stores:

* `normalized_path` as a top-level column and again inside `record_json.identity`.
* settings as `extraction_settings_json` and again inside `record_json.extraction_settings`.
* duration as a top-level column and again inside the record.
* profile in a column and inside settings.
* full audio/video vectors even though the DB cannot query them without deserializing the whole row.

SQLite is not the real problem. A 50–100 MB SQLite cache would be acceptable if it were queryable and fast. The issue is that the DB stores unindexed JSON payloads instead of the compact search primitives the matcher actually needs.

A reasonable v2 target is:

* **Fast profile:** under 1–2 KB per file, including both audio and video anchors.
* **Full/hardening profile:** under 4–8 KB per file, but only generated for likely candidates or over time.
* **Peer wire signature:** well under the current 32 KB cap, ideally 1–4 KB.

---

## Why indexing takes hours

The slow path is mostly architectural.

The current fast video extractor runs `ffmpeg` separately for each sparse frame. With 12 frames and 4,000 files, that is about 48,000 process starts and seeks. Even if each one averaged only 250 ms, that is more than three hours for video extraction alone. This aligns with your observed “several hours.”

The audio side is also heavier than needed. Chromaprint is a good tool for near-identical audio identification and duplicate detection, but the current code stores raw token vectors and later compares them with a set-intersection score plus longest common subsequence. ([GitHub][2]) That comparison is not a scalable index strategy, and LCS is a poor default for “find candidates among thousands of files.”

There is also a subtle bug or at least a misleading profile name: `full_v1()` sets `audio_sample_seconds = 0`, and the Sorotte code only passes `-length` when that value is greater than zero. But `fpcalc` itself defaults to 120 seconds unless `-length` is explicitly provided; its source declares `g_max_duration = 120` and documents `-length SECS` as restricting processed duration with default 120. So “full-v1” audio is probably still only the first 120 seconds unless Codex changes the command to pass `-length 0`.

---

## Durability assessment

The current system will probably work for same-file, remux, and some simple transcodes. It is much less certain for the harder cases you care about.

### Audio

Chromaprint is appropriate as a source signal, but storing and comparing raw token sequences is not the right abstraction. The current audio score mixes token-set Jaccard-like overlap with LCS. That can work when the compared audio starts at roughly the same point, but it does not explicitly estimate offset, local overlap, or drift. It will also struggle with alternate cuts where only the middle body overlaps.

Better approach: treat audio as **time-stamped anchors**. Use chunked or rolling fingerprints, hash local token windows, and let matching be “which anchors agree at a consistent time offset?”

### Video

The current video hash is called “PDQ-style,” but the code is closer to a simple 64-bit average luma hash over an 8×8 grid. That can survive some resolution/bitrate changes, but it is fragile against crops, subtitles, letterboxing changes, static scenes, repeated intros/outros, overlays, and fades. The current alignment finds all pairwise frame hashes within a Hamming threshold, greedily assigns pairs, takes a median offset, and computes drift from first/last pairs.

That is a good prototype, but not a strong enough autoplay gate by itself. Wrong episodes from the same series can share openings/endings. Static shots and black frames can collide. A robust matcher should require a dominant offset hypothesis over a meaningful timeline span and should penalize evidence concentrated only in OP/ED regions.

### Offsets and TV/DVD/BD differences

The current model has a single best offset and a drift ratio. That is the right direction, but the alignment method is too weak. For real-world TV/DVD/BD differences, the matcher should produce:

```text
candidate_time ≈ scale * query_time + offset
```

where `scale` is usually near 1.0, but can allow small drift for speed or mastering differences. For more complex edits, it should ideally return aligned segments rather than a single global offset.

For Syncplay/autoplay purposes, a single strong aligned segment covering the current playback region may be enough. A global “same file” decision is less important than “can I map this user’s current timestamp to the other user’s timestamp safely?”

---

## Recommended v2 design

### 1. Keep three layers separate

Use separate concepts in code and storage:

```text
Media inventory:
  path, mtime, size, duration, codec/container metadata, optional cheap partial file hash

Fingerprint summary:
  version, profile, status, compact audio/video summaries, error state

Search anchors:
  time-stamped audio/video hash anchors indexed by hash bucket
```

Do not use `MediaFingerprintRecord` JSON as the database source of truth. Keep a small debug JSON only if useful.

### 2. Replace `record_json` with compact storage

A practical schema:

```sql
CREATE TABLE media_files (
    file_id INTEGER PRIMARY KEY,
    normalized_path TEXT NOT NULL UNIQUE,
    modified_unix_millis INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL,
    duration_ms INTEGER,
    container_format TEXT,
    video_codec TEXT,
    audio_codec TEXT,
    partial_content_hash BLOB,
    updated_unix_millis INTEGER NOT NULL
);

CREATE TABLE fingerprints (
    file_id INTEGER NOT NULL,
    version INTEGER NOT NULL,
    profile TEXT NOT NULL,
    status TEXT NOT NULL,
    settings_hash BLOB NOT NULL,
    duration_ms INTEGER,
    audio_summary BLOB,
    video_summary BLOB,
    audio_anchor_count INTEGER NOT NULL DEFAULT 0,
    video_anchor_count INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    updated_unix_millis INTEGER NOT NULL,
    PRIMARY KEY (file_id, version, profile),
    FOREIGN KEY (file_id) REFERENCES media_files(file_id) ON DELETE CASCADE
);

CREATE TABLE audio_anchors (
    version INTEGER NOT NULL,
    profile TEXT NOT NULL,
    bucket INTEGER NOT NULL,
    file_id INTEGER NOT NULL,
    t_ms INTEGER NOT NULL,
    weight INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (version, profile, bucket, file_id, t_ms)
);

CREATE INDEX idx_audio_anchor_lookup
    ON audio_anchors(version, profile, bucket);

CREATE TABLE video_anchors (
    version INTEGER NOT NULL,
    profile TEXT NOT NULL,
    bucket INTEGER NOT NULL,
    file_id INTEGER NOT NULL,
    t_ms INTEGER NOT NULL,
    hash64 INTEGER NOT NULL,
    weight INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (version, profile, bucket, file_id, t_ms)
);

CREATE INDEX idx_video_anchor_lookup
    ON video_anchors(version, profile, bucket);
```

Notes:

* `bucket` should be a stable 32-bit or 48-bit hash prefix derived from the anchor hash. SQLite stores integers efficiently enough.
* `audio_summary` and `video_summary` should be binary, not JSON.
* Use versioned binary formats with a magic header so future migrations are explicit.
* Store extraction failures explicitly. The current code silently drops audio/video errors with `.ok()`, which can make the cache look healthy when a modality failed.

### 3. Use anchors instead of raw token vectors

For audio:

1. Run `fpcalc` in a mode that gives time locality. Use chunking or a controlled duration.
2. Convert raw tokens into rolling k-gram hashes, for example 3–5 consecutive tokens.
3. Winnow/minhash them so each file stores a bounded number of anchors.
4. Store `(anchor_hash, t_ms, weight)`.

For fast profile, target something like:

```text
64–128 audio anchors
32–64 video anchors
```

For hardening/full profile:

```text
256–512 audio anchors
96–192 video anchors
```

The exact numbers should be empirically tuned, but the key is that storage is bounded and queryable.

### 4. Fix extraction process count

Fast video should not spawn one `ffmpeg` per frame. Use one `ffmpeg` invocation per file for the fast profile. FFmpeg’s `select` filter can select frames by expressions, including time-based spacing; the official docs show `select='isnan(prev_selected_t)+gte(t-prev_selected_t\,10)'` to select frames at least 10 seconds apart. FFmpeg also documents scene-based selection using `gt(scene\,0.4)`, with 0.3–0.5 described as a reasonable range for scene-change comparisons. ([FFmpeg][3])

A good fast extractor should produce frames from one command such as:

```text
ffmpeg -v error -nostdin -i INPUT
  -vf "select='...',scale=64:64:flags=bicubic,format=gray"
  -vsync vfr
  -frames:v N
  -f rawvideo -
```

You will need a reliable way to recover timestamps. Options:

* Use `showinfo` or `metadata=print` on stderr/stdout and parse PTS.
* Emit image frames with frame metadata if rawvideo timestamp recovery is awkward.
* Use `ffprobe` packet/frame metadata for selected frames, then extract nearby frames in one pass.

For audio, fix the full-profile bug:

* For a true unlimited/full Chromaprint run, pass `-length 0`.
* Otherwise rename the profile to make it explicit: `audio-first-120s`.

### 5. Change matching from pairwise scan to candidate retrieval

Current matching deserializes records and compares candidates directly. Instead:

1. Fingerprint the query/current file.
2. Look up its anchor buckets in `audio_anchors` and `video_anchors`.
3. Group hits by `file_id`.
4. Build offset histograms:

```text
offset_ms = candidate_anchor_t_ms - query_anchor_t_ms
```

5. Keep only top candidates by:

   * number of matching anchors,
   * dominant offset-bin strength,
   * evidence span,
   * duration plausibility,
   * filename/episode score as a weak prior.

6. Run robust alignment on only those top candidates.

This avoids comparing the current file to every row and gives you offset estimates as a natural output.

### 6. Use robust alignment

For each candidate, fit:

```text
candidate_t_ms = scale * query_t_ms + offset_ms
```

Use offset voting first, then RANSAC or a similar robust estimator. The decision evidence should include:

```rust
struct TimelineAlignmentEvidence {
    offset_ms: i64,
    scale_ppm: i32,              // e.g. 1_000_000 means 1.0
    aligned_audio_anchors: u16,
    aligned_video_anchors: u16,
    query_coverage: f32,
    candidate_coverage: f32,
    aligned_span_ms: u32,
    first_query_ms: u32,
    last_query_ms: u32,
    first_candidate_ms: u32,
    last_candidate_ms: u32,
    second_best_score_ratio: f32,
}
```

The `second_best_score_ratio` is important. A strong match should not merely have a good offset; it should have a dominant offset that is clearly better than other hypotheses.

### 7. Decision policy

Keep the current tier model, but make it stricter and more explainable.

Suggested rules:

```text
Exact:
  Same file identity, or same strong partial-content hash and metadata.

Strong:
  Audio and video both align to the same offset/drift over meaningful span,
  OR one modality is extremely strong over long span and duration is compatible.
  Eligible for autoplay only if user policy allows.

Probable:
  One modality aligns well, or both align weakly, but span/duration/confidence is insufficient.
  Diagnostic only.

Weak:
  Some shared evidence, possible same media, not enough for automation.

Reject:
  Conflicting offsets, insufficient anchors, or evidence concentrated only in common regions.
```

For strong autoplay I would require all of the following unless testing proves too strict:

```text
dominant offset bin exists
aligned span >= min(10 minutes, 30% of shorter duration) for movies
or aligned span >= 5 minutes / enough distinct scenes for episodes
second-best offset is clearly weaker
drift within configured bound
not solely intro/outro/common-edge evidence
```

For TV/DVD/BD offsets, do not reject solely because total durations differ. If anchors align strongly over the shared body, classify as probable or strong depending on span and modality agreement. Duration mismatch should downgrade confidence when evidence is otherwise thin.

---

## Indexing workflow that fits Sorotte

The workflow should be user-centered rather than “fingerprint my whole library before use.”

### On app start or root change

Do only inventory:

```text
walk media roots
record path/mtime/size
optionally ffprobe duration/codecs in a low-priority queue
do not run full fingerprinting for every file
```

### When the player opens a file

```text
fingerprint current file with fast profile
look up existing anchor matches
shortlist candidates by anchor hits + duration + filename
fingerprint missing top candidates only
return best match / offset
publish compact wire signature
```

### Background warmup

```text
run low-priority
cancelable
1–2 worker concurrency max
fast profile only by default
full profile only for likely matches or explicit hardening
checkpoint after each file
pause if active playback/matching needs CPU
```

The current branch already has a background worker, cancel handling, backup/restore behavior, and fast-then-full hardening concept. Keep those pieces. Change what the worker does.

---

## Wire format v2

The current wire path is conceptually good: peers share a bounded `mediaMatch` signature in the file payload, then each client compares remote signatures to its local file. Keep that, but make the signature compact and anchor-based.

Example shape:

```json
{
  "schema": "sorotte.mediaMatch.v2",
  "profiles": [
    {
      "profile": "fast-anchor-v2",
      "algorithmVersion": 2,
      "durationMs": 1423370,
      "audio": {
        "algorithm": "chromaprint-anchor-v2",
        "timeBaseMs": 1000,
        "anchors": "base64-binary-anchor-block"
      },
      "video": {
        "algorithm": "luma-scene-anchor-v2",
        "timeBaseMs": 1000,
        "anchors": "base64-binary-anchor-block"
      }
    }
  ]
}
```

Do not include local path, size, mtime, or raw filename in the media-match signature. Treat fingerprints as semi-private content-derived identifiers. Keep wire sharing opt-in/easily disabled.

---

## Migration plan

### Phase 1: stabilize and measure current v1

Have Codex add instrumentation before redesigning:

* Count external tool invocations per rebuild.
* Record per-file extraction time: ffprobe, fpcalc, ffmpeg.
* Record serialized row sizes by component.
* Add a CLI or test helper that prints DB size and per-profile counts.
* Add tests for “full_v1 audio actually processes unlimited or explicitly first-120s.”

Immediate fixes:

* Pass `-length 0` when a profile means full audio.
* Replace fast video’s 12 `ffmpeg` launches with one extraction command.
* Persist modality errors instead of silently dropping them.
* Stop storing duplicate `settings_json` and full `record_json` if still on schema v1, or compress it only as a temporary compatibility measure.

### Phase 2: add schema v2 beside v1

Codex should add schema v2 without deleting v1 immediately:

* `media_files`
* `fingerprints`
* `audio_anchors`
* `video_anchors`
* migration marker in `metadata`
* loader that prefers v2 and falls back to v1
* clear-cache removes both schemas
* UI cache status shows inventory/fast/full/anchor counts

### Phase 3: implement anchor extraction

* Audio: convert `fpcalc` output into bounded anchors.
* Video: one-pass extraction, scene/time anchors, better hash than the current 8×8 mean hash if feasible.
* Store anchors in lookup tables and compact summary blobs.
* Add deterministic unit tests for binary encoding/decoding.

### Phase 4: implement anchor matching

* Query anchors against SQLite.
* Build offset histograms.
* Fit robust alignment.
* Return `MediaMatchDecision` with offset/drift/span evidence.
* Keep v1 comparator only for migration/backward compatibility.

### Phase 5: validate

Build a test matrix:

```text
same file remux
same source different H.264/H.265/AV1 encodes
different audio codecs
resolution changes
bitrate changes
cropped/letterboxed versions
soft/hard subtitles
trimmed intro
extra studio logo
TV vs BD offset
wrong adjacent episode with same OP/ED
same movie trailer vs full movie
different video with same audio
same video with different dub
```

Acceptance gates:

```text
No false Strong on wrong-episode/shared-intro corpus.
Strong for normal transcodes/remuxes.
Correct offset within ±1s for simple offset cases.
Probable or Strong for shared-body TV/DVD/BD cases, depending on span.
Initial enable does not require full-library fingerprinting.
Fast extraction uses <= 3 external processes per file.
Fast DB storage target <= 2 KB/file average, excluding SQLite page overhead.
```

---

## Codex instruction you can paste

```text
We need to replace the current media-matching prototype with an anchor-indexed v2 design.

Context:
- Sorotte is a Syncplay-compatible Rust client/server. Media Matching should decide whether two users are playing the same underlying media timeline and, only when confidence is strong, allow readiness/autoplay behavior.
- The current branch stores full MediaFingerprintRecord JSON in SQLite. This is too large (~12 KB/row, mostly raw audio tokens) and not queryable.
- The current fast video extractor launches ffmpeg once per sampled frame. For 4000 files that becomes ~48,000 ffmpeg invocations.
- The current full_v1 audio profile sets audio_sample_seconds = 0 but does not pass `-length 0` to fpcalc, so fpcalc’s default 120-second limit still applies. Fix or rename this.

Build v2 with these requirements:

1. Add instrumentation first:
   - count ffmpeg/ffprobe/fpcalc invocations;
   - measure extraction time per modality;
   - measure serialized DB bytes per file/profile;
   - expose a debug/cache-status summary.

2. Create SQLite schema v2:
   - media_files(file_id, normalized_path, mtime, size, duration_ms, codecs/container, partial_content_hash, updated time)
   - fingerprints(file_id, version, profile, status, settings_hash, duration_ms, audio_summary BLOB, video_summary BLOB, anchor counts, error, updated time)
   - audio_anchors(version, profile, bucket, file_id, t_ms, weight)
   - video_anchors(version, profile, bucket, file_id, t_ms, hash64, weight)
   - indexes on (version, profile, bucket)
   Do not use record_json as the primary v2 storage format.

3. Implement compact binary summary encoding:
   - versioned magic header;
   - bounded anchor count;
   - delta-coded or fixed binary time/hash entries;
   - round-trip tests;
   - no JSON decimal arrays for raw token vectors.

4. Change extraction:
   - Fast video must use one ffmpeg process per file, not one per frame.
   - Use ffmpeg select/scene/time-based extraction and preserve timestamps.
   - Audio should produce time-local anchors from fpcalc output. Use chunking/overlap or rolling token windows.
   - If a profile means full audio, pass `-length 0` explicitly; otherwise name it as a 120-second profile.
   - Store modality errors explicitly.

5. Matching:
   - For the current/query file, retrieve candidate files by anchor bucket lookup in SQLite.
   - Group hits by file_id.
   - Build offset histograms from candidate_t_ms - query_t_ms.
   - Fit candidate_t = scale * query_t + offset with a robust method.
   - Score by aligned anchors, modality agreement, aligned span, drift, duration plausibility, and second-best-offset margin.
   - Return Exact/Strong/Probable/Weak/Reject/Unknown with evidence including offset, drift, span, and modality counts.
   - Strong is the only tier eligible for autoplay, and only if user policy allows it.

6. Workflow:
   - Initial root scan should inventory files, not fingerprint everything.
   - On current player file, fingerprint that file fast, shortlist candidates, fingerprint missing top candidates, and return a decision quickly.
   - Background warmup should be cancelable, low-priority, checkpointed, and fast-profile only by default.
   - Full/hardening fingerprints should run only for likely/ambiguous matches or explicit user rebuild, not as a prerequisite.

7. Wire format:
   - Add `sorotte.mediaMatch.v2`.
   - Share compact anchor signatures only, under the existing payload cap.
   - Do not include raw local path, mtime, size, or filename in the mediaMatch signature.
   - Keep v1 parsing for compatibility but prefer v2.

8. Tests and acceptance:
   - Add deterministic synthetic media/fingerprint tests for offset, drift, trims, wrong episode with shared intro/outro, and transcodes.
   - Add DB-size and process-count tests or benchmarks.
   - Target fast profile <= 2 KB/file average excluding SQLite page overhead.
   - Target fast extraction <= 3 external processes per file.
   - No false Strong for wrong episode/shared intro/outro fixtures.
   - Offset estimate within ±1s for simple offset fixtures.
```

---

## Keep from the prototype

The branch already has several useful pieces:

* Tool discovery/import/install UI for `ffmpeg`, `ffprobe`, and `fpcalc`.
* Settings separation: diagnostic-only vs autoplay-eligible policy.
* Background worker/cancel/checkpoint flow.
* Peer wire-signature concept with a payload cap.
* Tiered decisions and evidence objects.
* Tests that already encode important cases like shared intro/outro, trimmed intro/credits, and drift.

The main change is the internal representation: turn the feature from **JSON record comparison** into **compact anchor retrieval plus timeline alignment**. That is the path that best matches your storage, speed, and offset-tolerance requirements.

[1]: https://github.com/ropbet-radbyt/sorotte/tree/codex/media-matching-plugin "GitHub - ropbet-radbyt/sorotte at codex/media-matching-plugin · GitHub"
[2]: https://github.com/acoustid/chromaprint "GitHub - acoustid/chromaprint: C library for generating audio fingerprints used by AcoustID · GitHub"
[3]: https://ffmpeg.org/ffmpeg-filters.html "FFmpeg Filters Documentation"
