use super::*;

#[test]
fn gui_persisted_config_runtime_owner_pins_active_settings_but_keeps_explicit_controls_live() {
    #[derive(Debug, Default)]
    struct RecordingDetachedSessionState {
        runtime_settings:
            Vec<sorotte_client_app::app_boundary::state::StoredClientSettingsRuntimeSnapshot>,
        autoplay_enabled: Vec<bool>,
        autoplay_thresholds: Vec<usize>,
    }

    struct RecordingDetachedSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingDetachedSessionState>>,
    }

    impl GuiSessionRuntimeAdapter for RecordingDetachedSessionRuntimeAdapter {
        fn sync_runtime_settings(
            &mut self,
            runtime_settings: &sorotte_client_app::app_boundary::state::StoredClientSettingsRuntimeSnapshot,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .runtime_settings
                .push(runtime_settings.clone());
            Ok(())
        }

        fn sync_local_playback_telemetry(
            &mut self,
            _paused: Option<bool>,
            _position_seconds: Option<f64>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn set_autoplay_enabled(&mut self, enabled: bool) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .autoplay_enabled
                .push(enabled);
            Ok(())
        }

        fn set_autoplay_threshold(&mut self, threshold: usize) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .autoplay_thresholds
                .push(threshold);
            Ok(())
        }

        fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            _selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            _current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(Vec::new())
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    let recorded = std::sync::Arc::new(std::sync::Mutex::new(
        RecordingDetachedSessionState::default(),
    ));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(RecordingDetachedSessionRuntimeAdapter {
            state: recorded.clone(),
        }),
    );
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(12.5);

    let state_a = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        autoplay_initial_state: Some(true),
        dont_slow_down_with_me: Some(false),
        loop_single_files: Some(true),
        rewind_on_desync: Some(false),
        unpause_action: Some(sorotte_client_core::UnpauseActionMode::IfOthersReady),
        autoplay_min_users: Some(
            sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(3),
        ),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .sync_detached_session_preferences_and_player_state(&state_a)
        .expect("first detached-session preference sync should succeed");

    let state_b = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        autoplay_initial_state: Some(false),
        dont_slow_down_with_me: Some(true),
        loop_single_files: Some(false),
        rewind_on_desync: Some(true),
        unpause_action: Some(sorotte_client_core::UnpauseActionMode::Always),
        autoplay_min_users: Some(
            sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(5),
        ),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .sync_detached_session_preferences_and_player_state(&state_b)
        .expect("second detached-session preference sync should succeed");

    let mut recorded_state = recorded
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(recorded_state.runtime_settings.len(), 2);
    assert_eq!(
        recorded_state.runtime_settings[0]
            .settings
            .dont_slow_down_with_me,
        Some(false)
    );
    assert_eq!(
        recorded_state.runtime_settings[0]
            .settings
            .loop_single_files,
        Some(true)
    );
    assert_eq!(
        recorded_state.runtime_settings[0].settings.rewind_on_desync,
        Some(false)
    );
    assert_eq!(
        recorded_state.runtime_settings[0].settings.unpause_action,
        Some(sorotte_client_core::UnpauseActionMode::IfOthersReady)
    );
    assert_eq!(
        recorded_state.runtime_settings[1]
            .settings
            .dont_slow_down_with_me,
        Some(false)
    );
    assert_eq!(
        recorded_state.runtime_settings[1]
            .settings
            .loop_single_files,
        Some(true)
    );
    assert_eq!(
        recorded_state.runtime_settings[1].settings.rewind_on_desync,
        Some(false)
    );
    assert_eq!(
        recorded_state.runtime_settings[1].settings.unpause_action,
        Some(sorotte_client_core::UnpauseActionMode::IfOthersReady)
    );
    assert_eq!(
        recorded_state.autoplay_enabled,
        vec![
            state_a.main_window.autoplay_active,
            state_a.main_window.autoplay_active,
        ]
    );
    assert_eq!(
        recorded_state.autoplay_thresholds,
        vec![
            state_a.main_window.autoplay_threshold,
            state_a.main_window.autoplay_threshold,
        ]
    );
    recorded_state.autoplay_enabled.clear();
    recorded_state.autoplay_thresholds.clear();
    drop(recorded_state);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    handle.push_request(GuiRuntimeRequest::SetAutoplayEnabled(
        state_b.main_window.autoplay_active,
    ));
    handle.push_request(GuiRuntimeRequest::SetAutoplayThreshold(
        state_b.main_window.autoplay_threshold,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state_b);
    owner
        .sync_detached_session_preferences_and_player_state(&state_a)
        .expect("explicit autoplay controls should survive the next active-session sync");

    let recorded_state = recorded
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        recorded_state.autoplay_enabled.last().copied(),
        Some(state_b.main_window.autoplay_active)
    );
    assert_eq!(
        recorded_state.autoplay_thresholds.last().copied(),
        Some(state_b.main_window.autoplay_threshold)
    );
    let last_runtime_settings = recorded_state
        .runtime_settings
        .last()
        .expect("a post-command active-session sync should be recorded");
    assert_eq!(
        last_runtime_settings.settings.autoplay_initial_state,
        Some(state_b.main_window.autoplay_active)
    );
    assert_eq!(
        last_runtime_settings.settings.autoplay_min_users,
        Some(
            sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(
                state_b.main_window.autoplay_threshold,
            )
        )
    );
    drop(recorded_state);
    assert!(owner.active_session_settings.is_some());
    owner.remove_session_runtime();
    assert!(owner.active_session_settings.is_none());
}

#[test]
fn gui_persisted_config_runtime_owner_clamps_detached_session_position_to_file_duration() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>,
        playback_updates:
            std::collections::VecDeque<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
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

    #[derive(Debug, Default)]
    struct RecordingSessionState {
        synced_playback: Vec<(Option<bool>, Option<f64>)>,
    }

    struct RecordingSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingSessionState>>,
    }

    impl GuiSessionRuntimeAdapter for RecordingSessionRuntimeAdapter {
        fn sync_local_playback_telemetry(
            &mut self,
            paused: Option<bool>,
            position_seconds: Option<f64>,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .synced_playback
                .push((paused, position_seconds));
            Ok(())
        }

        fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            _selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            _current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(Vec::new())
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState::default()));
    {
        let mut player_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        player_state.local_file_updates.push_back(
            sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_path("C:/Media/episode1.mkv".to_owned())
                .with_duration_seconds(1510.0),
        );
    }

    let recorded = std::sync::Arc::new(std::sync::Mutex::new(RecordingSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(RecordingSessionRuntimeAdapter {
            state: recorded.clone(),
        }),
    );
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push_back(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(1511.0),
        );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    assert_eq!(owner.player_position_seconds, Some(1510.0));
    let recorded = recorded
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        !recorded.synced_playback.is_empty(),
        "detached-session sync should receive playback telemetry"
    );
    assert_eq!(
        recorded.synced_playback.last().copied(),
        Some((Some(false), Some(1510.0))),
        "the latest detached-session sync should reflect the clamped end-of-file position"
    );
    assert!(
        recorded
            .synced_playback
            .iter()
            .all(|(_, position_seconds)| {
                position_seconds
                    .map(|position_seconds| position_seconds <= 1510.0)
                    .unwrap_or(true)
            }),
        "detached-session sync should never see a position beyond the known media duration"
    );
}
