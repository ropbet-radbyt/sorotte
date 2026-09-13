use super::*;

#[test]
fn flush_controller_auth_notifications_dispatches_attempt_notification() {
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
        controlled_room_password_override: Some("AB-123-456".into()),
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

    let pending = runtime
        .pending_controller_auth_notification()
        .cloned()
        .expect("notification should be queued");
    assert!(
        flush_controller_auth_notifications(&mut runtime, &mut |_| anyhow::bail!(
            "output unavailable"
        ))
        .is_err()
    );
    assert_eq!(
        runtime.pending_controller_auth_notification(),
        Some(&pending)
    );
    let mut captured = Vec::new();
    flush_controller_auth_notifications(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("controller auth notifications should dispatch");
    assert!(runtime.pending_controller_auth_notification().is_none());
    assert_eq!(
        runtime.player().last_simulated_syncplay_osd_message(),
        Some(&(
            crate::controller_auth_transition_notification_message_localized(
                captured.last().unwrap(),
                crate::language_support::current_runtime_language_tag().as_deref()
            ),
            sorotte_player_mpv::SyncplayOsdKind::Notification,
        ))
    );
    flush_controller_auth_notifications(&mut runtime, &mut ignore_controller_auth_notification)
        .expect("drained controller auth notification queue should be empty");

    assert_eq!(
        captured,
        vec![ControllerAuthTransitionNotification::Attempting {
            room: "+room:ABCDEF123456".to_owned(),
        }]
    );
}

#[test]
fn flush_controller_auth_notifications_dispatches_outcome_notifications() {
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
    flush_controller_auth_notifications(&mut runtime, &mut |notification| {
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
fn flush_chat_notifications_dispatches_chat_messages() {
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
    flush_chat_notifications(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("chat notifications should dispatch");
    flush_chat_notifications(&mut runtime, &mut ignore_chat_notification)
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
fn flush_user_change_notifications_dispatches_visibility_metadata() {
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

    let pending = runtime.pending_user_change_notification().cloned().unwrap();
    assert!(
        flush_user_change_notifications(&mut runtime, &mut |_| anyhow::bail!("output unavailable"))
            .is_err()
    );
    assert_eq!(runtime.pending_user_change_notification(), Some(&pending));
    let mut captured = Vec::new();
    flush_user_change_notifications(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("user change notifications should dispatch");
    flush_user_change_notifications(&mut runtime, &mut ignore_user_change_notification)
        .expect("drained user change notification queue should be empty");

    assert!(runtime.pending_user_change_notification().is_none());
    assert_eq!(
        runtime.player().last_simulated_syncplay_osd_message(),
        None,
        "hidden user changes must not reach OSD"
    );
    assert_eq!(
        captured,
        vec![UserChangeNotification::Joined {
            username: "bob".to_owned(),
            room: "+room:ABCDEF123456".to_owned(),
            hide_from_osd: true,
        }]
    );
}

#[test]
fn chat_output_failure_preserves_queue_order_after_player_delivery_failure() {
    let (player, commands) = MpvAdapter::with_cleanup_recording_sorotte_bridge_test_ipc(
        sorotte_player_mpv::SyncplayUiSettings::default(),
        None,
    );
    // The recording transport accepts writes but returns EOF when awaiting a reply.
    let mut runtime = ClientApplication::with_default_session(player);
    for message in ["first", "second"] {
        runtime
            .session_mut()
            .apply_message_json(
                &serde_json::json!({
                    "Chat": {"username": "bob", "message": message}
                })
                .to_string(),
            )
            .unwrap();
    }
    runtime.run_chat_notifications_if_needed().unwrap();
    let first = runtime.pending_chat_notification().cloned().unwrap();
    assert!(
        flush_chat_notifications(&mut runtime, &mut |_| anyhow::bail!("output unavailable"))
            .is_err()
    );
    assert_eq!(runtime.pending_chat_notification(), Some(&first));
    assert!(
        commands.lock().unwrap().iter().any(|command| {
            command[0] == "script-message-to" && command[2] == "chat" && command[3] == "<bob> first"
        }),
        "the production flush must attempt player chat delivery before acknowledging output"
    );

    let mut delivered = Vec::new();
    flush_chat_notifications(&mut runtime, &mut |notification| {
        delivered.push(chat_notification_message(notification));
        Ok(())
    })
    .unwrap();
    assert_eq!(delivered, ["<bob> first", "<bob> second"]);
    assert!(runtime.pending_chat_notification().is_none());
    flush_chat_notifications(&mut runtime, &mut |_| {
        panic!("successful output must be drained once")
    })
    .unwrap();
}
