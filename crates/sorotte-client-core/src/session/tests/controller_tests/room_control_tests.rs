use super::*;

#[test]
fn room_change_clears_local_controller_flag_until_reidentified() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
            )
            .expect("controller update should apply");
    assert_eq!(session.local_can_control(), Some(true));

    session
        .apply_message_json(r#"{"Set":{"room":{"name":"+room2:123456ABCDEF"}}}"#)
        .expect("room change should apply");

    assert_eq!(session.user_controller("alice"), Some(false));
    assert_eq!(session.local_can_control(), Some(false));
}

#[test]
fn noncontroller_event_hide_from_osd_respects_behavior_config_and_controller_flag() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    assert!(
        session.noncontroller_event_hide_from_osd_legacy_compatible("bob"),
        "unknown users are treated as non-controllers when non-controller OSD is disabled"
    );

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("controller update should apply");
    assert!(
        !session.noncontroller_event_hide_from_osd_legacy_compatible("bob"),
        "controllers should remain visible on OSD"
    );

    session.behavior_config_mut().show_noncontroller_osd = true;
    assert!(
        !session.noncontroller_event_hide_from_osd_legacy_compatible("carol"),
        "non-controller OSD override should keep unknown/non-controller users visible"
    );
}

#[test]
fn reconnect_reset_restores_local_controller_flag_after_hello() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
            )
            .expect("controller update should apply");
    assert_eq!(session.local_can_control(), Some(true));

    session.reset_sync_state_for_reconnect();
    session.reset_sync_state_for_reconnect();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");

    assert_eq!(session.user_controller("alice"), Some(true));
    assert_eq!(session.local_can_control(), Some(true));
}

#[test]
fn non_controller_pause_request_sets_not_ready_without_driving_room_pause() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    session
            .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":false,"isReady":true}}}}"#,
            )
            .expect("controller flag should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    let actions = session.runtime_actions_for_local_pause_set(true);

    assert_eq!(
        actions,
        vec![ClientRuntimeAction::SetReady {
            ready: false,
            manually_initiated: true
        }]
    );
    assert_eq!(session.local_paused(), Some(false));
    assert_eq!(session.user_ready("alice"), Some(false));
}
