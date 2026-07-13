use super::*;

#[test]
fn gui_persisted_config_runtime_owner_marks_local_user_ready_when_attached_player_unpauses() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
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

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }

        fn set_position(
            &mut self,
            _position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
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
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(sorotte_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(false));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":true")),
        "attached-player unpause should queue a local ready update"
    );
    assert_eq!(owner.player_paused, Some(false));
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "python-compatible default unpause handling should not re-pause when no other users block playback"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_marks_local_user_not_ready_when_attached_player_pauses() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
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

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }

        fn set_position(
            &mut self,
            _position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
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
        r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(sorotte_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":false")),
        "attached-player pause should wait for a following runtime pump before clearing readiness"
    );
    assert_eq!(
        owner.player_paused,
        Some(true),
        "local attached-player pause should be remembered while waiting for confirmation"
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":false")),
        "attached-player pause should queue a local not-ready update"
    );
    assert_eq!(
        owner.player_paused,
        Some(true),
        "local attached-player pause should survive until the room playstate catches up"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "stale room pause snapshots should not immediately resume the attached player before the server echo arrives"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_ready_when_attached_player_pauses_for_cache() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
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

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }

        fn set_position(
            &mut self,
            _position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
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
        r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_paused_for_cache(true)
                .with_cache_buffering_percent(50.0),
        );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":false")),
        "mpv cache pause should not queue a local not-ready update"
    );
    assert_eq!(
        owner.player_paused,
        Some(false),
        "mpv cache pause should not overwrite the last user-visible pause state"
    );
    assert_eq!(owner.player_paused_for_cache, Some(true));
    assert_eq!(owner.player_cache_buffering_percent, Some(50.0));
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "mpv cache pause should not trigger a pause correction back into the player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_ready_for_transient_attached_player_startup_pause() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
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
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
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
        r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(sorotte_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":false")),
        "a one-pump attached-player startup pause should not immediately clear readiness"
    );
    assert_eq!(owner.player_paused, Some(true));
    assert!(
        owner
            .pending_attached_player_pause_confirmation_pump
            .is_some(),
        "startup pause should be held pending until the next pump confirms it"
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(sorotte_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(false));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":false")),
        "a resumed attached player should keep the local user ready"
    );
    assert_eq!(owner.player_paused, Some(false));
    assert!(
        owner
            .pending_attached_player_pause_confirmation_pump
            .is_none(),
        "resuming before confirmation should clear the pending startup pause"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "transient startup pause should not force a pause correction back into the player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_ready_when_host_unpauses_controlled_room() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
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
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let room = "+room:ABCDEF123456";
    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", room)
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room.to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        format!(
            r#"{{"Hello":{{"username":"alice","room":{{"name":"{room}"}},"version":"1.7.5","features":{{"chat":true,"readiness":true}}}}}}"#
        ),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(format!(
        r#"{{"Set":{{"user":{{"bob":{{"room":{{"name":"{room}"}},"controller":true}}}}}}}}"#
    ));
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":false")),
        "host-started playback must not clear readiness for an already-ready non-controller"
    );
    assert_eq!(
        owner.player_paused,
        Some(true),
        "legacy room unpause must still use observation-backed player state"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .contains(&false),
        "host-started playback should still resume the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_not_ready_when_host_unpauses_controlled_room() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
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
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let room = "+room:ABCDEF123456";
    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", room)
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room.to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        format!(
            r#"{{"Hello":{{"username":"alice","room":{{"name":"{room}"}},"version":"1.7.5","features":{{"chat":true,"readiness":true}}}}}}"#
        ),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"ready":{"isReady":false,"username":"alice"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(format!(
        r#"{{"Set":{{"user":{{"bob":{{"room":{{"name":"{room}"}},"controller":true}}}}}}}}"#
    ));
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"")),
        "host-started playback must not change readiness for a not-ready non-controller"
    );
    assert_eq!(
        owner.player_paused,
        Some(true),
        "unpause command acceptance must remain pending until player advancement is observed"
    );
    assert!(!state.actual_local_main_window_user_ready());
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .contains(&false),
        "host-started playback should still resume the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_not_ready_when_host_unpauses_uncontrolled_room() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
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
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"ready":{"isReady":false,"username":"alice"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"episode1.mkv"},"isReady":true}}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"")),
        "host-started playback must not mark a not-ready user ready in an uncontrolled room"
    );
    assert_eq!(
        owner.player_paused,
        Some(true),
        "legacy room unpause must still use observation-backed player state"
    );
    assert!(!state.actual_local_main_window_user_ready());
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .contains(&false),
        "host-started playback should still resume the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_ready_when_host_pauses_uncontrolled_room() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
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
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"episode1.mkv"},"isReady":true}}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"")),
        "host-paused playback must not clear readiness in an uncontrolled room"
    );
    assert_eq!(owner.player_paused, Some(true));
    assert!(state.actual_local_main_window_user_ready());
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .contains(&true),
        "host-paused playback should still pause the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_blocks_gui_unpause_when_readiness_gate_fails() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
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
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","duration":95.5},"isReady":false}}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    let toggle_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        !toggle_actions.contains(&GuiShellAction::AnnouncePlaybackResumed),
        "blocked GUI unpause should not announce a local resume"
    );
    assert_eq!(owner.player_paused, Some(true));
    assert!(
        state.main_window.playback_paused,
        "shell state should stay paused when readiness blocks the unpause"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "blocked GUI unpause should not momentarily resume the attached player"
    );

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":true")),
        "blocked GUI unpause should still mark the local user ready"
    );

    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"ready":{"isReady":false,"username":"alice"}}}"#.to_owned(),
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(sorotte_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(false));
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(owner.player_paused, Some(true));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot())
            .and_then(|snapshot| snapshot.pending_local_pause_intent),
        None,
        "a readiness-rejected direct mpv unpause must roll back its provisional core intent"
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![true],
        "a readiness-rejected direct mpv unpause must be restored on the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_emits_immediate_state_update_when_gui_unpause_is_allowed() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
        transport_updates: Vec<sorotte_player_api::PlayerTransportTelemetryUpdate>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_transport_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerTransportTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .transport_updates
                .pop()
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.set_paused_values.push(paused);
            if !paused {
                state.transport_updates.push(attached_transport_update(
                    false,
                    10.0,
                    1.0,
                    sorotte_player_api::PlayerTransportPhase::Playing,
                ));
            }
            Ok(())
        }
    }

    fn attached_transport_update(
        paused: bool,
        position_seconds: f64,
        observed_at_seconds: f64,
        phase: sorotte_player_api::PlayerTransportPhase,
    ) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
        let mut update = sorotte_player_api::PlayerTransportTelemetryUpdate::new(
            sorotte_player_api::PlayerMediaGeneration::new(1),
            sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                std::time::Duration::from_secs_f64(observed_at_seconds),
            ),
        )
        .with_phase(phase)
        .with_position_seconds(position_seconds)
        .with_logical_pause(paused);
        update.paused_for_cache = Some(false);
        update.seeking = Some(false);
        update.seekable = Some(true);
        update.core_idle = Some(paused);
        update
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .prepare_attached_playback_media(
            sorotte_client_core::LogicalMediaId::new("allowed-unpause-local-file")
                .expect("logical media id should be valid"),
            sorotte_client_core::MediaTransportKind::LocalFile,
            sorotte_client_core::MediaLoadIntent::TransportRefresh,
            crate::app::support::system_time_seconds(),
        )
        .expect("attached media preparation should succeed");
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push(attached_transport_update(
            true,
            10.0,
            0.0,
            sorotte_player_api::PlayerTransportPhase::ReadyPaused,
        ));

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","duration":95.5},"isReady":true}}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    let toggle_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        toggle_actions.contains(&GuiShellAction::AnnouncePlaybackResumed),
        "allowed GUI unpause should still announce the local resume"
    );
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(owner.pending_local_attached_pause_override, Some(false));
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false],
        "allowed GUI unpause should resume the attached player exactly once"
    );

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false],
        "an extra pre-echo coordinator pump must not replay stale paused=true"
    );

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":true")),
        "allowed GUI unpause should still mark the local user ready"
    );
    assert!(
        outbound_protocol_lines.iter().any(|line| {
            line.contains("\"State\"")
                && line.contains("\"paused\":false")
                && line.contains("\"position\":10.0")
        }),
        "allowed GUI unpause should emit an immediate paused=false state update"
    );

    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#
            .to_owned(),
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        owner.pending_local_attached_pause_override, None,
        "the canonical echo must retire both core and GUI local-pause ownership"
    );
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push(attached_transport_update(
            false,
            10.25,
            2.0,
            sorotte_player_api::PlayerTransportPhase::Playing,
        ));
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false],
        "the matching echo and continued advancement must not reissue pause"
    );
    assert!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot())
            .is_some_and(|snapshot| !snapshot.ordinary_correction_blocked),
        "continued play after the echo should complete coordinator reconciliation"
    );

    owner.pending_local_attached_pause_override = Some(false);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .prepare_attached_playback_media(
            sorotte_client_core::LogicalMediaId::new("replacement-local-file")
                .expect("replacement logical media id should be valid"),
            sorotte_client_core::MediaTransportKind::LocalFile,
            sorotte_client_core::MediaLoadIntent::NewPlayback,
            crate::app::support::system_time_seconds(),
        )
        .expect("replacement media preparation should succeed");
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert_eq!(
        owner.pending_local_attached_pause_override, None,
        "a new media generation must not inherit stale GUI pause ownership"
    );
}

#[test]
fn gui_direct_mpv_unpause_stages_before_same_pump_media_and_transport_updates() {
    #[derive(Debug, Default)]
    struct DirectTogglePlayerState {
        playback_updates: Vec<sorotte_player_api::PlayerPlaybackTelemetryUpdate>,
        transport_updates: Vec<sorotte_player_api::PlayerTransportTelemetryUpdate>,
        local_file_updates: Vec<sorotte_player_api::LocalFileUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct DirectTogglePlayer {
        state: std::sync::Arc<std::sync::Mutex<DirectTogglePlayerState>>,
    }

    impl PlayerAdapter for DirectTogglePlayer {
        fn name(&self) -> &'static str {
            "direct-toggle"
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

        fn take_transport_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerTransportTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .transport_updates
                .pop()
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop()
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

    fn transport(
        adapter_generation: u64,
        paused: bool,
        position_seconds: f64,
        observed_at_seconds: f64,
    ) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
        let mut update = sorotte_player_api::PlayerTransportTelemetryUpdate::new(
            sorotte_player_api::PlayerMediaGeneration::new(adapter_generation),
            sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                std::time::Duration::from_secs_f64(observed_at_seconds),
            ),
        )
        .with_phase(if paused {
            sorotte_player_api::PlayerTransportPhase::ReadyPaused
        } else {
            sorotte_player_api::PlayerTransportPhase::Playing
        })
        .with_position_seconds(position_seconds)
        .with_logical_pause(paused);
        update.paused_for_cache = Some(false);
        update.seeking = Some(false);
        update.seekable = Some(true);
        update.core_idle = Some(paused);
        update
    }

    let player_state =
        std::sync::Arc::new(std::sync::Mutex::new(DirectTogglePlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(DirectTogglePlayer {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("placeholder.mkv")
            .with_path("C:/Media/placeholder.mkv".to_owned()),
    );
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .prepare_attached_playback_media(
            sorotte_client_core::LogicalMediaId::new("placeholder-media")
                .expect("placeholder logical id should be valid"),
            sorotte_client_core::MediaTransportKind::LocalFile,
            sorotte_client_core::MediaLoadIntent::TransportRefresh,
            crate::app::support::system_time_seconds(),
        )
        .expect("placeholder media should prepare");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("initial paused room state should apply");
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push(transport(1, true, 10.0, 0.0));
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    {
        let mut recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recorded.playback_updates.push(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(10.0),
        );
        recorded.local_file_updates.push(
            sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_path("C:/Media/episode1.mkv".to_owned()),
        );
        recorded
            .transport_updates
            .push(transport(2, false, 10.0, 1.0));
    }

    owner.refresh_player_state_impl();
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot())
            .and_then(|snapshot| snapshot.pending_local_pause_intent),
        Some(false),
        "the direct mpv toggle must be restaged after same-pump media preparation"
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "stale canonical pause must not be reissued after direct mpv play"
    );

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("direct play echo should apply");
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert_eq!(owner.pending_local_attached_pause_override, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push(transport(2, false, 10.25, 2.0));
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot())
            .is_some_and(|snapshot| !snapshot.ordinary_correction_blocked)
    );

    let controlled_player_state =
        std::sync::Arc::new(std::sync::Mutex::new(DirectTogglePlayerState::default()));
    let (mut controlled_owner, _controlled_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_session_runtime("alice", "+room:ABCDEF123456")
            .expect("controlled client-core runtime owner should bootstrap");
    controlled_owner.player = Some(GuiOwnedPlayer::Custom(Box::new(DirectTogglePlayer {
        state: controlled_player_state.clone(),
    })));
    controlled_owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("controlled.mkv")
            .with_path("C:/Media/controlled.mkv".to_owned()),
    );
    controlled_owner.player_paused = Some(true);
    controlled_owner.player_position_seconds = Some(10.0);
    let controlled_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("+room:ABCDEF123456".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    controlled_owner
        .session
        .as_mut()
        .expect("controlled session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true}}}"#,
        )
        .expect("controlled-room hello should apply");
    controlled_owner
        .session
        .as_mut()
        .expect("controlled session should exist")
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":false}}}}"#,
        )
        .expect("non-controller projection should apply");
    controlled_owner
        .session
        .as_mut()
        .expect("controlled session should exist")
        .prepare_attached_playback_media(
            sorotte_client_core::LogicalMediaId::new("controlled-noncontroller-media")
                .expect("controlled logical id should be valid"),
            sorotte_client_core::MediaTransportKind::LocalFile,
            sorotte_client_core::MediaLoadIntent::TransportRefresh,
            crate::app::support::system_time_seconds(),
        )
        .expect("controlled media should prepare");
    controlled_owner
        .session
        .as_mut()
        .expect("controlled session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("controlled-room canonical pause should apply");
    {
        let mut recorded = controlled_player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recorded
            .transport_updates
            .push(transport(1, true, 10.0, 0.0));
    }
    controlled_owner.refresh_player_state_impl();
    controlled_owner.sync_session_playstate_to_attached_player_impl(&controlled_state, false);
    controlled_owner.pending_attached_player_pause_command = None;
    {
        let mut recorded = controlled_player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recorded.set_paused_values.clear();
        recorded.playback_updates.push(
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(10.0),
        );
        recorded
            .transport_updates
            .push(transport(1, false, 10.0, 1.0));
    }

    controlled_owner.refresh_player_state_impl();
    assert_eq!(
        controlled_owner.pending_local_attached_pause_override, None,
        "the GUI compatibility flag must mirror core rejection for a non-controller"
    );
    controlled_owner.sync_session_playstate_to_attached_player_impl(&controlled_state, false);
    assert!(
        controlled_player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .contains(&true),
        "a direct mpv play by a non-controller must be returned to canonical pause"
    );
}

#[test]
fn gui_autoplay_runtime_unpause_stages_before_attached_transport_echo() {
    #[derive(Debug, Default)]
    struct AutoplayPlayerState {
        transport_updates: Vec<sorotte_player_api::PlayerTransportTelemetryUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct AutoplayPlayer {
        state: std::sync::Arc<std::sync::Mutex<AutoplayPlayerState>>,
    }

    impl PlayerAdapter for AutoplayPlayer {
        fn name(&self) -> &'static str {
            "autoplay"
        }

        fn take_transport_telemetry_update(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerTransportTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .transport_updates
                .pop()
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.set_paused_values.push(paused);
            if !paused {
                state
                    .transport_updates
                    .push(autoplay_transport(false, 10.0, 1.0));
            }
            Ok(())
        }
    }

    fn autoplay_transport(
        paused: bool,
        position_seconds: f64,
        observed_at_seconds: f64,
    ) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
        let mut update = sorotte_player_api::PlayerTransportTelemetryUpdate::new(
            sorotte_player_api::PlayerMediaGeneration::new(1),
            sorotte_player_api::PlayerObservationTimestamp::from_adapter_start(
                std::time::Duration::from_secs_f64(observed_at_seconds),
            ),
        )
        .with_phase(if paused {
            sorotte_player_api::PlayerTransportPhase::ReadyPaused
        } else {
            sorotte_player_api::PlayerTransportPhase::Playing
        })
        .with_position_seconds(position_seconds)
        .with_logical_pause(paused);
        update.paused_for_cache = Some(false);
        update.seeking = Some(false);
        update.seekable = Some(true);
        update.core_idle = Some(paused);
        update
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(AutoplayPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(AutoplayPlayer {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let session = owner.session.as_mut().expect("session should exist");
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    session
        .prepare_attached_playback_media(
            sorotte_client_core::LogicalMediaId::new("autoplay-media")
                .expect("logical media id should be valid"),
            sorotte_client_core::MediaTransportKind::LocalFile,
            sorotte_client_core::MediaLoadIntent::TransportRefresh,
            crate::app::support::system_time_seconds(),
        )
        .expect("autoplay media should prepare");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("paused room state should apply");
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .transport_updates
        .push(autoplay_transport(true, 10.0, 0.0));
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .set_playback_paused(false)
        .expect("autoplay should optimistically unpause the local runtime");
    assert!(owner.apply_attached_player_runtime_actions_impl(
        vec![GuiAttachedPlayerRuntimeAction::Paused(false)],
        "autoplay runtime",
    ));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot())
            .and_then(|snapshot| snapshot.pending_local_pause_intent),
        Some(false),
        "the local runtime action must stage its own pause intent"
    );

    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false],
        "coordinator reconciliation must not undo autoplay before its server echo"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_commit_runtime_unpause_when_player_resume_fails() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        resume_attempts: usize,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            if !paused {
                self.state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .resume_attempts += 1;
                return Err(sorotte_player_api::PlayerError::OperationFailed(
                    "resume failed".to_owned(),
                ));
            }
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","duration":95.5},"isReady":true}}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    let toggle_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        !toggle_actions.contains(&GuiShellAction::AnnouncePlaybackResumed),
        "failed GUI unpause should not announce a local resume"
    );
    assert_eq!(owner.player_paused, Some(true));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_pause_state()),
        Some(true),
        "the detached runtime should stay paused when the physical resume fails"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_username()),
        Some("alice")
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .resume_attempts,
        1,
        "the attached player should still receive one resume attempt"
    );

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":true")),
        "a failed player resume must not optimistically mark the local user ready"
    );
    assert!(
        !outbound_protocol_lines.iter().any(|line| {
            line.contains("\"State\"")
                && line.contains("\"paused\":false")
                && line.contains("\"position\":10.0")
        }),
        "a failed player resume must not emit a paused=false heartbeat"
    );
}
