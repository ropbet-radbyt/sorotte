use super::*;

#[test]
fn flush_autoplay_notifications_to_sink_dispatches_notifications() {
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
        autoplay_enabled: true,
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
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };
    let mut runtime = create_client_runtime(&config);
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready should apply");
    runtime
        .session_mut()
        .readiness_autoplay_config_mut()
        .auto_play_threshold = Some(2);

    runtime
        .run_disconnect(0.0)
        .expect("disconnect should pause local player");
    runtime.update_autoplay_check(true, true, false, false);
    runtime
        .tick_autoplay(true, true, false, false)
        .expect("autoplay tick should emit countdown notification");

    let mut captured = Vec::new();
    flush_autoplay_notifications_to_sink(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("notification sink dispatch should succeed");

    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].ready_user_count, 2);
    assert_eq!(captured[0].seconds_left, 3);
}

#[test]
fn autoplay_countdown_notification_message_localized_legacy_compatible_localizes_user_visible_message()
 {
    let notification = AutoplayCountdownNotification {
        ready_user_count: 2,
        seconds_left: 3,
    };

    assert_eq!(
        crate::autoplay_countdown_notification_message_localized_legacy_compatible(
            &notification,
            Some("fr"),
        ),
        "Compte a rebours autoplay : utilisateurs_prets=2 secondes_restantes=3"
    );
    assert_eq!(
        crate::autoplay_countdown_notification_message_localized_legacy_compatible(
            &notification,
            None,
        ),
        "autoplay countdown: ready_users=2 seconds_left=3"
    );
}

#[test]
fn player_playback_telemetry_update_message_formats_present_fields() {
    let update = PlayerPlaybackTelemetryUpdate::default()
        .with_paused(true)
        .with_position_seconds(12.5)
        .with_playback_rate(0.95);

    let message = player_playback_telemetry_update_message(&update)
        .expect("expected telemetry message for populated update");
    assert_eq!(
        message,
        "player telemetry: paused=true position=12.500 speed=0.950"
    );

    assert_eq!(
        player_playback_telemetry_update_message(&PlayerPlaybackTelemetryUpdate::default()),
        None
    );
}

#[test]
fn player_playback_telemetry_update_message_localized_legacy_compatible_localizes_prefix() {
    let update = PlayerPlaybackTelemetryUpdate::default()
        .with_paused(true)
        .with_position_seconds(12.5);

    let message = crate::player_playback_telemetry_update_message_localized_legacy_compatible(
        &update,
        Some("fr"),
    )
    .expect("expected telemetry message for populated update");
    assert_eq!(
        message,
        "Telemetrie du lecteur: paused=true position=12.500"
    );
}

#[test]
fn player_playback_drift_diagnostic_messages_localized_legacy_compatible_localize_labels() {
    let update = PlayerPlaybackTelemetryUpdate::default()
        .with_paused(true)
        .with_position_seconds(12.5);
    let room = syncplay_client_core::RoomPlaystateView {
        paused: Some(false),
        position: Some(10.0),
        ..syncplay_client_core::RoomPlaystateView::default()
    };

    let messages = crate::player_playback_drift_diagnostic_messages_localized_legacy_compatible(
        &update,
        Some(&room),
        Some("de"),
    );
    assert_eq!(messages.len(), 2);
    assert!(messages[0].starts_with("Player-Abweichung: Pause-Abweichung "));
    assert!(messages[1].starts_with("Player-Abweichung: Positionsabweichung "));
}
