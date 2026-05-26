use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    MEDIA_MATCH_ANCHOR_VERSION,
    audio_v3::{AudioLandmarkV3, bounded_time_distributed_audio_landmarks_v3},
    identity::duration_seconds_to_millis,
    settings::{MediaFingerprintProfile, media_extraction_settings_hash},
    tuning::{
        V3_AUDIO_INDEX_LANDMARK_LIMIT, V3_AUDIO_VERIFY_LANDMARK_LIMIT, V3_VIDEO_BUCKET_KIND_SHIFT,
        V3_VIDEO_INDEX_LANDMARK_LIMIT, V3_VIDEO_VERIFY_LANDMARK_LIMIT,
    },
    types::MediaFingerprintRecord,
    video_v3::{
        V3_VIDEO_KIND_LUMA_FRAME, V3_VIDEO_KIND_TEMPORAL_SHINGLE, VideoFingerprint,
        VideoLandmarkV3, anchor_bucket, bounded_time_distributed_video_anchors,
        bounded_time_distributed_video_landmarks_v3, v3_video_bucket_for_kind,
        v3_video_kind_from_bucket, v3_video_kind_is_supported, video_lsh_buckets,
    },
};

const MAX_WIRE_ANCHORS: usize = 1024;
const MAX_V3_LANDMARKS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioAnchor {
    pub bucket: u32,
    pub t_ms: u32,
    pub weight: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoAnchor {
    pub bucket: u32,
    pub t_ms: u32,
    pub hash64: u64,
    #[serde(default)]
    pub kind: u8,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAnchorProfile {
    pub version: u32,
    pub profile: String,
    pub duration_ms: Option<u32>,
    pub audio_anchors: Vec<AudioAnchor>,
    pub video_anchors: Vec<VideoAnchor>,
}

impl MediaAnchorProfile {
    pub fn is_empty(&self) -> bool {
        self.audio_anchors.is_empty() && self.video_anchors.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFingerprintWireSummary {
    pub profile: String,
    pub settings_hash: [u8; 32],
    pub duration_ms: Option<u32>,
    pub audio_summary: Option<Vec<u8>>,
    pub video_summary: Option<Vec<u8>>,
    pub audio_anchor_count: usize,
    pub video_anchor_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaWireAnchorDecodeError {
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidLength,
    TooManyAnchors(usize),
    UnsupportedVideoKind(u8),
    MismatchedVideoBucketKind { kind: u8, bucket_kind: u8 },
    InvalidTemporalVideoBucket { expected: u32, actual: u32 },
}

impl fmt::Display for MediaWireAnchorDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(formatter, "invalid media wire anchor block magic"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported media wire anchor block version {version}"
                )
            }
            Self::InvalidLength => write!(formatter, "invalid media wire anchor block length"),
            Self::TooManyAnchors(count) => {
                write!(
                    formatter,
                    "media wire anchor block has too many anchors ({count})"
                )
            }
            Self::UnsupportedVideoKind(kind) => {
                write!(formatter, "unsupported media v3 video landmark kind {kind}")
            }
            Self::MismatchedVideoBucketKind { kind, bucket_kind } => {
                write!(
                    formatter,
                    "media v3 video landmark kind {kind} does not match bucket kind {bucket_kind}"
                )
            }
            Self::InvalidTemporalVideoBucket { expected, actual } => {
                write!(
                    formatter,
                    "media v3 temporal video bucket {actual} does not match expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for MediaWireAnchorDecodeError {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaFingerprintBlobV3 {
    pub duration_ms: Option<u64>,
    pub audio_landmarks: Vec<AudioLandmarkV3>,
    pub video_landmarks: Vec<VideoLandmarkV3>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaFingerprintBlobV3DecodeError {
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidLength,
    TooManyLandmarks(usize),
    InvalidSection(u8),
    NonMonotonicTime,
    UnsupportedVideoKind(u8),
    MismatchedVideoBucketKind { kind: u8, bucket_kind: u8 },
    InvalidTemporalVideoBucket { expected: u32, actual: u32 },
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, MediaWireAnchorDecodeError> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or(MediaWireAnchorDecodeError::InvalidLength)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, MediaWireAnchorDecodeError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or(MediaWireAnchorDecodeError::InvalidLength)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Result<u64, MediaWireAnchorDecodeError> {
    let slice = bytes
        .get(offset..offset + 8)
        .ok_or(MediaWireAnchorDecodeError::InvalidLength)?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

impl fmt::Display for MediaFingerprintBlobV3DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(formatter, "invalid media fingerprint v3 blob magic"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported media fingerprint v3 blob version {version}"
                )
            }
            Self::InvalidLength => write!(formatter, "invalid media fingerprint v3 blob length"),
            Self::TooManyLandmarks(count) => write!(
                formatter,
                "media fingerprint v3 blob has too many landmarks ({count})"
            ),
            Self::InvalidSection(section) => {
                write!(
                    formatter,
                    "invalid media fingerprint v3 blob section {section}"
                )
            }
            Self::NonMonotonicTime => write!(
                formatter,
                "media fingerprint v3 blob timestamps are not monotonic"
            ),
            Self::UnsupportedVideoKind(kind) => {
                write!(
                    formatter,
                    "unsupported media fingerprint v3 video landmark kind {kind}"
                )
            }
            Self::MismatchedVideoBucketKind { kind, bucket_kind } => {
                write!(
                    formatter,
                    "media fingerprint v3 video kind {kind} does not match bucket kind {bucket_kind}"
                )
            }
            Self::InvalidTemporalVideoBucket { expected, actual } => {
                write!(
                    formatter,
                    "media fingerprint v3 temporal video bucket {actual} does not match expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for MediaFingerprintBlobV3DecodeError {}

const AUDIO_SUMMARY_MAGIC: &[u8; 4] = b"SAU2";
const VIDEO_SUMMARY_MAGIC: &[u8; 4] = b"SVI3";
const SUMMARY_FORMAT_VERSION: u16 = 1;
const FINGERPRINT_BLOB_V3_MAGIC: &[u8; 4] = b"SMM3";
const FINGERPRINT_BLOB_V3_FORMAT_VERSION: u16 = 1;
const FINGERPRINT_BLOB_V3_SECTION_AUDIO: u8 = 1;
const FINGERPRINT_BLOB_V3_SECTION_VIDEO: u8 = 2;

pub fn media_fingerprint_wire_summary_from_record(
    record: &MediaFingerprintRecord,
) -> MediaFingerprintWireSummary {
    let audio_anchors = audio_anchors_from_record(record);
    let video_anchors = video_anchors_from_record(record);
    MediaFingerprintWireSummary {
        profile: record.extraction_settings.profile.label().to_owned(),
        settings_hash: media_extraction_settings_hash(&record.extraction_settings),
        duration_ms: record.duration_seconds.and_then(duration_seconds_to_millis),
        audio_summary: (!audio_anchors.is_empty())
            .then(|| encode_wire_audio_anchor_summary(&audio_anchors)),
        video_summary: (!video_anchors.is_empty())
            .then(|| encode_wire_video_anchor_summary(&video_anchors)),
        audio_anchor_count: audio_anchors.len(),
        video_anchor_count: video_anchors.len(),
    }
}

pub fn media_anchor_profile_from_record(record: &MediaFingerprintRecord) -> MediaAnchorProfile {
    MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: record.extraction_settings.profile.label().to_owned(),
        duration_ms: record.duration_seconds.and_then(duration_seconds_to_millis),
        audio_anchors: audio_anchors_from_record(record),
        video_anchors: video_anchors_from_record(record),
    }
}

pub fn media_anchor_profile_from_wire_summaries(
    profile: impl Into<String>,
    duration_ms: Option<u32>,
    audio_summary: Option<&[u8]>,
    video_summary: Option<&[u8]>,
) -> Result<MediaAnchorProfile, MediaWireAnchorDecodeError> {
    Ok(MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: profile.into(),
        duration_ms,
        audio_anchors: audio_summary
            .map(decode_wire_audio_anchor_summary)
            .transpose()?
            .unwrap_or_default(),
        video_anchors: video_summary
            .map(decode_wire_video_anchor_summary)
            .transpose()?
            .unwrap_or_default(),
    })
}

pub fn media_fingerprint_blob_v3_from_record(
    record: &MediaFingerprintRecord,
) -> MediaFingerprintBlobV3 {
    MediaFingerprintBlobV3 {
        duration_ms: record
            .duration_seconds
            .and_then(duration_seconds_to_millis)
            .map(u64::from),
        audio_landmarks: audio_landmarks_v3_from_record(record),
        video_landmarks: video_landmarks_v3_from_record(record),
    }
}

pub fn media_fingerprint_record_apply_blob_v3(
    record: &mut MediaFingerprintRecord,
    blob: MediaFingerprintBlobV3,
) {
    record.duration_seconds = blob
        .duration_ms
        .map(|duration_ms| duration_ms as f64 / 1000.0);
    record.audio_anchors = blob
        .audio_landmarks
        .into_iter()
        .map(|landmark| AudioAnchor {
            bucket: landmark.hash,
            t_ms: landmark.t_ms,
            weight: u16::from(landmark.weight.max(1)),
        })
        .collect();
    record.video_anchors = blob
        .video_landmarks
        .into_iter()
        .map(|landmark| VideoAnchor {
            bucket: landmark.bucket,
            t_ms: landmark.t_ms,
            hash64: landmark.hash64,
            kind: landmark.kind,
            weight: u16::from(landmark.weight.max(1)),
        })
        .collect();
}

pub fn encode_media_fingerprint_blob_v3(blob: &MediaFingerprintBlobV3) -> Vec<u8> {
    let mut audio = blob.audio_landmarks.clone();
    audio.sort_by_key(|landmark| (landmark.t_ms, landmark.hash, landmark.weight));
    audio.truncate(MAX_V3_LANDMARKS);
    let mut video = blob.video_landmarks.clone();
    video.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.bucket,
            landmark.hash64,
            landmark.kind,
            landmark.weight,
        )
    });
    video.truncate(MAX_V3_LANDMARKS);

    let section_count = u8::from(!audio.is_empty()) + u8::from(!video.is_empty());
    let mut bytes = Vec::with_capacity(16 + audio.len() * 7 + video.len() * 18);
    bytes.extend_from_slice(FINGERPRINT_BLOB_V3_MAGIC);
    bytes.extend_from_slice(&FINGERPRINT_BLOB_V3_FORMAT_VERSION.to_le_bytes());
    encode_varint(blob.duration_ms.unwrap_or(u64::MAX), &mut bytes);
    bytes.push(section_count);
    if !audio.is_empty() {
        bytes.push(FINGERPRINT_BLOB_V3_SECTION_AUDIO);
        encode_varint(audio.len() as u64, &mut bytes);
        let mut previous_t_ms = 0u32;
        for landmark in audio {
            encode_varint(
                u64::from(landmark.t_ms.saturating_sub(previous_t_ms)),
                &mut bytes,
            );
            previous_t_ms = landmark.t_ms;
            bytes.extend_from_slice(&landmark.hash.to_le_bytes());
            bytes.push(landmark.weight);
        }
    }
    if !video.is_empty() {
        bytes.push(FINGERPRINT_BLOB_V3_SECTION_VIDEO);
        encode_varint(video.len() as u64, &mut bytes);
        let mut previous_t_ms = 0u32;
        for landmark in video {
            encode_varint(
                u64::from(landmark.t_ms.saturating_sub(previous_t_ms)),
                &mut bytes,
            );
            previous_t_ms = landmark.t_ms;
            bytes.extend_from_slice(&landmark.bucket.to_le_bytes());
            bytes.extend_from_slice(&landmark.hash64.to_le_bytes());
            bytes.push(landmark.kind);
            bytes.push(landmark.weight);
        }
    }
    bytes
}

pub fn decode_media_fingerprint_blob_v3(
    bytes: &[u8],
) -> Result<MediaFingerprintBlobV3, MediaFingerprintBlobV3DecodeError> {
    if bytes.len() < 7 {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
    }
    if &bytes[0..4] != FINGERPRINT_BLOB_V3_MAGIC {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != FINGERPRINT_BLOB_V3_FORMAT_VERSION {
        return Err(MediaFingerprintBlobV3DecodeError::UnsupportedVersion(
            version,
        ));
    }
    let mut cursor = 6;
    let encoded_duration = decode_varint(bytes, &mut cursor)?;
    let duration_ms = (encoded_duration != u64::MAX).then_some(encoded_duration);
    let Some(section_count) = bytes.get(cursor).copied() else {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
    };
    cursor += 1;
    let mut blob = MediaFingerprintBlobV3 {
        duration_ms,
        audio_landmarks: Vec::new(),
        video_landmarks: Vec::new(),
    };
    for _ in 0..section_count {
        let Some(section) = bytes.get(cursor).copied() else {
            return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
        };
        cursor += 1;
        let count = decode_varint(bytes, &mut cursor)? as usize;
        if count > MAX_V3_LANDMARKS {
            return Err(MediaFingerprintBlobV3DecodeError::TooManyLandmarks(count));
        }
        match section {
            FINGERPRINT_BLOB_V3_SECTION_AUDIO => {
                let mut t_ms = 0u32;
                let mut landmarks = Vec::with_capacity(count);
                for _ in 0..count {
                    let delta = decode_varint(bytes, &mut cursor)?;
                    let delta = u32::try_from(delta)
                        .map_err(|_| MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    t_ms = t_ms
                        .checked_add(delta)
                        .ok_or(MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    if cursor + 5 > bytes.len() {
                        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
                    }
                    let hash = u32::from_le_bytes([
                        bytes[cursor],
                        bytes[cursor + 1],
                        bytes[cursor + 2],
                        bytes[cursor + 3],
                    ]);
                    cursor += 4;
                    let weight = bytes[cursor];
                    cursor += 1;
                    landmarks.push(AudioLandmarkV3 { hash, t_ms, weight });
                }
                blob.audio_landmarks = landmarks;
            }
            FINGERPRINT_BLOB_V3_SECTION_VIDEO => {
                let mut t_ms = 0u32;
                let mut landmarks = Vec::with_capacity(count);
                for _ in 0..count {
                    let delta = decode_varint(bytes, &mut cursor)?;
                    let delta = u32::try_from(delta)
                        .map_err(|_| MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    t_ms = t_ms
                        .checked_add(delta)
                        .ok_or(MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    if cursor + 14 > bytes.len() {
                        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
                    }
                    let bucket = u32::from_le_bytes([
                        bytes[cursor],
                        bytes[cursor + 1],
                        bytes[cursor + 2],
                        bytes[cursor + 3],
                    ]);
                    cursor += 4;
                    let hash64 = u64::from_le_bytes([
                        bytes[cursor],
                        bytes[cursor + 1],
                        bytes[cursor + 2],
                        bytes[cursor + 3],
                        bytes[cursor + 4],
                        bytes[cursor + 5],
                        bytes[cursor + 6],
                        bytes[cursor + 7],
                    ]);
                    cursor += 8;
                    let kind = bytes[cursor];
                    cursor += 1;
                    let weight = bytes[cursor];
                    cursor += 1;
                    if !v3_video_kind_is_supported(kind) {
                        return Err(MediaFingerprintBlobV3DecodeError::UnsupportedVideoKind(
                            kind,
                        ));
                    }
                    let Some(bucket_kind) = v3_video_kind_from_bucket(bucket) else {
                        return Err(MediaFingerprintBlobV3DecodeError::UnsupportedVideoKind(
                            (bucket >> V3_VIDEO_BUCKET_KIND_SHIFT) as u8,
                        ));
                    };
                    if bucket_kind != kind {
                        return Err(
                            MediaFingerprintBlobV3DecodeError::MismatchedVideoBucketKind {
                                kind,
                                bucket_kind,
                            },
                        );
                    }
                    if kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE {
                        let expected = v3_video_bucket_for_kind(kind, anchor_bucket(hash64));
                        if bucket != expected {
                            return Err(
                                MediaFingerprintBlobV3DecodeError::InvalidTemporalVideoBucket {
                                    expected,
                                    actual: bucket,
                                },
                            );
                        }
                    }
                    landmarks.push(VideoLandmarkV3 {
                        bucket,
                        hash64,
                        t_ms,
                        kind,
                        weight,
                    });
                }
                blob.video_landmarks = landmarks;
            }
            section => return Err(MediaFingerprintBlobV3DecodeError::InvalidSection(section)),
        }
    }
    if cursor != bytes.len() {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
    }
    Ok(blob)
}

fn encode_varint(mut value: u64, bytes: &mut Vec<u8>) {
    while value >= 0x80 {
        bytes.push((value as u8) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn decode_varint(
    bytes: &[u8],
    cursor: &mut usize,
) -> Result<u64, MediaFingerprintBlobV3DecodeError> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let Some(byte) = bytes.get(*cursor).copied() else {
            return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
        };
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= 64 {
            return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
        }
    }
}

pub fn audio_anchors_from_record(record: &MediaFingerprintRecord) -> Vec<AudioAnchor> {
    if !record.audio_anchors.is_empty() {
        return record.audio_anchors.clone();
    }
    Vec::new()
}

pub fn video_anchors_from_record(record: &MediaFingerprintRecord) -> Vec<VideoAnchor> {
    if !record.video_anchors.is_empty() {
        return record.video_anchors.clone();
    }
    if matches!(
        record.extraction_settings.profile,
        MediaFingerprintProfile::CombinedV3
    ) && let Some(video) = &record.video
        && !video.v3_landmarks.is_empty()
    {
        let mut anchors = video
            .v3_landmarks
            .iter()
            .map(|landmark| VideoAnchor {
                bucket: landmark.bucket,
                t_ms: landmark.t_ms,
                hash64: landmark.hash64,
                kind: landmark.kind,
                weight: u16::from(landmark.weight.max(1)),
            })
            .collect::<Vec<_>>();
        return bounded_time_distributed_video_anchors(
            &mut anchors,
            V3_VIDEO_VERIFY_LANDMARK_LIMIT,
        );
    }
    let limit = if matches!(
        record.extraction_settings.profile,
        MediaFingerprintProfile::CombinedV3
    ) {
        V3_VIDEO_VERIFY_LANDMARK_LIMIT
    } else {
        0
    };
    record
        .video
        .as_ref()
        .map(|video| video_anchors_from_fingerprint(video, limit))
        .unwrap_or_default()
}

pub fn audio_landmarks_v3_from_record(record: &MediaFingerprintRecord) -> Vec<AudioLandmarkV3> {
    let mut landmarks = audio_anchors_from_record(record)
        .into_iter()
        .map(|anchor| AudioLandmarkV3 {
            hash: anchor.bucket,
            t_ms: anchor.t_ms,
            weight: anchor.weight.min(u16::from(u8::MAX)).max(1) as u8,
        })
        .collect::<Vec<_>>();
    bounded_time_distributed_audio_landmarks_v3(&mut landmarks, V3_AUDIO_VERIFY_LANDMARK_LIMIT)
}

pub fn video_landmarks_v3_from_record(record: &MediaFingerprintRecord) -> Vec<VideoLandmarkV3> {
    if let Some(video) = &record.video
        && !video.v3_landmarks.is_empty()
    {
        let mut landmarks = video.v3_landmarks.clone();
        return bounded_time_distributed_video_landmarks_v3(
            &mut landmarks,
            V3_VIDEO_VERIFY_LANDMARK_LIMIT,
        );
    }
    let mut landmarks = video_anchors_from_record(record)
        .into_iter()
        .map(|anchor| VideoLandmarkV3 {
            bucket: anchor.bucket,
            hash64: anchor.hash64,
            t_ms: anchor.t_ms,
            kind: anchor.kind,
            weight: anchor.weight.min(u16::from(u8::MAX)).max(1) as u8,
        })
        .collect::<Vec<_>>();
    bounded_time_distributed_video_landmarks_v3(&mut landmarks, V3_VIDEO_VERIFY_LANDMARK_LIMIT)
}

pub fn audio_index_landmarks_v3_from_record(
    record: &MediaFingerprintRecord,
) -> Vec<AudioLandmarkV3> {
    let mut landmarks = audio_landmarks_v3_from_record(record);
    bounded_time_distributed_audio_landmarks_v3(&mut landmarks, V3_AUDIO_INDEX_LANDMARK_LIMIT)
}

pub fn video_index_landmarks_v3_from_record(
    record: &MediaFingerprintRecord,
) -> Vec<VideoLandmarkV3> {
    let mut landmarks = video_landmarks_v3_from_record(record);
    bounded_time_distributed_video_landmarks_v3(&mut landmarks, V3_VIDEO_INDEX_LANDMARK_LIMIT)
}

pub fn video_anchors_from_fingerprint(
    video: &VideoFingerprint,
    max_anchors: usize,
) -> Vec<VideoAnchor> {
    if max_anchors == 0 {
        return Vec::new();
    }
    let mut anchors = video
        .frames
        .iter()
        .flat_map(|frame| {
            let t_ms = frame.timestamp_millis.min(u64::from(u32::MAX)) as u32;
            video_lsh_buckets(frame.hash)
                .into_iter()
                .map(move |bucket| VideoAnchor {
                    bucket,
                    t_ms,
                    hash64: frame.hash,
                    kind: V3_VIDEO_KIND_LUMA_FRAME,
                    weight: 1,
                })
        })
        .collect::<Vec<_>>();
    bounded_time_distributed_video_anchors(&mut anchors, max_anchors)
}

pub fn encode_wire_audio_anchor_summary(anchors: &[AudioAnchor]) -> Vec<u8> {
    let mut sorted = anchors.to_vec();
    sorted.sort_by_key(|anchor| (anchor.t_ms, anchor.bucket, anchor.weight));
    let count = sorted.len().min(MAX_WIRE_ANCHORS);
    let mut bytes = Vec::with_capacity(8 + count * 10);
    bytes.extend_from_slice(AUDIO_SUMMARY_MAGIC);
    bytes.extend_from_slice(&SUMMARY_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(count as u16).to_le_bytes());
    let mut previous_t_ms = 0u32;
    for anchor in sorted.into_iter().take(count) {
        let delta_t_ms = anchor.t_ms.saturating_sub(previous_t_ms);
        previous_t_ms = anchor.t_ms;
        bytes.extend_from_slice(&delta_t_ms.to_le_bytes());
        bytes.extend_from_slice(&anchor.bucket.to_le_bytes());
        bytes.extend_from_slice(&anchor.weight.to_le_bytes());
    }
    bytes
}

pub fn decode_wire_audio_anchor_summary(
    bytes: &[u8],
) -> Result<Vec<AudioAnchor>, MediaWireAnchorDecodeError> {
    if bytes.len() < 8 {
        return Err(MediaWireAnchorDecodeError::InvalidLength);
    }
    if &bytes[0..4] != AUDIO_SUMMARY_MAGIC {
        return Err(MediaWireAnchorDecodeError::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != SUMMARY_FORMAT_VERSION {
        return Err(MediaWireAnchorDecodeError::UnsupportedVersion(version));
    }
    let count = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    if count > MAX_WIRE_ANCHORS {
        return Err(MediaWireAnchorDecodeError::TooManyAnchors(count));
    }
    let expected = 8 + count * 10;
    if bytes.len() != expected {
        return Err(MediaWireAnchorDecodeError::InvalidLength);
    }
    let mut anchors = Vec::with_capacity(count);
    let mut cursor = 8;
    let mut t_ms = 0u32;
    for _ in 0..count {
        let delta_t_ms = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let bucket = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let weight = read_u16_le(bytes, cursor)?;
        cursor += 2;
        t_ms = t_ms.saturating_add(delta_t_ms);
        anchors.push(AudioAnchor {
            bucket,
            t_ms,
            weight,
        });
    }
    Ok(anchors)
}

pub fn encode_wire_video_anchor_summary(anchors: &[VideoAnchor]) -> Vec<u8> {
    let mut sorted = anchors.to_vec();
    sorted.sort_by_key(|anchor| {
        (
            anchor.t_ms,
            anchor.bucket,
            anchor.hash64,
            anchor.kind,
            anchor.weight,
        )
    });
    let count = sorted.len().min(MAX_WIRE_ANCHORS);
    let mut bytes = Vec::with_capacity(8 + count * 19);
    bytes.extend_from_slice(VIDEO_SUMMARY_MAGIC);
    bytes.extend_from_slice(&SUMMARY_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(count as u16).to_le_bytes());
    let mut previous_t_ms = 0u32;
    for anchor in sorted.into_iter().take(count) {
        let delta_t_ms = anchor.t_ms.saturating_sub(previous_t_ms);
        previous_t_ms = anchor.t_ms;
        bytes.extend_from_slice(&delta_t_ms.to_le_bytes());
        bytes.extend_from_slice(&anchor.bucket.to_le_bytes());
        bytes.extend_from_slice(&anchor.hash64.to_le_bytes());
        bytes.push(anchor.kind);
        bytes.extend_from_slice(&anchor.weight.to_le_bytes());
    }
    bytes
}

pub fn decode_wire_video_anchor_summary(
    bytes: &[u8],
) -> Result<Vec<VideoAnchor>, MediaWireAnchorDecodeError> {
    if bytes.len() < 8 {
        return Err(MediaWireAnchorDecodeError::InvalidLength);
    }
    if &bytes[0..4] != VIDEO_SUMMARY_MAGIC {
        return Err(MediaWireAnchorDecodeError::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != SUMMARY_FORMAT_VERSION {
        return Err(MediaWireAnchorDecodeError::UnsupportedVersion(version));
    }
    let count = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    if count > MAX_WIRE_ANCHORS {
        return Err(MediaWireAnchorDecodeError::TooManyAnchors(count));
    }
    let expected = 8 + count * 19;
    if bytes.len() != expected {
        return Err(MediaWireAnchorDecodeError::InvalidLength);
    }
    let mut anchors = Vec::with_capacity(count);
    let mut cursor = 8;
    let mut t_ms = 0u32;
    for _ in 0..count {
        let delta_t_ms = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let bucket = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let hash64 = read_u64_le(bytes, cursor)?;
        cursor += 8;
        let kind = bytes[cursor];
        cursor += 1;
        let weight = read_u16_le(bytes, cursor)?;
        cursor += 2;
        if !v3_video_kind_is_supported(kind) {
            return Err(MediaWireAnchorDecodeError::UnsupportedVideoKind(kind));
        }
        let Some(bucket_kind) = v3_video_kind_from_bucket(bucket) else {
            return Err(MediaWireAnchorDecodeError::UnsupportedVideoKind(
                (bucket >> V3_VIDEO_BUCKET_KIND_SHIFT) as u8,
            ));
        };
        if bucket_kind != kind {
            return Err(MediaWireAnchorDecodeError::MismatchedVideoBucketKind {
                kind,
                bucket_kind,
            });
        }
        if kind == V3_VIDEO_KIND_TEMPORAL_SHINGLE {
            let expected = v3_video_bucket_for_kind(kind, anchor_bucket(hash64));
            if bucket != expected {
                return Err(MediaWireAnchorDecodeError::InvalidTemporalVideoBucket {
                    expected,
                    actual: bucket,
                });
            }
        }
        t_ms = t_ms.saturating_add(delta_t_ms);
        anchors.push(VideoAnchor {
            bucket,
            t_ms,
            hash64,
            kind,
            weight,
        });
    }
    Ok(anchors)
}
