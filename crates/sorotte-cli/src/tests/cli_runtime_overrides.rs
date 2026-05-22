use super::*;

#[test]
fn apply_legacy_client_arg_overrides_updates_client_loop_config() {
    let mut config = test_client_loop_config();
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        config_path: None,
        config_root: None,
        language: None,
        player_path: None,
        file: None,
        player_args: vec![],
        load_playlist_from_file: None,
        host: Some("legacy.example".to_owned()),
        port: Some(3210),
        username: Some("legacy-user".to_owned()),
        room: Some("+room:ABCDEF123456:AB-123-456".to_owned()),
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };

    apply_legacy_client_arg_overrides(&mut config, &overrides);

    assert_eq!(config.host, "legacy.example");
    assert_eq!(config.port, 3210);
    assert_eq!(config.username, "legacy-user");
    assert_eq!(config.room, "+room:ABCDEF123456");
    assert_eq!(
        config.controlled_room_password_override.as_deref(),
        Some("AB-123-456")
    );
}

#[test]
fn apply_legacy_client_arg_overrides_prefers_explicit_password_flag() {
    let mut config = test_client_loop_config();
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        config_path: None,
        config_root: None,
        language: None,
        player_path: None,
        file: None,
        player_args: vec![],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: Some("+room:ABCDEF123456:AB-123-456".to_owned()),
        controlled_room_password_override: Some("CD-987-654".to_owned()),
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };

    apply_legacy_client_arg_overrides(&mut config, &overrides);
    assert_eq!(
        config.controlled_room_password_override.as_deref(),
        Some("CD-987-654")
    );
}

#[test]
fn reconnect_transition_notification_message_uses_legacy_style_wording() {
    assert_eq!(
        reconnect_transition_notification_message(&ReconnectTransitionNotification::Attempting {
            retries: 2,
            delay_seconds: 0.4,
        }),
        "Connection with server lost, attempting to reconnect (retry=2, delay_seconds=0.400)"
    );
    assert_eq!(
        reconnect_transition_notification_message(&ReconnectTransitionNotification::Connected),
        "Reconnected to server"
    );
    assert_eq!(
        reconnect_transition_notification_message(&ReconnectTransitionNotification::Disconnected),
        "Connection with server lost, reconnect attempts exhausted"
    );
    assert_eq!(
        reconnect_transition_notification_message(&ReconnectTransitionNotification::RestoringState),
        "Restoring local state after reconnect..."
    );
    assert_eq!(
        reconnect_transition_notification_message(
            &ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: false,
                local_position: 117.5,
                room_position: 120.0,
                position_diff_seconds: 2.5,
            }
        ),
        "Reconnect state restore validation mismatch; correcting local player: player(paused=true, position=117.500) room(paused=false, position=120.000) diff=2.500"
    );
    assert_eq!(
        reconnect_transition_notification_message(
            &ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt: 1,
                max_attempts: 3,
                cooldown_ticks: 2,
            }
        ),
        "Reconnect state restore correction failed; scheduling retry (attempt=1/3, cooldown_ticks=2)"
    );
    assert_eq!(
        reconnect_transition_notification_message(
            &ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts: 4,
                max_attempts: 3,
            }
        ),
        "Reconnect state restore correction failed; retry budget exhausted (attempts=4, max_attempts=3), stopping auto-correction for this restore cycle"
    );
    assert_eq!(
            reconnect_transition_notification_message(
                &ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                    consecutive_mismatch_cycles: 3,
                    disable_after_mismatch_cycles: 3,
                }
            ),
            "Reconnect state restore correction disabled after repeated mismatches (consecutive_mismatch_cycles=3, threshold=3)"
        );
    assert_eq!(
            reconnect_transition_notification_message(
                &ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 1,
                }
            ),
            "Reconnect state restore correction suppressed for recovery cooldown (remaining_reconnect_cycles_after_this_cycle=1)"
        );
    assert_eq!(
            reconnect_transition_notification_message(
                &ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled
            ),
            "Reconnect state restore correction re-enabled after recovery cooldown"
        );
    assert_eq!(
        reconnect_transition_notification_message(
            &ReconnectTransitionNotification::RestoringPlaylist
        ),
        "Restoring playlist on reconnect..."
    );
}

#[test]
fn reconnect_transition_notification_message_localized_legacy_compatible_localizes_common_runtime_notifications()
 {
    assert_eq!(
        crate::reconnect_transition_notification_message_localized_legacy_compatible(
            &ReconnectTransitionNotification::Attempting {
                retries: 2,
                delay_seconds: 0.4,
            },
            Some("fr"),
        ),
        "Connexion au serveur perdue, tentative de reconnexion (retry=2, delay_seconds=0.400)"
    );
    assert_eq!(
        crate::reconnect_transition_notification_message_localized_legacy_compatible(
            &ReconnectTransitionNotification::Connected,
            Some("pt_BR"),
        ),
        "Reconectado ao servidor"
    );
    assert_eq!(
        crate::reconnect_transition_notification_message_localized_legacy_compatible(
            &ReconnectTransitionNotification::RestoringPlaylist,
            Some("de"),
        ),
        "Playlist nach Wiederverbindung wiederherstellen..."
    );
    assert_eq!(
            crate::reconnect_transition_notification_message_localized_legacy_compatible(
                &ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                Some("fr"),
            ),
            "Reconnect state restore correction re-enabled after recovery cooldown"
        );
}

#[test]
fn apply_legacy_client_arg_managed_mpv_overrides_uses_player_path_and_file_when_env_config_missing()
{
    let mut managed = ManagedMpvLaunchEnvConfig {
        enabled: true,
        mpv_bin: None,
        media_file: None,
        extra_args: Vec::new(),
        ipc_path: None,
        connect_timeout_ms: None,
        connect_poll_interval_ms: None,
    };
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        config_path: None,
        config_root: None,
        language: None,
        player_path: Some("C:/mpv/mpv.exe".to_owned()),
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--fs".to_owned()],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: None,
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };

    apply_legacy_client_arg_managed_mpv_overrides(&mut managed, Some(&overrides));

    assert!(managed.enabled);
    assert_eq!(managed.mpv_bin, Some(PathBuf::from("C:/mpv/mpv.exe")));
    assert_eq!(
        managed.media_file,
        Some(PathBuf::from("C:/media/movie.mkv"))
    );
    assert_eq!(managed.extra_args, vec!["--fs".to_owned()]);
}

#[test]
fn apply_legacy_client_arg_managed_mpv_overrides_does_not_override_explicit_env_managed_config() {
    let mut managed = ManagedMpvLaunchEnvConfig {
        enabled: true,
        mpv_bin: Some(PathBuf::from("D:/custom/mpv.exe")),
        media_file: Some(PathBuf::from("D:/custom/start.mkv")),
        extra_args: vec!["--profile=fast".to_owned()],
        ipc_path: None,
        connect_timeout_ms: None,
        connect_poll_interval_ms: None,
    };
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        config_path: None,
        config_root: None,
        language: None,
        player_path: Some("C:/mpv/mpv.exe".to_owned()),
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec![],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: None,
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };

    apply_legacy_client_arg_managed_mpv_overrides(&mut managed, Some(&overrides));

    assert_eq!(managed.mpv_bin, Some(PathBuf::from("D:/custom/mpv.exe")));
    assert_eq!(
        managed.media_file,
        Some(PathBuf::from("D:/custom/start.mkv"))
    );
    assert_eq!(managed.extra_args, vec!["--profile=fast".to_owned()]);
}

#[test]
fn apply_legacy_client_arg_managed_mpv_overrides_does_not_auto_enable_for_non_mpv_player_path() {
    let mut managed = ManagedMpvLaunchEnvConfig::default();
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        config_path: None,
        config_root: None,
        language: None,
        player_path: Some("C:/players/vlc.exe".to_owned()),
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--fullscreen".to_owned()],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: None,
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };

    apply_legacy_client_arg_managed_mpv_overrides(&mut managed, Some(&overrides));

    assert!(!managed.enabled);
    assert_eq!(managed.mpv_bin, None);
    assert_eq!(
        managed.media_file,
        Some(PathBuf::from("C:/media/movie.mkv"))
    );
    assert_eq!(managed.extra_args, vec!["--fullscreen".to_owned()]);
}

#[test]
fn apply_legacy_client_arg_managed_mpv_overrides_preserves_launch_only_args_for_managed_launch() {
    let mut managed = ManagedMpvLaunchEnvConfig {
        enabled: true,
        mpv_bin: None,
        media_file: None,
        extra_args: Vec::new(),
        ipc_path: None,
        connect_timeout_ms: None,
        connect_poll_interval_ms: None,
    };
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        config_path: None,
        config_root: None,
        language: None,
        player_path: Some("C:/mpv/mpv.exe".to_owned()),
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--profile=fast".to_owned(), "--msg-level=all=v".to_owned()],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: None,
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };

    apply_legacy_client_arg_managed_mpv_overrides(&mut managed, Some(&overrides));

    assert_eq!(
        managed.extra_args,
        vec!["--profile=fast".to_owned(), "--msg-level=all=v".to_owned(),]
    );
}
