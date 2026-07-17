use super::*;

#[test]
fn normalize_controlled_room_input_extracts_canonical_room_and_password() {
    let (room, password) =
        normalize_controlled_room_input("+room:ABCDEF123456:ab-123-456".to_owned());
    assert_eq!(room, "+room:ABCDEF123456");
    assert_eq!(password.as_deref(), Some("AB-123-456"));

    let (room, password) = normalize_controlled_room_input("room1".to_owned());
    assert_eq!(room, "room1");
    assert!(password.is_none());
}

#[test]
fn controlled_room_base_name_legacy_compatible_strips_managed_suffix() {
    assert_eq!(
        controlled_room_base_name_legacy_compatible("+base-room:ABCDEF123456"),
        "base-room"
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible("+room_name:ABCDEF12345_"),
        "room_name"
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible("room1"),
        "room1"
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible(" room1 "),
        " room1 "
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible("+room:SHORT"),
        "+room:SHORT"
    );
}

#[test]
fn generate_room_password_legacy_compatible_matches_expected_shape() {
    let password = generate_room_password_legacy_compatible();
    assert!(
        is_legacy_generated_room_password_shape(&password),
        "generated password should match legacy shape AA-999-999"
    );
}

#[test]
fn legacy_syncplay_ui_settings_from_stored_settings_uses_python_defaults_and_supported_overrides() {
    assert_eq!(
        legacy_syncplay_ui_settings_from_stored_settings(None),
        LegacySyncplayUiSettings::default()
    );

    let resolved =
        legacy_syncplay_ui_settings_from_stored_settings(Some(&StoredClientSettingsMvp {
            show_osd: Some(false),
            chat_input_enabled: Some(false),
            chat_input_font_family: Some("serif".to_owned()),
            chat_input_relative_font_size: Some(18),
            chat_input_font_weight: Some(50),
            chat_input_font_color: Some("#abcdef".to_owned()),
            chat_input_position: Some("Bottom".to_owned()),
            chat_output_enabled: Some(false),
            chat_output_font_family: Some("monospace".to_owned()),
            chat_output_relative_font_size: Some(30),
            chat_output_font_weight: Some(75),
            chat_output_mode: Some("Scrolling".to_owned()),
            chat_move_osd: Some(false),
            chat_max_lines: Some(9),
            chat_top_margin: Some(40),
            chat_left_margin: Some(35),
            chat_bottom_margin: Some(45),
            chat_osd_margin: Some(220),
            notification_timeout_seconds: Some(4),
            alert_timeout_seconds: Some(6),
            chat_timeout_seconds: Some(9),
            ..StoredClientSettingsMvp::default()
        }));

    assert_eq!(
        resolved,
        LegacySyncplayUiSettings {
            show_osd: false,
            chat_output_enabled: false,
            chat_input_enabled: false,
            chat_input_font_family: "serif".to_owned(),
            chat_input_relative_font_size: 18,
            chat_input_font_weight: 50,
            chat_input_font_color: "#abcdef".to_owned(),
            chat_input_position: "Bottom".to_owned(),
            chat_output_font_family: "monospace".to_owned(),
            chat_output_relative_font_size: 30,
            chat_output_font_weight: 75,
            chat_output_mode: "Scrolling".to_owned(),
            chat_move_osd: false,
            chat_max_lines: 9,
            chat_top_margin: 40,
            chat_left_margin: 35,
            chat_bottom_margin: 45,
            chat_osd_margin: 220,
            notification_timeout_ms: 4_000,
            alert_timeout_ms: 6_000,
            chat_timeout_ms: 9_000,
            ..LegacySyncplayUiSettings::default()
        }
    );
}

#[test]
fn create_client_runtime_with_managed_mpv_support_applies_legacy_syncplay_ui_settings() {
    let config = test_client_loop_config();
    let settings = StoredClientSettingsMvp {
        show_osd: Some(false),
        chat_output_enabled: Some(false),
        chat_input_enabled: Some(false),
        chat_input_position: Some("Bottom".to_owned()),
        chat_output_mode: Some("Scrolling".to_owned()),
        chat_move_osd: Some(false),
        chat_osd_margin: Some(180),
        notification_timeout_seconds: Some(2),
        alert_timeout_seconds: Some(4),
        chat_timeout_seconds: Some(8),
        ..StoredClientSettingsMvp::default()
    };

    let (runtime, _managed_guard) =
        create_client_runtime_with_managed_mpv_support(&config, None, Some(&settings))
            .expect("runtime creation should succeed");

    assert_eq!(
        runtime.player().legacy_syncplay_ui_settings(),
        &LegacySyncplayUiSettings {
            show_osd: false,
            chat_output_enabled: false,
            chat_input_enabled: false,
            chat_input_position: "Bottom".to_owned(),
            chat_output_mode: "Scrolling".to_owned(),
            chat_move_osd: false,
            chat_osd_margin: 180,
            notification_timeout_ms: 2_000,
            alert_timeout_ms: 4_000,
            chat_timeout_ms: 8_000,
            ..LegacySyncplayUiSettings::default()
        }
    );
}

#[test]
fn cli_runtime_retains_connected_mpv_when_optional_bridge_is_degraded() {
    let config = test_client_loop_config();
    let baseline = LegacySyncplayUiSettings {
        chat_move_osd: false,
        notification_timeout_ms: 3_000,
        ..LegacySyncplayUiSettings::default()
    };
    let player = MpvAdapter::with_unacknowledging_syncplayintf_test_ipc(baseline);
    let settings = StoredClientSettingsMvp {
        chat_move_osd: Some(false),
        notification_timeout_seconds: Some(9),
        ..StoredClientSettingsMvp::default()
    };

    let (mut runtime, bridge_health) =
        create_client_runtime_with_prepared_mpv_for_test(&config, Some(&settings), player)
            .expect("an optional bridge failure must not abort CLI runtime construction");

    let SorotteBridgeHealth::Degraded(failure) = bridge_health else {
        panic!("unacknowledged bridge settings should report degraded health");
    };
    assert!(
        !failure.reason.trim().is_empty(),
        "the CLI warning must include a useful bridge failure reason"
    );
    assert!(runtime.player().is_connected());
    assert_eq!(
        runtime
            .player()
            .legacy_syncplay_ui_settings()
            .notification_timeout_ms,
        9_000
    );

    runtime
        .with_player_io(|player| {
            player.open_file("C:/media/degraded-bridge.mkv")?;
            player.set_paused(false)?;
            player.set_position(12.5)?;
            let _ = player.take_playback_telemetry_update();
            Ok::<(), PlayerError>(())
        })
        .expect(
            "open, pause, seek, and telemetry polling must remain available while the bridge is degraded",
        );
    assert!(
        runtime.player().is_connected(),
        "telemetry polling must not disturb the healthy core mpv transport"
    );
}

#[test]
fn cli_production_finish_seam_retains_connected_player_on_script_load_degradation() {
    let config = test_client_loop_config();
    let player = MpvAdapter::with_unacknowledging_syncplayintf_test_ipc(LegacySyncplayUiSettings {
        chat_move_osd: false,
        notification_timeout_ms: 7_777,
        ..LegacySyncplayUiSettings::default()
    });
    let expected_health = SorotteBridgeHealth::Degraded(SorotteBridgeFailure {
        kind: SorotteBridgeFailureKind::ScriptLoad,
        reason: "injected bundled bridge script load failure".to_owned(),
    });

    let (mut runtime, health) = create_client_runtime_with_prepared_mpv_and_bridge_setup_for_test(
        &config,
        None,
        player,
        |_player, _settings| expected_health.clone(),
    )
    .expect("a ScriptLoad-only degradation must not abort production runtime construction");

    assert_eq!(health, expected_health);
    assert!(runtime.player().is_connected());
    assert_eq!(
        runtime
            .player()
            .legacy_syncplay_ui_settings()
            .notification_timeout_ms,
        7_777,
        "the connected player passed into the production finish seam must be retained"
    );
    runtime
        .with_player_io(|player| {
            player.open_file("C:/media/script-load-degraded.mkv")?;
            player.set_paused(true)
        })
        .expect("core playback must remain available after ScriptLoad degradation");
}

#[test]
fn cli_optional_osd_setup_failure_clears_bridge_readiness_and_player_chat_state() {
    let baseline = LegacySyncplayUiSettings {
        chat_move_osd: false,
        ..LegacySyncplayUiSettings::default()
    };
    let mut player = MpvAdapter::with_unacknowledging_syncplayintf_test_ipc(baseline);
    assert!(player.legacy_syncplayintf_options_ready());

    let health = apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible(
        &mut player,
        Some(&StoredClientSettingsMvp {
            chat_move_osd: Some(true),
            ..StoredClientSettingsMvp::default()
        }),
    );

    assert!(matches!(
        health,
        SorotteBridgeHealth::Degraded(ref failure)
            if failure.kind == SorotteBridgeFailureKind::IpcCommand
    ));
    assert_eq!(player.sorotte_bridge_health(), health);
    assert!(
        player.is_connected(),
        "the fake IPC transport remains healthy"
    );
    assert!(
        !player.legacy_syncplayintf_options_ready(),
        "degraded integration must stop player-originated chat polling immediately"
    );
}

#[test]
fn cli_runtime_rejects_transport_loss_during_optional_bridge_setup() {
    let config = test_client_loop_config();
    let baseline = LegacySyncplayUiSettings {
        chat_move_osd: false,
        ..LegacySyncplayUiSettings::default()
    };
    let player = MpvAdapter::with_unacknowledging_syncplayintf_test_ipc(baseline);

    let result = create_client_runtime_with_prepared_mpv_and_bridge_setup_for_test(
        &config,
        None,
        player,
        |player, _settings| {
            player.mark_test_ipc_unhealthy("test transport failed during bridge setup");
            player.mark_sorotte_bridge_degraded(
                SorotteBridgeFailureKind::IpcCommand,
                "optional bridge command failed after transport loss",
            )
        },
    );

    let Err(error) = result else {
        panic!("a previously healthy mpv transport becoming unavailable must remain fatal");
    };
    let message = error.to_string();
    assert!(message.contains("mpv JSON IPC became unavailable"));
    assert!(message.contains("optional bridge command failed after transport loss"));
}

#[test]
fn cli_can_materialize_embedded_bridge_without_an_executable_or_source_tree_resource() {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let cache_root = std::env::temp_dir().join(format!(
        "sorotte-cli-embedded-mpv-bridge-{}-{unique_suffix}",
        std::process::id()
    ));

    let first = sorotte_player_mpv::materialize_bundled_sorotte_bridge_in(&cache_root)
        .expect("the CLI dependency must materialize its embedded bridge");
    let second = sorotte_player_mpv::materialize_bundled_sorotte_bridge_in(&cache_root)
        .expect("materializing the same embedded bridge should be idempotent");

    assert_eq!(first, second);
    assert_eq!(
        first.file_name().and_then(|name| name.to_str()),
        Some("sorotte_syncplayintf.lua")
    );
    assert_eq!(
        first.parent().and_then(Path::parent),
        Some(cache_root.as_path()),
        "the resource should live under a content-address directory in the requested cache"
    );
    let content_hash = first
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .expect("the materialized bridge should have a content-address directory");
    assert_eq!(content_hash.len(), 64);
    assert!(
        content_hash
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    let script = std::fs::read_to_string(&first)
        .expect("the materialized embedded bridge should be readable");
    assert!(script.contains("sorotte-syncplayintf-v1"));
    assert!(script.contains("sorotte_syncplayintf"));

    let _ = std::fs::remove_dir_all(cache_root);
}

#[test]
fn create_client_runtime_applies_autoplay_require_same_filenames_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: true,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(
        runtime
            .session()
            .readiness_autoplay_config()
            .autoplay_require_same_filenames
    );
}

#[test]
fn create_client_runtime_applies_duration_comparison_override_flags() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: Some(false),
        different_duration_threshold_seconds_override: Some(1.0),
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    let readiness_config = runtime.session().readiness_autoplay_config();
    assert!(!readiness_config.show_duration_notification);
    assert_eq!(readiness_config.different_duration_threshold_seconds, 1.0);
}

#[test]
fn create_client_runtime_applies_show_same_room_osd_override_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: Some(false),
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(!runtime.session().behavior_config().show_same_room_osd);
}

#[test]
fn create_client_runtime_applies_show_noncontroller_osd_override_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: Some(true),
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(runtime.session().behavior_config().show_noncontroller_osd);
}

#[test]
fn create_client_runtime_applies_show_osd_warnings_override_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: Some(false),
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(!runtime.session().behavior_config().show_osd_warnings);
}

#[test]
fn create_client_runtime_applies_show_different_room_osd_override_flag() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: false,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: Some(true),
        controlled_room_password_override: None,
    };

    let runtime = create_client_runtime(&config);
    assert!(runtime.session().behavior_config().show_different_room_osd);
}
