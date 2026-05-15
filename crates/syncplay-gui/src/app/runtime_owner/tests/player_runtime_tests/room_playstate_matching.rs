use super::*;

#[test]
fn gui_persisted_config_runtime_owner_skips_self_origin_room_position_sync_for_attached_player() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
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

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
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
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(41.0);

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        .apply_message_json(
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("self-origin room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, true);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "force-sync should not replay the local user's own room position back into the attached player"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "force-sync should not replay the local user's own room pause state back into the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_ignores_unattributed_room_playstate_when_no_remote_users_are_known()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
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
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
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
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(41.0);
    owner.player_paused = Some(false);

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        .apply_message_json(r#"{"State":{"playstate":{"position":0.0,"paused":true}}}"#)
        .expect("unattributed room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "room playstate without remote authority should not rewind the attached player while alone"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "room playstate without remote authority should not pause the attached player while alone"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_local_file_before_applying_room_playstate() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
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
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
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

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(recorded.set_positions.is_empty());
        assert!(recorded.set_paused_values.is_empty());
    }
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded
            .set_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < 1.0),
        "room playstate should seek once the attached player reports a local file"
    );
    assert_eq!(recorded.set_paused_values, vec![true]);
}

#[test]
fn gui_persisted_config_runtime_owner_retries_room_unpause_after_cache_pause_release() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
        set_positions: Vec<f64>,
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
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
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
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(3.0);
    owner.player_paused = Some(false);
    owner.player_paused_for_cache = Some(true);

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":30.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room seek should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let mut recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !recorded.set_positions.is_empty(),
            "room seek should still be applied while cache pause defers unpause"
        );
        assert!(
            recorded.set_paused_values.is_empty(),
            "cache pause should defer room unpause"
        );
        recorded.set_positions.clear();
    }
    assert!(owner.pending_attached_cache_unpause);
    assert_eq!(owner.player_paused, Some(false));

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused_for_cache(false),
        );
    owner.refresh_player_state_impl();
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":34.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("post-cache room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        recorded.set_paused_values,
        vec![false],
        "cache release should retry room unpause even though logical player state was already unpaused"
    );
    assert!(
        !owner.pending_attached_cache_unpause,
        "successful post-cache unpause should clear the pending retry"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_pause_state()),
        Some(false),
        "post-cache unpause correction should not become a manual pause"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_force_room_sync_for_matched_playlist_target_without_reset_intent()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
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
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let root = test_temp_root("matched-playlist-target-no-reset");
    let media_path = root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("playlist target fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(media_path.to_string_lossy().into_owned()),
    );
    owner.player_position_seconds = Some(0.0);
    owner.player_paused = Some(false);

    let stored_settings = StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        rewind_on_desync: Some(false),
        fastforward_on_desync: Some(false),
        slow_on_desync: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&stored_settings);
    state.apply_shared_playlist_entries(vec!["episode1.mkv".to_owned()], Some(0), false);
    owner.active_shared_playlist_index = Some(0);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_runtime_settings(&stored_client_settings_runtime_snapshot_legacy_compatible(
            &stored_settings,
        ))
        .expect("runtime settings should sync");

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
        .apply_message_json(
            r#"{"State":{"playstate":{"position":41.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let mut recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recorded.set_positions.clear();
        recorded.set_paused_values.clear();
    }
    owner.player_position_seconds = Some(42.0);

    let selected_media_sync =
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(
        selected_media_sync,
        SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
    );

    let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
    );
    assert!(
        !selection_handoff_ready,
        "matched playlist targets without a pending reset should not force a room playstate handoff"
    );

    owner.apply_pending_playlist_index_reset_to_attached_player_impl(
        &state,
        selection_handoff_ready,
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, selection_handoff_ready);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "playlist updates that keep the current target selected should not rewind the attached player; recorded={recorded:?}"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "playlist updates that keep the current target selected should not toggle pause state; recorded={recorded:?}"
    );
    assert_eq!(owner.player_position_seconds, Some(42.0));

    let _ = std::fs::remove_dir_all(&root);
}
