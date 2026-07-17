use super::*;
use crate::app::runtime_owner::GuiUpdateRuntime;

#[test]
fn gui_persisted_config_runtime_owner_uses_attached_player_for_media_open_and_seek() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        local_file_updates: Vec<sorotte_player_api::LocalFileUpdate>,
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.opened_paths.push(path.to_owned());
            state.local_file_updates.push(
                sorotte_player_api::LocalFileUpdate::new(
                    std::path::Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path),
                )
                .with_path(path.to_owned()),
            );
            Ok(())
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop()
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
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
        legacy_projection: None,
        session: None,
        active_session_settings: None,
        active_session_configured_settings: None,
        session_generation: 0,
        session_projects_to_shell: false,
        session_transport: None,
        session_transport_driver: None,
        session_transport_reconnect_due_at: None,
        session_transport_reconnect_failures: 0,
        session_transport_disconnect_pending_cleanup: false,
        runtime_pump_generation: 0,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        startup_remote_actions_attempted: false,
        startup_remote_actions_rx: None,
        startup_public_server_hydration:
            crate::app::runtime_owner::StartupPublicServerHydrationState::default(),
        update_runtime: GuiUpdateRuntime::new(None),
        startup_stream_helper_probe_completed: false,
        startup_stream_helper_probe_rx: None,
        player: Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
            state: player_state.clone(),
        }))),
        player_launch_state: GuiPlayerLaunchRuntimeState::None,
        applied_player_launch_state: None,
        player_settings_reapply_required: false,
        explicit_mpv_osd_placement_restore: None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_integration_health: crate::app::runtime_owner::GuiPlayerIntegrationHealth::Ready,
        player_local_file: None,
        player_local_file_placeholder: false,
        last_published_local_file: None,
        last_published_media_match_signature: None,
        local_shared_playlist_media_match_signature_path: None,
        playlist_resolution: GuiPlaylistResolutionCoordinator::default(),
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
        pending_attached_room_unpause_observation: None,
        pending_attached_player_pause_confirmation_pump: None,
        pending_attached_player_pause_command: None,
        player_position_seconds: None,
        player_paused: None,
        player_paused_for_cache: None,
        player_cache_buffering_percent: None,
        active_shared_playlist_index: None,
        playlist_auto_advance_eof_latched: false,
        user_offset_seconds: 0.0,
        stream_helper_runtime_snapshot: Default::default(),
        stream_helper_remediation_runtime_snapshot: Default::default(),
        media_match_runtime_snapshot: Default::default(),
        media_match_remediation_runtime_snapshot: Default::default(),
        media_match_tool_worker_rx: None,
        media_match_background_worker_rx: None,
        media_match_background_worker_cancel: None,
        media_match_background_trigger_key: None,
        media_match_background_index_backup: None,
        media_match_background_cancel_disposition: None,
        media_match_remote_lookup_rx: None,
        media_match_remote_lookup_trigger_key: None,
        media_match_remote_lookup_result: None,
        media_match_wire_sync_token: None,
        plex_client: None,
        plex_auth_session: None,
        plex_auth_start_rx: None,
        plex_auth_poll_rx: None,
        plex_auth_poll_due_at: None,
        plex_servers: Vec::new(),
        plex_server_reachability: std::collections::HashMap::new(),
        startup_plex_server_refresh_attempted: false,
        plex_server_discovery:
            crate::app::runtime_owner::GuiPlexServerDiscoveryCoordinator::default(),
        plex_sync_engine: None,
        plex_sync_rx: None,
        plex_sync_next_tick_due_at: None,
        plex_runtime_snapshot: Default::default(),
        plex_playlist_job_generation: 0,
        plex_playlist_search_job: None,
        plex_playlist_resolve_job: None,
        plex_stream_resolve_rx: None,
        plex_stream_resolve_trigger_key: None,
        plex_stream_resolve_context: None,
        plex_stream_resolve_result: None,
        pending_playlist_source_resolution: None,
        pending_stream_retry_target: None,
        managed_stream_helper_refresh_required: false,
        pending_stream_feedback: std::collections::VecDeque::new(),
        pending_stream_load_context: None,
        pending_logical_media_override: None,
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let media_root = test_temp_root("attached-media-open-seek");
    let episode1_path = media_root.join("episode1.mkv");
    let episode2_path = media_root.join("episode2.mkv");
    std::fs::write(&episode1_path, b"one").expect("first media fixture should be written");
    std::fs::write(&episode2_path, b"two").expect("second media fixture should be written");

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            episode1_path.to_string_lossy().into_owned(),
            episode2_path.to_string_lossy().into_owned(),
        ],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let open_actions = without_media_match_runtime_snapshots(handle.drain_actions());
    let playlist_snapshots = open_actions
        .iter()
        .filter_map(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.playlist.len() == 2 =>
            {
                Some(snapshot)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(playlist_snapshots.len(), 2);
    let opened_entry_ids = playlist_snapshots[0].playlist_entry_ids.clone();
    assert_eq!(opened_entry_ids.len(), 2);
    for snapshot in playlist_snapshots {
        assert_eq!(snapshot.playlist_entry_ids, opened_entry_ids);
        assert_eq!(
            snapshot.playlist_source_states.len(),
            opened_entry_ids.len()
        );
        assert!(
            snapshot
                .playlist_source_states
                .iter()
                .zip(&opened_entry_ids)
                .all(|(source_state, entry_id)| source_state.entry_id == *entry_id),
            "runtime snapshots should keep source states aligned with stable playlist row IDs",
        );
    }
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
                playlist_entry_ids: opened_entry_ids.clone(),
                playlist_source_states: expected_playlist_source_states_for_entries(
                    &state,
                    &["episode1.mkv", "episode2.mkv"],
                    Some("Added from the local filesystem."),
                ),
                active_playlist_index: Some(0),
                chat: runtime_chat_pane_ready_rows(),
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
            GuiShellAction::SwitchView(GuiShellView::Room),
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
                playlist_entry_ids: opened_entry_ids,
                playlist_source_states: expected_playlist_source_states_for_entries(
                    &state,
                    &["episode1.mkv", "episode2.mkv"],
                    Some("Added from the local filesystem."),
                ),
                active_playlist_index: Some(0),
                chat: runtime_chat_pane_ready_rows(),
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
            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                GuiStreamHelperRuntimeSnapshot {
                    health: GuiStreamHelperHealth::Healthy,
                    message: None,
                    target: None,
                    install_supported: false,
                    integration_supported: false,
                    retry_available: false,
                    install_location: None,
                    downloader_status: Some(
                        "Missing from Sorotte's managed install and PATH for yt-dlp."
                            .to_owned(),
                    ),
                    js_runtime_status: Some(
                        "Missing from Sorotte's managed install and PATH for Deno."
                            .to_owned(),
                    ),
                    open_install_location_available: false,
                },
            ),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        id: MenuActionId::Play,
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        id: MenuActionId::Pause,
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        id: MenuActionId::TogglePause,
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        id: MenuActionId::Seek,
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        id: MenuActionId::SharedPlaylist,
                        enabled: true,
                    },
                ],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: false,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_saved_server: false,
                    can_disconnect_session: false,
                    can_connect_public_server: false,
                    can_refresh_public_servers: true,
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
        toggle_actions.contains(&GuiShellAction::CompletePlaybackPauseState(true)),
        "pending pause-toggle completion should emit the actual pause state",
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
        vec![episode1_path.to_string_lossy().into_owned()]
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

    assert!(state.apply(GuiShellAction::BeginPlaybackPause));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SetPlaybackPause(true))
    );
    state.main_window.playback_paused = true;
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SetPlaybackPause(true),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let targeted_pause_actions = handle.drain_actions();
    assert!(
        targeted_pause_actions.contains(&GuiShellAction::CompletePlaybackPauseState(true)),
        "explicit pause completion should keep the requested target even if shell state drifts",
    );
    for action in targeted_pause_actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback_paused);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(sorotte_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(false));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let _ = handle.drain_actions();
    assert_eq!(
        owner.player_paused,
        Some(true),
        "stale mpv paused=false telemetry must not immediately undo an explicit GUI pause command",
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![true, false, true]
    );

    handle.push_request(GuiRuntimeRequest::SeekOffset(12.5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
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
            GuiShellAction::SwitchView(GuiShellView::Room),
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
            GuiShellAction::SwitchView(GuiShellView::Room),
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
        vec![true, false, true]
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_commit_undo_seek_when_player_seek_fails() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_position_attempts: usize,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            _position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_position_attempts += 1;
            Err(sorotte_player_api::PlayerError::OperationFailed(
                "seek failed".to_owned(),
            ))
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(20.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .ensure_detached_client_core_chat_session(&state)
        .expect("detached client-core session should bootstrap");

    {
        let session = owner.session.as_mut().expect("session should exist");
        session
            .sync_local_playback_telemetry(Some(false), Some(10.0))
            .expect("initial local telemetry should sync");
        let _ = session
            .record_manual_seek_to_position(20.0)
            .expect("manual seek should record undo state");
        session
            .sync_local_playback_telemetry(Some(false), Some(20.0))
            .expect("post-seek local telemetry should sync");
    }

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();

    handle.push_request(GuiRuntimeRequest::UndoSeek);
    let undo_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        undo_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Error
                    && message.contains("Playback undo seek through the attached recording player failed")
        )),
        "failed undo seek should surface the player seek error"
    );
    assert_eq!(owner.player_position_seconds, Some(20.0));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(20.0),
        "the detached runtime should keep the pre-undo local position when the player seek fails"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.pending_undo_seek_target_position()),
        Some(10.0),
        "the undo target should remain available after a failed player seek"
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_position_attempts,
        1
    );
}
