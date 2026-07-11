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
fn desync_correction_skips_actions_when_set_by_matches_local_user() {
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

    let action = session.evaluate_desync_correction(0.0, 6.0, false, false, true);
    assert_eq!(action, DesyncCorrectionAction::None);
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
