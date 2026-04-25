use super::*;

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
