use super::*;
use crate::app::runtime_owner::GuiUpdateRuntime;

#[test]
fn gui_persisted_config_runtime_owner_keeps_offset_commands_on_global_timeline() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
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
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner {
        config_path: None,
        legacy_projection: None,
        session: None,
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
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_local_file_placeholder: false,
        last_published_local_file: None,
        last_published_media_match_signature: None,
        local_shared_playlist_media_match_signature_path: None,
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
        pending_attached_cache_unpause: false,
        pending_attached_player_pause_confirmation_pump: None,
        pending_attached_player_pause_command: None,
        player_position_seconds: Some(100.0),
        player_paused: Some(false),
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
        plex_playlist_search_rx: None,
        plex_playlist_resolve_rx: None,
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
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    handle.push_request(GuiRuntimeRequest::SetOffset(
        sorotte_client_app::app_boundary::commands::LocalOffsetCommand::Absolute(5.0),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let _ = handle.drain_actions();
    assert_eq!(owner.user_offset_seconds, 5.0);
    assert_eq!(
        owner.player_position_seconds,
        Some(100.0),
        "changing offset should not rewrite the stored global position"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(100.0),
        "offset changes should keep detached-session telemetry on the global timeline"
    );

    handle.push_request(GuiRuntimeRequest::SetOffset(
        sorotte_client_app::app_boundary::commands::LocalOffsetCommand::Absolute(7.0),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let _ = handle.drain_actions();
    assert_eq!(owner.user_offset_seconds, 7.0);
    assert_eq!(owner.player_position_seconds, Some(100.0));

    handle.push_request(GuiRuntimeRequest::SeekToPosition(42.0));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let _ = handle.drain_actions();

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions,
        vec![105.0, 107.0, 49.0],
        "offset commands should target player-local time, while global seeks add the active offset only once"
    );
    assert_eq!(
        owner.player_position_seconds,
        Some(42.0),
        "global seek state should remain offset-free after attached-player requests"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(42.0),
        "detached-session seek history should record the global seek target rather than the shifted player position"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_allows_offset_changes_without_a_player() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_position_seconds = Some(100.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        state.main_window.playback.can_set_offset,
        "offset controls should stay available even without an attached player"
    );

    handle.push_request(GuiRuntimeRequest::SetOffset(
        sorotte_client_app::app_boundary::commands::LocalOffsetCommand::Absolute(5.0),
    ));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(owner.user_offset_seconds, 5.0);
    assert_eq!(
        owner.player_position_seconds,
        Some(100.0),
        "offset changes without a player should preserve the stored global position"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(100.0),
        "offset changes without a player should still keep detached-session telemetry on the global timeline"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message.contains("offset")
        )),
        "offset changes without a player should still report success"
    );
    assert!(
        !actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Error
                    && message.contains("offset")
        )),
        "offset changes without a player should not surface a runtime-unavailable error"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_suppresses_attached_seeks_after_recent_rewind() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
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
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(2.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    {
        let session = owner.session.as_mut().expect("session should exist");
        session
            .sync_local_playback_telemetry(Some(false), Some(2.0))
            .expect("initial local telemetry should sync");
        session.note_local_playlist_index_reset_intent(true);
    }

    handle.push_request(GuiRuntimeRequest::SeekToPosition(10.0));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        !actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if (*level == GuiTransientNotificationLevel::Success
                    || *level == GuiTransientNotificationLevel::Error)
                    && message.contains("seek")
        )),
        "recent-rewind seek suppression should not emit a seek success or error notification"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions
            .is_empty(),
        "recent-rewind seek suppression should prevent the attached player seek"
    );
    assert_eq!(
        owner.player_position_seconds,
        Some(2.0),
        "suppressed attached seeks should leave the stored global position unchanged"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(2.0),
        "suppressed attached seeks should not advance detached-session telemetry"
    );
}
