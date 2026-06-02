use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    MEDIA_MATCH_ANCHOR_VERSION, MEDIA_MATCH_V3_PROFILE_LABEL,
    audio_v3::{AudioLandmarkV3, bounded_time_distributed_audio_landmarks_v3_for_duration},
    identity::duration_seconds_to_millis,
    settings::media_extraction_settings_hash,
    tuning::V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
    types::MediaFingerprintRecord,
};

const MAX_WIRE_ANCHORS: usize = 1024;
const MAX_V3_LANDMARKS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioAnchor {
    pub bucket: u32,
    pub t_ms: u32,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAnchorProfile {
    pub version: u32,
    pub profile: String,
    pub duration_ms: Option<u32>,
    pub audio_anchors: Vec<AudioAnchor>,
}

impl MediaAnchorProfile {
    pub fn is_empty(&self) -> bool {
        self.audio_anchors.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFingerprintWireSummary {
    pub profile: String,
    pub settings_hash: [u8; 32],
    pub duration_ms: Option<u32>,
    pub audio_summary: Option<Vec<u8>>,
    pub audio_anchor_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaWireAnchorDecodeError {
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidLength,
    TooManyAnchors(usize),
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
        }
    }
}

impl std::error::Error for MediaWireAnchorDecodeError {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaFingerprintBlobV3 {
    pub duration_ms: Option<u64>,
    pub audio_landmarks: Vec<AudioLandmarkV3>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaFingerprintBlobV3DecodeError {
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidLength,
    TooManyLandmarks(usize),
    InvalidSection(u8),
    NonMonotonicTime,
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
        }
    }
}

impl std::error::Error for MediaFingerprintBlobV3DecodeError {}

const AUDIO_SUMMARY_MAGIC: &[u8; 4] = b"SAU2";
const SUMMARY_FORMAT_VERSION: u16 = 1;
const FINGERPRINT_BLOB_V3_MAGIC: &[u8; 4] = b"SMA3";
const FINGERPRINT_BLOB_V3_FORMAT_VERSION: u16 = 1;
const FINGERPRINT_BLOB_V3_SECTION_AUDIO: u8 = 1;

pub fn media_fingerprint_wire_summary_from_record(
    record: &MediaFingerprintRecord,
) -> MediaFingerprintWireSummary {
    let audio_anchors = audio_anchors_from_record(record);
    MediaFingerprintWireSummary {
        profile: MEDIA_MATCH_V3_PROFILE_LABEL.to_owned(),
        settings_hash: media_extraction_settings_hash(&record.extraction_settings),
        duration_ms: record.duration_seconds.and_then(duration_seconds_to_millis),
        audio_summary: (!audio_anchors.is_empty())
            .then(|| encode_wire_audio_anchor_summary(&audio_anchors)),
        audio_anchor_count: audio_anchors.len(),
    }
}

pub fn media_anchor_profile_from_record(record: &MediaFingerprintRecord) -> MediaAnchorProfile {
    MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: MEDIA_MATCH_V3_PROFILE_LABEL.to_owned(),
        duration_ms: record.duration_seconds.and_then(duration_seconds_to_millis),
        audio_anchors: audio_anchors_from_record(record),
    }
}

pub fn media_anchor_profile_from_wire_summaries(
    profile: impl Into<String>,
    duration_ms: Option<u32>,
    audio_summary: Option<&[u8]>,
) -> Result<MediaAnchorProfile, MediaWireAnchorDecodeError> {
    Ok(MediaAnchorProfile {
        version: MEDIA_MATCH_ANCHOR_VERSION,
        profile: profile.into(),
        duration_ms,
        audio_anchors: audio_summary
            .map(decode_wire_audio_anchor_summary)
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
}

pub fn encode_media_fingerprint_blob_v3(blob: &MediaFingerprintBlobV3) -> Vec<u8> {
    let mut audio = blob.audio_landmarks.clone();
    audio.sort_by_key(|landmark| (landmark.t_ms, landmark.hash, landmark.weight));
    audio.truncate(MAX_V3_LANDMARKS);

    let mut bytes = Vec::with_capacity(16 + audio.len() * 7);
    bytes.extend_from_slice(FINGERPRINT_BLOB_V3_MAGIC);
    bytes.extend_from_slice(&FINGERPRINT_BLOB_V3_FORMAT_VERSION.to_le_bytes());
    encode_varint(blob.duration_ms.unwrap_or(u64::MAX), &mut bytes);
    bytes.push(u8::from(!audio.is_empty()));
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
    bytes
}

pub fn decode_media_fingerprint_blob_v3(
    bytes: &[u8],
) -> Result<MediaFingerprintBlobV3, MediaFingerprintBlobV3DecodeError> {
    if bytes.len() < 7 || &bytes[0..4] != FINGERPRINT_BLOB_V3_MAGIC {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != FINGERPRINT_BLOB_V3_FORMAT_VERSION {
        return Err(MediaFingerprintBlobV3DecodeError::UnsupportedVersion(
            version,
        ));
    }
    let mut cursor = 6usize;
    let duration = decode_varint(bytes, &mut cursor)
        .ok_or(MediaFingerprintBlobV3DecodeError::InvalidLength)?;
    let section_count = *bytes
        .get(cursor)
        .ok_or(MediaFingerprintBlobV3DecodeError::InvalidLength)?;
    cursor += 1;

    let mut audio_landmarks = Vec::new();
    for _ in 0..section_count {
        let section = *bytes
            .get(cursor)
            .ok_or(MediaFingerprintBlobV3DecodeError::InvalidLength)?;
        cursor += 1;
        let count = decode_varint(bytes, &mut cursor)
            .ok_or(MediaFingerprintBlobV3DecodeError::InvalidLength)? as usize;
        if count > MAX_V3_LANDMARKS {
            return Err(MediaFingerprintBlobV3DecodeError::TooManyLandmarks(count));
        }
        match section {
            FINGERPRINT_BLOB_V3_SECTION_AUDIO => {
                let mut previous_t_ms = 0u32;
                for _ in 0..count {
                    let delta = decode_varint(bytes, &mut cursor)
                        .ok_or(MediaFingerprintBlobV3DecodeError::InvalidLength)?
                        as u32;
                    let t_ms = previous_t_ms
                        .checked_add(delta)
                        .ok_or(MediaFingerprintBlobV3DecodeError::NonMonotonicTime)?;
                    let hash = read_u32_le_blob(bytes, &mut cursor)?;
                    let weight = *bytes
                        .get(cursor)
                        .ok_or(MediaFingerprintBlobV3DecodeError::InvalidLength)?;
                    cursor += 1;
                    previous_t_ms = t_ms;
                    audio_landmarks.push(AudioLandmarkV3 { hash, t_ms, weight });
                }
            }
            other => return Err(MediaFingerprintBlobV3DecodeError::InvalidSection(other)),
        }
    }
    if cursor != bytes.len() {
        return Err(MediaFingerprintBlobV3DecodeError::InvalidLength);
    }
    Ok(MediaFingerprintBlobV3 {
        duration_ms: (duration != u64::MAX).then_some(duration),
        audio_landmarks,
    })
}

pub fn audio_anchors_from_record(record: &MediaFingerprintRecord) -> Vec<AudioAnchor> {
    if !record.audio_anchors.is_empty() {
        return record.audio_anchors.clone();
    }
    Vec::new()
}

pub fn audio_landmarks_v3_from_record(record: &MediaFingerprintRecord) -> Vec<AudioLandmarkV3> {
    record
        .audio_anchors
        .iter()
        .map(|anchor| AudioLandmarkV3 {
            hash: anchor.bucket,
            t_ms: anchor.t_ms,
            weight: anchor.weight.min(u16::from(u8::MAX)) as u8,
        })
        .collect()
}

pub fn audio_index_landmarks_v3_from_record(
    record: &MediaFingerprintRecord,
) -> Vec<AudioLandmarkV3> {
    let mut landmarks = audio_landmarks_v3_from_record(record);
    bounded_time_distributed_audio_landmarks_v3_for_duration(
        &mut landmarks,
        V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT,
        record.duration_seconds,
    )
}

pub fn encode_wire_audio_anchor_summary(anchors: &[AudioAnchor]) -> Vec<u8> {
    let mut sorted = anchors.to_vec();
    sorted.sort_by_key(|anchor| (anchor.t_ms, anchor.bucket, anchor.weight));
    sorted.truncate(MAX_WIRE_ANCHORS);

    let mut bytes = Vec::with_capacity(8 + sorted.len() * 7);
    bytes.extend_from_slice(AUDIO_SUMMARY_MAGIC);
    bytes.extend_from_slice(&SUMMARY_FORMAT_VERSION.to_le_bytes());
    encode_varint(sorted.len() as u64, &mut bytes);
    let mut previous_t_ms = 0u32;
    for anchor in sorted {
        encode_varint(
            u64::from(anchor.t_ms.saturating_sub(previous_t_ms)),
            &mut bytes,
        );
        previous_t_ms = anchor.t_ms;
        bytes.extend_from_slice(&anchor.bucket.to_le_bytes());
        bytes.extend_from_slice(&anchor.weight.to_le_bytes());
    }
    bytes
}

pub fn decode_wire_audio_anchor_summary(
    bytes: &[u8],
) -> Result<Vec<AudioAnchor>, MediaWireAnchorDecodeError> {
    if bytes.len() < 6 || &bytes[0..4] != AUDIO_SUMMARY_MAGIC {
        return Err(MediaWireAnchorDecodeError::InvalidMagic);
    }
    let version = read_u16_le(bytes, 4)?;
    if version != SUMMARY_FORMAT_VERSION {
        return Err(MediaWireAnchorDecodeError::UnsupportedVersion(version));
    }
    let mut cursor = 6usize;
    let count = decode_varint(bytes, &mut cursor)
        .ok_or(MediaWireAnchorDecodeError::InvalidLength)? as usize;
    if count > MAX_WIRE_ANCHORS {
        return Err(MediaWireAnchorDecodeError::TooManyAnchors(count));
    }
    let mut anchors = Vec::with_capacity(count);
    let mut previous_t_ms = 0u32;
    for _ in 0..count {
        let delta = decode_varint(bytes, &mut cursor)
            .ok_or(MediaWireAnchorDecodeError::InvalidLength)? as u32;
        let t_ms = previous_t_ms
            .checked_add(delta)
            .ok_or(MediaWireAnchorDecodeError::InvalidLength)?;
        let bucket = read_u32_le(bytes, cursor)?;
        cursor += 4;
        let weight = read_u16_le(bytes, cursor)?;
        cursor += 2;
        previous_t_ms = t_ms;
        anchors.push(AudioAnchor {
            bucket,
            t_ms,
            weight,
        });
    }
    if cursor != bytes.len() {
        return Err(MediaWireAnchorDecodeError::InvalidLength);
    }
    Ok(anchors)
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

fn read_u32_le_blob(
    bytes: &[u8],
    cursor: &mut usize,
) -> Result<u32, MediaFingerprintBlobV3DecodeError> {
    let slice = bytes
        .get(*cursor..*cursor + 4)
        .ok_or(MediaFingerprintBlobV3DecodeError::InvalidLength)?;
    *cursor += 4;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn encode_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn decode_varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_wire_summary_round_trips() {
        let anchors = vec![
            AudioAnchor {
                bucket: 42,
                t_ms: 1_000,
                weight: 3,
            },
            AudioAnchor {
                bucket: 99,
                t_ms: 2_500,
                weight: 7,
            },
        ];

        let encoded = encode_wire_audio_anchor_summary(&anchors);
        let decoded = decode_wire_audio_anchor_summary(&encoded).expect("summary decodes");

        assert_eq!(decoded, anchors);
    }

    #[test]
    fn audio_blob_round_trips() {
        let blob = MediaFingerprintBlobV3 {
            duration_ms: Some(123_456),
            audio_landmarks: vec![
                AudioLandmarkV3 {
                    hash: 1,
                    t_ms: 10,
                    weight: 2,
                },
                AudioLandmarkV3 {
                    hash: 2,
                    t_ms: 20,
                    weight: 3,
                },
            ],
        };

        let encoded = encode_media_fingerprint_blob_v3(&blob);
        let decoded = decode_media_fingerprint_blob_v3(&encoded).expect("blob decodes");

        assert_eq!(decoded, blob);
    }

    #[test]
    fn audio_index_landmarks_are_bounded() {
        let record = MediaFingerprintRecord {
            identity: crate::MediaFileIdentity::new("a.mkv", 1, 2),
            algorithm_version: crate::MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings: crate::MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(1500.0),
            container_fingerprint: "container".to_owned(),
            audio_anchors: (0..(V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT + 100))
                .map(|index| AudioAnchor {
                    bucket: index as u32,
                    t_ms: index as u32 * 250,
                    weight: 10,
                })
                .collect(),
            audio_error: None,
        };

        assert!(
            audio_index_landmarks_v3_from_record(&record).len()
                <= V3_AUDIO_SAMPLED_FAST_INDEX_LANDMARK_LIMIT
        );
    }
}
