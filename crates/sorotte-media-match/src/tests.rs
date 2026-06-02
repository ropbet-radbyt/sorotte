use crate::{
    AudioAnchor, MatchClassV3, MediaFileIdentity, MediaFingerprintRecord, MediaMatchAutoplayPolicy,
    MediaMatchSettings, MediaMatchTier, decide_media_match, settings::MediaExtractionSettings,
};

#[test]
fn fixed_sampled_fast_is_the_only_normal_settings_path() {
    let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();

    assert!(settings.sampled_audio_policy.is_production_compatible());
    assert_eq!(settings.sampled_audio_policy.window_count, 3);
    assert_eq!(settings.sampled_audio_policy.window_seconds, 20);
    assert_eq!(settings.sampled_audio_policy.landmark_limit, 384);
}

#[test]
fn exact_same_path_can_autoplay_without_fingerprint_strength() {
    let record = record("same.mkv", 0);
    let settings = autoplay_settings();
    let decision = decide_media_match(&record, &record, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Exact);
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutStrong)
    );
    assert!(decision.same_media_for_autoplay(&settings));
}

#[test]
fn sampled_audio_match_is_probable_and_not_autoplay_eligible() {
    let query = record("query.mkv", 0);
    let candidate = record("candidate.mkv", 400);
    let settings = autoplay_settings();
    let decision = decide_media_match(&query, &candidate, &settings);

    assert_eq!(decision.tier, MediaMatchTier::Probable);
    assert_eq!(
        decision.evidence.v3_class,
        Some(MatchClassV3::SameCutProbable)
    );
    assert!(!decision.same_media_for_autoplay(&settings));
}

#[test]
fn unrelated_sampled_audio_rejects() {
    let query = record("query.mkv", 0);
    let mut candidate = record("candidate.mkv", 0);
    for anchor in &mut candidate.audio_anchors {
        anchor.bucket += 10_000;
    }
    let decision = decide_media_match(&query, &candidate, &autoplay_settings());

    assert_eq!(decision.tier, MediaMatchTier::Reject);
}

fn record(path: &str, offset_ms: u32) -> MediaFingerprintRecord {
    MediaFingerprintRecord {
        identity: MediaFileIdentity {
            normalized_path: path.to_owned(),
            modified_unix_millis: 1,
            size_bytes: 100,
        },
        algorithm_version: crate::MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings: MediaExtractionSettings::sampled_fast_audio_index_v3(),
        duration_seconds: Some(24.0 * 60.0),
        container_fingerprint: format!("fingerprint-{path}"),
        audio_anchors: (0..48)
            .map(|index| AudioAnchor {
                bucket: 100 + (index % 12),
                t_ms: index * 1_000 + offset_ms,
                weight: 10,
            })
            .collect(),
        audio_error: None,
    }
}

fn autoplay_settings() -> MediaMatchSettings {
    MediaMatchSettings {
        fingerprinting_enabled: true,
        autoplay_policy: MediaMatchAutoplayPolicy::AllowStrongSameMedia,
        ..MediaMatchSettings::default()
    }
}
