use super::*;
use crate::app::testing::support::runtime_state_for_shell;

#[test]
fn gui_persisted_config_runtime_owner_pins_active_settings_but_keeps_explicit_controls_live() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(crate::app::runtime_stack::test_support::active_session()),
    );
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(12.5);
    let state_a = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        autoplay_initial_state: Some(true),
        dont_slow_down_with_me: Some(false),
        loop_single_files: Some(true),
        rewind_on_desync: Some(false),
        unpause_action: Some(sorotte_client_core::UnpauseActionMode::IfOthersReady),
        autoplay_min_users: Some(
            sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(3),
        ),
        ..StoredClientSettings::default()
    });
    owner
        .sync_detached_session_preferences_and_player_state(&runtime_state_for_shell(&state_a))
        .unwrap();
    let settings_a = owner
        .session
        .as_ref()
        .unwrap()
        .configured_settings_for_test()
        .clone();
    let state_b = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        autoplay_initial_state: Some(false),
        dont_slow_down_with_me: Some(true),
        loop_single_files: Some(false),
        rewind_on_desync: Some(true),
        unpause_action: Some(sorotte_client_core::UnpauseActionMode::Always),
        autoplay_min_users: Some(
            sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(5),
        ),
        ..StoredClientSettings::default()
    });
    owner
        .sync_detached_session_preferences_and_player_state(&runtime_state_for_shell(&state_b))
        .unwrap();
    let session = owner.session.as_ref().unwrap();
    assert_eq!(
        session.configured_settings_for_test(),
        &settings_a,
        "an unsaved draft must not change the active session's settings"
    );
    assert_eq!(settings_a.dont_slow_down_with_me, Some(false));
    assert_eq!(settings_a.loop_single_files, Some(true));
    assert_eq!(settings_a.rewind_on_desync, Some(false));
    assert_eq!(
        settings_a.unpause_action,
        Some(sorotte_client_core::UnpauseActionMode::IfOthersReady)
    );
    assert_eq!(
        session.runtime.session().autoplay_enabled(),
        state_a.main_window.autoplay_active
    );
    assert_eq!(
        session
            .runtime
            .session()
            .readiness_autoplay_config()
            .auto_play_threshold,
        Some(3)
    );
    assert_eq!(session.local_position_seconds(), Some(12.5));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    handle.push_request(GuiRuntimeRequest::SetAutoplayEnabled(
        state_b.main_window.autoplay_active,
    ));
    handle.push_request(GuiRuntimeRequest::SetAutoplayThreshold(
        state_b.main_window.autoplay_threshold,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state_b);
    owner
        .sync_detached_session_preferences_and_player_state(&runtime_state_for_shell(&state_a))
        .unwrap();
    let session = owner.session.as_ref().unwrap();
    assert_eq!(
        session.runtime.session().autoplay_enabled(),
        state_b.main_window.autoplay_active
    );
    assert_eq!(
        session
            .runtime
            .session()
            .readiness_autoplay_config()
            .auto_play_threshold,
        Some(5)
    );
    assert_eq!(
        session
            .configured_settings_for_test()
            .autoplay_initial_state,
        Some(false)
    );
    assert_eq!(
        session.configured_settings_for_test().autoplay_min_users,
        Some(sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(5))
    );
    assert!(owner.active_session_settings.is_some());
    owner.remove_session_runtime();
    assert!(owner.active_session_settings.is_none());
}

#[test]
fn gui_persisted_config_runtime_owner_clamps_detached_session_position_to_file_duration() {
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
        synced_playback: Vec<(Option<bool>, Option<f64>)>,
    }

    struct SessionObservationProbe {
        state: std::sync::Arc<std::sync::Mutex<RecordingSessionState>>,
    }

    impl SessionObservationProbe {
        fn into_session(self) -> crate::app::GuiClientSession {
            crate::app::runtime_stack::test_support::active_session().with_observer(move |event| {
            if let crate::app::runtime_stack::test_support::SessionObservation::PlaybackObserved { paused, position } = event {
                self.state.lock().unwrap().synced_playback.push((paused, position));
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
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
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
                .with_paused(false)
                .with_position_seconds(1511.0),
        ));
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
