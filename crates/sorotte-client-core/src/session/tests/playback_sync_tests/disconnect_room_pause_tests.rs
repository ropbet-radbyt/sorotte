use super::*;

#[test]
fn handle_disconnect_with_pause_on_leave_sets_pause_and_timestamp() {
    let mut session = ClientSession::default();
    session.model.playback.local_paused = Some(false);

    let actions = session.handle_disconnect(123.4);
    assert_eq!(actions, vec![ClientRuntimeAction::SetPaused(true)]);
    assert_eq!(session.last_paused_on_leave_at_seconds(), Some(123.4));
}

#[test]
fn handle_disconnect_respects_pause_on_leave_toggle() {
    let mut session = ClientSession::default();
    session.model.playback.local_paused = Some(false);
    session.behavior_config_mut().pause_on_leave = false;

    let actions = session.handle_disconnect(200.0);
    assert!(actions.is_empty());
    assert_eq!(session.last_paused_on_leave_at_seconds(), None);
}

#[test]
fn handle_disconnect_clears_managed_rooms_support_until_next_hello() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"managedRooms":true}}}"#,
            )
            .expect("hello should apply");
    assert!(session.server_managed_rooms_supported());
    assert!(
        !session
            .runtime_actions_for_local_controller_auth_request(
                "+room:ABCDEF123456".to_owned(),
                "AB-123-456".into(),
            )
            .is_empty()
    );

    let _ = session.handle_disconnect(200.0);

    assert_eq!(session.connection_phase(), &ConnectionPhase::Disconnected);
    assert!(!session.server_managed_rooms_supported());
    assert!(
        session
            .runtime_actions_for_local_controller_auth_request(
                "+room:ABCDEF123456".to_owned(),
                "AB-123-456".into(),
            )
            .is_empty()
    );
}

#[test]
fn client_runtime_room_pause_sync_applies_remote_pause_mismatch_from_playstate() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("remote state should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default().with_paused(true),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_room_pause_sync_if_needed()
        .expect("room pause sync should dispatch");

    assert_eq!(
        runtime.player().paused,
        Some(false),
        "remote room playstate pause mismatch should issue player unpause"
    );
    assert_eq!(
        runtime.session().model.playback.local_paused,
        Some(false),
        "room pause sync should optimistically mirror local pause state until next telemetry sample"
    );
    assert_eq!(
        runtime.drain_player_playback_telemetry_updates(),
        vec![PlayerPlaybackTelemetryUpdate::default().with_paused(true)],
        "room pause sync should preserve synced telemetry for diagnostics drains"
    );
}

#[test]
fn client_runtime_room_pause_sync_unpauses_without_local_pause_telemetry() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("remote state should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_room_pause_sync_if_needed()
        .expect("room pause sync should dispatch");

    assert_eq!(
        runtime.player().paused,
        Some(false),
        "missing local pause telemetry should still allow the first remote unpause correction"
    );
    assert_eq!(
        runtime.session().model.playback.local_paused,
        Some(false),
        "successful remote unpause correction should mirror the effective local pause state"
    );
}

#[test]
fn client_runtime_room_pause_sync_skips_when_room_playstate_set_by_local_user() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("self-originated state should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default().with_paused(true),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_room_pause_sync_if_needed()
        .expect("room pause sync should not fail");

    assert_eq!(
        runtime.player().paused,
        None,
        "self-originated room playstate should not trigger local pause correction"
    );
    assert_eq!(
        runtime.session().model.playback.local_paused,
        Some(true),
        "telemetry sync should still update local paused snapshot"
    );
}
