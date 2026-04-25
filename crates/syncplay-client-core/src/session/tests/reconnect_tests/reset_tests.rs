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
    assert_eq!(session.username.as_deref(), Some("alice"));
    assert_eq!(session.room.as_deref(), Some("room1"));
    assert!(!session.recently_advanced(11.0));
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
        session.behind_first_detected_at_seconds,
        Some(0.0),
        "precondition: reconnect reset test should prime fastforward detection timer"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "reconnect reset should clear fastforward detection timer state"
    );
    assert!(
        !session.speed_changed,
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
        session.behind_first_detected_at_seconds,
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
        session.speed_changed,
        "slowdown action should set speed_changed again after reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "second reconnect reset should clear any restarted fastforward timer state"
    );
    assert!(
        !session.speed_changed,
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

#[test]
fn reset_sync_state_for_reconnect_prevents_stale_desync_speed_restore_after_pre_reconnect_slowdown()
{
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    let pre_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(
        pre_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "precondition: pre-reconnect desync evaluation should trigger slowdown"
    );
    assert!(
        session.speed_changed,
        "precondition: slowdown should mark speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.speed_changed,
        "reconnect reset should clear slowdown state so restore-speed is not emitted from stale state"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect synced remote state should apply");
    let post_reconnect_near_sync_actions =
        session.runtime_actions_for_desync_correction(1.0, 0.05, true, false, true);
    assert_eq!(
        post_reconnect_near_sync_actions,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect near-sync evaluation should not emit stale restore-speed action if slowdown state was reset"
    );
    assert!(
        !session.speed_changed,
        "near-sync evaluation should keep slowdown state cleared when no slowdown is active post-reconnect"
    );

    let post_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "post-reconnect slowdown should still trigger normally from a fresh state"
    );
}

#[test]
fn reset_sync_state_for_reconnect_prevents_stale_fastforward_after_pre_reconnect_behind_detection_and_post_reconnect_do_seek_transition()
 {
    let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

    let pre_reconnect_behind_detection =
        session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
    assert_eq!(
        pre_reconnect_behind_detection,
        Vec::<ClientRuntimeAction>::new(),
        "precondition: pre-reconnect behind detection should only start fastforward timer"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds,
        Some(0.0),
        "precondition: pre-reconnect behind timer should be primed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "reconnect reset should clear any pre-reconnect fastforward detection timer state"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"carol"}}}"#,
            )
            .expect("post-reconnect doSeek state should apply");
    let do_seek_suppressed =
        session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
    assert_eq!(
        do_seek_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect doSeek state should suppress desync correction"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "doSeek suppression after reconnect should keep fastforward timer cleared"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"dave"}}}"#,
            )
            .expect("post-reconnect doSeek-clear state should apply");
    let restarted_after_do_seek_clear =
        session.runtime_actions_for_desync_correction(4.1, 0.0, false, false, true);
    assert_eq!(
        restarted_after_do_seek_clear,
        Vec::<ClientRuntimeAction>::new(),
        "after reconnect + doSeek clears, fastforward detection should restart fresh instead of using stale pre-reconnect timing"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds,
        Some(4.1),
        "post-reconnect fastforward timer should restart from doSeek-clear evaluation time"
    );

    let before_threshold =
        session.runtime_actions_for_desync_correction(7.3, 0.0, false, false, true);
    assert_eq!(
        before_threshold,
        Vec::<ClientRuntimeAction>::new(),
        "restarted post-reconnect fastforward window should not trigger before threshold elapses"
    );

    let after_threshold =
        session.runtime_actions_for_desync_correction(7.5, 0.0, false, false, true);
    assert_eq!(
        after_threshold,
        vec![ClientRuntimeAction::SetPosition(10.25)],
        "fastforward should trigger only after the restarted post-reconnect window elapses"
    );
}

#[test]
fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_when_post_reconnect_state_resumes_paused_then_unpauses()
 {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    let pre_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(
        pre_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "precondition: pre-reconnect desync evaluation should trigger slowdown"
    );
    assert!(
        session.speed_changed,
        "precondition: slowdown should mark speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.speed_changed,
        "reconnect reset should clear slowdown state before post-reconnect paused/unpaused evaluations"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect paused remote state should apply");

    let paused_near_sync =
        session.runtime_actions_for_desync_correction(1.0, 0.05, true, false, true);
    assert_eq!(
        paused_near_sync,
        Vec::<ClientRuntimeAction>::new(),
        "paused post-reconnect near-sync evaluation should not emit stale restore-speed action"
    );
    assert!(
        !session.speed_changed,
        "paused post-reconnect near-sync evaluation should keep slowdown state cleared"
    );

    let paused_ahead = session.runtime_actions_for_desync_correction(1.5, 2.0, true, false, true);
    assert_eq!(
        paused_ahead,
        Vec::<ClientRuntimeAction>::new(),
        "paused post-reconnect desync evaluation should not emit slowdown while room is paused"
    );
    assert!(
        !session.speed_changed,
        "paused post-reconnect desync evaluation should not re-prime slowdown state"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect unpaused remote state should apply");

    let unpaused_near_sync =
        session.runtime_actions_for_desync_correction(2.0, 0.05, true, false, true);
    assert_eq!(
        unpaused_near_sync,
        Vec::<ClientRuntimeAction>::new(),
        "unpaused post-reconnect near-sync evaluation should still not emit stale restore-speed action"
    );
    assert!(
        !session.speed_changed,
        "unpaused near-sync evaluation should keep slowdown state cleared until a real slowdown trigger"
    );

    let unpaused_ahead = session.runtime_actions_for_desync_correction(3.0, 2.0, true, false, true);
    assert_eq!(
        unpaused_ahead,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "after unpause, post-reconnect desync slowdown should trigger normally from a fresh state"
    );
}

#[test]
fn reset_sync_state_for_reconnect_clears_self_setby_fastforward_suppression_window_before_post_reconnect_desync_evaluation()
 {
    let mut session = desync_session_with_remote_state(10.0, false, false, "alice");

    let pre_reconnect_timer_start =
        session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
    assert_eq!(
        pre_reconnect_timer_start,
        Vec::<ClientRuntimeAction>::new(),
        "precondition: initial behind detection should only start fastforward timer"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds,
        Some(0.0),
        "precondition: behind timer should start at first detection time"
    );

    let pre_reconnect_self_setby_suppressed =
        session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
    assert_eq!(
        pre_reconnect_self_setby_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "self-attributed fastforward candidate should be suppressed before reconnect"
    );
    assert!(
        session
            .behind_first_detected_at_seconds
            .is_some_and(|t| t > 4.0),
        "self-attributed fastforward suppression should leave a future suppression-window timer"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "reconnect reset should clear stale self-setby fastforward suppression window"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect remote state should apply");

    let post_reconnect_timer_restart =
        session.runtime_actions_for_desync_correction(4.1, 0.0, false, false, true);
    assert_eq!(
        post_reconnect_timer_restart,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect behind detection should restart instead of inheriting stale self-setby suppression window"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds,
        Some(4.1),
        "post-reconnect behind timer should restart from new detection time"
    );

    let post_reconnect_before_threshold =
        session.runtime_actions_for_desync_correction(7.3, 0.0, false, false, true);
    assert_eq!(
        post_reconnect_before_threshold,
        Vec::<ClientRuntimeAction>::new(),
        "restarted post-reconnect fastforward window should not trigger before threshold elapses"
    );

    let post_reconnect_after_threshold =
        session.runtime_actions_for_desync_correction(7.5, 0.0, false, false, true);
    assert_eq!(
        post_reconnect_after_threshold,
        vec![ClientRuntimeAction::SetPosition(10.25)],
        "post-reconnect fastforward should trigger only after restarted window elapses against non-self setBy"
    );
}

#[test]
fn reset_sync_state_for_reconnect_clears_fastforward_action_cooldown_window_before_post_reconnect_desync_evaluation()
 {
    let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

    let pre_reconnect_timer_start =
        session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
    assert_eq!(
        pre_reconnect_timer_start,
        Vec::<ClientRuntimeAction>::new(),
        "precondition: initial behind detection should only start fastforward timer"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds,
        Some(0.0),
        "precondition: behind timer should start at first detection time"
    );

    let pre_reconnect_fastforward =
        session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
    assert_eq!(
        pre_reconnect_fastforward,
        vec![ClientRuntimeAction::SetPosition(10.25)],
        "precondition: non-self fastforward should trigger before reconnect"
    );
    assert!(
        session
            .behind_first_detected_at_seconds
            .is_some_and(|t| t > 4.0),
        "fastforward action should leave a future cooldown/suppression timer before reconnect"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "reconnect reset should clear stale fastforward action cooldown window"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect remote state should apply");

    let post_reconnect_timer_restart =
        session.runtime_actions_for_desync_correction(4.1, 0.0, false, false, true);
    assert_eq!(
        post_reconnect_timer_restart,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect behind detection should restart instead of inheriting stale fastforward cooldown window"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds,
        Some(4.1),
        "post-reconnect behind timer should restart from new detection time"
    );

    let post_reconnect_before_threshold =
        session.runtime_actions_for_desync_correction(7.3, 0.0, false, false, true);
    assert_eq!(
        post_reconnect_before_threshold,
        Vec::<ClientRuntimeAction>::new(),
        "restarted post-reconnect fastforward window should not trigger before threshold elapses"
    );

    let post_reconnect_after_threshold =
        session.runtime_actions_for_desync_correction(7.5, 0.0, false, false, true);
    assert_eq!(
        post_reconnect_after_threshold,
        vec![ClientRuntimeAction::SetPosition(10.25)],
        "post-reconnect fastforward should trigger only after restarted window elapses"
    );
}

#[test]
fn reset_sync_state_for_reconnect_preserves_rewind_suppression_ordering_across_self_setby_and_post_reconnect_do_seek_transition()
 {
    let mut session = desync_session_with_remote_state(0.0, false, false, "alice");

    let pre_reconnect_self_setby_rewind_suppressed =
        session.runtime_actions_for_desync_correction(0.0, 6.0, false, false, true);
    assert_eq!(
        pre_reconnect_self_setby_rewind_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "pre-reconnect self-attributed rewind candidate should be suppressed"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "rewind/self-setBy suppression path should not leave a behind-detection timer"
    );
    assert!(
        !session.speed_changed,
        "rewind/self-setBy suppression path should not touch slowdown state"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "reconnect reset should keep rewind-related fastforward timer state cleared"
    );
    assert!(
        !session.speed_changed,
        "reconnect reset should keep slowdown state cleared before post-reconnect rewind evaluations"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect doSeek state should apply");
    let post_reconnect_do_seek_rewind_suppressed =
        session.runtime_actions_for_desync_correction(1.0, 6.0, false, false, true);
    assert_eq!(
        post_reconnect_do_seek_rewind_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect doSeek state should suppress rewind correction before doSeek clears"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect doSeek-clear state should apply");
    let post_reconnect_remote_rewind =
        session.runtime_actions_for_desync_correction(1.1, 6.0, false, false, true);
    assert_eq!(
        post_reconnect_remote_rewind,
        vec![ClientRuntimeAction::SetPosition(0.0)],
        "once post-reconnect doSeek clears and setBy is remote, rewind should trigger immediately"
    );
}

#[test]
fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_when_post_reconnect_rewind_precedes_near_sync()
 {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    let pre_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(
        pre_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "precondition: pre-reconnect ahead-state should trigger slowdown"
    );
    assert!(
        session.speed_changed,
        "precondition: slowdown should prime speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.speed_changed,
        "reconnect reset should clear slowdown state before post-reconnect rewind path"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect remote state should apply");

    let post_reconnect_rewind =
        session.runtime_actions_for_desync_correction(1.0, 6.0, false, false, true);
    assert_eq!(
        post_reconnect_rewind,
        vec![ClientRuntimeAction::SetPosition(0.0)],
        "post-reconnect rewind should still trigger immediately on large ahead desync"
    );
    assert!(
        !session.speed_changed,
        "rewind branch should not resurrect stale slowdown state after reconnect reset"
    );

    let post_reconnect_near_sync =
        session.runtime_actions_for_desync_correction(1.1, 0.05, true, false, true);
    assert_eq!(
        post_reconnect_near_sync,
        Vec::<ClientRuntimeAction>::new(),
        "near-sync after post-reconnect rewind should not emit stale restore-speed action"
    );
    assert!(
        !session.speed_changed,
        "near-sync after rewind should keep slowdown state cleared when no slowdown is active"
    );

    let post_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "post-reconnect slowdown should still trigger normally after rewind/near-sync from a fresh state"
    );
}

#[test]
fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_when_post_reconnect_self_setby_rewind_is_suppressed_before_near_sync()
 {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    let pre_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(
        pre_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "precondition: pre-reconnect ahead-state should trigger slowdown"
    );
    assert!(
        session.speed_changed,
        "precondition: slowdown should prime speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.speed_changed,
        "reconnect reset should clear slowdown state before post-reconnect self-setBy rewind suppression"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect self-setBy remote state should apply");

    let post_reconnect_self_setby_rewind_suppressed =
        session.runtime_actions_for_desync_correction(1.0, 6.0, false, false, true);
    assert_eq!(
        post_reconnect_self_setby_rewind_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect self-attributed rewind candidate should remain suppressed"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "self-setBy rewind suppression should not prime fastforward timer state"
    );
    assert!(
        !session.speed_changed,
        "self-setBy rewind suppression should not resurrect stale slowdown state"
    );

    let post_reconnect_near_sync =
        session.runtime_actions_for_desync_correction(1.1, 0.05, true, false, true);
    assert_eq!(
        post_reconnect_near_sync,
        Vec::<ClientRuntimeAction>::new(),
        "near-sync after self-setBy rewind suppression should not emit stale restore-speed action"
    );
    assert!(
        !session.speed_changed,
        "near-sync after self-setBy rewind suppression should keep slowdown state cleared"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect non-self remote state should apply");
    let post_reconnect_remote_slowdown =
        session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_remote_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "post-reconnect slowdown should still trigger normally after self-setBy rewind suppression and near-sync from a fresh state"
    );
}

#[test]
fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_across_post_reconnect_do_seek_paused_and_self_setby_rewind_suppression_branches()
 {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    let pre_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(
        pre_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "precondition: pre-reconnect ahead-state should trigger slowdown"
    );
    assert!(
        session.speed_changed,
        "precondition: slowdown should prime speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.speed_changed,
        "reconnect reset should clear slowdown state before post-reconnect branch sequence"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":true,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect paused doSeek self-setBy state should apply");
    let post_reconnect_do_seek_suppressed =
        session.runtime_actions_for_desync_correction(1.0, 6.0, false, false, true);
    assert_eq!(
        post_reconnect_do_seek_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect doSeek state should suppress desync correction before other branches"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "doSeek suppression should keep fastforward timer state cleared"
    );
    assert!(
        !session.speed_changed,
        "doSeek suppression should not resurrect stale slowdown state"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect paused self-setBy state should apply");
    let post_reconnect_paused_self_setby_rewind_suppressed =
        session.runtime_actions_for_desync_correction(1.1, 6.0, false, false, true);
    assert_eq!(
        post_reconnect_paused_self_setby_rewind_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "paused self-attributed rewind candidate should remain suppressed after reconnect"
    );
    assert_eq!(
        session.behind_first_detected_at_seconds, None,
        "rewind/self-setBy suppression path should not prime fastforward timer state"
    );
    assert!(
        !session.speed_changed,
        "paused self-setBy rewind suppression should not resurrect stale slowdown state"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect unpaused self-setBy state should apply");
    let post_reconnect_near_sync =
        session.runtime_actions_for_desync_correction(1.2, 0.05, true, false, true);
    assert_eq!(
        post_reconnect_near_sync,
        Vec::<ClientRuntimeAction>::new(),
        "near-sync after doSeek+paused+self-setBy suppression sequence should not emit stale restore-speed action"
    );
    assert!(
        !session.speed_changed,
        "near-sync after branch sequence should keep slowdown state cleared"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect non-self state should apply");
    let post_reconnect_remote_slowdown =
        session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_remote_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "post-reconnect slowdown should still trigger normally after branch sequence from a fresh state"
    );
}

#[test]
fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_when_post_reconnect_self_setby_slowdown_is_suppressed_before_near_sync()
 {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    let pre_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(
        pre_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "precondition: pre-reconnect ahead-state should trigger slowdown"
    );
    assert!(
        session.speed_changed,
        "precondition: slowdown should prime speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.speed_changed,
        "reconnect reset should clear slowdown state before post-reconnect self-setBy slowdown suppression"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect self-setBy state should apply");

    let post_reconnect_self_setby_slowdown_suppressed =
        session.runtime_actions_for_desync_correction(1.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_self_setby_slowdown_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect self-attributed slowdown candidate should remain suppressed"
    );
    assert!(
        !session.speed_changed,
        "self-setBy slowdown suppression should not resurrect stale slowdown state"
    );

    let post_reconnect_near_sync =
        session.runtime_actions_for_desync_correction(1.1, 0.05, true, false, true);
    assert_eq!(
        post_reconnect_near_sync,
        Vec::<ClientRuntimeAction>::new(),
        "near-sync after self-setBy slowdown suppression should not emit stale restore-speed action"
    );
    assert!(
        !session.speed_changed,
        "near-sync after self-setBy slowdown suppression should keep slowdown state cleared"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect non-self state should apply");
    let post_reconnect_remote_slowdown =
        session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_remote_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "post-reconnect slowdown should still trigger normally after self-setBy slowdown suppression and near-sync from a fresh state"
    );
}

#[test]
fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_across_post_reconnect_do_seek_paused_and_self_setby_slowdown_suppression_branches()
 {
    let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

    let pre_reconnect_slowdown =
        session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
    assert_eq!(
        pre_reconnect_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "precondition: pre-reconnect ahead-state should trigger slowdown"
    );
    assert!(
        session.speed_changed,
        "precondition: slowdown should prime speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.speed_changed,
        "reconnect reset should clear slowdown state before post-reconnect branch sequence"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":true,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect doSeek self-setBy state should apply");
    let post_reconnect_do_seek_suppressed =
        session.runtime_actions_for_desync_correction(1.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_do_seek_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect doSeek state should suppress slowdown evaluation before other branches"
    );
    assert!(
        !session.speed_changed,
        "doSeek suppression should not resurrect stale slowdown state"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect paused self-setBy state should apply");
    let post_reconnect_paused_slowdown_suppressed =
        session.runtime_actions_for_desync_correction(1.1, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_paused_slowdown_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "paused post-reconnect state should suppress slowdown before self-setBy slowdown branch"
    );
    assert!(
        !session.speed_changed,
        "paused slowdown suppression should keep slowdown state cleared"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect unpaused self-setBy state should apply");
    let post_reconnect_self_setby_slowdown_suppressed =
        session.runtime_actions_for_desync_correction(1.2, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_self_setby_slowdown_suppressed,
        Vec::<ClientRuntimeAction>::new(),
        "post-reconnect self-attributed slowdown candidate should remain suppressed"
    );
    assert!(
        !session.speed_changed,
        "self-setBy slowdown suppression should not resurrect stale slowdown state"
    );

    let post_reconnect_near_sync =
        session.runtime_actions_for_desync_correction(1.3, 0.05, true, false, true);
    assert_eq!(
        post_reconnect_near_sync,
        Vec::<ClientRuntimeAction>::new(),
        "near-sync after doSeek+paused+self-setBy slowdown-suppression sequence should not emit stale restore-speed action"
    );
    assert!(
        !session.speed_changed,
        "near-sync after branch sequence should keep slowdown state cleared"
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect non-self state should apply");
    let post_reconnect_remote_slowdown =
        session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
    assert_eq!(
        post_reconnect_remote_slowdown,
        vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
        "post-reconnect slowdown should still trigger normally after branch sequence from a fresh state"
    );
}

#[test]
fn reset_sync_state_for_reconnect_clears_readiness_support_until_next_hello() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"setOthersReadiness":true}}}"#,
            )
            .expect("hello should apply");

    session.reset_sync_state_for_reconnect();

    assert_eq!(session.server_readiness_supported(), None);
    assert_eq!(session.server_set_others_readiness_supported(), None);
    assert!(
        session
            .runtime_actions_for_local_ready_toggle(true)
            .is_empty()
    );
    assert!(
        session
            .runtime_actions_for_local_user_ready_set("bob".to_owned(), true, true)
            .is_empty()
    );
}

#[test]
fn reset_sync_state_for_reconnect_clears_managed_rooms_support_until_next_hello() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"managedRooms":true}}}"#,
            )
            .expect("hello should apply");

    session.reset_sync_state_for_reconnect();

    assert_eq!(session.server_managed_rooms_supported(), None);
    assert!(
        session
            .runtime_actions_for_local_controller_auth_request(
                "+room:ABCDEF123456".to_owned(),
                "AB-123-456".to_owned(),
            )
            .is_empty()
    );
}
