use super::*;

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
        session.model.playback.behind_first_detected_at_seconds, None,
        "rewind/self-setBy suppression path should not leave a behind-detection timer"
    );
    assert!(
        !session.model.playback.speed_changed,
        "rewind/self-setBy suppression path should not touch slowdown state"
    );

    session.reset_sync_state_for_reconnect();
    assert_eq!(
        session.model.playback.behind_first_detected_at_seconds, None,
        "reconnect reset should keep rewind-related fastforward timer state cleared"
    );
    assert!(
        !session.model.playback.speed_changed,
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
        session.model.playback.speed_changed,
        "precondition: slowdown should prime speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.model.playback.speed_changed,
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
        !session.model.playback.speed_changed,
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
        !session.model.playback.speed_changed,
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
        session.model.playback.speed_changed,
        "precondition: slowdown should prime speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.model.playback.speed_changed,
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
        session.model.playback.behind_first_detected_at_seconds, None,
        "self-setBy rewind suppression should not prime fastforward timer state"
    );
    assert!(
        !session.model.playback.speed_changed,
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
        !session.model.playback.speed_changed,
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
        session.model.playback.speed_changed,
        "precondition: slowdown should prime speed_changed before reconnect reset"
    );

    session.reset_sync_state_for_reconnect();
    assert!(
        !session.model.playback.speed_changed,
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
        session.model.playback.behind_first_detected_at_seconds, None,
        "doSeek suppression should keep fastforward timer state cleared"
    );
    assert!(
        !session.model.playback.speed_changed,
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
        session.model.playback.behind_first_detected_at_seconds, None,
        "rewind/self-setBy suppression path should not prime fastforward timer state"
    );
    assert!(
        !session.model.playback.speed_changed,
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
        !session.model.playback.speed_changed,
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
