use super::*;
use crate::app::runtime_owner::player::SelectedPlaylistMediaSyncOutcome;
use crate::app::testing::support::runtime_state_for_shell;
use sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot;

#[test]
fn gui_persisted_config_runtime_owner_auto_advances_shared_playlist_once_at_eof() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        events: Option<ScriptedPlayerEvents>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn take_player_event_batch(&mut self) -> Option<sorotte_player_api::PlayerEventBatch> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_ref()
                .and_then(ScriptedPlayerEvents::peek)
        }
        fn acknowledge_player_event_batch(
            &mut self,
            token: sorotte_player_api::PlayerEventAcknowledgementToken,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_mut()
                .expect("scripted ingress")
                .acknowledge(token)
        }
    }

    #[derive(Debug, Default)]
    struct RecordingSessionState {
        advance_calls: usize,
        pause_intent_stages: usize,
        pause_dispatches: Vec<bool>,
        eof_observations: usize,
    }

    struct SessionObservationProbe {
        state: std::sync::Arc<std::sync::Mutex<RecordingSessionState>>,
    }

    impl SessionObservationProbe {
        fn into_session(self) -> crate::app::GuiClientSession {
            let mut session = crate::app::runtime_stack::test_support::active_session();
            session.apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"},"playlistIndex":{"index":0,"user":"bob"}}}"#).unwrap();
            session
                .apply_message_json(
                    r#"{"State":{"playstate":{"position":0.0,"paused":false,"setBy":"bob"}}}"#,
                )
                .unwrap();
            session.with_observer(move |event| {
                use crate::app::runtime_stack::test_support::SessionObservation::*;
                let mut state = self.state.lock().unwrap();
                match event {
                    PlaylistAdvance => state.advance_calls += 1,
                    EndOfFile => state.eof_observations += 1,
                    PauseIntentStaged(_) => state.pause_intent_stages += 1,
                    PauseRequested(paused) => state.pause_dispatches.push(paused),
                    _ => {}
                }
            })
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState {
        events: Some(active_player_events(1)),
    }));
    {
        let mut player_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        player_state
            .events
            .as_mut()
            .expect("scripted physical observations")
            .push_event(file_event(
                1,
                sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                    .with_path("C:/Media/episode1.mkv".to_owned())
                    .with_duration_seconds(1510.0),
            ));
    }

    let recorded = std::sync::Arc::new(std::sync::Mutex::new(RecordingSessionState::default()));
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(Box::new(
            SessionObservationProbe {
                state: recorded.clone(),
            }
            .into_session(),
        ));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let active_settings = StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    };
    owner.active_session_settings = Some(stored_client_settings_runtime_snapshot(&active_settings));
    let mut state = SorotteGuiShellAppState::from_stored_settings(&active_settings);
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: false,
    }));
    assert!(
        !state.main_window.shared_playlist_enabled,
        "the test must exercise an unsaved draft value opposite to the active session"
    );

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        let _ = state.apply(action);
    }
    assert!(
        state.main_window.shared_playlist_enabled,
        "runtime projection must restore the active session's enabled playlist state"
    );
    assert!(
        state.main_window.playback.can_manage_playlist,
        "runtime command availability must follow the active session rather than the draft"
    );
    assert!(
        state.main_window.playlist.len() == 2,
        "an unsaved disable must not replace the session playlist with player-local media"
    );
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .events
        .as_mut()
        .expect("scripted physical observations")
        .push_event(playback_event(
            1,
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(1510.0),
        ));
    player_state
        .lock()
        .unwrap()
        .events
        .as_mut()
        .unwrap()
        .push_event(sorotte_player_api::PlayerEvent::LogicalPlaybackTerminal {
            attempt_id: sorotte_player_api::LoadAttemptId::new(1),
            media_generation: sorotte_player_api::PlayerMediaGeneration::new(1),
            outcome: sorotte_player_api::PlayerPhysicalLoadOutcome::Ended,
        });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    assert_eq!(
        recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .advance_calls,
        1,
        "EOF should trigger one playlist advance when the player pauses at the file end"
    );
    {
        let recorded = recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(recorded.eof_observations, 1);
        assert_eq!(
            recorded.pause_intent_stages, 0,
            "natural EOF must not be staged as a direct player gesture"
        );
        assert!(
            recorded.pause_dispatches.is_empty(),
            "natural EOF must not be sent through the user pause mutation seam"
        );
    }

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    assert_eq!(
        recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .advance_calls,
        1,
        "the EOF auto-advance should stay latched until the player leaves the end-of-file state"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_pins_playlist_target_lookup_to_active_settings() {
    let enabled_settings = StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.session_projects_to_shell = true;
    owner.active_session_settings =
        Some(stored_client_settings_runtime_snapshot(&enabled_settings));
    owner.active_shared_playlist_index = Some(0);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&enabled_settings);
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: false,
    }));
    assert_eq!(
        owner
            .current_shared_playlist_target(&runtime_state_for_shell(&state))
            .as_deref(),
        Some("episode.mkv"),
        "an unsaved disable must not hide the active session playlist target"
    );

    let disabled_settings = StoredClientSettings {
        shared_playlist_enabled: Some(false),
        ..StoredClientSettings::default()
    };
    owner.active_session_settings =
        Some(stored_client_settings_runtime_snapshot(&disabled_settings));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::PlaybackSharedPlaylists,
        value: true,
    }));
    assert_eq!(
        owner.current_shared_playlist_target(&runtime_state_for_shell(&state)),
        None,
        "an unsaved enable must not activate playlist lookup for a disabled session"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_ready_when_opening_auto_advanced_playlist_item() {
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    fn ready_state_from_protocol_line(line: &str) -> Option<bool> {
        let message = serde_json::from_str::<serde_json::Value>(line).ok()?;
        message.get("Set")?.get("ready")?.get("isReady")?.as_bool()
    }

    let mut session = crate::app::GuiClientSession::new("alice", "room1");
    let _ = session
        .deliver_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready state should apply");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let root = test_temp_root("auto-advanced-playlist-ready-preserve");
    let episode_two_path = root.join("episode2.mkv");
    std::fs::write(&episode_two_path, b"test")
        .expect("auto-advance ready preservation fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.active_shared_playlist_index = Some(0);
    owner.playlist_auto_advance_eof_latched = true;
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .seed_playlist_reset_intent_for_test(true);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    state.apply_shared_playlist_entries(
        vec![episode_two_path.to_string_lossy().into_owned()],
        Some(0),
        false,
    );
    let opened = owner.sync_selected_shared_playlist_media_to_attached_player_impl(
        &runtime_state_for_shell(&state),
    );

    assert_eq!(opened, SelectedPlaylistMediaSyncOutcome::StartedLoading);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![episode_two_path.to_string_lossy().into_owned()]
    );
    let open_lines = owner
        .session
        .as_mut()
        .expect("session should exist")
        .deliver_outbound_protocol_lines()
        .expect("open protocol lines should encode");
    assert!(
        open_lines
            .iter()
            .all(|line| ready_state_from_protocol_line(line) != Some(false)),
        "opening the auto-advanced playlist item should not cancel readiness/autoplay; lines={open_lines:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_applies_autoplay_unpause_to_attached_player_without_remote_playstate()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        applied_pauses: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_pauses
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut session = crate::app::runtime_stack::test_support::active_session();
    session
        .sync_local_playback_telemetry(Some(true), Some(0.0))
        .unwrap();
    session
        .sync_runtime_settings(&stored_client_settings_runtime_snapshot(
            &StoredClientSettings {
                username: Some("alice".to_owned()),
                room: Some("room1".to_owned()),
                autoplay_initial_state: Some(true),
                autoplay_min_users: Some(
                    sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(1),
                ),
                ..Default::default()
            },
        ))
        .unwrap();
    let mut config = session
        .runtime
        .session()
        .readiness_autoplay_config()
        .clone();
    config.autoplay_delay_seconds = 0.0;
    session
        .runtime
        .session_mut()
        .set_readiness_autoplay_config(config);
    session.apply_message_json(r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"episode2.mkv"},"isReady":true}}}}"#).unwrap();
    session.force_autoplay_tick_for_test();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_path("C:/Media/episode2.mkv".to_owned()),
    );
    owner.player_local_file_placeholder = false;
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(0.0);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    let _ = owner
        .session
        .as_mut()
        .unwrap()
        .drain_gui_actions(&runtime_state_for_shell(&state));
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .applied_pauses,
        vec![false],
        "client-core autoplay unpause must be applied to the attached player even before a remote room playstate exists"
    );
    assert_eq!(
        (
            owner.session.as_ref().unwrap().local_pause_state(),
            owner.session.as_ref().unwrap().local_position_seconds()
        ),
        (Some(false), Some(0.0)),
        "the applied local autoplay unpause should be mirrored back into session telemetry"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_auto_loops_single_item_shared_playlist_at_eof() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        events: Option<ScriptedPlayerEvents>,
        applied_pauses: Vec<bool>,
        applied_positions: Vec<f64>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_pauses
                .push(paused);
            Ok(())
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_positions
                .push(position_seconds);
            Ok(())
        }

        fn take_player_event_batch(&mut self) -> Option<sorotte_player_api::PlayerEventBatch> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_ref()
                .and_then(ScriptedPlayerEvents::peek)
        }
        fn acknowledge_player_event_batch(
            &mut self,
            token: sorotte_player_api::PlayerEventAcknowledgementToken,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_mut()
                .expect("scripted ingress")
                .acknowledge(token)
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState {
        events: Some(active_player_events(1)),
        ..Default::default()
    }));
    {
        let mut player_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        player_state
            .events
            .as_mut()
            .expect("scripted physical observations")
            .push_event(file_event(
                1,
                sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                    .with_path("C:/Media/episode1.mkv".to_owned())
                    .with_duration_seconds(1510.0),
            ));
        player_state
            .events
            .as_mut()
            .expect("scripted physical observations")
            .push_event(playback_event(
                1,
                sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(false)
                    .with_position_seconds(0.0),
            ));
    }

    let mut session = crate::app::GuiClientSession::new("alice", "room1");
    let startup_lines = session
        .deliver_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":1510.0}}}}}"#,
        )
        .expect("local file update should apply");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        loop_single_files: Some(true),
        ..StoredClientSettings::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .events
        .as_mut()
        .expect("scripted physical observations")
        .push_event(playback_event(
            1,
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(1510.0),
        ));
    player_state
        .lock()
        .unwrap()
        .events
        .as_mut()
        .unwrap()
        .push_event(sorotte_player_api::PlayerEvent::LogicalPlaybackTerminal {
            attempt_id: sorotte_player_api::LoadAttemptId::new(1),
            media_generation: sorotte_player_api::PlayerMediaGeneration::new(1),
            outcome: sorotte_player_api::PlayerPhysicalLoadOutcome::Ended,
        });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        recorded.applied_positions,
        vec![0.0],
        "single-item loop EOF should rewind the attached player"
    );
    assert_eq!(
        recorded.applied_pauses,
        vec![false],
        "single-item loop EOF should resume the attached player after rewinding"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_auto_loops_single_item_shared_playlist_at_eof_with_offset() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        events: Option<ScriptedPlayerEvents>,
        applied_pauses: Vec<bool>,
        applied_positions: Vec<f64>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_pauses
                .push(paused);
            Ok(())
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .applied_positions
                .push(position_seconds);
            Ok(())
        }

        fn take_player_event_batch(&mut self) -> Option<sorotte_player_api::PlayerEventBatch> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_ref()
                .and_then(ScriptedPlayerEvents::peek)
        }
        fn acknowledge_player_event_batch(
            &mut self,
            token: sorotte_player_api::PlayerEventAcknowledgementToken,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_mut()
                .expect("scripted ingress")
                .acknowledge(token)
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState {
        events: Some(active_player_events(1)),
        ..Default::default()
    }));
    {
        let mut player_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        player_state
            .events
            .as_mut()
            .expect("scripted physical observations")
            .push_event(file_event(
                1,
                sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                    .with_path("C:/Media/episode1.mkv".to_owned())
                    .with_duration_seconds(1510.0),
            ));
        player_state
            .events
            .as_mut()
            .expect("scripted physical observations")
            .push_event(playback_event(
                1,
                sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(false)
                    .with_position_seconds(0.0),
            ));
    }

    let mut session = crate::app::GuiClientSession::new("alice", "room1");
    let startup_lines = session
        .deliver_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("playlist index should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":1510.0}}}}}"#,
        )
        .expect("local file update should apply");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.user_offset_seconds = 5.0;

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        loop_single_files: Some(true),
        ..StoredClientSettings::default()
    });

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .events
        .as_mut()
        .expect("scripted physical observations")
        .push_event(playback_event(
            1,
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(1510.0),
        ));
    player_state
        .lock()
        .unwrap()
        .events
        .as_mut()
        .unwrap()
        .push_event(sorotte_player_api::PlayerEvent::LogicalPlaybackTerminal {
            attempt_id: sorotte_player_api::LoadAttemptId::new(1),
            media_generation: sorotte_player_api::PlayerMediaGeneration::new(1),
            outcome: sorotte_player_api::PlayerPhysicalLoadOutcome::Ended,
        });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        recorded.applied_positions,
        vec![5.0],
        "single-item loop EOF should rewind the attached player on the offset-adjusted timeline"
    );
    assert_eq!(
        recorded.applied_pauses,
        vec![false],
        "single-item loop EOF should still resume the attached player after rewinding"
    );
}
