use super::*;
use crate::app::runtime_owner::GuiPendingAttachedRoomUnpauseObservation;
use crate::app::runtime_owner::GuiUpdateRuntime;

use sorotte_plex::{
    PlexMatchedItem, PlexMediaType, PlexPlaylistUri, PlexStreamTarget, SecretPlexPlaybackUrl,
};

fn disabled_shared_playlist_state_with_two_rows() -> SorotteGuiShellAppState {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(
        vec!["old-a.mkv".to_owned(), "old-b.mkv".to_owned()],
        Some(1),
        false,
    );
    state.main_window.active_playlist_index = Some(1);
    state
}

fn owner_with_attached_local_file() -> GuiPersistedConfigRuntimeOwner {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(sorotte_player_api::LocalFileUpdate::new("attached.mkv"));
    owner
}

fn assert_disabled_playlist_replacement_snapshot(
    mut state: SorotteGuiShellAppState,
    snapshot: MainWindowRuntimeSnapshot,
) {
    assert_eq!(snapshot.playlist, vec!["attached.mkv".to_owned()]);
    assert!(snapshot.playlist_entry_ids.is_empty());
    assert!(snapshot.playlist_source_states.is_empty());
    assert_eq!(snapshot.active_playlist_index, None);
    assert!(
        state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)),
        "different-length player projection should apply: {:?}",
        state.validation.last_action_error
    );
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["attached.mkv".to_owned()]
    );
}

#[test]
fn sessionless_snapshot_clears_metadata_for_different_length_disabled_playlist() {
    let state = disabled_shared_playlist_state_with_two_rows();
    let owner = owner_with_attached_local_file();

    let snapshot = owner.sessionless_main_window_snapshot(&state);

    assert_disabled_playlist_replacement_snapshot(state, snapshot);
}

#[test]
fn player_sync_clears_metadata_for_different_length_disabled_playlist() {
    let state = disabled_shared_playlist_state_with_two_rows();
    let mut owner = owner_with_attached_local_file();
    let handle = GuiQueuedRuntimeBridgeHandle::default();

    owner.sync_player_runtime_state(&handle, &state);
    let snapshot = handle
        .drain_actions()
        .into_iter()
        .find_map(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .expect("player sync should replace the disabled shared playlist");

    assert_disabled_playlist_replacement_snapshot(state, snapshot);
}

#[test]
fn gui_persisted_config_runtime_owner_syncs_attached_player_runtime_state() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: Vec<sorotte_player_api::LocalFileUpdate>,
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
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
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
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
        player: Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
            state: player_state.clone(),
        }))),
        player_launch_state: GuiPlayerLaunchRuntimeState::None,
        player_apply_state: Default::default(),
        managed_mpv_process: None,
        player_unavailability_reason: None,
        core_player_configuration_health: Default::default(),
        network_options_hook_failure_reason: None,
        network_options_runtime_health_revision: None,
        test_queue_network_options_hook_recovery_before_player_commands: false,
        pending_apply_requirements_refresh_required: false,
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
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let bootstrap_actions = without_media_match_runtime_snapshots(handle.drain_actions());
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
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: false,
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
        id: SettingId::ChatOutputEnabled,
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
    let refreshed_command_actions = without_media_match_runtime_snapshots(handle.drain_actions());
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
            sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_duration_seconds(93.5)
                .with_size_bytes(734003200),
        );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let local_file_actions = without_media_match_runtime_snapshots(handle.drain_actions());
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
        .push(sorotte_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let paused_actions = without_media_match_runtime_snapshots(handle.drain_actions());
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
                playlist_entry_ids: state
                    .main_window
                    .playlist
                    .iter()
                    .map(|row| row.entry_id)
                    .collect(),
                playlist_source_states: expected_playlist_source_states_for_entries(
                    &state,
                    &["episode1.mkv [93.500s, 734003200 bytes]"],
                    None,
                ),
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
        without_media_match_runtime_snapshots(handle.drain_actions()).is_empty(),
        "idle runtime pumps should not emit redundant player projection actions"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_clears_placeholder_after_media_load_failure() {
    #[derive(Default)]
    struct FailingLoadPlayerAdapter {
        outcomes: Vec<sorotte_player_api::PlayerMediaLoadOutcome>,
    }

    impl PlayerAdapter for FailingLoadPlayerAdapter {
        fn name(&self) -> &'static str {
            "failing-load"
        }

        fn take_media_load_outcome(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerMediaLoadOutcome> {
            self.outcomes.pop()
        }
    }

    let requested_target = "https://cdn.example.com/broken.m3u8".to_owned();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(FailingLoadPlayerAdapter {
        outcomes: vec![sorotte_player_api::PlayerMediaLoadOutcome::failure(
            requested_target.clone(),
            None,
            sorotte_player_api::PlayerMediaLoadFailureKind::Unknown,
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
fn gui_persisted_config_runtime_owner_opens_plex_stream_as_logical_playlist_uri() {
    let playlist_uri = PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "123".to_owned(),
        title: Some("Episode 1".to_owned()),
        file_name: Some("Episode 1.mkv".to_owned()),
        duration_millis: Some(90_000),
        size_bytes: Some(123_456),
        media_type: Some(PlexMediaType::Episode),
    };
    let logical_uri = playlist_uri.to_string();
    let logical_file = sorotte_player_api::LocalFileUpdate::new("Episode 1.mkv")
        .with_path(logical_uri.clone())
        .with_duration_seconds(90.0)
        .with_size_bytes(123_456);
    let stream_target = PlexStreamTarget {
        playlist_uri,
        matched_item: PlexMatchedItem {
            rating_key: "123".to_owned(),
            title: "Episode 1".to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
        logical_file: logical_file.clone(),
        playback_url: SecretPlexPlaybackUrl::new(
            "http://127.0.0.1:32400/library/parts/1/file.mkv?X-Plex-Token=secret-token",
        ),
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let message = owner
        .open_plex_stream_target_through_attached_player_result_impl(
            &logical_uri,
            stream_target,
            true,
        )
        .expect("Plex stream open should have a player")
        .expect("Plex stream open should succeed");

    assert!(message.contains("Episode 1.mkv"));
    assert!(!message.contains("secret-token"));
    assert_eq!(owner.player_local_file, Some(logical_file));
    assert!(!owner.player_local_file_placeholder);
    assert!(owner.pending_logical_media_override.is_some());
    assert_eq!(owner.pending_stream_retry_target, None);
}

#[test]
fn gui_persisted_config_runtime_owner_sets_player_media_titles_for_plex_and_local_opens() {
    #[derive(Debug, Default)]
    struct TitlePlayerState {
        calls: Vec<String>,
    }

    struct TitlePlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TitlePlayerState>>,
    }

    impl PlayerAdapter for TitlePlayerAdapter {
        fn name(&self) -> &'static str {
            "title-recorder"
        }

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .calls
                .push(format!("open:{path}"));
            Ok(())
        }

        fn set_option_string(
            &mut self,
            name: &str,
            value: &str,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .calls
                .push(format!("set:{name}={value}"));
            Ok(())
        }
    }

    let playlist_uri = PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "123".to_owned(),
        title: Some("Metadata Episode Title".to_owned()),
        file_name: Some("Episode 1.mkv".to_owned()),
        duration_millis: Some(90_000),
        size_bytes: Some(123_456),
        media_type: Some(PlexMediaType::Episode),
    };
    let logical_uri = playlist_uri.to_string();
    let logical_file = sorotte_player_api::LocalFileUpdate::new("Episode 1.mkv")
        .with_path(logical_uri.clone())
        .with_duration_seconds(90.0)
        .with_size_bytes(123_456);
    let playback_url = "http://127.0.0.1:32400/library/parts/1/file.mkv?X-Plex-Token=secret-token";
    let stream_target = PlexStreamTarget {
        playlist_uri,
        matched_item: PlexMatchedItem {
            rating_key: "123".to_owned(),
            title: "Matched Episode Title".to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
        logical_file,
        playback_url: SecretPlexPlaybackUrl::new(playback_url),
    };
    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TitlePlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TitlePlayerAdapter {
        state: player_state.clone(),
    })));

    owner
        .open_plex_stream_target_through_attached_player_result_impl(
            &logical_uri,
            stream_target,
            true,
        )
        .expect("Plex stream open should have a player")
        .expect("Plex stream open should succeed");
    owner.open_media_files_through_attached_player_impl(
        &GuiQueuedRuntimeBridgeHandle::default(),
        vec!["C:/media/Local Episode.mkv".to_owned()],
    );

    let calls = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .calls
        .clone();
    assert_eq!(
        calls,
        vec![
            format!("open:{playback_url}"),
            "set:force-media-title=Metadata Episode Title".to_owned(),
            "open:C:/media/Local Episode.mkv".to_owned(),
            "set:force-media-title=Local Episode.mkv".to_owned(),
        ]
    );
    assert!(
        calls
            .iter()
            .filter(|call| call.starts_with("set:force-media-title="))
            .all(|call| !call.contains("secret-token")),
        "mpv media title updates must not expose Plex stream tokens: {calls:?}"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_publishes_plex_stream_logical_file_before_player_metadata() {
    #[derive(Debug, Default)]
    struct DeferredMetadataPlayerState {
        opened_paths: Vec<String>,
    }

    struct DeferredMetadataPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<DeferredMetadataPlayerState>>,
    }

    impl PlayerAdapter for DeferredMetadataPlayerAdapter {
        fn name(&self) -> &'static str {
            "deferred-metadata"
        }

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let playlist_uri = PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "123".to_owned(),
        title: Some("Episode 1".to_owned()),
        file_name: Some("Episode 1.mkv".to_owned()),
        duration_millis: Some(90_000),
        size_bytes: Some(123_456),
        media_type: Some(PlexMediaType::Episode),
    };
    let sparse_logical_uri = "plex://machine-1/metadata/123?title=Episode%201";
    let logical_uri = playlist_uri.to_string();
    let logical_file = sorotte_player_api::LocalFileUpdate::new("Episode 1.mkv")
        .with_path(logical_uri.clone())
        .with_duration_seconds(90.0)
        .with_size_bytes(123_456);
    let playback_url = "http://127.0.0.1:32400/library/parts/1/file.mkv?X-Plex-Token=secret-token";
    let stream_target = PlexStreamTarget {
        playlist_uri,
        matched_item: PlexMatchedItem {
            rating_key: "123".to_owned(),
            title: "Episode 1".to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
        logical_file: logical_file.clone(),
        playback_url: SecretPlexPlaybackUrl::new(playback_url),
    };
    let stored_settings = StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SorotteGuiShellAppState::from_stored_settings(&stored_settings);
    state.apply_shared_playlist_entries(vec![sparse_logical_uri.to_owned()], Some(0), false);
    let player_state =
        std::sync::Arc::new(std::sync::Mutex::new(DeferredMetadataPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        DeferredMetadataPlayerAdapter {
            state: player_state.clone(),
        },
    )));
    owner.active_shared_playlist_index = Some(0);
    owner.player_paused_for_cache = Some(true);
    owner.player_cache_buffering_percent = Some(99.0);
    owner.pending_attached_room_unpause_observation =
        Some(GuiPendingAttachedRoomUnpauseObservation::CachePaused);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    let _ = owner
        .session
        .as_mut()
        .expect("session should exist")
        .flush_outbound_protocol_lines()
        .expect("initial outbound lines should flush");

    owner
        .open_plex_stream_target_through_attached_player_result_impl(
            sparse_logical_uri,
            stream_target,
            false,
        )
        .expect("Plex stream open should have a player")
        .expect("Plex stream open should succeed");

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![playback_url.to_owned()]
    );
    assert_eq!(owner.player_local_file, Some(logical_file.clone()));
    assert!(
        !owner.player_local_file_placeholder,
        "Plex logical file metadata is complete before mpv finishes loading the secret stream URL"
    );
    assert_eq!(owner.player_paused_for_cache, None);
    assert_eq!(owner.player_cache_buffering_percent, None);
    assert!(owner.pending_attached_room_unpause_observation.is_none());
    assert!(
        owner.current_player_matches_media_target(sparse_logical_uri),
        "Plex stream identity should match by machine/rating key even when the opened logical URI has richer query metadata"
    );

    owner
        .sync_detached_session_preferences_and_player_state(&state)
        .expect("detached session sync should publish the Plex logical file");
    let outbound_lines = owner
        .session
        .as_mut()
        .expect("session should exist")
        .flush_outbound_protocol_lines()
        .expect("outbound lines should flush");
    assert!(
        outbound_lines
            .iter()
            .any(|line| line.contains(r#""file""#) && line.contains("Episode 1.mkv")),
        "Plex logical file should publish immediately without waiting for player metadata; outbound_lines={outbound_lines:?}"
    );
    assert!(
        outbound_lines
            .iter()
            .all(|line| !line.contains("secret-token")),
        "Plex stream publication must not expose the playback token"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_plex_stream_logical_identity_across_url_telemetry() {
    #[derive(Debug, Default)]
    struct PlexStreamTelemetryState {
        local_file_updates: std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>,
        playback_updates:
            std::collections::VecDeque<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        media_load_outcomes: std::collections::VecDeque<sorotte_player_api::PlayerMediaLoadOutcome>,
    }

    struct PlexStreamTelemetryAdapter {
        state: std::sync::Arc<std::sync::Mutex<PlexStreamTelemetryState>>,
    }

    impl PlayerAdapter for PlexStreamTelemetryAdapter {
        fn name(&self) -> &'static str {
            "plex-telemetry"
        }

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .local_file_updates
                .push_back(sorotte_player_api::LocalFileUpdate::new(path).with_path(path));
            state.media_load_outcomes.push_back(
                sorotte_player_api::PlayerMediaLoadOutcome::success(path, Some(path.to_owned())),
            );
            state.playback_updates.push_back(
                sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                    .with_position_seconds(0.0),
            );
            Ok(())
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop_front()
        }

        fn take_media_load_outcome(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerMediaLoadOutcome> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .media_load_outcomes
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop_front()
        }
    }

    let playlist_uri = PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "123".to_owned(),
        title: Some("Episode 1".to_owned()),
        file_name: Some("Episode 1.mkv".to_owned()),
        duration_millis: Some(90_000),
        size_bytes: Some(123_456),
        media_type: Some(PlexMediaType::Episode),
    };
    let logical_uri = playlist_uri.to_string();
    let logical_file = sorotte_player_api::LocalFileUpdate::new("Episode 1.mkv")
        .with_path(logical_uri.clone())
        .with_duration_seconds(90.0)
        .with_size_bytes(123_456);
    let loaded_url = "http://plex.local:32400/library/parts/1/file.mkv?X-Plex-Token=secret-token";
    let redirected_url = "https://87-121-73-171.example.plex.direct/library/parts/1/file.mkv?X-Plex-Token=secret-token";
    let stream_target = PlexStreamTarget {
        playlist_uri,
        matched_item: PlexMatchedItem {
            rating_key: "123".to_owned(),
            title: "Episode 1".to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
        logical_file: logical_file.clone(),
        playback_url: SecretPlexPlaybackUrl::new(loaded_url),
    };
    let player_state =
        std::sync::Arc::new(std::sync::Mutex::new(PlexStreamTelemetryState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
        PlexStreamTelemetryAdapter {
            state: player_state.clone(),
        },
    )));

    owner
        .open_plex_stream_target_through_attached_player_result_impl(
            &logical_uri,
            stream_target,
            false,
        )
        .expect("Plex stream open should have a player")
        .expect("Plex stream open should succeed");
    assert_eq!(owner.player_local_file, Some(logical_file.clone()));
    assert_eq!(owner.player_position_seconds, Some(0.0));
    assert!(owner.pending_logical_media_override.is_some());

    owner.player_position_seconds = Some(42.0);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .local_file_updates
        .push_back(
            sorotte_player_api::LocalFileUpdate::new(redirected_url).with_path(redirected_url),
        );
    owner.refresh_player_state_impl();

    assert_eq!(owner.player_local_file, Some(logical_file));
    assert!(!owner.player_local_file_placeholder);
    assert_eq!(
        owner.player_position_seconds,
        Some(42.0),
        "repeated Plex URL telemetry for the active stream should not reset playback"
    );
    assert!(owner.pending_logical_media_override.is_some());
}

#[test]
fn gui_persisted_config_runtime_owner_resets_stale_position_when_the_player_reports_a_new_file() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop_front()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque::from([
            sorotte_player_api::LocalFileUpdate::new("episode2.mkv")
                .with_path("C:/Media/episode2.mkv"),
        ]),
    }));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state,
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv").with_path("C:/Media/episode1.mkv"),
    );
    owner.player_position_seconds = Some(42.0);
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
