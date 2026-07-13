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
}

#[test]
fn gui_persisted_config_runtime_owner_emits_immediate_state_update_when_gui_unpause_is_allowed() {
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
