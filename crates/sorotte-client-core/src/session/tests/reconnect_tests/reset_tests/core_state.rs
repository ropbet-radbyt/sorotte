use super::*;

#[test]
fn reset_sync_state_for_reconnect_clears_sync_runtime_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("state should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("remote state should apply");

    let outbound =
        session.reconcile_state_and_build_response(StatePayload::new(), 0.0, true, 300.0, 0.3);
    assert!(
        outbound
            .ignoring_on_the_fly
            .as_ref()
            .is_some_and(|ignore| ignore.client.is_some()),
        "reconcile call should populate client ignore counter for changed local state"
    );
    assert_eq!(session.client_ignoring_on_the_fly(), 1);

    let behind_initial = session.evaluate_desync_correction(0.0, 0.0, false, false, true);
    assert_eq!(behind_initial, DesyncCorrectionAction::None);
    session.apply_player_playback_telemetry_update(
        &PlayerPlaybackTelemetryUpdate::default()
            .with_paused_for_cache(true)
            .with_cache_buffering_percent(42.5),
    );
    assert_eq!(session.local_paused_for_cache(), Some(true));
    assert_eq!(session.local_cache_buffering_percent(), Some(42.5));
    assert!(session.model.playback.pending_cache_room_playstate_resync);

    session
        .apply_message_json_at(
            r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#,
            10.0,
        )
        .expect("local playlist index should apply");
    assert!(session.recently_advanced(11.0));

    session.reset_sync_state_for_reconnect();

    assert_eq!(session.client_ignoring_on_the_fly(), 0);
    assert_eq!(session.server_ignoring_on_the_fly(), 0);
    assert!(session.current_room_playstate().is_none());
    let post_reset = session.evaluate_desync_correction(4.0, 0.0, false, false, true);
    assert_eq!(post_reset, DesyncCorrectionAction::None);
    assert_eq!(session.model.connection.username.as_deref(), Some("alice"));
    assert_eq!(session.model.room.name.as_deref(), Some("room1"));
    assert!(!session.recently_advanced(11.0));
    assert_eq!(session.local_paused_for_cache(), None);
    assert_eq!(session.local_cache_buffering_percent(), None);
    assert!(!session.model.playback.pending_cache_room_playstate_resync);
}

#[test]
fn reset_sync_state_for_reconnect_resets_desync_transient_state_before_post_reconnect_evaluation() {
    let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

    let pre_reset_behind_detection =
        session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
    assert_eq!(
        pre_reset_behind_detection,
        Vec::<ClientRuntimeAction>::new(),
        "initial behind detection should only start the fastforward timer pre-reconnect"
    );
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds,
        Some(0.0),
        "precondition: reconnect reset test should prime fastforward detection timer"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds, None,
        "reconnect reset should clear fastforward detection timer state"
    );
    assert!(
        !session.model.playback.speed_changed,
        "reconnect reset should clear slowdown state"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect remote state should apply");
    let post_reconnect_behind_detection =
        session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
    assert_eq!(
        post_reconnect_behind_detection,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect behind detection should restart fresh instead of using stale pre-reconnect timer state"
    );
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds,
        Some(4.0),
        "post-reconnect fastforward timer should start from the new evaluation time"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect ahead-state update should apply");
    let post_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(5.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "post-reconnect desync evaluation should be able to re-enter slowdown from a cleared state"
    );
    assert!(
        session.model.playback.speed_changed,
        "slowdown action should set speed_changed again after reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds, None,
        "second reconnect reset should clear any restarted fastforward timer state"
    );
    assert!(
        !session.model.playback.speed_changed,
        "second reconnect reset should clear the re-primed slowdown state"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-second-reconnect ahead-state update should apply");
    let second_post_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(6.0, 2.0, true, false, true);
    assert_eq!(
        second_post_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "cleared slowdown state should not suppress the first post-reconnect slowdown action"
    );
}
