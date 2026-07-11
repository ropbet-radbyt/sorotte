use super::*;

#[test]
fn reset_sync_state_for_reconnect_clears_readiness_support_until_next_hello() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"setOthersReadiness":true}}}"#,
            )
            .expect("hello should apply");

    session.reset_sync_state_for_reconnect();

    assert_eq!(
        session.connection_phase(),
        &ConnectionPhase::Reconnecting { attempt: 0 }
    );
    assert!(!session.server_readiness_supported());
    assert!(!session.server_set_others_readiness_supported());
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

    assert_eq!(
        session.connection_phase(),
        &ConnectionPhase::Reconnecting { attempt: 0 }
    );
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
