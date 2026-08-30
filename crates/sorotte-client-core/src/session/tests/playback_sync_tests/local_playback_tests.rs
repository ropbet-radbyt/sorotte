use super::*;
use crate::LocalPauseChangeHealth;

#[test]
fn determine_local_state_change_uses_aged_room_position_for_seek_detection() {
    let mut session = ClientSession::default();
    session.model.room.name = Some("room1".to_owned());
    session.model.room.playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(0.0),
            paused: Some(false),
            do_seek: Some(false),
            set_by: Some("bob".to_owned()),
        },
    );
    session.model.room.playstate_updated_at_seconds.insert(
        "room1".to_owned(),
        unix_wall_clock_time_seconds_legacy_compatible() - 1.15,
    );
    session.model.playback.local_position = Some(0.0);
    session.model.playback.local_paused = Some(false);

    let (pause_change, seeked) = session.determine_local_state_change(false, 1.2);

    assert!(!pause_change);
    assert!(
        !seeked,
        "seek detection should compare against the room position at the current time instead of the stale stored snapshot"
    );
}

#[test]
fn reconcile_state_response_uses_override_room_position_for_seek_detection() {
    let mut session = ClientSession::default();
    session.model.room.name = Some("room1".to_owned());
    session.model.room.playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(10.0),
            paused: Some(false),
            do_seek: Some(false),
            set_by: Some("bob".to_owned()),
        },
    );
    session.model.playback.local_position = Some(10.0);
    session.model.playback.local_paused = Some(false);

    let response = session.reconcile_state_and_build_response_with_local_state_change_override(
        StatePayload::new(),
        11.2,
        false,
        100.0,
        0.25,
        Some(RoomPlaystateView {
            position: Some(11.3),
            paused: Some(false),
            do_seek: Some(false),
            set_by: Some("bob".to_owned()),
        }),
    );

    assert_eq!(
        response
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.do_seek),
        None,
        "an override room position that already includes forward delay should suppress spurious seek classification"
    );
}

#[test]
fn desync_correction_ignores_threshold_actions_on_do_seek_messages() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

    let action = session.evaluate_desync_correction(0.0, 6.0, false, false, true);
    assert_eq!(action, DesyncCorrectionAction::None);
}

#[test]
fn runtime_actions_for_desync_correction_do_seek_resets_fastforward_detection_window() {
    let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

    let step1 = session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
    assert_eq!(
        step1,
        Vec::<ClientRuntimeAction>::new(),
        "initial behind detection should only start the fastforward timer"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("doSeek state update should apply");
    let step2 = session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
    assert_eq!(
        step2,
        Vec::<ClientRuntimeAction>::new(),
        "doSeek updates should suppress desync correction and reset behind detection timing"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-doSeek state update should apply");
    let step3 = session.runtime_actions_for_desync_correction(4.1, 0.0, false, false, true);
    assert_eq!(
        step3,
        Vec::<ClientRuntimeAction>::new(),
        "after doSeek clears, fastforward detection window should restart from this point"
    );

    let step4 = session.runtime_actions_for_desync_correction(7.3, 0.0, false, false, true);
    assert_eq!(
        step4,
        Vec::<ClientRuntimeAction>::new(),
        "restarted fastforward window should not trigger before the threshold duration elapses again"
    );

    let step5 = session.runtime_actions_for_desync_correction(7.5, 0.0, false, false, true);
    assert_eq!(
        step5,
        vec![ClientRuntimeAction::SetPosition(10.25)],
        "fastforward should retrigger only after the post-doSeek detection window fully elapses"
    );
}

#[test]
fn client_runtime_room_pause_sync_seeks_before_pausing_remote_pause() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":12.5,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("remote paused state should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(3.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_room_pause_sync_if_needed()
        .expect("room pause sync should dispatch");

    assert_eq!(
        runtime.player().position,
        Some(12.5),
        "remote pause sync should seek to the room position before pausing"
    );
    assert_eq!(
        runtime.player().paused,
        Some(true),
        "remote pause sync should pause after seeking"
    );
    assert_eq!(
        runtime.session().model.playback.local_position,
        Some(12.5),
        "room pause sync should optimistically mirror the corrected position"
    );
    assert_eq!(runtime.session().model.playback.local_paused, Some(true));
}

#[test]
fn client_runtime_room_pause_sync_applies_remote_seek_without_pause_change() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":12.5,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("remote seek state should apply");
    let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
    session
        .model
        .room
        .playstate_updated_at_seconds
        .insert("room1".to_owned(), now_seconds - 2.0);

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(3.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_room_pause_sync_if_needed()
        .expect("room playstate sync should dispatch");

    let synced_position = runtime
        .player()
        .position
        .expect("remote seek should issue a player seek");
    assert!(
        synced_position > 14.0,
        "remote doSeek should use the aged room playstate instead of the stale stored snapshot"
    );
    assert_eq!(
        runtime.player().paused,
        None,
        "remote doSeek without a pause mismatch should not send an extra pause action"
    );
    assert_eq!(
        runtime.session().model.playback.local_position,
        Some(synced_position),
        "room playstate sync should optimistically mirror the corrected seek target"
    );
    assert_eq!(runtime.session().model.playback.local_paused, Some(false));
}

#[test]
fn client_runtime_room_pause_sync_does_not_seek_or_replay_unpause_on_cache_release() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready state should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":30.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("remote seek state should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(3.0)
                .with_paused(true)
                .with_paused_for_cache(true),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_room_pause_sync_if_needed()
        .expect("remote seek should dispatch while cache pause defers unpause");

    let initial_seek_position = runtime
        .player()
        .position
        .expect("the explicit remote doSeek should still dispatch once");

    assert_eq!(
        runtime.player().paused,
        None,
        "cache pause should defer room unpause while buffering"
    );
    assert!(
        runtime
            .session()
            .model
            .playback
            .pending_cache_room_playstate_resync,
        "cache pause should retain the desired room playstate for observation-based recovery"
    );
    assert_eq!(
        runtime.session().user_ready("alice"),
        Some(true),
        "cache pause must not clear readiness"
    );

    runtime
            .session_mut_for_test()
            .apply_message_json(
                r#"{"State":{"playstate":{"position":34.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-seek room playstate should apply");
    runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update =
        Some(PlayerPlaybackTelemetryUpdate::default().with_paused_for_cache(false));

    runtime
        .run_room_pause_sync_if_needed()
        .expect("cache release observation should not fail");

    assert_eq!(
        runtime.player().position,
        Some(initial_seek_position),
        "cache release alone must not issue a second seek to the advancing room position"
    );
    assert_eq!(
        runtime.player().paused,
        None,
        "cache release alone must not replay an unpause command"
    );
    assert!(
        runtime
            .session()
            .model
            .playback
            .pending_cache_room_playstate_resync,
        "desired play must remain pending after command acceptance and cache release"
    );

    runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update = Some(
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(initial_seek_position)
            .with_paused(false),
    );
    runtime
        .run_room_pause_sync_if_needed()
        .expect("first post-cache position observation should not fail");
    assert!(
        runtime
            .session()
            .model
            .playback
            .pending_cache_room_playstate_resync,
        "a matching pause property and one stationary position sample are not playback advancement"
    );

    runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update = Some(
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(initial_seek_position + 0.25)
            .with_paused(false),
    );
    runtime
        .run_room_pause_sync_if_needed()
        .expect("advancing post-cache position observation should not fail");
    assert!(
        !runtime
            .session()
            .model
            .playback
            .pending_cache_room_playstate_resync,
        "desired play may be acknowledged after observed forward advancement"
    );
    assert_eq!(
        runtime.player().position,
        Some(initial_seek_position),
        "closing recovery from observations must not issue another seek"
    );
    assert_eq!(
        runtime.player().paused,
        None,
        "closing recovery from observations must not issue an unpause command"
    );
    assert_eq!(
        runtime.session().user_ready("alice"),
        Some(true),
        "cache recovery must not mark the user not-ready"
    );
}

#[test]
fn client_runtime_room_pause_sync_does_not_mirror_failed_seek_corrections() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":12.5,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("remote seek state should apply");

    let player = RecordingPlayer {
        fail_set_position: true,
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(3.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let error = runtime
        .run_room_pause_sync_if_needed()
        .expect_err("room playstate sync should surface player seek failures");

    assert_eq!(error, PlayerError::Unsupported("set_position_failed"));
    assert_eq!(
        runtime.session().model.playback.local_position,
        Some(3.0),
        "failed seek corrections should leave the last confirmed local telemetry position intact"
    );
    assert_eq!(
        runtime.session().model.playback.local_paused,
        Some(false),
        "failed seek corrections should not mark the session as corrected"
    );
}

#[test]
fn client_runtime_room_pause_sync_rolls_back_seek_when_pause_fails() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":12.5,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("remote paused state should apply");

    let player = RecordingPlayer {
        fail_set_paused: true,
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(3.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let error = runtime
        .run_room_pause_sync_if_needed()
        .expect_err("pause failure should be surfaced after compensation");

    assert_eq!(error, PlayerError::Unsupported("set_paused_failed"));
    assert_eq!(
        runtime.player().player_effects,
        vec![
            ClientEffect::SetPlayerPosition(12.5),
            ClientEffect::SetPlayerPaused(true),
            ClientEffect::SetPlayerPosition(3.0),
        ],
        "the reducer should compensate the successful seek after the pause failure"
    );
    assert_eq!(runtime.player().position, Some(3.0));
    assert_eq!(runtime.session().model.playback.local_position, Some(3.0));
    assert_eq!(runtime.session().model.playback.local_paused, Some(false));
}

#[test]
fn client_runtime_toggle_pause_dispatches_player_state_updates() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_toggle_pause()
            .expect("toggle pause should not fail"),
        "toggle pause should emit a local SetPaused action"
    );
    assert_eq!(runtime.player().paused, Some(false));
    assert!(
        runtime
            .run_toggle_pause()
            .expect("toggle pause should not fail"),
        "toggle pause should emit a second local SetPaused action"
    );
    assert_eq!(runtime.player().paused, Some(true));
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "local pause toggles should not directly emit protocol lines"
    );
}

#[test]
fn client_runtime_set_paused_dispatches_only_when_state_changes() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_set_paused(true)
            .expect("setting the default paused state should not fail"),
        "setting paused when the default state is already treated as paused should be suppressed"
    );
    assert_eq!(runtime.player().paused, None);
    assert!(
        runtime
            .run_set_paused(false)
            .expect("resuming should not fail"),
        "changing away from the default paused state should emit a local SetPaused action"
    );
    assert_eq!(runtime.player().paused, Some(false));
    assert_eq!(runtime.session().local_paused(), Some(false));
    assert!(
        !runtime
            .run_set_paused(false)
            .expect("setting the same paused state should not fail"),
        "setting the same paused state should be suppressed"
    );
    assert_eq!(runtime.player().paused, Some(false));
    assert!(
        runtime
            .run_set_paused(true)
            .expect("pausing should not fail"),
        "setting paused after an explicit unpause should emit a local SetPaused action"
    );
    assert_eq!(runtime.player().paused, Some(true));
    assert_eq!(runtime.session().local_paused(), Some(true));
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "local pause changes should not directly emit protocol lines"
    );
}

#[test]
fn client_runtime_set_paused_retains_intent_when_player_pause_fails() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    session.model.playback.local_paused = Some(false);

    let player = RecordingPlayer {
        fail_set_paused: true,
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let error = runtime
        .run_set_paused(true)
        .expect_err("pause failures should surface back to the caller");

    assert_eq!(error, PlayerError::Unsupported("set_paused_failed"));
    assert_eq!(
        runtime.session().local_paused(),
        Some(false),
        "failed pause requests should restore the last confirmed local pause state"
    );
    assert_eq!(
        runtime.session().user_ready("alice"),
        Some(false),
        "failed physical pause must not roll back deliberate Not Ready intent"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 1);
}

#[derive(Debug, Default)]
struct FailFirstReadyEffectSink {
    attempted_effects: Vec<ClientEffect>,
    remaining_ready_failures: usize,
}

impl ClientEffectSink for FailFirstReadyEffectSink {
    fn emit(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
        self.attempted_effects.push(effect.clone());
        if matches!(effect, ClientEffect::SetReady { .. }) && self.remaining_ready_failures > 0 {
            self.remaining_ready_failures -= 1;
            return Err(ClientEffectError::OperationFailed(
                "forced ready delivery failure".to_owned(),
            ));
        }
        Ok(())
    }
}

#[test]
fn client_runtime_pause_keeps_player_truth_when_following_ready_effect_fails() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    session.model.playback.local_paused = Some(false);

    let player = RecordingPlayer::default();
    let control = FailFirstReadyEffectSink {
        remaining_ready_failures: 1,
        ..FailFirstReadyEffectSink::default()
    };
    let mut runtime = ClientRuntime::new(session, player, control);

    let error = runtime
        .run_set_paused(true)
        .expect_err("the failed ready effect should surface after the player pause succeeds");

    assert!(matches!(error, PlayerError::OperationFailed(_)));
    assert_eq!(runtime.player().paused, Some(true));
    assert_eq!(runtime.session().local_paused(), Some(true));
    assert_eq!(
        runtime.session().user_ready("alice"),
        Some(false),
        "an undelivered readiness operation must retain its semantic intent"
    );
    assert_eq!(
        runtime.session().local_pause_change_health(),
        LocalPauseChangeHealth::ControlEffectFailedAfterPlayerChange
    );
    assert!(!runtime.session().model.local_pause_change_in_flight());
    assert_eq!(
        runtime.control().attempted_effects,
        vec![ClientEffect::SetReady {
            ready: false,
            manually_initiated: true,
        }]
    );

    assert!(
        runtime
            .run_set_paused(false)
            .expect("a later successful player change should recover pause health")
    );
    assert_eq!(runtime.player().paused, Some(false));
    assert_eq!(runtime.session().local_paused(), Some(false));
    assert_eq!(
        runtime.session().local_pause_change_health(),
        LocalPauseChangeHealth::Healthy
    );
}

#[test]
fn client_runtime_noncontroller_host_unpause_does_not_clear_existing_ready_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"readiness":true}}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
        )
        .expect("controller update should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("host unpause should apply");
    session.model.playback.local_paused = Some(true);

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_system_owned_pause(false)
        .expect("host-driven unpause should dispatch to the player");

    assert_eq!(runtime.player().paused, Some(false));
    assert_eq!(runtime.session().local_paused(), Some(false));
    assert_eq!(
        runtime.session().user_ready("alice"),
        Some(true),
        "host-driven unpause must preserve an already-ready non-controller"
    );
    assert!(
        runtime
            .control()
            .outbound_messages()
            .iter()
            .all(|message| match message {
                ProtocolMessage::Set(set_message) =>
                    set_message
                        .set
                        .ready
                        .as_ref()
                        .and_then(|ready| ready.is_ready)
                        != Some(false),
                _ => true,
            }),
        "host-driven unpause must not queue isReady=false"
    );
}

#[test]
fn client_runtime_noncontroller_host_unpause_does_not_set_not_ready_user_ready() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"readiness":true}}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":false,"username":"alice"}}}"#)
        .expect("local not-ready state should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
        )
        .expect("controller update should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("host unpause should apply");
    session.model.playback.local_paused = Some(true);

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_system_owned_pause(false)
        .expect("host-driven unpause should dispatch to the player");

    assert_eq!(runtime.player().paused, Some(false));
    assert_eq!(runtime.session().local_paused(), Some(false));
    assert_eq!(
        runtime.session().user_ready("alice"),
        Some(false),
        "host-driven unpause must preserve a not-ready non-controller"
    );
    assert!(
        runtime
            .control()
            .outbound_messages()
            .iter()
            .all(|message| match message {
                ProtocolMessage::Set(set_message) => set_message.set.ready.is_none(),
                _ => true,
            }),
        "host-driven unpause must not queue any ready update"
    );
}

#[test]
fn client_runtime_seek_to_position_dispatches_player_position_updates() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_seek_to_position(42.5)
            .expect("seek-to should not fail"),
        "seek-to should emit a local SetPosition action"
    );
    assert_eq!(runtime.player().position, Some(42.5));
    assert_eq!(runtime.session().local_position_seconds(), Some(42.5));
    assert_eq!(
        runtime.session().last_seek_position_before_manual_seek(),
        Some(0.0)
    );
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "local seek should not directly emit protocol lines"
    );
}

#[test]
fn client_runtime_seek_to_position_clamps_negative_targets_to_zero() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_seek_to_position(-3.0)
            .expect("negative seek should not fail"),
        "negative seek targets should still emit a clamped local SetPosition action"
    );
    assert_eq!(runtime.player().position, Some(0.0));
    assert_eq!(runtime.session().local_position_seconds(), Some(0.0));
    assert_eq!(
        runtime.session().last_seek_position_before_manual_seek(),
        Some(0.0)
    );
}

#[test]
fn client_runtime_seek_to_position_suppresses_recent_rewind_stale_seek() {
    let mut session = ClientSession::default();
    session.model.playback.last_rewound_at_seconds =
        Some(unix_wall_clock_time_seconds_legacy_compatible());

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        !runtime
            .run_seek_to_position(10.0)
            .expect("recent-rewind seek suppression should not fail"),
        "late seeks beyond the rewind guard threshold should be ignored right after a rewind"
    );
    assert_eq!(runtime.player().position, None);
    assert_eq!(runtime.session().local_position_seconds(), None);
    assert_eq!(
        runtime.session().last_seek_position_before_manual_seek(),
        None
    );
}

#[test]
fn client_runtime_seek_to_position_restores_session_state_when_player_seek_fails() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session.model.playback.local_position = Some(2.0);
    session.model.playlist.last_seek_position_before_manual_seek = Some(1.0);

    let player = RecordingPlayer {
        fail_set_position: true,
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let error = runtime
        .run_seek_to_position(9.0)
        .expect_err("seek failures should surface back to the caller");

    assert_eq!(error, PlayerError::Unsupported("set_position_failed"));
    assert_eq!(
        runtime.session().local_position_seconds(),
        Some(2.0),
        "failed seek requests should restore the previous local position snapshot"
    );
    assert_eq!(
        runtime.session().last_seek_position_before_manual_seek(),
        Some(1.0),
        "failed seek requests should restore the previous seek history too"
    );
    assert_eq!(
        runtime.session().model.playback.client_ignoring_on_the_fly,
        0,
        "failed player seeks must not retain an unsent ignore counter"
    );
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "server authority must not move when the physical player seek fails"
    );
}

#[test]
fn client_runtime_seek_by_offset_uses_global_position_when_available() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_seek_by_offset(2.25)
            .expect("seek-by should not fail"),
        "seek-by should emit a local SetPosition action"
    );
    assert_eq!(runtime.player().position, Some(12.25));
    assert_eq!(
        runtime.control().outbound_messages().len(),
        1,
        "an active local seek must immediately publish canonical server intent"
    );
    let ProtocolMessage::State(state) = &runtime.control().outbound_messages()[0] else {
        panic!("active local seek should queue a State message");
    };
    let playstate = state
        .state
        .playstate
        .as_ref()
        .expect("active local seek should include playstate");
    assert_eq!(playstate.position, Some(12.25));
    assert_eq!(playstate.paused, Some(false));
    assert_eq!(playstate.do_seek, Some(true));
    assert_eq!(playstate.set_by, None);
    assert_eq!(
        state
            .state
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|ignore| ignore.client),
        Some(1),
        "the explicit seek must use the same client-ignore handshake as inferred seeks"
    );
}

#[test]
fn client_runtime_seek_by_offset_falls_back_to_last_local_seek_position() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_seek_to_position(5.0)
        .expect("initial seek should not fail");
    assert!(
        runtime
            .run_seek_by_offset(3.0)
            .expect("seek-by should not fail"),
        "seek-by should emit a local SetPosition action"
    );
    assert_eq!(runtime.player().position, Some(8.0));
}

#[test]
fn client_runtime_undo_seek_is_omitted_without_seek_history() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime.run_undo_seek().expect("undo seek should not fail"),
        "undo seek should be suppressed when no previous seek position is available"
    );
    assert_eq!(runtime.player().position, None);
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_undo_seek_toggles_between_current_and_previous_positions() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    runtime
        .run_seek_to_position(10.0)
        .expect("initial seek should not fail");
    assert_eq!(runtime.player().position, Some(10.0));

    assert!(
        runtime.run_undo_seek().expect("undo seek should not fail"),
        "undo seek should emit a local SetPosition action"
    );
    assert_eq!(runtime.player().position, Some(0.0));

    assert!(
        runtime.run_undo_seek().expect("undo seek should not fail"),
        "second undo seek should toggle to previous position"
    );
    assert_eq!(runtime.player().position, Some(10.0));
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_toggle_pause_pre_syncs_pending_telemetry_and_preserves_drain() {
    let mut session = ClientSession::default();
    session.model.playback.local_paused = Some(true);
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default().with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_toggle_pause()
            .expect("toggle pause should pre-sync pending telemetry"),
        "toggle pause should emit a local SetPaused action"
    );
    assert_eq!(
        runtime.player().paused,
        Some(true),
        "toggle should invert telemetry-confirmed paused=false, not stale local_paused=true"
    );

    let drained = runtime.drain_player_playback_telemetry_updates();
    assert_eq!(
        drained,
        vec![PlayerPlaybackTelemetryUpdate::default().with_paused(false)]
    );
}

#[test]
fn client_runtime_seek_by_offset_pre_syncs_pending_telemetry_position() {
    let mut session = ClientSession::default();
    session.model.playback.local_position = Some(1.0);
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default().with_position_seconds(12.5),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime
            .run_seek_by_offset(2.0)
            .expect("seek-by-offset should pre-sync pending telemetry position"),
        "seek-by-offset should emit a local SetPosition action"
    );
    assert_eq!(
        runtime.player().position,
        Some(14.5),
        "offset seek should use telemetry-confirmed local position as the baseline"
    );
    assert_eq!(
        runtime.session().model.playback.local_position,
        Some(14.5),
        "local session state should reflect the commanded seek target after applying telemetry baseline"
    );

    let drained = runtime.drain_player_playback_telemetry_updates();
    assert_eq!(
        drained,
        vec![PlayerPlaybackTelemetryUpdate::default().with_position_seconds(12.5)]
    );
}
