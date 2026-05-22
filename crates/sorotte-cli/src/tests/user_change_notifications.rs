use super::*;

#[test]
fn user_change_notification_message_uses_legacy_style_wording() {
    assert_eq!(
        user_change_notification_message(&UserChangeNotification::Joined {
            username: "bob".to_owned(),
            room: "room1".to_owned(),
            hide_from_osd: true,
        }),
        "bob has joined the room: 'room1'"
    );
    assert_eq!(
        user_change_notification_message(&UserChangeNotification::Playing {
            username: "bob".to_owned(),
            room: "room1".to_owned(),
            file_name: Some("movie.mkv".to_owned()),
            file_duration: None,
            include_room_addendum: true,
            hide_from_osd: false,
        }),
        "bob is playing 'movie.mkv' in room: 'room1'"
    );
    assert_eq!(
        user_change_notification_message(&UserChangeNotification::Playing {
            username: "bob".to_owned(),
            room: "room1".to_owned(),
            file_name: Some("movie.mkv".to_owned()),
            file_duration: None,
            include_room_addendum: false,
            hide_from_osd: false,
        }),
        "bob is playing 'movie.mkv'"
    );
    assert_eq!(
        user_change_notification_message(&UserChangeNotification::Playing {
            username: "bob".to_owned(),
            room: "room1".to_owned(),
            file_name: None,
            file_duration: None,
            include_room_addendum: false,
            hide_from_osd: false,
        }),
        "bob is playing a file"
    );
    assert_eq!(
        user_change_notification_message(&UserChangeNotification::Left {
            username: "bob".to_owned(),
            hide_from_osd: true,
        }),
        "bob has left"
    );
}

#[test]
fn user_change_notification_message_localized_legacy_compatible_localizes_common_runtime_notifications()
 {
    assert_eq!(
        crate::user_change_notification_message_localized_legacy_compatible(
            &UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: true,
            },
            Some("es"),
        ),
        "bob se ha unido a la sala: 'room1'"
    );
    assert_eq!(
        crate::user_change_notification_message_localized_legacy_compatible(
            &UserChangeNotification::Playing {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                file_name: Some("movie.mkv".to_owned()),
                file_duration: None,
                include_room_addendum: true,
                hide_from_osd: false,
            },
            Some("fr"),
        ),
        "bob lit 'movie.mkv' dans la salle: 'room1'"
    );
    assert_eq!(
        crate::user_change_notification_message_localized_legacy_compatible(
            &UserChangeNotification::Left {
                username: "bob".to_owned(),
                hide_from_osd: true,
            },
            Some("de"),
        ),
        "bob hat den Raum verlassen"
    );
}

#[test]
fn user_change_notification_hidden_from_osd_uses_visibility_metadata() {
    assert!(user_change_notification_hidden_from_osd(
        &UserChangeNotification::Joined {
            username: "bob".to_owned(),
            room: "room1".to_owned(),
            hide_from_osd: true,
        }
    ));
    assert!(!user_change_notification_hidden_from_osd(
        &UserChangeNotification::Playing {
            username: "bob".to_owned(),
            room: "room1".to_owned(),
            file_name: Some("movie.mkv".to_owned()),
            file_duration: None,
            include_room_addendum: false,
            hide_from_osd: false,
        }
    ));
    assert!(user_change_notification_hidden_from_osd(
        &UserChangeNotification::Left {
            username: "bob".to_owned(),
            hide_from_osd: true,
        }
    ));
}

#[test]
fn format_duration_legacy_matches_python_shape() {
    assert_eq!(format_duration_legacy(95.5), "01:36");
    assert_eq!(format_duration_legacy(3600.0), "01:00:00");
    assert_eq!(format_duration_legacy(604800.0), "00:00 (Title 1)");
    assert_eq!(format_duration_legacy(-1.5), "-00:02");
}

#[test]
fn user_change_playing_message_includes_formatted_duration_when_available() {
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
        .apply_message_json(
            r#"{"Hello":{"username":"cli-user","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"file":{"name":"movie.mkv","duration":95.5}}}}}"#,
            )
            .expect("playing update should apply");
    runtime
        .run_user_change_notifications_if_needed()
        .expect("user-change notification dispatch should succeed");

    let mut captured = Vec::new();
    flush_user_change_notifications_to_sink(&mut runtime, &mut |notification| {
        captured.push(user_change_notification_message(notification));
        Ok(())
    })
    .expect("notification sink dispatch should succeed");

    assert_eq!(
        captured,
        vec!["bob is playing 'movie.mkv' (01:36) in room: 'room2'"]
    );
}
