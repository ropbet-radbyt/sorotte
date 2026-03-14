use super::*;

#[test]
fn gui_persisted_config_runtime_owner_syncs_attached_player_runtime_state() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: Vec<syncplay_player_api::LocalFileUpdate>,
        playback_updates: Vec<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner {
        config_path: None,
        session: None,
        session_projects_to_shell: false,
        session_transport: None,
        session_transport_driver: None,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        player: Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
            state: player_state.clone(),
        }))),
        player_launch_state: GuiPlayerLaunchRuntimeState::None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_position_seconds: None,
        player_paused: None,
        user_offset_seconds: 0.0,
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let bootstrap_actions = handle.drain_actions();
    assert_eq!(
        bootstrap_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Play",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Toggle Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Seek",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Advanced",
                        action_label: "Set Offset",
                        enabled: true,
                    },
                ],
                tls_prompt_expected: false,
                update_notice_expected: false,
                about_dialog_available: true,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: false,
                    can_search_missing_media: false,
                    can_toggle_pause: true,
                    can_send_chat_message: false,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in bootstrap_actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback.can_toggle_pause);
    assert!(state.main_window.playback.can_seek);
    assert!(state.commands.can_toggle_pause);

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));
    assert!(
        state.commands.can_send_chat_message,
        "config-driven chat availability should update immediately when no runtime field override is active"
    );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let refreshed_command_actions = handle.drain_actions();
    assert!(refreshed_command_actions.is_empty());
    for action in refreshed_command_actions {
        assert!(state.apply(action));
    }
    assert!(state.commands.can_send_chat_message);
    assert!(state.commands.can_reset_configuration);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .local_file_updates
        .push(
            syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_duration_seconds(93.5)
                .with_size_bytes(734003200),
        );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let local_file_actions = handle.drain_actions();
    assert_eq!(
        local_file_actions,
        vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
            MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv [93.500s, 734003200 bytes]".to_owned()],
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            },
        )]
    );
    for action in local_file_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv [93.500s, 734003200 bytes]"]
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(syncplay_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
            MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv [93.500s, 734003200 bytes]".to_owned()],
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: true,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            },
        )]
    );
}

#[test]
fn gui_persisted_config_runtime_owner_uses_attached_player_for_media_open_and_seek() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        set_paused_values: Vec<bool>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner {
        config_path: None,
        session: None,
        session_projects_to_shell: false,
        session_transport: None,
        session_transport_driver: None,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        player: Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
            state: player_state.clone(),
        }))),
        player_launch_state: GuiPlayerLaunchRuntimeState::None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_position_seconds: None,
        player_paused: None,
        user_offset_seconds: 0.0,
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            "C:/Media/episode1.mkv".to_owned(),
            "C:/Media/episode2.mkv".to_owned(),
        ],
        load_into_shared_playlist: false,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let open_actions = handle.drain_actions();
    assert_eq!(
        open_actions,
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Opened the first selected media file through the attached recording player: C:/Media/episode1.mkv. Ignored 1 additional selections."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Opened the first selected media file through the attached recording player: C:/Media/episode1.mkv. Ignored 1 additional selections."
                    .to_owned(),
            ),
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv".to_owned()],
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Play",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Toggle Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Seek",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Advanced",
                        action_label: "Set Offset",
                        enabled: true,
                    },
                ],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_saved_server: false,
                    can_disconnect_session: false,
                    can_connect_public_server: false,
                    can_refresh_public_servers: true,
                    can_search_missing_media: false,
                    can_toggle_pause: true,
                    can_send_chat_message: false,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in open_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv"]
    );

    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::TogglePlaybackPause,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let toggle_actions = handle.drain_actions();
    assert_eq!(
        toggle_actions,
        vec![GuiShellAction::CompletePlaybackPauseToggle]
    );
    for action in toggle_actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback_paused);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec!["C:/Media/episode1.mkv".to_owned()]
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![true]
    );

    handle.push_request(GuiRuntimeRequest::SeekOffset(12.5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Applied a 12.5 second seek via the attached recording player (target 12.500 seconds)."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Applied a 12.5 second seek via the attached recording player (target 12.500 seconds)."
                    .to_owned(),
            ),
        ]
    );

    handle.push_request(GuiRuntimeRequest::SeekOffset(-2.5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Applied a -2.5 second seek via the attached recording player (target 10.000 seconds)."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Applied a -2.5 second seek via the attached recording player (target 10.000 seconds)."
                    .to_owned(),
            ),
        ]
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions,
        vec![12.5, 10.0]
    );
}
