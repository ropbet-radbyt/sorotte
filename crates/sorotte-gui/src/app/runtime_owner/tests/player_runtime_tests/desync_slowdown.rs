use super::*;

#[test]
fn gui_persisted_config_runtime_owner_applies_desync_slowdown_when_room_playstate_is_unchanged() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_playback_rates: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_playback_rate(&mut self, rate: f64) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_playback_rates
                .push(rate);
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
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

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
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_local_playback_telemetry(Some(false), Some(10.0))
        .expect("initial local telemetry should sync");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_playback_rates
        .clear();

    owner.player_position_seconds = Some(12.0);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_local_playback_telemetry(Some(false), Some(12.0))
        .expect("desynced local telemetry should sync");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        recorded.set_playback_rates,
        vec![0.95],
        "steady-state attached-player sync should still apply slowdown corrections while playback continues"
    );
}

#[test]
fn slowdown_osd_follows_the_active_session_setting() {
    for (language, show_slowdown_osd, expected_slowdown_message, expected_restore_message) in [
        (
            None,
            true,
            Some("Slowing playback to synchronize with the room."),
            Some("Restoring normal playback speed."),
        ),
        (
            Some("fr"),
            true,
            Some("Ralentissement de la lecture pour synchroniser avec la salle."),
            Some("Retour a la vitesse de lecture normale."),
        ),
        (None, false, None, None),
    ] {
        let settings = StoredClientSettingsMvp {
            language: language.map(str::to_owned),
            show_slowdown_osd: Some(show_slowdown_osd),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.active_session_settings = Some(
            sorotte_client_app::app_boundary::state::
                stored_client_settings_runtime_snapshot_legacy_compatible(&settings),
        );
        owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(
            sorotte_player_mpv::SimulatedPlayer::new().into_inner(),
        )));

        owner.apply_attached_player_runtime_actions_impl(
            vec![GuiAttachedPlayerRuntimeAction::PlaybackRate(0.95)],
            "desync slowdown",
        );

        let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
            panic!("slowdown OSD fixture should retain its simulated mpv adapter");
        };
        assert_eq!(
            player
                .last_simulated_legacy_syncplay_osd_message()
                .map(|(message, _)| message.as_str()),
            expected_slowdown_message,
            "slowdown OSD emission must be gated by the frozen active-session setting"
        );

        owner.apply_attached_player_runtime_actions_impl(
            vec![GuiAttachedPlayerRuntimeAction::PlaybackRate(1.0)],
            "desync speed restore",
        );
        let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
            panic!("slowdown OSD fixture should retain its simulated mpv adapter");
        };
        assert_eq!(
            player
                .last_simulated_legacy_syncplay_osd_message()
                .map(|(message, _)| message.as_str()),
            expected_restore_message,
            "normal-speed restoration must follow the same active-session OSD setting"
        );
    }
}
