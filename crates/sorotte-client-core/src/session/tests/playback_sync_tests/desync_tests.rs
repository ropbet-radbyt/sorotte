use super::*;

#[test]
fn current_room_playstate_at_advances_unpaused_position() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
                100.0,
            )
            .expect("room playstate should apply");

    let raw_playstate = session
        .current_room_playstate()
        .expect("raw room playstate should be stored");
    assert_eq!(
        raw_playstate.position,
        Some(10.0),
        "stored room playstate should preserve the inbound snapshot"
    );

    let advanced_playstate = session
        .current_room_playstate_at(103.25)
        .expect("effective room playstate should be available");
    assert_eq!(
        advanced_playstate.position,
        Some(13.25),
        "effective room playstate should advance while the room is playing"
    );
    assert_eq!(advanced_playstate.paused, Some(false));
    assert_eq!(advanced_playstate.do_seek, Some(false));
    assert_eq!(advanced_playstate.set_by.as_deref(), Some("bob"));
}

#[test]
fn determine_local_state_change_requires_divergence_from_previous_local_position() {
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
    session.model.playback.local_position = Some(0.6);
    session.model.playback.local_paused = Some(false);

    let (pause_change, seeked) = session.determine_local_state_change(false, 1.2);

    assert!(!pause_change);
    assert!(
        !seeked,
        "smooth playback progress should not be classified as a seek when it remains close to the last local telemetry position"
    );
}

#[test]
fn desync_correction_rewinds_when_client_is_ahead_beyond_threshold() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

    let action = session.evaluate_desync_correction(0.0, 5.0, false, false, true);
    assert_eq!(
        action,
        DesyncCorrectionAction::Rewind {
            target_position: 0.0,
            set_by: Some("bob".to_owned())
        }
    );
}

#[test]
fn desync_correction_slowdown_then_restore_speed_when_delta_recovers() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

    let slowdown = session.evaluate_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(
        slowdown,
        DesyncCorrectionAction::SlowDown {
            rate: 0.95,
            set_by: Some("bob".to_owned())
        }
    );

    let restore = session.evaluate_desync_correction(1.0, 0.05, true, false, true);
    assert_eq!(restore, DesyncCorrectionAction::RestoreSpeed { rate: 1.0 });
}

#[test]
fn desync_correction_fastforward_requires_sustained_behind_window() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

    let initial = session.evaluate_desync_correction(0.0, 0.0, false, false, true);
    assert_eq!(initial, DesyncCorrectionAction::None);

    let fastforward = session.evaluate_desync_correction(4.0, 0.0, false, false, true);
    assert_eq!(
        fastforward,
        DesyncCorrectionAction::FastForward {
            target_position: 10.25,
            set_by: Some("bob".to_owned())
        }
    );
}

#[test]
fn desync_correction_rearms_after_local_room_state_echo() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("state should apply");
    session
        .model
        .room
        .playstate_authority_changed_at_seconds
        .insert("room1".to_owned(), 0.0);

    let action = session.evaluate_desync_correction(3.0, 6.0, false, false, true);
    assert_eq!(
        action,
        DesyncCorrectionAction::Rewind {
            target_position: 0.0,
            set_by: Some("alice".to_owned())
        },
        "the last room controller must not remain exempt from steady-state correction forever"
    );
}

#[test]
fn repeated_self_attributed_room_updates_do_not_extend_correction_grace() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .expect("initial self echo should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            10.0,
        )
        .expect("periodic self-attributed state should apply");

    assert_eq!(
        session.evaluate_desync_correction(10.0, 16.0, false, false, true),
        DesyncCorrectionAction::Rewind {
            target_position: 10.0,
            set_by: Some("alice".to_owned())
        }
    );
}

#[test]
fn a_new_self_attributed_seek_gets_a_fresh_bounded_correction_grace() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .expect("initial self echo should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"alice"}}}"#,
            10.0,
        )
        .expect("new self seek should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.1,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            10.1,
        )
        .expect("post-seek steady state should apply");

    assert_eq!(
        session.evaluate_desync_correction(11.0, 16.0, false, false, true),
        DesyncCorrectionAction::None
    );
    assert!(matches!(
        session.evaluate_desync_correction(13.0, 20.0, false, false, true),
        DesyncCorrectionAction::Rewind { .. }
    ));
}

#[test]
fn self_origin_grace_covers_the_deferred_fastforward_window() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .expect("self-originated state should apply");

    assert_eq!(
        session.evaluate_desync_correction(0.0, 0.0, false, false, true),
        DesyncCorrectionAction::None
    );
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds,
        Some(0.0),
        "the candidate remains available if room authority changes to a remote user"
    );

    assert_eq!(
        session.evaluate_desync_correction(4.0, 0.0, false, false, true),
        DesyncCorrectionAction::None,
        "the self-origin grace must cover the configured sustain window"
    );
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds,
        Some(7.0)
    );
    assert!(
        matches!(
            session.evaluate_desync_correction(11.0, 0.0, false, false, true),
            DesyncCorrectionAction::FastForward { .. }
        ),
        "stale self attribution must still be bounded"
    );
}

#[test]
fn reconciled_self_origin_state_uses_the_runtime_clock_for_correction_grace() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(10.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());

    runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
        StatePayload::new().with_playstate(
            PlaystatePayload::new()
                .with_position(0.0)
                .with_paused(false)
                .with_do_seek(false)
                .with_set_by("alice"),
        ),
        false,
        42.0,
    );

    assert_eq!(
        runtime
            .session()
            .model
            .room
            .playstate_authority_changed_at_seconds
            .get("room1"),
        Some(&42.0),
        "inbound authority timestamps must share the correction loop's clock domain"
    );
    runtime
        .run_desync_correction_if_needed(42.5, false, false, true)
        .expect("fresh self-origin state should suppress correction");
    assert!(runtime.player().player_effects.is_empty());
}

#[test]
fn ordinary_behind_drift_does_not_change_playback_rate() {
    let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

    let action = session.evaluate_desync_correction(0.0, 6.0, false, false, true);
    assert_eq!(
        action,
        DesyncCorrectionAction::None,
        "ordinary drift must not feed a high-latency client speed-up back into the room clock"
    );
}

#[test]
fn behind_controller_drift_does_not_change_playback_rate() {
    let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

    assert_eq!(
        session.evaluate_desync_correction(0.0, 6.0, true, false, true),
        DesyncCorrectionAction::None,
        "controllers must not accelerate toward a room clock that may already be based on their delayed sample"
    );
}

#[test]
fn slowdown_restores_normal_speed_when_client_jumps_behind_room_position() {
    let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

    assert!(matches!(
        session.evaluate_desync_correction(0.0, 12.0, true, false, true),
        DesyncCorrectionAction::SlowDown { rate: 0.95, .. }
    ));
    assert_eq!(
        session.evaluate_desync_correction(0.1, 6.0, true, false, true),
        DesyncCorrectionAction::RestoreSpeed { rate: 1.0 },
        "crossing from ahead to behind must neutralize slowdown"
    );
}

#[test]
fn desync_correction_reasserts_rate_after_player_reports_external_reset() {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    assert!(matches!(
        session.evaluate_desync_correction(0.0, 2.0, true, false, true),
        DesyncCorrectionAction::SlowDown { rate: 0.95, .. }
    ));
    session.apply_player_playback_telemetry_update(
        &PlayerPlaybackTelemetryUpdate::default().with_playback_rate(1.0),
    );
    assert!(matches!(
        session.evaluate_desync_correction(0.1, 2.0, true, false, true),
        DesyncCorrectionAction::SlowDown { rate: 0.95, .. }
    ));
}

#[test]
fn runtime_actions_for_desync_correction_maps_rewind_to_set_position() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

    let actions = session.runtime_actions_for_desync_correction(0.0, 6.0, false, false, true);
    assert_eq!(actions, vec![ClientRuntimeAction::SetPosition(0.0)]);
}

#[test]
fn runtime_actions_for_desync_correction_maps_slowdown_to_rate_change() {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    let actions = session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(actions, vec![ClientRuntimeAction::SetPlaybackRate(0.95)]);
}

#[test]
fn failed_desync_rate_command_rolls_back_correction_ownership() {
    let session = desync_session_with_remote_state(0.0, false, false, "bob");
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(2.0)
                .with_paused(false),
        ),
        fail_set_playback_rate: true,
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());

    assert!(
        runtime
            .run_desync_correction_if_needed(0.0, true, false, true)
            .is_err()
    );
    assert!(!runtime.session().model.playback.speed_changed);
    assert_eq!(runtime.session().model.playback.speed_correction_rate, None);
    assert_eq!(runtime.session().model.playback.local_playback_rate, None);
}

#[test]
fn disabling_slow_on_desync_restores_speed_and_failed_restore_retains_ownership() {
    let session = desync_session_with_remote_state(0.0, false, false, "bob");
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(2.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());

    runtime
        .run_desync_correction_if_needed(0.0, true, false, true)
        .expect("slowdown command should apply");
    assert_eq!(runtime.player().playback_rate, Some(0.95));

    let mut config = runtime.session().desync_config().clone();
    config.slow_on_desync = false;
    runtime.session_mut().set_desync_config(config);
    runtime.player_mut_for_test().fail_set_playback_rate = true;

    assert!(
        runtime
            .run_desync_correction_if_needed(0.1, true, false, true)
            .is_err()
    );
    assert!(runtime.session().model.playback.speed_changed);
    assert_eq!(
        runtime.session().model.playback.speed_correction_rate,
        Some(0.95)
    );
    assert_eq!(
        runtime.session().model.playback.local_playback_rate,
        Some(0.95)
    );

    runtime.player_mut_for_test().fail_set_playback_rate = false;
    runtime
        .run_desync_correction_if_needed(0.2, true, false, true)
        .expect("retry should restore normal speed");
    assert_eq!(runtime.player().playback_rate, Some(1.0));
    assert!(!runtime.session().model.playback.speed_changed);
    assert_eq!(runtime.session().model.playback.speed_correction_rate, None);
}

#[test]
fn client_runtime_suppresses_desync_correction_until_cache_recovery_advancement_is_observed() {
    let session = desync_session_with_remote_state(0.0, false, false, "bob");
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(6.0)
                .with_paused(false)
                .with_paused_for_cache(true),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_desync_correction_if_needed(0.0, false, false, true)
        .expect("cache-paused desync evaluation should not fail");
    assert!(
        runtime.player().player_effects.is_empty(),
        "ordinary correction must not seek or change speed while cache-paused"
    );
    assert!(
        runtime
            .session()
            .model
            .playback
            .pending_cache_room_playstate_resync
    );

    runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update = Some(
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(6.0)
            .with_paused(false)
            .with_paused_for_cache(false),
    );
    runtime
        .run_desync_correction_if_needed(1.0, false, false, true)
        .expect("first post-cache observation should not fail");
    assert!(
        runtime.player().player_effects.is_empty(),
        "cache release and its first position sample must not trigger correction"
    );
    assert!(
        runtime
            .session()
            .model
            .playback
            .pending_cache_room_playstate_resync,
        "recovery must remain pending until a later sample proves forward advancement"
    );

    runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update = Some(
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(6.25)
            .with_paused(false),
    );
    runtime
        .run_desync_correction_if_needed(2.0, false, false, true)
        .expect("advancing post-cache observation should not fail");
    assert!(
        !runtime
            .session()
            .model
            .playback
            .pending_cache_room_playstate_resync,
        "observed forward playback should close the conservative P0 recovery gate"
    );
    assert!(
        runtime
            .player()
            .player_effects
            .iter()
            .any(|effect| matches!(effect, ClientEffect::SetPlayerPosition(_))),
        "ordinary correction may resume only after playback advancement is observed"
    );
}

#[test]
fn runtime_actions_for_desync_correction_scenario_fastforward_window_reset_and_retrigger() {
    let mut session = desync_session_with_remote_state(10.0, false, false, "bob");
    let steps = vec![
        DesyncRuntimeScenarioStep {
            now_seconds: 0.0,
            local_position: 0.0,
            local_can_control: false,
            dont_slow_down_with_me: false,
            speed_supported: true,
            expected_actions: Vec::new(),
        },
        DesyncRuntimeScenarioStep {
            now_seconds: 4.0,
            local_position: 0.0,
            local_can_control: false,
            dont_slow_down_with_me: false,
            speed_supported: true,
            expected_actions: vec![ClientRuntimeAction::SetPosition(10.25)],
        },
        DesyncRuntimeScenarioStep {
            now_seconds: 5.0,
            local_position: 0.0,
            local_can_control: false,
            dont_slow_down_with_me: false,
            speed_supported: true,
            expected_actions: Vec::new(),
        },
        DesyncRuntimeScenarioStep {
            now_seconds: 11.0,
            local_position: 0.0,
            local_can_control: false,
            dont_slow_down_with_me: false,
            speed_supported: true,
            expected_actions: vec![ClientRuntimeAction::SetPosition(10.25)],
        },
    ];

    run_desync_runtime_scenario(&mut session, &steps);
}

#[test]
fn runtime_actions_for_desync_correction_scenario_slowdown_restore_then_rewind() {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");
    let steps = vec![
        DesyncRuntimeScenarioStep {
            now_seconds: 0.0,
            local_position: 2.0,
            local_can_control: true,
            dont_slow_down_with_me: false,
            speed_supported: true,
            expected_actions: vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        },
        DesyncRuntimeScenarioStep {
            now_seconds: 0.5,
            local_position: 0.05,
            local_can_control: true,
            dont_slow_down_with_me: false,
            speed_supported: true,
            expected_actions: vec![ClientRuntimeAction::SetPlaybackRate(1.0)],
        },
        DesyncRuntimeScenarioStep {
            now_seconds: 1.0,
            local_position: 4.5,
            local_can_control: true,
            dont_slow_down_with_me: false,
            speed_supported: true,
            expected_actions: vec![ClientRuntimeAction::SetPosition(0.0)],
        },
        DesyncRuntimeScenarioStep {
            now_seconds: 1.5,
            local_position: 0.0,
            local_can_control: true,
            dont_slow_down_with_me: false,
            speed_supported: true,
            expected_actions: Vec::new(),
        },
    ];

    run_desync_runtime_scenario(&mut session, &steps);
}
