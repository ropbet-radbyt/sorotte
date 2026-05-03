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
        session_transport_reconnect_due_at: None,
        session_transport_reconnect_failures: 0,
        session_transport_disconnect_pending_cleanup: false,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        startup_remote_actions_attempted: false,
        startup_remote_actions_rx: None,
        startup_stream_helper_probe_completed: false,
        startup_stream_helper_probe_rx: None,
        player: Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
            state: player_state.clone(),
        }))),
        player_launch_state: GuiPlayerLaunchRuntimeState::None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_local_file_placeholder: false,
        last_published_local_file: None,
        attached_media_search_index: None,
        attached_media_search_next_retry_at: None,
        pending_attached_media_resolution: None,
        attached_media_search_progress: None,
        attached_media_search_progress_updated_at: None,
        attached_media_search_build_state: GuiAttachedMediaSearchBuildState::Idle,
        attached_media_search_build_roots: Vec::new(),
        attached_media_search_index_revision: 0,
        unresolved_attached_media_target: None,
        last_attached_media_resolution_trigger: None,
        last_applied_attached_room_playstate: None,
        suppressed_attached_room_playstate_after_playlist_reset: None,
        pending_local_attached_pause_override: None,
        player_position_seconds: None,
        player_paused: None,
        active_shared_playlist_index: None,
        playlist_auto_advance_eof_latched: false,
        user_offset_seconds: 0.0,
        stream_helper_runtime_snapshot: Default::default(),
        stream_helper_remediation_runtime_snapshot: Default::default(),
        plex_client: None,
        plex_auth_session: None,
        plex_auth_poll_due_at: None,
        plex_servers: Vec::new(),
        plex_server_reachability: std::collections::HashMap::new(),
        startup_plex_server_refresh_attempted: false,
        startup_plex_server_refresh_rx: None,
        plex_sync_engine: None,
        plex_runtime_snapshot: Default::default(),
        pending_stream_retry_target: None,
        managed_stream_helper_refresh_required: false,
        pending_stream_feedback: std::collections::VecDeque::new(),
        pending_stream_load_context: None,
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
                chat: runtime_chat_pane_ready_rows(),
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
                    chat_unavailable_reason: Some(
                        "Chat input is unavailable because no session runtime is connected."
                            .to_owned(),
                    ),
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
        label: "Chat Output",
        value: false,
    }));
    assert!(
        !state.commands.can_send_chat_message,
        "chat send should stay disabled while no session runtime is connected"
    );
    assert_eq!(
        state.commands.chat_unavailable_reason.as_deref(),
        Some("Chat input is unavailable because no session runtime is connected.")
    );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let refreshed_command_actions = handle.drain_actions();
    assert!(refreshed_command_actions.is_empty());
    for action in refreshed_command_actions {
        assert!(state.apply(action));
    }
    assert!(!state.commands.can_send_chat_message);
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
    let paused_actions = handle.drain_actions();
    assert_eq!(
        paused_actions,
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
    for action in paused_actions {
        assert!(state.apply(action));
    }

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(
        handle.drain_actions().is_empty(),
        "idle runtime pumps should not emit redundant player projection actions"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_clears_placeholder_after_media_load_failure() {
    #[derive(Default)]
    struct FailingLoadPlayerAdapter {
        outcomes: Vec<syncplay_player_api::PlayerMediaLoadOutcome>,
    }

    impl PlayerAdapter for FailingLoadPlayerAdapter {
        fn name(&self) -> &'static str {
            "failing-load"
        }

        fn take_media_load_outcome(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerMediaLoadOutcome> {
            self.outcomes.pop()
        }
    }

    let requested_target = "https://cdn.example.com/broken.m3u8".to_owned();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(FailingLoadPlayerAdapter {
        outcomes: vec![syncplay_player_api::PlayerMediaLoadOutcome::failure(
            requested_target.clone(),
            None,
            syncplay_player_api::PlayerMediaLoadFailureKind::Unknown,
            "network timeout",
        )],
    })));
    owner.player_local_file =
        Some(GuiPersistedConfigRuntimeOwner::placeholder_local_file_for_path(&requested_target));
    owner.player_local_file_placeholder = true;
    owner.pending_stream_retry_target = Some(requested_target.clone());
    owner.pending_stream_load_context = Some(GuiPendingStreamLoadContext {
        requested_target: requested_target.clone(),
        user_initiated: true,
    });

    owner.refresh_player_state_impl();

    assert_eq!(owner.player_local_file, None);
    assert!(!owner.player_local_file_placeholder);
    assert_eq!(owner.player_position_seconds, None);
    assert_eq!(
        owner.pending_stream_retry_target.as_deref(),
        Some(requested_target.as_str())
    );
    assert_eq!(owner.pending_stream_load_context, None);
    assert_eq!(owner.pending_stream_feedback.len(), 1);
    let actions = owner
        .pending_stream_feedback
        .front()
        .expect("media-load failure should queue GUI feedback");
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message.contains("network timeout")
    )));
}

#[test]
fn gui_persisted_config_runtime_owner_resets_stale_position_when_the_player_reports_a_new_file() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<syncplay_player_api::LocalFileUpdate>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop_front()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque::from([
            syncplay_player_api::LocalFileUpdate::new("episode2.mkv")
                .with_path("C:/Media/episode2.mkv"),
        ]),
    }));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state,
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv"),
    );
    owner.player_position_seconds = Some(42.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .ensure_detached_client_core_chat_session(&state)
        .expect("detached client-core session should bootstrap");

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        owner.player_position_seconds,
        Some(0.0),
        "a newly reported file should reset the stored global playback position"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(0.0),
        "detached-session telemetry should publish the new file from the start instead of reusing the old timestamp"
    );
}
