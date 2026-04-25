use super::*;

#[test]
fn user_change_join_notification_hides_noncontroller_when_osd_is_disabled() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"}}}}}"#)
        .expect("join update should apply");

    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            },
        )]
    );
}

#[test]
fn user_change_playing_notification_respects_controller_visibility_override() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
        )
        .expect("controller update should apply");
    let _ = session.runtime_actions_for_user_change_notifications_if_needed();

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"file":{"name":"movie.mkv","duration":123.4}}}}}"#,
        )
        .expect("file update should apply");

    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Playing {
                username: "bob".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                file_name: Some("movie.mkv".to_owned()),
                file_duration: Some(json!(123.4)),
                include_room_addendum: false,
                hide_from_osd: false,
            },
        )]
    );
}

#[test]
fn user_change_playing_notification_room_addendum_matches_legacy_room_scope() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("controller update should apply");
    let _ = session.runtime_actions_for_user_change_notifications_if_needed();

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"file":{"name":"movie.mkv","duration":123.4}}}}}"#,
        )
        .expect("same-room file update should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Playing {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                file_name: Some("movie.mkv".to_owned()),
                file_duration: Some(json!(123.4)),
                include_room_addendum: false,
                hide_from_osd: false,
            },
        )]
    );

    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room2"}}}}}"#)
        .expect("different-room update should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Playing {
                username: "bob".to_owned(),
                room: "room2".to_owned(),
                file_name: Some("movie.mkv".to_owned()),
                file_duration: Some(json!(123.4)),
                include_room_addendum: true,
                hide_from_osd: true,
            },
        )]
    );
}

#[test]
fn user_change_notifications_respect_different_room_visibility_toggle() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"controller":true}}}}"#,
        )
        .expect("join update should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room2".to_owned(),
                hide_from_osd: true,
            },
        )]
    );

    session.behavior_config_mut().show_different_room_osd = true;
    session
        .apply_message_json(
            r#"{"Set":{"user":{"carol":{"room":{"name":"room3"},"controller":true}}}}"#,
        )
        .expect("second join update should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Joined {
                username: "carol".to_owned(),
                room: "room3".to_owned(),
                hide_from_osd: false,
            },
        )]
    );
}

#[test]
fn user_change_notifications_respect_osd_warnings_toggle_for_same_room_events() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("same-room join should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: false,
            },
        )]
    );

    session.behavior_config_mut().show_osd_warnings = false;
    session
        .apply_message_json(
            r#"{"Set":{"user":{"carol":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("second same-room join should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Joined {
                username: "carol".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: true,
            },
        )]
    );
}

#[test]
fn user_change_notifications_are_not_gated_by_show_same_room_osd() {
    let mut session = ClientSession::default();
    session.behavior_config_mut().show_same_room_osd = false;
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("same-room join should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: false,
            },
        )]
    );
}

#[test]
fn user_change_room_switch_uses_previous_room_for_visibility_scope() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("same-room join should apply");
    let _ = session.runtime_actions_for_user_change_notifications_if_needed();

    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room2"}}}}}"#)
        .expect("room switch should apply");
    assert_eq!(session.user_controller("bob"), Some(false));
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room2".to_owned(),
                hide_from_osd: true,
            },
        )]
    );

    session.behavior_config_mut().show_osd_warnings = false;
    session
        .apply_message_json(
            r#"{"Set":{"user":{"carol":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("carol join should apply");
    let _ = session.runtime_actions_for_user_change_notifications_if_needed();
    session
        .apply_message_json(r#"{"Set":{"user":{"carol":{"room":{"name":"room2"}}}}}"#)
        .expect("carol room switch should apply");
    assert_eq!(session.user_controller("carol"), Some(false));
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Joined {
                username: "carol".to_owned(),
                room: "room2".to_owned(),
                hide_from_osd: true,
            },
        )]
    );
}

#[test]
fn user_left_notifications_respect_same_and_different_room_visibility() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("same-room join should apply");
    let _ = session.runtime_actions_for_user_change_notifications_if_needed();
    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#)
        .expect("same-room left should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Left {
                username: "bob".to_owned(),
                hide_from_osd: false,
            },
        )]
    );

    session
        .apply_message_json(
            r#"{"Set":{"user":{"carol":{"room":{"name":"room2"},"controller":true}}}}"#,
        )
        .expect("different-room join should apply");
    let _ = session.runtime_actions_for_user_change_notifications_if_needed();
    session
        .apply_message_json(r#"{"Set":{"user":{"carol":{"event":{"left":true}}}}}"#)
        .expect("different-room left should apply");
    assert_eq!(
        session.runtime_actions_for_user_change_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyUserChange(
            UserChangeNotification::Left {
                username: "carol".to_owned(),
                hide_from_osd: true,
            },
        )]
    );
}
