use super::*;

#[test]
fn format_file_difference_summary_uses_legacy_difference_order() {
    assert_eq!(
        format_file_difference_summary(FileDifferenceSummary {
            filename: true,
            filesize: true,
            fileduration: true,
        }),
        Some("filename, filesize, duration".to_owned())
    );
    assert_eq!(
        format_file_difference_summary(FileDifferenceSummary {
            filename: false,
            filesize: false,
            fileduration: false,
        }),
        None
    );
}

#[test]
fn localized_file_difference_summary_legacy_compatible_localizes_user_visible_tokens() {
    assert_eq!(
        crate::localized_file_difference_summary_legacy_compatible(
            "filename, filesize, duration",
            Some("de"),
        ),
        "Dateiname, Dateigroesse, Dauer"
    );
    assert_eq!(
        crate::localized_file_difference_summary_legacy_compatible(
            "filename, filesize",
            Some("fr"),
        ),
        "nom du fichier, taille du fichier"
    );
    assert_eq!(
        crate::localized_file_difference_summary_legacy_compatible("filename, duration", None,),
        "filename, duration"
    );
}

#[test]
fn flush_file_difference_notifications_to_sink_dedupes_and_honors_duration_overrides() {
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
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.0}}}}}"#,
            )
            .expect("local user file should apply");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":100.0}}}}}"#,
            )
            .expect("peer duration mismatch should apply");

    let mut state = crate::FileDifferenceNotificationState::default();
    let mut captured = Vec::new();
    flush_file_difference_notifications_to_sink(&runtime, &mut state, &mut |summary| {
        captured.push(summary.to_owned());
        Ok(())
    })
    .expect("duration mismatch should emit one notification");
    flush_file_difference_notifications_to_sink(&runtime, &mut state, &mut |summary| {
        captured.push(summary.to_owned());
        Ok(())
    })
    .expect("identical summary should not emit duplicate notification");
    assert_eq!(captured, vec!["duration"]);

    let mut readiness = runtime.session().readiness_autoplay_config().clone();
    readiness.show_duration_notification = false;
    runtime
        .session_mut()
        .set_readiness_autoplay_config(readiness);
    flush_file_difference_notifications_to_sink(&runtime, &mut state, &mut |summary| {
        captured.push(summary.to_owned());
        Ok(())
    })
    .expect("disabling duration notifications should clear difference summary");
    assert_eq!(captured, vec!["duration"]);

    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"other.mkv","size":123456789,"duration":100.0}}}}}"#,
            )
            .expect("peer filename mismatch should apply");
    flush_file_difference_notifications_to_sink(&runtime, &mut state, &mut |summary| {
        captured.push(summary.to_owned());
        Ok(())
    })
    .expect("new filename mismatch should emit notification");
    assert_eq!(captured, vec!["duration", "filename"]);
}
