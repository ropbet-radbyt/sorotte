use super::*;

#[test]
fn flush_controller_auth_notifications_to_sink_dispatches_attempt_notification() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "+room:ABCDEF123456".to_owned(),
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
        show_different_room_osd_override: None,
        controlled_room_password_override: Some("AB-123-456".to_owned()),
    };
    let mut runtime = create_client_runtime(&config);
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255","features":{"managedRooms":true}}}"#,
            )
            .expect("hello should apply");
    runtime
        .run_controller_reidentify_if_needed()
        .expect("controller reidentify should dispatch");

    let mut captured = Vec::new();
    flush_controller_auth_notifications_to_sink(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("controller auth notifications should dispatch");
    flush_controller_auth_notifications_to_sink(
        &mut runtime,
        &mut ignore_controller_auth_notification,
    )
    .expect("drained controller auth notification queue should be empty");

    assert_eq!(
        captured,
        vec![ControllerAuthTransitionNotification::Attempting {
            room: "+room:ABCDEF123456".to_owned(),
        }]
    );
}

#[test]
fn flush_controller_auth_notifications_to_sink_dispatches_outcome_notifications() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "+room:ABCDEF123456".to_owned(),
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
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };
    let mut runtime = create_client_runtime(&config);
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"cli-user","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"cli-user","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("controller auth notifications should dispatch");

    let mut captured = Vec::new();
    flush_controller_auth_notifications_to_sink(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("controller auth notifications should dispatch");

    assert_eq!(
        captured,
        vec![
            ControllerAuthTransitionNotification::Succeeded {
                username: "cli-user".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            },
            ControllerAuthTransitionNotification::Failed {
                username: "cli-user".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            },
        ]
    );
}

#[test]
fn flush_chat_notifications_to_sink_dispatches_chat_messages() {
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
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };
    let mut runtime = create_client_runtime(&config);
    runtime
        .session_mut()
        .apply_message_json(r#"{"Chat":{"username":"bob","message":"hello everyone"}}"#)
        .expect("chat should apply");
    runtime
        .run_chat_notifications_if_needed()
        .expect("chat notifications should dispatch");

    let mut captured = Vec::new();
    flush_chat_notifications_to_sink(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("chat notifications should dispatch");
    flush_chat_notifications_to_sink(&mut runtime, &mut ignore_chat_notification)
        .expect("drained chat notification queue should be empty");

    assert_eq!(
        captured,
        vec![ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: "hello everyone".to_owned(),
        }]
    );
}

#[test]
fn flush_user_change_notifications_to_sink_dispatches_visibility_metadata() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "+room:ABCDEF123456".to_owned(),
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
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };
    let mut runtime = create_client_runtime(&config);
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"cli-user","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"}}}}}"#)
        .expect("user join should apply");
    runtime
        .run_user_change_notifications_if_needed()
        .expect("user change notifications should dispatch");

    let mut captured = Vec::new();
    flush_user_change_notifications_to_sink(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("user change notifications should dispatch");
    flush_user_change_notifications_to_sink(&mut runtime, &mut ignore_user_change_notification)
        .expect("drained user change notification queue should be empty");

    assert_eq!(
        captured,
        vec![UserChangeNotification::Joined {
            username: "bob".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            hide_from_osd: true,
        }]
    );
}
