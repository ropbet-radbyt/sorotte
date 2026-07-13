use super::*;
use crate::RoomPlaystateAuthority;

#[test]
fn reconcile_state_builds_client_ignore_and_waits_for_ack_before_applying_new_global_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("initial state should apply");

    let outbound =
        session.reconcile_state_and_build_response(StatePayload::new(), 12.0, false, 123.0, 0.12);
    let outbound_playstate = outbound
        .playstate
        .as_ref()
        .expect("outbound state should include playstate");
    assert_eq!(outbound_playstate.position, Some(12.0));
    assert_eq!(outbound_playstate.paused, Some(false));
    assert_eq!(outbound_playstate.do_seek, Some(true));
    assert_eq!(session.client_ignoring_on_the_fly(), 1);
    assert_eq!(
        outbound
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|ignore| ignore.client),
        Some(1)
    );

    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("local state reflection should apply");

    let inbound_without_ack = StatePayload::new().with_playstate(
        PlaystatePayload::new()
            .with_position(99.0)
            .with_paused(true)
            .with_do_seek(true)
            .with_set_by("bob"),
    );
    let outbound_while_waiting =
        session.reconcile_state_and_build_response(inbound_without_ack, 12.0, false, 124.0, 0.13);
    assert!(
        outbound_while_waiting.playstate.is_none(),
        "outbound playstate should be suppressed while waiting for client ignore ack"
    );
    let preserved = session
        .current_room_playstate()
        .expect("room playstate should remain available");
    assert_eq!(preserved.position, Some(1.0));
    assert_eq!(session.client_ignoring_on_the_fly(), 1);

    let inbound_with_ack = StatePayload::new()
        .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
        .with_playstate(
            PlaystatePayload::new()
                .with_position(99.0)
                .with_paused(true)
                .with_do_seek(true)
                .with_set_by("bob"),
        );
    let outbound_after_ack =
        session.reconcile_state_and_build_response(inbound_with_ack, 99.0, true, 125.0, 0.14);
    assert!(
        outbound_after_ack.playstate.is_some(),
        "outbound playstate should resume once ack clears client ignore"
    );
    assert_eq!(session.client_ignoring_on_the_fly(), 0);
    let updated = session
        .current_room_playstate()
        .expect("room playstate should be updated after ack");
    assert_eq!(updated.position, Some(99.0));
    assert_eq!(updated.paused, Some(true));
    assert_eq!(updated.do_seek, Some(true));
    assert_eq!(updated.set_by.as_deref(), Some("bob"));
}

#[test]
fn reconcile_state_echoes_server_ignore_and_clears_server_counter_after_emit() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("initial state should apply");

    let inbound =
        StatePayload::new().with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_server(3));
    let outbound = session.reconcile_state_and_build_response(inbound, 0.0, true, 200.0, 0.2);

    let ignore = outbound
        .ignoring_on_the_fly
        .as_ref()
        .expect("outbound should include ignoringOnTheFly");
    assert_eq!(ignore.server, Some(3));
    assert_eq!(session.server_ignoring_on_the_fly(), 0);
}

#[test]
fn local_pause_toggle_uses_room_pause_state_when_local_telemetry_is_unknown() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    let actions = session.runtime_actions_for_local_pause_toggle();

    assert_eq!(actions, vec![ClientRuntimeAction::SetPaused(false)]);
    assert_eq!(session.local_paused(), Some(false));
}

#[test]
fn client_session_current_room_playstate_remote_authority_requires_remote_user_or_set_by() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(r#"{"State":{"playstate":{"position":5.0,"paused":true}}}"#)
        .expect("playstate without setBy should apply");

    assert!(
        !session.current_room_playstate_has_remote_authority(),
        "room playstate without setBy should not be treated as authoritative when no remote users are known"
    );
    assert_eq!(
        session.current_room_playstate_authority(),
        None,
        "unattributed state in an otherwise empty room has no coordinator authority"
    );

    session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","duration":95.5}}}}}"#,
            )
            .expect("remote user should apply");
    assert!(
        session.current_room_playstate_has_remote_authority(),
        "known remote users should make an un-attributed room playstate authoritative"
    );

    session
        .apply_message_json(r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#)
        .expect("remote user leaving should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":6.0,"paused":true,"setBy":"alice"}}}"#,
        )
        .expect("self-origin playstate should apply");
    assert!(
        !session.current_room_playstate_has_remote_authority(),
        "self-origin room playstate should not be treated as remotely authoritative"
    );
    assert_eq!(
        session.current_room_playstate_authority(),
        Some(RoomPlaystateAuthority::LegacyLocalEcho)
    );

    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":7.0,"paused":true,"setBy":"bob"}}}"#,
        )
        .expect("remote-origin playstate should apply");
    assert!(
        session.current_room_playstate_has_remote_authority(),
        "remote-origin room playstate should remain authoritative even before a later user list refresh"
    );
}

#[test]
fn local_room_command_target_with_legacy_fallback_prefers_local_room_over_file_name() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"default-room"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"+Test:77F8DA30FB3E"},"file":{"name":"episode1.mkv"}}}}}"#,
            )
            .expect("local user update should apply");

    assert_eq!(
        session.local_room_command_target_with_legacy_fallback("fallback-room"),
        "+Test:77F8DA30FB3E"
    );
}
