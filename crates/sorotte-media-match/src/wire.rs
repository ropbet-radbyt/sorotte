use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    MEDIA_MATCH_ANCHOR_VERSION, MEDIA_MATCH_WIRE_MAX_BYTES, MEDIA_MATCH_WIRE_SCHEMA_V3,
    MediaAnchorProfile, MediaExtractionSettings, MediaFingerprintRecord, MediaMatchDecision,
    MediaMatchSettings, decide_media_match_anchors, encode_audio_anchor_summary,
    encode_video_anchor_summary, media_anchor_profile_from_record,
    media_anchor_profile_from_summaries, media_match_tier_rank,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchWireSignature {
    pub schema: String,
    pub profiles: Vec<MediaMatchWireProfile>,
}

impl Default for MediaMatchWireSignature {
    fn default() -> Self {
        Self {
            schema: MEDIA_MATCH_WIRE_SCHEMA_V3.to_owned(),
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchWireProfile {
    pub profile: String,
    pub algorithm_version: u32,
    pub duration_ms: Option<u32>,
    pub audio: Option<MediaMatchWireAnchorBlock>,
    pub video: Option<MediaMatchWireAnchorBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchWireAnchorBlock {
    pub algorithm: String,
    pub time_base_ms: u32,
    pub anchors: String,
}

pub fn media_match_wire_signature_from_records(
    records: &[MediaFingerprintRecord],
) -> MediaMatchWireSignature {
    let mut signature = MediaMatchWireSignature::default();
    for record in records {
        if let Some(profile) = media_match_wire_anchor_profile_from_record(record) {
            signature.profiles.push(profile);
        }
    }
    signature
}

pub fn media_match_wire_value_from_records(records: &[MediaFingerprintRecord]) -> Option<Value> {
    let signature = media_match_wire_signature_from_records(records);
    if signature.profiles.is_empty() {
        return None;
    }
    let value = serde_json::to_value(&signature).ok()?;
    let bytes = serde_json::to_vec(&value).ok()?;
    (bytes.len() <= MEDIA_MATCH_WIRE_MAX_BYTES).then_some(value)
}

pub fn media_match_wire_signature_from_value(
    value: &Value,
) -> Result<MediaMatchWireSignature, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("media match wire signature could not serialize: {error}"))?;
    if bytes.len() > MEDIA_MATCH_WIRE_MAX_BYTES {
        return Err("media match wire signature exceeds the payload limit".to_owned());
    }
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| "media match wire signature has no schema".to_owned())?;
    if schema != MEDIA_MATCH_WIRE_SCHEMA_V3 {
        return Err("media match wire signature schema is unsupported".to_owned());
    }
    let signature: MediaMatchWireSignature = serde_json::from_value(value.clone())
        .map_err(|error| format!("media match v3 wire signature is invalid: {error}"))?;
    if signature.profiles.is_empty() {
        return Err("media match wire signature has no profiles".to_owned());
    }
    for profile in &signature.profiles {
        media_anchor_profile_from_wire_profile(profile)?;
    }
    Ok(signature)
}

pub fn decide_media_match_against_wire_signature(
    query: &MediaFingerprintRecord,
    signature: &MediaMatchWireSignature,
    settings: &MediaMatchSettings,
) -> MediaMatchDecision {
    let query_profile = media_anchor_profile_from_record(query);
    let mut ranked = signature
        .profiles
        .iter()
        .filter_map(|profile| media_anchor_profile_from_wire_profile(profile).ok())
        .map(|candidate| decide_media_match_anchors(&query_profile, &candidate, settings))
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        media_match_tier_rank(right.tier).cmp(&media_match_tier_rank(left.tier))
    });
    ranked
        .into_iter()
        .next()
        .unwrap_or_else(|| MediaMatchDecision::unknown("no comparable media match wire profiles"))
}

fn media_match_wire_anchor_profile_from_record(
    record: &MediaFingerprintRecord,
) -> Option<MediaMatchWireProfile> {
    let anchor_profile = media_anchor_profile_from_record(record);
    media_match_wire_anchor_profile_from_anchor_profile(
        &anchor_profile,
        &record.extraction_settings.audio_algorithm,
        &record.extraction_settings.video_algorithm,
    )
}

pub fn media_match_wire_anchor_profile_from_anchor_profile(
    profile: &MediaAnchorProfile,
    audio_algorithm: &str,
    video_algorithm: &str,
) -> Option<MediaMatchWireProfile> {
    if profile.is_empty() {
        return None;
    }
    let audio_summary = (!profile.audio_anchors.is_empty())
        .then(|| encode_audio_anchor_summary(&profile.audio_anchors));
    let video_summary = (!profile.video_anchors.is_empty())
        .then(|| encode_video_anchor_summary(&profile.video_anchors));
    Some(MediaMatchWireProfile {
        profile: profile.profile.clone(),
        algorithm_version: profile.version,
        duration_ms: profile.duration_ms,
        audio: audio_summary.map(|summary| MediaMatchWireAnchorBlock {
            algorithm: audio_algorithm.to_owned(),
            time_base_ms: 1,
            anchors: base64::engine::general_purpose::STANDARD.encode(summary),
        }),
        video: video_summary.map(|summary| MediaMatchWireAnchorBlock {
            algorithm: video_algorithm.to_owned(),
            time_base_ms: 1,
            anchors: base64::engine::general_purpose::STANDARD.encode(summary),
        }),
    })
}

pub fn media_anchor_profile_from_wire_profile(
    profile: &MediaMatchWireProfile,
) -> Result<MediaAnchorProfile, String> {
    if profile.algorithm_version != MEDIA_MATCH_ANCHOR_VERSION {
        return Err(format!(
            "media match v3 profile '{}' uses unsupported algorithm version {}",
            profile.profile, profile.algorithm_version
        ));
    }
    let expected_settings = media_extraction_settings_for_profile_label(&profile.profile)
        .ok_or_else(|| format!("media match v3 profile '{}' is unknown", profile.profile))?;
    if let Some(block) = profile.audio.as_ref() {
        validate_wire_anchor_block(
            "audio",
            block,
            &expected_settings.audio_algorithm,
            profile.profile.as_str(),
        )?;
    }
    if let Some(block) = profile.video.as_ref() {
        validate_wire_anchor_block(
            "video",
            block,
            &expected_settings.video_algorithm,
            profile.profile.as_str(),
        )?;
    }
    let audio_summary = profile
        .audio
        .as_ref()
        .map(|block| {
            base64::engine::general_purpose::STANDARD
                .decode(block.anchors.as_bytes())
                .map_err(|error| format!("media match v3 audio anchors are not base64: {error}"))
        })
        .transpose()?;
    let video_summary = profile
        .video
        .as_ref()
        .map(|block| {
            base64::engine::general_purpose::STANDARD
                .decode(block.anchors.as_bytes())
                .map_err(|error| format!("media match v3 video anchors are not base64: {error}"))
        })
        .transpose()?;
    media_anchor_profile_from_summaries(
        profile.profile.clone(),
        profile.duration_ms,
        audio_summary.as_deref(),
        video_summary.as_deref(),
    )
    .map_err(|error| format!("media match v3 anchors could not decode: {error}"))
}

fn media_extraction_settings_for_profile_label(label: &str) -> Option<MediaExtractionSettings> {
    match label {
        "audio-constellation-v3" => Some(MediaExtractionSettings::audio_constellation_v3()),
        "combined-v3" => Some(MediaExtractionSettings::combined_v3()),
        _ => None,
    }
}

fn validate_wire_anchor_block(
    modality: &str,
    block: &MediaMatchWireAnchorBlock,
    expected_algorithm: &str,
    profile_label: &str,
) -> Result<(), String> {
    if block.algorithm != expected_algorithm {
        return Err(format!(
            "media match v3 {modality} algorithm '{}' is unsupported for profile '{profile_label}'",
            block.algorithm
        ));
    }
    if block.time_base_ms != 1 {
        return Err(format!(
            "media match v3 {modality} time base {}ms is unsupported",
            block.time_base_ms
        ));
    }
    Ok(())
}
