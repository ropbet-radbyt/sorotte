use super::*;
use crate::app::runtime_owner::GuiUpdateRuntime;
use sorotte_client_core::{LogicalMediaId, MediaLoadIntent, MediaTransportKind};
use sorotte_player_api::{
    PlayerMediaGeneration, PlayerObservationTimestamp, PlayerSeekableRange, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};
use sorotte_protocol::{
    PlaybackBarrierPhase, PlaybackBarrierPolicy, PlaybackBarrierSetExtension,
    PlaybackBarrierStatusPayload, PlaystatePayload, PrepareMediaPayload, ProtocolMessage,
    SetPayload, StatePayload, decode_message_line_items, encode_message_line,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

#[derive(Debug, Default)]
struct OffsetTimelinePlayerState {
    transport_updates: VecDeque<PlayerTransportTelemetryUpdate>,
    set_positions: Vec<f64>,
}

struct OffsetTimelinePlayer {
    state: std::sync::Arc<std::sync::Mutex<OffsetTimelinePlayerState>>,
}

impl PlayerAdapter for OffsetTimelinePlayer {
    fn name(&self) -> &'static str {
        "offset-timeline"
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

    fn take_transport_telemetry_update(&mut self) -> Option<PlayerTransportTelemetryUpdate> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .transport_updates
            .pop_front()
    }
}

fn offset_transport(
    observed_at_seconds: f64,
    phase: PlayerTransportPhase,
    player_position_seconds: f64,
    logical_pause: bool,
    seekable_range: PlayerSeekableRange,
) -> PlayerTransportTelemetryUpdate {
    let mut update = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(
            observed_at_seconds,
        )),
    )
    .with_phase(phase)
    .with_position_seconds(player_position_seconds)
    .with_logical_pause(logical_pause);
    update.paused_for_cache = Some(phase == PlayerTransportPhase::Rebuffering);
    update.seeking = Some(false);
    update.seekable = Some(true);
    update.seekable_ranges = Some(vec![seekable_range]);
    update.core_idle = Some(phase == PlayerTransportPhase::ReadyPaused);
    update.playback_restart_sequence = Some(1);
    update
}

fn apply_offset_test_protocol(
    session: &mut dyn GuiSessionRuntimeAdapter,
    message: ProtocolMessage,
) {
    let line = encode_message_line(&message).expect("offset test protocol frame should encode");
    session
        .apply_message_json(&line)
        .expect("offset test protocol frame should apply through the real GUI adapter");
}

fn offset_test_owner(
    offset_seconds: f64,
    media_kind: MediaTransportKind,
) -> (
    GuiPersistedConfigRuntimeOwner,
    std::sync::Arc<std::sync::Mutex<OffsetTimelinePlayerState>>,
    SorotteGuiShellAppState,
) {
    let player_state =
        std::sync::Arc::new(std::sync::Mutex::new(OffsetTimelinePlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("real client-core GUI session should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OffsetTimelinePlayer {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("offset-episode.mkv")
            .with_path("C:/Media/offset-episode.mkv".to_owned()),
    );
    owner.player_position_seconds = None;
    owner.player_paused = None;
    owner.user_offset_seconds = offset_seconds;
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let session = owner.session.as_mut().expect("GUI session should exist");
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("barrier-aware server Hello should apply");
    session
        .prepare_attached_playback_media(
            LogicalMediaId::new("sha256:gui-offset-integration")
                .expect("logical media ID should be valid"),
            media_kind,
            MediaLoadIntent::NewPlayback,
            100.0,
        )
        .expect("GUI media preparation should succeed");
    (owner, player_state, state)
}

#[test]
fn real_gui_positive_offset_normalizes_barrier_readiness_on_the_room_timeline() {
    let (mut owner, player_state, state) = offset_test_owner(5.0, MediaTransportKind::NetworkVod);
    let session = owner.session.as_mut().expect("GUI session should exist");
    apply_offset_test_protocol(
        session.as_mut(),
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("alice"),
            ),
        ),
    );
    apply_offset_test_protocol(
        session.as_mut(),
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_prepare(
                        PrepareMediaPayload::new(
                            51,
                            "sha256:gui-offset-integration",
                            10.0,
                            PlaybackBarrierPolicy::Controller,
                        )
                        .with_request_nonce(1),
                    )
                    .with_status(PlaybackBarrierStatusPayload {
                        media_generation: 51,
                        state_revision: None,
                        phase: PlaybackBarrierPhase::Preparing,
                        policy: PlaybackBarrierPolicy::Controller,
                        quorum: None,
                        deadline: 120.0,
                        participants: BTreeMap::new(),
                        excluded_legacy_clients: BTreeSet::new(),
                    }),
            ),
        ),
    );
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push_back(offset_transport(
            1.0,
            PlayerTransportPhase::ReadyPaused,
            15.0,
            true,
            PlayerSeekableRange::new(15.0, 25.0),
        ));

    owner.refresh_player_state_impl();
    assert_eq!(
        owner.player_position_seconds,
        Some(10.0),
        "positive-offset player telemetry must be stored on the room timeline"
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let outbound = owner
        .session
        .as_mut()
        .expect("GUI session should exist")
        .flush_outbound_protocol_lines()
        .expect("GUI outbox should encode");
    let ready = outbound
        .into_iter()
        .flat_map(|line| {
            decode_message_line_items(&line)
                .expect("GUI outbox line should decode")
                .into_iter()
        })
        .filter_map(|item| item.message.ok())
        .find_map(|message| match message {
            ProtocolMessage::State(state) => state
                .state
                .playback_barrier_v1()
                .expect("barrier State extension should decode")
                .and_then(|extension| extension.ready),
            _ => None,
        })
        .expect("room-timeline ReadyPaused observation should emit MediaReady");
    assert_eq!(ready.media_generation, 51);
    assert!(ready.loaded);
    assert!(ready.buffer_ready);
    assert_eq!(ready.seekable, Some(true));
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions
            .is_empty(),
        "an offset-normalized observation already at the barrier target must not be re-sought"
    );
}

#[test]
fn real_gui_negative_offset_normalizes_recovery_and_shifts_coordinator_seek_once() {
    let (mut owner, player_state, state) = offset_test_owner(-5.0, MediaTransportKind::LiveSliding);
    apply_offset_test_protocol(
        owner
            .session
            .as_mut()
            .expect("GUI session should exist")
            .as_mut(),
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("bob"),
            ),
        ),
    );
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push_back(offset_transport(
            1.0,
            PlayerTransportPhase::Playing,
            5.0,
            false,
            PlayerSeekableRange::new(0.0, 16.0),
        ));
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert_eq!(
        owner.player_position_seconds,
        Some(10.0),
        "negative-offset normal playback telemetry must be stored globally"
    );
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_positions
        .clear();

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push_back(offset_transport(
            2.0,
            PlayerTransportPhase::Rebuffering,
            5.0,
            false,
            PlayerSeekableRange::new(0.0, 16.0),
        ));
    owner.refresh_player_state_impl();
    assert_eq!(owner.player_position_seconds, Some(10.0));
    assert!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot())
            .and_then(|snapshot| snapshot.recovery_episode)
            .is_some(),
        "the normalized rebuffer observation should enter coordinator-owned recovery"
    );

    apply_offset_test_protocol(
        owner
            .session
            .as_mut()
            .expect("GUI session should exist")
            .as_mut(),
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(20.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("bob"),
            ),
        ),
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions
            .is_empty(),
        "Rebuffering must block the forced correction until transport becomes safe"
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push_back(offset_transport(
            3.0,
            PlayerTransportPhase::ReadyPaused,
            5.0,
            true,
            // On the room timeline this player-local [0, 16] window is
            // [5, 21]. The live-edge safety clamp therefore permits the
            // requested global 20s target only when the range is shifted.
            PlayerSeekableRange::new(0.0, 16.0),
        ));
    owner.refresh_player_state_impl();

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions,
        vec![15.0],
        "the coordinator's global 20s seek must cross the GUI boundary as player-local 15s exactly once"
    );
}

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
        pending_attached_room_unpause_observation: None,
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
