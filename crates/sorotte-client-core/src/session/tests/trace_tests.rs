use super::*;

#[test]
fn python_trace_fanout_scenario_reconciles_client_sessions() {
    let sessions = replay_python_trace_fixture("server_runtime_fanout.python_trace.json");

    let client_1 = sessions
        .get("client-1")
        .expect("fanout trace should include client-1 session");
    assert_eq!(client_1.username(), Some("alice"));
    assert_eq!(client_1.room(), Some("room1"));
    assert_eq!(client_1.user_room("bob"), Some("room2"));
    assert_eq!(client_1.user_ready("alice"), Some(true));
    assert_eq!(
        client_1.user_ready("bob"),
        None,
        "legacy clients without readiness support must remain unknown, not not-ready"
    );
    let client_1_playlist = client_1
        .current_room_playlist()
        .expect("client-1 should have current room playlist");
    assert!(client_1_playlist.files.is_empty());

    let client_2 = sessions
        .get("client-2")
        .expect("fanout trace should include client-2 session");
    assert_eq!(client_2.username(), Some("bob"));
    assert_eq!(client_2.room(), Some("room2"));
    assert_eq!(client_2.user_room("alice"), Some("room1"));
    assert_eq!(client_2.user_ready("alice"), Some(true));
    let client_2_playstate = client_2
        .current_room_playstate()
        .expect("client-2 should have current room playstate");
    assert_eq!(client_2_playstate.position, Some(10.0));
    assert_eq!(client_2_playstate.paused, Some(false));
    assert_eq!(client_2_playstate.do_seek, Some(false));
    assert_eq!(client_2_playstate.set_by.as_deref(), Some("bob"));
}

#[test]
fn python_trace_cross_room_ready_list_reconciles_room_membership_and_readiness() {
    let sessions =
        replay_python_trace_fixture("server_runtime_cross_room_ready_list.python_trace.json");

    let client_3 = sessions
        .get("client-3")
        .expect("cross-room trace should include client-3 session");
    assert_eq!(client_3.username(), Some("carol"));
    assert_eq!(client_3.room(), Some("room1"));
    assert_eq!(client_3.user_room("alice"), Some("room1"));
    assert_eq!(client_3.user_room("bob"), Some("room1"));
    assert_eq!(client_3.user_room("carol"), Some("room1"));
    assert_eq!(client_3.user_ready("alice"), Some(true));
    assert_eq!(
        client_3.user_ready("bob"),
        None,
        "legacy clients without readiness support must remain unknown, not not-ready"
    );
    assert_eq!(client_3.user_ready("carol"), Some(true));
    assert_eq!(
        client_3.room_playstate("room1"),
        None,
        "membership replay must not synthesize playstate without a State message"
    );
}

#[test]
fn python_trace_controlled_room_state_forced_correction_reconciles_forced_state_and_room_membership()
 {
    let sessions = replay_python_trace_fixture(
        "server_runtime_controlled_room_state_forced_correction.python_trace.json",
    );
    let controlled_room = "+room1:CB39A19549E8";

    let client_1 = sessions
        .get("client-1")
        .expect("forced-correction trace should include client-1 session");
    assert_eq!(client_1.username(), Some("alice"));
    assert_eq!(client_1.room(), Some(controlled_room));
    assert_eq!(client_1.user_room("alice"), Some(controlled_room));
    assert_eq!(client_1.user_room("bob"), Some(controlled_room));
    let client_1_playstate = client_1
        .current_room_playstate()
        .expect("client-1 should track controlled room playstate");
    assert_eq!(client_1_playstate.position, Some(0.0));
    assert_eq!(client_1_playstate.paused, Some(true));
    assert_eq!(client_1_playstate.do_seek, Some(true));
    let client_1_playlist = client_1
        .current_room_playlist()
        .expect("client-1 should keep controlled room playlist snapshot");
    assert!(
        client_1_playlist.files.is_empty(),
        "controlled room playlist should remain empty in forced-correction scenario"
    );

    let client_2 = sessions
        .get("client-2")
        .expect("forced-correction trace should include client-2 session");
    assert_eq!(client_2.username(), Some("bob"));
    assert_eq!(client_2.room(), Some(controlled_room));
    assert_eq!(client_2.user_room("alice"), Some(controlled_room));
    assert_eq!(client_2.user_room("bob"), Some(controlled_room));
    let client_2_playstate = client_2
        .current_room_playstate()
        .expect("client-2 should track controlled room playstate");
    assert_eq!(client_2_playstate.position, Some(0.0));
    assert_eq!(client_2_playstate.paused, Some(true));
    assert_eq!(client_2_playstate.do_seek, Some(true));
    let client_2_playlist = client_2
        .current_room_playlist()
        .expect("client-2 should keep controlled room playlist snapshot");
    assert!(
        client_2_playlist.files.is_empty(),
        "controlled room playlist should remain empty in forced-correction scenario"
    );
}
