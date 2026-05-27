use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

use super::*;
use crate::anchors::*;
use crate::audio_v3::*;
use crate::extraction::*;
use crate::identity::container_fingerprint_from_metadata;
use crate::matching::*;
use crate::tuning::*;
use crate::video_v3::*;

fn unique_test_root(label: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "sorotte-media-match-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    root
}

fn write_fake_tool(root: &Path, name: &str, stdout_line: Option<&str>) -> PathBuf {
    #[cfg(windows)]
    let path = root.join(format!("{name}.cmd"));
    #[cfg(not(windows))]
    let path = root.join(name);

    #[cfg(windows)]
    let script = match stdout_line {
        Some(line) => format!("@echo off\r\necho {line}\r\nexit /b 0\r\n"),
        None => "@echo off\r\nexit /b 0\r\n".to_owned(),
    };
    #[cfg(not(windows))]
    let script = match stdout_line {
        Some(line) => format!("#!/bin/sh\nprintf '%s\\n' '{line}'\n"),
        None => "#!/bin/sh\nexit 0\n".to_owned(),
    };
    std::fs::write(&path, script).expect("fake tool should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)
            .expect("fake tool metadata should load")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("fake tool should be executable");
    }
    path
}

fn record(
    path: &str,
    size: u64,
    duration: Option<f64>,
    video: Option<VideoFingerprint>,
) -> MediaFingerprintRecord {
    let normalized_path = normalize_media_path(path);
    MediaFingerprintRecord {
        identity: MediaFileIdentity {
            normalized_path: normalized_path.clone(),
            modified_unix_millis: 1000,
            size_bytes: size,
        },
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings: MediaExtractionSettings::combined_v3(),
        duration_seconds: duration,
        container_fingerprint: container_fingerprint_from_metadata(
            &normalized_path,
            1000,
            size,
            duration,
        ),
        video,
        audio_anchors: Vec::new(),
        video_anchors: Vec::new(),
        audio_error: None,
        video_error: None,
    }
}

fn record_with_extraction_settings(
    path: &str,
    size: u64,
    duration: Option<f64>,
    video: Option<VideoFingerprint>,
    extraction_settings: MediaExtractionSettings,
) -> MediaFingerprintRecord {
    let mut record = record(path, size, duration, video);
    record.extraction_settings = extraction_settings;
    record
}

fn record_from_anchor_profile(
    path: &str,
    size: u64,
    profile: MediaAnchorProfile,
) -> MediaFingerprintRecord {
    let mut record = record(
        path,
        size,
        profile.duration_ms.map(|duration| duration as f64 / 1000.0),
        None,
    );
    record.extraction_settings = MediaExtractionSettings::audio_constellation_v3();
    record.audio_anchors = profile.audio_anchors;
    record.video_anchors = profile.video_anchors;
    record
}

fn video_from_hashes(start_second: u64, step_seconds: u64, hashes: &[u64]) -> VideoFingerprint {
    VideoFingerprint {
        duration_seconds: Some(start_second as u32 + step_seconds as u32 * hashes.len() as u32),
        frames: hashes
            .iter()
            .enumerate()
            .map(|(index, hash)| {
                FrameFingerprint::new((start_second + step_seconds * index as u64) as f64, *hash)
            })
            .collect(),
        v3_landmarks: Vec::new(),
    }
}

fn shifted_video(offset_seconds: u64, hashes: &[u64]) -> VideoFingerprint {
    video_from_hashes(offset_seconds, 10, hashes)
}

fn synthetic_hash(value: u64) -> u64 {
    let mut x = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn synthetic_hashes(values: &[u64]) -> Vec<u64> {
    values.iter().copied().map(synthetic_hash).collect()
}

fn synthetic_luma_pattern(width: usize, height: usize) -> Vec<u8> {
    synthetic_luma_pattern_seed(width, height, 0)
}

fn synthetic_luma_pattern_seed(width: usize, height: usize, seed: usize) -> Vec<u8> {
    (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let base = ((x * (9 + seed % 5)
                    + y * (13 + seed % 7)
                    + ((x / 4 + y / 4 + seed) % 2) * 70
                    + seed * 17)
                    % 220) as u8;
                base.saturating_add(20)
            })
        })
        .collect()
}

fn brightness_shift_luma(luma: &[u8], delta: i16) -> Vec<u8> {
    luma.iter()
        .map(|value| (i16::from(*value) + delta).clamp(0, i16::from(u8::MAX)) as u8)
        .collect()
}

fn v3_landmark_hash_for_kind(landmarks: &[VideoLandmarkV3], kind: u8) -> u64 {
    landmarks
        .iter()
        .find(|landmark| landmark.kind == kind)
        .map(|landmark| landmark.hash64)
        .expect("landmark kind should exist")
}

fn anchor_profile(
    duration_ms: u32,
    audio: &[(u32, u32)],
    video: &[(u32, u32, u64)],
) -> MediaAnchorProfile {
    MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(duration_ms),
        audio_anchors: audio
            .iter()
            .map(|(bucket, t_ms)| AudioAnchor {
                bucket: *bucket,
                t_ms: *t_ms,
                weight: 1,
            })
            .collect(),
        video_anchors: video
            .iter()
            .map(|(bucket, t_ms, hash64)| VideoAnchor {
                bucket: *bucket,
                t_ms: *t_ms,
                hash64: *hash64,
                kind: V3_VIDEO_KIND_LUMA_FRAME,
                weight: 1,
            })
            .collect(),
    }
}

fn regular_anchor_profile(duration_ms: u32, offset_ms: i32, drift_ppm: i32) -> MediaAnchorProfile {
    let query_times = (0..12).map(|index| 60_000 + index * 60_000);
    let audio = query_times
        .clone()
        .map(|t_ms| {
            let candidate_t = shifted_anchor_time(t_ms, offset_ms, drift_ppm);
            (t_ms / 60_000 + 1, candidate_t)
        })
        .collect::<Vec<_>>();
    let video = query_times
        .map(|t_ms| {
            let candidate_t = shifted_anchor_time(t_ms, offset_ms, drift_ppm);
            let hash = synthetic_hash(u64::from(t_ms));
            (t_ms / 60_000 + 100, candidate_t, hash)
        })
        .collect::<Vec<_>>();
    anchor_profile(duration_ms, &audio, &video)
}

fn audio_only_v3_anchor_profile(
    duration_ms: u32,
    offset_ms: i32,
    drift_ppm: i32,
) -> MediaAnchorProfile {
    let query_times = (0..24).map(|index| 60_000 + index * 45_000);
    let audio = query_times
        .map(|t_ms| AudioAnchor {
            bucket: t_ms / 45_000 + 1,
            t_ms: shifted_anchor_time(t_ms, offset_ms, drift_ppm),
            weight: 4,
        })
        .collect::<Vec<_>>();
    MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "audio-constellation-v3".to_owned(),
        duration_ms: Some(duration_ms),
        audio_anchors: audio,
        video_anchors: Vec::new(),
    }
}

fn v3_profile_from_times(
    duration_ms: u32,
    audio_times: &[(u32, u32)],
    video_times: &[(u32, u32, u64)],
) -> MediaAnchorProfile {
    MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(duration_ms),
        audio_anchors: audio_times
            .iter()
            .map(|(bucket, t_ms)| AudioAnchor {
                bucket: *bucket,
                t_ms: *t_ms,
                weight: 4,
            })
            .collect(),
        video_anchors: video_times
            .iter()
            .map(|(bucket, t_ms, hash64)| VideoAnchor {
                bucket: *bucket,
                t_ms: *t_ms,
                hash64: *hash64,
                kind: V3_VIDEO_KIND_LUMA_FRAME,
                weight: 4,
            })
            .collect(),
    }
}

fn v3_audio_times(start_ms: u32, count: u32, step_ms: u32) -> Vec<(u32, u32)> {
    (0..count)
        .map(|index| (1_000 + index, start_ms + (index * step_ms)))
        .collect()
}

fn v3_shift_audio_times(times: &[(u32, u32)], offset_ms: i32, drift_ppm: i32) -> Vec<(u32, u32)> {
    times
        .iter()
        .map(|(bucket, t_ms)| (*bucket, shifted_anchor_time(*t_ms, offset_ms, drift_ppm)))
        .collect()
}

fn v3_video_times_from_audio(times: &[(u32, u32)]) -> Vec<(u32, u32, u64)> {
    times
        .iter()
        .map(|(bucket, t_ms)| (*bucket + 10_000, *t_ms, synthetic_hash(u64::from(*bucket))))
        .collect()
}

fn v3_shift_video_times(
    times: &[(u32, u32, u64)],
    offset_ms: i32,
    drift_ppm: i32,
) -> Vec<(u32, u32, u64)> {
    times
        .iter()
        .map(|(bucket, t_ms, hash)| {
            (
                *bucket,
                shifted_anchor_time(*t_ms, offset_ms, drift_ppm),
                *hash,
            )
        })
        .collect()
}

fn shifted_anchor_time(t_ms: u32, offset_ms: i32, drift_ppm: i32) -> u32 {
    let scaled = i64::from(t_ms) + ((i64::from(t_ms) * i64::from(drift_ppm)) / 1_000_000);
    (scaled + i64::from(offset_ms))
        .max(0)
        .min(i64::from(u32::MAX)) as u32
}

fn enabled_settings() -> MediaMatchSettings {
    MediaMatchSettings {
        fingerprinting_enabled: true,
        ..MediaMatchSettings::default()
    }
}

#[test]
fn compact_audio_wire_anchor_block_round_trips_with_delta_times() {
    let anchors = vec![
        AudioAnchor {
            bucket: 7,
            t_ms: 2_000,
            weight: 2,
        },
        AudioAnchor {
            bucket: 5,
            t_ms: 500,
            weight: 1,
        },
    ];

    let encoded = encode_wire_audio_anchor_summary(&anchors);
    let decoded = decode_wire_audio_anchor_summary(&encoded).expect("audio summary should decode");

    assert_eq!(
        decoded,
        vec![
            AudioAnchor {
                bucket: 5,
                t_ms: 500,
                weight: 1,
            },
            AudioAnchor {
                bucket: 7,
                t_ms: 2_000,
                weight: 2,
            },
        ]
    );
    assert!(encoded.len() < 64);
}

#[test]
fn compact_video_wire_anchor_block_round_trips_hashes() {
    let anchors = vec![
        VideoAnchor {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 9),
            t_ms: 1_000,
            hash64: 0x0123_4567_89ab_cdef,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 1,
        },
        VideoAnchor {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_EDGE, 10),
            t_ms: 3_000,
            hash64: 0xfedc_ba98_7654_3210,
            kind: V3_VIDEO_KIND_EDGE,
            weight: 3,
        },
    ];

    let encoded = encode_wire_video_anchor_summary(&anchors);
    let decoded = decode_wire_video_anchor_summary(&encoded).expect("video summary should decode");

    assert_eq!(decoded, anchors);
    assert!(encoded.len() < 84);
}

#[test]
fn v3_blob_round_trips_delta_encoded_landmarks() {
    let blob = MediaFingerprintBlobV3 {
        duration_ms: Some(1_413_000),
        audio_landmarks: vec![
            AudioLandmarkV3 {
                hash: 0x1234_5678,
                t_ms: 10_000,
                weight: 9,
            },
            AudioLandmarkV3 {
                hash: 0x90ab_cdef,
                t_ms: 42_000,
                weight: 3,
            },
        ],
        video_landmarks: vec![VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_CENTER_DCT, 7),
            hash64: 0x0123_4567_89ab_cdef,
            t_ms: 48_000,
            kind: V3_VIDEO_KIND_CENTER_DCT,
            weight: 5,
        }],
    };

    let encoded = encode_media_fingerprint_blob_v3(&blob);
    let decoded = decode_media_fingerprint_blob_v3(&encoded).expect("v3 blob should decode");

    assert_eq!(decoded, blob);
    assert!(encoded.len() < 80);
}

#[test]
fn v3_blob_rejects_corrupted_input() {
    assert!(matches!(
        decode_media_fingerprint_blob_v3(b"not-smm3"),
        Err(MediaFingerprintBlobV3DecodeError::InvalidMagic)
    ));

    let blob = MediaFingerprintBlobV3 {
        duration_ms: Some(1),
        audio_landmarks: vec![AudioLandmarkV3 {
            hash: 1,
            t_ms: 1,
            weight: 1,
        }],
        video_landmarks: Vec::new(),
    };
    let mut encoded = encode_media_fingerprint_blob_v3(&blob);
    encoded.truncate(encoded.len() - 1);

    assert!(matches!(
        decode_media_fingerprint_blob_v3(&encoded),
        Err(MediaFingerprintBlobV3DecodeError::InvalidLength)
    ));
}

#[test]
fn v3_blob_rejects_unknown_video_kind() {
    let blob = MediaFingerprintBlobV3 {
        duration_ms: Some(1),
        audio_landmarks: Vec::new(),
        video_landmarks: vec![VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(9, 1),
            hash64: 1,
            t_ms: 1,
            kind: 9,
            weight: 1,
        }],
    };

    let encoded = encode_media_fingerprint_blob_v3(&blob);

    assert!(matches!(
        decode_media_fingerprint_blob_v3(&encoded),
        Err(MediaFingerprintBlobV3DecodeError::UnsupportedVideoKind(9))
    ));
}

#[test]
fn v3_blob_rejects_mismatched_video_bucket_kind() {
    let blob = MediaFingerprintBlobV3 {
        duration_ms: Some(1),
        audio_landmarks: Vec::new(),
        video_landmarks: vec![VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_EDGE, 1),
            hash64: 1,
            t_ms: 1,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 1,
        }],
    };

    let encoded = encode_media_fingerprint_blob_v3(&blob);

    assert!(matches!(
        decode_media_fingerprint_blob_v3(&encoded),
        Err(
            MediaFingerprintBlobV3DecodeError::MismatchedVideoBucketKind {
                kind: V3_VIDEO_KIND_GLOBAL_DCT,
                bucket_kind: V3_VIDEO_KIND_EDGE
            }
        )
    ));
}

#[test]
fn v3_blob_rejects_invalid_temporal_video_bucket() {
    let hash64 = 0x1234_5678_9abc_def0;
    let blob = MediaFingerprintBlobV3 {
        duration_ms: Some(1),
        audio_landmarks: Vec::new(),
        video_landmarks: vec![VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_TEMPORAL_SHINGLE, 0xfeed_beef),
            hash64,
            t_ms: 1,
            kind: V3_VIDEO_KIND_TEMPORAL_SHINGLE,
            weight: 1,
        }],
    };

    let encoded = encode_media_fingerprint_blob_v3(&blob);

    assert!(matches!(
        decode_media_fingerprint_blob_v3(&encoded),
        Err(MediaFingerprintBlobV3DecodeError::InvalidTemporalVideoBucket { .. })
    ));
}

#[test]
fn v3_wire_profile_rejects_unknown_video_kind() {
    let summary = encode_wire_video_anchor_summary(&[VideoAnchor {
        bucket: v3_video_bucket_for_kind(9, 1),
        t_ms: 1,
        hash64: 1,
        kind: 9,
        weight: 1,
    }]);
    let profile = MediaMatchWireProfile {
        profile: "combined-v3".to_owned(),
        algorithm_version: MEDIA_MATCH_ANCHOR_VERSION,
        duration_ms: Some(1),
        audio: None,
        video: Some(MediaMatchWireAnchorBlock {
            algorithm: MediaExtractionSettings::combined_v3().video_algorithm,
            time_base_ms: 1,
            anchors: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, summary),
        }),
    };

    let error = media_anchor_profile_from_wire_profile(&profile).expect_err("invalid kind");

    assert!(error.contains("unsupported media v3 video landmark kind 9"));
}

#[test]
fn video_landmark_with_bucket_kind_mismatch_is_not_matched() {
    let hash = 0x0123_4567_89ab_cdef;
    let query = MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(60_000),
        audio_anchors: Vec::new(),
        video_anchors: vec![VideoAnchor {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_EDGE, 3),
            t_ms: 1_000,
            hash64: hash,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        }],
    };
    let candidate = MediaAnchorProfile {
        video_anchors: vec![VideoAnchor {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 3),
            t_ms: 1_000,
            hash64: hash,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        }],
        ..query.clone()
    };

    assert!(collect_anchor_match_pairs(&query, &candidate).is_empty());
}

#[test]
fn black_bar_detection_letterbox() {
    let width = 32;
    let height = 32;
    let mut luma = synthetic_luma_pattern(width, height);
    for y in 0..6 {
        for x in 0..width {
            luma[y * width + x] = 0;
            luma[(height - 1 - y) * width + x] = 0;
        }
    }

    let rect = detect_content_window_luma(width, height, &luma).expect("content rect");

    assert_eq!(rect.y, 6);
    assert_eq!(rect.height, 20);
}

#[test]
fn black_bar_detection_pillarbox() {
    let width = 32;
    let height = 32;
    let mut luma = synthetic_luma_pattern(width, height);
    for y in 0..height {
        for x in 0..5 {
            luma[y * width + x] = 0;
            luma[y * width + (width - 1 - x)] = 0;
        }
    }

    let rect = detect_content_window_luma(width, height, &luma).expect("content rect");

    assert_eq!(rect.x, 5);
    assert_eq!(rect.width, 22);
}

#[test]
fn all_black_frame_is_ignored_for_v3_video() {
    let luma = vec![0; VIDEO_FRAME_BYTES];
    let rect = detect_content_window_luma(VIDEO_FRAME_WIDTH, VIDEO_FRAME_HEIGHT, &luma).unwrap();

    assert_eq!(
        rect,
        LumaRect {
            x: 0,
            y: 0,
            width: VIDEO_FRAME_WIDTH,
            height: VIDEO_FRAME_HEIGHT
        }
    );
    assert!(video_landmarks_v3_from_luma_frame(32, 32, &luma, 0).is_empty());
}

#[test]
fn global_dct_hash_stable_under_brightness_shift() {
    let luma = synthetic_luma_pattern(32, 32);
    let brighter = brightness_shift_luma(&luma, 22);
    let left = video_landmarks_v3_from_luma_frame(32, 32, &luma, 1_000);
    let right = video_landmarks_v3_from_luma_frame(32, 32, &brighter, 1_000);

    let distance = frame_hash_distance(
        v3_landmark_hash_for_kind(&left, V3_VIDEO_KIND_GLOBAL_DCT),
        v3_landmark_hash_for_kind(&right, V3_VIDEO_KIND_GLOBAL_DCT),
    );

    assert!(distance <= v3_video_hamming_threshold(V3_VIDEO_KIND_GLOBAL_DCT));
}

#[test]
fn center_crop_resists_hard_subtitle_band() {
    let luma = synthetic_luma_pattern(32, 32);
    let mut subtitled = luma.clone();
    for y in 25..30 {
        for x in 6..26 {
            subtitled[y * 32 + x] = 255;
        }
    }
    let left = video_landmarks_v3_from_luma_frame(32, 32, &luma, 1_000);
    let right = video_landmarks_v3_from_luma_frame(32, 32, &subtitled, 1_000);

    let distance = frame_hash_distance(
        v3_landmark_hash_for_kind(&left, V3_VIDEO_KIND_CENTER_DCT),
        v3_landmark_hash_for_kind(&right, V3_VIDEO_KIND_CENTER_DCT),
    );

    assert!(distance <= v3_video_hamming_threshold(V3_VIDEO_KIND_CENTER_DCT));
}

#[test]
fn edge_hash_resists_brightness_shift() {
    let luma = synthetic_luma_pattern(32, 32);
    let brighter = brightness_shift_luma(&luma, 30);
    let left = video_landmarks_v3_from_luma_frame(32, 32, &luma, 1_000);
    let right = video_landmarks_v3_from_luma_frame(32, 32, &brighter, 1_000);

    let distance = frame_hash_distance(
        v3_landmark_hash_for_kind(&left, V3_VIDEO_KIND_EDGE),
        v3_landmark_hash_for_kind(&right, V3_VIDEO_KIND_EDGE),
    );

    assert!(distance <= v3_video_hamming_threshold(V3_VIDEO_KIND_EDGE));
}

#[test]
fn temporal_shingle_requires_order() {
    let mut forward = vec![
        VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 1),
            hash64: 0x1000,
            t_ms: 0,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        },
        VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 2),
            hash64: 0x2000,
            t_ms: 10_000,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        },
        VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 3),
            hash64: 0x3000,
            t_ms: 20_000,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        },
    ];
    let mut backward = vec![
        VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 3),
            hash64: 0x3000,
            t_ms: 0,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        },
        VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 2),
            hash64: 0x2000,
            t_ms: 10_000,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        },
        VideoLandmarkV3 {
            bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 1),
            hash64: 0x1000,
            t_ms: 20_000,
            kind: V3_VIDEO_KIND_GLOBAL_DCT,
            weight: 2,
        },
    ];
    add_v3_temporal_video_shingles(&mut forward);
    add_v3_temporal_video_shingles(&mut backward);
    let forward_shingles = forward
        .iter()
        .filter(|landmark| landmark.kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE)
        .map(|landmark| landmark.hash64)
        .collect::<HashSet<_>>();
    let backward_shingles = backward
        .iter()
        .filter(|landmark| landmark.kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE)
        .map(|landmark| landmark.hash64)
        .collect::<HashSet<_>>();

    assert!(!forward_shingles.is_empty());
    assert!(forward_shingles.is_disjoint(&backward_shingles));
}

#[test]
fn temporal_shingles_match_exactly() {
    let hash = 0x0123_4567_89ab_cdef;
    assert!(v3_video_anchor_hashes_match(
        V3_VIDEO_KIND_TEMPORAL_SHINGLE,
        hash,
        hash
    ));
    assert!(!v3_video_anchor_hashes_match(
        V3_VIDEO_KIND_TEMPORAL_SHINGLE,
        hash,
        hash ^ 1
    ));
}

#[test]
fn video_descriptor_kinds_do_not_cross_match() {
    let hash = 0x0123_4567_89ab_cdef;
    let query = VideoAnchor {
        bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_GLOBAL_DCT, 7),
        t_ms: 1_000,
        hash64: hash,
        kind: V3_VIDEO_KIND_GLOBAL_DCT,
        weight: 2,
    };
    let candidate = VideoAnchor {
        bucket: v3_video_bucket_for_kind(V3_VIDEO_KIND_EDGE, 7),
        t_ms: 1_000,
        hash64: hash,
        kind: V3_VIDEO_KIND_EDGE,
        weight: 2,
    };
    let query = MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(120_000),
        audio_anchors: Vec::new(),
        video_anchors: vec![query],
    };
    let candidate = MediaAnchorProfile {
        video_anchors: vec![candidate],
        ..query.clone()
    };

    assert!(collect_anchor_match_pairs(&query, &candidate).is_empty());
}

#[test]
fn combined_v3_video_landmarks_include_multiple_kinds() {
    let frames = vec![
        (0, synthetic_luma_pattern_seed(32, 32, 1)),
        (10_000, synthetic_luma_pattern_seed(32, 32, 2)),
        (20_000, synthetic_luma_pattern_seed(32, 32, 3)),
    ];
    let landmarks = video_landmarks_v3_from_luma_frames(32, 32, &frames);
    let video = VideoFingerprint {
        duration_seconds: Some(30),
        frames: Vec::new(),
        v3_landmarks: landmarks,
    };
    let record = record_with_extraction_settings(
        "video.mkv",
        100,
        Some(30.0),
        Some(video),
        MediaExtractionSettings::combined_v3(),
    );
    let kinds = video_landmarks_v3_from_record(&record)
        .into_iter()
        .map(|landmark| landmark.kind)
        .collect::<HashSet<_>>();

    assert!(kinds.contains(&V3_VIDEO_KIND_GLOBAL_DCT));
    assert!(kinds.contains(&V3_VIDEO_KIND_CENTER_DCT));
    assert!(kinds.contains(&V3_VIDEO_KIND_EDGE));
    assert!(kinds.contains(&V3_VIDEO_KIND_TEMPORAL_SHINGLE));
}

#[test]
fn combined_v3_video_bounding_preserves_descriptor_kinds() {
    let frames = (0..80)
        .map(|index| {
            (
                index * 10_000,
                synthetic_luma_pattern_seed(32, 32, index as usize + 100),
            )
        })
        .collect::<Vec<_>>();
    let landmarks = video_landmarks_v3_from_luma_frames(32, 32, &frames);
    let kinds = landmarks
        .iter()
        .map(|landmark| landmark.kind)
        .collect::<HashSet<_>>();

    assert!(landmarks.len() <= V3_VIDEO_VERIFY_LANDMARK_LIMIT);
    assert!(kinds.contains(&V3_VIDEO_KIND_GLOBAL_DCT));
    assert!(kinds.contains(&V3_VIDEO_KIND_CENTER_DCT));
    assert!(kinds.contains(&V3_VIDEO_KIND_EDGE));
    assert!(kinds.contains(&V3_VIDEO_KIND_TEMPORAL_SHINGLE));
}

#[test]
fn combined_v3_video_index_bounding_prefers_temporal_shingles() {
    let frames = (0..80)
        .map(|index| {
            (
                index * 10_000,
                synthetic_luma_pattern_seed(32, 32, index as usize + 200),
            )
        })
        .collect::<Vec<_>>();
    let landmarks = video_landmarks_v3_from_luma_frames(32, 32, &frames);
    let video = VideoFingerprint {
        duration_seconds: Some(800),
        frames: Vec::new(),
        v3_landmarks: landmarks,
    };
    let record = record_with_extraction_settings(
        "index-bounds.mkv",
        100,
        Some(800.0),
        Some(video),
        MediaExtractionSettings::combined_v3(),
    );
    let index = video_index_landmarks_v3_from_record(&record);

    assert!(index.len() <= V3_VIDEO_INDEX_LANDMARK_LIMIT);
    assert!(
        index
            .iter()
            .any(|landmark| landmark.kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE)
    );
}

#[test]
fn cropped_or_letterboxed_same_video_still_matches() {
    let content = synthetic_luma_pattern_seed(32, 20, 7);
    let mut letterboxed = vec![0u8; 32 * 32];
    for y in 0..20 {
        for x in 0..32 {
            letterboxed[(y + 6) * 32 + x] = content[y * 32 + x];
        }
    }
    let plain = video_landmarks_v3_from_luma_frame(32, 20, &content, 10_000);
    let boxed = video_landmarks_v3_from_luma_frame(32, 32, &letterboxed, 10_000);

    for kind in [
        V3_VIDEO_KIND_GLOBAL_DCT,
        V3_VIDEO_KIND_CENTER_DCT,
        V3_VIDEO_KIND_EDGE,
    ] {
        let distance = frame_hash_distance(
            v3_landmark_hash_for_kind(&plain, kind),
            v3_landmark_hash_for_kind(&boxed, kind),
        );
        assert!(
            distance <= v3_video_hamming_threshold(kind),
            "kind {kind} distance {distance} should stay matchable"
        );
    }
}

#[test]
fn combined_v3_video_storage_limits_are_bounded() {
    let frames = (0..80)
        .map(|index| {
            (
                index * 10_000,
                synthetic_luma_pattern_seed(32, 32, index as usize),
            )
        })
        .collect::<Vec<_>>();
    let landmarks = video_landmarks_v3_from_luma_frames(32, 32, &frames);
    let video = VideoFingerprint {
        duration_seconds: Some(800),
        frames: Vec::new(),
        v3_landmarks: landmarks,
    };
    let record = record_with_extraction_settings(
        "storage.mkv",
        100,
        Some(800.0),
        Some(video),
        MediaExtractionSettings::combined_v3(),
    );

    assert!(video_landmarks_v3_from_record(&record).len() <= V3_VIDEO_VERIFY_LANDMARK_LIMIT);
    assert!(video_index_landmarks_v3_from_record(&record).len() <= V3_VIDEO_INDEX_LANDMARK_LIMIT);
}

#[test]
fn v3_record_diagnostics_report_blob_and_index_counts() {
    let audio = v3_audio_times(120_000, 12, 45_000)
        .into_iter()
        .map(|(bucket, t_ms)| AudioAnchor {
            bucket,
            t_ms,
            weight: 2,
        })
        .collect::<Vec<_>>();
    let frames = (0..12)
        .map(|index| {
            (
                index * 10_000,
                synthetic_luma_pattern_seed(32, 32, index as usize + 300),
            )
        })
        .collect::<Vec<_>>();
    let video = VideoFingerprint {
        duration_seconds: Some(120),
        frames: Vec::new(),
        v3_landmarks: video_landmarks_v3_from_luma_frames(32, 32, &frames),
    };
    let mut record = record_with_extraction_settings(
        "diagnostics.mkv",
        100,
        Some(120.0),
        Some(video),
        MediaExtractionSettings::combined_v3(),
    );
    record.audio_anchors = audio;

    let summary = summarize_record_v3_diagnostics(&record);

    assert_eq!(summary.profile, "combined-v3");
    assert!(summary.audio_verify_count > 0);
    assert!(summary.video_verify_count > 0);
    assert!(summary.audio_index_count > 0);
    assert!(summary.video_index_count > 0);
    assert!(summary.audio_blob_bytes > 0);
    assert!(summary.video_blob_bytes > 0);
}

#[test]
fn v3_diagnostics_serializes_stable_stream_metric_names() {
    let record = record_with_extraction_settings(
        "stream-diagnostics.mkv",
        100,
        Some(120.0),
        None,
        MediaExtractionSettings::audio_constellation_v3(),
    );
    let fingerprint = InstrumentedMediaFingerprint {
        record,
        report: MediaFingerprintExtractionReport {
            audio_stream: MediaAudioStreamMetrics {
                streamed_bytes: 10_000,
                streamed_samples: 5_000,
                peak_frames: 12,
                raw_landmarks_emitted: 360,
                raw_landmarks_before_bounding: 300,
                final_landmarks: 96,
                max_buffer_samples: V3_AUDIO_WINDOW_SAMPLES + V3_AUDIO_HOP_SAMPLES,
                max_raw_landmarks_seen: 1_100,
                max_raw_landmarks_after_compaction: 512,
                raw_landmark_compactions: 2,
                analyzer_millis: 31,
                peak_selection_millis: 9,
                pairing_millis: 11,
                compaction_millis: 7,
                reservoir_millis: 5,
                final_selection_millis: 3,
                ffmpeg_process_wall_millis: 43,
                pcm_decode_drain_millis: 41,
                ffmpeg_decode_stream_millis: 41,
                sampled_audio_seconds_decoded: 0,
                sampled_audio_windows_decoded: 0,
                full_audio_seconds_decoded: 120,
            },
            ..MediaFingerprintExtractionReport::default()
        },
    };

    let value = serde_json::to_value(summarize_instrumented_record_v3_diagnostics(&fingerprint))
        .expect("diagnostics should serialize");

    assert_eq!(value["streamedBytes"], 10_000);
    assert_eq!(value["streamedSamples"], 5_000);
    assert_eq!(value["peakFrames"], 12);
    assert_eq!(value["rawLandmarksEmitted"], 360);
    assert_eq!(value["rawLandmarksBeforeBounding"], 300);
    assert_eq!(value["rawLandmarksKeptBeforeFinal"], 300);
    assert_eq!(value["finalLandmarks"], 96);
    assert_eq!(
        value["maxBufferSamples"],
        V3_AUDIO_WINDOW_SAMPLES + V3_AUDIO_HOP_SAMPLES
    );
    assert_eq!(value["maxRawLandmarksSeen"], 1_100);
    assert_eq!(value["maxRawLandmarksAfterCompaction"], 512);
    assert_eq!(value["rawLandmarkCompactions"], 2);
    assert_eq!(value["ffmpegProcessWallMillis"], 43);
    assert_eq!(value["pcmDecodeDrainMillis"], 41);
    assert_eq!(value["analyzerMillis"], 31);
    assert_eq!(value["peakSelectionMillis"], 9);
    assert_eq!(value["pairingMillis"], 11);
    assert_eq!(value["compactionMillis"], 7);
    assert_eq!(value["reservoirMillis"], 5);
    assert_eq!(value["finalSelectionMillis"], 3);
    assert_eq!(value["sampledAudioSecondsDecoded"], 0);
    assert_eq!(value["sampledAudioWindowsDecoded"], 0);
    assert_eq!(value["fullAudioSecondsDecoded"], 120);
    assert!(
        (value["effectiveDecodedSecondsPerSecond"].as_f64().unwrap() - (120.0 / 0.043)).abs()
            < 0.001
    );
    assert_eq!(value["indexQuality"], "full-verify");
}

#[test]
fn v3_audio_constellation_generates_sparse_landmarks_from_pcm() {
    let sample_rate = V3_AUDIO_SAMPLE_RATE;
    let seconds = 8;
    let samples = (0..sample_rate as usize * seconds)
        .map(|index| {
            let t = index as f32 / sample_rate as f32;
            let frequency = 440.0 + ((t / 2.0).floor() * 110.0);
            (frequency.mul_add(std::f32::consts::TAU * t, 0.0).sin() * f32::from(i16::MAX) * 0.5)
                .round() as i16
        })
        .collect::<Vec<_>>();

    let landmarks =
        audio_constellation_landmarks_v3_from_pcm(&samples, sample_rate, Some(seconds as f64));

    assert!(!landmarks.is_empty());
    assert!(landmarks.len() <= V3_AUDIO_VERIFY_LANDMARK_LIMIT);
    assert!(landmarks.iter().all(|landmark| landmark.weight > 0));
}

#[test]
fn v3_audio_streaming_builder_is_chunk_boundary_stable() {
    let sample_rate = V3_AUDIO_SAMPLE_RATE;
    let seconds = 5;
    let samples = synthetic_audio_samples_v3(sample_rate, seconds);
    let full =
        audio_constellation_landmarks_v3_from_pcm(&samples, sample_rate, Some(seconds as f64));
    let uneven_chunks = samples.chunks(777).collect::<Vec<_>>();
    let tiny_chunks = samples.chunks(113).collect::<Vec<_>>();

    let (streamed, metrics) = audio_constellation_landmarks_v3_from_pcm_chunks(
        &uneven_chunks,
        sample_rate,
        Some(seconds as f64),
    );
    let (streamed_tiny, tiny_metrics) = audio_constellation_landmarks_v3_from_pcm_chunks(
        &tiny_chunks,
        sample_rate,
        Some(seconds as f64),
    );

    assert!(!streamed.is_empty());
    assert_eq!(streamed, streamed_tiny);
    assert!(audio_streaming_reference_overlap(&full, &streamed) >= 0.90);
    assert_eq!(metrics.streamed_samples, samples.len());
    assert_eq!(tiny_metrics.streamed_samples, samples.len());
}

#[test]
fn v3_audio_streaming_rejects_odd_trailing_pcm_byte() {
    let error = audio_constellation_stream_rejects_odd_trailing_byte_for_test(&[1])
        .expect_err("odd trailing byte must fail");

    assert!(matches!(
        error,
        MediaFingerprintError::InvalidToolOutput { tool: "ffmpeg", .. }
    ));
}

#[test]
fn v3_audio_streaming_decode_handles_split_pcm_samples() {
    let samples = synthetic_audio_samples_v3(V3_AUDIO_SAMPLE_RATE, 6);
    let bytes = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect::<Vec<_>>();
    let (landmarks, metrics) =
        audio_constellation_streaming_decode_pcm_bytes_for_test(&bytes).expect("decode");
    let split_metrics =
        audio_constellation_streaming_decode_split_bytes_for_test(&bytes).expect("split");

    assert!(!landmarks.is_empty());
    assert_eq!(metrics.streamed_bytes, bytes.len());
    assert_eq!(split_metrics.streamed_bytes, bytes.len());
    assert_eq!(split_metrics.streamed_samples, samples.len());
}

#[test]
fn v3_audio_streaming_builder_keeps_rolling_buffer_bounded() {
    let sample_rate = V3_AUDIO_SAMPLE_RATE;
    let seconds = 45;
    let samples = synthetic_audio_samples_v3(sample_rate, seconds);

    let (_landmarks, metrics) =
        audio_constellation_landmarks_v3_from_pcm_streaming(&samples, sample_rate, Some(45.0));

    assert!(metrics.final_landmarks <= V3_AUDIO_VERIFY_LANDMARK_LIMIT);
    assert!(
        metrics.max_buffer_samples <= V3_AUDIO_WINDOW_SAMPLES,
        "{metrics:?}"
    );
    assert!(
        metrics.max_raw_landmarks_after_compaction <= V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT,
        "{metrics:?}"
    );
    assert!(
        metrics.max_raw_landmarks_seen <= V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT,
        "{metrics:?}"
    );
    assert!(
        metrics.raw_landmarks_emitted > metrics.max_raw_landmarks_seen,
        "reservoir should emit more raw landmarks than it retains: {metrics:?}"
    );
    assert!(
        metrics.max_raw_landmarks_after_compaction <= V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT,
        "{metrics:?}"
    );
    assert_eq!(
        metrics.raw_landmark_compactions, 0,
        "online region reservoirs should avoid repeated sort/truncate compactions: {metrics:?}"
    );
    assert!(
        metrics.reservoir_millis > 0,
        "long synthetic audio should exercise online reservoir insertion: {metrics:?}"
    );
    assert!(metrics.streamed_samples > V3_AUDIO_WINDOW_SAMPLES * 100);
}

#[test]
fn streaming_stdout_callback_error_aborts_promptly() {
    let (executable, args) = streaming_stdout_error_test_command();
    let started_at = Instant::now();
    let result = run_tool_streaming_stdout(
        "test-tool",
        &executable,
        args,
        None,
        Duration::from_secs(20),
        |_chunk| {
            Err(MediaFingerprintError::InvalidToolOutput {
                tool: "test-tool",
                reason: "intentional callback failure".to_owned(),
            })
        },
    );

    assert!(matches!(
        result,
        Err(MediaFingerprintError::InvalidToolOutput {
            tool: "test-tool",
            ..
        })
    ));
    assert!(
        started_at.elapsed() < Duration::from_secs(5),
        "callback failure should not wait for timeout"
    );
}

#[cfg(windows)]
fn streaming_stdout_error_test_command() -> (PathBuf, Vec<OsString>) {
    (
        PathBuf::from("powershell"),
        vec![
            "-NoProfile".into(),
            "-Command".into(),
            "Write-Output chunk; Start-Sleep -Seconds 30".into(),
        ],
    )
}

#[cfg(not(windows))]
fn streaming_stdout_error_test_command() -> (PathBuf, Vec<OsString>) {
    (
        PathBuf::from("sh"),
        vec!["-c".into(), "printf chunk; exec sleep 30".into()],
    )
}

fn synthetic_audio_samples_v3(sample_rate: u32, seconds: usize) -> Vec<i16> {
    (0..sample_rate as usize * seconds)
        .map(|index| {
            let t = index as f32 / sample_rate as f32;
            let frequency = 330.0 + ((t / 1.5).floor() * 77.0);
            ((frequency * std::f32::consts::TAU * t).sin() * f32::from(i16::MAX) * 0.45).round()
                as i16
        })
        .collect()
}

#[test]
fn same_cut_single_segment_maps_position() {
    let map = test_timeline_map_v3(
        MatchClassV3::SameCutStrong,
        vec![test_segment_v3(10_000, 110_000, 12_000, 112_000, 1_000_000)],
    );

    let mapped = map_query_position_to_candidate_ms(&map, 60_000)
        .expect("position should map inside segment");

    assert_eq!(mapped.mapped_ms, 62_000);
    assert_eq!(mapped.segment_index, 0);
    assert_eq!(mapped.class_at_position, MatchClassV3::SameCutStrong);
    assert_eq!(mapped.local_offset_ms, 2_000);
    assert!(timeline_map_contains_query_position(&map, 60_000));
}

#[test]
fn inserted_logo_two_segments_maps_each_segment() {
    let map = test_timeline_map_v3(
        MatchClassV3::SameMediaDifferentCut,
        vec![
            test_segment_v3(0, 120_000, 0, 120_000, 1_000_000),
            test_segment_v3(120_000, 240_000, 180_000, 300_000, 1_000_000),
        ],
    );

    let first = map_query_position_to_candidate_ms(&map, 90_000).expect("first segment");
    let second = map_query_position_to_candidate_ms(&map, 180_000).expect("second segment");

    assert_eq!(first.mapped_ms, 90_000);
    assert_eq!(first.segment_index, 0);
    assert_eq!(second.mapped_ms, 240_000);
    assert_eq!(second.segment_index, 1);
    assert_eq!(
        second.class_at_position,
        MatchClassV3::SameMediaDifferentCut
    );
}

#[test]
fn position_in_edit_gap_returns_none() {
    let map = test_timeline_map_v3(
        MatchClassV3::SameMediaDifferentCut,
        vec![
            test_segment_v3(0, 90_000, 0, 90_000, 1_000_000),
            test_segment_v3(150_000, 240_000, 180_000, 270_000, 1_000_000),
        ],
    );

    assert!(map_query_position_to_candidate_ms(&map, 120_000).is_none());
    assert!(!timeline_map_contains_query_position(&map, 120_000));
}

#[test]
fn reverse_mapping_round_trips_inside_segment() {
    let map = test_timeline_map_v3(
        MatchClassV3::SameCutStrong,
        vec![test_segment_v3(10_000, 110_000, 20_000, 121_000, 1_010_000)],
    );

    let forward = map_query_position_to_candidate_ms(&map, 60_000).expect("forward map");
    let reverse = map_candidate_position_to_query_ms(&map, forward.mapped_ms).expect("reverse map");

    assert!((i64::from(reverse.mapped_ms) - 60_000).abs() <= 1);
    assert_eq!(reverse.segment_index, 0);
    assert!((reverse.local_offset_ms - (i64::from(forward.mapped_ms) - 60_000)).abs() <= 1);
}

#[test]
fn same_media_different_cut_maps_but_not_autoplay() {
    let map = test_timeline_map_v3(
        MatchClassV3::SameMediaDifferentCut,
        vec![test_segment_v3(0, 180_000, 30_000, 210_000, 1_000_000)],
    );
    let mapped =
        map_query_position_to_candidate_ms(&map, 90_000).expect("different cut maps locally");
    let decision = MediaMatchDecision {
        tier: MediaMatchTier::Strong,
        evidence: MediaMatchEvidence {
            v3_class: Some(MatchClassV3::SameMediaDifferentCut),
            timeline_map_v3: Some(map),
            ..MediaMatchEvidence::default()
        },
        explanation: "different cut".to_owned(),
    };
    let settings = MediaMatchSettings {
        autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        ..MediaMatchSettings::default()
    };

    assert_eq!(mapped.mapped_ms, 120_000);
    assert_eq!(
        mapped.class_at_position,
        MatchClassV3::SameMediaDifferentCut
    );
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn shared_intro_outro_maps_low_confidence() {
    let map = test_timeline_map_v3(
        MatchClassV3::SharedIntroOutroOnly,
        vec![test_segment_v3(0, 90_000, 0, 90_000, 1_000_000)],
    );

    let mapped =
        map_query_position_to_candidate_ms(&map, 30_000).expect("edge segment maps diagnostically");

    assert_eq!(mapped.class_at_position, MatchClassV3::SharedIntroOutroOnly);
    assert!(mapped.confidence <= 0.25, "{mapped:?}");
}

#[test]
fn timeline_mapping_rejects_non_positive_scale() {
    let zero = test_timeline_map_v3(
        MatchClassV3::SameCutStrong,
        vec![test_segment_v3(0, 90_000, 0, 90_000, 0)],
    );
    let negative = test_timeline_map_v3(
        MatchClassV3::SameCutStrong,
        vec![test_segment_v3(0, 90_000, 0, 90_000, -1)],
    );

    assert!(map_query_position_to_candidate_ms(&zero, 30_000).is_none());
    assert!(map_query_position_to_candidate_ms(&negative, 30_000).is_none());
    assert!(map_candidate_position_to_query_ms(&zero, 30_000).is_none());
    assert!(map_candidate_position_to_query_ms(&negative, 30_000).is_none());
}

#[test]
fn timeline_mapping_absurd_scale_does_not_panic() {
    let map = test_timeline_map_v3(
        MatchClassV3::SameCutStrong,
        vec![test_segment_v3(0, u32::MAX, 0, u32::MAX, i32::MAX)],
    );

    let mapped = map_query_position_to_candidate_ms(&map, u32::MAX)
        .expect("i128 arithmetic should handle public extreme scale safely");

    assert_eq!(mapped.mapped_ms, u32::MAX);
}

fn test_segment_v3(
    query_start_ms: u32,
    query_end_ms: u32,
    candidate_start_ms: u32,
    candidate_end_ms: u32,
    scale_ppm: i32,
) -> AlignedSegmentV3 {
    AlignedSegmentV3 {
        query_start_ms,
        query_end_ms,
        candidate_start_ms,
        candidate_end_ms,
        scale_ppm,
        audio_pairs: 8,
        video_pairs: 0,
        weighted_score: 8,
        residual_ms: 0.0,
        audio_score: 1.0,
        video_score: 0.0,
        confidence: 1.0,
    }
}

fn test_timeline_map_v3(
    global_class: MatchClassV3,
    segments: Vec<AlignedSegmentV3>,
) -> MediaTimelineMapV3 {
    let total_aligned_span_ms = segments
        .iter()
        .map(|segment| segment.query_end_ms.saturating_sub(segment.query_start_ms))
        .sum();
    MediaTimelineMapV3 {
        global_class,
        current_position_class: global_class,
        segments,
        total_aligned_span_ms,
        largest_gap_ms: 0,
        edge_only: false,
        audio_video_conflict: false,
        best_segment_score: 8,
        second_best_segment_score: 0,
        piecewise_pair_count: 8,
        piecewise_hypothesis_count: 1,
        piecewise_segment_candidate_count: 1,
        piecewise_segment_chain_count: 1,
        piecewise_fit_millis: 0,
    }
}

#[test]
fn v3_audio_only_offset_recovery_is_within_one_second() {
    let query = audio_only_v3_anchor_profile(1_200_000, 0, 0);
    let candidate = audio_only_v3_anchor_profile(1_201_000, 1_000, 0);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert_eq!(decision.tier, MediaMatchTier::Strong, "{decision:?}");
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutStrong)
    );
    let alignment = decision.evidence.alignment.expect("alignment evidence");
    assert!((alignment.offset_seconds - 1.0).abs() <= 1.0);
    assert_eq!(alignment.aligned_video_anchors, 0);
    assert!(alignment.aligned_audio_anchors >= 16);
    assert!(
        decision
            .evidence
            .timeline_map_v3
            .as_ref()
            .is_some_and(|map| !map.segments.is_empty())
    );
}

#[test]
fn v3_audio_only_drift_recovery_reports_affine_scale() {
    let query = audio_only_v3_anchor_profile(1_200_000, 0, 0);
    let candidate = audio_only_v3_anchor_profile(1_202_000, 0, 1_500);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(
        matches!(
            decision.tier,
            MediaMatchTier::Strong | MediaMatchTier::Probable
        ),
        "{decision:?}"
    );
    let alignment = decision.evidence.alignment.expect("alignment evidence");
    assert!(
        (alignment.scale_ppm - 1_001_500).abs() <= 300,
        "{alignment:?}"
    );
}

#[test]
fn same_cut_strong_single_segment_is_autoplay_eligible() {
    let audio = v3_audio_times(120_000, 18, 45_000);
    let query = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate = v3_profile_from_times(1_200_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);
    let mut settings = enabled_settings();
    settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

    let decision = decide_media_match_anchors(&query, &candidate, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Strong);
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutStrong)
    );
    assert!(decision.same_media_for_autoplay(&settings));
    let map = decision.evidence.timeline_map_v3.expect("timeline map");
    assert_eq!(map.global_class, MatchClassV3::SameCutStrong);
    assert_eq!(map.segments.len(), 1);
    assert!(map.total_aligned_span_ms >= 600_000);
}

#[test]
fn sampled_only_same_cut_is_probable_and_not_autoplay_eligible() {
    let audio = v3_audio_times(120_000, 18, 45_000);
    let query_profile = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate_profile =
        v3_profile_from_times(1_200_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);
    let mut query = record_from_anchor_profile("query.mkv", 10, query_profile);
    let mut candidate = record_from_anchor_profile("candidate.mkv", 11, candidate_profile);
    query.extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
    candidate.extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
    let mut settings = enabled_settings();
    settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

    let decision = decide_media_match(&query, &candidate, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Probable);
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutProbable)
    );
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn sparse_full_same_cut_is_probable_and_not_autoplay_eligible() {
    let audio = v3_audio_times(120_000, 18, 45_000);
    let query_profile = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate_profile =
        v3_profile_from_times(1_200_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);
    let mut query = record_from_anchor_profile("query.mkv", 10, query_profile);
    let mut candidate = record_from_anchor_profile("candidate.mkv", 11, candidate_profile);
    query.extraction_settings = MediaExtractionSettings::sparse_full_audio_v3();
    candidate.extraction_settings = MediaExtractionSettings::sparse_full_audio_v3();
    let mut settings = enabled_settings();
    settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

    let decision = decide_media_match(&query, &candidate, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Probable);
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutProbable)
    );
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn v3_decision_diagnostics_include_class_and_segment_counts() {
    let audio = v3_audio_times(120_000, 18, 45_000);
    let query = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate = v3_profile_from_times(1_200_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());
    let summary = summarize_decision_v3_diagnostics(&decision);

    assert_eq!(summary.decision_tier, Some(MediaMatchTier::Strong));
    assert_eq!(summary.decision_class, Some(MatchClassV3::SameCutStrong));
    assert_eq!(summary.piecewise_segment_count, Some(1));
    assert!(summary.piecewise_pair_count.unwrap_or_default() > 0);
    assert!(summary.notes.iter().any(|note| note.contains("segments=1")));
}

#[test]
fn affine_drift_single_segment_reports_v3_scale() {
    let audio = v3_audio_times(120_000, 18, 45_000);
    let query = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate = v3_profile_from_times(1_202_000, &v3_shift_audio_times(&audio, 0, 1_500), &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(
        matches!(
            decision.tier,
            MediaMatchTier::Strong | MediaMatchTier::Probable
        ),
        "{decision:?}"
    );
    let map = decision.evidence.timeline_map_v3.expect("timeline map");
    assert_eq!(map.segments.len(), 1);
    assert!((map.segments[0].scale_ppm - 1_001_500).abs() <= 300);
}

#[test]
fn piecewise_hypothesis_pair_selection_is_capped_and_preserves_modalities() {
    let pairs = (0..700)
        .map(|index| AnchorMatchPair {
            query_t_ms: 60_000 + index * 2_000,
            candidate_t_ms: 65_000 + index * 2_000,
            modality: if index % 5 == 0 {
                AnchorModality::Video
            } else {
                AnchorModality::Audio
            },
            weight: if index % 7 == 0 { 8 } else { 1 },
        })
        .collect::<Vec<_>>();

    let selected = select_v3_piecewise_hypothesis_pairs(&pairs);

    assert!(selected.len() <= V3_PIECEWISE_MAX_HYPOTHESIS_PAIRS);
    assert!(
        selected
            .iter()
            .any(|pair| pair.modality == AnchorModality::Audio)
    );
    assert!(
        selected
            .iter()
            .any(|pair| pair.modality == AnchorModality::Video)
    );
}

#[test]
fn audio_only_body_same_cut_uses_fast_verifier() {
    let audio = v3_audio_times(180_000, 28, 36_000);
    let query = v3_profile_from_times(1_400_000, &audio, &[]);
    let candidate = v3_profile_from_times(1_400_000, &v3_shift_audio_times(&audio, 750, 0), &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutStrong),
        "{decision:?}"
    );
    let map = decision.evidence.timeline_map_v3.expect("timeline map");
    assert_eq!(map.piecewise_fit_millis, 0, "{map:?}");
    assert_eq!(map.piecewise_hypothesis_count, 0, "{map:?}");
    assert!(
        decision
            .evidence
            .notes
            .iter()
            .any(|note| note.contains("fast_audio_verifier class=SameCutStrong")),
        "{:?}",
        decision.evidence.notes
    );
}

#[test]
fn wrong_episode_shared_edges_do_not_pass_fast_audio_verifier() {
    let audio = vec![
        (1_000, 0),
        (1_001, 30_000),
        (1_002, 60_000),
        (1_003, 1_100_000),
        (1_004, 1_130_000),
        (1_005, 1_160_000),
    ];
    let query = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate = v3_profile_from_times(1_200_000, &v3_shift_audio_times(&audio, 0, 0), &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(!matches!(decision.tier, MediaMatchTier::Strong));
    assert!(
        !decision
            .evidence
            .notes
            .iter()
            .any(|note| note.contains("fast_audio_verifier class=SameCutStrong"))
    );
    assert!(matches!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SharedIntroOutroOnly | MatchClassV3::Reject)
    ));
}

#[test]
fn piecewise_hypothesis_generation_is_hard_capped() {
    let audio = v3_audio_times(120_000, 220, 5_000);
    let mut candidate =
        v3_profile_from_times(1_400_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);
    candidate.video_anchors.push(VideoAnchor {
        bucket: 42,
        t_ms: 500_000,
        hash64: 42,
        kind: V3_VIDEO_KIND_LUMA_FRAME,
        weight: 1,
    });
    let query = v3_profile_from_times(1_400_000, &audio, &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());
    let map = decision.evidence.timeline_map_v3.expect("timeline map");

    assert!(
        map.piecewise_hypothesis_count <= V3_PIECEWISE_MAX_HYPOTHESES,
        "{map:?}"
    );
}

#[test]
fn sparse_same_cut_common_gap_is_not_different_cut() {
    let mut audio = v3_audio_times(120_000, 8, 45_000);
    audio.extend(
        (0..8)
            .map(|index| (2_000 + index, 850_000 + (index * 45_000)))
            .collect::<Vec<_>>(),
    );
    let query = v3_profile_from_times(1_400_000, &audio, &[]);
    let candidate = v3_profile_from_times(1_400_000, &v3_shift_audio_times(&audio, 5_000, 0), &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert_ne!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameMediaDifferentCut),
        "{decision:?}"
    );
    assert!(matches!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutStrong | MatchClassV3::SameCutProbable)
    ));
    let map = decision.evidence.timeline_map_v3.expect("timeline map");
    assert!(map.segments.len() >= 2, "{map:?}");
}

#[test]
fn trimmed_intro_maps_as_different_cut_not_autoplay() {
    let audio = v3_audio_times(60_000, 24, 45_000);
    let query = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate_audio = audio[4..].to_vec();
    let candidate = v3_profile_from_times(1_020_000, &candidate_audio, &[]);
    let mut settings = enabled_settings();
    settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

    let decision = decide_media_match_anchors(&query, &candidate, &settings);

    assert!(matches!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameMediaDifferentCut | MatchClassV3::PartialOverlap)
    ));
    assert_ne!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SharedIntroOutroOnly)
    );
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn inserted_logo_piecewise_chain_maps_two_segments() {
    let audio = v3_audio_times(120_000, 20, 45_000);
    let query = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate_audio = audio
        .iter()
        .enumerate()
        .map(|(index, (bucket, t_ms))| {
            let offset = if index < 9 { 5_000 } else { 80_000 };
            (*bucket, shifted_anchor_time(*t_ms, offset, 0))
        })
        .collect::<Vec<_>>();
    let candidate = v3_profile_from_times(1_280_000, &candidate_audio, &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    let map = decision.evidence.timeline_map_v3.expect("timeline map");
    assert_eq!(map.global_class, MatchClassV3::SameMediaDifferentCut);
    assert!(map.segments.len() >= 2, "{map:?}");
}

#[test]
fn removed_recap_piecewise_chain_maps_two_segments() {
    let mut audio = v3_audio_times(120_000, 8, 45_000);
    audio.extend(
        (0..12)
            .map(|index| (1_008 + index, 600_000 + (index * 45_000)))
            .collect::<Vec<_>>(),
    );
    let query = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate_audio = audio
        .iter()
        .enumerate()
        .map(|(index, (bucket, t_ms))| {
            let offset = if index < 8 { 5_000 } else { -65_000 };
            (*bucket, shifted_anchor_time(*t_ms, offset, 0))
        })
        .collect::<Vec<_>>();
    let candidate = v3_profile_from_times(1_130_000, &candidate_audio, &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    let map = decision.evidence.timeline_map_v3.expect("timeline map");
    assert_eq!(map.global_class, MatchClassV3::SameMediaDifferentCut);
    assert!(map.segments.len() >= 2, "{map:?}");
}

#[test]
fn wrong_episode_shared_intro_outro_is_edge_only() {
    let audio = vec![
        (1_000, 0),
        (1_001, 30_000),
        (1_002, 60_000),
        (1_003, 1_100_000),
        (1_004, 1_130_000),
        (1_005, 1_160_000),
    ];
    let video = v3_video_times_from_audio(&audio);
    let query = v3_profile_from_times(1_200_000, &audio, &video);
    let candidate = query.clone();

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(!matches!(decision.tier, MediaMatchTier::Strong));
    assert!(matches!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SharedIntroOutroOnly | MatchClassV3::Reject)
    ));
    assert!(
        decision
            .evidence
            .timeline_map_v3
            .as_ref()
            .is_some_and(|map| map.edge_only)
    );
}

#[test]
fn partial_overlap_trailer_or_clip_is_not_same_cut() {
    let audio = v3_audio_times(420_000, 6, 30_000);
    let query = v3_profile_from_times(1_200_000, &audio, &[]);
    let candidate = v3_profile_from_times(240_000, &v3_shift_audio_times(&audio, -360_000, 0), &[]);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::PartialOverlap)
    );
    assert_ne!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutStrong)
    );
}

#[test]
fn same_audio_weak_video_is_not_false_conflict() {
    let audio = v3_audio_times(120_000, 18, 45_000);
    let query_video = vec![(10_000, 120_000, synthetic_hash(1))];
    let candidate_video = vec![(20_000, 125_000, synthetic_hash(2))];
    let query = v3_profile_from_times(1_200_000, &audio, &query_video);
    let candidate = v3_profile_from_times(
        1_200_000,
        &v3_shift_audio_times(&audio, 5_000, 0),
        &candidate_video,
    );

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(
        matches!(
            decision.evidence.v3_class,
            Some(MatchClassV3::SameCutStrong | MatchClassV3::SameCutProbable)
        ),
        "{decision:?}"
    );
    assert_ne!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameAudioDifferentVideo)
    );
}

#[test]
fn same_audio_different_video_is_not_autoplay() {
    let audio = v3_audio_times(120_000, 18, 45_000);
    let video = v3_video_times_from_audio(&audio);
    let query = v3_profile_from_times(1_200_000, &audio, &video);
    let candidate = v3_profile_from_times(
        1_200_000,
        &v3_shift_audio_times(&audio, 5_000, 0),
        &v3_shift_video_times(&video, 90_000, 0),
    );
    let mut settings = enabled_settings();
    settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

    let decision = decide_media_match_anchors(&query, &candidate, &settings);

    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameAudioDifferentVideo)
    );
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn same_video_different_audio_requires_contradictory_audio() {
    let video_source = v3_audio_times(120_000, 18, 45_000);
    let video = v3_video_times_from_audio(&video_source);
    let audio = v3_audio_times(120_000, 6, 45_000);
    let query = v3_profile_from_times(1_200_000, &audio, &video);
    let candidate = v3_profile_from_times(
        1_200_000,
        &v3_shift_audio_times(&audio, 90_000, 0),
        &v3_shift_video_times(&video, 5_000, 0),
    );

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameVideoDifferentAudio)
    );
}

#[test]
fn same_video_different_audio_is_not_autoplay() {
    let audio = v3_audio_times(120_000, 12, 45_000);
    let video = v3_video_times_from_audio(&audio);
    let query = v3_profile_from_times(1_200_000, &[], &video);
    let candidate = v3_profile_from_times(1_200_000, &[], &v3_shift_video_times(&video, 5_000, 0));
    let mut settings = enabled_settings();
    settings.autoplay_policy = MediaMatchAutoplayPolicy::AllowStrongSameMedia;

    let decision = decide_media_match_anchors(&query, &candidate, &settings);

    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameVideoDifferentAudio)
    );
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn anchor_matching_estimates_simple_offset_within_one_second() {
    let query = regular_anchor_profile(900_000, 0, 0);
    let candidate = regular_anchor_profile(901_000, 1_000, 0);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert_eq!(decision.tier, MediaMatchTier::Strong);
    let alignment = decision.evidence.alignment.expect("alignment evidence");
    assert!((alignment.offset_seconds - 1.0).abs() <= 1.0);
    assert!(alignment.aligned_audio_anchors >= 10);
    assert!(alignment.aligned_video_anchors >= 10);
}

#[test]
fn anchor_matching_accepts_small_drift_but_reports_it() {
    let query = regular_anchor_profile(900_000, 0, 0);
    let candidate = regular_anchor_profile(901_000, 1_000, 2_000);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(
        matches!(
            decision.tier,
            MediaMatchTier::Strong | MediaMatchTier::Probable
        ),
        "{decision:?}"
    );
    let alignment = decision.evidence.alignment.expect("alignment evidence");
    assert!(alignment.drift_ratio <= 0.015);
    assert!(alignment.scale_ppm > 1_000_000);
}

#[test]
fn video_lsh_matches_hashes_with_high_bit_differences() {
    let query_hash = 0x0123_4567_89ab_cdef;
    let candidate_hash = query_hash ^ (1 << 60);
    assert!(video_anchor_hashes_match(query_hash, candidate_hash));

    let query_video = VideoFingerprint {
        duration_seconds: Some(120),
        frames: vec![FrameFingerprint {
            timestamp_millis: 30_000,
            hash: query_hash,
        }],
        v3_landmarks: Vec::new(),
    };
    let candidate_video = VideoFingerprint {
        duration_seconds: Some(120),
        frames: vec![FrameFingerprint {
            timestamp_millis: 31_000,
            hash: candidate_hash,
        }],
        v3_landmarks: Vec::new(),
    };
    let query = MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(120_000),
        audio_anchors: Vec::new(),
        video_anchors: video_anchors_from_fingerprint(&query_video, 4),
    };
    let candidate = MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(120_000),
        audio_anchors: Vec::new(),
        video_anchors: video_anchors_from_fingerprint(&candidate_video, 4),
    };

    let pairs = collect_anchor_match_pairs(&query, &candidate);

    assert!(
        !pairs.is_empty(),
        "multi-bucket LSH should find a Hamming-near video hash even when high bits differ"
    );
}

#[test]
fn video_matching_falls_back_when_hamming_near_hash_touches_every_lsh_band() {
    let query_hash = 0x0123_4567_89ab_cdef;
    let candidate_hash = query_hash ^ 0x0001_0001_0001_0001;
    assert!(video_anchor_hashes_match(query_hash, candidate_hash));
    assert!(
        video_lsh_buckets(query_hash)
            .iter()
            .all(|bucket| !video_lsh_buckets(candidate_hash).contains(bucket)),
        "fixture must touch every contiguous LSH band"
    );
    let query_video = VideoFingerprint {
        duration_seconds: Some(120),
        frames: vec![FrameFingerprint {
            timestamp_millis: 30_000,
            hash: query_hash,
        }],
        v3_landmarks: Vec::new(),
    };
    let candidate_video = VideoFingerprint {
        duration_seconds: Some(120),
        frames: vec![FrameFingerprint {
            timestamp_millis: 31_000,
            hash: candidate_hash,
        }],
        v3_landmarks: Vec::new(),
    };
    let query = MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(120_000),
        audio_anchors: Vec::new(),
        video_anchors: video_anchors_from_fingerprint(&query_video, 4),
    };
    let candidate = MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(120_000),
        audio_anchors: Vec::new(),
        video_anchors: video_anchors_from_fingerprint(&candidate_video, 4),
    };

    let pairs = collect_anchor_match_pairs(&query, &candidate);

    assert!(
        !pairs.is_empty(),
        "Hamming fallback should recover a near perceptual hash even when LSH buckets all differ"
    );
}

#[test]
fn video_anchor_coverage_counts_unique_frames_not_lsh_bands() {
    let hash = synthetic_hash(42);
    let query_video = video_from_hashes(30, 10, &[hash]);
    let candidate_video = video_from_hashes(32, 10, &[hash]);
    let query = MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(120_000),
        audio_anchors: Vec::new(),
        video_anchors: video_anchors_from_fingerprint(&query_video, 4),
    };
    let candidate = MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: "combined-v3".to_owned(),
        duration_ms: Some(120_000),
        audio_anchors: Vec::new(),
        video_anchors: video_anchors_from_fingerprint(&candidate_video, 4),
    };

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());
    let video = decision
        .evidence
        .video
        .expect("video evidence should be present");

    assert_eq!(video.aligned_pairs, 1);
    assert_eq!(video.query_coverage, 1.0);
    assert_eq!(video.candidate_coverage, 1.0);
}

#[test]
fn anchor_matching_handles_trimmed_start_body_overlap() {
    let query = regular_anchor_profile(1_200_000, 0, 0);
    let candidate_audio = query.audio_anchors[3..].to_vec();
    let candidate_video = query.video_anchors[3..].to_vec();
    let candidate = MediaAnchorProfile {
        audio_anchors: candidate_audio,
        video_anchors: candidate_video,
        duration_ms: Some(1_020_000),
        ..query.clone()
    };

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(
        matches!(
            decision.tier,
            MediaMatchTier::Strong | MediaMatchTier::Probable
        ),
        "{decision:?}"
    );
    assert!(
        decision
            .evidence
            .alignment
            .as_ref()
            .is_some_and(|alignment| alignment.aligned_span_seconds >= 300.0)
    );
}

#[test]
fn anchor_matching_rejects_wrong_episode_with_shared_edges() {
    let intro_times = [0, 30_000, 60_000];
    let outro_times = [1_100_000, 1_130_000, 1_160_000];
    let query_audio = intro_times
        .into_iter()
        .chain(outro_times)
        .enumerate()
        .map(|(index, t_ms)| (index as u32 + 1, t_ms))
        .collect::<Vec<_>>();
    let query_video = query_audio
        .iter()
        .map(|(bucket, t_ms)| (*bucket + 100, *t_ms, synthetic_hash(u64::from(*bucket))))
        .collect::<Vec<_>>();
    let query = anchor_profile(1_200_000, &query_audio, &query_video);
    let candidate = query.clone();

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(
        !matches!(decision.tier, MediaMatchTier::Strong),
        "shared intro/outro anchors must not be strong: {decision:?}"
    );
}

#[test]
fn anchor_matching_fits_long_duration_drift_when_offset_bins_are_spread() {
    let query_times = (0..20)
        .map(|index| 120_000 + index * 180_000)
        .collect::<Vec<_>>();
    let query_audio = query_times
        .iter()
        .enumerate()
        .map(|(index, t_ms)| (index as u32 + 1, *t_ms))
        .collect::<Vec<_>>();
    let query_video = query_times
        .iter()
        .enumerate()
        .map(|(index, t_ms)| (index as u32 + 100, *t_ms, synthetic_hash(index as u64 + 1)))
        .collect::<Vec<_>>();
    let candidate_audio = query_times
        .iter()
        .enumerate()
        .map(|(index, t_ms)| (index as u32 + 1, shifted_anchor_time(*t_ms, 0, 1_200)))
        .collect::<Vec<_>>();
    let candidate_video = query_times
        .iter()
        .enumerate()
        .map(|(index, t_ms)| {
            (
                index as u32 + 100,
                shifted_anchor_time(*t_ms, 0, 1_200),
                synthetic_hash(index as u64 + 1),
            )
        })
        .collect::<Vec<_>>();
    let query = anchor_profile(3_800_000, &query_audio, &query_video);
    let candidate = anchor_profile(3_800_000, &candidate_audio, &candidate_video);

    let decision = decide_media_match_anchors(&query, &candidate, &enabled_settings());

    assert!(
        matches!(
            decision.tier,
            MediaMatchTier::Strong | MediaMatchTier::Probable
        ),
        "{decision:?}"
    );
    let alignment = decision.evidence.alignment.expect("alignment evidence");
    assert!(alignment.aligned_pairs >= 30, "{alignment:?}");
    assert!(
        (alignment.scale_ppm - 1_001_200).abs() <= 250,
        "{alignment:?}"
    );
}

#[test]
fn audio_constellation_v3_process_budget_is_audio_only() {
    let counts =
        expected_media_tool_invocation_counts(&MediaExtractionSettings::audio_constellation_v3());
    assert_eq!(counts.ffmpeg + counts.ffprobe, 2);
    assert_eq!(counts.ffmpeg, 1);
}

#[test]
fn combined_v3_process_budget_includes_video() {
    let counts = expected_media_tool_invocation_counts(&MediaExtractionSettings::combined_v3());
    assert_eq!(counts.ffmpeg, 2);
    assert_eq!(counts.ffprobe, 1);
}

#[test]
fn audio_constellation_v3_extraction_uses_ffmpeg_and_ffprobe_only() {
    let root = unique_test_root("audio-v3-tools");
    let media_path = root.join("episode.mkv");
    std::fs::write(&media_path, b"not real media").expect("test media should be written");
    let tools = MediaMatchToolPaths {
        ffmpeg: write_fake_tool(&root, "ffmpeg", None),
        ffprobe: write_fake_tool(&root, "ffprobe", Some("1.0")),
    };

    let result = fingerprint_media_file_with_report(
        &media_path,
        &tools,
        &MediaExtractionSettings::audio_constellation_v3(),
        None,
    )
    .expect("V3 fingerprint should tolerate empty fake ffmpeg as a modality error");

    assert_eq!(result.report.invocations.ffprobe, 1);
    assert_eq!(result.report.invocations.ffmpeg, 1);
    assert!(result.record.audio_error.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn combined_v3_extraction_uses_ffmpeg_and_ffprobe_only() {
    let root = unique_test_root("combined-v3-tools");
    let media_path = root.join("episode.mkv");
    std::fs::write(&media_path, b"not real media").expect("test media should be written");
    let tools = MediaMatchToolPaths {
        ffmpeg: write_fake_tool(&root, "ffmpeg", None),
        ffprobe: write_fake_tool(&root, "ffprobe", Some("1.0")),
    };

    let result = fingerprint_media_file_with_report(
        &media_path,
        &tools,
        &MediaExtractionSettings::combined_v3(),
        None,
    )
    .expect("combined V3 fingerprint should tolerate empty fake ffmpeg as modality errors");

    assert_eq!(result.report.invocations.ffprobe, 1);
    assert_eq!(result.report.invocations.ffmpeg, 2);
    assert!(result.record.audio_error.is_some());
    assert!(result.record.video_error.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn wire_signature_round_trips_v3_audio_profile() {
    let mut record = record_with_extraction_settings(
        "[Judas] Show - S01E07.mkv",
        100,
        Some(1412.37),
        None,
        MediaExtractionSettings::audio_constellation_v3(),
    );
    record.audio_anchors = audio_only_v3_anchor_profile(1_412_370, 0, 0).audio_anchors;

    let value = media_match_wire_value_from_records(std::slice::from_ref(&record))
        .expect("wire value should serialize");
    let signature =
        media_match_wire_signature_from_value(&value).expect("wire signature should parse");
    let profile = media_anchor_profile_from_wire_profile(&signature.profiles[0])
        .expect("v3 profile should decode");

    assert_eq!(signature.schema, MEDIA_MATCH_WIRE_SCHEMA_V3);
    assert_eq!(signature.profiles[0].profile, "audio-constellation-v3");
    assert!(!profile.audio_anchors.is_empty());
    assert!(profile.video_anchors.is_empty());
}

#[test]
fn wire_signature_compares_local_record_to_remote_profile() {
    let query_profile = audio_only_v3_anchor_profile(1_412_000, 0, 0);
    let remote_profile = audio_only_v3_anchor_profile(1_413_000, 1_000, 0);
    let mut query = record_with_extraction_settings(
        "[Judas] Show - S01E07.mkv",
        100,
        Some(1412.0),
        None,
        MediaExtractionSettings::audio_constellation_v3(),
    );
    query.audio_anchors = query_profile.audio_anchors;
    let mut remote = record_with_extraction_settings(
        "[Erai-raws] Show - 07.mkv",
        200,
        Some(1413.0),
        None,
        MediaExtractionSettings::audio_constellation_v3(),
    );
    remote.audio_anchors = remote_profile.audio_anchors;
    let value =
        media_match_wire_value_from_records(&[remote]).expect("wire value should serialize");
    let signature =
        media_match_wire_signature_from_value(&value).expect("wire signature should parse");

    let decision =
        decide_media_match_against_wire_signature(&query, &signature, &enabled_settings());

    assert_eq!(decision.tier, MediaMatchTier::Strong);
}

#[test]
fn malformed_wire_signatures_are_ignored_for_autoplay() {
    let unsupported = serde_json::json!({
        "schema": "sorotte.mediaMatch.v999",
        "profiles": []
    });
    assert!(media_match_wire_signature_from_value(&unsupported).is_err());

    let legacy_v1 = serde_json::json!({
        "schema": format!("sorotte.mediaMatch.v{}", 1),
        "profiles": [{"profile": format!("fast-v{}", 1)}]
    });
    assert!(media_match_wire_signature_from_value(&legacy_v1).is_err());
}

#[test]
fn wire_signature_rejects_unsupported_v3_profile_fields() {
    let mut record = record_with_extraction_settings(
        "episode.mkv",
        100,
        Some(120.0),
        None,
        MediaExtractionSettings::audio_constellation_v3(),
    );
    record.audio_anchors = audio_only_v3_anchor_profile(120_000, 0, 0).audio_anchors;
    let value = media_match_wire_value_from_records(std::slice::from_ref(&record))
        .expect("wire value should serialize");

    let mut unsupported_version = value.clone();
    unsupported_version["profiles"][0]["algorithmVersion"] =
        serde_json::json!(MEDIA_MATCH_ANCHOR_VERSION + 1);
    assert!(media_match_wire_signature_from_value(&unsupported_version).is_err());

    let mut unknown_profile = value.clone();
    unknown_profile["profiles"][0]["profile"] = serde_json::json!("audio-v999");
    assert!(media_match_wire_signature_from_value(&unknown_profile).is_err());

    let mut wrong_time_base = value.clone();
    wrong_time_base["profiles"][0]["audio"]["timeBaseMs"] = serde_json::json!(1000);
    assert!(media_match_wire_signature_from_value(&wrong_time_base).is_err());

    let mut wrong_algorithm = value;
    wrong_algorithm["profiles"][0]["audio"]["algorithm"] =
        serde_json::json!("unsupported-audio-anchor-algorithm");
    assert!(media_match_wire_signature_from_value(&wrong_algorithm).is_err());
}

#[test]
fn ffmpeg_showinfo_parser_preserves_irregular_frame_pts() {
    let stderr = "\
[Parsed_showinfo_1 @ 000001] n:   0 pts: 48000 pts_time:2.000 pos:0
[Parsed_showinfo_1 @ 000001] n:   1 pts: 103200 pts_time:4.300 pos:0
[Parsed_showinfo_1 @ 000001] n:   2 pts: 247200 pts_time:10.300 pos:0
";

    assert_eq!(
        parse_ffmpeg_showinfo_pts_times(stderr),
        vec![2.0, 4.3, 10.3]
    );
}

#[test]
fn ffmpeg_rawvideo_parser_uses_showinfo_pts_for_full_profile_frames() {
    let mut stdout = vec![32u8; VIDEO_FRAME_BYTES];
    stdout.extend(std::iter::repeat_n(224u8, VIDEO_FRAME_BYTES));
    let stderr = "\
[Parsed_showinfo_1 @ 000001] n:   0 pts: 48000 pts_time:2.000 pos:0
[Parsed_showinfo_1 @ 000001] n:   1 pts: 103200 pts_time:4.300 pos:0
";

    let frames = video_frames_from_ffmpeg_rawvideo(&stdout, stderr.as_bytes())
        .expect("frames should decode");

    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].timestamp_millis, 2_000);
    assert_eq!(frames[1].timestamp_millis, 4_300);
}

#[test]
#[ignore = "requires ffmpeg/ffprobe in SOROTTE_MEDIA_MATCH_FFMPEG/SOROTTE_MEDIA_MATCH_FFPROBE or PATH"]
fn combined_v3_ffmpeg_generates_v3_video_kinds() {
    let Some(ffmpeg) = test_ffmpeg_path() else {
        eprintln!("skipping ignored ffmpeg test: ffmpeg is not available");
        return;
    };
    let Some(ffprobe) = test_ffprobe_path() else {
        eprintln!("skipping ignored ffmpeg test: ffprobe is not available");
        return;
    };
    let media_path = temp_media_match_path("combined-v3-kinds", "mkv");
    let status = Command::new(&ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x64:rate=1:duration=90",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=90",
            "-shortest",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ])
        .arg(&media_path)
        .status()
        .expect("ffmpeg should create synthetic media");
    assert!(status.success(), "ffmpeg fixture generation failed");
    let tools = MediaMatchToolPaths { ffmpeg, ffprobe };
    let fingerprint = fingerprint_media_file_with_report(
        &media_path,
        &tools,
        &MediaExtractionSettings::combined_v3(),
        None,
    )
    .expect("combined v3 fingerprint should extract");
    let _ = std::fs::remove_file(&media_path);
    let video_landmarks = fingerprint
        .record
        .video
        .as_ref()
        .map(|video| video.v3_landmarks.as_slice())
        .unwrap_or_default();
    let kinds = video_landmarks
        .iter()
        .map(|landmark| landmark.kind)
        .collect::<HashSet<_>>();

    assert!(kinds.contains(&V3_VIDEO_KIND_GLOBAL_DCT));
    assert!(kinds.contains(&V3_VIDEO_KIND_CENTER_DCT));
    assert!(kinds.contains(&V3_VIDEO_KIND_EDGE));
    assert!(kinds.contains(&V3_VIDEO_KIND_TEMPORAL_SHINGLE));
}

#[test]
#[ignore = "requires ffmpeg/ffprobe in SOROTTE_MEDIA_MATCH_FFMPEG/SOROTTE_MEDIA_MATCH_FFPROBE or PATH"]
fn audio_v3_streaming_extracts_synthetic_audio() {
    let Some(ffmpeg) = test_ffmpeg_path() else {
        eprintln!("skipping ignored ffmpeg test: ffmpeg is not available");
        return;
    };
    let Some(ffprobe) = test_ffprobe_path() else {
        eprintln!("skipping ignored ffmpeg test: ffprobe is not available");
        return;
    };
    let media_path = temp_media_match_path("audio-v3-streaming", "wav");
    let status = Command::new(&ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:sample_rate=44100:duration=12",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&media_path)
        .status()
        .expect("ffmpeg should create synthetic audio");
    assert!(status.success(), "ffmpeg fixture generation failed");
    let tools = MediaMatchToolPaths { ffmpeg, ffprobe };
    let fingerprint = fingerprint_media_file_with_report(
        &media_path,
        &tools,
        &MediaExtractionSettings::audio_constellation_v3(),
        None,
    )
    .expect("audio v3 fingerprint should extract");
    let _ = std::fs::remove_file(&media_path);

    assert!(!audio_landmarks_v3_from_record(&fingerprint.record).is_empty());
    assert!(fingerprint.report.audio_stream.streamed_bytes > 0);
    assert!(fingerprint.report.audio_stream.max_buffer_samples <= V3_AUDIO_WINDOW_SAMPLES);
    assert!(
        fingerprint
            .report
            .audio_stream
            .max_raw_landmarks_after_compaction
            <= V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT
    );
}

#[test]
#[ignore = "requires ffmpeg/ffprobe in SOROTTE_MEDIA_MATCH_FFMPEG/SOROTTE_MEDIA_MATCH_FFPROBE or PATH"]
fn combined_v3_storage_bound_on_synthetic_media() {
    let Some(ffmpeg) = test_ffmpeg_path() else {
        eprintln!("skipping ignored ffmpeg test: ffmpeg is not available");
        return;
    };
    let Some(ffprobe) = test_ffprobe_path() else {
        eprintln!("skipping ignored ffmpeg test: ffprobe is not available");
        return;
    };
    let media_path = temp_media_match_path("combined-v3-storage", "mkv");
    let status = Command::new(&ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x64:rate=1:duration=120",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=520:sample_rate=44100:duration=120",
            "-shortest",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ])
        .arg(&media_path)
        .status()
        .expect("ffmpeg should create synthetic media");
    assert!(status.success(), "ffmpeg fixture generation failed");
    let tools = MediaMatchToolPaths { ffmpeg, ffprobe };
    let fingerprint = fingerprint_media_file_with_report(
        &media_path,
        &tools,
        &MediaExtractionSettings::combined_v3(),
        None,
    )
    .expect("combined v3 fingerprint should extract");
    let _ = std::fs::remove_file(&media_path);
    let diagnostics = summarize_record_v3_diagnostics(&fingerprint.record);

    assert!(diagnostics.video_verify_count <= V3_VIDEO_VERIFY_LANDMARK_LIMIT);
    assert!(diagnostics.video_index_count <= V3_VIDEO_INDEX_LANDMARK_LIMIT);
    assert!(diagnostics.audio_blob_bytes > 0);
    assert!(diagnostics.video_blob_bytes > 0);
}

fn test_ffmpeg_path() -> Option<PathBuf> {
    test_tool_path("SOROTTE_MEDIA_MATCH_FFMPEG", "ffmpeg")
}

fn test_ffprobe_path() -> Option<PathBuf> {
    test_tool_path("SOROTTE_MEDIA_MATCH_FFPROBE", "ffprobe")
}

fn test_tool_path(env_key: &str, default_name: &str) -> Option<PathBuf> {
    let path = std::env::var_os(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default_name));
    let status = Command::new(&path)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    status.success().then_some(path)
}

fn temp_media_match_path(prefix: &str, extension: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "sorotte-{prefix}-{}-{extension}.{extension}",
        std::process::id()
    ));
    path
}

#[test]
fn exact_decision_uses_path_mtime_and_size() {
    let query = record("C:/Media/Movie.mkv", 100, Some(100.0), None);
    let candidate = query.clone();

    let decision = decide_media_match(&query, &candidate, &enabled_settings());

    assert_eq!(decision.tier, MediaMatchTier::Exact);
}

#[test]
fn strong_decision_requires_strong_fingerprint_evidence() {
    let query_profile = regular_anchor_profile(900_000, 0, 0);
    let candidate_profile = regular_anchor_profile(901_000, 20_000, 0);
    let query = record_from_anchor_profile("show.s01e01.web.mkv", 100, query_profile);
    let candidate = record_from_anchor_profile("Show - 01 BluRay.mkv", 120, candidate_profile);

    let decision = decide_media_match(&query, &candidate, &enabled_settings());

    assert_eq!(decision.tier, MediaMatchTier::Strong);
    assert!(
        decision
            .evidence
            .alignment
            .as_ref()
            .is_some_and(|alignment| alignment.offset_seconds > 19.0)
    );
}

#[test]
fn fast_strong_requires_audio_video_and_runtime_evidence() {
    let query_profile = regular_anchor_profile(900_000, 0, 0);
    let mut candidate_profile = regular_anchor_profile(901_000, 20_000, 0);
    candidate_profile.video_anchors.truncate(9);
    let query = record_from_anchor_profile("[Judas] Show - 07.mkv", 100, query_profile);
    let candidate =
        record_from_anchor_profile("[Erai-raws] Show - 07.mkv", 120, candidate_profile.clone());
    let mut no_audio_profile = candidate_profile.clone();
    no_audio_profile.audio_anchors.clear();
    let no_audio =
        record_from_anchor_profile("[Erai-raws] Show - 07 no-audio.mkv", 121, no_audio_profile);
    let mut no_video_profile = candidate_profile.clone();
    no_video_profile.video_anchors.clear();
    let no_video =
        record_from_anchor_profile("[Erai-raws] Show - 07 no-video.mkv", 122, no_video_profile);
    let mut wrong_runtime_profile = candidate_profile;
    wrong_runtime_profile.duration_ms = Some(910_000);
    let wrong_runtime =
        record_from_anchor_profile("[Erai-raws] Show - 07 long.mkv", 123, wrong_runtime_profile);
    let settings = enabled_settings();

    assert_eq!(
        decide_media_match(&query, &candidate, &settings).tier,
        MediaMatchTier::Strong
    );
    assert_ne!(
        decide_media_match(&query, &no_audio, &settings).tier,
        MediaMatchTier::Strong
    );
    assert_ne!(
        decide_media_match(&query, &no_video, &settings).tier,
        MediaMatchTier::Strong
    );
    assert_ne!(
        decide_media_match(&query, &wrong_runtime, &settings).tier,
        MediaMatchTier::Strong
    );
}

#[test]
fn probable_decision_is_not_autoplay_eligible() {
    let mut query_profile = regular_anchor_profile(900_000, 0, 0);
    query_profile.video_anchors.clear();
    query_profile.audio_anchors.truncate(8);
    let mut candidate_profile = regular_anchor_profile(900_000, 0, 0);
    candidate_profile.video_anchors.clear();
    candidate_profile.audio_anchors.truncate(8);
    let query = record_from_anchor_profile("episode-a.mkv", 100, query_profile);
    let candidate = record_from_anchor_profile("episode-b.mkv", 110, candidate_profile);
    let settings = MediaMatchSettings {
        autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        ..enabled_settings()
    };

    let decision = decide_media_match(&query, &candidate, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Probable);
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn weak_or_reject_for_wrong_episode_with_shared_intro_and_outro() {
    let query_hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    let candidate_hashes = synthetic_hashes(&[1, 2, 3, 90, 91, 92, 93, 94, 95, 10, 11, 12]);
    let query = record(
        "show-e01.mkv",
        100,
        Some(1200.0),
        Some(shifted_video(0, &query_hashes)),
    );
    let candidate = record(
        "show-e02.mkv",
        100,
        Some(1200.0),
        Some(shifted_video(0, &candidate_hashes)),
    );

    let decision = decide_media_match(&query, &candidate, &enabled_settings());

    assert!(
        matches!(decision.tier, MediaMatchTier::Weak | MediaMatchTier::Reject),
        "shared intro/outro must not be strong/probable: {decision:?}"
    );
}

#[test]
fn synthetic_alignment_handles_trimmed_intro() {
    let query_hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let candidate_hashes = synthetic_hashes(&[3, 4, 5, 6, 7, 8]);
    let query = video_from_hashes(0, 10, &query_hashes);
    let candidate = video_from_hashes(0, 10, &candidate_hashes);

    let evidence = align_video_fingerprints(&query, &candidate).expect("should align");

    assert_eq!(evidence.aligned_pairs, 6);
    assert!(evidence.query_coverage >= 0.75);
    assert_eq!(evidence.candidate_coverage, 1.0);
    assert!(evidence.best_offset_seconds < -19.0);
}

#[test]
fn synthetic_alignment_handles_trimmed_credits() {
    let query_hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let candidate_hashes = synthetic_hashes(&[1, 2, 3, 4, 5, 6]);
    let query = video_from_hashes(0, 10, &query_hashes);
    let candidate = video_from_hashes(0, 10, &candidate_hashes);

    let evidence = align_video_fingerprints(&query, &candidate).expect("should align");

    assert_eq!(evidence.aligned_pairs, 6);
    assert!(evidence.query_coverage >= 0.75);
    assert_eq!(evidence.candidate_coverage, 1.0);
}

#[test]
fn synthetic_alignment_rejects_mild_drift_as_strong() {
    let hashes = synthetic_hashes(&[10, 20, 30, 40, 50, 60, 70, 80]);
    let query = VideoFingerprint {
        duration_seconds: Some(80),
        frames: hashes
            .iter()
            .enumerate()
            .map(|(index, hash)| FrameFingerprint::new(index as f64 * 10.0, *hash))
            .collect(),
        v3_landmarks: Vec::new(),
    };
    let candidate = VideoFingerprint {
        duration_seconds: Some(86),
        frames: hashes
            .iter()
            .enumerate()
            .map(|(index, hash)| FrameFingerprint::new(index as f64 * 10.8, *hash))
            .collect(),
        v3_landmarks: Vec::new(),
    };

    let evidence = align_video_fingerprints(&query, &candidate).expect("should align");

    assert!(evidence.drift_ratio > 0.015);
}

#[test]
fn candidate_ranking_prefers_stronger_media_match_tiers() {
    let query_profile = regular_anchor_profile(900_000, 0, 0);
    let strong_profile = regular_anchor_profile(901_000, 10_000, 0);
    let mut weak_profile = regular_anchor_profile(900_000, 0, 0);
    weak_profile.audio_anchors.truncate(2);
    weak_profile.video_anchors.clear();
    let query = record_from_anchor_profile("episode.web.mkv", 100, query_profile);
    let weak = record_from_anchor_profile("maybe-episode.mkv", 110, weak_profile);
    let strong = record_from_anchor_profile("episode.bluray.mkv", 120, strong_profile);

    let ranked = rank_media_match_candidates(&query, [&weak, &strong], &enabled_settings());

    assert_eq!(ranked[0].decision.tier, MediaMatchTier::Strong);
    assert_eq!(
        ranked[0].candidate_path,
        normalize_media_path("episode.bluray.mkv")
    );
}

#[test]
fn candidate_ranking_prefers_nearest_reject_with_timeline_evidence() {
    let query_profile = anchor_profile(900_000, &[(42, 10_000)], &[]);
    let nearest_profile = anchor_profile(900_000, &[(42, 12_000)], &[]);
    let unrelated_profile = anchor_profile(900_000, &[(84, 10_000)], &[]);
    let query = record_from_anchor_profile("episode.web.mkv", 100, query_profile);
    let nearest = record_from_anchor_profile("episode-nearest.mkv", 110, nearest_profile);
    let unrelated = record_from_anchor_profile("episode-unrelated.mkv", 120, unrelated_profile);

    let ranked = rank_media_match_candidates(&query, [&unrelated, &nearest], &enabled_settings());

    assert_eq!(ranked[0].decision.tier, MediaMatchTier::Reject);
    assert_eq!(
        ranked[0].candidate_path,
        normalize_media_path("episode-nearest.mkv")
    );
    assert_eq!(
        ranked[0]
            .decision
            .evidence
            .alignment
            .as_ref()
            .map(|alignment| alignment.aligned_pairs),
        Some(1)
    );
}

#[test]
fn cache_invalidates_on_identity_and_algorithm_inputs() {
    let settings = MediaExtractionSettings::combined_v3();
    let audio_settings = MediaExtractionSettings::audio_constellation_v3();
    let mut cache = MediaMatchCache::default();
    let record = record("movie.mkv", 100, Some(10.0), None);
    cache.insert(record);

    assert!(
        cache
            .get_valid(
                "movie.mkv",
                1000,
                100,
                MEDIA_MATCH_ALGORITHM_VERSION,
                &settings
            )
            .is_some()
    );
    assert!(
        cache
            .get_valid(
                "movie.mkv",
                1001,
                100,
                MEDIA_MATCH_ALGORITHM_VERSION,
                &settings
            )
            .is_none()
    );
    assert!(
        cache
            .get_valid(
                "movie.mkv",
                1000,
                101,
                MEDIA_MATCH_ALGORITHM_VERSION,
                &settings
            )
            .is_none()
    );
    assert!(
        cache
            .get_valid(
                "movie.mkv",
                1000,
                100,
                MEDIA_MATCH_ALGORITHM_VERSION + 1,
                &settings
            )
            .is_none()
    );
    assert!(
        cache
            .get_valid(
                "movie.mkv",
                1000,
                100,
                MEDIA_MATCH_ALGORITHM_VERSION,
                &audio_settings
            )
            .is_none()
    );
}

#[test]
fn media_tool_runner_times_out_long_running_processes() {
    #[cfg(windows)]
    let (executable, args) = (
        Path::new("powershell.exe"),
        vec![
            std::ffi::OsString::from("-NoProfile"),
            std::ffi::OsString::from("-Command"),
            std::ffi::OsString::from("Start-Sleep -Seconds 2"),
        ],
    );
    #[cfg(not(windows))]
    let (executable, args) = (
        Path::new("/bin/sh"),
        vec![
            std::ffi::OsString::from("-c"),
            std::ffi::OsString::from("sleep 2"),
        ],
    );

    let error = run_tool_output(
        "test-tool",
        executable,
        args,
        None,
        Duration::from_millis(75),
    )
    .expect_err("long-running media helper should time out");

    assert_eq!(
        error,
        MediaFingerprintError::TimedOut {
            tool: "test-tool",
            timeout_seconds: 1,
        }
    );
}

#[test]
fn pdq_style_luma_hash_is_stable_for_same_pixels() {
    let luma = (0u8..64).collect::<Vec<_>>();

    let left = pdq_style_luma_hash(8, 8, &luma).expect("hash");
    let right = pdq_style_luma_hash(8, 8, &luma).expect("hash");

    assert_eq!(left, right);
}
