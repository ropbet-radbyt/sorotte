use super::*;

#[test]
fn local_can_control_is_true_for_uncontrolled_room() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    assert_eq!(session.local_can_control(), Some(true));
}

#[test]
fn local_can_control_requires_controller_flag_for_controlled_room() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    assert_eq!(session.local_can_control(), Some(false));

    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
            )
            .expect("controller update should apply");
    assert_eq!(session.local_can_control(), Some(true));
}

#[test]
fn controller_auth_success_sets_controller_flag_for_target_user() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
    assert_eq!(session.local_can_control(), Some(false));

    session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");
    assert_eq!(session.user_controller("alice"), Some(true));
    assert_eq!(session.local_can_control(), Some(true));
}

#[test]
fn controller_auth_success_emits_transition_notification() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");

    assert_eq!(
        session.runtime_actions_for_controller_auth_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyControllerAuthTransition(
            ControllerAuthTransitionNotification::Succeeded {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            },
        )]
    );
    assert!(
        session
            .runtime_actions_for_controller_auth_notifications_if_needed()
            .is_empty(),
        "controller auth notifications should drain after first retrieval"
    );
}

#[test]
fn controller_auth_success_for_same_room_user_emits_transition_notification() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"controllerAuth":{"user":"bob","room":"room1","success":true}}}"#,
        )
        .expect("controller auth success should apply");

    assert_eq!(
        session.runtime_actions_for_controller_auth_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyControllerAuthTransition(
            ControllerAuthTransitionNotification::Succeeded {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: false,
            },
        )]
    );
    assert_eq!(session.user_controller("bob"), Some(true));
}

#[test]
fn controller_auth_success_hides_from_osd_when_same_room_osd_is_disabled() {
    let mut session = ClientSession::default();
    session.behavior_config_mut().show_same_room_osd = false;
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"controllerAuth":{"user":"alice","room":"room1","success":true}}}"#,
        )
        .expect("controller auth success should apply");

    assert_eq!(
        session.runtime_actions_for_controller_auth_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyControllerAuthTransition(
            ControllerAuthTransitionNotification::Succeeded {
                username: "alice".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: true,
            },
        )]
    );
}

#[test]
fn controller_auth_success_for_different_room_suppresses_transition_notification() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(
            r#"{"Set":{"controllerAuth":{"user":"bob","room":"room2","success":true}}}"#,
        )
        .expect("controller auth success should apply");

    assert!(
        session
            .runtime_actions_for_controller_auth_notifications_if_needed()
            .is_empty(),
        "controller-auth success should only notify in local room"
    );
    assert_eq!(session.user_controller("bob"), Some(true));
}

#[test]
fn controller_auth_failure_emits_transition_notification() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");

    assert_eq!(
        session.runtime_actions_for_controller_auth_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyControllerAuthTransition(
            ControllerAuthTransitionNotification::Failed {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            },
        )]
    );
    assert!(
        session
            .runtime_actions_for_controller_auth_notifications_if_needed()
            .is_empty(),
        "controller auth notifications should drain after first retrieval"
    );
}

#[test]
fn controller_auth_failure_hide_from_osd_respects_show_noncontroller_osd_setting() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");
    assert_eq!(
        session.runtime_actions_for_controller_auth_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyControllerAuthTransition(
            ControllerAuthTransitionNotification::Failed {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            },
        )]
    );

    session.behavior_config_mut().show_noncontroller_osd = true;
    session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");
    assert_eq!(
        session.runtime_actions_for_controller_auth_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyControllerAuthTransition(
            ControllerAuthTransitionNotification::Failed {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            },
        )]
    );
}

#[test]
fn controller_auth_failure_for_other_user_suppresses_transition_notification() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

    session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"bob","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");

    assert!(
        session
            .runtime_actions_for_controller_auth_notifications_if_needed()
            .is_empty(),
        "controller-auth failure should only notify for local user"
    );
}

#[test]
fn new_controlled_room_message_queues_room_switch_and_auth_request() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.3.0"}}"#,
        )
        .expect("hello should apply");
    assert_eq!(session.local_can_control(), Some(true));

    session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}"#,
            )
            .expect("new controlled room message should apply");

    assert_eq!(
        session.model.room.name.as_deref(),
        Some("+room:ABCDEF123456")
    );
    assert_eq!(session.user_room("alice"), Some("+room:ABCDEF123456"));
    assert_eq!(session.user_controller("alice"), Some(false));
    assert_eq!(session.local_can_control(), Some(false));

    let actions = session.runtime_actions_for_controller_reidentify_if_needed();
    assert_eq!(
        actions,
        vec![
            ClientRuntimeAction::SetRoom {
                room: "+room:ABCDEF123456".to_owned(),
            },
            ClientRuntimeAction::RequestUserList,
            ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting {
                    room: "+room:ABCDEF123456".to_owned(),
                },
            ),
            ClientRuntimeAction::RequestControllerAuth {
                room: "+room:ABCDEF123456".to_owned(),
                password: "AB-123-456".into(),
            },
        ]
    );
    assert!(
        session
            .runtime_actions_for_controller_reidentify_if_needed()
            .is_empty(),
        "controller reidentify actions should drain after first retrieval"
    );
}

#[test]
fn new_controlled_room_message_resets_autoplay_state_like_python_client() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.3.0"}}"#,
        )
        .expect("hello should apply");
    session.set_autoplay_enabled(true);
    session.start_autoplay_countdown();
    assert!(session.autoplay_enabled());
    assert!(session.autoplay_timer_is_running());

    session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}"#,
            )
            .expect("new controlled room message should apply");

    assert!(
        !session.autoplay_enabled(),
        "creating a controlled room should reset autoplay like the Python client"
    );
    assert!(
        !session.autoplay_timer_is_running(),
        "creating a controlled room should stop any running autoplay countdown"
    );
}

#[test]
fn controller_reidentify_action_emits_after_hello_when_password_is_stored() {
    let mut session = ClientSession::default();
    session.remember_control_password_for_room("+room:ABCDEF123456", "ab-123-456 !!");

    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.3.0"}}"#,
            )
            .expect("hello should apply");

    let actions = session.runtime_actions_for_controller_reidentify_if_needed();
    assert_eq!(
        actions,
        vec![
            ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting {
                    room: "+room:ABCDEF123456".to_owned(),
                },
            ),
            ClientRuntimeAction::RequestControllerAuth {
                room: "+room:ABCDEF123456".to_owned(),
                password: "AB-123-456".into(),
            },
        ]
    );
    assert!(
        session
            .runtime_actions_for_controller_reidentify_if_needed()
            .is_empty(),
        "controller reidentify actions should drain after first retrieval"
    );
}

#[test]
fn new_controlled_room_message_stores_password_for_future_reidentify() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}"#,
            )
            .expect("new controlled room message should apply");
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.3.0"}}"#,
            )
            .expect("hello should apply");

    assert_eq!(
        session.runtime_actions_for_controller_reidentify_if_needed(),
        vec![
            ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting {
                    room: "+room:ABCDEF123456".to_owned(),
                },
            ),
            ClientRuntimeAction::RequestControllerAuth {
                room: "+room:ABCDEF123456".to_owned(),
                password: "AB-123-456".into(),
            },
        ]
    );
}

#[test]
fn manual_controller_auth_success_stores_password_for_future_reidentify() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true}}}"#,
            )
            .expect("hello should apply");
    assert!(
        !session
            .runtime_actions_for_local_controller_auth_request(
                "+room:ABCDEF123456".to_owned(),
                "ab_123-456!".into(),
            )
            .is_empty(),
        "manual controller auth should record the attempted password"
    );

    session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");

    session.reset_sync_state_for_reconnect();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true}}}"#,
            )
            .expect("reconnect hello should apply");

    assert_eq!(
        session.runtime_actions_for_controller_reidentify_if_needed(),
        vec![
            ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting {
                    room: "+room:ABCDEF123456".to_owned(),
                },
            ),
            ClientRuntimeAction::RequestControllerAuth {
                room: "+room:ABCDEF123456".to_owned(),
                password: "AB123-456".into(),
            },
        ]
    );
}

#[test]
fn new_controlled_room_message_emits_creation_notification() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"ab 123 456"}}}"#,
            )
            .expect("new controlled room message should apply");

    assert_eq!(
        session.runtime_actions_for_controlled_room_creation_notifications_if_needed(),
        vec![ClientRuntimeAction::NotifyControlledRoomCreation(
            ControlledRoomCreationNotification::Created {
                room: "+room:ABCDEF123456".to_owned(),
                password: "AB123456".into(),
            },
        )]
    );
    assert!(
        session
            .runtime_actions_for_controlled_room_creation_notifications_if_needed()
            .is_empty(),
        "controlled room creation notifications should drain after first retrieval"
    );
}
