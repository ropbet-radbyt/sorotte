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
        last_published_local_file: None,
        attached_media_search_index: None,
        attached_media_search_next_retry_at: None,
        pending_attached_media_resolution: None,
        unresolved_attached_media_target: None,
        last_applied_attached_room_playstate: None,
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
                room_control_status: "Unavailable: no active server session.".to_owned(),
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
                room_control_status: "Unavailable: no active server session.".to_owned(),
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
                room_control_status: "Unavailable: no active server session.".to_owned(),
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
        last_published_local_file: None,
        attached_media_search_index: None,
        attached_media_search_next_retry_at: None,
        pending_attached_media_resolution: None,
        unresolved_attached_media_target: None,
        last_applied_attached_room_playstate: None,
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
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                room_control_status: "Unavailable: no active server session.".to_owned(),
                shared_playlist_enabled: true,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_offset: false,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            }),
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Loaded 2 selected media entries into the shared playlist."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Loaded 2 selected media entries into the shared playlist."
                    .to_owned(),
            ),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
                    .to_owned(),
            ),
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                room_control_status: "Unavailable: no active server session.".to_owned(),
                shared_playlist_enabled: true,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: true,
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
                        section_title: "Playback",
                        action_label: "Playlist Actions",
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
        vec!["episode1.mkv", "episode2.mkv"]
    );

    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::TogglePlaybackPause,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let toggle_actions = handle.drain_actions();
    assert!(
        toggle_actions.contains(&GuiShellAction::CompletePlaybackPauseToggle),
        "pending pause-toggle completion should still emit the completion action",
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

    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let direct_toggle_actions = handle.drain_actions();
    assert!(
        direct_toggle_actions.contains(&GuiShellAction::AnnouncePlaybackResumed),
        "direct pause toggles should still resume playback",
    );
    for action in direct_toggle_actions {
        assert!(state.apply(action));
    }
    assert!(!state.main_window.playback_paused);

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

    handle.push_request(GuiRuntimeRequest::SeekToPosition(42.0));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Applied an absolute seek via the attached recording player (target 42.000 seconds)."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Applied an absolute seek via the attached recording player (target 42.000 seconds)."
                    .to_owned(),
            ),
        ]
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions,
        vec![12.5, 10.0, 42.0]
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![true, false]
    );
}

#[test]
fn gui_persisted_config_runtime_owner_resolves_inbound_shared_playlist_media_in_background_and_applies_room_playstate()
 {
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

    let root = test_temp_root("shared-playlist-background-search");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&nested_directory)
        .expect("background shared-playlist search fixture directory should be created");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("background shared-playlist search fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"bob"},"ping":{"latencyCalculation":123.0}}}"#
            .to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref()),
        "background shared-playlist search should eventually open the selected media"
    );
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 42.0).abs() < f64::EPSILON),
        "background shared-playlist search should apply the current room seek target"
    );
    assert!(
        recorded_state.set_paused_values.contains(&false),
        "background shared-playlist search should apply the current room pause state"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reuses_media_search_index_for_later_playlist_selection() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
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
    }

    let root = test_temp_root("shared-playlist-search-cache");
    let season_directory = root.join("season-1");
    std::fs::create_dir_all(&season_directory)
        .expect("shared-playlist cache fixture directory should be created");
    let episode_two_path = season_directory.join("episode2.mkv");
    let episode_three_path = season_directory.join("episode3.mkv");
    std::fs::write(&episode_two_path, b"test")
        .expect("shared-playlist cache fixture episode two should be written");
    std::fs::write(&episode_three_path, b"test")
        .expect("shared-playlist cache fixture episode three should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == episode_two_path.to_string_lossy().as_ref())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        owner.attached_media_search_index.is_some(),
        "first background search should populate the reusable media index"
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .opened_paths
        .clear();

    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":2,"user":"bob"}}}"#.to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == episode_three_path.to_string_lossy().as_ref()),
        "later playlist selections should resolve immediately from the cached media index"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_apply_room_playstate_while_selected_playlist_target_is_unresolved()
 {
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

    let root = test_temp_root("shared-playlist-unresolved-playstate");
    let current_media_path = root.join("episode1.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("existing attached-player media fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"bob"},"ping":{"latencyCalculation":123.0}}}"#
            .to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state.opened_paths.is_empty(),
        "unresolved shared-playlist targets should not open a replacement file until resolution succeeds"
    );
    assert!(
        recorded_state.set_positions.is_empty(),
        "room seek state should not be applied to the previously-open file while the new playlist target is unresolved"
    );
    assert!(
        recorded_state.set_paused_values.is_empty(),
        "room pause state should not be applied to the previously-open file while the new playlist target is unresolved"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_retries_unresolved_shared_playlist_media_after_double_check_interval()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
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
    }

    let root = test_temp_root("shared-playlist-double-check-retry");
    let delayed_directory = root.join("delayed");
    let selected_media_path = delayed_directory.join("episode2.mkv");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        folder_search_timeout_seconds: Some(1.0),
        folder_search_double_check_interval_seconds: Some(0.05),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );

    let first_scan_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < first_scan_deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if owner.attached_media_search_index.is_some()
            && owner.pending_attached_media_resolution.is_none()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        owner.attached_media_search_index.is_some(),
        "first missing-media scan should populate the reusable index even when the target is still missing"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .is_empty(),
        "no file should open before the missing target appears on disk"
    );

    std::fs::create_dir_all(&delayed_directory)
        .expect("delayed shared-playlist search fixture directory should be created");
    std::fs::write(&selected_media_path, b"test")
        .expect("delayed shared-playlist search fixture should be written");

    let retry_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < retry_deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref()),
        "automatic missing-media resolution should retry after the configured double-check interval and open files that appear later"
    );

    let _ = std::fs::remove_dir_all(&root);
}
