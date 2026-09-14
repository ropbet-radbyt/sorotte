use super::*;
use crate::sorotte_ini::{
    parse_sorotte_ini_stored_client_settings, upsert_sorotte_ini_stored_client_settings,
};

#[test]
fn sparse_and_imported_settings_keep_standard_and_existing_cache_defaults() {
    for settings in [
        StoredClientSettings::default(),
        parse_sorotte_ini_stored_client_settings(
            "[client_settings]\nhost = sync.example\nroom = lounge\n",
        ),
    ] {
        assert_eq!(
            detect_synchronization_preset(&settings),
            Some(SynchronizationPreset::Standard)
        );
        let config = ClientConfig::try_from_stored(&settings).unwrap();
        assert_eq!(config.playback.streaming.buffering.target.get(), 5.0);
        assert_eq!(config.playback.streaming.buffering.read_ahead.get(), 30.0);
        assert_eq!(
            config.playback.streaming.buffering.memory_cache_mebibytes,
            150
        );
        assert!(!config.playback.streaming.buffering.disk_cache_enabled);
    }
}

#[test]
fn presets_round_trip_and_preserve_all_unowned_overrides() {
    let baseline = StoredClientSettings {
        host: Some("sync.example".into()),
        room: Some("lounge".into()),
        server_password: Some("secret".into()),
        streaming_buffer_target_seconds: Some(8.0),
        streaming_read_ahead_seconds: Some(45.0),
        streaming_memory_cache_mebibytes: Some(256),
        streaming_disk_cache_enabled: Some(true),
        streaming_quality_preset: Some("720p".into()),
        streaming_recovery_policy: Some("preserve-content".into()),
        rewind_threshold_seconds: Some(9.0),
        ready_at_start: Some(false),
        ..StoredClientSettings::default()
    };
    for preset in SynchronizationPreset::ALL {
        let mut settings = baseline.clone();
        preset.apply_to(&mut settings);
        let once = settings.clone();
        preset.apply_to(&mut settings);
        assert_eq!(settings, once);
        assert_eq!(detect_synchronization_preset(&settings), Some(preset));
        let original = ClientConfig::try_from_stored(&baseline).unwrap();
        let resolved = ClientConfig::try_from_stored(&settings).unwrap();
        assert_eq!(
            resolved.playback.streaming.buffering,
            original.playback.streaming.buffering
        );
        assert_eq!(
            resolved.playback.streaming.recovery,
            original.playback.streaming.recovery
        );
        assert_eq!(resolved.synchronization, original.synchronization);
        let text = upsert_sorotte_ini_stored_client_settings(
            "# keep\n[extra]\nunknown = yes\n",
            &settings,
        );
        assert!(text.contains("unknown = yes"));
        assert_eq!(parse_sorotte_ini_stored_client_settings(&text), settings);
        // Removing the seven documented overrides must recover the entire input.
        let mut unowned = settings.clone();
        unowned.streaming_start_policy = None;
        unowned.streaming_start_quorum_percent = None;
        unowned.streaming_start_timeout_seconds = None;
        unowned.streaming_start_timeout_action = None;
        unowned.streaming_room_buffering_policy = None;
        unowned.streaming_room_quorum_percent = None;
        unowned.streaming_room_max_pause_seconds = None;
        assert_eq!(unowned, baseline);
        assert_eq!(settings.server_password, baseline.server_password);
        assert_eq!(settings.ready_at_start, Some(false));
    }
}

#[test]
fn watch_together_requests_bounded_wait_and_a_controller_decision() {
    let mut settings = StoredClientSettings::default();
    SynchronizationPreset::WatchTogether.apply_to(&mut settings);
    let streaming = ClientConfig::try_from_stored(&settings)
        .unwrap()
        .playback
        .streaming;
    assert_eq!(
        streaming.start_synchronization.policy,
        StartSynchronizationPolicy::WaitForAllEligible
    );
    assert_eq!(streaming.start_synchronization.timeout.get(), 30.0);
    assert_eq!(
        streaming.start_synchronization.timeout_action,
        StartTimeoutAction::AskController
    );
    assert_eq!(
        streaming.room_buffering.policy,
        RoomBufferingPolicy::PauseEligible
    );
    assert_eq!(streaming.room_buffering.maximum_pause.get(), 30.0);
}

#[test]
fn each_invalid_owned_override_is_custom_and_reapplication_repairs_it() {
    let invalid = [
        "streamingStartPolicy = invalid",
        "streamingStartQuorumPercent = 0",
        "streamingStartTimeout = 0",
        "streamingStartTimeoutAction = invalid",
        "streamingRoomBufferingPolicy = invalid",
        "streamingRoomQuorumPercent = 0",
        "streamingRoomMaxPause = 0",
    ];
    for field in invalid {
        let mut settings =
            parse_sorotte_ini_stored_client_settings(&format!("[client_settings]\n{field}\n"));
        assert_eq!(detect_synchronization_preset(&settings), None, "{field}");
        SynchronizationPreset::Standard.apply_to(&mut settings);
        assert!(
            ClientConfig::resolve(&settings).issues.is_empty(),
            "{field}"
        );
        assert_eq!(
            detect_synchronization_preset(&settings),
            Some(SynchronizationPreset::Standard)
        );
    }
}

#[test]
fn owned_changes_are_custom_but_unrelated_validation_does_not_change_the_preset() {
    let mut settings = StoredClientSettings {
        streaming_quality_preset: Some("invalid".into()),
        ..StoredClientSettings::default()
    };
    assert_eq!(
        detect_synchronization_preset(&settings),
        Some(SynchronizationPreset::Standard)
    );
    settings.streaming_start_timeout_seconds = Some(16.0);
    assert_eq!(detect_synchronization_preset(&settings), None);
    SynchronizationPreset::WatchTogether.apply_to(&mut settings);
    assert_eq!(
        settings.streaming_quality_preset.as_deref(),
        Some("invalid")
    );
    assert_eq!(
        detect_synchronization_preset(&settings),
        Some(SynchronizationPreset::WatchTogether)
    );
    assert!(!ClientConfig::resolve(&settings).issues.is_empty());
}
