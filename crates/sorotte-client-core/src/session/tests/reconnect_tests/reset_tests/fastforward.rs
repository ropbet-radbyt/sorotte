use super::*;

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
        session.model.playback.behind_first_detected_at_seconds,
        Some(0.0),
        "precondition: pre-reconnect behind timer should be primed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds, None,
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
        session.model.playback.behind_first_detected_at_seconds, None,
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
        session.model.playback.behind_first_detected_at_seconds,
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
        session.model.playback.behind_first_detected_at_seconds,
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
            .model
            .playback
            .behind_first_detected_at_seconds
            .is_some_and(|t| t > 4.0),
        "self-attributed fastforward suppression should leave a future suppression-window timer"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds, None,
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
        session.model.playback.behind_first_detected_at_seconds,
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
        session.model.playback.behind_first_detected_at_seconds,
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
            .model
            .playback
            .behind_first_detected_at_seconds
            .is_some_and(|t| t > 4.0),
        "fastforward action should leave a future cooldown/suppression timer before reconnect"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds, None,
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
        session.model.playback.behind_first_detected_at_seconds,
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
